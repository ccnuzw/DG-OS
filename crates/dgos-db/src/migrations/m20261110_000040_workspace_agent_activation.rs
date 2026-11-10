use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
ALTER TABLE development_workspace_trust ADD COLUMN agent_revoked INTEGER NOT NULL DEFAULT 0 CHECK(agent_revoked IN (0,1) AND NOT(agent=1 AND agent_revoked=1));
-- Old false rows do not retain whether an Agent grant was explicitly revoked.
-- Preserve denial rather than silently restoring a possibly revoked authority.
UPDATE development_workspace_trust SET agent_revoked=1 WHERE agent=0;
"#).await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Workspace Agent activation is forward-only".into()))
    }
}
