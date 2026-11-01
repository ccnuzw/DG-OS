use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // V1-FR-008 (D087, D090). `users.id` is TEXT: the first account is
        // `default-user`, so every existing row keeps its owner. No table gets a
        // foreign key to `users`; account deletion purges each domain
        // explicitly. There is no session table: desktop sessions live only in
        // host memory.
        //
        // `host_settings` holds settings that belong to no user (last signed-in
        // user, LAN switch and port). `mcp_credential_cleanup` is the MCP
        // twin of `provider_credential_cleanup`: a vault item is listed here
        // before its row goes away and stays listed until the vault removal
        // succeeds.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE users (
                    id TEXT PRIMARY KEY NOT NULL,
                    username TEXT NOT NULL,
                    username_key TEXT NOT NULL UNIQUE,
                    password_hash TEXT NOT NULL,
                    failed_attempts INTEGER NOT NULL DEFAULT 0,
                    locked_until BIGINT,
                    last_sign_in_at BIGINT,
                    deleting_since BIGINT,
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL
                );
                CREATE TABLE host_settings (
                    setting_key TEXT PRIMARY KEY NOT NULL,
                    setting_value TEXT NOT NULL,
                    updated_at BIGINT NOT NULL
                );
                CREATE TABLE mcp_credential_cleanup (
                    credential_ref TEXT PRIMARY KEY NOT NULL,
                    user_id TEXT NOT NULL,
                    server_id TEXT NOT NULL,
                    endpoint TEXT NOT NULL
                );
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP TABLE IF EXISTS mcp_credential_cleanup;
                 DROP TABLE IF EXISTS host_settings;
                 DROP TABLE IF EXISTS users;",
            )
            .await
            .map(|_| ())
    }
}
