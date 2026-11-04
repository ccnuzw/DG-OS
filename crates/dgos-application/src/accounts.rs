//! Local accounts (V1-FR-008): create, sign in, switch, sign out, change the
//! password and delete, with the session-end order of D086/D087/D088 and the
//! recoverable deletion of D087/D090.
//!
//! Identity rules:
//! - the first account is `default-user`, so existing data keeps its owner;
//! - the session lives only in host memory behind `SessionPort`; nothing
//!   here hands a token or an internal user id to the renderer;
//! - change password and delete act only on the signed-in user.
//!
//! Every operation is serialized by one async mutex (technical design
//! "并发控制"). Argon2id runs in `spawn_blocking` inside it.

mod password;

pub use password::{Password, PASSWORD_ITERATIONS, PASSWORD_MEMORY_KIB, PASSWORD_PARALLELISM};

use async_trait::async_trait;
use dgos_db::sea_orm::DatabaseConnection;
use dgos_db::{
    HostSettingsRepository, NewUser, UserDataDomain, UserDataRepository, UserRecord,
    UserRepository, LAST_SIGN_IN_USER,
};
use dgos_types::{DgosError, DgosResult, DEFAULT_USER_ID};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;

pub const MIN_PASSWORD_CHARS: usize = 8;
pub const MAX_PASSWORD_CHARS: usize = 128;
pub const MAX_USERNAME_CHARS: usize = 32;
/// Failures in a row before the back-off starts (D087).
pub const THROTTLE_AFTER_FAILURES: u32 = 5;
const THROTTLE_FIRST_SECS: i64 = 30;
const THROTTLE_MAX_SECS: i64 = 15 * 60;

/// The host's desktop session (FR-008 technical design `SessionResolver`).
/// `Switching` and `Deleting` are internal: every other command answers
/// `1008007` while the account service is ending a session.
#[derive(Clone, PartialEq, Eq)]
pub enum SessionState {
    /// No account exists: the host runs as `default-user` without a gate.
    NoAccounts,
    SignedOut,
    SignedIn(String),
    Switching,
    Deleting,
}

impl SessionState {
    /// The user commands run as, if any.
    pub fn user_id(&self) -> Option<&str> {
        match self {
            SessionState::NoAccounts => Some(DEFAULT_USER_ID),
            SessionState::SignedIn(user) => Some(user),
            _ => None,
        }
    }
}

/// Never prints the user id: session state stays out of logs (D090).
impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SessionState::NoAccounts => "NoAccounts",
            SessionState::SignedOut => "SignedOut",
            SessionState::SignedIn(_) => "SignedIn",
            SessionState::Switching => "Switching",
            SessionState::Deleting => "Deleting",
        })
    }
}

/// The host side of a session: the in-memory resolver and the user's
/// running work. Implemented by the Tauri host.
#[async_trait]
pub trait SessionPort: Send + Sync {
    fn state(&self) -> SessionState;
    fn set_state(&self, state: SessionState);
    /// Best-effort release of owner work; the session still ends when cleanup is incomplete.
    async fn release(&self, user_id: &str);
}

/// FR-010 port (D088): revoke the remote sessions issued while `user_id`
/// was signed in.
#[async_trait]
pub trait RemoteSessionRevoker: Send + Sync {
    async fn revoke_issued_by(&self, user_id: &str);
}

/// FR-010 is not implemented: there are no remote sessions to revoke.
pub struct NoRemoteSessions;

#[async_trait]
impl RemoteSessionRevoker for NoRemoteSessions {
    async fn revoke_issued_by(&self, _user_id: &str) {}
}

/// One domain's removal of a user's data. Idempotent; the deletion marker
/// makes a failed or interrupted purge resume at startup.
#[async_trait]
pub trait UserPurge: Send + Sync {
    fn domain(&self) -> &'static str;
    async fn purge(&self, user_id: &str) -> DgosResult<()>;
}

/// The SQLite purge of one `UserDataDomain`.
pub struct DomainPurge {
    repository: UserDataRepository,
    domain: UserDataDomain,
}

#[async_trait]
impl UserPurge for DomainPurge {
    fn domain(&self) -> &'static str {
        self.domain.name()
    }

    async fn purge(&self, user_id: &str) -> DgosResult<()> {
        self.repository
            .purge(self.domain, user_id)
            .await
            .map(|_| ())
    }
}

