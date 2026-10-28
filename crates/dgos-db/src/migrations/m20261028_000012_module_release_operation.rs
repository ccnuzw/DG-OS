use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Keep the history migration forward-only while making operation
        // identity explicit for databases created before D031 was frozen.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE module_update_events
                    ADD COLUMN operation_id TEXT NOT NULL DEFAULT 'legacy';
                CREATE INDEX idx_module_update_events_operation
                    ON module_update_events(operation_id, created_at);
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Module release operation identity is forward-only".into(),
        ))
    }
}
