//! V1-FR-008 account service. Real SQLite (migrated, file-backed so
//! `secure_delete` applies as in production), real Argon2id, fake host
//! session port and clock. Proven: AC02/03/04/05/06/07/08/09/10/11/12 at the
//! service layer, B08-19～56 branches, the D086/D087 session-end order, the
//! persistent failure count and back-off, recoverable deletion and the
//! Provider vault outbox. NOT proven here: the Tauri resolver and command
//! gates (src-tauri tests), the WebView flow (frontend tests, smoke).

use super::*;
use async_trait::async_trait;
use dgos_db::migrations::Migrator;
use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_providers::test_support::{seed_provider, MemoryCredentialVault};
use dgos_providers::{
    ProviderCredentialScope, ProviderCredentialVault, ProviderManager, SecretValue,
};
use dgos_types::ProviderConfig;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct FakeSession {
    state: StdMutex<Option<SessionState>>,
    log: StdMutex<Vec<String>>,
}

impl FakeSession {
    fn log(&self) -> Vec<String> { self.log.lock().unwrap().clone() }
    fn clear(&self) { self.log.lock().unwrap().clear(); }
}

#[async_trait]
impl SessionPort for FakeSession {
    fn state(&self) -> SessionState {
        self.state.lock().unwrap().clone().unwrap_or(SessionState::Switching)
    }
    fn set_state(&self, state: SessionState) {
        let label = match &state {
            SessionState::SignedIn(user) => format!("state:signed_in:{user}"),
            other => format!("state:{other:?}"),
        };
        self.log.lock().unwrap().push(label);
        *self.state.lock().unwrap() = Some(state);
    }
    async fn release(&self, user_id: &str) {
        self.log.lock().unwrap().push(format!("release:{user_id}"));
    }
}

struct Revoker(Arc<FakeSession>);

#[async_trait]
impl RemoteSessionRevoker for Revoker {
    async fn revoke_issued_by(&self, user_id: &str) {
        self.0.log.lock().unwrap().push(format!("revoke:{user_id}"));
    }
}

/// A purge that fails while `fail` is set, otherwise does nothing.
struct Flaky(Arc<AtomicBool>);

#[async_trait]
impl UserPurge for Flaky {
    fn domain(&self) -> &'static str {
        "flaky"
    }
    async fn purge(&self, _: &str) -> DgosResult<()> {
        if self.0.load(Ordering::SeqCst) {
            Err(DgosError::DatabaseError("injected".into()))
        } else {
            Ok(())
        }
    }
}

/// A vault whose removals fail while `fail` is set.
struct SwitchVault {
    inner: MemoryCredentialVault,
    fail: AtomicBool,
}

#[async_trait]
impl ProviderCredentialVault for SwitchVault {
    async fn store(&self, scope: &ProviderCredentialScope, secret: SecretValue) -> DgosResult<()> {
        self.inner.store(scope, secret).await
    }
    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue> {
        self.inner.load(scope).await
    }
    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()> {
        if self.fail.load(Ordering::SeqCst) {
            Err(DgosError::CredentialUnavailable)
        } else {
            self.inner.remove(scope).await
        }
    }
}

struct ProviderOutbox(Arc<ProviderManager>);

