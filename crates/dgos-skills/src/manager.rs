use crate::dependency::DependencyChecker;
use crate::entity::{
    CallType, Skill, SkillDetail, SkillManifest, SkillRegistryCache,
    SkillUpdate, SkillUsageLog,
};
use crate::error::{Result, SkillError};
use crate::repository::{RegistryRepository, SkillRepository};
use dgos_db::sea_orm::DatabaseConnection;
use flate2::read::GzDecoder;
use reqwest::Client;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tar::Archive;
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{debug, info};
use uuid::Uuid;

const REGISTRY_URL: &str = "https://skills.dgos.ai/registry.json";
const SKILL_START_TIMEOUT: u64 = 5;
const SKILL_CALL_TIMEOUT: u64 = 30;

pub struct SkillsManager {
    skills_dir: PathBuf,
    registry_url: String,
    skill_repo: SkillRepository,
    registry_repo: RegistryRepository,
    http_clients: Arc<RwLock<HashMap<String, HttpSkillClient>>>,
    http_client: Client,
}

struct HttpSkillClient {
    base_url: String,
    client: Client,
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
            registry_url: REGISTRY_URL.to_string(),
            skill_repo: SkillRepository::new(db.clone()),
            registry_repo: RegistryRepository::new(db),
            http_clients: Arc::new(RwLock::new(HashMap::new())),
            http_client: Client::new(),
        }
    }

    // Network failures are expected offline; database failures still propagate.
    pub async fn fetch_registry(&self) -> Result<crate::entity::RegistrySnapshot> {
        let warning = match self.sync_registry().await {
            Ok(()) => None,
            Err(SkillError::RegistryUnreachable(message)) => Some(message),
            Err(error) => return Err(error),
        };

        Ok(crate::entity::RegistrySnapshot {
            skills: self.registry_repo.list_registry_skills(None).await?,
            registry_url: self.registry_url.clone(),
            offline: warning.is_some(),
            warning,
        })
    }

    // B02-01: Fetch skills from registry
    async fn sync_registry(&self) -> Result<()> {
        info!("Fetching skills registry from {}", self.registry_url);

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

        let registry: crate::entity::SkillRegistry = response
            .json()
            .await
            .map_err(|e| SkillError::RegistryUnreachable(e.to_string()))?;

        // Cache to database
        let now = chrono::Utc::now().timestamp();
        for entry in &registry.skills {
            let cache_entry = SkillRegistryCache {
                id: entry.id.clone(),
                skill_id: entry.id.clone(),
                name: entry.name.clone(),
                description: entry.description.clone(),
                latest_version: entry.version.clone(),
                author: entry.author.clone(),
                category: entry.category.clone(),
                tags: entry.tags.clone(),
                download_url: entry.download_url.clone(),
                manifest: entry.manifest.clone(),
                synced_at: now,
            };

            self.registry_repo.upsert_registry_entry(&cache_entry).await?;
        }

        Ok(())
    }

    pub async fn search_skills(&self, query: &str) -> Result<Vec<crate::entity::SkillInfo>> {
        self.registry_repo.search_skills(query).await
    }

    pub async fn get_skill_detail(&self, skill_id: &str) -> Result<SkillDetail> {
        self.registry_repo.get_registry_skill(skill_id).await
    }

    // B02-02: Install skill
    pub async fn install_skill(&self, skill_id: &str, user_id: &str) -> Result<Skill> {
        info!("Installing skill: {} for user: {}", skill_id, user_id);

        // Check if already installed
        if self.skill_repo.skill_exists(skill_id, user_id).await? {
            return Err(SkillError::SkillAlreadyInstalled(skill_id.to_string()));
        }

        // Get skill info from registry
        let skill_detail = self.registry_repo.get_registry_skill(skill_id).await?;

        // B02-03: Check dependencies
        let dep_status = DependencyChecker::check_dependencies(&skill_detail.manifest.dependencies)?;
        if !dep_status.satisfied {
            return Err(SkillError::DependencyNotSatisfied(format!(
                "Missing: {}",
                dep_status.missing.join(", ")
            )));
        }

        // Download skill
        debug!("Downloading skill from: {}", skill_detail.download_url);
        let response = self
            .http_client
            .get(&skill_detail.download_url)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;

        let bytes = response.bytes().await?;

        // Save to temp file
        let temp_path = std::env::temp_dir().join(format!("dgos-skill-{}.tar.gz", skill_id));
        fs::write(&temp_path, &bytes).await?;

        // Extract to install directory
        let install_path = self.skills_dir.join(skill_id);
        fs::create_dir_all(&install_path).await?;

        self.extract_tarball(&temp_path, &install_path).await?;

        // Verify integrity
        self.verify_skill_integrity(&install_path).await?;

        // Create skill record
        let now = chrono::Utc::now().timestamp();
        let skill = Skill {
            id: Uuid::new_v4().to_string(),
            skill_id: skill_id.to_string(),
            name: skill_detail.name,
            description: skill_detail.description,
            version: skill_detail.latest_version,
            author: skill_detail.author,
            install_path: install_path.to_string_lossy().to_string(),
            call_type: skill_detail.manifest.call_type.clone(),
            enabled: true,
            config: None,
            manifest: skill_detail.manifest,
            installed_at: now,
            updated_at: now,
            user_id: user_id.to_string(),
        };

        // Save to database
        self.skill_repo.insert_skill(&skill).await?;

        // Cleanup temp file
        let _ = fs::remove_file(&temp_path).await;

        info!("Skill {} installed successfully", skill_id);
        Ok(skill)
    }

    async fn extract_tarball(&self, archive_path: &Path, dest: &Path) -> Result<()> {
        let file = std::fs::File::open(archive_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);
        archive.unpack(dest)?;
        Ok(())
    }

    async fn verify_skill_integrity(&self, install_path: &Path) -> Result<()> {
        // Check manifest.json exists
        let manifest_path = install_path.join("manifest.json");
        if !manifest_path.exists() {
            return Err(SkillError::InvalidManifest(
                "manifest.json not found".to_string(),
            ));
        }

        // Parse and validate manifest
        let manifest_str = fs::read_to_string(&manifest_path).await?;
        let _manifest: SkillManifest = serde_json::from_str(&manifest_str)
            .map_err(|e| SkillError::InvalidManifest(e.to_string()))?;

        Ok(())
    }

    pub async fn uninstall_skill(&self, skill_id: &str, user_id: &str) -> Result<()> {
        info!("Uninstalling skill: {} for user: {}", skill_id, user_id);

        let skill = self.skill_repo.get_skill_by_skill_id(skill_id, user_id).await?;

        // Stop HTTP service if running
        if skill.call_type == CallType::Http {
            let mut clients = self.http_clients.write().await;
            clients.remove(skill_id);
        }

        // Delete files
        let install_path = PathBuf::from(&skill.install_path);
        if install_path.exists() {
            fs::remove_dir_all(&install_path).await?;
        }

        // Delete from database
        self.skill_repo.delete_skill(skill_id, user_id).await?;

        info!("Skill {} uninstalled successfully", skill_id);
        Ok(())
    }

    pub async fn list_installed_skills(&self, user_id: &str) -> Result<Vec<Skill>> {
        self.skill_repo.list_skills(user_id, false).await
    }

    pub async fn toggle_skill(&self, skill_id: &str, user_id: &str, enabled: bool) -> Result<()> {
        self.skill_repo.toggle_skill(skill_id, user_id, enabled).await
    }

    pub async fn update_skill_config(
        &self,
        skill_id: &str,
        user_id: &str,
        config: JsonValue,
    ) -> Result<()> {
        self.skill_repo
            .update_skill_config(skill_id, user_id, config)
            .await
    }

    // B02-04: Call HTTP skill
    pub async fn call_http_skill(
        &self,
        skill_id: &str,
        user_id: &str,
        params: JsonValue,
    ) -> Result<JsonValue> {
        let skill = self.skill_repo.get_skill_by_skill_id(skill_id, user_id).await?;

        // B02-05: Check if enabled
        if !skill.enabled {
            return Err(SkillError::SkillDisabled(skill_id.to_string()));
        }

        // B02-06: Check config
        if skill.manifest.config_schema.is_some() && skill.config.is_none() {
            return Err(SkillError::SkillNotConfigured(skill_id.to_string()));
        }

        if skill.call_type != CallType::Http {
            return Err(SkillError::NotHttpSkill);
        }

        // B02-07: Start HTTP service if needed
        {
            let clients = self.http_clients.read().await;
            if !clients.contains_key(skill_id) {
                drop(clients);
                self.start_http_skill(&skill).await?;
            }
        }

        // B02-08: Send HTTP request
        let start_time = Instant::now();
        let client = {
            let clients = self.http_clients.read().await;
            clients
                .get(skill_id)
                .ok_or_else(|| SkillError::SkillNotStarted(skill_id.to_string()))?
                .client
                .clone()
        };

        let base_url = {
            let clients = self.http_clients.read().await;
            clients
                .get(skill_id)
                .map(|c| c.base_url.clone())
                .ok_or_else(|| SkillError::SkillNotStarted(skill_id.to_string()))?
        };

        let response = client
            .post(format!("{}/execute", base_url))
            .json(&params)
            .timeout(Duration::from_secs(SKILL_CALL_TIMEOUT))
            .send()
            .await
            .map_err(|e| SkillError::SkillExecutionFailed(e.to_string()))?;

        let latency_ms = start_time.elapsed().as_millis() as i32;

        // B02-09: Parse result
        if !response.status().is_success() {
            let error_msg = format!("HTTP {}", response.status());
            self.log_usage(skill_id, user_id, &params, &json!({"error": &error_msg}), false, latency_ms)
                .await?;
            return Err(SkillError::SkillExecutionFailed(error_msg));
        }

        let result: JsonValue = response
            .json()
            .await
            .map_err(|e| SkillError::SkillExecutionFailed(e.to_string()))?;

        // Log usage
        self.log_usage(skill_id, user_id, &params, &result, true, latency_ms)
            .await?;

        Ok(result)
    }

    // B02-10: Start HTTP skill
    async fn start_http_skill(&self, skill: &Skill) -> Result<()> {
        let http_config = skill
            .manifest
            .http
            .as_ref()
            .ok_or(SkillError::NotHttpSkill)?;

        info!("Starting HTTP skill: {}", skill.skill_id);

        let cwd = PathBuf::from(&skill.install_path);
        let command = &http_config.start_command;

        // Start process
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .env("PORT", http_config.port.to_string());

        if let Some(config) = &skill.config {
            cmd.env("CONFIG", serde_json::to_string(config)?);
        }

        let _child = cmd
            .spawn()
            .map_err(|e| SkillError::SkillStartTimeout(e.to_string()))?;

        // Wait for service to be ready
        let base_url = format!("http://localhost:{}", http_config.port);
        let health_url = format!("{}{}", base_url, http_config.health_check);

        for i in 0..10 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Ok(response) = self.http_client.get(&health_url).send().await {
                if response.status().is_success() {
                    // Service is ready
                    let client = HttpSkillClient {
                        base_url: base_url.clone(),
                        client: Client::new(),
                    };

                    let mut clients = self.http_clients.write().await;
                    clients.insert(skill.skill_id.clone(), client);

                    info!("HTTP skill {} started successfully", skill.skill_id);
                    return Ok(());
                }
            }

            if i == 9 {
                return Err(SkillError::SkillStartTimeout(format!(
                    "Health check failed after {} seconds",
                    SKILL_START_TIMEOUT
                )));
            }
        }

        Ok(())
    }

    // B02-11: Call CLI skill
    pub async fn call_cli_skill(
        &self,
        skill_id: &str,
        user_id: &str,
        params: JsonValue,
    ) -> Result<JsonValue> {
        let skill = self.skill_repo.get_skill_by_skill_id(skill_id, user_id).await?;

        if !skill.enabled {
            return Err(SkillError::SkillDisabled(skill_id.to_string()));
        }

        if skill.call_type != CallType::Cli {
            return Err(SkillError::NotCliSkill);
        }

        let cli_config = skill.manifest.cli.as_ref().ok_or(SkillError::NotCliSkill)?;

        let cwd = PathBuf::from(&skill.install_path);
        let command = &cli_config.command;

        // B02-12: Execute command
        let start_time = Instant::now();
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(&cwd)
            .env("INPUT", serde_json::to_string(&params)?);

        if let Some(config) = &skill.config {
            cmd.env("CONFIG", serde_json::to_string(config)?);
        }

        let output = cmd
            .output()
            .map_err(|e| SkillError::SkillExecutionFailed(e.to_string()))?;

        let latency_ms = start_time.elapsed().as_millis() as i32;

        // B02-13: Parse output
        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr).to_string();
            self.log_usage(
                skill_id,
                user_id,
                &params,
                &json!({"error": &error_msg}),
                false,
                latency_ms,
            )
            .await?;
            return Err(SkillError::SkillExecutionFailed(error_msg));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: JsonValue = serde_json::from_str(&stdout)
            .map_err(|e| SkillError::SkillExecutionFailed(e.to_string()))?;

        self.log_usage(skill_id, user_id, &params, &result, true, latency_ms)
            .await?;

        Ok(result)
    }

    // B02-14: Update skill
    pub async fn update_skill(&self, skill_id: &str, user_id: &str) -> Result<Skill> {
        info!("Updating skill: {} for user: {}", skill_id, user_id);

        let old_skill = self.skill_repo.get_skill_by_skill_id(skill_id, user_id).await?;
        let skill_detail = self.registry_repo.get_registry_skill(skill_id).await?;

        // Check if update needed
        if old_skill.version == skill_detail.latest_version {
            info!("Skill {} is already up to date", skill_id);
            return Ok(old_skill);
        }

        // B02-15: Backup old version
        let backup_path = format!("{}.backup", old_skill.install_path);
        fs::rename(&old_skill.install_path, &backup_path).await?;

        // Download new version
        let response = self
            .http_client
            .get(&skill_detail.download_url)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;

        let bytes = response.bytes().await?;
        let temp_path = std::env::temp_dir().join(format!("dgos-skill-{}-update.tar.gz", skill_id));
        fs::write(&temp_path, &bytes).await?;

        // Extract
        let install_path = PathBuf::from(&old_skill.install_path);
        fs::create_dir_all(&install_path).await?;
        self.extract_tarball(&temp_path, &install_path).await?;

        // Update database
        self.skill_repo
            .update_skill_version(skill_id, user_id, &skill_detail.latest_version)
            .await?;

        // Cleanup
        let _ = fs::remove_file(&temp_path).await;
        let _ = fs::remove_dir_all(&backup_path).await;

        // Restart HTTP service if needed
        if old_skill.call_type == CallType::Http {
            let mut clients = self.http_clients.write().await;
            clients.remove(skill_id);
        }

        info!("Skill {} updated successfully", skill_id);
        self.skill_repo.get_skill_by_skill_id(skill_id, user_id).await
    }

    pub async fn check_skill_updates(&self, user_id: &str) -> Result<Vec<SkillUpdate>> {
        let installed = self.skill_repo.list_skills(user_id, false).await?;
        let mut updates = Vec::new();

        for skill in installed {
            if let Ok(detail) = self.registry_repo.get_registry_skill(&skill.skill_id).await {
                if skill.version != detail.latest_version {
                    updates.push(SkillUpdate {
                        skill_id: skill.skill_id.clone(),
                        current_version: skill.version.clone(),
                        latest_version: detail.latest_version,
                        changelog: None,
                    });
                }
            }
        }

        Ok(updates)
    }

    async fn log_usage(
        &self,
        skill_id: &str,
        user_id: &str,
        params: &JsonValue,
        result: &JsonValue,
        success: bool,
        latency_ms: i32,
    ) -> Result<()> {
        let log = SkillUsageLog {
            id: Uuid::new_v4().to_string(),
            skill_id: skill_id.to_string(),
            user_id: user_id.to_string(),
            input_params: params.clone(),
            output_result: Some(result.clone()),
            success,
            error_message: if success {
                None
            } else {
                result.get("error").and_then(|e| e.as_str()).map(String::from)
            },
            latency_ms,
            created_at: chrono::Utc::now().timestamp(),
        };

        self.skill_repo.insert_usage_log(&log).await
    }

    pub async fn list_usage_logs(&self, user_id: &str, limit: Option<usize>) -> Result<Vec<SkillUsageLog>> {
        self.skill_repo.list_usage_logs(user_id, limit).await
    }
}
