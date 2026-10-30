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
        .create_provider(
            config(&server.url(), "synthetic-first"),
            "local".into(),
            None,
        )
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
        .test_provider_by_id(&provider.id, "local", None)
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
        ..Default::default()
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
                ..Default::default()
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
        ..Default::default()
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

fn user_request() -> dgos_types::ChatRequest {
    dgos_types::ChatRequest {
        messages: vec![dgos_types::ChatMessage {
            role: "user".into(),
            content: "synthetic".into(),
        }],
        temperature: None,
        max_tokens: Some(16),
        stream: Some(true),
        ..Default::default()
    }
}

async fn usage_rows(manager: &ProviderManager) -> Vec<dgos_types::UsageLog> {
    // A stream dropped early is recorded from a spawned task; give it a moment.
    for _ in 0..40 {
        let rows = manager.list_usage_logs("local", None).await.unwrap();
        if !rows.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            return manager.list_usage_logs("local", None).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Vec::new()
}

#[tokio::test]
async fn invalid_api_key_on_create_is_1001002_and_leaves_nothing_behind() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault.clone())
        .await
        .unwrap();
    let mut server = mockito::Server::new_async().await;
    let models = server
        .mock("GET", "/models")
        .with_status(401)
        .with_body(r#"{"error":{"message":"upstream-secret"}}"#)
        .expect(2)
        .create_async()
        .await;
    let first = manager
        .create_provider(config(&server.url(), "synthetic-bad"), "local".into(), None)
        .await
        .unwrap_err();
    let second = manager
        .create_provider_with_models(
            config(&server.url(), "synthetic-bad"),
            "local".into(),
            vec![],
            None,
        )
        .await
        .unwrap_err();
    for error in [first, second] {
        assert!(matches!(error, dgos_types::DgosError::InvalidApiKey));
        assert_eq!(error.error_code(), 1001002);
        assert!(!format!("{error:?}").contains("upstream-secret"));
    }
    models.assert_async().await;
    assert_eq!(vault.entry_count().await, 0);
    assert!(ProviderRepository::new(db)
        .list("local")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn manual_models_work_for_a_service_without_a_model_list() {
    let (_dir, url, _db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let mut server = mockito::Server::new_async().await;
    let missing = server
        .mock("GET", "/models")
        .with_status(404)
        .expect(2)
        .create_async()
        .await;
    // The service has no model list. Creation without a probe model refuses to save an endpoint
    // that was never verified; with one, the probe request decides.
    let unverified = manager
        .create_provider(config(&server.url(), "synthetic-key"), "local".into(), None)
        .await;
    assert!(matches!(
        unverified,
        Err(dgos_types::DgosError::RequestNotSupported)
    ));
    assert!(manager.list_providers("local").await.unwrap().is_empty());
    let probe = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(
            serde_json::json!({"model": "probe-model", "max_tokens": 16}),
        ))
        .with_status(200)
        .with_body(r#"{"choices":[]}"#)
        .expect(1)
        .create_async()
        .await;
    let provider = manager
        .create_provider(
            config(&server.url(), "synthetic-key"),
            "local".into(),
            Some("probe-model"),
        )
        .await
        .unwrap();
    probe.assert_async().await;
    missing.assert_async().await;
    assert!(manager
        .list_models(&provider.id, "local")
        .await
        .unwrap()
        .is_empty());

    let embedder = manager
        .add_manual_model(&provider.id, "local", "  my-embedder  ", Some("embedding"))
        .await
        .unwrap();
    assert_eq!(embedder.model_id, "my-embedder");
    assert_eq!(embedder.source, dgos_types::ModelSource::Manual);
    assert_eq!(embedder.model_type, "embedding");
    assert_eq!(embedder.kind_source, dgos_types::KindSource::Manual);
    let again = manager
        .add_manual_model(&provider.id, "local", "my-embedder", Some("rerank"))
        .await
        .unwrap();
    assert_eq!(
        again.id, embedder.id,
        "a repeat must not create a second row"
    );
    assert_eq!(again.model_type, "embedding");
    let chat = manager
        .add_manual_model(&provider.id, "local", "my-chat", None)
        .await
        .unwrap();
    assert_eq!(chat.model_type, "text-generation");
    assert_eq!(chat.kind_source, dgos_types::KindSource::Fallback);
    assert_eq!(
        manager
            .list_models(&provider.id, "local")
            .await
            .unwrap()
            .len(),
        2
    );

    // Invalid input is rejected without a write.
    for (id, kind) in [
        ("", None),
        ("   ", None),
        ("ok", Some("chat")),
        ("ok", Some("Embedding")),
    ] {
        assert!(matches!(
            manager
                .add_manual_model(&provider.id, "local", id, kind)
                .await,
            Err(dgos_types::DgosError::RequestNotSupported)
        ));
    }
    assert!(matches!(
        manager
            .add_manual_model(&provider.id, "someone-else", "x", None)
            .await,
        Err(dgos_types::DgosError::ProviderNotFound(_))
    ));
    assert_eq!(
        manager
            .list_models(&provider.id, "local")
            .await
            .unwrap()
            .len(),
        2
    );

    // A refresh replaces remote rows only; a remote item with a manual id is skipped.
    missing.remove_async().await;
    let list = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[{"id":"my-chat"},{"id":"remote-one"}]}"#)
        .create_async()
        .await;
    let refreshed = manager.refresh_models(&provider.id, "local").await.unwrap();
    list.assert_async().await;
    let by_id: std::collections::BTreeMap<_, _> = refreshed
        .iter()
        .map(|m| (m.model_id.as_str(), (m.source, m.model_type.as_str())))
        .collect();
    assert_eq!(by_id.len(), 3, "{by_id:?}");
    assert_eq!(
        by_id["my-embedder"],
        (dgos_types::ModelSource::Manual, "embedding")
    );
    assert_eq!(by_id["my-chat"].0, dgos_types::ModelSource::Manual);
    assert_eq!(by_id["remote-one"].0, dgos_types::ModelSource::Remote);

    manager
        .remove_manual_model(&provider.id, "local", "my-embedder")
        .await
        .unwrap();
    manager
        .remove_manual_model(&provider.id, "local", "remote-one")
        .await
        .unwrap();
    manager
        .remove_manual_model(&provider.id, "local", "never-existed")
        .await
        .unwrap();
    let left: Vec<_> = manager
        .list_models(&provider.id, "local")
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.model_id)
        .collect();
    assert_eq!(left.len(), 2);
    assert!(left.contains(&"my-chat".to_string()) && left.contains(&"remote-one".to_string()));
}

#[tokio::test]
async fn renderer_models_recomputed() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault.clone())
        .await
        .unwrap();
    let mut server = mockito::Server::new_async().await;
    let _models = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .create_async()
        .await;
    let forged = dgos_types::Model {
        id: "forged-primary-key".into(),
        provider_id: "someone-elses-provider".into(),
        model_id: "text-embedding-3-small".into(),
        model_name: "Shown name".into(),
        model_type: "text-generation".into(),
        context_window: Some(1_000_000),
        max_output_tokens: Some(1_000_000),
        supports_streaming: true,
        supports_vision: true,
        kind_source: dgos_types::KindSource::Manual,
        source: dgos_types::ModelSource::Manual,
        ..Default::default()
    };
    let provider = manager
        .create_provider_with_models(
            config(&server.url(), "synthetic-key"),
            "local".into(),
            vec![forged.clone()],
            None,
        )
        .await
        .unwrap();
    let stored = manager.list_models(&provider.id, "local").await.unwrap();
    assert_eq!(stored.len(), 1);
    let model = &stored[0];
    assert_ne!(model.id, "forged-primary-key");
    assert_eq!(model.provider_id, provider.id);
    assert_eq!(model.model_name, "Shown name");
    assert_eq!(model.model_type, "embedding");
    assert_eq!(model.kind_source, dgos_types::KindSource::Rule);
    assert_eq!(model.source, dgos_types::ModelSource::Remote);
    assert_eq!(
        (model.context_window, model.max_output_tokens),
        (None, None)
    );
    assert!(!model.supports_streaming && !model.supports_vision);

    // One illegal id rejects the whole creation: no provider, no credential, no request.
    let before = vault.entry_count().await;
    let providers_before = ProviderRepository::new(db.clone())
        .list("local")
        .await
        .unwrap()
        .len();
    let probe = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .expect(0)
        .create_async()
        .await;
    for bad in [String::new(), "x".repeat(256)] {
        let mut invalid = forged.clone();
        invalid.model_id = bad;
        let result = manager
            .create_provider_with_models(
                config(&server.url(), "synthetic-key"),
                "local".into(),
                vec![forged.clone(), invalid],
                None,
            )
            .await;
        assert!(matches!(
            result,
            Err(dgos_types::DgosError::RequestNotSupported)
        ));
    }
    probe.assert_async().await;
    assert_eq!(vault.entry_count().await, before);
    assert_eq!(
        ProviderRepository::new(db)
            .list("local")
            .await
            .unwrap()
            .len(),
        providers_before
    );
}

