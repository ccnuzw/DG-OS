//! The desktop session resolver (V1-FR-008, D087, D090).
//!
//! The only production source of the user a command runs as. It lives in
//! host memory: no token is handed to the renderer or written to disk, so a
//! restart starts signed out (or `no_accounts`). `host::context`,
//! `host::module_session` and `host::module_confirmation_session` resolve
//! through it; the account service (`dgos_application::AccountService`)
//! drives its transitions through `HostSessionPort`.

use dgos_application::{
    CredentialOutbox, ModuleMcpConfirmations, ModuleOperationManager, SessionPort, SessionState,
};
use dgos_mcp::{ConnectionScope, McpManager};
use dgos_types::{DgosError, DgosResult, HostSession};
use std::sync::{Arc, RwLock};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

struct ResolvedState {
    account: SessionState,
    stopping: bool,
    agent_lease: CancellationToken,
    native_generation: uuid::Uuid,
    native_contexts: Vec<(uuid::Uuid, dgos_agents::native_extensions::HostContext)>,
}

pub struct SessionResolver {
    state: RwLock<ResolvedState>,
}

impl SessionResolver {
    /// Production start: closed until account recovery settles the state.
    pub fn starting() -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(ResolvedState { account: SessionState::Switching, stopping: false, agent_lease: CancellationToken::new(), native_generation: uuid::Uuid::new_v4(), native_contexts: Vec::new() }),
        })
    }

    /// The pre-account host (`default-user`, no gate). For hosts without
    /// account storage: the WebView smoke harnesses and tests.
    #[cfg(any(test, feature = "webview-smoke"))]
    pub fn no_accounts() -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(ResolvedState { account: SessionState::NoAccounts, stopping: false, agent_lease: CancellationToken::new(), native_generation: uuid::Uuid::new_v4(), native_contexts: Vec::new() }),
        })
    }

    pub fn state(&self) -> SessionState {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .account.clone()
    }

    fn set(&self, state: SessionState) {
        let mut current = self.state.write().unwrap_or_else(|p| p.into_inner());
        if current.stopping { return; }
        current.agent_lease.cancel();
        let mut revoked = true;
        for (_, context) in &current.native_contexts { revoked &= context.revoke().is_ok(); }
        *current = ResolvedState { account: if revoked { state } else { SessionState::Switching }, stopping: false, agent_lease: CancellationToken::new(), native_generation: uuid::Uuid::new_v4(), native_contexts: Vec::new() };
    }

    /// The current session; `1008007` while signed out, switching or
    /// deleting (technical design `resolve`). Only the user id is copied out
    /// under the lock.
    pub fn current(&self) -> Result<HostSession, DgosError> {
        self.state()
            .user_id()
            .map(HostSession::for_user)
            .ok_or(DgosError::SessionInvalid)
    }

    pub fn agent_session(&self) -> Result<(HostSession, CancellationToken), DgosError> {
        let current = self.state.read().unwrap_or_else(|p| p.into_inner());
        let owner = current.account.user_id().ok_or(DgosError::SessionInvalid)?;
        Ok((HostSession::for_user(owner), current.agent_lease.clone()))
    }



    /// Snapshot only; registration below checks the same generation under the write lock.
    pub(crate) fn native_session(&self) -> Result<(String, uuid::Uuid, CancellationToken), String> {
        let current = self.state.read().map_err(|_| "session_invalid")?;
        if current.agent_lease.is_cancelled() { return Err("session_invalid".into()); }
        Ok((current.account.user_id().ok_or("session_invalid")?.to_owned(), current.native_generation, current.agent_lease.clone()))
    }

    pub(crate) fn register_native(&self, owner: &str, generation: uuid::Uuid, roots: dgos_agents::native_extensions::TrustedRoots, access: dgos_agents::native_extensions::Access) -> Result<(dgos_agents::native_extensions::NativeExtensions, dgos_agents::native_extensions::HostContext), String> {
        let context = dgos_agents::native_extensions::HostContext::from_host(owner.to_owned(), generation.to_string(), roots, access).map_err(|e| e.to_string())?;
        let api = dgos_agents::native_extensions::NativeExtensions::new(context.clone()).map_err(|e| e.to_string())?;
        let mut current = self.state.write().map_err(|_| "session_invalid")?;
        if current.account.user_id() != Some(owner) || current.native_generation != generation || current.agent_lease.is_cancelled() { return Err("session_invalid".into()); }
        current.native_contexts.retain(|(registered_generation, context)| {
            if *registered_generation == generation { true } else { let _ = context.revoke(); false }
        });
        if current.native_contexts.len() >= 4096 { return Err("native_extension_Limit".into()); }
        current.native_contexts.push((generation, context.clone()));
        Ok((api, context))
    }

    pub fn close_agents(&self) {
        let mut current = self.state.write().unwrap_or_else(|p| p.into_inner());
        current.account = SessionState::Switching;
        current.stopping = true;
        current.agent_lease.cancel();
        for (_, context) in &current.native_contexts { let _ = context.revoke(); }
        current.native_contexts.clear();
        current.native_generation = uuid::Uuid::new_v4();
    }
}

