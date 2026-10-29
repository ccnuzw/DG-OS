//! Transport-independent application services.
//!
//! Commands, HTTP handlers and a future module runtime should call these
//! use-case services rather than composing capability checks and domain
//! managers independently. The services bind trusted execution context and
//! leave resource ownership and provider invariants to their domain managers.

mod module_crashes;
mod module_management;
mod module_operations;
pub use module_crashes::{
    sanitize_error_name, sanitize_stack, DisableRetryPolicy, ModuleCrashService,
    AUTO_DISABLE_CRASHES,
};
pub use module_management::ModuleManagementService;
pub use module_operations::ModuleOperationManager;

use dgos_db::{ModulePermission, ModulePermissionRepository};
use dgos_modules::{BoundModuleCall, ModulePublisher, ModuleRequestOutcome, ModuleRuntimeHandle};
use dgos_providers::ProviderManager;
use dgos_types::{
    BuiltinCaller, ChatRequest, ChatResponse, DgosError, DgosResult, ExecutionProvenance,
    HostExecutionContext,
};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Total host-owned deadline of one module `providers.call` (D051).
pub const PROVIDER_CALL_DEADLINE: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct HostCapabilityService {
    permissions: Arc<ModulePermissionRepository>,
    publisher: Option<Arc<ModulePublisher>>,
}

#[derive(Clone, Debug)]
pub struct CapabilityGrant {
    pub module_id: String,
    pub version: String,
    pub integrity: String,
    pub permission: ModulePermission,
    pub request_id: String,
}

impl HostCapabilityService {
    #[cfg(test)]
    pub(crate) fn new(permissions: Arc<ModulePermissionRepository>) -> Self {
        Self {
            permissions,
            publisher: None,
        }
    }

    pub fn with_publisher(
        permissions: Arc<ModulePermissionRepository>,
        publisher: Arc<ModulePublisher>,
    ) -> Self {
        Self {
            permissions,
            publisher: Some(publisher),
        }
    }

    /// Validate identity and authorization immediately before dispatch. A
    /// failed audit is never allowed to turn a denial into authorization.
    pub async fn authorize(
        &self,
        context: &HostExecutionContext,
        permission: ModulePermission,
    ) -> DgosResult<CapabilityGrant> {
        let (module_id, version, integrity) = context
            .module_identity()
            .ok_or(DgosError::PermissionDenied)?;
        if let Some(publisher) = &self.publisher {
            if !publisher
                .validate_execution_identity(module_id, version, integrity)
                .await
                .map_err(|_| DgosError::PermissionDenied)?
            {
                return Err(DgosError::PermissionDenied);
            }
        }
        if !self
            .permissions
            .validate_module_capability(module_id, version, integrity, permission)
            .await?
        {
            return Err(DgosError::PermissionDenied);
        }
        if !self
            .permissions
            .check_with_request(
                context.user_id(),
                module_id,
                permission,
                Some(context.request_id()),
            )
            .await?
        {
            return Err(DgosError::PermissionDenied);
        }
        Ok(CapabilityGrant {
            module_id: module_id.to_owned(),
            version: version.to_owned(),
            integrity: integrity.to_owned(),
            permission,
            request_id: context.request_id().to_owned(),
        })
    }

    /// The host dispatch boundary: authorization is checked immediately before
    /// the supplied domain operation, never by a renderer-visible preflight.
    pub async fn dispatch<T, F, Fut>(
        &self,
        context: &HostExecutionContext,
        permission: ModulePermission,
        operation: F,
    ) -> DgosResult<T>
    where
        F: FnOnce(CapabilityGrant) -> Fut,
        Fut: Future<Output = DgosResult<T>>,
    {
        let grant = self.authorize(context, permission).await?;
        operation(grant).await
    }

    /// Provider is the first concrete capability adapter. Domain ownership,
    /// provider revision and external-call invariants remain in the manager.
    pub async fn call_provider(
        &self,
        context: &HostExecutionContext,
        providers: &ProviderManager,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<ChatResponse> {
        let user_id = context.user_id().to_owned();
        let provenance = context.provenance();
        if context.module_identity().is_some() {
            self.dispatch(
                context,
                ModulePermission::ProvidersCall,
                move |_| async move {
                    providers
                        .call_provider_bound_with_provenance(
                            provider_id,
                            model_id,
                            request,
                            &user_id,
                            None,
                            Some(&provenance),
                        )
                        .await
                },
            )
            .await
        } else {
            providers
                .call_provider_bound_with_provenance(
                    provider_id,
                    model_id,
                    request,
                    &user_id,
                    None,
                    Some(&provenance),
                )
                .await
        }
    }
}

/// A transport-neutral use case for a provider call. It keeps command/HTTP
/// adapters from knowing how capability authorization is composed.
#[derive(Clone)]
pub struct ProviderCallService {
    capabilities: HostCapabilityService,
}

/// Runtime-only capability adapter (D033). A module runtime must present a
/// ready, publisher-issued handle rather than manufacturing an execution
/// context. Admission, authorization and the operation share one in-flight
/// request, so a draining runtime waits for authorization too and every
/// admitted request settles exactly once. Nothing is retried or replayed.
#[derive(Clone)]
pub struct ModuleCapabilityDispatchService {
    capabilities: HostCapabilityService,
    provider_deadline: Duration,
}

impl ModuleCapabilityDispatchService {
    pub fn new(capabilities: HostCapabilityService) -> Self {
        Self {
            capabilities,
            provider_deadline: PROVIDER_CALL_DEADLINE,
        }
    }

