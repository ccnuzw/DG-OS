//! Stage-3 host processes reuse the execution ledger; persisted records never own PIDs.
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared("ALTER TABLE development_runs RENAME TO development_runs_stage2;
CREATE TABLE development_runs(
id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL CHECK(length(owner)>0), session_id TEXT,
task_id TEXT NOT NULL, project_id TEXT NOT NULL,
engine TEXT NOT NULL CHECK(engine IN ('codex','claude','host')),
source TEXT NOT NULL CHECK(source IN ('native_agent','host_terminal','host_command','host_service')),
status TEXT NOT NULL CHECK(status IN ('running','waiting','completed','failed','cancelled','unknown')),
started_at BIGINT NOT NULL, finished_at BIGINT,
summary TEXT NOT NULL CHECK(length(CAST(summary AS BLOB))<=16384),
reason TEXT CHECK(reason IS NULL OR length(CAST(reason AS BLOB))<=4096),
events TEXT NOT NULL CHECK(length(CAST(events AS BLOB))<=16777216),
fingerprint TEXT NOT NULL, start_fingerprint TEXT NOT NULL,
truncated INTEGER NOT NULL CHECK(truncated IN (0,1)), request_fingerprint TEXT NOT NULL,
runtime TEXT CHECK(runtime IS NULL OR length(CAST(runtime AS BLOB))<=16777216),
UNIQUE(id,owner),
CHECK((engine='host' AND session_id IS NULL AND source!='native_agent') OR (engine!='host' AND session_id IS NOT NULL AND source='native_agent')),
FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner),
FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner));
INSERT INTO development_runs SELECT id,owner,session_id,task_id,project_id,engine,'native_agent',status,started_at,finished_at,summary,reason,events,fingerprint,start_fingerprint,truncated,request_fingerprint,NULL FROM development_runs_stage2;
DROP TABLE development_runs_stage2;
CREATE INDEX idx_development_runs_task ON development_runs(owner,task_id,started_at DESC);
CREATE TABLE development_commands(
id TEXT NOT NULL, owner TEXT NOT NULL CHECK(length(owner)>0), project_id TEXT NOT NULL,
configuration TEXT NOT NULL CHECK(length(CAST(configuration AS BLOB))<=65536),
PRIMARY KEY(owner,project_id,id),
FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner) ON DELETE CASCADE);").await.map(|_| ())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> { Err(DbErr::Custom("development runtime is forward-only".into())) }
}
