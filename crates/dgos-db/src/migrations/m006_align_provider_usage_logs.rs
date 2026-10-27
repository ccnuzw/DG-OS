use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // D019: keep the historical table and copy, never rename/drop it.
        manager.get_connection().execute_unprepared(
            "CREATE TABLE IF NOT EXISTS provider_usage_logs (
                id TEXT PRIMARY KEY NOT NULL,
                provider_id TEXT NOT NULL REFERENCES providers(id),
                model_id TEXT NOT NULL, user_id TEXT NOT NULL,
                request_tokens INTEGER NOT NULL DEFAULT 0,
                response_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                success BOOLEAN NOT NULL, error_code TEXT, created_at TEXT NOT NULL
            );
            INSERT INTO provider_usage_logs
                (id, provider_id, model_id, user_id, request_tokens, response_tokens,
                 total_tokens, latency_ms, success, error_code, created_at)
            SELECT old.id, old.provider_id, old.model_id, old.user_id, old.request_tokens, old.response_tokens,
                   old.total_tokens, old.latency_ms, old.success, old.error_code, old.created_at FROM usage_logs AS old
            INNER JOIN providers AS provider ON provider.id = old.provider_id
            ON CONFLICT(id) DO NOTHING;
            CREATE INDEX IF NOT EXISTS idx_provider_usage_user_time ON provider_usage_logs(user_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_provider_usage_provider_time ON provider_usage_logs(provider_id, created_at);"
        ).await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE provider_usage_logs;")
            .await?;
        Ok(())
    }
}
