use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
            ALTER TABLE chat_sessions ADD COLUMN skill_refs TEXT NOT NULL DEFAULT '[]';
            ALTER TABLE chat_sessions ADD COLUMN generation_parameters TEXT NOT NULL DEFAULT '{}';
            UPDATE chat_sessions SET skill_refs=json_array(json_object('skill_id',skill_id,'version_id',skill_version_id,'content_hash',skill_content_hash)) WHERE skill_id IS NOT NULL;
            ALTER TABLE workbench_task_skills ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE workbench_tasks ADD COLUMN generation_parameters TEXT NOT NULL DEFAULT '{}';
        "#).await.map(|_| ())
    }
    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> { Err(DbErr::Custom("generation snapshots are forward-only".into())) }
}
