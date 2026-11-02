//! Local accounts (V1-FR-008, D087, D090) and host-level settings.
//!
//! The repository never sees a plaintext password: callers pass the Argon2id
//! PHC string. `UserRecord`'s `Debug` hides the hash, and no error built here
//! contains a username, hash or SQL text.

use dgos_types::{DgosError, DgosResult, DEFAULT_USER_ID};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, DbErr, QueryResult, Statement,
    TransactionTrait, Value,
};

/// `host_settings` key of the last successfully signed-in user (AC11).
pub const LAST_SIGN_IN_USER: &str = "last_sign_in_user";

#[derive(Clone)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub failed_attempts: u32,
    pub locked_until: Option<i64>,
    pub last_sign_in_at: Option<i64>,
    pub deleting_since: Option<i64>,
}

impl std::fmt::Debug for UserRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserRecord")
            .field("failed_attempts", &self.failed_attempts)
            .field("locked_until", &self.locked_until)
            .field("deleting", &self.deleting_since.is_some())
            .finish_non_exhaustive()
    }
}

pub struct NewUser<'a> {
    pub id: &'a str,
    pub username: &'a str,
    pub username_key: &'a str,
    pub password_hash: &'a str,
    pub now: i64,
}

#[derive(Clone)]
pub struct UserRepository {
    db: DatabaseConnection,
}

const COLUMNS: &str =
    "id, username, password_hash, failed_attempts, locked_until, last_sign_in_at, deleting_since";

