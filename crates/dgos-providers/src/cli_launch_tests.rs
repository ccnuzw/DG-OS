use super::{config, database};
use crate::{
    test_support::MemoryCredentialVault, CliProviderProtocol, ProviderCredentialScope,
    ProviderCredentialVault, ProviderManager, SecretValue,
};
use async_trait::async_trait;
use dgos_db::ProviderRepository;
use dgos_types::DgosError;
use std::sync::Arc;
use tokio::sync::Notify;

async fn select_model(manager: &ProviderManager, provider_id: &str, model_id: &str) -> i64 {
    manager.add_manual_model(provider_id, "local", model_id, None).await.unwrap();
    manager.get_provider(provider_id, "local").await.unwrap().unwrap().config_revision
}

#[test]
fn provider_types_produce_only_unverified_protocol_candidates() {
    assert_eq!(CliProviderProtocol::candidate_for_provider_type("openai_compatible"), Some(CliProviderProtocol::OpenAiResponses));
    assert_eq!(CliProviderProtocol::candidate_for_provider_type("anthropic"), Some(CliProviderProtocol::AnthropicMessages));
    for unsupported in ["openai", "responses", "chat/completions", ""] {
        assert_eq!(CliProviderProtocol::candidate_for_provider_type(unsupported), None);
    }
}

#[tokio::test]
async fn launch_binds_owner_revision_model_and_normalized_base_without_protocol_claims() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault.clone()).await.unwrap();
    for (provider_type, base, expected_base, candidate) in [
        ("openai_compatible", "https://example.invalid/v1/", "https://example.invalid/v1", CliProviderProtocol::OpenAiResponses),
        ("anthropic", "https://example.invalid/v1/", "https://example.invalid", CliProviderProtocol::AnthropicMessages),
    ] {
        let mut options = config(base, "synthetic-cli-key");
        options.provider_type = provider_type.into();
        let provider = crate::test_support::seed_provider(db.clone(), &vault, options, "local").await.unwrap();
        let revision = select_model(&manager, &provider.id, "fixture-chat").await;
        let launch = manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, candidate).await.unwrap();
        let metadata = launch.metadata();
        assert_eq!(metadata.user_id, "local");
        assert_eq!(metadata.provider_id, provider.id);
        assert_eq!(metadata.config_revision, revision);
        assert_eq!(metadata.model.model_id, "fixture-chat");
        assert_eq!(metadata.base_url, expected_base);
        assert_eq!(metadata.protocol_candidate, candidate);
        assert!(!metadata.protocol_verified);
        assert!(!metadata.protocol_declared);
        assert_eq!(launch.credential().expose(), "synthetic-cli-key");
        assert!(!format!("{metadata:?}").contains("synthetic-cli-key"));
        assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "other", revision, candidate).await, Err(DgosError::ProviderNotFound(_))));
        assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision - 1, candidate).await, Err(DgosError::ProviderConfigurationChanged)));
        assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "missing", "local", revision, candidate).await, Err(DgosError::ModelNotFound(_))));
        manager.toggle_provider(&provider.id, false, "local").await.unwrap();
        assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, candidate).await, Err(DgosError::ProviderConfigurationChanged)));
        let disabled = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
        assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", disabled.config_revision, candidate).await, Err(DgosError::ProviderConfigurationChanged)));
    }
}

#[tokio::test]
async fn launch_rejects_non_text_models_and_rechecks_endpoint_policy() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault.clone()).await.unwrap();
    let provider = crate::test_support::seed_provider(db.clone(), &vault, config("https://example.invalid/v1", "synthetic-cli-key"), "local").await.unwrap();
    let revision = select_model(&manager, &provider.id, "text-embedding-3-small").await;
    assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "text-embedding-3-small", "local", revision, CliProviderProtocol::OpenAiResponses).await, Err(DgosError::RequestNotSupported)));
    let revision = select_model(&manager, &provider.id, "fixture-chat").await;
    // Simulates a configuration persisted by an older host: call-time validation still applies.
    use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "UPDATE providers SET base_url = ? WHERE id = ?", ["http://public.example/v1".into(), provider.id.clone().into()]
    )).await.unwrap();
    assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, CliProviderProtocol::OpenAiResponses).await, Err(DgosError::InsecureEndpoint)));
}

