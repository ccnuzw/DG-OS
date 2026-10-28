use crate::{test_support::MemoryCredentialVault, ProviderManager};
use dgos_db::{
    migrations::Migrator,
    sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement},
    sea_orm_migration::MigratorTrait,
    ProviderRepository,
};
use dgos_types::{BuiltinCaller, CredentialStatus, ExecutionProvenance, ProviderConfig};
use futures::StreamExt;
use std::sync::Arc;

async fn database() -> (tempfile::TempDir, String, DatabaseConnection) {
    database_at(None).await
}

async fn database_at(steps: Option<u32>) -> (tempfile::TempDir, String, DatabaseConnection) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("fixture.db").display()
    );
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db, steps).await.unwrap();
    (dir, url, db)
}

fn config(endpoint: &str, key: &str) -> ProviderConfig {
    ProviderConfig {
        name: "Synthetic fixture".into(),
        provider_type: "openai_compatible".into(),
        base_url: endpoint.into(),
        api_key: key.into(),
        enabled: true,
        config: None,
    }
}

#[tokio::test]
async fn normal_create_rotate_endpoint_and_delete_use_vault_references() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault.clone())
        .await
        .unwrap();
    let mut server = mockito::Server::new_async().await;
    let models = server
        .mock("GET", "/models")
        .match_header("authorization", "Bearer synthetic-first")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .create_async()
        .await;
    let first = manager
        .create_provider(config(&server.url(), "synthetic-first"), "local".into())
        .await
        .unwrap();
    models.assert_async().await;
    assert_eq!(first.credential_status, CredentialStatus::Stored);
    assert!(first.credential_ref.is_some());
    assert_eq!(vault.entry_count().await, 1);
    let json = serde_json::to_value(&first).unwrap();
    assert!(json.get("api_key").is_none());
    let row = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT api_key FROM providers",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "api_key").unwrap(), "");

    let rotated = manager
        .update_provider(
            &first.id,
            config(&server.url(), "synthetic-second"),
            "local",
        )
        .await
        .unwrap();
    assert_ne!(first.credential_ref, rotated.credential_ref);
    assert_eq!(vault.entry_count().await, 1);
    assert!(ProviderRepository::new(db.clone())
        .pending_credential_cleanup()
        .await
        .unwrap()
        .is_empty());

    let mut changed = config("https://example.invalid/new-endpoint", "synthetic-third");
    changed.name = "Changed endpoint".into();
    let changed = manager
        .update_provider(&first.id, changed, "local")
        .await
        .unwrap();
    assert_eq!(changed.base_url, "https://example.invalid/new-endpoint");
    assert_ne!(rotated.credential_ref, changed.credential_ref);
    manager.delete_provider(&first.id, "local").await.unwrap();
    assert_eq!(vault.entry_count().await, 0);
    assert!(manager.list_providers("local").await.unwrap().is_empty());
    assert!(ProviderRepository::new(db)
        .pending_credential_cleanup()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn normal_metadata_edit_preserves_binding_and_saved_connection_uses_vault() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut server = mockito::Server::new_async().await;
    let provider = crate::test_support::seed_provider(
        db.clone(),
        &vault,
        config(&server.url(), "synthetic-saved"),
        "local",
    )
    .await
    .unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let mut renamed = config(&server.url(), "");
    renamed.name = "Renamed metadata".into();
    let updated = manager
        .update_provider(&provider.id, renamed, "local")
        .await
        .unwrap();
    assert_eq!(updated.credential_ref, provider.credential_ref);
    let models = server
        .mock("GET", "/models")
        .match_header("authorization", "Bearer synthetic-saved")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .create_async()
        .await;
    assert!(manager
        .test_provider_by_id(&provider.id, "local")
        .await
        .unwrap());
    models.assert_async().await;
}

