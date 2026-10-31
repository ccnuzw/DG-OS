//! Real-WebView smoke for the AI workbench chat (V1-FR-006, V1-E2E-06 at fixture level).
//!
//! The page is the shipped `dist/` on `/workbench`; every chat command is the production handler over a
//! temp SQLite database (`ChatManager`, `ProviderManager`, `McpManager` are the real types). The UI is
//! driven by DOM events from the evaluated script (click, input, Radix Select via keyboard events), which
//! is not the same as a user's pointer; two checks (clipboard) use real `NSEvent` key presses and read the
//! real `NSPasteboard`. The IPC `Channel` is the real one the chat store creates; a spy installed on
//! `transformCallback` only records what Tauri delivered.
//!
//! Replaced for the run (fixture level, NOT a real Provider): the Provider is a loopback server that plays
//! scripted OpenAI-style SSE/JSON replies, the credential vault is in memory, and the MCP server is the
//! `dgos-mcp` stdio fixture run with `node`. Temp directory only; nothing touches the real user's data.
//!
//! Not proven: a real vendor, VoiceOver, other platforms' WebViews, real pointer hit-testing,
//! `WebviewWindow::close()` (the window is `destroy()`ed, which emits the same `Destroyed` event).

use super::modules::build_world_with;
use super::{eval, finish, goto, wait_load, Checks, LoadEvents, Pending};
use crate::commands;
use crate::host::LOCAL_USER;
use crate::module_host::{self, StartupLoad};
use async_trait::async_trait;
use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_mcp::{
    CreateMcpServerRequest, CredentialScope, McpCredentialVault, McpError, McpManager, SecretToken,
    StdioConfig, TransportKind,
};
use dgos_providers::test_support::{seed_provider, MemoryCredentialVault};
use dgos_providers::ProviderManager;
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::sleep;

const MAIN: &str = "main";
const CHAT_WATCHDOG: Duration = Duration::from_secs(900);
const PROVIDER_NAME: &str = "Smoke Loopback";
const MARKER: &str = "SENTINEL-TOOL-ARGUMENT-4e2b";
const MARKER_REJECT: &str = "SENTINEL-REJECTED-ARGUMENT-77c1";
const MARKER_EXPIRE: &str = "SENTINEL-EXPIRED-ARGUMENT-90d3";
const FINAL_ANSWER: &str = "final answer after the tool";

// ---------------------------------------------------------------------------------------------
// Scripted loopback upstream

enum Step {
    Send(String),
    /// Wait until the harness calls `release(name)`.
    Gate(String),
    /// Stay silent until the client closes the connection.
    Hold,
}

enum Script {
    Sse(Vec<Step>),
    Json(Value),
}

#[derive(Default)]
struct Upstream {
    scripts: Mutex<VecDeque<Script>>,
    bodies: Mutex<Vec<Value>>,
    /// Request paths other than `/chat/completions`: anything the page or host fetched from this origin.
    stray: Mutex<Vec<String>>,
    released: Mutex<HashSet<String>>,
    /// Connections whose `Hold` ended because the client went away.
    held_closed: AtomicUsize,
}

impl Upstream {
    fn script(&self, script: Script) {
        self.scripts.lock().unwrap().push_back(script);
    }
    fn release(&self, name: &str) {
        self.released.lock().unwrap().insert(name.to_owned());
    }
    fn bodies(&self) -> usize {
        self.bodies.lock().unwrap().len()
    }
    fn body(&self, index: usize) -> Value {
        self.bodies.lock().unwrap()[index].clone()
    }
    fn strays(&self) -> Vec<String> {
        self.stray.lock().unwrap().clone()
    }
}

fn data(value: Value) -> Step {
    Step::Send(format!("data: {value}\n\n"))
}
fn delta(text: &str) -> Step {
    data(json!({"choices":[{"delta":{"content":text},"finish_reason":null}]}))
}
fn stop_steps() -> Vec<Step> {
    vec![
        data(json!({"choices":[{"delta":{},"finish_reason":"stop"}]})),
        data(
            json!({"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}),
        ),
        Step::Send("data: [DONE]\n\n".into()),
    ]
}
fn reply(text: &str) -> Script {
    let mut steps = vec![delta(text)];
    steps.extend(stop_steps());
    Script::Sse(steps)
}
fn proposal(marker: &str) -> Script {
    Script::Json(
        json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{
        "id":"call_1","type":"function",
        "function":{"name":"dgos_tool_0","arguments":json!({"text":marker}).to_string()}
    }]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}),
    )
}
fn answer_json(text: &str) -> Script {
    Script::Json(
        json!({"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":8,"completion_tokens":3,"total_tokens":11}}),
    )
}

async fn serve(listener: TcpListener, upstream: Arc<Upstream>) {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let upstream = upstream.clone();
        tauri::async_runtime::spawn(async move {
            handle(&mut socket, &upstream).await;
        });
    }
}

