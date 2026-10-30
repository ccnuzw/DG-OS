use dgos_types::{
    CredentialStatus, DgosError, DgosResult, Model, ModelSource, Provider, ProviderConfig,
};
use sea_orm::*;
use uuid::Uuid;

use super::ModelRepository;
use crate::entities::providers;

/// Provider repository for CRUD operations
pub struct ProviderRepository {
    db: DatabaseConnection,
}

#[derive(Clone, Debug)]
pub struct CredentialCleanup {
    pub user_id: String,
    pub provider_id: String,
    pub endpoint: String,
    pub credential_ref: String,
}

pub struct ModelRefreshTicket {
    pub provider: Provider,
    refresh_revision: i64,
}

impl ProviderRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Create a new provider
    pub async fn create(&self, config: ProviderConfig, user_id: String) -> DgosResult<Provider> {
        self.create_with_credential_ref(Uuid::new_v4().to_string(), config, user_id, None)
            .await
    }

    /// Create a provider while keeping its credential outside SQLite.
    pub async fn create_with_credential_ref(
        &self,
        id: String,
        config: ProviderConfig,
        user_id: String,
        credential_ref: Option<String>,
    ) -> DgosResult<Provider> {
        self.create_with_models(id, config, user_id, credential_ref, vec![])
            .await
    }

    pub async fn create_with_models(
        &self,
        id: String,
        config: ProviderConfig,
        user_id: String,
        credential_ref: Option<String>,
        models: Vec<Model>,
    ) -> DgosResult<Provider> {
        metadata_only(&config)?;
        let tx = self.db.begin().await.map_err(db_error)?;
        let now = chrono::Utc::now();

        // Get the highest sort_order and add 1
        let max_sort_order = providers::Entity::find()
            .filter(providers::Column::UserId.eq(&user_id))
            .order_by_desc(providers::Column::SortOrder)
            .one(&tx)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?
            .map(|p| p.sort_order)
            .unwrap_or(-1);

        let provider_model = providers::ActiveModel {
            id: Set(id.clone()),
            name: Set(config.name.clone()),
            provider_type: Set(config.provider_type.clone()),
            base_url: Set(config.base_url.clone()),
            api_key: Set(String::new()),
            credential_ref: Set(credential_ref.clone()),
            config_revision: Set(0),
            model_refresh_revision: Set(0),
            enabled: Set(config.enabled),
            sort_order: Set(max_sort_order + 1),
            config: Set(config.config.clone()),
            created_at: Set(now),
            updated_at: Set(now),
            user_id: Set(user_id.clone()),
        };

        let model = provider_model.insert(&tx).await.map_err(db_error)?;
        ModelRepository::replace_in(&tx, &id, models).await?;
        if let Some(reference) = credential_ref {
            acknowledge_cleanup(&tx, &reference).await?;
        }
        tx.commit().await.map_err(db_error)?;
        Ok(self.model_to_provider(model))
    }

    /// Get a provider by ID
    pub async fn get(&self, id: &str, user_id: &str) -> DgosResult<Option<Provider>> {
        let provider = providers::Entity::find_by_id(id)
            .filter(providers::Column::UserId.eq(user_id))
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
    pub async fn update(
        &self,
        id: &str,
        config: ProviderConfig,
        user_id: &str,
    ) -> DgosResult<Provider> {
        self.update_with_credential_ref(id, config, user_id, None)
            .await
    }

    /// Update a provider and optionally replace its vault reference. `None`
    /// preserves the current reference. Secret input never reaches this layer.
    pub async fn update_with_credential_ref(
        &self,
        id: &str,
        config: ProviderConfig,
        user_id: &str,
        credential_ref: Option<String>,
    ) -> DgosResult<Provider> {
        metadata_only(&config)?;
        let tx = self.db.begin().await.map_err(db_error)?;
        let provider = providers::Entity::find_by_id(id)
            .filter(providers::Column::UserId.eq(user_id))
            .one(&tx)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?
            .ok_or_else(|| DgosError::ProviderNotFound(id.to_string()))?;

        if credential_ref.is_none()
            && (provider.base_url != config.base_url
                || provider.provider_type != config.provider_type)
        {
            return Err(DgosError::CredentialRequired);
        }
        let invalidate_models = provider.base_url != config.base_url
            || provider.provider_type != config.provider_type
            || credential_ref
                .as_ref()
                .is_some_and(|reference| provider.credential_ref.as_ref() != Some(reference));
        let revision = provider.config_revision;
        if credential_ref.is_some() {
            if let Some(old_ref) = &provider.credential_ref {
                stage_cleanup(
                    &tx,
                    &CredentialCleanup {
                        user_id: user_id.into(),
                        provider_id: id.into(),
                        endpoint: provider.base_url.clone(),
                        credential_ref: old_ref.clone(),
                    },
                )
                .await?;
            }
        }
        let mut provider: providers::ActiveModel = provider.into();
        provider.name = Set(config.name);
        provider.provider_type = Set(config.provider_type);
        provider.base_url = Set(config.base_url);

        if credential_ref.is_some() {
            provider.api_key = Set(String::new());
            provider.credential_ref = Set(credential_ref.clone());
        }

        provider.enabled = Set(config.enabled);
        provider.config = Set(config.config);
        provider.config_revision = Set(revision + 1);
        provider.updated_at = Set(chrono::Utc::now());

        let updated = provider
            .update(&tx)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        if invalidate_models {
            ModelRepository::replace_in(&tx, id, vec![]).await?;
        }

        if let Some(reference) = credential_ref {
            acknowledge_cleanup(&tx, &reference).await?;
        }
        tx.commit().await.map_err(db_error)?;
        Ok(self.model_to_provider(updated))
    }

    /// Delete a provider
    pub async fn delete(&self, id: &str, user_id: &str) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        let provider = providers::Entity::find_by_id(id)
            .filter(providers::Column::UserId.eq(user_id))
            .one(&tx)
            .await
            .map_err(db_error)?
            .ok_or_else(|| DgosError::ProviderNotFound(id.into()))?;
        if let Some(reference) = provider.credential_ref {
            stage_cleanup(
                &tx,
                &CredentialCleanup {
                    user_id: user_id.into(),
                    provider_id: id.into(),
                    endpoint: provider.base_url,
                    credential_ref: reference,
                },
            )
            .await?;
        }
        let result = providers::Entity::delete_many()
            .filter(providers::Column::Id.eq(id))
            .filter(providers::Column::UserId.eq(user_id))
            .exec(&tx)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(DgosError::ProviderNotFound(id.to_string()));
        }

        tx.commit().await.map_err(db_error)
    }
    /// All requested rows are scoped and reordered in one transaction.
    pub async fn update_order(&self, provider_ids: Vec<String>, user_id: &str) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        for (index, id) in provider_ids.iter().enumerate() {
            let result = providers::Entity::update_many()
                .col_expr(
                    providers::Column::SortOrder,
                    sea_query::Expr::value(index as i32),
                )
                .filter(providers::Column::Id.eq(id))
                .filter(providers::Column::UserId.eq(user_id))
                .exec(&tx)
                .await
                .map_err(db_error)?;
            if result.rows_affected != 1 {
                return Err(DgosError::ProviderNotFound(id.clone()));
            }
        }
        tx.commit().await.map_err(db_error)
    }

    /// Toggle provider enabled status
    pub async fn toggle_enabled(
        &self,
        id: &str,
        enabled: bool,
        user_id: &str,
    ) -> DgosResult<Provider> {
        let tx = self.db.begin().await.map_err(db_error)?;
        let provider = providers::Entity::find_by_id(id)
            .filter(providers::Column::UserId.eq(user_id))
            .one(&tx)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?
            .ok_or_else(|| DgosError::ProviderNotFound(id.to_string()))?;

        let revision = provider.config_revision;
        let mut provider: providers::ActiveModel = provider.into();
        provider.enabled = Set(enabled);
        provider.config_revision = Set(revision + 1);
        provider.updated_at = Set(chrono::Utc::now());

        let updated = provider
            .update(&tx)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        tx.commit().await.map_err(db_error)?;
        Ok(self.model_to_provider(updated))
    }

    fn model_to_provider(&self, model: providers::Model) -> Provider {
        let credential_status = if model.credential_ref.is_some() {
            CredentialStatus::Stored
        } else if !model.api_key.is_empty() {
            CredentialStatus::MigrationRequired
        } else {
            CredentialStatus::Missing
        };
        Provider {
            id: model.id,
            name: model.name,
            provider_type: model.provider_type,
            base_url: model.base_url,
            credential_ref: model.credential_ref,
            credential_status,
            config_revision: model.config_revision,
            enabled: model.enabled,
            sort_order: model.sort_order,
            config: model.config,
            created_at: model.created_at,
            updated_at: model.updated_at,
            user_id: model.user_id,
        }
    }

    /// Legacy values are available only to the explicit host migration path.
    pub async fn legacy_api_key(&self, id: &str, user_id: &str) -> DgosResult<Option<String>> {
        let row = providers::Entity::find_by_id(id)
            .filter(providers::Column::UserId.eq(user_id))
            .filter(providers::Column::CredentialRef.is_null())
            .one(&self.db)
            .await
            .map_err(db_error)?;
        Ok(row.and_then(|row| {
            if row.api_key.is_empty() {
                None
            } else {
                Some(row.api_key)
            }
        }))
    }

    pub async fn commit_credential_migration(
        &self,
        snapshot: &Provider,
        reference: &str,
    ) -> DgosResult<Provider> {
        let tx = self.db.begin().await.map_err(db_error)?;
        let result = providers::Entity::update_many()
            .col_expr(providers::Column::ApiKey, sea_query::Expr::value(""))
            .col_expr(
                providers::Column::CredentialRef,
                sea_query::Expr::value(reference),
            )
            .col_expr(
                providers::Column::UpdatedAt,
                sea_query::Expr::value(chrono::Utc::now()),
            )
            .col_expr(
                providers::Column::ConfigRevision,
                sea_query::Expr::col(providers::Column::ConfigRevision).add(1),
            )
            .filter(providers::Column::Id.eq(&snapshot.id))
            .filter(providers::Column::UserId.eq(&snapshot.user_id))
            .filter(providers::Column::BaseUrl.eq(&snapshot.base_url))
            .filter(providers::Column::UpdatedAt.eq(snapshot.updated_at))
            .filter(providers::Column::CredentialRef.is_null())
            .exec(&tx)
            .await
            .map_err(db_error)?;
        if result.rows_affected != 1 {
            return Err(DgosError::ProviderUnavailable(
                "Configuration changed; retry migration".into(),
            ));
        }
        acknowledge_cleanup(&tx, reference).await?;
        let row = providers::Entity::find_by_id(&snapshot.id)
            .one(&tx)
            .await
            .map_err(db_error)?
            .ok_or_else(|| DgosError::ProviderNotFound(snapshot.id.clone()))?;
        tx.commit().await.map_err(db_error)?;
        Ok(self.model_to_provider(row))
    }

    pub async fn stage_credential_cleanup(&self, entry: &CredentialCleanup) -> DgosResult<()> {
        stage_cleanup(&self.db, entry).await
    }

    pub async fn pending_credential_cleanup(&self) -> DgosResult<Vec<CredentialCleanup>> {
        self.db.query_all(Statement::from_string(DbBackend::Sqlite,
            "SELECT user_id, provider_id, endpoint, credential_ref FROM provider_credential_cleanup"))
            .await.map_err(db_error)?.into_iter().map(|row| Ok(CredentialCleanup {
                user_id: row.try_get("", "user_id").map_err(db_error)?,
                provider_id: row.try_get("", "provider_id").map_err(db_error)?,
                endpoint: row.try_get("", "endpoint").map_err(db_error)?,
                credential_ref: row.try_get("", "credential_ref").map_err(db_error)?,
            })).collect()
    }

    pub async fn credential_is_active(&self, reference: &str) -> DgosResult<bool> {
        Ok(providers::Entity::find()
            .filter(providers::Column::CredentialRef.eq(reference))
            .count(&self.db)
            .await
            .map_err(db_error)?
            > 0)
    }

    pub async fn acknowledge_credential_cleanup(&self, reference: &str) -> DgosResult<()> {
        acknowledge_cleanup(&self.db, reference).await
    }

    pub async fn model_snapshot(
        &self,
        id: &str,
        user_id: &str,
        model_id: &str,
    ) -> DgosResult<(Provider, Model)> {
        let tx = self.db.begin().await.map_err(db_error)?;
        let provider = providers::Entity::find_by_id(id)
            .filter(providers::Column::UserId.eq(user_id))
            .one(&tx)
            .await
            .map_err(db_error)?
            .ok_or_else(|| DgosError::ProviderNotFound(id.into()))?;
        let model = ModelRepository::get_in(&tx, id, model_id)
            .await?
            .ok_or_else(|| DgosError::ModelNotFound(model_id.into()))?;
        tx.commit().await.map_err(db_error)?;
        Ok((self.model_to_provider(provider), model))
    }

    pub async fn begin_model_refresh(
        &self,
        id: &str,
        user_id: &str,
    ) -> DgosResult<ModelRefreshTicket> {
        let tx = self.db.begin().await.map_err(db_error)?;
        let update = providers::Entity::update_many()
            .col_expr(
                providers::Column::ModelRefreshRevision,
                sea_query::Expr::col(providers::Column::ModelRefreshRevision).add(1),
            )
            .filter(providers::Column::Id.eq(id))
            .filter(providers::Column::UserId.eq(user_id))
            .exec(&tx)
            .await
            .map_err(db_error)?;
        if update.rows_affected != 1 {
            return Err(DgosError::ProviderNotFound(id.into()));
        }
        let provider = providers::Entity::find_by_id(id)
            .one(&tx)
            .await
            .map_err(db_error)?
            .ok_or_else(|| DgosError::ProviderNotFound(id.into()))?;
        let refresh_revision = provider.model_refresh_revision;
        tx.commit().await.map_err(db_error)?;
        Ok(ModelRefreshTicket {
            provider: self.model_to_provider(provider),
            refresh_revision,
        })
    }

    pub async fn publish_model_refresh(
        &self,
        ticket: &ModelRefreshTicket,
        models: Vec<Model>,
    ) -> DgosResult<Vec<Model>> {
        let tx = self.db.begin().await.map_err(db_error)?;
        // The conditional write takes SQLite's write lock before replacing any
        // cache rows, even when another host connection changed the provider.
        let update = providers::Entity::update_many()
            .col_expr(
                providers::Column::ConfigRevision,
                sea_query::Expr::col(providers::Column::ConfigRevision).add(1),
            )
            .col_expr(
                providers::Column::UpdatedAt,
                sea_query::Expr::value(chrono::Utc::now()),
            )
            .filter(providers::Column::Id.eq(&ticket.provider.id))
            .filter(providers::Column::UserId.eq(&ticket.provider.user_id))
            .filter(providers::Column::ConfigRevision.eq(ticket.provider.config_revision))
            .filter(providers::Column::ModelRefreshRevision.eq(ticket.refresh_revision))
            .exec(&tx)
            .await
            .map_err(db_error)?;
        if update.rows_affected != 1 {
            return Err(DgosError::ProviderConfigurationChanged);
        }
        ModelRepository::replace_remote_in(&tx, &ticket.provider.id, models).await?;
        let models = ModelRepository::list_in(&tx, &ticket.provider.id).await?;
        tx.commit().await.map_err(db_error)?;
        Ok(models)
    }

    /// Inserts a host-built `source = manual` model. Idempotent per `(provider_id, model_id)`:
    /// an existing row is returned untouched and the revision is not advanced. A real insert
    /// advances `config_revision` in the same transaction so an in-flight refresh is rejected.
    pub async fn add_manual_model(
        &self,
        provider_id: &str,
        user_id: &str,
        mut model: Model,
    ) -> DgosResult<Model> {
        let tx = self.db.begin().await.map_err(db_error)?;
        self.owned_row(&tx, provider_id, user_id).await?;
        if let Some(existing) =
            ModelRepository::find_row_in(&tx, provider_id, &model.model_id).await?
        {
            let existing = ModelRepository::to_model_in(&tx, existing).await?;
            tx.commit().await.map_err(db_error)?;
            return Ok(existing);
        }
        model.provider_id = provider_id.to_owned();
        model.source = ModelSource::Manual;
        ModelRepository::insert_in(
            &tx,
            provider_id,
            &model,
            ModelSource::Manual,
            chrono::Utc::now(),
        )
        .await?;
        bump_revision(&tx, provider_id, user_id).await?;
        let stored = ModelRepository::get_in(&tx, provider_id, &model.model_id)
            .await?
            .ok_or_else(|| DgosError::ModelNotFound(model.model_id.clone()))?;
        tx.commit().await.map_err(db_error)?;
        Ok(stored)
    }

    /// Deletes only `source = manual` rows. A missing model, or a remote one, is a successful
    /// no-op and does not advance the revision.
    pub async fn remove_manual_model(
        &self,
        provider_id: &str,
        user_id: &str,
        model_id: &str,
    ) -> DgosResult<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        self.owned_row(&tx, provider_id, user_id).await?;
        if ModelRepository::delete_manual_in(&tx, provider_id, model_id).await? > 0 {
            bump_revision(&tx, provider_id, user_id).await?;
        }
        tx.commit().await.map_err(db_error)
    }

    async fn owned_row(
        &self,
        db: &impl ConnectionTrait,
        provider_id: &str,
        user_id: &str,
    ) -> DgosResult<providers::Model> {
        providers::Entity::find_by_id(provider_id)
            .filter(providers::Column::UserId.eq(user_id))
            .one(db)
            .await
            .map_err(db_error)?
            .ok_or_else(|| DgosError::ProviderNotFound(provider_id.into()))
    }
}

