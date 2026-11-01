//! Per-user, per-module key-value storage behind `storage.read` /
//! `storage.write` (D093).
//!
//! The scope `(user_id, module_id)` is always supplied by the host from the
//! capability grant; nothing here reads it from module input. Values are
//! opaque to this layer: they are never logged and never put in an error.

use crate::entities::module_storage;
use dgos_types::{DgosError, DgosResult};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QueryFilter,
    Statement, TransactionTrait,
};
use serde::{Deserialize, Serialize};

/// Longest key, in Unicode scalar values.
pub const MAX_STORAGE_KEY_CHARS: usize = 128;
/// Largest serialized JSON value, in bytes (64 KiB).
pub const MAX_STORAGE_VALUE_BYTES: usize = 64 * 1024;
/// Sum of `size_bytes` allowed per (user, module) (5 MiB).
pub const MAX_STORAGE_TOTAL_BYTES: i64 = 5 * 1024 * 1024;

/// One key of a listing. Never carries the value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModuleStorageEntry {
    pub key: String,
    pub size_bytes: u64,
}

#[derive(Clone)]
pub struct ModuleStorageRepository {
    db: DatabaseConnection,
}

fn db_error(error: sea_orm::DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}

fn stmt(sql: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

fn validate_key(key: &str) -> DgosResult<()> {
    let chars = key.chars().count();
    if chars == 0 || chars > MAX_STORAGE_KEY_CHARS || key.chars().any(char::is_control) {
        return Err(DgosError::ModuleCapabilityInputInvalid);
    }
    Ok(())
}

impl ModuleStorageRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// The stored JSON value, or `None` when the key does not exist.
    pub async fn get(
        &self,
        user_id: &str,
        module_id: &str,
        key: &str,
    ) -> DgosResult<Option<serde_json::Value>> {
        validate_key(key)?;
        let row = module_storage::Entity::find_by_id((
            user_id.to_owned(),
            module_id.to_owned(),
            key.to_owned(),
        ))
        .one(&self.db)
        .await
        .map_err(db_error)?;
        row.map(|row| {
            // The text was written by `set` from a serialized value; a parse
            // failure means the row was damaged. The text stays out of the error.
            serde_json::from_str(&row.value)
                .map_err(|_| DgosError::DatabaseError("module storage value is unreadable".into()))
        })
        .transpose()
    }

    /// Insert or replace one key. The size limit is checked before touching
    /// the database; the quota is computed inside the write transaction, so a
    /// replacement counts the new size instead of the old one and concurrent
    /// writers cannot jointly exceed it.
    pub async fn set(
        &self,
        user_id: &str,
        module_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> DgosResult<()> {
        validate_key(key)?;
        let text =
            serde_json::to_string(value).map_err(|_| DgosError::ModuleCapabilityInputInvalid)?;
        if text.len() > MAX_STORAGE_VALUE_BYTES {
            return Err(DgosError::ModuleCapabilityInputInvalid);
        }
        let tx = self.db.begin().await.map_err(db_error)?;
        // The upsert is the first statement, so the transaction takes SQLite's
        // write lock before the total is read.
        tx.execute(stmt(
            "INSERT INTO module_storage (user_id, module_id, \"key\", value, size_bytes, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id, module_id, \"key\") DO UPDATE SET \
             value = excluded.value, size_bytes = excluded.size_bytes, updated_at = excluded.updated_at",
            vec![
                user_id.into(),
                module_id.into(),
                key.into(),
                text.as_str().into(),
                (text.len() as i64).into(),
                chrono::Utc::now().timestamp().into(),
            ],
        ))
        .await
        .map_err(db_error)?;
        let total: i64 = tx
            .query_one(stmt(
                "SELECT COALESCE(SUM(size_bytes), 0) AS total FROM module_storage \
                 WHERE user_id = ? AND module_id = ?",
                vec![user_id.into(), module_id.into()],
            ))
            .await
            .map_err(db_error)?
            .map(|row| row.try_get("", "total"))
            .transpose()
            .map_err(db_error)?
            .unwrap_or(0);
        if total > MAX_STORAGE_TOTAL_BYTES {
            // Dropping the transaction rolls the upsert back.
            tx.rollback().await.map_err(db_error)?;
            return Err(DgosError::ModuleStorageQuotaExceeded);
        }
        tx.commit().await.map_err(db_error)
    }

    /// Idempotent: deleting a missing key succeeds.
    pub async fn delete(&self, user_id: &str, module_id: &str, key: &str) -> DgosResult<()> {
        validate_key(key)?;
        module_storage::Entity::delete_many()
            .filter(module_storage::Column::UserId.eq(user_id))
            .filter(module_storage::Column::ModuleId.eq(module_id))
            .filter(module_storage::Column::Key.eq(key))
            .exec(&self.db)
            .await
            .map(|_| ())
            .map_err(db_error)
    }

    /// Keys in ascending (Unicode scalar value) order with their sizes. Values
    /// are not selected.
    pub async fn list(
        &self,
        user_id: &str,
        module_id: &str,
    ) -> DgosResult<Vec<ModuleStorageEntry>> {
        self.db
            .query_all(stmt(
                "SELECT \"key\" AS entry_key, size_bytes FROM module_storage \
                 WHERE user_id = ? AND module_id = ? ORDER BY \"key\" ASC",
                vec![user_id.into(), module_id.into()],
            ))
            .await
            .map_err(db_error)?
            .iter()
            .map(|row| {
                Ok(ModuleStorageEntry {
                    key: row.try_get("", "entry_key").map_err(db_error)?,
                    size_bytes: row
                        .try_get::<i64>("", "size_bytes")
                        .map_err(db_error)?
                        .max(0) as u64,
                })
            })
            .collect()
    }

    /// Account deletion (FR-008, D093): remove everything the user's modules
    /// stored. Returns the number of removed keys.
    pub async fn purge_user(&self, user_id: &str) -> DgosResult<u64> {
        module_storage::Entity::delete_many()
            .filter(module_storage::Column::UserId.eq(user_id))
            .exec(&self.db)
            .await
            .map(|result| result.rows_affected)
            .map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::Migrator;
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;
    use serde_json::json;

    const USER: &str = "default-user";

    async fn repository() -> ModuleStorageRepository {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        ModuleStorageRepository::new(db)
    }

    /// A JSON string value whose serialization is exactly `bytes` long.
    fn sized(bytes: usize) -> serde_json::Value {
        serde_json::Value::String("x".repeat(bytes - 2))
    }

    #[tokio::test]
    async fn ac12_a_value_round_trips_and_a_missing_key_reads_as_none() {
        let repo = repository().await;
        assert_eq!(repo.get(USER, "mod-a", "k").await.unwrap(), None);
        let value = json!({"n": 1, "list": [true, null, "é"], "nested": {"a": 1.5}});
        repo.set(USER, "mod-a", "k", &value).await.unwrap();
        assert_eq!(repo.get(USER, "mod-a", "k").await.unwrap(), Some(value));
        // Scalars and null are values too, and a string that looks like a number stays a string.
        for scalar in [json!(null), json!(0), json!("123"), json!(false), json!([])] {
            repo.set(USER, "mod-a", "s", &scalar).await.unwrap();
            assert_eq!(repo.get(USER, "mod-a", "s").await.unwrap(), Some(scalar));
        }
    }

    #[tokio::test]
    async fn ac12_key_limits_are_scalar_values_without_control_characters() {
        let repo = repository().await;
        let ok = json!(1);
        repo.set(USER, "m", "k", &ok).await.unwrap();
        repo.set(USER, "m", &"a".repeat(128), &ok).await.unwrap();
        // 128 scalar values, 4 bytes each: the limit counts characters, not bytes.
        repo.set(USER, "m", &"😀".repeat(128), &ok).await.unwrap();
        repo.set(USER, "m", " spaced / key ", &ok).await.unwrap();
        for bad in [
            String::new(),
            "a".repeat(129),
            "😀".repeat(129),
            "tab\tkey".into(),
            "line\nbreak".into(),
            "nul\0".into(),
            "\u{7f}".into(),
            "\u{85}".into(),
        ] {
            assert!(
                matches!(
                    repo.set(USER, "m", &bad, &ok).await,
                    Err(DgosError::ModuleCapabilityInputInvalid)
                ),
                "set accepted {bad:?}"
            );
            assert!(matches!(
                repo.get(USER, "m", &bad).await,
                Err(DgosError::ModuleCapabilityInputInvalid)
            ));
            assert!(matches!(
                repo.delete(USER, "m", &bad).await,
                Err(DgosError::ModuleCapabilityInputInvalid)
            ));
        }
        assert_eq!(repo.list(USER, "m").await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn ac12_a_value_is_limited_to_64_kib_of_serialized_json() {
        let repo = repository().await;
        let at_limit = sized(MAX_STORAGE_VALUE_BYTES);
        repo.set(USER, "m", "big", &at_limit).await.unwrap();
        assert_eq!(repo.get(USER, "m", "big").await.unwrap(), Some(at_limit));
        let over = sized(MAX_STORAGE_VALUE_BYTES + 1);
        assert!(matches!(
            repo.set(USER, "m", "bigger", &over).await,
            Err(DgosError::ModuleCapabilityInputInvalid)
        ));
        // A refused write leaves nothing behind, and a replacement that is too
        // large keeps the previous value.
        assert_eq!(repo.get(USER, "m", "bigger").await.unwrap(), None);
        assert!(matches!(
            repo.set(USER, "m", "big", &over).await,
            Err(DgosError::ModuleCapabilityInputInvalid)
        ));
        assert_eq!(
            repo.get(USER, "m", "big").await.unwrap(),
            Some(sized(MAX_STORAGE_VALUE_BYTES))
        );
        // The size is the serialized byte length, so multi-byte text counts in bytes.
        let multibyte = serde_json::Value::String("é".repeat(MAX_STORAGE_VALUE_BYTES / 2));
        assert!(matches!(
            repo.set(USER, "m", "wide", &multibyte).await,
            Err(DgosError::ModuleCapabilityInputInvalid)
        ));
    }

    #[tokio::test]
    async fn ac12_the_quota_is_5_mib_per_user_and_module_and_replacement_counts_the_difference() {
        let repo = repository().await;
        // 80 keys of exactly 64 KiB make exactly 5 MiB.
        for index in 0..80 {
            repo.set(
                USER,
                "m",
                &format!("k{index:02}"),
                &sized(MAX_STORAGE_VALUE_BYTES),
            )
            .await
            .unwrap();
        }
        let used: u64 = repo
            .list(USER, "m")
            .await
            .unwrap()
            .iter()
            .map(|e| e.size_bytes)
            .sum();
        assert_eq!(used, MAX_STORAGE_TOTAL_BYTES as u64);

        // One more byte, in a new key, is over.
        assert!(matches!(
            repo.set(USER, "m", "extra", &json!(1)).await,
            Err(DgosError::ModuleStorageQuotaExceeded)
        ));
        assert_eq!(repo.get(USER, "m", "extra").await.unwrap(), None);

        // Replacing a full-size value with an equal or smaller one fits (new minus old).
        repo.set(USER, "m", "k00", &sized(MAX_STORAGE_VALUE_BYTES))
            .await
            .unwrap();
        repo.set(USER, "m", "k00", &sized(10)).await.unwrap();
        // The freed room is usable, up to the byte.
        repo.set(USER, "m", "extra", &sized(MAX_STORAGE_VALUE_BYTES - 10))
            .await
            .unwrap();
        // Growing a replacement beyond the room left is refused and keeps the old value.
        assert!(matches!(
            repo.set(USER, "m", "k00", &sized(11)).await,
            Err(DgosError::ModuleStorageQuotaExceeded)
        ));
        assert_eq!(repo.get(USER, "m", "k00").await.unwrap(), Some(sized(10)));

        // Deleting frees room again.
        repo.delete(USER, "m", "extra").await.unwrap();
        repo.set(USER, "m", "k00", &sized(MAX_STORAGE_VALUE_BYTES))
            .await
            .unwrap();

        // Other modules and other users have their own quota.
        repo.set(USER, "other-module", "k", &sized(MAX_STORAGE_VALUE_BYTES))
            .await
            .unwrap();
        repo.set("someone-else", "m", "k", &sized(MAX_STORAGE_VALUE_BYTES))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn ac12_storage_is_isolated_across_users_and_modules() {
        let repo = repository().await;
        repo.set("alice", "mod-a", "k", &json!("alice-a"))
            .await
            .unwrap();
        repo.set("alice", "mod-b", "k", &json!("alice-b"))
            .await
            .unwrap();
        repo.set("bob", "mod-a", "k", &json!("bob-a"))
            .await
            .unwrap();
        assert_eq!(
            repo.get("alice", "mod-a", "k").await.unwrap(),
            Some(json!("alice-a"))
        );
        assert_eq!(
            repo.get("alice", "mod-b", "k").await.unwrap(),
            Some(json!("alice-b"))
        );
        assert_eq!(
            repo.get("bob", "mod-a", "k").await.unwrap(),
            Some(json!("bob-a"))
        );
        assert_eq!(repo.get("bob", "mod-b", "k").await.unwrap(), None);

        repo.delete("alice", "mod-a", "k").await.unwrap();
        assert_eq!(repo.get("alice", "mod-a", "k").await.unwrap(), None);
        assert_eq!(
            repo.get("alice", "mod-b", "k").await.unwrap(),
            Some(json!("alice-b"))
        );
        assert_eq!(
            repo.get("bob", "mod-a", "k").await.unwrap(),
            Some(json!("bob-a"))
        );
        assert_eq!(repo.list("bob", "mod-b").await.unwrap(), vec![]);
        assert_eq!(repo.list("alice", "mod-a").await.unwrap(), vec![]);
    }

    #[tokio::test]
    async fn ac12_delete_is_idempotent() {
        let repo = repository().await;
        repo.delete(USER, "m", "never-written").await.unwrap();
        repo.set(USER, "m", "k", &json!(1)).await.unwrap();
        repo.delete(USER, "m", "k").await.unwrap();
        repo.delete(USER, "m", "k").await.unwrap();
        assert_eq!(repo.get(USER, "m", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn ac12_list_is_key_ascending_with_sizes_and_never_values() {
        let repo = repository().await;
        for (key, value) in [
            ("b", json!("two")),
            ("a", json!(1)),
            ("é", json!({"x": 1})),
            ("B", json!(null)),
            ("😀", json!([1, 2])),
            ("a1", json!("secret-value-list-check")),
        ] {
            repo.set(USER, "m", key, &value).await.unwrap();
        }
        let listed = repo.list(USER, "m").await.unwrap();
        // Order is by Unicode scalar value: uppercase, lowercase, then non-ASCII.
        assert_eq!(
            listed.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            ["B", "a", "a1", "b", "é", "😀"]
        );
        let sizes: Vec<_> = listed.iter().map(|e| e.size_bytes).collect();
        assert_eq!(sizes, [4, 1, 25, 5, 7, 5]);
        assert!(!format!("{listed:?}").contains("secret-value-list-check"));
        // Replacing a value updates its size.
        repo.set(USER, "m", "a", &json!("longer")).await.unwrap();
        let after = repo.list(USER, "m").await.unwrap();
        assert_eq!(after.iter().find(|e| e.key == "a").unwrap().size_bytes, 8);
    }

    #[tokio::test]
    async fn ac12_purge_user_removes_only_that_users_storage() {
        let repo = repository().await;
        for module in ["mod-a", "mod-b"] {
            repo.set("alice", module, "k1", &json!(1)).await.unwrap();
            repo.set("alice", module, "k2", &json!(2)).await.unwrap();
            repo.set("bob", module, "k1", &json!(3)).await.unwrap();
        }
        assert_eq!(repo.purge_user("alice").await.unwrap(), 4);
        assert!(repo.list("alice", "mod-a").await.unwrap().is_empty());
        assert!(repo.list("alice", "mod-b").await.unwrap().is_empty());
        assert_eq!(
            repo.get("bob", "mod-a", "k1").await.unwrap(),
            Some(json!(3))
        );
        assert_eq!(
            repo.get("bob", "mod-b", "k1").await.unwrap(),
            Some(json!(3))
        );
        assert_eq!(repo.purge_user("alice").await.unwrap(), 0);
        assert_eq!(repo.purge_user("nobody").await.unwrap(), 0);
    }
}
