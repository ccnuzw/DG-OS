//! Real Stage 4 desktop acceptance; production DOM, IPC, SQLite and capability service.
//!
//! Prerequisites: built dist/agent bridge, explicit provider environment, desktop session.
//! The launcher supplies fixture packages and isolated CODEX_HOME/CLAUDE_CONFIG_DIR.
//! From the repository root, export all six DGOS_STAGE4_{CODEX,CLAUDE}_{URL,KEY,MODEL}
//! variables, then run: bun scripts/stage4-acceptance.ts --desktop
//! Or launch the desktop test directly against prepared fixture/config directories:
//! DGOS_STAGE4_FIXTURE_ROOT=/absolute/bootstrap-artifact \
//! DGOS_PROJECT_ACCEPTANCE_ROOT=/absolute/fresh/desktop-artifact \
//! DGOS_PROJECT_ACCEPTANCE_PHASE=stage4-ui \
//! cargo test -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
//!
//! Required environment: DGOS_STAGE4_{CODEX,CLAUDE}_{URL,KEY,MODEL}. Keys are consumed
//! into an ephemeral vault, removed from the environment, never sent to the renderer.
//! Evidence: <desktop-artifact>/stage4-ui-result.json and stage4-ui-<engine>.json;
//! dgos.db contains real revisions, sessions, run snapshots and MCP usage audit.
//! Four real model turns (two per engine): first an explicit acknowledgement, then
//! two fixture calls in the next turn. Approve the first, revoke the second.
//! This checks desktop behavior; the independent Rust harness owns full model proof.
//!
//! The launcher reads only the six explicit provider environment variables; it does
//! not inspect native global CLI configuration. Keep keys out of command arguments,
//! files, renderer DTOs and logs. See the launcher's printed artifact root for evidence.
use super::*;
use async_trait::async_trait;
use dgos_agents::{capabilities::ProjectCapabilities, Engine};
use dgos_db::ProviderRepository;
use dgos_mcp::{CreateMcpServerRequest, McpCallPolicy, StdioConfig, TransportKind};
use dgos_providers::{new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, ProviderManager, SecretValue};
use dgos_skills::SkillsManager;
use dgos_types::{DgosError, DgosResult, ProviderConfig};
use std::collections::HashMap;
use tokio::sync::{Mutex, RwLock};

#[derive(Default)]
struct Vault(Mutex<HashMap<String, SecretValue>>);
#[async_trait]
impl ProviderCredentialVault for Vault {
    async fn store(&self, scope: &ProviderCredentialScope, secret: SecretValue) -> DgosResult<()> { self.0.lock().await.insert(scope.account(), secret); Ok(()) }
    async fn load(&self, scope: &ProviderCredentialScope) -> DgosResult<SecretValue> {
        let values = self.0.lock().await;
        SecretValue::new(values.get(&scope.account()).ok_or(DgosError::CredentialUnavailable)?.expose().into())
    }
    async fn remove(&self, scope: &ProviderCredentialScope) -> DgosResult<()> { self.0.lock().await.remove(&scope.account()); Ok(()) }
}
struct FixturePolicy;
#[async_trait]
impl McpCallPolicy for FixturePolicy {
    async fn allows_call(&self, _: &str, _: &str, tool: &str) -> bool { tool == "stage4_ui_receipt" }
}
struct ProviderSeed { engine: Engine, endpoint: String, model: String, secret: SecretValue }
#[derive(Clone)]
struct Services {
    providers: ProviderManager,
    vault: Arc<Vault>,
    skills: Arc<RwLock<SkillsManager>>,
    mcp: Arc<McpManager>,
    seeds: Arc<Mutex<Vec<ProviderSeed>>>,
}

