use crate::entity::{SkillDetail, SkillInfo, SkillRegistryCache};
use crate::error::{Result, SkillError};
use dgos_db::sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};

pub struct RegistryRepository {
    db: DatabaseConnection,
}

impl RegistryRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn upsert_registry_entry(&self, entry: &SkillRegistryCache) -> Result<()> {
        use dgos_db::entities::skill_registry_cache;

        // Check if exists
        let existing = skill_registry_cache::Entity::find()
            .filter(skill_registry_cache::Column::SkillId.eq(&entry.skill_id))
            .one(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        if let Some(model) = existing {
            // Update
            let mut active: skill_registry_cache::ActiveModel = model.into();
            active.name = Set(entry.name.clone());
            active.description = Set(entry.description.clone());
            active.latest_version = Set(entry.latest_version.clone());
            active.author = Set(entry.author.clone());
            active.category = Set(entry.category.clone());
            active.tags = Set(serde_json::to_value(&entry.tags).unwrap());
            active.download_url = Set(entry.download_url.clone());
            active.manifest = Set(serde_json::to_value(&entry.manifest).unwrap());
            active.synced_at = Set(entry.synced_at);

            active
                .update(&self.db)
                .await
                .map_err(|e| SkillError::DatabaseError(e.to_string()))?;
        } else {
            // Insert
            let model = skill_registry_cache::ActiveModel {
                id: Set(entry.id.clone()),
                skill_id: Set(entry.skill_id.clone()),
                name: Set(entry.name.clone()),
                description: Set(entry.description.clone()),
                latest_version: Set(entry.latest_version.clone()),
                author: Set(entry.author.clone()),
                category: Set(entry.category.clone()),
                tags: Set(serde_json::to_value(&entry.tags).unwrap()),
                download_url: Set(entry.download_url.clone()),
                manifest: Set(serde_json::to_value(&entry.manifest).unwrap()),
                synced_at: Set(entry.synced_at),
            };

            model
                .insert(&self.db)
                .await
                .map_err(|e| SkillError::DatabaseError(e.to_string()))?;
        }

        Ok(())
    }

    pub async fn list_registry_skills(&self, category: Option<&str>) -> Result<Vec<SkillInfo>> {
        use dgos_db::entities::skill_registry_cache;

        let mut query = skill_registry_cache::Entity::find();

        if let Some(cat) = category {
            query = query.filter(skill_registry_cache::Column::Category.eq(cat));
        }

        let models = query
            .order_by_asc(skill_registry_cache::Column::Name)
            .all(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(models
            .into_iter()
            .map(|m| SkillInfo {
                skill_id: m.skill_id,
                name: m.name,
                description: m.description,
                latest_version: m.latest_version,
                author: m.author,
                category: m.category,
                tags: serde_json::from_value(m.tags).unwrap_or_default(),
                identity: "legacy_registry".to_string(),
                source_kind: "legacy_registry".to_string(),
                license_spdx: None,
                compatible_apps: Vec::new(),
            })
            .collect())
    }

    pub async fn get_registry_skill(&self, skill_id: &str) -> Result<SkillDetail> {
        use dgos_db::entities::skill_registry_cache;

        let model = skill_registry_cache::Entity::find()
            .filter(skill_registry_cache::Column::SkillId.eq(skill_id))
            .one(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?
            .ok_or_else(|| SkillError::SkillNotFound(skill_id.to_string()))?;

        Ok(SkillDetail {
            skill_id: model.skill_id,
            name: model.name,
            description: model.description,
            latest_version: model.latest_version,
            author: model.author,
            category: model.category,
            tags: serde_json::from_value(model.tags).unwrap_or_default(),
            manifest: serde_json::from_value(model.manifest)
                .map_err(|e| SkillError::InvalidManifest(e.to_string()))?,
            download_url: model.download_url,
            identity: "legacy_registry".to_string(),
            source_kind: "legacy_registry".to_string(),
            license_spdx: None,
            compatible_apps: Vec::new(),
            format: "legacy_manifest".to_string(),
            entry: Some("manifest.json".to_string()),
            content: None,
        })
    }

    pub async fn search_skills(&self, query: &str) -> Result<Vec<SkillInfo>> {
        use dgos_db::entities::skill_registry_cache;

        let pattern = query.to_lowercase();

        let models = skill_registry_cache::Entity::find()
            .filter(
                skill_registry_cache::Column::Name
                    .contains(&pattern)
                    .or(skill_registry_cache::Column::Description.contains(&pattern)),
            )
            .order_by_asc(skill_registry_cache::Column::Name)
            .all(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(models
            .into_iter()
            .map(|m| SkillInfo {
                skill_id: m.skill_id,
                name: m.name,
                description: m.description,
                latest_version: m.latest_version,
                author: m.author,
                category: m.category,
                tags: serde_json::from_value(m.tags).unwrap_or_default(),
                identity: "legacy_registry".to_string(),
                source_kind: "legacy_registry".to_string(),
                license_spdx: None,
                compatible_apps: Vec::new(),
            })
            .collect())
    }
}
