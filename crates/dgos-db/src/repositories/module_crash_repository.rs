//! Persistence for post-mount module crashes (D055).
//!
//! Callers pass already-sanitized values; this repository still bounds what
//! it stores, so a caller bug cannot write unbounded text.

use crate::entities::module_crash_logs;
use dgos_types::{DgosError, DgosResult};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use uuid::Uuid;

/// Newest crashes kept per module.
pub const MAX_CRASHES_PER_MODULE: u64 = 100;
const MAX_STACK_BYTES: usize = 8 * 1024;

pub struct CrashRecord<'a> {
    pub user_id: &'a str,
    pub module_id: &'a str,
    pub module_version: &'a str,
    pub runtime_id: &'a str,
    pub source: &'a str,
    pub error_name: &'a str,
    pub stack: &'a str,
}

#[derive(Clone)]
pub struct ModuleCrashRepository {
    db: DatabaseConnection,
}

fn db_error(error: sea_orm::DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}

fn truncated(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

impl ModuleCrashRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Store one crash and drop the oldest beyond the per-module bound.
    /// Returns the new crash id.
    pub async fn record(&self, crash: CrashRecord<'_>) -> DgosResult<String> {
        let id = Uuid::new_v4().to_string();
        module_crash_logs::Entity::insert(module_crash_logs::ActiveModel {
            id: Set(id.clone()),
            user_id: Set(crash.user_id.to_owned()),
            module_id: Set(crash.module_id.to_owned()),
            module_version: Set(crash.module_version.to_owned()),
            runtime_id: Set(crash.runtime_id.to_owned()),
            source: Set(truncated(crash.source, 32).to_owned()),
            error_name: Set(truncated(crash.error_name, 64).to_owned()),
            stack: Set(truncated(crash.stack, MAX_STACK_BYTES).to_owned()),
            created_at: Set(chrono::Utc::now().timestamp()),
        })
        .exec(&self.db)
        .await
        .map_err(db_error)?;

        let stale: Vec<_> = module_crash_logs::Entity::find()
            .filter(module_crash_logs::Column::UserId.eq(crash.user_id))
            .filter(module_crash_logs::Column::ModuleId.eq(crash.module_id))
            .order_by_desc(module_crash_logs::Column::CreatedAt)
            .order_by_desc(module_crash_logs::Column::Id)
            .all(&self.db)
            .await
            .map_err(db_error)?
            .into_iter()
            .skip(MAX_CRASHES_PER_MODULE as usize)
            .collect();
        if !stale.is_empty() {
            module_crash_logs::Entity::delete_many()
                .filter(module_crash_logs::Column::Id.is_in(stale.into_iter().map(|row| row.id)))
                .exec(&self.db)
                .await
                .map_err(db_error)?;
        }
        Ok(id)
    }

    /// Newest first.
    pub async fn list(
        &self,
        user_id: &str,
        module_id: &str,
        limit: u64,
    ) -> DgosResult<Vec<module_crash_logs::Model>> {
        module_crash_logs::Entity::find()
            .filter(module_crash_logs::Column::UserId.eq(user_id))
            .filter(module_crash_logs::Column::ModuleId.eq(module_id))
            .order_by_desc(module_crash_logs::Column::CreatedAt)
            .order_by_desc(module_crash_logs::Column::Id)
            .limit(limit.min(MAX_CRASHES_PER_MODULE))
            .all(&self.db)
            .await
            .map_err(db_error)
    }

    /// Crashes of one module version since `since` (Unix seconds).
    pub async fn count_since(
        &self,
        user_id: &str,
        module_id: &str,
        module_version: Option<&str>,
        since: i64,
    ) -> DgosResult<u64> {
        let mut query = module_crash_logs::Entity::find()
            .filter(module_crash_logs::Column::UserId.eq(user_id))
            .filter(module_crash_logs::Column::ModuleId.eq(module_id))
            .filter(module_crash_logs::Column::CreatedAt.gte(since));
        if let Some(version) = module_version {
            query = query.filter(module_crash_logs::Column::ModuleVersion.eq(version));
        }
        query.count(&self.db).await.map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::Migrator;
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    async fn repository() -> ModuleCrashRepository {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        ModuleCrashRepository::new(db)
    }

    fn crash<'a>(module: &'a str, version: &'a str, stack: &'a str) -> CrashRecord<'a> {
        CrashRecord {
            user_id: "default-user",
            module_id: module,
            module_version: version,
            runtime_id: "rt-1",
            source: "render",
            error_name: "TypeError",
            stack,
        }
    }

    #[tokio::test]
    async fn a_crash_is_stored_with_its_stack_and_a_timestamp() {
        let repo = repository().await;
        let before = chrono::Utc::now().timestamp();
        repo.record(crash("mod-a", "1.0.0", "render@module:1:2\n"))
            .await
            .unwrap();
        let rows = repo.list("default-user", "mod-a", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stack, "render@module:1:2\n");
        assert_eq!(rows[0].error_name, "TypeError");
        assert!(rows[0].created_at >= before);
        // Scoped by user and module.
        assert!(repo
            .list("someone-else", "mod-a", 10)
            .await
            .unwrap()
            .is_empty());
        assert!(repo
            .list("default-user", "mod-b", 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn history_is_bounded_per_module_and_oversized_stacks_are_cut() {
        let repo = repository().await;
        let huge = "f@module:1:1\n".repeat(2000);
        for _ in 0..(MAX_CRASHES_PER_MODULE + 5) {
            repo.record(crash("mod-a", "1.0.0", &huge)).await.unwrap();
        }
        repo.record(crash("mod-b", "1.0.0", "g@module:1:1\n"))
            .await
            .unwrap();
        let kept = repo.list("default-user", "mod-a", 1000).await.unwrap();
        assert_eq!(kept.len() as u64, MAX_CRASHES_PER_MODULE);
        assert!(kept.iter().all(|row| row.stack.len() <= 8 * 1024));
        assert_eq!(
            repo.list("default-user", "mod-b", 10).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn crashes_are_counted_per_version_inside_a_window() {
        let repo = repository().await;
        for version in ["1.0.0", "1.0.0", "2.0.0"] {
            repo.record(crash("mod-a", version, "f@module:1:1\n"))
                .await
                .unwrap();
        }
        let since = chrono::Utc::now().timestamp() - 60;
        let count = |version| repo.count_since("default-user", "mod-a", version, since);
        assert_eq!(count(Some("1.0.0")).await.unwrap(), 2);
        assert_eq!(count(Some("2.0.0")).await.unwrap(), 1);
        assert_eq!(count(None).await.unwrap(), 3);
        let future = chrono::Utc::now().timestamp() + 60;
        assert_eq!(
            repo.count_since("default-user", "mod-a", None, future)
                .await
                .unwrap(),
            0
        );
    }
}
