use crate::os_credentials::{self, CredentialStoreFailure};
use async_trait::async_trait;
use dgos_providers::{ProviderCredentialScope, ProviderCredentialVault, SecretValue};
use dgos_types::{DgosError, DgosResult};

pub struct OsProviderCredentialVault;

#[async_trait]
impl ProviderCredentialVault for OsProviderCredentialVault {
    async fn store(&self, scope: &ProviderCredentialScope, secret: SecretValue) -> DgosResult<()> {
        let account = scope.account();
        os_credentials::run("provider.store", move || {
            let entry =
                keyring::Entry::new("com.dgos.app.providers", &account).map_err(|error| error)?;
            entry.set_password(secret.expose())
        })
        .await
        .map_err(|failure| match failure {
            CredentialStoreFailure::Missing | CredentialStoreFailure::Unavailable => {
                DgosError::CredentialUnavailable
            }
        })
    }

    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue> {
        let account = scope.account();
        let value = os_credentials::run("provider.load", move || {
            let value = keyring::Entry::new("com.dgos.app.providers", &account)
                .map_err(|error| error)?
                .get_password()
                .map_err(|error| error)?;
            Ok(value)
        })
        .await
        .map_err(|failure| match failure {
            // A row with a stale opaque reference is recoverable by saving the
            // key again; it is not evidence that the whole OS vault is down.
            CredentialStoreFailure::Missing => DgosError::CredentialRequired,
            CredentialStoreFailure::Unavailable => DgosError::CredentialUnavailable,
        })?;
        SecretValue::new(value)
    }

    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()> {
        let account = scope.account();
        os_credentials::run("provider.remove", move || {
            let entry =
                keyring::Entry::new("com.dgos.app.providers", &account).map_err(|error| error)?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(|_| DgosError::CredentialUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "explicit opt-in: creates and removes one isolated OS credential"]
    async fn provider_os_vault_normal_lifecycle() {
        // A running desktop instance may have an interactive Keychain Access
        // prompt open for an older development signature. Do not run this
        // destructive OS integration test concurrently with the app process.
        assert!(
            std::env::var("DGOS_TEST_PROVIDER_OS_VAULT_ALLOW_CONCURRENT").as_deref() == Ok("1"),
            "run this test only after closing DG-OS and any Keychain Access prompt"
        );
        assert_eq!(
            std::env::var("DGOS_TEST_PROVIDER_OS_VAULT").as_deref(),
            Ok("1")
        );
        let scope = ProviderCredentialScope {
            user_id: "provider-vault-opt-in-test".into(),
            provider_id: dgos_providers::new_credential_ref(),
            endpoint: "https://example.invalid/provider-vault-fixture".into(),
            credential_ref: dgos_providers::new_credential_ref(),
        };
        let vault = OsProviderCredentialVault;
        let stored = vault
            .store(
                &scope,
                SecretValue::new("synthetic-provider-os-fixture".into()).unwrap(),
            )
            .await;
        let loaded = vault.load(&scope).await;
        // Attempt cleanup before asserting so even a failed read is not left
        // behind as a test credential. No formal account is enumerated or read.
        let removed = vault.remove(&scope).await;
        stored.expect("Could not store isolated fixture");
        let secret = loaded.expect("Could not read isolated fixture");
        assert!(secret.expose() == "synthetic-provider-os-fixture");
        removed.expect("Could not remove isolated fixture");
    }
}
