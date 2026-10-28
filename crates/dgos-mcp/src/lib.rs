mod calls;
mod credentials;
mod error;
mod legacy_sse;
mod manager;
mod policy;
mod protocol;
mod transport;

#[cfg(test)]
mod transport_tests;

pub use credentials::{CredentialScope, McpCredentialVault, SecretToken};
pub use error::{McpError, Result};
pub use manager::{
    CallMcpToolRequest, CallMcpToolResponse, CreateMcpServerRequest, HttpConfig, McpConnectionLog,
    McpManager, McpServer, McpTool, McpUsageLog, StdioConfig, TransportKind,
    UpdateMcpServerRequest,
};
pub use policy::McpCallPolicy;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ensure_response, initialize_params, parse_tools, request};
    use serde_json::json;

    #[test]
    fn validates_transport_configs() {
        for url in [
            "https://example.com/mcp",
            "http://localhost:8080/mcp",
            "http://127.0.0.1:8080/mcp",
        ] {
            assert!(crate::transport::validate_http_url(url).is_ok());
        }
        assert!(crate::transport::validate_http_url("http://example.com/mcp").is_err());
        assert!(crate::transport::validate_http_url("http://[::1]:8080/mcp").is_ok());
        assert!(
            crate::transport::validate_http_url_with_consent("http://example.com/mcp", true)
                .is_ok()
        );
        for url in ["http://localhost.evil/mcp", "http://127.0.0.2/mcp"] {
            assert!(crate::transport::validate_http_url(url).is_err());
        }
        for url in [
            "file:///tmp/server",
            "https://name:secret@example.com/mcp",
            "http://example.com/mcp#secret",
        ] {
            assert!(crate::transport::validate_http_url_with_consent(url, true).is_err());
        }
        assert!(crate::transport::validate_executable("/bin/sh").is_err());
    }

    #[test]
    fn protocol_parses_tool_list_and_errors() {
        let response = crate::protocol::JsonRpcResponse {
            id: Some(1),
            result: Some(json!({
                "tools": [{"name": "read_file", "inputSchema": {"type": "object"}}]
            })),
            error: None,
        };
        let tools = parse_tools(ensure_response(response).unwrap()).unwrap();
        assert_eq!(tools[0].name, "read_file");
        let request = request(4, "tools/list", json!({}));
        assert_eq!(request.jsonrpc, "2.0");
        assert!(initialize_params().get("clientInfo").is_some());
    }

    #[test]
    fn error_codes_follow_d021_categories() {
        assert_eq!(McpError::PermissionDenied.error_code(), 1003014);
        assert_eq!(
            McpError::ToolInputInvalid("bad".into()).error_code(),
            1003015
        );
        assert_eq!(McpError::CredentialUnavailable.error_code(), 1003017);
        assert_eq!(McpError::PermissionDenied.class(), "mcp_permission_denied");
    }
}
