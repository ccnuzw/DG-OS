use crate::entities::{module_permission_audits, module_permissions, modules};
use dgos_types::{DgosError, DgosResult};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use uuid::Uuid;

/// The only permission values accepted by the V1 module broker (D006).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModulePermission {
    ProvidersCall,
    SkillsExecute,
    McpCall,
    StorageRead,
    StorageWrite,
    NetworkLocal,
}

impl ModulePermission {
    pub const ALL: [Self; 6] = [
        Self::ProvidersCall,
        Self::SkillsExecute,
        Self::McpCall,
        Self::StorageRead,
        Self::StorageWrite,
        Self::NetworkLocal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProvidersCall => "providers.call",
            Self::SkillsExecute => "skills.execute",
            Self::McpCall => "mcp.call",
            Self::StorageRead => "storage.read",
            Self::StorageWrite => "storage.write",
            Self::NetworkLocal => "network.local",
        }
    }

    pub fn parse(value: &str) -> DgosResult<Self> {
        Self::ALL
            .into_iter()
            .find(|permission| permission.as_str() == value)
            .ok_or_else(|| DgosError::InternalError("invalid module permission".into()))
    }
}

pub trait ModulePermissionInput {
    fn resolve(self) -> DgosResult<ModulePermission>;
}

impl ModulePermissionInput for ModulePermission {
    fn resolve(self) -> DgosResult<ModulePermission> {
        Ok(self)
    }
}

impl ModulePermissionInput for &str {
    fn resolve(self) -> DgosResult<ModulePermission> {
        ModulePermission::parse(self)
    }
}

impl ModulePermissionInput for String {
    fn resolve(self) -> DgosResult<ModulePermission> {
        ModulePermission::parse(&self)
    }
}

pub struct ModulePermissionRepository {
    db: DatabaseConnection,
}

