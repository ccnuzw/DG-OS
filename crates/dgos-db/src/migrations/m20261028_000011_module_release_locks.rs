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
            CREATE TABLE module_release_locks (
                lock_key VARCHAR(128) PRIMARY KEY NOT NULL,
                owner_id VARCHAR(64) NOT NULL,
                acquired_at BIGINT NOT NULL,
                expires_at BIGINT NOT NULL
            );
            CREATE INDEX idx_module_release_locks_expiry
                ON module_release_locks(expires_at);
            "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Module release locks are forward-only".into(),
        ))
    }
}
