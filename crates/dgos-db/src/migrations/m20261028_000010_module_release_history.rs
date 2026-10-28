use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
            CREATE TABLE module_versions (
                id TEXT PRIMARY KEY NOT NULL,
                module_id TEXT NOT NULL,
                version VARCHAR(50) NOT NULL,
                integrity CHAR(64) NOT NULL,
                entry_path TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                UNIQUE(module_id, version),
                FOREIGN KEY(module_id) REFERENCES modules(id)
            );
            CREATE TABLE module_update_events (
                id TEXT PRIMARY KEY NOT NULL,
                module_id TEXT NOT NULL,
                from_version VARCHAR(50),
                to_version VARCHAR(50) NOT NULL,
                stage VARCHAR(32) NOT NULL,
                outcome VARCHAR(24) NOT NULL,
                error_class VARCHAR(80),
                created_at BIGINT NOT NULL
            );
            CREATE INDEX idx_module_update_events_module_time
                ON module_update_events(module_id, created_at);
            "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Module release history is forward-only".into(),
        ))
    }
}
