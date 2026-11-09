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
//! in each SAME native session and checks its saved Provider snapshot. Evidence:
//! state.json, evidence/<engine>-round-<n>.json, evidence/<engine>-startup-<n>.json,
//! evidence/approvals.jsonl, fixture-observations.jsonl, acceptance.sqlite.
//! All MCP output is a local fixture. All native CLI/model turns are real provider calls.
//! For no-paid AgentManager lifecycle coverage, run `bun scripts/stage4-provider-errors.ts`.
use async_trait::async_trait;
use dgos_agents::{capabilities::{CliProviderConfiguration, CliProviderRef, CliProviders, SaveCliProviderInput, SessionCreate, SessionConfigurationView}, native_extensions::{self, mcp::{Change as McpChange, Field as McpField, ServerPatch}, package::NativeSkillPackage, Access, HostContext, NativeExtensions, ResourceAction, ResourceSource, Scope as NativeScope, Target}, projects::{CreateProjectInput, SaveTaskInput}, AgentManager, Engine, RunRecord, SessionEvent, SessionOptions};
use dgos_db::{development_repository::CliProviderSnapshot, migrations::Migrator, sea_orm_migration::MigratorTrait, DevelopmentRepository, ProviderRepository};
use dgos_providers::{new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, ProviderManager, SecretValue};
use dgos_types::{DgosError, DgosResult, ProviderConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::{HashMap, HashSet}, io::Write, os::unix::fs::PermissionsExt, path::{Path, PathBuf}, sync::Arc, time::Duration};
use tokio::sync::Mutex;
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
#[derive(Serialize, Deserialize, Default)]
struct State { schema_version: u32, engines: Vec<SavedEngine> }
#[derive(Serialize, Deserialize)]
struct SavedEngine {
    engine: Engine, provider_id: String, provider_revision: i64, model: String, endpoint: String,
    project: String, task: String, session: String, provider_revision_head: i64,
    skill: String, resource_marker: String, body_marker: String,
    snapshot: Option<CliProviderSnapshot>, native_id: Option<String>, completed_rounds: u32,
    #[serde(default)] blocked: Option<String>,
}
fn label(engine: Engine) -> &'static str { match engine { Engine::Codex => "codex", Engine::Claude => "claude" } }
fn io(error: impl std::fmt::Display) -> String { format!("artifact_io: {error}") }
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
    for (name, subdir) in [("CODEX_HOME", "codex-home"), ("CLAUDE_CONFIG_DIR", "claude-home")] {
        require(PathBuf::from(env(name)?).canonicalize().map_err(io)? == root.join(subdir).canonicalize().map_err(io)?, "native_config_not_isolated")?;
    }
    let keys = [env("DGOS_STAGE4_CODEX_KEY")?, env("DGOS_STAGE4_CLAUDE_KEY")?];
    for name in ["DGOS_STAGE4_CODEX_KEY", "DGOS_STAGE4_CLAUDE_KEY", "OPENAI_API_KEY", "OPENAI_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "DGOS_PROVIDER_KEY", "DGOS_MCP_BEARER"] { std::env::remove_var(name); }
    let isolated_home = root.join("home");
    std::fs::create_dir_all(&isolated_home).map_err(io)?;
    std::fs::set_permissions(&isolated_home, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    std::env::set_var("HOME", &isolated_home);
    let url = format!("sqlite://{}?mode=rwc", root.join("acceptance.sqlite").display());
    let db = dgos_db::establish_connection(&url).await.map_err(io)?;
    Migrator::up(&db, None).await.map_err(io)?;
    let repository = DevelopmentRepository::new(db.clone());
    let vault = Arc::new(EphemeralVault::default());
    let providers = ProviderManager::with_vault(&url, vault.clone()).await.map_err(io)?;
    let service = CliProviders::new(repository.clone(), providers.clone())?;
    let agents = AgentManager::new(root.join("retired-session-index.json"), repository.clone(), service.clone());
    agents.load().await?;
    let state_path = root.join("state.json");
    let mut state: State = if resume { serde_json::from_slice(&std::fs::read(&state_path).map_err(io)?).map_err(io)? } else { require(!state_path.exists(), "state_exists: use --resume")?; State { schema_version: 2, engines: Vec::new() } };
    if resume { require(state.schema_version == 2, "resume_artifact_incompatible_legacy_format")?; }
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
                std::fs::create_dir_all(&workspace).map_err(io)?;
                std::fs::set_permissions(&workspace, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
                let project = agents.create_project(OWNER, CreateProjectInput { kind: "import".into(), name: format!("Stage4 {}", label(engine)), path: workspace.to_string_lossy().into(), source: None }, CancellationToken::new()).await?;
                let task = agents.save_development_task(OWNER, SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Real Provider/native Skills/MCP acceptance".into(), requirements: "Read the installed native Skill body and resource, discover the native MCP tool catalog, obtain native approval, and execute fresh receipts across rounds".into(), status: "todo".into() }, CancellationToken::new()).await?;
                let (skill, native_material) = install_native_fixture(&root, engine, &workspace)?;
                require(native_material["skill_committed"] == true && native_material["mcp_committed"] == true, "native_fixture_install_not_committed")?;
                let markers: Value = serde_json::from_slice(&std::fs::read(root.join(format!("{}-markers.json", label(engine)))).map_err(io)?).map_err(io)?;
                let revision = service.save(OWNER, SaveCliProviderInput { project_id: project.id.clone(), expected_revision: 0, configuration: CliProviderConfiguration { engine, provider: CliProviderRef { provider_id: id.clone(), model_id: model.clone(), provider_revision } } }).await?;
                state.engines.push(SavedEngine { engine, provider_id: id, provider_revision, model, endpoint, project: project.id, task: task.id, session: Uuid::new_v4().to_string(), provider_revision_head: revision.revision, skill, body_marker: markers["body"].as_str().ok_or("body_marker_missing")?.into(), resource_marker: markers["resource"].as_str().ok_or("resource_marker_missing")?.into(), snapshot: None, native_id: None, completed_rounds: 0, blocked: None });
                artifact(&root.join(format!("evidence/{}-native-install.json", label(engine))), native_material, &keys)?;
            }
            artifact(&state_path, serde_json::to_value(&state).map_err(io)?, &keys)?;
        } else {
            require(state.engines.len() == 2, "resume_requires_both_engines")?;
            for (index, saved) in state.engines.iter().enumerate() {
                if saved.blocked.is_some() { continue; }
                require((2..=3).contains(&saved.completed_rounds), "resume_requires_completed_first_two_rounds")?;
                let provider = providers.get_provider(&saved.provider_id, OWNER).await.map_err(io)?.ok_or("provider_missing")?;
                require(provider.config_revision == saved.provider_revision && provider.base_url == saved.endpoint, "provider_changed_since_first_run")?;
                let prefix = format!("DGOS_STAGE4_{}", label(saved.engine).to_uppercase());
                require(env(&format!("{prefix}_MODEL"))? == saved.model && env(&format!("{prefix}_URL"))? == saved.endpoint, "native_provider_configuration_changed")?;
                vault.store(&ProviderCredentialScope { user_id: OWNER.into(), provider_id: provider.id, endpoint: provider.base_url, credential_ref: provider.credential_ref.ok_or("credential_reference_missing")? }, SecretValue::new(keys[index].clone()).map_err(io)?).await.map_err(io)?;
            }
        }
        let mut engine_failures = Vec::new();
        for index in 0..state.engines.len() {
            let engine_result = async {
                let saved = &mut state.engines[index];
                if resume && saved.blocked.is_some() { return Err(format!("previously_blocked:{}", saved.blocked.as_deref().unwrap_or("unknown"))); }
                if resume && saved.completed_rounds == 3 { return Ok::<(), String>(()); }
                let workspace = root.join(format!("project-{}", label(saved.engine)));
            let opened = if resume { agents.resume_session(OWNER, &saved.session, true, CancellationToken::new()).await? } else { agents.open_session(OWNER, saved.engine, saved.session.clone(), SessionCreate { options: SessionOptions { workspace: workspace.to_string_lossy().into(), resume_id: None, executable: None, model: None, trust_workspace: true }, project_revision: saved.provider_revision_head, provider: CliProviderRef { provider_id: saved.provider_id.clone(), model_id: saved.model.clone(), provider_revision: saved.provider_revision } }, Some(saved.task.clone()), CancellationToken::new()).await? };
            let SessionConfigurationView::ProviderBound { snapshot, .. } = opened.configuration else { return Err("snapshot_missing".into()); };
            if resume {
                let old = saved.snapshot.as_ref().ok_or("previous_snapshot_missing")?;
                require(old.provider_id == snapshot.provider_id && old.model_id == snapshot.model_id && old.provider_revision == snapshot.provider_revision && old.revision == snapshot.revision, "restored_provider_snapshot_changed")?;
            } else { saved.snapshot = Some(snapshot.clone()); }
            let first = saved.completed_rounds + 1;
            let target = if resume { 3 } else { 2 };
            let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(120);
            loop {
                let view = agents.snapshot(OWNER, &saved.session).await?.view;
                if view.status == "idle" { break; }
                require(!matches!(view.status.as_str(), "error" | "unknown" | "closed") && tokio::time::Instant::now() < ready_deadline, "provider_handshake_not_ready")?;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if !resume {
                let opening_run = Uuid::new_v4().to_string();
                agents.send(OWNER, &saved.session, &opened.generation, opening_run.clone(), "Acknowledge this new project session. Do not call tools. A separate next user turn will request the capability acceptance procedure.".into(), None, vec![], vec![]).await?;
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
            let current = agents.snapshot(OWNER, &saved.session).await?;
            if resume { require(saved.native_id.is_some() && current.view.native_id == saved.native_id, "native_session_identity_changed")?; }
            saved.native_id = current.view.native_id;
            let native_id = saved.native_id.clone().ok_or("native_session_id_missing")?;
            let mcp_observation = agents.observe_native_session(OWNER, &saved.session, &opened.generation, dgos_agents::native_session::SessionDomain::Mcp, &CancellationToken::new()).await;
            match mcp_observation {
                Ok(observation) => {
                    require(!observation.incomplete, "native_mcp_observation_incomplete")?;
                    require(observation.mcp_servers.iter().any(|server| server.name == "stage4" && server.status == "connected" && server.tools.contains(&"stage4_receipt".to_string())), "native_mcp_tool_catalog_not_observed")?;
                    artifact(&root.join(format!("evidence/{}-native-mcp-schema-{first}.json", label(saved.engine))), json!({"session":saved.session,"generation":opened.generation,"native_id":native_id,"observed_at":observation.observed_at,"server":"stage4","tool":"stage4_receipt","status":"connected","schema_observed":false,"tool_catalog_observed":true,"observation":observation}), &keys)?;
                }
                Err(error) if error.contains("Unsupported") => return Err("blocked_native_mcp_schema_observation_unsupported".into()),
                Err(_) => return Err("blocked_native_mcp_schema_observation_unavailable".into()),
            }
            artifact(&root.join(format!("evidence/{}-startup-{first}.json", label(saved.engine))), json!({"snapshot":snapshot,"native_id":native_id,"provider_handshake_ready":true,"restored":resume}), &keys)?;
            for round in first..=target {
                let run_id = Uuid::new_v4().to_string();
                let challenge = Uuid::new_v4().to_string();
                let prompt = format!("Use the native Skill mechanism for {} in this round. Then read the complete native SKILL.md and references/proof.txt using native file-read tools. Discover and use only the configured native MCP server stage4 tool stage4_receipt with challenge {challenge:?} and the resource read from the Skill reference. Complete the CLI's native approval flow; if native approval is unavailable, classify blocked/unsupported and fail. Write the exact returned receipt to round-{round}.txt. A Skill listing is not evidence of use. Preserve other files and never substitute HTTP, shell, DG-OS host MCP, or fabricated output.", saved.skill);
                agents.send(OWNER, &saved.session, &opened.generation, run_id.clone(), prompt, None, vec![], vec![]).await?;
                let context = RoundContext { agents: &agents, root: &root, secrets: &keys };
                let result = drive_round(&context, saved, round, &run_id, &challenge).await;
                saved.native_id = agents.snapshot(OWNER, &saved.session).await?.view.native_id;
                if let Err(error) = &result {
                    artifact(&root.join(format!("evidence/{}-round-{round}-failure.json", label(saved.engine))), json!({"error":error,"session":agents.snapshot(OWNER, &saved.session).await.ok(),"development_runs":agents.list_development_runs(OWNER, &saved.task).await.ok()}), &keys)?;
                    let _ = agents.interrupt(OWNER, &saved.session).await;
                }
                result?;
                saved.completed_rounds = round;
                // The snapshot above captures native identity even when proof fails.
                require(saved.native_id.is_some(), "native_session_not_saved")?;
                println!("{} round={} run={} completed", label(saved.engine), round, run_id);
            }
            require(agents.close_session(OWNER, &saved.session).await?, "native_cleanup_pending")?;
            Ok::<(), String>(())
            }.await;
            if let Err(error) = engine_result {
                let engine = label(state.engines[index].engine);
                state.engines[index].blocked = Some(error.clone());
                engine_failures.push(format!("{engine}:{error}"));
                let _ = agents.interrupt(OWNER, &state.engines[index].session).await;
                let _ = agents.close_session(OWNER, &state.engines[index].session).await;
            }
            artifact(&state_path, serde_json::to_value(&state).map_err(io)?, &keys)?;
            if state.engines[index].blocked.is_some() {
                let engine = label(state.engines[index].engine);
                artifact(&root.join(format!("evidence/{engine}-blocked.json")), json!({"engine":engine,"classification":state.engines[index].blocked,"completed_rounds":state.engines[index].completed_rounds,"native_id":state.engines[index].native_id,"resume_supported":false}), &keys)?;
            }
        }
        if !engine_failures.is_empty() { return Err(format!("native_acceptance_blocked: {}", engine_failures.join(","))); }
        Ok::<_, String>(())
    }.await;
    let agents_stopped = agents.shutdown().await;
    if let Err(error) = &execution { artifact(&root.join("failure.json"), json!({"error":error}), &keys)?; }
    execution?;
    require(agents_stopped, "native_shutdown_failed")?;
    println!("evidence={} phase={}", root.display(), if resume { "restart-third-round" } else { "initial-two-rounds" });
    Ok(())
}
fn install_native_fixture(root: &Path, engine: Engine, workspace: &Path) -> Result<(String, Value)> {
    use std::os::unix::fs::PermissionsExt;
    use native_extensions::State as NativeState;
    let engine_label = label(engine);
    let native_root = root.join(format!("native-{engine_label}"));
    let claude_home = root.join("claude-home");
    let journal = native_root.join("journal");
    for directory in [&native_root, &journal] { std::fs::create_dir_all(directory).map_err(io)?; std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(io)?; }
    let context = HostContext::from_host(OWNER.into(), format!("stage4-{engine_label}"), native_extensions::TrustedRoots {
        os_home: root.join("home"), codex_home: root.join("codex-home"), claude_home: claude_home.clone(), claude_user_file: claude_home.join(".claude.json"), project: Some(workspace.to_path_buf()), journal,
    }, Access::Manage).map_err(io)?;
    let api = NativeExtensions::new(context).map_err(io)?;
    let target = if engine == Engine::Codex { Target::Codex } else { Target::Claude };
    let skill = format!("stage4-{engine_label}-proof");
    let bytes = std::fs::read(root.join(format!("{engine_label}-skill.zip"))).map_err(io)?;
    let package = NativeSkillPackage::from_zip(&bytes, None).map_err(io)?;
    let source = ResourceSource { source_hash: package.content_hash().to_owned(), commit: None };
    let preview = api.preview_resource(target, NativeScope::ProjectShared, &skill, ResourceAction::Install { package, source }).map_err(io)?;
    let skill_receipt = api.commit_resource(&preview.receipt.operation_id).map_err(io)?;
    require(skill_receipt.state == NativeState::Committed, "native_skill_install_failed_or_unknown")?;
    let patch = ServerPatch { program: McpField::Set("node".into()), args: McpField::Set(vec![root.join("fixture.mjs").to_string_lossy().into_owned(), engine_label.into(), root.to_string_lossy().into_owned()]), cwd: if engine == Engine::Codex { McpField::Set(root.to_string_lossy().into_owned()) } else { McpField::Keep }, ..Default::default() };
    let preview = api.preview_mcp(target, NativeScope::ProjectShared, McpChange::Upsert { name: "stage4".into(), fields: patch }).map_err(io)?;
    let mcp_receipt = api.commit(&preview.receipt.operation_id).map_err(io)?;
    require(mcp_receipt.state == NativeState::Committed, "native_mcp_config_failed_or_unknown")?;
    let skills = api.discover_resources(target, NativeScope::ProjectShared, &skill).map_err(io)?;
    let servers = api.discover_mcp(target, NativeScope::ProjectShared).map_err(io)?;
    require(!skills.incomplete && skills.instances.iter().any(|item| item.slug == skill && item.owner == native_extensions::Owner::Loose), "native_skill_discovery_unproven")?;
    require(!servers.incomplete && servers.instances.iter().any(|item| item.native_key == "stage4" && item.owner == native_extensions::Owner::Loose), "native_mcp_config_discovery_unproven")?;
    Ok((skill, json!({"target":engine,"scope":"project_shared","skill_operation":skill_receipt,"mcp_operation":mcp_receipt,"skill_committed":true,"mcp_committed":true,"skill_discovered":true,"mcp_configuration_discovered":true,"native_config_paths":{"codex":".codex/config.toml","claude":".mcp.json"},"native_skill_paths":{"codex":".agents/skills","claude":".claude/skills"},"schema_discovery":"requires_session_observation","execution":"not_attempted"})))
}

struct RoundContext<'a> {
    agents: &'a AgentManager,
    root: &'a Path,
    secrets: &'a [String],
}

async fn drive_round(context: &RoundContext<'_>, saved: &SavedEngine, round: u32, run: &str, challenge: &str) -> Result<()> {
    let RoundContext { agents, root, secrets } = *context;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    let mut native_responses = HashSet::new();
    let mut approved_mcp_requests = Vec::new();
    let mut approved_mcp_tool_uses = HashSet::new();
    let mut stream_observations = Vec::new();
    let mut seen = HashSet::new();
    let record: RunRecord = loop {
        require(tokio::time::Instant::now() < deadline, "round_timeout")?;
        let session = agents.snapshot(OWNER, &saved.session).await?;
        let view = session.view;
        for event in view.events.iter().filter(|e| e.run.as_deref() == Some(run)) {
            if seen.insert(event.seq) && matches!(event.kind.as_str(), "item/agentMessage/delta" | "text_delta") {
                stream_observations.push(json!({"seq":event.seq,"elapsed_ms":600000u128.saturating_sub(deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis()),"bytes":event.text.as_ref().map_or(0,String::len),"observed_while_status":view.status}));
            }
        }
        for pending in view.pending {
            if !native_responses.insert(pending.id.clone()) { continue; }
            let mcp_tool = pending.data.get("tool").and_then(Value::as_str).is_some_and(|tool| tool == "mcp__stage4__stage4_receipt");
            let mcp_approval = pending.kind == "claude/toolPermission" && mcp_tool
                && pending.data.pointer("/input/challenge").and_then(Value::as_str) == Some(challenge)
                && pending.data.pointer("/input/resource").and_then(Value::as_str) == Some(saved.resource_marker.as_str())
                && pending.data.get("tool_use_id").and_then(Value::as_str).is_some();
            let answer = match pending.kind.as_str() {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({"decision":"accept"}),
                "mcpServer/elicitation/request" => json!({"action":"accept","content":null}),
                "claude/toolPermission" if pending.title.starts_with("mcp__") && !mcp_approval => return Err("blocked_native_mcp_approval_identity_mismatch".into()),
                "claude/toolPermission" if pending.title != "AskUserQuestion" => json!({"behavior":"allow"}),
                _ => return Err(format!("blocked_native_approval_unsupported: {}", pending.kind)),
            };
            agents.respond(OWNER, &saved.session, &session.generation, pending.id.clone(), answer).await?;
            append(&root.join("evidence/approvals.jsonl"), json!({"boundary":"native_cli_permission","engine":saved.engine,"session":saved.session,"run":run,"kind":pending.kind,"request_id":pending.id,"mcp_tool_approval":mcp_approval,"approved":true}), secrets)?;
            if mcp_approval {
                approved_mcp_requests.push(pending.id);
                if let Some(id) = pending.data.get("tool_use_id").and_then(Value::as_str) { approved_mcp_tool_uses.insert(id.to_owned()); }
            }
        }
        let records = agents.list_development_runs(OWNER, &saved.task).await?;
        if let Some(record) = records.into_iter().find(|r| r.id == run && !matches!(r.status.as_str(), "running" | "waiting")) { break record; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let fixture: Vec<Value> = std::fs::read_to_string(root.join("fixture-observations.jsonl")).unwrap_or_default().lines().map(serde_json::from_str).collect::<std::result::Result<_,_>>().map_err(io)?;
    let receipts: Vec<_> = fixture.iter().filter(|r| r["engine"].as_str() == Some(label(saved.engine)) && r["challenge"] == challenge && r["resource"] == saved.resource_marker && r["kind"] == "execution").collect();
    let receipt = receipts.first().and_then(|r| r["receipt"].as_str()).unwrap_or("");
    let read_body = read_proof(&record.events, "SKILL.md", &saved.body_marker);
    let read_resource = read_proof(&record.events, "references/proof.txt", &saved.resource_marker);
    let mcp_result = !receipt.is_empty() && mcp_receipt_proof(&record.events, receipt, &approved_mcp_tool_uses);
    let file_result = std::fs::read_to_string(root.join(format!("project-{}/round-{round}.txt", label(saved.engine)))).is_ok_and(|text| !receipt.is_empty() && text.trim() == receipt);
    let snapshot_matches = record.configuration.as_ref().is_some_and(|actual| matches!(actual, SessionConfigurationView::ProviderBound { snapshot, .. } if saved.snapshot.as_ref().is_some_and(|old| old.provider_id == snapshot.provider_id && old.provider_revision == snapshot.provider_revision && old.model_id == snapshot.model_id && old.revision == snapshot.revision)));
    let skill_invoked = saved.engine == Engine::Claude && record.events.iter().any(|event| event.kind == "message" && event.data["type"] == "assistant" && event.data["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_use" && part["name"] == "Skill" && part.pointer("/input/skill").and_then(Value::as_str) == Some(saved.skill.as_str()))));
    let schema_discovery = fixture.iter().any(|r| r["engine"].as_str() == Some(label(saved.engine)) && r["kind"] == "schema_discovery" && r["tool"] == "stage4_receipt");
    let native_mcp_approval = !approved_mcp_requests.is_empty();
    // The pinned protocols do not expose an independent activation receipt.
    // A Claude Skill tool request is useful attribution, not proof of a read.
    let proofs = json!({"completed":record.status == "completed","stream_delta_observed_before_completion":stream_observations.iter().any(|o|o["observed_while_status"] == "running" || o["observed_while_status"] == "waiting"),"skill_body_read_tool_event":read_body,"resource_read_tool_event":read_resource,"native_mcp_schema_discovery":schema_discovery,"fixture_dispatch":receipts.len() == 1,"native_mcp_approval_observed":native_mcp_approval,"receipt_in_native_mcp_tool_result":mcp_result,"receipt_written_to_project":file_result,"production_run_snapshot_matches_session":snapshot_matches});
    let unsupported = if !native_mcp_approval { Some("blocked_native_mcp_tool_approval_unobservable") } else { None };
    let skill_observation = json!({"independent_activation_receipt":"unsupported","structured_skill_input":"not_submitted_text_only_product_entry","native_skill_tool_request_observed":skill_invoked,"body_read_observed":read_body,"resource_read_observed":read_resource});
    artifact(&root.join(format!("evidence/{}-round-{round}.json", label(saved.engine))), json!({"execution_source":"real_native_cli_with_selected_provider","mcp_source":"native_local_stdio_fixture","engine":saved.engine,"round":round,"session":saved.session,"skill":saved.skill,"skill_observation":skill_observation,"capability_classification":unsupported,"proofs":proofs,"stream_observations":stream_observations,"fixture_receipts":receipts,"native_mcp_approval_request_ids":approved_mcp_requests,"development_run":record}), secrets)?;
    if let Some(reason) = unsupported { return Err(reason.into()); }
    let all = proofs.as_object().ok_or("proof_encoding_failed")?.values().all(|v| v.as_bool() == Some(true));
    require(all, &format!("acceptance_evidence_missing: {} round {round}", label(saved.engine)))
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

fn mcp_receipt_proof(events: &[SessionEvent], receipt: &str, approved_tool_uses: &HashSet<String>) -> bool {
    for event in events {
        if event.kind == "mcpToolCall" && event.data.get("tool").or_else(|| event.data.get("toolName")).and_then(Value::as_str) == Some("stage4_receipt") && native_tool_output_contains(event, receipt) { return true; }
        if event.kind != "message" || event.data["type"] != "assistant" { continue; }
        let Some(parts) = event.data["content"].as_array() else { continue; };
        for part in parts.iter().filter(|part| part["type"] == "tool_use" && part["name"] == "mcp__stage4__stage4_receipt" && part["id"].as_str().is_some_and(|id| approved_tool_uses.contains(id))) {
            let Some(id) = part["id"].as_str() else { continue; };
            if events.iter().any(|result| result.kind == "message" && result.data["type"] == "user" && result.data["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_result" && part["tool_use_id"] == id && part.get("is_error").and_then(Value::as_bool) != Some(true) && part.get("content").is_some_and(|v| v.to_string().contains(receipt))))) { return true; }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: &str, data: Value) -> SessionEvent { SessionEvent { seq: 1, kind: kind.into(), text: None, data, run: Some("fixture-run".into()) } }
    #[test]
    fn skill_reads_require_tool_output_not_a_claim_or_activation_label() {
        let claim = event("agentMessage", json!({"text":"BODY_RESOURCE","skill":"fixture","activated":true}));
        assert!(!read_proof(&[claim], "SKILL.md", "BODY_RESOURCE"));
        let mut read = event("commandExecution", json!({"command":"cat .agents/skills/fixture/SKILL.md", "exitCode":0,"aggregatedOutput":"BODY_RESOURCE"}));
        assert!(read_proof(&[read.clone()], "SKILL.md", "BODY_RESOURCE"));
        assert!(!read_proof(&[read.clone()], "references/proof.txt", "BODY_RESOURCE"));
        read.data["exitCode"] = json!(1);
        assert!(!read_proof(&[read], "SKILL.md", "BODY_RESOURCE"));
        let request = event("message", json!({"type":"assistant","content":[{"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"/fixture/SKILL.md"}}]}));
        let mut result = event("message", json!({"type":"user","content":[{"type":"tool_result","tool_use_id":"other-read","content":"BODY_RESOURCE"}]}));
        assert!(!read_proof(&[request.clone(), result.clone()], "SKILL.md", "BODY_RESOURCE"));
        result.data["content"][0]["tool_use_id"] = json!("read-1");
        assert!(read_proof(&[request, result], "SKILL.md", "BODY_RESOURCE"));
    }
}
