//! Project-task acceptance and the default agent smoke (V7 phase 1;
//! docs/02-产品与版本/后续版本/V7-项目任务基础技术契约.md).
//!
//! Default (`DGOS_PROJECT_ACCEPTANCE_ROOT` unset): both engines, one project task,
//! handshake, snapshot and close through production IPC.
//!
//! Acceptance: `DGOS_PROJECT_ACCEPTANCE_ROOT=<dir>` plus
//! `DGOS_PROJECT_ACCEPTANCE_PHASE=first|restart|stage3-first|stage3-restart|stage4-ui`. `<dir>`
//! holds the database, fixture projects and evidence shared by both processes.
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
async fn load_agents(fx: &Fx) -> AgentManager {
    let manager = AgentManager::new(fx.root.join("agents.json"), DevelopmentRepository::new(fx.db.clone()));
    manager.load().await.expect("agent index");
    manager
}

/// Records one check on stdout and in the evidence list.
fn record(ck: &mut Checks, ev: &mut Vec<Value>, name: &str, ok: bool, detail: impl Display) {
    let detail = detail.to_string();
    ck.check(name, ok, &detail);
    ev.push(json!({"check": name, "ok": ok, "detail": detail}));
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
    match tokio::time::timeout(STEP_TIMEOUT, receiver).await {
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
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const txt = (el) => ((el && el.textContent) || '').trim();
const labelOf = (el) => { if (el.labels && el.labels[0]) return txt(el.labels[0]); const by = el.getAttribute('aria-labelledby'); if (by) return txt(document.getElementById(by)); return el.getAttribute('aria-label') || ''; };
const tid = (id) => document.querySelector(`[data-testid="${id}"]`);
const need = async (id, ms = 20000) => { const el = await until(() => tid(id), ms); if (!el) throw new Error('missing ' + id); return el; };
const setInput = (f, v) => { const p = f.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(p, 'value').set.call(f, v); f.dispatchEvent(new Event('input', { bubbles: true })); };
const fill = async (id, v) => { const el = await need(id); const f = el.matches('input,textarea') ? el : el.querySelector('input,textarea'); if (!f) throw new Error('no field in ' + id); setInput(f, v); return true; };
const click = async (id) => { (await need(id)).click(); return true; };
const btn = (t) => $$('button').find((b) => txt(b) === t && !b.disabled);
const items = (prefix) => $$(`[data-testid^="${prefix}"]`).map((e) => ({ id: e.getAttribute('data-testid').slice(prefix.length), text: txt(e) }));
const itemFor = (prefix, text) => items(prefix).find((i) => i.text.includes(text));
const isChecked = (c) => c.checked === true || c.getAttribute('aria-checked') === 'true';
const msgs = () => { const m = document.querySelector('.agent-messages'); return m ? m.textContent : ''; };
"##;

/// Evaluates one UI step: the shared helpers, then `A` (the arguments), then `body`.
async fn ui(app: &AppHandle, args: Value, body: &str) -> Value {
    eval_long(app, &format!("{DOM}\nconst A = {args};\n{body}")).await
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

/// Imports a project through the dev workbench form (directory path typed into the form).
async fn ui_import_project(app: &AppHandle, ck: &mut Checks, ev: &mut Vec<Value>, name: &str, path: &Path) -> bool {
    let reply = ui(app, json!({"name": name, "path": path.to_string_lossy()}), IMPORT_PROJECT).await;
    record(ck, ev, &format!("{name} imported via project form"), ui_ok(&reply), reply.to_string());
    ui_ok(&reply)
}

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
await click('dev-project-create-open');
await need('dev-project-form');
await fill('dev-project-name', A.name);
await fill('dev-project-path', A.path);
await click('dev-project-submit');
const row = await until(() => $$('[data-testid^="dev-project-item-"]').find((e) => txt(e).includes(A.name)));
return {ok: !!row};
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
return {ok: !!composer};
"#;

const SEND: &str = r#"
const composer = await until(() => document.getElementById('agent-message'));
if (!composer) throw new Error('no composer');
setInput(composer, A.text);
await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
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
