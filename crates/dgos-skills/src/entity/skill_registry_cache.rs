use serde::{Deserialize, Serialize};

use super::SkillManifest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRegistryCache {
    pub id: String,
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub latest_version: String,
    pub author: Option<String>,
    pub category: String,
    pub tags: Vec<String>,
    pub download_url: String,
    pub manifest: SkillManifest,
    pub synced_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub latest_version: String,
    pub author: Option<String>,
    pub category: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RegistrySnapshot {
    pub skills: Vec<SkillInfo>,
    pub registry_url: String,
    pub offline: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDetail {
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub latest_version: String,
    pub author: Option<String>,
    pub category: String,
    pub tags: Vec<String>,
    pub manifest: SkillManifest,
    pub download_url: String,
}

#[derive(Debug, Deserialize)]
pub struct SkillRegistry {
    pub skills: Vec<SkillRegistryEntry>,
}

#[derive(Debug, Deserialize)]
pub struct SkillRegistryEntry {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: String,
    pub author: Option<String>,
    pub category: String,
    pub tags: Vec<String>,
    pub download_url: String,
    pub manifest: SkillManifest,
}
