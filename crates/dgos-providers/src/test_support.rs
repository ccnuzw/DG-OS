//! Explicit, isolated fixture storage. Never injected by the desktop runtime.
use crate::{new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, SecretValue};
use async_trait::async_trait;
use dgos_db::{sea_orm::DatabaseConnection, ProviderRepository};
use dgos_types::{DgosError, DgosResult, Provider, ProviderConfig};
use std::collections::HashMap;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct MemoryCredentialVault(Mutex<HashMap<String, SecretValue>>);

impl MemoryCredentialVault {
    pub async fn entry_count(&self) -> usize {
        self.0.lock().await.len()
    }
}

#[async_trait]
impl ProviderCredentialVault for MemoryCredentialVault {
    async fn store(&self, scope: &ProviderCredentialScope, secret: SecretValue) -> DgosResult<()> {
        self.0.lock().await.insert(scope.account(), secret);
        Ok(())
    }
    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue> {
        let values = self.0.lock().await;
        let value = values
            .get(&scope.account())
            .ok_or(DgosError::CredentialUnavailable)?;
        SecretValue::new(value.expose().to_owned())
    }
    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()> {
        self.0.lock().await.remove(&scope.account());
        Ok(())
    }
}

pub async fn seed_provider(
    db: DatabaseConnection,
    vault: &MemoryCredentialVault,
    mut config: ProviderConfig,
    user_id: &str,
) -> DgosResult<Provider> {
    let id = uuid::Uuid::new_v4().to_string();
    let scope = ProviderCredentialScope {
        user_id: user_id.into(),
        provider_id: id.clone(),
        endpoint: config.base_url.clone(),
        credential_ref: new_credential_ref(),
    };
    vault
        .store(
            &scope,
            SecretValue::new(std::mem::take(&mut config.api_key))?,
        )
        .await?;
    ProviderRepository::new(db)
        .create_with_credential_ref(id, config, user_id.into(), Some(scope.credential_ref))
        .await
}
