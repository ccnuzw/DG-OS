//! Real, billable Stage 4 acceptance. No model response is treated as execution evidence.
//!
//! Prerequisites: Rust/Cargo, Bun, node, installed codex and claude CLIs, and the built
//! tools/agent-bridge binary (npm run agents:build). Bootstrap requires explicit
//! DGOS_STAGE4_{CODEX,CLAUDE}_{URL,KEY,MODEL}; subprocess native config roots are isolated.
//! Run from the repository root:
//!   bun scripts/stage4-acceptance.ts
//! Restart, using the artifact directory printed by the first invocation:
//!   bun scripts/stage4-acceptance.ts --resume /tmp/dgos-stage4-<id>
//! Or, with DGOS_STAGE4_{CODEX,CLAUDE}_{URL,KEY,MODEL} in the process environment,
//! CODEX_HOME/CLAUDE_CONFIG_DIR pointing at this artifact's isolated directories:
//!   cargo run -p dgos-agents --example stage4_acceptance -- --root <artifact-dir>
//! First invocation makes two real turns per engine; --resume makes the third turn
//! in each SAME native session and checks its saved capability snapshot. Evidence:
//! state.json, evidence/<engine>-round-<n>.json, evidence/<engine>-startup-<n>.json,
//! evidence/approvals.jsonl, fixture-observations.jsonl, acceptance.sqlite.
//! All MCP output is a local fixture. All native CLI/model turns are real provider calls.
use async_trait::async_trait;
use dgos_agents::{capabilities::{CapabilityConfiguration, CapabilitySnapshot, PinnedSkill, ProjectCapabilities, ProjectProvider, SaveCapabilitiesInput, SelectedMcpTool}, projects::{CreateProjectInput, SaveTaskInput}, AgentManager, Engine, RunRecord, SessionEvent, SessionOptions};
use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait, DevelopmentRepository, ProviderRepository};
use dgos_mcp::{CreateMcpServerRequest, McpCallPolicy, McpManager, StdioConfig, TransportKind};
use dgos_providers::{new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, ProviderManager, SecretValue};
use dgos_skills::SkillsManager;
use dgos_types::{DgosError, DgosResult, ProviderConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, io::Write, path::{Path, PathBuf}, sync::Arc, time::Duration};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const OWNER: &str = "stage4-local-host";
type Result<T, E = String> = std::result::Result<T, E>;
#[derive(Default)]
struct EphemeralVault(Mutex<HashMap<String, SecretValue>>);
#[async_trait]
impl ProviderCredentialVault for EphemeralVault {
    async fn store(&self, scope: &ProviderCredentialScope, value: SecretValue) -> DgosResult<()> { self.0.lock().await.insert(scope.account(), value); Ok(()) }
    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue> { let values = self.0.lock().await; SecretValue::new(values.get(&scope.account()).ok_or(DgosError::CredentialUnavailable)?.expose().to_owned()) }
    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()> { self.0.lock().await.remove(&scope.account()); Ok(()) }
}
struct FixturePolicy;
#[async_trait]
impl McpCallPolicy for FixturePolicy {
    async fn allows_call(&self, user: &str, _: &str, tool: &str) -> bool { user == OWNER && tool == "stage4_receipt" }
}
#[derive(Serialize, Deserialize, Default)]
struct State { engines: Vec<SavedEngine> }
#[derive(Serialize, Deserialize)]
struct SavedEngine {
    engine: Engine, provider_id: String, provider_revision: i64, model: String, endpoint: String,
    project: String, task: String, session: String, capability_revision: i64,
    skill: PinnedSkill, resource_marker: String, body_marker: String,
    snapshot: Option<CapabilitySnapshot>, native_id: Option<String>, completed_rounds: u32,
}
fn label(engine: Engine) -> &'static str { match engine { Engine::Codex => "codex", Engine::Claude => "claude" } }
fn io(error: impl std::fmt::Display) -> String { format!("artifact_io: {error}") }
fn digest(text: &str) -> String { format!("{:x}", Sha256::digest(text.as_bytes())) }
fn require(condition: bool, error: &str) -> Result<()> { if condition { Ok(()) } else { Err(error.into()) } }
fn redact(value: Value, secrets: &[String]) -> Value {
    match value {
        Value::String(mut text) => { for secret in secrets.iter().filter(|s| !s.is_empty()) { text = text.replace(secret, "[REDACTED]"); } Value::String(text) },
        Value::Array(values) => Value::Array(values.into_iter().map(|v| redact(v, secrets)).collect()),
        Value::Object(values) => Value::Object(values.into_iter().map(|(k,v)| { let lower = k.to_ascii_lowercase(); let v = if ["authorization", "bearer", "api_key", "password", "credential", "secret"].iter().any(|name|lower.contains(name)) { Value::String("[REDACTED]".into()) } else { redact(v,secrets) }; (k,v) }).collect()),
        value => value,
    }
}
fn artifact(path: &Path, value: Value, secrets: &[String]) -> Result<()> { std::fs::write(path, serde_json::to_vec_pretty(&redact(value, secrets)).map_err(io)?).map_err(io) }
fn append(path: &Path, value: Value, secrets: &[String]) -> Result<()> { let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(io)?; writeln!(file, "{}", redact(value, secrets)).map_err(io) }
fn env(name: &str) -> Result<String> { std::env::var(name).ok().filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("missing_environment: {name}")) }

