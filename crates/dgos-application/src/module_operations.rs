//! Host-level module lifecycle orchestration (D034).
//!
//! `ModuleOperationManager` is the single owner of `ModulePublisher` and
//! `ModuleRuntimeSupervisor` for transports. A transport submits `start`,
//! `stop` or `update` and immediately receives a host-generated operation id;
//! the work runs in a detached task, so dropping the caller's future (a
//! cancelled command, a closed window) only stops waiting and never cancels
//! an operation the host already accepted.
//!
//! Rules enforced here:
//! - identity comes from the `HostSession` the host passes in; the only
//!   renderer-derived input is a `module_id` selector, validated with the
//!   publisher's id rule and never used as an authority;
//! - updates read only from the host-owned `LocalModuleSources` root;
//! - operations run one at a time in submission order, and a module can have
//!   at most one non-terminal operation (conflicts are refused, not queued);
//! - start and update are refused until startup recovery succeeded
//!   (fail-closed); stop is always accepted so loader resources can be
//!   released;
//! - errors reach callers only as `ModuleOperationErrorKind`.

use chrono::Utc;
use dgos_modules::{
    valid_module_id, LocalModuleSources, ModuleLoader, ModulePackageError, ModulePublisher,
    ModuleRuntimeState, ModuleRuntimeSupervisor, ModuleSourceError, ModuleSupervisorError,
    ModuleUpdateOutcome,
};
use dgos_types::{
    HostSession, ModuleOperationAccepted, ModuleOperationErrorKind as ErrorKind,
    ModuleOperationKind, ModuleOperationOutcome, ModuleOperationState, ModuleOperationView,
    ModuleRuntimeStatusState, ModuleRuntimeStatusView,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;
use uuid::Uuid;

/// Finished operations kept for status queries; older ones are evicted.
const MAX_FINISHED_OPERATIONS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Availability {
    Uninitialized,
    Recovering,
    Available,
    Unavailable,
}

struct Record {
    view: ModuleOperationView,
    done: watch::Receiver<bool>,
}

struct State {
    availability: Availability,
    records: HashMap<String, Record>,
    finished: VecDeque<String>,
    /// module id → its single non-terminal operation id.
    active: HashMap<String, String>,
    /// Completion signal of the most recently accepted operation.
    tail: Option<watch::Receiver<bool>>,
}

struct Inner {
    publisher: Arc<ModulePublisher>,
    supervisor: ModuleRuntimeSupervisor,
    sources: LocalModuleSources,
    state: Mutex<State>,
}

#[derive(Clone)]
pub struct ModuleOperationManager {
    inner: Arc<Inner>,
}

impl ModuleOperationManager {
    pub fn new(
        publisher: Arc<ModulePublisher>,
        loader: Arc<dyn ModuleLoader>,
        sources: LocalModuleSources,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                supervisor: ModuleRuntimeSupervisor::new(publisher.clone(), loader),
                publisher,
                sources,
                state: Mutex::new(State {
                    availability: Availability::Uninitialized,
                    records: HashMap::new(),
                    finished: VecDeque::new(),
                    active: HashMap::new(),
                    tail: None,
                }),
            }),
        }
    }

    /// Startup recovery: reconcile every registered module's pointer, row
    /// and package. Until this succeeds `start` and `update` are refused. A
    /// failure leaves the host unavailable (fail-closed) and may be retried;
    /// dropping the future mid-way does the same. No runtime is started.
    pub async fn recover(&self) -> Result<Vec<String>, ErrorKind> {
        {
            let mut state = self.inner.state();
            match state.availability {
                Availability::Uninitialized | Availability::Unavailable => {
                    state.availability = Availability::Recovering;
                }
                Availability::Recovering => return Err(ErrorKind::OperationInProgress),
                Availability::Available => return Err(ErrorKind::Internal),
            }
        }
        let mut guard = RecoveryGuard {
            inner: &self.inner,
            settled: false,
        };
        let result = self.inner.publisher.recover_registered().await;
        {
            let mut state = self.inner.state();
            state.availability = if result.is_ok() {
                Availability::Available
            } else {
                Availability::Unavailable
            };
        }
        guard.settled = true;
        result.map_err(|error| {
            tracing::error!(%error, "module startup recovery failed; module host is unavailable");
            ErrorKind::HostUnavailable
        })
    }

    pub fn is_available(&self) -> bool {
        self.inner.state().availability == Availability::Available
    }

    pub fn start(
        &self,
        session: &HostSession,
        module_id: &str,
    ) -> Result<ModuleOperationAccepted, ErrorKind> {
        self.submit(ModuleOperationKind::Start, session, module_id)
    }

    pub fn stop(
        &self,
        session: &HostSession,
        module_id: &str,
    ) -> Result<ModuleOperationAccepted, ErrorKind> {
        self.submit(ModuleOperationKind::Stop, session, module_id)
    }

    /// Update `module_id` from its directory in the host-owned local source
    /// root. The caller supplies no path or URL.
    pub fn update(
        &self,
        session: &HostSession,
        module_id: &str,
    ) -> Result<ModuleOperationAccepted, ErrorKind> {
        self.submit(ModuleOperationKind::Update, session, module_id)
    }

    pub fn operation(&self, operation_id: &str) -> Result<ModuleOperationView, ErrorKind> {
        self.inner
            .state()
            .records
            .get(operation_id)
            .map(|record| record.view.clone())
            .ok_or(ErrorKind::OperationNotFound)
    }

    /// Wait for an operation to reach a terminal state. Cancel-safe: dropping
    /// this future never affects the operation.
    pub async fn wait(&self, operation_id: &str) -> Result<ModuleOperationView, ErrorKind> {
        let mut done = self
            .inner
            .state()
            .records
            .get(operation_id)
            .map(|record| record.done.clone())
            .ok_or(ErrorKind::OperationNotFound)?;
        // An error means the host is shutting down; report the last view.
        let _ = done.wait_for(|finished| *finished).await;
        self.operation(operation_id)
    }

    pub fn runtime_status(&self, module_id: &str) -> Result<ModuleRuntimeStatusView, ErrorKind> {
        if !valid_module_id(module_id) {
            return Err(ErrorKind::ModuleNotFound);
        }
        let supervisor = &self.inner.supervisor;
        let handle = supervisor.runtime(module_id);
        let identity = handle.as_ref().and_then(|handle| handle.module_identity());
        Ok(ModuleRuntimeStatusView {
            module_id: module_id.to_owned(),
            state: handle
                .as_ref()
                .map_or(ModuleRuntimeStatusState::NotLoaded, |handle| {
                    status_state(handle.state())
                }),
            runtime_id: handle.as_ref().map(|handle| handle.runtime_id().to_owned()),
            version: identity.map(|(_, version, _)| version.to_owned()),
            integrity: identity.map(|(_, _, integrity)| integrity.to_owned()),
            in_flight: handle
                .as_ref()
                .map_or(0, |handle| handle.request_stats().in_flight),
            pending_releases: supervisor.pending_release_count(module_id),
            active_operation_id: self.inner.state().active.get(module_id).cloned(),
        })
    }

    fn submit(
        &self,
        kind: ModuleOperationKind,
        session: &HostSession,
        module_id: &str,
    ) -> Result<ModuleOperationAccepted, ErrorKind> {
        if !valid_module_id(module_id) {
            return Err(ErrorKind::ModuleNotFound);
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| ErrorKind::Internal)?;
        let operation_id = Uuid::new_v4().to_string();
        let (done_tx, done_rx) = watch::channel(false);

        // Acceptance is synchronous and atomic: either the operation is
        // registered and spawned, or nothing changed.
        let previous = {
            let mut state = self.inner.state();
            if kind != ModuleOperationKind::Stop && state.availability != Availability::Available {
                return Err(ErrorKind::HostUnavailable);
            }
            if state.active.contains_key(module_id) {
                return Err(ErrorKind::OperationInProgress);
            }
            state
                .active
                .insert(module_id.to_owned(), operation_id.clone());
            state.records.insert(
                operation_id.clone(),
                Record {
                    view: ModuleOperationView {
                        operation_id: operation_id.clone(),
                        module_id: module_id.to_owned(),
                        kind,
                        state: ModuleOperationState::Queued,
                        outcome: None,
                        failure: None,
                        submitted_at: Utc::now().timestamp(),
                        finished_at: None,
                    },
                    done: done_rx.clone(),
                },
            );
            state.tail.replace(done_rx)
        };

        let inner = self.inner.clone();
        let session = session.clone();
        let module = module_id.to_owned();
        let id = operation_id.clone();
        runtime.spawn(async move {
            if let Some(mut previous) = previous {
                // A dropped predecessor sender only means it is gone.
                let _ = previous.wait_for(|finished| *finished).await;
            }
            inner.mark_running(&id);
            // A separate task isolates a panic in the loader or supervisor:
            // it fails this operation and the queue keeps moving.
            let worker = tokio::spawn({
                let inner = inner.clone();
                let (module, id) = (module.clone(), id.clone());
                async move { inner.execute(kind, &session, &module, &id).await }
            });
            let result = worker.await.unwrap_or(Err(ErrorKind::Internal));
            inner.finish(&id, &module, result);
            let _ = done_tx.send(true);
        });
        Ok(ModuleOperationAccepted { operation_id })
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn mark_running(&self, operation_id: &str) {
        if let Some(record) = self.state().records.get_mut(operation_id) {
            record.view.state = ModuleOperationState::Running;
        }
    }

    async fn execute(
        &self,
        kind: ModuleOperationKind,
        session: &HostSession,
        module_id: &str,
        operation_id: &str,
    ) -> Result<ModuleOperationOutcome, ErrorKind> {
        match kind {
            ModuleOperationKind::Start => self
                .supervisor
                .start(session, module_id)
                .await
                .map(|_| ModuleOperationOutcome::Started)
                .map_err(|error| map_supervisor_error(kind, &error)),
            ModuleOperationKind::Stop => self
                .supervisor
                .stop(module_id)
                .await
                .map(|()| ModuleOperationOutcome::Stopped)
                .map_err(|error| map_supervisor_error(kind, &error)),
            ModuleOperationKind::Update => {
                let source = self.sources.resolve(module_id).map_err(map_source_error)?;
                self.supervisor
                    .update_with_operation_id(session, module_id, &source, operation_id)
                    .await
                    .map(|report| match report.outcome {
                        ModuleUpdateOutcome::Unchanged => ModuleOperationOutcome::Unchanged,
                        ModuleUpdateOutcome::Switched {
                            previous_stopped: true,
                        } => ModuleOperationOutcome::Updated,
                        ModuleUpdateOutcome::Switched {
                            previous_stopped: false,
                        } => ModuleOperationOutcome::UpdatedPendingRelease,
                    })
                    .map_err(|error| map_supervisor_error(kind, &error))
            }
        }
    }

    fn finish(
        &self,
        operation_id: &str,
        module_id: &str,
        result: Result<ModuleOperationOutcome, ErrorKind>,
    ) {
        let mut guard = self.state();
        let state = &mut *guard;
        if let Some(record) = state.records.get_mut(operation_id) {
            match result {
                Ok(outcome) => {
                    record.view.state = ModuleOperationState::Succeeded;
                    record.view.outcome = Some(outcome);
                }
                Err(kind) => {
                    record.view.state = ModuleOperationState::Failed;
                    record.view.failure = Some(kind.into());
                }
            }
            record.view.finished_at = Some(Utc::now().timestamp());
        }
        if state.active.get(module_id).map(String::as_str) == Some(operation_id) {
            state.active.remove(module_id);
        }
        state.finished.push_back(operation_id.to_owned());
        while state.finished.len() > MAX_FINISHED_OPERATIONS {
            if let Some(oldest) = state.finished.pop_front() {
                state.records.remove(&oldest);
            }
        }
    }
}