#[tokio::test]
async fn insecure_endpoints_are_rejected_on_save_and_before_any_call() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault.clone())
        .await
        .unwrap();
    for base in [
        "http://example.com/v1",
        "file:///etc/passwd",
        "ftp://example.com",
    ] {
        let created = manager
            .create_provider(config(base, "synthetic-key"), "local".into(), None)
            .await;
        assert!(
            matches!(created, Err(dgos_types::DgosError::InsecureEndpoint)),
            "{base}"
        );
        assert_eq!(created.unwrap_err().error_code(), 1001023);
    }
    assert_eq!(vault.entry_count().await, 0);
    assert!(matches!(
        manager
            .test_connection(&config("http://example.com/v1", "k"), None)
            .await,
        Err(dgos_types::DgosError::InsecureEndpoint)
    ));

    // The opt-in must be a boolean and must be set to allow plain http to a public host.
    let mut opted_in = config("http://example.com/v1", "synthetic-key");
    opted_in.config = Some(serde_json::json!({"allow_insecure_http": "yes"}));
    assert!(matches!(
        manager
            .create_provider(opted_in, "local".into(), None)
            .await,
        Err(dgos_types::DgosError::RequestNotSupported)
    ));

    // A configuration saved earlier (or written by an older version) is refused at call time.
    let stored = crate::test_support::seed_provider(
        db,
        &vault,
        config("http://example.com/v1", "synthetic-saved"),
        "local",
    )
    .await
    .unwrap();
    assert!(matches!(
        manager
            .call_provider(&stored.id, "m", user_request(), "local")
            .await,
        Err(dgos_types::DgosError::InsecureEndpoint)
    ));
    assert!(matches!(
        manager
            .stream_provider(&stored.id, "m", user_request(), "local")
            .await,
        Err(dgos_types::DgosError::InsecureEndpoint)
    ));
}

#[tokio::test]
async fn local_plain_http_endpoints_are_saved_normalised() {
    let (_dir, url, _db) = database().await;
    let manager = ProviderManager::with_vault(&url, Arc::new(MemoryCredentialVault::default()))
        .await
        .unwrap();
    let mut server = mockito::Server::new_async().await;
    let _models = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .create_async()
        .await;
    let provider = manager
        .create_provider(
            config(&format!("{}/", server.url()), "synthetic-key"),
            "local".into(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(provider.base_url, server.url(), "trailing slash removed");
    // Resubmitting the same address with the slash is not an endpoint change.
    let updated = manager
        .update_provider(
            &provider.id,
            config(&format!("{}/", server.url()), ""),
            "local",
        )
        .await
        .unwrap();
    assert_eq!(updated.base_url, server.url());
}

#[tokio::test]
async fn stream_usage_is_recorded_once_with_the_reported_tokens() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut server = mockito::Server::new_async().await;
    let provider = crate::test_support::seed_provider(
        db,
        &vault,
        config(&server.url(), "synthetic-stream"),
        "local",
    )
    .await
    .unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let call = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(
            serde_json::json!({"stream": true, "stream_options": {"include_usage": true}}),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}\n\n",
            "data: [DONE]\n\n"
        ))
        .expect(1)
        .create_async()
        .await;
    let mut stream = manager
        .stream_provider(&provider.id, "m", user_request(), "local")
        .await
        .unwrap();
    let mut text = String::new();
    let mut last = None;
    while let Some(item) = stream.next().await {
        let chunk = item.unwrap();
        text.push_str(&chunk.delta);
        last = Some(chunk);
    }
    drop(stream);
    call.assert_async().await;
    assert_eq!(text, "ok");
    assert_eq!(last.unwrap().usage.unwrap().total_tokens, 18);
    let rows = usage_rows(&manager).await;
    assert_eq!(rows.len(), 1, "exactly one record per stream: {rows:?}");
    assert!(rows[0].success && rows[0].error_code.is_none());
    assert_eq!((rows[0].request_tokens, rows[0].response_tokens), (11, 7));
}

#[tokio::test]
async fn stream_failure_after_output_is_recorded_once_as_interrupted() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut server = mockito::Server::new_async().await;
    let provider = crate::test_support::seed_provider(
        db,
        &vault,
        config(&server.url(), "synthetic-stream"),
        "local",
    )
    .await
    .unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let _call = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"par\"},\"finish_reason\":null}]}\n\n",
            "event: error\ndata: {\"message\":\"overloaded\"}\n\n"
        ))
        .expect(1)
        .create_async()
        .await;
    let mut stream = manager
        .stream_provider(&provider.id, "m", user_request(), "local")
        .await
        .unwrap();
    let mut delivered = String::new();
    let mut errors = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => delivered.push_str(&chunk.delta),
            Err(error) => errors.push(error.error_code()),
        }
    }
    drop(stream);
    assert_eq!(delivered, "par");
    assert_eq!(errors, [1001007], "one interruption event");
    let rows = usage_rows(&manager).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(!rows[0].success);
    assert_eq!(rows[0].error_code.as_deref(), Some("1001007"));
}

#[tokio::test]
async fn stream_dropped_early_is_recorded_once_as_interrupted() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut server = mockito::Server::new_async().await;
    let provider = crate::test_support::seed_provider(
        db,
        &vault,
        config(&server.url(), "synthetic-stream"),
        "local",
    )
    .await
    .unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let _call = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"a\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"b\"},\"finish_reason\":null}]}\n\n"
        ))
        .create_async()
        .await;
    let mut stream = manager
        .stream_provider(&provider.id, "m", user_request(), "local")
        .await
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().delta, "a");
    drop(stream);
    let rows = usage_rows(&manager).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(!rows[0].success);
    assert_eq!(rows[0].error_code.as_deref(), Some("1001007"));
}

#[tokio::test]
async fn stream_rejected_before_it_starts_is_recorded_once_with_the_upstream_code() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut server = mockito::Server::new_async().await;
    let provider = crate::test_support::seed_provider(
        db,
        &vault,
        config(&server.url(), "synthetic-stream"),
        "local",
    )
    .await
    .unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    let _call = server
        .mock("POST", "/chat/completions")
        .with_status(401)
        .create_async()
        .await;
    let error = manager
        .stream_provider(&provider.id, "m", user_request(), "local")
        .await
        .err()
        .unwrap();
    assert_eq!(error.error_code(), 1001002);
    let rows = usage_rows(&manager).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].error_code.as_deref(), Some("1001002"));
}

// =============================================================================================
// V1-FR-001 round two at manager level: vendor validation (AC16), Anthropic creation (AC15),
// probe_model (AC17), max_tokens fill (AC18), usage records, structured output, cache rules.
// =============================================================================================

use dgos_db::ModelRepository;
use dgos_types::DgosError;
use mockito::Matcher;
use serde_json::json;

const SECRET: &str = "synthetic-r2-secret-key";
const LEAK_MARKER: &str = "UPSTREAM-LEAK-MARKER-7731";
const REASONING_MARKER: &str = "REASONING-MARKER-4410";
const KINDS: [&str; 2] = ["openai_compatible", "anthropic"];

fn typed(kind: &str, base: &str, key: &str, extra: Option<serde_json::Value>) -> ProviderConfig {
    ProviderConfig {
        name: "Round two fixture".into(),
        provider_type: kind.into(),
        base_url: base.into(),
        api_key: key.into(),
        enabled: true,
        config: extra,
    }
}

