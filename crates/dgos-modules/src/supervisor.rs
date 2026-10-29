//! Module runtime supervisor (D033).
//!
//! The supervisor is the only driver of runtime lifecycle transitions. It
//! starts publisher-issued handles through a `ModuleLoader`, drains and stops
//! them, records host-detected failures, and performs the minimal hot update:
//! the previous runtime keeps serving until the new package has been
//! verified, smoke-loaded, committed and started; a failed start rolls the
//! release back, and a failed rollback disables the module.

use crate::{
    admission::ModuleAdmission,
    error_class,
    loader::{LoadPhase, ModuleLoader, VerifiedModuleArtifact},
    runtime::{ModuleRuntimeError, ModuleRuntimeHandle, ModuleRuntimeState},
    ModuleLoadError, ModulePackageError, ModulePublisher, ReleaseSnapshot, StagedPackage,
};
use dgos_types::HostSession;
use std::{
    collections::HashMap,
    future::Future,
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
    /// The package is not on the host admission list, or an executing
    /// loader has no admission policy (D041).
    #[error("module package is not admitted for execution")]
    NotAdmitted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleUpdateOutcome {
    /// The same `(module_id, version, integrity)` is already active; nothing
    /// was restarted.
    Unchanged,
    /// The new runtime is active. `previous_stopped` is false when the
    /// previous runtime did not drain in time (or its loader failed to stop):
    /// it stays `failed` and is kept as a pending release that the next
    /// `stop`/`start` of the module retries once its requests have settled.
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

/// Deadlines for single loader calls (D044). On expiry only the loader
/// future is dropped, never the supervisor's own, and the normal failure
/// cleanup runs afterwards. Loaders must therefore be cancel-safe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoaderTimeouts {
    pub smoke: Duration,
    pub start: Duration,
    pub stop: Duration,
}

impl LoaderTimeouts {
    pub const DEFAULT: Self = Self {
        smoke: Duration::from_secs(5),
        start: Duration::from_secs(15),
        stop: Duration::from_secs(10),
    };
}

impl Default for LoaderTimeouts {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Run one loader call under its deadline.
async fn bounded<T>(
    phase: LoadPhase,
    limit: Duration,
    call: impl Future<Output = Result<T, ModuleLoadError>>,
) -> Result<T, ModuleLoadError> {
    match tokio::time::timeout(limit, call).await {
        Ok(result) => result,
        Err(_) => Err(ModuleLoadError::Timeout(phase)),
    }
}

/// Told once a runtime has become `ready`, before `start`/`update` return
/// (D051). Used by the host to hand the page a call route for exactly that
/// runtime. Best effort and synchronous: it must not block, and a failure
/// inside it never changes the runtime's state.
pub trait ModuleReadyObserver: Send + Sync {
    fn runtime_ready(&self, runtime: &ModuleRuntimeHandle);
}

pub struct ModuleRuntimeSupervisor {
    publisher: Arc<ModulePublisher>,
    loader: Arc<dyn ModuleLoader>,
    // Host policy deciding what may execute (D041). Without one, only
    // loaders that do not execute module code are usable.
    admission: Option<Arc<dyn ModuleAdmission>>,
    ready_observer: Option<Arc<dyn ModuleReadyObserver>>,
    timeouts: LoaderTimeouts,
    drain_timeout: Duration,
    // Serializes start/stop/update. Request admission never takes this lock.
    lifecycle: tokio::sync::Mutex<()>,
    runtimes: Mutex<HashMap<String, ModuleRuntimeHandle>>,
    // Runtimes that left `runtimes` (hot-update switch, failed candidate) but
    // whose loader release is not confirmed. They stay reachable so a failed
    // release can be retried instead of leaking loader resources.
    retired: Mutex<HashMap<String, Vec<ModuleRuntimeHandle>>>,
}

impl ModuleRuntimeSupervisor {
    pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn new(publisher: Arc<ModulePublisher>, loader: Arc<dyn ModuleLoader>) -> Self {
        Self {
            publisher,
            loader,
            admission: None,
            ready_observer: None,
            timeouts: LoaderTimeouts::DEFAULT,
            drain_timeout: Self::DEFAULT_DRAIN_TIMEOUT,
            lifecycle: tokio::sync::Mutex::new(()),
            runtimes: Mutex::new(HashMap::new()),
            retired: Mutex::new(HashMap::new()),
        }
    }

    /// Enforce `admission` before every preflight, start and update commit.
    pub fn with_admission(mut self, admission: Arc<dyn ModuleAdmission>) -> Self {
        self.admission = Some(admission);
        self
    }

    /// Observe every runtime that becomes `ready` (D051).
    pub fn with_ready_observer(mut self, observer: Arc<dyn ModuleReadyObserver>) -> Self {
        self.ready_observer = Some(observer);
        self
    }

    pub fn with_loader_timeouts(mut self, timeouts: LoaderTimeouts) -> Self {
        self.timeouts = timeouts;
        self
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

    /// Whether the configured loader runs module code. Lets a status view
    /// tell a really loaded `ready` runtime from a validated-only one.
    pub fn executes_module_code(&self) -> bool {
        self.loader.executes_module_code()
    }

    /// D041: execution needs an explicit admission. A loader that executes
    /// module code without a policy is refused, never trusted by default.
    fn admit(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleSupervisorError> {
        let admitted = match &self.admission {
            Some(admission) => admission.admits(
                artifact.module_id(),
                artifact.version(),
                artifact.integrity(),
            ),
            None => !self.loader.executes_module_code(),
        };
        if admitted {
            Ok(())
        } else {
            Err(ModuleSupervisorError::NotAdmitted)
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
        // A previous instance of this module must release its loader
        // resources before a new one starts; a failed release keeps it
        // registered and fails the start closed.
        self.release_retired(module_id).await?;
        if let Some(previous) = self.runtime(module_id) {
            if previous.state() != ModuleRuntimeState::Stopped {
                self.retire(&previous).await?;
            }
        }
        let handle = self.publisher.runtime_handle(session, module_id).await?;
        // Not admitted: refuse before the handle is registered, so nothing
        // needs cleaning up and nothing is preflighted or started (D041).
        self.admit(handle.artifact())?;
        self.registry().insert(module_id.to_owned(), handle.clone());
        let loaded = async {
            bounded(
                LoadPhase::Smoke,
                self.timeouts.smoke,
                self.loader.smoke(handle.artifact()),
            )
            .await?;
            self.activate(&handle).await
        }
        .await;
        if let Err(error) = loaded {
            // A failed release leaves the handle registered as `failed`; the
            // next `stop`/`start` retries it.
            let _ = self.abort_start(&handle).await;
            return Err(error);
        }
        Ok(handle)
    }

    /// Drain and stop the module's runtime and retry every pending release
    /// of runtimes it retired earlier. New requests are refused at once;
    /// admitted requests settle before loader resources are released. All
    /// releases are attempted; the first failure is returned and the
    /// unreleased runtimes stay pending for the next call.
    pub async fn stop(&self, module_id: &str) -> Result<(), ModuleSupervisorError> {
        let _lifecycle = self.lifecycle.lock().await;
        let had_pending = self.pending_release_count(module_id) > 0;
        let retired = self.release_retired(module_id).await;
        let current = match self.runtime(module_id) {
            Some(handle) => handle,
            None if had_pending => return retired,
            None => return Err(ModuleSupervisorError::NotRunning),
        };
        if current.state() == ModuleRuntimeState::Stopped {
            return retired;
        }
        // Ready → draining. A failed runtime already refuses requests.
        let _ = current.begin_draining();
        let current = self.retire(&current).await;
        retired.and(current)
    }

    /// Number of runtimes of `module_id` that were retired but whose loader
    /// release is still unconfirmed.
    pub fn pending_release_count(&self, module_id: &str) -> usize {
        self.retired_registry()
            .get(module_id)
            .map_or(0, |handles| handles.len())
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
                if self.abort_start(&candidate).await.is_err() {
                    // Never registered: keep it pending so its release is retried.
                    self.park_retired(module_id, candidate);
                }
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
        let previous_stopped = match self.retire(&previous).await {
            Ok(()) => true,
            Err(_) => {
                self.park_retired(module_id, previous);
                false
            }
        };
        report.outcome = ModuleUpdateOutcome::Switched { previous_stopped };
        Ok(report)
    }

    async fn smoke_staged(&self, staged: &StagedPackage) -> Result<(), ModuleSupervisorError> {
        let artifact = self.publisher.verify_staged(staged)?;
        // Admission comes first so an unadmitted package is never even
        // preflighted, and the pointer cannot move for it (D041).
        self.admit(&artifact)?;
        bounded(
            LoadPhase::Smoke,
            self.timeouts.smoke,
            self.loader.smoke(&artifact),
        )
        .await?;
        Ok(())
    }

    async fn activate(&self, handle: &ModuleRuntimeHandle) -> Result<(), ModuleSupervisorError> {
        // Defense in depth: the same decision again right before code runs.
        self.admit(handle.artifact())?;
        bounded(
            LoadPhase::Start,
            self.timeouts.start,
            self.loader.start(handle.runtime_id(), handle.artifact()),
        )
        .await?;
        handle.mark_ready()?;
        if let Some(observer) = &self.ready_observer {
            observer.runtime_ready(handle);
        }
        Ok(())
    }

    /// A runtime that failed to start never admitted a request; mark it
    /// failed and release whatever the loader may hold.
    async fn abort_start(&self, handle: &ModuleRuntimeHandle) -> Result<(), ModuleLoadError> {
        let _ = handle.fail();
        release_runtime(self.loader.as_ref(), handle, self.timeouts.stop).await
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
        // Exactly once per instance: a background release that already
        // succeeded is not repeated, and a failed one is retried here.
        if let Err(error) = release_runtime(self.loader.as_ref(), handle, self.timeouts.stop).await
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
        let stop_timeout = self.timeouts.stop;
        runtime.spawn(async move {
            handle.wait_settled().await;
            // A failure here leaves the release pending for the next `stop`.
            let _ = release_runtime(loader.as_ref(), &handle, stop_timeout).await;
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

    fn retired_registry(&self) -> MutexGuard<'_, HashMap<String, Vec<ModuleRuntimeHandle>>> {
        self.retired
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn park_retired(&self, module_id: &str, handle: ModuleRuntimeHandle) {
        let mut retired = self.retired_registry();
        let handles = retired.entry(module_id.to_owned()).or_default();
        if !handles.iter().any(|known| known.is_same_runtime(&handle)) {
            handles.push(handle);
        }
    }

    fn unpark_retired(&self, module_id: &str, handle: &ModuleRuntimeHandle) {
        let mut retired = self.retired_registry();
        if let Some(handles) = retired.get_mut(module_id) {
            handles.retain(|known| !known.is_same_runtime(handle));
            if handles.is_empty() {
                retired.remove(module_id);
            }
        }
    }

    /// Retry every pending release of `module_id`. A handle leaves the
    /// pending list only after it reached `stopped`, so dropping this future
    /// part-way loses nothing. Returns the first failure after trying all.
    async fn release_retired(&self, module_id: &str) -> Result<(), ModuleSupervisorError> {
        let pending = self
            .retired_registry()
            .get(module_id)
            .cloned()
            .unwrap_or_default();
        let mut first_error = None;
        for handle in pending {
            match self.retire(&handle).await {
                Ok(()) => self.unpark_retired(module_id, &handle),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// The only place that calls `ModuleLoader::stop`. The handle records a
/// confirmed release, so the loader sees at most one successful `stop` per
/// runtime instance however `report_failure`, drain timeout, explicit `stop`
/// and failed-start cleanup interleave.
async fn release_runtime(
    loader: &dyn ModuleLoader,
    handle: &ModuleRuntimeHandle,
    stop_timeout: Duration,
) -> Result<(), ModuleLoadError> {
    // A timed-out stop is not recorded as released, so the next call retries.
    handle
        .release_once(|| {
            bounded(
                LoadPhase::Stop,
                stop_timeout,
                loader.stop(handle.runtime_id(), handle.artifact()),
            )
        })
        .await
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
        ModuleSupervisorError::NotAdmitted => "admission".into(),
        _ => "lifecycle".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        digest, package_integrity, LocalArtifactLoader, ModuleRequestOutcome,
        VerifiedModuleArtifact,
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
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use tempfile::{tempdir, TempDir};

    /// Real local checks, plus scripted start/stop failures and loader
    /// accounting. `stops` lists confirmed releases; `stop_attempts` counts
    /// every call, including scripted failures.
    #[derive(Default)]
    struct ScriptedLoader {
        fail_start_version: Mutex<Option<String>>,
        executes: AtomicBool,
        hang_smoke: AtomicBool,
        hang_start: AtomicBool,
        hanging_stops: AtomicUsize,
        smokes: AtomicUsize,
        starts: AtomicUsize,
        stop_attempts: AtomicUsize,
        failing_stops: AtomicUsize,
        stop_delay: Mutex<Duration>,
        stops: Mutex<Vec<String>>,
    }

    impl ScriptedLoader {
        fn fail_start(&self, version: Option<&str>) {
            *self.fail_start_version.lock().unwrap() = version.map(str::to_owned);
        }

        fn hang_smoke(&self, hang: bool) {
            self.hang_smoke.store(hang, Ordering::SeqCst);
        }

        fn hang_start(&self, hang: bool) {
            self.hang_start.store(hang, Ordering::SeqCst);
        }

        /// The next `count` stop calls never return (until dropped).
        fn hang_stops(&self, count: usize) {
            self.hanging_stops.store(count, Ordering::SeqCst);
        }

        /// The next `count` stop calls fail without releasing anything.
        fn fail_next_stops(&self, count: usize) {
            self.failing_stops.store(count, Ordering::SeqCst);
        }

        fn delay_stops(&self, delay: Duration) {
            *self.stop_delay.lock().unwrap() = delay;
        }

        fn stop_attempts(&self) -> usize {
            self.stop_attempts.load(Ordering::SeqCst)
        }

        fn stops(&self) -> Vec<String> {
            self.stops.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ModuleLoader for ScriptedLoader {
        fn executes_module_code(&self) -> bool {
            self.executes.load(Ordering::SeqCst)
        }

        async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
            self.smokes.fetch_add(1, Ordering::SeqCst);
            if self.hang_smoke.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            LocalArtifactLoader.smoke(artifact).await
        }

        async fn start(
            &self,
            runtime_id: &str,
            artifact: &VerifiedModuleArtifact,
        ) -> Result<(), ModuleLoadError> {
            if self.hang_start.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
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
            self.stop_attempts.fetch_add(1, Ordering::SeqCst);
            let hang = self
                .hanging_stops
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok();
            if hang {
                std::future::pending::<()>().await;
            }
            let delay = *self.stop_delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let scripted_failure = self
                .failing_stops
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok();
            if scripted_failure {
                return Err(ModuleLoadError::Rejected("scripted stop failure".into()));
            }
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
        requires: Option<serde_json::Value>,
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
                requires: None,
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
        let mut manifest = serde_json::json!({
            "id": spec.module_id,
            "name": "Example",
            "version": spec.version,
            "entry": spec.entry,
            "artifact": {"version": spec.version, "sha256": "0".repeat(64)},
            "permissions": spec.permissions,
        });
        if let Some(requires) = spec.requires {
            manifest["requires"] = requires;
        }
        fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
        // The manifest is part of the v2 digest, the declared value is not:
        // seal it after the rest is written. A package that cannot be
        // digested (invalid entry) keeps the placeholder.
        let declared = spec.digest.map(str::to_owned).unwrap_or_else(|| {
            package_integrity(dir.path())
                .ok()
                .and_then(|integrity| digest::v2_hex(&integrity).map(str::to_owned))
                .unwrap_or_else(|| "0".repeat(64))
        });
        manifest["artifact"]["sha256"] = declared.into();
        fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
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

    #[derive(Default)]
    struct RecordingObserver {
        seen: Mutex<Vec<(String, ModuleRuntimeState)>>,
    }

    impl ModuleReadyObserver for RecordingObserver {
        fn runtime_ready(&self, runtime: &ModuleRuntimeHandle) {
            self.seen
                .lock()
                .unwrap()
                .push((runtime.runtime_id().to_owned(), runtime.state()));
        }
    }

    #[tokio::test]
    async fn the_ready_observer_sees_each_runtime_once_and_only_when_it_is_ready() {
        let mut fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        let observer = Arc::new(RecordingObserver::default());
        fixture.supervisor = ModuleRuntimeSupervisor::with_drain_timeout(
            fixture.publisher.clone(),
            fixture.loader.clone(),
            ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT,
        )
        .with_ready_observer(observer.clone());
        fixture.publish("1.0.0", &[]).await;

        // A failed start never reaches the observer.
        fixture.loader.fail_start(Some("1.0.0"));
        assert!(fixture
            .supervisor
            .start(&HostSession::local_default(), MODULE)
            .await
            .is_err());
        assert!(observer.seen.lock().unwrap().is_empty());
        fixture.supervisor.stop(MODULE).await.unwrap();
        fixture.loader.fail_start(None);

        let first = fixture.start().await;
        let source = package(PackageSpec {
            version: "2.0.0",
            ..PackageSpec::default()
        });
        fixture.update(&source).await.unwrap();
        let second = fixture.supervisor.runtime(MODULE).unwrap();

        // The observer saw the candidate of each successful activation, in
        // `ready` state, before the call returned.
        assert_eq!(
            *observer.seen.lock().unwrap(),
            vec![
                (first.runtime_id().to_owned(), ModuleRuntimeState::Ready),
                (second.runtime_id().to_owned(), ModuleRuntimeState::Ready),
            ]
        );

        // A candidate whose start fails is rolled back and never observed.
        let broken = package(PackageSpec {
            version: "3.0.0",
            ..PackageSpec::default()
        });
        fixture.loader.fail_start(Some("3.0.0"));
        assert!(fixture.update(&broken).await.is_err());
        assert_eq!(observer.seen.lock().unwrap().len(), 2);
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
        assert_eq!(fixture.loader.stop_attempts(), 1);
    }

    #[tokio::test]
    async fn drain_timeout_then_immediate_stop_releases_loader_once() {
        let fixture = Fixture::new(Duration::from_millis(30)).await;
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        let request = handle.begin_request().unwrap();
        assert!(matches!(
            fixture.supervisor.stop(MODULE).await,
            Err(ModuleSupervisorError::DrainTimeout)
        ));

        // Settle and stop right away: the background release and the
        // explicit stop contend, and only one reaches the loader.
        request.finish(ModuleRequestOutcome::Succeeded);
        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(fixture.loader.stops(), vec![handle.runtime_id().to_owned()]);
        assert_eq!(fixture.loader.stop_attempts(), 1);
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
        let digest = package_integrity(&target).unwrap();
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
        assert_eq!(fixture.loader.stops(), vec![handle.runtime_id().to_owned()]);
        assert_eq!(fixture.loader.stop_attempts(), 1);
    }

    #[tokio::test]
    async fn explicit_stop_during_background_release_waits_and_does_not_repeat_it() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        fixture.loader.delay_stops(Duration::from_millis(60));
        fixture
            .supervisor
            .report_failure(MODULE, handle.runtime_id())
            .unwrap();

        // The background release is now inside `loader.stop`; the explicit
        // stop must wait for it instead of starting a second release.
        wait_until(|| fixture.loader.stop_attempts() == 1).await;
        assert!(fixture.loader.stops().is_empty());
        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        assert_eq!(fixture.loader.stops(), vec![handle.runtime_id().to_owned()]);
        assert_eq!(fixture.loader.stop_attempts(), 1);
    }

    #[tokio::test]
    async fn duplicate_report_failure_is_idempotent_and_releases_once() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        let request = handle.begin_request().unwrap();
        for _ in 0..3 {
            fixture
                .supervisor
                .report_failure(MODULE, handle.runtime_id())
                .unwrap();
        }
        assert_eq!(handle.state(), ModuleRuntimeState::Failed);
        tokio::task::yield_now().await;
        assert_eq!(fixture.loader.stop_attempts(), 0);

        request.finish(ModuleRequestOutcome::Succeeded);
        wait_until(|| fixture.loader.stops().len() == 1).await;
        // A report after the release is still accepted while the runtime is
        // merely failed, and must not release again.
        fixture
            .supervisor
            .report_failure(MODULE, handle.runtime_id())
            .unwrap();
        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);

        // Once stopped, the instance no longer accepts failure reports.
        assert!(matches!(
            fixture
                .supervisor
                .report_failure(MODULE, handle.runtime_id()),
            Err(ModuleSupervisorError::Runtime(
                ModuleRuntimeError::InvalidTransition
            ))
        ));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(fixture.loader.stops(), vec![handle.runtime_id().to_owned()]);
        assert_eq!(fixture.loader.stop_attempts(), 1);
        let stats = handle.request_stats();
        assert_eq!(
            (stats.admitted, stats.succeeded, stats.in_flight),
            (1, 1, 0)
        );
    }

    #[tokio::test]
    async fn failed_loader_stop_is_retried_by_the_next_stop() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        fixture.loader.fail_next_stops(1);

        assert!(matches!(
            fixture.supervisor.stop(MODULE).await,
            Err(ModuleSupervisorError::Load(_))
        ));
        assert_eq!(handle.state(), ModuleRuntimeState::Failed);
        assert!(handle.begin_request().is_err());
        assert!(fixture.loader.stops().is_empty());
        assert_eq!(fixture.loader.stop_attempts(), 1);

        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        assert_eq!(fixture.loader.stops(), vec![handle.runtime_id().to_owned()]);
        assert_eq!(fixture.loader.stop_attempts(), 2);

        // Released: further stops are no-ops and never reach the loader.
        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(fixture.loader.stop_attempts(), 2);
    }

    #[tokio::test]
    async fn failed_background_release_is_retried_by_explicit_stop() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        fixture.loader.fail_next_stops(1);
        fixture
            .supervisor
            .report_failure(MODULE, handle.runtime_id())
            .unwrap();
        wait_until(|| fixture.loader.stop_attempts() == 1).await;
        tokio::task::yield_now().await;
        assert!(fixture.loader.stops().is_empty());
        assert_eq!(handle.state(), ModuleRuntimeState::Failed);

        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        assert_eq!(fixture.loader.stops(), vec![handle.runtime_id().to_owned()]);
        assert_eq!(fixture.loader.stop_attempts(), 2);
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
    async fn update_refuses_an_incompatible_package_and_leaves_the_previous_release_running() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        for requires in [
            serde_json::json!({"react": "^20"}),
            serde_json::json!({"api": 2}),
        ] {
            let source = package(PackageSpec {
                version: "2.0.0",
                requires: Some(requires),
                ..PackageSpec::default()
            });
            assert!(matches!(
                fixture.update(&source).await,
                Err(ModuleSupervisorError::Package(
                    ModulePackageError::Incompatible
                ))
            ));
        }

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
    async fn start_refuses_an_incompatible_published_release_and_recovery_keeps_it_enabled() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        // A release that is already incompatible cannot be staged any more, so
        // it is placed the way an older host published it: a sealed package,
        // its files under `versions/`, and the committed rows (D055).
        let source = package(PackageSpec {
            requires: Some(serde_json::json!({"react": "^20"})),
            ..PackageSpec::default()
        });
        let integrity = package_integrity(source.path()).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(source.path().join("manifest.json")).unwrap())
                .unwrap();
        let target = fixture
            .store
            .path()
            .join("versions")
            .join(MODULE)
            .join("1.0.0");
        crate::copy_dir(source.path(), &target).unwrap();
        modules::ActiveModel {
            id: Set(MODULE.into()),
            module_name: Set(MODULE.into()),
            display_name: Set("Example".into()),
            current_version: Set("1.0.0".into()),
            description: Set(None),
            manifest: Set(manifest),
            integrity: Set(integrity.clone()),
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
        dgos_db::entities::module_versions::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            module_id: Set(MODULE.into()),
            version: Set("1.0.0".into()),
            integrity: Set(integrity),
            entry_path: Set("entry.js".into()),
            created_at: Set(0),
        }
        .insert(&fixture.db)
        .await
        .unwrap();

        assert_eq!(
            fixture.publisher.recover_registered().await.unwrap(),
            vec![MODULE]
        );
        assert!(fixture.module_row().await.is_enabled);
        assert_eq!(fixture.pointer().as_deref(), Some("1.0.0"));

        assert!(matches!(
            fixture
                .supervisor
                .start(&HostSession::local_default(), MODULE)
                .await,
            Err(ModuleSupervisorError::Package(
                ModulePackageError::Incompatible
            ))
        ));
        assert!(fixture.supervisor.runtime(MODULE).is_none());
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 0);
        assert!(fixture.module_row().await.is_enabled);
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

    #[tokio::test]
    async fn retired_runtime_with_failed_release_stays_pending_and_stop_retries_it() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        let source = package(PackageSpec {
            version: "2.0.0",
            ..PackageSpec::default()
        });
        fixture.loader.fail_next_stops(1);

        let report = fixture.update(&source).await.unwrap();
        assert_eq!(
            report.outcome,
            ModuleUpdateOutcome::Switched {
                previous_stopped: false
            }
        );
        let current = fixture.supervisor.runtime(MODULE).unwrap();
        assert_eq!(current.state(), ModuleRuntimeState::Ready);
        assert_eq!(previous.state(), ModuleRuntimeState::Failed);
        assert_eq!(fixture.supervisor.pending_release_count(MODULE), 1);
        assert!(fixture.loader.stops().is_empty());

        // The retry of the retired runtime fails again: still pending, and
        // the current runtime is still drained and released.
        fixture.loader.fail_next_stops(1);
        assert!(matches!(
            fixture.supervisor.stop(MODULE).await,
            Err(ModuleSupervisorError::Load(_))
        ));
        assert_eq!(previous.state(), ModuleRuntimeState::Failed);
        assert_eq!(current.state(), ModuleRuntimeState::Stopped);
        assert_eq!(fixture.supervisor.pending_release_count(MODULE), 1);

        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(previous.state(), ModuleRuntimeState::Stopped);
        assert_eq!(fixture.supervisor.pending_release_count(MODULE), 0);
        let mut stops = fixture.loader.stops();
        stops.sort();
        let mut expected = vec![
            previous.runtime_id().to_owned(),
            current.runtime_id().to_owned(),
        ];
        expected.sort();
        assert_eq!(stops, expected);
        // Released once each: nothing is pending, so further stops are no-ops.
        let attempts = fixture.loader.stop_attempts();
        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(fixture.loader.stop_attempts(), attempts);
    }

    #[tokio::test]
    async fn retired_runtime_released_only_after_its_requests_settle() {
        let fixture = Fixture::new(Duration::from_millis(30)).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        let request = previous.begin_request().unwrap();
        let source = package(PackageSpec {
            version: "2.0.0",
            ..PackageSpec::default()
        });

        let report = fixture.update(&source).await.unwrap();
        assert_eq!(
            report.outcome,
            ModuleUpdateOutcome::Switched {
                previous_stopped: false
            }
        );
        assert_eq!(fixture.supervisor.pending_release_count(MODULE), 1);
        assert!(fixture.loader.stops().is_empty());

        // Still in flight: stop reports the pending release and releases nothing for it.
        assert!(matches!(
            fixture.supervisor.stop(MODULE).await,
            Err(ModuleSupervisorError::DrainTimeout)
        ));
        assert_eq!(previous.state(), ModuleRuntimeState::Failed);
        assert!(!fixture
            .loader
            .stops()
            .contains(&previous.runtime_id().to_owned()));

        request.finish(ModuleRequestOutcome::Succeeded);
        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(previous.state(), ModuleRuntimeState::Stopped);
        assert_eq!(fixture.supervisor.pending_release_count(MODULE), 0);
        assert_eq!(previous.request_stats().succeeded, 1);
        let stops = fixture.loader.stops();
        assert_eq!(
            stops
                .iter()
                .filter(|id| id.as_str() == previous.runtime_id())
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn candidate_with_failed_release_after_rollback_is_retried_by_stop() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        fixture.loader.fail_start(Some("2.0.0"));
        fixture.loader.fail_next_stops(1);
        let source = package(PackageSpec {
            version: "2.0.0",
            ..PackageSpec::default()
        });

        assert!(matches!(
            fixture.update(&source).await,
            Err(ModuleSupervisorError::RolledBack(_))
        ));
        assert_eq!(previous.state(), ModuleRuntimeState::Ready);
        assert_eq!(fixture.supervisor.pending_release_count(MODULE), 1);
        assert!(fixture.loader.stops().is_empty());

        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(fixture.supervisor.pending_release_count(MODULE), 0);
        assert_eq!(previous.state(), ModuleRuntimeState::Stopped);
        // Previous plus the failed candidate, each exactly once.
        assert_eq!(fixture.loader.stops().len(), 2);
    }

    #[tokio::test]
    async fn start_settles_a_failed_previous_instance_before_replacing_it() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        fixture.loader.fail_next_stops(1);
        assert!(fixture.supervisor.stop(MODULE).await.is_err());
        assert_eq!(previous.state(), ModuleRuntimeState::Failed);

        // The release fails again, so start fails closed and keeps the
        // previous instance registered for another retry.
        fixture.loader.fail_next_stops(1);
        assert!(fixture
            .supervisor
            .start(&HostSession::local_default(), MODULE)
            .await
            .is_err());
        assert!(fixture
            .supervisor
            .runtime(MODULE)
            .unwrap()
            .is_same_runtime(&previous));
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 1);

        let restarted = fixture.start().await;
        assert_eq!(previous.state(), ModuleRuntimeState::Stopped);
        assert!(!restarted.is_same_runtime(&previous));
        assert_eq!(restarted.state(), ModuleRuntimeState::Ready);
        assert_eq!(
            fixture.loader.stops(),
            vec![previous.runtime_id().to_owned()]
        );
    }

    fn pin_current(fixture: &Fixture, version: &str, integrity: &str) -> Arc<dyn ModuleAdmission> {
        let _ = fixture;
        Arc::new(crate::PinnedModuleAdmission::new([(
            MODULE, version, integrity,
        )]))
    }

    async fn current_integrity(fixture: &Fixture) -> String {
        fixture
            .publisher
            .release_snapshot(MODULE)
            .await
            .unwrap()
            .unwrap()
            .integrity()
            .to_owned()
    }

    #[tokio::test]
    async fn executing_loader_without_admission_is_refused_before_any_loader_call() {
        let fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.loader.executes.store(true, Ordering::SeqCst);
        fixture.publish("1.0.0", &[]).await;

        assert!(matches!(
            fixture
                .supervisor
                .start(&HostSession::local_default(), MODULE)
                .await,
            Err(ModuleSupervisorError::NotAdmitted)
        ));
        assert!(fixture.supervisor.runtime(MODULE).is_none());
        assert_eq!(fixture.loader.smokes.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.loader.starts.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.loader.stop_attempts(), 0);
    }

    #[tokio::test]
    async fn only_pinned_releases_run_and_unpinned_updates_never_commit() {
        let mut fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.publish("1.0.0", &[]).await;
        let integrity = current_integrity(&fixture).await;
        fixture.supervisor = ModuleRuntimeSupervisor::with_drain_timeout(
            fixture.publisher.clone(),
            fixture.loader.clone(),
            ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT,
        )
        .with_admission(pin_current(&fixture, "1.0.0", &integrity));
        fixture.loader.executes.store(true, Ordering::SeqCst);

        let previous = fixture.start().await;
        assert_eq!(previous.state(), ModuleRuntimeState::Ready);
        let smokes = fixture.loader.smokes.load(Ordering::SeqCst);

        // A self-consistent package that is not pinned: its digest matches
        // its own manifest, which is exactly what admission must not trust.
        let source = package(PackageSpec {
            version: "2.0.0",
            ..PackageSpec::default()
        });
        assert!(matches!(
            fixture.update(&source).await,
            Err(ModuleSupervisorError::NotAdmitted)
        ));
        assert_eq!(fixture.pointer().as_deref(), Some("1.0.0"));
        assert_eq!(fixture.module_row().await.current_version, "1.0.0");
        assert!(fixture.staging_is_empty());
        // Refused before the loader preflight, and the old runtime serves on.
        assert_eq!(fixture.loader.smokes.load(Ordering::SeqCst), smokes);
        assert_eq!(previous.state(), ModuleRuntimeState::Ready);
        assert!(fixture
            .supervisor
            .runtime(MODULE)
            .unwrap()
            .is_same_runtime(&previous));
    }

    #[tokio::test]
    async fn start_deadline_fails_the_runtime_releases_the_loader_and_frees_the_lifecycle() {
        let mut fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.supervisor = ModuleRuntimeSupervisor::with_drain_timeout(
            fixture.publisher.clone(),
            fixture.loader.clone(),
            ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT,
        )
        .with_loader_timeouts(LoaderTimeouts {
            smoke: Duration::from_secs(5),
            start: Duration::from_millis(50),
            stop: Duration::from_secs(5),
        });
        fixture.publish("1.0.0", &[]).await;
        fixture.loader.hang_start(true);

        let error = fixture
            .supervisor
            .start(&HostSession::local_default(), MODULE)
            .await
            .err()
            .expect("start must time out");
        assert!(matches!(
            error,
            ModuleSupervisorError::Load(ModuleLoadError::Timeout(LoadPhase::Start))
        ));
        let failed = fixture.supervisor.runtime(MODULE).unwrap();
        assert_eq!(failed.state(), ModuleRuntimeState::Failed);
        assert!(failed.begin_request().is_err());
        // The hung loader call was abandoned, and cleanup still reached it.
        assert_eq!(fixture.loader.stops(), vec![failed.runtime_id().to_owned()]);

        // The lifecycle lock is free and a retry is a new runtime.
        fixture.loader.hang_start(false);
        let restarted = fixture.start().await;
        assert_eq!(restarted.state(), ModuleRuntimeState::Ready);
        assert_ne!(restarted.runtime_id(), failed.runtime_id());
        assert_eq!(failed.state(), ModuleRuntimeState::Stopped);
    }

    #[tokio::test]
    async fn stop_deadline_is_retryable_and_releases_exactly_once() {
        let mut fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.supervisor = ModuleRuntimeSupervisor::with_drain_timeout(
            fixture.publisher.clone(),
            fixture.loader.clone(),
            ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT,
        )
        .with_loader_timeouts(LoaderTimeouts {
            smoke: Duration::from_secs(5),
            start: Duration::from_secs(5),
            stop: Duration::from_millis(50),
        });
        fixture.publish("1.0.0", &[]).await;
        let handle = fixture.start().await;
        fixture.loader.hang_stops(1);

        assert!(matches!(
            fixture.supervisor.stop(MODULE).await,
            Err(ModuleSupervisorError::Load(ModuleLoadError::Timeout(
                LoadPhase::Stop
            )))
        ));
        // Unconfirmed release: refused for new requests but not stopped.
        assert_eq!(handle.state(), ModuleRuntimeState::Failed);
        assert!(handle.begin_request().is_err());
        assert!(fixture.loader.stops().is_empty());

        fixture.supervisor.stop(MODULE).await.unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        assert_eq!(fixture.loader.stops(), vec![handle.runtime_id().to_owned()]);
        assert_eq!(fixture.loader.stop_attempts(), 2);
    }

    #[tokio::test]
    async fn preflight_deadline_during_update_leaves_pointer_and_runtime_unchanged() {
        let mut fixture = Fixture::new(ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT).await;
        fixture.supervisor = ModuleRuntimeSupervisor::with_drain_timeout(
            fixture.publisher.clone(),
            fixture.loader.clone(),
            ModuleRuntimeSupervisor::DEFAULT_DRAIN_TIMEOUT,
        )
        .with_loader_timeouts(LoaderTimeouts {
            smoke: Duration::from_millis(50),
            start: Duration::from_secs(5),
            stop: Duration::from_secs(5),
        });
        fixture.publish("1.0.0", &[]).await;
        let previous = fixture.start().await;
        fixture.loader.hang_smoke(true);

        let source = package(PackageSpec {
            version: "2.0.0",
            ..PackageSpec::default()
        });
        assert!(matches!(
            fixture.update(&source).await,
            Err(ModuleSupervisorError::Load(ModuleLoadError::Timeout(
                LoadPhase::Smoke
            )))
        ));
        assert_eq!(fixture.pointer().as_deref(), Some("1.0.0"));
        assert_eq!(fixture.module_row().await.current_version, "1.0.0");
        assert!(fixture.staging_is_empty());
        assert_eq!(previous.state(), ModuleRuntimeState::Ready);
        assert!(fixture
            .events()
            .await
            .contains(&("load_smoke".into(), "failure".into())));
    }
}