#[async_trait]
impl CredentialOutbox for ProviderOutbox {
    async fn drain(&self) -> DgosResult<()> {
        self.0.drain_credential_cleanup().await.map(|_| ())
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    url: String,
    db: DatabaseConnection,
    session: Arc<FakeSession>,
    clock: Arc<AtomicI64>,
    flaky: Arc<AtomicBool>,
    vault: Arc<SwitchVault>,
    providers: Arc<ProviderManager>,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("dgos.db").display());
        let db = dgos_db::establish_connection(&url).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let vault = Arc::new(SwitchVault {
            inner: MemoryCredentialVault::default(),
            fail: AtomicBool::new(false),
        });
        let providers = Arc::new(
            ProviderManager::with_vault(&url, vault.clone())
                .await
                .unwrap(),
        );
        Self {
            _dir: dir,
            url,
            db,
            session: Arc::new(FakeSession::default()),
            clock: Arc::new(AtomicI64::new(1_000_000)),
            flaky: Arc::new(AtomicBool::new(false)),
            vault,
            providers,
        }
    }

    /// A service as the host builds it; a second call models a restart.
    async fn service(&self) -> AccountService {
        let clock = self.clock.clone();
        let mut purges = default_purges(&self.db);
        purges.insert(0, Arc::new(Flaky(self.flaky.clone())));
        let service = AccountService::new(
            self.db.clone(),
            AccountPorts {
                session: self.session.clone(),
                remote: Arc::new(Revoker(self.session.clone())),
                outboxes: vec![Arc::new(ProviderOutbox(self.providers.clone()))],
                purges,
                clock: Arc::new(move || clock.load(Ordering::SeqCst)),
            },
        );
        service.recover().await.unwrap();
        service
    }

    fn advance(&self, seconds: i64) {
        self.clock.fetch_add(seconds, Ordering::SeqCst);
    }

    async fn scalar(&self, sql: &str) -> i64 {
        self.db
            .query_one(Statement::from_string(DbBackend::Sqlite, sql))
            .await
            .unwrap()
            .unwrap()
            .try_get_by_index(0)
            .unwrap()
    }

    async fn seed(&self, user: &str) -> String {
        seed_provider(
            self.db.clone(),
            &self.vault.inner,
            ProviderConfig {
                name: format!("provider-{user}"),
                provider_type: "openai".into(),
                base_url: "https://api.example.com/v1".into(),
                api_key: "sk-test-secret-value".into(),
                enabled: true,
                config: None,
            },
            user,
        )
        .await
        .unwrap()
        .id
    }
}

fn pw(value: &str) -> Password {
    Password::new(value.into())
}

fn create(username: &str, password: &str) -> CreateAccountInput {
    CreateAccountInput {
        username: username.into(),
        password: pw(password),
    }
}

fn sign_in(username: &str, password: &str) -> SignInInput {
    SignInInput {
        username: username.into(),
        password: pw(password),
    }
}

fn user_of(session: &FakeSession) -> Option<String> {
    session.state().user_id().map(str::to_owned)
}

#[tokio::test]
async fn upgraded_database_starts_without_accounts_as_default_user() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    let state = accounts.state().await.unwrap();
    assert_eq!(state.mode, AccountMode::NoAccounts);
    assert!(state.accounts.is_empty());
    assert_eq!(user_of(&f.session).as_deref(), Some(DEFAULT_USER_ID));
}

#[tokio::test]
async fn first_account_takes_over_default_user_signs_in_and_stores_only_argon2id() {
    let f = Fixture::new().await;
    let provider = f.seed(DEFAULT_USER_ID).await;
    let accounts = f.service().await;
    let state = accounts
        .create(create("Admin", "correct horse"))
        .await
        .unwrap();

    assert_eq!(state.mode, AccountMode::SignedIn);
    assert_eq!(state.current.as_ref().unwrap().username, "Admin");
    assert_eq!(user_of(&f.session).as_deref(), Some(DEFAULT_USER_ID));
    // Existing data keeps its owner: no row was rewritten (D087).
    assert_eq!(
        f.providers.list_providers(DEFAULT_USER_ID).await.unwrap()[0].id,
        provider
    );
    let hash: String =
        f.db.query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT password_hash FROM users",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "password_hash")
        .unwrap();
    assert_ne!(hash, "correct horse");
    assert!(!hash.contains("correct horse"));
    assert!(
        hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "{hash}"
    );
}

