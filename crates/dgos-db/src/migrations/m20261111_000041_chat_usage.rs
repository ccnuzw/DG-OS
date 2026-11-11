use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Session-owned facts deliberately have no Provider or task-audit FK. Removing a
        // Provider may remove its audit logs, but must not change a retained chat's usage.
        manager.get_connection().execute_unprepared(r#"
CREATE TABLE chat_usage_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('measured','legacy_log','legacy_total')),
    status TEXT NOT NULL CHECK(status IN ('pending','success','failure','interrupted','unknown')),
    input_tokens BIGINT CHECK(input_tokens IS NULL OR (typeof(input_tokens)='integer' AND input_tokens>=0)),
    output_tokens BIGINT CHECK(output_tokens IS NULL OR (typeof(output_tokens)='integer' AND output_tokens>=0)),
    total_tokens BIGINT CHECK(total_tokens IS NULL OR (typeof(total_tokens)='integer' AND total_tokens>=0)),
    cache_read_tokens BIGINT CHECK(cache_read_tokens IS NULL OR (typeof(cache_read_tokens)='integer' AND cache_read_tokens>=0)),
    cache_write_tokens BIGINT CHECK(cache_write_tokens IS NULL OR (typeof(cache_write_tokens)='integer' AND cache_write_tokens>=0)),
    reasoning_tokens BIGINT CHECK(reasoning_tokens IS NULL OR (typeof(reasoning_tokens)='integer' AND reasoning_tokens>=0)),
    duration_ms BIGINT CHECK(duration_ms IS NULL OR (typeof(duration_ms)='integer' AND duration_ms>=0)),
    normalization_version INTEGER NOT NULL DEFAULT 1 CHECK(normalization_version=1),
    created_at BIGINT NOT NULL,
    settled_at BIGINT,
    FOREIGN KEY(session_id,user_id) REFERENCES chat_sessions(id,user_id) ON DELETE CASCADE,
    CHECK((status='pending' AND settled_at IS NULL) OR (status<>'pending' AND settled_at IS NOT NULL)),
    CHECK(status<>'pending' OR (input_tokens IS NULL AND output_tokens IS NULL AND total_tokens IS NULL AND cache_read_tokens IS NULL AND cache_write_tokens IS NULL AND reasoning_tokens IS NULL AND duration_ms IS NULL))
);
CREATE INDEX chat_usage_owner_session ON chat_usage_attempts(user_id,session_id);
CREATE INDEX chat_usage_latest ON chat_usage_attempts(user_id,session_id,created_at DESC,id DESC) WHERE status<>'pending';
CREATE INDEX chat_usage_run ON chat_usage_attempts(user_id,run_id);
CREATE UNIQUE INDEX chat_usage_one_legacy_total ON chat_usage_attempts(user_id,run_id) WHERE source='legacy_total';
CREATE TRIGGER chat_usage_source BEFORE INSERT ON chat_usage_attempts
WHEN NOT EXISTS(SELECT 1 FROM chat_runs r WHERE r.task_id=NEW.run_id AND r.user_id=NEW.user_id AND r.session_id=NEW.session_id)
 AND (NEW.source='measured' OR NOT EXISTS(
     SELECT 1 FROM chat_messages m JOIN workbench_tasks t ON t.id=m.task_id AND t.user_id=m.user_id AND t.kind='chat' AND t.session_id=m.session_id
     WHERE m.task_id=NEW.run_id AND m.user_id=NEW.user_id AND m.session_id=NEW.session_id AND m.role='assistant'))
BEGIN SELECT RAISE(ABORT,'invalid chat usage owner'); END;
CREATE TRIGGER chat_usage_identity_immutable BEFORE UPDATE OF id,user_id,session_id,run_id,provider_id,model_id,source,normalization_version,created_at ON chat_usage_attempts
BEGIN SELECT RAISE(ABORT,'immutable chat usage identity'); END;
CREATE TRIGGER chat_usage_terminal_immutable BEFORE UPDATE ON chat_usage_attempts WHEN OLD.status<>'pending'
BEGIN SELECT RAISE(ABORT,'chat usage already settled'); END;
-- Usage timestamps are epoch milliseconds; existing chat run/message timestamps are seconds.
-- Only explicit startup/orphan recovery seals pending attempts as unknown. Ordinary live
-- terminalization must not race the Provider's owned audit/fact settlement barrier.
CREATE TRIGGER chat_usage_account_delete BEFORE DELETE ON users
BEGIN DELETE FROM chat_usage_attempts WHERE user_id=OLD.id; END;

