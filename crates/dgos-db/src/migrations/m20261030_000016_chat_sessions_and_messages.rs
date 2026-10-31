use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // V1-FR-006 (D072, D073, D077). Additive and forward-only. Chat text lives only in
        // `chat_*`; `workbench_tasks` stays the content-free audit and gains three columns.
        // `user_id` is host-derived TEXT without a foreign key (there is no `users` table); the
        // composite key keeps a message and its session under one user. Provider, model and skill
        // references on a session are plain text so a deleted Provider never blocks reading.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE chat_sessions (
                    id TEXT PRIMARY KEY NOT NULL,
                    user_id TEXT NOT NULL,
                    title TEXT NOT NULL,
                    title_source TEXT NOT NULL DEFAULT 'auto' CHECK (title_source IN ('auto','user')),
                    provider_id TEXT,
                    model_id TEXT,
                    skill_id TEXT,
                    skill_version_id TEXT,
                    skill_content_hash TEXT,
                    no_skill_confirmed INTEGER NOT NULL DEFAULT 0 CHECK (no_skill_confirmed IN (0,1)),
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL,
                    UNIQUE (id, user_id)
                );
                CREATE INDEX idx_chat_sessions_user_updated
                    ON chat_sessions (user_id, updated_at DESC, id DESC);

                CREATE TABLE chat_messages (
                    id TEXT PRIMARY KEY NOT NULL,
                    session_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    seq BIGINT NOT NULL,
                    role TEXT NOT NULL CHECK (role IN ('user','assistant')),
                    content TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL CHECK (status IN ('generating','completed','interrupted','failed')),
                    task_id TEXT,
                    replaces_id TEXT,
                    finish_reason TEXT,
                    error_code TEXT,
                    created_at BIGINT NOT NULL,
                    updated_at BIGINT NOT NULL,
                    FOREIGN KEY (session_id, user_id) REFERENCES chat_sessions (id, user_id) ON DELETE CASCADE,
                    UNIQUE (session_id, seq)
                );
                CREATE UNIQUE INDEX idx_chat_messages_one_generating
                    ON chat_messages (user_id) WHERE role = 'assistant' AND status = 'generating';
                CREATE INDEX idx_chat_messages_task
                    ON chat_messages (user_id, task_id) WHERE task_id IS NOT NULL;

                ALTER TABLE workbench_tasks ADD COLUMN session_id TEXT;
                ALTER TABLE workbench_tasks ADD COLUMN kind TEXT NOT NULL DEFAULT 'single'
                    CHECK (kind IN ('single','chat'));
                ALTER TABLE workbench_tasks ADD COLUMN endpoint_fingerprint TEXT;
                CREATE INDEX idx_workbench_tasks_session
                    ON workbench_tasks (session_id) WHERE session_id IS NOT NULL;
                "#,
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Chat tables are forward-only".into()))
    }
}
