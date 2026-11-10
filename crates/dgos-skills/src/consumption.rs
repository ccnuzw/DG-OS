use crate::{ContentSkillMetadata, Result, SkillError, SkillUsageLog, SkillsManager};
use dgos_types::AiSkillRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Installed, enabled, workbench-compatible metadata. Discovery never opens a package file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCatalogEntry {
    pub pin: AiSkillRef,
    pub name: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
}

/// Lexical metadata matches, not a semantic/model recommendation or authorization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRecommendation {
    pub entry: SkillCatalogEntry,
    pub score: u32,
    pub reasons: Vec<String>,
}

/// Content-free evidence of the exact bytes read, never evidence of model understanding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillLoadReceipt {
    pub pin: AiSkillRef,
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SkillReadResult {
    pub content: String,
    pub receipt: SkillLoadReceipt,
}

impl std::fmt::Debug for SkillReadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillReadResult").field("receipt", &self.receipt).finish()
    }
}

fn metadata(mut value: serde_json::Value) -> Result<ContentSkillMetadata> {
    if let Some(object) = value.as_object_mut() {
        object.remove("provenance");
        object.remove("file_manifest");
    }
    Ok(serde_json::from_value(value)?)
}

fn compatible(meta: &ContentSkillMetadata) -> bool {
    meta.compatible_apps.iter().any(|app| app == "ai-workbench") && meta.required_tools.is_empty()
}

/// Only the body and supported data resources are model-readable. No normalization:
/// a noncanonical request is rejected, not silently rewritten to another asset.
pub fn validate_skill_read_path(path: &str) -> Result<()> {
    let parts: Vec<_> = path.split('/').collect();
    let canonical = !path.is_empty()
        && parts.len() <= 8
        && parts.iter().all(|part| !part.is_empty() && *part != "." && *part != "..")
        && path.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte));
    let resource = parts.len() >= 2
        && ["references", "templates", "examples"].contains(&parts[0])
        && matches!(std::path::Path::new(path).extension().and_then(|v| v.to_str()), Some("md" | "txt" | "json" | "csv"));
    if !canonical || (path != "SKILL.md" && !resource) {
        return Err(SkillError::InvalidManifest("unsupported Skill read path".into()));
    }
    Ok(())
}

impl SkillsManager {
    pub async fn chat_skill_catalog(&self, user: &str) -> Result<Vec<SkillCatalogEntry>> {
        let mut entries = Vec::new();
        for skill in self.content_repo.user_skills(user).await? {
            if !skill.enabled { continue; }
            let row = self.content_repo.enabled_version(&skill.skill_id, user, &skill.version).await?;
            let meta = metadata(row.metadata)?;
            if meta.id != skill.skill_id || meta.version != skill.version || !compatible(&meta) { continue; }
            entries.push(SkillCatalogEntry {
                pin: AiSkillRef { skill_id: skill.skill_id, version: skill.version, content_hash: row.content_hash },
                name: meta.name,
                description: meta.description,
                category: meta.category,
                tags: meta.tags,
            });
        }
        entries.sort_by(|a, b| a.pin.skill_id.cmp(&b.pin.skill_id));
        Ok(entries)
    }

    /// Metadata-only authorization recheck. Actual content reads additionally verify the full snapshot.
    pub async fn validate_chat_skill_pin(&self, user: &str, pin: &AiSkillRef) -> Result<()> {
        let row = self.content_repo.enabled_version(&pin.skill_id, user, &pin.version).await?;
        let meta = metadata(row.metadata)?;
        if row.content_hash != pin.content_hash || meta.id != pin.skill_id || meta.version != pin.version {
            return Err(SkillError::InvalidManifest("selected Skill pin changed".into()));
        }
        if !compatible(&meta) {
            return Err(SkillError::DependencyNotSatisfied("workbench compatibility or tools unavailable".into()));
        }
        Ok(())
    }

