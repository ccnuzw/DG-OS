use super::{dto::Selection, fail, Domain, NativeHost, Ticket};
use dgos_agents::{native_extensions::{self as core, mcp::{ConfigProjection, ProbeConfig}}, AgentManager};
use dgos_db::ActivityPermit;
use dgos_mcp::probe::{container_plan, ContainerCleanupPlan, ProbeConnection, ProbeEndpoint};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::{Path, PathBuf}, sync::{Arc, Mutex}, time::{Duration, SystemTime, UNIX_EPOCH}};
use tokio_util::sync::CancellationToken;

const REGISTRY: &str = "https://registry.modelcontextprotocol.io/v0/servers";
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |v| v.as_secs()) }
fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
#[derive(Default)]
pub struct McpCatalogState { inner: Arc<Mutex<State>>, workers: Mutex<Vec<tokio::task::JoinHandle<()>>> }
#[derive(Default)]
struct State { cache: HashMap<String, Cache>, probes: HashMap<String, ProbeRecord>, active: Option<String>, closing: bool }
#[derive(Clone)]
struct Cache { page: RegistryPage, etag: Option<String>, filter: String }
struct ProbeRecord { owner: Ticket, input: ProbeInput, snapshot: ProbeSnapshot, expected: String, cancel: CancellationToken, connection: Option<Arc<ProbeConnection>>, permit: Option<Arc<ActivityPermit>> }
impl McpCatalogState {
    pub(super) async fn shutdown(&self) -> bool {
        let workers = {
            let Ok(mut workers) = self.workers.lock() else { return false; };
            let Ok(mut state) = self.inner.lock() else { return false; };
            state.closing = true;
            for record in state.probes.values() { record.cancel.cancel(); }
            std::mem::take(&mut *workers)
        };
        let drained = async {
            for worker in workers { if worker.await.is_err() { return false; } }
            let unknown = {
                let Ok(state) = self.inner.lock() else { return false; };
                state.active.as_ref().and_then(|id| state.probes.get(id).and_then(|record| record.connection.clone().map(|connection| (id.clone(), connection))))
            };
            if let Some((id, connection)) = unknown {
                if !matches!(tokio::time::timeout(Duration::from_secs(2), connection.cleanup()).await, Ok(true)) { return false; }
                let Ok(mut state) = self.inner.lock() else { return false; };
                if state.active.as_deref() == Some(id.as_str()) {
                    state.active = None;
                    if let Some(record) = state.probes.get_mut(&id) { record.connection = None; record.permit = None; record.snapshot.cleanup = "confirmed".into(); }
                }
            }
            self.inner.lock().is_ok_and(|state| state.active.is_none())
        };
        tokio::time::timeout(Duration::from_secs(3), drained).await.unwrap_or(false)
    }
}
impl State {
    fn registry_cache(&self, key: &str, filter: &str, cursor: Option<&str>) -> Result<Option<Cache>, String> {
        let cached = self.cache.get(key).cloned();
        if cursor.is_some_and(|cursor| cached.is_none() && !self.cache.values().any(|cache| cache.filter == filter && cache.page.next_cursor.as_deref() == Some(cursor))) { return Err(fail("InvalidFormat")); }
        Ok(cached)
    }
    fn record_registry_result(&mut self, key: String, result: Result<Cache, String>, cached: Option<Cache>) -> Result<RegistryPage, String> {
        let cache = match result {
            Ok(cache) => cache,
            Err(error) => { let Some(mut cache) = cached else { return Err(error); }; cache.page.stale = true; cache.page.error = Some(error); cache },
        };
        if self.cache.len() >= 64 && !self.cache.contains_key(&key) { self.cache.clear(); }
        self.cache.insert(key, cache.clone());
        Ok(cache.page)
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrySearch { pub selection: Selection, pub search: String, pub cursor: Option<String>, pub refresh: bool }
#[derive(Clone, Serialize)]
pub struct RegistryPage { pub servers: Vec<Value>, pub next_cursor: Option<String>, pub fetched_at: u64, pub stale: bool, pub error: Option<String>, pub source: &'static str }
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionInput { pub selection: Selection, pub server_name: String }
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProbeSource { Existing { server_name: String }, Draft { draft: ProbeDraft } }
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeDraft {
    pub name: String, pub transport: String, pub program: String, pub args: Vec<String>, pub url: String,
    pub cwd: String, pub bearer_env: String, pub forwarded_env: Vec<String>, pub header_env: Vec<(String, String)>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeInput { pub selection: Selection, pub source: ProbeSource }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeOperation { pub probe_id: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeApproval { pub probe_id: String, pub approved_digest: String }
#[derive(Clone, Serialize)]
pub struct ProbeSnapshot {
    pub probe_id: String, pub selection: Selection, pub config: ConfigProjection, pub digest: String,
    pub probe_args: Vec<String>, pub cleanup_plan: Option<ContainerCleanupPlan>,
    pub executable: Option<String>, pub cwd: String, pub root: String, pub platform_identity: String,
    pub executable_identity: Option<String>, pub expires_at: u64, pub state: String, pub phase: String,
    pub cleanup: String, pub error: Option<String>, pub observed_at: Option<u64>, pub protocol_version: Option<String>,
    pub tools: Vec<Value>, pub schema_source: &'static str, pub may_download_execute: bool,
}
struct Prepared { config: ProbeConfig, endpoint: ProbeEndpoint, fingerprint: String, executable: Option<String>, executable_identity: Option<String>, cwd: String, root: String, platform_identity: String, probe_args: Vec<String>, cleanup_plan: Option<ContainerCleanupPlan> }
fn env(name: &str) -> Result<String, String> { std::env::var(name).map_err(|_| "credential_unavailable".to_owned()) }
fn expand(value: &str) -> Result<String, String> {
    match value.strip_prefix("${").and_then(|value| value.strip_suffix('}')) { Some(name) if valid_env(name) => env(name), _ if value.contains("${") => Err(fail("Unsupported")), _ => Ok(value.to_owned()) }
}
fn valid_env(name: &str) -> bool { !name.is_empty() && name.len() <= 128 && name.bytes().enumerate().all(|(i,b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())) }
fn binary(program: &str) -> Result<PathBuf, String> {
    let path = Path::new(program);
    let candidate = if path.is_absolute() { path.to_path_buf() } else {
        if path.components().count() != 1 { return Err(fail("UnsafePath")); }
        let search = core::dependencies::Environment::native_host().and_then(|environment| environment.path_value()).map_err(|e| e.to_string())?;
        std::env::split_paths(&search).map(|root| root.join(program)).find(|path| executable(path)).ok_or_else(|| fail("NotFound"))?
    };
    let path = std::fs::canonicalize(candidate).map_err(|_| fail("NotFound"))?;
    if !executable(&path) { return Err(fail("AccessDenied")); } Ok(path)
}
fn executable(path: &Path) -> bool {
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0) }
    #[cfg(not(unix))] { let _ = path; false }
}
fn platform_binary(target: core::Target, paths: &dgos_db::CliPaths) -> Result<PathBuf, String> {
    let (name, explicit) = match target { core::Target::Codex => ("codex", paths.codex.as_deref()), core::Target::Claude => ("claude", paths.claude.as_deref()) };
    let executable = dgos_agents::session::resolve_executable(name, explicit).map_err(|_| fail("NotFound"))?;
    std::fs::canonicalize(executable).map_err(|_| fail("NotFound"))
}
fn docker_program(configured: Option<&str>, resolved: &Path) -> bool {
    let name = |path: &Path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| matches!(name, "docker" | "docker.exe"));
    name(resolved) || configured.is_some_and(|configured| name(Path::new(configured)))
}
fn identity(path: &Path, directory: bool) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|_| fail("RootChanged"))?;
    if directory != metadata.is_dir() { return Err(fail("RootChanged")); }
    #[cfg(unix)] { use std::os::unix::fs::MetadataExt;
        if directory { return Ok(format!("{}:{}:{}", path.display(), metadata.dev(), metadata.ino())); }
        let executable = core::dependencies::VersionExecutable::capture(path).map_err(|e| e.to_string())?;
        let snapshot = serde_json::to_vec(&executable).map_err(|_| fail("Unavailable"))?;
        Ok(format!("{}:{}", hash(&snapshot), executable.digest))
    }
    #[cfg(not(unix))] { Err(fail("Unsupported")) }
}
fn source_config(api: &core::NativeExtensions, input: &ProbeInput) -> Result<ProbeConfig, String> {
    match &input.source {
        ProbeSource::Existing { server_name } => api.probe_mcp_config(input.selection.target, input.selection.scope.clone(), server_name).map_err(|e| e.to_string()),
        ProbeSource::Draft { draft } => {
            use core::mcp::{Field, ServerPatch, Transport};
            let transport = match draft.transport.as_str() { "Stdio" => Transport::Stdio, "Http" => Transport::Http, "Sse" if input.selection.target == core::Target::Claude => Transport::Sse, _ => return Err(fail("Unsupported")) };
            if (transport == Transport::Stdio && draft.program.is_empty()) || (transport != Transport::Stdio && draft.url.is_empty()) { return Err(fail("InvalidFormat")); }
            api.probe_binding(input.selection.target, &input.selection.scope).map_err(|e| e.to_string())?;
            core::mcp::probe_draft(input.selection.target, &input.selection.scope, draft.name.clone(), ServerPatch {
                transport: Field::Set(transport),
                program: if transport == Transport::Stdio { Field::Set(draft.program.clone()) } else { Field::Keep },
                args: if transport == Transport::Stdio { Field::Set(draft.args.clone()) } else { Field::Keep },
                url: if transport != Transport::Stdio { Field::Set(draft.url.clone()) } else { Field::Keep },
                cwd: if !draft.cwd.is_empty() { Field::Set(draft.cwd.clone()) } else { Field::Keep },
                bearer_env: if !draft.bearer_env.is_empty() { Field::Set(draft.bearer_env.clone()) } else { Field::Keep },
                forwarded_env: if !draft.forwarded_env.is_empty() { Field::Set(draft.forwarded_env.clone()) } else { Field::Keep },
                header_env: if !draft.header_env.is_empty() { Field::Set(draft.header_env.clone()) } else { Field::Keep },
            }).map_err(|e| e.to_string())
        }
    }
}
fn prepare(api: &core::NativeExtensions, input: &ProbeInput, root: &Path, nonce: &str, paths: &dgos_db::CliPaths) -> Result<Prepared, String> {
    let config = source_config(api, input)?;
    if config.projection.redacted { return Err(fail("Unsupported")); }
    if !config.credential_supported { return Err("credential_unavailable".into()); }
    let root = std::fs::canonicalize(root).map_err(|_| fail("RootChanged"))?;
    let cwd = config.projection.cwd.as_ref().map(PathBuf::from).unwrap_or_else(|| root.clone());
    let cwd = if cwd.is_absolute() { cwd } else { root.join(cwd) };
    let cwd = std::fs::canonicalize(cwd).map_err(|_| fail("RootChanged"))?;
    let cwd_identity = identity(&cwd, true)?; let root_identity = identity(&root, true)?;
    let platform = platform_binary(input.selection.target, paths)?;
    let platform_identity = identity(&platform, false)?;
    let mut environment = Vec::new();
    for (name, value) in &config.environment { environment.push((name.clone(), if input.selection.target == core::Target::Claude { expand(value)? } else { value.clone() })); }
    for name in &config.forwarded_env { environment.push((name.clone(), env(name)?)); }
    let mut headers = config.headers.clone();
    for (header, name) in &config.header_env { headers.push((header.clone(), env(name)?)); }
    if let Some(name) = &config.bearer_env { headers.push(("Authorization".into(), format!("Bearer {}", env(name)?))); }
    let mut inherited = Vec::new();
    for name in ["PATH", "HOME", "USERPROFILE", "SystemRoot", "TMPDIR", "TEMP", "LANG"] { if let Ok(value) = std::env::var(name) { inherited.push((name, value)); } }
    let mut probe_args = config.projection.args.clone(); let mut cleanup_plan = None;
    let (endpoint, executable, executable_identity) = match config.projection.transport {
        core::mcp::Transport::Stdio => {
            let executable = binary(config.projection.program.as_deref().ok_or_else(|| fail("InvalidFormat"))?)?;
            let binary_identity = identity(&executable, false)?;
            let container = if docker_program(config.projection.program.as_deref(), &executable) {
                let (effective, spec) = container_plan(&executable.to_string_lossy(), &probe_args, nonce, binary_identity.rsplit(':').next().ok_or_else(|| fail("Unavailable"))?.to_owned()).map_err(|_| "probe_docker_invocation_unsupported".to_owned())?;
                probe_args = effective; cleanup_plan = Some(spec.plan.clone()); Some(spec)
            } else { None };
            (ProbeEndpoint::Stdio { executable: executable.to_string_lossy().into_owned(), args: probe_args.clone(), cwd: cwd.to_string_lossy().into_owned(), environment: environment.clone(), container }, Some(executable.to_string_lossy().into_owned()), Some(binary_identity))
        }
        core::mcp::Transport::Http | core::mcp::Transport::Sse => (ProbeEndpoint::Http { url: config.projection.url.clone().ok_or_else(|| fail("InvalidFormat"))?, headers: headers.clone(), legacy_sse: config.projection.transport == core::mcp::Transport::Sse }, None, None),
        _ => return Err(fail("Unsupported")),
    };
    let fingerprint = hash(&serde_json::to_vec(&json!([nonce, config.projection.config_digest, environment, headers, inherited, cwd_identity, root_identity, platform_identity, executable_identity, input.selection, probe_args, cleanup_plan])).map_err(|_| fail("Unavailable"))?);
    Ok(Prepared { config, endpoint, fingerprint, executable, executable_identity, cwd: cwd.to_string_lossy().into_owned(), root: root.to_string_lossy().into_owned(), platform_identity: hash(platform_identity.as_bytes()), probe_args, cleanup_plan })
}
impl NativeHost {
    pub async fn search_mcp_registry(&self, agents: &AgentManager, input: RegistrySearch, permit: ActivityPermit, ticket: Ticket) -> Result<RegistryPage, String> {
        let search = input.search.trim();
        if search.len() > 200 || (!search.is_empty() && search.chars().count() < 3) || input.cursor.as_ref().is_some_and(|c| c.len() > 4096 || c.chars().any(char::is_control)) { return Err(fail("InvalidFormat")); }
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection.clone(), Domain::Mcp, false, permit.clone(), &ticket).await?;
        drop(admitted);
        let lease = self.library_lease(&ticket)?;
        let filter = serde_json::to_string(&(&ticket, &input.selection, search)).map_err(|_| fail("InvalidFormat"))?;
        let key = serde_json::to_string(&(&filter, &input.cursor)).map_err(|_| fail("InvalidFormat"))?;
        let cached = self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?.registry_cache(&key, &filter, input.cursor.as_deref())?;
        if !input.refresh { if let Some(mut cached) = cached.clone() { cached.page.stale = cached.page.error.is_some() || now().saturating_sub(cached.page.fetched_at) > 900; self.library_lease(&ticket)?; return Ok(cached.page); } }
        let fetch = async {
            let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(15)).build().map_err(|_| "registry_unavailable".to_owned())?;
            let mut request = client.get(REGISTRY).query(&[("limit", "30"), ("version", "latest"), ("search", search)]);
            if let Some(cursor) = &input.cursor { request = request.query(&[("cursor", cursor)]); }
            if let Some(etag) = cached.as_ref().and_then(|cache| cache.etag.as_ref()) { request = request.header(reqwest::header::IF_NONE_MATCH, etag); }
            let mut response = request.send().await.map_err(|_| "registry_offline".to_owned())?;
            if response.status().as_u16() == 304 { let mut cache = cached.clone().ok_or_else(|| "registry_invalid".to_owned())?; cache.page.fetched_at = now(); cache.page.stale = false; cache.page.error = None; return Ok(cache); }
            if !response.status().is_success() { return Err(match response.status().as_u16() { 429 => "registry_rate_limited", 401 | 403 => "registry_access_denied", _ => "registry_unavailable" }.to_owned()); }
            let etag = response.headers().get(reqwest::header::ETAG).and_then(|v| v.to_str().ok()).filter(|v| v.len() < 1024).map(str::to_owned);
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| "registry_offline".to_owned())? { if bytes.len() + chunk.len() > 4 * 1024 * 1024 { return Err("registry_limit".into()); } bytes.extend_from_slice(&chunk); }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| "registry_invalid".to_owned())?;
            let empty_servers = Vec::new();
            let servers = match value.get("servers") { Some(Value::Array(servers)) => servers, Some(Value::Null) => &empty_servers, _ => return Err("registry_invalid".into()) };
            if servers.len() > 30 || servers.iter().any(|entry| !valid_registry_entry(entry)) || value.pointer("/metadata/count").and_then(Value::as_u64) != Some(servers.len() as u64) { return Err("registry_invalid".into()); }
            let next_cursor = match value.pointer("/metadata/nextCursor") { None | Some(Value::Null) => None, Some(Value::String(cursor)) if cursor.is_empty() => None, Some(Value::String(cursor)) => Some(cursor.clone()), _ => return Err("registry_invalid".into()) };
            if next_cursor.as_ref().is_some_and(|v| v.len() > 4096 || v.chars().any(char::is_control) || input.cursor.as_ref() == Some(v)) { return Err("registry_invalid".into()); }
            Ok(Cache { page: RegistryPage { servers: servers.clone(), next_cursor, fetched_at: now(), stale: false, error: None, source: REGISTRY }, etag, filter: filter.clone() })
        };
        let result: Result<Cache, String> = tokio::select! { _ = lease.cancelled() => return Err("session_invalid".into()), value = fetch => value };
        let current = self.resolver.native_session()?;
        if (current.0, current.1) != ticket || lease.is_cancelled() { return Err("session_invalid".into()); }
        self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?.record_registry_result(key, result, cached)
    }
    pub async fn project_mcp_config(&self, agents: &AgentManager, input: ProjectionInput, permit: ActivityPermit, ticket: Ticket) -> Result<ConfigProjection, String> {
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection.clone(), Domain::Mcp, false, permit.clone(), &ticket).await?;
        tokio::task::spawn_blocking(move || { let _permit = permit; admitted.api.probe_mcp_config(input.selection.target, input.selection.scope, &input.server_name).map(|v| v.projection).map_err(|e| e.to_string()) }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn preview_mcp_probe(&self, agents: &AgentManager, cli: &dgos_db::CliRepository, input: ProbeInput, permit: ActivityPermit, ticket: Ticket) -> Result<ProbeSnapshot, String> {
        let permit = Arc::new(permit);
        self.reclaim_orphaned_probe(&ticket).await?;
        let admitted = self.admit(agents, input.selection.clone(), Domain::Mcp, false, permit.clone(), &ticket).await?;
        let paths = cli.get(&ticket.0).await?.paths;
        let root = admitted.project.clone().unwrap_or_else(|| self.roots.home.clone());
        let id = uuid::Uuid::new_v4().to_string(); let nonce = id.clone(); let request = input.clone(); let api = admitted.api.clone();
        let prepared = tokio::task::spawn_blocking(move || prepare(&api, &request, &root, &nonce, &paths)).await.map_err(|_| fail("Unavailable"))??;
        drop(admitted);
        let current = self.resolver.native_session()?;
        if (current.0, current.1) != ticket || current.2.is_cancelled() { return Err("session_invalid".into()); }
        let snapshot = ProbeSnapshot { probe_id: id.clone(), selection: input.selection.clone(), config: prepared.config.projection.clone(), digest: prepared.fingerprint.clone(), executable: prepared.executable, cwd: prepared.cwd, root: prepared.root, platform_identity: prepared.platform_identity, executable_identity: prepared.executable_identity.map(|v| hash(v.as_bytes())), expires_at: now() + 120, state: "ready".into(), phase: "approval".into(), cleanup: "not_started".into(), error: None, observed_at: None, protocol_version: None, tools: Vec::new(), schema_source: "independent_host_probe", may_download_execute: prepared.config.projection.transport == core::mcp::Transport::Stdio, probe_args: prepared.probe_args, cleanup_plan: prepared.cleanup_plan };
        let mut state = self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?;
        state.probes.retain(|_, record| record.owner == ticket || record.snapshot.state == "running" || record.snapshot.state == "unknown");
        if state.active.is_some() { return Err(fail("Busy")); }
        if state.probes.len() >= 64 { state.probes.retain(|_, record| record.snapshot.state == "unknown" || record.snapshot.state == "running"); }
        if state.closing { return Err(fail("Unavailable")); }
        state.probes.insert(id, ProbeRecord { owner: ticket, input, snapshot: snapshot.clone(), expected: prepared.fingerprint, cancel: CancellationToken::new(), connection: None, permit: None });
        Ok(snapshot)
    }
    pub async fn run_mcp_probe(&self, agents: &AgentManager, cli: &dgos_db::CliRepository, input: ProbeApproval, permit: ActivityPermit, ticket: Ticket) -> Result<ProbeSnapshot, String> {
        self.library_lease(&ticket)?;
        let (request, expected, initial) = { let state = self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?; let record = state.probes.get(&input.probe_id).filter(|r| r.owner == ticket).ok_or_else(|| fail("NotFound"))?; if record.snapshot.state != "ready" { return Ok(record.snapshot.clone()); } if input.approved_digest != record.expected { return Err(fail("Conflict")); } if now() > record.snapshot.expires_at { return Err(fail("Expired")); } (record.input.clone(), record.expected.clone(), record.snapshot.clone()) };
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, request.selection.clone(), Domain::Mcp, false, permit.clone(), &ticket).await?;
        let paths = cli.get(&ticket.0).await?.paths;
        let api = admitted.api.clone(); let root = admitted.project.clone().unwrap_or_else(|| self.roots.home.clone());
        let nonce = input.probe_id.clone(); let req = request.clone(); let prepared_api = api.clone(); let root_copy = root.clone();
        let prepared = tokio::task::spawn_blocking(move || prepare(&prepared_api, &req, &root_copy, &nonce, &paths)).await.map_err(|_| fail("Unavailable"))??;
        if prepared.fingerprint != expected { return Err(fail("Conflict")); }
        let (owner, generation, lease) = self.resolver.native_session()?;
        if (owner, generation) != ticket || lease.is_cancelled() { return Err("session_invalid".into()); }
        let inner = self.mcp_catalog.inner.clone(); let id = input.probe_id;
        let cli = cli.clone(); let owner = ticket.0.clone();
        let mut workers = self.mcp_catalog.workers.lock().map_err(|_| fail("Unavailable"))?;
        workers.retain(|worker| !worker.is_finished());
        let cancel = { let mut state = inner.lock().map_err(|_| fail("Unavailable"))?; if state.closing { return Err(fail("Unavailable")); } if state.active.is_some() { return Err(fail("Busy")); } let record = state.probes.get_mut(&id).ok_or_else(|| fail("NotFound"))?; if record.snapshot.state != "ready" { return Ok(record.snapshot.clone()); } if now() > record.snapshot.expires_at { return Err(fail("Expired")); } record.permit = Some(permit.clone()); record.snapshot.state = "running".into(); record.snapshot.phase = "connect".into(); record.snapshot.cleanup = "pending".into(); let cancel = record.cancel.clone(); state.active = Some(id.clone()); cancel };
        let worker = tokio::spawn(async move {
            let (_permit, _admitted) = (permit, admitted);
            let mut connection = None;
            let work = async {
                let value = Arc::new(ProbeConnection::connect(prepared.endpoint).map_err(|_| "probe_connect_failed".to_owned())?);
                connection = Some(value.clone());
                { let mut state = inner.lock().map_err(|_| fail("Unavailable"))?; if let Some(record) = state.probes.get_mut(&id) { record.connection = Some(value.clone()); record.snapshot.phase = "initialize_tools".into(); } }
                value.read_tools(|phase| { if let Ok(mut state) = inner.lock() { if let Some(record) = state.probes.get_mut(&id) { record.snapshot.phase = phase.into(); } } }).await.map_err(|error| match error { dgos_mcp::McpError::CredentialUnavailable | dgos_mcp::McpError::PermissionDenied => "probe_auth_required".into(), _ => "probe_protocol_failed".into() })
            };
            let result = tokio::select! { biased; _ = lease.cancelled() => Err("session_invalid".into()), _ = cancel.cancelled() => Err("cancelled".into()), result = tokio::time::timeout(Duration::from_secs(15), work) => result.unwrap_or_else(|_| Err("probe_timeout".into())) };
            let cleaned = if let Some(connection) = &connection { matches!(tokio::time::timeout(Duration::from_secs(2), connection.cleanup()).await, Ok(true)) } else { true };
            let mut result = result;
            if !lease.is_cancelled() && !cancel.is_cancelled() {
                let check_id = id.clone();
                let paths = cli.get(&owner).await;
                if let Ok(paths) = paths {
                    let current = tokio::task::spawn_blocking(move || prepare(&api, &request, &root, &check_id, &paths.paths)).await;
                    if !matches!(&current, Ok(Ok(value)) if value.fingerprint == expected) { result = Err(fail("Conflict")); }
                } else { result = Err(fail("Conflict")); }
            }
            let mut state = match inner.lock() { Ok(state) => state, Err(_) => return };
            if let Some(record) = state.probes.get_mut(&id) {
                record.snapshot.observed_at = Some(now()); if !cleaned { record.snapshot.phase = "cleanup".into(); } else if result.is_ok() { record.snapshot.phase = "finished".into(); } record.snapshot.cleanup = if cleaned { "confirmed" } else { "unknown" }.into();
                match result { Ok(tools) => { record.snapshot.state = if cleaned { "succeeded" } else { "unknown" }.into(); record.snapshot.protocol_version = Some(tools.protocol_version); record.snapshot.tools = tools.tools; }, Err(error) => { record.snapshot.state = if !cleaned { "unknown" } else if error == "cancelled" || error == "session_invalid" { "cancelled" } else { "failed" }.into(); record.snapshot.error = Some(error); } }
                if cleaned { record.connection = None; record.permit = None; }
            }
            if cleaned { state.active = None; }
            drop(state);
            if !cleaned && !lease.is_cancelled() {
                let cleanup_inner = inner.clone(); let cleanup_id = id.clone();
                tokio::spawn(async move {
                    lease.cancelled().await;
                    if let Some(connection) = connection {
                        if matches!(tokio::time::timeout(Duration::from_secs(2), connection.cleanup()).await, Ok(true)) {
                            if let Ok(mut state) = cleanup_inner.lock() {
                                if state.active.as_deref() == Some(cleanup_id.as_str()) { state.active = None; state.probes.remove(&cleanup_id); }
                            }
                        }
                    }
                });
            }
        });
        workers.push(worker);
        Ok(ProbeSnapshot { state: "running".into(), phase: "connect".into(), cleanup: "pending".into(), ..initial })
    }
    pub async fn inspect_mcp_probe(&self, input: ProbeOperation, cancel: bool, _permit: ActivityPermit, ticket: Ticket) -> Result<ProbeSnapshot, String> {
        self.library_lease(&ticket)?;
        let (connection, unknown) = { let mut state = self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?; let record = state.probes.get_mut(&input.probe_id).filter(|r| r.owner == ticket).ok_or_else(|| fail("NotFound"))?;
            if cancel { record.cancel.cancel(); if record.snapshot.state == "ready" { record.snapshot.state = "cancelled".into(); record.snapshot.cleanup = "confirmed".into(); } }
            (record.connection.clone(), record.snapshot.state == "unknown") };
        if unknown {
            if let Some(connection) = connection { if matches!(tokio::time::timeout(Duration::from_secs(2), connection.cleanup()).await, Ok(true)) {
                let mut state = self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?;
                if let Some(record) = state.probes.get_mut(&input.probe_id) { record.snapshot.cleanup = "confirmed".into(); if record.snapshot.error.is_none() { record.snapshot.phase = "finished".into(); } record.snapshot.state = if record.snapshot.error.is_none() { "succeeded" } else if matches!(record.snapshot.error.as_deref(), Some("cancelled" | "session_invalid")) { "cancelled" } else { "failed" }.into(); record.connection = None; record.permit = None; }
                if state.active.as_deref() == Some(&input.probe_id) { state.active = None; }
            } }
        }
        self.library_lease(&ticket)?;
        self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?.probes.get(&input.probe_id).filter(|r| r.owner == ticket).map(|r| r.snapshot.clone()).ok_or_else(|| fail("NotFound"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_references_are_not_shell_expansions() {
        assert!(valid_env("NATIVE_TOKEN")); assert!(!valid_env("TOKEN:-fallback")); assert!(expand("${TOKEN:-fallback}").is_err());
    }
    #[test]
    fn probe_approval_has_no_renderer_execution_fields() {
        assert!(serde_json::from_value::<ProbeApproval>(json!({"probe_id":"id", "approved_digest":"hash", "program":"bad"})).is_err());
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryResolve { pub selection: Selection, pub name: String, pub version: String, pub expected_server: Value }
impl NativeHost {
    pub async fn resolve_mcp_registry(&self, agents: &AgentManager, input: RegistryResolve, permit: ActivityPermit, ticket: Ticket) -> Result<Value, String> {
        if input.name.len() > 200 || input.version.is_empty() || input.version.len() > 255 || input.name.split('/').count() != 2 || input.name.chars().any(char::is_control) || input.version.chars().any(char::is_control) { return Err(fail("InvalidFormat")); }
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection, Domain::Mcp, false, permit.clone(), &ticket).await?;
        drop(admitted);
        let (owner, generation, lease) = self.resolver.native_session()?;
        if (owner, generation) != ticket { return Err("session_invalid".into()); }
        let mut url = reqwest::Url::parse(REGISTRY).map_err(|_| fail("Unavailable"))?;
        url.path_segments_mut().map_err(|_| fail("Unavailable"))?.push(&input.name).push("versions").push(&input.version);
        let request = async {
            let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(15)).build().map_err(|_| "registry_unavailable".to_owned())?;
            let mut response = client.get(url).send().await.map_err(|_| "registry_offline".to_owned())?;
            if !response.status().is_success() { return Err(if response.status().as_u16() == 429 { "registry_rate_limited" } else { "registry_unavailable" }.to_owned()); }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| "registry_offline".to_owned())? { if bytes.len() + chunk.len() > 1024 * 1024 { return Err("registry_limit".into()); } bytes.extend_from_slice(&chunk); }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| "registry_invalid".to_owned())?;
            if !valid_registry_entry(&value) { return Err("registry_invalid".into()); }
            if value.get("server") != Some(&input.expected_server) || value.pointer("/_meta/io.modelcontextprotocol.registry~1official/status").and_then(Value::as_str) != Some("active") { return Err(fail("Conflict")); }
            Ok(value)
        };
        tokio::select! { _ = lease.cancelled() => Err("session_invalid".into()), result = request => { let (owner, generation, _) = self.resolver.native_session()?; if (owner,generation) != ticket { Err("session_invalid".into()) } else { result } } }
    }
}

