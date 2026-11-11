//! Project-task acceptance and the default agent smoke (V7 phase 1;
//! docs/02-产品与版本/后续版本/V7-项目任务基础技术契约.md).
//!
//! Default (`DGOS_PROJECT_ACCEPTANCE_ROOT` unset): both engines, one project task,
//! handshake, snapshot and close through production IPC.
//! `DGOS_AGENT_CLOSE_ONLY=1` selects a focused real-Codex surface close/cancel run, with no
//! model-answer assertion; result-close-boundary.json records whether initial-open pending was observed.
//! Acceptance: `DGOS_PROJECT_ACCEPTANCE_ROOT=<dir>` plus
//! `DGOS_PROJECT_ACCEPTANCE_PHASE=first|restart|stage3-first|stage3-restart|stage4-ui|stage6-first|stage6-restart|terminal-activity|worktree-runtime`.
//! `<dir>` holds the database, fixture projects and evidence shared by both processes.
//! Stage6 requires a fresh directory under `/private/tmp` for its first phase; it persists
//! `stage6-checkpoint.json` per action and model snapshots while waiting, stopping on failure.
//! `terminal-activity` uses only /bin/sh PTYs and the production terminal UI; no CLI/provider run.
//! Build the production dist first, then use a fresh root and the existing webview_smoke_agents test.
//! Evidence: stage3-result-terminal-activity.json plus terminal-activity-checkpoint.json and IPC trace.
//! Set `DGOS_TERMINAL_TYPOGRAPHY_ONLY=1` with `terminal-activity` for live installed-font/size
//! changes, native PTY identity/stty and rendered Latin/CJK evidence. Retains terminal-typography.json/png.
//! Requires a foreground macOS desktop and Screen Recording permission; no model provider or CLI run.
//! By default `phase6-local` is the safe deterministic App/WKWebView/IPC mode:
//! a fresh temporary root, isolated HOME/config roots, memory vault and
//! protocol-only peers. It never reads the user's native CLI configuration.
//! Explicit real opt-in: add `DGOS_CODEX_REAL_LIFECYCLE_SMOKE=1` and authorized
//! `DGOS_STAGE6_CODEX_URL`, `DGOS_STAGE6_CODEX_KEY`, `DGOS_STAGE6_CODEX_MODEL` to
//! `phase6-local`. This bypasses every protocol fixture, pins installed Codex
//! 0.167.0, isolates native config and uses only the selected memory-vault Provider.
//! The real journey prepares without dispatch, selects Plan/high/read_only before
//! the first actual model turn, steers and interrupts a bounded read-only task,
//! sends again, closes and resumes without replay. Native approvals are denied
//! only if genuinely requested; absence is explicitly uncovered, never fabricated.
//! Evidence is content-free `phase6-real-codex-*.json` plus masked actual-window
//! captures before first dispatch and after resume; no raw protocol/transcript dump.
//! `worktree-runtime` verifies actual task-worktree npm/Cargo execution, captures macOS editor
//! open events with a temporary local application, and checks real IPC owner/lease rejection.
//! The real CLIs, AccountService, SessionResolver, HostSessionPort and AgentManager run
//! in production configuration. The main flow is driven in the shipped `main` WebView
//! through its DOM (sign-in, project import, task, session start/send/close, task
//! switch, resume, relocate, account switch). Account setup, project lifecycle and
//! cross-account refusals use production IPC as supplementary evidence.
//! Session startup uses the production composer model route and one-time directory confirmation;
//! remembered grants automatically prepare without a message. Saved history resumes on composer
//! focus, and terminal/service permissions are independently confirmed rather than inferred.

use super::{goto, ipc, wait_load, Checks, LoadEvents, Pending, Reply, NEXT_ID};
use crate::commands;
use crate::session::{HostSessionPort, SessionResolver};
use dgos_agents::AgentManager;
use dgos_application::{AccountPorts, AccountService};
use dgos_db::sea_orm::DatabaseConnection;
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::{CliRepository, DevelopmentRepository};
use dgos_mcp::McpManager;
use serde_json::{json, Value};
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::{broadcast, oneshot};

const ACCEPTANCE_WATCHDOG: Duration = Duration::from_secs(3600);
const DEFAULT_WATCHDOG: Duration = Duration::from_secs(900);
const STEP_TIMEOUT: Duration = Duration::from_secs(330);
const TURN_TIMEOUT: Duration = Duration::from_secs(90);
const ENGINES: [&str; 2] = ["codex", "claude"];
const ALICE: (&str, &str) = ("alice", "alice-password");
const BOB: (&str, &str) = ("bob", "bob-password");
const RECALL: &str = "What code did I ask you to remember? Reply with the code only.";


