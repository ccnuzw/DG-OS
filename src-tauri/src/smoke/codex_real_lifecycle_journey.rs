// Real-model desktop acceptance, deliberately separate from the protocol fixture.
// DGOS_CODEX_REAL_LIFECYCLE_SMOKE=1 DGOS_PROJECT_ACCEPTANCE_PHASE=phase6-local
// DGOS_PROJECT_ACCEPTANCE_ROOT=<nonexistent directory under the OS temp root>
// DGOS_STAGE6_CODEX_URL/KEY/MODEL must be supplied in the environment, never argv.
// cargo test --offline -p dgos --features webview-smoke --test webview_smoke_agents -- --nocapture
// No production database, CLI auth/config, OS vault or native-account fallback.

#[derive(Clone, Copy, PartialEq, Eq)]
enum RealCodexMode { Readonly33, Coding, Operations }
impl RealCodexMode {
    fn from_environment() -> Self {
        match std::env::var("DGOS_CODEX_REAL_MODE").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("readonly33") => Self::Readonly33,
            Ok("coding") => Self::Coding,
            Ok("operations") => Self::Operations,
            _ => panic!("explicit real Codex mode must be readonly33, coding or operations"),
        }
    }
    fn name(self) -> &'static str {
        match self { Self::Readonly33 => "readonly33", Self::Coding => "coding", Self::Operations => "operations" }
    }
}

struct RealCodexSettings {
    mode: RealCodexMode,
    executable: PathBuf,
    endpoint: String,
    model: String,
    key: std::sync::Mutex<Option<String>>,
}
static REAL_CODEX_SETTINGS: std::sync::OnceLock<RealCodexSettings> = std::sync::OnceLock::new();

pub(super) fn real_codex_enabled() -> bool {
    std::env::var("DGOS_CODEX_REAL_LIFECYCLE_SMOKE").is_ok_and(|value| value == "1")
}

fn isolate_real_codex(root: &Path) {
    for flag in ["DGOS_CODEX_LIFECYCLE_SMOKE", "DGOS_GIT_COMMIT_SMOKE", "DGOS_EXPLORER_SMOKE"] {
        assert!(std::env::var_os(flag).is_none(), "real Codex cannot be combined with a local fixture journey");
    }
    let required = |name| std::env::var(name).ok().filter(|value| !value.trim().is_empty()).expect("explicit DGOS_STAGE6_CODEX_URL/KEY/MODEL required");
    let settings = RealCodexSettings {
        mode: RealCodexMode::from_environment(),
        // Resolve before HOME/PATH sanitation; never install or select a protocol peer.
        executable: dgos_agents::session::resolve_executable("codex", None).expect("installed Codex required").canonicalize().expect("canonical Codex executable"),
        endpoint: required("DGOS_STAGE6_CODEX_URL"),
        model: required("DGOS_STAGE6_CODEX_MODEL"),
        key: std::sync::Mutex::new(Some(required("DGOS_STAGE6_CODEX_KEY"))),
    };
    let mut executable_path = vec![settings.executable.parent().expect("Codex parent").to_path_buf()];
    // npm's installed Codex entrypoint may use /usr/bin/env node. Resolve the
    // installed runtime before PATH is cleared, without retaining the old PATH.
    if let Ok(node) = dgos_agents::session::resolve_executable("node", None) {
        if let Some(parent) = node.parent() { executable_path.push(parent.to_path_buf()); }
    }
    executable_path.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin"), PathBuf::from("/usr/sbin"), PathBuf::from("/sbin")]);
    assert!(!root.exists(), "real Codex requires a nonexistent temporary root");
    let parent = root.parent().expect("temporary root parent").canonicalize().expect("canonical parent");
    let temporary = std::env::temp_dir().canonicalize().expect("canonical temp directory");
    assert!(parent.starts_with(&temporary) || parent.starts_with(Path::new("/private/tmp")), "real Codex root must be temporary");
    fs::create_dir(root).expect("fresh real Codex root");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).expect("private real Codex root");
    }
    let root = root.canonicalize().expect("canonical real Codex root");
    for directory in ["home", "codex-home", "claude-home", "config", "cache", "data"] {
        fs::create_dir(root.join(directory)).expect("isolated native root");
    }
    // Entry thread only: no runtime, WebView or worker exists yet. Keep only the
    // desktop/process essentials; keys live in memory until the memory-vault seed.
    unsafe {
        for (name, _) in std::env::vars_os().collect::<Vec<_>>() {
            let key = name.to_string_lossy();
            if !matches!(key.as_ref(), "TMPDIR" | "LANG" | "__CF_USER_TEXT_ENCODING" | "DISPLAY" | "TERM" | "RUST_BACKTRACE" | "DGOS_PROJECT_ACCEPTANCE_ROOT" | "DGOS_PROJECT_ACCEPTANCE_PHASE" | "DGOS_CODEX_REAL_LIFECYCLE_SMOKE") && !key.starts_with("LC_") {
                std::env::remove_var(name);
            }
        }
        for (name, directory) in [("HOME", "home"), ("CODEX_HOME", "codex-home"), ("CLAUDE_CONFIG_DIR", "claude-home"), ("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"), ("XDG_DATA_HOME", "data")] {
            std::env::set_var(name, root.join(directory));
        }
        std::env::set_var("PATH", std::env::join_paths(executable_path).expect("isolated executable PATH"));
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_TERMINAL_PROMPT", "0");
    }
    let version = Command::new(&settings.executable).arg("--version").output().expect("installed Codex version probe");
    assert!(version.status.success() && String::from_utf8_lossy(&version.stdout).trim() == "codex-cli 0.167.0", "real journey requires codex-cli 0.167.0");
    assert!(REAL_CODEX_SETTINGS.set(settings).is_ok(), "real Codex isolation runs once");
}

