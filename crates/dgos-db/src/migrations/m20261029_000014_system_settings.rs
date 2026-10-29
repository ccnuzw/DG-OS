use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // D065 / V1-FR-005 Phase 3. `user_id` is TEXT NOT NULL like every other
        // user-scoped table (providers, skills, module_crash_logs): the host-derived
        // identity is the string `default-user` (D023) and there is no `users` table
        // to reference, so the data model's INTEGER foreign key cannot be created.
        // NOT NULL (no "global" NULL rows) because SQLite treats NULLs as distinct in
        // a unique index, which would allow duplicate global keys. Defaults are applied
        // by the host when reading; no rows are seeded. Forward-only and additive.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE system_settings (
                    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
                    user_id TEXT NOT NULL,
                    setting_key TEXT NOT NULL,
                    setting_value TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE UNIQUE INDEX idx_system_settings_unique
                    ON system_settings(user_id, setting_key);
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("System settings are forward-only".into()))
    }
}