#[derive(Clone)]
struct Fx {
    root: PathBuf,
    db: DatabaseConnection,
}

async fn open_fx(root: &Path) -> Fx {
    fs::create_dir_all(root).expect("acceptance root");
    let root = root.canonicalize().expect("canonical root");
    let url = format!("sqlite://{}?mode=rwc", root.join("dgos.db").display());
    let db = dgos_db::establish_connection(&url).await.expect("database");
    dgos_db::migrations::Migrator::up(&db, None).await.expect("migrations");
    Fx { root, db }
}

/// Production construction: the repository is injected, `load` reads the legacy index once.
async fn load_agents(fx: &Fx) -> (AgentManager, dgos_agents::capabilities::CliProviders, dgos_providers::ProviderManager, Arc<dgos_providers::test_support::MemoryCredentialVault>) {
    let repository = DevelopmentRepository::new(fx.db.clone());
    let vault = Arc::new(dgos_providers::test_support::MemoryCredentialVault::default());
    let providers = dgos_providers::ProviderManager::with_vault(&format!("sqlite://{}?mode=rwc",fx.root.join("dgos.db").display()),vault.clone()).await.expect("smoke providers");
    let service = dgos_agents::capabilities::CliProviders::new(repository.clone(), providers.clone()).expect("CLI provider service");
    let manager = AgentManager::new(fx.root.join("agents.json"), repository, service.clone());
    manager.load().await.expect("agent index");
    (manager, service, providers, vault)
}

/// Records one check on stdout and in the evidence list.
fn record(ck: &mut Checks, ev: &mut Vec<Value>, name: &str, ok: bool, detail: impl Display) {
    let detail = detail.to_string();
    ck.check(name, ok, &detail);
    ev.push(json!({"check": name, "ok": ok, "detail": detail}));
    let phase = std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").unwrap_or_default();
    if phase.starts_with("stage6-") || phase == "terminal-activity" {
        if let Some(fx) = ck_stage6_root() {
            let name = if phase == "terminal-activity" { "terminal-activity-checkpoint.json" } else { "stage6-checkpoint.json" };
            fs::write(fx.join(name), serde_json::to_vec_pretty(&json!({"passed":ck.passed,"failed":ck.failed,"checks":ev})).expect("checkpoint JSON")).expect("acceptance checkpoint");
        }
    }
}

fn ck_stage6_root() -> Option<PathBuf> {
    std::env::var_os("DGOS_PROJECT_ACCEPTANCE_ROOT").map(PathBuf::from)
}

fn value(reply: &Reply) -> Value {
    reply.0["value"].clone()
}

fn succeeded(reply: &Reply) -> bool {
    reply.0["ok"] == true
}



/// Runs `body` in the main WebView with the long deadline (one model turn can take minutes).
async fn eval_long(app: &AppHandle, body: &str) -> Value {
    let Some(window) = app.get_webview_window("main") else {
        return json!({"ok": false, "error": "no such window"});
    };
    let id = format!("r{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
    let (sender, receiver) = oneshot::channel();
    app.state::<Pending>().0.lock().unwrap().insert(id.clone(), sender);
    let script = format!(
        "(async()=>{{let __r;try{{__r={{ok:true,value:await (async()=>{{{body}}})()}};}}\
         catch(e){{__r={{ok:false,error:String((e&&e.message)||e)}};}}\
         await window.__TAURI_INTERNALS__.invoke('smoke_report',{{id:{id:?},payload:JSON.stringify(__r)}});}})()"
    );
    if window.eval(script).is_err() {
        app.state::<Pending>().0.lock().unwrap().remove(&id);
        return json!({"ok": false, "error": "eval failed"});
    }
    let timeout = if std::env::var_os("DGOS_TERMINAL_TYPOGRAPHY_PREFLIGHT").is_some() { Duration::from_secs(30) } else if std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| phase.starts_with("stage6-")) { Duration::from_secs(60) } else { STEP_TIMEOUT };
    match tokio::time::timeout(timeout, receiver).await {
        Ok(Ok(payload)) => serde_json::from_str(&payload).unwrap_or(Value::Null),
        _ => {
            app.state::<Pending>().0.lock().unwrap().remove(&id);
            json!({"ok": false, "error": "harness timeout"})
        }
    }
}

