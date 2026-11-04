//! Real storage sources for the five cleanup categories (D133 item 5).
//!
//! Account-owned reads and deletes are scoped to one `user_id`. Every delete
//! runs in its own transaction and repeats its eligibility check together with
//! the candidate version from the preview, so a row that changed, became
//! referenced or started after the preview is kept.

use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, QueryResult, Statement,
    TransactionTrait, Value,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageCategory {
    OldChats,
    SessionAssets,
    TempFiles,
    Cache,
    ActivityRecords,
}

impl StorageCategory {
    pub fn key(self) -> &'static str {
        match self {
            Self::OldChats => "old_chats",
            Self::SessionAssets => "session_assets",
            Self::TempFiles => "temp_files",
            Self::Cache => "cache",
            Self::ActivityRecords => "activity_records",
        }
    }
}

/// One candidate. `id` addresses the row. `version` must still match at delete
/// time: `updated_at` for sessions, a JSON tuple of timestamp/state/path for
/// install operations, and `created_at` for activity rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub version: String,
}

/// Non-security usage tables with the SQL expression that yields integer epoch seconds for `created_at`.
/// `provider_usage_logs.created_at` is a text datetime, so it is converted with `strftime`; the other
/// tables store integer seconds. Permission and security audits, LAN audits and credentials are never listed.
pub const ACTIVITY_TABLES: [(&str, &str); 4] = [
    ("provider_usage_logs", "CAST(strftime('%s', created_at) AS INTEGER)"),
    ("mcp_usage_logs", "created_at"),
    ("skill_usage_logs", "created_at"),
    ("mcp_connection_logs", "created_at"),
];

pub struct StorageRepository {
    db: DatabaseConnection,
}

