use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
CREATE TABLE chat_runs (
 task_id TEXT PRIMARY KEY NOT NULL,
 user_id TEXT NOT NULL,
 session_id TEXT NOT NULL,
 assistant_message_id TEXT NOT NULL,
 source_run_id TEXT,
 kind TEXT NOT NULL DEFAULT 'generation',
 status TEXT NOT NULL DEFAULT 'running',
 partial_content TEXT NOT NULL DEFAULT '',
 dispatched INTEGER NOT NULL DEFAULT 0,
 result_unknown INTEGER NOT NULL DEFAULT 0,
 audit_warning TEXT,
 total_tokens BIGINT,
 request_identity TEXT,
 created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL,
 FOREIGN KEY(session_id,user_id) REFERENCES chat_sessions(id,user_id) ON DELETE CASCADE
);
CREATE INDEX chat_runs_owner_session ON chat_runs(user_id,session_id,created_at);
CREATE TABLE chat_run_steps (
 id TEXT PRIMARY KEY NOT NULL,
 task_id TEXT NOT NULL REFERENCES chat_runs(task_id) ON DELETE CASCADE,
 ordinal INTEGER NOT NULL,
 kind TEXT NOT NULL, phase TEXT NOT NULL, status TEXT NOT NULL,
 dispatched INTEGER NOT NULL DEFAULT 0, result_unknown INTEGER NOT NULL DEFAULT 0,
 receipt TEXT,
 created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL,
 UNIQUE(task_id,ordinal)
);
CREATE TABLE chat_summaries (
 id TEXT PRIMARY KEY NOT NULL,
 user_id TEXT NOT NULL,
 session_id TEXT NOT NULL UNIQUE,
 source_first_seq BIGINT NOT NULL, source_last_seq BIGINT NOT NULL,
 source_digest TEXT NOT NULL, source_json TEXT NOT NULL,
 content TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL,
 created_at BIGINT NOT NULL,
 FOREIGN KEY(session_id,user_id) REFERENCES chat_sessions(id,user_id) ON DELETE CASCADE
);
CREATE TRIGGER chat_run_start AFTER INSERT ON chat_messages WHEN NEW.role='assistant' AND NEW.status='generating' AND NEW.task_id IS NOT NULL BEGIN
 INSERT INTO chat_runs(task_id,user_id,session_id,assistant_message_id,created_at,updated_at)
 VALUES(NEW.task_id,NEW.user_id,NEW.session_id,NEW.id,NEW.created_at,NEW.updated_at);
END;
CREATE TRIGGER chat_run_terminal AFTER UPDATE OF status ON chat_messages
WHEN OLD.status='generating' AND NEW.status<>'generating' BEGIN
 UPDATE chat_runs SET status=NEW.status,partial_content=NEW.content,updated_at=NEW.updated_at,
 total_tokens=(SELECT total_tokens FROM workbench_tasks WHERE id=NEW.task_id AND user_id=NEW.user_id)
 WHERE task_id=NEW.task_id AND user_id=NEW.user_id;
END;
CREATE TRIGGER chat_summary_changed AFTER UPDATE OF content,status ON chat_messages
WHEN OLD.content IS NOT NEW.content OR OLD.status IS NOT NEW.status BEGIN
 DELETE FROM chat_summaries WHERE session_id=OLD.session_id AND user_id=OLD.user_id AND OLD.seq<=source_last_seq;
END;
CREATE TRIGGER chat_summary_deleted AFTER DELETE ON chat_messages BEGIN
 DELETE FROM chat_summaries WHERE session_id=OLD.session_id AND user_id=OLD.user_id AND OLD.seq<=source_last_seq;
END;
CREATE TRIGGER chat_summary_regeneration BEFORE INSERT ON chat_messages WHEN NEW.replaces_id IS NOT NULL BEGIN
 DELETE FROM chat_summaries WHERE session_id=NEW.session_id AND user_id=NEW.user_id;
END;
CREATE TRIGGER chat_content_account_delete BEFORE DELETE ON users BEGIN
 DELETE FROM chat_summaries WHERE user_id=OLD.id;
 DELETE FROM chat_runs WHERE user_id=OLD.id;
END;
INSERT INTO chat_runs(task_id,user_id,session_id,assistant_message_id,status,partial_content,total_tokens,dispatched,result_unknown,audit_warning,created_at,updated_at)
 SELECT m.task_id,m.user_id,m.session_id,m.id,m.status,m.content,t.total_tokens,1,1,'legacy_checkpoint_unavailable',m.created_at,m.updated_at
 FROM chat_messages m LEFT JOIN workbench_tasks t ON t.id=m.task_id AND t.user_id=m.user_id
 WHERE m.role='assistant' AND m.task_id IS NOT NULL;
"#).await.map(|_| ())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("chat summaries and checkpoints are forward-only".into()))
    }
}
