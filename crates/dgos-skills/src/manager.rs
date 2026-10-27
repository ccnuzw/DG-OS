use crate::{
    repository::{ContentRepository, RegistryRepository, SkillRepository},
    *,
};
use dgos_db::sea_orm::DatabaseConnection;
use reqwest::Client;
use std::{path::PathBuf, time::Duration};

pub struct SkillsManager {
    pub(crate) skills_dir: PathBuf,
    // No undeployed public registry is required for local startup. Only legacy
    // tests configure this endpoint; future external sources get adapters.
    registry_url: String,
    skill_repo: SkillRepository,
    registry_repo: RegistryRepository,
    pub(crate) content_repo: ContentRepository,
    http_client: Client,
}

impl SkillsManager {
    #[cfg(test)]
    pub(crate) fn with_test_registry(mut self, url: String) -> Self {
        self.registry_url = url;
        self
    }

    pub fn new(db: DatabaseConnection, skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            registry_url: String::new(),
            skill_repo: SkillRepository::new(db.clone()),
            registry_repo: RegistryRepository::new(db.clone()),
            content_repo: ContentRepository::new(db),
            http_client: Client::new(),
        }
    }

    pub async fn fetch_registry(&self) -> Result<RegistrySnapshot> {
        let warning = match self.sync_registry().await {
            Ok(()) => None,
            Err(SkillError::RegistryUnreachable(message)) => Some(message),
            Err(error) => return Err(error),
        };
        let mut skills = self.content_repo.list_catalog().await?;
        skills.extend(self.registry_repo.list_registry_skills(None).await?);
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(RegistrySnapshot {
            skills,
            registry_url: if self.registry_url.is_empty() {
                "bundled://official".into()
            } else {
                self.registry_url.clone()
            },
            offline: warning.is_some(),
            warning,
        })
    }

    async fn sync_registry(&self) -> Result<()> {
        if self.registry_url.is_empty() {
            return Ok(());
        }
        let response = self
            .http_client
            .get(&self.registry_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| SkillError::RegistryUnreachable(e.to_string()))?;
        if !response.status().is_success() {
            return Err(SkillError::RegistryUnreachable(format!(
                "HTTP {}",
                response.status()
            )));
        }
        let registry: SkillRegistry = response
            .json()
            .await
            .map_err(|e| SkillError::RegistryUnreachable(e.to_string()))?;
        for entry in registry.skills {
            self.registry_repo
                .upsert_registry_entry(&SkillRegistryCache {
                    id: entry.id.clone(),
                    skill_id: entry.id,
                    name: entry.name,
                    description: entry.description,
                    latest_version: entry.version,
                    author: entry.author,
                    category: entry.category,
                    tags: entry.tags,
                    download_url: entry.download_url,
                    manifest: entry.manifest,
                    synced_at: chrono::Utc::now().timestamp(),
                })
                .await?;
        }
        Ok(())
    }

    pub async fn search_skills(&self, query: &str) -> Result<Vec<SkillInfo>> {
        let mut skills = self.content_repo.search_catalog(query).await?;
        skills.extend(self.registry_repo.search_skills(query).await?);
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(skills)
    }

    pub async fn get_skill_detail(&self, id: &str) -> Result<SkillDetail> {
        if let Some(entry) = self.content_repo.get_catalog_entry(id).await? {
            return self.content_detail(entry).await;
        }
        self.registry_repo.get_registry_skill(id).await
    }

    pub async fn install_skill(&self, id: &str, user: &str) -> Result<Skill> {
        if self.content_repo.get_catalog_entry(id).await?.is_none() {
            return Err(SkillError::LegacyFormatUnsupported);
        }
        self.get_skill_detail(id).await?;
        self.content_repo.add_user(id, user).await?;
        self.content_repo
            .user_skills(user)
            .await?
            .into_iter()
            .find(|skill| skill.skill_id == id)
            .ok_or_else(|| SkillError::SkillNotFound(id.into()))
    }

    pub async fn list_installed_skills(&self, user: &str) -> Result<Vec<Skill>> {
        let mut skills = self.content_repo.user_skills(user).await?;
        // Preserve all legacy rows, credentials and records without converting
        // them or permitting downloaded commands to execute.
        skills.extend(self.skill_repo.list_skills(user, false).await?);
        Ok(skills)
    }

    pub async fn toggle_skill(&self, id: &str, user: &str, enabled: bool) -> Result<()> {
        if self.content_repo.get_catalog_entry(id).await?.is_some() {
            return self.content_repo.set_enabled(id, user, enabled).await;
        }
        if enabled {
            return Err(SkillError::LegacyFormatUnsupported);
        }
        self.skill_repo.toggle_skill(id, user, false).await
    }

    pub async fn uninstall_skill(&self, id: &str, _user: &str) -> Result<()> {
        if self.content_repo.get_catalog_entry(id).await?.is_some() {
            return Err(SkillError::InvalidManifest(
                "Bundled content cannot be uninstalled; disable it instead".into(),
            ));
        }
        // Deletion requires a migration/ownership review: old installs may share
        // files across users. Never delete arbitrary legacy install_path values.
        Err(SkillError::LegacyFormatUnsupported)
    }

    pub async fn update_skill_config(
        &self,
        _id: &str,
        _user: &str,
        _config: serde_json::Value,
    ) -> Result<()> {
        Err(SkillError::LegacyFormatUnsupported)
    }
    pub async fn call_http_skill(
        &self,
        _id: &str,
        _user: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        Err(SkillError::LegacyFormatUnsupported)
    }
    pub async fn call_cli_skill(
        &self,
        _id: &str,
        _user: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        Err(SkillError::LegacyFormatUnsupported)
    }
    pub async fn update_skill(&self, _id: &str, _user: &str) -> Result<Skill> {
        Err(SkillError::LegacyFormatUnsupported)
    }
    pub async fn check_skill_updates(&self, _user: &str) -> Result<Vec<SkillUpdate>> {
        Ok(Vec::new())
    }
    pub async fn list_usage_logs(
        &self,
        user: &str,
        limit: Option<usize>,
    ) -> Result<Vec<SkillUsageLog>> {
        self.skill_repo
            .list_usage_logs(user, limit.map(|n| n.min(100)))
            .await
    }
}
