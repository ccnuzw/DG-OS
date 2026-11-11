use super::{config, database};
use crate::{test_support::MemoryCredentialVault, CliProviderBinding, CliProviderChange,
    CliProviderChangeKind, CliProviderChangeSink, ProviderCredentialScope,
    ProviderCredentialVault, ProviderManager, SecretValue};
use async_trait::async_trait;
use dgos_types::{DgosError, DgosResult};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Sink(Mutex<Vec<(CliProviderChange, Vec<CliProviderBinding>)>>);
impl CliProviderChangeSink for Sink {
    fn changed(&self, change: &CliProviderChange, targets: &[CliProviderBinding]) {
        self.0.lock().unwrap().push((change.clone(), targets.to_vec()));
    }
}

async fn setup() -> (tempfile::TempDir, ProviderManager, Arc<MemoryCredentialVault>, dgos_types::Provider, Arc<Sink>, dgos_db::sea_orm::DatabaseConnection) {
    let (dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = crate::test_support::seed_provider(db.clone(), &vault,
        config("https://example.invalid/v1", "synthetic-key"), "local").await.unwrap();
    let manager = ProviderManager::with_vault(&url, vault.clone()).await.unwrap();
    manager.add_manual_model(&provider.id, "local", "fixture-chat", None).await.unwrap();
    let provider = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    let sink = Arc::new(Sink::default());
    manager.install_cli_change_sink(sink.clone()).unwrap();
    (dir, manager, vault, provider, sink, db)
}

fn binding(provider: &dgos_types::Provider) -> CliProviderBinding {
    CliProviderBinding { binding_id: uuid::Uuid::new_v4().to_string(), owner: provider.user_id.clone(),
        session_id: uuid::Uuid::new_v4().to_string(), host_instance_id: "fixture-host".into(),
        provider_id: provider.id.clone(), model_id: "fixture-chat".into(),
        provider_revision: provider.config_revision }
}

#[tokio::test]
async fn opening_revocation_blocks_spawn_and_reenable_never_revives_old_lease() {
    let (_dir, manager, _vault, provider, sink, _db) = setup().await;
    let parent = CancellationToken::new();
    let lease = manager.register_cli_opening(binding(&provider), &parent).await.unwrap();
    let launch = manager.resolve_cli_provider_for_lease(&lease, crate::CliProviderProtocol::OpenAiResponses).await.unwrap();
    manager.clone().toggle_provider(&provider.id, false, "local").await.unwrap();
    assert!(lease.is_cancelled());
    assert!(!parent.is_cancelled());
    let spawned = AtomicBool::new(false);
    assert!(manager.with_cli_spawn(&lease, &launch, |_| { spawned.store(true, Ordering::SeqCst); Ok(()) }).await.is_err());
    assert!(!spawned.load(Ordering::SeqCst));
    let enabled = manager.toggle_provider(&provider.id, true, "local").await.unwrap();
    assert!(manager.validate_cli_lease(&lease).await.is_err());
    let fresh = manager.register_cli_opening(binding(&enabled), &parent).await.unwrap();
    let fresh_launch = manager.resolve_cli_provider_for_lease(&fresh, crate::CliProviderProtocol::OpenAiResponses).await.unwrap();
    let observed = manager.with_cli_spawn(&fresh, &fresh_launch, |launch| Ok(launch.metadata().config_revision)).await.unwrap();
    assert_eq!(observed, enabled.config_revision);
    let duplicated = AtomicBool::new(false);
    assert!(matches!(manager.with_cli_spawn(&fresh, &fresh_launch, |_| {
        duplicated.store(true, Ordering::SeqCst); Ok(())
    }).await, Err(DgosError::ProviderRequestRejected)));
    assert!(!duplicated.load(Ordering::SeqCst));
    assert!(!fresh.is_cancelled());
    let events = sink.0.lock().unwrap();
    assert_eq!(events[0].1, vec![lease.binding().clone()]);
    assert!(events[1].1.is_empty());
}

#[tokio::test]
async fn metadata_and_order_preserve_process_but_rotation_and_delete_cancel_only_their_scope() {
    let (_dir, manager, vault, provider, sink, db) = setup().await;
    let other = crate::test_support::seed_provider(db.clone(), &vault,
        config("https://other.invalid/v1", "synthetic-other"), "other-owner").await.unwrap();
    manager.add_manual_model(&other.id, "other-owner", "fixture-chat", None).await.unwrap();
    let other = manager.get_provider(&other.id, "other-owner").await.unwrap().unwrap();
    let same_owner = crate::test_support::seed_provider(db, &vault,
        config("https://third.invalid/v1", "synthetic-third"), "local").await.unwrap();
    manager.add_manual_model(&same_owner.id, "local", "fixture-chat", None).await.unwrap();
    let same_owner = manager.get_provider(&same_owner.id, "local").await.unwrap().unwrap();
    let token = CancellationToken::new();
    let lease = manager.register_cli_opening(binding(&provider), &token).await.unwrap();
    let other_lease = manager.register_cli_opening(binding(&other), &token).await.unwrap();
    let third_lease = manager.register_cli_opening(binding(&same_owner), &token).await.unwrap();
    let mut renamed = config(&provider.base_url, "");
    renamed.name = "Renamed".into();
    manager.update_provider(&provider.id, renamed, "local").await.unwrap();
    manager.update_provider_order(vec![provider.id.clone(), same_owner.id.clone()], "local").await.unwrap();
    assert!(!lease.is_cancelled());
    assert!(manager.validate_cli_lease(&lease).await.is_err()); // old exact revision cannot send again
    manager.update_provider(&provider.id, config("https://new.invalid/v1", "synthetic-rotated"), "local").await.unwrap();
    assert!(lease.is_cancelled());
    assert!(!other_lease.is_cancelled());
    assert!(!third_lease.is_cancelled());
    manager.delete_provider(&same_owner.id, "local").await.unwrap();
    assert!(third_lease.is_cancelled());
    assert!(!other_lease.is_cancelled());
    assert!(!token.is_cancelled());
    assert!(sink.0.lock().unwrap().iter().any(|(change, targets)| change.kind == CliProviderChangeKind::MetadataChanged && targets.is_empty()));
}

struct ControlledVault {
    inner: Arc<MemoryCredentialVault>,
    pause: AtomicBool,
    fail_store: AtomicBool,
    loaded: Notify,
    release: Notify,
}
#[async_trait]
impl ProviderCredentialVault for ControlledVault {
    async fn store(&self, scope: &ProviderCredentialScope, secret: SecretValue) -> DgosResult<()> {
        if self.fail_store.load(Ordering::SeqCst) { return Err(DgosError::CredentialUnavailable); }
        self.inner.store(scope, secret).await
    }
    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue> {
        let secret = self.inner.load(scope).await?;
        if self.pause.swap(false, Ordering::SeqCst) {
            self.loaded.notify_one();
            self.release.notified().await;
        }
        Ok(secret)
    }
    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()> { self.inner.remove(scope).await }
}

#[tokio::test]
async fn resolving_lease_cancelled_during_vault_read_never_returns_a_launch() {
    let (_dir, url, db) = database().await;
    let memory = Arc::new(MemoryCredentialVault::default());
    let provider = crate::test_support::seed_provider(db, &memory,
        config("https://example.invalid/v1", "synthetic-key"), "local").await.unwrap();
    let vault = Arc::new(ControlledVault { inner: memory, pause: AtomicBool::new(false),
        fail_store: AtomicBool::new(false), loaded: Notify::new(), release: Notify::new() });
    let manager = ProviderManager::with_vault(&url, vault.clone()).await.unwrap();
    manager.add_manual_model(&provider.id, "local", "fixture-chat", None).await.unwrap();
    manager.install_cli_change_sink(Arc::new(Sink::default())).unwrap();
    let provider = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    let lease = Arc::new(manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap());
    vault.pause.store(true, Ordering::SeqCst);
    let reader = manager.clone();
    let reading = lease.clone();
    let task = tokio::spawn(async move { reader.resolve_cli_provider_for_lease(&reading, crate::CliProviderProtocol::OpenAiResponses).await });
    vault.loaded.notified().await;
    lease.cancel();
    vault.release.notify_one();
    assert!(matches!(task.await.unwrap(), Err(DgosError::ProviderConfigurationChanged)));
    assert!(manager.validate_cli_lease(&lease).await.is_err());
}

#[tokio::test]
async fn failed_credential_store_preserves_binding_and_emits_no_success_notification() {
    let (_dir, url, db) = database().await;
    let memory = Arc::new(MemoryCredentialVault::default());
    let provider = crate::test_support::seed_provider(db, &memory,
        config("https://example.invalid/v1", "synthetic-key"), "local").await.unwrap();
    let vault = Arc::new(ControlledVault { inner: memory, pause: AtomicBool::new(false),
        fail_store: AtomicBool::new(true), loaded: Notify::new(), release: Notify::new() });
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    manager.add_manual_model(&provider.id, "local", "fixture-chat", None).await.unwrap();
    let sink = Arc::new(Sink::default());
    manager.install_cli_change_sink(sink.clone()).unwrap();
    let before = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    let lease = manager.register_cli_opening(binding(&before), &CancellationToken::new()).await.unwrap();
    assert!(manager.update_provider(&provider.id, config(&provider.base_url, "synthetic-new"), "local").await.is_err());
    let after = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    assert_eq!(after.config_revision, before.config_revision);
    assert_eq!(after.credential_ref, before.credential_ref);
    assert!(!lease.is_cancelled());
    assert!(sink.0.lock().unwrap().is_empty());
    assert_eq!(manager.resolve_cli_provider_for_lease(&lease, crate::CliProviderProtocol::OpenAiResponses).await.unwrap().credential().expose(), "synthetic-key");
}

#[tokio::test]
async fn missing_sink_rejects_cli_registration_without_breaking_regular_provider_access() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = crate::test_support::seed_provider(db, &vault,
        config("https://example.invalid/v1", "synthetic-key"), "local").await.unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    manager.add_manual_model(&provider.id, "local", "fixture-chat", None).await.unwrap();
    let provider = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    assert!(manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.is_err());
    assert_eq!(manager.model_snapshot(&provider.id, "fixture-chat", "local").await.unwrap().0.id, provider.id);
    assert!(manager.resolve_cli_provider_launch(&provider.id, "fixture-chat", "local", provider.config_revision, crate::CliProviderProtocol::OpenAiResponses).await.is_ok());
}

