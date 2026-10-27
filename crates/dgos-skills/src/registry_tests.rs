use super::manager::SkillsManager;
use crate::SkillError;
use dgos_db::{migrations::Migrator, sea_orm::Database, sea_orm_migration::MigratorTrait};
use serde_json::json;
use std::path::PathBuf;

async fn manager(url: String) -> SkillsManager {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    SkillsManager::new(db, PathBuf::from("unused-in-registry-tests")).with_test_registry(url)
}

fn registry() -> String {
    json!({ "skills": [{
        "id": "calculator", "name": "Calculator", "version": "1.0.0",
        "category": "compute", "tags": ["math"],
        "download_url": "https://example.invalid/calculator.tar.gz",
        "manifest": {
            "id": "calculator", "name": "Calculator", "version": "1.0.0",
            "description": "Calculate", "author": "DGOS", "category": "compute",
            "tags": ["math"], "call_type": "cli", "dependencies": {},
            "cli": { "command": "calculator" }
        }
    }] }).to_string()
}

#[tokio::test]
async fn registry_fetch_caches_data_and_recovers_after_outage() {
    let mut server = mockito::Server::new_async().await;
    let manager = manager(format!("{}/registry.json", server.url())).await;
    let online = server.mock("GET", "/registry.json")
        .with_status(200).with_body(registry()).create_async().await;
    let snapshot = manager.fetch_registry().await.unwrap();
    assert!(!snapshot.offline);
    assert!(snapshot.warning.is_none());
    assert_eq!(snapshot.skills[0].skill_id, "calculator");
    assert_eq!(snapshot.registry_url, format!("{}/registry.json", server.url()));
    let serialized = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(serialized["skills"][0]["skill_id"], "calculator");
    assert_eq!(serialized["offline"], false);
    online.assert_async().await;
    online.remove_async().await;

    for body in [None, Some("invalid JSON")] {
        let offline = server.mock("GET", "/registry.json")
            .with_status(if body.is_some() { 200 } else { 503 })
            .with_body(body.unwrap_or("unavailable")).create_async().await;
        let snapshot = manager.fetch_registry().await.unwrap();
        assert!(snapshot.offline);
        assert!(snapshot.warning.is_some());
        assert_eq!(snapshot.skills.len(), 1);
        assert_eq!(manager.search_skills("Calc").await.unwrap().len(), 1);
        assert_eq!(manager.get_skill_detail("calculator").await.unwrap().manifest.version, "1.0.0");
        offline.assert_async().await;
        offline.remove_async().await;
    }

    let restored = server.mock("GET", "/registry.json")
        .with_status(200).with_body(registry().replace("1.0.0", "1.1.0"))
        .create_async().await;
    let snapshot = manager.fetch_registry().await.unwrap();
    assert!(!snapshot.offline);
    assert!(snapshot.warning.is_none());
    assert_eq!(snapshot.skills[0].latest_version, "1.1.0");
    restored.assert_async().await;
}

#[tokio::test]
async fn registry_unreachable_without_cache_is_explicitly_offline() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/registry.json", listener.local_addr().unwrap());
    drop(listener);
    let manager = manager(url).await;
    let snapshot = manager.fetch_registry().await.unwrap();
    assert!(snapshot.offline);
    assert!(snapshot.skills.is_empty());
    assert!(snapshot.warning.is_some());
    assert!(manager.list_installed_skills("default-user").await.unwrap().is_empty());
}

#[tokio::test]
async fn registry_database_error_is_not_masked_as_offline_success() {
    let mut server = mockito::Server::new_async().await;
    let _offline = server.mock("GET", "/registry.json").with_status(503).create_async().await;
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SkillsManager::new(db, PathBuf::new())
        .with_test_registry(format!("{}/registry.json", server.url()));
    assert!(matches!(manager.fetch_registry().await, Err(SkillError::DatabaseError(_))));
}
