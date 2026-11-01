//! Persistent state for FR-002 Batch B install operations.
//!
//! The table stores only operation metadata and relative paths. Package
//! bodies, access tokens, command lines and absolute local paths never enter
//! this repository.

use crate::entities::skill_install_operations as operations;
use dgos_types::{DgosError, DgosResult};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

fn db_error(error: sea_orm::DbErr) -> DgosError {
    DgosError::DatabaseError(error.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillInstallOperation {
    pub operation_id: String,
    pub user_id: String,
    pub idempotency_key: String,
    pub source_kind: String,
    pub source_ref: String,
    pub state: String,
    pub skill_id: Option<String>,
    pub version_id: Option<String>,
    pub content_hash: Option<String>,
    pub staging_relpath: Option<String>,
    pub published_relpath: Option<String>,
    pub metadata: serde_json::Value,
    pub last_error_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default)]
pub struct OperationPatch {
    pub state: Option<String>,
    pub skill_id: Option<Option<String>>,
    pub version_id: Option<Option<String>>,
    pub content_hash: Option<Option<String>>,
    pub staging_relpath: Option<Option<String>>,
    pub published_relpath: Option<Option<String>>,
    pub metadata: Option<serde_json::Value>,
    pub last_error_key: Option<Option<String>>,
}

#[derive(Clone)]
pub struct SkillInstallRepository {
    db: DatabaseConnection,
}

impl SkillInstallRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn insert(&self, operation: SkillInstallOperation) -> DgosResult<()> {
        operations::ActiveModel {
            operation_id: Set(operation.operation_id),
            user_id: Set(operation.user_id),
            idempotency_key: Set(operation.idempotency_key),
            source_kind: Set(operation.source_kind),
            source_ref: Set(operation.source_ref),
            state: Set(operation.state),
            skill_id: Set(operation.skill_id),
            version_id: Set(operation.version_id),
            content_hash: Set(operation.content_hash),
            staging_relpath: Set(operation.staging_relpath),
            published_relpath: Set(operation.published_relpath),
            metadata: Set(operation.metadata),
            last_error_key: Set(operation.last_error_key),
            created_at: Set(operation.created_at),
            updated_at: Set(operation.updated_at),
        }
        .insert(&self.db)
        .await
        .map(|_| ())
        .map_err(db_error)
    }

    pub async fn find_for_user(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> DgosResult<Option<SkillInstallOperation>> {
        operations::Entity::find_by_id(operation_id)
            .filter(operations::Column::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map(|row| row.map(Into::into))
            .map_err(db_error)
    }

    pub async fn find_by_idempotency(
        &self,
        user_id: &str,
        idempotency_key: &str,
    ) -> DgosResult<Option<SkillInstallOperation>> {
        operations::Entity::find()
            .filter(operations::Column::UserId.eq(user_id))
            .filter(operations::Column::IdempotencyKey.eq(idempotency_key))
            .one(&self.db)
            .await
            .map(|row| row.map(Into::into))
            .map_err(db_error)
    }

    pub async fn update(
        &self,
        user_id: &str,
        operation_id: &str,
        patch: OperationPatch,
    ) -> DgosResult<SkillInstallOperation> {
        let row = operations::Entity::find_by_id(operation_id)
            .filter(operations::Column::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(db_error)?
            .ok_or_else(|| DgosError::DatabaseError("skill install operation not found".into()))?;
        let mut active: operations::ActiveModel = row.into();
        if let Some(value) = patch.state {
            active.state = Set(value);
        }
        if let Some(value) = patch.skill_id {
            active.skill_id = Set(value);
        }
        if let Some(value) = patch.version_id {
            active.version_id = Set(value);
        }
        if let Some(value) = patch.content_hash {
            active.content_hash = Set(value);
        }
        if let Some(value) = patch.staging_relpath {
            active.staging_relpath = Set(value);
        }
        if let Some(value) = patch.published_relpath {
            active.published_relpath = Set(value);
        }
        if let Some(value) = patch.metadata {
            active.metadata = Set(value);
        }
        if let Some(value) = patch.last_error_key {
            active.last_error_key = Set(value);
        }
        active.updated_at = Set(chrono::Utc::now().timestamp());
        active
            .update(&self.db)
            .await
            .map(Into::into)
            .map_err(db_error)
    }

    pub async fn list_active(&self, user_id: &str) -> DgosResult<Vec<SkillInstallOperation>> {
        operations::Entity::find()
            .filter(operations::Column::UserId.eq(user_id))
            .filter(operations::Column::State.is_in(["previewed", "publishing"]))
            .all(&self.db)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(db_error)
    }

    /// Startup recovery needs all in-flight rows, while normal commands must
    /// always use the user-filtered methods above.
    pub async fn list_all_active(&self) -> DgosResult<Vec<SkillInstallOperation>> {
        operations::Entity::find()
            .filter(operations::Column::State.is_in(["previewed", "publishing"]))
            .all(&self.db)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(db_error)
    }
}

impl From<operations::Model> for SkillInstallOperation {
    fn from(row: operations::Model) -> Self {
        Self {
            operation_id: row.operation_id,
            user_id: row.user_id,
            idempotency_key: row.idempotency_key,
            source_kind: row.source_kind,
            source_ref: row.source_ref,
            state: row.state,
            skill_id: row.skill_id,
            version_id: row.version_id,
            content_hash: row.content_hash,
            staging_relpath: row.staging_relpath,
            published_relpath: row.published_relpath,
            metadata: row.metadata,
            last_error_key: row.last_error_key,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
