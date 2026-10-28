use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS mcp_connection_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT NOT NULL,
                server_id TEXT NOT NULL,
                event TEXT NOT NULL,
                status TEXT NOT NULL,
                error_class TEXT,
                duration_ms INTEGER,
                reconnect_attempt INTEGER,
                created_at BIGINT NOT NULL,
                FOREIGN KEY(server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_mcp_connection_logs_server_created
                ON mcp_connection_logs(server_id, created_at);",
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
