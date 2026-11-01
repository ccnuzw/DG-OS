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
                CREATE TABLE skill_install_operations (
                    operation_id TEXT PRIMARY KEY NOT NULL,
                    user_id TEXT NOT NULL,
                    idempotency_key TEXT NOT NULL UNIQUE,
                    source_kind TEXT NOT NULL CHECK (source_kind IN ('local_zip', 'github')),
                    source_ref TEXT NOT NULL,
                    state TEXT NOT NULL CHECK (state IN (
                        'previewed', 'publishing', 'committed', 'failed', 'abandoned'
                    )),
                    skill_id TEXT,
                    version_id TEXT,
                    content_hash TEXT,
                    staging_relpath TEXT,
                    published_relpath TEXT,
                    metadata TEXT NOT NULL DEFAULT '{}',
                    last_error_key TEXT,
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL
                );
                CREATE INDEX idx_skill_install_operations_user_state
                    ON skill_install_operations (user_id, state, updated_at);
                CREATE INDEX idx_skill_install_operations_hash
                    ON skill_install_operations (user_id, content_hash, state);
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS skill_install_operations;")
            .await
            .map(|_| ())
    }
}
