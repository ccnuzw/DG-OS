use super::*;
use async_trait::async_trait;
use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait};
use dgos_providers::test_support::MemoryCredentialVault;
use dgos_types::{DgosError, DgosResult};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

async fn database() -> DatabaseConnection {
    let db = dgos_db::sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

#[tokio::test]
async fn fetch_is_independent_of_search_key_and_access_is_request_scoped() {
    let db = database().await;
    let service = WebService::new(db, Arc::new(dgos_providers::UnavailableCredentialVault));
    let status = service.status("default-user").await.unwrap();
    assert!(!status.search_configured);
    assert!(status.fetch_available);
    let mut runtime = service.runtime("default-user", "run", WebAccess { search: false, fetch: true }).await.unwrap();
    assert_eq!(runtime.definitions().len(), 1);
    assert_eq!(runtime.definitions()[0].function.name, "dgos_web_fetch");
    assert_eq!(runtime.dispatch("dgos_web_search", json!({"query":"test"}), 8192).await.unwrap_err(), "web_tool_not_authorized");
    assert_eq!(runtime.dispatch("dgos_web_fetch", json!({"url":"https://127.0.0.1/"}), 8192).await.unwrap_err(), "web_url_not_allowed");
    let error = service.runtime("default-user", "run", WebAccess { search: true, fetch: false }).await.err().unwrap();
    assert_eq!(error, "web_search_key_required");
    assert!(serde_json::from_value::<WebAccess>(json!({"fetch":true,"owner":"foreign"})).is_err());
}

#[tokio::test]
async fn credentials_rotate_without_plaintext_and_account_deletion_stages_cleanup() {
    let db = database().await;
    // A fixture account is sufficient for mark_deleting; no password/authentication is exercised.
    db.execute_unprepared("INSERT INTO users (id,username,username_key,password_hash,created_at,updated_at) VALUES ('alice','Alice','alice','fixture-only',0,0)").await.unwrap();
    let vault = Arc::new(MemoryCredentialVault::default());
    let service = WebService::new(db.clone(), vault.clone());
    service.configure("alice", Some("web-secret-never-in-sqlite".into()), false).await.unwrap();
    let old = service.credential_ref("alice").await.unwrap().unwrap();
    assert!(ProviderRepository::new(db.clone()).credential_is_active(&old).await.unwrap());
    let row = db.query_one(sql("SELECT setting_value FROM system_settings WHERE user_id='alice' AND setting_key=?", vec![BRAVE_CREDENTIAL_SETTING.into()])).await.unwrap().unwrap();
    let value: String = row.try_get("", "setting_value").unwrap();
    assert!(!value.contains("web-secret"));
    assert_eq!(serde_json::from_str::<String>(&value).unwrap(), old);
    service.configure("alice", Some("replacement-web-secret".into()), false).await.unwrap();
    assert!(vault.load(&scope("alice", old)).await.is_err());
    assert_eq!(vault.entry_count().await, 1);
    let current = service.credential_ref("alice").await.unwrap().unwrap();
    dgos_db::UserRepository::new(db.clone()).mark_deleting("alice", 1).await.unwrap();
    let pending = ProviderRepository::new(db.clone()).pending_credential_cleanup().await.unwrap();
    assert!(pending.iter().any(|entry| entry.user_id == "alice" && entry.credential_ref == current && entry.provider_id == BRAVE_PROVIDER_ID));
    assert_eq!(service.configure("alice", Some("late-secret".into()), false).await.unwrap_err(), "web_account_changed");
    assert_eq!(service.credential_ref("alice").await.unwrap().unwrap(), current);
    assert_eq!(vault.entry_count().await, 1);
}

#[derive(Default)]
struct ControlledVault {
    inner: MemoryCredentialVault,
    block_store: AtomicBool,
    fail_remove: AtomicBool,
    loads: AtomicUsize,
    started: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
#[async_trait]
impl ProviderCredentialVault for ControlledVault {
    async fn store(&self, scope: &ProviderCredentialScope, secret: SecretValue) -> DgosResult<()> {
        if self.block_store.load(Ordering::SeqCst) {
            self.started.notify_one();
            self.resume.notified().await;
        }
        self.inner.store(scope, secret).await
    }
    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load(scope).await
    }
    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()> {
        if self.fail_remove.load(Ordering::SeqCst) { return Err(DgosError::CredentialUnavailable); }
        self.inner.remove(scope).await
    }
}

#[tokio::test]
async fn cancelled_account_cannot_publish_late_secret_and_failed_removal_remains_retryable() {
    let db = database().await;
    let vault = Arc::new(ControlledVault::default());
    let lease = CancellationToken::new();
    let service = WebService::new(db.clone(), vault.clone()).with_account_lease(lease.clone());
    vault.block_store.store(true, Ordering::SeqCst);
    vault.fail_remove.store(true, Ordering::SeqCst);
    let owner = service.clone();
    let configure = tokio::spawn(async move { owner.configure("default-user", Some("secret-to-reap".into()), false).await });
    vault.started.notified().await;
    lease.cancel();
    vault.resume.notify_one();
    assert_eq!(configure.await.unwrap().unwrap_err(), "web_account_changed");
    assert!(service.credential_ref("default-user").await.unwrap().is_none());
    let pending = ProviderRepository::new(db).pending_credential_cleanup().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].provider_id, BRAVE_PROVIDER_ID);
    assert_eq!(vault.inner.entry_count().await, 1);
}