-- A valid run total wins: old audit writes could fail after one of several calls, so
-- retained logs cannot establish complete coverage. Logs supply a known subtotal only
-- when there is no owned run/task total. No temporal/model-name association is inferred.
WITH historical_runs AS (
    SELECT task_id,user_id,session_id,total_tokens,created_at,updated_at FROM chat_runs
    UNION ALL
    SELECT m.task_id,m.user_id,m.session_id,t.total_tokens,m.created_at,m.updated_at
    FROM chat_messages m JOIN workbench_tasks t ON t.id=m.task_id AND t.user_id=m.user_id AND t.kind='chat' AND t.session_id=m.session_id
    WHERE m.role='assistant' AND NOT EXISTS(SELECT 1 FROM chat_runs r WHERE r.task_id=m.task_id)
      AND m.id=(SELECT last.id FROM chat_messages last WHERE last.task_id=m.task_id AND last.user_id=m.user_id AND last.role='assistant' ORDER BY last.seq DESC LIMIT 1)
), historical_logs AS (
    -- SeaORM's ISO/RFC3339 SQLite datetimes are parsed with their timezone. Zero is
    -- an explicitly unavailable timestamp when malformed, never the run's guessed date.
    SELECT l.*,MAX(0,COALESCE(CAST(ROUND((julianday(l.created_at)-2440587.5)*86400000) AS INTEGER),0)) AS reported_at_ms
    FROM provider_usage_logs l
)
INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,input_tokens,output_tokens,total_tokens,duration_ms,created_at,settled_at)
SELECT 'legacy-log:'||l.id,r.user_id,r.session_id,r.task_id,l.provider_id,l.model_id,'legacy_log',
       CASE WHEN l.success=1 THEN 'success' ELSE 'failure' END,
       -- The old global flag cannot prove individual zero fields were actually present.
       CASE WHEN l.usage_reported IS NOT 0 AND l.request_tokens>0 THEN l.request_tokens END,
       CASE WHEN l.usage_reported IS NOT 0 AND l.response_tokens>0 THEN l.response_tokens END,
       CASE WHEN l.usage_reported IS NOT 0 AND l.total_tokens>0 THEN l.total_tokens END,
       CASE WHEN l.latency_ms>=0 THEN l.latency_ms END,l.reported_at_ms,l.reported_at_ms
FROM historical_runs r
JOIN historical_logs l ON l.user_id=r.user_id AND l.request_id=r.task_id AND l.caller='workbench'
LEFT JOIN workbench_tasks t ON t.id=r.task_id AND t.user_id=r.user_id AND t.kind='chat' AND t.session_id=r.session_id
WHERE (r.total_tokens IS NULL OR r.total_tokens<0) AND (t.total_tokens IS NULL OR t.total_tokens<0);

-- An aggregate marker preserves historical known totals without inventing a request
-- count. Runs without any evidence of dispatch do not create a zero-usage request.
WITH historical_runs AS (
    SELECT task_id,user_id,session_id,total_tokens,dispatched,audit_warning,created_at,updated_at FROM chat_runs
    UNION ALL
    SELECT m.task_id,m.user_id,m.session_id,t.total_tokens,1,'legacy_message',m.created_at,m.updated_at
    FROM chat_messages m JOIN workbench_tasks t ON t.id=m.task_id AND t.user_id=m.user_id AND t.kind='chat' AND t.session_id=m.session_id
    WHERE m.role='assistant' AND NOT EXISTS(SELECT 1 FROM chat_runs r WHERE r.task_id=m.task_id)
      AND m.id=(SELECT last.id FROM chat_messages last WHERE last.task_id=m.task_id AND last.user_id=m.user_id AND last.role='assistant' ORDER BY last.seq DESC LIMIT 1)
)
INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,total_tokens,created_at,settled_at)
SELECT 'legacy-run:'||r.task_id,r.user_id,r.session_id,r.task_id,
       COALESCE(t.provider_id,s.provider_id,''),COALESCE(t.model_id,s.model_id,''),'legacy_total','unknown',
       CASE WHEN r.total_tokens>=0 THEN r.total_tokens WHEN t.total_tokens>=0 THEN t.total_tokens END,
       r.created_at*1000,r.updated_at*1000
FROM historical_runs r JOIN chat_sessions s ON s.id=r.session_id AND s.user_id=r.user_id
LEFT JOIN workbench_tasks t ON t.id=r.task_id AND t.user_id=r.user_id AND t.kind='chat' AND t.session_id=r.session_id
WHERE NOT EXISTS(SELECT 1 FROM chat_usage_attempts f WHERE f.user_id=r.user_id AND f.run_id=r.task_id)
  AND (r.dispatched=1 OR r.total_tokens>=0 OR t.total_tokens>=0 OR r.audit_warning='legacy_checkpoint_unavailable');
"#).await.map(|_| ())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("chat usage facts are forward-only".into()))
    }
}
