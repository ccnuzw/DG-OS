//! Safe local phase for the existing shipped App/WKWebView harness.
//! No real CLI/model/provider request or OS credential vault is used. Run only
//! with DGOS_PROJECT_ACCEPTANCE_PHASE=phase6-local and a nonexistent temp root.
use super::*;
use super::super::ui_keys::{self, Key};

type Journey<T> = Result<T, String>;

/// Must run on the entry thread, before the async runtime or WebView is created.
/// Both catalog discovery and explicit session launch resolve only fixture peers.
pub(super) fn isolate(root: &Path) {
    assert!(!root.exists(), "phase6-local requires a nonexistent root");
    let parent = root.parent().expect("temporary root parent").canonicalize().expect("canonical parent");
    let temporary = std::env::temp_dir().canonicalize().expect("canonical temp directory");
    assert!(parent.starts_with(&temporary) || parent == Path::new("/private/tmp"), "phase6-local root must be temporary");
    fs::create_dir(root).expect("fresh phase6 root");
    let root = root.canonicalize().expect("canonical phase6 root");
    let bin = root.join("bin");
    fs::create_dir(&bin).expect("fixture bin");
    for name in ["home", "codex-home", "claude-home", "config", "cache", "data"] { fs::create_dir(root.join(name)).expect("isolated home"); }
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::write(bin.join("codex"), PROTOCOL).expect("protocol fixture");
        fs::write(bin.join("claude"), "#!/bin/sh\nif [ \"${1-}\" = --version ]; then printf '2.1.0 (Claude Code)\\n'; exit 0; fi\nprintf 'phase6: Claude execution forbidden\\n' >&2\nexit 79\n").expect("claude discovery fixture");
        fs::write(bin.join("node"), "#!/bin/sh\nif [ \"${1-}\" = --version ]; then printf 'v22.0.0\\n'; exit 0; fi\nprintf 'phase6: Node execution forbidden\\n' >&2\nexit 79\n").expect("node discovery fixture");
        for name in ["codex", "claude", "node"] { fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o700)).expect("fixture permissions"); }
    }
    // No workers exist yet. Do not mutate process environment after this point.
    unsafe {
        for (key, _) in std::env::vars_os().filter(|(key, _)| {
            let key = key.to_string_lossy();
            key.starts_with("GIT_") || key.starts_with("OPENAI_") || key.starts_with("ANTHROPIC_") || key.starts_with("AZURE_") || key.starts_with("CODEX_") || key.starts_with("CLAUDE_") || key.starts_with("DGOS_STAGE6_")
        }).collect::<Vec<_>>() { std::env::remove_var(key); }
        for (key, directory) in [("HOME", "home"), ("CODEX_HOME", "codex-home"), ("CLAUDE_CONFIG_DIR", "claude-home"), ("XDG_CONFIG_HOME", "config"), ("XDG_CACHE_HOME", "cache"), ("XDG_DATA_HOME", "data")] { std::env::set_var(key, root.join(directory)); }
        std::env::set_var("PATH", std::env::join_paths([bin, PathBuf::from("/usr/bin"), PathBuf::from("/bin")]).expect("fixture PATH"));
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_TERMINAL_PROMPT", "0");
        std::env::remove_var("DGOS_AGENT_CLOSE_ONLY");
        std::env::remove_var("DGOS_STAGE2_ACCEPTANCE");
    }
}

const PROTOCOL: &str = r##"#!/bin/sh
if [ "${1-}" = --version ]; then printf 'codex-cli 0.167.0\n'; exit 0; fi
printf '%s\n' "$$" > phase6-child.pid
while IFS= read -r line; do
printf '%s\n' "$line" >> phase6-protocol.jsonl
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"config/read"'*) printf '{"id":%s,"result":{"config":{"model_provider":"dgos_project","model_providers":{"dgos_project":{"base_url":"http://127.0.0.1:9/v1","wire_api":"responses","env_key":"DGOS_PROVIDER_KEY","requires_openai_auth":false}}}}}\n' "$request";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"phase6-local-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"turn/start"'*)
printf '{"id":%s,"result":{"turn":{"id":"phase6-local-turn"}}}\n' "$request"
i=0
while [ "$i" -lt 60 ]; do
printf '{"method":"item/completed","params":{"threadId":"phase6-local-thread","turnId":"phase6-local-turn","item":{"id":"line-%s","type":"agentMessage","text":"SYNTHETIC PROTOCOL ONLY row %s: deterministic local transcript, no model invocation."}}}\n' "$i" "$i"
i=$((i+1))
done
printf '%s\n' '{"method":"turn/completed","params":{"threadId":"phase6-local-thread","turn":{"id":"phase6-local-turn","status":"completed"}}}'
;;
esac
done
"##;