/// `SessionPort` for the account service: the resolver plus the work that
/// runs as a user. Managed state is looked up when a session ends, so the
/// port can be built before the managers are registered.
pub struct HostSessionPort {
    resolver: Arc<SessionResolver>,
    mcp: Arc<McpManager>,
    app: tauri::AppHandle,
}

impl HostSessionPort {
    pub fn new(
        resolver: Arc<SessionResolver>,
        mcp: Arc<McpManager>,
        app: tauri::AppHandle,
    ) -> Self {
        Self { resolver, mcp, app }
    }
}

/// Which users' MCP servers the background monitor may connect.
pub fn connection_scope(state: &SessionState) -> ConnectionScope {
    match state {
        SessionState::NoAccounts => ConnectionScope::AllUsers,
        SessionState::SignedIn(user) => ConnectionScope::User(user.clone()),
        _ => ConnectionScope::Nobody,
    }
}

#[async_trait::async_trait]
impl SessionPort for HostSessionPort {
    fn state(&self) -> SessionState {
        self.resolver.state()
    }

    fn set_state(&self, state: SessionState) {
        self.resolver.set(state);
        self.mcp.set_connection_scope(connection_scope(&self.resolver.state()));
    }

    async fn release(&self, user_id: &str) {
        if let Some(appearance) = self.app.try_state::<dgos_db::AppearanceRepository>() {
            appearance.invalidate_recovery(user_id);
        }
        self.mcp.set_connection_scope(ConnectionScope::Nobody);
        // Revoke the page generation before awaiting any lifecycle hook. Old module routes
        // must stay unusable even when a runtime's unload fails or a load finishes late.
        if let Some(bridge) = self.app.try_state::<crate::module_host::ModuleHostBridge>() {
            bridge.page_started(crate::module_host::MODULE_WINDOW);
        }
        if let Some(streams) = self.app.try_state::<crate::provider_streams::ProviderStreams>() {
            streams.cancel_owner(crate::module_host::MODULE_WINDOW);
        }
        if let Some(confirmations) = self.app.try_state::<Arc<ModuleMcpConfirmations>>() {
            confirmations.cancel_user(user_id);
        }
        let ai = async {
            if let Some(ai) = self.app.try_state::<dgos_application::AiService>() {
                if !ai.cancel_user(user_id).await { tracing::warn!("AI runs still settling at session end"); }
            }
        };
        let agents = async {
            if let Some(agents) = self.app.try_state::<dgos_agents::AgentManager>() {
                if !agents.cancel_user(user_id).await { tracing::warn!("Agent processes still settling at session end"); }
            }
        };
        let modules = async {
            if let Some(modules) = self.app.try_state::<ModuleOperationManager>() {
                let failed = modules.stop_all(&HostSession::for_user(user_id)).await;
                if !failed.is_empty() { tracing::warn!(count = failed.len(), "module runtimes did not stop at session end"); }
            }
        };
        let chat = async {
            if let Some(chat) = self.app.try_state::<dgos_workbench::ChatManager>() {
                if !chat.cancel_user(user_id).await { tracing::warn!("chat generations still settling at session end"); }
            }
        };
        let workbench = async {
            if let Some(workbench) = self.app.try_state::<dgos_workbench::WorkbenchManager>() {
                if !workbench.cancel_user(user_id).await { tracing::warn!("single tasks still settling at session end"); }
            }
        };
        let mcp = async {
            if self.mcp.disconnect_user(user_id).await.is_err() { tracing::warn!("MCP connections could not all be closed at session end"); }
        };
        tokio::join!(agents, modules, chat, workbench, mcp, ai);
    }
}

/// The Provider credential outbox (D023), drained by the managed manager.
pub struct ProviderOutbox(pub tauri::AppHandle);

#[async_trait::async_trait]
impl CredentialOutbox for ProviderOutbox {
    async fn drain(&self) -> DgosResult<()> {
        let providers = self
            .0
            .try_state::<dgos_providers::ProviderManager>()
            .ok_or_else(|| DgosError::InternalError("provider manager unavailable".into()))?;
        providers.drain_credential_cleanup().await.map(|_| ())
    }
}

/// The MCP credential outbox (D090).
pub struct McpOutbox(pub Arc<McpManager>);

