use async_trait::async_trait;
use dgos_types::{DgosError, DgosResult};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Secret storage owned by the desktop host. Provider code only receives a
/// secret for the short duration needed to construct an adapter.
#[async_trait]
pub trait ProviderCredentialVault: Send + Sync {
    async fn store(&self, scope: &ProviderCredentialScope, secret: SecretValue) -> DgosResult<()>;
    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue>;
    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()>;
}

#[derive(Clone)]
pub struct ProviderCredentialScope {
    pub user_id: String,
    pub provider_id: String,
    pub endpoint: String,
    pub credential_ref: String,
}

impl ProviderCredentialScope {
    pub fn account(&self) -> String {
        let fields = serde_json::json!([
            self.user_id,
            self.provider_id,
            self.endpoint,
            self.credential_ref
        ]);
        format!(
            "provider:{:x}",
            Sha256::digest(fields.to_string().as_bytes())
        )
    }
}

/// A non-serializable, zeroizing secret wrapper.
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(value: String) -> DgosResult<Self> {
        let value = Zeroizing::new(value);
        let normalized = Zeroizing::new(value.trim().to_owned());
        if normalized.is_empty() {
            return Err(DgosError::CredentialRequired);
        }
        if normalized.len() > 8192 || normalized.bytes().any(|b| b <= b' ' || b >= 127) {
            return Err(DgosError::InvalidApiKey);
        }
        Ok(Self(normalized))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// Library construction is fail-closed until a host injects a vault.
pub struct UnavailableCredentialVault;

#[async_trait]
impl ProviderCredentialVault for UnavailableCredentialVault {
    async fn store(&self, _: &ProviderCredentialScope, _: SecretValue) -> DgosResult<()> {
        Err(DgosError::CredentialUnavailable)
    }
    async fn load(&self, _: &ProviderCredentialScope) -> DgosResult<SecretValue> {
        Err(DgosError::CredentialUnavailable)
    }
    async fn remove(&self, _: &ProviderCredentialScope) -> DgosResult<()> {
        Err(DgosError::CredentialUnavailable)
    }
}

/// Generate an opaque reference; the vault implementation determines how it
/// maps this reference to its native account/keychain entry.
pub fn new_credential_ref() -> String {
    Uuid::new_v4().to_string()
}
