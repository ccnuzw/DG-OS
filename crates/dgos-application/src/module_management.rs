//! What the module management page reads and changes (D055).
//!
//! Permissions are granted per declared capability by the user; the
//! repository rejects anything the manifest does not declare. Lifecycle
//! changes (enable, disable, uninstall) go through `ModuleOperationManager`.

use crate::{ModuleCrashService, ModuleOperationManager};
use dgos_db::{ModulePermission, ModulePermissionRepository};
use dgos_types::{
    DgosError, DgosResult, HostSession, ModulePermissionAuditPageView, ModulePermissionAuditView,
    ModuleSummaryView,
};
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

    /// One page of the session user's permission audit, newest first. The
    /// user comes from the session, never from the caller. `before` is the
    /// opaque cursor from a previous page.
    pub async fn list_audits(
        &self,
        session: &HostSession,
        module_id: Option<&str>,
        before: Option<&str>,
        limit: Option<u32>,
    ) -> DgosResult<ModulePermissionAuditPageView> {
        let before = before.map(parse_cursor).transpose()?;
        let page = self
            .permissions
            .list_audits(session.user_id(), module_id, before, clamp_limit(limit))
            .await?;
        Ok(ModulePermissionAuditPageView {
            entries: page
                .entries
                .into_iter()
                .map(|entry| ModulePermissionAuditView {
                    module_id: entry.module_id,
                    permission: entry.permission,
                    action: entry.action,
                    created_at: entry.created_at,
                })
                .collect(),
            next_before: page.next_before.map(|cursor| cursor.to_string()),
        })
    }
}

const DEFAULT_AUDIT_LIMIT: u32 = 50;
const MAX_AUDIT_LIMIT: u32 = 200;

fn clamp_limit(limit: Option<u32>) -> u64 {
    u64::from(
        limit
            .unwrap_or(DEFAULT_AUDIT_LIMIT)
            .clamp(1, MAX_AUDIT_LIMIT),
    )
}

/// The cursor is a positive `rowid` in decimal. Anything else is invalid
/// input; it is never interpreted further.
fn parse_cursor(value: &str) -> DgosResult<i64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DgosError::ModuleCapabilityInputInvalid);
    }
    value
        .parse::<i64>()
        .map_err(|_| DgosError::ModuleCapabilityInputInvalid)
}

/// An unknown string is a denied request, not an internal error.
fn parse(value: &str) -> DgosResult<ModulePermission> {
    ModulePermission::parse(value).map_err(|_| DgosError::PermissionDenied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_limit_defaults_and_clamps() {
        assert_eq!(clamp_limit(None), 50);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(7)), 7);
        assert_eq!(clamp_limit(Some(200)), 200);
        assert_eq!(clamp_limit(Some(5000)), 200);
    }

    #[test]
    fn audit_cursor_must_be_a_decimal_rowid() {
        assert_eq!(parse_cursor("42").unwrap(), 42);
        for bad in ["", "-1", "+1", "1.5", "abc", " 1", "99999999999999999999"] {
            assert!(
                matches!(
                    parse_cursor(bad),
                    Err(DgosError::ModuleCapabilityInputInvalid)
                ),
                "{bad:?} must be rejected"
            );
        }
    }
}