fn save(fx: &Fx, name: &str, data: &Value) {
    fs::write(fx.root.join(format!("phase6-local-{name}.json")), serde_json::to_vec_pretty(data).expect("local JSON")).expect("local evidence");
}
fn require(fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, ok: bool, detail: impl Display) -> Journey<()> {
    record(ck, ev, name, ok, detail);
    save(fx, "checkpoint", &json!({"step":name,"passed":ck.passed,"failed":ck.failed,"checks":ev}));
    if ok { Ok(()) } else { Err(name.into()) }
}
async fn host(app: &AppHandle, command: &str, args: Value) -> Journey<Value> {
    let reply = ipc(app, "main", command, args).await;
    if !succeeded(&reply) { return Err(format!("{command}: {}", reply.describe())); }
    let result = value(&reply);
    if result.get("success").is_some() {
        if result["success"] == true { Ok(result["data"].clone()) } else { Err(format!("{command}: {result}")) }
    } else { Ok(result) }
}
async fn dom(app: &AppHandle, args: Value, body: &str) -> Value {
    eval_long(app, &format!("const stage6UI=true;\n{DOM}\nconst A={args};\n{body}")).await
}
async fn step(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, args: Value, body: &str) -> Journey<Value> {
    let reply = dom(app, args, body).await;
    require(fx, ck, ev, name, ui_ok(&reply), reply.to_string())?;
    Ok(reply["value"].clone())
}
async fn settled(app: &AppHandle, sid: &str) -> Journey<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let session = host(app, "get_agent_session", json!({"sessionId":sid})).await?;
        if session["status"] == "idle" && session["events"].as_array().is_some_and(|events| events.iter().any(|event| event["kind"] == "userMessage")) { return Ok(session); }
        if tokio::time::Instant::now() >= deadline { return Err(format!("fixture turn timeout: {session}")); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn journey(app: &AppHandle, fx: &Fx, ck: &mut Checks, ev: &mut Vec<Value>) -> Journey<()> {
    let (account, _) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|_| "local account absent")?;
    let owner = account.user_id().to_owned();
    let current = host(app, "get_cli_preferences", json!({})).await?;
    let mut paths = current["paths"].clone();
    paths["codex"] = json!(fx.root.join("bin/codex"));
    paths["claude"] = json!(fx.root.join("bin/claude"));
    let saved = host(app, "set_cli_preferences", json!({"paths":paths,"expectedRevision":current["revision"]})).await?;
    require(fx, ck, ev, "host preferences select only deterministic local executable peers", saved["saved"] == true, saved.to_string())?;
    let providers = app.state::<dgos_providers::ProviderManager>();
    let vault = app.state::<Arc<dgos_providers::test_support::MemoryCredentialVault>>();
    let provider = dgos_providers::test_support::seed_provider(fx.db.clone(), vault.inner(), dgos_types::ProviderConfig { name: "Phase6 local protocol ONLY".into(), provider_type: "openai_compatible".into(), base_url: "http://127.0.0.1:9/v1".into(), api_key: "synthetic-never-transmitted".into(), enabled: true, config: Some(json!({"allow_insecure_http":true,"auth_style":"bearer"})) }, &owner).await.map_err(|error| error.to_string())?;
    providers.add_manual_model(&provider.id, &owner, "fixture-model", None).await.map_err(|error| error.to_string())?;
    let path = fx.root.join("project-a");
    make_repo(&path);
    fs::write(path.join("source.txt"), "local source\n").map_err(|error| error.to_string())?;
    let project = host(app, "create_development_project", json!({"input":{"kind":"import","name":"Phase6 local A","path":path,"source":null}})).await?;
    require(fx, ck, ev, "local project imported through production IPC", project["id"].is_string(), project.to_string())?;
    require(fx, ck, ev, "reload shipped workbench with isolated catalog", goto(app, "main", "/tasks/agent").await, "production App")?;
    step(app, fx, ck, ev, "observe real IPC and keyboard receipts without replacing responses", json!({"project":project["id"]}), r#"
const original=window.fetch,p={original,events:[],keys:[]};window.__phase6=p;
window.fetch=function(url,options,...rest){p.events.push({command:String(url).split('/').at(-1),args:JSON.parse(options?.body||'{}'),at:Date.now()});return original.call(this,url,options,...rest);};
document.addEventListener('keydown',e=>p.keys.push({key:e.key,isTrusted:e.isTrusted,target:e.target?.id||e.target?.className}),true);
const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();await click('dev-project-item-'+A.project);return {ok:!!await until(()=>tid('dev-project-conversation'))};
"#).await?;
    step(app, fx, ck, ev, "select synthetic model and explicitly authorize first send", json!({}), r#"
await click('composer-model-picker-trigger');const d=await need('composer-model-picker-dialog');const option=await until(()=>$$('.composer-model-picker-option',d).find(e=>txt(e).includes('fixture-model')&&!e.disabled));if(!option)throw Error('synthetic provider route unavailable');option.click();if(!await until(()=>!tid('composer-model-picker-dialog')))throw Error('model selection not committed');const trust=await need('direct-conversation-trust');if(!trust.checked)trust.click();setInput(document.getElementById('agent-message'),'PHASE6 LOCAL explicit protocol request; no model');const send=await until(()=>{const b=tid('direct-conversation-send');return b&&!b.disabled&&b});if(!send)throw Error('first send disabled');send.click();return {ok:!!await until(()=>!tid('dev-project-conversation')&&document.getElementById('agent-message'))};
"#).await?;
    let sessions = host(app, "list_agent_sessions", json!({})).await?;
    let rows = sessions.as_array().ok_or("session list absent")?;
    require(fx, ck, ev, "first explicit send creates one real host session", rows.len() == 1, sessions.to_string())?;
    let sid = rows[0]["id"].as_str().ok_or("session ID absent")?.to_owned();
    let original = settled(app, &sid).await?;
    let workspace = host(app, "read_development_workspace", json!({})).await?;
    let task = workspace["selection"]["task_id"].as_str().ok_or("task absent")?.to_owned();
    save(fx, "session", &original);
    require(fx, ck, ev, "prepare real WKWebView for native synthesized keyboard", ui_keys::prepare(app).await, "NSEvent, not physical input")?;
    step(app, fx, ck, ev, "record independently scrolled transcript and unsent draft", json!({}), r#"
const transcript=await until(()=>{const e=$$('.agent-dialogue-transcript',tid('dev-panel-agent'))[0];return e&&txt(e).includes('row 59')&&e});if(!transcript)throw Error('native transcript not rendered');if(transcript.scrollHeight<=transcript.clientHeight+200)throw Error('fixture transcript does not scroll');transcript.scrollTop=120;transcript.dispatchEvent(new Event('scroll',{bubbles:true}));const composer=document.getElementById('agent-message');setInput(composer,'unsent phase6 draft');window.__phase6.reading={transcript,composer,top:transcript.scrollTop};const b=await need('dev-tool-files');b.focus();return {ok:document.activeElement===b,top:transcript.scrollTop,height:transcript.scrollHeight};
"#).await?;
    ui_keys::key(app, Key::Enter).await;
    step(app, fx, ck, ev, "native keyboard opens files without changing conversation identity or reading", json!({}), r#"
await need('dev-file-search');const p=window.__phase6.reading;return {ok:p.composer===document.getElementById('agent-message')&&p.transcript.isConnected&&p.composer.value==='unsent phase6 draft'&&Math.abs(p.transcript.scrollTop-p.top)<2&&document.activeElement!==p.composer,top:p.transcript.scrollTop,active:document.activeElement?.outerHTML};
"#).await?;
    step(app, fx, ck, ev, "open source in production main editor", json!({}), "await fill('dev-file-search','source.txt');await click('dev-file-item-source.txt');return {ok:!!await until(()=>tid('dev-file-editor')?.value==='local source\\n')};").await?;
    step(app, fx, ck, ev, "return from source preserves hidden transcript and composer", json!({}), r#"
const nav=$$('.dev-main-view-nav')[0],b=$$('button',nav).find(e=>txt(e)==='对话');if(!b)throw Error('conversation view control absent');b.click();const p=window.__phase6.reading;return {ok:!!await until(()=>visible(p.composer))&&p.composer===document.getElementById('agent-message')&&p.composer.value==='unsent phase6 draft'&&Math.abs(p.transcript.scrollTop-p.top)<2,top:p.transcript.scrollTop};
"#).await?;
    step(app, fx, ck, ev, "close only conversation tab after explicit draft discard", json!({}), r#"
await new Promise(requestAnimationFrame);const composer=document.getElementById('agent-message');if(!composer||!visible(composer))throw Error('conversation absent');setInput(composer,'');const tab=$$('.dev-conversation-tab[data-selected=true]')[0];if(!tab)throw Error('active tab absent');const buttons=$$('button',tab);buttons.at(-1).focus();return {ok:document.activeElement===buttons.at(-1)};
"#).await?;
    ui_keys::key(app, Key::Enter).await;
    step(app, fx, ck, ev, "native Enter closes tab without stopping host session", json!({"sid":sid}), r#"
if(!await until(()=>!document.getElementById('agent-message')))throw Error('tab remains open');const p=window.__phase6;return {ok:!p.events.some(e=>['interrupt_agent_session','close_agent_session'].includes(e.command)),events:p.events,keys:p.keys};
"#).await?;
    let retained = host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    require(fx, ck, ev, "tab close preserves original native identity and history", retained["native_id"] == original["native_id"] && retained["events"] == original["events"] && retained["status"] == "idle", retained.to_string())?;
    step(app, fx, ck, ev, "history reopen only attaches existing host session", json!({"sid":sid}), r#"
const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();const row=await need('dev-session-item-'+A.sid),b=row.querySelector('.dev-session-select');if(!b||b.disabled)throw Error('history select unavailable');b.click();return {ok:!!await until(()=>document.getElementById('agent-message'))};
"#).await?;
    let protocol = fs::read_to_string(path.join("phase6-protocol.jsonl")).map_err(|error| error.to_string())?;
    let requests = protocol.lines().map(serde_json::from_str::<Value>).collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    require(fx, ck, ev, "one explicit native protocol turn and no automatic resend", requests.iter().filter(|request| request["method"] == "turn/start").count() == 1, protocol)?;
    step(app, fx, ck, ev, "focus explicit session end after non-destructive tab reopen", json!({}), "const b=btn('结束会话');if(!b)throw Error('explicit session end absent');b.focus();return {ok:document.activeElement===b};").await?;
    ui_keys::key(app, Key::Enter).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if host(app, "get_agent_session", json!({"sessionId":sid})).await?["status"] == "closed" { break; }
        if tokio::time::Instant::now() >= deadline { return Err("explicit native UI session close timeout".into()); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let closed = host(app, "get_agent_session", json!({"sessionId":sid})).await?;
    require(fx, ck, ev, "explicit host session close reaches confirmed terminal state", closed["status"] == "closed", closed.to_string())?;
    // Respect the production directory lease: start the real host PTY only
    // after explicit native session close. Never release occupancy implicitly.
    step(app, fx, ck, ev, "open real production runtime view", json!({}), r#"
const b=$$('button',$$('.dev-main-view-nav')[0]).find(e=>txt(e)==='终端与运行');if(!b)throw Error('runtime view control absent');b.click();await need('dev-development-runtime');return {ok:true};
"#).await?;
    let terminal = host(app, "start_development_process", json!({"input":{"id":uuid::Uuid::new_v4().to_string(),"task_id":task,"kind":"terminal","program":"/bin/sh","args":["-c","stty -echo; printf 'PHASE6_PTY_READY\\n'; read line; stty size; printf 'PHASE6_PTY_INPUT:%s\\n' \"$line\"; sleep 30"],"cwd":"","env_refs":{},"trust_workspace":true,"verification":false,"readiness":null,"native_run_id":null}})).await?;
    let id = terminal["id"].as_str().ok_or("PTY id absent")?.to_owned();
    step(app, fx, ck, ev, "select exact native PTY receipt", json!({"id":id}), r#"
const refresh=await until(()=>document.querySelector('section[aria-label="宿主执行历史"] > .dev-review-toolbar button'));if(!refresh)throw Error('history refresh absent');refresh.click();const row=await until(()=>tid('dev-process-history-'+A.id));if(!row)throw Error('PTY receipt absent');row.click();const f=await until(()=>tid('dev-terminal')?.querySelector('.xterm-helper-textarea'));if(!f)throw Error('xterm input absent');f.focus();return {ok:document.activeElement===f};
"#).await?;
    host(app, "resize_development_terminal", json!({"input":{"id":id,"cols":101,"rows":37}})).await?;
    host(app, "write_development_terminal", json!({"input":{"id":id,"data":"phase6-native-enter"}})).await?;
    ui_keys::key(app, Key::Enter).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let receipt = loop {
        let receipt = host(app, "read_development_process", json!({"id":id})).await?;
        if receipt["output"].as_str().is_some_and(|text| text.contains("PHASE6_PTY_INPUT:phase6-native-enter")) { break receipt; }
        if tokio::time::Instant::now() >= deadline { return Err(format!("native PTY Enter timeout: {receipt}")); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    require(fx, ck, ev, "native xterm Enter forwards one original PTY input and explicit size", receipt["task_id"] == task && receipt["output"].as_str().is_some_and(|text| text.contains("37 101") && text.matches("PHASE6_PTY_INPUT:phase6-native-enter").count() == 1), receipt.to_string())?;
    save(fx, "pty", &receipt);
    let stopped = host(app, "stop_development_process", json!({"id":id})).await?;
    require(fx, ck, ev, "explicit stop confirms original PTY cleanup", stopped["finished_at"].is_number() && stopped["status"] == "cancelled", stopped.to_string())?;
    let observed = step(app, fx, ck, ev, "record exact production IPC and trusted native key counts", json!({}), r#"
const p=window.__phase6;return {ok:p.events.filter(e=>e.command==='open_agent_session').length===1&&p.events.filter(e=>e.command==='send_agent_message').length===1&&p.keys.some(e=>e.isTrusted&&e.key==='Enter'),events:p.events,keys:p.keys};
"#).await?;
    save(fx, "ipc-native-keys", &observed);
    Ok(())
}

pub(super) async fn run(app: &AppHandle, fx: &Fx, mut ck: Checks, mut ev: Vec<Value>) {
    save(fx, "safety", &json!({"phase":"phase6-local","database":fx.root.join("dgos.db"),"vault":"MemoryCredentialVault","provider":"synthetic loopback port 9; deterministic child never performs HTTP","native_cli":false,"model_calls":0,"configuration_roots":["home","codex-home","claude-home","config","cache","data"],"source":"production App/WKWebView/IPC/components with deterministic protocol subprocess","keyboard":"synthesized NSEvents, not physical keyboard","browser_replacement":false}));
    let renderer = dom(app, json!({}), "return {ok:!!window.__TAURI_INTERNALS__&&location.protocol!=='http:'&&location.protocol!=='https:',url:location.href,userAgent:navigator.userAgent,scripts:[...document.querySelectorAll('script[src]')].map(e=>e.src)};").await;
    save(fx, "renderer", &renderer);
    record(&mut ck, &mut ev, "shipped non-browser-replacement renderer provenance", ui_ok(&renderer), renderer.to_string());
    let result = journey(app, fx, &mut ck, &mut ev).await;
    if let Err(error) = result {
        record(&mut ck, &mut ev, "local native acceptance completes", false, error);
        save(fx, "failure-dom", &dom(app, json!({}), "return {ok:true,body:document.body.innerText,active:document.activeElement?.outerHTML,events:window.__phase6?.events,keys:window.__phase6?.keys};").await);
    }
    let _ = dom(app, json!({}), "if(window.__phase6)window.fetch=window.__phase6.original;return {ok:true};").await;
    record(&mut ck, &mut ev, "all owned local protocol sessions and PTYs stop", app.state::<AgentManager>().shutdown().await, "production shutdown");
    save(fx, "result", &json!({"passed":ck.passed,"failed":ck.failed,"checks":ev,"native_cli":false,"model_calls":0,"limitations":["Deterministic protocol fixture, not real Codex/Claude/model acceptance","Native synthesized keys, not physical keyboard/OS IME","Separate browser matrix is supplementary, not a WebView replacement"]}));
    app.exit(if ck.failed.is_empty() { 0 } else { 1 });
}
