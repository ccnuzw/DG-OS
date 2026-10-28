use dgos_types::{Capability, DgosError, DgosResult, Model};
use sea_orm::*;

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
        let tx = self.db.begin().await.map_err(db_error)?;
        Self::replace_in(&tx, provider_id, new_models).await?;
        let models = Self::list_in(&tx, provider_id).await?;
        tx.commit().await.map_err(db_error)?;
        Ok(models)
    }

    pub(crate) async fn replace_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
        new_models: Vec<Model>,
    ) -> DgosResult<()> {
        models::Entity::delete_many()
            .filter(models::Column::ProviderId.eq(provider_id))
            .exec(db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        let now = chrono::Utc::now();

        // Insert new models
        for model in &new_models {
            // Build capabilities from boolean flags
            let mut capabilities = Vec::new();
            if model.supports_streaming {
                capabilities.push(Capability::Streaming);
            }
            if model.supports_vision {
                capabilities.push(Capability::Vision);
            }

            let model_entity = models::ActiveModel {
                id: Set(model.id.clone()),
                provider_id: Set(provider_id.to_owned()),
                model_id: Set(model.model_id.clone()),
                model_name: Set(model.model_name.clone()),
                capabilities: Set(serde_json::to_value(&capabilities)
                    .map_err(|e| DgosError::JsonError(e.to_string()))?),
                context_length: Set(model.context_window),
                is_available: Set(true),
                synced_at: Set(now),
            };

            models::Entity::insert(model_entity)
                .exec(db)
                .await
                .map_err(|e| DgosError::DatabaseError(e.to_string()))?;
        }

        Ok(())
    }

    /// List all models for a provider
    pub async fn list(&self, provider_id: &str) -> DgosResult<Vec<Model>> {
        Self::list_in(&self.db, provider_id).await
    }

    pub(crate) async fn list_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
    ) -> DgosResult<Vec<Model>> {
        let models_data = models::Entity::find()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::IsAvailable.eq(true))
            .all(db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        let mut result = Vec::new();
        for m in models_data {
            let capabilities: Vec<Capability> =
                serde_json::from_value(m.capabilities).unwrap_or_default();

            result.push(Model {
                id: m.id.clone(),
                provider_id: m.provider_id.clone(),
                model_id: m.model_id.clone(),
                model_name: m.model_name.clone(),
                model_type: "chat".to_string(), // Default type
                context_window: m.context_length,
                max_output_tokens: None,
                supports_streaming: capabilities.contains(&Capability::Streaming),
                supports_vision: capabilities.contains(&Capability::Vision),
                cached_at: m.synced_at.to_rfc3339(),
            });
        }

        Ok(result)
    }

    /// Get a specific model
    pub async fn get(&self, provider_id: &str, model_id: &str) -> DgosResult<Option<Model>> {
        Self::get_in(&self.db, provider_id, model_id).await
    }

    pub(crate) async fn get_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
        model_id: &str,
    ) -> DgosResult<Option<Model>> {
        let model = models::Entity::find()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::ModelId.eq(model_id))
            .filter(models::Column::IsAvailable.eq(true))
            .one(db)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        if let Some(m) = model {
            let capabilities: Vec<Capability> =
                serde_json::from_value(m.capabilities).unwrap_or_default();

            Ok(Some(Model {
                id: m.id.clone(),
                provider_id: m.provider_id.clone(),
                model_id: m.model_id.clone(),
                model_name: m.model_name.clone(),
                model_type: "chat".to_string(),
                context_window: m.context_length,
                max_output_tokens: None,
                supports_streaming: capabilities.contains(&Capability::Streaming),
                supports_vision: capabilities.contains(&Capability::Vision),
                cached_at: m.synced_at.to_rfc3339(),
            }))
        } else {
            Ok(None)
        }
    }
}

fn db_error(error: DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}