#[tokio::test]
async fn initial_provider_revision_zero_can_register_resolve_and_spawn() {
    let (_dir, url, db) = database().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let scope = ProviderCredentialScope { user_id: "local".into(), provider_id: uuid::Uuid::new_v4().to_string(),
        endpoint: "https://example.invalid/v1".into(), credential_ref: crate::new_credential_ref() };
    vault.store(&scope, SecretValue::new("synthetic-key".into()).unwrap()).await.unwrap();
    let model = dgos_types::build_model("openai_compatible", "fixture-chat", None, dgos_types::ModelSource::Manual, None);
    let provider = dgos_db::ProviderRepository::new(db).create_with_models(scope.provider_id.clone(),
        config(&scope.endpoint, ""), "local".into(), Some(scope.credential_ref), vec![model]).await.unwrap();
    assert_eq!(provider.config_revision, 0);
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    manager.install_cli_change_sink(Arc::new(Sink::default())).unwrap();
    let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    let launch = manager.resolve_cli_provider_for_lease(&lease, crate::CliProviderProtocol::OpenAiResponses).await.unwrap();
    assert_eq!(manager.with_cli_spawn(&lease, &launch, |launch| Ok(launch.metadata().config_revision)).await.unwrap(), 0);
}

#[tokio::test]
async fn disable_queued_during_resolution_cannot_leave_an_authorized_stale_spawn() {
    let (_dir, url, db) = database().await;
    let memory = Arc::new(MemoryCredentialVault::default());
    let provider = crate::test_support::seed_provider(db, &memory,
        config("https://example.invalid/v1", "synthetic-key"), "local").await.unwrap();
    let vault = Arc::new(ControlledVault { inner: memory, pause: AtomicBool::new(false),
        fail_store: AtomicBool::new(false), loaded: Notify::new(), release: Notify::new() });
    let manager = ProviderManager::with_vault(&url, vault.clone()).await.unwrap();
    manager.add_manual_model(&provider.id, "local", "fixture-chat", None).await.unwrap();
    manager.install_cli_change_sink(Arc::new(Sink::default())).unwrap();
    let provider = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    let lease = Arc::new(manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap());
    vault.pause.store(true, Ordering::SeqCst);
    let reader = manager.clone();
    let reading = lease.clone();
    let resolve = tokio::spawn(async move { reader.resolve_cli_provider_for_lease(&reading, crate::CliProviderProtocol::OpenAiResponses).await });
    vault.loaded.notified().await;
    let writer = manager.clone();
    let id = provider.id.clone();
    let disable = tokio::spawn(async move { writer.toggle_provider(&id, false, "local").await });
    vault.release.notify_one();
    let resolved = resolve.await.unwrap();
    disable.await.unwrap().unwrap();
    assert!(lease.is_cancelled());
    if let Ok(launch) = resolved {
        let spawned = AtomicBool::new(false);
        assert!(manager.with_cli_spawn(&lease, &launch, |_| { spawned.store(true, Ordering::SeqCst); Ok(()) }).await.is_err());
        assert!(!spawned.load(Ordering::SeqCst));
    }
    assert!(manager.validate_cli_lease(&lease).await.is_err());
}

