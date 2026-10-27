use dgos_types::{DgosError, DgosResult, Provider, ProviderConfig};
use sea_orm::*;
use uuid::Uuid;

use crate::entities::providers;

/// Provider repository for CRUD operations
pub struct ProviderRepository {
    db: DatabaseConnection,
}

impl ProviderRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Create a new provider
    pub async fn create(&self, config: ProviderConfig, user_id: String) -> DgosResult<Provider> {
        let now = chrono::Utc::now();
        let id = Uuid::new_v4().to_string();

        // Get the highest sort_order and add 1
        let max_sort_order = providers::Entity::find()
            .filter(providers::Column::UserId.eq(&user_id))
            .order_by_desc(providers::Column::SortOrder)
            .one(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?
            .map(|p| p.sort_order)
            .unwrap_or(-1);

        let provider_model = providers::ActiveModel {
            id: Set(id.clone()),
            name: Set(config.name.clone()),
            provider_type: Set(config.provider_type.clone()),
            base_url: Set(config.base_url.clone()),
            api_key: Set(config.api_key.clone()),
            enabled: Set(config.enabled),
            sort_order: Set(max_sort_order + 1),
            config: Set(config.config.clone()),
            created_at: Set(now),
            updated_at: Set(now),
            user_id: Set(user_id.clone()),
        };

        providers::Entity::insert(provider_model)
            .exec(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        self.get(&id)
            .await?
            .ok_or_else(|| DgosError::ProviderNotFound(id))
    }

    /// Get a provider by ID
    pub async fn get(&self, id: &str) -> DgosResult<Option<Provider>> {
        let provider = providers::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        Ok(provider.map(|p| self.model_to_provider(p)))
    }

    /// List all providers for a user
    pub async fn list(&self, user_id: &str) -> DgosResult<Vec<Provider>> {
        let providers = providers::Entity::find()
            .filter(providers::Column::UserId.eq(user_id))
            .order_by_asc(providers::Column::SortOrder)
            .all(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        Ok(providers
            .into_iter()
            .map(|p| self.model_to_provider(p))
            .collect())
    }

    /// Update a provider
    pub async fn update(&self, id: &str, config: ProviderConfig) -> DgosResult<Provider> {
        let provider = providers::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?
            .ok_or_else(|| DgosError::ProviderNotFound(id.to_string()))?;

        let mut provider: providers::ActiveModel = provider.into();
        provider.name = Set(config.name);
        provider.provider_type = Set(config.provider_type);
        provider.base_url = Set(config.base_url);

        // Only update API key if a new one is provided (not empty)
        if !config.api_key.is_empty() {
            provider.api_key = Set(config.api_key);
        }

        provider.enabled = Set(config.enabled);
        provider.config = Set(config.config);
        provider.updated_at = Set(chrono::Utc::now());

        let updated = provider
            .update(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        Ok(self.model_to_provider(updated))
    }

    /// Delete a provider
    pub async fn delete(&self, id: &str) -> DgosResult<()> {
        let result = providers::Entity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(DgosError::ProviderNotFound(id.to_string()));
        }

        Ok(())
    }
    /// Update provider sort order
    pub async fn update_order(&self, provider_ids: Vec<String>) -> DgosResult<()> {
        for (index, id) in provider_ids.iter().enumerate() {
            let provider = providers::Entity::find_by_id(id)
                .one(&self.db)
                .await
                .map_err(|e| DgosError::DatabaseError(e.to_string()))?
                .ok_or_else(|| DgosError::ProviderNotFound(id.to_string()))?;

            let mut provider: providers::ActiveModel = provider.into();
            provider.sort_order = Set(index as i32);

            provider
                .update(&self.db)
                .await
                .map_err(|e| DgosError::DatabaseError(e.to_string()))?;
        }

        Ok(())
    }

    /// Toggle provider enabled status
    pub async fn toggle_enabled(&self, id: &str, enabled: bool) -> DgosResult<Provider> {
        let provider = providers::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?
            .ok_or_else(|| DgosError::ProviderNotFound(id.to_string()))?;

        let mut provider: providers::ActiveModel = provider.into();
        provider.enabled = Set(enabled);
        provider.updated_at = Set(chrono::Utc::now());

        let updated = provider
            .update(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        Ok(self.model_to_provider(updated))
    }

    fn model_to_provider(&self, model: providers::Model) -> Provider {
        Provider {
            id: model.id,
            name: model.name,
            provider_type: model.provider_type,
            base_url: model.base_url,
            api_key: model.api_key,
            enabled: model.enabled,
            sort_order: model.sort_order,
            config: model.config,
            created_at: model.created_at,
            updated_at: model.updated_at,
            user_id: model.user_id,
        }
    }
}