impl UserRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Accounts that can sign in (no deletion in progress).
    pub async fn list_active(&self) -> DgosResult<Vec<UserRecord>> {
        self.query(
            &format!(
                "SELECT {COLUMNS} FROM users WHERE deleting_since IS NULL \
                 ORDER BY last_sign_in_at IS NULL, last_sign_in_at DESC, created_at ASC"
            ),
            [],
        )
        .await
    }

    /// Every row, including accounts whose deletion has not finished. A row
    /// being deleted still blocks the `default-user` id and keeps the host
    /// out of `no_accounts` (D087).
    pub async fn count_all(&self) -> DgosResult<u64> {
        let row = self
            .db
            .query_one(stmt("SELECT COUNT(*) AS n FROM users", []))
            .await
            .map_err(db_error)?;
        Ok(row
            .map(|row| row.try_get::<i64>("", "n"))
            .transpose()
            .map_err(db_error)?
            .unwrap_or(0) as u64)
    }

    pub async fn ids(&self) -> DgosResult<Vec<String>> {
        let rows = self
            .db
            .query_all(stmt("SELECT id FROM users ORDER BY created_at", []))
            .await
            .map_err(db_error)?;
        rows.iter()
            .map(|row| row.try_get("", "id").map_err(db_error))
            .collect()
    }

    pub async fn deleting_ids(&self) -> DgosResult<Vec<String>> {
        let rows = self
            .db
            .query_all(stmt(
                "SELECT id FROM users WHERE deleting_since IS NOT NULL ORDER BY deleting_since",
                [],
            ))
            .await
            .map_err(db_error)?;
        rows.iter()
            .map(|row| row.try_get("", "id").map_err(db_error))
            .collect()
    }

    pub async fn default_user_taken(&self) -> DgosResult<bool> {
        Ok(self.by_id(DEFAULT_USER_ID).await?.is_some())
    }

    /// Lookup for sign-in: an account being deleted cannot sign in.
    pub async fn find_active_by_key(&self, username_key: &str) -> DgosResult<Option<UserRecord>> {
        Ok(self
            .query(
                &format!(
                    "SELECT {COLUMNS} FROM users WHERE username_key = ? AND deleting_since IS NULL"
                ),
                [username_key.into()],
            )
            .await?
            .pop())
    }

    pub async fn by_id(&self, id: &str) -> DgosResult<Option<UserRecord>> {
        Ok(self
            .query(
                &format!("SELECT {COLUMNS} FROM users WHERE id = ?"),
                [id.into()],
            )
            .await?
            .pop())
    }

    /// `UsernameTaken` when the case-folded name exists; a second
    /// `default-user` row is impossible because `id` is the primary key.
    pub async fn insert(&self, user: NewUser<'_>) -> DgosResult<()> {
        let result = self
            .db
            .execute(stmt(
                "INSERT INTO users (id, username, username_key, password_hash, failed_attempts, \
                 created_at, updated_at) VALUES (?, ?, ?, ?, 0, ?, ?)",
                [
                    user.id.into(),
                    user.username.into(),
                    user.username_key.into(),
                    user.password_hash.into(),
                    user.now.into(),
                    user.now.into(),
                ],
            ))
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error) if is_unique_violation(&error, "users.username_key") => {
                Err(DgosError::UsernameTaken)
            }
            Err(error) => Err(db_error(error)),
        }
    }

    pub async fn record_failure(
        &self,
        id: &str,
        failed_attempts: u32,
        locked_until: Option<i64>,
        now: i64,
    ) -> DgosResult<()> {
        self.exec(
            "UPDATE users SET failed_attempts = ?, locked_until = ?, updated_at = ? WHERE id = ?",
            [
                i64::from(failed_attempts).into(),
                locked_until.into(),
                now.into(),
                id.into(),
            ],
        )
        .await
    }

    /// A correct password: the failure count and back-off are cleared.
    pub async fn clear_failures(&self, id: &str, now: i64) -> DgosResult<()> {
        self.exec(
            "UPDATE users SET failed_attempts = 0, locked_until = NULL, updated_at = ? \
             WHERE id = ? AND (failed_attempts <> 0 OR locked_until IS NOT NULL)",
            [now.into(), id.into()],
        )
        .await
    }

    pub async fn record_sign_in(&self, id: &str, now: i64) -> DgosResult<()> {
        self.exec(
            "UPDATE users SET last_sign_in_at = ?, updated_at = ? WHERE id = ?",
            [now.into(), now.into(), id.into()],
        )
        .await
    }

    pub async fn set_password_hash(&self, id: &str, hash: &str, now: i64) -> DgosResult<()> {
        self.exec(
            "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
            [hash.into(), now.into(), id.into()],
        )
        .await
    }

    /// One transaction (D090, technical design B08-48/49): the persistent
    /// deletion marker, and every vault item of the user listed in the
    /// Provider and MCP credential outboxes before any row that names it is
    /// removed. Soft-deleted MCP servers are included: their credential may
    /// never have left the vault.
    pub async fn mark_deleting(&self, id: &str, now: i64) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        tx.execute(stmt(
            "UPDATE users SET deleting_since = COALESCE(deleting_since, ?), updated_at = ? \
             WHERE id = ?",
            [now.into(), now.into(), id.into()],
        ))
        .await
        .map_err(db_error)?;
        tx.execute(stmt(
            "INSERT INTO provider_credential_cleanup (credential_ref, user_id, provider_id, endpoint) \
             SELECT credential_ref, user_id, id, base_url FROM providers \
             WHERE user_id = ? AND credential_ref IS NOT NULL \
             ON CONFLICT(credential_ref) DO NOTHING",
            [id.into()],
        ))
        .await
        .map_err(db_error)?;
        tx.execute(stmt(
            "INSERT INTO mcp_credential_cleanup (credential_ref, user_id, server_id, endpoint) \
             SELECT credential_ref, user_id, id, json_extract(config_json, '$.url') FROM mcp_servers \
             WHERE user_id = ? AND credential_ref IS NOT NULL AND transport = 'http' \
             AND json_extract(config_json, '$.url') IS NOT NULL \
             ON CONFLICT(credential_ref) DO NOTHING",
            [id.into()],
        ))
        .await
        .map_err(db_error)?;
        tx.commit().await.map_err(db_error)
    }

    /// Last step of a deletion: the "last signed-in" pointer to the user and
    /// the account row. Other host settings stay (D087).
    pub async fn remove(&self, id: &str) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        tx.execute(stmt(
            "DELETE FROM host_settings WHERE setting_key = ? AND setting_value = ?",
            [LAST_SIGN_IN_USER.into(), id.into()],
        ))
        .await
        .map_err(db_error)?;
        tx.execute(stmt("DELETE FROM users WHERE id = ?", [id.into()]))
            .await
            .map_err(db_error)?;
        tx.commit().await.map_err(db_error)
    }

    async fn query<const N: usize>(
        &self,
        sql: &str,
        values: [Value; N],
    ) -> DgosResult<Vec<UserRecord>> {
        let rows = self
            .db
            .query_all(stmt(sql, values))
            .await
            .map_err(db_error)?;
        rows.iter().map(record).collect()
    }

    async fn exec<const N: usize>(&self, sql: &str, values: [Value; N]) -> DgosResult<()> {
        self.db
            .execute(stmt(sql, values))
            .await
            .map(|_| ())
            .map_err(db_error)
    }
}

