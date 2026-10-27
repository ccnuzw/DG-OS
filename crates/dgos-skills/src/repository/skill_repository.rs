use crate::entity::{Skill, SkillUsageLog};
use crate::error::{Result, SkillError};
use dgos_db::sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set};

pub struct SkillRepository {
    db: DatabaseConnection,
}

impl SkillRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn insert_skill(&self, skill: &Skill) -> Result<()> {
        use dgos_db::entities::skills;

        let model = skills::ActiveModel {
            id: Set(skill.id.clone()),
            skill_id: Set(skill.skill_id.clone()),
            name: Set(skill.name.clone()),
            description: Set(skill.description.clone()),
            version: Set(skill.version.clone()),
            author: Set(skill.author.clone()),
            install_path: Set(skill.install_path.clone()),
            call_type: Set(skill.call_type.as_str().to_string()),
            enabled: Set(skill.enabled),
            config: Set(skill.config.clone()),
            manifest: Set(serde_json::to_value(&skill.manifest).unwrap()),
            installed_at: Set(skill.installed_at),
            updated_at: Set(skill.updated_at),
            user_id: Set(skill.user_id.clone()),
        };

        model
            .insert(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub async fn get_skill_by_skill_id(&self, skill_id: &str, user_id: &str) -> Result<Skill> {
        use dgos_db::entities::skills;

        let model = skills::Entity::find()
            .filter(skills::Column::SkillId.eq(skill_id))
            .filter(skills::Column::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?
            .ok_or_else(|| SkillError::SkillNotFound(skill_id.to_string()))?;

        Ok(Self::model_to_skill(model)?)
    }

    pub async fn list_skills(&self, user_id: &str, enabled_only: bool) -> Result<Vec<Skill>> {
        use dgos_db::entities::skills;

        let mut query = skills::Entity::find().filter(skills::Column::UserId.eq(user_id));

        if enabled_only {
            query = query.filter(skills::Column::Enabled.eq(true));
        }

        let models = query
            .order_by_asc(skills::Column::Name)
            .all(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        models.into_iter().map(|m| Self::model_to_skill(m)).collect()
    }

    pub async fn update_skill_config(
        &self,
        skill_id: &str,
        user_id: &str,
        config: serde_json::Value,
    ) -> Result<()> {
        use dgos_db::entities::skills;

        let model = skills::Entity::find()
            .filter(skills::Column::SkillId.eq(skill_id))
            .filter(skills::Column::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?
            .ok_or_else(|| SkillError::SkillNotFound(skill_id.to_string()))?;

        let mut active: skills::ActiveModel = model.into();
        active.config = Set(Some(config));
        active.updated_at = Set(chrono::Utc::now().timestamp());

        active
            .update(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub async fn toggle_skill(&self, skill_id: &str, user_id: &str, enabled: bool) -> Result<()> {
        use dgos_db::entities::skills;

        let model = skills::Entity::find()
            .filter(skills::Column::SkillId.eq(skill_id))
            .filter(skills::Column::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?
            .ok_or_else(|| SkillError::SkillNotFound(skill_id.to_string()))?;

        let mut active: skills::ActiveModel = model.into();
        active.enabled = Set(enabled);
        active.updated_at = Set(chrono::Utc::now().timestamp());

        active
            .update(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub async fn update_skill_version(
        &self,
        skill_id: &str,
        user_id: &str,
        version: &str,
    ) -> Result<()> {
        use dgos_db::entities::skills;

        let model = skills::Entity::find()
            .filter(skills::Column::SkillId.eq(skill_id))
            .filter(skills::Column::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?
            .ok_or_else(|| SkillError::SkillNotFound(skill_id.to_string()))?;

        let mut active: skills::ActiveModel = model.into();
        active.version = Set(version.to_string());
        active.updated_at = Set(chrono::Utc::now().timestamp());

        active
            .update(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub async fn delete_skill(&self, skill_id: &str, user_id: &str) -> Result<()> {
        use dgos_db::entities::skills;

        let model = skills::Entity::find()
            .filter(skills::Column::SkillId.eq(skill_id))
            .filter(skills::Column::UserId.eq(user_id))
            .one(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?
            .ok_or_else(|| SkillError::SkillNotFound(skill_id.to_string()))?;

        let active: skills::ActiveModel = model.into();
        active
            .delete(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub async fn skill_exists(&self, skill_id: &str, user_id: &str) -> Result<bool> {
        use dgos_db::entities::skills;

        let count = skills::Entity::find()
            .filter(skills::Column::SkillId.eq(skill_id))
            .filter(skills::Column::UserId.eq(user_id))
            .count(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(count > 0)
    }

    fn model_to_skill(model: dgos_db::entities::skills::Model) -> Result<Skill> {
        use crate::entity::CallType;

        Ok(Skill {
            id: model.id,
            skill_id: model.skill_id,
            name: model.name,
            description: model.description,
            version: model.version,
            author: model.author,
            install_path: model.install_path,
            call_type: CallType::from_str(&model.call_type)
                .ok_or_else(|| SkillError::Internal("Invalid call_type".to_string()))?,
            enabled: model.enabled,
            config: model.config,
            manifest: serde_json::from_value(model.manifest)
                .map_err(|e| SkillError::InvalidManifest(e.to_string()))?,
            installed_at: model.installed_at,
            updated_at: model.updated_at,
            user_id: model.user_id,
        })
    }

    // Usage logs
    pub async fn insert_usage_log(&self, log: &SkillUsageLog) -> Result<()> {
        use dgos_db::entities::skill_usage_logs;

        let model = skill_usage_logs::ActiveModel {
            id: Set(log.id.clone()),
            skill_id: Set(log.skill_id.clone()),
            user_id: Set(log.user_id.clone()),
            input_params: Set(log.input_params.clone()),
            output_result: Set(log.output_result.clone()),
            success: Set(log.success),
            error_message: Set(log.error_message.clone()),
            latency_ms: Set(log.latency_ms),
            created_at: Set(log.created_at),
        };

        model
            .insert(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub async fn list_usage_logs(
        &self,
        user_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<SkillUsageLog>> {
        use dgos_db::entities::skill_usage_logs;

        let mut query = skill_usage_logs::Entity::find()
            .filter(skill_usage_logs::Column::UserId.eq(user_id))
            .order_by_desc(skill_usage_logs::Column::CreatedAt);

        if let Some(limit) = limit {
            query = query.limit(limit as u64);
        }

        let models = query
            .all(&self.db)
            .await
            .map_err(|e| SkillError::DatabaseError(e.to_string()))?;

        Ok(models
            .into_iter()
            .map(|m| SkillUsageLog {
                id: m.id,
                skill_id: m.skill_id,
                user_id: m.user_id,
                input_params: m.input_params,
                output_result: m.output_result,
                success: m.success,
                error_message: m.error_message,
                latency_ms: m.latency_ms,
                created_at: m.created_at,
            })
            .collect())
    }
}