/// Same production construction as lib.rs, with a process-local credential vault.
/// The exact same MCP manager also participates in account disconnect cleanup.
pub(super) async fn install(app: &AppHandle, fx: &Fx) -> Arc<McpManager> {
    let mut seeds = Vec::new();
    for (engine, label) in [(Engine::Codex, "CODEX"), (Engine::Claude, "CLAUDE")] {
        let take = |suffix: &str| {
            let name = format!("DGOS_STAGE4_{label}_{suffix}");
            let value = std::env::var(&name).unwrap_or_default();
            if suffix == "KEY" { std::env::remove_var(name); }
            value
        };
        seeds.push(ProviderSeed { engine, endpoint: take("URL"), model: take("MODEL"), secret: SecretValue::new(take("KEY")).expect("stage4 provider environment required") });
    }
    for key in ["OPENAI_API_KEY", "OPENAI_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "DGOS_PROVIDER_KEY", "DGOS_MCP_BEARER"] { std::env::remove_var(key); }
    let vault = Arc::new(Vault::default());
    let url = format!("sqlite://{}?mode=rwc", fx.root.join("dgos.db").display());
    let providers = ProviderManager::with_vault(&url, vault.clone()).await.expect("stage4 provider manager");
    let skills = Arc::new(RwLock::new(SkillsManager::new(fx.db.clone(), fx.root.join("skills"))));
    let mcp = Arc::new(McpManager::with_policy(fx.db.clone(), Arc::new(FixturePolicy)));
    let capabilities = ProjectCapabilities::new(DevelopmentRepository::new(fx.db.clone()), providers.clone(), skills.clone(), mcp.clone(), fx.root.join("capability-material"));
    app.state::<AgentManager>().install_capabilities(capabilities.clone());
    app.manage(capabilities);
    app.manage(Services { providers, vault, skills, mcp: mcp.clone(), seeds: Arc::new(Mutex::new(seeds)) });
    mcp
}

const FIXTURE: &str = r#"import readline from 'node:readline';
import {appendFileSync} from 'node:fs';
import {randomUUID} from 'node:crypto';
let ready=false;
const reply=(id,result)=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,result})+'\n');
readline.createInterface({input:process.stdin}).on('line',line=>{
 const r=JSON.parse(line);
 if(r.method==='initialize')reply(r.id,{protocolVersion:'2025-03-26',capabilities:{tools:{}},serverInfo:{name:'stage4-ui-local-fixture',version:'1.0.0'}});
 else if(r.method==='notifications/initialized')ready=true;
 else if(r.method==='ping')reply(r.id,{});
 else if(r.method==='tools/list'&&ready)reply(r.id,{tools:[{name:'stage4_ui_receipt',description:'Local desktop acceptance fixture. Requires separate host confirmation.',inputSchema:{type:'object',required:['challenge'],additionalProperties:false,properties:{challenge:{type:'string'}}}}]});
 else if(r.method==='tools/call'){
  if(r.params.name!=='stage4_ui_receipt')return;
  const receipt='UI_RECEIPT_'+randomUUID();
  appendFileSync('stage4-ui-fixture.jsonl',JSON.stringify({challenge:r.params.arguments.challenge,receipt})+'\n',{mode:0o600});
  reply(r.id,{isError:false,content:[{type:'text',text:receipt}]});
 }
});
"#;