/// Leaves `Recovering` if `recover` is dropped before it settles, so the
/// host stays unavailable (fail-closed) and recovery can be retried.
struct RecoveryGuard<'a> {
    inner: &'a Inner,
    settled: bool,
}

impl Drop for RecoveryGuard<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.inner.state().availability = Availability::Unavailable;
        }
    }
}

fn status_state(state: ModuleRuntimeState) -> ModuleRuntimeStatusState {
    match state {
        ModuleRuntimeState::Starting => ModuleRuntimeStatusState::Starting,
        ModuleRuntimeState::Ready => ModuleRuntimeStatusState::Ready,
        ModuleRuntimeState::Draining => ModuleRuntimeStatusState::Draining,
        ModuleRuntimeState::Stopped => ModuleRuntimeStatusState::Stopped,
        ModuleRuntimeState::Failed => ModuleRuntimeStatusState::Failed,
    }
}

/// Total over `ModuleSupervisorError`: raw messages, paths and SQL text are
/// dropped here and never reach a transport.
fn map_supervisor_error(kind: ModuleOperationKind, error: &ModuleSupervisorError) -> ErrorKind {
    let stopping = kind == ModuleOperationKind::Stop;
    match error {
        ModuleSupervisorError::Package(ModulePackageError::Database(_)) => ErrorKind::Internal,
        ModuleSupervisorError::Package(ModulePackageError::NotPublished) => {
            ErrorKind::ModuleNotFound
        }
        ModuleSupervisorError::Package(ModulePackageError::Disabled) => ErrorKind::Disabled,
        ModuleSupervisorError::Package(_) => ErrorKind::PackageInvalid,
        ModuleSupervisorError::Load(_) if stopping => ErrorKind::StopFailed,
        ModuleSupervisorError::Load(_) => ErrorKind::LoadFailed,
        ModuleSupervisorError::Runtime(_) if stopping => ErrorKind::StopFailed,
        ModuleSupervisorError::Runtime(_) => ErrorKind::Internal,
        ModuleSupervisorError::AlreadyRunning => ErrorKind::AlreadyRunning,
        ModuleSupervisorError::NotRunning => ErrorKind::NotRunning,
        ModuleSupervisorError::StaleRuntime | ModuleSupervisorError::VersionNotCurrent => {
            ErrorKind::VersionConflict
        }
        ModuleSupervisorError::ModuleMismatch => ErrorKind::PackageInvalid,
        ModuleSupervisorError::DrainTimeout => ErrorKind::StopFailed,
        ModuleSupervisorError::RolledBack(_) => ErrorKind::UpdateFailed,
        ModuleSupervisorError::RollbackFailed {
            module_disabled: true,
        } => ErrorKind::Disabled,
        ModuleSupervisorError::RollbackFailed {
            module_disabled: false,
        } => ErrorKind::Internal,
    }
}

