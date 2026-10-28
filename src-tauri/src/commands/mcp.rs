use async_trait::async_trait;
use dgos_mcp::{
    CallMcpToolRequest, CallMcpToolResponse, CreateMcpServerRequest, CredentialScope,
    McpConnectionLog, McpCredentialVault, McpError, McpManager, McpServer, McpTool, McpUsageLog,
    SecretToken, UpdateMcpServerRequest,
};
use std::sync::Arc;
use tauri::{State, WebviewWindow};

pub struct McpState {
    pub manager: Arc<McpManager>,
}

/// OS-backed credential storage. Values never cross the renderer boundary or enter SQLite.
pub struct OsCredentialVault;

#[async_trait]
impl McpCredentialVault for OsCredentialVault {
    async fn store(&self, scope: &CredentialScope, token: SecretToken) -> dgos_mcp::Result<()> {
        let account = scope.account();
        tokio::task::spawn_blocking(move || {
            keyring::Entry::new("com.dgos.app.mcp", &account)
                .map_err(|_| McpError::CredentialUnavailable)?
                .set_password(token.expose())
                .map_err(|_| McpError::CredentialUnavailable)
        })
        .await
        .map_err(|_| McpError::CredentialUnavailable)??;
        Ok(())
    }

    async fn load(&self, scope: &CredentialScope) -> dgos_mcp::Result<SecretToken> {
        let account = scope.account();
        let value = tokio::task::spawn_blocking(move || {
            keyring::Entry::new("com.dgos.app.mcp", &account)
                .map_err(|_| McpError::CredentialUnavailable)?
                .get_password()
                .map_err(|_| McpError::CredentialUnavailable)
        })
        .await
        .map_err(|_| McpError::CredentialUnavailable)??;
        SecretToken::new(value)
    }

    async fn remove(&self, scope: &CredentialScope) -> dgos_mcp::Result<()> {
        let account = scope.account();
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new("com.dgos.app.mcp", &account)
                .map_err(|_| McpError::CredentialUnavailable)?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(McpError::CredentialUnavailable),
            }
        })
        .await
        .map_err(|_| McpError::CredentialUnavailable)??;
        Ok(())
    }
}

