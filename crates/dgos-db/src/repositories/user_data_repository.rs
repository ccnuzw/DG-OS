//! Per-user data removal for account deletion (V1-FR-008, D087, D090).
//!
//! There is no foreign key to `users`, so every table with a `user_id` column
//! is listed under exactly one domain here, or in `RETAINED_FOR_VAULT_RETRY`.
//! The schema test (`user_scope_tests`) fails when a migration adds a
//! `user_id` table that is in neither list, so a new user-scoped table cannot
//! be forgotten by account deletion.
//!
//! Each domain purge is its own transaction and is idempotent; the account
//! service runs them in `UserDataDomain::ORDER` after writing the persistent
//! deletion marker, and startup recovery repeats them after a crash.

use super::chat_repository::{ChatRepository, ChatStoreError};
use dgos_types::{DgosError, DgosResult, DEFAULT_USER_ID};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UserDataDomain {
    Chat,
    Workbench,
    Mcp,
    Providers,
    Skills,
    Modules,
    Settings,
    /// FR-010 audit rows bound to the user; NULL rows (no session yet) stay.
    Lan,
}

/// Tables that keep `user_id` rows after the account row is gone on purpose:
/// the vault cleanup outboxes. An entry is removed only when its vault item
/// was removed (D023); it needs the user id to rebuild the vault account.
pub const RETAINED_FOR_VAULT_RETRY: [&str; 2] =
    ["provider_credential_cleanup", "mcp_credential_cleanup"];

impl UserDataDomain {
    /// Purge order (technical design `finish_delete`): chat before the task
    /// audit it points at, child rows before their parents.
    pub const ORDER: [UserDataDomain; 8] = [
        UserDataDomain::Chat,
        UserDataDomain::Workbench,
        UserDataDomain::Mcp,
        UserDataDomain::Providers,
        UserDataDomain::Skills,
        UserDataDomain::Modules,
        UserDataDomain::Settings,
        UserDataDomain::Lan,
    ];

    pub fn name(self) -> &'static str {
        match self {
            UserDataDomain::Chat => "chat",
            UserDataDomain::Workbench => "workbench",
            UserDataDomain::Mcp => "mcp",
            UserDataDomain::Providers => "providers",
            UserDataDomain::Skills => "skills",
            UserDataDomain::Modules => "modules",
            UserDataDomain::Settings => "settings",
            UserDataDomain::Lan => "lan",
        }
    }

    /// Tables keyed by `owner`, purged with the workbench domain and included in orphan accounting.
    pub fn owner_tables(self) -> &'static [&'static str] {
        match self {
            UserDataDomain::Workbench => &["development_handoffs", "development_file_writes", "development_task_workspaces", "development_workspaces", "development_run_capabilities", "development_session_capabilities", "development_capability_heads", "development_capability_revisions", "development_selection", "development_commands", "development_runs", "development_review_observations", "development_review_baselines", "development_agent_sessions", "development_project_operations", "development_tasks", "development_projects"],
            _ => &[],
        }
    }

    /// Tables keyed by `user_id`; child tables without an owner column are removed through their parents.
    pub fn user_tables(self) -> &'static [&'static str] {
        match self {
            UserDataDomain::Chat => &["chat_message_assets", "asset_leases", "chat_assets", "chat_messages", "chat_sessions"],
            UserDataDomain::Workbench => &["workbench_tasks"],
            UserDataDomain::Mcp => &["mcp_usage_logs", "mcp_connection_logs", "mcp_servers"],
            UserDataDomain::Providers => &["provider_usage_logs", "usage_logs", "providers"],
            UserDataDomain::Skills => &[
                "skill_user_states",
                "skill_usage_logs",
                "skills",
                "skill_install_operations",
            ],
            UserDataDomain::Modules => &[
                "module_permissions",
                "module_permission_audits",
                "module_storage",
                "module_crash_logs",
            ],
            UserDataDomain::Settings => &["system_settings"],
            UserDataDomain::Lan => &["lan_audit_events"],
        }
    }
}

#[derive(Clone)]
pub struct UserDataRepository {
    db: DatabaseConnection,
}

