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
    for table in [
        "modules",
        "module_permissions",
        "module_permission_audits",
        "module_release_locks",
    ] {
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

#[tokio::test]
async fn grants_cannot_exceed_the_published_manifest() {
    let (_db, broker) = broker().await;
    assert!(broker
        .grant("alice", "module-1", ModulePermission::McpCall)
        .await
        .is_err());
    assert!(!broker
        .check("alice", "module-1", ModulePermission::McpCall)
        .await
        .unwrap());
}

async fn grant_cycle(broker: &ModulePermissionRepository, user: &str, times: usize) {
    for _ in 0..times {
        broker
            .grant(user, "module-1", ModulePermission::ProvidersCall)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn audit_listing_is_user_isolated_newest_first_and_hides_request_ids() {
    let (_db, broker) = broker().await;
    broker
        .grant_with_request(
            "alice",
            "module-1",
            ModulePermission::ProvidersCall,
            Some("req-a"),
        )
        .await
        .unwrap();
    broker
        .revoke_with_request(
            "alice",
            "module-1",
            ModulePermission::ProvidersCall,
            Some("req-b"),
        )
        .await
        .unwrap();
    broker
        .grant("bob", "module-1", ModulePermission::ProvidersCall)
        .await
        .unwrap();

    let page = broker.list_audits("alice", None, None, 50).await.unwrap();
    let actions: Vec<_> = page.entries.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(actions, ["revoked", "granted"]);
    assert!(page.next_before.is_none());
    assert!(page.entries.iter().all(|e| e.module_id == "module-1"));
    let bob = broker.list_audits("bob", None, None, 50).await.unwrap();
    assert_eq!(bob.entries.len(), 1);
    assert!(broker
        .list_audits("carol", None, None, 50)
        .await
        .unwrap()
        .entries
        .is_empty());
}

#[tokio::test]
async fn audit_cursor_paging_is_stable_within_one_second() {
    let (db, broker) = broker().await;
    grant_cycle(&broker, "alice", 5).await;
    // Force every row into the same second: only rowid can order them now.
    db.execute_unprepared("UPDATE module_permission_audits SET created_at = 1700000000")
        .await
        .unwrap();

    let first = broker.list_audits("alice", None, None, 2).await.unwrap();
    assert_eq!(first.entries.len(), 2);
    let cursor = first.next_before.expect("more rows remain");
    assert_eq!(cursor, first.entries[1].cursor);
    let second = broker
        .list_audits("alice", None, Some(cursor), 2)
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 2);
    let cursor = second.next_before.expect("more rows remain");
    let third = broker
        .list_audits("alice", None, Some(cursor), 2)
        .await
        .unwrap();
    assert_eq!(third.entries.len(), 1);
    assert!(third.next_before.is_none());

    let mut seen: Vec<i64> = [first, second, third]
        .into_iter()
        .flat_map(|page| page.entries.into_iter().map(|e| e.cursor))
        .collect();
    assert_eq!(seen.len(), 5);
    assert!(
        seen.windows(2).all(|pair| pair[0] > pair[1]),
        "strictly newest first, no repeats"
    );
    seen.dedup();
    assert_eq!(seen.len(), 5);

    // An exactly-full last page reports no further page.
    let exact = broker.list_audits("alice", None, None, 5).await.unwrap();
    assert_eq!(exact.entries.len(), 5);
    assert!(exact.next_before.is_none());
}

#[tokio::test]
async fn audit_listing_filters_by_module() {
    let (_db, broker) = broker().await;
    broker
        .register_module(
            "module-2",
            "example.two",
            "Two",
            "1.0.0",
            serde_json::json!({"permissions":["providers.call"]}),
            "b".repeat(64),
            true,
        )
        .await
        .unwrap();
    broker
        .grant("alice", "module-1", ModulePermission::ProvidersCall)
        .await
        .unwrap();
    broker
        .grant("alice", "module-2", ModulePermission::ProvidersCall)
        .await
        .unwrap();
    broker
        .grant("alice", "module-2", ModulePermission::McpCall)
        .await
        .unwrap_err();

    let two = broker
        .list_audits("alice", Some("module-2"), None, 50)
        .await
        .unwrap();
    assert_eq!(two.entries.len(), 2);
    assert!(two.entries.iter().all(|e| e.module_id == "module-2"));
    let one = broker
        .list_audits("alice", Some("module-1"), None, 50)
        .await
        .unwrap();
    assert_eq!(one.entries.len(), 1);
    let all = broker.list_audits("alice", None, None, 50).await.unwrap();
    assert_eq!(all.entries.len(), 3);
}

#[tokio::test]
async fn audit_retention_trims_oldest_rows_per_user_only() {
    let db = database().await;
    let broker = ModulePermissionRepository::with_audit_cap(db.clone(), 3);
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
    broker
        .grant("bob", "module-1", ModulePermission::ProvidersCall)
        .await
        .unwrap();
    broker
        .revoke("bob", "module-1", ModulePermission::ProvidersCall)
        .await
        .unwrap();
    grant_cycle(&broker, "alice", 6).await;

    let alice = broker.list_audits("alice", None, None, 50).await.unwrap();
    assert_eq!(alice.entries.len(), 3, "alice is capped at 3");
    let bob = broker.list_audits("bob", None, None, 50).await.unwrap();
    assert_eq!(bob.entries.len(), 2, "bob's rows are untouched");

    // The survivors are alice's newest three.
    let newest = db
        .query_all(Statement::from_string(
            db.get_database_backend(),
            "SELECT rowid AS r FROM module_permission_audits WHERE user_id = 'alice' ORDER BY rowid DESC LIMIT 3",
        ))
        .await
        .unwrap();
    let expected: Vec<i64> = newest
        .iter()
        .map(|row| row.try_get("", "r").unwrap())
        .collect();
    let got: Vec<i64> = alice.entries.iter().map(|e| e.cursor).collect();
    assert_eq!(got, expected);
}

#[tokio::test]
async fn denied_check_is_audited_and_still_trimmed_by_the_cap() {
    let db = database().await;
    let broker = ModulePermissionRepository::with_audit_cap(db.clone(), 2);
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
    for _ in 0..4 {
        assert!(!broker
            .check_with_request(
                "alice",
                "module-1",
                ModulePermission::ProvidersCall,
                Some("req")
            )
            .await
            .unwrap());
    }
    let page = broker.list_audits("alice", None, None, 50).await.unwrap();
    assert_eq!(page.entries.len(), 2);
    assert!(page.entries.iter().all(|e| e.action == "denied"));
}
