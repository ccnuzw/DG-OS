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

pub struct SessionResolver {
    state: RwLock<SessionState>,
}

impl SessionResolver {
    /// Production start: closed until account recovery settles the state.
    pub fn starting() -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(SessionState::Switching),
        })
    }

    /// The pre-account host (`default-user`, no gate). For hosts without
    /// account storage: the WebView smoke harnesses and tests.
    #[cfg(any(test, feature = "webview-smoke"))]
    pub fn no_accounts() -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(SessionState::NoAccounts),
        })
    }

    pub fn state(&self) -> SessionState {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set(&self, state: SessionState) {
        *self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
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
        self.mcp.set_connection_scope(connection_scope(&state));
        self.resolver.set(state);
    }

    /// Interface contract §9.4 steps 2-3, in order. The resolver is already
    /// switching, so no new command can start work for the user meanwhile.
    async fn release(&self, user_id: &str) {
        self.mcp.set_connection_scope(ConnectionScope::Nobody);
        let session = HostSession::for_user(user_id);
        if let Some(modules) = self.app.try_state::<ModuleOperationManager>() {
            let failed = modules.stop_all(&session).await;
            if !failed.is_empty() {
                tracing::warn!(
                    count = failed.len(),
                    "module runtimes did not stop at session end"
                );
            }
        }
        if let Some(chat) = self.app.try_state::<dgos_workbench::ChatManager>() {
            if !chat.cancel_user(user_id).await {
                tracing::warn!("chat generations still settling at session end");
            }
        }
        if let Some(workbench) = self.app.try_state::<dgos_workbench::WorkbenchManager>() {
            if !workbench.cancel_user(user_id).await {
                tracing::warn!("single tasks still settling at session end");
            }
        }
        // Desktop streams are owned by the only window, which belongs to the
        // session that is ending.
        if let Some(streams) = self
            .app
            .try_state::<crate::provider_streams::ProviderStreams>()
        {
            streams.cancel_owner(crate::module_host::MODULE_WINDOW);
        }
        if let Some(confirmations) = self.app.try_state::<Arc<ModuleMcpConfirmations>>() {
            confirmations.cancel_user(user_id);
        }
        if self.mcp.disconnect_user(user_id).await.is_err() {
            tracing::warn!("MCP connections could not all be closed at session end");
        }
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
}
