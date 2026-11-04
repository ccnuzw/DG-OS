//! D134 project and task metadata. Physical name `000026`: `000023` is taken by chat assets.
//!
//! Composite foreign keys keep every project, task and session reference inside one
//! account. A session's task must also belong to the session's project. Columns that
//! hold a reference may be NULL, and SQLite skips a composite FK when any column is NULL.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE development_projects(\
                    id TEXT PRIMARY KEY NOT NULL,\
                    owner TEXT NOT NULL CHECK(length(owner)>0),\
                    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 200),\
                    path TEXT NOT NULL CHECK(length(path)>0),\
                    directory_identity TEXT NOT NULL CHECK(length(directory_identity)>0),\
                    archived INTEGER NOT NULL CHECK(archived IN (0,1)),\
                    created_at BIGINT NOT NULL,\
                    last_opened_at BIGINT NOT NULL,\
                    UNIQUE(owner,path),\
                    UNIQUE(id,owner));\
                CREATE TABLE development_tasks(\
                    id TEXT PRIMARY KEY NOT NULL,\
                    owner TEXT NOT NULL CHECK(length(owner)>0),\
                    project_id TEXT NOT NULL,\
                    goal TEXT NOT NULL CHECK(length(CAST(goal AS BLOB)) BETWEEN 1 AND 8192),\
                    requirements TEXT NOT NULL CHECK(length(CAST(requirements AS BLOB)) BETWEEN 1 AND 16384),\
                    status TEXT NOT NULL CHECK(status IN ('todo','in_progress','blocked','review','completed','cancelled')),\
                    created_at BIGINT NOT NULL,\
                    UNIQUE(id,project_id,owner),\
                    FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner));\
                CREATE TABLE development_agent_sessions(\
                    id TEXT PRIMARY KEY NOT NULL,\
                    owner TEXT NOT NULL CHECK(length(owner)>0),\
                    engine TEXT NOT NULL CHECK(engine IN ('codex','claude')),\
                    native_id TEXT,\
                    workspace TEXT NOT NULL CHECK(length(workspace)>0),\
                    executable TEXT,\
                    model TEXT,\
                    project_id TEXT,\
                    task_id TEXT,\
                    CHECK((project_id IS NULL) = (task_id IS NULL)),\
                    UNIQUE(id,owner),\
                    FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner),\
                    FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner));\
                CREATE TABLE development_selection(\
                    owner TEXT PRIMARY KEY NOT NULL CHECK(length(owner)>0),\
                    project_id TEXT,\
                    task_id TEXT,\
                    session_id TEXT,\
                    CHECK(task_id IS NULL OR project_id IS NOT NULL),\
                    FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner),\
                    FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner),\
                    FOREIGN KEY(session_id,owner) REFERENCES development_agent_sessions(id,owner));\
                CREATE TABLE development_migrations(key TEXT PRIMARY KEY NOT NULL);\
                CREATE TABLE development_project_operations(\
                    id TEXT PRIMARY KEY NOT NULL,\
                    owner TEXT NOT NULL CHECK(length(owner)>0),\
                    kind TEXT NOT NULL CHECK(kind IN ('create','clone')),\
                    path TEXT NOT NULL CHECK(length(path)>0),\
                    status TEXT NOT NULL CHECK(status IN ('pending','succeeded','failed','unknown')),\
                    project_id TEXT);\
                CREATE INDEX idx_development_projects_owner ON development_projects(owner);\
                CREATE INDEX idx_development_tasks_project ON development_tasks(owner,project_id);\
                CREATE INDEX idx_development_sessions_project ON development_agent_sessions(owner,project_id);\
                CREATE INDEX idx_development_operations_owner ON development_project_operations(owner);",
            )
            .await
            .map(|_| ())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("project tasks are forward-only".into()))
    }
}
