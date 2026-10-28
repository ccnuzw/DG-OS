//! Transport-independent application services.
//!
//! Commands, HTTP handlers and a future module runtime should call these
//! use-case services rather than composing capability checks and domain
//! managers independently. The services bind trusted execution context and
//! leave resource ownership and provider invariants to their domain managers.

mod module_operations;

pub use module_operations::ModuleOperationManager;

use dgos_db::{ModulePermission, ModulePermissionRepository};
use dgos_modules::{ModulePublisher, ModuleRequestOutcome, ModuleRuntimeHandle};
use dgos_providers::ProviderManager;
use dgos_types::{ChatRequest, ChatResponse, DgosError, DgosResult, HostExecutionContext};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

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
}

impl ModuleCapabilityDispatchService {
    pub fn new(capabilities: HostCapabilityService) -> Self {
        Self { capabilities }
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
        let digest = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(b"entry.js");
            hasher.update([0]);
            hasher.update(b"export default {};");
            format!("{:x}", hasher.finalize())
        };
        fs::write(
            source.path().join("manifest.json"),
            serde_json::json!({
                "id":"module-1", "name":"Example", "version":"1.0.0",
                "entry":"entry.js", "artifact":{"version":"1.0.0", "sha256":digest},
                "permissions":[]
            })
            .to_string(),
        )
        .unwrap();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let store = tempdir().unwrap();
        let publisher = Arc::new(ModulePublisher::new(store.path(), db));
        let service = ModuleReleaseService::new(publisher.clone());
        let result = service.publish(source.path()).await.unwrap();
        assert_eq!(result.module_id, "module-1");
        assert_eq!(result.version, "1.0.0");
        assert_eq!(
            publisher.recover("module-1").unwrap().as_deref(),
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

        fn package(version: &str) -> TempDir {
            use sha2::{Digest, Sha256};
            let dir = tempdir().unwrap();
            let body = format!("export default '{version}';");
            fs::write(dir.path().join("entry.js"), &body).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(b"entry.js");
            hasher.update([0]);
            hasher.update(body.as_bytes());
            fs::write(
                dir.path().join("manifest.json"),
                serde_json::json!({
                    "id": MODULE, "name": "Example", "version": version, "entry": "entry.js",
                    "artifact": {"version": version, "sha256": format!("{:x}", hasher.finalize())},
                    "permissions": ["providers.call"]
                })
                .to_string(),
            )
            .unwrap();
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
}