#[tokio::test]
async fn usernames_are_unique_case_insensitively_and_validated() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("admin", "password-1"))
        .await
        .unwrap();
    for name in ["ADMIN", "Admin"] {
        assert_eq!(
            accounts
                .create(create(name, "password-2"))
                .await
                .unwrap_err()
                .error_code(),
            1008001
        );
    }
    // 1-32 scalar values, not trimmed.
    for name in ["", " admin2", "admin2 ", "ad\u{7}min", &"名".repeat(33)] {
        assert_eq!(
            accounts
                .create(create(name, "password-2"))
                .await
                .unwrap_err()
                .error_code(),
            1008003,
            "{name:?}"
        );
    }
    accounts
        .create(create(&"名".repeat(32), "password-2"))
        .await
        .unwrap();
    // 8-128 scalar values, counted as characters, not bytes.
    for password in ["short", "七个字符七个字", &"p".repeat(129)] {
        assert_eq!(
            accounts
                .create(create("bob", password))
                .await
                .unwrap_err()
                .error_code(),
            1008002
        );
    }
    accounts
        .create(create("bob", "八个字符八个字符"))
        .await
        .unwrap();
    assert_eq!(accounts.state().await.unwrap().accounts.len(), 3);
}

#[tokio::test]
async fn restart_requires_sign_in_and_only_preselects_the_last_user() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    accounts
        .create(create("bob", "bob-password"))
        .await
        .unwrap();
    f.advance(10);
    accounts
        .sign_in(sign_in("bob", "bob-password"))
        .await
        .unwrap();

    let restarted = f.service().await;
    let state = restarted.state().await.unwrap();
    assert_eq!(state.mode, AccountMode::SignedOut);
    assert_eq!(state.current, None);
    assert_eq!(state.preselected.as_deref(), Some("bob"));
    assert_eq!(state.accounts[0].username, "bob", "newest sign-in first");
    assert_eq!(user_of(&f.session), None);

    // The sign-in page cannot create accounts (D090).
    assert_eq!(
        restarted
            .create(create("mallory", "mallory-pw"))
            .await
            .unwrap_err()
            .error_code(),
        1008007
    );
    let state = restarted
        .sign_in(sign_in("BOB", "bob-password"))
        .await
        .unwrap();
    assert_eq!(state.mode, AccountMode::SignedIn);
}

#[tokio::test]
async fn wrong_and_unknown_credentials_share_one_error_and_failures_back_off() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    accounts.sign_out().await.unwrap();

    assert_eq!(
        accounts
            .sign_in(sign_in("nobody", "whatever-pw"))
            .await
            .unwrap_err()
            .error_code(),
        1008005
    );
    for _ in 0..4 {
        assert_eq!(
            accounts
                .sign_in(sign_in("alice", "wrong-password"))
                .await
                .unwrap_err()
                .error_code(),
            1008005
        );
    }
    assert_eq!(
        f.scalar("SELECT failed_attempts FROM users").await,
        4,
        "unknown names are not counted; the count is persistent"
    );
    // The fifth failure starts a 30 s back-off; even the right password waits.
    accounts
        .sign_in(sign_in("alice", "wrong-password"))
        .await
        .unwrap_err();
    let throttled = accounts
        .sign_in(sign_in("alice", "alice-password"))
        .await
        .unwrap_err();
    assert_eq!(throttled.error_code(), 1008011);
    assert_eq!(
        throttled.detail(),
        Some(serde_json::json!({ "retry_after_seconds": 30 }))
    );
    // The count survives a restart.
    let restarted = f.service().await;
    f.advance(30);
    restarted
        .sign_in(sign_in("alice", "wrong-password"))
        .await
        .unwrap_err();
    let DgosError::SignInThrottled {
        retry_after_seconds,
    } = restarted
        .sign_in(sign_in("alice", "alice-password"))
        .await
        .unwrap_err()
    else {
        panic!("expected back-off")
    };
    assert_eq!(retry_after_seconds, 60, "doubles per further failure");
    f.advance(60);
    restarted
        .sign_in(sign_in("alice", "alice-password"))
        .await
        .unwrap();
    assert_eq!(f.scalar("SELECT failed_attempts FROM users").await, 0);
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM users WHERE locked_until IS NOT NULL")
            .await,
        0
    );
}