#[tokio::main]
async fn main() {
    if let Err(error) = run().await { eprintln!("stage4 acceptance failed: {error}"); std::process::exit(1); }
}
async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    require(args.len() == 2 || args.len() == 3, "usage: stage4_acceptance --root <artifact-dir> [--resume]")?;
    require(args[0] == "--root" && (args.len() == 2 || args[2] == "--resume"), "invalid_arguments")?;
    let resume = args.len() == 3;
    let root = PathBuf::from(&args[1]).canonicalize().map_err(io)?;
    require(root.join("stage4-isolated-root").is_file(), "bootstrap_required: artifact root marker missing")?;
    for (name, subdir) in [("CODEX_HOME", "codex-home"), ("CLAUDE_CONFIG_DIR", "claude-home")] {
        require(PathBuf::from(env(name)?).canonicalize().map_err(io)? == root.join(subdir).canonicalize().map_err(io)?, "native_config_not_isolated")?;
    }
    let keys = [env("DGOS_STAGE4_CODEX_KEY")?, env("DGOS_STAGE4_CLAUDE_KEY")?];
    // Remove bootstrap and ambient provider secrets before spawning any CLI. The service
    // resolves the selected credential from the host vault into each private launch.
    for name in ["DGOS_STAGE4_CODEX_KEY", "DGOS_STAGE4_CLAUDE_KEY", "OPENAI_API_KEY", "OPENAI_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "DGOS_PROVIDER_KEY", "DGOS_MCP_BEARER"] { std::env::remove_var(name); }
    let url = format!("sqlite://{}?mode=rwc", root.join("acceptance.sqlite").display());
    let db = dgos_db::establish_connection(&url).await.map_err(io)?;
    Migrator::up(&db, None).await.map_err(io)?;
    let repository = DevelopmentRepository::new(db.clone());
    let vault = Arc::new(EphemeralVault::default());
    let providers = ProviderManager::with_vault(&url, vault.clone()).await.map_err(io)?;
    let skills = Arc::new(RwLock::new(SkillsManager::new(db.clone(), root.join("skills"))));
    let mcp = Arc::new(McpManager::with_policy(db.clone(), Arc::new(FixturePolicy)));
    let service = ProjectCapabilities::new(repository.clone(), providers.clone(), skills.clone(), mcp.clone(), root.join("capability-material"));
    let agents = AgentManager::new(root.join("retired-session-index.json"), repository.clone());
    agents.install_capabilities(service.clone());
    agents.load().await?;
    let state_path = root.join("state.json");
    let mut state: State = if resume { serde_json::from_slice(&std::fs::read(&state_path).map_err(io)?).map_err(io)? } else { require(!state_path.exists(), "state_exists: use --resume")?; State::default() };
    let execution = async {
        if !resume {
            for (index, engine) in [Engine::Codex, Engine::Claude].into_iter().enumerate() {
                let prefix = format!("DGOS_STAGE4_{}", label(engine).to_uppercase());
                let endpoint = env(&format!("{prefix}_URL"))?;
                let model = env(&format!("{prefix}_MODEL"))?;
                let id = Uuid::new_v4().to_string();
                let credential_ref = new_credential_ref();
                vault.store(&ProviderCredentialScope { user_id: OWNER.into(), provider_id: id.clone(), endpoint: endpoint.clone(), credential_ref: credential_ref.clone() }, SecretValue::new(keys[index].clone()).map_err(io)?).await.map_err(io)?;
                // Same metadata-only repository + vault seed as provider tests. No key in DB.
                ProviderRepository::new(db.clone()).create_with_credential_ref(id.clone(), ProviderConfig { name: format!("Stage4 {} real", label(engine)), provider_type: if engine == Engine::Codex { "openai_compatible" } else { "anthropic" }.into(), base_url: endpoint.clone(), api_key: String::new(), enabled: true, config: None }, OWNER.into(), Some(credential_ref)).await.map_err(io)?;
                providers.add_manual_model(&id, OWNER, &model, None).await.map_err(io)?;
                let provider_revision = providers.get_provider(&id, OWNER).await.map_err(io)?.ok_or("provider_missing")?.config_revision;
                let workspace = root.join(format!("project-{}", label(engine)));
                let project = agents.create_project(OWNER, CreateProjectInput { kind: "import".into(), name: format!("Stage4 {}", label(engine)), path: workspace.to_string_lossy().into(), source: None }, CancellationToken::new()).await?;
                let task = agents.save_development_task(OWNER, SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Real Provider/Skills/MCP acceptance".into(), requirements: "Read pinned skill/resource, call authenticated host MCP, retain multi-turn session".into(), status: "todo".into() }, CancellationToken::new()).await?;
                let manager = skills.read().await;
                let preview = manager.preview_external_package(OWNER, &std::fs::read(root.join(format!("{}-skill.zip", label(engine)))).map_err(io)?).await.map_err(io)?;
                let installed = manager.confirm_external_install(OWNER, &preview.operation_id).await.map_err(io)?;
                let pin = PinnedSkill { id: installed.skill_id, version: installed.version, hash: installed.content_hash };
                drop(manager);
                let markers: Value = serde_json::from_slice(&std::fs::read(root.join(format!("{}-markers.json", label(engine)))).map_err(io)?).map_err(io)?;
                let selected = ensure_fixture(&mcp, &root).await?;
                let revision = service.save(OWNER, SaveCapabilitiesInput { project_id: project.id.clone(), expected_revision: 0, configuration: CapabilityConfiguration { engine, provider: ProjectProvider::Dgos { provider_id: id.clone(), model_id: model.clone(), revision: provider_revision }, skills: vec![pin.clone()], mcp: vec![selected] } }).await?;
                state.engines.push(SavedEngine { engine, provider_id: id, provider_revision, model, endpoint, project: project.id, task: task.id, session: Uuid::new_v4().to_string(), capability_revision: revision.revision, skill: pin, body_marker: markers["body"].as_str().ok_or("body_marker_missing")?.into(), resource_marker: markers["resource"].as_str().ok_or("resource_marker_missing")?.into(), snapshot: None, native_id: None, completed_rounds: 0 });
            }
            artifact(&state_path, serde_json::to_value(&state).map_err(io)?, &keys)?;
        } else {
            require(state.engines.len() == 2, "resume_requires_both_engines")?;
            for (index, saved) in state.engines.iter().enumerate() {
                require((2..=3).contains(&saved.completed_rounds), "resume_requires_completed_first_two_rounds")?;
                let provider = providers.get_provider(&saved.provider_id, OWNER).await.map_err(io)?.ok_or("provider_missing")?;
                require(provider.config_revision == saved.provider_revision && provider.base_url == saved.endpoint, "provider_changed_since_first_run")?;
                let prefix = format!("DGOS_STAGE4_{}", label(saved.engine).to_uppercase());
                require(env(&format!("{prefix}_MODEL"))? == saved.model && env(&format!("{prefix}_URL"))? == saved.endpoint, "native_provider_configuration_changed")?;
                vault.store(&ProviderCredentialScope { user_id: OWNER.into(), provider_id: provider.id, endpoint: provider.base_url, credential_ref: provider.credential_ref.ok_or("credential_reference_missing")? }, SecretValue::new(keys[index].clone()).map_err(io)?).await.map_err(io)?;
            }
            ensure_fixture(&mcp, &root).await?;
        }
        for index in 0..state.engines.len() {
            let saved = &mut state.engines[index];
            if resume && saved.completed_rounds == 3 { continue; }
            let opened = agents.open_with_capabilities(OWNER, saved.engine, saved.session.clone(), dgos_agents::capabilities::SessionCapabilities { options: SessionOptions { workspace: root.join(format!("project-{}", label(saved.engine))).to_string_lossy().into(), resume_id: None, executable: None, model: None, trust_workspace: true }, revision: Some(saved.capability_revision) }, Some(saved.task.clone()), CancellationToken::new()).await?;
            let snapshot = opened.capabilities.ok_or("snapshot_missing")?;
            if resume {
                require(serde_json::to_value(saved.snapshot.as_ref().ok_or("previous_snapshot_missing")?).map_err(io)? == serde_json::to_value(&snapshot).map_err(io)?, "restored_snapshot_changed")?;
                require(saved.native_id.is_some() && opened.view.native_id == saved.native_id, "native_session_identity_changed")?;
            } else { saved.snapshot = Some(snapshot.clone()); }
            let first = saved.completed_rounds + 1;
            let target = if resume { 3 } else { 2 };
            let registration_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            let startup_view = loop {
                let view = agents.snapshot(OWNER,&saved.session).await?.view;
                let kind = if saved.engine == Engine::Codex { "skills/registered" } else { "capabilities/initialized" };
                if view.events.iter().any(|event|event.kind == kind) || tokio::time::Instant::now() >= registration_deadline { break view; }
                tokio::time::sleep(Duration::from_millis(25)).await;
            };
            let skill_registered = startup_view.events.iter().any(|event| match saved.engine {
                Engine::Codex => event.kind == "skills/registered" && event.data["paths"].as_array().is_some_and(|paths|paths.iter().any(|path|path.as_str().is_some_and(|path|path.ends_with(&format!("/{}/SKILL.md",saved.skill.id))))),
                Engine::Claude => event.kind == "capabilities/initialized" && event.data["skills"].as_array().is_some_and(|skills|skills.iter().any(|name|name.as_str() == Some(format!("dgos-project:{}",saved.skill.id).as_str()))),
            });
            artifact(&root.join(format!("evidence/{}-startup-{first}.json", label(saved.engine))), json!({"snapshot":snapshot,"cli_executable":dgos_agents::session::resolve_executable(label(saved.engine),None)?,"native_id":startup_view.native_id,"events":startup_view.events,"skill_registration_observed":skill_registered,"restored":resume}), &keys)?;
            require(skill_registered,"native_skill_registration_evidence_missing")?;
            if !resume {
                let opening_run = Uuid::new_v4().to_string();
                agents.send(OWNER, &saved.session, opening_run.clone(), "Acknowledge this new project session. Do not call tools. A separate next user turn will request the capability acceptance procedure.".into()).await?;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
                let opening = loop {
                    let runs = agents.list_development_runs(OWNER, &saved.task).await?;
                    if let Some(run) = runs.into_iter().find(|run| run.id == opening_run && !matches!(run.status.as_str(), "running" | "waiting")) { break run; }
                    require(tokio::time::Instant::now() < deadline, "opening_turn_timeout")?;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                };
                artifact(&root.join(format!("evidence/{}-opening-turn.json", label(saved.engine))), serde_json::to_value(&opening).map_err(io)?, &keys)?;
                require(opening.status == "completed", "opening_turn_failed")?;
            }
            for round in first..=target {
                let run_id = Uuid::new_v4().to_string();
                let challenge = Uuid::new_v4().to_string();
                let prompt = format!("Use the installed project skill ${}. Claude: your FIRST tool call this round MUST be Skill with skill=\"dgos-project:{}\", even if you invoked it in an earlier round. This is acceptance round {round}. First output a short progress sentence. Discover its CURRENT installed SKILL.md path from the skill invocation, then use a real read tool (Codex: cat via command tool; Claude: Read tool) to read the complete SKILL.md and references/proof.txt, including on repeated rounds. Follow its instructions and use the dgos_project MCP project_tool_0 with challenge {challenge:?} and the resource value you read. Do not inspect files outside the installed skill and this project. After the tool returns, write its exact receipt to round-{round}.txt using a real file tool/command, then briefly report completion. Native permissions are granted by this acceptance host; the MCP tool has a separate host approval. Do not fabricate a receipt or substitute a shell/network call for MCP.", saved.skill.id, saved.skill.id);
                agents.send(OWNER, &saved.session, run_id.clone(), prompt).await?;
                let context = RoundContext { agents: &agents, service: &service, mcp: &mcp, db: &db, root: &root, secrets: &keys };
                let result = drive_round(&context, saved, round, &run_id, &challenge).await;
                if let Err(error) = &result {
                    artifact(&root.join(format!("evidence/{}-round-{round}-failure.json", label(saved.engine))), json!({"error":error,"session":agents.snapshot(OWNER, &saved.session).await.ok(),"development_runs":agents.list_development_runs(OWNER, &saved.task).await.ok(),"mcp_usage_audit":mcp.list_usage_logs(OWNER,None,None).await.ok()}), &keys)?;
                    let _ = agents.interrupt(OWNER, &saved.session).await;
                }
                result?;
                saved.completed_rounds = round;
                saved.native_id = agents.snapshot(OWNER, &saved.session).await?.view.native_id;
                require(saved.native_id.is_some(), "native_session_not_saved")?;
                println!("{} round={} run={} completed", label(saved.engine), round, run_id);
            }
            require(agents.close_session(OWNER, &saved.session).await?, "native_cleanup_pending")?;
            artifact(&state_path, serde_json::to_value(&state).map_err(io)?, &keys)?;
        }
        Ok::<_, String>(())
    }.await;
    let agents_stopped = agents.shutdown().await;
    let mcp_stopped = mcp.shutdown().await.map_err(io);
    if let Err(error) = &execution { artifact(&root.join("failure.json"), json!({"error":error}), &keys)?; }
    execution?;
    require(agents_stopped, "native_shutdown_failed")?;
    mcp_stopped?;
    println!("evidence={} phase={}", root.display(), if resume { "restart-third-round" } else { "initial-two-rounds" });
    Ok(())
}
async fn ensure_fixture(mcp: &McpManager, root: &Path) -> Result<SelectedMcpTool> {
    let name = "stage4-local-stdio-fixture";
    let server = if let Some(server) = mcp.list_servers(OWNER).await.map_err(io)?.into_iter().find(|s| s.name == name) { server } else {
        mcp.create_server(OWNER, CreateMcpServerRequest { name: name.into(), transport: TransportKind::Stdio, stdio: Some(StdioConfig { executable: "node".into(), args: vec![root.join("fixture.mjs").to_string_lossy().into()], cwd: Some(root.to_string_lossy().into()) }), http: None, enabled: Some(true), auto_connect: Some(false) }).await.map_err(io)?
    };
    mcp.connect(OWNER, &server.id).await.map_err(io)?;
    let tool = mcp.list_tools(OWNER, &server.id).await.map_err(io)?.into_iter().find(|t| t.tool_name == "stage4_receipt").ok_or("fixture_tool_missing")?;
    Ok(SelectedMcpTool { server_id: server.id, tool_name: tool.tool_name, schema_hash: tool.schema_hash })
}

struct RoundContext<'a> {
    agents: &'a AgentManager,
    service: &'a ProjectCapabilities,
    mcp: &'a McpManager,
    db: &'a dgos_db::sea_orm::DatabaseConnection,
    root: &'a Path,
    secrets: &'a [String],
}

