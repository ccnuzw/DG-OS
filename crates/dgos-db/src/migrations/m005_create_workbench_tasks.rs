use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // V1 is SQLite-only. No prompt, output, credential or raw error columns.
        manager.get_connection().execute_unprepared(
            "CREATE TABLE workbench_tasks (
                id TEXT PRIMARY KEY NOT NULL, user_id TEXT NOT NULL,
                app_id TEXT NOT NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('running','succeeded','failed','interrupted')),
                created_at BIGINT NOT NULL, finished_at BIGINT,
                error_code TEXT, total_tokens BIGINT
            );
            CREATE INDEX idx_workbench_tasks_user ON workbench_tasks(user_id, created_at);
            CREATE TABLE workbench_task_skills (
                task_id TEXT NOT NULL REFERENCES workbench_tasks(id),
                version_id TEXT NOT NULL REFERENCES skill_versions(id),
                skill_id TEXT NOT NULL, content_hash TEXT NOT NULL,
                PRIMARY KEY(task_id, version_id)
            );"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE workbench_task_skills; DROP TABLE workbench_tasks;")
            .await?;
        Ok(())
    }
}