#[tokio::test]
async fn retry_metadata_and_equivalent_auth_defaults_preserve_native_processes() {
    let (_dir, manager, _vault, provider, sink, _db) = setup().await;
    let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    let mut update = config(&provider.base_url, "");
    update.config = Some(serde_json::json!({"max_retries": 2, "display_note": "local", "auth_style": "bearer"}));
    manager.update_provider(&provider.id, update, "local").await.unwrap();
    assert!(!lease.is_cancelled());
    assert!(sink.0.lock().unwrap().iter().all(|(change, targets)| change.kind == CliProviderChangeKind::MetadataChanged && targets.is_empty()));
    let mut update = config(&provider.base_url, "");
    update.config = Some(serde_json::json!({"auth_style": "both"}));
    manager.update_provider(&provider.id, update, "local").await.unwrap();
    assert!(lease.is_cancelled());
    assert_eq!(sink.0.lock().unwrap().last().unwrap().1, vec![lease.binding().clone()]);
}

#[tokio::test]
async fn removing_one_manual_model_preserves_other_selected_models() {
    let (_dir, manager, _vault, provider, sink, _db) = setup().await;
    manager.add_manual_model(&provider.id, "local", "other-chat", None).await.unwrap();
    let provider = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    let a = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    let mut selected_b = binding(&provider); selected_b.model_id = "other-chat".into();
    let b = manager.register_cli_opening(selected_b, &CancellationToken::new()).await.unwrap();
    manager.remove_manual_model(&provider.id, "local", "other-chat").await.unwrap();
    assert!(!a.is_cancelled()); assert!(b.is_cancelled());
    assert_eq!(sink.0.lock().unwrap().last().unwrap().1, vec![b.binding().clone()]);
}

