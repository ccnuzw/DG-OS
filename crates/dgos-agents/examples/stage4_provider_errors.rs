//! Real native-CLI HTTP failure acceptance, plus throwaway fail-closed recovery cases.
//! Run from the repository root: bun scripts/stage4-provider-errors.ts
//! The launcher creates empty native config/HOME directories; no native credentials are read.
//! Direct equivalent after bootstrap: cargo run -p dgos-agents --example
//! stage4_provider_errors -- --root <printed-artifact-dir>.
//! Only the final result and artifact path reach stdout. Evidence deliberately excludes
//! raw requests, headers, native transcripts and model summaries. Credentials are random,
//! process-local fixture values stored only in an in-memory ProviderCredentialVault.
use async_trait::async_trait;
use axum::{extract::State, http::{HeaderMap, Method, StatusCode, Uri}, response::IntoResponse, Json, Router};
use dgos_agents::{capabilities::{CapabilityConfiguration, CapabilitySnapshot, PinnedSkill, ProjectCapabilities, ProjectProvider, SaveCapabilitiesInput}, projects::{CreateProjectInput, SaveTaskInput}, AgentManager, Engine, RunRecord, SessionOptions};
use dgos_db::{development_repository::StoredAgentSession, migrations::Migrator, sea_orm_migration::MigratorTrait, DevelopmentRepository, ProviderRepository};
use dgos_mcp::McpManager;
use dgos_providers::{new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, ProviderManager, SecretValue};
use dgos_skills::SkillsManager;
use dgos_types::{DgosError, DgosResult, ProviderConfig};
use serde::Serialize;
use serde_json::json;
use std::{collections::HashMap, path::{Path, PathBuf}, sync::Arc, time::Duration};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const OWNER: &str = "stage4-provider-errors";
const DEADLINE_SECONDS: u64 = 75;
type Result<T, E = String> = std::result::Result<T, E>;

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, SecretValue>>);
#[async_trait]
impl ProviderCredentialVault for MemoryVault {
    async fn store(&self, scope: &ProviderCredentialScope, value: SecretValue) -> DgosResult<()> {
        self.0.lock().await.insert(scope.account(), value);
        Ok(())
    }
    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue> {
        let values = self.0.lock().await;
        SecretValue::new(values.get(&scope.account()).ok_or(DgosError::CredentialUnavailable)?.expose().to_owned())
    }
    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()> {
        self.0.lock().await.remove(&scope.account());
        Ok(())
    }
}

#[derive(Clone, Serialize)]
struct Observation {
    engine: String,
    requested_status: u16,
    response_status: u16,
    method: String,
    path: String,
    auth_present: bool,
    auth_matches: bool,
    supported_path: bool,
}
#[derive(Clone)]
struct HttpFixture {
    // Never serialize this state or any header value.
    keys: Arc<HashMap<String, String>>,
    observations: Arc<Mutex<Vec<Observation>>>,
}
async fn handle_http(State(state): State<HttpFixture>, method: Method, uri: Uri, headers: HeaderMap) -> impl IntoResponse {
    let parts: Vec<_> = uri.path().trim_matches('/').split('/').collect();
    let engine = parts.first().copied().unwrap_or("");
    let requested = parts.get(1).and_then(|s| s.parse::<u16>().ok()).filter(|s| matches!(*s, 400 | 401)).unwrap_or(400);
    let authorization = headers.get("authorization").and_then(|s| s.to_str().ok());
    let api_key = headers.get("x-api-key").and_then(|s| s.to_str().ok());
    let auth_present = authorization.is_some_and(|s| !s.is_empty()) || api_key.is_some_and(|s| !s.is_empty());
    let auth_matches = state.keys.get(engine).is_some_and(|key| {
        authorization.and_then(|s| s.strip_prefix("Bearer ")).is_some_and(|s| s == key) || api_key.is_some_and(|s| s == key)
    });
    let supported_path = method == Method::POST && match engine {
        "codex" => uri.path() == format!("/codex/{requested}/v1/responses"),
        "claude" => uri.path() == format!("/claude/{requested}/v1/messages"),
        _ => false,
    };
    let status = if !auth_matches { 401 } else if !supported_path { 404 } else { requested };
    // Record only fixed structural facts, never URI queries, bodies or header values.
    state.observations.lock().await.push(Observation {
        engine: if matches!(engine, "codex" | "claude") { engine.into() } else { "other".into() },
        requested_status: requested, response_status: status, method: method.to_string(),
        path: if supported_path { uri.path().into() } else { "unsupported_path".into() },
        auth_present, auth_matches, supported_path,
    });
    let kind = if status == 401 { "authentication_error" } else { "invalid_request_error" };
    let marker = format!("stage4_fixture_http_{status}");
    let body = if engine == "claude" {
        json!({"type":"error","error":{"type":kind,"message":marker}})
    } else {
        json!({"error":{"type":kind,"message":marker,"code":if status == 401 { "invalid_api_key" } else { "invalid_request" },"param":null}})
    };
    (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST), [("x-should-retry", "false"), ("request-id", "stage4-provider-error-fixture")], Json(body))
}

