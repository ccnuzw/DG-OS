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
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub source_kind: String,
    #[serde(default)]
    pub license_spdx: Option<String>,
    #[serde(default)]
    pub compatible_apps: Vec<String>,
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
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub source_kind: String,
    #[serde(default)]
    pub license_spdx: Option<String>,
    #[serde(default)]
    pub compatible_apps: Vec<String>,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub content: Option<ContentPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPreview {
    pub body: String,
    pub content_hash: String,
    pub source_revision: Option<String>,
    pub verification: ContentVerification,
    pub license_text: String,
    pub required_tools: Vec<String>,
}

/// Metadata used by the SKILL.md content package. It intentionally contains
/// no executable command or permission grant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentSkillMetadata {
    pub schema_version: String,
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub identity: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub entry: String,
    #[serde(default)]
    pub locales: Vec<String>,
    #[serde(default)]
    pub compatible_apps: Vec<String>,
    #[serde(default)]
    pub required_tools: Vec<String>,
    pub author: ContentAuthor,
    pub license: ContentLicense,
    pub source: ContentSource,
    pub verification: ContentVerification,
}

impl ContentSkillMetadata {
    // Transitional desktop DTO only; never persists a content skill in the
    // legacy skills table or invents an executable command.
    pub fn compatibility_manifest(&self) -> SkillManifest {
        SkillManifest {
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            author: self.author.name.clone(),
            category: self.category.clone(),
            tags: self.tags.clone(),
            call_type: super::CallType::Content,
            dependencies: super::DependencyConfig {
                python: None,
                node: None,
                packages: None,
            },
            config_schema: None,
            http: None,
            cli: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAuthor {
    pub name: String,
    #[serde(default, rename = "type")]
    pub author_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentLicense {
    pub spdx: Option<String>,
    #[serde(default)]
    pub redistributable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentSource {
    pub kind: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentVerification {
    pub status: String,
    #[serde(default)]
    pub scope: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BundledSkill {
    pub metadata: ContentSkillMetadata,
    pub install_path: String,
    pub content_hash: String,
    pub files: std::collections::BTreeMap<String, String>,
    pub body: String,
    pub license_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedContent {
    pub skill_id: String,
    pub version: String,
    pub content_hash: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillPackageFileKind {
    Body,
    Manifest,
    License,
    Resource,
    Script,
    Hook,
}

/// Host-only package bytes. Debug intentionally omits the payload.
pub struct SkillPackageFile {
    pub path: String,
    pub kind: SkillPackageFileKind,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for SkillPackageFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillPackageFile")
            .field("path", &self.path)
            .field("kind", &self.kind)
            .field("byte_count", &self.bytes.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillPackageFeatureSupport {
    Unsupported,
}

#[derive(Debug)]
pub struct SkillPackageCompatibility {
    pub compatible_apps: Vec<String>,
    pub required_tools: Vec<String>,
    pub scripts: SkillPackageFeatureSupport,
    pub hooks: SkillPackageFeatureSupport,
    pub reason: &'static str,
}

/// Verified immutable bytes for publication under the host's application data
/// directory. This host-only type deliberately has no serialization contract.
/// Loading or enabling content grants no script, Hook, or native-shell authority.
#[derive(Debug)]
pub struct PinnedSkillPackage {
    pub skill_id: String,
    pub version: String,
    pub content_hash: String,
    pub compatibility: SkillPackageCompatibility,
    pub(crate) files: Vec<SkillPackageFile>,
    pub(crate) body_index: usize,
}

impl PinnedSkillPackage {
    /// Borrow the verified UTF-8 body without duplicating its file bytes.
    pub fn body(&self) -> &str {
        std::str::from_utf8(&self.files[self.body_index].bytes)
            .expect("validated package body is UTF-8")
    }

    /// Includes SKILL.md, manifest/license when present, and every resource.
    pub fn files(&self) -> &[SkillPackageFile] {
        &self.files
    }

    /// Transfer ownership to an application-data publisher without copying.
    pub fn into_files(self) -> Vec<SkillPackageFile> {
        self.files
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BundledCatalog {
    pub schema_version: String,
    pub catalog_version: String,
    pub skills: Vec<BundledCatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BundledCatalogEntry {
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub path: String,
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