async fn seed(app: &AppHandle, fx: &Fx, owner: &str) -> Result<Vec<Value>, String> {
    let services = app.state::<Services>().inner().clone();
    let source = PathBuf::from(std::env::var_os("DGOS_STAGE4_FIXTURE_ROOT").ok_or("missing_fixture_root")?);
    fs::write(fx.root.join("stage4-ui-fixture.mjs"), FIXTURE).map_err(|_| "fixture_write_failed")?;
    let server = services.mcp.create_server(owner, CreateMcpServerRequest {
        name: "Stage4 UI local fixture".into(), transport: TransportKind::Stdio,
        stdio: Some(StdioConfig { executable: "node".into(), args: vec![fx.root.join("stage4-ui-fixture.mjs").to_string_lossy().into()], cwd: Some(fx.root.to_string_lossy().into()) }),
        http: None, enabled: Some(true), auto_connect: Some(false),
    }).await.map_err(|_| "fixture_create_failed")?;
    services.mcp.connect(owner, &server.id).await.map_err(|_| "fixture_connect_failed")?;
    let tool = services.mcp.list_tools(owner, &server.id).await.map_err(|_| "fixture_tools_failed")?.into_iter().find(|tool| tool.tool_name == "stage4_ui_receipt").ok_or("fixture_tool_missing")?;
    let mut rows = Vec::new();
    for seed in services.seeds.lock().await.drain(..) {
        let label = if seed.engine == Engine::Codex { "codex" } else { "claude" };
        if seed.endpoint.is_empty() || seed.model.is_empty() { return Err("missing_provider_metadata".into()); }
        let id = uuid::Uuid::new_v4().to_string();
        let credential_ref = new_credential_ref();
        services.vault.store(&ProviderCredentialScope { user_id: owner.into(), provider_id: id.clone(), endpoint: seed.endpoint.clone(), credential_ref: credential_ref.clone() }, seed.secret).await.map_err(|_| "vault_failed")?;
        ProviderRepository::new(fx.db.clone()).create_with_credential_ref(id.clone(), ProviderConfig { name: format!("Stage4 UI {label}"), provider_type: if seed.engine == Engine::Codex { "openai_compatible" } else { "anthropic" }.into(), base_url: seed.endpoint, api_key: String::new(), enabled: true, config: None }, owner.into(), Some(credential_ref)).await.map_err(|_| "provider_seed_failed")?;
        services.providers.add_manual_model(&id, owner, &seed.model, None).await.map_err(|_| "model_seed_failed")?;
        let provider = services.providers.get_provider(&id, owner).await.map_err(|_| "provider_read_failed")?.ok_or("provider_missing")?;
        let manager = services.skills.read().await;
        let bytes = fs::read(source.join(format!("{label}-skill.zip"))).map_err(|_| "skill_fixture_missing")?;
        let preview = manager.preview_external_package(owner, &bytes).await.map_err(|_| "skill_preview_failed")?;
        let skill = manager.confirm_external_install(owner, &preview.operation_id).await.map_err(|_| "skill_install_failed")?;
        rows.push(json!({"engine":label,"provider_id":id,"provider_revision":provider.config_revision,"model":seed.model,"skill_id":skill.skill_id,"skill_version":skill.version,"skill_hash":skill.content_hash,"server_id":server.id,"tool_name":tool.tool_name,"schema_hash":tool.schema_hash}));
    }
    Ok(rows)
}