#[test]
fn backoff_starts_at_the_fifth_failure_doubles_and_caps_at_fifteen_minutes() {
    assert_eq!(backoff_until(4, 0), None);
    let seconds: Vec<_> = (5..=12).map(|n| backoff_until(n, 0).unwrap()).collect();
    assert_eq!(seconds, [30, 60, 120, 240, 480, 900, 900, 900]);
    assert_eq!(backoff_until(u32::MAX, 0), Some(900));
}

#[tokio::test]
async fn switching_ends_the_old_session_in_order_and_a_failed_switch_keeps_it() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    accounts
        .create(create("bob", "bob-password"))
        .await
        .unwrap();
    let alice = user_of(&f.session).unwrap();
    f.session.clear();

    accounts
        .sign_in(sign_in("bob", "wrong-password"))
        .await
        .unwrap_err();
    assert!(f.session.log().is_empty(), "a failed sign-in ends nothing");
    assert_eq!(user_of(&f.session).as_deref(), Some(alice.as_str()));

    accounts
        .sign_in(sign_in("alice", "alice-password"))
        .await
        .unwrap();
    assert!(f.session.log().is_empty(), "same account: idempotent");

    accounts
        .sign_in(sign_in("bob", "bob-password"))
        .await
        .unwrap();
    let bob = user_of(&f.session).unwrap();
    assert_ne!(bob, alice);
    assert_ne!(bob, DEFAULT_USER_ID, "later accounts get their own id");
    assert_eq!(
        f.session.log(),
        [
            "state:Switching".to_owned(),
            format!("release:{alice}"),
            format!("revoke:{alice}"),
            format!("state:signed_in:{bob}"),
        ]
    );
}


#[tokio::test]
async fn data_of_one_account_is_not_visible_to_another() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    let alice = user_of(&f.session).unwrap();
    let alice_provider = f.seed(&alice).await;
    accounts
        .create(create("bob", "bob-password"))
        .await
        .unwrap();
    accounts
        .sign_in(sign_in("bob", "bob-password"))
        .await
        .unwrap();
    let bob = user_of(&f.session).unwrap();

    assert!(f.providers.list_providers(&bob).await.unwrap().is_empty());
    assert!(f
        .providers
        .delete_provider(&alice_provider, &bob)
        .await
        .is_err());
    accounts
        .sign_in(sign_in("alice", "alice-password"))
        .await
        .unwrap();
    let mine = f
        .providers
        .list_providers(&user_of(&f.session).unwrap())
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, alice_provider);
}

#[tokio::test]
async fn changing_the_password_ends_the_session_and_retires_the_old_one() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();

    assert_eq!(
        accounts
            .change_password(ChangePasswordInput {
                current_password: pw("not-the-password"),
                new_password: pw("new-password"),
            })
            .await
            .unwrap_err()
            .error_code(),
        1008005
    );
    assert_eq!(
        accounts
            .change_password(ChangePasswordInput {
                current_password: pw("alice-password"),
                new_password: pw("short"),
            })
            .await
            .unwrap_err()
            .error_code(),
        1008002
    );
    assert!(
        user_of(&f.session).is_some(),
        "rejected changes keep the session"
    );

    f.session.clear();
    let state = accounts
        .change_password(ChangePasswordInput {
            current_password: pw("alice-password"),
            new_password: pw("new-password"),
        })
        .await
        .unwrap();
    assert_eq!(state.mode, AccountMode::SignedOut);
    assert_eq!(f.session.log()[0], "state:Switching");
    assert_eq!(f.session.log().last().unwrap(), "state:SignedOut");
    accounts
        .sign_in(sign_in("alice", "alice-password"))
        .await
        .unwrap_err();
    accounts
        .sign_in(sign_in("alice", "new-password"))
        .await
        .unwrap();
}

