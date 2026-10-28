use crate::{McpError, Result};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// No serialization, logging or IPC return path is provided for secret values.
pub struct SecretToken(Zeroizing<String>);

impl SecretToken {
    pub fn new(value: String) -> Result<Self> {
        let value = Zeroizing::new(value);
        if value.is_empty() || value.len() > 8192 || value.bytes().any(|b| b <= b' ' || b >= 127) {
            return Err(McpError::InvalidConfig(
                "Bearer token must be nonempty ASCII without whitespace, at most 8192 bytes".into(),
            ));
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
pub struct CredentialScope {
    pub user_id: String,
    pub server_id: String,
    pub endpoint: String,
    pub reference: String,
}

impl CredentialScope {
    // The entire endpoint is bound, not merely its origin; references cannot be
    // reused by another user, server or a changed URL on the same host.
    pub fn account(&self) -> String {
        let fields =
            serde_json::json!([self.user_id, self.server_id, self.endpoint, self.reference]);
        format!("mcp:{:x}", Sha256::digest(fields.to_string().as_bytes()))
    }
}

#[async_trait]
pub trait McpCredentialVault: Send + Sync {
    async fn store(&self, scope: &CredentialScope, token: SecretToken) -> Result<()>;
    async fn load(&self, scope: &CredentialScope) -> Result<SecretToken>;
    async fn remove(&self, scope: &CredentialScope) -> Result<()>;
}

pub(crate) struct UnavailableVault;

#[async_trait]
impl McpCredentialVault for UnavailableVault {
    async fn store(&self, _: &CredentialScope, _: SecretToken) -> Result<()> {
        Err(McpError::CredentialUnavailable)
    }
    async fn load(&self, _: &CredentialScope) -> Result<SecretToken> {
        Err(McpError::CredentialUnavailable)
    }
    async fn remove(&self, _: &CredentialScope) -> Result<()> {
        Err(McpError::CredentialUnavailable)
    }
}