#[tokio::test]
async fn launch_never_falls_forward_after_credential_rotation_or_missing_vault_entry() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let manager = ProviderManager::with_vault(&url, vault.clone()).await.unwrap();
    let provider = crate::test_support::seed_provider(db, &vault, config("https://example.invalid/v1", "synthetic-old-key"), "local").await.unwrap();
    let revision = select_model(&manager, &provider.id, "fixture-chat").await;
    let rotated = manager.update_provider(&provider.id, config("https://example.invalid/v1", "synthetic-new-key"), "local").await.unwrap();
    // Rotation clears the model cache. The stale binding still fails as a revision conflict.
    assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, CliProviderProtocol::OpenAiResponses).await, Err(DgosError::ProviderConfigurationChanged)));
    let revision = select_model(&manager, &provider.id, "fixture-chat").await;
    let launch = manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, crate::CliProviderProtocol::OpenAiResponses).await.unwrap();
    assert_eq!(launch.credential().expose(), "synthetic-new-key");
    vault.remove(&ProviderCredentialScope {
        user_id: "local".into(), provider_id: provider.id.clone(), endpoint: rotated.base_url,
        credential_ref: rotated.credential_ref.unwrap(),
    }).await.unwrap();
    assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, CliProviderProtocol::OpenAiResponses).await, Err(DgosError::CredentialUnavailable)));
}

struct FailingVault;

#[async_trait]
impl ProviderCredentialVault for FailingVault {
    async fn store(&self, _: &ProviderCredentialScope, _: SecretValue) -> dgos_types::DgosResult<()> { unreachable!() }
    async fn load(&self, _: &ProviderCredentialScope) -> dgos_types::DgosResult<SecretValue> {
        Err(DgosError::InternalError("synthetic-secret-in-vault-error".into()))
    }
    async fn remove(&self, _: &ProviderCredentialScope) -> dgos_types::DgosResult<()> { unreachable!() }
}

#[tokio::test]
async fn launch_never_echoes_a_vault_error_or_reads_legacy_plaintext() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = crate::test_support::seed_provider(db.clone(), &vault, config("https://example.invalid/v1", "synthetic-cli-key"), "local").await.unwrap();
    let manager = ProviderManager::with_vault(&url, Arc::new(FailingVault)).await.unwrap();
    let revision = select_model(&manager, &provider.id, "fixture-chat").await;
    let error = manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, crate::CliProviderProtocol::OpenAiResponses).await.err().unwrap();
    assert!(matches!(error, DgosError::CredentialUnavailable));
    assert!(!format!("{error:?} {error}").contains("synthetic-secret"));
    use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "UPDATE providers SET credential_ref = NULL, api_key = 'synthetic-legacy-key' WHERE id = ?", [provider.id.clone().into()]
    )).await.unwrap();
    assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, CliProviderProtocol::OpenAiResponses).await, Err(DgosError::CredentialUnavailable)));
}

struct PausingVault {
    inner: Arc<MemoryCredentialVault>,
    loaded: Notify,
    release: Notify,
}

#[async_trait]
impl ProviderCredentialVault for PausingVault {
    async fn store(&self, _: &ProviderCredentialScope, _: SecretValue) -> dgos_types::DgosResult<()> { unreachable!() }
    async fn load(&self, scope: &ProviderCredentialScope) -> dgos_types::DgosResult<SecretValue> {
        let secret = self.inner.load(scope).await?;
        self.loaded.notify_one();
        self.release.notified().await;
        Ok(secret)
    }
    async fn remove(&self, _: &ProviderCredentialScope) -> dgos_types::DgosResult<()> { unreachable!() }
}

#[tokio::test]
async fn launch_detects_external_rotation_and_revocation_during_vault_read() {
    for revoke in [false, true] {
        let (_dir, url, db) = database().await;
        let memory = Arc::new(MemoryCredentialVault::default());
        let provider = crate::test_support::seed_provider(db.clone(), &memory, config("https://example.invalid/v1", "synthetic-old-key"), "local").await.unwrap();
        let writer = ProviderManager::with_vault(&url, memory.clone()).await.unwrap();
        let revision = select_model(&writer, &provider.id, "fixture-chat").await;
        let vault = Arc::new(PausingVault { inner: memory, loaded: Notify::new(), release: Notify::new() });
        let manager = Arc::new(ProviderManager::with_vault(&url, vault.clone()).await.unwrap());
        let id = provider.id.clone();
        let read = tokio::spawn(async move {
            manager.resolve_cli_provider_launch(&id, "fixture-chat", "local", revision, crate::CliProviderProtocol::OpenAiResponses).await
        });
        vault.loaded.notified().await;
        if revoke {
            ProviderRepository::new(db).delete(&provider.id, "local").await.unwrap();
        } else {
            writer.update_provider(&provider.id, config("https://example.invalid/v1", "synthetic-new-key"), "local").await.unwrap();
        }
        vault.release.notify_one();
        let error = read.await.unwrap().err().unwrap();
        if revoke {
            assert!(matches!(error, DgosError::ProviderNotFound(_)));
        } else {
            assert!(matches!(error, DgosError::ProviderConfigurationChanged));
        }
    }
}