/// DOM helpers. Testids follow the dev workbench contract; AgentWorkbench controls are matched by
/// their shipped ids, labels and text.
const DOM: &str = r##"
const until = async (fn, ms = 20000) => { const t = Date.now(); while (Date.now() - t < ms) { try { const v = fn(); if (v) return v; } catch (e) {} await new Promise((r) => setTimeout(r, 50)); } return null; };
const visible = (el) => !!el && el.isConnected && !el.closest('[hidden],[inert],[aria-hidden="true"]') && getComputedStyle(el).display !== 'none' && getComputedStyle(el).visibility !== 'hidden' && el.getClientRects().length > 0;
const $$ = (s, r = document) => [...r.querySelectorAll(s)].filter(el => !stage6UI || visible(el));
const txt = (el) => ((el && el.textContent) || '').trim();
const labelOf = (el) => { if (el.labels && el.labels[0]) { const label=el.labels[0].cloneNode(true); for(const control of label.querySelectorAll('input,textarea,select,button'))control.remove(); return txt(label); } const by = el.getAttribute('aria-labelledby'); if (by) return txt(document.getElementById(by)); return el.getAttribute('aria-label') || ''; };
const tid = (id) => stage6UI ? $$('[data-testid]').find(el => el.getAttribute('data-testid') === id) : document.querySelector(`[data-testid="${id}"]`);
const need = async (id, ms = 20000) => { const el = await until(() => tid(id), ms); if (!el) throw new Error('missing ' + id); return el; };
const setInput = (f, v) => { const p = f.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(p, 'value').set.call(f, v); f.dispatchEvent(new Event('input', { bubbles: true })); };
const fill = async (id, v) => { const el = await need(id); const f = el.matches('input,textarea') ? el : el.querySelector('input,textarea'); if (!f) throw new Error('no field in ' + id); setInput(f, v); return true; };
const click = async (id) => { (await need(id)).click(); return true; };
const btn = (t) => $$('button', stage6UI ? ($$('.agent-workbench')[0] || document) : document).find((b) => (b.getAttribute('aria-label') || txt(b)) === t && !b.disabled);
const items = (prefix) => $$(`[data-testid^="${prefix}"]`).map((e) => ({ id: e.getAttribute('data-testid').slice(prefix.length), text: txt(e) }));
const itemFor = (prefix, text) => items(prefix).find((i) => i.text.includes(text));
const isChecked = (c) => c.checked === true || c.getAttribute('aria-checked') === 'true';
const msgs = () => { const m = document.querySelector('.agent-messages'); return m ? m.textContent : ''; };
// Exercise shipped controls only: grants are scoped host receipts, never renderer checkboxes.
const confirmWorkspaceGrant = async () => {
 const dialog=await until(()=>$$('[role=dialog],[role=alertdialog]').find(e=>$$('button',e).some(b=>/^(允许并记住|Allow and remember)$/.test(txt(b)))));
 if(!dialog)throw Error('workspace permission confirmation absent');
 const allow=$$('button',dialog).find(b=>/^(允许并记住|Allow and remember)$/.test(txt(b))&&!b.disabled);if(!allow)throw Error('workspace permission confirmation disabled');allow.click();
 if(!await until(()=>!dialog.isConnected))throw Error('workspace permission confirmation not committed');
};
const chooseConversationModel = async (engine, providerName=null, modelId=null) => {
 await showConversationView();
 let nextOpen=0;const dialog=await until(()=>{const d=tid('composer-model-picker-dialog');if(d)return d;const b=tid('composer-model-picker-trigger');if(b&&!b.disabled&&b.getAttribute('aria-expanded')!=='true'&&Date.now()>=nextOpen){nextOpen=Date.now()+500;b.click();}return null;});if(!dialog)throw Error('model picker unavailable');const name=engine==='codex'?'Codex':'Claude';
 const provider=dialog.querySelector('.composer-model-picker-provider-select select');if(providerName&&provider){const option=[...provider.options].find(o=>o.textContent===providerName);if(!option)throw Error('Provider unavailable: '+providerName);provider.value=option.value;provider.dispatchEvent(new Event('change',{bubbles:true}));}
 const advanced=dialog.querySelector('.composer-model-picker-footer details');if(advanced)advanced.open=true;
 const route=await until(()=>$$('.composer-model-picker-routes > div',dialog).filter(row=>!modelId||txt(row.querySelector('span'))===modelId).flatMap(row=>$$('button',row)).find(b=>txt(b)===name&&!b.disabled));if(!route)throw Error('selected model route unavailable');route.click();
 if(!await until(()=>!tid('composer-model-picker-dialog')))throw Error('model selection not committed');
};
const authorizeConversation = async () => {
 if(!await until(()=>!tid('dev-project-conversation')&&document.getElementById('agent-message')))throw Error('automatically trusted conversation did not prepare');
};
const prepareSavedConversation = async sid => {
 await showConversationView();
 const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();
 const select=await until(()=>{const b=tid('dev-session-item-'+sid);return b&&!b.disabled&&b});if(!select)throw Error('saved session select unavailable');if(select.getAttribute('aria-pressed')!=='true'){select.focus();select.click();}
 const entry=await until(()=>tid('dev-session-item-'+sid)?.getAttribute('aria-pressed')==='true'&&(tid('saved-conversation-composer')||document.getElementById('agent-message')));if(!entry)throw Error('selected saved conversation composer absent');
 if(tid('saved-conversation-composer')){
  const start=await until(()=>{const s=tid('saved-conversation-composer');if(!s)return null;const grant=$$('button',s).find(b=>/^(信任项目并开始|Trust project and start)$/.test(txt(b)));if(grant)return grant.disabled?null:grant;const recovery=s.querySelector('details button');return recovery&&!recovery.disabled?s.querySelector('textarea:not(:disabled)'):null});
  if(!start)throw Error('saved conversation continuation unavailable');
  if(start.tagName==='BUTTON'){
   start.click();const confirmation=await until(()=>$$('[role=dialog],[role=alertdialog]').some(e=>$$('button',e).some(b=>/^(允许并记住|Allow and remember)$/.test(txt(b))))?'confirm':!tid('saved-conversation-composer')&&document.getElementById('agent-message')?'prepared':null);
   if(confirmation==='confirm')await confirmWorkspaceGrant();else if(confirmation!=='prepared')throw Error('saved conversation authorization outcome absent');
  }else start.focus();
 }
 if(!await until(()=>!tid('saved-conversation-composer')&&document.getElementById('agent-message')&&!document.getElementById('agent-message').disabled))throw Error('saved conversation native prepare did not finish');
};
const authorizeRuntime = async scope => {
 const root=await need('dev-development-runtime'),label=scope==='service'?/^(可与 Agent 编辑同时运行的开发服务|Development services that run alongside Agent editing)$/:/^(Shell 命令与交互终端|Shell commands and interactive terminals)$/;
 const details=[...root.querySelectorAll('details')].find(d=>[...d.querySelectorAll('span')].some(s=>label.test(txt(s))));if(!details)throw Error(scope+' permission controls absent');for(let parent=details;parent&&root.contains(parent);parent=parent.parentElement){if(parent.tagName==='DETAILS')parent.open=true;}
 const row=await until(()=>$$('.dev-review-toolbar',details).find(r=>$$('span',r).some(s=>label.test(txt(s)))));if(!row)throw Error(scope+' permission row absent');
 const button=await until(()=>$$('button',row).find(b=>!b.disabled));if(!button)throw Error(scope+' permission action unavailable');
 if(/^(撤销授权|Revoke permission)$/.test(txt(button)))return;
 if(!/^(允许项目运行|Allow project to run)$/.test(txt(button)))throw Error('unexpected '+scope+' permission action');button.click();await confirmWorkspaceGrant();
 if(!await until(()=>$$('button',row).some(b=>/^(撤销授权|Revoke permission)$/.test(txt(b)))))throw Error(scope+' grant not reflected');
};
const showRuntimeView = async () => {
 const projects=tid('dev-tool-projects');if(document.querySelector('.agent-workbench')?.dataset.compact==='true'&&projects?.getAttribute('aria-expanded')==='true')projects.click();
 const button=await until(()=>$$('.dev-main-view-nav button').find(b=>/^(终端与运行|Terminal and runs)$/.test(txt(b))&&!b.disabled));if(!button)throw Error('runtime view control absent');button.click();
 const processes=await need('dev-runtime-processes');processes.click();await need('dev-development-runtime');
};
const showConversationView = async () => {
 const button=await until(()=>$$('.dev-main-view-nav button').find(b=>/^(对话|Conversation)$/.test(txt(b))&&!b.disabled));if(!button)throw Error('conversation view control absent');button.click();
};
const showProjectGoals = async () => {
 const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();
 const more=await until(()=>document.querySelector('.dev-tree-project-row.is-current button[aria-haspopup="menu"]'));if(!more||more.disabled)throw Error('current project action menu unavailable');
 more.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,pointerType:'mouse',ctrlKey:false}));more.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,button:0,pointerType:'mouse'}));
 const goals=await until(()=>$$('[role=menuitem]').find(item=>/^(目标与完成要求|Goals and completion requirements)$/.test(txt(item))));if(!goals)throw Error('project goals action absent');goals.click();await need('dev-task-new');
};
"##;

