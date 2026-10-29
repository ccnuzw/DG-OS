//! Post-mount module crash handling (D055, D059).
//!
//! The page reports a caught render error. This service marks the runtime
//! failed so the UI can offer a reload, stores a sanitized copy, and disables
//! a module that keeps crashing. It never stores message text: an error
//! message is module-controlled and may carry anything.

use crate::ModuleOperationManager;
use dgos_db::ModuleCrashRepository;
use dgos_modules::ModuleRuntimeHandle;
use dgos_types::{
    DgosError, DgosResult, HostSession, ModuleCrashView, ModuleOperationAccepted,
    ModuleOperationErrorKind,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Crashes of one version inside the window that disable the module.
pub const AUTO_DISABLE_CRASHES: u64 = 3;
pub const AUTO_DISABLE_WINDOW_SECS: i64 = 10 * 60;
/// Window of the `recent_crashes` count shown on the management page.
pub const RECENT_WINDOW_SECS: i64 = 24 * 60 * 60;

const MAX_FRAMES: usize = 50;
const MAX_STACK_BYTES: usize = 8 * 1024;
const MAX_NAME: usize = 64;
const MAX_NUMBER_DIGITS: usize = 9;

/// Only `[A-Za-z0-9_]`, at most 64 characters. Anything else is dropped.
pub fn sanitize_error_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(MAX_NAME)
        .collect();
    if name.is_empty() {
        "Error".to_owned()
    } else {
        name
    }
}

fn frame_function(raw: &str) -> String {
    let raw = raw.trim();
    let valid = !raw.is_empty()
        && raw.len() <= MAX_NAME
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.' | '<' | '>'));
    if valid {
        raw.to_owned()
    } else {
        "anonymous".to_owned()
    }
}

fn number(raw: &str) -> Option<&str> {
    (!raw.is_empty() && raw.len() <= MAX_NUMBER_DIGITS && raw.bytes().all(|b| b.is_ascii_digit()))
        .then_some(raw)
}

/// `line:col` from a location such as `blob:http://x/abc:12:34`; the file part
/// is discarded.
fn location(raw: &str) -> Option<(String, String)> {
    let mut parts = raw.trim().rsplitn(3, ':');
    let column = number(parts.next()?)?;
    let line = number(parts.next()?)?;
    parts.next()?;
    Some((line.to_owned(), column.to_owned()))
}

fn frame(line: &str) -> Option<String> {
    let line = line.trim();
    let (function, loc) = if let Some(rest) = line.strip_prefix("at ") {
        match rest.strip_suffix(')').and_then(|r| r.rsplit_once(" (")) {
            Some((function, loc)) => (function, loc),
            None => ("", rest),
        }
    } else {
        let (function, loc) = line.split_once('@')?;
        (function, loc)
    };
    let (line_no, column) = location(loc)?;
    Some(format!(
        "{}@module:{line_no}:{column}",
        frame_function(function)
    ))
}

/// Keeps frame lines only, rewritten as `function@module:line:col`. V8 puts
/// `Name: message` on the first stack line and JavaScriptCore does not; either
/// way a line that is not a frame is dropped, so no message text, URL or path
/// is kept. Module code shares the page realm and can fabricate any stack
/// (D042); this bounds what a first-party bug can leak, it is not a defense
/// against a hostile module.
pub fn sanitize_stack(raw: &str) -> String {
    let mut out = String::new();
    for frame in raw.lines().filter_map(frame).take(MAX_FRAMES) {
        if out.len() + frame.len() + 1 > MAX_STACK_BYTES {
            break;
        }
        out.push_str(&frame);
        out.push('\n');
    }
    out
}

/// Backoff of the background disable retry (D059). Starts at `initial`,
/// doubles up to `max`, and gives up (with a warning) after `total`.
#[derive(Clone, Copy, Debug)]
pub struct DisableRetryPolicy {
    pub initial: Duration,
    pub max: Duration,
    pub total: Duration,
}

impl Default for DisableRetryPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(50),
            max: Duration::from_secs(5),
            total: Duration::from_secs(10 * 60),
        }
    }
}

/// Most crash instants kept per (user, module, version) in process.
const MAX_WINDOW_ENTRIES: usize = 64;

