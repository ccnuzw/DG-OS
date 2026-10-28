use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("MCP server not found")]
    ServerNotFound,
    #[error("MCP server name already exists")]
    DuplicateServer,
    #[error("MCP configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("MCP transport is unsupported")]
    UnsupportedTransport,
    #[error("MCP server is disabled")]
    Disabled,
    #[error("MCP server is not connected")]
    NotConnected,
    #[error("MCP connection failed: {0}")]
    ConnectionFailed(String),
    #[error("MCP handshake failed: {0}")]
    HandshakeFailed(String),
    #[error("MCP tool was not found")]
    ToolNotFound,
    #[error("MCP tool input is invalid: {0}")]
    ToolInputInvalid(String),
    #[error("MCP permission denied")]
    PermissionDenied,
    #[error("MCP confirmation required")]
    ConfirmationRequired,
    #[error("MCP call timed out")]
    Timeout,
    #[error("MCP call was cancelled")]
    Cancelled,
    #[error("MCP credential is unavailable")]
    CredentialUnavailable,
    #[error("MCP protocol error: {0}")]
    Protocol(String),
    #[error("MCP server returned an error: {0}")]
    ServerReturned(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("transport error: {0}")]
    Transport(String),
}

impl McpError {
    pub fn error_code(&self) -> u32 {
        match self {
            Self::ServerNotFound => 1003001,
            Self::DuplicateServer => 1003008,
            Self::Disabled => 1003014,
            Self::HandshakeFailed(_) => 1003010,
            Self::NotConnected => 1003002,
            Self::ConnectionFailed(_) => 1003003,
            Self::ToolNotFound => 1003004,
            Self::ServerReturned(_) => 1003005,
            Self::Timeout => 1003006,
            Self::InvalidConfig(_) => 1003007,
            Self::UnsupportedTransport => 1003009,
            Self::Protocol(_) => 1003010,
            Self::ToolInputInvalid(_) => 1003015,
            Self::Cancelled => 1003016,
            Self::CredentialUnavailable => 1003017,
            Self::Transport(_) => 1003018,
            Self::PermissionDenied => 1003014,
            Self::ConfirmationRequired => 1003019,
            Self::Database(_) | Self::Serialization(_) => 1000000,
        }
    }

    pub fn class(&self) -> &'static str {
        match self {
            Self::ServerNotFound => "mcp_server_not_found",
            Self::DuplicateServer => "duplicate_mcp_server",
            Self::Disabled => "mcp_permission_denied",
            Self::HandshakeFailed(_) => "mcp_protocol_error",
            Self::NotConnected => "mcp_not_connected",
            Self::ConnectionFailed(_) => "mcp_connection_failed",
            Self::ToolNotFound => "mcp_tool_not_found",
            Self::ServerReturned(_) => "mcp_tool_call_failed",
            Self::Timeout => "mcp_timeout",
            Self::InvalidConfig(_) => "invalid_mcp_config",
            Self::UnsupportedTransport => "mcp_connection_type_unsupported",
            Self::Protocol(_) => "mcp_protocol_error",
            Self::ToolInputInvalid(_) => "mcp_tool_input_invalid",
            Self::Cancelled => "mcp_cancelled",
            Self::CredentialUnavailable => "mcp_credential_unavailable",
            Self::Transport(_) => "mcp_transport_error",
            Self::PermissionDenied => "mcp_permission_denied",
            Self::ConfirmationRequired => "mcp_confirmation_required",
            _ => "mcp_internal_error",
        }
    }
}

pub type Result<T> = std::result::Result<T, McpError>;