/// Evaluates one UI step: the shared helpers, then `A` (the arguments), then `body`.
async fn ui(app: &AppHandle, args: Value, body: &str) -> Value {
    let stage6 = std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| phase.starts_with("stage6-"));
    eval_long(app, &format!("const stage6UI = {stage6};\n{DOM}\nconst A = {args};\n{body}")).await
}

fn ui_ok(reply: &Value) -> bool {
    reply["ok"] == true && reply["value"]["ok"] == true
}




async fn sessions_of(app: &AppHandle) -> Vec<Value> {
    value(&ipc(app, "main", "list_agent_sessions", json!({})).await).as_array().cloned().unwrap_or_default()
}

fn ids(rows: &[Value]) -> Vec<String> {
    rows.iter().filter_map(|r| r["id"].as_str().map(str::to_owned)).collect()
}

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").arg("-C").arg(dir).args(args).status().is_ok_and(|s| s.success())
}

/// A real Git repository with one commit, created by the harness.
fn make_repo(dir: &Path) {
    fs::create_dir_all(dir).expect("repo dir");
    assert!(git(dir, &["init", "-q"]), "git init");
    fs::write(dir.join("README.md"), "acceptance\n").expect("readme");
    assert!(git(dir, &["add", "README.md"]), "git add");
    assert!(git(dir, &["-c", "user.name=dgos", "-c", "user.email=dgos@example.invalid", "commit", "-q", "-m", "init"]), "git commit");
}