/// Settings that belong to no user (D087): the last signed-in user, later the
/// LAN switch and port (FR-010).
#[derive(Clone)]
pub struct HostSettingsRepository {
    db: DatabaseConnection,
}

pub const UI_LANGUAGE_KEY: &str = "ui.language";

impl HostSettingsRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn get(&self, key: &str) -> DgosResult<Option<String>> {
        let row = self
            .db
            .query_one(stmt(
                "SELECT setting_value FROM host_settings WHERE setting_key = ?",
                [key.into()],
            ))
            .await
            .map_err(db_error)?;
        row.map(|row| row.try_get("", "setting_value"))
            .transpose()
            .map_err(db_error)
    }

    pub async fn set(&self, key: &str, value: &str, now: i64) -> DgosResult<()> {
        self.db
            .execute(stmt(
                "INSERT INTO host_settings (setting_key, setting_value, updated_at) VALUES (?, ?, ?) \
                 ON CONFLICT(setting_key) DO UPDATE SET setting_value = excluded.setting_value, \
                 updated_at = excluded.updated_at",
                [key.into(), value.into(), now.into()],
            ))
            .await
            .map(|_| ())
            .map_err(db_error)
    }

    pub async fn get_language(&self) -> DgosResult<Option<dgos_types::LanguagePreference>> {
        let raw = self.get(UI_LANGUAGE_KEY).await?;
        Ok(raw.and_then(|value| dgos_types::LanguagePreference::parse(&value)))
    }

    pub async fn set_language(
        &self,
        value: dgos_types::LanguagePreference,
        now: i64,
    ) -> DgosResult<()> {
        self.set(UI_LANGUAGE_KEY, value.as_str(), now).await
    }
}

fn record(row: &QueryResult) -> DgosResult<UserRecord> {
    let failed: i64 = row.try_get("", "failed_attempts").map_err(db_error)?;
    Ok(UserRecord {
        id: row.try_get("", "id").map_err(db_error)?,
        username: row.try_get("", "username").map_err(db_error)?,
        password_hash: row.try_get("", "password_hash").map_err(db_error)?,
        failed_attempts: u32::try_from(failed.max(0)).unwrap_or(u32::MAX),
        locked_until: row.try_get("", "locked_until").map_err(db_error)?,
        last_sign_in_at: row.try_get("", "last_sign_in_at").map_err(db_error)?,
        deleting_since: row.try_get("", "deleting_since").map_err(db_error)?,
    })
}

fn stmt<const N: usize>(sql: &str, values: [Value; N]) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

fn is_unique_violation(error: &DbErr, column: &str) -> bool {
    let text = error.to_string();
    text.contains("UNIQUE constraint failed") && text.contains(column)
}

/// Driver text can contain SQL; it goes to the log only (callers map storage
/// failures to a fixed message).
fn db_error(error: DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}