#[tokio::test]
async fn refresh_preserves_manual_and_still_eligible_remote_models() {
    let (_dir, url, db) = database().await;
    let mut server = mockito::Server::new_async().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = crate::test_support::seed_provider(db, &vault, config(&server.url(), "synthetic-key"), "local").await.unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    manager.add_manual_model(&provider.id, "local", "fixture-chat", None).await.unwrap();
    let first = server.mock("GET", "/models").with_status(200).with_body(r#"{"data":[{"id":"remote-kept"},{"id":"remote-removed"}]}"#).create_async().await;
    manager.refresh_models(&provider.id, "local").await.unwrap(); first.remove_async().await;
    let sink = Arc::new(Sink::default()); manager.install_cli_change_sink(sink.clone()).unwrap();
    let provider = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    let manual = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    let mut selected = binding(&provider); selected.model_id = "remote-kept".into();
    let kept = manager.register_cli_opening(selected, &CancellationToken::new()).await.unwrap();
    let mut selected = binding(&provider); selected.model_id = "remote-removed".into();
    let removed = manager.register_cli_opening(selected, &CancellationToken::new()).await.unwrap();
    let next = server.mock("GET", "/models").with_status(200).with_body(r#"{"data":[{"id":"remote-kept"}]}"#).create_async().await;
    manager.refresh_models(&provider.id, "local").await.unwrap(); next.assert_async().await;
    assert!(!manual.is_cancelled()); assert!(!kept.is_cancelled()); assert!(removed.is_cancelled());
    assert_eq!(sink.0.lock().unwrap().last().unwrap().1, vec![removed.binding().clone()]);
}

#[tokio::test]
async fn meaningful_cli_route_changes_cancel_leases_without_clearing_provider_models_or_keys() {
    use crate::CliProviderProtocol::{AnthropicMessages, OpenAiResponses};
    use serde_json::json;
    for changed_routes in [
        json!({"responses": {"enabled": false}, "messages": {"enabled": true, "base_url": "https://messages.example.invalid/v1/", "auth_style": "both"}}),
        json!({"responses": {"enabled": true, "base_url": "https://responses.example.invalid/v1"}, "messages": {"enabled": true, "base_url": "https://messages.example.invalid/v1/", "auth_style": "both"}}),
        json!({"responses": {"enabled": true}, "messages": {"enabled": false}}),
        json!({"responses": {"enabled": true}, "messages": {"enabled": true, "base_url": "https://changed.example.invalid/v1", "auth_style": "both"}}),
        json!({"responses": {"enabled": true}, "messages": {"enabled": true, "base_url": "https://messages.example.invalid/v1/", "auth_style": "x-api-key"}}),
    ] {
        let (_dir, manager, vault, provider, sink, _db) = setup().await;
        let mut initial = config(&provider.base_url, "");
        initial.config = Some(json!({"cli_routes": {
            "responses": {"enabled": true},
            "messages": {"enabled": true, "base_url": "https://messages.example.invalid/v1/", "auth_style": "both"}
        }}));
        let provider = manager.update_provider(&provider.id, initial, "local").await.unwrap();
        let cached = serde_json::to_value(manager.list_models(&provider.id, "local").await.unwrap()).unwrap();
        let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
        let launch = manager.resolve_cli_provider_for_lease(&lease, OpenAiResponses).await.unwrap();
        sink.0.lock().unwrap().clear();
        let mut update = config(&provider.base_url, "");
        let saved_config = json!({"display_note": {"preserve": true}, "cli_routes": changed_routes});
        update.config = Some(saved_config.clone());
        let after = manager.update_provider(&provider.id, update, "local").await.unwrap();
        assert!(after.config_revision > provider.config_revision);
        assert_eq!(after.base_url, provider.base_url);
        assert_eq!(after.provider_type, provider.provider_type);
        assert_eq!(after.config, Some(saved_config));
        assert_eq!(after.credential_ref, provider.credential_ref);
        assert_eq!(vault.entry_count().await, 1);
        assert_eq!(serde_json::to_value(manager.list_models(&provider.id, "local").await.unwrap()).unwrap(), cached);
        assert!(lease.is_cancelled());
        assert!(matches!(manager.resolve_cli_provider_for_lease(&lease, OpenAiResponses).await, Err(DgosError::ProviderConfigurationChanged)));
        assert!(matches!(manager.with_cli_spawn(&lease, &launch, |_| Ok(())).await, Err(DgosError::ProviderConfigurationChanged)));
        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0.kind, CliProviderChangeKind::ExecutionInvalidated);
        assert_eq!(events[0].1, vec![lease.binding().clone()]);
        drop(events);
        for protocol in [OpenAiResponses, AnthropicMessages] {
            let route = crate::resolve_cli_provider_route(&after.provider_type, &after.base_url, after.config.as_ref(), protocol);
            let resolved = manager.resolve_cli_provider_launch(&after.id, "fixture-chat", "local", after.config_revision, protocol).await;
            match route {
                Ok(route) => {
                    let launch = resolved.unwrap();
                    assert_eq!(launch.metadata().base_url, route.base_url);
                    assert_eq!(launch.metadata().auth_style, route.auth_style);
                    assert_eq!(launch.credential().expose(), "synthetic-key");
                }
                Err(DgosError::CliProviderProtocolUnsupported) => assert!(matches!(resolved, Err(DgosError::CliProviderProtocolUnsupported))),
                Err(error) => panic!("unexpected route error: {error}"),
            }
        }
    }
}

#[tokio::test]
async fn equivalent_cli_route_declarations_and_unrelated_metadata_do_not_cancel_processes() {
    use serde_json::json;
    let (_dir, manager, _vault, provider, sink, _db) = setup().await;
    let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    let mut update = config(&provider.base_url, "");
    update.config = Some(json!({"display_note": "local", "cli_routes": {
        "responses": {"enabled": true, "base_url": "https://example.invalid:443/v1/", "auth_style": "bearer"},
        "messages": {"enabled": false}
    }}));
    let provider = manager.update_provider(&provider.id, update, "local").await.unwrap();
    assert!(!lease.is_cancelled());
    assert!(sink.0.lock().unwrap().iter().all(|(change, targets)| change.kind == CliProviderChangeKind::MetadataChanged && targets.is_empty()));
    let mut initial = config(&provider.base_url, "");
    initial.config = Some(json!({"cli_routes": {
        "responses": {"enabled": true},
        "messages": {"enabled": true, "base_url": "https://messages.example.invalid/v1/"}
    }}));
    let provider = manager.update_provider(&provider.id, initial, "local").await.unwrap();
    let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    sink.0.lock().unwrap().clear();
    let mut update = config(&provider.base_url, "");
    update.config = Some(json!({"max_retries": 1, "display_note": {"ignored": true}, "cli_routes": {
        "responses": {"enabled": true, "base_url": "  ", "auth_style": "bearer"},
        "messages": {"enabled": true, "base_url": "https://messages.example.invalid/", "auth_style": "both"}
    }}));
    manager.update_provider(&provider.id, update, "local").await.unwrap();
    assert!(!lease.is_cancelled());
    assert!(sink.0.lock().unwrap().iter().all(|(change, targets)| change.kind == CliProviderChangeKind::MetadataChanged && targets.is_empty()));
}

#[tokio::test]
async fn invalid_cli_routes_are_rejected_at_save_without_mutation_or_lease_revocation() {
    use serde_json::json;
    for (routes, expected_code) in [
        (json!({"responses": {"enabled": true, "auth_style": "both"}}), 1001027),
        (json!({"messages": {"enabled": true, "base_url": "http://public.example.invalid"}}), 1001028),
        (json!({"messages": {"enabled": true, "base_url": "https://example.invalid/v1/messages"}}), 1001028),
        (json!({"responses": {"enabled": true, "credential_ref": "forbidden"}}), 1001028),
    ] {
        let (_dir, manager, vault, provider, sink, _db) = setup().await;
        let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
        let cached = serde_json::to_value(manager.list_models(&provider.id, "local").await.unwrap()).unwrap();
        let mut update = config(&provider.base_url, "");
        update.config = Some(json!({"cli_routes": routes}));
        let error = manager.update_provider(&provider.id, update, "local").await.unwrap_err();
        assert_eq!(error.error_code(), expected_code);
        let after = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
        assert_eq!(after.config_revision, provider.config_revision);
        assert_eq!(after.config, provider.config);
        assert_eq!(after.credential_ref, provider.credential_ref);
        assert_eq!(vault.entry_count().await, 1);
        assert_eq!(serde_json::to_value(manager.list_models(&provider.id, "local").await.unwrap()).unwrap(), cached);
        assert!(!lease.is_cancelled());
        assert!(sink.0.lock().unwrap().is_empty());
    }
}