#[tauri::command]
pub async fn list_mcp_servers(
    window: WebviewWindow,
    state: State<'_, McpState>,
) -> Result<Vec<McpServer>, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .list_servers(LOCAL_MCP_USER)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_mcp_server_config(
    window: WebviewWindow,
    server_id: String,
    state: State<'_, McpState>,
) -> Result<CreateMcpServerRequest, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .server_config(LOCAL_MCP_USER, &server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_mcp_server(
    window: WebviewWindow,
    request: CreateMcpServerRequest,
    state: State<'_, McpState>,
) -> Result<McpServer, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .create_server(LOCAL_MCP_USER, request)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_mcp_server(
    window: WebviewWindow,
    server_id: String,
    request: UpdateMcpServerRequest,
    state: State<'_, McpState>,
) -> Result<McpServer, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .update_server(LOCAL_MCP_USER, &server_id, request)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_mcp_server(
    window: WebviewWindow,
    server_id: String,
    enabled: bool,
    state: State<'_, McpState>,
) -> Result<McpServer, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .toggle(LOCAL_MCP_USER, &server_id, enabled)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_mcp_server(
    window: WebviewWindow,
    server_id: String,
    state: State<'_, McpState>,
) -> Result<(), String> {
    require_local_main_window(&window)?;
    state
        .manager
        .delete_server(LOCAL_MCP_USER, &server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn connect_mcp_server(
    window: WebviewWindow,
    server_id: String,
    state: State<'_, McpState>,
) -> Result<McpServer, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .connect(LOCAL_MCP_USER, &server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn disconnect_mcp_server(
    window: WebviewWindow,
    server_id: String,
    state: State<'_, McpState>,
) -> Result<McpServer, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .disconnect(LOCAL_MCP_USER, &server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_mcp_tools(
    window: WebviewWindow,
    server_id: String,
    state: State<'_, McpState>,
) -> Result<Vec<McpTool>, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .list_tools(LOCAL_MCP_USER, &server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn refresh_mcp_tools(
    window: WebviewWindow,
    server_id: String,
    state: State<'_, McpState>,
) -> Result<Vec<McpTool>, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .refresh_tools(LOCAL_MCP_USER, &server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn call_mcp_tool(
    window: WebviewWindow,
    request: CallMcpToolRequest,
    state: State<'_, McpState>,
) -> Result<CallMcpToolResponse, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .call_tool(LOCAL_MCP_USER, request)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_mcp_tool(
    window: WebviewWindow,
    server_id: String,
    call_id: String,
    state: State<'_, McpState>,
) -> Result<bool, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .cancel_call(LOCAL_MCP_USER, &server_id, &call_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_mcp_usage_logs(
    window: WebviewWindow,
    server_id: Option<String>,
    limit: Option<u64>,
    state: State<'_, McpState>,
) -> Result<Vec<McpUsageLog>, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .list_usage_logs(LOCAL_MCP_USER, server_id.as_deref(), limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_mcp_connection_logs(
    window: WebviewWindow,
    server_id: Option<String>,
    limit: Option<u64>,
    state: State<'_, McpState>,
) -> Result<Vec<McpConnectionLog>, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .list_connection_logs(LOCAL_MCP_USER, server_id.as_deref(), limit)
        .await
        .map_err(|e| e.to_string())
}

// V1 currently has one local desktop identity; renderer arguments cannot choose users.
pub const LOCAL_MCP_USER: &str = "default-user";

pub(crate) fn require_local_main_window(window: &WebviewWindow) -> Result<(), String> {
    let url = window
        .url()
        .map_err(|_| "MCP permission denied".to_owned())?;
    let packaged = url.scheme() == "tauri"
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"));
    let development = cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
        && url.port() == Some(1420);
    if window.label() == "main" && (packaged || development) {
        Ok(())
    } else {
        Err("MCP permission denied".into())
    }
}

pub struct DesktopMcpPolicy;

#[async_trait::async_trait]
impl dgos_mcp::McpCallPolicy for DesktopMcpPolicy {
    async fn allows_call(&self, user_id: &str, _: &str, _: &str) -> bool {
        user_id == LOCAL_MCP_USER
    }
}

#[tauri::command]
pub async fn confirm_mcp_tool(
    window: WebviewWindow,
    request: CallMcpToolRequest,
    state: State<'_, McpState>,
) -> Result<String, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .confirm_call(LOCAL_MCP_USER, &request)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_mcp_bearer_token(
    window: WebviewWindow,
    server_id: String,
    token: String,
    state: State<'_, McpState>,
) -> Result<McpServer, String> {
    require_local_main_window(&window)?;
    let token = SecretToken::new(token).map_err(|e| e.to_string())?;
    state
        .manager
        .set_bearer_token(LOCAL_MCP_USER, &server_id, token)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_mcp_bearer_token(
    window: WebviewWindow,
    server_id: String,
    state: State<'_, McpState>,
) -> Result<McpServer, String> {
    require_local_main_window(&window)?;
    state
        .manager
        .clear_bearer_token(LOCAL_MCP_USER, &server_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "writes and deletes an isolated synthetic OS keychain item; opt-in only"]
    async fn os_keychain_roundtrip_for_isolated_synthetic_token() {
        let scope = CredentialScope {
            user_id: "synthetic-keychain-test".into(),
            server_id: format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ),
            endpoint: "https://example.invalid/mcp-test".into(),
            reference: "isolated-test-reference".into(),
        };
        let vault = OsCredentialVault;
        let stored = vault
            .store(
                &scope,
                SecretToken::new("synthetic-keychain-test-value".into()).unwrap(),
            )
            .await;
        let loaded = vault.load(&scope).await;
        let removed = vault.remove(&scope).await;
        removed.expect("isolated synthetic item must be cleaned up");
        stored.expect("OS vault must store synthetic token");
        assert!(loaded.is_ok());
        assert!(loaded.unwrap().expose() == "synthetic-keychain-test-value");
        assert!(matches!(
            vault.load(&scope).await,
            Err(McpError::CredentialUnavailable)
        ));
    }
}
