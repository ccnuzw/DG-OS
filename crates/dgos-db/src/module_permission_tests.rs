use crate::{migrations::Migrator, ModulePermission, ModulePermissionRepository};
use sea_orm::{ConnectionTrait, Database, Statement};
use sea_orm_migration::MigratorTrait;

async fn database() -> sea_orm::DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

async fn broker() -> (sea_orm::DatabaseConnection, ModulePermissionRepository) {
    let db = database().await;
    let broker = ModulePermissionRepository::new(db.clone());
    broker
        .register_module(
            "module-1",
            "example.module",
            "Example",
            "1.0.0",
            serde_json::json!({"permissions":["providers.call"]}),
            "a".repeat(64),
            true,
        )
        .await
        .unwrap();
    (db, broker)
}

#[tokio::test]
async fn migration_creates_module_broker_tables() {
    let db = database().await;
    for table in ["modules", "module_permissions", "module_permission_audits"] {
        let row = db
            .query_one(Statement::from_string(
                db.get_database_backend(),
                format!("SELECT name FROM sqlite_master WHERE type='table' AND name='{table}'"),
            ))
            .await
            .unwrap();
        assert!(row.is_some(), "missing {table}");
    }
}

#[tokio::test]
async fn migration_appends_broker_tables_without_losing_existing_rows() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let before_broker = Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m20261028_000009_module_permissions")
        .unwrap();
    Migrator::up(&db, Some(before_broker as u32)).await.unwrap();
    db.execute_unprepared("INSERT INTO providers (id, name, provider_type, base_url, api_key, enabled, sort_order, user_id, created_at, updated_at) VALUES ('legacy-provider', 'Legacy', 'openai_compatible', 'https://example.invalid/v1', '', 1, 0, 'alice', '2026-10-27T00:00:00+00:00', '2026-10-27T00:00:00+00:00');")
        .await
        .unwrap();
    Migrator::up(&db, None).await.unwrap();
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT name FROM providers WHERE id = 'legacy-provider'",
        ))
        .await
        .unwrap();
    assert!(row.is_some());
    let broker_table = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT name FROM sqlite_master WHERE type='table' AND name='module_permissions'",
        ))
        .await
        .unwrap();
    assert!(broker_table.is_some());
}

#[tokio::test]
async fn grant_revoke_is_immediate_and_user_scoped() {
    let (db, broker) = broker().await;
    assert!(!broker
        .check("alice", "module-1", ModulePermission::ProvidersCall)
        .await
        .unwrap());
    broker
        .grant_with_request(
            "alice",
            "module-1",
            ModulePermission::ProvidersCall,
            Some("grant-1"),
        )
        .await
        .unwrap();
    assert!(broker
        .check("alice", "module-1", ModulePermission::ProvidersCall)
        .await
        .unwrap());
    assert!(!broker
        .check("bob", "module-1", ModulePermission::ProvidersCall)
        .await
        .unwrap());
    broker
        .revoke_with_request(
            "alice",
            "module-1",
            ModulePermission::ProvidersCall,
            Some("revoke-1"),
        )
        .await
        .unwrap();
    assert!(!broker
        .check("alice", "module-1", ModulePermission::ProvidersCall)
        .await
        .unwrap());

    let audits = db
        .query_all(Statement::from_string(
            db.get_database_backend(),
            "SELECT action, permission, request_id, resource_id FROM module_permission_audits ORDER BY created_at, id",
        ))
        .await
        .unwrap();
    assert!(audits.len() >= 4); // initial deny, grant, revoke, post-revoke deny
    for row in audits {
        assert_eq!(
            row.try_get::<String>("", "permission").unwrap(),
            "providers.call"
        );
        assert!(row
            .try_get::<Option<String>>("", "resource_id")
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn invalid_permission_and_disabled_or_unknown_modules_are_rejected() {
    let (db, broker) = broker().await;
    assert!(ModulePermission::parse("provider_access").is_err());
    assert!(broker
        .grant("alice", "module-1", "provider_access")
        .await
        .is_err());
    assert!(broker
        .grant("alice", "missing", ModulePermission::McpCall)
        .await
        .is_err());
    db.execute_unprepared("UPDATE modules SET is_enabled = FALSE WHERE id = 'module-1'")
        .await
        .unwrap();
    assert!(!broker
        .check("alice", "module-1", ModulePermission::McpCall)
        .await
        .unwrap());
    assert!(broker
        .grant("alice", "module-1", ModulePermission::McpCall)
        .await
        .is_err());
    let denied = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT action FROM module_permission_audits WHERE action = 'denied' LIMIT 1",
        ))
        .await
        .unwrap();
    assert!(denied.is_some());
}
