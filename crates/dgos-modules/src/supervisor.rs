//! Module runtime supervisor (D033).
//!
//! The supervisor is the only driver of runtime lifecycle transitions. It
//! starts publisher-issued handles through a `ModuleLoader`, drains and stops
//! them, records host-detected failures, and performs the minimal hot update:
//! the previous runtime keeps serving until the new package has been
//! verified, smoke-loaded, committed and started; a failed start rolls the
//! release back, and a failed rollback disables the module.

use crate::{
    error_class,
    loader::ModuleLoader,
    runtime::{ModuleRuntimeError, ModuleRuntimeHandle, ModuleRuntimeState},
    ModuleLoadError, ModulePackageError, ModulePublisher, ReleaseSnapshot, StagedPackage,
};
use dgos_types::HostSession;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ModuleSupervisorError {
    #[error(transparent)]
    Package(#[from] ModulePackageError),
    #[error(transparent)]
    Load(#[from] ModuleLoadError),
    #[error(transparent)]
    Runtime(#[from] ModuleRuntimeError),
    #[error("module runtime is already active")]
    AlreadyRunning,
    #[error("module has no matching runtime")]
    NotRunning,
    #[error("running module no longer matches the published release")]
    StaleRuntime,
    #[error("package belongs to a different module")]
    ModuleMismatch,
    #[error("release is already published but is not the current version")]
    VersionNotCurrent,
    #[error("module runtime did not drain before the timeout")]
    DrainTimeout,
    #[error("update failed and was rolled back to the previous release")]
    RolledBack(#[source] Box<ModuleSupervisorError>),
    #[error("update and rollback failed (module disabled: {module_disabled})")]
    RollbackFailed { module_disabled: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleUpdateOutcome {
    /// The same `(module_id, version, integrity)` is already active; nothing
    /// was restarted.
    Unchanged,
    /// The new runtime is active. `previous_stopped` is false when the
    /// previous runtime did not drain in time (or its loader failed to stop)
    /// and was left `failed` until its admitted requests settle.
    Switched { previous_stopped: bool },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleUpdateReport {
    pub operation_id: String,
    pub module_id: String,
    pub from_version: String,
    pub to_version: String,
    pub integrity: String,
    pub outcome: ModuleUpdateOutcome,
}

pub struct ModuleRuntimeSupervisor {
    publisher: Arc<ModulePublisher>,
    loader: Arc<dyn ModuleLoader>,
    drain_timeout: Duration,
    // Serializes start/stop/update. Request admission never takes this lock.
    lifecycle: tokio::sync::Mutex<()>,
    runtimes: Mutex<HashMap<String, ModuleRuntimeHandle>>,
}

impl ModuleRuntimeSupervisor {
    pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn new(publisher: Arc<ModulePublisher>, loader: Arc<dyn ModuleLoader>) -> Self {
        Self {
            publisher,
            loader,
            drain_timeout: Self::DEFAULT_DRAIN_TIMEOUT,
            lifecycle: tokio::sync::Mutex::new(()),
            runtimes: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_drain_timeout(
        publisher: Arc<ModulePublisher>,
        loader: Arc<dyn ModuleLoader>,
        drain_timeout: Duration,
    ) -> Self {
        Self {
            drain_timeout,
            ..Self::new(publisher, loader)
        }
    }

    /// The module's current runtime, including a failed or stopped one so
    /// the last lifecycle outcome stays observable.
    pub fn runtime(&self, module_id: &str) -> Option<ModuleRuntimeHandle> {
        self.registry().get(module_id).cloned()
    }

    /// Issue a handle for the current published release, smoke-load it,
    /// start it and mark it ready. A failed start stays registered as
    /// `failed` and never admits a request.
    pub async fn start(
        &self,
        session: &HostSession,
        module_id: &str,
    ) -> Result<ModuleRuntimeHandle, ModuleSupervisorError> {
        let _lifecycle = self.lifecycle.lock().await;
        if self
            .runtime(module_id)
            .is_some_and(|handle| is_live(handle.state()))
        {
            return Err(ModuleSupervisorError::AlreadyRunning);
        }
        let handle = self.publisher.runtime_handle(session, module_id).await?;
        self.registry().insert(module_id.to_owned(), handle.clone());
        let loaded = async {
            self.loader.smoke(handle.artifact()).await?;
            self.activate(&handle).await
        }
        .await;
        if let Err(error) = loaded {
            self.abort_start(&handle).await;
            return Err(error);
        }
        Ok(handle)
    }

    /// Drain and stop the module's runtime. New requests are refused at
    /// once; admitted requests settle before loader resources are released.
    pub async fn stop(&self, module_id: &str) -> Result<(), ModuleSupervisorError> {
        let _lifecycle = self.lifecycle.lock().await;
        let handle = self
            .runtime(module_id)
            .ok_or(ModuleSupervisorError::NotRunning)?;
        if handle.state() == ModuleRuntimeState::Stopped {
            return Ok(());
        }
        // Ready → draining. A failed runtime already refuses requests.
        let _ = handle.begin_draining();
        self.retire(&handle).await
    }

    /// Host-detected crash of one runtime instance: refuse new requests
    /// immediately; admitted requests settle on their own and the loader is
    /// released afterwards (in the background when a Tokio runtime is
    /// available, otherwise on the next `stop`). The runtime stays `failed`
    /// until an explicit `stop`. Reports for a retired instance are refused.
    pub fn report_failure(
        &self,
        module_id: &str,
        runtime_id: &str,
    ) -> Result<(), ModuleSupervisorError> {
        let handle = self
            .runtime(module_id)
            .filter(|handle| handle.runtime_id() == runtime_id)
            .ok_or(ModuleSupervisorError::NotRunning)?;
        handle.fail()?;
        self.release_when_settled(handle);
        Ok(())
    }

    pub async fn update(
        &self,
        session: &HostSession,
        module_id: &str,
        source: &Path,
    ) -> Result<ModuleUpdateReport, ModuleSupervisorError> {
        self.update_with_operation_id(session, module_id, source, Uuid::new_v4().to_string())
            .await
    }

    /// Minimal hot update of a ready runtime. The previous runtime keeps
    /// serving until the new one is ready; nothing admitted on the previous
    /// runtime is moved to or replayed on the new one.
    pub async fn update_with_operation_id(
        &self,
        session: &HostSession,
        module_id: &str,
        source: &Path,
        operation_id: impl Into<String>,
    ) -> Result<ModuleUpdateReport, ModuleSupervisorError> {
        let _lifecycle = self.lifecycle.lock().await;
        let previous = self
            .runtime(module_id)
            .filter(|handle| handle.state() == ModuleRuntimeState::Ready)
            .ok_or(ModuleSupervisorError::NotRunning)?;
        let snapshot = self
            .publisher
            .release_snapshot(module_id)
            .await?
            .filter(|snapshot| {
                previous.module_identity()
                    == Some((module_id, snapshot.version(), snapshot.integrity()))
            })
            .ok_or(ModuleSupervisorError::StaleRuntime)?;

        // 1. stage: manifest, local entry, symlink and digest validation.
        let staged = self
            .publisher
            .stage_with_operation_id(source, operation_id)?;
        if staged.module_id() != module_id {
            self.publisher.discard_staged(&staged);
            return Err(ModuleSupervisorError::ModuleMismatch);
        }
        let mut report = ModuleUpdateReport {
            operation_id: staged.operation_id().to_owned(),
            module_id: module_id.to_owned(),
            from_version: snapshot.version().to_owned(),
            to_version: staged.version().to_owned(),
            integrity: staged.integrity().to_owned(),
            outcome: ModuleUpdateOutcome::Unchanged,
        };
        if staged.version() == snapshot.version() && staged.integrity() == snapshot.integrity() {
            self.publisher.discard_staged(&staged);
            return Ok(report);
        }

        // 2. load smoke on the staged artifact, before the pointer can move.
        if let Err(error) = self.smoke_staged(&staged).await {
            self.publisher.discard_staged(&staged);
            self.audit(&report, "load_smoke", &error).await;
            return Err(error);
        }

        // 3. commit pointer + DB (the publisher records its own failures).
        if let Err(error) = self.publisher.commit(staged.clone()).await {
            self.publisher.discard_staged(&staged);
            return Err(error.into());
        }
        if let Ok(Some(current)) = self.publisher.release_snapshot(module_id).await {
            if current.version() == snapshot.version()
                && current.integrity() == snapshot.integrity()
            {
                // `commit` was an idempotent no-op for a version that exists
                // but is not current; re-activation needs a new version.
                return Err(ModuleSupervisorError::VersionNotCurrent);
            }
        }

        // 4. issue and start the new runtime; roll back on any failure.
        let activated = async {
            let candidate = self.publisher.runtime_handle(session, module_id).await?;
            if candidate.module_identity()
                != Some((module_id, staged.version(), staged.integrity()))
            {
                return Err(ModuleSupervisorError::StaleRuntime);
            }
            if let Err(error) = self.activate(&candidate).await {
                self.abort_start(&candidate).await;
                return Err(error);
            }
            Ok(candidate)
        }
        .await;
        let candidate = match activated {
            Ok(candidate) => candidate,
            Err(error) => return Err(self.roll_back(&report, &snapshot, &previous, error).await),
        };

        // 5. switch, then drain and stop the previous runtime.
        self.registry().insert(module_id.to_owned(), candidate);
        let _ = previous.begin_draining();
        report.outcome = ModuleUpdateOutcome::Switched {
            previous_stopped: self.retire(&previous).await.is_ok(),
        };
        Ok(report)
    }

    async fn smoke_staged(&self, staged: &StagedPackage) -> Result<(), ModuleSupervisorError> {
        let artifact = self.publisher.verify_staged(staged)?;
        self.loader.smoke(&artifact).await?;
        Ok(())
    }

    async fn activate(&self, handle: &ModuleRuntimeHandle) -> Result<(), ModuleSupervisorError> {
        self.loader
            .start(handle.runtime_id(), handle.artifact())
            .await?;
        handle.mark_ready()?;
        Ok(())
    }

    /// A runtime that failed to start never admitted a request; mark it
    /// failed and release whatever the loader may hold.
    async fn abort_start(&self, handle: &ModuleRuntimeHandle) {
        let _ = handle.fail();
        let _ = self
            .loader
            .stop(handle.runtime_id(), handle.artifact())
            .await;
    }

    /// Restore the previous release after a failed activation. On success
    /// the previous runtime keeps serving; otherwise it fails and the module
    /// is disabled so nothing can load an unknown release.
    async fn roll_back(
        &self,
        report: &ModuleUpdateReport,
        snapshot: &ReleaseSnapshot,
        previous: &ModuleRuntimeHandle,
        cause: ModuleSupervisorError,
    ) -> ModuleSupervisorError {
        self.audit(report, "activation", &cause).await;
        if self
            .publisher
            .reactivate(
                &report.operation_id,
                snapshot,
                &report.to_version,
                &report.integrity,
            )
            .await
            .is_ok()
        {
            return ModuleSupervisorError::RolledBack(Box::new(cause));
        }
        let _ = previous.fail();
        self.release_when_settled(previous.clone());
        let module_disabled = self
            .publisher
            .disable_module(
                &report.operation_id,
                &report.module_id,
                Some(&report.from_version),
                &report.to_version,
                "rollback_failed",
            )
            .await
            .is_ok();
        ModuleSupervisorError::RollbackFailed { module_disabled }
    }

    /// Wait for admitted requests to settle, release loader resources and
    /// stop. On timeout the runtime fails and is released in the background
    /// once it settles; in-flight results are never cancelled or faked.
    async fn retire(&self, handle: &ModuleRuntimeHandle) -> Result<(), ModuleSupervisorError> {
        if !handle.wait_drained(self.drain_timeout).await {
            let _ = handle.fail();
            self.release_when_settled(handle.clone());
            return Err(ModuleSupervisorError::DrainTimeout);
        }
        if let Err(error) = self
            .loader
            .stop(handle.runtime_id(), handle.artifact())
            .await
        {
            let _ = handle.fail();
            return Err(error.into());
        }
        handle.stop()?;
        Ok(())
    }

    fn release_when_settled(&self, handle: ModuleRuntimeHandle) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let loader = self.loader.clone();
        runtime.spawn(async move {
            handle.wait_settled().await;
            let _ = loader.stop(handle.runtime_id(), handle.artifact()).await;
        });
    }

    async fn audit(&self, report: &ModuleUpdateReport, stage: &str, error: &ModuleSupervisorError) {
        // Best effort: an audit failure must not change the lifecycle result.
        let _ = self
            .publisher
            .record_update_event(
                &report.operation_id,
                &report.module_id,
                Some(&report.from_version),
                &report.to_version,
                stage,
                "failure",
                Some(supervisor_error_class(error)),
            )
            .await;
    }

    fn registry(&self) -> MutexGuard<'_, HashMap<String, ModuleRuntimeHandle>> {
        self.runtimes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn is_live(state: ModuleRuntimeState) -> bool {
    matches!(
        state,
        ModuleRuntimeState::Starting | ModuleRuntimeState::Ready | ModuleRuntimeState::Draining
    )
}

fn supervisor_error_class(error: &ModuleSupervisorError) -> String {
    match error {
        ModuleSupervisorError::Package(error) => error_class(error),
        ModuleSupervisorError::Load(_) => "load".into(),
        ModuleSupervisorError::Runtime(_) => "runtime".into(),
        _ => "lifecycle".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        package_digest, LocalArtifactLoader, ModuleRequestOutcome, VerifiedModuleArtifact,
    };
    use async_trait::async_trait;
    use dgos_db::{
        entities::{module_update_events, modules},
        migrations::Migrator,
        sea_orm::{ActiveModelTrait, Database, DatabaseConnection, EntityTrait, Set},
    };
    use sea_orm_migration::MigratorTrait;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tempfile::{tempdir, TempDir};

    /// Real local checks, plus scripted start failures and loader accounting.
    #[derive(Default)]
    struct ScriptedLoader {
        fail_start_version: Mutex<Option<String>>,
        starts: AtomicUsize,
        stops: Mutex<Vec<String>>,
    }

    impl ScriptedLoader {
        fn fail_start(&self, version: Option<&str>) {
            *self.fail_start_version.lock().unwrap() = version.map(str::to_owned);
        }

        fn stops(&self) -> Vec<String> {
            self.stops.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ModuleLoader for ScriptedLoader {
        async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
            LocalArtifactLoader.smoke(artifact).await
        }

        async fn start(
            &self,
            runtime_id: &str,
            artifact: &VerifiedModuleArtifact,
        ) -> Result<(), ModuleLoadError> {
            if self.fail_start_version.lock().unwrap().as_deref() == Some(artifact.version()) {
                return Err(ModuleLoadError::Rejected("scripted start failure".into()));
            }
            self.starts.fetch_add(1, Ordering::SeqCst);
            LocalArtifactLoader.start(runtime_id, artifact).await
        }

        async fn stop(
            &self,
            runtime_id: &str,
            _artifact: &VerifiedModuleArtifact,
        ) -> Result<(), ModuleLoadError> {
            self.stops.lock().unwrap().push(runtime_id.to_owned());
            Ok(())
        }
    }

    struct Fixture {
        store: TempDir,
        db: DatabaseConnection,
        publisher: Arc<ModulePublisher>,
        loader: Arc<ScriptedLoader>,
        supervisor: ModuleRuntimeSupervisor,
    }

    impl Fixture {
        async fn new(drain_timeout: Duration) -> Self {
            let db = Database::connect("sqlite::memory:").await.unwrap();
            Migrator::up(&db, None).await.unwrap();
            let store = tempdir().unwrap();
            let publisher = Arc::new(ModulePublisher::new(store.path(), db.clone()));
            let loader = Arc::new(ScriptedLoader::default());
            let supervisor = ModuleRuntimeSupervisor::with_drain_timeout(
                publisher.clone(),
                loader.clone(),
                drain_timeout,
            );
            Self {
                store,
                db,
                publisher,
                loader,
                supervisor,
            }
        }

        async fn publish(&self, version: &str, permissions: &[&str]) {
            let source = package(PackageSpec {
                version,
                permissions,
                ..PackageSpec::default()
            });
            let staged = self.publisher.stage(source.path()).unwrap();
            self.publisher.commit(staged).await.unwrap();
        }

        async fn start(&self) -> ModuleRuntimeHandle {
            self.supervisor
                .start(&HostSession::local_default(), MODULE)
                .await
                .unwrap()
        }

        async fn update(
            &self,
            source: &TempDir,
        ) -> Result<ModuleUpdateReport, ModuleSupervisorError> {
            self.supervisor
                .update(&HostSession::local_default(), MODULE, source.path())
                .await
        }

        async fn module_row(&self) -> modules::Model {
            modules::Entity::find_by_id(MODULE)
                .one(&self.db)
                .await
                .unwrap()
                .unwrap()
        }

        async fn events(&self) -> Vec<(String, String)> {
            module_update_events::Entity::find()
                .all(&self.db)
                .await
                .unwrap()
                .into_iter()
                .map(|event| (event.stage, event.outcome))
                .collect()
        }

        fn pointer(&self) -> Option<String> {
            fs::read_to_string(
                self.store
                    .path()
                    .join("current")
                    .join(MODULE)
                    .join("version"),
            )
            .ok()
            .map(|value| value.trim().to_owned())
        }

        fn version_entry(&self, version: &str) -> std::path::PathBuf {
            self.store
                .path()
                .join("versions")
                .join(MODULE)
                .join(version)
                .join("entry.js")
        }

        fn staging_is_empty(&self) -> bool {
            fs::read_dir(self.store.path().join("staging"))
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true)
        }
    }

    const MODULE: &str = "module-1";

    struct PackageSpec<'a> {
        module_id: &'a str,
        version: &'a str,
        entry: &'a str,
        /// `None` writes a version-specific body.
        body: Option<&'a str>,
        permissions: &'a [&'a str],
        digest: Option<&'a str>,
    }

    impl Default for PackageSpec<'_> {
        fn default() -> Self {
            Self {
                module_id: MODULE,
                version: "1.0.0",
                entry: "entry.js",
                body: None,
                permissions: &[],
                digest: None,
            }
        }
    }

    fn package(spec: PackageSpec<'_>) -> TempDir {
        let dir = tempdir().unwrap();
        let default_body = format!("export default '{}';", spec.version);
        fs::write(
            dir.path().join("entry.js"),
            spec.body.unwrap_or(&default_body),
        )
        .unwrap();
        let digest = spec
            .digest
            .map(str::to_owned)
            .unwrap_or_else(|| package_digest(dir.path()).unwrap());
        fs::write(
            dir.path().join("manifest.json"),
            serde_json::json!({
                "id": spec.module_id,
                "name": "Example",
                "version": spec.version,
                "entry": spec.entry,
                "artifact": {"version": spec.version, "sha256": digest},
                "permissions": spec.permissions,
            })
            .to_string(),
        )
        .unwrap();
        dir
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        for _ in 0..2000 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("condition not reached");
    }

    #[tokio::test]
    async fn start_marks_ready_and_stop_waits_for_admitted_requests() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        assert_eq!(handle.state(), ModuleRuntimeState::Ready);
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 1);
        assert!(matches!(
            fixture
                .supervisor
                .start(&HostSession::local_default(), MODULE)
                .await,
            Err(ModuleSupervisorError::AlreadyRunning)
        ));

        let request = handle.begin_request().unwrap();
        let (stopped, ()) = tokio::join!(fixture.supervisor.stop(MODULE), async {
            wait_until(|| handle.state() == ModuleRuntimeState::Draining).await;
            assert_eq!(
                handle.begin_request().err(),
                Some(ModuleRuntimeError::NotAccepting(
                    ModuleRuntimeState::Draining
                ))
            );
            assert!(fixture.loader.stops().is_empty());
            request.finish(ModuleRequestOutcome::Succeeded);
        });
        stopped.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        assert_eq!(fixture.loader.stops(), vec![handle.runtime_id().to_owned()]);
        let stats = handle.request_stats();
        assert_eq!(
            (
                stats.admitted,
                stats.succeeded,
                stats.rejected,
                stats.in_flight
            ),
            (1, 1, 1, 0)
        );
    }

    #[tokio::test]
    async fn drain_timeout_fails_runtime_and_releases_only_after_requests_settle() {
        let fixture = Fixture::new(Duration::from_millis(30)).await;
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        let request = handle.begin_request().unwrap();

        assert!(matches!(
            fixture.supervisor.stop(MODULE).await,
            Err(ModuleSupervisorError::DrainTimeout)
        ));
        assert_eq!(handle.state(), ModuleRuntimeState::Failed);
        assert!(handle.begin_request().is_err());
        assert!(fixture.loader.stops().is_empty());

        // The admitted request still reaches its own terminal outcome.
        request.finish(ModuleRequestOutcome::Failed);
        wait_until(|| fixture.loader.stops().len() == 1).await;
        assert_eq!(handle.state(), ModuleRuntimeState::Failed);
        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        assert_eq!(handle.request_stats().failed, 1);
    }

    #[tokio::test]
    async fn start_refuses_tampered_and_uncommitted_releases() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        fs::write(fixture.version_entry("1.0.0"), "export default 'tampered';").unwrap();
        assert!(matches!(
            fixture
                .supervisor
                .start(&HostSession::local_default(), MODULE)
                .await,
            Err(ModuleSupervisorError::Package(
                ModulePackageError::Integrity
            ))
        ));
        assert!(fixture.supervisor.runtime(MODULE).is_none());
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 0);

        // A package directory, pointer and module row that did not come from
        // `commit` is not a verified release, even with a matching digest.
        let source = package(PackageSpec {
            module_id: "module-2",
            ..PackageSpec::default()
        });
        let target = fixture.store.path().join("versions/module-2/1.0.0");
        fs::create_dir_all(&target).unwrap();
        for name in ["entry.js", "manifest.json"] {
            fs::copy(source.path().join(name), target.join(name)).unwrap();
        }
        let digest = package_digest(&target).unwrap();
        fixture
            .publisher
            .write_pointer("module-2", "1.0.0")
            .unwrap();
        modules::ActiveModel {
            id: Set("module-2".into()),
            module_name: Set("module-2".into()),
            display_name: Set("Example".into()),
            current_version: Set("1.0.0".into()),
            description: Set(None),
            manifest: Set(serde_json::json!({"permissions": []})),
            integrity: Set(digest),
            entry_url: Set(Some("entry.js".into())),
            icon_url: Set(None),
            is_official: Set(false),
            is_enabled: Set(true),
            installed_at: Set(0),
            updated_at: Set(0),
        }
        .insert(&fixture.db)
        .await
        .unwrap();
        assert!(matches!(
            fixture
                .supervisor
                .start(&HostSession::local_default(), "module-2")
                .await,
            Err(ModuleSupervisorError::Package(ModulePackageError::Manifest(reason)))
                if reason.contains("not committed")
        ));
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_refuses_entry_replaced_by_symlink_outside_package() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("evil.js"), "export default 'evil';").unwrap();
        let entry = fixture.version_entry("1.0.0");
        fs::remove_file(&entry).unwrap();
        std::os::unix::fs::symlink(outside.path().join("evil.js"), &entry).unwrap();
        assert!(fixture
            .supervisor
            .start(&HostSession::local_default(), MODULE)
            .await
            .is_err());
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failed_start_is_observable_refuses_requests_and_can_restart() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        fixture.loader.fail_start(Some("1.0.0"));
        assert!(matches!(
            fixture
                .supervisor
                .start(&HostSession::local_default(), MODULE)
                .await,
            Err(ModuleSupervisorError::Load(_))
        ));
        let failed = fixture.supervisor.runtime(MODULE).unwrap();
        assert_eq!(failed.state(), ModuleRuntimeState::Failed);
        assert_eq!(
            failed.begin_request().err(),
            Some(ModuleRuntimeError::NotAccepting(ModuleRuntimeState::Failed))
        );
        assert_eq!(fixture.loader.stops(), vec![failed.runtime_id().to_owned()]);

        fixture.loader.fail_start(None);
        let restarted = fixture.start().await;
        assert_eq!(restarted.state(), ModuleRuntimeState::Ready);
        assert_ne!(restarted.runtime_id(), failed.runtime_id());
    }

    #[tokio::test]
    async fn reported_failure_refuses_new_requests_and_releases_after_settle() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        let request = handle.begin_request().unwrap();

        assert!(matches!(
            fixture
                .supervisor
                .report_failure(MODULE, "another-instance"),
            Err(ModuleSupervisorError::NotRunning)
        ));
        fixture
            .supervisor
            .report_failure(MODULE, handle.runtime_id())
            .unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Failed);
        assert!(handle.begin_request().is_err());
        tokio::task::yield_now().await;
        assert!(fixture.loader.stops().is_empty());

        drop(request);
        wait_until(|| fixture.loader.stops().len() == 1).await;
        assert_eq!(handle.request_stats().interrupted, 1);
        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
    }

    #[tokio::test]
    async fn update_switches_only_after_smoke_and_retires_previous_runtime() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        let source = package(PackageSpec {
            version: "2.0.0",
            ..PackageSpec::default()
        });

        let report = fixture.update(&source).await.unwrap();
        assert_eq!(
            report.outcome,
            ModuleUpdateOutcome::Switched {
                previous_stopped: true
            }
        );
        assert_eq!(
            (report.from_version.as_str(), report.to_version.as_str()),
            ("1.0.0", "2.0.0")
        );
        let current = fixture.supervisor.runtime(MODULE).unwrap();
        assert!(!current.is_same_runtime(&previous));
        assert_eq!(current.state(), ModuleRuntimeState::Ready);
        assert_eq!(current.module_identity().unwrap().1, "2.0.0");
        assert_eq!(previous.state(), ModuleRuntimeState::Stopped);
        assert_eq!(
            fixture.loader.stops(),
            vec![previous.runtime_id().to_owned()]
        );
        assert_eq!(fixture.module_row().await.current_version, "2.0.0");
        assert_eq!(fixture.pointer().as_deref(), Some("2.0.0"));
    }

    #[tokio::test]
    async fn update_rejects_remote_escaping_mismatched_and_foreign_packages() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        let wrong_digest = "0".repeat(64);
        let rejected = [
            package(PackageSpec {
                version: "2.0.0",
                entry: "https://example.invalid/entry.js",
                ..PackageSpec::default()
            }),
            package(PackageSpec {
                version: "2.0.0",
                entry: "data:text/javascript,export default 1",
                ..PackageSpec::default()
            }),
            package(PackageSpec {
                version: "2.0.0",
                entry: "../entry.js",
                ..PackageSpec::default()
            }),
            package(PackageSpec {
                version: "2.0.0",
                digest: Some(&wrong_digest),
                ..PackageSpec::default()
            }),
        ];
        for source in &rejected {
            assert!(matches!(
                fixture.update(source).await,
                Err(ModuleSupervisorError::Package(_))
            ));
        }
        let foreign = package(PackageSpec {
            module_id: "module-2",
            version: "2.0.0",
            ..PackageSpec::default()
        });
        assert!(matches!(
            fixture.update(&foreign).await,
            Err(ModuleSupervisorError::ModuleMismatch)
        ));

        assert!(fixture
            .supervisor
            .runtime(MODULE)
            .unwrap()
            .is_same_runtime(&previous));
        assert_eq!(previous.state(), ModuleRuntimeState::Ready);
        assert_eq!(fixture.module_row().await.current_version, "1.0.0");
        assert_eq!(fixture.pointer().as_deref(), Some("1.0.0"));
        assert!(fixture.staging_is_empty());
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn smoke_failure_keeps_previous_release_and_discards_staging() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        let empty_entry = package(PackageSpec {
            version: "2.0.0",
            body: Some(""),
            ..PackageSpec::default()
        });

        assert!(matches!(
            fixture.update(&empty_entry).await,
            Err(ModuleSupervisorError::Load(ModuleLoadError::Rejected(_)))
        ));
        assert_eq!(previous.state(), ModuleRuntimeState::Ready);
        assert_eq!(fixture.module_row().await.current_version, "1.0.0");
        assert_eq!(fixture.pointer().as_deref(), Some("1.0.0"));
        assert!(!fixture.version_entry("2.0.0").exists());
        assert!(fixture.staging_is_empty());
        assert!(fixture
            .events()
            .await
            .contains(&("load_smoke".into(), "failure".into())));
    }

    #[tokio::test]
    async fn start_failure_after_commit_rolls_back_to_previous_runtime() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &["providers.call"]).await;
        let previous = fixture.start().await;
        let previous_row = fixture.module_row().await;
        fixture.loader.fail_start(Some("2.0.0"));
        let source = package(PackageSpec {
            version: "2.0.0",
            permissions: &["providers.call", "mcp.call"],
            ..PackageSpec::default()
        });

        let error = fixture.update(&source).await.unwrap_err();
        assert!(matches!(
            &error,
            ModuleSupervisorError::RolledBack(cause)
                if matches!(**cause, ModuleSupervisorError::Load(_))
        ));
        assert!(fixture
            .supervisor
            .runtime(MODULE)
            .unwrap()
            .is_same_runtime(&previous));
        assert_eq!(previous.state(), ModuleRuntimeState::Ready);
        let row = fixture.module_row().await;
        assert_eq!(row.current_version, "1.0.0");
        assert_eq!(row.integrity, previous_row.integrity);
        assert_eq!(row.manifest, previous_row.manifest);
        assert!(row.is_enabled);
        assert_eq!(fixture.pointer().as_deref(), Some("1.0.0"));
        let events = fixture.events().await;
        assert!(events.contains(&("activation".into(), "failure".into())));
        assert!(events.contains(&("rolled_back".into(), "success".into())));
        // The failed candidate released its loader resources and was never
        // registered.
        assert_eq!(fixture.loader.stops().len(), 1);
        assert_ne!(fixture.loader.stops()[0], previous.runtime_id());

        // Republishing the same failed version does not re-activate it.
        fixture.loader.fail_start(None);
        assert!(matches!(
            fixture.update(&source).await,
            Err(ModuleSupervisorError::VersionNotCurrent)
        ));
        assert_eq!(previous.state(), ModuleRuntimeState::Ready);
        assert_eq!(fixture.module_row().await.current_version, "1.0.0");
    }

    #[tokio::test]
    async fn rollback_failure_disables_module_and_fails_previous_runtime() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        // The previous release is damaged on disk, so it cannot be restored.
        fs::write(fixture.version_entry("1.0.0"), "export default 'damaged';").unwrap();
        fixture.loader.fail_start(Some("2.0.0"));
        let source = package(PackageSpec {
            version: "2.0.0",
            ..PackageSpec::default()
        });

        assert!(matches!(
            fixture.update(&source).await,
            Err(ModuleSupervisorError::RollbackFailed {
                module_disabled: true
            })
        ));
        assert_eq!(previous.state(), ModuleRuntimeState::Failed);
        assert!(previous.begin_request().is_err());
        assert!(!fixture.module_row().await.is_enabled);
        assert_eq!(fixture.pointer(), None);
        assert!(fixture
            .events()
            .await
            .contains(&("disabled".into(), "failure".into())));
        assert!(fixture
            .supervisor
            .start(&HostSession::local_default(), MODULE)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn identical_update_is_unchanged_and_requires_matching_ready_runtime() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let same = package(PackageSpec::default());
        assert!(matches!(
            fixture.update(&same).await,
            Err(ModuleSupervisorError::NotRunning)
        ));

        let previous = fixture.start().await;
        let report = fixture.update(&same).await.unwrap();
        assert_eq!(report.outcome, ModuleUpdateOutcome::Unchanged);
        assert!(fixture
            .supervisor
            .runtime(MODULE)
            .unwrap()
            .is_same_runtime(&previous));
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 1);
        assert!(fixture.staging_is_empty());
    }
}