    /// Override the host-owned `providers.call` deadline. Production keeps
    /// `PROVIDER_CALL_DEADLINE`; a module can never extend it, only the host
    /// assembling this service chooses it.
    pub fn with_provider_deadline(mut self, deadline: Duration) -> Self {
        self.provider_deadline = deadline;
        self
    }

    pub async fn dispatch<T, F, Fut>(
        &self,
        handle: &ModuleRuntimeHandle,
        permission: ModulePermission,
        operation: F,
    ) -> DgosResult<T>
    where
        F: FnOnce(CapabilityGrant) -> Fut,
        Fut: Future<Output = DgosResult<T>>,
    {
        let request = handle
            .begin_request()
            .map_err(|_| DgosError::PermissionDenied)?;
        let grant = match self
            .capabilities
            .authorize(request.context(), permission)
            .await
        {
            Ok(grant) => grant,
            Err(error) => {
                request.finish(ModuleRequestOutcome::Denied);
                return Err(error);
            }
        };
        // If this future is dropped from here on, the request settles as
        // `Interrupted`: the external result is unknown and not replayed.
        let result = operation(grant).await;
        request.finish(if result.is_ok() {
            ModuleRequestOutcome::Succeeded
        } else {
            ModuleRequestOutcome::Failed
        });
        result
    }

