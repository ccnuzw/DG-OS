//! What the module management page reads and changes (D055).
//!
//! Permissions are granted per declared capability by the user; the
//! repository rejects anything the manifest does not declare. Lifecycle
//! changes (enable, disable, uninstall) go through `ModuleOperationManager`.

use crate::{ModuleCrashService, ModuleOperationManager};
use dgos_db::{ModulePermission, ModulePermissionRepository};
use dgos_types::{DgosError, DgosResult, HostSession, ModuleSummaryView};
use std::sync::Arc;

#[derive(Clone)]
pub struct ModuleManagementService {
    permissions: Arc<ModulePermissionRepository>,
    operations: ModuleOperationManager,
    crashes: ModuleCrashService,
}

impl ModuleManagementService {
    pub fn new(
        permissions: Arc<ModulePermissionRepository>,
        operations: ModuleOperationManager,
        crashes: ModuleCrashService,
    ) -> Self {
        Self {
            permissions,
            operations,
            crashes,
        }
    }

    pub async fn list(&self, session: &HostSession) -> DgosResult<Vec<ModuleSummaryView>> {
        let mut views = self
            .operations
            .modules(session)
            .await
            .map_err(DgosError::ModuleOperation)?;
        for view in &mut views {
            view.recent_crashes = self.crashes.recent(session, &view.module_id).await?;
        }
        Ok(views)
    }

    async fn summary(
        &self,
        session: &HostSession,
        module_id: &str,
    ) -> DgosResult<ModuleSummaryView> {
        self.list(session)
            .await?
            .into_iter()
            .find(|view| view.module_id == module_id)
            .ok_or(DgosError::ModuleOperation(
                dgos_types::ModuleOperationErrorKind::ModuleNotFound,
            ))
    }

    /// Grant one declared permission. The permission string is parsed against
    /// the D006 set and checked against the manifest by the repository.
    pub async fn grant(
        &self,
        session: &HostSession,
        module_id: &str,
        permission: &str,
    ) -> DgosResult<ModuleSummaryView> {
        let permission = parse(permission)?;
        self.permissions
            .grant(session.user_id(), module_id, permission)
            .await?;
        self.summary(session, module_id).await
    }

    pub async fn revoke(
        &self,
        session: &HostSession,
        module_id: &str,
        permission: &str,
    ) -> DgosResult<ModuleSummaryView> {
        let permission = parse(permission)?;
        self.permissions
            .revoke(session.user_id(), module_id, permission)
            .await?;
        self.summary(session, module_id).await
    }
}

/// An unknown string is a denied request, not an internal error.
fn parse(value: &str) -> DgosResult<ModulePermission> {
    ModulePermission::parse(value).map_err(|_| DgosError::PermissionDenied)
}
