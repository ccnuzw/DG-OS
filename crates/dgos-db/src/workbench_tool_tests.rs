#[cfg(test)]
mod tests {
    use crate::migrations::Migrator;
    use sea_orm::{ConnectionTrait, Database};
    use sea_orm_migration::MigratorTrait;

    #[tokio::test]
    async fn tool_reference_migration_preserves_tasks_and_rejects_duplicate_references() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, Some(10)).await.unwrap();
        db.execute_unprepared("INSERT INTO workbench_tasks (id,user_id,app_id,provider_id,model_id,status,created_at) VALUES ('existing','alice','ai-workbench','provider','model','running',1)").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let row = db
            .query_one(sea_orm::Statement::from_string(
                db.get_database_backend(),
                "SELECT status FROM workbench_tasks WHERE id='existing'",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "status").unwrap(), "running");
        let insert = "INSERT INTO workbench_task_tools (task_id,server_id,tool_name,schema_hash) VALUES ('existing','historical-server','echo','hash')";
        db.execute_unprepared(insert).await.unwrap();
        assert!(db.execute_unprepared(insert).await.is_err());
        assert!(db.execute_unprepared("INSERT INTO workbench_task_tools (task_id,server_id,tool_name,schema_hash) VALUES ('missing','server','echo','hash')").await.is_err());
        assert!(db
            .execute_unprepared(
                "UPDATE workbench_tasks SET status='awaiting_confirmation' WHERE id='existing'"
            )
            .await
            .is_err());
    }
}