async fn tab(app: &AppHandle, name: &str) -> Value {
    ui(app, json!({"name":name}), r#"const t=await need('dev-review-tab-'+A.name);t.focus();t.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));return {ok:!!(await until(()=>t.getAttribute('data-state')==='active'))};"#).await
}
async fn read(app: &AppHandle, project: &str) -> Reply { ipc(app, "main", "read_development_capabilities", json!({"projectId":project})).await }
fn write_json(path: impl AsRef<Path>, payload: &Value) { fs::write(path, serde_json::to_vec_pretty(payload).unwrap()).expect("stage4 evidence"); }

const CONFIGURE: &str = r#"
const section=await until(()=>document.querySelector('section[aria-label="项目能力"]'));
if(!section)throw Error('capability editor absent');
const engine=await until(()=>$$('[role=combobox]',section).find(e=>labelOf(e)==='智能体'));
if(!engine)throw Error('capability engine absent');
engine.click();(await until(()=>$$('[role=option]').find(e=>txt(e)===(A.engine==='codex'?'Codex':'Claude')))).click();
const source=await until(()=>$$('select',section).find(e=>[...e.options].some(o=>o.value===A.provider_id)));
if(!source)throw Error('provider not in production catalog');
Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(source,A.provider_id);source.dispatchEvent(new Event('change',{bubbles:true}));
await until(()=>$$('select',section).some(e=>e.value===A.model));
for(const marker of [A.skill_hash,A.schema_hash]){
 const label=await until(()=>$$('label',section).find(e=>txt(e).includes(marker)));
 if(!label)throw Error('catalog pin absent');const input=label.querySelector('input');if(!input.checked)input.click();
 await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
}
const save=await until(()=>$$('button',section).find(b=>txt(b)==='保存新修订'&&!b.disabled));if(!save)throw Error('save disabled');save.click();
return {ok:!!(await until(()=>txt(section).includes('项目能力修订已保存')))};
"#;

async fn pending(app: &AppHandle, sid: &str, seen: &str, from_seq: u64) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let reply = ipc(app, "main", "list_development_capability_confirmations", json!({"sessionId":sid})).await;
        if let Some(item) = value(&reply).as_array().into_iter().flatten().find(|item| item["id"].as_str() != Some(seen)) { return item.clone(); }
        let session = value(&ipc(app, "main", "get_agent_session", json!({"sessionId":sid})).await);
        // Native CLI permissions and the host MCP decision are separate boundaries.
        for request in session["pending"].as_array().into_iter().flatten() {
            let response = match request["kind"].as_str().unwrap_or_default() {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({"decision":"accept"}),
                "mcpServer/elicitation/request" => json!({"action":"accept","content":null}),
                "claude/toolPermission" if request["title"] != "AskUserQuestion" => json!({"behavior":"allow"}),
                _ => continue,
            };
            ipc(app, "main", "respond_agent_request", json!({"input":{"session_id":sid,"request_id":request["id"],"response":response}})).await;
        }
        let terminal_without_request = session["pending"].as_array().is_some_and(Vec::is_empty)
            && session["active_run"].is_null()
            && matches!(session["status"].as_str(), Some("idle"|"closed"|"error"|"unknown"))
            && session["events"].as_array().is_some_and(|events| events.iter().any(|event| event["seq"].as_u64().unwrap_or(0) > from_seq && matches!(event["kind"].as_str(), Some("turn/completed"|"result"|"failure"|"error"))));
        if terminal_without_request { return Value::Null; }
        if session["status"] == "error" || tokio::time::Instant::now() >= deadline { return Value::Null; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn per_engine(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, row: &Value) -> Value {
    let engine = row["engine"].as_str().unwrap();
    let directory = fx.root.join(format!("stage4-ui-{engine}"));make_repo(&directory);
    ui_import_project(app, ck, ev, &format!("Stage4 UI {engine}"), &directory).await;
    let goal = format!("Stage4 UI capabilities {engine}");ui_new_task(app, ck, ev, &goal).await;
    let selection = value(&ipc(app, "main", "read_development_workspace", json!({})).await)["selection"].clone();
    let project = selection["project_id"].as_str().unwrap_or_default();
    let task = selection["task_id"].as_str().unwrap_or_default();
    let opened = tab(app, "capabilities").await;
    record(ck, ev, "project capability tab opens", ui_ok(&opened), opened.to_string());
    let configured = ui(app, row.clone(), CONFIGURE).await;
    record(ck, ev, "catalog provider model skill and MCP selected and saved through DOM", ui_ok(&configured), configured.to_string());
    let revision = read(app, project).await;
    let saved = value(&revision);
    record(ck, ev, "production IPC reads exact UI saved content pins", succeeded(&revision) && saved["revision"] == 1 && saved["configuration"]["provider"]["provider_id"] == row["provider_id"] && saved["configuration"]["skills"][0]["hash"] == row["skill_hash"] && saved["configuration"]["mcp"][0]["schema_hash"] == row["schema_hash"], saved.to_string());
    let stale = ipc(app, "main", "save_development_capabilities", json!({"input":{"project_id":project,"expected_revision":0,"configuration":saved["configuration"]}})).await;
    record(ck, ev, "stale CAS refuses without overwriting UI revision", stale.0["ok"] == false && stale.describe().contains("capability_conflict") && value(&read(app, project).await) == saved, stale.describe());
    let selected = ui(app, json!({"revision":saved["revision"]}), r#"const c=await until(()=>$$('[role=combobox]').find(e=>labelOf(e)==='新会话能力来源'));if(!c)throw Error('revision selector absent');c.click();const option=await until(()=>$$('[role=option]').find(e=>txt(e)==='项目配置 · 修订 '+A.revision));if(!option)throw Error('saved revision absent');option.click();return {ok:!!(await until(()=>txt(c).includes('修订 '+A.revision)))};"#).await;
    record(ck, ev, "saved revision explicitly selected for new session", ui_ok(&selected), selected.to_string());
    let before = ids(&sessions_of(app).await);ui_start_session(app, ck, ev, engine).await;
    let Some(sid) = new_session_id(app, &before).await else { record(ck, ev, "real capability session exists", false, engine);return json!({"project":project,"task":task}); };
    let snapshot = ipc(app, "main", "get_development_session_capabilities", json!({"sessionId":sid})).await;
    let actual = value(&snapshot);
    let shown = ui(app, json!({"actual":actual}), r#"const stable=v=>JSON.stringify(v&&typeof v==='object'?Array.isArray(v)?v.map(x=>JSON.parse(stable(x))):Object.fromEntries(Object.keys(v).sort().map(k=>[k,JSON.parse(stable(v[k]))])):v);const section=await until(()=>$$('section[aria-label="会话实际能力快照"]').find(e=>$$('pre',e).some(p=>{try{return stable(JSON.parse(p.textContent))===stable(A.actual)}catch{return false}})));return {ok:!!section};"#).await;
    record(ck, ev, "session actual snapshot equals production IPC", succeeded(&snapshot) && actual["revision"] == saved["revision"] && actual["configuration"] == saved["configuration"] && ui_ok(&shown), shown.to_string());
    let warmup_from_seq = last_seq(app, &sid).await;
    let warmup_view = stage2::exercise_turn(app, ck, ev, &sid, "This is the first desktop acceptance turn. Acknowledge that you are ready for a separate capability tool request in my next message. Do not call any tools or change files in this turn.").await;
    let warmup_to_seq = last_seq(app, &sid).await;
    let warmup_run_id = warmup_view["events"].as_array().into_iter().flatten().find(|event| event["seq"].as_u64().unwrap_or(0) > warmup_from_seq && event["kind"] == "userMessage").and_then(|event| event["run"].as_str());
    let warmup_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let warmup_run = loop {
        let runs = value(&ipc(app, "main", "list_development_runs", json!({"taskId":task})).await);
        let run = runs.as_array().into_iter().flatten().find(|run| warmup_run_id.is_some_and(|id| run["id"] == id) && run["session_id"] == sid).cloned().unwrap_or(Value::Null);
        if run["status"] == "completed" || tokio::time::Instant::now() >= warmup_deadline { break run; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let warmup = json!({"requested":"explicit acknowledgement before separate tool turn","from_seq":warmup_from_seq,"to_seq":warmup_to_seq,"run":warmup_run,"actual_view":warmup_view});
    record(ck, ev, "explicit first acknowledgement turn has terminal run and new sequence", warmup_run["status"] == "completed" && warmup_to_seq > warmup_from_seq, warmup.to_string());
    write_json(fx.root.join(format!("stage4-ui-{engine}-warmup.json")), &warmup);
    println!("STAGE4 UI {engine} ACK: run={} seq={warmup_from_seq}..{warmup_to_seq} actual={}", warmup_run["id"], warmup_run["summary"]);
    let challenge = uuid::Uuid::new_v4().to_string();
    ui_send(app, ck, ev, &format!("Desktop capability acceptance. Call the dgos_project MCP project_tool_0 tool (Claude full tool name: mcp__dgos_project__project_tool_0; Codex: mcp__dgos_project.project_tool_0) exactly twice, sequentially, with arguments {{\"challenge\":\"{challenge}-approved\"}} then {{\"challenge\":\"{challenge}-revoked\"}}. Use the actual MCP tool, not Shell or invented outputs. After the first receipt immediately request the second call. If the second call is refused or revoked report that and finish. Do not use the installed proof skill: this desktop fixture only accepts a challenge argument.")).await;
    let first = pending(app, &sid, "", warmup_to_seq).await;
    record(ck, ev, "real CLI requests host confirmation with exact active run scope", !first.is_null() && first["project_id"] == project && first["task_id"] == task && first["session_id"] == sid && first["schema_hash"] == row["schema_hash"] && first["arguments"] == json!({"challenge":format!("{challenge}-approved")}), first.to_string());
    record(ck, ev, "second explicit tool turn uses a distinct native run", first["run_id"].is_string() && first["run_id"] != warmup_run["id"], json!({"ack_run":warmup_run["id"],"tool_run":first["run_id"],"tool_from_seq":warmup_to_seq}).to_string());
    let approved = if !first.is_null() { ui(app, json!({"pending":first}), r#"const s=await until(()=>$$('section[aria-label="DG-OS 宿主 MCP 确认"]').find(e=>txt(e).includes(A.pending.run_id)&&txt(e).includes(A.pending.schema_hash)&&txt(e).includes(A.pending.arguments.challenge)));if(!s)throw Error('host confirmation UI absent');const b=$$('button',s).find(e=>txt(e)==='允许本次'&&!e.disabled);if(!b)throw Error('host allow absent');b.click();return {ok:!!(await until(()=>!s.isConnected))};"#).await } else { json!({"ok":false}) };
    record(ck, ev, "host approval answered through shipped UI", ui_ok(&approved), approved.to_string());
    let second = if !first.is_null() { pending(app, &sid, first["id"].as_str().unwrap_or_default(), warmup_to_seq).await } else { Value::Null };
    record(ck, ev, "second real call separately awaits host approval", !second.is_null() && second["id"] != first["id"] && second["arguments"] == json!({"challenge":format!("{challenge}-revoked")}), second.to_string());
    let revoked = ipc(app, "main", "revoke_development_capabilities", json!({"projectId":project})).await;
    let epoch = value(&read(app, project).await)["permission_epoch"].clone();
    let expired = ipc(app, "main", "respond_development_capability_confirmation", json!({"input":{"session_id":sid,"confirmation_id":second["id"].as_str().unwrap_or("missing"),"approved":true}})).await;
    record(ck, ev, "active UI host capability revocation advances epoch and rejects pending approval", succeeded(&revoked) && epoch.as_i64().unwrap_or(0) > actual["permission_epoch"].as_i64().unwrap_or(0) && expired.0["ok"] == false && expired.describe().contains("capability_revoked"), json!({"revoke":revoked.describe(),"epoch":epoch,"approval":expired.describe()}).to_string());
    let hidden = ui(app, json!({}), r#"return {ok:!!(await until(()=>!document.querySelector('section[aria-label="DG-OS 宿主 MCP 确认"]')))};"#).await;
    record(ck, ev, "revoked pending host approval disappears from actual UI", ui_ok(&hidden), hidden.to_string());
    let receipts: Vec<Value> = fs::read_to_string(fx.root.join("stage4-ui-fixture.jsonl")).unwrap_or_default().lines().filter_map(|line|serde_json::from_str(line).ok()).collect();
    record(ck, ev, "fixture dispatch proves approval and no revoked side effect", receipts.iter().filter(|r|r["challenge"] == format!("{challenge}-approved")).count() == 1 && !receipts.iter().any(|r|r["challenge"] == format!("{challenge}-revoked")), json!(receipts).to_string());
    ipc(app, "main", "interrupt_agent_session", json!({"sessionId":sid})).await;
    acceptance::close_ui(app, ck, ev, &sid).await;
    let run_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let run = loop {
        let runs = value(&ipc(app, "main", "list_development_runs", json!({"taskId":task})).await);
        let run = runs.as_array().into_iter().flatten().find(|r| r["session_id"] == sid && r["id"] == first["run_id"]).cloned().unwrap_or(Value::Null);
        if (!run.is_null() && !matches!(run["status"].as_str(), Some("running"|"waiting"))) || tokio::time::Instant::now() >= run_deadline { break run; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let tool_sequences = run["events"].as_array().is_some_and(|events| events.iter().any(|event| event["seq"].as_u64().unwrap_or(0) > warmup_to_seq && event["kind"] == "userMessage"));
    record(ck, ev, "acknowledgement and tool turns retain separate run IDs and sequences", run["id"].is_string() && run["id"] != warmup_run["id"] && run["id"] == first["run_id"] && tool_sequences, json!({"ack_run":warmup_run["id"],"ack_last_seq":warmup_to_seq,"tool_run":run["id"],"tool_user_event_after_ack":tool_sequences}).to_string());
    record(ck, ev, "persisted native task run keeps original actual snapshot after revoke", !run.is_null() && run["source"] == "native_agent" && run["capabilities"] == actual && !matches!(run["status"].as_str(), Some("running"|"waiting")), run.to_string());
    tab(app, "runs").await;
    let history = ui(app, json!({"run":run,"actual":actual}), r#"const stable=v=>JSON.stringify(v&&typeof v==='object'?Array.isArray(v)?v.map(x=>JSON.parse(stable(x))):Object.fromEntries(Object.keys(v).sort().map(k=>[k,JSON.parse(stable(v[k]))])):v);const h=await need('dev-run-history');const article=await until(()=>$$('article.dev-run',h).find(e=>txt(e).includes(A.run.id)));if(!article)return {ok:false};const details=$$('section[aria-label="会话实际能力快照"]',article)[0];return {ok:!!details&&$$('pre',details).some(e=>{try{return stable(JSON.parse(e.textContent))===stable(A.actual)}catch{return false}})};"#).await;
    record(ck, ev, "task run view displays persisted snapshot and result", ui_ok(&history), history.to_string());
    let result = json!({"engine":engine,"project":project,"task":task,"session":sid,"saved":saved,"actual":actual,"warmup":warmup,"tool_from_seq":warmup_to_seq,"first_confirmation":first,"revoked_confirmation":second,"run":run});
    write_json(fx.root.join(format!("stage4-ui-{engine}.json")), &result);result
}

async fn boundary(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, label: &str, row: &Value) {
    for (command, args) in [
        ("read_development_capability_catalog",json!({})),
        ("read_development_capabilities",json!({"projectId":row["project"]})),
        ("save_development_capabilities",json!({"input":{"project_id":row["project"],"expected_revision":1,"configuration":row["saved"]["configuration"]}})),
        ("revoke_development_capabilities",json!({"projectId":row["project"]})),
        ("get_development_session_capabilities",json!({"sessionId":row["session"]})),
        ("list_development_capability_confirmations",json!({"sessionId":row["session"]})),
        ("respond_development_capability_confirmation",json!({"input":{"session_id":row["session"],"confirmation_id":row["first_confirmation"]["id"].as_str().unwrap_or("missing"),"approved":true}})),
    ] {
        let reply = ipc(app, label, command, args).await;
        let expected = if label == "main" { "No valid session for this operation" } else { "Host permission denied" };
        record(ck, ev, &format!("{label} boundary refuses {command}"), reply.0["ok"] == false && reply.describe().contains(expected), reply.describe());
    }
}

pub(super) async fn run(app: AppHandle, loaded: bool) {
    let fx = app.state::<Fx>().inner().clone();let mut ck = Checks::default();let mut ev = Vec::new();
    record(&mut ck, &mut ev, "production main WebView loaded", loaded, "/workbench");
    for (user, password) in [ALICE, BOB] {
        let reply = ipc(&app, "main", "create_account", json!({"input":{"username":user,"password":password}})).await;
        record(&mut ck, &mut ev, "fresh isolated SQLite account creation", reply.success(), reply.describe());
    }
    ipc(&app, "main", "sign_out", json!({})).await;
    let execution = async {
        if !ui_sign_in(&app, &mut ck, &mut ev, ALICE).await { return Err("alice_sign_in_failed".to_string()); }
        let resolver = app.state::<Arc<SessionResolver>>();
        let (session, lease) = resolver.agent_session().map_err(|_|"account_identity_missing")?;
        if lease.is_cancelled() { return Err("account_disconnected".into()); }
        let owner = session.user_id().to_owned();
        let seeds = seed(&app, &fx, &owner).await?;
        record(&mut ck, &mut ev, "main signed-in gate admits production capability catalog", succeeded(&ipc(&app,"main","read_development_capability_catalog",json!({})).await), "host-resolved Alice");
        record(&mut ck, &mut ev, "navigate actual project task surface", goto(&app, "main", "/tasks/agent").await, "/tasks/agent");
        let mut rows = Vec::new();
        for seed in seeds { rows.push(per_engine(&app, &fx, &mut ck, &mut ev, &seed).await); }
        let row = rows.iter().find(|row| row["session"].is_string() && row["saved"]["configuration"].is_object()).ok_or("complete_engine_row_missing_for_boundary_checks")?;
        let events = app.state::<LoadEvents>().0.subscribe();
        let window = WebviewWindowBuilder::new(&app, "stage4-untrusted", WebviewUrl::App("workbench".into())).visible(false).build().map_err(|_|"untrusted_window_failed")?;
        let loaded = wait_load(events, "stage4-untrusted", "/workbench").await;
        record(&mut ck, &mut ev, "non-main WebView really loaded", loaded, "same app origin, different label");
        boundary(&app, &mut ck, &mut ev, "stage4-untrusted", row).await;
        window.close().map_err(|_|"untrusted_window_close_failed")?;
        ui_sign_out(&app, &mut ck, &mut ev, ALICE.0).await;
        boundary(&app, &mut ck, &mut ev, "main", row).await;
        if !ui_sign_in(&app, &mut ck, &mut ev, BOB).await { return Err("bob_sign_in_failed".into()); }
        let catalog = value(&ipc(&app, "main", "read_development_capability_catalog", json!({})).await);
        record(&mut ck, &mut ev, "Bob catalog excludes Alice provider skill and MCP", ["providers","skills","mcp"].iter().all(|key|catalog[key].as_array().is_some_and(Vec::is_empty)), catalog.to_string());
        // Catalog is valid for Bob; all Alice object IDs remain inaccessible.
        for (command, args) in [
            ("read_development_capabilities",json!({"projectId":row["project"]})),
            ("save_development_capabilities",json!({"input":{"project_id":row["project"],"expected_revision":1,"configuration":row["saved"]["configuration"]}})),
            ("revoke_development_capabilities",json!({"projectId":row["project"]})),
            ("get_development_session_capabilities",json!({"sessionId":row["session"]})),
            ("list_development_capability_confirmations",json!({"sessionId":row["session"]})),
            ("respond_development_capability_confirmation",json!({"input":{"session_id":row["session"],"confirmation_id":"foreign","approved":true}})),
            ("list_development_runs",json!({"taskId":row["task"]})),
        ] { let reply=ipc(&app,"main",command,args).await;record(&mut ck,&mut ev,&format!("Bob cannot access Alice {command}"),reply.0["ok"]==false,reply.describe()); }
        write_json(fx.root.join("stage4-ui-state.json"), &json!({"engines":rows}));
        Ok::<_,String>(())
    }.await;
    if let Err(error) = execution { record(&mut ck, &mut ev, "desktop scenario completes", false, error); }
    let agents_stopped = app.state::<AgentManager>().shutdown().await;
    let mcp_stopped = app.state::<Services>().mcp.shutdown().await.is_ok();
    record(&mut ck, &mut ev, "real CLI and fixture shutdown complete", agents_stopped && mcp_stopped, "production manager shutdown");
    write_json(fx.root.join("stage4-ui-result.json"), &json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"execution_source":"real_desktop_production_ipc_and_native_clis","mcp_source":"local_stdio_fixture_via_host_localhost_gateway"}));
    println!("STAGE4 UI: {} passed, {} failed; evidence={}",ck.passed,ck.failed.len(),fx.root.display());
    app.exit(if ck.failed.is_empty() {0} else {1});
}
