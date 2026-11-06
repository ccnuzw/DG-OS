//! Stage 5 workspace identities, Git receipts and cross-engine handoffs.
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
CREATE TABLE development_project_operations_next(
 id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL CHECK(length(owner)>0),
 kind TEXT NOT NULL CHECK(kind IN ('create','clone','workspace_create','workspace_import','workspace_select','workspace_remove','git_stage','git_unstage','git_commit','git_push','git_switch','git_merge','git_discard')),
 path TEXT NOT NULL CHECK(length(path)>0), status TEXT NOT NULL CHECK(status IN ('pending','succeeded','failed','unknown')),
 project_id TEXT, task_id TEXT, workspace_id TEXT, request_fingerprint TEXT, details TEXT);
INSERT INTO development_project_operations_next(id,owner,kind,path,status,project_id)
 SELECT id,owner,kind,path,status,project_id FROM development_project_operations;
DROP TABLE development_project_operations;
ALTER TABLE development_project_operations_next RENAME TO development_project_operations;
CREATE INDEX idx_development_operations_owner ON development_project_operations(owner);
CREATE INDEX idx_development_operations_task ON development_project_operations(owner,task_id);
CREATE TABLE development_workspaces(
 id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL, project_id TEXT NOT NULL,
 path TEXT NOT NULL, directory_identity TEXT NOT NULL,
 common_dir TEXT, common_identity TEXT,
 kind TEXT NOT NULL CHECK(kind IN ('original','imported','managed')),
 created_operation TEXT, base_commit TEXT,
 CHECK((common_dir IS NULL)=(common_identity IS NULL)),
 CHECK(kind!='managed' OR (created_operation IS NOT NULL AND base_commit IS NOT NULL AND common_dir IS NOT NULL)),
 UNIQUE(id,project_id,owner), UNIQUE(owner,project_id,path),
 FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner));
CREATE TABLE development_task_workspaces(
 task_id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL, project_id TEXT NOT NULL, workspace_id TEXT NOT NULL,
 FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner),
 FOREIGN KEY(workspace_id,project_id,owner) REFERENCES development_workspaces(id,project_id,owner));
CREATE INDEX idx_development_workspace_common ON development_workspaces(common_dir);
CREATE TABLE development_handoffs(
 id TEXT PRIMARY KEY NOT NULL, owner TEXT NOT NULL, project_id TEXT NOT NULL, task_id TEXT NOT NULL,
 source_session_id TEXT NOT NULL, source_run_id TEXT,
 target_engine TEXT NOT NULL CHECK(target_engine IN ('codex','claude')), permission_epoch BIGINT NOT NULL,
 summary TEXT NOT NULL, fingerprint TEXT NOT NULL, target_session_id TEXT, created_at BIGINT NOT NULL,
 FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner),
 FOREIGN KEY(source_session_id,owner) REFERENCES development_agent_sessions(id,owner),
 UNIQUE(owner,target_session_id));
CREATE TABLE development_file_writes(
 owner TEXT NOT NULL, task_id TEXT NOT NULL, project_id TEXT NOT NULL, path TEXT NOT NULL,
 before_hash TEXT NOT NULL, after_hash TEXT NOT NULL, valid INTEGER NOT NULL CHECK(valid IN (0,1)),
 PRIMARY KEY(owner,task_id,path),
 FOREIGN KEY(task_id,project_id,owner) REFERENCES development_tasks(id,project_id,owner));
"#).await.map(|_| ())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> { Err(DbErr::Custom("task workspaces are forward-only".into())) }
}
