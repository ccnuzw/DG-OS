use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite-compatible additive migration. Runtime connection handles and
        // secrets are intentionally not persisted here.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS mcp_servers (
                    id TEXT PRIMARY KEY NOT NULL,
                    user_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    transport TEXT NOT NULL CHECK (transport IN ('stdio', 'http')),
                    config_json TEXT NOT NULL,
                    credential_ref TEXT,
                    enabled BOOLEAN NOT NULL DEFAULT 0,
                    auto_connect BOOLEAN NOT NULL DEFAULT 0,
                    status TEXT NOT NULL DEFAULT 'configured',
                    last_error_class TEXT,
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL,
                    deleted_at BIGINT
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_mcp_servers_user_name
                    ON mcp_servers(user_id, name) WHERE deleted_at IS NULL;
                CREATE INDEX IF NOT EXISTS idx_mcp_servers_user_enabled
                    ON mcp_servers(user_id, enabled) WHERE deleted_at IS NULL;
                CREATE TABLE IF NOT EXISTS mcp_server_tools (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    server_id TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    title TEXT,
                    description TEXT,
                    input_schema_json TEXT NOT NULL,
                    output_schema_json TEXT,
                    schema_hash TEXT NOT NULL,
                    discovered_at BIGINT NOT NULL,
                    UNIQUE(server_id, tool_name),
                    FOREIGN KEY(server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_mcp_server_tools_server
                    ON mcp_server_tools(server_id);
                CREATE TABLE IF NOT EXISTS mcp_usage_logs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    user_id TEXT NOT NULL,
                    server_id TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    task_ref TEXT,
                    request_hash TEXT,
                    outcome TEXT NOT NULL,
                    error_class TEXT,
                    duration_ms INTEGER,
                    created_at BIGINT NOT NULL,
                    FOREIGN KEY(server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_mcp_usage_logs_user_created
                    ON mcp_usage_logs(user_id, created_at);
                CREATE INDEX IF NOT EXISTS idx_mcp_usage_logs_server_created
                    ON mcp_usage_logs(server_id, created_at);",
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Keep the migration non-destructive for local user data.
        Ok(())
    }
}