fn real_save(fx: &Fx, name: &str, data: &Value) {
    fs::write(fx.root.join(format!("phase6-real-codex-{name}.json")), serde_json::to_vec_pretty(data).expect("sanitized evidence JSON")).expect("real Codex evidence");
}
fn real_require(fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, ok: bool, detail: Value) -> Journey<()> {
    record(ck, ev, name, ok, detail);
    real_save(fx, "checkpoint", &json!({"step":name,"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    if ok { Ok(()) } else { Err(name.into()) }
}
async fn real_host(app: &AppHandle, command: &str, args: Value) -> Journey<Value> {
    let reply = ipc(app, "main", command, args).await;
    // Native error payloads can contain provider/tool output. Never persist them.
    if !succeeded(&reply) { return Err(format!("{command}: host rejected ({})", real_safe_host_error(&reply.0["error"]))); }
    let data = value(&reply);
    if data.get("success").is_some() {
        if data["success"] == true { Ok(data["data"].clone()) } else { Err(format!("{command}: rejected ({})", real_safe_host_error(&data["error"]))) }
    } else { Ok(data) }
}

fn real_safe_host_error(value: &Value) -> &'static str {
    // Exact codes only. No substring matching, native message, or payload output.
    match value.as_str().or_else(|| value.get("message").and_then(Value::as_str)) {
        Some("invalid_codex_operation") => "invalid_codex_operation",
        Some("invalid_request_id") => "invalid_request_id",
        Some("session_not_found") => "session_not_found",
        Some("stale_generation") => "stale_generation",
        Some("session_closed") => "session_closed",
        Some("busy") => "busy",
        Some("session_busy") => "session_busy",
        Some("engine_rejected") => "engine_rejected",
        Some("method_unsupported") => "method_unsupported",
        Some("codex_operation_unsupported") => "codex_operation_unsupported",
        Some("codex_child_unobserved") => "codex_child_unobserved",
        Some("not_running") => "not_running",
        Some("codex_terminal_unobserved") => "codex_terminal_unobserved",
        Some("codex_queue_unobserved") => "codex_queue_unobserved",
        Some("codex_queue_empty") => "codex_queue_empty",
        Some("codex_queue_membership_changed") => "codex_queue_membership_changed",
        Some("codex_turn_unobserved") => "codex_turn_unobserved",
        Some("codex_observation_incomplete") => "codex_observation_incomplete",
        Some("result_unknown") => "result_unknown",
        Some("request_conflict") => "request_conflict",
        _ => "unavailable; payload omitted",
    }
}
async fn real_step(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, args: Value, body: &str) -> Journey<Value> {
    let reply = dom(app, args, body).await;
    real_require(fx, ck, ev, name, ui_ok(&reply), json!({"source":"production DOM/IPC","payload":"omitted"}))?;
    Ok(reply["value"].clone())
}
fn real_events(view: &Value) -> impl Iterator<Item = &Value> { view["events"].as_array().into_iter().flatten() }
fn real_summary(view: &Value) -> Value {
    json!({"status":view["status"],"cli_version":view["cli_version"],"has_native_thread":view["native_id"].is_string(),"has_active_run":view["active_run"].is_string(),"events":real_events(view).count(),"user_messages":real_events(view).filter(|event| event["kind"] == "userMessage").count(),"steering_receipts":real_events(view).filter(|event| event["kind"] == "userSteering").count(),"pending":view["pending"].as_array().map(Vec::len),"reasoning":view["controls"]["reasoning"]["current"],"permission":view["controls"]["permission"]["current"],"collaboration":view["controls"]["collaboration"]["current"]})
}
fn real_answer(view: &Value, run: &str) -> String {
    real_events(view).filter(|event| event["run"] == run && matches!(event["kind"].as_str(), Some("agentMessage" | "item/agentMessage/delta"))).filter_map(|event| event["text"].as_str()).collect::<Vec<_>>().join("")
}
async fn real_wait<F: Fn(&Value) -> bool>(app: &AppHandle, sid: &str, seconds: u64, approvals: &mut std::collections::HashSet<String>, ready: F) -> Journey<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        let view = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
        if matches!(view["status"].as_str(), Some("error" | "unknown")) { return Err("native lifecycle entered error/unknown; payload omitted".into()); }
        if let Some(pending) = view["pending"].as_array().filter(|rows| !rows.is_empty()) {
            if pending.iter().any(|row| !matches!(row["kind"].as_str(), Some("item/commandExecution/requestApproval" | "item/fileChange/requestApproval" | "item/permissions/requestApproval"))) {
                return Err("native non-approval interaction requested; no fabricated answer".into());
            }
            for request in pending {
                if let Some(id) = request["id"].as_str() { approvals.insert(id.to_owned()); }
            }
            let denied = dom(app, json!({}), r#"
const request=await until(()=>$$('.agent-dialogue-requests section',tid('dev-panel-agent')).find(e=>$$('button',e).some(b=>['拒绝','Deny'].includes(txt(b))&&!b.disabled)));
const deny=request&&$$('button',request).find(b=>['拒绝','Deny'].includes(txt(b))&&!b.disabled);if(!deny)throw Error('native deny control missing');deny.click();return {ok:true};
"#).await;
            if !ui_ok(&denied) { return Err("actual native approval denial unavailable".into()); }
        } else if ready(&view) { return Ok(view); }
        if tokio::time::Instant::now() >= deadline { return Err("native lifecycle observation timed out; payload omitted".into()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn real_capture(app: &AppHandle, fx: &Fx, name: &str) -> Journey<()> {
    // Force synchronous style resolution: background WKWebViews may never run
    // requestAnimationFrame. Capture remains mandatory and target-window only.
    let masked = tokio::time::timeout(Duration::from_secs(10), dom(app, json!({}), r#"
document.getElementById('real-codex-capture-mask')?.remove();
const hidden='.agent-dialogue-transcript,.agent-dialogue-requests,textarea,[data-testid^="agent-image-preview-"],[data-testid="agent-image-dialog"],[data-testid="codex-operations"] input,[data-testid="codex-operations"] select,[data-testid="codex-operations"] pre';
const privateText='[data-testid="codex-operations"] *:not(button)';
const style=document.createElement('style');style.id='real-codex-capture-mask';style.textContent=hidden+'{visibility:hidden!important}'+privateText+'{color:transparent!important;text-shadow:none!important}';document.head.append(style);
const hiddenApplied=[...document.querySelectorAll(hidden)].every(node=>getComputedStyle(node).visibility==='hidden');
const textApplied=[...document.querySelectorAll(privateText)].every(node=>{const css=getComputedStyle(node);return css.visibility==='hidden'||css.color==='rgba(0, 0, 0, 0)'||css.color==='transparent'});
return {ok:style.isConnected&&!!style.sheet&&hiddenApplied&&textApplied};
"#)).await;
    let capture = if masked.as_ref().is_ok_and(|reply| ui_ok(reply)) {
        tokio::time::timeout(Duration::from_secs(20), super::super::native_capture::capture_native(app, &fx.root, &format!("phase6-real-codex-{name}"))).await
            .map_err(|_| "actual target-window screenshot timed out".to_owned())
            .and_then(|result| result.map(|_| ()).map_err(|_| "actual target-window screenshot unavailable".to_owned()))
    } else { Err("screenshot masking failed".to_owned()) };
    // Even partial mask installation or a failed capture must restore the UI.
    let restored = tokio::time::timeout(Duration::from_secs(10), dom(app, json!({}), "document.getElementById('real-codex-capture-mask')?.remove();return {ok:!document.getElementById('real-codex-capture-mask')};")).await;
    if !restored.as_ref().is_ok_and(|reply| ui_ok(reply)) { return Err("screenshot masking restoration failed".into()); }
    capture
}

async fn real_capture_check(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, stage: &str) -> Journey<()> {
    if let Err(error) = real_capture(app, fx, stage).await {
        let check = format!("mandatory masked target-window screenshot: {stage}");
        record(ck, ev, &check, false, &error);
        real_save(fx, "checkpoint", &json!({"step":check,"passed":ck.passed,"failed":ck.failed,"checks":ev}));
        // An unavailable screenshot remains an acceptance failure, but must not
        // hide independently reachable functional evidence. Unsafe restoration
        // is different: do not continue interacting with a still-masked UI.
        if error == "screenshot masking restoration failed" { return Err(error); }
    }
    Ok(())
}

async fn real_codex_journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, approvals: &mut std::collections::HashSet<String>) -> Journey<()> {
    let settings = REAL_CODEX_SETTINGS.get().ok_or("real Codex isolation missing")?;
    let (account, _) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|_| "isolated account absent")?;
    let owner = account.user_id().to_owned();
    let preferences = real_host(app, "get_cli_preferences", json!({})).await?;
    let mut paths = preferences["paths"].clone();
    paths["codex"] = json!(settings.executable);
    let saved = real_host(app, "set_cli_preferences", json!({"paths":paths,"expectedRevision":preferences["revision"]})).await?;
    real_require(fx, ck, ev, "installed Codex selected explicitly in isolated preferences", saved["saved"] == true, json!({"protocol_fixture":false,"version":"0.167.0"}))?;
    let vault = app.state::<Arc<dgos_providers::test_support::MemoryCredentialVault>>();
    let key = settings.key.lock().map_err(|_| "memory credential lock failed")?.take().ok_or("memory credential already consumed")?;
    let provider = dgos_providers::test_support::seed_provider(fx.db.clone(), vault.inner(), dgos_types::ProviderConfig { name:"Authorized real Codex acceptance".into(),provider_type:"openai_compatible".into(),base_url:settings.endpoint.clone(),api_key:key,enabled:true,config:None }, &owner).await.map_err(|_| "authorized Provider seed failed (payload omitted)")?;
    app.state::<dgos_providers::ProviderManager>().add_manual_model(&provider.id, &owner, &settings.model, None).await.map_err(|_| "authorized model seed failed")?;
    let path = fx.root.join("real-codex-project");
    make_repo(&path);
    fs::write(path.join("source.txt"), "Authorized temporary read-only lifecycle research. 17 + 25 = 42.\n").map_err(|_| "temporary source creation failed")?;
    let project = real_host(app, "create_development_project", json!({"input":{"kind":"import","name":"Real Codex isolated lifecycle","path":path,"source":null}})).await?;
    real_require(fx, ck, ev, "temporary project imported through production IPC", project["id"].is_string(), json!({"temporary":true}))?;
    real_require(fx, ck, ev, "shipped workbench reloads isolated real catalog", goto(app, "main", "/tasks/agent").await, json!({"renderer":"production App/WKWebView"}))?;
    real_step(app, fx, ck, ev, "observe production dispatch without response replacement", json!({"project":project["id"]}), r#"
const original=window.fetch;window.__realCodex={original,events:[]};window.fetch=function(url,options,...rest){const command=String(url).split('/').at(-1);let args={};try{args=JSON.parse(options?.body||'{}')}catch{}window.__realCodex.events.push({command,settings:command==='send_agent_message'?args.input?.settings:undefined});return original.call(this,url,options,...rest)};
const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();await click('dev-project-item-'+A.project);return {ok:!!await until(()=>tid('dev-project-conversation'))};
"#).await?;
    // Plan mode permits read-only research. The command is deliberately bounded,
    // has no network or writes, and only reads this freshly created temp project.
    const FIRST: &str = "This is an authorized read-only research lifecycle check in this temporary project only. Stay in Plan mode, high reasoning, read-only sandbox. First send a brief user-visible commentary saying the numerical answer to 17 + 25 (42). Then actually run this read-only shell research command now, not merely propose it: /bin/sh -c 'printf DGOS_REAL_RESEARCH_STARTED; /bin/sleep 120; /bin/cat source.txt'. The bounded sleep provides time for my steering and interruption. Do not write files, access other directories, use network tools, request escalation, or start background processes. After the command, give a brief plan. Do not ask questions.";
    real_step(app, fx, ck, ev, "selected authorized model and trust do not send", json!({"model":settings.model,"draft":FIRST}), r#"
await click('composer-model-picker-trigger');const dialog=await need('composer-model-picker-dialog');const option=await until(()=>$$('.composer-model-picker-option',dialog).find(e=>txt(e).includes(A.model)&&!e.disabled));if(!option)throw Error('authorized model absent');option.click();if(!await until(()=>!tid('composer-model-picker-dialog')))throw Error('model not committed');setInput(document.getElementById('agent-message'),A.draft);await click('direct-conversation-authorize');return {ok:!!await until(()=>$$('[role=dialog] button').find(b=>['允许并记住','Allow and remember'].includes(txt(b))))&&!window.__realCodex.events.some(e=>['open_agent_session','send_agent_message'].includes(e.command))};
"#).await?;
    real_step(app, fx, ck, ev, "confirmed directory authorization auto-prepares without dispatch and retains first draft", json!({"draft":FIRST}), r#"
await confirmWorkspaceGrant();return {ok:!!await until(()=>!tid('dev-project-conversation')&&document.getElementById('agent-message')?.value===A.draft)&&!window.__realCodex.events.some(e=>e.command==='send_agent_message')};
"#).await?;
    let sessions = real_host(app, "list_agent_sessions", json!({})).await?;
    let rows = sessions.as_array().ok_or("session list absent")?;
    real_require(fx, ck, ev, "prepare creates one real native session", rows.len() == 1, json!({"session_count":rows.len()}))?;
    let sid = rows[0]["id"].as_str().ok_or("session ID absent")?.to_owned();
    let prepared = real_wait(app, &sid, 60, approvals, |view| view["status"] == "idle").await?;
    real_require(fx, ck, ev, "prepared real session has no user message or active run", prepared["native_id"].is_string() && prepared["cli_version"] == "codex-cli 0.167.0" && prepared["active_run"].is_null() && real_events(&prepared).all(|event| event["kind"] != "userMessage"), real_summary(&prepared))?;
    let controls = &prepared["controls"];
    real_require(fx, ck, ev, "native bound-model metadata genuinely supports high read-only Plan and steering", controls["reasoning"]["available"] == true && controls["reasoning"]["options"].as_array().is_some_and(|options| options.iter().any(|option| option == "high")) && controls["permission"]["available"] == true && controls["permission"]["options"].as_array().is_some_and(|options| options.iter().any(|option| option == "read_only")) && controls["collaboration"]["available"] == true && controls["collaboration"]["options"].as_array().is_some_and(|options| options.iter().any(|option| option == "plan")) && controls["steering"] == true, json!({"reasoning_available":controls["reasoning"]["available"],"permission_available":controls["permission"]["available"],"collaboration_available":controls["collaboration"]["available"],"steering":controls["steering"],"no_synthetic_metadata":true}))?;
    real_step(app, fx, ck, ev, "select high reasoning read-only sandbox and Plan before first send", json!({}), r#"
const trigger=labels=>$$('button[aria-label]',tid('agent-composer-controls')).find(b=>labels.includes(b.getAttribute('aria-label'))&&!b.disabled);
const reasoning=trigger(['推理强度','Reasoning']);if(!reasoning)throw Error('reasoning absent');reasoning.click();const high=await until(()=>$$('.agent-control-popover button').find(b=>txt(b)==='high'&&!b.disabled));if(!high)throw Error('high unavailable');high.click();if(!await until(()=>!$$('.agent-control-popover').length))throw Error('reasoning unsettled');
const permission=trigger(['权限','Permissions']);if(!permission)throw Error('permission absent');permission.click();const readOnly=await until(()=>$$('.agent-control-popover button').find(b=>['只读沙箱','Read-only sandbox'].includes(txt(b))&&!b.disabled));if(!readOnly)throw Error('read-only absent');readOnly.click();if(!await until(()=>!$$('.agent-control-popover').length))throw Error('permission unsettled');
trigger(['权限','Permissions']).click();const plan=await until(()=>$$('.agent-control-popover button').find(b=>txt(b)==='Plan'&&!b.disabled));if(!plan)throw Error('Plan disabled');plan.click();return {ok:!!await until(()=>!$$('.agent-control-popover').length&&txt(trigger(['推理强度','Reasoning'])).includes('high')&&['只读沙箱','Read-only sandbox'].some(label=>txt(trigger(['权限','Permissions'])).includes(label)))&&!window.__realCodex.events.some(e=>e.command==='send_agent_message')};
"#).await?;
    real_capture_check(app, fx, ck, ev, "before-first-send").await?;
    real_step(app, fx, ck, ev, "first explicit send dispatches real model request", json!({}), r#"
const send=await until(()=>$$('.agent-dialogue-composer button',tid('dev-panel-agent')).find(b=>['发送','Send'].includes(txt(b))&&!b.disabled));if(!send)throw Error('send absent');send.click();return {ok:!!await until(()=>document.getElementById('agent-message')?.value==='')};
"#).await?;
    let active = real_wait(app, &sid, 180, approvals, |view| {
        let Some(run) = view["active_run"].as_str() else { return false; };
        view["status"] == "running" && real_answer(view, run).contains("42") && real_events(view).any(|event| event["run"] == run && event["kind"] == "item/started" && event["data"]["type"] == "commandExecution" && event["data"]["command"].as_str().is_some_and(|command| command.contains("sleep 120") && command.contains("source.txt")))
    }).await?;
    let first_run = active["active_run"].as_str().ok_or("first run absent")?.to_owned();
    real_require(fx, ck, ev, "actual first model answer and bounded native read-only tool observed", active["controls"]["reasoning"]["current"] == "high" && active["controls"]["permission"]["current"] == "read_only" && active["controls"]["collaboration"]["current"] == "plan", real_summary(&active))?;
    const STEER: &str = "Keep the same research turn and read-only boundaries. I am explicitly steering this running task: after the current bounded read completes, do not run another command; summarize the answer in one sentence. I may interrupt this turn now.";
    real_step(app, fx, ck, ev, "explicit steering targets the genuinely active first native turn", json!({"text":STEER}), r#"
setInput(document.getElementById('agent-message'),A.text);const steer=await until(()=>$$('button',tid('dev-panel-agent')).find(b=>['追加到本次执行','Add to current turn'].includes(txt(b))&&!b.disabled));if(!steer)throw Error('steer absent');steer.click();return {ok:!!await until(()=>document.getElementById('agent-message')?.value==='')};
"#).await?;
    let steered = real_wait(app, &sid, 30, approvals, |view| view["active_run"] == first_run && real_events(view).any(|event| event["kind"] == "userSteering" && event["run"] == first_run && event["text"] == STEER)).await?;
    real_require(fx, ck, ev, "positive native steering receipt belongs to original run", real_events(&steered).filter(|event| event["kind"] == "userSteering" && event["run"] == first_run).count() == 1, real_summary(&steered))?;
    real_step(app, fx, ck, ev, "interrupt original running turn through production control", json!({}), r#"
const stop=await until(()=>$$('.agent-dialogue-actions button',tid('dev-panel-agent')).find(b=>['停止本次执行','Stop this turn'].includes(txt(b))&&!b.disabled));if(!stop)throw Error('interrupt absent');stop.click();return {ok:true};
"#).await?;
    let interrupted = real_wait(app, &sid, 60, approvals, |view| view["status"] == "idle" && view["active_run"].is_null() && real_events(view).any(|event| event["run"] == first_run && event["data"]["run_status"] == "cancelled")).await?;
    real_require(fx, ck, ev, "native interruption reaches confirmed cancelled terminal boundary", true, real_summary(&interrupted))?;
    real_step(app, fx, ck, ev, "second explicit send follows confirmed interruption", json!({"text":"This is my second explicit turn after interruption. Do not run tools or modify anything. Give the numerical answer to 19 + 23 in a brief user-visible final response, then stop."}), r#"
setInput(document.getElementById('agent-message'),A.text);const send=await until(()=>$$('.agent-dialogue-composer button',tid('dev-panel-agent')).find(b=>['发送','Send'].includes(txt(b))&&!b.disabled));if(!send)throw Error('second send absent');send.click();return {ok:!!await until(()=>document.getElementById('agent-message')?.value==='')};
"#).await?;
    let second = real_wait(app, &sid, 180, approvals, |view| view["status"] == "idle" && real_events(view).any(|event| event["run"].as_str().is_some_and(|run| run != first_run) && event["data"]["run_status"] == "completed" && real_answer(view, event["run"].as_str().unwrap_or_default()).contains("42"))).await?;
    real_require(fx, ck, ev, "second distinct native model turn completes with actual answer", real_events(&second).filter(|event| event["kind"] == "userMessage").count() == 2, real_summary(&second))?;
    let workspace = real_host(app, "read_development_workspace", json!({})).await?;
    let task = workspace["selection"]["task_id"].as_str().ok_or("task absent")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let runs = real_host(app, "list_development_runs", json!({"taskId":task})).await?;
        let rows = runs.as_array().ok_or("durable run list absent")?;
        let owned = rows.iter().filter(|run| run["session_id"] == sid).collect::<Vec<_>>();
        if owned.len() == 2 && owned.iter().all(|run| run["finished_at"].is_number()) {
            real_require(fx, ck, ev, "two explicit sends persist distinct cancelled and completed runs", owned.iter().any(|run| run["id"] == first_run && run["status"] == "cancelled") && owned.iter().any(|run| run["id"] != first_run && run["status"] == "completed"), json!({"durable_runs":2,"statuses":["cancelled","completed"]}))?;
            break;
        }
        if tokio::time::Instant::now() >= deadline { return Err("durable terminal run projection timed out".into()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    real_step(app, fx, ck, ev, "explicit session end is separate from interruption", json!({}), r#"
const close=await until(()=>$$('.agent-dialogue-actions button',tid('dev-panel-agent')).find(b=>['结束会话','End session'].includes(txt(b))&&!b.disabled));if(!close)throw Error('close absent');close.click();return {ok:!!await until(()=>tid('saved-conversation-composer'))};
"#).await?;
    let closed = real_wait(app, &sid, 60, approvals, |view| view["status"] == "closed").await?;
    real_require(fx, ck, ev, "explicit close preserves original native thread", closed["native_id"] == prepared["native_id"], real_summary(&closed))?;
    real_step(app, fx, ck, ev, "focusing authorized saved composer resumes original thread without replay", json!({"sid":sid}), r#"
await prepareSavedConversation(A.sid);return {ok:!!document.getElementById('agent-message')&&!tid('saved-conversation-composer')};
"#).await?;
    let resumed = real_wait(app, &sid, 60, approvals, |view| view["status"] == "idle").await?;
    real_require(fx, ck, ev, "resume uses same thread and fresh generation without active run", resumed["native_id"] == prepared["native_id"] && resumed["generation"] != prepared["generation"] && resumed["active_run"].is_null(), real_summary(&resumed))?;
    // Observe a quiet window after resume as well as retained history; no implicit
    // dispatch can pass solely because its asynchronous projection was not ready.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let quiet = real_host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    real_require(fx, ck, ev, "resume quiet window retains hydrated history without new user turn", quiet["status"] == "idle" && quiet["active_run"].is_null() && real_events(&quiet).filter(|event| event["kind"] == "userMessage").count() == real_events(&resumed).filter(|event| event["kind"] == "userMessage").count() && real_events(&quiet).any(|event| event["kind"] == "userMessage" && (event["text"].as_str().is_some_and(|text| text.contains("second explicit turn after interruption")) || event["data"]["content"].as_array().is_some_and(|content| content.iter().any(|part| part["text"].as_str().is_some_and(|text| text.contains("second explicit turn after interruption")))))), real_summary(&quiet))?;
    let after_resume = real_host(app, "list_development_runs", json!({"taskId":task})).await?;
    real_require(fx, ck, ev, "resume does not create a replay run", after_resume.as_array().is_some_and(|runs| runs.iter().filter(|run| run["session_id"] == sid).count() == 2), json!({"durable_runs":2}))?;
    real_step(app, fx, ck, ev, "production dispatch counts and first-turn settings match explicit actions", json!({}), r#"
const events=window.__realCodex.events,count=command=>events.filter(e=>e.command===command).length,sends=events.filter(e=>e.command==='send_agent_message'),settings=sends[0]?.settings;
return {ok:count('open_agent_session')===2&&sends.length===2&&count('steer_agent_turn')===1&&count('interrupt_agent_session')===1&&count('close_agent_session')===1&&settings?.reasoning_effort==='high'&&settings?.permission_mode==='read_only'&&settings?.collaboration_mode==='plan'};
"#).await?;
    real_capture_check(app, fx, ck, ev, "final-resumed").await?;
    real_save(fx, "lifecycle", &json!({"source":"installed Codex app-server and authorized real Provider; not protocol fixture","prepared":real_summary(&prepared),"active_first":real_summary(&active),"steered":real_summary(&steered),"interrupted":real_summary(&interrupted),"second":real_summary(&second),"closed":real_summary(&closed),"resumed":real_summary(&quiet),"explicit_sends":2,"native_thread_retained":true,"fresh_generation":true,"automatic_replay_observed":false,"approvals_requested":approvals.len(),"approval_coverage":if approvals.is_empty(){"absent; approval UI not exercised in real run"}else{"actual native requests denied through production UI"},"raw_content_persisted_in_evidence":false}));
    Ok(())
}

async fn run_real_codex(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    let mode = REAL_CODEX_SETTINGS.get().expect("isolated real settings").mode;
    real_save(fx, "safety", &json!({"mode":mode.name(),"source":"production App/WKWebView/IPC/AgentManager with installed Codex 0.167.0","protocol_fixture":false,"native_account_fallback":false,"vault":"MemoryCredentialVault","selected_providers":1,"engines":["codex"],"isolated_roots":["HOME","CODEX_HOME","CLAUDE_CONFIG_DIR","XDG_CONFIG_HOME","XDG_CACHE_HOME","XDG_DATA_HOME"],"production_database_access":false,"evidence":"content-free summaries and masked actual target-window screenshots","tool_task":if mode == RealCodexMode::Readonly33 {"bounded read-only research"}else{"isolated multi-file bugfix, single scoped marker approval and independent tests"}}));
    let mut approvals = std::collections::HashSet::new();
    let result = match mode {
        RealCodexMode::Readonly33 => real_codex_journey(app, fx, &mut ck, &mut ev, &mut approvals).await,
        RealCodexMode::Coding | RealCodexMode::Operations => real_coding_journey(app, fx, &mut ck, &mut ev, &mut approvals, mode == RealCodexMode::Operations).await,
    };
    if let Err(error) = result {
        let restoration_failed = error == "screenshot masking restoration failed";
        record(&mut ck, &mut ev, "real Codex lifecycle completes", false, error);
        if !restoration_failed { let _ = real_capture_check(app, fx, &mut ck, &mut ev, "failure-masked").await; }
    }
    let _ = dom(app, json!({}), "if(window.__realCodex)window.fetch=window.__realCodex.original;return {ok:true};").await;
    record(&mut ck, &mut ev, "owned real Codex processes stop through production shutdown", app.state::<AgentManager>().shutdown().await, "owned isolated lifecycle only");
    real_save(fx, "result", &json!({"mode":mode.name(),"passed":ck.passed,"failed":ck.failed,"checks":ev,"native_cli":true,"protocol_fixture":false,"approvals_requested":approvals.len(),"approval_coverage":if approvals.is_empty(){"absent; unexercised, not simulated"}else if mode == RealCodexMode::Readonly33{"actual native requests denied"}else{"actual native requests; per-action outcome in coding evidence"},"limitations":["No physical keyboard or OS IME claim","Content intentionally masked in screenshots","Approval absence is not real approval UI proof","Deterministic boundary fixtures remain separate evidence, not real model proof","External HTTP request count is not measured"]}));
    app.exit(if ck.failed.is_empty() { 0 } else { 1 });
}

include!("codex_real_coding_journey.rs");
