//! Provider-bound CLI execution. Legacy snapshots are display-only data.
use crate::{Engine, SessionOptions};
use dgos_db::development_repository::{
    CliProviderSnapshot, DevelopmentRepository, OpeningBinding,
    Project, StoredAgentSession,
};
use dgos_providers::{
    CliProviderBinding, CliProviderLease, CliProviderLaunch, CliProviderProtocol, ProviderManager,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::{BTreeMap, HashMap}, sync::{Arc, Mutex}};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliProviderRef {
    pub provider_id: String,
    pub model_id: String,
    pub provider_revision: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliProviderConfiguration {
    pub engine: Engine,
    pub provider: CliProviderRef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliProviderRevision {
    pub revision: i64,
    pub permission_epoch: i64,
    pub configuration: CliProviderConfiguration,
    pub created_at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveCliProviderInput {
    pub project_id: String,
    pub expected_revision: i64,
    pub configuration: CliProviderConfiguration,
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum SessionConfigurationView {
    LegacyHistory { history: Option<Value>, read_only: bool },
    ProviderBound { snapshot: CliProviderSnapshot, read_only: bool },
}

/// Secret-bearing launch material. Never serialize, log or retain after synchronous spawn.
pub(crate) struct ProviderLaunch {
    pub env: BTreeMap<String, String>,
    pub codex_overrides: Vec<String>,
    pub model: String,
    pub launch: CliProviderLaunch,
    pub lease: Arc<CliProviderLease>,
    pub binding: OpeningBinding,
    pub snapshot: CliProviderSnapshot,
}

pub struct SessionCreate {
    pub options: SessionOptions,
    pub project_revision: i64,
    pub provider: CliProviderRef,
}

#[derive(Clone)]
pub struct CliProviders {
    repository: DevelopmentRepository,
    providers: ProviderManager,
    host_instance_id: String,
    boot_identity: Option<String>,
    bindings: Arc<Mutex<HashMap<String, CancellationToken>>>,
    changes: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<bool>>>>,
    change_tx: tokio::sync::mpsc::UnboundedSender<ChangeMessage>,
}

pub(crate) fn request_digest(session_id: &str, engine: Engine, task_id: &str, project_id: &str, options: &SessionOptions, revision: i64, provider: &CliProviderRef) -> Result<String, String> {
    let input = (session_id, engine, task_id, project_id, revision, provider, &options.workspace, &options.executable, &options.resume_id, &options.model, options.trust_workspace);
    Ok(format!("{:x}", sha2::Sha256::digest(serde_json::to_vec(&input).map_err(|_| "invalid_request")?)))
}

impl CliProviders {
    pub fn new(repository: DevelopmentRepository, providers: ProviderManager) -> Result<Self, String> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ChangeMessage>();
        providers.install_cli_change_sink(Arc::new(ChangeSink(tx.clone()))).map_err(map_provider_error)?;
        let repo = repository.clone();
        let changes = tokio::spawn(async move {
            let mut persisted = true;
            while let Some(message) = rx.recv().await {
                let ChangeMessage::Bindings(targets) = message else { break; };
                for target in targets {
                    let target = dgos_db::development_repository::CleanupTarget { owner: target.owner, session_id: target.session_id, binding_id: target.binding_id, host_instance_id: target.host_instance_id };
                    if repo.revoke_cli_binding(&target).await.is_err() { persisted = false; tracing::error!("CLI binding revocation persistence failed"); }
                }
            }
            persisted
        });
        Ok(Self { repository, providers, host_instance_id: uuid::Uuid::new_v4().to_string(), boot_identity: host_boot_identity(), bindings: Arc::default(), changes: Arc::new(tokio::sync::Mutex::new(Some(changes))), change_tx: tx })
    }

    pub async fn read(&self, owner: &str, project_id: &str) -> Result<Option<CliProviderRevision>, String> {
        self.repository.project(owner, project_id).await?;
        self.repository.cli_provider_revision(owner, project_id, None).await?.map(|row| {
            let configuration = decode_configuration(&row.configuration)?;
            Ok(CliProviderRevision { revision: row.revision, permission_epoch: row.permission_epoch, configuration, created_at: row.created_at })
        }).transpose()
    }

    pub async fn save(&self, owner: &str, input: SaveCliProviderInput) -> Result<CliProviderRevision, String> {
        if input.configuration.provider.provider_revision < 0 { return Err("provider_required".into()); }
        let project = self.repository.project(owner, &input.project_id).await?;
        if project.archived { return Err("project_archived".into()); }
        self.resolve_provider(owner, &input.configuration).await?;
        let row = self.repository.save_cli_provider_revision(owner, &input.project_id, input.expected_revision, &input.configuration.into()).await?;
        let configuration = decode_configuration(&row.configuration)?;
        Ok(CliProviderRevision { revision: row.revision, permission_epoch: row.permission_epoch, configuration, created_at: row.created_at })
    }

    pub async fn catalog(&self, owner: &str, project_id: &str, engine: Engine) -> Result<Value, String> {
        self.repository.project(owner, project_id).await?;
        let cli = crate::cli::probe(match engine { Engine::Codex => crate::cli::CliTool::Codex, Engine::Claude => crate::cli::CliTool::Claude }, None).await;
        let cli_available = cli.startup_error().is_none();
        let mut providers = Vec::new();
        for provider in self.providers.list_providers(owner).await.map_err(|_| "provider_unavailable")? {
            let mut availability = provider_availability(engine, &provider.provider_type, &provider.base_url, provider.config.as_ref(), provider.enabled, cli_available);
            if engine == Engine::Codex && availability["reason"] == "cli_incompatible" {
                availability["reason"] = json!(cli.startup_error());
            }
            let models = self.providers.list_models(&provider.id, owner).await.map_err(|_| "provider_unavailable")?;
            providers.push(json!({"id":provider.id,"name":provider.name,"config_revision":provider.config_revision,"enabled":provider.enabled,"availability":availability,"models":models.into_iter().filter(|m| matches!(m.model_type.as_str(), "text-generation" | "chat")).map(|m| json!({"id":m.model_id,"name":m.model_name,"model_type":m.model_type})).collect::<Vec<_>>()}));
        }
        Ok(json!({"project_id":project_id,"engine":engine,"providers":providers,"cli":cli}))
    }

    pub async fn revoke(&self, owner: &str, project_id: &str) -> Result<i64, String> {
        let targets = self.repository.revoke_cli_bindings(owner, &dgos_db::development_repository::CliRevocationScope::Project { project_id: project_id.into() }).await?;
        for target in targets {
            if target.host_instance_id == self.host_instance_id {
                if let Some(token) = self.bindings.lock().unwrap_or_else(|e| e.into_inner()).get(&target.binding_id) { token.cancel(); }
            }
        }
        self.repository.capability_epoch(owner, project_id).await
    }

    pub(crate) async fn prepare(&self, owner: &str, session_id: &str, task_id: &str, project: &Project, engine: Engine, options: &SessionOptions, revision: i64, provider_ref: CliProviderRef, account_lease: &CancellationToken) -> Result<ProviderLaunch, String> {
        if revision <= 0 || provider_ref.provider_revision < 0 { return Err("provider_required".into()); }
        let row = self.repository.cli_provider_revision(owner, &project.id, Some(revision)).await?.ok_or("capability_unavailable")?;
        let config = decode_configuration(&row.configuration)?;
        if config.engine != engine || config.provider.provider_id != provider_ref.provider_id || config.provider.model_id != provider_ref.model_id || config.provider.provider_revision != provider_ref.provider_revision { return Err("provider_revision_changed".into()); }
        let binding_id = uuid::Uuid::new_v4().to_string();
        let binding = OpeningBinding { binding_id: binding_id.clone(), project_id: project.id.clone(), task_id: task_id.into(), revision, permission_epoch: row.permission_epoch, provider_id: provider_ref.provider_id.clone(), model_id: provider_ref.model_id.clone(), provider_revision: provider_ref.provider_revision, host_instance_id: self.host_instance_id.clone(), boot_identity: self.boot_identity.clone(), request_digest: request_digest(session_id, engine, task_id, &project.id, options, revision, &provider_ref)? };
        let snapshot = CliProviderSnapshot { binding_id: binding_id.clone(), engine: engine_name(engine).into(), provider_id: provider_ref.provider_id.clone(), model_id: provider_ref.model_id.clone(), provider_revision: provider_ref.provider_revision, revision, permission_epoch: row.permission_epoch, evidence: serde_json::Map::new() };
        let session = StoredAgentSession { id: session_id.into(), owner: owner.into(), engine: engine_name(engine).into(), native_id: options.resume_id.clone(), workspace: options.workspace.clone(), executable: options.executable.clone(), model: Some(provider_ref.model_id.clone()), project_id: Some(project.id.clone()), task_id: Some(task_id.into()) };
        self.repository.reserve_cli_session(owner, &session, &binding, &snapshot).await?;
        let provider_binding = CliProviderBinding { binding_id, owner: owner.into(), session_id: session_id.into(), host_instance_id: self.host_instance_id.clone(), provider_id: provider_ref.provider_id.clone(), model_id: provider_ref.model_id.clone(), provider_revision: provider_ref.provider_revision };
        let prepared = async {
            let lease = Arc::new(self.providers.register_cli_opening(provider_binding, account_lease).await.map_err(map_provider_error)?);
            self.bindings.lock().unwrap_or_else(|e| e.into_inner()).insert(binding.binding_id.clone(), lease.cancellation_token());
            let launch = self.providers.resolve_cli_provider_for_lease(&lease).await.map_err(map_provider_error)?;
            if !matches!((engine, launch.metadata().protocol_candidate), (Engine::Codex, CliProviderProtocol::OpenAiResponses) | (Engine::Claude, CliProviderProtocol::AnthropicMessages)) { return Err("provider_incompatible".into()); }
            if engine == Engine::Codex && launch.metadata().auth_style != dgos_providers::AuthStyle::Bearer { return Err("provider_incompatible".into()); }
            let (env, codex_overrides) = provider_material(engine, &launch)?;
            Ok(ProviderLaunch { env, codex_overrides, model: provider_ref.model_id, launch, lease, binding: binding.clone(), snapshot })
        }.await;
        if prepared.is_err() && self.cleanup(owner, session_id, &binding, true, false).await.is_err() { return Err("cleanup_pending".into()); }
        prepared
    }

    pub(crate) async fn activate(&self, owner: &str, session: &str, launch: &ProviderLaunch, native_id: &str) -> Result<(), String> {
        self.providers.validate_cli_lease(&launch.lease).await.map_err(map_provider_error)?;
        self.repository.activate_cli_session(owner, session, &launch.binding.binding_id, &self.host_instance_id, native_id, launch.binding.permission_epoch).await?;
        Ok(())
    }

    pub async fn read_session(&self, owner: &str, session: &str) -> Result<SessionConfigurationView, String> {
        let view = self.repository.read_cli_session_configuration(owner, session).await?;
        if view.legacy_read_only {
            let history = view.legacy_history.map(|raw| serde_json::from_str(&raw).unwrap_or(Value::Null));
            return Ok(SessionConfigurationView::LegacyHistory { history, read_only: true });
        }
        let raw = view.snapshot.ok_or("storage_failed")?;
        let snapshot = serde_json::from_str(&raw).map_err(|_| "storage_failed")?;
        Ok(SessionConfigurationView::ProviderBound { snapshot, read_only: view.binding.is_none_or(|b| b.state != "active") })
    }

    pub(crate) async fn validate(&self, owner: &str, session: &str) -> Result<(), String> {
        let binding = self.repository.read_cli_session_configuration(owner, session).await?.binding.ok_or("legacy_session_read_only")?;
        if binding.execution_policy != "provider_bound" { return Err("legacy_session_read_only".into()); }
        if binding.state != "active" || binding.host_instance_id.as_deref() != Some(&self.host_instance_id) { return Err("capability_revoked".into()); }
        let live = self.bindings.lock().unwrap_or_else(|e| e.into_inner()).get(&binding.binding_id).is_some_and(|token| !token.is_cancelled());
        if !live { return Err("capability_revoked".into()); }
        let project = binding.project_id.as_deref().ok_or("project_not_found")?;
        let current = self.repository.cli_provider_revision(owner, project, None).await?.ok_or("capability_revoked")?;
        if Some(current.revision) != binding.revision || current.permission_epoch != binding.permission_epoch { return Err("capability_revoked".into()); }
        Ok(())
    }

    pub(crate) async fn cleanup(&self, owner: &str, session: &str, binding: &OpeningBinding, reaped: bool, executed: bool) -> Result<(), String> {
        if reaped { self.bindings.lock().unwrap_or_else(|e| e.into_inner()).remove(&binding.binding_id); }
        self.repository.record_cli_cleanup(owner, session, &binding.binding_id, &binding.host_instance_id,
            &dgos_db::development_repository::CleanupObservation { binding_id: binding.binding_id.clone(), state: if reaped { "closed" } else { "unknown" }.into(), proof: if !reaped { "" } else if executed { "owned_exit" } else { "no_execution" }.into(), reason: (!reaped).then(|| "cleanup_pending".into()) }).await?;
        Ok(())
    }

    pub(crate) fn boot_identity(&self) -> Option<String> { self.boot_identity.clone() }

    pub async fn recheck_cleanup(&self) -> Result<u64, String> {
        let boot_identity = host_boot_identity().ok_or("boot_identity_unavailable")?;
        self.repository.recheck_cli_cleanup(&dgos_db::development_repository::HostBootObservation { boot_identity }).await
    }

    pub(crate) async fn recover(&self) -> Result<(), String> {
        self.repository.recover_cli_bindings(&self.host_instance_id, self.boot_identity.as_deref()).await?;
        if self.boot_identity.is_some() { self.recheck_cleanup().await?; }
        Ok(())
    }

    pub async fn shutdown(&self) -> bool {
        let mutations = self.providers.shutdown_provider_mutations().await.is_ok();
        let mut changes = self.changes.lock().await;
        let persisted = if let Some(worker) = changes.take() {
            let sent = self.change_tx.send(ChangeMessage::Stop).is_ok();
            let joined = matches!(worker.await, Ok(true));
            sent && joined
        } else { true };
        mutations && persisted
    }

    pub(crate) fn cancel_all(&self) {
        for token in self.bindings.lock().unwrap_or_else(|e| e.into_inner()).values() { token.cancel(); }
    }

    pub(crate) fn providers(&self) -> &ProviderManager { &self.providers }
}

enum ChangeMessage { Bindings(Vec<CliProviderBinding>), Stop }
struct ChangeSink(tokio::sync::mpsc::UnboundedSender<ChangeMessage>);
impl dgos_providers::CliProviderChangeSink for ChangeSink {
    fn changed(&self, _: &dgos_providers::CliProviderChange, targets: &[CliProviderBinding]) {
        if !targets.is_empty() && self.0.send(ChangeMessage::Bindings(targets.to_vec())).is_err() { tracing::error!("CLI revocation receiver unavailable"); }
    }
}

fn decode_configuration(raw: &str) -> Result<CliProviderConfiguration, String> {
    let row: dgos_db::development_repository::CliProviderConfiguration = serde_json::from_str(raw).map_err(|_| "storage_failed")?;
    let engine = match row.engine.as_str() { "codex" => Engine::Codex, "claude" => Engine::Claude, _ => return Err("storage_failed".into()) };
    Ok(CliProviderConfiguration { engine, provider: CliProviderRef { provider_id: row.provider_id, model_id: row.model_id, provider_revision: row.provider_revision } })
}

impl From<CliProviderConfiguration> for dgos_db::development_repository::CliProviderConfiguration {
    fn from(value: CliProviderConfiguration) -> Self { Self { engine: engine_name(value.engine).into(), provider_id: value.provider.provider_id, model_id: value.provider.model_id, provider_revision: value.provider.provider_revision } }
}

async fn resolve_provider(owner: &str, providers: &ProviderManager, configuration: &CliProviderConfiguration) -> Result<(), String> {
    let launch = providers.resolve_cli_provider_launch(&configuration.provider.provider_id, &configuration.provider.model_id, owner, configuration.provider.provider_revision).await.map_err(map_provider_error)?;
    if !matches!((configuration.engine, launch.metadata().protocol_candidate), (Engine::Codex, CliProviderProtocol::OpenAiResponses) | (Engine::Claude, CliProviderProtocol::AnthropicMessages)) { return Err("provider_incompatible".into()); }
    if configuration.engine == Engine::Codex && launch.metadata().auth_style != dgos_providers::AuthStyle::Bearer { return Err("provider_incompatible".into()); }
    Ok(())
}

impl CliProviders {
    async fn resolve_provider(&self, owner: &str, configuration: &CliProviderConfiguration) -> Result<(), String> { resolve_provider(owner, &self.providers, configuration).await }
}

fn provider_availability(engine: Engine, provider_type: &str, base_url: &str, config: Option<&Value>, enabled: bool, cli_available: bool) -> Value {
    if !enabled { return json!({"available": false, "reason": "provider_unavailable"}); }
    let Some(candidate) = CliProviderProtocol::candidate_for_provider_type(provider_type) else {
        return json!({"available": false, "reason": "provider_incompatible"});
    };
    if !matches!((engine, candidate), (Engine::Codex, CliProviderProtocol::OpenAiResponses) | (Engine::Claude, CliProviderProtocol::AnthropicMessages)) {
        return json!({"available": false, "reason": "provider_incompatible"});
    }
    match dgos_providers::vendor::validate_provider_config(provider_type, base_url, config) {
        Ok(normalized) if engine != Engine::Codex || normalized.auth_override.unwrap_or(normalized.profile.auth) == dgos_providers::AuthStyle::Bearer => {
            if cli_available { json!({"available": true, "reason": null}) } else { json!({"available": false, "reason": "cli_incompatible"}) }
        }
        _ => json!({"available": false, "reason": "provider_incompatible"}),
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;

    fn reason(value: Value) -> Option<String> { value["reason"].as_str().map(str::to_owned) }

    #[test]
    fn provider_availability_uses_only_local_engine_policy() {
        assert_eq!(reason(provider_availability(Engine::Codex, "openai_compatible", "https://api.example/v1", None, true, true)), None);
        assert_eq!(reason(provider_availability(Engine::Codex, "anthropic", "https://api.example", None, true, true)).as_deref(), Some("provider_incompatible"));
        assert_eq!(reason(provider_availability(Engine::Codex, "openai_compatible", "https://api.example/v1", Some(&json!({"auth_style":"x-api-key"})), true, true)).as_deref(), Some("provider_incompatible"));
        assert_eq!(reason(provider_availability(Engine::Claude, "anthropic", "https://api.example", None, true, false)).as_deref(), Some("cli_incompatible"));
        assert_eq!(reason(provider_availability(Engine::Claude, "anthropic", "https://api.example", None, false, true)).as_deref(), Some("provider_unavailable"));
        assert_eq!(reason(provider_availability(Engine::Claude, "anthropic", "invalid", None, true, true)).as_deref(), Some("provider_incompatible"));
    }
}

fn provider_material(engine: Engine, launch: &CliProviderLaunch) -> Result<(BTreeMap<String,String>, Vec<String>), String> {
    let metadata = launch.metadata();
    let key = launch.credential().expose().to_owned();
    let mut env = BTreeMap::new();
    let mut overrides = Vec::new();
    match engine {
        Engine::Codex => {
            env.insert("DGOS_PROVIDER_KEY".into(), key);
            overrides.extend(["model_provider=\"dgos_project\"".into(), "model_providers.dgos_project.name=\"DG-OS\"".into(), format!("model_providers.dgos_project.base_url={}", serde_json::to_string(&metadata.base_url).map_err(|_| "provider_unavailable")?), "model_providers.dgos_project.wire_api=\"responses\"".into(), "model_providers.dgos_project.requires_openai_auth=false".into(), "model_providers.dgos_project.env_key=\"DGOS_PROVIDER_KEY\"".into()]);
        }
        Engine::Claude => {
            env.insert("ANTHROPIC_BASE_URL".into(), metadata.base_url.clone());
            env.insert("ANTHROPIC_AUTH_TOKEN".into(), if metadata.auth_style.sends_bearer() { key.clone() } else { String::new() });
            env.insert("ANTHROPIC_API_KEY".into(), if metadata.auth_style.sends_x_api_key() { key } else { String::new() });
            for name in ["ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_SMALL_FAST_MODEL"] { env.insert(name.into(), metadata.model.model_id.clone()); }
        }
    }
    Ok((env, overrides))
}

pub(crate) fn engine_name(engine: Engine) -> &'static str { match engine { Engine::Codex => "codex", Engine::Claude => "claude" } }

fn map_provider_error(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    if text.contains("Credential") { "credential_unavailable".into() }
    else if text.contains("ConfigurationChanged") { "provider_revision_changed".into() }
    else if text.contains("UnsupportedProviderType") || text.contains("RequestNotSupported") { "provider_incompatible".into() }
    else { "provider_unavailable".into() }
}

use sha2::Digest;

/// Includes machine identity: moving a database is never evidence of a reboot.
fn host_boot_identity() -> Option<String> {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut machine = [0u8; 16];
        let timeout = libc::timespec { tv_sec: 1, tv_nsec: 0 };
        if libc::gethostuuid(machine.as_mut_ptr(), &timeout) != 0 { return None; }
        let mut boot = [0u8; 128];
        let mut size = boot.len();
        if libc::sysctlbyname(c"kern.bootsessionuuid".as_ptr(), boot.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) != 0 { return None; }
        let boot = std::str::from_utf8(&boot[..size]).ok()?.trim_end_matches('\0');
        if boot.is_empty() { return None; }
        return Some(format!("{}:{boot}", uuid::Uuid::from_bytes(machine)));
    }
    #[cfg(target_os = "linux")]
    {
        let machine = std::fs::read_to_string("/etc/machine-id").ok()?;
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        if machine.trim().is_empty() || boot.trim().is_empty() { return None; }
        return Some(format!("{}:{}", machine.trim(), boot.trim()));
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    None
}