fn assert_no_leak(error: &DgosError) {
    let text = format!("{error} | {error:?}");
    assert!(!text.contains(SECRET), "{text}");
    assert!(!text.contains(LEAK_MARKER), "{text}");
    assert!(!text.contains(REASONING_MARKER), "{text}");
}

fn chat(max_tokens: Option<u32>, stream: bool) -> dgos_types::ChatRequest {
    dgos_types::ChatRequest {
        messages: vec![dgos_types::ChatMessage {
            role: "user".into(),
            content: "round two".into(),
        }],
        temperature: None,
        max_tokens,
        stream: Some(stream),
        ..Default::default()
    }
}

fn sample_model(id: &str) -> dgos_types::Model {
    dgos_types::Model {
        id: format!("forged-{id}"),
        provider_id: "forged-provider".into(),
        model_id: id.into(),
        model_name: id.into(),
        model_type: "text-generation".into(),
        ..Default::default()
    }
}

async fn fixture() -> (
    tempfile::TempDir,
    DatabaseConnection,
    Arc<MemoryCredentialVault>,
    ProviderManager,
) {
    let (dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault.clone())
        .await
        .unwrap();
    (dir, db, vault, manager)
}

async fn assert_nothing_saved(
    manager: &ProviderManager,
    vault: &MemoryCredentialVault,
    db: &DatabaseConnection,
    label: &str,
) {
    assert!(
        manager.list_providers("local").await.unwrap().is_empty(),
        "{label}: a provider was saved"
    );
    assert_eq!(
        vault.entry_count().await,
        0,
        "{label}: a credential was stored"
    );
    assert!(
        ProviderRepository::new(db.clone())
            .pending_credential_cleanup()
            .await
            .unwrap()
            .is_empty(),
        "{label}: credential cleanup left staged"
    );
}

/// `(label, provider_type, config, expected error code)`.
fn rejected_configs() -> Vec<(&'static str, &'static str, serde_json::Value, u32)> {
    vec![
        (
            "unknown vendor",
            "openai_compatible",
            json!({"vendor_id": "nope"}),
            1001022,
        ),
        (
            "unknown vendor on anthropic",
            "anthropic",
            json!({"vendor_id": "nope"}),
            1001022,
        ),
        (
            "anthropic vendor on openai_compatible",
            "openai_compatible",
            json!({"vendor_id": "anthropic"}),
            1001022,
        ),
        (
            "openai vendor on anthropic",
            "anthropic",
            json!({"vendor_id": "openai"}),
            1001022,
        ),
        (
            "number vendor_id",
            "openai_compatible",
            json!({"vendor_id": 7}),
            1001022,
        ),
        (
            "boolean vendor_id",
            "anthropic",
            json!({"vendor_id": true}),
            1001022,
        ),
        (
            "array vendor_id",
            "anthropic",
            json!({"vendor_id": ["custom"]}),
            1001022,
        ),
        (
            "empty vendor_id",
            "openai_compatible",
            json!({"vendor_id": ""}),
            1001022,
        ),
        (
            "unknown adapter",
            "gemini",
            json!({"vendor_id": "custom"}),
            1001022,
        ),
        (
            "auth_style basic",
            "openai_compatible",
            json!({"vendor_id": "custom", "auth_style": "basic"}),
            1001020,
        ),
        (
            "auth_style not a string",
            "anthropic",
            json!({"vendor_id": "custom", "auth_style": 1}),
            1001020,
        ),
        (
            "allow_insecure_http string",
            "openai_compatible",
            json!({"vendor_id": "custom", "allow_insecure_http": "yes"}),
            1001020,
        ),
        (
            "allow_insecure_http number",
            "anthropic",
            json!({"vendor_id": "custom", "allow_insecure_http": 1}),
            1001020,
        ),
        (
            "workspace_id with a space",
            "anthropic",
            json!({"vendor_id": "custom", "workspace_id": "has space"}),
            1001020,
        ),
        (
            "workspace_id too long",
            "anthropic",
            json!({"vendor_id": "custom", "workspace_id": "w".repeat(129)}),
            1001020,
        ),
        (
            "workspace_id not a string",
            "anthropic",
            json!({"vendor_id": "custom", "workspace_id": 5}),
            1001020,
        ),
        (
            "workspace_id non-ascii",
            "openai_compatible",
            json!({"vendor_id": "custom", "workspace_id": "w\u{f6}rk"}),
            1001020,
        ),
        (
            "vendor error wins over option error",
            "openai_compatible",
            json!({"vendor_id": "nope", "auth_style": "basic"}),
            1001022,
        ),
    ]
}

#[tokio::test]
async fn invalid_vendor_or_option_on_create_is_rejected_without_request_provider_or_credential() {
    let (_dir, db, vault, manager) = fixture().await;
    let mut server = mockito::Server::new_async().await;
    let any_get = server
        .mock("GET", Matcher::Any)
        .expect(0)
        .create_async()
        .await;
    let any_post = server
        .mock("POST", Matcher::Any)
        .expect(0)
        .create_async()
        .await;
    let base = server.url();
    for (label, kind, extra, code) in rejected_configs() {
        let created = manager
            .create_provider(
                typed(kind, &base, SECRET, Some(extra.clone())),
                "local".into(),
                Some("probe-model"),
            )
            .await;
        let error = created
            .err()
            .unwrap_or_else(|| panic!("create_provider accepted: {label}"));
        assert_eq!(error.error_code(), code, "create_provider: {label}");
        assert_no_leak(&error);

        let created = manager
            .create_provider_with_models(
                typed(kind, &base, SECRET, Some(extra)),
                "local".into(),
                vec![sample_model("m1")],
                Some("probe-model"),
            )
            .await;
        let error = created
            .err()
            .unwrap_or_else(|| panic!("create_provider_with_models accepted: {label}"));
        assert_eq!(
            error.error_code(),
            code,
            "create_provider_with_models: {label}"
        );
        assert_no_leak(&error);
    }
    any_get.assert_async().await;
    any_post.assert_async().await;
    assert_nothing_saved(&manager, &vault, &db, "create").await;
}

#[tokio::test]
async fn invalid_vendor_or_option_on_update_is_rejected_and_the_saved_provider_is_untouched() {
    let (_dir, db, vault, manager) = fixture().await;
    let mut server = mockito::Server::new_async().await;
    let list = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .expect(1)
        .create_async()
        .await;
    let base = server.url();
    let provider = manager
        .create_provider(
            typed(
                "openai_compatible",
                &base,
                SECRET,
                Some(json!({"vendor_id": "custom"})),
            ),
            "local".into(),
            None,
        )
        .await
        .unwrap();
    manager
        .add_manual_model(&provider.id, "local", "kept-model", None)
        .await
        .unwrap();
    let before = serde_json::to_value(
        manager
            .get_provider(&provider.id, "local")
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();

    for (label, kind, extra, code) in rejected_configs() {
        for key in ["", SECRET] {
            let updated = manager
                .update_provider(
                    &provider.id,
                    typed(kind, &base, key, Some(extra.clone())),
                    "local",
                )
                .await;
            let error = updated
                .err()
                .unwrap_or_else(|| panic!("update_provider accepted: {label} (key {key:?})"));
            assert_eq!(error.error_code(), code, "{label} (key {key:?})");
            assert_no_leak(&error);
        }
    }

    let after = serde_json::to_value(
        manager
            .get_provider(&provider.id, "local")
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        before, after,
        "a rejected update must not change the provider"
    );
    let ids: Vec<_> = manager
        .list_models(&provider.id, "local")
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.model_id)
        .collect();
    assert_eq!(ids, ["kept-model"]);
    assert_eq!(
        vault.entry_count().await,
        1,
        "no credential may be added or removed"
    );
    assert!(ProviderRepository::new(db)
        .pending_credential_cleanup()
        .await
        .unwrap()
        .is_empty());
    list.assert_async().await;
}