#[tokio::test]
async fn normal_model_refresh_advances_snapshot_and_bound_call_uses_saved_revision() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut server = mockito::Server::new_async().await;
    let provider = crate::test_support::seed_provider(
        db.clone(),
        &vault,
        config(&server.url(), "synthetic-revision"),
        "local",
    )
    .await
    .unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let models = server
        .mock("GET", "/models")
        .match_header("authorization", "Bearer synthetic-revision")
        .with_status(200)
        .with_body(r#"{"data":[{"id":"fixture-chat"}]}"#)
        .expect(1)
        .create_async()
        .await;
    let refreshed = manager.refresh_models(&provider.id, "local").await.unwrap();
    assert_eq!(refreshed.len(), 1);
    let (snapshot, cached) = manager
        .model_snapshot(&provider.id, "fixture-chat", "local")
        .await
        .unwrap();
    assert_eq!(cached.provider_id, provider.id);
    assert_eq!(snapshot.config_revision, provider.config_revision + 1);
    let request = dgos_types::ChatRequest {
        messages: vec![dgos_types::ChatMessage {
            role: "user".into(),
            content: "Synthetic lifecycle".into(),
        }],
        temperature: None,
        max_tokens: Some(32),
        stream: Some(false),
    };
    assert!(matches!(
        manager
            .call_provider_bound(
                &provider.id,
                "fixture-chat",
                request.clone(),
                "local",
                Some(provider.config_revision)
            )
            .await,
        Err(dgos_types::DgosError::ProviderConfigurationChanged)
    ));
    let call = server.mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer synthetic-revision")
        .with_status(200).with_body(r#"{"choices":[{"message":{"content":"Completed"}}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}"#)
        .expect(1).create_async().await;
    let provenance = ExecutionProvenance {
        caller: BuiltinCaller::Workbench,
        request_id: "fixture-request-id".into(),
    };
    let response = manager
        .call_provider_bound_with_provenance(
            &provider.id,
            "fixture-chat",
            request,
            "local",
            Some(snapshot.config_revision),
            Some(&provenance),
        )
        .await
        .unwrap();
    assert_eq!(response.content, "Completed");
    let usage = manager.list_usage_logs("local", None).await.unwrap();
    assert_eq!(usage[0].caller.as_deref(), Some("workbench"));
    assert_eq!(usage[0].request_id.as_deref(), Some("fixture-request-id"));
    models.assert_async().await;
    call.assert_async().await;

    // A completed external response remains usable when only its audit sink is unavailable.
    db.execute(Statement::from_string(
        dgos_db::sea_orm::DbBackend::Sqlite,
        "DROP TABLE provider_usage_logs",
    ))
    .await
    .unwrap();
    let warning_call = server
        .mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer synthetic-revision")
        .with_status(200)
        .with_body(r#"{"choices":[{"message":{"content":"Completed with audit warning"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#)
        .expect(1)
        .create_async()
        .await;
    let warning = manager
        .call_provider_bound(
            &provider.id,
            "fixture-chat",
            dgos_types::ChatRequest {
                messages: vec![dgos_types::ChatMessage {
                    role: "user".into(),
                    content: "Audit sink fixture".into(),
                }],
                temperature: None,
                max_tokens: Some(32),
                stream: Some(false),
            },
            "local",
            Some(snapshot.config_revision),
        )
        .await
        .unwrap();
    assert_eq!(warning.content, "Completed with audit warning");
    assert!(warning.audit_warning);
    warning_call.assert_async().await;
}

#[tokio::test]
async fn streaming_call_rejects_a_stale_provider_revision_before_dispatch() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut server = mockito::Server::new_async().await;
    let provider = crate::test_support::seed_provider(
        db.clone(),
        &vault,
        config(&server.url(), "synthetic-stream"),
        "local",
    )
    .await
    .unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let updated = manager
        .update_provider(
            &provider.id,
            {
                let mut value = config(&server.url(), "");
                value.name = "stream revision update".into();
                value
            },
            "local",
        )
        .await
        .unwrap();
    let request = dgos_types::ChatRequest {
        messages: vec![dgos_types::ChatMessage {
            role: "user".into(),
            content: "stream fixture".into(),
        }],
        temperature: None,
        max_tokens: Some(16),
        stream: Some(true),
    };
    let stale = manager
        .stream_provider_bound(
            &provider.id,
            "fixture-chat",
            request.clone(),
            "local",
            Some(provider.config_revision),
        )
        .await;
    assert!(matches!(
        stale,
        Err(dgos_types::DgosError::ProviderConfigurationChanged)
    ));
    let stream_mock = server
        .mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer synthetic-stream")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n")
        .expect(1)
        .create_async()
        .await;
    let mut stream = manager
        .stream_provider_bound(
            &provider.id,
            "fixture-chat",
            request,
            "local",
            Some(updated.config_revision),
        )
        .await
        .unwrap();
    let mut deltas = String::new();
    while let Some(chunk) = stream.next().await {
        deltas.push_str(&chunk.unwrap().delta);
    }
    assert_eq!(deltas, "ok");
    stream_mock.assert_async().await;
}

