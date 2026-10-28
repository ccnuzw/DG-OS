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
    pub async fn register_module(
        &self,
        id: impl Into<String>,
        module_name: impl Into<String>,
        display_name: impl Into<String>,
        version: impl Into<String>,
        manifest: serde_json::Value,
        integrity: impl Into<String>,
        is_enabled: bool,
    ) -> DgosResult<()> {
        let now = chrono::Utc::now().timestamp();
        modules::ActiveModel {
            id: Set(id.into()),
            module_name: Set(module_name.into()),
            display_name: Set(display_name.into()),
            current_version: Set(version.into()),
            description: Set(None),
            manifest: Set(manifest),
            integrity: Set(integrity.into()),
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
        if module.as_ref().is_none_or(|module| !module.is_enabled) {
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
