#[cfg(test)]
mod tests {
    use crate::migrations::Migrator;
    use sea_orm::{ConnectionTrait, Database};
    use sea_orm_migration::MigratorTrait;

    #[tokio::test]
    async fn mcp_schema_is_created_with_safe_defaults() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let rows = db
            .query_one(sea_orm::Statement::from_string(
                db.get_database_backend(),
                "SELECT transport, enabled, auto_connect, status FROM mcp_servers LIMIT 0",
            ))
            .await
            .unwrap();
        assert!(rows.is_none());
        let tables = db
            .query_one(sea_orm::Statement::from_string(
                db.get_database_backend(),
                "SELECT name FROM sqlite_master WHERE type='table' AND name='mcp_usage_logs'",
            ))
            .await
            .unwrap();
        assert!(tables.is_some());
    }

    #[tokio::test]
    async fn connection_events_migrate_existing_mcp_database_without_data_loss() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, Some(9)).await.unwrap();
        db.execute_unprepared("INSERT INTO mcp_servers (id, user_id, name, transport, config_json, created_at, updated_at) VALUES ('existing', 'alice', 'existing', 'stdio', '{}', 1, 1);").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let existing = db
            .query_one(sea_orm::Statement::from_string(
                db.get_database_backend(),
                "SELECT id FROM mcp_servers WHERE id = 'existing'",
            ))
            .await
            .unwrap();
        assert!(existing.is_some());
        db.execute_unprepared("INSERT INTO mcp_connection_logs (user_id, server_id, event, status, created_at) VALUES ('alice', 'existing', 'connected', 'ready', 2);").await.unwrap();
    }
}