fn stmt(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

fn to_candidates(rows: Vec<QueryResult>) -> Result<Vec<Candidate>, DbErr> {
    rows.iter()
        .map(|row| {
            Ok(Candidate {
                id: row.try_get("", "id")?,
                version: row.try_get("", "version")?,
            })
        })
        .collect()
}

impl StorageRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Idle chat sessions older than `cutoff`. A session with a generating
    /// message or a running workbench task is never a candidate.
    pub async fn old_chat_candidates(&self, user: &str, cutoff: i64) -> Result<Vec<Candidate>, DbErr> {
        let rows = self
            .db
            .query_all(stmt(
                "SELECT s.id AS id, CAST(s.updated_at AS TEXT) AS version FROM chat_sessions s
                 WHERE s.user_id = ? AND s.updated_at < ?
                   AND NOT EXISTS (SELECT 1 FROM chat_messages m
                        WHERE m.session_id = s.id AND m.user_id = s.user_id AND m.status = 'generating')
                   AND NOT EXISTS (SELECT 1 FROM workbench_tasks t
                        WHERE t.session_id = s.id AND t.user_id = s.user_id AND t.status = 'running')
                 ORDER BY s.updated_at ASC, s.id ASC",
                vec![user.into(), cutoff.into()],
            ))
            .await?;
        to_candidates(rows)
    }

    /// Summed UTF-8 byte length of the messages of the given sessions.
    pub async fn old_chat_bytes(&self, user: &str, candidates: &[Candidate]) -> Result<i64, DbErr> {
        let mut bytes = 0i64;
        for candidate in candidates {
            let row = self
                .db
                .query_one(stmt(
                    "SELECT COALESCE(SUM(LENGTH(CAST(content AS BLOB))), 0) AS b
                     FROM chat_messages WHERE session_id = ? AND user_id = ?",
                    vec![candidate.id.clone().into(), user.into()],
                ))
                .await?;
            if let Some(row) = row {
                bytes += row.try_get::<i64>("", "b")?;
            }
        }
        Ok(bytes)
    }

    /// Deletes each candidate in its own transaction if it still matches the
    /// eligibility predicate and its `updated_at` equals the candidate version.
    /// Returns the ids that were removed; the others are reported as skipped.
    pub async fn delete_old_chats(
        &self,
        user: &str,
        cutoff: i64,
        candidates: &[Candidate],
    ) -> Result<Vec<String>, DbErr> {
        let mut removed = Vec::new();
        for candidate in candidates {
            let tx = self.db.begin().await?;
            let still = tx
                .query_one(stmt(
                    "SELECT s.id AS id FROM chat_sessions s
                     WHERE s.id = ? AND s.user_id = ? AND s.updated_at < ?
                       AND CAST(s.updated_at AS TEXT) = ?
                       AND NOT EXISTS (SELECT 1 FROM chat_messages m
                            WHERE m.session_id = s.id AND m.user_id = s.user_id AND m.status = 'generating')
                       AND NOT EXISTS (SELECT 1 FROM workbench_tasks t
                            WHERE t.session_id = s.id AND t.user_id = s.user_id AND t.status = 'running')",
                    vec![
                        candidate.id.clone().into(),
                        user.into(),
                        cutoff.into(),
                        candidate.version.clone().into(),
                    ],
                ))
                .await?;
            if still.is_none() {
                tx.rollback().await?;
                continue;
            }
            let id = candidate.id.clone();
            tx.execute(stmt(
                "UPDATE workbench_tasks SET session_id = NULL WHERE session_id = ? AND user_id = ?",
                vec![id.clone().into(), user.into()],
            ))
            .await?;
            tx.execute(stmt(
                "DELETE FROM chat_message_assets WHERE message_id IN
                   (SELECT id FROM chat_messages WHERE session_id = ? AND user_id = ?)",
                vec![id.clone().into(), user.into()],
            ))
            .await?;
            tx.execute(stmt(
                "DELETE FROM chat_messages WHERE session_id = ? AND user_id = ?",
                vec![id.clone().into(), user.into()],
            ))
            .await?;
            let deleted = tx
                .execute(stmt(
                    "DELETE FROM chat_sessions WHERE id = ? AND user_id = ?",
                    vec![id.clone().into(), user.into()],
                ))
                .await?;
            if deleted.rows_affected() == 1 {
                tx.commit().await?;
                removed.push(id);
            } else {
                tx.rollback().await?;
            }
        }
        Ok(removed)
    }

    /// Deletes only the previewed asset size, repeating both reference and live lease checks.
    pub async fn delete_session_asset(&self, user: &str, candidate: &Candidate) -> Result<bool, DbErr> {
        let result = self.db.execute(stmt(
            "DELETE FROM chat_assets WHERE id = ? AND user_id = ? AND CAST(LENGTH(bytes) AS TEXT) = ?
             AND NOT EXISTS (SELECT 1 FROM chat_message_assets m WHERE m.asset_id = chat_assets.id AND m.user_id = chat_assets.user_id)
             AND NOT EXISTS (SELECT 1 FROM asset_leases l WHERE l.asset_id = chat_assets.id AND l.user_id = chat_assets.user_id AND l.expires_at > ?)",
            vec![candidate.id.clone().into(), user.into(), candidate.version.clone().into(), chrono::Utc::now().timestamp().into()],
        )).await?;
        Ok(result.rows_affected() == 1)
    }

    /// Expired terminal install operations. The version fixes timestamp, state and path;
    /// paths registered by any other operation are excluded across all accounts.
    pub async fn temp_candidates(&self, user: &str, cutoff: i64) -> Result<Vec<Candidate>, DbErr> {
        let rows = self
            .db
            .query_all(stmt(
                "SELECT o.operation_id AS id, json_array(o.updated_at, o.state, o.staging_relpath) AS version FROM skill_install_operations o
                 WHERE o.user_id = ? AND o.updated_at < ?
                   AND o.state IN ('failed', 'abandoned')
                   AND o.staging_relpath = 'staging/' || o.operation_id
                   AND NOT EXISTS (SELECT 1 FROM skill_install_operations other WHERE other.operation_id != o.operation_id
                       AND (other.staging_relpath = o.staging_relpath OR other.published_relpath = o.staging_relpath))
                 ORDER BY o.operation_id ASC",
                vec![user.into(), cutoff.into()],
            ))
            .await?;
        to_candidates(rows)
    }

    /// Acquires the SQLite write lock before checking the operation. Keep this transaction
    /// through directory removal so no installer or cleanup can change its ownership or state.
    pub async fn begin_temp_cleanup(&self, user: &str, cutoff: i64, candidate: &Candidate) -> Result<Option<DatabaseTransaction>, DbErr> {
        let tx = self.db.begin().await?;
        let claimed = tx.execute(stmt(
            "UPDATE skill_install_operations AS o SET staging_relpath = staging_relpath
             WHERE o.operation_id = ? AND o.user_id = ? AND o.updated_at < ?
             AND json_array(o.updated_at, o.state, o.staging_relpath) = ? AND o.state IN ('failed', 'abandoned')
             AND o.staging_relpath = 'staging/' || o.operation_id
             AND NOT EXISTS (SELECT 1 FROM skill_install_operations other WHERE other.operation_id != o.operation_id
                 AND (other.staging_relpath = o.staging_relpath OR other.published_relpath = o.staging_relpath))",
            vec![candidate.id.clone().into(), user.into(), cutoff.into(), candidate.version.clone().into()],
        )).await?;
        if claimed.rows_affected() == 1 {
            Ok(Some(tx))
        } else {
            tx.rollback().await?;
            Ok(None)
        }
    }

    /// Clears the path and releases the write lock after its directory was removed.
    pub async fn forget_temp(&self, tx: DatabaseTransaction, user: &str, candidate: &Candidate) -> Result<bool, DbErr> {
        let changed = tx.execute(stmt(
            "UPDATE skill_install_operations SET staging_relpath = NULL
             WHERE operation_id = ? AND user_id = ? AND json_array(updated_at, state, staging_relpath) = ?",
            vec![candidate.id.clone().into(), user.into(), candidate.version.clone().into()],
        )).await?;
        if changed.rows_affected() == 1 {
            tx.commit().await?;
            Ok(true)
        } else {
            tx.rollback().await?;
            Ok(false)
        }
    }

    /// Non-security usage rows older than `cutoff` seconds. The id is `table:rowid` and the version is the
    /// raw `created_at` text, so a delete can prove the row is the one previewed. Permission and security
    /// audits are not listed.
    pub async fn activity_candidates(&self, user: &str, cutoff: i64) -> Result<Vec<Candidate>, DbErr> {
        let mut out = Vec::new();
        for (table, expression) in ACTIVITY_TABLES {
            let sql = format!(
                "SELECT rowid AS rid, CAST(created_at AS TEXT) AS version FROM {table}
                 WHERE user_id = ? AND ({expression}) < ? ORDER BY rowid ASC"
            );
            let rows = self.db.query_all(stmt(&sql, vec![user.into(), cutoff.into()])).await?;
            for row in rows {
                let rid: i64 = row.try_get("", "rid")?;
                out.push(Candidate {
                    id: format!("{table}:{rid}"),
                    version: row.try_get("", "version")?,
                });
            }
        }
        Ok(out)
    }

    /// Deletes one activity row in its own transaction, only if it is still owned by `user` and its
    /// `created_at` text equals the candidate version.
    pub async fn delete_activity(&self, user: &str, candidate: &Candidate) -> Result<bool, DbErr> {
        let Some((table, rid)) = candidate.id.rsplit_once(':') else {
            return Ok(false);
        };
        let Some(&(table, _)) = ACTIVITY_TABLES.iter().find(|entry| entry.0 == table) else {
            return Ok(false);
        };
        let Ok(rid) = rid.parse::<i64>() else {
            return Ok(false);
        };
        let tx = self.db.begin().await?;
        let sql = format!(
            "DELETE FROM {table} WHERE rowid = ? AND user_id = ? AND CAST(created_at AS TEXT) = ?"
        );
        let result = tx
            .execute(stmt(&sql, vec![rid.into(), user.into(), candidate.version.clone().into()]))
            .await?;
        if result.rows_affected() == 1 {
            tx.commit().await?;
            Ok(true)
        } else {
            tx.rollback().await?;
            Ok(false)
        }
    }

    /// Page-level size of the shared database in bytes. Used before and after VACUUM so the result
    /// reports the real change instead of a claimed erasure. Physical erasure is not promised.
    pub async fn database_bytes(&self) -> Result<i64, DbErr> {
        let row = self
            .db
            .query_one(stmt(
                "SELECT (SELECT page_count FROM pragma_page_count) * (SELECT page_size FROM pragma_page_size) AS b",
                vec![],
            ))
            .await?;
        match row {
            Some(row) => row.try_get("", "b"),
            None => Ok(0),
        }
    }

    /// Runs VACUUM on the shared database. Callers must hold a `MaintenanceLease`; this method does not
    /// check admission itself, so it is only reachable through the service's lease path.
    pub async fn vacuum(&self) -> Result<(), DbErr> {
        self.db.execute(stmt("VACUUM", vec![])).await.map(|_| ())
    }

}
