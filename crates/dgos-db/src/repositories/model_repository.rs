use dgos_types::{
    classify_model, DgosError, DgosResult, Hint, KindSource, Model, ModelCapabilities, ModelKind,
    ModelSource, PROTOCOL_OPENAI_COMPATIBLE,
};
use sea_orm::*;
use std::collections::HashSet;

use crate::entities::{models, providers};

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

    /// Replaces every row, manual ones included. Used by creation and by endpoint or
    /// credential changes, which invalidate the whole cache.
    pub(crate) async fn replace_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
        new_models: Vec<Model>,
    ) -> DgosResult<()> {
        models::Entity::delete_many()
            .filter(models::Column::ProviderId.eq(provider_id))
            .exec(db)
            .await
            .map_err(db_error)?;
        let now = chrono::Utc::now();
        for model in &new_models {
            Self::insert_in(db, provider_id, model, model.source, now).await?;
        }
        Ok(())
    }

    /// Refresh semantics (V1-FR-001 §2.4.4): only `source = remote` rows are replaced and a
    /// remote item with the id of a manual model is skipped.
    pub(crate) async fn replace_remote_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
        new_models: Vec<Model>,
    ) -> DgosResult<()> {
        models::Entity::delete_many()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::Source.eq(ModelSource::Remote.as_str()))
            .exec(db)
            .await
            .map_err(db_error)?;
        let manual: HashSet<String> = models::Entity::find()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::Source.eq(ModelSource::Manual.as_str()))
            .all(db)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|row| row.model_id)
            .collect();
        let now = chrono::Utc::now();
        for model in &new_models {
            if manual.contains(&model.model_id) {
                continue;
            }
            Self::insert_in(db, provider_id, model, ModelSource::Remote, now).await?;
        }
        Ok(())
    }

    pub(crate) async fn insert_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
        model: &Model,
        source: ModelSource,
        now: chrono::DateTime<chrono::Utc>,
    ) -> DgosResult<()> {
        // A kind is persisted only when the caller supplied a real one. The legacy `chat`
        // placeholder stays NULL so the reader keeps deriving it from the rules.
        let kind = ModelKind::parse(&model.model_type);
        let mut capabilities = model.capabilities;
        if model.supports_streaming && capabilities.streaming == Hint::Unknown {
            capabilities.streaming = Hint::Supported;
        }
        if model.supports_vision && capabilities.vision == Hint::Unknown {
            capabilities.vision = Hint::Supported;
        }
        models::Entity::insert(models::ActiveModel {
            id: Set(model.id.clone()),
            provider_id: Set(provider_id.to_owned()),
            model_id: Set(model.model_id.clone()),
            model_name: Set(model.model_name.clone()),
            capabilities: Set(capabilities.to_json()),
            context_length: Set(model.context_window),
            is_available: Set(true),
            synced_at: Set(now),
            source: Set(source.as_str().to_owned()),
            kind: Set(kind.map(|kind| kind.as_str().to_owned())),
            kind_source: Set(kind.map(|_| model.kind_source.as_str().to_owned())),
            max_output_tokens: Set(model.max_output_tokens),
        })
        .exec(db)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    /// Row lookup that ignores `is_available`: the unique index covers every row.
    pub(crate) async fn find_row_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
        model_id: &str,
    ) -> DgosResult<Option<models::Model>> {
        models::Entity::find()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::ModelId.eq(model_id))
            .one(db)
            .await
            .map_err(db_error)
    }

    /// Deletes a `source = manual` row; returns the number of rows removed.
    pub(crate) async fn delete_manual_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
        model_id: &str,
    ) -> DgosResult<u64> {
        Ok(models::Entity::delete_many()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::ModelId.eq(model_id))
            .filter(models::Column::Source.eq(ModelSource::Manual.as_str()))
            .exec(db)
            .await
            .map_err(db_error)?
            .rows_affected)
    }

    /// List all models for a provider
    pub async fn list(&self, provider_id: &str) -> DgosResult<Vec<Model>> {
        Self::list_in(&self.db, provider_id).await
    }

    pub(crate) async fn list_in(
        db: &impl ConnectionTrait,
        provider_id: &str,
    ) -> DgosResult<Vec<Model>> {
        let rows = models::Entity::find()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::IsAvailable.eq(true))
            .all(db)
            .await
            .map_err(db_error)?;
        let protocol = protocol_of(db, provider_id).await?;
        Ok(rows
            .into_iter()
            .map(|row| row_to_model(row, &protocol))
            .collect())
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
        let row = models::Entity::find()
            .filter(models::Column::ProviderId.eq(provider_id))
            .filter(models::Column::ModelId.eq(model_id))
            .filter(models::Column::IsAvailable.eq(true))
            .one(db)
            .await
            .map_err(db_error)?;
        match row {
            Some(row) => {
                let protocol = protocol_of(db, provider_id).await?;
                Ok(Some(row_to_model(row, &protocol)))
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn to_model_in(
        db: &impl ConnectionTrait,
        row: models::Model,
    ) -> DgosResult<Model> {
        let protocol = protocol_of(db, &row.provider_id).await?;
        Ok(row_to_model(row, &protocol))
    }
}

async fn protocol_of(db: &impl ConnectionTrait, provider_id: &str) -> DgosResult<String> {
    Ok(providers::Entity::find_by_id(provider_id)
        .one(db)
        .await
        .map_err(db_error)?
        .map(|provider| provider.provider_type)
        .unwrap_or_else(|| PROTOCOL_OPENAI_COMPATIBLE.to_owned()))
}

/// Rows with a NULL or unknown `kind` (written before the kind migration) are classified
/// from `(protocol, model_id)` here and never written back (§4.3).
fn row_to_model(row: models::Model, protocol: &str) -> Model {
    let (kind, kind_source) = match row.kind.as_deref().and_then(ModelKind::parse) {
        Some(kind) => (
            kind,
            row.kind_source
                .as_deref()
                .and_then(KindSource::parse)
                .unwrap_or(KindSource::Fallback),
        ),
        None => classify_model(protocol, &row.model_id),
    };
    let capabilities = ModelCapabilities::from_json(&row.capabilities);
    Model {
        id: row.id,
        provider_id: row.provider_id,
        model_id: row.model_id,
        model_name: row.model_name,
        model_type: kind.as_str().to_owned(),
        context_window: row.context_length,
        max_output_tokens: row.max_output_tokens,
        supports_streaming: capabilities.streaming == Hint::Supported,
        supports_vision: capabilities.vision == Hint::Supported,
        cached_at: row.synced_at.to_rfc3339(),
        kind_source,
        source: ModelSource::parse(&row.source).unwrap_or_default(),
        capabilities,
    }
}

fn db_error(error: DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}
