//! Explicit project capability revisions. Persisted snapshots describe configuration, never authority.
use crate::{Engine, SessionOptions};
use dgos_db::development_repository::{DevelopmentRepository, Project};
use dgos_mcp::McpManager;
use dgos_providers::{CliProviderProtocol, ProviderManager};
use dgos_skills::SkillsManager;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

mod gateway;
pub use gateway::CapabilityConfirmation;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectProvider {
    Native,
    Dgos { provider_id: String, model_id: String, revision: i64 },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedSkill { pub id: String, pub version: String, pub hash: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedMcpTool { pub server_id: String, pub tool_name: String, pub schema_hash: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityConfiguration {
    pub engine: Engine,
    pub provider: ProjectProvider,
    pub skills: Vec<PinnedSkill>,
    pub mcp: Vec<SelectedMcpTool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityRevision {
    pub revision: i64,
    pub permission_epoch: i64,
    pub configuration: CapabilityConfiguration,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    pub revision: i64,
    pub permission_epoch: i64,
    pub engine: Engine,
    pub cli_version: String,
    pub configuration: CapabilityConfiguration,
    pub provider_status: String,
    pub skills_status: String,
    pub mcp_status: String,
    pub native_source: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveCapabilitiesInput {
    pub project_id: String,
    pub expected_revision: i64,
    pub configuration: CapabilityConfiguration,
}

/// Host-only process material. Not serializable or debug-printable: environment includes secrets.
#[derive(Default)]
pub(crate) struct CapabilityLaunch {
    pub env: BTreeMap<String, String>,
    pub codex_overrides: Vec<String>,
    pub skill_root: Option<String>,
    pub claude_plugin: Option<String>,
    pub mcp_url: Option<String>,
    pub mcp_bearer: Option<String>,
    pub model: Option<String>,
    pub managed: bool,
}

#[derive(Clone)]
pub struct ProjectCapabilities {
    pub(crate) repository: DevelopmentRepository,
    providers: ProviderManager,
    skills: Arc<RwLock<SkillsManager>>,
    pub(crate) mcp: Arc<McpManager>,
    root: PathBuf,
    bindings: Arc<Mutex<BTreeMap<String, Arc<gateway::Binding>>>>,
    confirmations: Arc<Mutex<BTreeMap<String, gateway::PendingConfirmation>>>,
}
impl ProjectCapabilities {
    pub fn new(repository: DevelopmentRepository, providers: ProviderManager, skills: Arc<RwLock<SkillsManager>>, mcp: Arc<McpManager>, root: PathBuf) -> Self {
        Self { repository, providers, skills, mcp, root, bindings: Arc::default(), confirmations: Arc::default() }
    }
    pub async fn read(&self, owner: &str, project: &str) -> Result<Option<CapabilityRevision>, String> {
        self.repository.project(owner, project).await?;
        self.repository.capability_revision(owner, project, None).await?.map(|row| {
            Ok(CapabilityRevision { revision: row.revision, permission_epoch: row.permission_epoch, configuration: serde_json::from_str(&row.configuration).map_err(|_| "storage_failed")?, created_at: row.created_at })
        }).transpose()
    }
    pub async fn save(&self, owner: &str, input: SaveCapabilitiesInput) -> Result<CapabilityRevision, String> {
        let project = self.repository.project(owner, &input.project_id).await?;
        if project.archived { return Err("project_archived".into()); }
        self.validate_configuration(owner, &input.configuration).await?;
        let configuration = serde_json::to_string(&input.configuration).map_err(|_| "capability_unavailable")?;
        let row = self.repository.save_capability_revision(owner, &input.project_id, input.expected_revision, &configuration).await?;
        Ok(CapabilityRevision { revision: row.revision, permission_epoch: row.permission_epoch, configuration: input.configuration, created_at: row.created_at })
    }
    async fn validate_configuration(&self, owner: &str, configuration: &CapabilityConfiguration) -> Result<(), String> {
        if configuration.skills.len() > 32 || configuration.mcp.len() > 128 { return Err("capability_unavailable".into()); }
        if let ProjectProvider::Dgos { provider_id, model_id, revision } = &configuration.provider {
            let launch = self.providers.resolve_cli_provider_launch(provider_id, model_id, owner, *revision).await.map_err(|_| "provider_unavailable")?;
            let compatible = matches!((configuration.engine, launch.metadata().protocol_candidate), (Engine::Codex, CliProviderProtocol::OpenAiResponses) | (Engine::Claude, CliProviderProtocol::AnthropicMessages));
            if !compatible { return Err("provider_incompatible".into()); }
            if configuration.engine == Engine::Codex && launch.metadata().auth_style != dgos_providers::AuthStyle::Bearer { return Err("provider_incompatible".into()); }
        }
        let skills = self.skills.read().await;
        let mut ids = std::collections::BTreeSet::new();
        for skill in &configuration.skills {
            if !ids.insert(&skill.id) { return Err("skill_unavailable".into()); }
            skills.load_pinned_package(owner, &skill.id, &skill.version, &skill.hash).await.map_err(|_| "skill_unavailable")?;
        }
        let mut tools = std::collections::BTreeSet::new();
        for tool in &configuration.mcp {
            if !tools.insert((&tool.server_id, &tool.tool_name)) { return Err("capability_unavailable".into()); }
            let current = self.mcp.list_tools(owner, &tool.server_id).await.map_err(|_| "mcp_unavailable")?;
            if !current.iter().any(|t| t.tool_name == tool.tool_name && t.schema_hash == tool.schema_hash) { return Err("schema_changed".into()); }
        }
        Ok(())
    }
    pub async fn catalog(&self, owner: &str) -> Result<serde_json::Value, String> {
        let mut providers = Vec::new();
        for provider in self.providers.list_providers(owner).await.map_err(|_| "provider_unavailable")? {
            let models = self.providers.list_models(&provider.id, owner).await.map_err(|_| "provider_unavailable")?;
            providers.push(serde_json::json!({"id":provider.id,"name":provider.name,"provider_type":provider.provider_type,"config_revision":provider.config_revision,"enabled":provider.enabled,"models":models.into_iter().map(|m| serde_json::json!({"id":m.model_id,"name":m.model_name,"model_type":m.model_type})).collect::<Vec<_>>()}));
        }
        let manager = self.skills.read().await;
        let mut skills = Vec::new();
        for (skill, hash) in manager.installed_content_pins(owner).await.map_err(|_| "skill_unavailable")? {
            skills.push(serde_json::json!({"id":skill.skill_id,"name":skill.name,"version":skill.version,"hash":hash,"enabled":skill.enabled}));
        }
        let mut mcp = Vec::new();
        for server in self.mcp.list_servers(owner).await.map_err(|_| "mcp_unavailable")? {
            for tool in self.mcp.list_tools(owner, &server.id).await.map_err(|_| "mcp_unavailable")? { mcp.push(serde_json::json!({"server_id":server.id,"server_name":server.name,"tool_name":tool.tool_name,"schema_hash":tool.schema_hash,"input_schema":tool.input_schema})); }
        }
        Ok(serde_json::json!({"providers":providers,"skills":skills,"mcp":mcp}))
    }
    pub async fn revoke(&self, owner: &str, project: &str) -> Result<i64, String> {
        let epoch = self.repository.revoke_project_capabilities(owner, project).await?;
        for binding in self.bindings.lock().await.values().filter(|b| b.owner == owner && b.project_id == project) { binding.stop.cancel(); }
        self.confirmations.lock().await.retain(|_, pending| pending.prompt.project_id != project || pending.owner != owner);
        Ok(epoch)
    }
    pub(crate) async fn prepare(&self, owner: &str, session: &str, task: &str, project: &Project, engine: Engine, options: &SessionOptions, revision: Option<i64>, restoring: bool, lease: CancellationToken) -> Result<(CapabilitySnapshot, CapabilityLaunch), String> {
        let saved = if restoring { self.repository.read_session_capabilities(owner, session).await? } else { None };
        if revision.is_some_and(|r| r < 0) { return Err("capability_unavailable".into()); }
        if restoring && saved.is_none() && revision.is_some_and(|r| r > 0) { return Err("capability_conflict".into()); }
        let epoch = self.repository.capability_epoch(owner, &project.id).await?;
        let (revision, configuration, previous_version) = if let Some(saved) = saved {
            if saved.permission_epoch != epoch { return Err("capability_revoked".into()); }
            let snapshot: CapabilitySnapshot = serde_json::from_str(&saved.snapshot).map_err(|_| "storage_failed")?;
            if revision.is_some_and(|r| r != snapshot.revision) { return Err("capability_conflict".into()); }
            (snapshot.revision, snapshot.configuration, Some(snapshot.cli_version))
        } else if let Some(revision) = revision.filter(|r| *r > 0) {
            let row = self.repository.capability_revision(owner, &project.id, Some(revision)).await?.ok_or("capability_unavailable")?;
            (revision, serde_json::from_str(&row.configuration).map_err(|_| "storage_failed")?, None)
        } else {
            (0, CapabilityConfiguration { engine, provider: ProjectProvider::Native, skills: Vec::new(), mcp: Vec::new() }, None)
        };
        if configuration.engine != engine { return Err("provider_incompatible".into()); }
        let executable = crate::session::resolve_executable(match engine { Engine::Codex => "codex", Engine::Claude => "claude" }, options.executable.as_deref())?;
        let output = tokio::time::timeout(std::time::Duration::from_secs(10), tokio::process::Command::new(executable).arg("--version").kill_on_drop(true).output()).await.map_err(|_| "adapter_unavailable")?.map_err(|_| "adapter_unavailable")?;
        if !output.status.success() || output.stdout.len() > 4096 { return Err("adapter_unavailable".into()); }
        let cli_version = String::from_utf8(output.stdout).map_err(|_| "adapter_unavailable")?.trim().to_owned();
        if previous_version.is_some_and(|version| version != cli_version) { return Err("cli_version_changed".into()); }
        self.validate_configuration(owner, &configuration).await?;
        let mut launch = CapabilityLaunch { managed: revision > 0, ..Default::default() };
        if let ProjectProvider::Dgos { provider_id, model_id, revision } = &configuration.provider {
            let provider = self.providers.resolve_cli_provider_launch(provider_id, model_id, owner, *revision).await.map_err(|_| "provider_unavailable")?;
            let meta = provider.metadata();
            launch.model = Some(model_id.clone());
            match engine {
                Engine::Codex => {
                    launch.env.insert("DGOS_PROVIDER_KEY".into(), provider.credential().expose().into());
                    launch.codex_overrides.extend(["model_provider=\"dgos_project\"".into(), "model_providers.dgos_project.name=\"DG-OS project\"".into(), format!("model_providers.dgos_project.base_url={}", serde_json::to_string(&meta.base_url).map_err(|_| "provider_unavailable")?), "model_providers.dgos_project.wire_api=\"responses\"".into(), "model_providers.dgos_project.requires_openai_auth=false".into(), "model_providers.dgos_project.env_key=\"DGOS_PROVIDER_KEY\"".into()]);
                }
                Engine::Claude => {
                    launch.env.insert("ANTHROPIC_BASE_URL".into(), meta.base_url.clone());
                    launch.env.insert("ANTHROPIC_AUTH_TOKEN".into(), if meta.auth_style.sends_bearer() { provider.credential().expose().into() } else { String::new() });
                    launch.env.insert("ANTHROPIC_API_KEY".into(), if meta.auth_style.sends_x_api_key() { provider.credential().expose().into() } else { String::new() });
                    // Auxiliary model aliases must use the selected service/model too.
                    for key in ["ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_SMALL_FAST_MODEL"] { launch.env.insert(key.into(), model_id.clone()); }
                }
            }
        }
        let snapshot = CapabilitySnapshot { revision, permission_epoch: epoch, engine, cli_version, configuration, provider_status: if revision == 0 { "native_unresolved" } else { "configured_unverified_combination" }.into(), skills_status: "prepared_not_execution_authority; scripts_hooks_unsupported".into(), mcp_status: if revision == 0 { "native_unresolved_not_host_audited" } else { "host_gateway; native_mcp_isolated" }.into(), native_source: "Native instructions, Shell and Hooks retain CLI trust/permissions; not constrained by DG-OS MCP policy".into() };
        self.publish_skills(owner, session, &snapshot, &mut launch).await?;
        let binding = Arc::new(gateway::Binding::new(owner, session, task, &project.id, snapshot.clone(), lease));
        if launch.managed {
            let (url, bearer) = self.start_gateway(binding.clone()).await?;
            launch.mcp_url = Some(url.clone()); launch.mcp_bearer = Some(bearer.clone());
            launch.env.insert("DGOS_MCP_BEARER".into(), bearer);
            launch.codex_overrides.push(format!("mcp_servers={{dgos_project={{url={},bearer_token_env_var=\"DGOS_MCP_BEARER\"}}}}", serde_json::to_string(&url).map_err(|_| "mcp_unavailable")?));
        }
        if let Some(old) = self.bindings.lock().await.insert(session.into(), binding) { old.stop.cancel(); }
        Ok((snapshot, launch))
    }
    async fn publish_skills(&self, owner: &str, session: &str, snapshot: &CapabilitySnapshot, launch: &mut CapabilityLaunch) -> Result<(), String> {
        if snapshot.configuration.skills.is_empty() { return Ok(()); }
        let root = self.root.join(uuid::Uuid::new_v4().to_string()).join(session);
        tokio::fs::create_dir_all(&root).await.map_err(|_| "skill_unavailable")?;
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; tokio::fs::set_permissions(root.parent().ok_or("skill_unavailable")?, std::fs::Permissions::from_mode(0o700)).await.map_err(|_| "skill_unavailable")?; }
        let skill_root = root.join("skills");
        let manager = self.skills.read().await;
        for pin in &snapshot.configuration.skills {
            let package = manager.load_pinned_package(owner, &pin.id, &pin.version, &pin.hash).await.map_err(|_| "skill_unavailable")?;
            let dest = skill_root.join(&pin.id);
            for file in package.files() {
                let path = dest.join(&file.path);
                tokio::fs::create_dir_all(path.parent().ok_or("skill_unavailable")?).await.map_err(|_| "skill_unavailable")?;
                tokio::fs::write(path, &file.bytes).await.map_err(|_| "skill_unavailable")?;
            }
        }
        launch.skill_root = Some(skill_root.to_string_lossy().into_owned());
        tokio::fs::create_dir(root.join(".claude-plugin")).await.map_err(|_| "skill_unavailable")?;
        tokio::fs::write(root.join(".claude-plugin/plugin.json"), b"{\"name\":\"dgos-project\",\"version\":\"1.0.0\"}").await.map_err(|_| "skill_unavailable")?;
        launch.claude_plugin = Some(root.to_string_lossy().into_owned());
        Ok(())
    }
    pub async fn session_snapshot(&self, owner: &str, session: &str) -> Result<Option<CapabilitySnapshot>, String> {
        self.repository.read_session_capabilities(owner, session).await?.map(|row| serde_json::from_str(&row.snapshot).map_err(|_| "storage_failed".into())).transpose()
    }
    pub(crate) async fn shutdown(&self) {
        let ids: Vec<_> = self.bindings.lock().await.keys().cloned().collect();
        for id in ids { self.close_session(&id).await; }
    }
    pub(crate) async fn close_session(&self, session: &str) {
        if let Some(binding) = self.bindings.lock().await.remove(session) { binding.stop.cancel(); }
        self.confirmations.lock().await.retain(|_, pending| pending.prompt.session_id != session);
    }
    pub(crate) async fn begin_run(&self, owner: &str, session: &str, run: &str) -> Result<(), String> {
        if let Some(binding) = self.bindings.lock().await.get(session).cloned() {
            binding.authorize(self).await?;
            if binding.owner != owner { return Err("session_not_found".into()); }
            self.validate_configuration(owner, &binding.snapshot.configuration).await?;
            *binding.run.lock().await = Some((run.into(), CancellationToken::new()));
            let snapshot = serde_json::to_string(&binding.snapshot).map_err(|_| "storage_failed")?;
            self.repository.save_run_capabilities(owner, run, &snapshot).await?;
        }
        Ok(())
    }
    pub(crate) async fn end_run(&self, session: &str) {
        if let Some(binding) = self.bindings.lock().await.get(session) { if let Some((_, token)) = binding.run.lock().await.take() { token.cancel(); } }
        self.confirmations.lock().await.retain(|_, pending| pending.prompt.session_id != session);
    }
}
