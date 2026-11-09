//! Real-WebView smoke for the AI workbench chat (V1-FR-006, V1-E2E-06 at fixture level).
//!
//! The page normally resolves `/workbench` from embedded `dist/`; every chat command is the production
//! handler over a temp SQLite database (`ChatManager`, `ProviderManager`, `McpManager` are the real
//! types). The UI is driven by DOM events from the evaluated script (click, input, Radix Select via keyboard events), which
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
//!
//! Layout-only run (does not certify a whole-tree build): start `npm run dev -- --host 127.0.0.1`,
//! then run `DGOS_CHAT_SMOKE_ONLY=13 DGOS_CHAT_SMOKE_URL=http://localhost:1420/workbench
//! DGOS_CHAT_SMOKE_EXPECT_ORIGIN=http://localhost:1420 cargo test -p dgos --features webview-smoke
//! --test webview_smoke_chat -- --nocapture`. This feature embeds dist even in debug;
//! only check 13 accepts this explicit loopback development URL override.
//! Preserve the `INFO chat-layout resources` JSON (resource URLs + SHA-256, inline CSS hashes),
//! executable path and resulting checks. Response hashes use fresh same-origin fetches, so keep Vite
//! and source stable during the run. For embedded resources, first build the current frontend
//! successfully, rebuild this harness, omit both URL env vars, and compare logged /assets/ hashes
//! with that dist. A pre-existing executable or dist is never evidence of a current whole-tree pass.

