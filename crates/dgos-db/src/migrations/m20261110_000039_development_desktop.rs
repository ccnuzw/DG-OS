use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
CREATE TABLE development_desktop_state (
 owner TEXT PRIMARY KEY NOT NULL CHECK(length(owner)>0),
 revision INTEGER NOT NULL CHECK(revision>0),
 value TEXT NOT NULL CHECK(json_valid(value) AND json_type(value)='object' AND length(CAST(value AS BLOB))<=8388608)
);
CREATE TABLE development_workspace_trust (
 owner TEXT NOT NULL,
 project_id TEXT NOT NULL,
 path TEXT NOT NULL,
 directory_identity TEXT NOT NULL,
 agent INTEGER NOT NULL DEFAULT 0 CHECK(agent IN (0,1)),
 terminal INTEGER NOT NULL DEFAULT 0 CHECK(terminal IN (0,1)),
 service INTEGER NOT NULL DEFAULT 0 CHECK(service IN (0,1)),
 revision INTEGER NOT NULL CHECK(revision>0),
 PRIMARY KEY(owner,project_id,path,directory_identity),
 FOREIGN KEY(project_id,owner) REFERENCES development_projects(id,owner) ON DELETE CASCADE
);
CREATE TRIGGER development_trust_project_relocation AFTER UPDATE OF path,directory_identity ON development_projects
WHEN NEW.path IS NOT OLD.path OR NEW.directory_identity IS NOT OLD.directory_identity
BEGIN DELETE FROM development_workspace_trust WHERE owner=OLD.owner AND project_id=OLD.id; END;
CREATE TRIGGER development_trust_workspace_removal AFTER DELETE ON development_workspaces
BEGIN DELETE FROM development_workspace_trust WHERE owner=OLD.owner AND project_id=OLD.project_id AND path=OLD.path AND directory_identity=OLD.directory_identity; END;
"#).await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Desktop state and trust are forward-only".into()))
    }
}