impl UserDataRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Remove every row of `user_id` in `domain`. Idempotent. Chat refuses
    /// while a generation of the user is still `generating`; the caller has
    /// cancelled it first, so this is a retryable failure.
    pub async fn purge(&self, domain: UserDataDomain, user_id: &str) -> DgosResult<u64> {
        if domain == UserDataDomain::Chat {
            return ChatRepository::new(self.db.clone())
                .purge_user(user_id)
                .await
                .map_err(|error| match error {
                    ChatStoreError::GenerationInProgress => {
                        DgosError::InternalError("chat generation still in progress".into())
                    }
                    _ => DgosError::DatabaseError("chat purge failed".into()),
                });
        }
        let tx = self.db.begin().await.map_err(db_error)?;
        let mut removed = 0;
        for sql in statements(domain) {
            removed += tx
                .execute(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    *sql,
                    [user_id.into()],
                ))
                .await
                .map_err(db_error)?
                .rows_affected();
        }
        tx.commit().await.map_err(db_error)?;
        Ok(removed)
    }

    /// Rows in user-scoped tables whose owner is no account. `default-user`
    /// rows are not counted while no account exists (D087). Used for the
    /// startup warning only; nothing is deleted (D090).
    pub async fn orphan_count(&self) -> DgosResult<u64> {
        let mut total = 0;
        for domain in UserDataDomain::ORDER {
            for (table, column) in domain.user_tables().iter().map(|table| (*table, "user_id"))
                .chain(domain.owner_tables().iter().map(|table| (*table, "owner"))) {
                let sql = format!(
                    "SELECT COUNT(*) AS n FROM {table} WHERE {column} NOT IN (SELECT id FROM users) \
                     AND NOT ({column} = ? AND NOT EXISTS (SELECT 1 FROM users))"
                );
                let row = self
                    .db
                    .query_one(Statement::from_sql_and_values(
                        DbBackend::Sqlite,
                        sql,
                        [DEFAULT_USER_ID.into()],
                    ))
                    .await
                    .map_err(db_error)?;
                let count: i64 = row
                    .map(|row| row.try_get("", "n"))
                    .transpose()
                    .map_err(db_error)?
                    .unwrap_or(0);
                total += count.max(0) as u64;
            }
        }
        Ok(total)
    }

    /// Every table of the current schema that has a `user_id` column.
    pub async fn user_scoped_tables(&self) -> DgosResult<Vec<String>> {
        let rows = self
            .db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT m.name AS name FROM sqlite_master m \
                 WHERE m.type = 'table' AND EXISTS \
                 (SELECT 1 FROM pragma_table_info(m.name) p WHERE p.name = 'user_id') \
                 ORDER BY m.name",
            ))
            .await
            .map_err(db_error)?;
        rows.iter()
            .map(|row| row.try_get("", "name").map_err(db_error))
            .collect()
    }

    /// Vault removals of `user_id` still listed in either outbox.
    pub async fn pending_credential_cleanup(&self, user_id: &str) -> DgosResult<u64> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT (SELECT COUNT(*) FROM provider_credential_cleanup WHERE user_id = ?1) + \
                 (SELECT COUNT(*) FROM mcp_credential_cleanup WHERE user_id = ?1) AS n",
                [user_id.into()],
            ))
            .await
            .map_err(db_error)?;
        let count: i64 = row
            .map(|row| row.try_get("", "n"))
            .transpose()
            .map_err(db_error)?
            .unwrap_or(0);
        Ok(count.max(0) as u64)
    }
}

/// Child rows first. Every statement binds the user id once.
fn statements(domain: UserDataDomain) -> &'static [&'static str] {
    match domain {
        UserDataDomain::Chat => &[],
        UserDataDomain::Workbench => &[
            "DELETE FROM development_handoffs WHERE owner = ?1",
            "DELETE FROM development_file_writes WHERE owner = ?1",
            "DELETE FROM development_task_workspaces WHERE owner = ?1",
            "DELETE FROM development_workspaces WHERE owner = ?1",
            "DELETE FROM development_run_capabilities WHERE owner = ?1",
            "DELETE FROM development_session_capabilities WHERE owner = ?1",
            "DELETE FROM development_capability_heads WHERE owner = ?1",
            "DELETE FROM development_capability_revisions WHERE owner = ?1",
            "DELETE FROM development_selection WHERE owner = ?1",
            "DELETE FROM development_commands WHERE owner = ?1",
            "DELETE FROM development_runs WHERE owner = ?1",
            "DELETE FROM development_review_observations WHERE owner = ?1",
            "DELETE FROM development_review_baselines WHERE owner = ?1",
            "DELETE FROM development_agent_sessions WHERE owner = ?1",
            "DELETE FROM development_project_operations WHERE owner = ?1",
            "DELETE FROM development_tasks WHERE owner = ?1",
            "DELETE FROM development_projects WHERE owner = ?1",
            "DELETE FROM workbench_task_skills WHERE task_id IN \
             (SELECT id FROM workbench_tasks WHERE user_id = ?1)",
            "DELETE FROM workbench_task_tools WHERE task_id IN \
             (SELECT id FROM workbench_tasks WHERE user_id = ?1)",
            "DELETE FROM workbench_tasks WHERE user_id = ?1",
        ],
        UserDataDomain::Mcp => &[
            "DELETE FROM mcp_server_tools WHERE server_id IN \
             (SELECT id FROM mcp_servers WHERE user_id = ?1)",
            "DELETE FROM mcp_usage_logs WHERE user_id = ?1",
            "DELETE FROM mcp_connection_logs WHERE user_id = ?1",
            "DELETE FROM mcp_servers WHERE user_id = ?1",
        ],
        UserDataDomain::Providers => &[
            "DELETE FROM provider_usage_logs WHERE user_id = ?1 OR provider_id IN \
             (SELECT id FROM providers WHERE user_id = ?1)",
            "DELETE FROM usage_logs WHERE user_id = ?1 OR provider_id IN \
             (SELECT id FROM providers WHERE user_id = ?1)",
            "DELETE FROM models WHERE provider_id IN (SELECT id FROM providers WHERE user_id = ?1)",
            "DELETE FROM providers WHERE user_id = ?1",
        ],
        UserDataDomain::Skills => &[
            "DELETE FROM skill_user_states WHERE user_id = ?1",
            "DELETE FROM skill_usage_logs WHERE user_id = ?1",
            "DELETE FROM skills WHERE user_id = ?1",
            "DELETE FROM skill_install_operations WHERE user_id = ?1",
        ],
        UserDataDomain::Modules => &[
            "DELETE FROM module_permissions WHERE user_id = ?1",
            "DELETE FROM module_permission_audits WHERE user_id = ?1",
            "DELETE FROM module_storage WHERE user_id = ?1",
            "DELETE FROM module_crash_logs WHERE user_id = ?1",
        ],
        UserDataDomain::Settings => &["DELETE FROM system_settings WHERE user_id = ?1"],
        UserDataDomain::Lan => &["DELETE FROM lan_audit_events WHERE user_id = ?1"],
    }
}

fn db_error(error: DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}
