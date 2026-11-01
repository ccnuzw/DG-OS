//! MCP credential cleanup outbox (D090), the twin of
//! `provider_credential_cleanup` (D023). An entry is written before the row
//! that names the vault item stops naming it, and is removed only after the
//! vault removal succeeded, so a failed removal is retried and never reported
//! as done. Entries are keyed by `credential_ref`: staging twice is a no-op.

use dgos_types::{DgosError, DgosResult};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpCredentialCleanup {
    pub user_id: String,
    pub server_id: String,
    pub endpoint: String,
    pub credential_ref: String,
}

#[derive(Clone)]
pub struct McpCredentialCleanupRepository {
    db: DatabaseConnection,
}

impl McpCredentialCleanupRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn stage(&self, entry: &McpCredentialCleanup) -> DgosResult<()> {
        stage_mcp_credential_cleanup(&self.db, entry).await
    }

    pub async fn pending(&self) -> DgosResult<Vec<McpCredentialCleanup>> {
        let rows = self
            .db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT credential_ref, user_id, server_id, endpoint FROM mcp_credential_cleanup",
            ))
            .await
            .map_err(db_error)?;
        rows.iter()
            .map(|row| {
                Ok(McpCredentialCleanup {
                    credential_ref: row.try_get("", "credential_ref").map_err(db_error)?,
                    user_id: row.try_get("", "user_id").map_err(db_error)?,
                    server_id: row.try_get("", "server_id").map_err(db_error)?,
                    endpoint: row.try_get("", "endpoint").map_err(db_error)?,
                })
            })
            .collect()
    }

    /// A live (not soft-deleted) server still uses the reference, so the
    /// vault item must stay.
    pub async fn is_active(&self, credential_ref: &str) -> DgosResult<bool> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM mcp_servers WHERE credential_ref = ? AND deleted_at IS NULL",
                [credential_ref.into()],
            ))
            .await
            .map_err(db_error)?;
        let count: i64 = row
            .map(|row| row.try_get("", "n"))
            .transpose()
            .map_err(db_error)?
            .unwrap_or(0);
        Ok(count > 0)
    }

    pub async fn acknowledge(&self, credential_ref: &str) -> DgosResult<()> {
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "DELETE FROM mcp_credential_cleanup WHERE credential_ref = ?",
                [credential_ref.into()],
            ))
            .await
            .map(|_| ())
            .map_err(db_error)
    }
}

/// Stage inside the caller's transaction when there is one.
pub async fn stage_mcp_credential_cleanup(
    db: &impl ConnectionTrait,
    entry: &McpCredentialCleanup,
) -> DgosResult<()> {
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "INSERT INTO mcp_credential_cleanup (credential_ref, user_id, server_id, endpoint) \
         VALUES (?, ?, ?, ?) ON CONFLICT(credential_ref) DO NOTHING",
        [
            entry.credential_ref.clone().into(),
            entry.user_id.clone().into(),
            entry.server_id.clone().into(),
            entry.endpoint.clone().into(),
        ],
    ))
    .await
    .map(|_| ())
    .map_err(db_error)
}

fn db_error(error: DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}