#[tokio::test]
async fn vendor_id_can_change_on_update_without_touching_credential_or_models() {
    let (_dir, _db, vault, manager) = fixture().await;
    let mut server = mockito::Server::new_async().await;
    let _list = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .create_async()
        .await;
    let base = server.url();
    let provider = manager
        .create_provider(
            typed(
                "openai_compatible",
                &base,
                SECRET,
                Some(json!({"vendor_id": "custom"})),
            ),
            "local".into(),
            None,
        )
        .await
        .unwrap();
    manager
        .add_manual_model(&provider.id, "local", "kept-model", None)
        .await
        .unwrap();
    let revision = manager
        .get_provider(&provider.id, "local")
        .await
        .unwrap()
        .unwrap()
        .config_revision;
    let updated = manager
        .update_provider(
            &provider.id,
            typed(
                "openai_compatible",
                &base,
                "",
                Some(json!({"vendor_id": "openai"})),
            ),
            "local",
        )
        .await
        .unwrap();
    assert_eq!(updated.config, Some(json!({"vendor_id": "openai"})));
    assert_eq!(updated.credential_ref, provider.credential_ref);
    assert_eq!(updated.config_revision, revision + 1);
    assert_eq!(vault.entry_count().await, 1);
    assert_eq!(
        manager
            .list_models(&provider.id, "local")
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn valid_vendor_and_adapter_combinations_save_and_use_their_own_wire_protocol() {
    for (kind, vendor, list_path) in [
        ("anthropic", "custom", "/v1/models?limit=100"),
        ("anthropic", "anthropic", "/v1/models?limit=100"),
        ("openai_compatible", "openai", "/models"),
        ("openai_compatible", "custom", "/models"),
    ] {
        let label = format!("{vendor} + {kind}");
        let (_dir, _db, vault, manager) = fixture().await;
        let mut server = mockito::Server::new_async().await;
        let bearer = format!("Bearer {SECRET}");
        let mut list = server
            .mock("GET", list_path)
            .match_header("authorization", bearer.as_str())
            .with_status(200)
            .with_body(r#"{"data":[]}"#)
            .expect(2);
        list = if kind == "anthropic" {
            list.match_header("x-api-key", SECRET)
                .match_header("anthropic-version", "2023-06-01")
        } else {
            list.match_header("x-api-key", Matcher::Missing)
                .match_header("anthropic-version", Matcher::Missing)
        };
        let list = list.create_async().await;
        let extra = json!({"vendor_id": vendor});
        let first = manager
            .create_provider(
                typed(kind, &server.url(), SECRET, Some(extra.clone())),
                "local".into(),
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("{label}: create_provider failed: {e}"));
        let second = manager
            .create_provider_with_models(
                typed(kind, &server.url(), SECRET, Some(extra.clone())),
                "local".into(),
                vec![sample_model("m1")],
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("{label}: create_provider_with_models failed: {e}"));
        for provider in [&first, &second] {
            assert_eq!(provider.provider_type, kind, "{label}");
            assert_eq!(provider.config, Some(extra.clone()), "{label}");
            assert_eq!(provider.base_url, server.url(), "{label}");
        }
        assert_eq!(vault.entry_count().await, 2, "{label}");

        // Same address and adapter: a metadata update needs no key and sends no request.
        let mut renamed = typed(kind, &server.url(), "", Some(extra.clone()));
        renamed.name = "Renamed".into();
        let updated = manager
            .update_provider(&first.id, renamed, "local")
            .await
            .unwrap_or_else(|e| panic!("{label}: update_provider failed: {e}"));
        assert_eq!(updated.name, "Renamed", "{label}");
        assert_eq!(updated.config, Some(extra), "{label}");
        list.assert_async().await;
    }
}

#[tokio::test]
async fn legacy_provider_without_vendor_id_is_created_and_called_as_before() {
    for extra in [None, Some(json!({})), Some(json!({"unrelated_key": 1}))] {
        let (_dir, _db, vault, manager) = fixture().await;
        let mut server = mockito::Server::new_async().await;
        let bearer = format!("Bearer {SECRET}");
        let list = server
            .mock("GET", "/models")
            .match_header("authorization", bearer.as_str())
            .with_status(200)
            .with_body(r#"{"data":[]}"#)
            .expect(1)
            .create_async()
            .await;
        let call = server
            .mock("POST", "/chat/completions")
            .match_header("authorization", bearer.as_str())
            .match_body(Matcher::PartialJson(
                json!({"model": "legacy-model", "max_tokens": 16, "stream": false}),
            ))
            .with_status(200)
            .with_body(
                json!({"choices": [{"message": {"content": "Legacy answer"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}})
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        let provider = manager
            .create_provider(
                typed("openai_compatible", &server.url(), SECRET, extra.clone()),
                "local".into(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(provider.config, extra);
        assert_eq!(vault.entry_count().await, 1);
        let response = manager
            .call_provider(&provider.id, "legacy-model", chat(Some(16), false), "local")
            .await
            .unwrap();
        assert_eq!(response.content, "Legacy answer");
        let rows = manager.list_usage_logs("local", None).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].success && rows[0].error_code.is_none());
        assert_eq!((rows[0].request_tokens, rows[0].response_tokens), (3, 2));
        list.assert_async().await;
        call.assert_async().await;
    }
}

#[tokio::test]
async fn retry_keys_coexist_with_vendor_id_and_drive_the_retry_count() {
    let (_dir, _db, _vault, manager) = fixture().await;
    let mut server = mockito::Server::new_async().await;
    let _list = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .create_async()
        .await;
    let extra =
        json!({"vendor_id": "custom", "max_retries": 2, "initial_delay_ms": 1, "max_delay_ms": 2});
    let provider = manager
        .create_provider(
            typed(
                "openai_compatible",
                &server.url(),
                SECRET,
                Some(extra.clone()),
            ),
            "local".into(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(provider.config, Some(extra));
    // One attempt plus the two configured retries; the default policy would make four.
    let failing = server
        .mock("POST", "/chat/completions")
        .with_status(503)
        .with_body(format!(r#"{{"error":{{"message":"{LEAK_MARKER}"}}}}"#))
        .expect(3)
        .create_async()
        .await;
    let error = manager
        .call_provider(&provider.id, "m", chat(Some(8), false), "local")
        .await
        .err()
        .unwrap();
    assert!(matches!(error, DgosError::MaxRetriesExceeded), "{error:?}");
    assert_eq!(error.error_code(), 1001008);
    assert_no_leak(&error);
    failing.assert_async().await;
    let rows = manager.list_usage_logs("local", None).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].error_code.as_deref(), Some("1001008"));
}

#[tokio::test]
async fn anthropic_creation_follows_pages_strips_v1_and_never_returns_the_key() {
    let (_dir, db, vault, manager) = fixture().await;
    let mut server = mockito::Server::new_async().await;
    let bearer = format!("Bearer {SECRET}");
    let page1 = server
        .mock("GET", "/v1/models?limit=100")
        .match_header("authorization", bearer.as_str())
        .match_header("x-api-key", SECRET)
        .match_header("anthropic-version", "2023-06-01")
        .with_status(200)
        .with_body(
            json!({"data": [{"type": "model", "id": "claude-a", "display_name": "Claude A",
                "max_input_tokens": 200000, "max_tokens": 64000,
                "capabilities": {"thinking": {"supported": true}}}],
                "has_more": true, "first_id": "claude-a", "last_id": "claude-a"})
            .to_string(),
        )
        // create (connection test), refresh, test_provider_by_id
        .expect(3)
        .create_async()
        .await;
    let page2 = server
        .mock("GET", "/v1/models?limit=100&after_id=claude-a")
        .match_header("authorization", bearer.as_str())
        .match_header("x-api-key", SECRET)
        .match_header("anthropic-version", "2023-06-01")
        .with_status(200)
        .with_body(
            json!({"data": [{"type": "model", "id": "claude-b", "display_name": "Claude B",
                "max_input_tokens": 100000, "max_tokens": 8192,
                "capabilities": {"image_input": {"supported": true}}}],
                "has_more": false, "first_id": "claude-b", "last_id": "claude-b"})
            .to_string(),
        )
        .expect(3)
        .create_async()
        .await;

    let provider = manager
        .create_provider(
            typed(
                "anthropic",
                &format!("{}/v1", server.url()),
                SECRET,
                Some(json!({"vendor_id": "custom"})),
            ),
            "local".into(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        provider.base_url,
        server.url(),
        "a pasted /v1 suffix is removed"
    );
    assert_eq!(provider.provider_type, "anthropic");
    assert_eq!(vault.entry_count().await, 1);
    let text = serde_json::to_string(&provider).unwrap();
    assert!(!text.contains(SECRET), "{text}");
    assert!(serde_json::to_value(&provider)
        .unwrap()
        .get("api_key")
        .is_none());
    let row = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT api_key, base_url FROM providers",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "api_key").unwrap(), "");
    assert_eq!(row.try_get::<String>("", "base_url").unwrap(), server.url());
    assert!(manager
        .list_models(&provider.id, "local")
        .await
        .unwrap()
        .is_empty());

    let mut refreshed = manager.refresh_models(&provider.id, "local").await.unwrap();
    refreshed.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    assert_eq!(refreshed.len(), 2, "both pages are stored");
    let (a, b) = (&refreshed[0], &refreshed[1]);
    assert_eq!(
        (a.model_id.as_str(), a.model_name.as_str()),
        ("claude-a", "Claude A")
    );
    assert_eq!(
        (b.model_id.as_str(), b.model_name.as_str()),
        ("claude-b", "Claude B")
    );
    for model in [a, b] {
        assert_eq!(model.provider_id, provider.id);
        assert_eq!(model.source, dgos_types::ModelSource::Remote);
        assert_eq!(model.model_type, "text-generation");
        assert_eq!(model.kind_source, dgos_types::KindSource::Protocol);
    }
    assert_eq!(
        (a.context_window, a.max_output_tokens),
        (Some(200000), Some(64000))
    );
    assert_eq!(
        (b.context_window, b.max_output_tokens),
        (Some(100000), Some(8192))
    );
    assert_eq!(a.capabilities.reasoning, dgos_types::Hint::Supported);
    assert_eq!(b.capabilities.vision, dgos_types::Hint::Supported);
    let text = serde_json::to_string(&refreshed).unwrap();
    assert!(!text.contains(SECRET), "{text}");

    let mut stored = manager.list_models(&provider.id, "local").await.unwrap();
    stored.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].max_output_tokens, Some(64000));
    assert_eq!(stored[1].context_window, Some(100000));
    assert!(manager
        .test_provider_by_id(&provider.id, "local", None)
        .await
        .unwrap());
    page1.assert_async().await;
    page2.assert_async().await;
}

/// `(list path, probe path)` of each adapter.
fn paths(kind: &str) -> (&'static str, &'static str) {
    if kind == "anthropic" {
        ("/v1/models?limit=100", "/v1/messages")
    } else {
        ("/models", "/chat/completions")
    }
}

#[tokio::test]
async fn missing_model_list_without_a_probe_model_saves_nothing() {
    for kind in KINDS {
        for status in [404, 405] {
            let label = format!("{kind} list {status}");
            let (_dir, db, vault, manager) = fixture().await;
            let mut server = mockito::Server::new_async().await;
            let (list_path, probe_path) = paths(kind);
            let list = server
                .mock("GET", list_path)
                .with_status(status)
                .expect(2)
                .create_async()
                .await;
            let probe = server
                .mock("POST", probe_path)
                .with_status(200)
                .with_body("{}")
                .expect(0)
                .create_async()
                .await;
            let plain = manager
                .create_provider(
                    typed(kind, &server.url(), SECRET, None),
                    "local".into(),
                    None,
                )
                .await
                .err()
                .unwrap_or_else(|| panic!("{label}: create_provider accepted"));
            assert!(
                matches!(plain, DgosError::RequestNotSupported),
                "{label}: {plain:?}"
            );
            assert_eq!(plain.error_code(), 1001020);
            let with_models = manager
                .create_provider_with_models(
                    typed(kind, &server.url(), SECRET, None),
                    "local".into(),
                    vec![sample_model("m1")],
                    None,
                )
                .await
                .err()
                .unwrap_or_else(|| panic!("{label}: create_provider_with_models accepted"));
            assert!(
                matches!(with_models, DgosError::RequestNotSupported),
                "{label}: {with_models:?}"
            );
            list.assert_async().await;
            probe.assert_async().await;
            assert_nothing_saved(&manager, &vault, &db, &label).await;
        }
    }
}

#[tokio::test]
async fn probe_response_decides_whether_a_service_without_a_model_list_is_saved() {
    for kind in KINDS {
        for status in [404, 405] {
            let label = format!("{kind} list {status}");
            let (list_path, probe_path) = paths(kind);
            let probe_body =
                Matcher::PartialJson(json!({"model": "probe-model", "max_tokens": 16}));

            // The probe is rejected: nothing is saved, on both creation paths.
            {
                let (_dir, db, vault, manager) = fixture().await;
                let mut server = mockito::Server::new_async().await;
                let list = server
                    .mock("GET", list_path)
                    .with_status(status)
                    .expect(2)
                    .create_async()
                    .await;
                let probe = server
                    .mock("POST", probe_path)
                    .match_body(probe_body.clone())
                    .with_status(401)
                    .with_body(format!(r#"{{"error":{{"message":"{LEAK_MARKER}"}}}}"#))
                    .expect(2)
                    .create_async()
                    .await;
                let plain = manager
                    .create_provider(
                        typed(kind, &server.url(), SECRET, None),
                        "local".into(),
                        Some("probe-model"),
                    )
                    .await
                    .err()
                    .unwrap_or_else(|| panic!("{label}: accepted"));
                let with_models = manager
                    .create_provider_with_models(
                        typed(kind, &server.url(), SECRET, None),
                        "local".into(),
                        vec![sample_model("m1")],
                        Some("probe-model"),
                    )
                    .await
                    .err()
                    .unwrap_or_else(|| panic!("{label}: accepted with models"));
                for error in [plain, with_models] {
                    assert!(
                        matches!(error, DgosError::InvalidApiKey),
                        "{label}: {error:?}"
                    );
                    assert_eq!(error.error_code(), 1001002);
                    assert_no_leak(&error);
                }
                list.assert_async().await;
                probe.assert_async().await;
                assert_nothing_saved(&manager, &vault, &db, &label).await;
            }

            // The probe succeeds: both creation paths save. Submitted models are recomputed.
            {
                let (_dir, _db, vault, manager) = fixture().await;
                let mut server = mockito::Server::new_async().await;
                let list = server
                    .mock("GET", list_path)
                    .with_status(status)
                    .expect(2)
                    .create_async()
                    .await;
                let probe = server
                    .mock("POST", probe_path)
                    .match_body(probe_body)
                    .with_status(200)
                    .with_body("{}")
                    .expect(2)
                    .create_async()
                    .await;
                let plain = manager
                    .create_provider(
                        typed(kind, &server.url(), SECRET, None),
                        "local".into(),
                        Some("probe-model"),
                    )
                    .await
                    .unwrap_or_else(|e| panic!("{label}: create_provider failed: {e}"));
                let mut forged = sample_model("text-embedding-3-small");
                forged.source = dgos_types::ModelSource::Manual;
                forged.kind_source = dgos_types::KindSource::Manual;
                forged.max_output_tokens = Some(999_999);
                let with_models = manager
                    .create_provider_with_models(
                        typed(kind, &server.url(), SECRET, None),
                        "local".into(),
                        vec![forged],
                        Some("probe-model"),
                    )
                    .await
                    .unwrap_or_else(|e| panic!("{label}: create_provider_with_models failed: {e}"));
                assert_eq!(
                    manager.list_providers("local").await.unwrap().len(),
                    2,
                    "{label}"
                );
                assert_eq!(vault.entry_count().await, 2, "{label}");
                assert!(manager
                    .list_models(&plain.id, "local")
                    .await
                    .unwrap()
                    .is_empty());
                let models = manager.list_models(&with_models.id, "local").await.unwrap();
                assert_eq!(models.len(), 1, "{label}");
                let model = &models[0];
                assert_eq!(model.provider_id, with_models.id, "{label}");
                assert_eq!(model.source, dgos_types::ModelSource::Remote, "{label}");
                assert_eq!(model.max_output_tokens, None, "{label}");
                if kind == "anthropic" {
                    assert_eq!(model.model_type, "text-generation", "{label}");
                    assert_eq!(
                        model.kind_source,
                        dgos_types::KindSource::Protocol,
                        "{label}"
                    );
                } else {
                    assert_eq!(model.model_type, "embedding", "{label}");
                    assert_eq!(model.kind_source, dgos_types::KindSource::Rule, "{label}");
                }
                list.assert_async().await;
                probe.assert_async().await;
            }
        }
    }
}

#[tokio::test]
async fn invalid_submitted_model_id_is_rejected_before_any_network_call() {
    for kind in KINDS {
        let (_dir, db, vault, manager) = fixture().await;
        let mut server = mockito::Server::new_async().await;
        let any_get = server
            .mock("GET", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        let any_post = server
            .mock("POST", Matcher::Any)
            .expect(0)
            .create_async()
            .await;
        for bad in [String::new(), "x".repeat(256)] {
            let error = manager
                .create_provider_with_models(
                    typed(kind, &server.url(), SECRET, None),
                    "local".into(),
                    vec![sample_model("fine"), sample_model(&bad)],
                    Some("probe-model"),
                )
                .await
                .err()
                .unwrap_or_else(|| panic!("{kind}: invalid model id accepted"));
            assert!(
                matches!(error, DgosError::RequestNotSupported),
                "{kind}: {error:?}"
            );
        }
        any_get.assert_async().await;
        any_post.assert_async().await;
        assert_nothing_saved(&manager, &vault, &db, kind).await;
    }
}

#[tokio::test]
async fn invalid_probe_model_or_rejected_model_list_never_reaches_the_probe_request() {
    for kind in KINDS {
        let (list_path, probe_path) = paths(kind);

        // An unusable probe model name: the list is asked, the probe is not sent.
        {
            let (_dir, db, vault, manager) = fixture().await;
            let mut server = mockito::Server::new_async().await;
            let list = server
                .mock("GET", list_path)
                .with_status(404)
                .expect(4)
                .create_async()
                .await;
            let probe = server
                .mock("POST", probe_path)
                .with_status(200)
                .with_body("{}")
                .expect(0)
                .create_async()
                .await;
            for name in ["", "   "] {
                let plain = manager
                    .create_provider(
                        typed(kind, &server.url(), SECRET, None),
                        "local".into(),
                        Some(name),
                    )
                    .await
                    .err()
                    .unwrap_or_else(|| panic!("{kind}: probe {name:?} accepted"));
                assert!(
                    matches!(plain, DgosError::RequestNotSupported),
                    "{kind}: {plain:?}"
                );
                let with_models = manager
                    .create_provider_with_models(
                        typed(kind, &server.url(), SECRET, None),
                        "local".into(),
                        vec![],
                        Some(name),
                    )
                    .await
                    .err()
                    .unwrap_or_else(|| panic!("{kind}: probe {name:?} accepted with models"));
                assert!(
                    matches!(with_models, DgosError::RequestNotSupported),
                    "{kind}: {with_models:?}"
                );
            }
            list.assert_async().await;
            probe.assert_async().await;
            assert_nothing_saved(&manager, &vault, &db, kind).await;
        }

        // A 401 on the model list is final: no probe, the key is reported as invalid.
        {
            let (_dir, db, vault, manager) = fixture().await;
            let mut server = mockito::Server::new_async().await;
            let list = server
                .mock("GET", list_path)
                .with_status(401)
                .with_body(format!(r#"{{"error":{{"message":"{LEAK_MARKER}"}}}}"#))
                .expect(2)
                .create_async()
                .await;
            let probe = server
                .mock("POST", probe_path)
                .with_status(200)
                .with_body("{}")
                .expect(0)
                .create_async()
                .await;
            let plain = manager
                .create_provider(
                    typed(kind, &server.url(), SECRET, None),
                    "local".into(),
                    Some("probe-model"),
                )
                .await
                .err()
                .unwrap();
            let with_models = manager
                .create_provider_with_models(
                    typed(kind, &server.url(), SECRET, None),
                    "local".into(),
                    vec![sample_model("m1")],
                    Some("probe-model"),
                )
                .await
                .err()
                .unwrap();
            for error in [plain, with_models] {
                assert!(
                    matches!(error, DgosError::InvalidApiKey),
                    "{kind}: {error:?}"
                );
                assert_no_leak(&error);
            }
            list.assert_async().await;
            probe.assert_async().await;
            assert_nothing_saved(&manager, &vault, &db, kind).await;
        }
    }
}

/// A saved provider (generic profile, loopback address) with the given model rows
/// `(model_id, max_output_tokens)`.
async fn seeded(
    kind: &str,
    base: &str,
    models: &[(&str, Option<i32>)],
) -> (tempfile::TempDir, ProviderManager, dgos_types::Provider) {
    let (dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = crate::test_support::seed_provider(
        db.clone(),
        &vault,
        typed(kind, base, SECRET, None),
        "local",
    )
    .await
    .unwrap();
    ModelRepository::new(db)
        .sync_models(
            &provider.id,
            models
                .iter()
                .map(|(id, max_output)| dgos_types::Model {
                    id: uuid::Uuid::new_v4().to_string(),
                    provider_id: provider.id.clone(),
                    model_id: (*id).into(),
                    model_name: (*id).into(),
                    model_type: "text-generation".into(),
                    max_output_tokens: *max_output,
                    ..Default::default()
                })
                .collect(),
        )
        .await
        .unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    (dir, manager, provider)
}

fn anthropic_reply(text: &str) -> String {
    json!({"id": "msg_1", "type": "message", "role": "assistant",
        "content": [{"type": "text", "text": text}], "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "cache_creation_input_tokens": 3,
            "cache_read_input_tokens": 2, "output_tokens": 7}})
    .to_string()
}

#[tokio::test]
async fn anthropic_max_tokens_comes_from_the_request_then_the_model_row_then_the_profile() {
    let mut server = mockito::Server::new_async().await;
    let (_dir, manager, provider) = seeded(
        "anthropic",
        &server.url(),
        &[("capped", Some(2048)), ("plain", None), ("zero", Some(0))],
    )
    .await;
    let mut mocks = Vec::new();
    for (model, max_tokens, reply) in [
        ("capped", 2048, "from the model row"),
        ("capped", 100, "from the request"),
        ("plain", 4096, "profile default"),
        ("zero", 4096, "zero is not a limit"),
        ("unseeded", 4096, "no model row"),
    ] {
        mocks.push(
            server
                .mock("POST", "/v1/messages")
                .match_body(Matcher::PartialJson(
                    json!({"model": model, "max_tokens": max_tokens, "stream": false}),
                ))
                .with_status(200)
                .with_body(anthropic_reply(reply))
                .expect(1)
                .create_async()
                .await,
        );
    }

    let first = manager
        .call_provider(&provider.id, "capped", chat(None, false), "local")
        .await
        .unwrap();
    assert_eq!(first.content, "from the model row");
    // Usage log: completion = output_tokens, prompt = sum of the input counters.
    let rows = manager.list_usage_logs("local", None).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].success && rows[0].error_code.is_none());
    assert_eq!((rows[0].request_tokens, rows[0].response_tokens), (15, 7));
    assert_eq!(rows[0].total_tokens, 22);
    assert_eq!(
        (first.usage.prompt_tokens, first.usage.completion_tokens),
        (15, 7)
    );

    let second = manager
        .call_provider(&provider.id, "capped", chat(Some(100), false), "local")
        .await
        .unwrap();
    assert_eq!(second.content, "from the request");
    for (model, expected) in [
        ("plain", "profile default"),
        ("zero", "zero is not a limit"),
        ("unseeded", "no model row"),
    ] {
        let response = manager
            .call_provider(&provider.id, model, chat(None, false), "local")
            .await
            .unwrap();
        assert_eq!(response.content, expected);
    }
    for mock in &mocks {
        mock.assert_async().await;
    }
    let rows = manager.list_usage_logs("local", None).await.unwrap();
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|row| row.success));
}

#[tokio::test]
async fn openai_compatible_gets_no_max_tokens_from_the_manager() {
    let mut server = mockito::Server::new_async().await;
    let (_dir, manager, provider) =
        seeded("openai_compatible", &server.url(), &[("chat", Some(2048))]).await;
    let reply = json!({"choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})
    .to_string();
    let absent = server
        .mock("POST", "/chat/completions")
        .match_request(|request| {
            let Ok(body) = request.body() else {
                return false;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
                return false;
            };
            value["model"] == "chat"
                && value.get("max_tokens").is_none()
                && value.get("max_completion_tokens").is_none()
        })
        .with_status(200)
        .with_body(reply.clone())
        .expect(1)
        .create_async()
        .await;
    manager
        .call_provider(&provider.id, "chat", chat(None, false), "local")
        .await
        .unwrap();
    absent.assert_async().await;

    let kept = server
        .mock("POST", "/chat/completions")
        .match_body(Matcher::PartialJson(
            json!({"model": "chat", "max_tokens": 100}),
        ))
        .with_status(200)
        .with_body(reply)
        .expect(1)
        .create_async()
        .await;
    manager
        .call_provider(&provider.id, "chat", chat(Some(100), false), "local")
        .await
        .unwrap();
    kept.assert_async().await;
}

fn sse(events: &[(&str, serde_json::Value)]) -> String {
    events
        .iter()
        .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
        .collect()
}

struct Drained {
    text: String,
    reasoning: String,
    errors: Vec<DgosError>,
    usage: Option<dgos_types::TokenUsage>,
}

async fn drain_chunks(mut stream: crate::traits::BoxStream<crate::ChatChunk>) -> Drained {
    let mut drained = Drained {
        text: String::new(),
        reasoning: String::new(),
        errors: Vec::new(),
        usage: None,
    };
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                drained.text.push_str(&chunk.delta);
                drained
                    .reasoning
                    .push_str(&chunk.reasoning_delta.unwrap_or_default());
                if chunk.usage.is_some() {
                    drained.usage = chunk.usage;
                }
            }
            Err(error) => drained.errors.push(error),
        }
    }
    drained
}

#[tokio::test]
async fn anthropic_stream_writes_one_success_record_with_tokens_from_the_events() {
    let mut server = mockito::Server::new_async().await;
    let (_dir, manager, provider) =
        seeded("anthropic", &server.url(), &[("claude-x", Some(2048))]).await;
    let body = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"id": "m", "type": "message",
            "role": "assistant", "content": [],
            "usage": {"input_tokens": 10, "cache_creation_input_tokens": 3,
                "cache_read_input_tokens": 2, "output_tokens": 1}}}),
        ),
        ("ping", json!({"type": "ping"})),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "thinking", "thinking": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "thinking_delta", "thinking": REASONING_MARKER}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 1,
            "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1,
            "delta": {"type": "text_delta", "text": "Hel"}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1,
            "delta": {"type": "text_delta", "text": "lo"}}),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 1}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 9}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    let call = server
        .mock("POST", "/v1/messages")
        .match_body(Matcher::PartialJson(
            json!({"model": "claude-x", "stream": true, "max_tokens": 2048}),
        ))
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .expect(1)
        .create_async()
        .await;
    let stream = manager
        .stream_provider(&provider.id, "claude-x", chat(None, true), "local")
        .await
        .unwrap();
    let drained = drain_chunks(stream).await;
    call.assert_async().await;
    assert!(drained.errors.is_empty(), "{:?}", drained.errors);
    assert_eq!(drained.text, "Hello");
    assert_eq!(drained.reasoning, REASONING_MARKER);
    let usage = drained.usage.unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (15, 9, 24)
    );

    let rows = usage_rows(&manager).await;
    assert_eq!(rows.len(), 1, "exactly one record per stream: {rows:?}");
    assert!(rows[0].success && rows[0].error_code.is_none());
    assert_eq!((rows[0].request_tokens, rows[0].response_tokens), (15, 9));
    assert_eq!(rows[0].total_tokens, 24);
    let serialized = serde_json::to_string(&rows).unwrap();
    assert!(!serialized.contains(REASONING_MARKER), "{serialized}");
}

