use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Independent from module audit/task rows: chat uses the same content domain.
        manager.get_connection().execute_unprepared(r#"
CREATE TABLE ai_content_resources (
 id TEXT PRIMARY KEY NOT NULL,
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 scope TEXT NOT NULL,
 name TEXT NOT NULL, mime TEXT NOT NULL,
 bytes BLOB NOT NULL CHECK(length(bytes)<=8388608),
 sha256 TEXT NOT NULL CHECK(length(sha256)=64),
 created_at BIGINT NOT NULL, expires_at BIGINT NOT NULL
);
CREATE INDEX ai_content_resources_owner ON ai_content_resources(user_id,scope,expires_at);
CREATE TABLE ai_content_runs (
 run_id TEXT PRIMARY KEY NOT NULL,
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 scope TEXT NOT NULL, source_run_id TEXT,
 retain INTEGER NOT NULL CHECK(retain IN (0,1)),
 status TEXT NOT NULL DEFAULT 'running' CHECK(status IN ('running','succeeded','failed','cancelled','interrupted')),
 content TEXT CHECK(content IS NULL OR length(CAST(content AS BLOB))<=65536),
 content_expired INTEGER NOT NULL DEFAULT 0,
 output_valid INTEGER NOT NULL DEFAULT 0,
 audit_warning INTEGER NOT NULL DEFAULT 0,
 context_receipts TEXT NOT NULL DEFAULT '[]', skill_receipts TEXT NOT NULL DEFAULT '[]',
 acknowledged_at BIGINT, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL,
 expires_at BIGINT NOT NULL,
 CHECK(retain=1 OR content IS NULL)
);
CREATE INDEX ai_content_runs_owner ON ai_content_runs(user_id,scope,created_at);
CREATE TABLE ai_content_tool_receipts (
 run_id TEXT NOT NULL REFERENCES ai_content_runs(run_id) ON DELETE CASCADE,
 call_id TEXT NOT NULL, fingerprint TEXT NOT NULL CHECK(length(fingerprint)=64),
 outcome TEXT CHECK(outcome IS NULL OR outcome IN ('succeeded','failed','cancelled','unknown')),
 unknown INTEGER NOT NULL DEFAULT 0, inherited INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(run_id,fingerprint)
);
CREATE UNIQUE INDEX ai_content_tool_call ON ai_content_tool_receipts(run_id,call_id) WHERE inherited=0;
CREATE TRIGGER ai_content_chat_deleted AFTER DELETE ON chat_sessions BEGIN
 DELETE FROM ai_content_resources WHERE user_id=OLD.user_id AND scope='chat:'||OLD.id;
 DELETE FROM ai_content_runs WHERE user_id=OLD.user_id AND scope='chat:'||OLD.id;
END;
"#).await.map(|_| ())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("AI content receipts are forward-only".into()))
    }
}