    /// Read one immutable text asset; never installs, executes scripts, resolves links or grants tools.
    /// Every read validates the complete bounded package and rechecks current user state.
    /// The durable usage row is content-free; audit failure withholds content from the caller.
    pub async fn read_pinned_skill_text(
        &self, user: &str, pin: &AiSkillRef, path: &str, max_bytes: usize,
    ) -> Result<SkillReadResult> {
        let started = std::time::Instant::now();
        validate_skill_read_path(path)?;
        if max_bytes == 0 { return Err(SkillError::InvalidManifest("Skill context byte budget exhausted".into())); }
        self.validate_chat_skill_pin(user, pin).await?;
        let package = self.load_pinned_package(user, &pin.skill_id, &pin.version, &pin.content_hash).await?;
        let file = package.into_files().into_iter().find(|file| file.path == path)
            .ok_or_else(|| SkillError::InvalidManifest("Skill resource not found in pinned package".into()))?;
        if file.bytes.len() > max_bytes.min(crate::content_package::MAX_FILE_BYTES as usize) {
            return Err(SkillError::InvalidManifest("Skill resource exceeds context byte budget".into()));
        }
        let receipt = SkillLoadReceipt {
            pin: pin.clone(), path: path.into(), sha256: format!("{:x}", Sha256::digest(&file.bytes)), bytes: file.bytes.len(),
        };
        let content = String::from_utf8(file.bytes)
            .map_err(|_| SkillError::InvalidManifest("Skill resource is not UTF-8".into()))?;
        self.skill_repo.insert_usage_log(&SkillUsageLog {
            id: uuid::Uuid::new_v4().to_string(), skill_id: pin.skill_id.clone(), user_id: user.into(),
            caller: Some("workbench".into()), request_id: None, task_ref: None,
            input_params: serde_json::json!({"operation":"read_pinned_skill_text", "path":path}),
            output_result: Some(serde_json::to_value(&receipt)?), success: true, error_message: None,
            latency_ms: started.elapsed().as_millis().min(i32::MAX as u128) as i32, created_at: chrono::Utc::now().timestamp(),
        }).await?;
        self.validate_chat_skill_pin(user, pin).await?;
        Ok(SkillReadResult { content, receipt })
    }
}

fn terms(query: &str) -> BTreeSet<String> {
    let lower = query.chars().take(4096).collect::<String>().to_lowercase();
    let mut terms: BTreeSet<String> = lower.split(|c: char| !c.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2).take(128).map(str::to_owned).collect();
    // CJK has no mandatory spaces: deterministic bigrams make Chinese metadata discoverable.
    let chars: Vec<char> = lower.chars().take(2048).collect();
    for pair in chars.windows(2) {
        if pair.iter().all(|c| ('\u{3400}'..='\u{9fff}').contains(c)) {
            terms.insert(pair.iter().collect());
        }
    }
    terms
}

pub fn recommend_skills(catalog: &[SkillCatalogEntry], query: &str, limit: usize) -> Vec<SkillRecommendation> {
    let terms = terms(query);
    let mut recommendations = Vec::new();
    for entry in catalog {
        let fields = [("name", entry.name.to_lowercase(), 4), ("tags", entry.tags.join(" ").to_lowercase(), 3),
            ("description", entry.description.to_lowercase(), 1), ("category", entry.category.to_lowercase(), 1)];
        let mut reasons = Vec::new();
        let mut score = 0;
        for (field, value, weight) in fields {
            let matches: Vec<_> = terms.iter().filter(|term| value.contains(term.as_str())).cloned().collect();
            if !matches.is_empty() {
                score += (matches.len() as u32) * weight;
                reasons.push(format!("lexical_{field}:{}", matches.join(",")));
            }
        }
        if score > 0 { recommendations.push(SkillRecommendation { entry: entry.clone(), score, reasons }); }
    }
    recommendations.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.entry.pin.skill_id.cmp(&b.entry.pin.skill_id)));
    recommendations.truncate(limit.min(32));
    recommendations
}
