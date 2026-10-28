use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // This is only the persistence boundary for the broker. It does not
        // install, load, or execute module code.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE IF NOT EXISTS modules (
                    id TEXT PRIMARY KEY NOT NULL,
                    module_name VARCHAR(100) NOT NULL UNIQUE,
                    display_name VARCHAR(100) NOT NULL,
                    current_version VARCHAR(50) NOT NULL,
                    description TEXT,
                    manifest JSON NOT NULL,
                    integrity CHAR(64) NOT NULL,
                    entry_url TEXT,
                    icon_url TEXT,
                    is_official BOOLEAN NOT NULL DEFAULT FALSE,
                    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
                    installed_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_modules_enabled ON modules(is_enabled);
                CREATE TABLE IF NOT EXISTS module_permissions (
                    id TEXT PRIMARY KEY NOT NULL,
                    user_id TEXT NOT NULL,
                    module_id TEXT NOT NULL,
                    permission VARCHAR(40) NOT NULL CHECK (permission IN (
                        'providers.call', 'skills.execute', 'mcp.call',
                        'storage.read', 'storage.write', 'network.local'
                    )),
                    granted BOOLEAN NOT NULL DEFAULT FALSE,
                    granted_at BIGINT,
                    revoked_at BIGINT,
                    UNIQUE(user_id, module_id, permission),
                    FOREIGN KEY(module_id) REFERENCES modules(id)
                );
                CREATE INDEX IF NOT EXISTS idx_module_permissions_user
                    ON module_permissions(user_id, module_id);
                CREATE TABLE IF NOT EXISTS module_permission_audits (
                    id TEXT PRIMARY KEY NOT NULL,
                    user_id TEXT NOT NULL,
                    module_id TEXT NOT NULL,
                    permission VARCHAR(40) NOT NULL CHECK (permission IN (
                        'providers.call', 'skills.execute', 'mcp.call',
                        'storage.read', 'storage.write', 'network.local'
                    )),
                    action VARCHAR(16) NOT NULL CHECK (action IN ('granted', 'revoked', 'denied')),
                    resource_id TEXT,
                    request_id TEXT,
                    created_at BIGINT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_module_permission_audits_user_time
                    ON module_permission_audits(user_id, created_at);
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP TABLE IF EXISTS module_permission_audits; DROP TABLE IF EXISTS module_permissions; DROP TABLE IF EXISTS modules;",
            )
            .await
            .map(|_| ())
    }
}