    /// `providers.call` for a runtime-bound module (D048). The user comes
    /// from the supervisor-issued handle and the request id from the
    /// per-request grant; the caller supplies neither. Authorization is
    /// re-checked on every call, so a revoked grant stops the next call but
    /// never cancels one already in flight (D033). Calls are independent
    /// external side effects: nothing is deduplicated, retried or replayed
    /// here. Provider failures keep their own error; only admission and
    /// authorization failures map to `PermissionDenied`.
    pub async fn call_provider(
        &self,
        handle: &ModuleRuntimeHandle,
        providers: &ProviderManager,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<ChatResponse> {
        let user_id = handle.context().user_id().to_owned();
        self.dispatch(
            handle,
            ModulePermission::ProvidersCall,
            move |grant| async move {
                let provenance = ExecutionProvenance {
                    caller: BuiltinCaller::Module,
                    request_id: grant.request_id,
                };
                providers
                    .call_provider_bound_with_provenance(
                        provider_id,
                        model_id,
                        request,
                        &user_id,
                        None,
                        Some(&provenance),
                    )
                    .await
            },
        )
        .await
    }

    /// `providers.call` for a call already admitted by the channel (D051).
    /// The `BoundModuleCall` owns the runtime request, so the user, module
    /// identity and `request_id` all come from the host. Authorization is
    /// re-checked here on every call. The total deadline is host-owned: on
    /// expiry the external call may still complete, so the request settles as
    /// `Interrupted` (unknown) and the caller gets `Timeout`, never a
    /// retry-safe failure. Dropping this future also settles `Interrupted`.
    pub async fn call_provider_bound(
        &self,
        call: BoundModuleCall,
        providers: &ProviderManager,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<ChatResponse> {
        self.call_provider_bound_within(
            call,
            providers,
            provider_id,
            model_id,
            request,
            self.provider_deadline,
        )
        .await
    }

    pub async fn call_provider_bound_within(
        &self,
        call: BoundModuleCall,
        providers: &ProviderManager,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        deadline: Duration,
    ) -> DgosResult<ChatResponse> {
        let grant = match self
            .capabilities
            .authorize(call.context(), ModulePermission::ProvidersCall)
            .await
        {
            Ok(grant) => grant,
            Err(error) => {
                call.finish(ModuleRequestOutcome::Denied);
                return Err(error);
            }
        };
        let user_id = call.context().user_id().to_owned();
        let provenance = ExecutionProvenance {
            caller: BuiltinCaller::Module,
            request_id: grant.request_id,
        };
        let external = providers.call_provider_bound_with_provenance(
            provider_id,
            model_id,
            request,
            &user_id,
            None,
            Some(&provenance),
        );
        match tokio::time::timeout(deadline, external).await {
            Ok(result) => {
                call.finish(if result.is_ok() {
                    ModuleRequestOutcome::Succeeded
                } else {
                    ModuleRequestOutcome::Failed
                });
                result
            }
            Err(_) => {
                call.finish(ModuleRequestOutcome::Interrupted);
                Err(DgosError::Timeout)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleReleaseResult {
    pub operation_id: String,
    pub module_id: String,
    pub version: String,
    pub integrity: String,
}

/// Application-level release use case. File staging, package validation and
/// pointer/DB recovery remain owned by ModulePublisher; this service owns the
/// use-case boundary and keeps transports from composing those steps.
#[derive(Clone)]
pub struct ModuleReleaseService {
    publisher: Arc<ModulePublisher>,
}

impl ModuleReleaseService {
    pub fn new(publisher: Arc<ModulePublisher>) -> Self {
        Self { publisher }
    }

    pub async fn publish(
        &self,
        source: &Path,
    ) -> Result<ModuleReleaseResult, dgos_modules::ModulePackageError> {
        self.publish_with_operation_id(source, uuid::Uuid::new_v4().to_string())
            .await
    }

    pub async fn publish_with_operation_id(
        &self,
        source: &Path,
        operation_id: impl Into<String>,
    ) -> Result<ModuleReleaseResult, dgos_modules::ModulePackageError> {
        let staged = self
            .publisher
            .stage_with_operation_id(source, operation_id)?;
        let result = ModuleReleaseResult {
            operation_id: staged.operation_id().to_owned(),
            module_id: staged.module_id().to_owned(),
            version: staged.version().to_owned(),
            integrity: staged.integrity().to_owned(),
        };
        self.publisher.commit(staged).await?;
        Ok(result)
    }
}

impl ProviderCallService {
    pub fn new(capabilities: HostCapabilityService) -> Self {
        Self { capabilities }
    }

    pub async fn execute(
        &self,
        context: &HostExecutionContext,
        providers: &ProviderManager,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<ChatResponse> {
        self.capabilities
            .call_provider(context, providers, provider_id, model_id, request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::sea_orm_migration::MigratorTrait;
    use dgos_db::{
        entities::modules,
        migrations::Migrator,
        sea_orm::{ActiveModelTrait, Database, Set},
    };
    use dgos_modules::ModulePublisher;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn module_dispatch_requires_manifest_and_persisted_grant() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        modules::ActiveModel {
            id: Set("module-1".into()),
            module_name: Set("example.module".into()),
            display_name: Set("Example".into()),
            current_version: Set("1.0.0".into()),
            description: Set(None),
            manifest: Set(serde_json::json!({"permissions":["providers.call"]})),
            integrity: Set("a".repeat(64)),
            entry_url: Set(None),
            icon_url: Set(None),
            is_official: Set(false),
            is_enabled: Set(true),
            installed_at: Set(chrono::Utc::now().timestamp()),
            updated_at: Set(chrono::Utc::now().timestamp()),
        }
        .insert(&db)
        .await
        .unwrap();
        let repository = Arc::new(ModulePermissionRepository::new(db));
        let service = HostCapabilityService::new(repository.clone());
        let context = HostExecutionContext::for_module_session(
            &dgos_types::HostSession::local_default(),
            "module-1",
            "1.0.0",
            &"a".repeat(64),
        );
        assert!(service
            .authorize(&context, ModulePermission::ProvidersCall)
            .await
            .is_err());
        repository
            .grant("default-user", "module-1", ModulePermission::ProvidersCall)
            .await
            .unwrap();
        assert!(service
            .authorize(&context, ModulePermission::ProvidersCall)
            .await
            .is_ok());

        repository
            .revoke("default-user", "module-1", ModulePermission::ProvidersCall)
            .await
            .unwrap();
        let operation_started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let operation_started_for_dispatch = operation_started.clone();
        assert!(service
            .dispatch(
                &context,
                ModulePermission::ProvidersCall,
                move |_| async move {
                    operation_started_for_dispatch.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok::<_, DgosError>(())
                }
            )
            .await
            .is_err());
        assert!(!operation_started.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn release_service_composes_stage_commit_and_returns_release_identity() {
        let source = tempdir().unwrap();
        fs::write(source.path().join("entry.js"), "export default {};").unwrap();
        // The digest excludes `artifact.sha256`, so write a placeholder first.
        let manifest = |sha256: &str| {
            serde_json::json!({
                "id":"module-1", "name":"Example", "version":"1.0.0",
                "entry":"entry.js", "artifact":{"version":"1.0.0", "sha256":sha256},
                "permissions":[]
            })
            .to_string()
        };
        fs::write(
            source.path().join("manifest.json"),
            manifest(&"0".repeat(64)),
        )
        .unwrap();
        let integrity = dgos_modules::package_integrity(source.path()).unwrap();
        let hex = integrity.strip_prefix("v2:").unwrap();
        fs::write(source.path().join("manifest.json"), manifest(hex)).unwrap();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = Arc::new(ModulePublisher::new(store.path(), db));
        let service = ModuleReleaseService::new(publisher.clone());
        let result = service.publish(source.path()).await.unwrap();
        assert_eq!(result.module_id, "module-1");
        assert_eq!(result.version, "1.0.0");
        assert_eq!(
            publisher.recover("module-1").await.unwrap().as_deref(),
            Some("1.0.0")
        );
    }

    mod runtime_dispatch {
        use super::*;
        use dgos_modules::{
            LocalArtifactLoader, ModuleRuntimeState, ModuleRuntimeSupervisor, ModuleUpdateOutcome,
        };
        use dgos_types::HostSession;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tempfile::TempDir;

        const MODULE: &str = "module-1";

        pub(super) fn package(version: &str) -> TempDir {
            let dir = tempdir().unwrap();
            fs::write(
                dir.path().join("entry.js"),
                format!("export default '{version}';"),
            )
            .unwrap();
            // The digest excludes `artifact.sha256`, so write a placeholder first.
            let manifest = |sha256: &str| {
                serde_json::json!({
                    "id": MODULE, "name": "Example", "version": version, "entry": "entry.js",
                    "artifact": {"version": version, "sha256": sha256},
                    "permissions": ["providers.call"]
                })
                .to_string()
            };
            fs::write(dir.path().join("manifest.json"), manifest(&"0".repeat(64))).unwrap();
            let integrity = dgos_modules::package_integrity(dir.path()).unwrap();
            let hex = integrity.strip_prefix("v2:").unwrap();
            fs::write(dir.path().join("manifest.json"), manifest(hex)).unwrap();
            dir
        }

        struct Fixture {
            _store: TempDir,
            permissions: Arc<ModulePermissionRepository>,
            dispatch: ModuleCapabilityDispatchService,
            supervisor: Arc<ModuleRuntimeSupervisor>,
        }

        impl Fixture {
            async fn new() -> Self {
                let db = Database::connect("sqlite::memory:").await.unwrap();
                Migrator::up(&db, None).await.unwrap();
                let store = tempdir().unwrap();
                let publisher = Arc::new(ModulePublisher::new(store.path(), db.clone()));
                ModuleReleaseService::new(publisher.clone())
                    .publish(package("1.0.0").path())
                    .await
                    .unwrap();
                let permissions = Arc::new(ModulePermissionRepository::new(db));
                permissions
                    .grant("default-user", MODULE, ModulePermission::ProvidersCall)
                    .await
                    .unwrap();
                let dispatch = ModuleCapabilityDispatchService::new(
                    HostCapabilityService::with_publisher(permissions.clone(), publisher.clone()),
                );
                let supervisor = Arc::new(ModuleRuntimeSupervisor::new(
                    publisher,
                    Arc::new(LocalArtifactLoader),
                ));
                Self {
                    _store: store,
                    permissions,
                    dispatch,
                    supervisor,
                }
            }

            async fn start(&self) -> ModuleRuntimeHandle {
                self.supervisor
                    .start(&HostSession::local_default(), MODULE)
                    .await
                    .unwrap()
            }
        }

        async fn wait_until(mut condition: impl FnMut() -> bool) {
            for _ in 0..5000 {
                if condition() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            panic!("condition not reached");
        }

        #[tokio::test]
        async fn every_dispatch_settles_once_with_fresh_request_ids() {
            let fixture = Fixture::new().await;
            let handle = fixture.start().await;
            let runs = Arc::new(AtomicUsize::new(0));
            let dispatch = |permission, fail: bool| {
                let runs = runs.clone();
                fixture
                    .dispatch
                    .dispatch(&handle, permission, move |grant| async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        if fail {
                            Err(DgosError::InternalError("adapter failed".into()))
                        } else {
                            Ok(grant.request_id)
                        }
                    })
            };

            let first = dispatch(ModulePermission::ProvidersCall, false)
                .await
                .unwrap();
            let second = dispatch(ModulePermission::ProvidersCall, false)
                .await
                .unwrap();
            assert_ne!(first, second);
            assert_ne!(first, handle.context().request_id());
            assert!(dispatch(ModulePermission::ProvidersCall, true)
                .await
                .is_err());
            // Undeclared capability and revoked grant: the operation never runs.
            assert!(matches!(
                dispatch(ModulePermission::McpCall, false).await,
                Err(DgosError::PermissionDenied)
            ));
            fixture
                .permissions
                .revoke("default-user", MODULE, ModulePermission::ProvidersCall)
                .await
                .unwrap();
            assert!(matches!(
                dispatch(ModulePermission::ProvidersCall, false).await,
                Err(DgosError::PermissionDenied)
            ));
            assert_eq!(runs.load(Ordering::SeqCst), 3);
            let stats = handle.request_stats();
            assert_eq!(
                (
                    stats.admitted,
                    stats.succeeded,
                    stats.failed,
                    stats.denied,
                    stats.in_flight
                ),
                (5, 2, 1, 2, 0)
            );

            fixture.supervisor.stop(MODULE).await.unwrap();
            assert!(matches!(
                dispatch(ModulePermission::ProvidersCall, false).await,
                Err(DgosError::PermissionDenied)
            ));
            assert_eq!(handle.request_stats().rejected, 1);
            assert_eq!(runs.load(Ordering::SeqCst), 3);
        }

        #[tokio::test]
        async fn draining_refuses_new_dispatch_and_in_flight_request_finishes_once() {
            let fixture = Fixture::new().await;
            let handle = fixture.start().await;
            let runs = Arc::new(AtomicUsize::new(0));
            let (release, released) = tokio::sync::oneshot::channel::<()>();
            let in_flight = tokio::spawn({
                let (dispatch, handle, runs) =
                    (fixture.dispatch.clone(), handle.clone(), runs.clone());
                async move {
                    dispatch
                        .dispatch(
                            &handle,
                            ModulePermission::ProvidersCall,
                            move |_| async move {
                                runs.fetch_add(1, Ordering::SeqCst);
                                released.await.unwrap();
                                Ok("done")
                            },
                        )
                        .await
                }
            });
            wait_until(|| runs.load(Ordering::SeqCst) == 1).await;

            let stopping = tokio::spawn({
                let supervisor = fixture.supervisor.clone();
                async move { supervisor.stop(MODULE).await }
            });
            wait_until(|| handle.state() == ModuleRuntimeState::Draining).await;
            let refused_ran = Arc::new(AtomicUsize::new(0));
            let refused = {
                let refused_ran = refused_ran.clone();
                fixture
                    .dispatch
                    .dispatch(
                        &handle,
                        ModulePermission::ProvidersCall,
                        move |_| async move {
                            refused_ran.fetch_add(1, Ordering::SeqCst);
                            Ok("late")
                        },
                    )
                    .await
            };
            assert!(matches!(refused, Err(DgosError::PermissionDenied)));
            assert_eq!(refused_ran.load(Ordering::SeqCst), 0);
            assert_eq!(handle.state(), ModuleRuntimeState::Draining);

            release.send(()).unwrap();
            assert_eq!(in_flight.await.unwrap().unwrap(), "done");
            stopping.await.unwrap().unwrap();
            assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
            assert_eq!(runs.load(Ordering::SeqCst), 1);
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
        async fn hot_update_keeps_in_flight_request_on_previous_runtime_without_replay() {
            let fixture = Fixture::new().await;
            let previous = fixture.start().await;
            let runs = Arc::new(AtomicUsize::new(0));
            let (release, released) = tokio::sync::oneshot::channel::<()>();
            let in_flight = tokio::spawn({
                let (dispatch, handle, runs) =
                    (fixture.dispatch.clone(), previous.clone(), runs.clone());
                async move {
                    dispatch
                        .dispatch(
                            &handle,
                            ModulePermission::ProvidersCall,
                            move |grant| async move {
                                runs.fetch_add(1, Ordering::SeqCst);
                                released.await.unwrap();
                                Ok(grant.version)
                            },
                        )
                        .await
                }
            });
            wait_until(|| runs.load(Ordering::SeqCst) == 1).await;

            let source = package("2.0.0");
            let updating = tokio::spawn({
                let supervisor = fixture.supervisor.clone();
                let path = source.path().to_owned();
                async move {
                    supervisor
                        .update(&HostSession::local_default(), MODULE, &path)
                        .await
                }
            });
            wait_until(|| previous.state() == ModuleRuntimeState::Draining).await;
            let current = fixture.supervisor.runtime(MODULE).unwrap();
            assert!(!current.is_same_runtime(&previous));
            let served_by = fixture
                .dispatch
                .dispatch(
                    &current,
                    ModulePermission::ProvidersCall,
                    |grant| async move { Ok(grant.version) },
                )
                .await
                .unwrap();
            assert_eq!(served_by, "2.0.0");
            assert!(matches!(
                fixture
                    .dispatch
                    .dispatch(&previous, ModulePermission::ProvidersCall, |_| async {
                        Ok(String::new())
                    })
                    .await,
                Err(DgosError::PermissionDenied)
            ));

            release.send(()).unwrap();
            assert_eq!(in_flight.await.unwrap().unwrap(), "1.0.0");
            let report = updating.await.unwrap().unwrap();
            assert_eq!(
                report.outcome,
                ModuleUpdateOutcome::Switched {
                    previous_stopped: true
                }
            );
            assert_eq!(previous.state(), ModuleRuntimeState::Stopped);
            // The in-flight operation ran exactly once, on the old runtime.
            assert_eq!(runs.load(Ordering::SeqCst), 1);
            assert_eq!(previous.request_stats().succeeded, 1);
            assert_eq!(current.request_stats().admitted, 1);
        }

        #[tokio::test]
        async fn dropped_dispatch_settles_as_interrupted_and_is_not_replayed() {
            let fixture = Fixture::new().await;
            let handle = fixture.start().await;
            let runs = Arc::new(AtomicUsize::new(0));
            let (started, operation_started) = tokio::sync::oneshot::channel::<()>();
            let mut pending = Box::pin({
                let runs = runs.clone();
                fixture.dispatch.dispatch(
                    &handle,
                    ModulePermission::ProvidersCall,
                    move |_| async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        let _ = started.send(());
                        std::future::pending::<DgosResult<()>>().await
                    },
                )
            });
            tokio::select! {
                _ = &mut pending => panic!("operation never completes"),
                started = operation_started => started.unwrap(),
            }
            drop(pending);

            let stats = handle.request_stats();
            assert_eq!(
                (stats.admitted, stats.interrupted, stats.in_flight),
                (1, 1, 0)
            );
            fixture.supervisor.stop(MODULE).await.unwrap();
            assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
            assert_eq!(runs.load(Ordering::SeqCst), 1);
        }
    }

    /// D048 vertical slice. A runtime admitted by a pinned list and really
    /// loaded through `PageModuleLoader` calls `providers.call` through
    /// `ModuleCapabilityDispatchService` against a real `ProviderManager`,
    /// SQLite and an HTTP fixture. Only the WebView page is scripted
    /// (receipts are answered by a task); no real module JS runs here.
    mod trusted_runtime_call {
        use super::*;
        use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
        use dgos_modules::{
            ModulePageTransport, ModuleRuntimeSupervisor, ModuleSupervisorError, PageInstruction,
            PageLoaderConfig, PageModuleLoader, PageReceipt, PageSendError, PinnedModuleAdmission,
            RuntimeLost,
        };
        use dgos_providers::test_support::{seed_provider, MemoryCredentialVault};
        use dgos_types::{ChatMessage, HostSession, ProviderConfig};
        use tempfile::TempDir;
        use tokio::sync::mpsc;

        const MODULE: &str = "module-1";
        const SECRET_PROMPT: &str = "SECRET-PROMPT-7f3a";
        const SYNTHETIC_KEY: &str = "synthetic-module-key-91c2";
        const SECRET_REPLY: &str = "SECRET-REPLY-2b9d";

        fn reply() -> String {
            format!(
                r#"{{"choices":[{{"message":{{"content":"{SECRET_REPLY}"}}}}],"usage":{{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}}}"#
            )
        }

        fn chat() -> ChatRequest {
            ChatRequest {
                messages: vec![ChatMessage {
                    role: "user".into(),
                    content: SECRET_PROMPT.into(),
                }],
                temperature: None,
                max_tokens: Some(32),
                stream: Some(false),
            }
        }

        struct ScriptedPage(mpsc::UnboundedSender<(String, PageInstruction)>);

        impl ModulePageTransport for ScriptedPage {
            fn send(
                &self,
                attachment_id: &str,
                instruction: PageInstruction,
            ) -> Result<(), PageSendError> {
                self.0
                    .send((attachment_id.to_owned(), instruction))
                    .map_err(|_| PageSendError)
            }
        }

        enum Pin {
            Exact,
            WrongIntegrity,
            WrongVersion,
            Builtin,
        }

        struct World {
            _db_dir: TempDir,
            _store: TempDir,
            db: DatabaseConnection,
            server: mockito::ServerGuard,
            providers: ProviderManager,
            provider_id: String,
            permissions: Arc<ModulePermissionRepository>,
            dispatch: ModuleCapabilityDispatchService,
            supervisor: ModuleRuntimeSupervisor,
            router: dgos_modules::ModuleCallRouter,
            attachment_id: String,
            integrity: String,
            _lost: mpsc::UnboundedReceiver<RuntimeLost>,
            driver: tokio::task::JoinHandle<()>,
        }

        impl Drop for World {
            fn drop(&mut self) {
                self.driver.abort();
            }
        }

        impl World {
            async fn new(pin: Pin) -> Self {
                let db_dir = tempdir().unwrap();
                let url = format!(
                    "sqlite://{}?mode=rwc",
                    db_dir.path().join("fixture.db").display()
                );
                let db = dgos_db::establish_connection(&url).await.unwrap();
                Migrator::up(&db, None).await.unwrap();
                let store = tempdir().unwrap();
                let publisher = Arc::new(ModulePublisher::new(store.path(), db.clone()));
                let release = ModuleReleaseService::new(publisher.clone())
                    .publish(super::runtime_dispatch::package("1.0.0").path())
                    .await
                    .unwrap();

                let server = mockito::Server::new_async().await;
                let vault = Arc::new(MemoryCredentialVault::default());
                let provider = seed_provider(
                    db.clone(),
                    &vault,
                    ProviderConfig {
                        name: "Synthetic fixture".into(),
                        provider_type: "openai_compatible".into(),
                        base_url: server.url(),
                        api_key: SYNTHETIC_KEY.into(),
                        enabled: true,
                        config: None,
                    },
                    "default-user",
                )
                .await
                .unwrap();
                let providers = ProviderManager::with_vault(&url, vault).await.unwrap();

                let wrong_integrity = "0".repeat(64);
                let admission = match pin {
                    Pin::Exact => {
                        PinnedModuleAdmission::new([(MODULE, "1.0.0", release.integrity.as_str())])
                    }
                    Pin::WrongIntegrity => {
                        PinnedModuleAdmission::new([(MODULE, "1.0.0", wrong_integrity.as_str())])
                    }
                    Pin::WrongVersion => {
                        PinnedModuleAdmission::new([(MODULE, "2.0.0", release.integrity.as_str())])
                    }
                    Pin::Builtin => PinnedModuleAdmission::builtin(),
                };

                let (sender, receiver) = mpsc::unbounded_channel();
                let (loader, lost) = PageModuleLoader::new(
                    Arc::new(ScriptedPage(sender)),
                    PageLoaderConfig::default(),
                );
                let attachment_id = loader.attach("main").attachment_id;
                let router = dgos_modules::ModuleCallRouter::new(loader.clone());
                let driver = answer_page(loader.clone(), receiver);

                let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
                let dispatch = ModuleCapabilityDispatchService::new(
                    HostCapabilityService::with_publisher(permissions.clone(), publisher.clone()),
                );
                let supervisor = ModuleRuntimeSupervisor::new(publisher, Arc::new(loader))
                    .with_admission(Arc::new(admission));
                Self {
                    _db_dir: db_dir,
                    _store: store,
                    db,
                    server,
                    providers,
                    provider_id: provider.id,
                    permissions,
                    dispatch,
                    supervisor,
                    router,
                    attachment_id,
                    integrity: release.integrity,
                    _lost: lost,
                    driver,
                }
            }

            async fn start(&self) -> Result<ModuleRuntimeHandle, ModuleSupervisorError> {
                self.supervisor
                    .start(&HostSession::local_default(), MODULE)
                    .await
            }

            async fn grant(&self) {
                self.permissions
                    .grant("default-user", MODULE, ModulePermission::ProvidersCall)
                    .await
                    .unwrap();
            }

            async fn revoke(&self) {
                self.permissions
                    .revoke("default-user", MODULE, ModulePermission::ProvidersCall)
                    .await
                    .unwrap();
            }

            fn chat_mock(&mut self, status: usize, body: &str) -> mockito::Mock {
                self.server
                    .mock("POST", "/chat/completions")
                    .match_header("authorization", format!("Bearer {SYNTHETIC_KEY}").as_str())
                    .with_status(status)
                    .with_body(body)
            }

            async fn call(&self, handle: &ModuleRuntimeHandle) -> DgosResult<ChatResponse> {
                self.dispatch
                    .call_provider(
                        handle,
                        &self.providers,
                        &self.provider_id,
                        "fixture-chat",
                        chat(),
                    )
                    .await
            }

            /// Every audit column of every permission audit row, joined.
            async fn audit_dump(&self) -> String {
                self.db
                    .query_all(Statement::from_string(
                        self.db.get_database_backend(),
                        "SELECT user_id, module_id, permission, action, resource_id, request_id FROM module_permission_audits",
                    ))
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| {
                        let text = |name: &str| row.try_get::<Option<String>>("", name).unwrap();
                        format!(
                            "{:?}",
                            [
                                text("user_id"),
                                text("module_id"),
                                text("permission"),
                                text("action"),
                                text("resource_id"),
                                text("request_id"),
                            ]
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }

            /// `(request_id)` of audit rows with the given action.
            async fn audited_request_ids(&self, action: &str) -> Vec<String> {
                self.db
                    .query_all(Statement::from_string(
                        self.db.get_database_backend(),
                        format!(
                            "SELECT request_id FROM module_permission_audits WHERE action = '{action}' AND module_id = '{MODULE}'"
                        ),
                    ))
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| {
                        row.try_get::<Option<String>>("", "request_id")
                            .unwrap()
                            .expect("denials carry the host-generated request id")
                    })
                    .collect()
            }
        }

        /// The scripted page: acknowledges every instruction like a healthy
        /// WebView would, through the same `receive` path the real commands use.
        fn answer_page(
            loader: PageModuleLoader,
            mut instructions: mpsc::UnboundedReceiver<(String, PageInstruction)>,
        ) -> tokio::task::JoinHandle<()> {
            tokio::spawn(async move {
                while let Some((attachment, instruction)) = instructions.recv().await {
                    let (ticket, receipt) = match instruction {
                        PageInstruction::Load { ticket, .. } => (ticket, PageReceipt::Loaded),
                        PageInstruction::Unload { ticket, .. } => {
                            (ticket, PageReceipt::Unloaded { hook_failed: false })
                        }
                    };
                    loader.receive(&attachment, &ticket, receipt);
                }
            })
        }

        fn assert_redacted(label: &str, text: &str) {
            for secret in [SECRET_PROMPT, SYNTHETIC_KEY, SECRET_REPLY] {
                assert!(!text.contains(secret), "{label} leaked {secret}");
            }
        }

        #[tokio::test]
        async fn admitted_runtime_calls_a_provider_under_per_call_authorization() {
            let mut world = World::new(Pin::Exact).await;
            let handle = world.start().await.unwrap();
            assert_eq!(
                handle.module_identity(),
                Some((MODULE, "1.0.0", world.integrity.as_str()))
            );

            // Admitted but not authorized: the provider is never reached.
            let unreached = world
                .chat_mock(200, &reply())
                .expect(0)
                .create_async()
                .await;
            assert!(matches!(
                world.call(&handle).await,
                Err(DgosError::PermissionDenied)
            ));
            unreached.assert_async().await;
            drop(unreached);

            // Two identical sequential calls plus eight concurrent ones are
            // ten independent provider requests: no dedupe, no replay.
            world.grant().await;
            let served = world
                .chat_mock(200, &reply())
                .expect(10)
                .create_async()
                .await;
            for _ in 0..2 {
                assert_eq!(world.call(&handle).await.unwrap().content, SECRET_REPLY);
            }
            let concurrent = futures::future::join_all((0..8).map(|_| world.call(&handle))).await;
            assert!(concurrent.iter().all(|result| result.is_ok()));

            // Revocation takes effect on the next call, before the provider.
            world.revoke().await;
            assert!(matches!(
                world.call(&handle).await,
                Err(DgosError::PermissionDenied)
            ));
            served.assert_async().await;

            let stats = handle.request_stats();
            assert_eq!(
                (
                    stats.admitted,
                    stats.succeeded,
                    stats.failed,
                    stats.denied,
                    stats.in_flight
                ),
                (12, 10, 0, 2, 0)
            );

            // Usage rows are attributed to the module with the per-call
            // request ids the host generated, all distinct.
            let usage = world
                .providers
                .list_usage_logs("default-user", None)
                .await
                .unwrap();
            assert_eq!(usage.len(), 10);
            let used: std::collections::HashSet<_> = usage
                .iter()
                .map(|log| log.request_id.clone().expect("request id"))
                .collect();
            assert_eq!(used.len(), 10);
            assert!(!used.contains(handle.context().request_id()));
            assert!(usage
                .iter()
                .all(|log| log.caller.as_deref() == Some("module") && log.success));

            // Both denials are audited under their own request ids, never
            // reused by a served call.
            let denied = world.audited_request_ids("denied").await;
            assert_eq!(denied.len(), 2);
            assert_eq!(
                denied
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                2
            );
            assert!(denied.iter().all(|id| !used.contains(id)));

            assert_redacted("usage logs", &format!("{usage:?}"));
            assert_redacted("permission audits", &world.audit_dump().await);
        }

        #[tokio::test]
        async fn a_package_outside_the_pinned_list_never_gets_a_runtime() {
            for pin in [Pin::Builtin, Pin::WrongIntegrity, Pin::WrongVersion] {
                let mut world = World::new(pin).await;
                // A persisted grant is not admission (D041).
                world.grant().await;
                let unreached = world
                    .chat_mock(200, &reply())
                    .expect(0)
                    .create_async()
                    .await;
                assert!(matches!(
                    world.start().await,
                    Err(ModuleSupervisorError::NotAdmitted)
                ));
                unreached.assert_async().await;
            }
        }

        #[tokio::test]
        async fn a_stopped_runtime_handle_is_dead_and_a_restart_gets_a_new_identity() {
            let mut world = World::new(Pin::Exact).await;
            world.grant().await;
            let served = world
                .chat_mock(200, &reply())
                .expect(2)
                .create_async()
                .await;

            let first = world.start().await.unwrap();
            world.call(&first).await.unwrap();
            world.supervisor.stop(MODULE).await.unwrap();
            assert!(matches!(
                world.call(&first).await,
                Err(DgosError::PermissionDenied)
            ));
            assert_eq!(first.request_stats().rejected, 1);

            let second = world.start().await.unwrap();
            assert_ne!(first.runtime_id(), second.runtime_id());
            world.call(&second).await.unwrap();
            assert!(matches!(
                world.call(&first).await,
                Err(DgosError::PermissionDenied)
            ));
            served.assert_async().await;
        }

        #[tokio::test]
        async fn a_provider_failure_is_a_failed_call_not_a_permission_error() {
            let mut world = World::new(Pin::Exact).await;
            world.grant().await;
            let handle = world.start().await.unwrap();
            let rejected = world
                .chat_mock(401, r#"{"error":{"message":"rejected"}}"#)
                .expect_at_least(1)
                .create_async()
                .await;
            let error = world.call(&handle).await.unwrap_err();
            rejected.assert_async().await;
            assert!(!matches!(error, DgosError::PermissionDenied));
            assert_redacted("error", &format!("{error} {error:?}"));

            let stats = handle.request_stats();
            assert_eq!((stats.failed, stats.denied, stats.in_flight), (1, 0, 0));
            let usage = world
                .providers
                .list_usage_logs("default-user", None)
                .await
                .unwrap();
            assert_eq!(usage.len(), 1);
            assert!(!usage[0].success);
            assert_eq!(usage[0].caller.as_deref(), Some("module"));
            assert_redacted("usage logs", &format!("{usage:?}"));
        }

        impl World {
            /// What the channel does for a page call: issue the route for a ready
            /// runtime, then admit one call from the page's attachment.
            fn admit(&self, handle: &ModuleRuntimeHandle) -> dgos_modules::BoundModuleCall {
                let route = self.router.issue(handle).unwrap();
                self.router
                    .admit(route.as_str(), &self.attachment_id)
                    .unwrap()
            }
        }

        #[tokio::test]
        async fn a_channel_admitted_call_is_authorized_every_time_and_settles_once() {
            let mut world = World::new(Pin::Exact).await;
            let handle = world.start().await.unwrap();

            // Not granted: refused before the Provider, and counted as denied.
            let never = world
                .chat_mock(200, &reply())
                .expect(0)
                .create_async()
                .await;
            let denied = world
                .dispatch
                .call_provider_bound(
                    world.admit(&handle),
                    &world.providers,
                    &world.provider_id,
                    "fixture-chat",
                    chat(),
                )
                .await;
            assert!(matches!(denied, Err(DgosError::PermissionDenied)));
            never.assert_async().await;
            never.remove_async().await;

            world.grant().await;
            let served = world
                .chat_mock(200, &reply())
                .expect(1)
                .create_async()
                .await;
            let response = world
                .dispatch
                .call_provider_bound(
                    world.admit(&handle),
                    &world.providers,
                    &world.provider_id,
                    "fixture-chat",
                    chat(),
                )
                .await
                .unwrap();
            assert_eq!(response.content, SECRET_REPLY);
            served.assert_async().await;
            served.remove_async().await;

            // Revocation applies to the next call, which never reaches the Provider.
            world.revoke().await;
            let after = world
                .chat_mock(200, &reply())
                .expect(0)
                .create_async()
                .await;
            let revoked = world
                .dispatch
                .call_provider_bound(
                    world.admit(&handle),
                    &world.providers,
                    &world.provider_id,
                    "fixture-chat",
                    chat(),
                )
                .await;
            assert!(matches!(revoked, Err(DgosError::PermissionDenied)));
            after.assert_async().await;

            let stats = handle.request_stats();
            assert_eq!(
                (
                    stats.admitted,
                    stats.succeeded,
                    stats.denied,
                    stats.in_flight
                ),
                (3, 1, 2, 0)
            );
            let usage = world
                .providers
                .list_usage_logs("default-user", None)
                .await
                .unwrap();
            assert_eq!(usage.len(), 1);
            assert_eq!(usage[0].caller.as_deref(), Some("module"));
            assert_redacted("usage logs", &format!("{usage:?}"));
            assert_redacted("audit", &world.audit_dump().await);
        }

        #[tokio::test]
        async fn the_host_deadline_settles_an_unfinished_call_as_interrupted_not_failed() {
            let mut world = World::new(Pin::Exact).await;
            world.grant().await;
            let handle = world.start().await.unwrap();
            let body = reply();
            let _slow = world
                .chat_mock(200, "")
                .with_chunked_body(move |writer| {
                    std::thread::sleep(Duration::from_millis(600));
                    std::io::Write::write_all(writer, body.as_bytes())
                })
                .create_async()
                .await;

            let result = world
                .dispatch
                .call_provider_bound_within(
                    world.admit(&handle),
                    &world.providers,
                    &world.provider_id,
                    "fixture-chat",
                    chat(),
                    Duration::from_millis(100),
                )
                .await;
            // Unknown external result: a timeout error, never a retry-safe failure.
            assert!(matches!(result, Err(DgosError::Timeout)));
            let stats = handle.request_stats();
            assert_eq!(
                (
                    stats.interrupted,
                    stats.failed,
                    stats.succeeded,
                    stats.in_flight
                ),
                (1, 0, 0, 0)
            );
            assert_redacted("error", &format!("{result:?}"));
        }
    }
}