impl ModulePermissionRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Register a module row for the broker. This stores only a validated
    /// manifest summary; it does not install or execute a module package.
    #[cfg(test)]
    pub(crate) async fn register_module(
        &self,
        id: impl Into<String>,
        module_name: impl Into<String>,
        display_name: impl Into<String>,
        version: impl Into<String>,
        manifest: serde_json::Value,
        integrity: impl Into<String>,
        is_enabled: bool,
    ) -> DgosResult<()> {
        let id = id.into();
        let version = version.into();
        let integrity = integrity.into();
        validate_module_registration(&id, &version, &integrity, &manifest)?;
        // The broker owns persistence only. A trusted publisher must still be
        // the caller that supplies the package-backed identity before this row
        // can be used by Host Capability API.
        let now = chrono::Utc::now().timestamp();
        modules::ActiveModel {
            id: Set(id),
            module_name: Set(module_name.into()),
            display_name: Set(display_name.into()),
            current_version: Set(version),
            description: Set(None),
            manifest: Set(manifest),
            integrity: Set(integrity),
            entry_url: Set(None),
            icon_url: Set(None),
            is_official: Set(false),
            is_enabled: Set(is_enabled),
            installed_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await
        .map(|_| ())
        .map_err(db_error)
    }

    pub async fn grant<P: ModulePermissionInput>(
        &self,
        user_id: &str,
        module_id: &str,
        permission: P,
    ) -> DgosResult<()> {
        self.grant_with_request(user_id, module_id, permission, None)
            .await
    }

    /// Validates the host-bound module identity before any capability check.
    /// The renderer cannot satisfy this check by choosing a module id alone:
    /// version and integrity must match the currently published database row.
    pub async fn validate_module_identity(
        &self,
        module_id: &str,
        version: &str,
        integrity: &str,
    ) -> DgosResult<bool> {
        let module = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?;
        Ok(module.is_some_and(|module| {
            module.is_enabled && module.current_version == version && module.integrity == integrity
        }))
    }

    /// Validate the complete module capability contract. A persisted grant is
    /// not sufficient when the currently published manifest did not request
    /// the capability; this prevents stale grants from expanding a package's
    /// authority after an update.
    pub async fn validate_module_capability(
        &self,
        module_id: &str,
        version: &str,
        integrity: &str,
        permission: ModulePermission,
    ) -> DgosResult<bool> {
        let module = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?;
        Ok(module.is_some_and(|module| {
            module.is_enabled
                && module.current_version == version
                && module.integrity == integrity
                && manifest_requests_permission(&module.manifest, permission)
        }))
    }

    pub async fn grant_with_request<P: ModulePermissionInput>(
        &self,
        user_id: &str,
        module_id: &str,
        permission: P,
        request_id: Option<&str>,
    ) -> DgosResult<()> {
        let permission = permission.resolve()?;
        let tx = self.db.begin().await.map_err(db_error)?;
        let module = modules::Entity::find_by_id(module_id)
            .one(&tx)
            .await
            .map_err(db_error)?;
        if module.as_ref().is_none_or(|module| {
            !module.is_enabled || !manifest_requests_permission(&module.manifest, permission)
        }) {
            self.audit_in(&tx, user_id, module_id, permission, "denied", request_id)
                .await?;
            tx.commit().await.map_err(db_error)?;
            return Err(DgosError::PermissionDenied);
        }

        let now = chrono::Utc::now().timestamp();
        let current = module_permissions::Entity::find()
            .filter(module_permissions::Column::UserId.eq(user_id))
            .filter(module_permissions::Column::ModuleId.eq(module_id))
            .filter(module_permissions::Column::Permission.eq(permission.as_str()))
            .one(&tx)
            .await
            .map_err(db_error)?;
        if let Some(current) = current {
            let mut model: module_permissions::ActiveModel = current.into();
            model.granted = Set(true);
            model.granted_at = Set(Some(now));
            model.revoked_at = Set(None);
            model.update(&tx).await.map_err(db_error)?;
        } else {
            module_permissions::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                user_id: Set(user_id.to_owned()),
                module_id: Set(module_id.to_owned()),
                permission: Set(permission.as_str().to_owned()),
                granted: Set(true),
                granted_at: Set(Some(now)),
                revoked_at: Set(None),
            }
            .insert(&tx)
            .await
            .map_err(db_error)?;
        }
        self.audit_in(&tx, user_id, module_id, permission, "granted", request_id)
            .await?;
        tx.commit().await.map_err(db_error)
    }

    pub async fn revoke<P: ModulePermissionInput>(
        &self,
        user_id: &str,
        module_id: &str,
        permission: P,
    ) -> DgosResult<()> {
        self.revoke_with_request(user_id, module_id, permission, None)
            .await
    }

    pub async fn revoke_with_request<P: ModulePermissionInput>(
        &self,
        user_id: &str,
        module_id: &str,
        permission: P,
        request_id: Option<&str>,
    ) -> DgosResult<()> {
        let permission = permission.resolve()?;
        let tx = self.db.begin().await.map_err(db_error)?;
        if modules::Entity::find_by_id(module_id)
            .one(&tx)
            .await
            .map_err(db_error)?
            .is_none()
        {
            self.audit_in(&tx, user_id, module_id, permission, "denied", request_id)
                .await?;
            tx.commit().await.map_err(db_error)?;
            return Err(DgosError::PermissionDenied);
        }
        let now = chrono::Utc::now().timestamp();
        let current = module_permissions::Entity::find()
            .filter(module_permissions::Column::UserId.eq(user_id))
            .filter(module_permissions::Column::ModuleId.eq(module_id))
            .filter(module_permissions::Column::Permission.eq(permission.as_str()))
            .one(&tx)
            .await
            .map_err(db_error)?;
        if let Some(current) = current {
            let mut model: module_permissions::ActiveModel = current.into();
            model.granted = Set(false);
            model.revoked_at = Set(Some(now));
            model.update(&tx).await.map_err(db_error)?;
        } else {
            module_permissions::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                user_id: Set(user_id.to_owned()),
                module_id: Set(module_id.to_owned()),
                permission: Set(permission.as_str().to_owned()),
                granted: Set(false),
                granted_at: Set(None),
                revoked_at: Set(Some(now)),
            }
            .insert(&tx)
            .await
            .map_err(db_error)?;
        }
        self.audit_in(&tx, user_id, module_id, permission, "revoked", request_id)
            .await?;
        tx.commit().await.map_err(db_error)
    }

    /// Reads the current row on every call; revocation therefore takes effect
    /// without waiting for a cache expiry.
    pub async fn check<P: ModulePermissionInput>(
        &self,
        user_id: &str,
        module_id: &str,
        permission: P,
    ) -> DgosResult<bool> {
        self.check_with_request(user_id, module_id, permission, None)
            .await
    }

    pub async fn check_with_request<P: ModulePermissionInput>(
        &self,
        user_id: &str,
        module_id: &str,
        permission: P,
        request_id: Option<&str>,
    ) -> DgosResult<bool> {
        let permission = permission.resolve()?;
        let module_enabled = modules::Entity::find_by_id(module_id)
            .one(&self.db)
            .await
            .map_err(db_error)?
            .is_some_and(|module| module.is_enabled);
        let granted = module_enabled
            && module_permissions::Entity::find()
                .filter(module_permissions::Column::UserId.eq(user_id))
                .filter(module_permissions::Column::ModuleId.eq(module_id))
                .filter(module_permissions::Column::Permission.eq(permission.as_str()))
                .one(&self.db)
                .await
                .map_err(db_error)?
                .is_some_and(|row| row.granted);
        if !granted {
            // A failed audit must never turn a denial into an authorization.
            let _ = self
                .audit(user_id, module_id, permission, "denied", request_id)
                .await;
        }
        Ok(granted)
    }

    async fn audit(
        &self,
        user_id: &str,
        module_id: &str,
        permission: ModulePermission,
        action: &str,
        request_id: Option<&str>,
    ) -> DgosResult<()> {
        module_permission_audits::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            user_id: Set(user_id.to_owned()),
            module_id: Set(module_id.to_owned()),
            permission: Set(permission.as_str().to_owned()),
            action: Set(action.to_owned()),
            resource_id: Set(None),
            request_id: Set(request_id.map(str::to_owned)),
            created_at: Set(chrono::Utc::now().timestamp()),
        }
        .insert(&self.db)
        .await
        .map(|_| ())
        .map_err(db_error)
    }

    async fn audit_in(
        &self,
        tx: &sea_orm::DatabaseTransaction,
        user_id: &str,
        module_id: &str,
        permission: ModulePermission,
        action: &str,
        request_id: Option<&str>,
    ) -> DgosResult<()> {
        module_permission_audits::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            user_id: Set(user_id.to_owned()),
            module_id: Set(module_id.to_owned()),
            permission: Set(permission.as_str().to_owned()),
            action: Set(action.to_owned()),
            resource_id: Set(None),
            request_id: Set(request_id.map(str::to_owned)),
            created_at: Set(chrono::Utc::now().timestamp()),
        }
        .insert(tx)
        .await
        .map(|_| ())
        .map_err(db_error)
    }
}

fn db_error(error: sea_orm::DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}

fn manifest_requests_permission(
    manifest: &serde_json::Value,
    permission: ModulePermission,
) -> bool {
    manifest
        .get("permissions")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|permissions| {
            permissions
                .iter()
                .any(|value| value.as_str() == Some(permission.as_str()))
        })
}

#[cfg(test)]
fn validate_module_registration(
    id: &str,
    version: &str,
    integrity: &str,
    manifest: &serde_json::Value,
) -> DgosResult<()> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id == "."
        || id == ".."
        || version.is_empty()
        || version.contains('/')
        || version.contains('\\')
        || integrity.len() != 64
        || !integrity.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DgosError::InternalError(
            "invalid module registration identity".into(),
        ));
    }
    let permissions = manifest
        .get("permissions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| DgosError::InternalError("module manifest permissions missing".into()))?;
    for permission in permissions {
        let value = permission
            .as_str()
            .ok_or_else(|| DgosError::InternalError("invalid module permission".into()))?;
        ModulePermission::parse(value)?;
    }
    Ok(())
}