#[tokio::test]
async fn status_does_not_probe_vault_and_key_changes_revoke_existing_runtime() {
    let db = database().await;
    let vault = Arc::new(ControlledVault::default());
    let service = WebService::new(db, vault.clone());
    service.configure("default-user", Some("configured-search-key".into()), false).await.unwrap();
    vault.loads.store(0, Ordering::SeqCst);
    assert!(service.status("default-user").await.unwrap().search_configured);
    assert_eq!(vault.loads.load(Ordering::SeqCst), 0);
    let runtime = service.runtime("default-user", "run", WebAccess { search: true, fetch: true }).await.unwrap();
    service.configure("default-user", None, true).await.unwrap();
    assert_eq!(runtime.revalidate().await.unwrap_err(), "web_configuration_changed");
    assert_eq!(vault.inner.entry_count().await, 0);
}

#[test]
fn result_budget_includes_escaped_json_and_never_invents_sources() {
    let source = WebSource { id: "host-created-source".into(), url: "https://example.org/".into(), title: "Title".into(), excerpt: "Excerpt".into(), retrieved_at: 1, kind: "web_page".into(), truncated: false };
    let result = fit_outcome(WebOutcome::new(vec![source], Some("\"你好\\".repeat(5000)), false), 1024).unwrap();
    let content = serde_json::to_string(&result).unwrap();
    assert!(serde_json::to_vec(&content).unwrap().len() <= 1024);
    assert!(result.truncated);
    assert_eq!(result.sources.len(), 1);
    assert_eq!(result.sources[0].id, "host-created-source");
    assert!(result.sources[0].truncated);
}

#[tokio::test]
async fn interrupted_activity_settles_with_same_id_and_no_claimed_sources() {
    let service = WebService::new(database().await, Arc::new(MemoryCredentialVault::default()));
    let mut runtime = service.runtime("default-user", "run", WebAccess { search: false, fetch: true }).await.unwrap();
    let events = Arc::new(std::sync::Mutex::new(Vec::<WebActivity>::new()));
    let output = events.clone();
    runtime.set_activity_sink(Arc::new(move |activity| {
        output.lock().unwrap().push(activity);
        Box::pin(async { true })
    }));
    let activity = WebActivity { id: "host-running-id".into(), operation: "fetch".into(), status: "running".into(), query: None, url: Some("https://example.org/".into()), sources: Vec::new(), error: None, truncated: false };
    runtime.pending = Some(activity.clone());
    assert!(runtime.record(activity).await);
    runtime.cancel_pending().await;
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].status, "running");
    assert_eq!(events[1].status, "failed");
    assert_eq!(events[0].id, events[1].id);
    assert_eq!(events[1].error.as_deref(), Some("web_cancelled"));
    assert!(events[1].sources.is_empty());
}
