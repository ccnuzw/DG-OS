use async_trait::async_trait;

/// Implemented by the trusted host, never deserialized from an IPC request.
#[async_trait]
pub trait McpCallPolicy: Send + Sync {
    async fn allows_call(&self, user_id: &str, server_id: &str, tool_name: &str) -> bool;
}

pub(crate) struct DenyCalls;

#[async_trait]
impl McpCallPolicy for DenyCalls {
    async fn allows_call(&self, _: &str, _: &str, _: &str) -> bool {
        false
    }
}