struct Harness {
    root: PathBuf,
    repository: DevelopmentRepository,
    agents: AgentManager,
    capabilities: ProjectCapabilities,
    providers: ProviderManager,
    provider_repository: ProviderRepository,
    vault: Arc<MemoryVault>,
    skills: Arc<RwLock<SkillsManager>>,
    mcp: Arc<McpManager>,
    fixture: HttpFixture,
    address: String,
}
struct Chain {
    project: String,
    task: String,
    workspace: String,
}
fn label(engine: Engine) -> &'static str { match engine { Engine::Codex => "codex", Engine::Claude => "claude" } }
fn require(condition: bool, reason: &str) -> Result<()> { if condition { Ok(()) } else { Err(reason.into()) } }
fn opaque(_: impl std::fmt::Display) -> String { "artifact_or_service_failed".into() }
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(opaque)?;
    std::fs::write(path, bytes).map_err(opaque)
}
fn env(name: &str) -> Result<String> { std::env::var(name).ok().filter(|s| !s.is_empty()).ok_or_else(|| format!("missing_environment_{name}")) }
fn options(chain: &Chain, engine: Engine, root: &Path) -> SessionOptions {
    SessionOptions { workspace: chain.workspace.clone(), resume_id: None, executable: Some(root.join(format!("{}-cli", label(engine))).to_string_lossy().into()), model: None, trust_workspace: true }
}
fn configuration(engine: Engine, provider: ProjectProvider) -> CapabilityConfiguration {
    CapabilityConfiguration { engine, provider, skills: Vec::new(), mcp: Vec::new() }
}
fn provider_config(engine: Engine, endpoint: &str, enabled: bool, key: String) -> ProviderConfig {
    ProviderConfig { name: format!("Stage4 {} local error fixture", label(engine)), provider_type: if engine == Engine::Codex { "openai_compatible" } else { "anthropic" }.into(), base_url: endpoint.into(), api_key: key, enabled, config: Some(json!({"allow_insecure_http":true,"auth_style":if engine == Engine::Codex { "bearer" } else { "x-api-key" }})) }
}

