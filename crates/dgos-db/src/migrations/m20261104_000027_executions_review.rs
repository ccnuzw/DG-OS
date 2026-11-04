//! Stage-2 bounded execution and review persistence.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE development_review_baselines(\
                owner TEXT NOT NULL CHECK(length(owner)>0),\
                task_id TEXT NOT NULL,\
                project_id TEXT NOT NULL,\
                baseline TEXT NOT NULL CHECK(length(CAST(baseline AS BLOB))<=12582912),\
                PRIMARY KEY(owner,task_id),\
                FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner));\
            CREATE TABLE development_review_observations(\
                owner TEXT NOT NULL CHECK(length(owner)>0),\
                task_id TEXT NOT NULL,\
                project_id TEXT NOT NULL,\
                observation TEXT NOT NULL CHECK(length(CAST(observation AS BLOB))<=1048576),\
                PRIMARY KEY(owner,task_id),\
                FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner));\
            CREATE TABLE development_runs(\
                id TEXT PRIMARY KEY NOT NULL,\
                owner TEXT NOT NULL CHECK(length(owner)>0),\
                session_id TEXT NOT NULL,\
                task_id TEXT NOT NULL,\
                project_id TEXT NOT NULL,\
                engine TEXT NOT NULL CHECK(engine IN ('codex','claude')),
                status TEXT NOT NULL CHECK(status IN ('running','waiting','completed','failed','cancelled','unknown')),
                started_at BIGINT NOT NULL,\
                finished_at BIGINT,\
                summary TEXT NOT NULL CHECK(length(CAST(summary AS BLOB))<=16384),\
                reason TEXT CHECK(reason IS NULL OR length(CAST(reason AS BLOB))<=4096),\
                events TEXT NOT NULL CHECK(length(CAST(events AS BLOB))<=16777216),\
                fingerprint TEXT NOT NULL,\
                start_fingerprint TEXT NOT NULL,\
                truncated INTEGER NOT NULL CHECK(truncated IN (0,1)),\
                request_fingerprint TEXT NOT NULL,\
                UNIQUE(id,owner),\
                FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner),\
                FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner));\
            CREATE INDEX idx_development_runs_task ON development_runs(owner,task_id,started_at DESC);"
        ).await.map(|_| ())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("execution review is forward-only".into()))
    }
}