#[tokio::test]
async fn normal_upgrade_migrates_legacy_row_once_and_retains_metadata() {
    let before_credentials = Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m20261028_000004_provider_credentials")
        .unwrap();
    let (_dir, url, db) = database_at(Some(before_credentials as u32)).await;
    // Isolated synthetic legacy input, not a copy of a user's database.
    db.execute_unprepared("INSERT INTO providers (id, name, provider_type, base_url, api_key, enabled, sort_order, user_id, created_at, updated_at) VALUES ('legacy-fixture', 'Legacy fixture', 'openai_compatible', 'https://example.invalid/v1', 'synthetic-legacy', 1, 0, 'local', '2026-10-27T00:00:00+00:00', '2026-10-27T00:00:00+00:00');")
        .await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let provider = ProviderRepository::new(db.clone())
        .get("legacy-fixture", "local")
        .await
        .unwrap()
        .unwrap();
    let manager = ProviderManager::with_vault(&url, Arc::new(MemoryCredentialVault::default()))
        .await
        .unwrap();
    assert_eq!(
        manager.list_providers("local").await.unwrap()[0].credential_status,
        CredentialStatus::MigrationRequired
    );
    manager.recover_credentials("local").await.unwrap();
    let migrated = manager
        .get_provider(&provider.id, "local")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(migrated.name, provider.name);
    assert_eq!(migrated.base_url, provider.base_url);
    assert_eq!(migrated.created_at, provider.created_at);
    assert_eq!(migrated.credential_status, CredentialStatus::Stored);
    assert!(ProviderRepository::new(db.clone())
        .legacy_api_key(&provider.id, "local")
        .await
        .unwrap()
        .is_none());
    manager.recover_credentials("local").await.unwrap();
    assert_eq!(
        manager
            .get_provider(&provider.id, "local")
            .await
            .unwrap()
            .unwrap()
            .credential_ref,
        migrated.credential_ref
    );
}

#[tokio::test]
async fn locked_vault_leaves_migration_pending_and_normal_retry_completes() {
    let (_dir, url, db) = database().await;
    let repo = ProviderRepository::new(db.clone());
    let provider = repo
        .create(config("https://example.invalid/v1", ""), "local".into())
        .await
        .unwrap();
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE providers SET api_key = ? WHERE id = ?",
        ["synthetic-pending".into(), provider.id.clone().into()],
    ))
    .await
    .unwrap();
    let unavailable = ProviderManager::new(&url).await.unwrap();
    unavailable.recover_credentials("local").await.unwrap();
    assert_eq!(
        unavailable.list_providers("local").await.unwrap()[0].credential_status,
        CredentialStatus::MigrationRequired
    );
    assert_eq!(
        repo.legacy_api_key(&provider.id, "local")
            .await
            .unwrap()
            .as_deref(),
        Some("synthetic-pending")
    );
    let available = ProviderManager::with_vault(&url, Arc::new(MemoryCredentialVault::default()))
        .await
        .unwrap();
    let migrated = available
        .migrate_provider_credential(&provider.id, "local")
        .await
        .unwrap();
    assert_eq!(migrated.credential_status, CredentialStatus::Stored);
    assert!(repo
        .legacy_api_key(&provider.id, "local")
        .await
        .unwrap()
        .is_none());
    assert!(repo.pending_credential_cleanup().await.unwrap().is_empty());
}

#[tokio::test]
async fn startup_finishes_staged_unreferenced_credential_cleanup() {
    use crate::{
        new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, SecretValue,
    };
    use dgos_db::repositories::provider_repository::CredentialCleanup;
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let scope = ProviderCredentialScope {
        user_id: "local".into(),
        provider_id: "staged-fixture".into(),
        endpoint: "https://example.invalid/v1".into(),
        credential_ref: new_credential_ref(),
    };
    let repo = ProviderRepository::new(db);
    repo.stage_credential_cleanup(&CredentialCleanup {
        user_id: scope.user_id.clone(),
        provider_id: scope.provider_id.clone(),
        endpoint: scope.endpoint.clone(),
        credential_ref: scope.credential_ref.clone(),
    })
    .await
    .unwrap();
    vault
        .store(&scope, SecretValue::new("synthetic-staged".into()).unwrap())
        .await
        .unwrap();
    let manager = ProviderManager::with_vault(&url, vault.clone())
        .await
        .unwrap();
    manager.recover_credentials("local").await.unwrap();
    assert!(repo.pending_credential_cleanup().await.unwrap().is_empty());
    assert!(vault.load(&scope).await.is_err());
}