async fn bump_revision(
    db: &impl ConnectionTrait,
    provider_id: &str,
    user_id: &str,
) -> DgosResult<()> {
    let result = providers::Entity::update_many()
        .col_expr(
            providers::Column::ConfigRevision,
            sea_query::Expr::col(providers::Column::ConfigRevision).add(1),
        )
        .col_expr(
            providers::Column::UpdatedAt,
            sea_query::Expr::value(chrono::Utc::now()),
        )
        .filter(providers::Column::Id.eq(provider_id))
        .filter(providers::Column::UserId.eq(user_id))
        .exec(db)
        .await
        .map_err(db_error)?;
    if result.rows_affected != 1 {
        return Err(DgosError::ProviderNotFound(provider_id.into()));
    }
    Ok(())
}

fn metadata_only(config: &ProviderConfig) -> DgosResult<()> {
    if !config.api_key.is_empty() {
        return Err(DgosError::InternalError(
            "Provider secrets must be stored by the host vault".into(),
        ));
    }
    Ok(())
}

async fn stage_cleanup(db: &impl ConnectionTrait, entry: &CredentialCleanup) -> DgosResult<()> {
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO provider_credential_cleanup (credential_ref, user_id, provider_id, endpoint) VALUES (?, ?, ?, ?) ON CONFLICT(credential_ref) DO NOTHING",
        [entry.credential_ref.clone().into(), entry.user_id.clone().into(), entry.provider_id.clone().into(), entry.endpoint.clone().into()]))
        .await.map_err(db_error)?;
    Ok(())
}

async fn acknowledge_cleanup(db: &impl ConnectionTrait, reference: &str) -> DgosResult<()> {
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "DELETE FROM provider_credential_cleanup WHERE credential_ref = ?",
        [reference.into()],
    ))
    .await
    .map_err(db_error)?;
    Ok(())
}

fn db_error(error: DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}