fn map_source_error(error: ModuleSourceError) -> ErrorKind {
    match error {
        ModuleSourceError::InvalidModuleId => ErrorKind::ModuleNotFound,
        ModuleSourceError::NotFound | ModuleSourceError::Rejected => ErrorKind::PackageInvalid,
        ModuleSourceError::Io(_) => ErrorKind::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModuleReleaseService;
    use async_trait::async_trait;
    use dgos_db::sea_orm_migration::MigratorTrait;
    use dgos_db::{
        migrations::Migrator,
        sea_orm::{ConnectionTrait, Database, DatabaseConnection},
    };
    use dgos_modules::{
        LocalArtifactLoader, ModuleLoadError, ModuleRuntimeError, VerifiedModuleArtifact,
    };
    use std::fs;
    use std::future::Future;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::task::Poll;
    use std::time::Duration;
    use tempfile::{tempdir, TempDir};

    const MODULE: &str = "module-1";
    const OTHER: &str = "module-2";

    /// Real local checks plus scripted gating, panics and release failures.
    #[derive(Default)]
    struct TestLoader {
        gate: Mutex<Option<watch::Receiver<bool>>>,
        entered_starts: AtomicUsize,
        panic_next_start: AtomicBool,
        failing_stops: AtomicUsize,
        log: Mutex<Vec<String>>,
    }

    impl TestLoader {
        /// Starts block until the returned sender is set to `true`.
        fn close_gate(&self) -> watch::Sender<bool> {
            let (sender, receiver) = watch::channel(false);
            *self.gate.lock().unwrap() = Some(receiver);
            sender
        }

        fn fail_next_stops(&self, count: usize) {
            self.failing_stops.store(count, Ordering::SeqCst);
        }

        fn entered(&self) -> usize {
            self.entered_starts.load(Ordering::SeqCst)
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ModuleLoader for TestLoader {
        async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
            LocalArtifactLoader.smoke(artifact).await
        }

        async fn start(
            &self,
            runtime_id: &str,
            artifact: &VerifiedModuleArtifact,
        ) -> Result<(), ModuleLoadError> {
            self.entered_starts.fetch_add(1, Ordering::SeqCst);
            self.log
                .lock()
                .unwrap()
                .push(format!("start:{}", artifact.module_id()));
            if self.panic_next_start.swap(false, Ordering::SeqCst) {
                panic!("scripted loader panic");
            }
            let gate = self.gate.lock().unwrap().clone();
            if let Some(mut gate) = gate {
                let _ = gate.wait_for(|open| *open).await;
            }
            LocalArtifactLoader.start(runtime_id, artifact).await
        }

        async fn stop(
            &self,
            _runtime_id: &str,
            _artifact: &VerifiedModuleArtifact,
        ) -> Result<(), ModuleLoadError> {
            let scripted = self
                .failing_stops
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok();
            if scripted {
                return Err(ModuleLoadError::Rejected("scripted stop failure".into()));
            }
            Ok(())
        }
    }

    fn write_package(dir: &Path, module_id: &str, version: &str) {
        use sha2::{Digest, Sha256};
        fs::create_dir_all(dir).unwrap();
        let body = format!("export default '{module_id}@{version}';");
        fs::write(dir.join("entry.js"), &body).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(b"entry.js");
        hasher.update([0]);
        hasher.update(body.as_bytes());
        fs::write(
            dir.join("manifest.json"),
            serde_json::json!({
                "id": module_id, "name": "Example", "version": version, "entry": "entry.js",
                "artifact": {"version": version, "sha256": format!("{:x}", hasher.finalize())},
                "permissions": ["providers.call"]
            })
            .to_string(),
        )
        .unwrap();
    }

    struct Fixture {
        store: TempDir,
        sources: TempDir,
        db: DatabaseConnection,
        loader: Arc<TestLoader>,
        manager: ModuleOperationManager,
        session: HostSession,
    }

    impl Fixture {
        /// Publishes `1.0.0` of every module; startup recovery has not run.
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
            let loader = Arc::new(TestLoader::default());
            let manager = ModuleOperationManager::new(
                publisher,
                loader.clone(),
                LocalModuleSources::new(sources.path()),
            );
            Self {
                store,
                sources,
                db,
                loader,
                manager,
                session: HostSession::local_default(),
            }
        }

        async fn new(modules: &[&str]) -> Self {
            let fixture = Self::unrecovered(modules).await;
            fixture.manager.recover().await.unwrap();
            fixture
        }

        async fn finish(&self, operation_id: &str) -> ModuleOperationView {
            tokio::time::timeout(Duration::from_secs(5), self.manager.wait(operation_id))
                .await
                .expect("operation did not finish")
                .unwrap()
        }

        async fn run_start(&self, module: &str) -> ModuleOperationView {
            let accepted = self.manager.start(&self.session, module).unwrap();
            self.finish(&accepted.operation_id).await
        }

        async fn run_stop(&self, module: &str) -> ModuleOperationView {
            let accepted = self.manager.stop(&self.session, module).unwrap();
            self.finish(&accepted.operation_id).await
        }

        fn stage_update(&self, module: &str, version: &str) {
            write_package(&self.sources.path().join(module), module, version);
        }
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
    async fn start_runs_in_the_background_and_reports_operation_and_runtime_status() {
        let fixture = Fixture::new(&[MODULE]).await;
        let gate = fixture.loader.close_gate();

        let accepted = fixture.manager.start(&fixture.session, MODULE).unwrap();
        wait_until(|| fixture.loader.entered() == 1).await;
        let view = fixture.manager.operation(&accepted.operation_id).unwrap();
        assert_eq!(
            (view.kind, view.state),
            (ModuleOperationKind::Start, ModuleOperationState::Running)
        );
        let status = fixture.manager.runtime_status(MODULE).unwrap();
        assert_eq!(status.state, ModuleRuntimeStatusState::Starting);
        assert_eq!(
            status.active_operation_id.as_deref(),
            Some(accepted.operation_id.as_str())
        );

        gate.send(true).unwrap();
        let view = fixture.finish(&accepted.operation_id).await;
        assert_eq!(view.state, ModuleOperationState::Succeeded);
        assert_eq!(view.outcome, Some(ModuleOperationOutcome::Started));
        assert!(view.failure.is_none() && view.finished_at.is_some());
        let status = fixture.manager.runtime_status(MODULE).unwrap();
        assert_eq!(status.state, ModuleRuntimeStatusState::Ready);
        assert_eq!(status.version.as_deref(), Some("1.0.0"));
        assert_eq!(status.in_flight, 0);
        assert!(status.active_operation_id.is_none() && status.runtime_id.is_some());
    }

    #[tokio::test]
    async fn cancelling_the_command_future_does_not_cancel_the_operation() {
        let fixture = Fixture::new(&[MODULE]).await;
        let gate = fixture.loader.close_gate();

        // A timed-out waiter.
        let accepted = fixture.manager.start(&fixture.session, MODULE).unwrap();
        wait_until(|| fixture.loader.entered() == 1).await;
        let waited = tokio::time::timeout(
            Duration::from_millis(20),
            fixture.manager.wait(&accepted.operation_id),
        )
        .await;
        assert!(waited.is_err());
        assert_eq!(
            fixture
                .manager
                .operation(&accepted.operation_id)
                .unwrap()
                .state,
            ModuleOperationState::Running
        );
        gate.send(true).unwrap();
        assert_eq!(
            fixture.finish(&accepted.operation_id).await.state,
            ModuleOperationState::Succeeded
        );

        // A whole command task aborted after the operation was accepted.
        assert_eq!(
            fixture.run_stop(MODULE).await.state,
            ModuleOperationState::Succeeded
        );
        let gate = fixture.loader.close_gate();
        let command = tokio::spawn({
            let manager = fixture.manager.clone();
            let session = fixture.session.clone();
            async move {
                let accepted = manager.start(&session, MODULE).unwrap();
                manager.wait(&accepted.operation_id).await
            }
        });
        wait_until(|| fixture.loader.entered() == 2).await;
        let start_id = fixture
            .manager
            .runtime_status(MODULE)
            .unwrap()
            .active_operation_id
            .unwrap();
        command.abort();
        assert!(command.await.unwrap_err().is_cancelled());
        assert_eq!(
            fixture.manager.operation(&start_id).unwrap().state,
            ModuleOperationState::Running
        );

        gate.send(true).unwrap();
        let view = fixture.finish(&start_id).await;
        assert_eq!(view.state, ModuleOperationState::Succeeded);
        assert_eq!(view.outcome, Some(ModuleOperationOutcome::Started));
        assert_eq!(
            fixture.manager.runtime_status(MODULE).unwrap().state,
            ModuleRuntimeStatusState::Ready
        );
    }

    #[tokio::test]
    async fn same_module_conflicts_are_refused_and_other_modules_run_in_submission_order() {
        let fixture = Fixture::new(&[MODULE, OTHER]).await;
        let gate = fixture.loader.close_gate();

        let first = fixture.manager.start(&fixture.session, MODULE).unwrap();
        wait_until(|| fixture.loader.entered() == 1).await;
        for result in [
            fixture.manager.start(&fixture.session, MODULE),
            fixture.manager.stop(&fixture.session, MODULE),
            fixture.manager.update(&fixture.session, MODULE),
        ] {
            assert_eq!(result.unwrap_err(), ErrorKind::OperationInProgress);
        }

        let second = fixture.manager.start(&fixture.session, OTHER).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            fixture
                .manager
                .operation(&second.operation_id)
                .unwrap()
                .state,
            ModuleOperationState::Queued
        );
        assert_eq!(fixture.loader.entered(), 1);

        gate.send(true).unwrap();
        assert_eq!(
            fixture.finish(&first.operation_id).await.state,
            ModuleOperationState::Succeeded
        );
        assert_eq!(
            fixture.finish(&second.operation_id).await.state,
            ModuleOperationState::Succeeded
        );
        assert_eq!(
            fixture.loader.log(),
            vec!["start:module-1".to_owned(), "start:module-2".to_owned()]
        );
    }

    #[tokio::test]
    async fn failed_release_is_reported_as_retryable_and_a_later_stop_retries_it() {
        let fixture = Fixture::new(&[MODULE]).await;
        assert_eq!(
            fixture.run_start(MODULE).await.state,
            ModuleOperationState::Succeeded
        );
        fixture.loader.fail_next_stops(1);

        let failed = fixture.run_stop(MODULE).await;
        assert_eq!(failed.state, ModuleOperationState::Failed);
        let failure = failed.failure.unwrap();
        assert_eq!(
            (failure.kind, failure.code, failure.retryable),
            (ErrorKind::StopFailed, 1004007, true)
        );
        assert_eq!(
            fixture.manager.runtime_status(MODULE).unwrap().state,
            ModuleRuntimeStatusState::Failed
        );

        let retried = fixture.run_stop(MODULE).await;
        assert_eq!(retried.outcome, Some(ModuleOperationOutcome::Stopped));
        assert_eq!(
            fixture.manager.runtime_status(MODULE).unwrap().state,
            ModuleRuntimeStatusState::Stopped
        );
    }

    #[tokio::test]
    async fn update_reads_the_local_source_and_surfaces_a_pending_release() {
        let fixture = Fixture::new(&[MODULE]).await;
        fixture.run_start(MODULE).await;
        fixture.stage_update(MODULE, "2.0.0");
        fixture.loader.fail_next_stops(1);

        let accepted = fixture.manager.update(&fixture.session, MODULE).unwrap();
        let view = fixture.finish(&accepted.operation_id).await;
        assert_eq!(view.kind, ModuleOperationKind::Update);
        assert_eq!(
            view.outcome,
            Some(ModuleOperationOutcome::UpdatedPendingRelease)
        );
        let status = fixture.manager.runtime_status(MODULE).unwrap();
        assert_eq!(status.state, ModuleRuntimeStatusState::Ready);
        assert_eq!(status.version.as_deref(), Some("2.0.0"));
        assert_eq!(status.pending_releases, 1);

        let stopped = fixture.run_stop(MODULE).await;
        assert_eq!(stopped.state, ModuleOperationState::Succeeded);
        let status = fixture.manager.runtime_status(MODULE).unwrap();
        assert_eq!(status.state, ModuleRuntimeStatusState::Stopped);
        assert_eq!(status.pending_releases, 0);
    }

    #[tokio::test]
    async fn update_without_a_local_source_fails_without_leaking_paths() {
        let fixture = Fixture::new(&[MODULE]).await;
        fixture.run_start(MODULE).await;

        let accepted = fixture.manager.update(&fixture.session, MODULE).unwrap();
        let view = fixture.finish(&accepted.operation_id).await;
        assert_eq!(view.state, ModuleOperationState::Failed);
        assert_eq!(view.failure.unwrap().kind, ErrorKind::PackageInvalid);
        let json = format!("{view:?}");
        assert!(!json.contains(fixture.sources.path().to_str().unwrap()));
        let status = fixture.manager.runtime_status(MODULE).unwrap();
        assert_eq!(status.state, ModuleRuntimeStatusState::Ready);
        assert_eq!(status.version.as_deref(), Some("1.0.0"));
    }

    #[tokio::test]
    async fn module_ids_that_are_paths_or_urls_never_create_an_operation() {
        let fixture = Fixture::new(&[MODULE]).await;
        for id in [
            "",
            ".",
            "..",
            "../module-1",
            "a/b",
            "https://example.invalid/m",
            "file:///tmp/m",
        ] {
            assert_eq!(
                fixture.manager.start(&fixture.session, id).unwrap_err(),
                ErrorKind::ModuleNotFound,
                "{id:?}"
            );
            assert_eq!(
                fixture.manager.update(&fixture.session, id).unwrap_err(),
                ErrorKind::ModuleNotFound
            );
            assert_eq!(
                fixture.manager.runtime_status(id).unwrap_err(),
                ErrorKind::ModuleNotFound
            );
        }
        assert_eq!(fixture.loader.entered(), 0);
    }

    #[tokio::test]
    async fn start_and_update_are_refused_until_recovery_succeeds() {
        let fixture = Fixture::unrecovered(&[MODULE]).await;
        assert!(!fixture.manager.is_available());
        assert_eq!(
            fixture.manager.start(&fixture.session, MODULE).unwrap_err(),
            ErrorKind::HostUnavailable
        );
        assert_eq!(
            fixture
                .manager
                .update(&fixture.session, MODULE)
                .unwrap_err(),
            ErrorKind::HostUnavailable
        );
        // Stop stays available so loader resources can always be released.
        let stopped = fixture.run_stop(MODULE).await;
        assert_eq!(stopped.failure.unwrap().kind, ErrorKind::NotRunning);

        assert_eq!(
            fixture.manager.recover().await.unwrap(),
            vec![MODULE.to_owned()]
        );
        assert!(fixture.manager.is_available());
        assert_eq!(
            fixture.run_start(MODULE).await.state,
            ModuleOperationState::Succeeded
        );
        // Recovery is a startup step, not a way to reset a live host.
        assert_eq!(
            fixture.manager.recover().await.unwrap_err(),
            ErrorKind::Internal
        );
    }

    #[tokio::test]
    async fn failed_recovery_keeps_the_host_unavailable() {
        let fixture = Fixture::unrecovered(&[MODULE]).await;
        fixture
            .db
            .execute_unprepared("DROP TABLE module_release_locks")
            .await
            .unwrap();

        assert_eq!(
            fixture.manager.recover().await.unwrap_err(),
            ErrorKind::HostUnavailable
        );
        assert!(!fixture.manager.is_available());
        assert_eq!(
            fixture.manager.start(&fixture.session, MODULE).unwrap_err(),
            ErrorKind::HostUnavailable
        );
        assert_eq!(fixture.loader.entered(), 0);
    }

    #[tokio::test]
    async fn dropping_recovery_midway_leaves_it_retryable_and_the_host_unavailable() {
        let fixture = Fixture::unrecovered(&[MODULE]).await;
        let mut recovery = Box::pin(fixture.manager.recover());
        let polled = std::future::poll_fn(|cx| Poll::Ready(recovery.as_mut().poll(cx))).await;
        assert!(polled.is_pending());
        assert_eq!(
            fixture.manager.start(&fixture.session, MODULE).unwrap_err(),
            ErrorKind::HostUnavailable
        );
        drop(recovery);

        assert_eq!(
            fixture.manager.recover().await.unwrap(),
            vec![MODULE.to_owned()]
        );
        assert!(fixture.manager.is_available());
    }

    #[tokio::test]
    async fn recovery_disables_a_tampered_package_and_start_fails_closed() {
        let fixture = Fixture::unrecovered(&[MODULE]).await;
        fs::write(
            fixture
                .store
                .path()
                .join("versions/module-1/1.0.0/entry.js"),
            "export default 'tampered';",
        )
        .unwrap();

        assert!(fixture.manager.recover().await.unwrap().is_empty());
        let view = fixture.run_start(MODULE).await;
        assert_eq!(view.state, ModuleOperationState::Failed);
        // Recovery disabled the tampered release, so the module now reports
        // `disabled` (D035) rather than a package error.
        assert_eq!(view.failure.unwrap().kind, ErrorKind::Disabled);
        assert_eq!(fixture.loader.entered(), 0);
        assert_eq!(
            fixture.manager.runtime_status(MODULE).unwrap().state,
            ModuleRuntimeStatusState::NotLoaded
        );
    }

    #[tokio::test]
    async fn start_classifies_unpublished_disabled_and_invalid_packages_separately() {
        let fixture = Fixture::new(&[MODULE, OTHER]).await;

        // Never published: no registered row.
        let unpublished = fixture.run_start("never-published").await;
        assert_eq!(unpublished.failure.unwrap().kind, ErrorKind::ModuleNotFound);

        // Registered but disabled by the host.
        fixture
            .db
            .execute_unprepared("UPDATE modules SET is_enabled = 0 WHERE id = 'module-2'")
            .await
            .unwrap();
        let disabled = fixture.run_start(OTHER).await;
        assert_eq!(disabled.failure.unwrap().kind, ErrorKind::Disabled);

        // Published and enabled, but the package on disk is invalid.
        fs::write(
            fixture
                .store
                .path()
                .join("versions/module-1/1.0.0/entry.js"),
            "export default 'tampered';",
        )
        .unwrap();
        let invalid = fixture.run_start(MODULE).await;
        assert_eq!(invalid.failure.unwrap().kind, ErrorKind::PackageInvalid);
        assert_eq!(fixture.loader.entered(), 0);
    }

    #[tokio::test]
    async fn a_panicking_loader_fails_its_operation_and_the_queue_keeps_moving() {
        let fixture = Fixture::new(&[MODULE, OTHER]).await;
        fixture
            .loader
            .panic_next_start
            .store(true, Ordering::SeqCst);

        let view = fixture.run_start(MODULE).await;
        assert_eq!(view.state, ModuleOperationState::Failed);
        assert_eq!(view.failure.unwrap().kind, ErrorKind::Internal);
        assert!(fixture
            .manager
            .runtime_status(MODULE)
            .unwrap()
            .active_operation_id
            .is_none());

        assert_eq!(
            fixture.run_start(OTHER).await.state,
            ModuleOperationState::Succeeded
        );
        assert!(fixture.manager.stop(&fixture.session, MODULE).is_ok());
    }

    #[tokio::test]
    async fn finished_history_is_bounded_and_unknown_operations_are_not_found() {
        let fixture = Fixture::unrecovered(&[MODULE]).await;
        assert_eq!(
            fixture.manager.operation("missing").unwrap_err(),
            ErrorKind::OperationNotFound
        );
        let mut ids = Vec::new();
        for _ in 0..MAX_FINISHED_OPERATIONS + 6 {
            let view = fixture.run_stop(MODULE).await;
            ids.push(view.operation_id);
        }
        assert_eq!(
            fixture.manager.operation(&ids[0]).unwrap_err(),
            ErrorKind::OperationNotFound
        );
        assert!(fixture.manager.operation(ids.last().unwrap()).is_ok());
        assert_eq!(
            fixture.manager.wait(&ids[0]).await.unwrap_err(),
            ErrorKind::OperationNotFound
        );
    }

    /// D035 restart semantics. A "restart" here is a second manager and
    /// publisher over the same store and database in the same process; it is
    /// not a real process restart.
    #[tokio::test]
    async fn operation_history_and_runtimes_do_not_survive_a_restart_but_update_audit_does() {
        use dgos_db::sea_orm::EntityTrait;
        let fixture = Fixture::new(&[MODULE]).await;
        fixture.run_start(MODULE).await;
        fixture.stage_update(MODULE, "2.0.0");
        let accepted = fixture.manager.update(&fixture.session, MODULE).unwrap();
        let before = fixture.finish(&accepted.operation_id).await;
        assert_eq!(before.state, ModuleOperationState::Succeeded);

        let publisher = Arc::new(ModulePublisher::new(
            fixture.store.path(),
            fixture.db.clone(),
        ));
        let restarted = ModuleOperationManager::new(
            publisher,
            fixture.loader.clone(),
            LocalModuleSources::new(fixture.sources.path()),
        );
        assert!(!restarted.is_available());
        assert_eq!(
            restarted.start(&fixture.session, MODULE).unwrap_err(),
            ErrorKind::HostUnavailable
        );
        assert_eq!(
            restarted.operation(&accepted.operation_id).unwrap_err(),
            ErrorKind::OperationNotFound
        );
        assert_eq!(
            restarted.runtime_status(MODULE).unwrap().state,
            ModuleRuntimeStatusState::NotLoaded
        );

        restarted.recover().await.unwrap();
        // Recovery starts nothing and restores no operation.
        assert_eq!(
            restarted.runtime_status(MODULE).unwrap().state,
            ModuleRuntimeStatusState::NotLoaded
        );
        assert_eq!(
            restarted.operation(&accepted.operation_id).unwrap_err(),
            ErrorKind::OperationNotFound
        );

        // The durable record is the update audit, keyed by the same id.
        let events = dgos_db::entities::module_update_events::Entity::find()
            .all(&fixture.db)
            .await
            .unwrap();
        assert!(events.iter().any(
            |event| event.operation_id == accepted.operation_id && event.to_version == "2.0.0"
        ));
        // The published release survives, so the caller can start again.
        let accepted = restarted.start(&fixture.session, MODULE).unwrap();
        let view = restarted.wait(&accepted.operation_id).await.unwrap();
        assert_eq!(view.state, ModuleOperationState::Succeeded);
        assert_eq!(
            restarted.runtime_status(MODULE).unwrap().version.as_deref(),
            Some("2.0.0")
        );
    }

    #[test]
    fn supervisor_errors_map_to_stable_kinds_without_raw_detail() {
        use ModuleOperationKind::{Start, Stop, Update};
        let io = || std::io::Error::other("/secret/path");
        let cases: Vec<(ModuleOperationKind, ModuleSupervisorError, ErrorKind)> = vec![
            (
                Start,
                ModuleSupervisorError::Package(ModulePackageError::Io(io())),
                ErrorKind::PackageInvalid,
            ),
            (
                Start,
                ModuleSupervisorError::Package(ModulePackageError::Manifest("x".into())),
                ErrorKind::PackageInvalid,
            ),
            (
                Start,
                ModuleSupervisorError::Package(ModulePackageError::Integrity),
                ErrorKind::PackageInvalid,
            ),
            (
                Start,
                ModuleSupervisorError::Package(ModulePackageError::Database("select 1".into())),
                ErrorKind::Internal,
            ),
            (
                Start,
                ModuleSupervisorError::Package(ModulePackageError::NotPublished),
                ErrorKind::ModuleNotFound,
            ),
            (
                Update,
                ModuleSupervisorError::Package(ModulePackageError::Disabled),
                ErrorKind::Disabled,
            ),
            (
                Start,
                ModuleSupervisorError::Load(ModuleLoadError::ArtifactChanged),
                ErrorKind::LoadFailed,
            ),
            (
                Update,
                ModuleSupervisorError::Load(ModuleLoadError::Rejected("x".into())),
                ErrorKind::LoadFailed,
            ),
            (
                Stop,
                ModuleSupervisorError::Load(ModuleLoadError::Io(io())),
                ErrorKind::StopFailed,
            ),
            (
                Start,
                ModuleSupervisorError::Runtime(ModuleRuntimeError::InvalidTransition),
                ErrorKind::Internal,
            ),
            (
                Stop,
                ModuleSupervisorError::Runtime(ModuleRuntimeError::RequestsInFlight(1)),
                ErrorKind::StopFailed,
            ),
            (
                Start,
                ModuleSupervisorError::AlreadyRunning,
                ErrorKind::AlreadyRunning,
            ),
            (
                Stop,
                ModuleSupervisorError::NotRunning,
                ErrorKind::NotRunning,
            ),
            (
                Update,
                ModuleSupervisorError::StaleRuntime,
                ErrorKind::VersionConflict,
            ),
            (
                Update,
                ModuleSupervisorError::VersionNotCurrent,
                ErrorKind::VersionConflict,
            ),
            (
                Update,
                ModuleSupervisorError::ModuleMismatch,
                ErrorKind::PackageInvalid,
            ),
            (
                Stop,
                ModuleSupervisorError::DrainTimeout,
                ErrorKind::StopFailed,
            ),
            (
                Update,
                ModuleSupervisorError::RolledBack(Box::new(ModuleSupervisorError::NotRunning)),
                ErrorKind::UpdateFailed,
            ),
            (
                Update,
                ModuleSupervisorError::RollbackFailed {
                    module_disabled: true,
                },
                ErrorKind::Disabled,
            ),
            (
                Update,
                ModuleSupervisorError::RollbackFailed {
                    module_disabled: false,
                },
                ErrorKind::Internal,
            ),
        ];
        for (kind, error, expected) in cases {
            assert_eq!(map_supervisor_error(kind, &error), expected, "{error:?}");
        }
        assert_eq!(
            map_source_error(ModuleSourceError::InvalidModuleId),
            ErrorKind::ModuleNotFound
        );
        assert_eq!(
            map_source_error(ModuleSourceError::NotFound),
            ErrorKind::PackageInvalid
        );
        assert_eq!(
            map_source_error(ModuleSourceError::Rejected),
            ErrorKind::PackageInvalid
        );
        assert_eq!(
            map_source_error(ModuleSourceError::Io(io())),
            ErrorKind::Internal
        );
    }
}
