use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Per-user, per-module key-value store behind `storage.read` /
        // `storage.write` (D093). `value` is the JSON text exactly as counted
        // by `size_bytes`; it is TEXT (not JSON) so SQLite never applies a
        // numeric affinity to it. There is no foreign key: uninstall and
        // account deletion remove the rows explicitly.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE IF NOT EXISTS module_storage (
                    user_id TEXT NOT NULL,
                    module_id TEXT NOT NULL,
                    key TEXT NOT NULL,
                    value TEXT NOT NULL,
                    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
                    updated_at BIGINT NOT NULL,
                    PRIMARY KEY (user_id, module_id, key)
                );
                CREATE INDEX IF NOT EXISTS idx_module_storage_module
                    ON module_storage(module_id);
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS module_storage;")
            .await
            .map(|_| ())
    }
}