#[tokio::main]
async fn main() {
    let result = run().await;
    let root = std::env::args().skip(1).collect::<Vec<_>>().get(1).cloned().unwrap_or_default();
    // No error values originating in CLIs, HTTP clients or credentials reach stdout/stderr.
    println!("{}", json!({"result":if result.is_ok() { "passed" } else { "failed" },"artifact":root}));
    if result.is_err() { std::process::exit(1); }
}
async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    require(args.len() == 2 && args[0] == "--root", "usage_requires_isolated_root")?;
    let root = PathBuf::from(&args[1]).canonicalize().map_err(opaque)?;
    require(root.join("stage4-provider-errors-isolated").is_file(), "launcher_required")?;
    for (name, directory) in [("CODEX_HOME", "codex-home"), ("CLAUDE_CONFIG_DIR", "claude-home"), ("HOME", "home")] {
        require(PathBuf::from(env(name)?).canonicalize().map_err(opaque)? == root.join(directory).canonicalize().map_err(opaque)?, "native_config_not_isolated")?;
    }
    let url = format!("sqlite://{}?mode=rwc", root.join("acceptance.sqlite").display());
    let db = dgos_db::establish_connection(&url).await.map_err(opaque)?;
    Migrator::up(&db, None).await.map_err(opaque)?;
    let repository = DevelopmentRepository::new(db.clone());
    let vault = Arc::new(MemoryVault::default());
    let providers = ProviderManager::with_vault(&url, vault.clone()).await.map_err(opaque)?;
    let skills = Arc::new(RwLock::new(SkillsManager::new(db.clone(), root.join("skills"))));
    let mcp = Arc::new(McpManager::new(db.clone()));
    let capabilities = ProjectCapabilities::new(repository.clone(), providers.clone(), skills.clone(), mcp.clone(), root.join("capability-material"));
    let agents = AgentManager::new(root.join("unused-legacy-session-index.json"), repository.clone());
    agents.install_capabilities(capabilities.clone());
    agents.load().await?;
    let keys = ["codex", "claude"].into_iter().map(|engine| (engine.into(), Uuid::new_v4().to_string())).collect();
    let fixture = HttpFixture { keys: Arc::new(keys), observations: Arc::default() };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(opaque)?;
    let address = format!("http://{}", listener.local_addr().map_err(opaque)?);
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let app = Router::new().fallback(handle_http).with_state(fixture.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).with_graceful_shutdown(stopped.cancelled_owned()).await });
    let harness = Harness { root: root.clone(), repository, agents, capabilities, providers, provider_repository: ProviderRepository::new(db), vault, skills, mcp, fixture, address };
    let mut results = Vec::new();
    for engine in [Engine::Codex, Engine::Claude] {
        for status in [401, 400] {
            let name = format!("{}-http-{status}", label(engine));
            let outcome = harness.http_error(engine, status).await;
            results.push(json!({"scenario":name,"passed":outcome.is_ok(),"failure":outcome.err().map(|reason|harness.safe_reason(&reason))}));
        }
        for scenario in ["missing-skill-restore", "cli-version-restore"] {
            let outcome = harness.inert_restore(engine, scenario).await;
            results.push(json!({"scenario":format!("{}-{scenario}",label(engine)),"passed":outcome.is_ok(),"failure":outcome.err().map(|reason|harness.safe_reason(&reason))}));
        }
    }
    let agents_stopped = harness.agents.shutdown().await;
    let mcp_stopped = harness.mcp.shutdown().await.is_ok();
    stop.cancel();
    let server_stopped = matches!(tokio::time::timeout(Duration::from_secs(5), server).await, Ok(Ok(Ok(()))));
    let passed = results.iter().all(|r| r["passed"] == true) && agents_stopped && mcp_stopped && server_stopped;
    write_json(&root.join("observations.json"), &*harness.fixture.observations.lock().await)?;
    write_json(&root.join("result.json"), &json!({"passed":passed,"scenarios":results,"cleanup":{"agents":agents_stopped,"mcp":mcp_stopped,"http":server_stopped},"deadline_seconds":DEADLINE_SECONDS,"evidence_policy":"production run status/reason/finished_at and native terminal status; assistant summaries excluded","recovery_fixture_policy":"disabled/rotation use genuine error sessions; skill/version recovery uses explicitly inert saved-session fixtures"}))?;
    require(passed, "acceptance_failed")
}

