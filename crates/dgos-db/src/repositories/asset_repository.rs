//! Account-controlled chat assets (V2 D133 §2). Bytes live only in `chat_assets`; no path is ever
//! stored or returned. Every statement filters by the host-derived `user_id`, so another account's
//! asset and a missing one are the same "not found".
//!
//! `delete_unreferenced` checks for references inside the DELETE itself, so an asset that gained a
//! message reference between a preview and the delete is kept.

use sea_orm::*;
use serde::Serialize;

/// Metadata shown to the renderer. The bytes are never part of this type.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AssetRef {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub bytes_len: i64,
}

#[derive(Clone)]
pub struct AssetRepository {
    db: DatabaseConnection,
}

fn stmt(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

/// Lease id of the import lease that protects a draft until a message links it.
pub const DRAFT_LEASE_ID: &str = "draft";
impl AssetRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Stores one asset owned by `user` and its import lease in one transaction, so an asset
    /// is never stored unprotected. Callers validate size and content signature first.
    pub async fn insert(
        &self,
        user: &str,
        id: &str,
        name: &str,
        mime: &str,
        bytes: &[u8],
        lease_expires_at: i64,
    ) -> Result<(), DbErr> {
        let tx = self.db.begin().await?;
        tx.execute(stmt(
            "INSERT INTO chat_assets (id, user_id, name, mime, bytes, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                id.into(),
                user.into(),
                name.into(),
                mime.into(),
                bytes.to_vec().into(),
                chrono::Utc::now().timestamp().into(),
            ],
        ))
        .await?;
        tx.execute(stmt(
            "INSERT INTO asset_leases (asset_id, user_id, lease_id, expires_at) \
             VALUES (?, ?, ?, ?)",
            vec![
                id.into(),
                user.into(),
                DRAFT_LEASE_ID.into(),
                lease_expires_at.into(),
            ],
        ))
        .await?;
        tx.commit().await
    }

    /// Metadata for the given ids, restricted to `user`. Ids that are not owned by `user` are
    /// omitted, so the caller can detect a mismatch by comparing lengths.
    pub async fn refs(&self, user: &str, ids: &[String]) -> Result<Vec<AssetRef>, DbErr> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let row = self
                .db
                .query_one(stmt(
                    "SELECT id, name, mime, LENGTH(bytes) AS bytes_len FROM chat_assets \
                     WHERE user_id = ? AND id = ?",
                    vec![user.into(), id.as_str().into()],
                ))
                .await?;
            if let Some(row) = row {
                out.push(AssetRef {
                    id: row.try_get("", "id")?,
                    name: row.try_get("", "name")?,
                    mime: row.try_get("", "mime")?,
                    bytes_len: row.try_get("", "bytes_len")?,
                });
            }
        }
        Ok(out)
    }

    /// Bytes of one asset owned by `user`, for sending to the upstream.
    pub async fn bytes(&self, user: &str, id: &str) -> Result<Option<Vec<u8>>, DbErr> {
        let row = self
            .db
            .query_one(stmt(
                "SELECT bytes FROM chat_assets WHERE user_id = ? AND id = ?",
                vec![user.into(), id.into()],
            ))
            .await?;
        match row {
            None => Ok(None),
            Some(row) => Ok(Some(row.try_get("", "bytes")?)),
        }
    }

    /// Assets of `user` that no message references. Used for the storage preview.
    pub async fn list_unreferenced(&self, user: &str) -> Result<Vec<AssetRef>, DbErr> {
        let rows = self
            .db
            .query_all(stmt(
                "SELECT id, name, mime, LENGTH(bytes) AS bytes_len FROM chat_assets \
                 WHERE user_id = ? AND NOT EXISTS (SELECT 1 FROM chat_message_assets m \
                 WHERE m.asset_id = chat_assets.id AND m.user_id = chat_assets.user_id) \
                 AND NOT EXISTS (SELECT 1 FROM asset_leases l \
                 WHERE l.asset_id = chat_assets.id AND l.user_id = chat_assets.user_id \
                 AND l.expires_at > ?) \
                 ORDER BY created_at, id",
                vec![user.into(), chrono::Utc::now().timestamp().into()],
            ))
            .await?;
        rows.iter()
            .map(|row| {
                Ok(AssetRef {
                    id: row.try_get("", "id")?,
                    name: row.try_get("", "name")?,
                    mime: row.try_get("", "mime")?,
                    bytes_len: row.try_get("", "bytes_len")?,
                })
            })
            .collect()
    }

    /// Deletes the listed assets of `user` that are still unreferenced, in one statement.
    /// Returns the number of rows removed.
    pub async fn delete_unreferenced(&self, user: &str, ids: &[String]) -> Result<u64, DbErr> {
        if ids.is_empty() {
            return Ok(0);
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut values: Vec<Value> = vec![user.into()];
        values.extend(ids.iter().map(|id| Value::from(id.as_str())));
        values.push(chrono::Utc::now().timestamp().into());
        let result = self
            .db
            .execute(stmt(
                &format!(
                    "DELETE FROM chat_assets WHERE user_id = ? AND id IN ({placeholders}) \
                     AND NOT EXISTS (SELECT 1 FROM chat_message_assets m \
                     WHERE m.asset_id = chat_assets.id AND m.user_id = chat_assets.user_id) \
                     AND NOT EXISTS (SELECT 1 FROM asset_leases l \
                     WHERE l.asset_id = chat_assets.id AND l.user_id = chat_assets.user_id \
                     AND l.expires_at > ?)"
                ),
                values,
            ))
            .await?;
        Ok(result.rows_affected())
    }

    /// Total stored asset bytes of `user`.
    pub async fn total_bytes(&self, user: &str) -> Result<i64, DbErr> {
        let row = self
            .db
            .query_one(stmt(
                "SELECT COALESCE(SUM(LENGTH(bytes)), 0) AS total FROM chat_assets WHERE user_id = ?",
                vec![user.into()],
            ))
            .await?;
        match row {
            None => Ok(0),
            Some(row) => row.try_get("", "total"),
        }
    }

    /// Protects a draft asset until `expires_at` (unix seconds). A later call with the same
    /// `lease_id` moves the expiry, so editing a draft can keep it alive.
    pub async fn set_lease(
        &self,
        user: &str,
        asset_id: &str,
        lease_id: &str,
        expires_at: i64,
    ) -> Result<bool, DbErr> {
        let result = self
            .db
            .execute(stmt(
                "INSERT INTO asset_leases (asset_id, user_id, lease_id, expires_at) \
                 SELECT id, user_id, ?, ? FROM chat_assets WHERE id = ? AND user_id = ? \
                 ON CONFLICT(asset_id, lease_id) DO UPDATE SET expires_at = excluded.expires_at",
                vec![
                    lease_id.into(),
                    expires_at.into(),
                    asset_id.into(),
                    user.into(),
                ],
            ))
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Removes one draft asset of `user` and its leases in one transaction. The owner's own
    /// lease does not protect it, but any message link does: the link check is in the DELETE, so
    /// a link added at any point keeps the asset. Returns 1 when removed, 0 otherwise.
    pub async fn remove_draft(&self, user: &str, asset_id: &str) -> Result<u64, DbErr> {
        let tx = self.db.begin().await?;
        let removed = tx
            .execute(stmt(
                "DELETE FROM chat_assets WHERE user_id = ? AND id = ? \
                 AND NOT EXISTS (SELECT 1 FROM chat_message_assets m \
                 WHERE m.asset_id = chat_assets.id AND m.user_id = chat_assets.user_id)",
                vec![user.into(), asset_id.into()],
            ))
            .await?
            .rows_affected();
        if removed == 1 {
            tx.execute(stmt(
                "DELETE FROM asset_leases WHERE user_id = ? AND asset_id = ?",
                vec![user.into(), asset_id.into()],
            ))
            .await?;
        }
        tx.commit().await?;
        Ok(removed)
    }

    /// Linked assets of the given messages, in attachment order, with bytes. Restricted to `user`.
    pub async fn for_messages(
        &self,
        user: &str,
        message_ids: &[String],
    ) -> Result<Vec<MessageAsset>, DbErr> {
        let mut out = Vec::new();
        for message_id in message_ids {
            let rows = self
                .db
                .query_all(stmt(
                    "SELECT m.message_id, a.id, a.name, a.mime, a.bytes \
                     FROM chat_message_assets m \
                     JOIN chat_assets a ON a.id = m.asset_id AND a.user_id = m.user_id \
                     WHERE m.user_id = ? AND m.message_id = ? ORDER BY m.position",
                    vec![user.into(), message_id.as_str().into()],
                ))
                .await?;
            for row in rows {
                out.push(MessageAsset {
                    message_id: row.try_get("", "message_id")?,
                    asset_id: row.try_get("", "id")?,
                    name: row.try_get("", "name")?,
                    mime: row.try_get("", "mime")?,
                    data: row.try_get("", "bytes")?,
                });
            }
        }
        Ok(out)
    }
}

/// One asset linked to a message, with its bytes. Only the chat layer reads `data`.
#[derive(Clone)]
pub struct MessageAsset {
    pub message_id: String,
    pub asset_id: String,
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}
