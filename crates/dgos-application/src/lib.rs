//! Transport-independent application services.
//!
//! Commands, HTTP handlers and a future module runtime should call these
//! use-case services rather than composing capability checks and domain
//! managers independently. The services bind trusted execution context and
//! leave resource ownership and provider invariants to their domain managers.

use dgos_db::{ModulePermission, ModulePermissionRepository};
use dgos_modules::ModulePublisher;
use dgos_providers::ProviderManager;
use dgos_types::{
    ChatRequest, ChatResponse, DgosError, DgosResult, HostExecutionContext, HostSession,
};
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

    pub async fn recover_registered(
        &self,
    ) -> Result<Vec<String>, dgos_modules::ModulePackageError> {
        self.publisher.recover_registered().await
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
            &HostSession::local_default(),
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
}
