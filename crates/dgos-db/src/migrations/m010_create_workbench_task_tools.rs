use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS workbench_task_tools (
                task_id TEXT NOT NULL REFERENCES workbench_tasks(id),
                server_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                schema_hash TEXT NOT NULL,
                PRIMARY KEY (task_id, server_id, tool_name)
            )",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Tool audit references are retained; destructive rollback is unsupported".into(),
        ))
    }
}
