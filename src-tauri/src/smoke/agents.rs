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
const btn = (t) => $$('button', stage6UI ? ($$('.agent-workbench')[0] || document) : document).find((b) => txt(b) === t && !b.disabled);
const items = (prefix) => $$(`[data-testid^="${prefix}"]`).map((e) => ({ id: e.getAttribute('data-testid').slice(prefix.length), text: txt(e) }));
const itemFor = (prefix, text) => items(prefix).find((i) => i.text.includes(text));
const isChecked = (c) => c.checked === true || c.getAttribute('aria-checked') === 'true';
const msgs = () => { const m = document.querySelector('.agent-messages'); return m ? m.textContent : ''; };
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
async fn pick_project_directory(app: &AppHandle, path: &Path) -> bool {
    use objc2::{class, msg_send};
    use objc2::runtime::{AnyObject, Bool};
    let Some(window)=app.get_webview_window("main") else { return false; };
    let Ok(path)=std::fs::canonicalize(path) else { return false; };
    // Live NSOpenPanel navigation is hosted by macOS; directoryURL is configuration-only.
    // Target this acceptance process, then verify the selected URL before confirming.
    let navigate = r#"on run argv
tell application "System Events"
    tell (first application process whose unix id is (item 1 of argv as integer))
        set frontmost to true
        keystroke "g" using {command down, shift down}
        delay 0.5
        keystroke "a" using command down
        keystroke (item 2 of argv)
        delay 1
        key code 36
    end tell
end tell
end run"#;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let result=tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e",navigate,&std::process::id().to_string(),&path.to_string_lossy()]).output().await;
    match result {
        Ok(output) if output.status.success()=>{},
        other=>{println!("INFO native folder navigation failed: {other:?}");return false;}
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    if let Some(root)=std::env::var_os("DGOS_PROJECT_ACCEPTANCE_ROOT") {
        let _=super::capture_native(app,Path::new(&root),"native-folder-before-confirm").await;
    }
    let expected=path.to_string_lossy().into_owned();
    let (send,receive)=oneshot::channel();
    if window.with_webview(move |_| unsafe {
            let app:*mut AnyObject=msg_send![class!(NSApplication),sharedApplication];
            let windows:*mut AnyObject=msg_send![app,windows];
            let count:usize=msg_send![windows,count];
            let mut panel:*mut AnyObject=std::ptr::null_mut();
            for index in 0..count {
                let candidate:*mut AnyObject=msg_send![windows,objectAtIndex:index];
                let is_panel:Bool=msg_send![candidate,isKindOfClass:class!(NSOpenPanel)];
                let visible:Bool=msg_send![candidate,isVisible];
                if is_panel.as_bool()&&visible.as_bool(){panel=candidate;break;}
            }
            if panel.is_null(){println!("INFO no visible NSOpenPanel among {count} owned windows");let _=send.send(false);return;}
                let chosen:*mut AnyObject=msg_send![panel,URL];
                if chosen.is_null(){let _=send.send(false);return;}
                let chosen_path:*mut AnyObject=msg_send![chosen,path];
                let bytes:*const std::ffi::c_char=msg_send![chosen_path,UTF8String];
                let actual=if bytes.is_null(){String::new()}else{std::ffi::CStr::from_ptr(bytes).to_string_lossy().into_owned()};
                if actual.trim_end_matches('/')!=expected.trim_end_matches('/'){println!("INFO native selected URL remains {actual:?}; refusing wrong-directory import");let _:()=msg_send![panel,cancel:std::ptr::null_mut::<AnyObject>()];let _=send.send(false);return;}
            let _=send.send(true);
        }).is_err(){return false;}
        if !receive.await.unwrap_or(false){return false;}
    let confirm=r#"on run argv
tell application "System Events"
    tell (first application process whose unix id is (item 1 of argv as integer))
        set frontmost to true
        key code 36
    end tell
end tell
end run"#;
    tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e",confirm,&std::process::id().to_string()]).status().await.is_ok_and(|s|s.success())
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

/// Starts a new session for `engine` under the selected task: trust, then the New session control.
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
const panel=await need('dev-project-list');const add=panel.querySelector('button[aria-haspopup="menu"]');if(!add)throw Error('current Add project menu absent');
add.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,pointerType:'mouse',ctrlKey:false}));
add.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,button:0,pointerType:'mouse'}));
const item=await until(()=>$$('[role=menuitem]').find(e=>/打开文件夹|Open folder/.test(txt(e))));if(!item)throw Error('Open folder menu item absent');item.click();
return {ok:true};
"#;

const NEW_TASK: &str = r#"
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
const row = await until(() => $$('[data-testid^="dev-task-item-"]').find((e) => txt(e).includes(A.goal)));
if (!row) throw new Error('no task row ' + A.goal);
row.click();
return {ok: true};
"#;

const START_SESSION: &str = r#"
await need('dev-trust');
const trust = tid('dev-trust');
const box = trust.matches('input,[role=checkbox]') ? trust : trust.querySelector('input,[role=checkbox]') || trust;
if (!(box.checked === true || box.getAttribute('aria-checked') === 'true')) box.click();
const start = await until(() => { const b=tid('dev-session-new-'+A.engine);return b&&!b.disabled&&b; });
if (!start) throw Error('new session disabled');
start.click();
const composer = await until(() => document.getElementById('agent-message'));
return {ok: !!composer, alerts: $$('[role=alert]').map(txt), body: composer ? null : document.body.innerText};
"#;

const SEND: &str = r#"
const composer = await until(() => document.getElementById('agent-message'));
if (!composer) throw new Error('no composer');
setInput(composer, A.text);
await new Promise(resolve => setTimeout(resolve, 50));
const send = await until(() => btn('发送'));
if (!send) throw new Error('no send button');
send.click();
return {ok: !!(await until(() => document.getElementById('agent-message')?.value === ''))};
"#;

const CLOSE_RUN: &str = r#"
const close = await until(() => btn('关闭运行'));
if (!close) throw new Error('no close-run button');
close.click();
return {ok: true};
"#;

/// The id of the session that appeared after the UI started one (not in `before`).
async fn new_session_id(app: &AppHandle, before: &[String]) -> Option<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let found = ids(&sessions_of(app).await).into_iter().find(|id| !before.contains(id));
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
