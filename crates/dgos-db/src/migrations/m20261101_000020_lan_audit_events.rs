use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // V1-FR-010 (D099 item 2, data model §4.8). At most 500 rows in the
        // whole table, trimmed in the insert transaction. Only fixed enums, the
        // peer IPv4, the session short id, the bound user and the error code:
        // never access codes, tokens or their hashes, cookies, the User-Agent,
        // device labels, paths, bodies, chat content or prompts. `user_id` is
        // NULL for rejections before a session exists; account deletion
        // removes the user's rows and keeps the NULL rows.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE lan_audit_events (
                    id TEXT PRIMARY KEY NOT NULL,
                    first_at BIGINT NOT NULL,
                    last_at BIGINT NOT NULL,
                    event TEXT NOT NULL CHECK (event IN (
                        'pair_succeeded', 'pair_rejected', 'pair_rate_limited',
                        'code_voided', 'session_revoked', 'request_rejected'
                    )),
                    reason TEXT,
                    peer_ip TEXT,
                    session_short_id TEXT,
                    user_id TEXT,
                    error_code INTEGER,
                    count INTEGER NOT NULL DEFAULT 1
                );
                CREATE INDEX idx_lan_audit_merge ON lan_audit_events (event, reason, peer_ip, last_at);
                CREATE INDEX idx_lan_audit_user ON lan_audit_events (user_id);
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS lan_audit_events;")
            .await
            .map(|_| ())
    }
}