#[async_trait::async_trait]
impl CredentialOutbox for McpOutbox {
    async fn drain(&self) -> DgosResult<()> {
        self.0
            .drain_credential_cleanup()
            .await
            .map(|_| ())
            .map_err(|_| DgosError::InternalError("MCP credential outbox unavailable".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_types::DEFAULT_USER_ID;

    #[test]
    fn only_no_accounts_and_signed_in_resolve_a_session() {
        let resolver = SessionResolver::starting();
        assert_eq!(resolver.current().unwrap_err().error_code(), 1008007);
        for (state, user) in [
            (SessionState::NoAccounts, Some(DEFAULT_USER_ID)),
            (SessionState::SignedIn("u-2".into()), Some("u-2")),
            (SessionState::SignedOut, None),
            (SessionState::Switching, None),
            (SessionState::Deleting, None),
        ] {
            resolver.set(state.clone());
            match user {
                Some(user) => assert_eq!(resolver.current().unwrap().user_id(), user),
                None => assert_eq!(
                    resolver.current().unwrap_err().error_code(),
                    1008007,
                    "{state:?}"
                ),
            }
        }
        assert_eq!(
            SessionResolver::no_accounts().current().unwrap().user_id(),
            DEFAULT_USER_ID
        );
    }

    #[test]
    fn the_mcp_monitor_follows_the_session() {
        assert_eq!(
            connection_scope(&SessionState::NoAccounts),
            ConnectionScope::AllUsers
        );
        assert_eq!(
            connection_scope(&SessionState::SignedIn("u".into())),
            ConnectionScope::User("u".into())
        );
        for state in [
            SessionState::SignedOut,
            SessionState::Switching,
            SessionState::Deleting,
        ] {
            assert_eq!(connection_scope(&state), ConnectionScope::Nobody);
        }
    }
    #[test]
    fn revoked_agent_lease_never_revives_after_switch_or_same_user_relogin() {
        let resolver = SessionResolver::no_accounts();
        let (_, old) = resolver.agent_session().unwrap();
        for state in [SessionState::Switching, SessionState::Deleting, SessionState::SignedOut] {
            resolver.set(state);
            assert!(old.is_cancelled());
            assert!(resolver.agent_session().is_err());
        }
        resolver.set(SessionState::NoAccounts);
        let (_, fresh) = resolver.agent_session().unwrap();
        assert!(!fresh.is_cancelled());
        resolver.close_agents();
        assert!(fresh.is_cancelled());
        assert_eq!(resolver.current().unwrap_err().error_code(), 1008007);
        assert!(resolver.agent_session().is_err());
        assert!(resolver.native_session().is_err());
        resolver.set(SessionState::SignedIn("late-sign-in".into()));
        assert_eq!(resolver.state(), SessionState::Switching);
        assert!(resolver.current().is_err());
    }

    #[test]
    fn native_contexts_are_revoked_and_cleared_on_account_generation_change() {
        use dgos_agents::native_extensions::{Access, TrustedRoots};
        let temp = tempfile::tempdir().unwrap();
        let resolver = SessionResolver::no_accounts();
        let (owner, generation, _) = resolver.native_session().unwrap();
        let roots_dir = temp.path().canonicalize().unwrap();
        for name in ["home", "codex", "claude", "journal"] { std::fs::create_dir(roots_dir.join(name)).unwrap(); }
        let roots = TrustedRoots {
            os_home: roots_dir.join("home"),
            codex_home: roots_dir.join("codex"),
            claude_home: roots_dir.join("claude"),
            claude_user_file: roots_dir.join("claude-user.json"),
            project: None,
            journal: roots_dir.join("journal"),
        };
        let mut missing_roots = roots.clone();
        missing_roots.os_home = roots_dir.join("missing-home");
        for _ in 0..4096 {
            assert!(resolver.register_native(&owner, generation, missing_roots.clone(), Access::Read).is_err());
        }
        assert!(resolver.state.read().unwrap().native_contexts.is_empty());
        let (_, first) = resolver.register_native(&owner, generation, roots.clone(), Access::Read).unwrap();
        let (_, second) = resolver.register_native(&owner, generation, roots, Access::Manage).unwrap();
        assert_eq!(resolver.state.read().unwrap().native_contexts.len(), 2);
        resolver.set(SessionState::SignedIn("next-user".into()));
        assert!(dgos_agents::native_extensions::NativeExtensions::new(first).is_err());
        assert!(dgos_agents::native_extensions::NativeExtensions::new(second).is_err());
        assert!(resolver.state.read().unwrap().native_contexts.is_empty());
        let (_, next_generation, _) = resolver.native_session().unwrap();
        assert_ne!(generation, next_generation);
    }
}