impl NativeHost {
    /// A new account may reclaim only cleanup ownership, never an old result or
    /// credentials. This does not replay initialize, tools/list, or execution.
    async fn reclaim_orphaned_probe(&self, ticket: &Ticket) -> Result<(), String> {
        let orphan = {
            let state = self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?;
            state.active.as_ref().and_then(|id| state.probes.get(id).filter(|record| record.owner != *ticket).map(|record| {
                record.cancel.cancel();
                (id.clone(), record.snapshot.state == "unknown", record.connection.clone())
            }))
        };
        if let Some((id, true, Some(connection))) = orphan {
            if matches!(tokio::time::timeout(Duration::from_secs(2), connection.cleanup()).await, Ok(true)) {
                let mut state = self.mcp_catalog.inner.lock().map_err(|_| fail("Unavailable"))?;
                if state.active.as_deref() == Some(id.as_str()) { state.active = None; state.probes.remove(&id); }
            }
        }
        Ok(())
    }
}
impl Drop for State {
    fn drop(&mut self) { for record in self.probes.values() { record.cancel.cancel(); } }
}

fn valid_registry_input(value: &Value) -> bool {
    value.is_object() && ["description", "default", "value", "placeholder", "format"].iter().all(|key| value.get(key).is_none_or(Value::is_string))
        && ["isRequired", "isSecret", "isRepeated"].iter().all(|key| value.get(key).is_none_or(Value::is_boolean))
        && value.get("choices").is_none_or(|value| value.is_null() || value.as_array().is_some_and(|values| values.len() <= 256 && values.iter().all(Value::is_string)))
        && value.get("variables").is_none_or(|variables| variables.as_object().is_some_and(|variables| variables.len() <= 128 && variables.values().all(valid_registry_input)))
}
fn valid_registry_transport(value: &Value) -> bool {
    value.is_object() && value.get("type").is_some_and(Value::is_string) && value.get("url").is_none_or(Value::is_string)
        && value.get("variables").is_none_or(|variables| variables.as_object().is_some_and(|variables| variables.len() <= 128 && variables.values().all(valid_registry_input)))
        && value.get("headers").is_none_or(|headers| headers.is_null() || headers.as_array().is_some_and(|headers| headers.len() <= 128 && headers.iter().all(|header| header.get("name").is_some_and(Value::is_string) && valid_registry_input(header))))
}
fn valid_registry_entry(value: &Value) -> bool {
    let Some(server) = value.get("server").filter(|server| server.is_object()) else { return false; };
    let Some(name) = server.get("name").and_then(Value::as_str) else { return false; };
    if name.len() > 200 || name.split('/').count() != 2 || name.split('/').any(str::is_empty) || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_/".contains(&b)) { return false; }
    if !["description", "version"].iter().all(|key| server.get(key).and_then(Value::as_str).is_some_and(|value| !value.is_empty() && value.len() <= 4096)) || server.get("title").is_some_and(|v| !v.is_string()) { return false; }
    if !value.get("_meta").is_some_and(Value::is_object) { return false; }
    if !server.get("repository").is_none_or(|repository| repository.is_object() && ["url", "source"].iter().all(|key| repository.get(key).is_none_or(Value::is_string))) { return false; }
    if !server.get("remotes").is_none_or(|remotes| remotes.is_null() || remotes.as_array().is_some_and(|remotes| remotes.len() <= 64 && remotes.iter().all(valid_registry_transport))) { return false; }
    server.get("packages").is_none_or(|packages| packages.is_null() || packages.as_array().is_some_and(|packages| packages.len() <= 64 && packages.iter().all(|package| {
        ["registryType", "identifier"].iter().all(|key| package.get(key).is_some_and(Value::is_string))
            && ["version", "runtimeHint", "registryBaseUrl", "fileSha256"].iter().all(|key| package.get(key).is_none_or(Value::is_string))
            && package.get("transport").is_some_and(valid_registry_transport)
            && ["runtimeArguments", "packageArguments"].iter().all(|key| package.get(key).is_none_or(|arguments| arguments.is_null() || arguments.as_array().is_some_and(|arguments| arguments.len() <= 256 && arguments.iter().all(|argument| argument.get("type").is_some_and(Value::is_string) && ["name", "valueHint"].iter().all(|key| argument.get(key).is_none_or(Value::is_string)) && valid_registry_input(argument)))))
            && package.get("environmentVariables").is_none_or(|variables| variables.is_null() || variables.as_array().is_some_and(|variables| variables.len() <= 128 && variables.iter().all(|variable| variable.get("name").is_some_and(Value::is_string) && valid_registry_input(variable))))
    })))
}