#[tokio::test]
async fn anthropic_stream_interrupted_by_an_error_event_writes_one_failure_record() {
    let mut server = mockito::Server::new_async().await;
    let (_dir, manager, provider) = seeded("anthropic", &server.url(), &[]).await;
    let body = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"id": "m", "type": "message",
            "role": "assistant", "content": [], "usage": {"input_tokens": 4, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "par"}}),
        ),
        (
            "error",
            json!({"type": "error",
            "error": {"type": "overloaded_error", "message": LEAK_MARKER}}),
        ),
    ]);
    let _call = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_header("content-type", "text/event-stream")
        .with_body(body)
        .expect(1)
        .create_async()
        .await;
    let stream = manager
        .stream_provider(&provider.id, "claude-x", chat(Some(64), true), "local")
        .await
        .unwrap();
    let drained = drain_chunks(stream).await;
    assert_eq!(drained.text, "par");
    assert_eq!(
        drained
            .errors
            .iter()
            .map(DgosError::error_code)
            .collect::<Vec<_>>(),
        [1001007],
        "one interruption"
    );
    for error in &drained.errors {
        assert_no_leak(error);
    }
    let rows = usage_rows(&manager).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(!rows[0].success);
    assert_eq!(rows[0].error_code.as_deref(), Some("1001007"));
    assert_eq!((rows[0].request_tokens, rows[0].response_tokens), (0, 0));
    assert!(!serde_json::to_string(&rows).unwrap().contains(LEAK_MARKER));
}

