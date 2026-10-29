use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // D055. No foreign key to `modules`: crash history stays as an audit
        // trail after an uninstall. `stack` holds host-sanitized frame lines
        // only, never message text, URLs or paths.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE module_crash_logs (
                    id TEXT PRIMARY KEY NOT NULL,
                    user_id TEXT NOT NULL,
                    module_id TEXT NOT NULL,
                    module_version TEXT NOT NULL,
                    runtime_id TEXT NOT NULL,
                    source TEXT NOT NULL,
                    error_name TEXT NOT NULL,
                    stack TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE INDEX idx_module_crash_logs_module
                    ON module_crash_logs(user_id, module_id, created_at);
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Module crash logs are forward-only".into()))
    }
}
