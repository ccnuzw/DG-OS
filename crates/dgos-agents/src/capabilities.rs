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
    pub codex_provider_id: String,
    pub model: String,
    pub launch: CliProviderLaunch,
    pub lease: Arc<CliProviderLease>,
    pub binding: OpeningBinding,
    pub snapshot: CliProviderSnapshot,
    pub ownership: Arc<crate::process_ownership::OwnershipJournal>,
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
    process_directory: Option<Arc<std::path::PathBuf>>,
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
        Ok(Self { repository, providers, host_instance_id: uuid::Uuid::new_v4().to_string(), boot_identity: host_boot_identity(), process_directory: None, bindings: Arc::default(), changes: Arc::new(tokio::sync::Mutex::new(Some(changes))), change_tx: tx })
    }

    pub(crate) fn with_process_directory(mut self, directory: std::path::PathBuf) -> Self {
        self.process_directory = Some(Arc::new(directory));
        self
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
        // The catalog is configuration data. Only an actual launch probes the CLI.
        let mut providers = Vec::new();
        for provider in self.providers.list_providers(owner).await.map_err(|_| "provider_unavailable")? {
            let availability = provider_availability(engine, &provider.provider_type, &provider.base_url, provider.config.as_ref(), provider.enabled, true);
            let models = self.providers.list_models(&provider.id, owner).await.map_err(|_| "provider_unavailable")?;
            providers.push(json!({"id":provider.id,"name":provider.name,"provider_type":provider.provider_type,"config_revision":provider.config_revision,"enabled":provider.enabled,"availability":availability,"models":models.into_iter().filter(|m| matches!(m.model_type.as_str(), "text-generation" | "chat")).map(|m| json!({"id":m.model_id,"name":m.model_name,"model_type":m.model_type})).collect::<Vec<_>>()}));
        }
        Ok(json!({"project_id":project_id,"engine":engine,"providers":providers}))
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
        let mut evidence = serde_json::Map::new();
        let codex_provider_id = if engine == Engine::Codex {
            if options.resume_id.is_some() {
                let SessionConfigurationView::ProviderBound { snapshot, .. } = self.read_session(owner, session_id).await? else { return Err("legacy_session_read_only".into()); };
                evidence = snapshot.evidence;
                // Native threads cannot change provider identity. Pre-route snapshots used
                // this fixed name; their effective auth must still pass the same readback.
                let saved = match evidence.get("codex_provider_id") {
                    None => "dgos_project",
                    Some(Value::String(id)) if id == "dgos_project" || id.strip_prefix("dgos_").is_some_and(|suffix| suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())) => id,
                    _ => return Err("provider_binding_unconfirmed".into()),
                };
                saved.to_owned()
            } else {
                format!("dgos_{}", binding_id.replace('-', ""))
            }
        } else { String::new() };
        if engine == Engine::Codex { evidence.insert("codex_provider_id".into(), json!(codex_provider_id)); }
        let directory = self.process_directory.as_ref().ok_or("storage_failed")?;
        let ownership = crate::process_ownership::OwnershipJournal::reserve(directory, &binding_id)?;
        let binding = OpeningBinding { binding_id: binding_id.clone(), project_id: project.id.clone(), task_id: task_id.into(), revision, permission_epoch: row.permission_epoch, provider_id: provider_ref.provider_id.clone(), model_id: provider_ref.model_id.clone(), provider_revision: provider_ref.provider_revision, host_instance_id: self.host_instance_id.clone(), boot_identity: self.boot_identity.clone(), request_digest: request_digest(session_id, engine, task_id, &project.id, options, revision, &provider_ref)? };
        let snapshot = CliProviderSnapshot { binding_id: binding_id.clone(), engine: engine_name(engine).into(), provider_id: provider_ref.provider_id.clone(), model_id: provider_ref.model_id.clone(), provider_revision: provider_ref.provider_revision, revision, permission_epoch: row.permission_epoch, evidence };
        let session = StoredAgentSession { id: session_id.into(), owner: owner.into(), engine: engine_name(engine).into(), native_id: options.resume_id.clone(), workspace: options.workspace.clone(), executable: options.executable.clone(), model: Some(provider_ref.model_id.clone()), project_id: Some(project.id.clone()), task_id: Some(task_id.into()) };
        self.repository.reserve_cli_session(owner, &session, &binding, &snapshot).await?;
        let provider_binding = CliProviderBinding { binding_id, owner: owner.into(), session_id: session_id.into(), host_instance_id: self.host_instance_id.clone(), provider_id: provider_ref.provider_id.clone(), model_id: provider_ref.model_id.clone(), provider_revision: provider_ref.provider_revision };
        let prepared = async {
            let lease = Arc::new(self.providers.register_cli_opening(provider_binding, account_lease).await.map_err(map_provider_error)?);
            self.bindings.lock().unwrap_or_else(|e| e.into_inner()).insert(binding.binding_id.clone(), lease.cancellation_token());
            let launch = self.providers.resolve_cli_provider_for_lease(&lease, engine_protocol(engine)).await.map_err(map_provider_error)?;
            let (env, codex_overrides) = provider_material(engine, &launch, &codex_provider_id)?;
            Ok(ProviderLaunch { env, codex_overrides, codex_provider_id, model: provider_ref.model_id, launch, lease, binding: binding.clone(), snapshot, ownership })
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
        if view.legacy_read_only || view.snapshot.is_none() && view.legacy_history.is_some() {
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
        self.repository.record_cli_cleanup(owner, session, &binding.binding_id, &binding.host_instance_id,
            &dgos_db::development_repository::CleanupObservation { binding_id: binding.binding_id.clone(), state: if reaped { "closed" } else { "unknown" }.into(), proof: if !reaped { "" } else if executed { "owned_exit" } else { "no_execution" }.into(), reason: (!reaped).then(|| "cleanup_pending".into()) }).await?;
        if reaped { self.bindings.lock().unwrap_or_else(|e| e.into_inner()).remove(&binding.binding_id); }
        Ok(())
    }

    pub(crate) fn boot_identity(&self) -> Option<String> { self.boot_identity.clone() }

    pub(crate) async fn recheck_cleanup(&self) -> Result<u64, String> {
        let mut changed = if let Some(boot_identity) = host_boot_identity() {
            self.repository.recheck_cli_cleanup(&dgos_db::development_repository::HostBootObservation { boot_identity }).await?
        } else { 0 };
        let Some(directory) = &self.process_directory else { return Ok(changed); };
        for binding in self.repository.pending_cli_processes().await? {
            let Some(host_instance_id) = binding.host_instance_id else { continue; };
            let root = directory.clone();
            let id = binding.binding_id.clone();
            let observed = tokio::task::spawn_blocking(move || crate::process_ownership::observe(&root, &id)).await.map_err(|_| "storage_failed")??;
            let proof = match observed {
                crate::process_ownership::Observation::NotStarted => "no_execution",
                crate::process_ownership::Observation::Stopped => "owned_exit",
                crate::process_ownership::Observation::Running | crate::process_ownership::Observation::Unavailable => continue,
            };
            let target = dgos_db::development_repository::CleanupTarget { owner: binding.owner, session_id: binding.session_id, binding_id: binding.binding_id, host_instance_id };
            changed += u64::from(self.repository.settle_cli_process_cleanup(&target, proof).await?);
        }
        Ok(changed)
    }

    pub(crate) async fn recover(&self) -> Result<(), String> {
        self.repository.recover_cli_bindings(&self.host_instance_id, self.boot_identity.as_deref()).await?;
        self.recheck_cleanup().await?;
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
    providers.resolve_cli_provider_launch(&configuration.provider.provider_id, &configuration.provider.model_id, owner, configuration.provider.provider_revision, engine_protocol(configuration.engine)).await.map_err(map_provider_error)?;
    Ok(())
}

fn engine_protocol(engine: Engine) -> CliProviderProtocol {
    match engine { Engine::Codex => CliProviderProtocol::OpenAiResponses, Engine::Claude => CliProviderProtocol::AnthropicMessages }
}

impl CliProviders {
    async fn resolve_provider(&self, owner: &str, configuration: &CliProviderConfiguration) -> Result<(), String> { resolve_provider(owner, &self.providers, configuration).await }
}

fn provider_availability(engine: Engine, provider_type: &str, base_url: &str, config: Option<&Value>, enabled: bool, cli_available: bool) -> Value {
    if !enabled { return json!({"available": false, "reason": "provider_unavailable"}); }
    match dgos_providers::resolve_cli_provider_route(provider_type, base_url, config, engine_protocol(engine)) {
        Ok(route) => json!({"available": cli_available, "reason": if cli_available { None } else { Some("cli_incompatible") }, "protocol": match engine { Engine::Codex => "responses", Engine::Claude => "messages" }, "declared": route.declared, "verified": false}),
        Err(error) => json!({"available": false, "reason": map_provider_error(error)}),
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;

    fn reason(value: Value) -> Option<String> { value["reason"].as_str().map(str::to_owned) }

    #[test]
    fn provider_availability_uses_only_local_engine_policy() {
        assert_eq!(reason(provider_availability(Engine::Codex, "openai_compatible", "https://api.example/v1", None, true, true)), None);
        assert_eq!(reason(provider_availability(Engine::Codex, "anthropic", "https://api.example", None, true, true)).as_deref(), Some("provider_protocol_unsupported"));
        assert_eq!(reason(provider_availability(Engine::Codex, "openai_compatible", "https://api.example/v1", Some(&json!({"auth_style":"x-api-key"})), true, true)).as_deref(), Some("provider_auth_unsupported"));
        assert_eq!(reason(provider_availability(Engine::Claude, "anthropic", "https://api.example", None, true, false)).as_deref(), Some("cli_incompatible"));
        assert_eq!(reason(provider_availability(Engine::Claude, "anthropic", "https://api.example", None, false, true)).as_deref(), Some("provider_unavailable"));
        assert_eq!(reason(provider_availability(Engine::Claude, "anthropic", "invalid", None, true, true)).as_deref(), Some("provider_route_invalid"));
    }

    #[test]
    fn explicit_routes_override_candidates_without_claiming_verification() {
        let config = json!({"cli_routes":{"responses":{"enabled":false},"messages":{"enabled":true,"base_url":"https://messages.example/v1"}}});
        assert_eq!(reason(provider_availability(Engine::Codex, "openai_compatible", "https://chat.example/v1", Some(&config), true, true)).as_deref(), Some("provider_protocol_unsupported"));
        let route = provider_availability(Engine::Claude, "openai_compatible", "https://chat.example/v1", Some(&config), true, true);
        assert_eq!(route["available"], true);
        assert_eq!(route["protocol"], "messages");
        assert_eq!(route["declared"], true);
        assert_eq!(route["verified"], false);
    }
}

fn provider_material(engine: Engine, launch: &CliProviderLaunch, codex_provider_id: &str) -> Result<(BTreeMap<String,String>, Vec<String>), String> {
    let metadata = launch.metadata();
    let key = launch.credential().expose().to_owned();
    let mut env = BTreeMap::new();
    let mut overrides = Vec::new();
    match engine {
        Engine::Codex => {
            env.insert("DGOS_PROVIDER_KEY".into(), key);
            // A session-owned namespace prevents unrelated native tables from merging
            // credentials into our route. Resume keeps it; effective auth is always checked.
            overrides.extend([format!("model_provider=\"{codex_provider_id}\""), format!("model_providers.{codex_provider_id}.name=\"DG-OS\""), format!("model_providers.{codex_provider_id}.base_url={}", serde_json::to_string(&metadata.base_url).map_err(|_| "provider_route_invalid")?), format!("model_providers.{codex_provider_id}.wire_api=\"responses\""), format!("model_providers.{codex_provider_id}.requires_openai_auth=false"), format!("model_providers.{codex_provider_id}.env_key=\"DGOS_PROVIDER_KEY\"")]);
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

fn map_provider_error(error: dgos_types::DgosError) -> String {
    use dgos_types::DgosError;
    match error {
        DgosError::CredentialUnavailable | DgosError::CredentialRequired | DgosError::InvalidApiKey => "credential_unavailable",
        DgosError::ProviderConfigurationChanged => "provider_revision_changed",
        DgosError::CliProviderProtocolUnsupported | DgosError::UnsupportedProviderType => "provider_protocol_unsupported",
        DgosError::CliProviderAuthUnsupported => "provider_auth_unsupported",
        DgosError::CliProviderRouteInvalid | DgosError::InsecureEndpoint => "provider_route_invalid",
        DgosError::RequestNotSupported => "provider_model_unsupported",
        _ => "provider_unavailable",
    }.into()
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