async fn handle(socket: &mut TcpStream, up: &Upstream) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = socket.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("")
        .to_owned();
    let length = head
        .to_ascii_lowercase()
        .lines()
        .find_map(|line| {
            line.strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let n = socket.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    if !path.ends_with("/chat/completions") {
        up.stray.lock().unwrap().push(path);
        let _ = socket
            .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await;
        return;
    }
    let body =
        serde_json::from_slice(&buffer[header_end..header_end + length]).unwrap_or(Value::Null);
    up.bodies.lock().unwrap().push(body);
    let script = up
        .scripts
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or_else(|| reply("default reply"));
    match script {
        Script::Json(value) => {
            let text = value.to_string();
            let out = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
            let _ = socket.write_all(out.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
        Script::Sse(steps) => {
            let head =
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            for step in steps {
                match step {
                    Step::Send(text) => {
                        if socket.write_all(text.as_bytes()).await.is_err() {
                            return;
                        }
                        let _ = socket.flush().await;
                    }
                    Step::Gate(name) => {
                        for _ in 0..1500 {
                            if up.released.lock().unwrap().contains(&name) {
                                break;
                            }
                            sleep(Duration::from_millis(20)).await;
                        }
                    }
                    Step::Hold => {
                        loop {
                            let read = socket.read(&mut chunk);
                            match tokio::time::timeout(Duration::from_secs(120), read).await {
                                Ok(Ok(0) | Err(_)) => {
                                    up.held_closed.fetch_add(1, Ordering::SeqCst);
                                    break;
                                }
                                Ok(Ok(_)) => {}
                                Err(_) => break,
                            }
                        }
                        return;
                    }
                }
            }
            let _ = socket.shutdown().await;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Harness state and helpers

struct Fixture {
    up: Arc<Upstream>,
    db: DatabaseConnection,
    root: std::path::PathBuf,
    mcp_note: Option<String>,
}

struct RefusingMcpVault;

#[async_trait]
impl McpCredentialVault for RefusingMcpVault {
    async fn store(&self, _: &CredentialScope, _: SecretToken) -> dgos_mcp::Result<()> {
        Err(McpError::CredentialUnavailable)
    }
    async fn load(&self, _: &CredentialScope) -> dgos_mcp::Result<SecretToken> {
        Err(McpError::CredentialUnavailable)
    }
    async fn remove(&self, _: &CredentialScope) -> dgos_mcp::Result<()> {
        Err(McpError::CredentialUnavailable)
    }
}

/// Page-side helpers prepended to every evaluated body. `spy()` records the chat Channel messages the
/// webview delivers and the command names the page invokes; it does not alter either.
const PRE: &str = r##"
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 8000) => { const t = Date.now(); while (Date.now() - t < ms) { try { const v = fn(); if (v) return v; } catch (e) {} await wait(40); } return false; };
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const txt = (el) => ((el && el.textContent) || '').trim();
const btn = (t, root = document) => $$('button', root).find((b) => txt(b) === t);
const btnHas = (t, root = document) => $$('button', root).find((b) => txt(b).includes(t));
const kd = (el, key) => { el.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })); };
const setVal = (el, v) => { const p = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(p, 'value').set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); };
const labelOf = (el) => { if (el.labels && el.labels[0]) return txt(el.labels[0]); const by = el.getAttribute('aria-labelledby'); if (by) return txt(document.getElementById(by)); return el.getAttribute('aria-label') || ''; };
const combo = (label) => $$('[role=combobox]').find((e) => labelOf(e) === label);
const STREAM = 'article[aria-label="助手消息（生成中）"]';
const DONE_A = 'article[aria-label="助手消息"]';
const USER_A = 'article[aria-label="用户消息"]';
const PROPOSAL = 'section[aria-labelledby="chat-proposal-heading"]';
const KNOWN = ['started', 'delta', 'reasoning_delta', 'tool_proposal', 'done', 'error'];
const spy = () => {
  if (window.__spy) return;
  window.__spy = { events: [], calls: [] };
  // __TAURI_INTERNALS__.invoke/transformCallback are non-writable; the callback registry Map and fetch are not.
  const cbs = window.__TAURI_INTERNALS__.callbacks;
  const set = cbs.set.bind(cbs);
  cbs.set = (id, fn) => set(id, (raw) => {
    try { if (raw && raw.message && KNOWN.includes(raw.message.kind)) window.__spy.events.push({ i: raw.index, m: raw.message }); } catch (e) {}
    return fn(raw);
  });
  const realFetch = window.fetch.bind(window);
  window.fetch = (input, init) => {
    try { const u = String(input && input.url ? input.url : input); const m = /^(?:ipc:\/\/localhost|https?:\/\/ipc\.localhost)\/([^/?#]+)/.exec(u); if (m && m[1] !== 'smoke_report') window.__spy.calls.push(decodeURIComponent(m[1])); } catch (e) {}
    return realFetch(input, init);
  };
};
spy();
const kinds = () => window.__spy.events.map((e) => e.m.kind);
const choose = async (label, opt) => {
  const c = await until(() => { const x = combo(label); return x && !x.disabled && x; });
  if (!c) throw new Error('combobox ' + label + ' missing');
  kd(c, 'ArrowDown');
  const o = await until(() => $$('[role=option]').find((x) => txt(x) === opt));
  if (!o) throw new Error('option ' + opt + ' missing: ' + $$('[role=option]').map(txt).join('|'));
  kd(o, 'Enter');
  await until(() => !$('[role=listbox]'));
  await wait(100);
};
const newSession = async (model) => {
  const nav = $('nav[aria-label="会话"]');
  const list = () => $$('section[aria-label="会话列表"] li').length;
  const before = list();
  btn('新建会话', nav).click();
  const ready = await until(() => list() > before && $('section[aria-label="会话目标"]') && txt(combo('Provider')).includes('选择 Provider'));
  if (!ready) throw new Error('new session did not appear');
  await choose('Provider', 'Smoke Loopback');
  await choose('模型', model);
  await choose('技能', '明确不使用技能');
  const ap = await until(() => { const b = btn('应用目标'); return b && !b.disabled && b; });
  if (!ap) throw new Error('apply button never enabled');
  ap.click();
  if (!(await until(() => $$('[role=status]').some((s) => txt(s).includes('目标已更新'))))) throw new Error('target was not applied');
  return true;
};
const send = async (text) => {
  const ta = await until(() => { const t = $('textarea[aria-label="消息输入"]'); return t && !t.disabled && t; });
  if (!ta) throw new Error('composer disabled');
  setVal(ta, text);
  const b = await until(() => { const x = btn('发送'); return x && !x.disabled && x; });
  if (!b) throw new Error('send disabled');
  b.click();
};
const lastDone = () => $$(DONE_A).pop();
const mdText = (a) => txt($('.leading-relaxed', a));
const idle = () => !$(STREAM) && !!btn('发送');
"##;

async fn js(app: &AppHandle, body: &str) -> Value {
    let full = format!("{PRE}\n{body}");
    let reply = eval(app, MAIN, &full).await;
    if reply["ok"] == true {
        reply["value"].clone()
    } else {
        json!({"__error": reply["error"]})
    }
}

fn failed(value: &Value) -> bool {
    value.get("__error").is_some()
}

async fn rows(db: &DatabaseConnection, sql: &str) -> Vec<Value> {
    let Ok(found) = db
        .query_all(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
    else {
        return vec![json!({"__error": "query failed"})];
    };
    found
        .iter()
        .filter_map(|row| row.try_get::<String>("", "j").ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

async fn wait_rows(
    db: &DatabaseConnection,
    sql: &str,
    ready: impl Fn(&[Value]) -> bool,
    seconds: u64,
) -> Vec<Value> {
    let mut last = Vec::new();
    for _ in 0..(seconds * 20) {
        last = rows(db, sql).await;
        if ready(&last) {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    last
}

/// The assistant reply that follows the user message `user_text`, with its task row.
fn turn_sql(user_text: &str) -> String {
    format!(
        "SELECT json_object('content',a.content,'status',a.status,'err',a.error_code,'finish',a.finish_reason,\
         'task_status',t.status,'task_err',t.error_code,'tokens',t.total_tokens,'session',a.session_id,'seq',a.seq,\
         'id',a.id) AS j \
         FROM chat_messages u JOIN chat_messages a ON a.session_id=u.session_id AND a.seq=u.seq+1 AND a.role='assistant' \
         LEFT JOIN workbench_tasks t ON t.id=COALESCE(a.task_id,u.task_id) \
         WHERE u.role='user' AND u.content='{user_text}'"
    )
}

fn terminal(rows: &[Value]) -> bool {
    rows.first()
        .is_some_and(|row| row["status"] != "generating" && row["task_status"] != "running")
}

fn wanted(n: u32) -> bool {
    match std::env::var("DGOS_CHAT_SMOKE_ONLY") {
        Ok(list) if !list.trim().is_empty() => list
            .split(',')
            .any(|item| item.trim().parse::<u32>().ok() == Some(n)),
        _ => true,
    }
}

// ---------------------------------------------------------------------------------------------
// macOS: real key presses and the real pasteboard

#[cfg(target_os = "macos")]
mod mac {
    use super::MAIN;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    use std::ffi::{c_char, c_void, CStr, CString};
    use tauri::{AppHandle, Manager};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    unsafe impl objc2::encode::Encode for Point {
        const ENCODING: objc2::encode::Encoding = objc2::encode::Encoding::Struct(
            "CGPoint",
            &[
                objc2::encode::Encoding::Double,
                objc2::encode::Encoding::Double,
            ],
        );
    }

    const TEXT: &str = "public.utf8-plain-text";

    unsafe fn ns(text: &str) -> *mut AnyObject {
        let c = CString::new(text).expect("no NUL");
        msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()]
    }

    unsafe fn rust(string: *mut AnyObject) -> Option<String> {
        if string.is_null() {
            return None;
        }
        let c: *const c_char = msg_send![string, UTF8String];
        Some(CStr::from_ptr(c).to_string_lossy().into_owned())
    }

    /// Runs `body` on the main thread (where AppKit objects may be touched) and returns its result.
    async fn on_main<T: Send + 'static>(
        app: &AppHandle,
        body: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let window = app.get_webview_window(MAIN)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        window
            .with_webview(move |_| {
                let _ = tx.send(body());
            })
            .ok()?;
        rx.await.ok()
    }

    pub async fn activate(app: &AppHandle) -> bool {
        let Some(window) = app.get_webview_window(MAIN) else {
            return false;
        };
        let _ = window.set_focus();
        window
            .with_webview(|webview| unsafe {
                let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![ns_app, activateIgnoringOtherApps: Bool::YES];
                let win = webview.ns_window() as *mut AnyObject;
                let _: () = msg_send![win, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
                let view = webview.inner() as *mut AnyObject;
                let _: Bool = msg_send![win, makeFirstResponder: view];
            })
            .is_ok()
    }

    /// One real Return key press (keyDown + keyUp through `-[NSWindow sendEvent:]`).
    pub async fn press_enter(app: &AppHandle) -> bool {
        let mut sent = true;
        for down in [true, false] {
            let Some(window) = app.get_webview_window(MAIN) else {
                return false;
            };
            sent &= window
                .with_webview(move |webview| unsafe {
                    let win = webview.ns_window() as *mut AnyObject;
                    let text = ns("\r");
                    let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
                    let timestamp: f64 = msg_send![info, systemUptime];
                    let number: isize = msg_send![win, windowNumber];
                    let kind: usize = if down { 10 } else { 11 };
                    let event: *mut AnyObject = msg_send![
                        class!(NSEvent),
                        keyEventWithType: kind,
                        location: Point { x: 0.0, y: 0.0 },
                        modifierFlags: 0usize,
                        timestamp: timestamp,
                        windowNumber: number,
                        context: std::ptr::null_mut::<AnyObject>(),
                        characters: text,
                        charactersIgnoringModifiers: text,
                        isARepeat: Bool::NO,
                        keyCode: 36u16
                    ];
                    let _: () = msg_send![win, sendEvent: event];
                })
                .is_ok();
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        sent
    }

    pub async fn get_text(app: &AppHandle) -> Option<Option<String>> {
        on_main(app, || unsafe {
            let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            let value: *mut AnyObject = msg_send![pb, stringForType: ns(TEXT)];
            rust(value)
        })
        .await
    }

    pub async fn set_text(app: &AppHandle, text: String) -> bool {
        on_main(app, move || unsafe {
            let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            let _: isize = msg_send![pb, clearContents];
            let ok: Bool = msg_send![pb, setString: ns(&text), forType: ns(TEXT)];
            ok.as_bool()
        })
        .await
        .unwrap_or(false)
    }

    /// Every item of the general pasteboard as `(type, bytes)` lists.
    pub type Snapshot = Vec<Vec<(String, Vec<u8>)>>;

    pub async fn snapshot(app: &AppHandle) -> Option<Snapshot> {
        on_main(app, || unsafe {
            let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            let items: *mut AnyObject = msg_send![pb, pasteboardItems];
            let mut out = Vec::new();
            if items.is_null() {
                return out;
            }
            let count: usize = msg_send![items, count];
            for i in 0..count {
                let item: *mut AnyObject = msg_send![items, objectAtIndex: i];
                let types: *mut AnyObject = msg_send![item, types];
                let n: usize = msg_send![types, count];
                let mut kinds = Vec::new();
                for j in 0..n {
                    let kind: *mut AnyObject = msg_send![types, objectAtIndex: j];
                    let data: *mut AnyObject = msg_send![item, dataForType: kind];
                    if data.is_null() {
                        continue;
                    }
                    let len: usize = msg_send![data, length];
                    let ptr: *const c_void = msg_send![data, bytes];
                    let bytes = if len == 0 || ptr.is_null() {
                        Vec::new()
                    } else {
                        std::slice::from_raw_parts(ptr as *const u8, len).to_vec()
                    };
                    if let Some(name) = rust(kind) {
                        kinds.push((name, bytes));
                    }
                }
                out.push(kinds);
            }
            out
        })
        .await
    }

    pub async fn restore(app: &AppHandle, snapshot: Snapshot) -> bool {
        on_main(app, move || unsafe {
            let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            let _: isize = msg_send![pb, clearContents];
            if snapshot.is_empty() {
                return true;
            }
            let objects: *mut AnyObject = msg_send![class!(NSMutableArray), array];
            for kinds in &snapshot {
                let item: Retained<AnyObject> = msg_send![class!(NSPasteboardItem), new];
                for (name, bytes) in kinds {
                    let data: *mut AnyObject = msg_send![
                        class!(NSData),
                        dataWithBytes: bytes.as_ptr() as *const c_void,
                        length: bytes.len()
                    ];
                    let _: Bool = msg_send![&*item, setData: data, forType: ns(name)];
                }
                let _: () = msg_send![objects, addObject: &*item];
            }
            let ok: Bool = msg_send![pb, writeObjects: objects];
            ok.as_bool()
        })
        .await
        .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------------------------
// 1-2: create, select, send, order of Channel events, partial render, DOM == snapshot

async fn block_stream(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let up = &fx.up;
    let setup = js(
        app,
        "await newSession('chat-model'); window.__spy.events = []; return { calls: window.__spy.calls.filter((c) => c === 'create_chat_session' || c === 'update_chat_session_target').join(',') };",
    )
    .await;
    checks.check(
        "[1] create a session through the UI and apply Provider, model and 'no skill'",
        !failed(&setup)
            && setup["calls"].as_str().is_some_and(|c| {
                c.contains("create_chat_session") && c.contains("update_chat_session_target")
            }),
        setup.to_string(),
    );
    let text = "T1 streaming order";
    up.script(Script::Sse({
        let mut steps = vec![
            delta("Hello "),
            delta("partial "),
            Step::Gate("g1".into()),
            delta("tail."),
        ];
        steps.extend(stop_steps());
        steps
    }));
    let before = up.bodies();
    let partial = js(
        app,
        &format!(
            "await send({text:?}); const b = await until(() => {{ const a = $(STREAM); return a && txt(a).includes('Hello partial') && a; }}, 12000); \
             const doneArticles = $$(DONE_A).filter((a) => txt(a).includes('tail')).length; \
             return {{ shown: !!b, busy: b ? b.getAttribute('aria-busy') : null, tail: b ? txt(b).includes('tail') : null, doneArticles, \
                      stopButton: !!btn('停止生成'), kinds: kinds() }};"
        ),
    )
    .await;
    checks.check(
        "[2] streaming render: the DOM shows the partial text in the busy bubble before 'done' (upstream is gated)",
        partial["shown"] == true
            && partial["busy"] == "true"
            && partial["tail"] == false
            && partial["doneArticles"] == 0
            && partial["stopButton"] == true
            && partial["kinds"].as_array().is_some_and(|k| k.iter().any(|x| x == "delta") && !k.iter().any(|x| x == "done"))
            && up.bodies() == before + 1,
        partial.to_string(),
    );
    up.release("g1");
    let end = js(
        app,
        "const ok = await until(() => !$(STREAM) && $$(DONE_A).length === 1 && idle(), 12000); const a = lastDone(); \
         const ev = window.__spy.events; const doneEv = ev.filter((e) => e.m.kind === 'done'); \
         return { ok, dom: a ? mdText(a) : null, kinds: kinds(), idx: ev.map((e) => e.i), doneContent: doneEv[0] && doneEv[0].m.message.content, \
                  deltas: ev.filter((e) => e.m.kind === 'delta').map((e) => e.m.text).join(''), startedTask: ev[0] && ev[0].m.task_id };",
    )
    .await;
    let kinds: Vec<String> = end["kinds"]
        .as_array()
        .map(|k| {
            k.iter()
                .map(|x| x.as_str().unwrap_or("?").to_owned())
                .collect()
        })
        .unwrap_or_default();
    let idx: Vec<u64> = end["idx"]
        .as_array()
        .map(|k| k.iter().map(|x| x.as_u64().unwrap_or(u64::MAX)).collect())
        .unwrap_or_default();
    let ordered = kinds.first().map(String::as_str) == Some("started")
        && kinds.last().map(String::as_str) == Some("done")
        && kinds.iter().filter(|k| *k == "done").count() == 1
        && kinds.iter().filter(|k| *k == "started").count() == 1
        && kinds[1..kinds.len().saturating_sub(1)]
            .iter()
            .all(|k| k == "delta")
        && kinds.iter().filter(|k| *k == "delta").count() >= 2
        && idx.iter().enumerate().all(|(i, v)| *v == i as u64);
    checks.check(
        "[1] the page's Channel delivered started, delta..., done in order with contiguous indexes",
        ordered,
        format!("{kinds:?} idx={idx:?}"),
    );
    let db = wait_rows(&fx.db, &turn_sql(text), terminal, 10).await;
    let row = db.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "[1] final DOM text equals the persisted assistant snapshot and the 'done' event snapshot",
        end["ok"] != false
            && end["dom"] == "Hello partial tail."
            && row["content"] == end["dom"]
            && end["doneContent"] == end["dom"]
            && end["deltas"] == end["dom"]
            && row["status"] == "completed"
            && row["task_status"] == "succeeded"
            && row["tokens"] == 18,
        format!("{end} db={row}"),
    );
}

// ---------------------------------------------------------------------------------------------
// 3 (+7): Markdown/code safety, then the copy button

fn markdown_reply(port: u16) -> Vec<String> {
    vec![
        "Here is code:\n\n```rust\nfn main() { let answer = 42; }\n```\n\n".into(),
        format!("<img src=\"http://127.0.0.1:{port}/x\" onerror=\"window.__pwn=(window.__pwn||0)+1\">\n\n"),
        "<script>window.__pwn=(window.__pwn||0)+1</script>\n\n".into(),
        "[click me](javascript:window.__pwn=(window.__pwn||0)+1)\n\n[safe link](https://example.com/docs)\n\n".into(),
        format!("![alt text](http://127.0.0.1:{port}/y.png)\n\ninline <img src=http://127.0.0.1:{port}/z onerror=window.__pwn=1> end"),
    ]
}

async fn block_markdown(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let up = &fx.up;
    let port = fx_port(app);
    let pieces = markdown_reply(port);
    let full: String = pieces.concat();
    let mut steps: Vec<Step> = pieces.iter().map(|p| delta(p)).collect();
    steps.extend(stop_steps());
    up.script(Script::Sse(steps));
    let setup = js(
        app,
        &format!(
            "await newSession('chat-model'); window.__spy.events = []; window.__pwn = 0; await send('T3 markdown safety'); \
             const ok = await until(() => !$(STREAM) && $$(DONE_A).length === 1 && idle(), 15000); return {{ ok }};"
        ),
    )
    .await;
    sleep(Duration::from_millis(1500)).await;
    let strays_after_render = up.strays();
    let page = js(
        app,
        r##"
        const a = lastDone();
        const md = $('.leading-relaxed', a);
        const text = txt(md);
        const own = (needle) => $$('span', a).find((s) => txt(s).startsWith(needle) && !s.querySelector('span'));
        const click = own('click me');
        const out = {
          pwn: window.__pwn || 0,
          imgs: $$('img', a).length, scripts: $$('script', a).length, frames: $$('iframe,object,embed,svg', a).length,
          jsLinks: $$('[href^="javascript"], [src^="javascript"]', a).length,
          clickMeIsText: !!click && !click.closest('a,button'),
          rawHtmlAsText: text.includes('<script>') && text.includes('<img'),
          highlighted: $$('code [class*="hljs-"]', a).length,
          codeText: txt($('pre code', a)),
          safeButton: !!btn('safe link', a),
        };
        if (click) { click.click(); await wait(400); }
        out.pwnAfterClick = window.__pwn || 0;
        const safe = btn('safe link', a);
        if (safe) {
          safe.click();
          const dialog = await until(() => $('[role=dialog]'));
          out.dialogShowsUrl = !!dialog && txt(dialog).includes('https://example.com/docs');
          out.locationBefore = location.href;
          if (dialog) { const cancel = btn('取消', dialog); if (cancel) cancel.click(); }
          await until(() => !$('[role=dialog]'));
          out.dialogClosed = !$('[role=dialog]');
          out.locationSame = location.href === out.locationBefore;
        }
        return out;
        "##,
    )
    .await;
    checks.check(
        "[3] Markdown/code: reply rendered and idle",
        setup["ok"] != false && !failed(&page),
        format!("{setup} {page}"),
    );
    checks.check(
        "[3] no script executed: sentinel window.__pwn stayed 0 (raw <img onerror>, <script>, javascript: link, also after clicking the link text)",
        page["pwn"] == 0 && page["pwnAfterClick"] == 0,
        page.to_string(),
    );
    checks.check(
        "[3] no request reached the image URLs on the stub (/x, /y.png, /z): zero stray hits",
        strays_after_render.is_empty() && up.strays().is_empty(),
        format!("{:?}", up.strays()),
    );
    checks.check(
        "[3] raw HTML/images are inert text: no img/script/iframe elements, markup shown as characters",
        page["imgs"] == 0 && page["scripts"] == 0 && page["frames"] == 0 && page["rawHtmlAsText"] == true,
        page.to_string(),
    );
    checks.check(
        "[3] javascript: link is not clickable (no href, plain text, no anchor/button) and https link opens a confirmation first",
        page["jsLinks"] == 0
            && page["clickMeIsText"] == true
            && page["safeButton"] == true
            && page["dialogShowsUrl"] == true
            && page["dialogClosed"] == true
            && page["locationSame"] == true,
        page.to_string(),
    );
    checks.check(
        "[3] fenced code block is highlighted (hljs-* spans) and keeps its exact text",
        page["highlighted"].as_u64().unwrap_or(0) > 0
            && page["codeText"] == "fn main() { let answer = 42; }",
        page.to_string(),
    );
    // Negative controls: the detectors must be able to fire.
    let control = js(
        app,
        &format!(
            "window.__ctl = 0; const i = new Image(); i.setAttribute('onerror', 'window.__ctl = 1'); i.src = 'http://127.0.0.1:{port}/ctl'; \
             await until(() => window.__ctl === 1, 5000); return {{ ctl: window.__ctl }};"
        ),
    )
    .await;
    sleep(Duration::from_millis(300)).await;
    checks.check(
        "[3] negative control: an injected <img onerror> DOES run in this WebView and DOES hit the stub (/ctl), so the zero counts above are meaningful",
        control["ctl"] == 1 && up.strays().iter().any(|p| p == "/ctl"),
        format!("{control} {:?}", up.strays()),
    );
    let db = wait_rows(&fx.db, &turn_sql("T3 markdown safety"), terminal, 10).await;
    let stored = db
        .first()
        .map(|r| r["content"].clone())
        .unwrap_or(Value::Null);
    checks.check(
        "[3] the stored reply is the raw Markdown the model sent (nothing sanitised away in storage)",
        stored == json!(full),
        format!("{stored}"),
    );
    if wanted(7) {
        block_copy(app, checks, &full).await;
    }
}

fn fx_port(app: &AppHandle) -> u16 {
    app.state::<Port>().0
}

struct Port(u16);

// ---------------------------------------------------------------------------------------------
// 7: copy button on the real pasteboard

#[cfg(target_os = "macos")]
async fn block_copy(app: &AppHandle, checks: &mut Checks, expected: &str) {
    let saved = mac::snapshot(app).await;
    mac::activate(app).await;
    let mut focused = false;
    for _ in 0..12 {
        mac::activate(app).await;
        sleep(Duration::from_millis(250)).await;
        if js(app, "return document.hasFocus();").await == true {
            focused = true;
            break;
        }
    }
    let prep = r##"
        const a = lastDone(); const foot = $('footer', a); const b = btn('复制', foot);
        window.__cb = { calls: 0, ok: 0, err: 0 };
        const real = navigator.clipboard.writeText.bind(navigator.clipboard);
        navigator.clipboard.writeText = (t) => { window.__cb.calls++; return real(t).then(() => { window.__cb.ok++; }, (e) => { window.__cb.err++; window.__cb.name = String(e && e.name); throw e; }); };
        b.focus();
        return { focused: document.activeElement === b };
    "##;
    let read_status = r##"
        const foot = $('footer', lastDone());
        return { status: txt($('[role=status]', foot)), cb: window.__cb };
    "##;
    // Path 1: the real navigator.clipboard.writeText.
    let ready = js(app, prep).await;
    let sentinel = "SENTINEL-PASTEBOARD-BEFORE-1";
    let set = mac::set_text(app, sentinel.into()).await;
    let pressed = mac::press_enter(app).await;
    let mut status = Value::Null;
    for _ in 0..40 {
        status = js(app, read_status).await;
        if status["status"].as_str().is_some_and(|s| !s.is_empty()) {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    let after = mac::get_text(app).await.flatten();
    let success = status["status"] == "已复制";
    checks.check(
        "[7] copy button, real navigator.clipboard.writeText path: pasteboard (NSPasteboard) holds the message text and the UI says 已复制",
        focused && ready["focused"] == true && set && pressed && success && status["cb"]["ok"] == 1 && after.as_deref() == Some(expected),
        format!("focused={focused} ready={ready} status={status} pasteboard_matches={}", after.as_deref() == Some(expected)),
    );
    // Path 2: writeText rejects, so the execCommand fallback (or a visible failure) must follow.
    sleep(Duration::from_millis(2800)).await;
    let ready2 = js(
        app,
        r##"
        navigator.clipboard.writeText = () => Promise.reject(new Error('stubbed rejection'));
        const foot = $('footer', lastDone()); const b = btn('复制', foot);
        await until(() => txt($('[role=status]', foot)) === '', 4000);
        b.focus();
        return { focused: document.activeElement === b };
        "##,
    )
    .await;
    let sentinel2 = "SENTINEL-PASTEBOARD-BEFORE-2";
    mac::set_text(app, sentinel2.into()).await;
    mac::activate(app).await;
    let pressed2 = mac::press_enter(app).await;
    let mut status2 = Value::Null;
    for _ in 0..40 {
        status2 = js(app, r##"const foot = $('footer', lastDone()); return { status: txt($('[role=status]', foot)) };"##).await;
        if status2["status"].as_str().is_some_and(|s| !s.is_empty()) {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    let after2 = mac::get_text(app).await.flatten();
    let copied = status2["status"] == "已复制";
    let notice = status2["status"]
        .as_str()
        .is_some_and(|s| s.contains("复制失败"));
    let outcome = if copied {
        "execCommand fallback copied"
    } else if notice {
        "visible failure notice"
    } else {
        "NO FEEDBACK"
    };
    println!(
        "INFO copy fallback outcome: {outcome} (pasteboard_matches={})",
        after2.as_deref() == Some(expected)
    );
    checks.check(
        "[7] copy fallback with writeText rejecting: either execCommand copied the text (pasteboard matches, 已复制) or the failure notice is visible and the pasteboard is untouched; never a false success",
        ready2["focused"] == true
            && pressed2
            && ((copied && after2.as_deref() == Some(expected)) || (notice && after2.as_deref() == Some(sentinel2))),
        format!("{ready2} {status2} outcome={outcome}"),
    );
    if let Some(snapshot) = saved {
        let restored = mac::restore(app, snapshot).await;
        println!("INFO pasteboard restored to its previous contents: {restored}");
    } else {
        println!("INFO pasteboard could not be snapshotted; it still holds a smoke sentinel");
    }
}

#[cfg(not(target_os = "macos"))]
async fn block_copy(_: &AppHandle, checks: &mut Checks, _: &str) {
    checks.check(
        "[7] copy button on the real pasteboard",
        false,
        "NOT COVERED: macOS only",
    );
}

// ---------------------------------------------------------------------------------------------
// 4: stop button

async fn block_stop(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let up = &fx.up;
    up.script(Script::Sse(vec![delta("Stop partial text"), Step::Hold]));
    let held = up.held_closed.load(Ordering::SeqCst);
    let started = js(
        app,
        "await newSession('chat-model'); window.__spy.events = []; await send('T4 stop me'); \
         const b = await until(() => { const a = $(STREAM); return a && txt(a).includes('Stop partial text') && a; }, 12000); \
         const stop = btn('停止生成'); if (stop) stop.click(); \
         const ok = await until(() => idle() && lastDone() && txt(lastDone()).includes('已中断'), 12000); \
         const a = lastDone(); \
         return { partial: !!b, ok, text: a ? txt(a) : null, md: a ? mdText(a) : null, generatingGone: !$(STREAM), send: !!btn('发送'), \
                  cancelCalled: window.__spy.calls.includes('cancel_chat_generation'), kinds: kinds(), \
                  notice: $$('[role=status],[role=alert]').map(txt).join('|') };",
    )
    .await;
    let db = wait_rows(&fx.db, &turn_sql("T4 stop me"), terminal, 10).await;
    let row = db.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "[4] stop button mid-stream: UI leaves the generating state, shows the 已中断 badge with the partial text, send is available again",
        started["partial"] == true
            && started["ok"] != false
            && started["generatingGone"] == true
            && started["send"] == true
            && started["cancelCalled"] == true
            && started["md"] == "Stop partial text"
            && started["text"].as_str().is_some_and(|t| t.contains("已中断")),
        started.to_string(),
    );
    checks.check(
        "[4] DB: assistant row interrupted with the partial text, task interrupted/task_cancelled, upstream connection closed",
        row["status"] == "interrupted"
            && row["content"] == "Stop partial text"
            && row["task_status"] == "interrupted"
            && row["task_err"] == "task_cancelled"
            && up.held_closed.load(Ordering::SeqCst) == held + 1,
        format!("{row} held_closed={}", up.held_closed.load(Ordering::SeqCst)),
    );
}

// ---------------------------------------------------------------------------------------------
// 5: page reload mid-stream

async fn block_reload(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let up = &fx.up;
    up.script(Script::Sse(vec![delta("Reload partial text"), Step::Hold]));
    let held = up.held_closed.load(Ordering::SeqCst);
    let started = js(
        app,
        "await newSession('chat-model'); await send('T5 reload mid stream'); \
         const b = await until(() => { const a = $(STREAM); return a && txt(a).includes('Reload partial text') && a; }, 12000); \
         return { partial: !!b };",
    )
    .await;
    let reloaded = goto(app, MAIN, "/workbench").await;
    let db = wait_rows(&fx.db, &turn_sql("T5 reload mid stream"), terminal, 10).await;
    let row = db.first().cloned().unwrap_or(Value::Null);
    sleep(Duration::from_millis(300)).await;
    checks.check(
        "[5] page reload mid-stream cancels the generation: DB assistant interrupted with the partial text, task interrupted/task_cancelled, upstream closed",
        started["partial"] == true
            && reloaded
            && row["status"] == "interrupted"
            && row["content"] == "Reload partial text"
            && row["task_status"] == "interrupted"
            && row["task_err"] == "task_cancelled"
            && up.held_closed.load(Ordering::SeqCst) == held + 1,
        format!("{started} reloaded={reloaded} {row} held_closed={}", up.held_closed.load(Ordering::SeqCst)),
    );
    let after = js(
        app,
        "const item = await until(() => $$('section[aria-label=\"会话列表\"] li button').find((b) => txt(b).includes('T5 reload mid stream')), 8000); \
         if (!item) throw new Error('session missing after reload'); item.click(); \
         const ok = await until(() => { const a = lastDone(); return a && txt(a).includes('已中断') && a; }, 10000); \
         const a = lastDone(); \
         return { ok: !!ok, md: a ? mdText(a) : null, text: a ? txt(a) : null, generating: !!$(STREAM), send: !!btn('发送') };",
    )
    .await;
    checks.check(
        "[5] after the reload the UI shows the terminal message: 已中断 badge, partial text, 已停止生成 text, no generating bubble",
        after["ok"] == true
            && after["md"] == "Reload partial text"
            && after["generating"] == false
            && after["text"].as_str().is_some_and(|t| t.contains("已停止生成")),
        after.to_string(),
    );
}

// ---------------------------------------------------------------------------------------------
// 8: sessions, search, paging, regenerate, delete

fn uuid_for(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

async fn seed_sessions(db: &DatabaseConnection) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut sql = String::from("BEGIN;");
    let sessions = [
        (1u32, "Alpha search session", now - 600),
        (2, "Beta search session", now - 700),
        (3, "Bulk paging session", now - 500),
    ];
    for (n, title, at) in sessions {
        sql.push_str(&format!(
            "INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES('{}','{LOCAL_USER}','{title}',{at},{at});",
            uuid_for(n)
        ));
    }
    let message = |id: String, session: u32, seq: u32, role: &str, content: &str| {
        format!(
            "INSERT INTO chat_messages(id,session_id,user_id,seq,role,content,status,created_at,updated_at) \
             VALUES('{id}','{}','{LOCAL_USER}',{seq},'{role}','{content}','completed',1,1);",
            uuid_for(session)
        )
    };
    sql.push_str(&message(
        "seed-a-1".into(),
        1,
        1,
        "user",
        "Total 100% sure literal",
    ));
    sql.push_str(&message(
        "seed-b-1".into(),
        2,
        1,
        "user",
        "about 100 percent only",
    ));
    for seq in 1..=260u32 {
        let role = if seq % 2 == 1 { "user" } else { "assistant" };
        sql.push_str(&message(
            format!("seed-bulk-{seq}"),
            3,
            seq,
            role,
            &format!("bulk message {seq}"),
        ));
    }
    sql.push_str("COMMIT;");
    db.execute_unprepared(&sql).await.expect("seed chat rows");
}

async fn block_lists(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let up = &fx.up;
    let listed = js(
        app,
        "const ok = await until(() => $$('section[aria-label=\"会话列表\"] li').length >= 3, 8000); \
         return { ok, titles: $$('section[aria-label=\"会话列表\"] li').map((li) => txt(li)) };",
    )
    .await;
    let titles = listed["titles"].to_string();
    checks.check(
        "[8] session list shows the seeded sessions newest first",
        listed["ok"] != false
            && titles.contains("Bulk paging session")
            && titles.contains("Alpha search session")
            && titles.contains("Beta search session")
            && titles.find("Bulk paging").unwrap_or(usize::MAX)
                < titles.find("Alpha search").unwrap_or(0)
            && titles.find("Alpha search").unwrap_or(usize::MAX)
                < titles.find("Beta search").unwrap_or(0),
        titles,
    );
    let search = |query: &'static str| async move {
        js(
            app,
            &format!(
                "const input = $('input[aria-label=\"搜索会话\"]'); setVal(input, {query:?}); \
                 await wait(900); const sec = await until(() => $('section[aria-label=\"搜索结果\"]'), 4000); await wait(400); \
                 return {{ hits: sec ? $$('li', sec).map((li) => txt(li)) : null }};"
            ),
        )
        .await
    };
    let literal = search("100%").await;
    let control = search("100").await;
    let hits = |v: &Value| v["hits"].as_array().cloned().unwrap_or_default();
    let all_hits = |v: &Value| {
        hits(v)
            .iter()
            .map(|h| h.as_str().unwrap_or("").to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    };
    checks.check(
        "[8] search treats % literally: '100%' matches only the session containing '100%' (not 'about 100 percent'); control '100' matches both of them",
        hits(&literal).len() == 1
            && hits(&literal)[0].as_str().is_some_and(|t| t.contains("Alpha search session"))
            && !all_hits(&literal).contains("Beta search session")
            && all_hits(&control).contains("Alpha search session")
            && all_hits(&control).contains("Beta search session"),
        format!("literal={literal} control={control}"),
    );
    js(
        app,
        "setVal($('input[aria-label=\"搜索会话\"]'), ''); await wait(300); return true;",
    )
    .await;
    // Pagination over 260 seeded messages.
    let step = r##"
        const count = () => $$('ol[aria-label="消息"] > li').length;
        const first = () => txt($$('ol[aria-label="消息"] > li')[0]);
        return { count: count(), first: first().slice(0, 40), older: !!btn('加载更早的消息'), latest: !!btn('回到最新'), trimmedText: document.body.textContent.includes('较新的消息已从当前窗口移除') };
    "##;
    let open = js(
        app,
        "const item = $$('section[aria-label=\"会话列表\"] li button').find((b) => txt(b).includes('Bulk paging session')); item.click(); \
         await until(() => $$('ol[aria-label=\"消息\"] > li').length >= 50, 8000); \
         return { count: $$('ol[aria-label=\"消息\"] > li').length };",
    )
    .await;
    let mut observed = vec![js(app, step).await];
    let mut max = observed[0]["count"].as_u64().unwrap_or(0);
    for _ in 0..5 {
        let signature = observed
            .last()
            .map(|v| format!("{}|{}", v["count"], v["first"]))
            .unwrap_or_default();
        let next = js(
            app,
            &format!(
                "const sig = () => $$('ol[aria-label=\"消息\"] > li').length + '|' + JSON.stringify(txt($$('ol[aria-label=\"消息\"] > li')[0]).slice(0, 40)); \
                 const b = btn('加载更早的消息'); if (!b) return {{ none: true }}; b.click(); \
                 await until(() => sig() !== {signature:?}, 6000); await wait(250); return {{ ok: true }};"
            ),
        )
        .await;
        if next["none"] == true {
            break;
        }
        let state = js(app, step).await;
        max = max.max(state["count"].as_u64().unwrap_or(0));
        observed.push(state);
    }
    let counts: Vec<u64> = observed
        .iter()
        .map(|v| v["count"].as_u64().unwrap_or(0))
        .collect();
    let firsts: Vec<String> = observed
        .iter()
        .map(|v| v["first"].as_str().unwrap_or("").to_owned())
        .collect();
    let trimmed_seen = observed
        .iter()
        .any(|v| v["trimmedText"] == true && v["latest"] == true);
    checks.check(
        "[8] message pagination: 50 newest load first, 'load older' grows the window by 50 and the DOM never holds more than 200 message nodes",
        open["count"] == 50 && counts.starts_with(&[50, 100, 150, 200]) && max <= 200 && firsts[0].contains("bulk message 211") && firsts[3].contains("bulk message 61"),
        format!("counts={counts:?} firsts={firsts:?} max={max}"),
    );
    checks.check(
        "[8] past the cap the newest side is trimmed: a notice and a 'back to latest' button appear while the window stays at 200 nodes",
        trimmed_seen && counts.iter().skip(3).all(|c| *c == 200),
        format!("counts={counts:?} trimmed_seen={trimmed_seen}"),
    );
    checks.check(
        "[8] the oldest message (seq 1) is reachable and the 'load older' button disappears at the start of the session",
        firsts.last().is_some_and(|f| f.contains("bulk message 1复制")) && observed.last().is_some_and(|v| v["older"] == false),
        format!("firsts={firsts:?} last_older={:?}", observed.last().map(|v| v["older"].clone())),
    );
    let latest = js(
        app,
        "const b = btn('回到最新'); if (!b) return { none: true }; b.click(); \
         await until(() => $$('ol[aria-label=\"消息\"] > li').length === 50 && !btn('回到最新'), 6000); \
         return { count: $$('ol[aria-label=\"消息\"] > li').length, first: txt($$('ol[aria-label=\"消息\"] > li')[0]).slice(0, 40), latest: !!btn('回到最新') };",
    )
    .await;
    checks.check(
        "[8] 'back to latest' reloads the 50 newest messages",
        latest["count"] == 50
            && latest["first"]
                .as_str()
                .is_some_and(|f| f.contains("bulk message 211"))
            && latest["latest"] == false,
        latest.to_string(),
    );

    // Regenerate, then delete.
    up.script(reply("Regen first answer"));
    let created = js(
        app,
        "await newSession('chat-model'); await send('T8 regen base'); \
         const ok = await until(() => idle() && $$(DONE_A).length === 1 && txt(lastDone()).includes('Regen first answer'), 12000); return { ok: !!ok };",
    )
    .await;
    up.script(reply("Regen second answer"));
    let before = up.bodies();
    let regen = js(
        app,
        "const b = await until(() => btn('重新生成')); if (!b) throw new Error('no regenerate button'); b.click(); \
         const ok = await until(() => idle() && $$(DONE_A).length === 1 && txt(lastDone()).includes('Regen second answer'), 12000); \
         return { ok: !!ok, assistants: $$(DONE_A).length, users: $$(USER_A).length, md: lastDone() ? mdText(lastDone()) : null };",
    )
    .await;
    let session_rows = rows(
        &fx.db,
        "SELECT json_object('id',m.id,'content',m.content,'status',m.status,'role',m.role,'replaces',m.replaces_id,'session',m.session_id) AS j \
         FROM chat_messages m WHERE m.session_id=(SELECT session_id FROM chat_messages WHERE role='user' AND content='T8 regen base') ORDER BY m.seq",
    )
    .await;
    let assistants: Vec<&Value> = session_rows
        .iter()
        .filter(|r| r["role"] == "assistant")
        .collect();
    let regen_body = if up.bodies() > before {
        up.body(before).to_string()
    } else {
        String::new()
    };
    checks.check(
        "[8] regenerate replaces the reply on success: one assistant node in the DOM, one stored assistant row with the new text, the old reply is not sent back upstream",
        created["ok"] == true
            && regen["ok"] == true
            && regen["assistants"] == 1
            && regen["users"] == 1
            && regen["md"] == "Regen second answer"
            && assistants.len() == 1
            && assistants[0]["content"] == "Regen second answer"
            && assistants[0]["status"] == "completed"
            && !regen_body.is_empty()
            && !regen_body.contains("Regen first answer")
            && regen_body.contains("T8 regen base"),
        format!("{created} {regen} rows={session_rows:?}"),
    );
    let session_id = session_rows
        .first()
        .map(|r| r["session"].as_str().unwrap_or("").to_owned())
        .unwrap_or_default();
    let tasks_before = rows(
        &fx.db,
        &format!("SELECT json_object('id',id,'kind',kind,'session',session_id) AS j FROM workbench_tasks WHERE session_id='{session_id}'"),
    )
    .await;
    let deleted = js(
        app,
        "const del = btn('删除会话'); if (!del) throw new Error('no delete button'); del.click(); \
         const dialog = await until(() => $('[role=dialog]')); const confirm = dialog && btn('删除', dialog); if (!confirm) throw new Error('no confirm'); confirm.click(); \
         const ok = await until(() => !$$('section[aria-label=\"会话列表\"] li').some((li) => txt(li).includes('T8 regen base')) && txt(document.body).includes('开始一次对话'), 8000); \
         return { ok: !!ok };",
    )
    .await;
    let remaining = rows(
        &fx.db,
        &format!(
            "SELECT json_object('sessions',(SELECT count(*) FROM chat_sessions WHERE id='{session_id}'),'messages',(SELECT count(*) FROM chat_messages WHERE session_id='{session_id}'),\
             'tasks',(SELECT count(*) FROM workbench_tasks WHERE kind='chat' AND session_id IS NULL AND id IN ({ids}))) AS j",
            ids = tasks_before
                .iter()
                .map(|t| format!("'{}'", t["id"].as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(",")
        ),
    )
    .await;
    let left = remaining.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "[8] delete session: UI drops it and returns to the empty state; DB removes session and messages, keeps its chat task rows with session_id cleared (D072)",
        deleted["ok"] == true
            && !session_id.is_empty()
            && tasks_before.len() == 2
            && left["sessions"] == 0
            && left["messages"] == 0
            && left["tasks"] == 2,
        format!("{deleted} tasks_before={tasks_before:?} after={left}"),
    );
}

// ---------------------------------------------------------------------------------------------
// 9: MCP proposal, approve, reject, expiry, and no tool text in the next turn

async fn block_mcp(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let up = &fx.up;
    if let Some(note) = &fx.mcp_note {
        checks.check("[9] MCP stdio fixture connected", false, note.clone());
        return;
    }
    let pick = r##"
        const pickTool = async () => {
          if (!$$('[role=checkbox]').length) { const t = btnHas('MCP 工具'); t.click(); }
          const c = await until(() => $$('[role=checkbox]').find((x) => labelOf(x).includes('fixture-server / echo')), 8000);
          if (!c) throw new Error('fixture tool not listed: ' + $$('section[aria-label="MCP 工具"]').map(txt).join('|'));
          if (c.getAttribute('aria-checked') !== 'true') c.click();
          await until(() => c.getAttribute('aria-checked') === 'true');
          return txt(btnHas('MCP 工具'));
        };
    "##;
    // Approve once.
    up.script(proposal(MARKER));
    up.script(answer_json(FINAL_ANSWER));
    let before = up.bodies();
    let shown = js(
        app,
        &format!(
            "{pick} await newSession('chat-model'); window.__spy.events = []; const picked = await pickTool(); await send('T9 approve once'); \
             const p = await until(() => $(PROPOSAL), 12000); \
             return {{ picked, proposal: !!p, text: p ? txt(p) : null, args: p ? txt($('pre', p)) : null, streamBubbleNotStreaming: txt($(STREAM)).includes('不流式'), \
                      approveEnabled: !!(p && btn('批准本次调用', p) && !btn('批准本次调用', p).disabled), rejectEnabled: !!(p && btn('拒绝并停止', p) && !btn('拒绝并停止', p).disabled) }};"
        ),
    )
    .await;
    checks.check(
        "[9] a tool proposal is rendered with the full arguments, a countdown and separate approve / reject buttons",
        shown["proposal"] == true
            && shown["args"].as_str().is_some_and(|a| a.contains(MARKER))
            && shown["text"].as_str().is_some_and(|t| t.contains("fixture-server / echo") && t.contains("剩余"))
            && shown["approveEnabled"] == true
            && shown["rejectEnabled"] == true,
        shown.to_string(),
    );
    let approved = js(
        app,
        &format!(
            "const p = $(PROPOSAL); btn('批准本次调用', p).click(); \
             const ok = await until(() => idle() && $$(DONE_A).length === 1 && txt(lastDone()).includes({FINAL_ANSWER:?}), 15000); \
             return {{ ok: !!ok, kinds: kinds() }};"
        ),
    )
    .await;
    let first = if up.bodies() > before {
        up.body(before).to_string()
    } else {
        String::new()
    };
    let followup = if up.bodies() > before + 1 {
        up.body(before + 1).to_string()
    } else {
        String::new()
    };
    let followup_json = if up.bodies() > before + 1 {
        up.body(before + 1)
    } else {
        Value::Null
    };
    let last_role = followup_json["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(|m| m["role"].clone());
    checks.check(
        "[9] approve once: exactly one tool call round trip, the follow-up request ends with the tool result (capture control), the final answer is shown",
        approved["ok"] == true
            && up.bodies() == before + 2
            && !first.contains("\"role\":\"tool\"")
            && last_role == Some(json!("tool"))
            && followup.contains(MARKER),
        format!("{approved} bodies={} last_role={last_role:?}", up.bodies() - before),
    );
    let db = wait_rows(&fx.db, &turn_sql("T9 approve once"), terminal, 10).await;
    let row = db.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "[9] the stored assistant message is the final answer only (no tool text), task succeeded",
        row["content"] == FINAL_ANSWER
            && row["status"] == "completed"
            && row["task_status"] == "succeeded",
        row.to_string(),
    );
    // The next plain turn must not carry the tool round trip.
    up.script(reply("Plain follow up"));
    let before = up.bodies();
    let plain = js(app, "await send('T9 plain follow'); const ok = await until(() => idle() && $$(DONE_A).length === 2 && txt(lastDone()).includes('Plain follow up'), 12000); return { ok: !!ok, picked: txt(btnHas('MCP 工具')) };").await;
    let next = if up.bodies() > before {
        up.body(before).to_string()
    } else {
        String::new()
    };
    checks.check(
        "[9] the next turn's request body contains the earlier final answer but NOT the tool argument/result text (tool round trips never enter later context)",
        plain["ok"] == true && up.bodies() == before + 1 && next.contains(FINAL_ANSWER) && next.contains("T9 approve once") && !next.contains(MARKER) && !next.contains("\"role\":\"tool\"") && !next.contains("tool_calls"),
        format!("{plain} marker_in_next={} len={}", next.contains(MARKER), next.len()),
    );
    // Reject.
    up.script(proposal(MARKER_REJECT));
    let before = up.bodies();
    let rejected = js(
        app,
        &format!(
            "{pick} const picked = await pickTool(); await send('T9 reject once'); const p = await until(() => $(PROPOSAL), 12000); \
             if (!p) throw new Error('no proposal'); btn('拒绝并停止', p).click(); \
             const ok = await until(() => idle() && !$(PROPOSAL), 12000); await wait(500); \
             return {{ ok: !!ok, picked, text: lastDone() ? txt(lastDone()) : null }};"
        ),
    )
    .await;
    let rejected_rows = wait_rows(&fx.db, &turn_sql("T9 reject once"), terminal, 10).await;
    let rrow = rejected_rows.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "[9] reject: the turn ends without a follow-up Provider call, the rejected argument text is not stored, the UI is idle",
        rejected["ok"] == true
            && up.bodies() == before + 1
            && rrow["status"] != "generating"
            && !rrow["content"].as_str().unwrap_or("").contains(MARKER_REJECT),
        format!("{rejected} row={rrow}"),
    );
    // Expiry (the proposal window is 120 s in production; it is waited out, not shortened).
    up.script(proposal(MARKER_EXPIRE));
    let before = up.bodies();
    let parked = js(
        app,
        &format!(
            "{pick} window.__spy.events = []; await pickTool(); await send('T9 expire it'); const p = await until(() => $(PROPOSAL), 12000); \
             const ev = window.__spy.events; const prop = ev.find((e) => e.m.kind === 'tool_proposal'); const started = ev.find((e) => e.m.kind === 'started'); \
             return {{ proposal: !!p, proposalId: prop && prop.m.proposal.id, taskId: started && started.m.task_id }};"
        ),
    )
    .await;
    let mut seen_expired_text = false;
    let mut expired_rows = Vec::new();
    for _ in 0..30 {
        sleep(Duration::from_secs(5)).await;
        let state = js(app, "const p = $(PROPOSAL); return { text: p ? txt(p) : null, approveDisabled: p ? btn('批准本次调用', p).disabled : null, idle: idle() };").await;
        if state["text"]
            .as_str()
            .is_some_and(|t| t.contains("提案已过期"))
        {
            seen_expired_text = state["approveDisabled"] == true;
        }
        expired_rows = rows(&fx.db, &turn_sql("T9 expire it")).await;
        if expired_rows
            .first()
            .is_some_and(|r| r["status"] != "generating")
            && state["idle"] == true
        {
            break;
        }
    }
    let erow = expired_rows.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "[9] expired proposal (120 s window waited out): host ends the turn, UI is idle, no follow-up Provider call, argument text not stored",
        parked["proposal"] == true
            && erow["status"] != "generating"
            && erow["status"] != Value::Null
            && up.bodies() == before + 1
            && !erow["content"].as_str().unwrap_or("").contains(MARKER_EXPIRE),
        format!("{parked} row={erow} approve_disabled_text_seen={seen_expired_text}"),
    );
    let stale = js(
        app,
        &format!(
            "const r = await window.__TAURI_INTERNALS__.invoke('advance_chat_tool_turn', {{ input: {{ task_id: {task:?}, action: {{ type: 'approve', proposal_id: {proposal:?} }} }} }}); \
             return {{ success: r.success, code: r.error && r.error.code }};",
            task = parked["taskId"].as_str().unwrap_or(""),
            proposal = parked["proposalId"].as_str().unwrap_or(""),
        ),
    )
    .await;
    checks.check(
        "[9] approving the expired proposal is refused by the host (success=false)",
        stale["success"] == false && stale["code"].is_number(),
        stale.to_string(),
    );
}

// ---------------------------------------------------------------------------------------------
// 6: window destroy mid-stream

async fn block_destroy(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let up = &fx.up;
    up.script(Script::Sse(vec![delta("Destroy partial text"), Step::Hold]));
    let held = up.held_closed.load(Ordering::SeqCst);
    let started = js(
        app,
        "await newSession('chat-model'); await send('T6 destroy window'); \
         const b = await until(() => { const a = $(STREAM); return a && txt(a).includes('Destroy partial text') && a; }, 12000); return { partial: !!b };",
    )
    .await;
    let destroyed = match app.get_webview_window(MAIN) {
        Some(window) => window.destroy().is_ok(),
        None => false,
    };
    let db = wait_rows(&fx.db, &turn_sql("T6 destroy window"), terminal, 10).await;
    let row = db.first().cloned().unwrap_or(Value::Null);
    sleep(Duration::from_millis(300)).await;
    checks.check(
        "[6] destroying the window mid-stream cancels that generation: DB assistant interrupted with the partial text, task interrupted/task_cancelled, upstream closed",
        started["partial"] == true
            && destroyed
            && row["status"] == "interrupted"
            && row["content"] == "Destroy partial text"
            && row["task_status"] == "interrupted"
            && row["task_err"] == "task_cancelled"
            && up.held_closed.load(Ordering::SeqCst) == held + 1,
        format!("{started} destroyed={destroyed} {row} held_closed={}", up.held_closed.load(Ordering::SeqCst)),
    );
}

// ---------------------------------------------------------------------------------------------

async fn scenario(app: AppHandle, root_ready: bool) {
    let mut checks = Checks::default();
    let (up, db, root, mcp_note) = {
        let fx = app.state::<Fixture>();
        (
            fx.up.clone(),
            fx.db.clone(),
            fx.root.clone(),
            fx.mcp_note.clone(),
        )
    };
    let fx = Fixture {
        up,
        db,
        root: root.clone(),
        mcp_note,
    };
    if !root_ready {
        checks.check("main window loaded /workbench", false, "page never loaded");
        finish(checks, &root);
    }
    let info = eval(
        &app,
        MAIN,
        "return { ua: navigator.userAgent, secure: window.isSecureContext, clipboard: !!navigator.clipboard };",
    )
    .await;
    println!("INFO webview {}", info["value"]);
    seed_sessions(&fx.db).await;
    let reloaded = goto(&app, MAIN, "/workbench").await;
    checks.check(
        "workbench page (re)loaded with seeded sessions",
        reloaded,
        "reload failed",
    );
    let shell = js(&app, "const ok = await until(() => $('h1') && txt($('h1')) === 'AI 工作台' && $('nav[aria-label=\"会话\"]'), 10000); return { ok: !!ok, boundary: !!$('[data-error-kind]') };").await;
    checks.check(
        "the real /workbench chat page renders (h1, session sidebar, no error boundary)",
        shell["ok"] == true && shell["boundary"] == false,
        shell.to_string(),
    );
    if wanted(1) || wanted(2) {
        block_stream(&app, &mut checks, &fx).await;
    }
    if wanted(3) {
        block_markdown(&app, &mut checks, &fx).await;
    }
    if wanted(4) {
        block_stop(&app, &mut checks, &fx).await;
    }
    if wanted(5) {
        block_reload(&app, &mut checks, &fx).await;
    }
    if wanted(8) {
        block_lists(&app, &mut checks, &fx).await;
    }
    if wanted(9) {
        block_mcp(&app, &mut checks, &fx).await;
    }
    if wanted(6) {
        block_destroy(&app, &mut checks, &fx).await;
    }
    println!("INFO upstream strays at the end: {:?}", fx.up.strays());
    finish(checks, &root);
}

pub fn run_chat() {
    std::thread::spawn(|| {
        std::thread::sleep(CHAT_WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        std::process::exit(2);
    });

    let (world, bridge) = tauri::async_runtime::block_on(build_world_with(&[]));
    let db_url = format!("sqlite://{}?mode=rwc", world.root.join("dgos.db").display());
    let upstream = Arc::new(Upstream::default());
    let (providers, skills, mcp, workbench, chat, appearance, probe, port, mcp_note) =
        tauri::async_runtime::block_on(async {
            let db = dgos_db::establish_connection(&db_url).await.expect("db");
            dgos_db::migrations::Migrator::up(&db, None)
                .await
                .expect("migrate");
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let address = listener.local_addr().expect("address");
            tauri::async_runtime::spawn(serve(listener, upstream.clone()));
            let vault = Arc::new(MemoryCredentialVault::default());
            let provider = seed_provider(
                db.clone(),
                &vault,
                dgos_types::ProviderConfig {
                    name: PROVIDER_NAME.into(),
                    provider_type: "openai_compatible".into(),
                    base_url: format!("http://{address}/v1"),
                    api_key: "synthetic-chat-smoke-key".into(),
                    enabled: true,
                    config: None,
                },
                LOCAL_USER,
            )
            .await
            .expect("seed provider");
            let providers = ProviderManager::with_vault(&db_url, vault)
                .await
                .expect("providers");
            providers
                .add_manual_model(
                    &provider.id,
                    LOCAL_USER,
                    "chat-model",
                    Some("text-generation"),
                )
                .await
                .expect("manual model");
            let skills_dir = world.root.join("skills");
            std::fs::create_dir_all(&skills_dir).unwrap();
            let skills = dgos_skills::SkillsManager::new(db.clone(), skills_dir);
            let mcp = Arc::new(McpManager::with_policy_and_vault(
                db.clone(),
                Arc::new(commands::DesktopMcpPolicy),
                Arc::new(RefusingMcpVault),
            ));
            let fixture = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../crates/dgos-mcp/tests/fixtures/mcp-server.mjs"
            );
            let note = match mcp
                .create_server(
                    LOCAL_USER,
                    CreateMcpServerRequest {
                        name: "fixture-server".into(),
                        transport: TransportKind::Stdio,
                        stdio: Some(StdioConfig {
                            executable: "node".into(),
                            args: vec![fixture.into(), "normal".into()],
                            cwd: None,
                        }),
                        http: None,
                        enabled: Some(true),
                        auto_connect: None,
                    },
                )
                .await
            {
                Ok(server) => match mcp.connect(LOCAL_USER, &server.id).await {
                    Ok(_) => None,
                    Err(error) => Some(format!("connect failed: {error:?}")),
                },
                Err(error) => Some(format!("create failed: {error:?}")),
            };
            let workbench = dgos_workbench::WorkbenchManager::new(db.clone());
            let chat = dgos_workbench::ChatManager::new(db.clone());
            let probe = dgos_db::establish_connection(&db_url)
                .await
                .expect("probe db");
            let appearance = dgos_db::AppearanceRepository::new(db);
            (
                providers,
                skills,
                mcp,
                workbench,
                chat,
                appearance,
                probe,
                address.port(),
                note,
            )
        });

    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();

    let manager = world.manager.clone();
    let router = world.router.clone();
    let crashes = world.crashes.clone();
    let management = world.management.clone();
    let root = world.root.clone();
    let scenario_world = Mutex::new(Some(world));
    let fixture = Fixture {
        up: upstream,
        db: probe,
        root,
        mcp_note,
    };
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(LoadEvents(load_sender))
        .manage(fixture)
        .manage(Port(port))
        .manage(manager)
        .manage(bridge)
        .manage(router)
        .manage(crashes)
        .manage(management)
        .manage(StartupLoad)
        .manage(providers)
        .manage(commands::SkillsState {
            manager: Arc::new(tokio::sync::RwLock::new(skills)),
        })
        .manage(commands::McpState { manager: mcp })
        .manage(workbench)
        .manage(chat)
        .manage(appearance)
        .on_page_load(|webview, payload| {
            module_host::handle_page_load(webview, payload);
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview
                    .app_handle()
                    .state::<LoadEvents>()
                    .0
                    .send((webview.label().to_owned(), payload.url().path().to_owned()));
            }
        })
        .on_window_event(module_host::handle_window_event)
        .setup(move |app| {
            let events = app.state::<LoadEvents>().0.subscribe();
            WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("workbench".into()))
                .inner_size(1000.0, 760.0)
                .min_inner_size(800.0, 600.0)
                .visible(true)
                .focused(false)
                .build()?;
            // Keeps the process alive when check 6 destroys `main`.
            WebviewWindowBuilder::new(app, "keepalive", WebviewUrl::App("settings".into()))
                .inner_size(400.0, 300.0)
                .visible(false)
                .build()?;
            let handle = app.handle().clone();
            let world = scenario_world.lock().unwrap().take().unwrap();
            tauri::async_runtime::spawn(async move {
                let _world = world;
                let ready = wait_load(events, MAIN, "/workbench").await;
                scenario(handle, ready).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_providers,
            commands::create_provider,
            commands::create_provider_with_models,
            commands::update_provider,
            commands::toggle_provider,
            commands::delete_provider,
            commands::test_provider_connection,
            commands::list_models_for_config,
            commands::test_provider_by_id,
            commands::list_models,
            commands::refresh_models,
            commands::update_provider_order,
            commands::list_usage_logs,
            commands::migrate_provider_credential,
            commands::fetch_skills_registry,
            commands::search_skills,
            commands::get_skill_detail,
            commands::install_skill,
            commands::uninstall_skill,
            commands::list_installed_skills,
            commands::toggle_skill,
            commands::update_skill_config,
            commands::check_skill_updates,
            commands::update_skill,
            commands::check_dependencies,
            commands::list_skill_usage_logs,
            commands::list_workbench_tasks,
            commands::run_workbench_task,
            commands::start_workbench_tool_task,
            commands::advance_workbench_tool_task,
            commands::cancel_workbench_tool_task,
            commands::cancel_workbench_task,
            commands::create_chat_session,
            commands::list_chat_sessions,
            commands::rename_chat_session,
            commands::update_chat_session_target,
            commands::delete_chat_session,
            commands::search_chat_sessions,
            commands::list_chat_messages,
            commands::send_chat_message,
            commands::regenerate_chat_message,
            commands::cancel_chat_generation,
            commands::advance_chat_tool_turn,
            commands::list_mcp_servers,
            commands::get_mcp_server_config,
            commands::create_mcp_server,
            commands::update_mcp_server,
            commands::toggle_mcp_server,
            commands::delete_mcp_server,
            commands::connect_mcp_server,
            commands::disconnect_mcp_server,
            commands::list_mcp_tools,
            commands::refresh_mcp_tools,
            commands::call_mcp_tool,
            commands::cancel_mcp_tool,
            commands::confirm_mcp_tool,
            commands::set_mcp_bearer_token,
            commands::clear_mcp_bearer_token,
            commands::list_mcp_usage_logs,
            commands::list_mcp_connection_logs,
            commands::start_module,
            commands::stop_module,
            commands::update_module,
            commands::get_module_operation,
            commands::get_module_runtime_status,
            commands::attach_module_host,
            commands::report_module_host,
            commands::report_module_crash,
            commands::get_appearance_preferences,
            commands::set_appearance_preferences,
            commands::enable_module,
            commands::disable_module,
            commands::uninstall_module,
            commands::list_modules,
            commands::grant_module_permission,
            commands::revoke_module_permission,
            commands::list_module_crashes,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