type WindowKey = (String, String, String);

#[derive(Default)]
struct Shared {
    /// In-process sliding window, independent of the crash log (D059).
    windows: Mutex<HashMap<WindowKey, VecDeque<Instant>>>,
    /// Modules that already have a disable driver task.
    disabling: Mutex<HashSet<(String, String)>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone)]
pub struct ModuleCrashService {
    crashes: ModuleCrashRepository,
    operations: ModuleOperationManager,
    shared: Arc<Shared>,
    retry: DisableRetryPolicy,
}

impl ModuleCrashService {
    pub fn new(crashes: ModuleCrashRepository, operations: ModuleOperationManager) -> Self {
        Self {
            crashes,
            operations,
            shared: Arc::new(Shared::default()),
            retry: DisableRetryPolicy::default(),
        }
    }

    /// Replace the disable retry backoff (tests use millisecond values).
    pub fn with_disable_retry(mut self, retry: DisableRetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Record a crash of `runtime`'s page instance. The runtime comes from the
    /// call router, so module, version and user are host-derived.
    ///
    /// Order (D059): the host fact first (the runtime becomes `failed`), then
    /// the in-process count, then the log. A log failure is returned to the
    /// caller only after the auto-disable decision was made.
    pub async fn report(
        &self,
        runtime: &ModuleRuntimeHandle,
        source: &str,
        error_name: &str,
        stack: &str,
    ) -> DgosResult<()> {
        let (module_id, version, _) = runtime
            .module_identity()
            .ok_or(DgosError::PermissionDenied)?;
        let user_id = runtime.context().user_id();
        let source = if source == "render" {
            "render"
        } else {
            "other"
        };
        // The instance is dead in the page; make the host say so. A report
        // for a runtime that was already replaced or stopped is refused here
        // and that refusal is the expected answer.
        if let Err(kind) = self
            .operations
            .report_runtime_lost(module_id, runtime.runtime_id())
        {
            if !matches!(
                kind,
                ModuleOperationErrorKind::NotRunning | ModuleOperationErrorKind::VersionConflict
            ) {
                tracing::warn!(
                    module_id,
                    code = kind.code(),
                    "crashed runtime not marked failed"
                );
            }
        }
        let in_process = self.note_crash(user_id, module_id, version);
        let recorded = self
            .crashes
            .record(dgos_db::CrashRecord {
                user_id,
                module_id,
                module_version: version,
                runtime_id: runtime.runtime_id(),
                source,
                error_name: &sanitize_error_name(error_name),
                stack: &sanitize_stack(stack),
            })
            .await;
        let since = chrono::Utc::now().timestamp() - AUTO_DISABLE_WINDOW_SECS;
        let persisted = match self
            .crashes
            .count_since(user_id, module_id, Some(version), since)
            .await
        {
            Ok(count) => count,
            Err(error) => {
                tracing::warn!(module_id, %error, "crash count unavailable; using in-process count");
                0
            }
        };
        if persisted.max(in_process) >= AUTO_DISABLE_CRASHES {
            self.ensure_disabled(user_id, module_id);
        }
        recorded.map(|_| ())
    }

    /// Disable every enabled module whose current version already has
    /// `AUTO_DISABLE_CRASHES` logged crashes inside the window (D059). Run it
    /// after recovery and before auto-load. Returns the module ids a disable
    /// was submitted for; the disables are driven to completion in the
    /// background. A module whose count cannot be read is skipped with a
    /// warning and the first such error is returned after the loop, so the
    /// other modules are still evaluated.
    pub async fn enforce_on_startup(&self, session: &HostSession) -> DgosResult<Vec<String>> {
        let enabled = self
            .operations
            .enabled_modules(session)
            .await
            .map_err(DgosError::ModuleOperation)?;
        let since = chrono::Utc::now().timestamp() - AUTO_DISABLE_WINDOW_SECS;
        let mut disabled = Vec::new();
        let mut first_error = None;
        for (module_id, version) in enabled {
            match self
                .crashes
                .count_since(session.user_id(), &module_id, Some(&version), since)
                .await
            {
                Ok(count) if count >= AUTO_DISABLE_CRASHES => {
                    self.ensure_disabled(session.user_id(), &module_id);
                    disabled.push(module_id);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%module_id, %error, "startup crash evaluation failed");
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(disabled),
        }
    }

    /// Modules that currently have a disable driver task (tests).
    #[cfg(test)]
    fn disabling_count(&self) -> usize {
        locked(&self.shared.disabling).len()
    }

    /// Count `now` in the in-process window and return how many crashes of
    /// this version fall inside it.
    fn note_crash(&self, user_id: &str, module_id: &str, version: &str) -> u64 {
        let now = Instant::now();
        let window = Duration::from_secs(AUTO_DISABLE_WINDOW_SECS as u64);
        let mut windows = locked(&self.shared.windows);
        let entries = windows
            .entry((user_id.to_owned(), module_id.to_owned(), version.to_owned()))
            .or_default();
        entries.push_back(now);
        while entries
            .front()
            .is_some_and(|at| now.duration_since(*at) > window)
        {
            entries.pop_front();
        }
        while entries.len() > MAX_WINDOW_ENTRIES {
            entries.pop_front();
        }
        entries.len() as u64
    }

    /// Make sure a disable of `module_id` gets accepted and applied. The
    /// first submission happens here; a refusal or a failed operation is
    /// retried by one background task per module, with bounded backoff.
    fn ensure_disabled(&self, user_id: &str, module_id: &str) {
        let key = (user_id.to_owned(), module_id.to_owned());
        if !locked(&self.shared.disabling).insert(key.clone()) {
            return;
        }
        let session = HostSession::local_default();
        let first = self.operations.disable(&session, module_id);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            locked(&self.shared.disabling).remove(&key);
            tracing::warn!(module_id, "no async runtime; module disable not retried");
            return;
        };
        let service = self.clone();
        let module = module_id.to_owned();
        runtime.spawn(async move {
            service.drive_disable(&session, &module, first).await;
            locked(&service.shared.disabling).remove(&key);
        });
    }

    async fn drive_disable(
        &self,
        session: &HostSession,
        module_id: &str,
        mut attempt: Result<ModuleOperationAccepted, ModuleOperationErrorKind>,
    ) {
        let started = Instant::now();
        let mut delay = self.retry.initial;
        loop {
            let failure = match attempt {
                Ok(accepted) => match self.operations.wait(&accepted.operation_id).await {
                    Ok(view) => match view.failure {
                        None => return,
                        Some(failure) => failure.kind,
                    },
                    Err(kind) => kind,
                },
                Err(kind) => kind,
            };
            match failure {
                // Nothing left to disable.
                ModuleOperationErrorKind::ModuleNotFound | ModuleOperationErrorKind::Disabled => {
                    return
                }
                _ => {}
            }
            if started.elapsed() >= self.retry.total {
                tracing::warn!(
                    module_id,
                    code = failure.code(),
                    "auto-disable of a crashing module was not applied"
                );
                return;
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(self.retry.max);
            attempt = self.operations.disable(session, module_id);
        }
    }

    pub async fn list(
        &self,
        session: &HostSession,
        module_id: &str,
    ) -> DgosResult<Vec<ModuleCrashView>> {
        Ok(self
            .crashes
            .list(session.user_id(), module_id, 50)
            .await?
            .into_iter()
            .map(|row| ModuleCrashView {
                crash_id: row.id,
                module_id: row.module_id,
                module_version: row.module_version,
                source: row.source,
                error_name: row.error_name,
                stack: row.stack,
                created_at: row.created_at,
            })
            .collect())
    }

    /// Crashes in the last 24 hours, for the management page.
    pub async fn recent(&self, session: &HostSession, module_id: &str) -> DgosResult<u32> {
        let since = chrono::Utc::now().timestamp() - RECENT_WINDOW_SECS;
        let count = self
            .crashes
            .count_since(session.user_id(), module_id, None, since)
            .await?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_names_keep_only_identifier_characters() {
        assert_eq!(sanitize_error_name("TypeError"), "TypeError");
        assert_eq!(
            sanitize_error_name("Bad Thing: sk-live-123!"),
            "BadThingsklive123"
        );
        assert_eq!(sanitize_error_name("   "), "Error");
        assert_eq!(sanitize_error_name(&"A".repeat(500)).len(), 64);
    }

    #[test]
    fn v8_stacks_lose_the_message_line_and_every_url() {
        let raw = "TypeError: token sk-live-SECRET is invalid\n    at render (blob:http://tauri.localhost/6f1c-aa:12:34)\n    at Object.run (https://evil.example/x.js?key=SECRET:5:6)\n    at blob:http://tauri.localhost/6f1c-aa:7:8";
        let clean = sanitize_stack(raw);
        assert_eq!(
            clean,
            "render@module:12:34\nObject.run@module:5:6\nanonymous@module:7:8\n"
        );
        for leaked in ["SECRET", "sk-live", "http", "evil", "blob", "TypeError"] {
            assert!(!clean.contains(leaked), "{leaked}");
        }
    }

    #[test]
    fn webkit_stacks_are_normalized_and_message_text_is_dropped() {
        let raw = "render@blob:http://tauri.localhost/aa:3:9\n@blob:http://tauri.localhost/aa:1:1\nglobal code\nsecret password is hunter2";
        assert_eq!(
            sanitize_stack(raw),
            "render@module:3:9\nanonymous@module:1:1\n"
        );
    }

    #[test]
    fn a_hostile_function_name_becomes_anonymous() {
        let raw = "    at key=SECRET stuff (blob:x:1:2)";
        assert_eq!(sanitize_stack(raw), "anonymous@module:1:2\n");
    }

    #[test]
    fn frames_and_bytes_are_bounded() {
        let one = "    at f (blob:x:1:2)\n";
        assert_eq!(sanitize_stack(&one.repeat(500)).lines().count(), 50);
        // Digits beyond the bound are not a frame.
        assert_eq!(sanitize_stack("    at f (blob:x:1:12345678901)"), "");
        assert_eq!(sanitize_stack(""), "");
    }
}

/// D059: crash reports against a real manager, supervisor and database.
#[cfg(test)]
mod reliability_tests {
    use super::*;
    use crate::ModuleReleaseService;
    use async_trait::async_trait;
    use dgos_db::sea_orm_migration::MigratorTrait;
    use dgos_db::{
        migrations::Migrator,
        sea_orm::{ConnectionTrait, Database, DatabaseConnection},
    };
    use dgos_modules::{
        LocalArtifactLoader, LocalModuleSources, ModuleLoadError, ModuleLoader, ModulePublisher,
        VerifiedModuleArtifact,
    };
    use dgos_types::ModuleRuntimeStatusState;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex as StdMutex;
    use tempfile::{tempdir, TempDir};
    use tokio::sync::watch;

    const MODULE: &str = "module-1";
    const OTHER: &str = "module-2";
    const THIRD: &str = "module-3";
    const FOURTH: &str = "module-4";

    /// Real local checks; `stop` can be held shut to keep a module busy.
    #[derive(Default)]
    struct GateLoader {
        stop_gate: StdMutex<Option<watch::Receiver<bool>>>,
    }

    impl GateLoader {
        fn close_stop_gate(&self) -> watch::Sender<bool> {
            let (sender, receiver) = watch::channel(false);
            *self.stop_gate.lock().unwrap() = Some(receiver);
            sender
        }
    }

    #[async_trait]
    impl ModuleLoader for GateLoader {
        async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
            LocalArtifactLoader.smoke(artifact).await
        }

        async fn start(
            &self,
            runtime_id: &str,
            artifact: &VerifiedModuleArtifact,
        ) -> Result<(), ModuleLoadError> {
            LocalArtifactLoader.start(runtime_id, artifact).await
        }

        async fn stop(
            &self,
            _runtime_id: &str,
            _artifact: &VerifiedModuleArtifact,
        ) -> Result<(), ModuleLoadError> {
            let gate = self.stop_gate.lock().unwrap().clone();
            if let Some(mut gate) = gate {
                let _ = gate.wait_for(|open| *open).await;
            }
            Ok(())
        }
    }

    fn write_package(dir: &Path, module_id: &str, version: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("entry.js"),
            format!("export default '{module_id}@{version}';"),
        )
        .unwrap();
        // The digest excludes `artifact.sha256`, so write a placeholder first.
        let manifest = |sha256: &str| {
            serde_json::json!({
                "id": module_id, "name": "Example", "version": version, "entry": "entry.js",
                "artifact": {"version": version, "sha256": sha256},
                "permissions": ["providers.call"]
            })
            .to_string()
        };
        fs::write(dir.join("manifest.json"), manifest(&"0".repeat(64))).unwrap();
        let integrity = dgos_modules::package_integrity(dir).unwrap();
        let hex = integrity.strip_prefix("v2:").unwrap();
        fs::write(dir.join("manifest.json"), manifest(hex)).unwrap();
    }

    struct Fixture {
        _store: TempDir,
        db: DatabaseConnection,
        loader: Arc<GateLoader>,
        manager: ModuleOperationManager,
        service: ModuleCrashService,
        session: HostSession,
    }

    impl Fixture {
        async fn unrecovered(modules: &[&str]) -> Self {
            let db = Database::connect("sqlite::memory:").await.unwrap();
            Migrator::up(&db, None).await.unwrap();
            let store = tempdir().unwrap();
            let sources = tempdir().unwrap();
            let publisher = Arc::new(ModulePublisher::new(store.path(), db.clone()));
            let release = ModuleReleaseService::new(publisher.clone());
            for module in modules {
                let package = tempdir().unwrap();
                write_package(package.path(), module, "1.0.0");
                release.publish(package.path()).await.unwrap();
            }
            let loader = Arc::new(GateLoader::default());
            let manager = ModuleOperationManager::new(
                publisher,
                loader.clone(),
                LocalModuleSources::new(sources.path()),
            );
            let service =
                ModuleCrashService::new(ModuleCrashRepository::new(db.clone()), manager.clone())
                    .with_disable_retry(DisableRetryPolicy {
                        initial: Duration::from_millis(5),
                        max: Duration::from_millis(20),
                        total: Duration::from_secs(30),
                    });
            Self {
                _store: store,
                db,
                loader,
                manager,
                service,
                session: HostSession::local_default(),
            }
        }

        async fn new(modules: &[&str]) -> Self {
            let fixture = Self::unrecovered(modules).await;
            fixture.manager.recover().await.unwrap();
            fixture
        }

        async fn start(&self, module: &str) -> ModuleRuntimeHandle {
            let accepted = self.manager.start(&self.session, module).unwrap();
            let view = tokio::time::timeout(
                Duration::from_secs(5),
                self.manager.wait(&accepted.operation_id),
            )
            .await
            .expect("start did not finish")
            .unwrap();
            assert!(view.failure.is_none(), "{view:?}");
            self.manager.runtime_handle(module).unwrap()
        }

        fn state(&self, module: &str) -> ModuleRuntimeStatusState {
            self.manager.runtime_status(module).unwrap().state
        }

        async fn is_enabled(&self, module: &str) -> bool {
            self.manager
                .enabled_modules(&self.session)
                .await
                .unwrap()
                .iter()
                .any(|(id, _)| id == module)
        }

        async fn wait_disabled(&self, module: &str) {
            for _ in 0..1000 {
                if !self.is_enabled(module).await {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("{module} was never disabled");
        }

        async fn report(&self, handle: &ModuleRuntimeHandle) -> DgosResult<()> {
            self.service
                .report(handle, "render", "TypeError", "f@module:1:1\n")
                .await
        }

        async fn drop_crash_log(&self) {
            self.db
                .execute_unprepared("DROP TABLE module_crash_logs")
                .await
                .unwrap();
        }

        async fn seed_crash(&self, module: &str, version: &str, age_secs: i64) {
            let created_at = chrono::Utc::now().timestamp() - age_secs;
            let id = uuid::Uuid::new_v4();
            self.db
                .execute_unprepared(&format!(
                    "INSERT INTO module_crash_logs (id, user_id, module_id, module_version, \
                     runtime_id, source, error_name, stack, created_at) VALUES \
                     ('{id}', 'default-user', '{module}', '{version}', 'rt', 'render', \
                     'TypeError', '', {created_at})"
                ))
                .await
                .unwrap();
        }
    }

    async fn wait_for(mut condition: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("condition not reached");
    }

    #[tokio::test]
    async fn a_failing_crash_log_still_marks_the_runtime_failed_and_returns_the_error() {
        let fixture = Fixture::new(&[MODULE]).await;
        let handle = fixture.start(MODULE).await;
        assert_eq!(fixture.state(MODULE), ModuleRuntimeStatusState::Ready);
        fixture.drop_crash_log().await;

        assert!(fixture.report(&handle).await.is_err());
        assert_eq!(fixture.state(MODULE), ModuleRuntimeStatusState::Failed);
    }

    #[tokio::test]
    async fn the_in_process_count_disables_a_module_when_the_log_is_unavailable() {
        let fixture = Fixture::new(&[MODULE]).await;
        let handle = fixture.start(MODULE).await;
        fixture.drop_crash_log().await;

        for _ in 0..(AUTO_DISABLE_CRASHES - 1) {
            assert!(fixture.report(&handle).await.is_err());
        }
        assert!(fixture.is_enabled(MODULE).await);
        assert_eq!(fixture.service.disabling_count(), 0);

        assert!(fixture.report(&handle).await.is_err());
        fixture.wait_disabled(MODULE).await;
    }

    #[tokio::test]
    async fn a_disable_refused_because_the_module_is_busy_is_retried_until_applied() {
        let fixture = Fixture::new(&[MODULE]).await;
        let handle = fixture.start(MODULE).await;
        // A stop that cannot finish keeps the module's operation slot taken.
        let gate = fixture.loader.close_stop_gate();
        fixture.manager.stop(&fixture.session, MODULE).unwrap();

        for _ in 0..AUTO_DISABLE_CRASHES {
            fixture.report(&handle).await.unwrap();
        }
        // Refused (operation in progress), so a retry task owns it.
        assert_eq!(fixture.service.disabling_count(), 1);
        assert!(fixture.is_enabled(MODULE).await);
        // More crashes while it is retrying do not add a second task.
        fixture.report(&handle).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(fixture.service.disabling_count(), 1);
        assert!(fixture.is_enabled(MODULE).await);

        gate.send(true).unwrap();
        fixture.wait_disabled(MODULE).await;
        wait_for(|| fixture.service.disabling_count() == 0).await;
    }

    #[tokio::test]
    async fn startup_disables_modules_with_enough_recent_crashes_of_their_current_version() {
        let fixture = Fixture::new(&[MODULE, OTHER, THIRD, FOURTH]).await;
        for _ in 0..AUTO_DISABLE_CRASHES {
            fixture.seed_crash(MODULE, "1.0.0", 60).await;
            // Older than the window.
            fixture
                .seed_crash(OTHER, "1.0.0", AUTO_DISABLE_WINDOW_SECS + 60)
                .await;
            // A version that is not the published one.
            fixture.seed_crash(FOURTH, "0.9.0", 60).await;
        }
        // Below the threshold.
        for _ in 0..(AUTO_DISABLE_CRASHES - 1) {
            fixture.seed_crash(THIRD, "1.0.0", 60).await;
        }

        let disabled = fixture
            .service
            .enforce_on_startup(&fixture.session)
            .await
            .unwrap();
        assert_eq!(disabled, vec![MODULE.to_owned()]);
        fixture.wait_disabled(MODULE).await;
        for other in [OTHER, THIRD, FOURTH] {
            assert!(fixture.is_enabled(other).await, "{other}");
        }
    }

    #[tokio::test]
    async fn startup_enforcement_before_recovery_is_retried_until_the_host_is_available() {
        let fixture = Fixture::unrecovered(&[MODULE]).await;
        for _ in 0..AUTO_DISABLE_CRASHES {
            fixture.seed_crash(MODULE, "1.0.0", 10).await;
        }
        let disabled = fixture
            .service
            .enforce_on_startup(&fixture.session)
            .await
            .unwrap();
        assert_eq!(disabled, vec![MODULE.to_owned()]);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(fixture.is_enabled(MODULE).await);

        fixture.manager.recover().await.unwrap();
        fixture.wait_disabled(MODULE).await;
    }
}