#[tokio::test]
async fn dual_protocol_launches_use_requested_route_and_only_the_original_provider_vault_scope() {
    use crate::{resolve_cli_provider_route, AuthStyle};
    use serde_json::json;
    for provider_type in ["openai_compatible", "anthropic"] {
        let (_dir, url, db) = database().await;
        let vault = Arc::new(MemoryCredentialVault::default());
        let manager = ProviderManager::with_vault(&url, vault.clone()).await.unwrap();
        let mut options = config("https://chat.example.invalid/v1", "synthetic-original-provider-key");
        options.provider_type = provider_type.into();
        options.config = Some(json!({"display_note": "preserved", "cli_routes": {
            "responses": {"enabled": true, "base_url": "https://responses.example.invalid/v1/"},
            "messages": {"enabled": true, "base_url": "https://messages.example.invalid/v1/", "auth_style": "x-api-key"}
        }}));
        let provider = crate::test_support::seed_provider(db, &vault, options, "local").await.unwrap();
        let revision = select_model(&manager, &provider.id, "fixture-chat").await;
        // A decoy under the override scope must never replace the original Provider key.
        for endpoint in ["https://responses.example.invalid/v1", "https://messages.example.invalid"] {
            let scope = ProviderCredentialScope { user_id: "local".into(), provider_id: provider.id.clone(),
                endpoint: endpoint.into(), credential_ref: provider.credential_ref.clone().unwrap() };
            vault.store(&scope, SecretValue::new("synthetic-wrong-override-key".into()).unwrap()).await.unwrap();
        }
        for (protocol, expected_auth) in [(CliProviderProtocol::OpenAiResponses, AuthStyle::Bearer),
            (CliProviderProtocol::AnthropicMessages, AuthStyle::XApiKey)] {
            let route = resolve_cli_provider_route(provider_type, &provider.base_url, provider.config.as_ref(), protocol).unwrap();
            let launch = manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision, protocol).await.unwrap();
            let metadata = launch.metadata();
            assert_eq!(metadata.provider_type, provider_type);
            assert_eq!(metadata.protocol_candidate, protocol);
            assert!(metadata.protocol_declared);
            assert!(!metadata.protocol_verified);
            assert_eq!(metadata.base_url, route.base_url);
            assert_eq!(metadata.auth_style, expected_auth);
            assert_eq!(metadata.anthropic_version, route.anthropic_version);
            assert_eq!(launch.credential().expose(), "synthetic-original-provider-key");
        }
        let saved = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
        assert_eq!(saved.base_url, provider.base_url);
        assert_eq!(saved.config, provider.config);
    }
}

#[tokio::test]
async fn disabled_requested_route_never_falls_back_or_reads_credentials() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let mut options = config("https://chat.example.invalid/v1", "synthetic-provider-key");
    options.config = Some(serde_json::json!({"cli_routes": {
        "responses": {"enabled": false}, "messages": {"enabled": true}
    }}));
    let provider = crate::test_support::seed_provider(db, &vault, options, "local").await.unwrap();
    let manager = ProviderManager::with_vault(&url, Arc::new(FailingVault)).await.unwrap();
    let revision = select_model(&manager, &provider.id, "fixture-chat").await;
    assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision,
        CliProviderProtocol::OpenAiResponses).await, Err(DgosError::CliProviderProtocolUnsupported)));
    assert!(matches!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision,
        CliProviderProtocol::AnthropicMessages).await, Err(DgosError::CredentialUnavailable)));
}

#[tokio::test]
async fn launch_revalidates_invalid_persisted_cli_routes_before_vault_access() {
    for (routes, expected_code) in [
        (serde_json::json!({"responses": {"enabled": true, "base_url": "https://route.example.invalid/v1?key=synthetic"}}), 1001028),
        (serde_json::json!({"responses": {"enabled": true, "auth_style": "both"}}), 1001027),
        (serde_json::json!({"responses": {"enabled": true, "unknown": true}}), 1001028),
    ] {
        let (_dir, url, db) = database().await;
        let vault = Arc::new(MemoryCredentialVault::default());
        let mut options = config("https://chat.example.invalid/v1", "synthetic-provider-key");
        options.config = Some(serde_json::json!({"cli_routes": routes}));
        // Fixture storage deliberately bypasses save validation, as an older host might have.
        let provider = crate::test_support::seed_provider(db, &vault, options, "local").await.unwrap();
        let manager = ProviderManager::with_vault(&url, Arc::new(FailingVault)).await.unwrap();
        let revision = select_model(&manager, &provider.id, "fixture-chat").await;
        let error = manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", revision,
            CliProviderProtocol::OpenAiResponses).await.err().unwrap();
        assert_eq!(error.error_code(), expected_code);
        assert!(!format!("{error:?} {error}").contains("synthetic"));
    }
}