fn token(prefix: &str) -> String {
    format!("{prefix}{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// Signs in through the shipped AccountGate: picks the user, types the password, submits.
async fn ui_sign_in(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, creds: (&str, &str)) -> bool {
    let reply = if goto(app, "main", "/").await {
        ui(app, json!({"user": creds.0, "pw": creds.1}), SIGN_IN).await
    } else {
        json!({"ok": false, "error": "gate did not load"})
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut signed_in = false;
    for _ in 0..100 {
        let state = ipc(app, "main", "get_account_state", json!({})).await;
        if state.data()["current"]["username"] == creds.0 { signed_in = true; break; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    record(ck, ev, "host confirms signed-in identity", signed_in, creds.0);
    record(ck, ev, &format!("{} signed in through the gate", creds.0), ui_ok(&reply), reply.to_string());
    ui_ok(&reply) && signed_in
}

/// Signs out through the AccountMenu of the signed-in user.
async fn ui_sign_out(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, user: &str) -> bool {
    let reply = ui(app, json!({"user": user}), SIGN_OUT).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut signed_out = false;
    for _ in 0..100 {
        let state = ipc(app, "main", "get_account_state", json!({})).await;
        if state.data()["mode"] == "signed_out" { signed_out = true; break; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    record(ck, ev, &format!("{user} signed out through the account menu"), ui_ok(&reply) && signed_out, reply.to_string());
    ui_ok(&reply) && signed_out
}

const SIGN_IN: &str = r#"
const option = await until(() => $$('label,button,[role=radio]').find((el) => txt(el) === A.user));
if (!option) throw new Error('no account option ' + A.user);
option.click();
const pass = await until(() => $$('input[type=password]')[0]);
if (!pass) throw new Error('no password field');
setInput(pass, A.pw);
const submit = await until(() => btn('登录'));
if (!submit) throw new Error('no sign-in button');
setTimeout(() => submit.click(), 100);
return {ok: true};
"#;

const SIGN_OUT: &str = r#"
const trigger = await until(() => $$('button').find((b) => b.getAttribute('aria-label') === A.user));
if (!trigger) throw new Error('no account menu for ' + A.user);
trigger.dispatchEvent(new PointerEvent('pointerdown', {bubbles:true,button:0,pointerType:'mouse',ctrlKey:false}));
trigger.dispatchEvent(new PointerEvent('pointerup', {bubbles:true,button:0,pointerType:'mouse'}));
const item = await until(() => $$('[role=menuitem],button').find((el) => txt(el) === '注销'));
if (!item) throw new Error('no sign-out item');
setTimeout(() => item.click(), 100);
return {ok: true};
"#;

/// Imports an existing fixture through Add project → Open folder and the real NSOpenPanel.
async fn ui_import_project(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, path: &Path) -> bool {
    let opened = ui(app, json!({}), IMPORT_PROJECT).await;
    record(ck, ev, &format!("{name} opens current project menu and native folder picker"), ui_ok(&opened), opened.to_string());
    if !ui_ok(&opened) { return false; }
    let picked = pick_project_directory(app, path).await;
    record(ck, ev, "real native directory panel selects fixture folder", picked, path.display().to_string());
    if !picked { return false; }
    // Allow the dialog plugin completion callback to publish the selected path before the list receipt.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let receipt = ui(app, json!({"name":path.file_name().unwrap_or_default().to_string_lossy(),"path":path.to_string_lossy()}), r#"
const norm=value=>{let s=String(value||'');while(s.endsWith('/'))s=s.slice(0,-1);return s;};
const rows=()=>$$('[data-testid^="dev-project-item-"]').map(e=>({id:e.getAttribute('data-testid'),path:e.querySelector('[title]')?.getAttribute('title'),text:txt(e),pressed:e.getAttribute('aria-pressed'),disabled:e.disabled}));
const expected=norm(A.path);
const row=await until(()=>rows().find(e=>norm(e.path)===expected),60000);
if(!row)return {ok:false,reason:'project row absent after native selection',expected,rows:rows(),location:location.pathname};
const selected=await until(()=>{const current=rows().find(e=>e.id===row.id);return current?.pressed==='true'?current:null},60000);
return {ok:!!selected,name:selected?.text,path:selected?.path,pressed:selected?.pressed,rows:rows(),expected};
"#).await;
    record(ck, ev, &format!("{name} imports and selects exact folder via production UI"), ui_ok(&receipt), receipt.to_string());
    ui_ok(&receipt)
}

#[cfg(target_os = "macos")]
async fn inspect_visible_native_folder(window: &tauri::WebviewWindow) -> Result<Option<Option<String>>, ()> {
    use objc2::{class, msg_send};
    use objc2::runtime::Bool;
    let (send, receive) = oneshot::channel();
    if window.with_webview(move |_| unsafe {
        let app: *mut objc2::runtime::AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let windows: *mut objc2::runtime::AnyObject = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        for index in 0..count {
            let candidate: *mut objc2::runtime::AnyObject = msg_send![windows, objectAtIndex: index];
            let is_panel: Bool = msg_send![candidate, isKindOfClass: class!(NSOpenPanel)];
            let visible: Bool = msg_send![candidate, isVisible];
            if !is_panel.as_bool() || !visible.as_bool() { continue; }
            let url: *mut objc2::runtime::AnyObject = msg_send![candidate, URL];
            if url.is_null() { let _ = send.send(Some(None)); return; }
            let path: *mut objc2::runtime::AnyObject = msg_send![url, path];
            let bytes: *const std::ffi::c_char = msg_send![path, UTF8String];
            let value = (!bytes.is_null()).then(|| std::ffi::CStr::from_ptr(bytes).to_string_lossy().into_owned());
            let _ = send.send(Some(value));
            return;
        }
        let _ = send.send(None);
    }).is_err() { return Err(()); }
    tokio::time::timeout(Duration::from_secs(2), receive).await.map_err(|_| ())?.map_err(|_| ())
}

#[cfg(target_os = "macos")]
async fn cancel_visible_native_folder(window: &tauri::WebviewWindow) {
    use objc2::{class, msg_send};
    use objc2::runtime::{AnyObject, Bool};
    let _ = window.with_webview(move |_| unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let windows: *mut AnyObject = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        for index in 0..count {
            let panel: *mut AnyObject = msg_send![windows, objectAtIndex: index];
            let is_panel: Bool = msg_send![panel, isKindOfClass: class!(NSOpenPanel)];
            let visible: Bool = msg_send![panel, isVisible];
            if is_panel.as_bool() && visible.as_bool() { let _: () = msg_send![panel, cancel: std::ptr::null_mut::<AnyObject>()]; return; }
        }
    });
}
#[cfg(target_os = "macos")]
async fn pick_project_directory(app: &AppHandle, path: &Path) -> bool {
    let Some(window)=app.get_webview_window("main") else { return false; };
    let Ok(path)=std::fs::canonicalize(path) else { return false; };
    // System Events keystrokes follow global focus even inside a process tell block.
    // Refuse them without verified foreground ownership; a configured directoryURL
    // is not proof that the remote native panel actually selected that directory.
    let navigate = r#"on run argv
tell application "System Events"
    tell (first application process whose unix id is (item 1 of argv as integer))
        set frontmost to true
        repeat 20 times
            if frontmost then exit repeat
            delay 0.1
        end repeat
        if not frontmost then error "native_picker_foreground_unavailable"
        keystroke "g" using {command down, shift down}
        repeat 20 times
            if exists text field 1 of sheet 1 of sheet 1 of window 1 then exit repeat
            delay 0.1
        end repeat
        tell text field 1 of sheet 1 of sheet 1 of window 1
            set value to (item 2 of argv)
            perform action "AXConfirm"
        end tell
        repeat 20 times
            if not (exists sheet 1 of sheet 1 of window 1) then exit repeat
            if exists row 2 of table 1 of scroll area 1 of sheet 1 of sheet 1 of window 1 then
                perform action "AXConfirm" of text field 1 of sheet 1 of sheet 1 of window 1
                exit repeat
            end if
            delay 0.1
        end repeat
    end tell
end tell
end run"#;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let navigation=tokio::process::Command::new("/usr/bin/osascript").args(["-e",navigate,&std::process::id().to_string(),&path.to_string_lossy()]).output().await;
    let navigated=matches!(&navigation, Ok(output) if output.status.success());
    let expected=path.to_string_lossy().into_owned();
    let mut selected=None;
    if navigated {
        for _ in 0..30 {
            match inspect_visible_native_folder(&window).await {
                Ok(Some(actual)) => {
                    selected=actual;
                    if selected.as_deref()==Some(expected.as_str()) { break; }
                }
                Ok(None) | Err(()) => break,
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    if !navigated || selected.as_deref()!=Some(expected.as_str()) {
        if let Some(root)=std::env::var_os("DGOS_PROJECT_ACCEPTANCE_ROOT") {
            let diagnostic=json!({"expected":expected,"selected":selected,"navigation_succeeded":navigated,"error":navigation.as_ref().map(|output|String::from_utf8_lossy(&output.stderr).into_owned()).unwrap_or_else(|error|error.to_string()),"confirmed":false});
            let _=fs::write(Path::new(&root).join("native-folder-failure.json"),serde_json::to_vec_pretty(&diagnostic).unwrap_or_default());
            let _=super::capture_native(app,Path::new(&root),"native-folder-navigation-failed").await;
        }
        cancel_visible_native_folder(&window).await;
        return false;
    }
    if let Some(root)=std::env::var_os("DGOS_PROJECT_ACCEPTANCE_ROOT") {
        let _=super::capture_native(app,Path::new(&root),"native-folder-before-confirm").await;
    }
    let confirm=r#"on run argv
tell application "System Events"
    tell (first application process whose unix id is (item 1 of argv as integer))
        if exists sheet 1 of sheet 1 of window 1 then error "native_picker_navigation_incomplete"
        perform action "AXPress" of (value of attribute "AXDefaultButton" of sheet 1 of window 1)
    end tell
end tell
end run"#;
    if !tokio::process::Command::new("/usr/bin/osascript").args(["-e",confirm,&std::process::id().to_string()]).status().await.is_ok_and(|s|s.success()) {
        cancel_visible_native_folder(&window).await;
        return false;
    }
    for _ in 0..30 {
        if matches!(inspect_visible_native_folder(&window).await, Ok(None)) { return true; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    cancel_visible_native_folder(&window).await;
    false
}

#[cfg(not(target_os = "macos"))]
async fn pick_project_directory(_: &AppHandle, _: &Path) -> bool { false }

/// Creates a task under the selected project through the task form.
async fn ui_new_task(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, goal: &str) -> bool {
    let reply = ui(app, json!({"goal": goal}), NEW_TASK).await;
    record(ck, ev, &format!("task '{goal}' saved via task form"), ui_ok(&reply), reply.to_string());
    ui_ok(&reply)
}

/// Selects a task in the list (switching only changes the projection).
async fn ui_select_task(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, goal: &str) -> bool {
    let reply = ui(app, json!({"goal": goal}), SELECT_TASK).await;
    record(ck, ev, &format!("select task '{goal}'"), ui_ok(&reply), reply.to_string());
    ui_ok(&reply)
}

/// Selects a production model route, confirms directory authority once and waits for automatic preparation.
async fn ui_start_session(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, engine: &str) -> bool {
    let reply = ui(app, json!({"engine": engine}), START_SESSION).await;
    record(ck, ev, &format!("{engine} session started from the UI"), ui_ok(&reply), reply.to_string());
    ui_ok(&reply)
}

/// Types a prompt into the Agent composer and presses send. Whether the model answered is
/// checked afterwards through `get_agent_session` events, not through UI text.
async fn ui_send(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, text: &str) -> bool {
    let reply = ui(app, json!({"text": text}), SEND).await;
    record(ck, ev, "prompt sent from the composer", ui_ok(&reply), reply.to_string());
    ui_ok(&reply)
}

/// Closes the running session with the close-run control.
async fn ui_close_run(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>) -> bool {
    let reply = ui(app, json!({}), CLOSE_RUN).await;
    record(ck, ev, "run closed from the UI", ui_ok(&reply), reply.to_string());
    ui_ok(&reply)
}

const IMPORT_PROJECT: &str = r#"
const projects=await need('dev-tool-projects');if(projects.getAttribute('aria-expanded')!=='true')projects.click();
const panel=await until(()=>document.getElementById('dev-navigation'));const add=panel?.querySelector('.dev-drawer-header button[aria-haspopup="menu"]');if(!add)throw Error('current Add project menu absent');
add.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,pointerType:'mouse',ctrlKey:false}));
add.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,button:0,pointerType:'mouse'}));
const item=await until(()=>$$('[role=menuitem]').find(e=>/打开文件夹|Open folder/.test(txt(e))));if(!item)throw Error('Open folder menu item absent');item.click();
return {ok:true};
"#;

const NEW_TASK: &str = r#"
await showProjectGoals();
await click('dev-task-new');
await until(() => { const f=tid('dev-task-goal');return f&&f.value===''; });
await need('dev-task-form');
await fill('dev-task-goal', A.goal);
await fill('dev-task-requirements', 'Keep the stated behaviour.');
const save=await until(()=>{const b=tid('dev-task-save');return b&&!b.disabled&&b});
if(!save)throw Error('task save disabled');
save.click();
const row = await until(() => $$('[data-testid^="dev-task-item-"]').find((e) => txt(e).includes(A.goal)));
return {ok: !!row};
"#;

const SELECT_TASK: &str = r#"
await showProjectGoals();
const row = await until(() => $$('[data-testid^="dev-task-item-"]').find((e) => txt(e).includes(A.goal)));
if (!row) throw new Error('no task row ' + A.goal);
row.click();
return {ok: true};
"#;

const START_SESSION: &str = r#"
await showConversationView();
await chooseConversationModel(A.engine);
await authorizeConversation();
const composer = await until(() => !tid('dev-project-conversation') && document.getElementById('agent-message'));
return {ok: !!composer, alerts: $$('[role=alert]').map(txt), body: composer ? null : document.body.innerText};
"#;

const SEND: &str = r#"
await showConversationView();
const composer = await until(() => document.getElementById('agent-message'));
if (!composer) throw new Error('no composer');
setInput(composer, A.text);
await new Promise(resolve => setTimeout(resolve, 50));
const send = await until(() => tid('direct-conversation-send') && !tid('direct-conversation-send').disabled ? tid('direct-conversation-send') : btn('发送') || btn('Send'));
if (!send) throw new Error('no send button');
send.click();
return {ok: !!(await until(() => document.getElementById('agent-message')?.value === ''))};
"#;

const CLOSE_RUN: &str = r#"
await showConversationView();
const close = await until(() => btn('结束会话') || btn('End session'));
if (!close) throw new Error('no end-session button');
close.click();
return {ok: true};
"#;

/// The id of the session that appeared after the UI started one (not in `before`).
async fn new_session_id(app: &AppHandle, before: &[String]) -> Option<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let found = sessions_of(app).await.into_iter().find(|row| row["status"] != "closed" && row["id"].as_str().is_some_and(|id| !before.iter().any(|old| old == id))).and_then(|row| row["id"].as_str().map(str::to_owned));
        if found.is_some() || tokio::time::Instant::now() >= deadline {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// Last event sequence number of a session, so later replies can be told apart.
async fn last_seq(app: &AppHandle, sid: &str) -> u64 {
    let view = ipc(app, "main", "get_agent_session", json!({"sessionId": sid})).await;
    value(&view)["events"].as_array().and_then(|e| e.last()).and_then(|e| e["seq"].as_u64()).unwrap_or(0)
}

/// Waits for events after `from_seq` to contain `needle`, read through `get_agent_session`
/// (not UI text). Returns the new event text, or an error with the last observed state.
async fn await_reply(app: &AppHandle, sid: &str, prompt: &str, needle: &str, from_seq: u64) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + TURN_TIMEOUT;
    loop {
        let view = ipc(app, "main", "get_agent_session", json!({"sessionId": sid})).await;
        if !succeeded(&view) {
            return Err(view.describe());
        }
        let status = value(&view)["status"].as_str().unwrap_or_default().to_owned();
        let text: String = value(&view)["events"]
            .as_array()
            .map(|events| events.iter().filter(|e| e["seq"].as_u64().unwrap_or(0) > from_seq).filter_map(|e| e["text"].as_str()).filter(|t| *t != prompt).collect::<Vec<_>>().concat())
            .unwrap_or_default();
        if status == "error" {
            return Err(view.describe());
        }
        if status == "idle" && text.contains(needle) {
            return Ok(text);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("no reply containing {needle}; status={status}; text={text}"));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[path = "agents_launch.rs"]
mod launch;
pub use launch::run_agents;
#[path = "agents_scenario.rs"]
mod acceptance;
use acceptance::scenario;
#[path = "agents_stage2.rs"]
mod stage2;
#[path = "agents_stage3.rs"]
mod stage3;
#[path = "agents_stage4.rs"]
mod stage4;
#[path = "agents_stage6.rs"]
mod stage6;
#[path = "agents_layout_round2.rs"]
mod layout_round2;
#[path = "agents_round3_terminal.rs"]
mod round3_terminal;
#[path = "agents_round3_workflows.rs"]
mod round3_workflows;
#[path = "agents_round4.rs"]
mod round4;
#[path = "agents_phase6_local.rs"]
mod phase6_local;