#[tokio::test]
async fn signed_out_refuses_account_operations_and_sign_out_is_idempotent() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    // no_accounts: nothing to sign in to or change.
    assert_eq!(
        accounts
            .sign_in(sign_in("a", "password-1"))
            .await
            .unwrap_err()
            .error_code(),
        1008007
    );
    assert_eq!(
        accounts.sign_out().await.unwrap().mode,
        AccountMode::NoAccounts
    );
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    accounts.sign_out().await.unwrap();
    assert_eq!(
        accounts.sign_out().await.unwrap().mode,
        AccountMode::SignedOut
    );
    let delete = accounts
        .delete(DeleteAccountInput {
            password: pw("alice-password"),
        })
        .await;
    assert_eq!(delete.unwrap_err().error_code(), 1008007);
}

#[tokio::test]
async fn deleting_the_last_account_purges_its_data_and_returns_to_first_run() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    f.seed(DEFAULT_USER_ID).await;
    assert_eq!(f.vault.inner.entry_count().await, 1);

    assert_eq!(
        accounts
            .delete(DeleteAccountInput {
                password: pw("wrong-password"),
            })
            .await
            .unwrap_err()
            .error_code(),
        1008005
    );
    let result = accounts
        .delete(DeleteAccountInput {
            password: pw("alice-password"),
        })
        .await
        .unwrap();
    assert_eq!(result.state.mode, AccountMode::NoAccounts);
    assert!(!result.credential_cleanup_pending);
    assert_eq!(f.scalar("SELECT COUNT(*) FROM users").await, 0);
    assert_eq!(f.scalar("SELECT COUNT(*) FROM providers").await, 0);
    assert_eq!(f.scalar("SELECT COUNT(*) FROM host_settings").await, 0);
    assert_eq!(f.vault.inner.entry_count().await, 0, "vault item removed");
    assert_eq!(user_of(&f.session).as_deref(), Some(DEFAULT_USER_ID));
    // First run again: the next first account is `default-user` once more.
    accounts
        .create(create("carol", "carol-password"))
        .await
        .unwrap();
    assert_eq!(user_of(&f.session).as_deref(), Some(DEFAULT_USER_ID));
}

#[tokio::test]
async fn deleting_one_of_two_accounts_keeps_the_other_and_its_data() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    let alice = user_of(&f.session).unwrap();
    f.seed(&alice).await;
    accounts
        .create(create("bob", "bob-password"))
        .await
        .unwrap();
    accounts
        .sign_in(sign_in("bob", "bob-password"))
        .await
        .unwrap();
    let bob = user_of(&f.session).unwrap();
    f.seed(&bob).await;

    let result = accounts
        .delete(DeleteAccountInput {
            password: pw("bob-password"),
        })
        .await
        .unwrap();
    assert_eq!(result.state.mode, AccountMode::SignedOut);
    assert_eq!(result.state.preselected, None, "pointer to bob was cleared");
    assert_eq!(
        result
            .state
            .accounts
            .iter()
            .map(|account| account.username.as_str())
            .collect::<Vec<_>>(),
        ["alice"]
    );
    assert_eq!(f.providers.list_providers(&alice).await.unwrap().len(), 1);
    assert!(f.providers.list_providers(&bob).await.unwrap().is_empty());
    assert_eq!(f.vault.inner.entry_count().await, 1, "alice's key stays");
}

