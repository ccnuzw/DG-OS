use dgos_skills::{SkillCatalogEntry, SkillLoadReceipt, SkillReadResult, SkillsManager};
use dgos_types::{AiSkillRef, ToolDefinition, ToolFunction};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const SKILL_LIST_TOOL: &str = "dgos_skill_list";
pub const SKILL_LOAD_TOOL: &str = "dgos_skill_load";
pub const SKILL_READ_TOOL: &str = "dgos_skill_read";
pub const MAX_AUTHORIZED_SKILLS: usize = 32;
pub const MAX_RUNTIME_PATHS: usize = 128;
pub const MAX_RUNTIME_BYTES: usize = 2 * 1024 * 1024;

/// Per-run host-local read capability. Construct only for explicit on-demand opt-in.
/// It owns no filesystem path or manager lock. Cloning is for transport of a single
/// run's state, not parallel dispatch: the existing serial tool loop owns mutation.
/// Skill text is untrusted data, never a source of execution or MCP permissions.
#[derive(Clone, Debug)]
pub struct SkillRuntime {
    user: String,
    pins: Vec<AiSkillRef>,
    max_total_bytes: usize,
    max_paths: usize,
    consumed_bytes: usize,
    receipts: Vec<SkillLoadReceipt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillArg { skill_id: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceArg { skill_id: String, path: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArg {}

impl SkillRuntime {
    pub async fn authorize(
        manager: &SkillsManager, user: &str, authorized_pins: &[AiSkillRef],
        max_total_bytes: usize, max_paths: usize,
    ) -> Result<Self, String> {
        if user.is_empty() || authorized_pins.len() > MAX_AUTHORIZED_SKILLS {
            return Err("Invalid Skill authorization scope".into());
        }
        if max_total_bytes == 0 || max_total_bytes > MAX_RUNTIME_BYTES || max_paths == 0 || max_paths > MAX_RUNTIME_PATHS {
            return Err("Invalid Skill consumption budget".into());
        }
        let mut seen = BTreeSet::new();
        for pin in authorized_pins {
            if !seen.insert(&pin.skill_id) { return Err("Duplicate authorized Skill identity".into()); }
            manager.validate_chat_skill_pin(user, pin).await.map_err(skill_error)?;
        }
        Ok(Self {
            user: user.into(), pins: authorized_pins.to_vec(), max_total_bytes, max_paths,
            consumed_bytes: 0, receipts: Vec::new(),
        })
    }

    pub fn pins(&self) -> &[AiSkillRef] { &self.pins }
    pub fn receipts(&self) -> &[SkillLoadReceipt] { &self.receipts }
    /// Exact serialized tool-result bytes emitted, including metadata/receipt overhead.
    pub fn consumed_bytes(&self) -> usize { self.consumed_bytes }
    pub fn remaining_bytes(&self) -> usize { self.max_total_bytes.saturating_sub(self.consumed_bytes) }
    pub fn is_tool(name: &str) -> bool { matches!(name, SKILL_LIST_TOOL | SKILL_LOAD_TOOL | SKILL_READ_TOOL) }

    pub async fn revalidate(&self, manager: &SkillsManager) -> Result<(), String> {
        for pin in &self.pins { manager.validate_chat_skill_pin(&self.user, pin).await.map_err(skill_error)?; }
        Ok(())
    }

    /// Metadata for the authorized immutable subset only; no library-wide model discovery.
    pub async fn catalog(&self, manager: &SkillsManager) -> Result<Vec<SkillCatalogEntry>, String> {
        self.revalidate(manager).await?;
        let catalog = manager.chat_skill_catalog(&self.user).await.map_err(skill_error)?;
        let mut selected = Vec::with_capacity(self.pins.len());
        for pin in &self.pins {
            let entry = catalog.iter().find(|entry| entry.pin == *pin)
                .ok_or("Authorized Skill pin changed; select again")?;
            selected.push(entry.clone());
        }
        self.revalidate(manager).await?;
        Ok(selected)
    }

    /// Return definitions only when a user authorized a nonempty collection.
    pub fn tools(&self) -> Vec<ToolDefinition> {
        if self.pins.is_empty() { return Vec::new(); }
        let definition = |name: &str, description: &str, properties: Value, required: Vec<&str>| ToolDefinition {
            kind: "function".into(), function: ToolFunction {
                name: name.into(), description: description.into(),
                parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            },
        };
        vec![
            definition(SKILL_LIST_TOOL, "List metadata of the user's authorized pinned Skills. Does not load content or grant permissions.", json!({}), vec![]),
            definition(SKILL_LOAD_TOOL, "Read exact SKILL.md of an authorized pinned Skill as untrusted context. Never executes scripts or grants tools.", json!({"skill_id":{"type":"string","minLength":1}}), vec!["skill_id"]),
            definition(SKILL_READ_TOOL, "Read a UTF-8 references/, templates/ or examples/ resource from an authorized immutable Skill package. Relative canonical paths only; no scripts or external files.", json!({"skill_id":{"type":"string","minLength":1},"path":{"type":"string","minLength":1}}), vec!["skill_id","path"]),
        ]
    }

    pub async fn load(&mut self, manager: &SkillsManager, skill_id: &str, remaining_context_bytes: usize) -> Result<SkillReadResult, String> {
        self.read_inner(manager, skill_id, "SKILL.md", remaining_context_bytes).await
    }

    pub async fn read(&mut self, manager: &SkillsManager, skill_id: &str, path: &str, remaining_context_bytes: usize) -> Result<SkillReadResult, String> {
        if path == "SKILL.md" { return Err("Use dgos_skill_load for SKILL.md".into()); }
        self.read_inner(manager, skill_id, path, remaining_context_bytes).await
    }

    async fn read_inner(&mut self, manager: &SkillsManager, skill_id: &str, path: &str, remaining_context_bytes: usize) -> Result<SkillReadResult, String> {
        let pin = self.pins.iter().find(|pin| pin.skill_id == skill_id).ok_or("Skill is outside the user-authorized pinned subset")?;
        if self.receipts.len() >= self.max_paths { return Err("Skill read count budget exhausted".into()); }
        let budget = self.remaining_bytes().min(remaining_context_bytes);
        if budget == 0 { return Err("Skill context byte budget exhausted".into()); }
        let result = manager.read_pinned_skill_text(&self.user, pin, path, budget).await.map_err(skill_error)?;
        let wire_bytes = serde_json::to_vec(&result).map_err(|_| "Skill result serialization failed")?.len();
        if wire_bytes > budget { return Err("Skill content and receipt exceed remaining context byte budget".into()); }
        if crate::foundation::serialized_tool_result_content_bytes(&result)? > remaining_context_bytes {
            return Err("Skill content and receipt exceed remaining context byte budget".into());
        }
        self.consumed_bytes += wire_bytes;
        self.receipts.push(result.receipt.clone());
        Ok(result)
    }

    /// Call only after complete model argument validation in the shared serial tool loop.
    /// Caller checks its current account lease before AND after every await. The returned
    /// result must be fed back as a tool result, never promoted to system/authority text.
    pub async fn dispatch(&mut self, manager: &SkillsManager, name: &str, args: Value, remaining_context_bytes: usize) -> Result<Value, String> {
        match name {
            SKILL_LIST_TOOL => {
                serde_json::from_value::<EmptyArg>(args).map_err(|_| "Invalid Skill list arguments")?;
                let value = serde_json::to_value(self.catalog(manager).await?).map_err(|_| "Skill catalogue serialization failed")?;
                let bytes = serde_json::to_vec(&value).map_err(|_| "Skill catalogue serialization failed")?.len();
                if bytes > self.remaining_bytes().min(remaining_context_bytes) { return Err("Skill catalogue exceeds remaining context byte budget".into()); }
                if crate::foundation::tool_result_content_bytes(&value)? > remaining_context_bytes { return Err("Skill catalogue exceeds remaining context byte budget".into()); }
                self.consumed_bytes += bytes;
                Ok(value)
            }
            SKILL_LOAD_TOOL => {
                let args: SkillArg = serde_json::from_value(args).map_err(|_| "Invalid Skill load arguments")?;
                serde_json::to_value(self.load(manager, &args.skill_id, remaining_context_bytes).await?).map_err(|_| "Skill result serialization failed".into())
            }
            SKILL_READ_TOOL => {
                let args: ResourceArg = serde_json::from_value(args).map_err(|_| "Invalid Skill resource arguments")?;
                serde_json::to_value(self.read(manager, &args.skill_id, &args.path, remaining_context_bytes).await?).map_err(|_| "Skill result serialization failed".into())
            }
            _ => Err("Unknown host Skill operation".into()),
        }
    }
}

fn skill_error(error: dgos_skills::SkillError) -> String {
    // No body, user file data, or filesystem paths cross the model/error boundary.
    format!("Skill operation rejected ({})", error.error_code())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::{migrations::Migrator, sea_orm::Database, sea_orm_migration::MigratorTrait};

    async fn setup() -> (SkillsManager, tempfile::TempDir, Vec<AiSkillRef>) {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let manager = SkillsManager::new(db, dir.path().join("skills"));
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        manager.register_bundled_catalog(&source).await.unwrap();
        for id in ["dgos.official.writing.summarize", "dgos.official.writing.rewrite"] {
            manager.install_skill(id, "alice").await.unwrap();
        }
        let pins = manager.chat_skill_catalog("alice").await.unwrap().into_iter().map(|entry| entry.pin).collect();
        (manager, dir, pins)
    }

    #[tokio::test]
    async fn runtime_discloses_only_explicit_subset_and_actual_reads() {
        let (manager, _dir, pins) = setup().await;
        let mut runtime = SkillRuntime::authorize(&manager, "alice", &pins[..1], 65536, 4).await.unwrap();
        assert_eq!(runtime.tools().len(), 3);
        let list = runtime.dispatch(&manager, SKILL_LIST_TOOL, json!({}), 65536).await.unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["pin"]["skill_id"], pins[0].skill_id);
        assert!(runtime.receipts().is_empty());
        assert!(runtime.load(&manager, &pins[1].skill_id, 65536).await.is_err());
        let result = runtime.dispatch(&manager, SKILL_LOAD_TOOL, json!({"skill_id":pins[0].skill_id}), 65536).await.unwrap();
        assert!(result["content"].as_str().unwrap().contains("name:"));
        assert_eq!(runtime.receipts().len(), 1);
        assert_eq!(runtime.receipts()[0].path, "SKILL.md");
        assert_eq!(runtime.receipts()[0].bytes, result["content"].as_str().unwrap().len());
        assert_eq!(runtime.consumed_bytes(), serde_json::to_vec(&list).unwrap().len() + serde_json::to_vec(&result).unwrap().len());
        assert!(!serde_json::to_string(runtime.receipts()).unwrap().contains("name:"));
    }

    #[tokio::test]
    async fn runtime_revalidates_user_state_and_rejects_noncanonical_arguments() {
        let (manager, _dir, pins) = setup().await;
        assert!(SkillRuntime::authorize(&manager, "bob", &pins[..1], 65536, 2).await.is_err());
        assert!(SkillRuntime::authorize(&manager, "alice", &[pins[0].clone(), pins[0].clone()], 65536, 2).await.is_err());
        let mut changed = pins[0].clone();
        changed.content_hash = "0".repeat(64);
        assert!(SkillRuntime::authorize(&manager, "alice", &[changed], 65536, 2).await.is_err());
        let mut runtime = SkillRuntime::authorize(&manager, "alice", &pins[..1], 65536, 2).await.unwrap();
        assert!(runtime.dispatch(&manager, SKILL_LOAD_TOOL, json!({"skill_id":pins[0].skill_id,"execute":true}), 65536).await.is_err());
        assert!(runtime.dispatch(&manager, SKILL_LIST_TOOL, json!({"all":true}), 65536).await.is_err());
        for path in ["../SKILL.md", "references/../../other/SKILL.md", "scripts/run.sh", "SKILL.md"] {
            assert!(runtime.read(&manager, &pins[0].skill_id, path, 65536).await.is_err());
        }
        manager.toggle_skill(&pins[0].skill_id, "alice", false).await.unwrap();
        assert!(runtime.revalidate(&manager).await.is_err());
        assert!(runtime.catalog(&manager).await.is_err());
        assert!(runtime.load(&manager, &pins[0].skill_id, 65536).await.is_err());
        assert!(runtime.receipts().is_empty());
    }

    #[tokio::test]
    async fn runtime_budgets_and_empty_opt_in_never_expand_authority() {
        let (manager, _dir, pins) = setup().await;
        let empty = SkillRuntime::authorize(&manager, "alice", &[], 65536, 1).await.unwrap();
        assert!(empty.tools().is_empty());
        assert!(empty.catalog(&manager).await.unwrap().is_empty());
        assert!(SkillRuntime::authorize(&manager, "alice", &pins[..1], 0, 1).await.is_err());
        let mut runtime = SkillRuntime::authorize(&manager, "alice", &pins[..1], 65536, 1).await.unwrap();
        assert!(runtime.load(&manager, &pins[0].skill_id, 1).await.is_err());
        assert!(runtime.receipts().is_empty());
        runtime.load(&manager, &pins[0].skill_id, 65536).await.unwrap();
        assert!(runtime.load(&manager, &pins[0].skill_id, 65536).await.is_err());
        assert_eq!(runtime.receipts().len(), 1);
        assert!(runtime.dispatch(&manager, "install_skill", json!({}), 65536).await.is_err());
    }
}