#[cfg(test)]
mod registry_shape_tests {
    use super::*;
    #[test]
    fn validates_real_server_wrapper_and_rejects_malformed_deployments() {
        let mut entry = json!({"server":{"name":"io.example/server","description":"Published server","version":"1.0.0","remotes":[{"type":"streamable-http","url":"https://example.com/mcp","headers":[{"name":"Authorization","isSecret":true}]}]},"_meta":{"io.modelcontextprotocol.registry/official":{"status":"active"}}});
        assert!(valid_registry_entry(&entry));
        entry["server"]["remotes"][0]["headers"] = json!({"Authorization":"literal"});
        assert!(!valid_registry_entry(&entry));
        entry["server"]["remotes"] = Value::Null;
        assert!(valid_registry_entry(&entry));
        entry["server"]["packages"] = json!([{"registryType":"npm","identifier":"package","transport":{"type":"stdio"},"packageArguments":[{"type":"named","name":"--path","variables":{"path":{"isRequired":"not-a-boolean"}}}]}]);
        assert!(!valid_registry_entry(&entry));
    }
    #[test]
    fn directory_changes_do_not_confuse_contents_with_root_identity() {
        let directory = tempfile::tempdir().unwrap();
        let before = identity(directory.path(), true).unwrap();
        std::fs::write(directory.path().join("new-file"), b"data").unwrap();
        assert_eq!(identity(directory.path(), true).unwrap(), before);
    }
    #[test]
    fn registry_failure_survives_reopen_and_cursors_cannot_change_filters() {
        let mut state = State::default();
        let cache = Cache { page: RegistryPage { servers: vec![json!({"server":"retained"})], next_cursor: Some("opaque-next".into()), fetched_at: now(), stale: false, error: None, source: REGISTRY }, etag: Some("version".into()), filter: "account/platform/search-a".into() };
        state.record_registry_result("first".into(), Ok(cache.clone()), None).unwrap();
        assert!(state.registry_cache("second", &cache.filter, Some("opaque-next")).unwrap().is_none());
        assert!(state.registry_cache("second", "account/platform/search-b", Some("opaque-next")).is_err());
        state.record_registry_result("first".into(), Err("registry_offline".into()), Some(cache.clone())).unwrap();
        let reopened = state.registry_cache("first", &cache.filter, None).unwrap().unwrap();
        assert!(reopened.page.stale); assert_eq!(reopened.page.error.as_deref(), Some("registry_offline"));
        assert_eq!(reopened.page.servers, cache.page.servers); assert_eq!(reopened.page.fetched_at, cache.page.fetched_at);
    }
    #[cfg(unix)]
    #[test]
    fn platform_identity_uses_trusted_target_overrides_and_detects_replacement() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let codex = root.path().join("custom-codex"); let claude = root.path().join("custom-claude");
        for (path, body) in [(&codex, b"codex".as_slice()), (&claude, b"claude".as_slice())] {
            std::fs::write(path, body).unwrap(); std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let paths = dgos_db::CliPaths { codex: Some(codex.to_string_lossy().into_owned()), claude: Some(claude.to_string_lossy().into_owned()), ..Default::default() };
        assert_eq!(platform_binary(core::Target::Codex, &paths).unwrap(), codex.canonicalize().unwrap());
        assert_eq!(platform_binary(core::Target::Claude, &paths).unwrap(), claude.canonicalize().unwrap());
        let previous = identity(&platform_binary(core::Target::Codex, &paths).unwrap(), false).unwrap();
        std::fs::write(&codex, b"replaced").unwrap();
        assert_ne!(previous, identity(&platform_binary(core::Target::Codex, &paths).unwrap(), false).unwrap());
        let docker = root.path().join("docker"); std::fs::write(&docker, b"docker").unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let alias = root.path().join("innocent-alias"); symlink(&docker, &alias).unwrap();
        assert!(docker_program(alias.to_str(), &binary(alias.to_str().unwrap()).unwrap()));
    }
    #[tokio::test]
    async fn shutdown_awaits_registered_probe_supervision_before_returning() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let state = McpCatalogState::default();
        let finished = Arc::new(AtomicBool::new(false)); let completed = finished.clone();
        state.workers.lock().unwrap().push(tokio::spawn(async move { tokio::task::yield_now().await; completed.store(true, Ordering::Release); }));
        assert!(state.shutdown().await);
        assert!(finished.load(Ordering::Acquire));
        assert!(state.inner.lock().unwrap().closing);
    }
}
