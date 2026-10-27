use crate::{BundledSkill, ContentSkillMetadata, Result, SkillError};
use dgos_db::entities::{
    skill_catalog as catalog, skill_user_states as states, skill_versions as versions,
};
use dgos_db::sea_orm::{
    sea_query::OnConflict, ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder, Set, TransactionTrait,
};

pub struct ContentRepository {
    db: DatabaseConnection,
}
fn db_error(error: dgos_db::sea_orm::DbErr) -> SkillError {
    SkillError::DatabaseError(error.to_string())
}
fn version_id(id: &str, version: &str) -> String {
    format!("{id}@{version}")
}

impl ContentRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    // All packages are validated before this transaction starts. A failed
    // registration never publishes a catalog pointing at a missing version.
    pub async fn register(&self, packages: &[BundledSkill]) -> Result<()> {
        let tx = self.db.begin().await.map_err(db_error)?;
        for package in packages {
            let meta = &package.metadata;
            let id = version_id(&meta.id, &meta.version);
            if let Some(existing) = versions::Entity::find_by_id(&id)
                .one(&tx)
                .await
                .map_err(db_error)?
            {
                if existing.content_hash != package.content_hash {
                    return Err(SkillError::InvalidManifest(
                        "published version content cannot change".into(),
                    ));
                }
                let mut active: versions::ActiveModel = existing.into();
                active.install_path = Set(package.install_path.clone());
                active.update(&tx).await.map_err(db_error)?;
            } else {
                versions::ActiveModel {
                    id: Set(id),
                    skill_id: Set(meta.id.clone()),
                    version: Set(meta.version.clone()),
                    content_hash: Set(package.content_hash.clone()),
                    install_path: Set(package.install_path.clone()),
                    is_bundled: Set(true),
                    verification_status: Set(meta.verification.status.clone()),
                    file_manifest: Set(serde_json::to_value(&package.files)?),
                    metadata: Set(serde_json::to_value(meta)?),
                    installed_at: Set(chrono::Utc::now().timestamp()),
                }
                .insert(&tx)
                .await
                .map_err(db_error)?;
            }
            let row = catalog::ActiveModel {
                skill_id: Set(meta.id.clone()),
                identity: Set(meta.identity.clone()),
                name: Set(meta.name.clone()),
                latest_version: Set(meta.version.clone()),
                description: Set(Some(meta.description.clone())),
                category: Set(meta.category.clone()),
                tags: Set(serde_json::to_value(&meta.tags)?),
                compatible_apps: Set(serde_json::to_value(&meta.compatible_apps)?),
                author: Set(Some(meta.author.name.clone())),
                license_spdx: Set(meta.license.spdx.clone()),
                source_kind: Set(meta.source.kind.clone()),
                source_ref: Set(meta.source.path.clone()),
                verification_status: Set(meta.verification.status.clone()),
                metadata: Set(serde_json::to_value(meta)?),
                synced_at: Set(chrono::Utc::now().timestamp()),
            };
            catalog::Entity::insert(row)
                .on_conflict(
                    OnConflict::column(catalog::Column::SkillId)
                        .update_columns([
                            catalog::Column::Identity,
                            catalog::Column::Name,
                            catalog::Column::LatestVersion,
                            catalog::Column::Description,
                            catalog::Column::Category,
                            catalog::Column::Tags,
                            catalog::Column::CompatibleApps,
                            catalog::Column::Author,
                            catalog::Column::LicenseSpdx,
                            catalog::Column::SourceKind,
                            catalog::Column::SourceRef,
                            catalog::Column::VerificationStatus,
                            catalog::Column::Metadata,
                            catalog::Column::SyncedAt,
                        ])
                        .to_owned(),
                )
                .exec(&tx)
                .await
                .map_err(db_error)?;
        }
        tx.commit().await.map_err(db_error)
    }

    pub async fn list_catalog(&self) -> Result<Vec<crate::SkillInfo>> {
        let rows = catalog::Entity::find()
            .order_by_asc(catalog::Column::Name)
            .all(&self.db)
            .await
            .map_err(db_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(crate::SkillInfo {
                    skill_id: row.skill_id,
                    name: row.name,
                    description: row.description,
                    latest_version: row.latest_version,
                    author: row.author,
                    category: row.category,
                    tags: serde_json::from_value(row.tags)?,
                    identity: row.identity,
                    source_kind: row.source_kind,
                    license_spdx: row.license_spdx,
                    compatible_apps: serde_json::from_value(row.compatible_apps)?,
                })
            })
            .collect()
    }

    pub async fn search_catalog(&self, query: &str) -> Result<Vec<crate::SkillInfo>> {
        let query = query.to_lowercase();
        Ok(self
            .list_catalog()
            .await?
            .into_iter()
            .filter(|skill| {
                skill.name.to_lowercase().contains(&query)
                    || skill
                        .description
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&query)
                    || skill
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query))
            })
            .collect())
    }

    pub async fn get_catalog_entry(&self, id: &str) -> Result<Option<catalog::Model>> {
        catalog::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(db_error)
    }

    pub async fn get_version(&self, id: &str, version: &str) -> Result<versions::Model> {
        versions::Entity::find_by_id(version_id(id, version))
            .one(&self.db)
            .await
            .map_err(db_error)?
            .ok_or_else(|| SkillError::SkillNotFound(format!("{id}@{version}")))
    }

    pub async fn add_user(&self, id: &str, user: &str) -> Result<()> {
        let entry = self
            .get_catalog_entry(id)
            .await?
            .ok_or_else(|| SkillError::SkillNotFound(id.into()))?;
        let version = self.get_version(id, &entry.latest_version).await?;
        states::Entity::insert(states::ActiveModel {
            user_id: Set(user.into()),
            skill_id: Set(id.into()),
            version_id: Set(version.id),
            enabled: Set(true),
            created_at: Set(chrono::Utc::now().timestamp()),
        })
        .on_conflict(
            OnConflict::columns([states::Column::UserId, states::Column::SkillId])
                .do_nothing()
                .to_owned(),
        )
        .do_nothing()
        .exec(&self.db)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    pub async fn user_skills(&self, user: &str) -> Result<Vec<crate::Skill>> {
        let rows = states::Entity::find()
            .filter(states::Column::UserId.eq(user))
            .all(&self.db)
            .await
            .map_err(db_error)?;
        let mut result = Vec::new();
        for row in rows {
            let version = versions::Entity::find_by_id(&row.version_id)
                .one(&self.db)
                .await
                .map_err(db_error)?
                .ok_or_else(|| SkillError::SkillNotFound(row.version_id.clone()))?;
            let meta: ContentSkillMetadata = serde_json::from_value(version.metadata)?;
            result.push(crate::Skill {
                id: row.version_id,
                skill_id: row.skill_id,
                name: meta.name.clone(),
                description: Some(meta.description.clone()),
                version: version.version,
                author: Some(meta.author.name.clone()),
                install_path: version.install_path,
                call_type: crate::CallType::Content,
                enabled: row.enabled,
                config: None,
                manifest: meta.compatibility_manifest(),
                installed_at: row.created_at,
                updated_at: row.created_at,
                user_id: user.into(),
            });
        }
        Ok(result)
    }

    pub async fn set_enabled(&self, id: &str, user: &str, enabled: bool) -> Result<()> {
        let row = states::Entity::find_by_id((user.to_string(), id.to_string()))
            .one(&self.db)
            .await
            .map_err(db_error)?
            .ok_or_else(|| SkillError::SkillNotFound(id.into()))?;
        let mut active: states::ActiveModel = row.into();
        active.enabled = Set(enabled);
        active.update(&self.db).await.map_err(db_error)?;
        Ok(())
    }
}