impl Harness {
    async fn chain(&self, name: &str) -> Result<Chain> {
        let workspace = self.root.join(name);
        std::fs::create_dir(&workspace).map_err(opaque)?;
        let project = self.agents.create_project(OWNER, CreateProjectInput { kind: "import".into(), name: name.into(), path: workspace.to_string_lossy().into(), source: None }, CancellationToken::new()).await?;
        let task = self.agents.save_development_task(OWNER, SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Observe native provider failure".into(), requirements: "Production terminal status and pinned capability evidence".into(), status: "todo".into() }, CancellationToken::new()).await?;
        Ok(Chain { project: project.id, task: task.id, workspace: project.path })
    }
    async fn save(&self, chain: &Chain, expected: i64, configuration: CapabilityConfiguration) -> Result<i64> {
        Ok(self.capabilities.save(OWNER, SaveCapabilitiesInput { project_id: chain.project.clone(), expected_revision: expected, configuration }).await?.revision)
    }
    async fn open(&self, engine: Engine, chain: &Chain, session: &str, revision: Option<i64>) -> Result<dgos_agents::OwnedSessionView> {
        let lease = CancellationToken::new();
        let opening = self.agents.open_with_capabilities(OWNER, engine, session.into(), options(chain, engine, &self.root), Some(chain.task.clone()), lease.clone(), revision);
        tokio::pin!(opening);
        tokio::select! {
            result = &mut opening => result,
            _ = tokio::time::sleep(Duration::from_secs(DEADLINE_SECONDS)) => {
                lease.cancel();
                let _ = tokio::time::timeout(Duration::from_secs(15), &mut opening).await;
                Err("open_deadline_exceeded".into())
            }
        }
    }
    async fn version(&self, engine: Engine) -> Result<String> {
        let output = tokio::time::timeout(Duration::from_secs(10), tokio::process::Command::new(self.root.join(format!("{}-cli",label(engine)))).arg("--version").kill_on_drop(true).output()).await.map_err(opaque)?.map_err(opaque)?;
        require(output.status.success() && output.stdout.len() <= 4096, "version_unavailable")?;
        String::from_utf8(output.stdout).map(|s|s.trim().into()).map_err(opaque)
    }
    async fn observations(&self, engine: Engine, status: u16) -> Vec<Observation> {
        self.fixture.observations.lock().await.iter().filter(|o|o.engine == label(engine) && o.requested_status == status).cloned().collect()
    }
    async fn terminal(&self, task: &str, run: &str) -> Result<Option<RunRecord>> {
        Ok(self.agents.list_development_runs(OWNER, task).await?.into_iter().find(|r| r.id == run && r.finished_at.is_some()))
    }
    async fn http_error(&self, engine: Engine, status: u16) -> Result<()> {
        let name = format!("{}-http-{status}",label(engine));
        let chain = self.chain(&name).await?;
        let endpoint = format!("{}/{}/{status}{}",self.address,label(engine),if engine == Engine::Codex { "/v1" } else { "" });
        let provider = Uuid::new_v4().to_string();
        let reference = new_credential_ref();
        self.vault.store(&ProviderCredentialScope { user_id: OWNER.into(), provider_id: provider.clone(), endpoint: endpoint.clone(), credential_ref: reference.clone() }, SecretValue::new(self.fixture.keys.get(label(engine)).ok_or("fixture_key_missing")?.clone()).map_err(opaque)?).await.map_err(opaque)?;
        self.provider_repository.create_with_credential_ref(provider.clone(),provider_config(engine,&endpoint,true,String::new()),OWNER.into(),Some(reference)).await.map_err(opaque)?;
        let model = if engine == Engine::Codex { "aigo/gpt-6.1-sol" } else { "claude-sonnet-4-6" };
        self.providers.add_manual_model(&provider,OWNER,model,None).await.map_err(opaque)?;
        let provider_revision = self.providers.get_provider(&provider,OWNER).await.map_err(opaque)?.ok_or("provider_missing")?.config_revision;
        let selected = configuration(engine,ProjectProvider::Dgos { provider_id: provider.clone(), model_id: model.into(), revision: provider_revision });
        let revision = self.save(&chain,0,selected).await?;
        let session = Uuid::new_v4().to_string();
        let evidence_path = self.root.join(format!("{name}.json"));
        let opened = match self.open(engine,&chain,&session,Some(revision)).await {
            Ok(opened) => opened,
            Err(_) => {
                write_json(&evidence_path,&json!({"passed":false,"stage":"production_open_failed","engine":label(engine),"status":status,"requests":self.observations(engine,status).await}))?;
                return Err("production_open_failed".into());
            }
        };
        let snapshot = opened.capabilities.ok_or("snapshot_missing")?;
        require(matches!(&snapshot.configuration.provider,ProjectProvider::Dgos { provider_id,model_id,revision } if provider_id == &provider && model_id == model && *revision == provider_revision),"selected_provider_mismatch")?;
        // Move the project's head before send. The session and run must retain the old selection.
        let new_head = self.save(&chain,revision,configuration(engine,ProjectProvider::Native)).await?;
        let run = Uuid::new_v4().to_string();
        let send = tokio::time::timeout(Duration::from_secs(20),self.agents.send(OWNER,&session,run.clone(),"Reply with one word. This acceptance expects an HTTP failure from the selected local provider.".into())).await;
        let send_result = match send { Ok(Ok(())) => "accepted", Ok(Err(_)) => "rejected", Err(_) => "send_deadline_exceeded" };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(DEADLINE_SECONDS);
        let terminal = loop {
            if let Some(run) = self.terminal(&chain.task,&run).await? { break Some(run); }
            if tokio::time::Instant::now() >= deadline { break None; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let observations = self.observations(engine,status).await;
        let saved = self.capabilities.session_snapshot(OWNER,&session).await?;
        let run_snapshot = terminal.as_ref().and_then(|r|r.capabilities.as_ref());
        let snapshot_fixed = saved.as_ref().map(serde_json::to_value).transpose().map_err(opaque)? == Some(serde_json::to_value(&snapshot).map_err(opaque)?) && run_snapshot.map(serde_json::to_value).transpose().map_err(opaque)? == Some(serde_json::to_value(&snapshot).map_err(opaque)?);
        // Only structured native terminal events qualify. No assistant/model text is inspected.
        let native_terminal_failed = terminal.as_ref().is_some_and(|run|run.events.iter().any(|event| {
            event.run.as_deref() == Some(run.id.as_str()) && event.data["run_status"] == "failed" && match engine {
                Engine::Codex => event.kind == "turn/completed" && event.data["native_status"] == "failed",
                Engine::Claude => matches!(event.kind.as_str(),"result"|"failure"),
            }
        }));
        let reason_present = terminal.as_ref().is_some_and(|r|r.reason.as_ref().is_some_and(|s|!s.trim().is_empty()));
        let request_proof = !observations.is_empty() && observations.iter().all(|o|o.supported_path && o.auth_present && o.auth_matches && o.response_status == status);
        let failed = terminal.as_ref().is_some_and(|r|r.status == "failed" && r.finished_at.is_some() && r.source == "native_agent");
        let passed = request_proof && failed && reason_present && native_terminal_failed && snapshot_fixed && send_result == "accepted";
        // Known fixture credential strings are never copied into the artifact, even if a CLI
        // unexpectedly includes one in its error. Persist a sanitized production reason only.
        let reason = terminal.as_ref().and_then(|r|r.reason.as_ref()).map(|text|self.safe_reason(text));
        write_json(&evidence_path,&json!({"passed":passed,"engine":label(engine),"http_status":status,"session":session,"run":run,"send":send_result,"deadline_exceeded":terminal.is_none(),"requests":observations,"retry_control":{"requested":true,"codex":"request_max_retries=0; stream_max_retries=0","claude":"CLAUDE_CODE_MAX_RETRIES=0; ANTHROPIC_MAX_RETRIES=0; MAX_RETRIES=0","fixture":"x-should-retry:false","observed_requests":observations.len(),"effective":if observations.len() == 1 { "single_request_observed" } else { "retry_control_not_proven_or_cli_retried" }},"production":{"status":terminal.as_ref().map(|r|r.status.as_str()),"reason":reason,"finished_at":terminal.as_ref().and_then(|r|r.finished_at),"source":terminal.as_ref().map(|r|r.source.as_str()),"native_terminal_failed":native_terminal_failed},"snapshot_fixed":snapshot_fixed,"session_snapshot":saved,"run_snapshot":run_snapshot,"project_head_revision":new_head,"model_summary_used_as_evidence":false}))?;
        if terminal.is_none() { let _ = self.agents.interrupt(OWNER,&session).await; }
        let closed = self.agents.close_session(OWNER,&session).await?;
        require(closed,"native_cleanup_pending")?;
        // These restore attempts use the genuine native session and its original snapshot.
        // They must stop before a native resume/provider call, even though the project head is native.
        let restore = if status == 401 { self.disabled_and_rotated(engine,&chain,&session,&provider,&endpoint,&snapshot).await } else { Ok(()) };
        restore?;
        require(passed,"http_failure_evidence_missing")
    }
    fn safe_reason(&self, text: &str) -> String {
        let mut safe = text.to_owned();
        for key in self.fixture.keys.values() { safe = safe.replace(key,"[REDACTED]"); }
        safe
    }
    async fn disabled_and_rotated(&self, engine: Engine, chain: &Chain, session: &str, provider: &str, endpoint: &str, snapshot: &CapabilitySnapshot) -> Result<()> {
        let before = self.fixture.observations.lock().await.len();
        self.providers.update_provider(provider,provider_config(engine,endpoint,false,String::new()),OWNER).await.map_err(opaque)?;
        let disabled = self.open(engine,chain,session,None).await;
        let disabled_rejected = matches!(&disabled,Err(reason) if reason == "provider_unavailable");
        if disabled.is_ok() { let _ = self.agents.close_session(OWNER,session).await; }
        // Real credential-pointer rotation through ProviderManager, rather than editing SQL.
        self.providers.update_provider(provider,provider_config(engine,endpoint,true,Uuid::new_v4().to_string()),OWNER).await.map_err(opaque)?;
        let current = self.providers.get_provider(provider,OWNER).await.map_err(opaque)?.ok_or("provider_missing")?;
        let rotated = self.open(engine,chain,session,None).await;
        let rotated_rejected = matches!(&rotated,Err(reason) if reason == "provider_unavailable");
        if rotated.is_ok() { let _ = self.agents.close_session(OWNER,session).await; }
        let after = self.fixture.observations.lock().await.len();
        let unchanged = serde_json::to_value(self.capabilities.session_snapshot(OWNER,session).await?).map_err(opaque)? == serde_json::to_value(Some(snapshot)).map_err(opaque)?;
        let pinned_revision = match &snapshot.configuration.provider { ProjectProvider::Dgos { revision,.. } => *revision, _ => 0 };
        let passed = disabled_rejected && rotated_rejected && before == after && unchanged && current.config_revision > pinned_revision;
        write_json(&self.root.join(format!("{}-disabled-rotation-restore.json",label(engine))),&json!({"passed":passed,"evidence":"genuine native error session; production restore API; no native resume expected","disabled":{"rejected":disabled_rejected,"reason":if disabled_rejected { Some("provider_unavailable") } else { None }},"rotated":{"rejected":rotated_rejected,"reason":if rotated_rejected { Some("provider_unavailable") } else { None },"pinned_revision":pinned_revision,"current_revision":current.config_revision},"snapshot_unchanged":unchanged,"new_http_requests":after.saturating_sub(before)}))?;
        require(passed,"restore_did_not_fail_closed")
    }
    async fn inert_restore(&self, engine: Engine, scenario: &str) -> Result<()> {
        let chain = self.chain(&format!("{}-{scenario}",label(engine))).await?;
        let mut configuration = configuration(engine,ProjectProvider::Native);
        let missing_skill = scenario == "missing-skill-restore";
        let pin = if missing_skill {
            let manager = self.skills.read().await;
            let bytes = std::fs::read(self.root.join(format!("{}-restore-skill.zip",label(engine)))).map_err(opaque)?;
            let preview = manager.preview_external_package(OWNER,&bytes).await.map_err(opaque)?;
            let installed = manager.confirm_external_install(OWNER,&preview.operation_id).await.map_err(opaque)?;
            Some(PinnedSkill { id: installed.skill_id, version: installed.version, hash: installed.content_hash })
        } else { None };
        if let Some(pin) = &pin { configuration.skills.push(pin.clone()); }
        let revision = self.save(&chain,0,configuration.clone()).await?;
        let snapshot = CapabilitySnapshot { revision, permission_epoch: 0, engine, cli_version: self.version(engine).await?, configuration, provider_status: "inert_restore_fixture".into(), skills_status: "inert_restore_fixture".into(), mcp_status: "inert_restore_fixture".into(), native_source: "Throwaway persisted session fixture; no successful native session or model execution is claimed".into() };
        let session = Uuid::new_v4().to_string();
        self.repository.save_session(&StoredAgentSession { id: session.clone(), owner: OWNER.into(), engine: label(engine).into(), native_id: Some(Uuid::new_v4().to_string()), workspace: chain.workspace.clone(), executable: options(&chain,engine,&self.root).executable, model: None, project_id: Some(chain.project.clone()), task_id: Some(chain.task.clone()) }).await?;
        self.repository.save_session_capabilities(OWNER,&session,&chain.project,revision,0,&serde_json::to_string(&snapshot).map_err(opaque)?).await?;
        // Reload only after all preceding real runs are terminal and closed.
        self.agents.load().await?;
        let marker = self.root.join(format!("{}-changed-version",label(engine)));
        let expected = if let Some(pin) = &pin {
            self.skills.read().await.uninstall_skill(&pin.id,OWNER).await.map_err(opaque)?;
            "skill_unavailable"
        } else {
            std::fs::write(&marker,b"controlled CLI version change\n").map_err(opaque)?;
            "cli_version_changed"
        };
        let before = self.fixture.observations.lock().await.len();
        let restored = self.open(engine,&chain,&session,None).await;
        if !missing_skill { std::fs::remove_file(&marker).map_err(opaque)?; }
        let rejected = matches!(&restored,Err(reason) if reason == expected);
        if restored.is_ok() { let _ = self.agents.close_session(OWNER,&session).await; }
        let after = self.fixture.observations.lock().await.len();
        let saved = self.capabilities.session_snapshot(OWNER,&session).await?;
        let unchanged = serde_json::to_value(saved).map_err(opaque)? == serde_json::to_value(Some(&snapshot)).map_err(opaque)?;
        let passed = rejected && before == after && unchanged;
        write_json(&self.root.join(format!("{}-{scenario}.json",label(engine))),&json!({"passed":passed,"evidence":"inert saved-session fixture through production capability save and AgentManager restore APIs","expected_reason":expected,"rejected":rejected,"snapshot_unchanged":unchanged,"new_http_requests":after.saturating_sub(before),"cli_version_before":snapshot.cli_version,"missing_skill_removed_via_production_api":missing_skill,"native_session_resume_attempted":restored.is_ok()}))?;
        require(passed,"inert_restore_did_not_fail_closed")
    }
}
