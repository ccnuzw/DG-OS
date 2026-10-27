use dgos_types::{Capability, DgosError, DgosResult, Model};
use sea_orm::*;
use uuid::Uuid;

use crate::entities::models;

/// Model repository for managing cached model lists
pub struct ModelRepository {
    db: DatabaseConnection,
}

impl ModelRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Sync models for a provider (replace all existing models)
    pub async fn sync_models(
        &self,
        provider_id: &str,
        new_models: Vec<Model>,
    ) -> DgosResult<Vec<Model>> {
        // Delete existing models for this provider
        models::Entity::delete_many()
            .filter(models::Column::ProviderId.eq(provider_id))
            .exec(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        let now = chrono::Utc::now();

        // Insert new models
        for model in &new_models {
            let model_entity = models::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                provider_id: Set(provider_id.to_string()),
                model_id: Set(model.id.clone()),
                model_name: Set(model.name.clone()),
                capabilities: Set(serde_json::to_value(&model.capabilities)
                    .map_err(|e| DgosError::JsonError(e.to_string()))?),
                context_length: Set(model.context_length.map(|c| c as i32)),
                is_available: Set(true),
                synced_at: Set(now),
            };

            models::Entity::insert(model_entity)
                .exec(&self.db)
                .await
                .map_err(|e| DgosError::DatabaseError(e.to_string()))?;
        }

        self.list(provider_id).await
    }

    /// List all models for a provider
    pub async fn list(&self, provider_id: &str) -> DgosResult<Vec<Model>> {
        let models_data = models::Entity::find()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::IsAvailable.eq(true))
            .all(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        let mut result = Vec::new();
        for m in models_data {
            let capabilities: Vec<Capability> = serde_json::from_value(m.capabilities)
                .map_err(|e| DgosError::JsonError(e.to_string()))?;

            result.push(Model {
                id: m.model_id,
                name: m.model_name,
                capabilities,
                context_length: m.context_length.map(|c| c as usize),
            });
        }

        Ok(result)
    }

    /// Get a specific model
    pub async fn get(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> DgosResult<Option<Model>> {
        let model = models::Entity::find()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::ModelId.eq(model_id))
            .filter(models::Column::IsAvailable.eq(true))
            .one(&self.db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        if let Some(m) = model {
            let capabilities: Vec<Capability> = serde_json::from_value(m.capabilities)
                .map_err(|e| DgosError::JsonError(e.to_string()))?;

            Ok(Some(Model {
                id: m.model_id,
                name: m.model_name,
                capabilities,
                context_length: m.context_length.map(|c| c as usize),
            }))
        } else {
            Ok(None)
        }
    }
}