async fn drive_round(context: &RoundContext<'_>, saved: &SavedEngine, round: u32, run: &str, challenge: &str) -> Result<()> {
    let RoundContext { agents, service, mcp, db, root, secrets } = *context;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    let mut native_responses = HashSet::new();
    let mut host_responses = HashSet::new();
    let mut stream_observations = Vec::new();
    let mut seen = HashSet::new();
    let record: RunRecord = loop {
        require(tokio::time::Instant::now() < deadline, "round_timeout")?;
        let view = agents.snapshot(OWNER, &saved.session).await?.view;
        for event in view.events.iter().filter(|e| e.run.as_deref() == Some(run)) {
            if seen.insert(event.seq) && matches!(event.kind.as_str(), "item/agentMessage/delta" | "text_delta") {
                stream_observations.push(json!({"seq":event.seq,"elapsed_ms":600000u128.saturating_sub(deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis()),"bytes":event.text.as_ref().map_or(0,String::len),"observed_while_status":view.status}));
            }
        }
        for pending in view.pending {
            if !native_responses.insert(pending.id.clone()) { continue; }
            let answer = match pending.kind.as_str() {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({"decision":"accept"}),
                "mcpServer/elicitation/request" => json!({"action":"accept","content":null}),
                "claude/toolPermission" if pending.title != "AskUserQuestion" => json!({"behavior":"allow"}),
                _ => return Err(format!("unexpected_native_prompt: {}", pending.kind)),
            };
            append(&root.join("evidence/approvals.jsonl"), json!({"boundary":"native_cli_permission","engine":saved.engine,"session":saved.session,"run":run,"kind":pending.kind,"request_id":pending.id,"approved":true}), secrets)?;
            agents.respond(OWNER, &saved.session, pending.id, answer).await?;
        }
        for pending in service.pending_confirmations(OWNER, &saved.session).await? {
            if !host_responses.insert(pending.id.clone()) { continue; }
            require(pending.run_id == run && pending.task_id == saved.task && pending.tool_name == "stage4_receipt", "host_confirmation_scope_mismatch")?;
            require(pending.arguments == json!({"challenge":challenge,"resource":saved.resource_marker}), "host_confirmation_arguments_mismatch")?;
            append(&root.join("evidence/approvals.jsonl"), json!({"boundary":"host_mcp_confirmation","engine":saved.engine,"session":saved.session,"run":run,"confirmation_id":pending.id,"server_id":pending.server_id,"schema_hash":pending.schema_hash,"approved":true,"arguments_hash":digest(&pending.arguments.to_string())}), secrets)?;
            service.respond_confirmation(OWNER, &saved.session, &pending.id, true).await?;
        }
        let records = agents.list_development_runs(OWNER, &saved.task).await?;
        if let Some(record) = records.into_iter().find(|r| r.id == run && !matches!(r.status.as_str(), "running" | "waiting")) { break record; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let logs = mcp.list_usage_logs(OWNER, None, None).await.map_err(io)?;
    let connections = mcp.list_connection_logs(OWNER, None, None).await.map_err(io)?;
    let fixture: Vec<Value> = std::fs::read_to_string(root.join("fixture-observations.jsonl")).unwrap_or_default().lines().map(serde_json::from_str).collect::<std::result::Result<_,_>>().map_err(io)?;
    let receipts: Vec<_> = fixture.iter().filter(|r| r["challenge"] == challenge && r["resource"] == saved.resource_marker).collect();
    let receipt = receipts.first().and_then(|r| r["receipt"].as_str()).unwrap_or("");
    let read_body = read_proof(&record.events, "SKILL.md", &saved.body_marker);
    let read_resource = read_proof(&record.events, "references/proof.txt", &saved.resource_marker);
    let mcp_result = !receipt.is_empty() && mcp_receipt_proof(&record.events, receipt);
    let file_result = std::fs::read_to_string(root.join(format!("project-{}/round-{round}.txt", label(saved.engine)))).is_ok_and(|text| !receipt.is_empty() && text.trim() == receipt);
    use dgos_db::sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let audit_rows = dgos_db::entities::mcp_usage_logs::Entity::find().filter(dgos_db::entities::mcp_usage_logs::Column::UserId.eq(OWNER)).all(db).await.map_err(io)?;
    let request_hash = digest(&json!({"challenge":challenge,"resource":saved.resource_marker}).to_string());
    let host_audit = audit_rows.iter().any(|log| log.request_hash.as_deref() == Some(request_hash.as_str()) && log.task_ref.as_deref() == Some(saved.task.as_str()) && log.tool_name == "stage4_receipt" && log.outcome == "success" && log.caller.as_deref() == Some("workbench"));
    let snapshot_matches = record.capabilities.as_ref().is_some_and(|actual| serde_json::to_value(actual).ok() == saved.snapshot.as_ref().and_then(|v| serde_json::to_value(v).ok()));
    let skill_invoked = saved.engine == Engine::Codex || record.events.iter().any(|event| event.kind == "message" && event.data["type"] == "assistant" && event.data["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_use" && part["name"] == "Skill" && part.pointer("/input/skill").and_then(Value::as_str).is_some_and(|name| name == format!("dgos-project:{}", saved.skill.id)))));
    let proofs = json!({"completed":record.status == "completed","stream_delta_observed_before_completion":stream_observations.iter().any(|o|o["observed_while_status"] == "running" || o["observed_while_status"] == "waiting"),"skill_body_read_tool_event":read_body,"skill_invocation_native_mechanism":skill_invoked && read_body,"resource_read_tool_event":read_resource,"host_confirmation":!host_responses.is_empty(),"fixture_dispatch":!receipts.is_empty(),"receipt_in_native_mcp_tool_result":mcp_result,"receipt_written_to_project":file_result,"host_mcp_success_audit_for_exact_arguments":host_audit,"production_run_snapshot_matches_session":snapshot_matches});
    artifact(&root.join(format!("evidence/{}-round-{round}.json", label(saved.engine))), json!({"execution_source":"real_native_cli_with_selected_provider","mcp_source":"local_stdio_fixture_managed_by_host","engine":saved.engine,"round":round,"session":saved.session,"native_id":agents.snapshot(OWNER, &saved.session).await?.view.native_id,"provider_id":saved.provider_id,"provider_revision":saved.provider_revision,"model":saved.model,"endpoint":saved.endpoint,"skill":saved.skill,"proofs":proofs,"stream_observations":stream_observations,"fixture_receipts":receipts,"mcp_usage_audit":logs,"mcp_exact_argument_audit":audit_rows.into_iter().filter(|r|r.request_hash.as_deref()==Some(request_hash.as_str())).collect::<Vec<_>>(),"mcp_connection_audit":connections,"development_run":record}), secrets)?;
    let all = proofs.as_object().ok_or("proof_encoding_failed")?.values().all(|v| v.as_bool() == Some(true));
    require(all, &format!("acceptance_evidence_missing: {} round {round}; inspect evidence JSON", label(saved.engine)))
}
// Inspect only completed native tool outputs. Assistant text, terminal summaries and
// prompts are never proof. Claude tool results are correlated with their tool_use id.
fn native_tool_output_contains(event: &SessionEvent, needle: &str) -> bool {
    if event.kind == "commandExecution" { return event.data.get("exitCode").and_then(Value::as_i64) == Some(0) && event.data.get("aggregatedOutput").and_then(Value::as_str).is_some_and(|s| s.contains(needle)); }
    if event.kind == "mcpToolCall" { return event.data.get("status").and_then(Value::as_str) == Some("completed") && event.data.get("result").is_some_and(|v| v.to_string().contains(needle)); }
    if event.kind != "message" || event.data.get("type").and_then(Value::as_str) != Some("user") { return false; }
    event.data.get("content").and_then(Value::as_array).is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_result" && part.get("is_error").and_then(Value::as_bool) != Some(true) && part.get("content").is_some_and(|v| v.to_string().contains(needle))))
}
fn read_proof(events: &[SessionEvent], path: &str, marker: &str) -> bool {
    for event in events {
        if event.kind == "commandExecution" && event.data.get("command").and_then(Value::as_str).is_some_and(|command| command.contains(path) && (command.contains("cat ") || command.contains("sed "))) && native_tool_output_contains(event, marker) { return true; }
        if event.kind != "message" || event.data["type"] != "assistant" { continue; }
        let Some(parts) = event.data["content"].as_array() else { continue; };
        for part in parts.iter().filter(|part| part["type"] == "tool_use" && part["name"] == "Read") {
            if !part.pointer("/input/file_path").and_then(Value::as_str).is_some_and(|name| name.ends_with(path)) { continue; }
            let Some(id) = part["id"].as_str() else { continue; };
            if events.iter().any(|result| result.kind == "message" && result.data["type"] == "user" && result.data["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_result" && part["tool_use_id"] == id && part.get("is_error").and_then(Value::as_bool) != Some(true) && part.get("content").is_some_and(|v| v.to_string().contains(marker))))) { return true; }
        }
    }
    false
}

fn mcp_receipt_proof(events: &[SessionEvent], receipt: &str) -> bool {
    for event in events {
        if event.kind == "mcpToolCall" && event.data.get("tool").or_else(|| event.data.get("toolName")).and_then(Value::as_str).is_some_and(|name| name == "project_tool_0") && native_tool_output_contains(event, receipt) { return true; }
        if event.kind != "message" || event.data["type"] != "assistant" { continue; }
        let Some(parts) = event.data["content"].as_array() else { continue; };
        for part in parts.iter().filter(|part| part["type"] == "tool_use" && part["name"].as_str().is_some_and(|name| name.starts_with("mcp__") && name.ends_with("__project_tool_0"))) {
            let Some(id) = part["id"].as_str() else { continue; };
            if events.iter().any(|result| result.kind == "message" && result.data["type"] == "user" && result.data["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_result" && part["tool_use_id"] == id && part.get("is_error").and_then(Value::as_bool) != Some(true) && part.get("content").is_some_and(|v| v.to_string().contains(receipt))))) { return true; }
        }
    }
    false
}