#[tokio::test]
async fn an_interrupted_deletion_blocks_sign_in_and_resumes_at_startup() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    accounts
        .create(create("bob", "bob-password"))
        .await
        .unwrap();
    accounts
        .sign_in(sign_in("bob", "bob-password"))
        .await
        .unwrap();
    f.seed(&user_of(&f.session).unwrap()).await;

    f.flaky.store(true, Ordering::SeqCst);
    let error = accounts
        .delete(DeleteAccountInput {
            password: pw("bob-password"),
        })
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1008009);
    assert_eq!(f.session.state(), SessionState::SignedOut);
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM users WHERE deleting_since IS NOT NULL")
            .await,
        1
    );
    let state = accounts.state().await.unwrap();
    assert_eq!(state.accounts.len(), 1, "bob is hidden while being deleted");
    assert_eq!(
        accounts
            .sign_in(sign_in("bob", "bob-password"))
            .await
            .unwrap_err()
            .error_code(),
        1008005
    );
    // The outbox was written in the marker transaction.
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM provider_credential_cleanup")
            .await,
        1
    );

    // A restart that fails again keeps the marker; the next one finishes.
    let report = f.service().await.recover().await.unwrap();
    assert_eq!(report.deletions_failed, 1);
    f.flaky.store(false, Ordering::SeqCst);
    let restarted = f.service().await;
    assert_eq!(f.scalar("SELECT COUNT(*) FROM users").await, 1);
    assert_eq!(f.scalar("SELECT COUNT(*) FROM providers").await, 0);
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM provider_credential_cleanup")
            .await,
        0
    );
    assert_eq!(f.vault.inner.entry_count().await, 0);
    assert_eq!(
        restarted.state().await.unwrap().mode,
        AccountMode::SignedOut
    );
}

#[tokio::test]
async fn a_failed_vault_removal_stays_in_the_outbox_and_is_reported_pending() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    f.seed(DEFAULT_USER_ID).await;
    f.vault.fail.store(true, Ordering::SeqCst);

    let result = accounts
        .delete(DeleteAccountInput {
            password: pw("alice-password"),
        })
        .await
        .unwrap();
    assert!(result.credential_cleanup_pending);
    assert_eq!(f.scalar("SELECT COUNT(*) FROM users").await, 0);
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM provider_credential_cleanup")
            .await,
        1
    );
    assert_eq!(
        f.vault.inner.entry_count().await,
        1,
        "not reported as removed"
    );

    // Draining again is harmless while the vault still fails.
    f.service().await;
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM provider_credential_cleanup")
            .await,
        1
    );
    f.vault.fail.store(false, Ordering::SeqCst);
    f.service().await;
    assert_eq!(
        f.scalar("SELECT COUNT(*) FROM provider_credential_cleanup")
            .await,
        0
    );
    assert_eq!(f.vault.inner.entry_count().await, 0);
    let _ = &f.url;
}

#[tokio::test]
async fn inputs_and_session_state_never_print_secrets() {
    let input = SignInInput {
        username: "alice".into(),
        password: pw("hunter2-secret"),
    };
    let printed = format!(
        "{input:?} {:?} {:?}",
        SessionState::SignedIn("user-123".into()),
        pw("hunter2-secret")
    );
    assert!(!printed.contains("hunter2"), "{printed}");
    assert!(!printed.contains("alice"), "{printed}");
    assert!(!printed.contains("user-123"), "{printed}");
    let parsed: SignInInput =
        serde_json::from_value(serde_json::json!({ "username": "a", "password": "p" })).unwrap();
    assert_eq!(parsed.password.expose(), "p");
    assert!(serde_json::from_value::<SignInInput>(
        serde_json::json!({ "username": "a", "password": "p", "user_id": "x" })
    )
    .is_err());
}

#[tokio::test]
async fn a_hash_with_older_parameters_verifies_and_is_upgraded_on_sign_in() {
    let f = Fixture::new().await;
    let accounts = f.service().await;
    accounts
        .create(create("alice", "alice-password"))
        .await
        .unwrap();
    accounts.sign_out().await.unwrap();
    let old = {
        use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
        let params = argon2::Params::new(8 * 1024, 1, 1, None).unwrap();
        argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
            .hash_password(b"alice-password", &SaltString::generate(&mut OsRng))
            .unwrap()
            .to_string()
    };
    f.db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE users SET password_hash = ?",
        [old.clone().into()],
    ))
    .await
    .unwrap();
    accounts
        .sign_in(sign_in("alice", "alice-password"))
        .await
        .unwrap();
    let hash: String =
        f.db.query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT password_hash FROM users",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "password_hash")
        .unwrap();
    assert_ne!(hash, old);
    assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
}