#[path = "phase6.rs"]
mod phase6;
use super::modules::build_world_with;
use super::{eval, finish, goto, ipc, wait_load, Checks, LoadEvents, Pending};
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
    if path.ends_with("/audio/transcriptions") {
        let body = String::from_utf8_lossy(&buffer[header_end..header_end + length]);
        let valid = head.contains("synthetic-chat-smoke-key") && body.contains("whisper-1") && body.contains("OggSphase6-controlled-audio");
        let text = if valid { r#"{"text":"PHASE6-TRANSCRIBED"}"# } else { r#"{"error":"invalid controlled request"}"# };
        let status = if valid { 200 } else { 400 };
        let out = format!("HTTP/1.1 {status} Status\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
        let _ = socket.write_all(out.as_bytes()).await;
        return;
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
const btn = (t, root = document) => $$('button', root).find((b) => (b.getAttribute('aria-label') || txt(b)) === t);
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
  const before = $$('[aria-label="AI 会话标签"] [role=tab]').length;
  const create = $('button[aria-label="新建会话"]') || btn('新建会话');
  if (!create) throw new Error('new session control missing');
  create.click();
  const ready = await until(() => $$('[aria-label="AI 会话标签"] [role=tab]').length > before && $('button[aria-label="会话目标"]'));
  if (!ready) throw new Error('new session did not appear');
  $('button[aria-label="会话目标"]').click();
  const provider = await until(() => $$('.chat-provider-option').find(x => x.getAttribute('aria-label') === 'Smoke Loopback'));
  if (!provider) throw new Error('provider missing');
  provider.click();
  const modelOption = await until(() => $$('.chat-model-option').find(x => x.getAttribute('aria-label') === model));
  if (!modelOption) throw new Error('model missing: ' + model);
  modelOption.click();
  if (!(await until(() => !$('.chat-target-popover') && txt($('.chat-model-button-label')).includes(model)))) throw new Error('target was not applied: ' + txt($('button[aria-label="会话目标"]')));
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
        "const tab = $('.workspace-session-tabs [role=tab][aria-selected=true]'); tab.dispatchEvent(new MouseEvent('contextmenu', {bubbles:true,clientX:100,clientY:100})); \
         const del = await until(() => $$('[role=menuitem]').find(e => txt(e)==='删除会话')); if (!del) throw new Error('no delete menu item'); del.click(); \
         const dialog = await until(() => $('[data-modal-kind][role=dialog]')); const confirm = dialog && btn('删除会话', dialog); if (!confirm) throw new Error('no confirm'); confirm.click(); \
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

async fn block_workspace_task(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    fx.up.script(Script::Sse(vec![Step::Hold]));
    let loaded = goto(app, MAIN, "/tasks/single").await;
    checks.check("[10] task surface loads", loaded, "navigation");
    let started = js(app, r#"
        await choose('Provider', 'Smoke Loopback');
        await choose('模型', 'chat-model');
        const noSkill = await until(() => $$('[role=checkbox]').find(el=>labelOf(el).includes('明确不使用技能')));
        noSkill.click();
        const input = $$('textarea').find(el=>labelOf(el)==='任务输入'); setVal(input, 'P3 keep this task input'); window.__taskInput = input;
        const run = await until(() => { const b=btn('不使用 Skill 运行'); return b && !b.disabled && b; }); run.click();
        return !!await until(() => btn('取消任务'));
    "#).await;
    checks.check("[10] task starts through production runner and IPC", started == true, started.to_string());
    let kept = js(app, r#"
        const link = name => $$('a').find(a=>txt(a)===name);
        link('桌面总览').click(); await until(()=>location.pathname==='/');
        link('设置').click(); await until(()=>$('[data-settings-shell]'));
        const preserved = window.__taskInput.isConnected && window.__taskInput.value==='P3 keep this task input';
        btn('关闭设置').click(); await until(()=>!$('[data-settings-shell]'));
        link('单任务').click(); await until(()=>location.pathname==='/tasks/single');
        return {preserved, same:window.__taskInput===$$('textarea').find(el=>labelOf(el)==='任务输入'), running:!!btn('取消任务'), cancelled:window.__spy.calls.includes('cancel_workbench_task')};
    "#).await;
    checks.check("[10] desktop and settings preserve task, input and DOM without cancelling", kept["preserved"]==true && kept["same"]==true && kept["running"]==true && kept["cancelled"]==false, kept.to_string());
    let refused = js(app, r#"
        $('button[aria-label="关闭 单任务"]').click();
        await until(()=>$('[role=alertdialog],[role=dialog]'));
        btn('取消').click(); await until(()=>!$('[role=alertdialog],[role=dialog]'));
        return {running:!!btn('取消任务'), cancelled:window.__spy.calls.includes('cancel_workbench_task')};
    "#).await;
    checks.check("[10] declining close leaves the host task running", refused["running"]==true && refused["cancelled"]==false, refused.to_string());
    let closed = js(app, r#"
        $('button[aria-label="关闭 单任务"]').click();
        const stop = await until(()=>btn('停止并关闭')); stop.click();
        const gone = await until(()=>!$('button[aria-label="关闭 单任务"]'),12000);
        const records = await window.__TAURI_INTERNALS__.invoke('list_workbench_tasks',{});
        return {gone:!!gone, cancelled:window.__spy.calls.includes('cancel_workbench_task'), terminal:records.some(r=>r.task.status==='interrupted' && r.task.error_code==='task_cancelled')};
    "#).await;
    checks.check("[10] stop-and-close waits for host cancellation terminal", closed["gone"]==true && closed["cancelled"]==true && closed["terminal"]==true, closed.to_string());
}

#[tauri::command]
async fn get_account_state(app: AppHandle, window: tauri::WebviewWindow) -> Result<commands::CommandResponse<dgos_application::AccountState>, String> {
    if app.try_state::<dgos_application::AccountService>().is_none() {
        return Err("Command get_account_state not found".into());
    }
    Ok(commands::get_account_state(app, window).await)
}

async fn block_account_workspace(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    let resolver = app.state::<Arc<crate::session::SessionResolver>>().inner().clone();
    let mcp = app.state::<commands::McpState>().manager.clone();
    let ports = dgos_application::AccountPorts::new(&fx.db, Arc::new(crate::session::HostSessionPort::new(resolver.clone(), mcp, app.clone())));
    let accounts = dgos_application::AccountService::new(fx.db.clone(), ports);
    accounts.recover().await.unwrap();
    app.manage(accounts);
    let repository = dgos_db::DevelopmentRepository::new(fx.db.clone());
    let cli_providers = dgos_agents::capabilities::CliProviders::new(repository.clone(), app.state::<ProviderManager>().inner().clone()).unwrap();
    let agent_manager = dgos_agents::AgentManager::with_maintenance(fx.root.join("agents.json"), repository, app.state::<dgos_db::MaintenanceGate>().inner().clone(), cli_providers.clone());
    agent_manager.load().await.unwrap();
    app.manage(agent_manager);
    app.manage(cli_providers);
    let alice = ipc(app, MAIN, "create_account", json!({"input":{"username":"alice","password":"alice-password"}})).await;
    let bob = ipc(app, MAIN, "create_account", json!({"input":{"username":"bob","password":"bob-password"}})).await;
    checks.check("[11] two real accounts created", alice.success() && bob.success(), format!("{} {}", alice.describe(), bob.describe()));
    goto(app, MAIN, "/tasks/single").await;
    fx.up.script(Script::Sse(vec![Step::Hold]));
    let task = js(app, r#"
        await choose('Provider','Smoke Loopback'); await choose('模型','chat-model');
        (await until(()=>$$('[role=checkbox]').find(el=>labelOf(el).includes('明确不使用技能')))).click();
        window.__oldInput=$$('textarea').find(el=>labelOf(el)==='任务输入'); setVal(window.__oldInput,'ACCOUNT-OLD-TASK-DRAFT');
        (await until(()=>{const b=btn('不使用 Skill 运行');return b&&!b.disabled&&b;})).click();
        return !!await until(()=>btn('取消任务'));
    "#).await;
    let provider = ipc(app, MAIN, "list_providers", json!({})).await.data()[0]["id"].as_str().unwrap().to_owned();
    let created = ipc(app, MAIN, "create_chat_session", json!({"input":{"target":{"provider_id":provider,"model_id":"chat-model","skill_ids":[],"no_skill":true}}})).await;
    checks.check("[11] old-owner chat created",created.success(),created.describe());
    let Some(session_id) = created.data()["id"].as_str().map(str::to_owned) else { return; };
    fx.up.script(Script::Sse(vec![delta("ACCOUNT-OLD-STREAM"),Step::Hold]));
    let chat = js(app, &format!(r#"
        const id={session_id:?};
        const callback=window.__TAURI_INTERNALS__.transformCallback(event=>{{ window.__accountEvents=(window.__accountEvents||[]).concat(event); }},false);
        const ack=await window.__TAURI_INTERNALS__.invoke('send_chat_message',{{input:{{session_id:id,content:'ACCOUNT-OLD-CHAT',client_request_id:crypto.randomUUID()}},onEvent:`__CHANNEL__:${{callback}}`}});
        return {{ack,stream:!!await until(()=>(window.__accountEvents||[]).some(e=>e.message?.kind==='delta'||e.kind==='delta'))}};
    "#)).await;
    let module = js(app, r#"
        history.pushState({},'', '/apps/mc-1'); window.dispatchEvent(new PopStateEvent('popstate'));
        return !!await until(()=>$('[data-module-id="mc-1"]'));
    "#).await;
    #[cfg(unix)]
    let agent_id = {
        use std::os::unix::fs::PermissionsExt;
        let executable=fx.root.join("codex-account-fixture");
        std::fs::write(&executable,r##"#!/bin/sh
if [ "$1" = "--version" ]; then printf '%s\n' 'codex-cli 0.167.0'; exit 0; fi
while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"config/read"'*) printf '%s\n' '{"id":2,"result":{"config":{"model_provider":"dgos_project","model_providers":{"dgos_project":{"base_url":"FIXTURE_BASE_URL","wire_api":"responses","env_key":"DGOS_PROVIDER_KEY","requires_openai_auth":false}}}}}';;
*'"method":"model/list"'*) printf '%s\n' '{"id":4,"result":{"data":[],"nextCursor":null}}';;
*'"method":"thread/start"'*) printf '{"id":3,"result":{"cwd":"%s","model":"chat-model","modelProvider":"dgos_project","thread":{"id":"account-thread","turns":[]}}}\n' "$PWD";;
*'"method":"turn/start"'*) printf '%s\n' '{"id":5,"result":{"turn":{"id":"account-turn"}}}' '{"method":"turn/started","params":{"threadId":"account-thread","turn":{"id":"account-turn"}}}' '{"id":"account-approval","method":"item/commandExecution/requestApproval","params":{"threadId":"account-thread","turnId":"account-turn","itemId":"account-item","command":"echo allowed","availableDecisions":["accept","decline"]}}';;
esac
done
"##.replace("FIXTURE_BASE_URL", &app.state::<ProviderManager>().list_providers(LOCAL_USER).await.unwrap()[0].base_url)).unwrap();
        std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o700)).unwrap();
        let id=uuid::Uuid::new_v4().to_string();
        let probe = ipc(app, MAIN, "probe_cli", json!({"tool":"codex","path":executable})).await;
        let current = ipc(app, MAIN, "get_cli_preferences", json!({})).await;
        let paths = json!({"codex":executable,"claude":null,"gemini":null,"node":null});
        let saved = ipc(app, MAIN, "set_cli_preferences", json!({"paths":paths,"expectedRevision":current.data()["revision"]})).await;
        let readback = ipc(app, MAIN, "get_cli_preferences", json!({})).await;
        let stale = ipc(app, MAIN, "set_cli_preferences", json!({"paths":paths,"expectedRevision":current.data()["revision"]})).await;
        checks.check("[11] CLI version, account CAS save and readback use production IPC",probe.success() && probe.data()["status"]=="available" && probe.data()["version"]=="codex-cli 0.167.0" && probe.data()["managed"]==false && saved.data()["saved"]==true && readback.data()["paths"]["codex"]==json!(executable) && stale.data()["saved"]==false,format!("{} {} {} {}",probe.describe(),saved.describe(),readback.describe(),stale.describe()));
        // The cached badge must survive renderer lifecycles even when its executable
        // is no longer runnable; entering settings must not run a fresh probe.
        std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o600)).unwrap();
        let cached = js(app, r#"
          const states=[];
          for(let visit=0;visit<2;visit++){
            $$('a').find(a=>a.getAttribute('href')==='/settings').click();
            const tab=await until(()=>$$('[role=tab]').find(e=>txt(e)==='CLI 工具')); kd(tab,'Enter');
            const version=await until(()=>$('.cli-tool-codex .cli-tool-version'));
            const input=$('.cli-tool-codex input');
            states.push({version:txt(version),path:input?.value,readOnly:input?.readOnly});
            btn('关闭设置').click(); await until(()=>!$('[data-settings-shell]'));
          }
          return states;
        "#).await;
        let unchanged = ipc(app, MAIN, "get_cli_preferences", json!({})).await;
        std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o700)).unwrap();
        checks.check("[11] persisted CLI badge reopens with read-only path and no re-probe", cached.as_array().is_some_and(|states| states.len()==2 && states.iter().all(|state| state["version"]=="codex-cli 0.167.0" && state["path"]==json!(executable) && state["readOnly"]==true)) && unchanged.data()["revision"]==readback.data()["revision"] && unchanged.data()["detections"]["codex"]["version"]=="codex-cli 0.167.0", format!("states={cached} {}",unchanged.describe()));
        let node_root=fx.root.join("selected-node");
        std::fs::create_dir_all(node_root.join("bin")).unwrap();
        let node_executable=node_root.join("bin/node");
        std::fs::write(&node_executable,"#!/bin/sh\n[ \"$1\" = --version ] || exit 2\nprintf 'v22.33.44\\n'\n").unwrap();
        std::fs::set_permissions(&node_executable,std::fs::Permissions::from_mode(0o700)).unwrap();
        let selected_node=ipc(app, MAIN, "probe_node_directory", json!({"directory":node_root})).await;
        let mut selected_paths=unchanged.data()["paths"].clone(); selected_paths["node"]=json!(node_executable);
        let node_saved=ipc(app, MAIN, "set_cli_preferences", json!({"paths":selected_paths,"expectedRevision":unchanged.data()["revision"]})).await;
        let node_readback=ipc(app, MAIN, "get_cli_preferences", json!({})).await;
        checks.check("[11] selected Node path and verified version persist together through IPC", selected_node.data()["status"]=="available" && node_saved.data()["saved"]==true && node_readback.data()["paths"]["node"]==json!(node_executable) && node_readback.data()["detections"]["node"]["version"]=="v22.33.44" && node_readback.data()["detections"]["codex"]["version"]=="codex-cli 0.167.0", format!("{} {} {}",selected_node.describe(),node_saved.describe(),node_readback.describe()));
        let manager = app.state::<dgos_agents::AgentManager>();
        let project = manager.create_project(LOCAL_USER, dgos_agents::projects::CreateProjectInput { kind: "import".into(), name: "Account boundary".into(), path: fx.root.to_string_lossy().into_owned(), source: None }, tokio_util::sync::CancellationToken::new()).await.unwrap();
        let development_task = manager.save_development_task(LOCAL_USER, dgos_agents::projects::SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Account boundary".into(), requirements: "Reject old account operations".into(), status: "todo".into() }, tokio_util::sync::CancellationToken::new()).await.unwrap();
        let selected = app.state::<ProviderManager>().list_providers(LOCAL_USER).await.unwrap().remove(0);
        let provider = dgos_agents::capabilities::CliProviderRef { provider_id: selected.id, model_id: "chat-model".into(), provider_revision: selected.config_revision };
        let head = app.state::<dgos_agents::capabilities::CliProviders>().save(LOCAL_USER, dgos_agents::capabilities::SaveCliProviderInput { project_id: project.id, expected_revision: 0, configuration: dgos_agents::capabilities::CliProviderConfiguration { engine: dgos_agents::Engine::Codex, provider: provider.clone() } }).await.unwrap();
        let opened=ipc(app,MAIN,"open_agent_session",json!({"input":{"intent":"create","id":id,"engine":"codex","task_id":development_task.id,"project_revision":head.revision,"provider":provider,"options":{"workspace":fx.root,"resume_id":null,"model":null,"trust_workspace":true}}})).await;
        let sent=ipc(app,MAIN,"send_agent_message",json!({"input":{"session_id":id,"generation":opened.data()["generation"],"request_id":uuid::Uuid::new_v4(),"text":"hold for account change"}})).await;
        checks.check("[11] production Agent process starts",opened.0["ok"]==true && sent.0["ok"]==true,format!("{} {}",opened.describe(),sent.describe()));
        id
    };
    let (_, old_lease)=resolver.agent_session().unwrap();
    checks.check("[11] chat, task and module run together",task==true && chat["stream"]==true && module==true,format!("task={task} chat={chat} module={module}"));
    let failed=ipc(app,MAIN,"sign_in",json!({"input":{"username":"bob","password":"incorrect"}})).await;
    let intact=js(app,"return window.__oldInput.isConnected && window.__oldInput.value==='ACCOUNT-OLD-TASK-DRAFT';").await;
    let running=fx.db.query_one(Statement::from_string(DbBackend::Sqlite,"SELECT COUNT(*) AS n FROM workbench_tasks WHERE status='running'".to_owned())).await.unwrap().unwrap().try_get::<i64>("","n").unwrap();
    checks.check("[11] failed authentication preserves all running work and draft",failed.code()==Some(1008005) && intact==true && running==2 && !old_lease.is_cancelled(),format!("{} intact={intact} running={running}",failed.describe()));
    let reload_path = app.get_webview_window(MAIN).and_then(|window| window.url().ok()).map(|url| url.path().to_owned()).unwrap_or_default();
    let reloaded=app.state::<LoadEvents>().0.subscribe();
    let submitted=js(app,r#"
        const account=await until(()=>$('button[aria-label="alice"]')); kd(account,'ArrowDown');
        const switchItem=await until(()=>$$('[role=menuitem]').find(el=>txt(el)==='切换用户')); kd(switchItem,'Enter');
        const dialog=await until(()=>$('[role=dialog]'));
        setVal($('input[type=password]',dialog),'bob-password'); await wait(50);
        const submit=btn('切换',dialog); if(!submit) return {error:'missing switch button',dialog:txt(dialog)}; submit.click(); return true;
    "#).await;
    let loaded=wait_load(reloaded,MAIN,&reload_path).await;
    let switched=ipc(app,MAIN,"get_account_state",json!({})).await;
    checks.check("[11] production account menu switches and reloads",submitted==true && loaded && switched.data()["current"]["username"]=="bob",format!("submitted={submitted} loaded={loaded} path={reload_path} {}",switched.describe()));
    let records=ipc(app,MAIN,"list_workbench_tasks",json!({})).await;
    let sessions=ipc(app,MAIN,"list_chat_sessions",json!({"input":{}})).await;
    let terminal=fx.db.query_one(Statement::from_string(DbBackend::Sqlite,"SELECT COUNT(*) AS n FROM workbench_tasks WHERE status='running'".to_owned())).await.unwrap().unwrap().try_get::<i64>("","n").unwrap();
    checks.check("[11] switching cancels old tasks and isolates new owner data",switched.success() && old_lease.is_cancelled() && terminal==0 && records.0["value"].as_array().is_some_and(|r|r.is_empty()) && sessions.data()["items"].as_array().is_some_and(|r|r.is_empty()),format!("{} records={} sessions={} running={terminal}",switched.describe(),records.describe(),sessions.describe()));
    #[cfg(unix)]
    checks.check("[11] new account cannot read old Agent",ipc(app,MAIN,"get_agent_session",json!({"sessionId":agent_id})).await.0["ok"]==false,"old Agent accessible");
    let gate=js(app,"return !!await until(()=>$('button[aria-label=\"bob\"]'));").await;
    let clean=js(app,"return !!await until(()=>window.__oldInput===undefined && !$$('textarea').some(el=>el.value.includes('ACCOUNT-OLD-')) && !document.body.textContent.includes('ACCOUNT-OLD-'));").await;
    checks.check("[11] new account page contains no old views or drafts",clean==true && gate==true,clean.to_string());
    let out=ipc(app,MAIN,"sign_out",json!({})).await;
    let denied=ipc(app,MAIN,"list_workbench_tasks",json!({})).await;
    let back=ipc(app,MAIN,"sign_in",json!({"input":{"username":"alice","password":"alice-password"}})).await;
    let (_, fresh)=resolver.agent_session().unwrap();
    #[cfg(unix)]
    let old_agent_denied=ipc(app,MAIN,"get_agent_session",json!({"sessionId":agent_id})).await.0["ok"]==false;
    #[cfg(not(unix))]
    let old_agent_denied=true;
    checks.check("[11] sign-out gates commands and same-owner relogin cannot revive old lease",out.success() && denied.0["ok"]==false && back.success() && old_lease.is_cancelled() && !fresh.is_cancelled() && old_agent_denied,format!("{} {} {}",out.describe(),denied.describe(),back.describe()));
}

// ---------------------------------------------------------------------------------------------
// 13: local layout, scroll and focus in the real WebView; no replacement chat state or IPC.

async fn block_layout(app: &AppHandle, checks: &mut Checks, fx: &Fixture) {
    println!("INFO chat-layout executable {:?}", std::env::current_exe());
    let setup = js(app, r##"
        try {
          if (!await until(() => $('button[aria-label="新建会话"]') || btn('新建会话'), 5000)) throw new Error('new session control missing');
          await newSession('chat-model');
          return {ready: !!$('textarea[aria-label="消息输入"]'), url: location.href};
        } catch (error) { return {ready: false, error: String(error), url: location.href, body: txt(document.body).slice(0, 1600)}; }
    "##).await;
    checks.check("[13] production chat session prepared", setup["ready"] == true, setup.to_string());
    if setup["ready"] != true { return; }
    let provenance = js(app, r##"
        const ready = !!await until(() => $('.chat-timeline') && $('.chat-composer-primary') && $('.chat-composer-body'), 5000);
        if (!ready) return {currentLayout: false, url: location.href, origin: location.origin, body: txt(document.body).slice(0, 1600), error: 'chat layout missing after session setup'};
        const hash = async bytes => [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(v => v.toString(16).padStart(2, '0')).join('');
        const entries = [...$$('script[src]').map(e => e.src), ...$$('link[rel=stylesheet]').map(e => e.href)].filter(u => !u.includes('/@vite/'));
        const observedUrls = [...new Set([
          ...entries,
          ...performance.getEntriesByType('resource').map(e => e.name).filter(u => /\.(?:[cm]?[jt]sx?|css)(?:[?#]|$)/.test(u)),
        ])].filter(u => { const resource = new URL(u), page = new URL(location.href); return resource.protocol === page.protocol && resource.host === page.host; }).sort();
        const urls = observedUrls.filter(u => entries.includes(u) || /\/(?:ChatWorkbench|MessageList|Composer)\.tsx(?:[?#]|$)|\/conversation\.css(?:[?#]|$)/.test(u)).slice(0, 8);
        const resources = await Promise.all(urls.map(async url => {
          const controller = new AbortController(), timer = setTimeout(() => controller.abort(), 4000);
          try { const response = await fetch(url, {cache: 'no-store', signal: controller.signal}); if (!response.ok) throw new Error(String(response.status)); const bytes = await response.arrayBuffer(); return {url, bytes: bytes.byteLength, sha256: await hash(bytes)}; }
          catch (error) { return {url, error: String(error)}; }
          finally { clearTimeout(timer); }
        }));
        const inlineStyles = await Promise.all($$('style').map(async e => ({id: e.getAttribute('data-vite-dev-id'), sha256: await hash(new TextEncoder().encode(e.textContent))})));
        return {url: location.href, origin: location.origin, ua: navigator.userAgent, resources, inlineStyles,
          observedUrls, unhashedUrls: observedUrls.filter(u => !urls.includes(u)), currentLayout: ready};
    "##).await;
    println!("INFO chat-layout resources {provenance}");
    let expected_origin = std::env::var("DGOS_CHAT_SMOKE_EXPECT_ORIGIN").ok();
    let resources_recorded = provenance["resources"].as_array().is_some_and(|resources| {
        !resources.is_empty() && resources.iter().all(|resource| resource["sha256"].as_str().is_some_and(|hash| hash.len() == 64))
    });
    let current = provenance["currentLayout"] == true && resources_recorded
        && expected_origin.as_ref().is_none_or(|origin| provenance["origin"].as_str() == Some(origin.as_str()));
    checks.check("[13] current chat layout and resource URL/SHA-256 provenance (expected origin enforced when set)", current, provenance.to_string());
    if !current { return; }

    let resized = app.get_webview_window(MAIN).is_some_and(|window| window.set_size(tauri::LogicalSize::new(800.0, 600.0)).is_ok());
    checks.check("[13] real native window resized to 800 × 600 logical pixels", resized, "set_size");
    let helpers = r##"
        const visible = el => {
          if (!el) return false;
          const r = el.getBoundingClientRect();
          if (r.width <= 0 || r.height <= 0 || r.left < -1 || r.top < -1 || r.right > innerWidth + 1 || r.bottom > innerHeight + 1) return false;
          for (let p = el.parentElement; p; p = p.parentElement) {
            const s = getComputedStyle(p), b = p.getBoundingClientRect();
            if (/(auto|scroll|hidden|clip)/.test(s.overflowY) && (r.top < b.top - 1 || r.bottom > b.bottom + 1)) return false;
            if (/(auto|scroll|hidden|clip)/.test(s.overflowX) && (r.left < b.left - 1 || r.right > b.right + 1)) return false;
          }
          return true;
        };
        const scroller = () => $('.chat-message-scroll');
        const input = () => $('textarea[aria-label="消息输入"]');
    "##;
    // Theme-token enlargement exercises the supported text-size variables without changing density.
    js(app, r##"
        window.__layoutRootStyle = document.documentElement.getAttribute('style');
        for (const [name, size] of [['--font-size-ui', '16px'], ['--font-size-reading', '20px'], ['--font-size-code', '18px']]) document.documentElement.style.setProperty(name, size);
        await wait(300); return true;
    "##).await;
    let long_text = (0..100).map(|n| format!("Layout line {n}: independently scrolling conversation content.\n\n")).collect::<String>();
    fx.up.script(reply(&long_text));
    let independent = js(app, &format!(r##"
        {helpers}
        await send('T13 independent scrolling');
        if (!await until(() => idle() && lastDone() && txt(lastDone()).includes('Layout line 99'), 12000)) throw new Error('long reply missing');
        const expand = btn('展开全部'); if (expand) expand.click();
        const area = input(), list = scroller(), primary = $('.chat-composer-primary'), body = $('.chat-composer-body');
        setVal(area, ('Long draft with independent input scrolling.\n').repeat(90)); await wait(300);
        const pageTop = document.scrollingElement.scrollTop, footerTop = primary.getBoundingClientRect().top;
        list.scrollTop = 0; list.dispatchEvent(new Event('scroll')); await wait(80);
        area.scrollTop = area.scrollHeight; await wait(80);
        const inputMovesOnly = area.scrollTop > 0 && list.scrollTop < 2;
        const inputTop = area.scrollTop;
        list.scrollTop = list.scrollHeight; list.dispatchEvent(new Event('scroll')); await wait(80);
        const listTop = list.scrollTop; body.scrollTop = body.scrollHeight; await wait(80);
        return {{width: innerWidth, height: innerHeight, readingSize: getComputedStyle(area).fontSize,
          listScrollable: list.scrollHeight > list.clientHeight && list.clientHeight > 0,
          inputScrollable: area.scrollHeight > area.clientHeight, inputMovesOnly,
          listMovesOnly: list.scrollTop > 0 && Math.abs(area.scrollTop - inputTop) < 2,
          bodyIndependent: Math.abs(list.scrollTop - listTop) < 2,
          bodyScrollable: body.scrollHeight > body.clientHeight, bodyScrollTop: body.scrollTop,
          footerStable: Math.abs(primary.getBoundingClientRect().top - footerTop) < 2,
          pageStable: document.scrollingElement.scrollTop === pageTop,
          sendVisible: visible(btn('发送', primary)), timelineWidth: $('.chat-timeline').getBoundingClientRect().width}};
    "##)).await;
    checks.check("[13] 800px window / supported maximum text sizes: long message and draft scroll independently; primary action stays visible", independent["width"].as_f64().is_some_and(|width| width <= 801.0)
        && independent["listScrollable"] == true && independent["inputScrollable"] == true && independent["inputMovesOnly"] == true
        && independent["readingSize"].as_str().is_some_and(|size| size == "20px")
        && independent["listMovesOnly"] == true && independent["bodyIndependent"] == true
        && independent["footerStable"] == true && independent["pageStable"] == true && independent["sendVisible"] == true, independent.to_string());

    fx.up.script(Script::Sse({
        let mut steps = vec![delta(&long_text), Step::Gate("layout-tail".into()), delta("\n\nLayout tail after scroll."), Step::Gate("layout-finish".into())];
        steps.extend(stop_steps());
        steps
    }));
    let streaming = js(app, &format!(r##"
        {helpers}
        await send('T13 streaming scroll');
        if (!await until(() => $(STREAM) && txt($(STREAM)).includes('Layout line 99'), 12000)) throw new Error('stream missing');
        const list = scroller(); list.scrollTop = 0; list.dispatchEvent(new Event('scroll')); await wait(100);
        const other = $('.workspace-session-tabs [role="tab"][aria-selected="true"]'); if (!other) throw new Error('workspace tab focus target missing'); other.focus({{preventScroll:true}});
        window.__layoutOther = other; window.__layoutScrollTop = list.scrollTop;
        return {{stopVisible: visible(btn('停止生成', $('.chat-composer-primary'))), jumpVisible: visible($('.chat-jump-latest button'))}};
    "##)).await;
    fx.up.release("layout-tail");
    let tail = js(app, &format!(r##"
        {helpers}
        const arrived = !!await until(() => $(STREAM) && txt($(STREAM)).includes('Layout tail after scroll.'));
        return {{arrived, topRetained: Math.abs(scroller().scrollTop - window.__layoutScrollTop) < 2,
          focusRetained: document.activeElement === window.__layoutOther}};
    "##)).await;
    checks.check("[13] streaming while reading history preserves scroll and outside focus; stop and back-to-bottom remain visible", streaming["stopVisible"] == true && streaming["jumpVisible"] == true
        && tail["arrived"] == true && tail["topRetained"] == true && tail["focusRetained"] == true, format!("{streaming} {tail}"));
    let jumped = js(app, &format!(r##"
        {helpers}
        const jump = $('.chat-jump-latest button'); if (!jump) return false; jump.click();
        return !!await until(() => scroller().scrollHeight - scroller().clientHeight - scroller().scrollTop < 48);
    "##)).await;
    checks.check("[13] explicit back-to-bottom reaches the current stream", jumped == true, jumped.to_string());
    // An unrelated focused control must remain focused when the stream terminates.
    js(app, "window.__layoutOther.focus({preventScroll:true}); return true;").await;
    fx.up.release("layout-finish");
    let focus = js(app, "await until(() => idle(), 12000); await wait(100); return {idle: idle(), retained: document.activeElement === window.__layoutOther};").await;
    checks.check("[13] completion does not steal focus from the workspace tab", focus["idle"] == true && focus["retained"] == true, focus.to_string());

    fx.up.script(Script::Sse(vec![delta("Layout stop focus"), Step::Hold]));
    let stopped = js(app, &format!(r##"
        {helpers}
        await send('T13 stop focus');
        if (!await until(() => $(STREAM) && txt($(STREAM)).includes('Layout stop focus'), 12000)) throw new Error('stop stream missing');
        const stop = await until(() => btn('停止生成')); if (!stop) throw new Error('stop missing');
        const shown = visible(stop); stop.focus({{preventScroll:true}}); stop.click();
        const restored = !!await until(() => idle() && document.activeElement === input(), 12000);
        return {{shown, restored}};
    "##)).await;
    checks.check("[13] visible stop action returns focus to its own composer", stopped["shown"] == true && stopped["restored"] == true, stopped.to_string());

    if let Some(note) = &fx.mcp_note {
        checks.check("[13] approval geometry uses the connected MCP fixture", false, note.clone());
    } else {
        fx.up.script(proposal(&"Layout approval argument\n".repeat(100)));
        let approval = js(app, &format!(r##"
            {helpers}
            const picker = btnHas('MCP 工具'); if (!picker) throw new Error('tool picker missing'); picker.click();
            const tool = await until(() => $$('[role=checkbox]').find(e => labelOf(e).includes('fixture-server / echo')));
            if (!tool) throw new Error('fixture tool missing'); if (tool.getAttribute('aria-checked') !== 'true') tool.click();
            kd(tool, 'Escape'); await until(() => !$('[role=dialog]')); await wait(100);
            await send('T13 long approval');
            const p = await until(() => $(PROPOSAL), 12000); if (!p) throw new Error('approval missing'); await wait(100);
            const args = $('pre', p), list = scroller(), top = list.scrollTop;
            const approve = btn('批准本次调用', p), reject = btn('拒绝并停止', p);
            const result = {{single: $$(PROPOSAL).length === 1, outsideTimelineScroll: !list.contains(p),
              approvalVisible: visible(approve) && visible(reject), stopVisible: visible(btn('停止生成', $('.chat-composer-primary'))),
              titleFocused: document.activeElement === $('#chat-proposal-heading'),
              argsScrollable: args.scrollHeight > args.clientHeight && args.clientHeight > 0}};
            args.scrollTop = args.scrollHeight; await wait(80);
            result.argsIndependent = args.scrollTop > 0 && Math.abs(list.scrollTop - top) < 2;
            const settings = $('a[aria-label="设置"]'); settings.focus(); settings.click();
            const overlay = await until(() => $('.workspace-settings-dialog'));
            const heading = $('#chat-proposal-heading'); heading.focus({{preventScroll:true}});
            result.coveredCannotFocus = !!overlay && document.activeElement !== heading && !!heading.closest('[inert]');
            const closeSettings = btn('关闭设置'); if (closeSettings) closeSettings.click();
            result.restoredWithoutSteal = !!await until(() => !$('.workspace-settings-dialog') && document.activeElement === settings);
            result.pendingRetained = !!$(PROPOSAL) && $$(PROPOSAL).length === 1;
            reject.focus({{preventScroll:true}}); reject.click(); await until(() => idle(), 12000);
            return result;
        "##)).await;
        checks.check("[13] approval scroll, fixed actions, settings coverage and return focus", approval["single"] == true && approval["outsideTimelineScroll"] == true
            && approval["approvalVisible"] == true && approval["stopVisible"] == true && approval["titleFocused"] == true && approval["argsScrollable"] == true && approval["argsIndependent"] == true
            && approval["coveredCannotFocus"] == true && approval["restoredWithoutSteal"] == true && approval["pendingRetained"] == true, approval.to_string());
    }
    // Stage 4: reuse the native key driver; JS only chooses a known starting control or fills fixture text.
    use super::ui_keys::{key, prepare, Key};
    checks.check("[13] native keyboard responder ready", prepare(app).await, "NSEvent through NSApplication");
    js(app, "const tab = $('.workspace-session-tabs [role=tab][aria-selected=true]'); tab.focus(); tab.dispatchEvent(new MouseEvent('contextmenu', {bubbles:true,clientX:100,clientY:100})); await until(() => $('[role=menu]')); return true;").await;
    key(app, Key::Down).await;
    key(app, Key::Enter).await;
    let editing = js(app, "return document.activeElement === $('input[aria-label=\"会话标题\"]');").await;
    key(app, Key::Escape).await;
    let rename_return = js(app, "return !$('[data-modal-kind][role=dialog]') && document.activeElement === $('.workspace-session-tabs [role=tab][aria-selected=true]');").await;
    checks.check("[13] native menu Enter edits title and Escape restores tab focus", editing == true && rename_return == true, format!("editing={editing} returned={rename_return}"));
    js(app, "const tab = $('.workspace-session-tabs [role=tab][aria-selected=true]'); tab.dispatchEvent(new MouseEvent('contextmenu', {bubbles:true,clientX:100,clientY:100})); await until(() => $('[role=menu]')); return true;").await;
    key(app, Key::Up).await;
    key(app, Key::Enter).await;
    let modal = js(app, "return !!$('[data-modal-kind][role=dialog]') && !!document.activeElement.closest('[data-modal-kind]');").await;
    key(app, Key::Escape).await;
    let delete_return = js(app, "return !$('[data-modal-kind][role=dialog]') && document.activeElement === $('.workspace-session-tabs [role=tab][aria-selected=true]');").await;
    checks.check("[13] native menu reaches delete Modal and Escape restores tab focus", modal == true && delete_return == true, format!("modal={modal} restored={delete_return}"));
    // The supported maximum text sizes remain active. Exercise narrow / high density geometry once.
    if let Some(window) = app.get_webview_window(MAIN) {
        window.set_min_size(Some(tauri::LogicalSize::new(380.0, 400.0))).ok();
        window.set_size(tauri::LogicalSize::new(520.0, 600.0)).ok();
    }
    let narrow = js(app, &format!(r##"
        {helpers}
        document.documentElement.dataset.density = '150';
        await wait(300);
        const area = input(); area.focus(); await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame);
        const r = area.getBoundingClientRect();
        return {{width: innerWidth, density: document.documentElement.dataset.density, inputVisible: visible(area), scrollWidth: document.documentElement.scrollWidth, inputSize: getComputedStyle(area).fontSize, inputHeight: r.height, rect:r.toJSON(), ancestors:[area.parentElement,area.closest('.chat-composer-body'),area.closest('.chat-composer'),area.closest('.chat-surface')].map(e=>({{class:e.className,rect:e.getBoundingClientRect().toJSON(),scroll:e.scrollTop,rows:getComputedStyle(e).gridTemplateRows}}))}};
    "##)).await;
    checks.check("[13] narrow native chat / 150% layout and enlarged text retains reachable input without horizontal overflow", narrow["density"] == "150" && narrow["inputSize"] == "20px" && narrow["width"].as_f64().is_some_and(|v| v <= 521.0) && narrow["inputVisible"] == true && narrow["scrollWidth"].as_f64().zip(narrow["width"].as_f64()).is_some_and(|(a,b)|a <= b + 1.0), narrow.to_string());
    js(app, "$('button[aria-label=\"会话目标\"]').focus(); return true;").await;
    key(app, Key::Enter).await;
    let popup = js(app, "return !!document.activeElement.closest('[role=dialog]');").await;
    key(app, Key::Escape).await;
    let popup_return = js(app, "return !$('[role=dialog]') && document.activeElement === $('button[aria-label=\"会话目标\"]');").await;
    checks.check("[13] narrow native target popup opens and closes with restored focus", popup == true && popup_return == true, format!("popup={popup} returned={popup_return}"));
    if fx.mcp_note.is_none() {
        fx.up.script(proposal(&"Layout narrow approval argument\n".repeat(100)));
        let approval = js(app, &format!(r##"
            {helpers}
            const picker = btnHas('MCP 工具'); picker.click();
            const tool = await until(() => $$('[role=checkbox]').find(e => labelOf(e).includes('fixture-server / echo')));
            if (!tool) throw new Error('fixture tool missing'); if (tool.getAttribute('aria-checked') !== 'true') tool.click();
            kd(tool, 'Escape'); await until(() => !$('[role=dialog]')); await wait(100);
            await send('T13 narrow approval');
            const proposal = await until(() => $(PROPOSAL), 12000);
            if (!proposal) throw new Error('narrow approval missing');
            const body = $('.chat-approval-body', proposal), args = $('pre', proposal);
            const approve = btn('批准本次调用', proposal), reject = btn('拒绝并停止', proposal);
            args.focus(); await wait(80);
            const result = {{bodyHeight: body.clientHeight, argsScrollable: args.scrollHeight > args.clientHeight,
                decisionsVisible: visible(approve) && visible(reject), stopVisible: visible(btn('停止生成', $('.chat-composer-primary')))}};
            args.scrollTop = args.scrollHeight; await wait(80);
            result.parametersReachable = args.scrollTop > 0 && body.clientHeight >= 32;
            reject.click(); await until(() => idle(), 12000);
            return result;
        "##)).await;
        checks.check("[13] narrow enlarged-text approval retains scrollable parameters and visible decision/stop actions", approval["parametersReachable"] == true && approval["argsScrollable"] == true && approval["decisionsVisible"] == true && approval["stopVisible"] == true, approval.to_string());
    }
    js(app, "const saved = window.__layoutRootStyle; if (saved === null) document.documentElement.removeAttribute('style'); else document.documentElement.setAttribute('style', saved); return true;").await;
}

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
    if std::env::var("DGOS_CHAT_SMOKE_ONLY").ok().as_deref() == Some("13") {
        block_layout(&app, &mut checks, &fx).await;
        finish(checks, &root);
    }
    if std::env::var("DGOS_CHAT_SMOKE_ONLY").ok().as_deref()==Some("12") {
        phase6::run(&app, &mut checks, &fx).await;
        finish(checks, &root);
    }
    if std::env::var("DGOS_CHAT_SMOKE_ONLY").ok().as_deref()==Some("11") {
        block_account_workspace(&app,&mut checks,&fx).await;
        finish(checks,&root);
    }
    if wanted(10) {
        block_workspace_task(&app, &mut checks, &fx).await;
        if std::env::var("DGOS_CHAT_SMOKE_ONLY").ok().as_deref() == Some("10") { finish(checks, &root); }
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
    let maintenance = world.maintenance.clone();
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
                .expect("providers").with_maintenance(maintenance.clone());
            providers
                .add_manual_model(
                    &provider.id,
                    LOCAL_USER,
                    "chat-model",
                    Some("text-generation"),
                )
                .await
                .expect("manual model");
            providers.add_manual_model(&provider.id, LOCAL_USER, "whisper-1", Some("speech-transcription")).await.expect("transcription model");
            let skills_dir = world.root.join("skills");
            std::fs::create_dir_all(&skills_dir).unwrap();
            let skills = dgos_skills::SkillsManager::new(db.clone(), skills_dir);
            if std::env::var("DGOS_CHAT_SMOKE_ONLY").ok().as_deref() == Some("12") {
                skills.register_bundled_catalog(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../skills")).await.expect("bundled catalog");
                for id in ["dgos.official.writing.summarize", "dgos.official.writing.rewrite"] { skills.install_skill(id, LOCAL_USER).await.expect("journey skill"); }
            }
            let mcp = Arc::new(McpManager::with_policy_and_vault(
                db.clone(),
                Arc::new(commands::DesktopMcpPolicy(
                    crate::session::SessionResolver::no_accounts(),
                )),
                Arc::new(RefusingMcpVault),
            ).with_maintenance(maintenance.clone()));
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
            let workbench = dgos_workbench::WorkbenchManager::new(db.clone()).with_maintenance(maintenance.clone());
            let chat = dgos_workbench::ChatManager::new(db.clone(), maintenance.clone());
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
    let capabilities = world.capabilities.clone();
    let root = world.root.clone();
    let scenario_world = Mutex::new(Some(world));
    let fixture = Fixture {
        up: upstream,
        db: probe,
        root,
        mcp_note,
    };
    let desktop = dgos_db::DesktopRepository::new(fixture.db.clone());
    let cli = dgos_db::CliRepository::new(fixture.db.clone());
    let chat_assets = dgos_workbench::ChatAssets::new(fixture.db.clone());
    let storage = Arc::new(dgos_application::StorageService::new(
        dgos_db::StorageRepository::new(fixture.db.clone()),
        dgos_db::AssetRepository::new(fixture.db.clone()),
        fixture.root.join("skills"),
        maintenance.clone(),
    ));
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(crate::session::SessionResolver::no_accounts())
        .manage(crate::capabilities::ProviderCallService::new(capabilities.clone()))
        .manage(crate::capabilities::ModuleCapabilityDispatchService::new(capabilities))
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
        .manage(desktop)
        .manage(cli)
        .manage(maintenance)
        .manage(chat_assets)
        .manage(crate::chat_media::Transcriptions::default())
        .manage(storage)
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
            // The smoke feature enables custom-protocol (embedded dist), even for a debug binary.
            // Keep the URL override confined to the explicit layout-only run and trusted dev origin.
            let main_url = if std::env::var("DGOS_CHAT_SMOKE_ONLY").ok().as_deref() == Some("13") {
                if let Ok(raw) = std::env::var("DGOS_CHAT_SMOKE_URL") {
                    let url = tauri::Url::parse(&raw)?;
                    if url.scheme() != "http" || !matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
                        || url.port() != Some(1420) || url.path() != "/workbench"
                        || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some()
                    {
                        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "chat layout smoke URL must be http://localhost:1420/workbench or http://127.0.0.1:1420/workbench").into());
                    }
                    WebviewUrl::External(url)
                } else {
                    WebviewUrl::App("workbench".into())
                }
            } else {
                WebviewUrl::App("workbench".into())
            };
            WebviewWindowBuilder::new(app, MAIN, main_url)
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
            get_account_state,
            commands::create_account,
            commands::sign_in,
            commands::sign_out,
            commands::open_agent_session,
            commands::send_agent_message,
            commands::get_agent_session,
            commands::list_agent_sessions,
            commands::close_agent_session,
            commands::module_list_provider_catalog,
            commands::module_call_provider,
            commands::cancel_module_call,
            commands::get_desktop_preferences,
            commands::set_desktop_preferences,
            commands::get_cli_preferences,
            commands::set_cli_preferences,
            commands::probe_cli,
            commands::probe_node_directory,
            commands::get_storage_overview,
            commands::preview_storage_cleanup,
            commands::execute_storage_cleanup,
            commands::preview_vacuum,
            commands::execute_vacuum,
            commands::import_chat_asset,
            commands::preview_chat_asset,
            commands::remove_chat_asset,
            commands::transcribe_chat_audio,
            commands::cancel_chat_transcription,
            commands::read_workspace_app_catalog,
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