fn answer_format() -> dgos_types::ResponseFormat {
    dgos_types::ResponseFormat {
        kind: "json_schema".into(),
        name: "answer".into(),
        schema: json!({"type": "object", "properties": {"n": {"type": "integer"}},
            "required": ["n"], "additionalProperties": false}),
        strict: None,
    }
}

fn openai_reply(content: &str, prompt: u32, completion: u32) -> String {
    json!({"choices": [{"message": {"content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": prompt, "completion_tokens": completion,
            "total_tokens": prompt + completion}})
    .to_string()
}

#[tokio::test]
async fn openai_structured_output_is_validated_and_the_usage_row_records_the_outcome() {
    let mut server = mockito::Server::new_async().await;
    let (_dir, manager, provider) = seeded("openai_compatible", &server.url(), &[]).await;
    let expected_format = json!({"response_format": {"type": "json_schema",
        "json_schema": {"name": "answer", "schema": answer_format().schema}}});
    let bad = server
        .mock("POST", "/chat/completions")
        .match_body(Matcher::PartialJson(
            json!({"model": "bad-model", "response_format": expected_format["response_format"]}),
        ))
        .with_status(200)
        .with_body(openai_reply(&format!(r#"{{"n":"{LEAK_MARKER}"}}"#), 4, 3))
        // Not retryable: one request only.
        .expect(1)
        .create_async()
        .await;
    let good = server
        .mock("POST", "/chat/completions")
        .match_body(Matcher::PartialJson(
            json!({"model": "good-model", "response_format": expected_format["response_format"]}),
        ))
        .with_status(200)
        .with_body(openai_reply(r#"{"n":3}"#, 6, 2))
        .expect(1)
        .create_async()
        .await;
    let request = || dgos_types::ChatRequest {
        response_format: Some(answer_format()),
        ..chat(Some(64), false)
    };

    let error = manager
        .call_provider(&provider.id, "bad-model", request(), "local")
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, DgosError::StructuredOutputInvalid),
        "{error:?}"
    );
    assert_eq!(error.error_code(), 1001021);
    assert_no_leak(&error);
    let rows = manager.list_usage_logs("local", None).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].success);
    assert_eq!(rows[0].error_code.as_deref(), Some("1001021"));
    assert_eq!((rows[0].request_tokens, rows[0].response_tokens), (4, 3));
    assert!(!serde_json::to_string(&rows).unwrap().contains(LEAK_MARKER));

    let response = manager
        .call_provider(&provider.id, "good-model", request(), "local")
        .await
        .unwrap();
    assert_eq!(response.structured, Some(json!({"n": 3})));
    assert_eq!(response.content, r#"{"n":3}"#);
    let rows = manager.list_usage_logs("local", None).await.unwrap();
    assert_eq!(rows.len(), 2);
    let success: Vec<_> = rows.iter().filter(|row| row.success).collect();
    assert_eq!(success.len(), 1);
    assert_eq!(
        (success[0].request_tokens, success[0].response_tokens),
        (6, 2)
    );
    bad.assert_async().await;
    good.assert_async().await;
}

#[tokio::test]
async fn anthropic_reasoning_and_structured_output_keep_thinking_out_of_the_usage_rows() {
    let mut server = mockito::Server::new_async().await;
    let (_dir, manager, provider) = seeded("anthropic", &server.url(), &[]).await;
    let reply = |text: &str| {
        json!({"id": "msg_1", "type": "message", "role": "assistant",
            "content": [
                {"type": "thinking", "thinking": REASONING_MARKER, "signature": "sig=="},
                {"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 11, "output_tokens": 5}})
        .to_string()
    };
    // Generic Anthropic profile: budget thinking, schema carried as a system instruction.
    let matcher = |model: &'static str| {
        Matcher::PartialJson(json!({"model": model, "max_tokens": 4096,
            "thinking": {"type": "enabled", "budget_tokens": 2048}}))
    };
    let has_instruction = |request: &mockito::Request| {
        request
            .body()
            .ok()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
            .and_then(|value| value["system"].as_str().map(|s| s.contains("JSON Schema")))
            .unwrap_or(false)
    };
    let good = server
        .mock("POST", "/v1/messages")
        .match_body(matcher("think-good"))
        .match_request(has_instruction)
        .with_status(200)
        .with_body(reply(r#"{"n":3}"#))
        .expect(1)
        .create_async()
        .await;
    let bad = server
        .mock("POST", "/v1/messages")
        .match_body(matcher("think-bad"))
        .match_request(has_instruction)
        .with_status(200)
        .with_body(reply(r#"{"n":"x"}"#))
        .expect(1)
        .create_async()
        .await;
    let request = || dgos_types::ChatRequest {
        reasoning: Some(dgos_types::ReasoningConfig {
            effort: Some("low".into()),
            budget_tokens: None,
        }),
        response_format: Some(answer_format()),
        ..chat(Some(4096), false)
    };

    let response = manager
        .call_provider(&provider.id, "think-good", request(), "local")
        .await
        .unwrap();
    assert_eq!(response.reasoning.as_deref(), Some(REASONING_MARKER));
    assert_eq!(response.structured, Some(json!({"n": 3})));
    assert_eq!(response.content, r#"{"n":3}"#);
    assert!(!response.content.contains(REASONING_MARKER));

    let error = manager
        .call_provider(&provider.id, "think-bad", request(), "local")
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, DgosError::StructuredOutputInvalid),
        "{error:?}"
    );
    assert_no_leak(&error);

    let rows = manager.list_usage_logs("local", None).await.unwrap();
    assert_eq!(rows.len(), 2);
    let serialized = serde_json::to_string(&rows).unwrap();
    assert!(!serialized.contains(REASONING_MARKER), "{serialized}");
    let ok = rows.iter().find(|row| row.success).unwrap();
    assert_eq!((ok.request_tokens, ok.response_tokens), (11, 5));
    let failed = rows.iter().find(|row| !row.success).unwrap();
    assert_eq!(failed.error_code.as_deref(), Some("1001021"));
    // The upstream answered (and billed) before the host rejected the answer: its numbers are
    // recorded, not 0.
    assert_eq!((failed.request_tokens, failed.response_tokens), (11, 5));
    good.assert_async().await;
    bad.assert_async().await;
}

async fn model_ids(manager: &ProviderManager, provider_id: &str) -> Vec<String> {
    let mut ids: Vec<String> = manager
        .list_models(provider_id, "local")
        .await
        .unwrap()
        .into_iter()
        .map(|model| model.model_id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn model_cache_is_cleared_only_by_credential_rotation_or_endpoint_change() {
    let (_dir, _db, vault, manager) = fixture().await;
    let mut first = mockito::Server::new_async().await;
    let mut second = mockito::Server::new_async().await;
    let bearer = |key: &str| format!("Bearer {key}");
    let _list_one = first
        .mock("GET", "/models")
        .match_header("authorization", bearer(SECRET).as_str())
        .with_status(200)
        .with_body(r#"{"data":[{"id":"remote-one"}]}"#)
        .create_async()
        .await;
    let _list_rotated = first
        .mock("GET", "/models")
        .match_header("authorization", bearer("rotated-key").as_str())
        .with_status(200)
        .with_body(r#"{"data":[{"id":"remote-two"}]}"#)
        .create_async()
        .await;
    let _list_moved = second
        .mock("GET", "/models")
        .match_header("authorization", bearer("moved-key").as_str())
        .with_status(200)
        .with_body(r#"{"data":[{"id":"remote-three"}]}"#)
        .create_async()
        .await;

    let provider = manager
        .create_provider_with_models(
            typed("openai_compatible", &first.url(), SECRET, None),
            "local".into(),
            vec![],
            None,
        )
        .await
        .unwrap();
    let populate = |remote: &'static str| {
        let manager = &manager;
        let id = provider.id.clone();
        async move {
            manager.refresh_models(&id, "local").await.unwrap();
            manager
                .add_manual_model(&id, "local", "manual-one", None)
                .await
                .unwrap();
            assert_eq!(model_ids(manager, &id).await, ["manual-one", remote]);
        }
    };
    populate("remote-one").await;

    // Name, enabled state and retry settings never clear the cache.
    let mut renamed = typed("openai_compatible", &first.url(), "", None);
    renamed.name = "Renamed".into();
    manager
        .update_provider(&provider.id, renamed, "local")
        .await
        .unwrap();
    assert_eq!(
        model_ids(&manager, &provider.id).await,
        ["manual-one", "remote-one"]
    );

    let mut disabled = typed("openai_compatible", &first.url(), "", None);
    disabled.enabled = false;
    manager
        .update_provider(&provider.id, disabled, "local")
        .await
        .unwrap();
    assert_eq!(
        model_ids(&manager, &provider.id).await,
        ["manual-one", "remote-one"]
    );
    manager
        .toggle_provider(&provider.id, true, "local")
        .await
        .unwrap();
    assert_eq!(
        model_ids(&manager, &provider.id).await,
        ["manual-one", "remote-one"]
    );

    let retry = json!({"max_retries": 1, "initial_delay_ms": 1, "max_delay_ms": 2});
    manager
        .update_provider(
            &provider.id,
            typed("openai_compatible", &first.url(), "", Some(retry)),
            "local",
        )
        .await
        .unwrap();
    assert_eq!(
        model_ids(&manager, &provider.id).await,
        ["manual-one", "remote-one"]
    );

    // An endpoint change without a new key is refused and changes nothing.
    let refused = manager
        .update_provider(
            &provider.id,
            typed("openai_compatible", &second.url(), "", None),
            "local",
        )
        .await
        .err()
        .unwrap();
    assert!(
        matches!(refused, DgosError::CredentialRequired),
        "{refused:?}"
    );
    assert_eq!(
        model_ids(&manager, &provider.id).await,
        ["manual-one", "remote-one"]
    );

    // Credential rotation clears remote and manual rows.
    manager
        .update_provider(
            &provider.id,
            typed("openai_compatible", &first.url(), "rotated-key", None),
            "local",
        )
        .await
        .unwrap();
    assert!(
        model_ids(&manager, &provider.id).await.is_empty(),
        "rotation keeps no rows"
    );
    assert_eq!(vault.entry_count().await, 1);
    // The rotated key is the one used from now on.
    populate("remote-two").await;

    // A changed base_url (with its key) clears both as well.
    let moved = manager
        .update_provider(
            &provider.id,
            typed("openai_compatible", &second.url(), "moved-key", None),
            "local",
        )
        .await
        .unwrap();
    assert_eq!(moved.base_url, second.url());
    assert!(
        model_ids(&manager, &provider.id).await.is_empty(),
        "endpoint change keeps no rows"
    );
    populate("remote-three").await;

    // A changed adapter type is an endpoint change too.
    manager
        .update_provider(
            &provider.id,
            typed("anthropic", &second.url(), "moved-key", None),
            "local",
        )
        .await
        .unwrap();
    assert!(
        model_ids(&manager, &provider.id).await.is_empty(),
        "adapter change keeps no rows"
    );
    assert_eq!(vault.entry_count().await, 1);
}
