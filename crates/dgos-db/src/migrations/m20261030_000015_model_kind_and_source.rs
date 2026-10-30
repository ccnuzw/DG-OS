use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // V1-FR-001 §4.3 (D063). Additive and forward-only. Existing rows become
        // `source = 'remote'`; `kind`, `kind_source` and `max_output_tokens` stay NULL and
        // are derived by the reader without being written back.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE models ADD COLUMN source TEXT NOT NULL DEFAULT 'remote';
                ALTER TABLE models ADD COLUMN kind TEXT;
                ALTER TABLE models ADD COLUMN kind_source TEXT;
                ALTER TABLE models ADD COLUMN max_output_tokens INTEGER;
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Model kind columns are forward-only".into()))
    }
}