/// Every domain in `UserDataDomain::ORDER`.
pub fn default_purges(db: &DatabaseConnection) -> Vec<Arc<dyn UserPurge>> {
    UserDataDomain::ORDER
        .into_iter()
        .map(|domain| {
            Arc::new(DomainPurge {
                repository: UserDataRepository::new(db.clone()),
                domain,
            }) as Arc<dyn UserPurge>
        })
        .collect()
}

/// A credential outbox (Provider, MCP): retry the listed vault removals.
#[async_trait]
pub trait CredentialOutbox: Send + Sync {
    async fn drain(&self) -> DgosResult<()>;
}

/// The host's collaborators. `clock` returns Unix seconds; tests replace it.
pub struct AccountPorts {
    pub session: Arc<dyn SessionPort>,
    pub remote: Arc<dyn RemoteSessionRevoker>,
    pub outboxes: Vec<Arc<dyn CredentialOutbox>>,
    pub purges: Vec<Arc<dyn UserPurge>>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl AccountPorts {
    pub fn new(db: &DatabaseConnection, session: Arc<dyn SessionPort>) -> Self {
        Self {
            session,
            remote: Arc::new(NoRemoteSessions),
            outboxes: Vec::new(),
            purges: default_purges(db),
            clock: Arc::new(|| chrono::Utc::now().timestamp()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountMode {
    NoAccounts,
    SignedOut,
    SignedIn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AccountSummary {
    pub username: String,
    pub last_sign_in_at: Option<i64>,
}

/// Interface contract §9.2. No internal user id, hash or session data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AccountState {
    pub mode: AccountMode,
    pub current: Option<AccountSummary>,
    pub accounts: Vec<AccountSummary>,
    pub preselected: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeleteAccountResult {
    pub state: AccountState,
    /// Some vault items could not be removed yet; they stay in the
    /// persistent outbox and are retried (D023). Never reported as removed.
    pub credential_cleanup_pending: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAccountInput {
    pub username: String,
    pub password: Password,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignInInput {
    pub username: String,
    pub password: Password,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangePasswordInput {
    pub current_password: Password,
    pub new_password: Password,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteAccountInput {
    pub password: Password,
}

macro_rules! opaque_debug {
    ($($name:ident),*) => {$(
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "(***)"))
            }
        }
    )*};
}
opaque_debug!(
    CreateAccountInput,
    SignInInput,
    ChangePasswordInput,
    DeleteAccountInput
);

/// What startup recovery did; counts only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub deletions_resumed: usize,
    pub deletions_failed: usize,
    pub orphan_rows: u64,
}

#[derive(Clone)]
pub struct AccountService {
    inner: Arc<Inner>,
}

struct Inner {
    users: UserRepository,
    settings: HostSettingsRepository,
    data: UserDataRepository,
    ports: AccountPorts,
    lock: Mutex<()>,
}

impl AccountService {
    pub fn new(db: DatabaseConnection, ports: AccountPorts) -> Self {
        Self {
            inner: Arc::new(Inner {
                users: UserRepository::new(db.clone()),
                settings: HostSettingsRepository::new(db.clone()),
                data: UserDataRepository::new(db),
                ports,
                lock: Mutex::new(()),
            }),
        }
    }

    /// Startup, before any command is served (technical design B08-54～56):
    /// finish every interrupted deletion, retry the credential outboxes,
    /// then settle the session. After a restart a signed-in user must sign
    /// in again; the last one is only preselected (AC11).
    pub async fn recover(&self) -> DgosResult<RecoveryReport> {
        let _guard = self.inner.lock.lock().await;
        blocking(|| {
            password::dummy_hash();
        })
        .await?;
        let mut report = RecoveryReport::default();
        for user_id in self.inner.users.deleting_ids().await.map_err(storage)? {
            match self.finish_delete(&user_id).await {
                Ok(_) => report.deletions_resumed += 1,
                Err(_) => report.deletions_failed += 1,
            }
        }
        self.drain_outboxes().await;
        report.orphan_rows = self.inner.data.orphan_count().await.unwrap_or(0);
        let resting = self.resting_state().await;
        self.inner.ports.session.set_state(resting);
        tracing::info!(
            resumed = report.deletions_resumed,
            failed = report.deletions_failed,
            orphan_rows = report.orphan_rows,
            "account recovery finished"
        );
        if report.orphan_rows > 0 {
            tracing::warn!(
                orphan_rows = report.orphan_rows,
                "rows owned by no account were found; they are kept (D090)"
            );
        }
        Ok(report)
    }

    /// Every account id, including deletions in progress, or `default-user`
    /// when there is none. Used by host startup work that runs per user.
    pub async fn known_user_ids(&self) -> DgosResult<Vec<String>> {
        let ids = self.inner.users.ids().await.map_err(storage)?;
        Ok(if ids.is_empty() {
            vec![DEFAULT_USER_ID.to_owned()]
        } else {
            ids
        })
    }

    pub async fn state(&self) -> DgosResult<AccountState> {
        let _guard = self.inner.lock.lock().await;
        self.snapshot().await
    }

    /// `no_accounts`: the first account, `default-user`, signed in at once
    /// (D087). `signed_in`: an account for someone else; the current session
    /// stays. Refused while signed out (D090: the sign-in page cannot create
    /// accounts).
    pub async fn create(&self, input: CreateAccountInput) -> DgosResult<AccountState> {
        let _guard = self.inner.lock.lock().await;
        let first = match self.inner.ports.session.state() {
            SessionState::NoAccounts => true,
            SessionState::SignedIn(_) => false,
            _ => return Err(DgosError::SessionInvalid),
        };
        let username_key = username_key(&input.username)?;
        check_password(&input.password)?;
        let password = input.password;
        let hash = blocking(move || password::hash(&password))
            .await?
            .ok_or_else(|| storage(DgosError::InternalError("hash".into())))?;
        let id = if first
            && !self
                .inner
                .users
                .default_user_taken()
                .await
                .map_err(storage)?
        {
            DEFAULT_USER_ID.to_owned()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let now = self.now();
        self.inner
            .users
            .insert(NewUser {
                id: &id,
                username: &input.username,
                username_key: &username_key,
                password_hash: &hash,
                now,
            })
            .await
            .map_err(storage)?;
        if first {
            // The process already runs as `default-user`; the first account
            // only adds the gate, so there is no session to end.
            self.record_sign_in(&id, now).await?;
            self.inner
                .ports
                .session
                .set_state(SessionState::SignedIn(id.clone()));
        }
        tracing::info!(user = %short(&id), first, "account created");
        self.snapshot().await
    }

    /// Verify first; only a correct password ends the current session
    /// (contract §9.3: a failed sign-in never signs the current user out).
    pub async fn sign_in(&self, input: SignInInput) -> DgosResult<AccountState> {
        let _guard = self.inner.lock.lock().await;
        let current = match self.inner.ports.session.state() {
            SessionState::SignedOut => None,
            SessionState::SignedIn(user) => Some(user),
            _ => return Err(DgosError::SessionInvalid),
        };
        let key = simple_lowercase(&input.username);
        let Some(user) = self.inner.users.find_active_by_key(&key).await.map_err(storage)? else {
            let password = input.password;
            blocking(move || password::verify_dummy(&password)).await?;
            return Err(DgosError::SignInRejected);
        };
        let upgraded = self.verify_throttled(&user, input.password).await?;
        if current.as_deref() == Some(user.id.as_str()) { return self.snapshot().await; }
        if let Some(previous) = current.as_deref() {
            self.end_session(previous, SessionState::Switching).await;
        }
        let now = self.now();
        let signed_in = async {
            if let Some(hash) = upgraded {
                self.inner.users.set_password_hash(&user.id, &hash, now).await.map_err(storage)?;
            }
            self.record_sign_in(&user.id, now).await
        }.await;
        if let Err(error) = signed_in {
            self.inner.ports.session.set_state(self.resting_state().await);
            return Err(error);
        }
        self.inner.ports.session.set_state(SessionState::SignedIn(user.id.clone()));
        tracing::info!(user = %short(&user.id), "signed in");
        self.snapshot().await
    }

    pub async fn sign_out(&self) -> DgosResult<AccountState> {
        let _guard = self.inner.lock.lock().await;
        if let SessionState::SignedIn(user) = self.inner.ports.session.state() {
            self.end_session(&user, SessionState::Switching).await;
            let resting = self.resting_state().await;
            self.inner.ports.session.set_state(resting);
            tracing::info!(user = %short(&user), "signed out");
        }
        self.snapshot().await
    }

    pub async fn change_password(&self, input: ChangePasswordInput) -> DgosResult<AccountState> {
        let _guard = self.inner.lock.lock().await;
        let user = self.signed_in_user().await?;
        self.verify_throttled(&user, input.current_password).await?;
        check_password(&input.new_password)?;
        let new_password = input.new_password;
        let hash = blocking(move || password::hash(&new_password)).await?.ok_or_else(|| storage(DgosError::InternalError("hash".into())))?;
        self.inner.users.set_password_hash(&user.id, &hash, self.now()).await.map_err(storage)?;
        self.end_session(&user.id, SessionState::Switching).await;
        let resting = self.resting_state().await;
        self.inner.ports.session.set_state(resting);
        tracing::info!(user = %short(&user.id), "password changed");
        self.snapshot().await
    }

    /// Current user only, with the password again (D087). Marker and
    /// outboxes first, then the session ends, then each domain is purged.
    /// A failure leaves the marker: the account cannot sign in and startup
    /// recovery continues (`1008009`).
    pub async fn delete(&self, input: DeleteAccountInput) -> DgosResult<DeleteAccountResult> {
        let _guard = self.inner.lock.lock().await;
        let user = self.signed_in_user().await?;
        self.verify_throttled(&user, input.password).await?;
        self.inner
            .users
            .mark_deleting(&user.id, self.now())
            .await
            .map_err(storage)?;
        self.end_session(&user.id, SessionState::Deleting).await;
        let result = self.finish_delete(&user.id).await;
        let resting = self.resting_state().await;
        self.inner.ports.session.set_state(resting);
        let pending = result?;
        tracing::info!(user = %short(&user.id), pending, "account deleted");
        Ok(DeleteAccountResult {
            state: self.snapshot().await?,
            credential_cleanup_pending: pending,
        })
    }

    /// Shared by `delete` and `recover`. Returns whether vault removals are
    /// still pending for the user.
    async fn finish_delete(&self, user_id: &str) -> DgosResult<bool> {
        for purge in &self.inner.ports.purges {
            if let Err(error) = purge.purge(user_id).await {
                tracing::error!(
                    user = %short(user_id),
                    domain = purge.domain(),
                    code = error.error_code(),
                    "account deletion stopped; startup recovery resumes it"
                );
                return Err(DgosError::AccountDeletionIncomplete);
            }
        }
        if let Err(error) = self.inner.users.remove(user_id).await {
            tracing::error!(user = %short(user_id), code = error.error_code(), "account row removal failed");
            return Err(DgosError::AccountDeletionIncomplete);
        }
        self.drain_outboxes().await;
        // The account is gone; if the outbox cannot be read, do not claim
        // that the vault is clean.
        Ok(self
            .inner
            .data
            .pending_credential_cleanup(user_id)
            .await
            .map_or(true, |pending| pending > 0))
    }

    async fn drain_outboxes(&self) {
        for outbox in &self.inner.ports.outboxes {
            if let Err(error) = outbox.drain().await {
                tracing::warn!(code = error.error_code(), "credential outbox retry failed");
            }
        }
    }

    /// Interface contract §9.4 steps 1-4. Step 5 (the next state) is the
    /// caller's; step 6 is the page reload.
    async fn end_session(&self, user_id: &str, transition: SessionState) {
        let session = &self.inner.ports.session;
        session.set_state(transition);
        session.release(user_id).await;
        self.inner.ports.remote.revoke_issued_by(user_id).await;
    }

    /// `1008011` while backing off, even for the right password; a wrong
    /// password is counted and may start the back-off; a right one clears
    /// the count. Returns a re-hash when the parameters were upgraded.
    async fn verify_throttled(
        &self,
        user: &UserRecord,
        password: Password,
    ) -> DgosResult<Option<String>> {
        let now = self.now();
        if let Some(until) = user.locked_until.filter(|until| *until > now) {
            return Err(DgosError::SignInThrottled {
                retry_after_seconds: (until - now) as u64,
            });
        }
        let phc = user.password_hash.clone();
        let (matched, upgraded) = blocking(move || password::verify(&password, &phc)).await?;
        if !matched {
            let failed = user.failed_attempts.saturating_add(1);
            self.inner
                .users
                .record_failure(&user.id, failed, backoff_until(failed, now), now)
                .await
                .map_err(storage)?;
            return Err(DgosError::SignInRejected);
        }
        self.inner
            .users
            .clear_failures(&user.id, now)
            .await
            .map_err(storage)?;
        Ok(upgraded)
    }

    async fn signed_in_user(&self) -> DgosResult<UserRecord> {
        let SessionState::SignedIn(user_id) = self.inner.ports.session.state() else {
            return Err(DgosError::SessionInvalid);
        };
        self.inner
            .users
            .by_id(&user_id)
            .await
            .map_err(storage)?
            .filter(|user| user.deleting_since.is_none())
            .ok_or(DgosError::SessionInvalid)
    }

    async fn record_sign_in(&self, user_id: &str, now: i64) -> DgosResult<()> {
        self.inner
            .users
            .record_sign_in(user_id, now)
            .await
            .map_err(storage)?;
        self.inner
            .settings
            .set(LAST_SIGN_IN_USER, user_id, now)
            .await
            .map_err(storage)
    }

    /// Where a session ends: the sign-in page while any account row exists
    /// (a deletion in progress still blocks `default-user`), first-run
    /// otherwise. Fails closed to signed out.
    async fn resting_state(&self) -> SessionState {
        match self.inner.users.count_all().await {
            Ok(0) => SessionState::NoAccounts,
            _ => SessionState::SignedOut,
        }
    }

    async fn snapshot(&self) -> DgosResult<AccountState> {
        let session = self.inner.ports.session.state();
        let active = self.inner.users.list_active().await.map_err(storage)?;
        let total = self.inner.users.count_all().await.map_err(storage)?;
        let summary = |user: &UserRecord| AccountSummary {
            username: user.username.clone(),
            last_sign_in_at: user.last_sign_in_at,
        };
        let current = match &session {
            SessionState::SignedIn(id) => active.iter().find(|user| &user.id == id).map(summary),
            _ => None,
        };
        let mode = if total == 0 {
            AccountMode::NoAccounts
        } else if current.is_some() {
            AccountMode::SignedIn
        } else {
            AccountMode::SignedOut
        };
        let last = self
            .inner
            .settings
            .get(LAST_SIGN_IN_USER)
            .await
            .map_err(storage)?;
        let preselected = last.and_then(|id| {
            active
                .iter()
                .find(|user| user.id == id)
                .map(|user| user.username.clone())
        });
        Ok(AccountState {
            mode,
            current,
            accounts: active.iter().map(summary).collect(),
            preselected,
        })
    }

    fn now(&self) -> i64 {
        (self.inner.ports.clock)()
    }
}

/// D087: from the fifth failure in a row, 30 s doubling per further failure,
/// at most 15 minutes. `None` below the threshold.
pub fn backoff_until(failed_attempts: u32, now: i64) -> Option<i64> {
    if failed_attempts < THROTTLE_AFTER_FAILURES {
        return None;
    }
    let doublings = (failed_attempts - THROTTLE_AFTER_FAILURES).min(16);
    let seconds = (THROTTLE_FIRST_SECS << doublings).min(THROTTLE_MAX_SECS);
    Some(now + seconds)
}

/// Interface contract §9.2: 1-32 scalar values, not trimmed, no surrounding
/// whitespace, no control characters. Returns the uniqueness key.
pub fn username_key(username: &str) -> DgosResult<String> {
    let length = username.chars().count();
    let surrounded =
        username.starts_with(char::is_whitespace) || username.ends_with(char::is_whitespace);
    if length == 0
        || length > MAX_USERNAME_CHARS
        || surrounded
        || username.chars().any(char::is_control)
    {
        return Err(DgosError::UsernameRejected);
    }
    Ok(simple_lowercase(username))
}

fn check_password(password: &Password) -> DgosResult<()> {
    let length = password.chars();
    if (MIN_PASSWORD_CHARS..=MAX_PASSWORD_CHARS).contains(&length) {
        Ok(())
    } else {
        Err(DgosError::PasswordRejected)
    }
}

/// Unicode simple lowercase mapping (one scalar value to one). Rust's
/// `to_lowercase` is the full mapping; its first scalar value is the simple
/// mapping for every character whose full mapping differs (only U+0130).
fn simple_lowercase(value: &str) -> String {
    value
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> DgosResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| storage(DgosError::InternalError("password worker".into())))
}

/// Storage and worker failures reach the renderer as one fixed message; the
/// code goes to the log, never driver text that could hold bound values.
fn storage(error: DgosError) -> DgosError {
    match error {
        DgosError::DatabaseError(_) | DgosError::InternalError(_) | DgosError::JsonError(_) => {
            tracing::error!(
                code = error.error_code(),
                "account storage operation failed"
            );
            DgosError::InternalError("account storage unavailable".into())
        }
        other => other,
    }
}

/// Log label for a user: a short hash, never the id or the username.
fn short(user_id: &str) -> String {
    let digest = Sha256::digest(user_id.as_bytes());
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests;
