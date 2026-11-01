//! Real-host, real-WebView smoke for V1-FR-010 LAN collaboration (V1-E2E-10, macOS local).
//!
//! The host side is the shipped `dist/` on `/settings/lan` in a real WKWebView. Every LAN, account and
//! appearance command is the production handler; the LAN service is built by the production
//! `crate::lan::build` (real `SystemNetwork`, real socket2 listener on this machine's private IPv4,
//! `host_settings`, `DbAuditSink` into `lan_audit_events`); FR-008's `AccountService` gets the production
//! `LanSessionRevoker` as its remote port, as in `lib.rs`. The remote devices are separate incognito
//! WKWebView windows (own cookie store each, iPhone / Android User-Agents) that load the shipped remote SPA
//! (`dist-lan/`) over `http://<private-ip>:<port>/` from the real LAN server.
//!
//! A restart is a real one: the harness re-runs its own binary three times over one data directory
//! (first start, restart after switching off, restart after switching on). Each phase is a fresh process
//! with a fresh Tauri app, database connection and LAN service, and each ends with the production exit
//! step (`LanService::shutdown`).
//!
//! Replaced for the run (NOT real): the Provider is a loopback server that streams one scripted
//! OpenAI-compatible SSE reply; its credential vault is in memory; MCP is not started. The "devices" run
//! on the host machine itself, so their source address is the host's own private IPv4.
//!
//! Not proven: a second physical device (iOS Safari, Android Chrome), Windows / Linux, the application
//! firewall and macOS local-network permission of a signed `.app`, several private interfaces, real
//! network changes, a public-IP peer (AC11 is covered by crate tests), the packaged release build.
//!
//! Run: `npm run smoke:tauri:lan`.

use super::{eval, ipc, wait_load, Checks, LoadEvents, Pending, STEP_TIMEOUT};
use crate::commands;
use crate::session::{HostSessionPort, SessionResolver};
use dgos_application::{AccountPorts, AccountService};
use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::AppearanceRepository;
use dgos_lan::{LanService, LanServiceState, LanStatus, Network};
use dgos_mcp::McpManager;
use dgos_providers::test_support::MemoryCredentialVault;
use dgos_providers::ProviderManager;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, oneshot};

const MAIN: &str = "main";
const PHASE_ENV: &str = "DGOS_LAN_SMOKE_PHASE";
const DIR_ENV: &str = "DGOS_LAN_SMOKE_DIR";
const PHASE_WATCHDOG: Duration = Duration::from_secs(600);
const PHASES: [&str; 3] = ["first-start", "restart-off", "restart-on"];

const UA_IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const UA_ANDROID: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
const LABEL_IPHONE: &str = "iPhone · Safari";
const LABEL_ANDROID: &str = "Android · Chrome";
const PROVIDER_NAME: &str = "Smoke Loopback";
const MODEL_ID: &str = "chat-model";
const PROVIDER_KEY: &str = "sk-lan-smoke-not-a-real-key";
const PROMPT: &str = "LAN-SMOKE-PROMPT-6f1d";
const REPLY: &str = "LAN-SMOKE-REPLY-a93c";
const PASSWORD: &str = "lan-smoke-password";

// ---------------------------------------------------------------------------------------------
// Loopback upstream: every request gets one scripted OpenAI-compatible SSE reply.

async fn upstream(listener: tokio::net::TcpListener, hits: Arc<AtomicUsize>) {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let hits = hits.clone();
        tauri::async_runtime::spawn(async move {
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
            let head = String::from_utf8_lossy(&buffer[..header_end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while buffer.len() < header_end + length {
                let n = socket.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                buffer.extend_from_slice(&chunk[..n]);
            }
            hits.fetch_add(1, Ordering::SeqCst);
            let frames = [
                json!({"choices":[{"delta":{"content":REPLY},"finish_reason":null}]}),
                json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
                json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}),
            ];
            let mut body = String::from(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
            );
            for frame in frames {
                body.push_str(&format!("data: {frame}\n\n"));
            }
            body.push_str("data: [DONE]\n\n");
            let _ = socket.write_all(body.as_bytes()).await;
            let _ = socket.flush().await;
        });
    }
}

// ---------------------------------------------------------------------------------------------
// Harness state

struct Fixture {
    dir: PathBuf,
    phase: String,
    /// A second connection to the same file, for read-back only.
    probe: DatabaseConnection,
    upstream_hits: Arc<AtomicUsize>,
}

/// Replies of scripts evaluated in remote windows. A remote page has no Tauri IPC (remote origin), so the
/// script hands its result back through `document.title`, which the window's title observer reads.
#[derive(Default)]
struct TitleReplies(Mutex<HashMap<String, oneshot::Sender<String>>>);

const TITLE_TAG: &str = "__dgos_lan_smoke__:";
static NEXT_REMOTE: AtomicUsize = AtomicUsize::new(1);

fn on_title(window: tauri::WebviewWindow, title: String) {
    let Some(rest) = title.strip_prefix(TITLE_TAG) else {
        return;
    };
    let Some((id, payload)) = rest.split_once(':') else {
        return;
    };
    let sender = window
        .app_handle()
        .state::<TitleReplies>()
        .0
        .lock()
        .unwrap()
        .remove(id);
    if let Some(sender) = sender {
        let _ = sender.send(payload.to_owned());
    }
}

/// Page helpers shared by the host page and the remote SPA.
const PRE: &str = r#"
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (fn, ms = 10000) => { const t = Date.now(); while (Date.now() - t < ms) { try { const v = fn(); if (v) return v; } catch (e) {} await wait(50); } return false; };
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const txt = (el) => ((el && el.textContent) || '').trim();
const btn = (t, root = document) => $$('button', root).find((b) => txt(b) === t);
const setVal = (el, v) => { const p = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(p, 'value').set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); };
const dialog = () => $('[role=dialog], [role=alertdialog]');
const confirmIn = async (label) => { const d = await until(dialog); if (!d) throw new Error('no confirmation dialog'); const b = btn(label, d); if (!b) throw new Error('no ' + label + ' in dialog: ' + txt(d)); b.click(); await until(() => !dialog()); };
const field = (term) => { const dt = $$('dt').find((x) => txt(x) === term); return dt ? txt(dt.nextElementSibling) : null; };
const sw = () => $('[role=switch][aria-label="启用局域网协同"]') || $$('[role=switch]').find((x) => x.labels && x.labels[0] && txt(x.labels[0]) === '启用局域网协同');
const rows = () => $$('table[aria-label="已连接设备"] tbody tr').map((tr) => $$('td', tr).map(txt));
"#;

/// Host page script; the value or `{"__error": ...}`.
async fn host_js(app: &AppHandle, body: &str) -> Value {
    let reply = eval(app, MAIN, &format!("{PRE}\n{body}")).await;
    if reply["ok"] == true {
        reply["value"].clone()
    } else {
        json!({ "__error": reply["error"] })
    }
}

/// Remote page script; the value or `{"__error": ...}`.
async fn remote_js(app: &AppHandle, label: &str, body: &str) -> Value {
    let Some(window) = app.get_webview_window(label) else {
        return json!({ "__error": "no such window" });
    };
    let id = format!("t{}", NEXT_REMOTE.fetch_add(1, Ordering::SeqCst));
    let (sender, receiver) = oneshot::channel();
    app.state::<TitleReplies>()
        .0
        .lock()
        .unwrap()
        .insert(id.clone(), sender);
    let script = format!(
        "(async()=>{{let __r;try{{__r={{ok:true,value:await (async()=>{{{PRE}\n{body}}})()}};}}\
         catch(e){{__r={{ok:false,error:String((e&&e.message)||e)}};}}\
         const __t=document.title;document.title={tag:?}+{id:?}+':'+JSON.stringify(__r);\
         await new Promise(r=>setTimeout(r,50));document.title=__t;}})()",
        tag = TITLE_TAG,
    );
    if window.eval(script).is_err() {
        return json!({ "__error": "eval failed" });
    }
    let reply = match tokio::time::timeout(STEP_TIMEOUT, receiver).await {
        Ok(Ok(payload)) => serde_json::from_str::<Value>(&payload).unwrap_or(Value::Null),
        _ => {
            app.state::<TitleReplies>().0.lock().unwrap().remove(&id);
            json!({"ok": false, "error": "harness timeout"})
        }
    };
    if reply["ok"] == true {
        reply["value"].clone()
    } else {
        json!({ "__error": reply["error"] })
    }
}

/// Open a remote "device": an incognito WKWebView (own cookie store) with its own User-Agent.
async fn open_remote(
    app: &AppHandle,
    label: &str,
    url: &str,
    user_agent: &str,
) -> Result<(), String> {
    let events = app.state::<LoadEvents>().0.subscribe();
    let url = tauri::Url::parse(url).map_err(|e| e.to_string())?;
    let path = url.path().to_owned();
    WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
        .inner_size(420.0, 760.0)
        .visible(true)
        .focused(false)
        .incognito(true)
        .user_agent(user_agent)
        .on_document_title_changed(on_title)
        .build()
        .map_err(|e| e.to_string())?;
    if wait_load(events, label, &path).await {
        Ok(())
    } else {
        Err("remote page never finished loading".into())
    }
}

fn close_remote(app: &AppHandle, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.destroy();
    }
}

fn failed(value: &Value) -> bool {
    value.get("__error").is_some()
}

async fn reachable(addr: &str) -> bool {
    let Ok(addr) = addr.parse::<SocketAddr>() else {
        return false;
    };
    matches!(
        tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

async fn scalar(probe: &DatabaseConnection, sql: &str) -> Option<String> {
    probe
        .query_one(Statement::from_string(
            probe.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get_by_index::<String>(0).ok())
}

async fn strings(probe: &DatabaseConnection, sql: &str) -> Vec<String> {
    probe
        .query_all(Statement::from_string(
            probe.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| row.try_get_by_index::<String>(0).ok())
        .collect()
}

async fn setting(probe: &DatabaseConnection, key: &str) -> Option<String> {
    scalar(
        probe,
        &format!("SELECT setting_value FROM host_settings WHERE setting_key = '{key}'"),
    )
    .await
}

const AUDIT_ROWS: &str =
    "SELECT event || '|' || IFNULL(reason,'') || '|' || IFNULL(peer_ip,'') || '|' || \
     IFNULL(session_short_id,'') || '|' || IFNULL(user_id,'') || '|' || IFNULL(error_code,'') \
     FROM lan_audit_events ORDER BY rowid";

/// The audit writer is asynchronous; wait until `wanted` rows are visible or the step times out.
async fn audit_until(
    probe: &DatabaseConnection,
    wanted: impl Fn(&[String]) -> bool,
) -> Vec<String> {
    let mut rows = Vec::new();
    for _ in 0..100 {
        rows = strings(probe, AUDIT_ROWS).await;
        if wanted(&rows) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    rows
}

fn status_of(app: &AppHandle) -> Arc<LanService> {
    app.state::<Arc<LanService>>().inner().clone()
}

struct StartStatus(LanStatus);

fn address_file(dir: &Path) -> PathBuf {
    dir.join("last-address")
}

// ---------------------------------------------------------------------------------------------
// Scripts

/// Wait until the host page shows `state_text` and return what it displays.
const HOST_VIEW: &str = r#"
const want = __STATE__;
// First run with no account (D087): the welcome page offers 暂不创建，继续使用, as a user would click.
const skip = await until(() => btn('暂不创建，继续使用') || sw(), 8000);
if (skip && skip.tagName === 'BUTTON' && txt(skip) === '暂不创建，继续使用') skip.click();
const ok = await until(() => sw() && field('服务状态') === want && (want !== '运行中' || $('[data-testid=lan-pair-url]')), 12000);
return { ok: !!ok, state: field('服务状态'), address: field('访问地址'), port: field('当前端口'),
  preferred: field('首选端口'), count: field('已连接设备'), checked: sw() ? sw().getAttribute('aria-checked') : null,
  code: txt($('[data-testid=lan-code]')), pairUrl: txt($('[data-testid=lan-pair-url]')),
  noCode: document.body.textContent.includes('服务未运行或尚未登录，无法生成访问凭据'),
  body: ok ? '' : (location.pathname + ' :: ' + txt(document.body).slice(0, 400)) };
"#;

fn host_view(state: &str) -> String {
    HOST_VIEW.replace("__STATE__", &json!(state).to_string())
}

const PAIRING_PAGE: &str = r#"
const ok = await until(() => $('#lan-access-code') && btn('连接'));
return { ok: !!ok, heading: txt($('h1')), hash: location.hash, cookie: document.cookie, notice: txt($('[role=status]')) };
"#;

const ENTER_CODE: &str = r#"
const input = await until(() => { const i = $('#lan-access-code'); return i && !i.disabled && i; });
if (!input) throw new Error('code field missing');
setVal(input, __CODE__);
const go = await until(() => { const b = btn('连接'); return b && !b.disabled && b; });
if (!go) throw new Error('connect disabled');
go.click();
// Submitting clears the previous error and disables the field while the request runs; wait for it to settle.
await until(() => !txt($('#lan-access-code-error')) || btn('退出'), 3000);
await until(() => btn('退出') ? txt($('footer')).includes(' · ') : ($('#lan-access-code') && !$('#lan-access-code').disabled && txt($('#lan-access-code-error'))), 10000);
return { alert: txt($('#lan-access-code-error')), footer: btn('退出') ? txt($('footer')) : '', cookie: document.cookie, hash: location.hash };
"#;

fn enter_code(code: &str) -> String {
    ENTER_CODE.replace("__CODE__", &json!(code).to_string())
}

const CHAT_FOOTER: &str = r#"
const ok = await until(() => btn('退出') && txt($('footer')).includes(' · '), 10000);
return { ok: !!ok, footer: txt($('footer')), hash: location.hash, cookie: document.cookie, href: location.href };
"#;

const CHAT_ROUND: &str = r#"
const start = await until(() => { const b = btn('开始聊天'); return b && !b.disabled && b; }, 10000);
if (!start) throw new Error('开始聊天 never enabled: ' + txt(document.body).slice(0, 300));
const provider = txt($$('select')[0] && $$('select')[0].selectedOptions[0]);
start.click();
const ta = await until(() => $('textarea[aria-label="消息内容"]'), 10000);
if (!ta) throw new Error('composer missing');
setVal(ta, __PROMPT__);
const send = await until(() => { const b = btn('发送'); return b && !b.disabled && b; }, 10000);
if (!send) throw new Error('发送 never enabled (stream not open?)');
send.click();
const got = await until(() => txt($('ul[role=log]')).includes(__REPLY__), 15000);
return { ok: !!got, provider, log: txt($('ul[role=log]')).slice(0, 300) };
"#;

const SESSION_PROBE: &str = r#"
const r = await fetch('/api/v1/lan/session', { credentials: 'same-origin' });
let code = null; try { code = (await r.json()).code ?? null; } catch (e) {}
return { status: r.status, code };
"#;

const KICKED: &str = r#"
const ok = await until(() => $('#lan-access-code') && txt($('[role=status]')), 12000);
return { ok: !!ok, notice: txt($('[role=status]')) };
"#;

const REGENERATE: &str = r#"
const old = txt($('[data-testid=lan-code]'));
const b = await until(() => { const x = btn('重新生成'); return x && !x.disabled && x; });
if (!b) throw new Error('重新生成 disabled');
b.click();
const ok = await until(() => { const c = txt($('[data-testid=lan-code]')); return c && c !== old; });
return { ok: !!ok, old, code: txt($('[data-testid=lan-code]')), pairUrl: txt($('[data-testid=lan-pair-url]')) };
"#;

const DEVICES: &str = r#"
const ok = await until(() => rows().length === __N__ && field('已连接设备') === String(__N__), 12000);
return { ok: !!ok, rows: rows(), count: field('已连接设备') };
"#;

const DISCONNECT: &str = r#"
const label = __LABEL__;
const b = await until(() => $('button[aria-label="断开 ' + label + '"]'));
if (!b) throw new Error('no disconnect button for ' + label + ': ' + JSON.stringify(rows()));
b.click();
await confirmIn('断开');
const ok = await until(() => !$('button[aria-label="断开 ' + label + '"]') && rows().length === 1, 12000);
return { ok: !!ok, rows: rows() };
"#;

const SWITCH: &str = r#"
const target = __ON__;
const s = await until(() => { const x = sw(); return x && !x.disabled && x; });
if (!s) throw new Error('switch missing');
s.click();
if (!target) await confirmIn('确认关闭');
const want = target ? '运行中' : '已停止';
const ok = await until(() => field('服务状态') === want && sw().getAttribute('aria-checked') === String(target) && (!target || $('[data-testid=lan-pair-url]')), 12000);
return { ok: !!ok, state: field('服务状态'), checked: sw().getAttribute('aria-checked'), address: field('访问地址'),
  code: txt($('[data-testid=lan-code]')), pairUrl: txt($('[data-testid=lan-pair-url]')),
  devicesEmpty: document.body.textContent.includes('当前没有已连接的设备') };
"#;

fn switch(on: bool) -> String {
    SWITCH.replace("__ON__", if on { "true" } else { "false" })
}

// ---------------------------------------------------------------------------------------------
// Phases

async fn phase_first_start(app: &AppHandle, checks: &mut Checks) {
    let fx = app.state::<Fixture>();
    let probe = fx.probe.clone();
    let start = app.state::<StartStatus>().0.clone();

    // AC13 on a fresh database: no `lan.*` row, the switch is on by default and the service started.
    checks.check(
        "AC13 fresh install: no lan.enabled row, service started by on_app_start without user action",
        setting(&probe, "lan.enabled").await.is_none()
            && start.enabled
            && start.state == LanServiceState::Running
            && start.address.is_some(),
        format!("{start:?}"),
    );
    let view = host_js(app, &host_view("运行中")).await;
    let address = view["address"].as_str().unwrap_or("").to_owned();
    let host_addr = address.trim_start_matches("http://").to_owned();
    checks.check(
        "AC01/AC13: /settings/lan shows 运行中, switch on, the address and the actual port",
        view["ok"] == true
            && view["checked"] == "true"
            && Some(host_addr.clone()) == start.address
            && view["port"].as_str() == host_addr.rsplit(':').next(),
        &view,
    );
    let ip: Option<std::net::Ipv4Addr> = host_addr.split(':').next().and_then(|ip| ip.parse().ok());
    checks.check(
        "AC03: the displayed address is a private IPv4 of this machine",
        ip.is_some_and(|ip| ip.is_private())
            && ip.is_some_and(|ip| dgos_lan::SystemNetwork.candidate_ipv4().contains(&ip)),
        &host_addr,
    );
    checks.check(
        "AC01: the displayed address accepts TCP connections",
        reachable(&host_addr).await,
        &host_addr,
    );
    let code = view["code"].as_str().unwrap_or("").to_owned();
    let pair_url = view["pairUrl"].as_str().unwrap_or("").to_owned();
    checks.check(
        "access code and pair URL shown once the service runs (pair URL is http://<address>/#code=<code>)",
        !code.is_empty() && pair_url == format!("{address}/#code={code}"),
        &view,
    );

    // AC04: a browser opening the address sees the pairing page; pairing with the code reaches the chat view.
    let root = format!("{address}/");
    let opened = open_remote(app, "remote-a", &root, UA_IPHONE).await;
    let page = remote_js(app, "remote-a", PAIRING_PAGE).await;
    checks.check(
        "AC04: a browser at http://<address>/ shows the pairing page (连接到 DG-OS)",
        opened.is_ok() && page["ok"] == true && page["heading"] == "连接到 DG-OS",
        format!("{opened:?} {page}"),
    );
    let wrong = remote_js(app, "remote-a", &enter_code("00000000000000000000000000")).await;
    checks.check(
        "AC06: a wrong access code is refused on the pairing page (访问凭据无效或已使用)",
        wrong["alert"] == "访问凭据无效或已使用" && wrong["footer"] == "",
        &wrong,
    );
    let paired = remote_js(app, "remote-a", &enter_code(&code)).await;
    let footer_a = paired["footer"].as_str().unwrap_or("").to_owned();
    checks.check(
        "AC04/AC05: the correct code pairs the iPhone browser and opens the chat view",
        footer_a.starts_with(&format!("{LABEL_IPHONE} · ")) && paired["alert"] == "",
        &paired,
    );
    checks.check(
        "the session cookie is HttpOnly (document.cookie is empty after pairing)",
        paired["cookie"] == "",
        &paired,
    );
    let short_a = footer_a.rsplit(" · ").next().unwrap_or("").to_owned();

    // AC09/AC10 through the real browser: create a chat, send, the reply streams over the WebSocket.
    let before = fx.upstream_hits.load(Ordering::SeqCst);
    let chat = remote_js(
        app,
        "remote-a",
        &CHAT_ROUND
            .replace("__PROMPT__", &json!(PROMPT).to_string())
            .replace("__REPLY__", &json!(REPLY).to_string()),
    )
    .await;
    checks.check(
        "AC09/AC10 in a real browser: remote chat with the loopback Provider, reply delivered over WebSocket",
        !failed(&chat)
            && chat["ok"] == true
            && chat["provider"] == PROVIDER_NAME
            && fx.upstream_hits.load(Ordering::SeqCst) > before,
        &chat,
    );

    // Second device through the pair URL (code in the fragment, auto-submitted, fragment removed).
    let regenerated = host_js(app, REGENERATE).await;
    let code_b = regenerated["code"].as_str().unwrap_or("").to_owned();
    let pair_b = regenerated["pairUrl"].as_str().unwrap_or("").to_owned();
    checks.check(
        "重新生成 issues a new code after the first one was used",
        regenerated["ok"] == true && !code_b.is_empty() && code_b != code,
        &regenerated,
    );
    let opened_b = open_remote(app, "remote-b", &pair_b, UA_ANDROID).await;
    let footer = remote_js(app, "remote-b", CHAT_FOOTER).await;
    checks.check(
        "AC04: opening the pair URL pairs the Android browser and removes #code= from the address",
        opened_b.is_ok()
            && footer["ok"] == true
            && footer["footer"]
                .as_str()
                .is_some_and(|f| f.starts_with(&format!("{LABEL_ANDROID} · ")))
            && footer["hash"] == ""
            && !footer["href"].as_str().unwrap_or("#").contains("code="),
        format!("{opened_b:?} {footer}"),
    );

    // AC07: the host page lists both devices.
    let devices = host_js(app, &DEVICES.replace("__N__", "2")).await;
    let listed = devices["rows"].as_array().cloned().unwrap_or_default();
    let has = |label: &str| {
        listed
            .iter()
            .any(|row| row[0] == label && row[1].as_str() == ip.map(|ip| ip.to_string()).as_deref())
    };
    checks.check(
        "AC07: the host device list shows both devices with label and source IP, count 2",
        devices["ok"] == true && has(LABEL_IPHONE) && has(LABEL_ANDROID),
        &devices,
    );
    checks.check(
        "AC07: the iPhone row carries the short id the remote page shows",
        listed
            .iter()
            .any(|row| row[0] == LABEL_IPHONE && row[2] == short_a.as_str()),
        format!("{short_a} {devices}"),
    );

    // AC08: the host disconnects the Android device; its session stops working, the other one keeps going.
    let kicked = host_js(
        app,
        &DISCONNECT.replace("__LABEL__", &json!(LABEL_ANDROID).to_string()),
    )
    .await;
    checks.check(
        "AC08: 断开 (with confirmation) removes the Android row from the host list",
        kicked["ok"] == true,
        &kicked,
    );
    let back = remote_js(app, "remote-b", KICKED).await;
    checks.check(
        "AC08: the disconnected browser returns to pairing with 已被主机断开连接",
        back["ok"] == true && back["notice"] == "已被主机断开连接",
        &back,
    );
    let gone = remote_js(app, "remote-b", SESSION_PROBE).await;
    checks.check(
        "AC08: the disconnected browser's session is invalid (401, 1010006)",
        gone["status"] == 401 && gone["code"] == 1010006,
        &gone,
    );
    let alive = remote_js(app, "remote-a", SESSION_PROBE).await;
    checks.check(
        "AC08: the other device's session still works (200)",
        alive["status"] == 200,
        &alive,
    );

    // Audit: the events are there, and nothing secret or personal is.
    let audit = audit_until(&probe, |rows| {
        rows.iter()
            .any(|r| r.starts_with("session_revoked|host_disconnect|"))
            && rows
                .iter()
                .filter(|r| r.starts_with("pair_succeeded|"))
                .count()
                >= 2
    })
    .await;
    checks.check(
        "audit: pair_succeeded x2, pair_rejected invalid_code and session_revoked host_disconnect recorded",
        audit.iter().filter(|r| r.starts_with("pair_succeeded|")).count() >= 2
            && audit.iter().any(|r| r.starts_with("pair_rejected|invalid_code|"))
            && audit.iter().any(|r| r.starts_with("session_revoked|host_disconnect|")),
        format!("{audit:?}"),
    );
    let columns = strings(
        &probe,
        "SELECT name FROM pragma_table_info('lan_audit_events') ORDER BY cid",
    )
    .await;
    checks.check(
        "audit table has only the whitelisted columns",
        columns
            == [
                "id",
                "first_at",
                "last_at",
                "event",
                "reason",
                "peer_ip",
                "session_short_id",
                "user_id",
                "error_code",
                "count",
            ],
        format!("{columns:?}"),
    );
    let plain = |c: &str| c.replace('-', "");
    let secrets = [
        code.clone(),
        plain(&code),
        code_b.clone(),
        plain(&code_b),
        UA_IPHONE.to_owned(),
        UA_ANDROID.to_owned(),
        "Pixel 8".to_owned(),
        LABEL_IPHONE.to_owned(),
        PROVIDER_KEY.to_owned(),
        PROMPT.to_owned(),
        REPLY.to_owned(),
        "dgos_lan".to_owned(),
    ];
    let leaked: Vec<&String> = secrets
        .iter()
        .filter(|s| !s.is_empty() && audit.iter().any(|row| row.contains(s.as_str())))
        .collect();
    checks.check(
        "audit rows hold no access code, User-Agent, device label, Provider key, prompt, reply or cookie name",
        leaked.is_empty() && !audit.is_empty(),
        format!("{leaked:?}"),
    );

    // AC12: the switch off stops the service, the connected device ends, the port closes, the value persists.
    let off = host_js(app, &switch(false)).await;
    checks.check(
        "AC12: switching off (with confirmation) shows 已停止, no address, no code, no devices",
        off["ok"] == true
            && off["checked"] == "false"
            && off["address"].is_null()
            && off["code"] == ""
            && off["devicesEmpty"] == true,
        &off,
    );
    let ended = remote_js(app, "remote-a", KICKED).await;
    checks.check(
        "AC12: the connected iPhone browser returns to pairing with 主机已停止服务",
        ended["ok"] == true && ended["notice"] == "主机已停止服务",
        &ended,
    );
    checks.check(
        "AC12: the service no longer accepts TCP connections on the old address",
        !reachable(&host_addr).await,
        &host_addr,
    );
    checks.check(
        "AC12/AC14: lan.enabled is persisted as false",
        setting(&probe, "lan.enabled").await.as_deref() == Some("false"),
        format!("{:?}", setting(&probe, "lan.enabled").await),
    );
    let audit = audit_until(&probe, |rows| {
        rows.iter()
            .any(|r| r.starts_with("session_revoked|switch_off|"))
    })
    .await;
    checks.check(
        "audit: the switch-off revocation is recorded as switch_off",
        audit
            .iter()
            .any(|r| r.starts_with("session_revoked|switch_off|")),
        format!("{audit:?}"),
    );
    let _ = std::fs::write(address_file(&fx.dir), &host_addr);
    close_remote(app, "remote-a");
    close_remote(app, "remote-b");
}

async fn phase_restart_off(app: &AppHandle, checks: &mut Checks) {
    let fx = app.state::<Fixture>();
    let probe = fx.probe.clone();
    let start = app.state::<StartStatus>().0.clone();
    let old = std::fs::read_to_string(address_file(&fx.dir)).unwrap_or_default();

    // AC14: after a real restart the switch is still off and nothing listens.
    checks.check(
        "AC14: after restart with lan.enabled=false the service is not started",
        !start.enabled && start.state == LanServiceState::Stopped && start.address.is_none(),
        format!("{start:?}"),
    );
    let view = host_js(app, &host_view("已停止")).await;
    checks.check(
        "AC14: /settings/lan shows 已停止, the switch off, no address and no access code",
        view["ok"] == true
            && view["checked"] == "false"
            && view["address"].is_null()
            && view["code"] == ""
            && view["noCode"] == true,
        &view,
    );
    checks.check(
        "AC14: nothing accepts connections on the previous address",
        !old.is_empty() && !reachable(&old).await,
        &old,
    );

    // AC01: the user turns the switch on; the service starts and the address appears.
    let on = host_js(app, &switch(true)).await;
    let address = on["address"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("http://")
        .to_owned();
    checks.check(
        "AC01: switching on shows 运行中, the address and an access code",
        on["ok"] == true && on["checked"] == "true" && !address.is_empty() && on["code"] != "",
        &on,
    );
    checks.check(
        "AC01: the started service accepts TCP connections",
        reachable(&address).await,
        &address,
    );
    checks.check(
        "AC01: lan.enabled is persisted as true",
        setting(&probe, "lan.enabled").await.as_deref() == Some("true"),
        format!("{:?}", setting(&probe, "lan.enabled").await),
    );
    let _ = std::fs::write(address_file(&fx.dir), &address);
}

async fn phase_restart_on(app: &AppHandle, checks: &mut Checks) {
    let fx = app.state::<Fixture>();
    let probe = fx.probe.clone();
    let start = app.state::<StartStatus>().0.clone();

    // AC13 after a restart with the switch on.
    checks.check(
        "AC13: after restart with lan.enabled=true the service starts without user action",
        start.enabled && start.state == LanServiceState::Running && start.address.is_some(),
        format!("{start:?}"),
    );
    let view = host_js(app, &host_view("运行中")).await;
    checks.check(
        "AC13: /settings/lan shows 运行中 and the address on open, with no click",
        view["ok"] == true
            && view["checked"] == "true"
            && view["address"]
                .as_str()
                .map(|a| a.trim_start_matches("http://").to_owned())
                == start.address,
        &view,
    );

    // FR-008 x FR-010: the remote session belongs to the desktop user; sign-out and deletion revoke it.
    let created = ipc(
        app,
        MAIN,
        "create_account",
        json!({"input": {"username": "lan-smoke", "password": PASSWORD}}),
    )
    .await;
    checks.check(
        "first account is created and signed in (adopts default-user)",
        created.success() && created.data()["mode"] == "signed_in",
        created.describe(),
    );
    let issued = ipc(app, MAIN, "issue_lan_access_code", json!({})).await;
    let pair_c = issued.data()["pair_url"].as_str().unwrap_or("").to_owned();
    let opened = open_remote(app, "remote-c", &pair_c, UA_IPHONE).await;
    let paired = remote_js(app, "remote-c", CHAT_FOOTER).await;
    checks.check(
        "a signed-in user's code pairs a browser",
        issued.success() && opened.is_ok() && paired["ok"] == true,
        format!("{} {opened:?} {paired}", issued.describe()),
    );
    let out = ipc(app, MAIN, "sign_out", json!({})).await;
    let back = remote_js(app, "remote-c", KICKED).await;
    let gone = remote_js(app, "remote-c", SESSION_PROBE).await;
    checks.check(
        "sign-out revokes the remote session: the browser returns to pairing, its session is 401 1010006",
        out.success() && back["ok"] == true && gone["status"] == 401 && gone["code"] == 1010006,
        format!("{} {back} {gone}", out.describe()),
    );
    checks.check(
        "after sign-out the LAN service has no sessions",
        status_of(app).list_devices().is_empty(),
        format!("{:?}", status_of(app).list_devices()),
    );
    let refused = ipc(app, MAIN, "issue_lan_access_code", json!({})).await;
    checks.check(
        "signed out: the host LAN commands are closed (1008007)",
        !refused.success() && refused.code() == Some(1008007),
        refused.describe(),
    );
    let audit = audit_until(&probe, |rows| {
        rows.iter()
            .any(|r| r.starts_with("session_revoked|issuer_ended|"))
    })
    .await;
    checks.check(
        "audit: the sign-out revocation is recorded as issuer_ended",
        audit
            .iter()
            .any(|r| r.starts_with("session_revoked|issuer_ended|")),
        format!("{audit:?}"),
    );

    let signed_in = ipc(
        app,
        MAIN,
        "sign_in",
        json!({"input": {"username": "lan-smoke", "password": PASSWORD}}),
    )
    .await;
    let issued = ipc(app, MAIN, "issue_lan_access_code", json!({})).await;
    let pair_d = issued.data()["pair_url"].as_str().unwrap_or("").to_owned();
    close_remote(app, "remote-c");
    let opened = open_remote(app, "remote-d", &pair_d, UA_ANDROID).await;
    let paired = remote_js(app, "remote-d", CHAT_FOOTER).await;
    checks.check(
        "after signing in again a new code pairs a browser",
        signed_in.success() && issued.success() && opened.is_ok() && paired["ok"] == true,
        format!(
            "{} {} {opened:?} {paired}",
            signed_in.describe(),
            issued.describe()
        ),
    );
    let deleted = ipc(
        app,
        MAIN,
        "delete_account",
        json!({"input": {"password": PASSWORD}}),
    )
    .await;
    let back = remote_js(app, "remote-d", KICKED).await;
    let gone = remote_js(app, "remote-d", SESSION_PROBE).await;
    checks.check(
        "account deletion revokes the remote session: pairing page, session 401 1010006",
        deleted.success() && back["ok"] == true && gone["status"] == 401 && gone["code"] == 1010006,
        format!("{} {back} {gone}", deleted.describe()),
    );
    // The revocation's audit row is written asynchronously; give it time, then the user's rows must be gone.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let user_rows = strings(
        &probe,
        "SELECT id FROM lan_audit_events WHERE user_id = 'default-user'",
    )
    .await;
    checks.check(
        "account deletion leaves no lan_audit_events rows of the deleted user",
        user_rows.is_empty(),
        format!("{} rows: {user_rows:?}", user_rows.len()),
    );
    close_remote(app, "remote-d");
}

async fn scenario(app: AppHandle, ready: bool) {
    let mut checks = Checks::default();
    let phase = app.state::<Fixture>().phase.clone();
    if !ready {
        checks.check(
            "main window loaded /settings/lan",
            false,
            "page never loaded",
        );
    } else {
        match phase.as_str() {
            "first-start" => phase_first_start(&app, &mut checks).await,
            "restart-off" => phase_restart_off(&app, &mut checks).await,
            "restart-on" => phase_restart_on(&app, &mut checks).await,
            other => checks.check("known phase", false, other),
        }
    }
    // The production exit step (`RunEvent::ExitRequested` in lib.rs): the switch keeps its value.
    let before = setting(&app.state::<Fixture>().probe, "lan.enabled").await;
    status_of(&app).shutdown().await;
    let after = setting(&app.state::<Fixture>().probe, "lan.enabled").await;
    checks.check(
        "exit stops the service and keeps lan.enabled unchanged",
        before == after && status_of(&app).status().await.state == LanServiceState::Stopped,
        format!("{before:?} -> {after:?}"),
    );
    println!(
        "SMOKE RESULT: {} passed, {} failed",
        checks.passed,
        checks.failed.len()
    );
    for line in &checks.failed {
        println!("  FAILED {line}");
    }
    let _ = std::io::stdout().flush();
    std::process::exit(if checks.failed.is_empty() { 0 } else { 1 });
}

// ---------------------------------------------------------------------------------------------
// Entry points

/// Orchestrator: one data directory, three real processes (start, restart, restart).
pub fn run_lan() {
    if let Ok(phase) = std::env::var(PHASE_ENV) {
        run_phase(phase);
    }
    let dir = std::env::temp_dir().join(format!(
        "dgos-lan-smoke-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let exe = std::env::current_exe().expect("current exe");
    let (mut passed, mut failed) = (0usize, Vec::new());
    for phase in PHASES {
        println!("== phase {phase} (new process)");
        let child = std::process::Command::new(&exe)
            .env(PHASE_ENV, phase)
            .env(DIR_ENV, &dir)
            .stdout(std::process::Stdio::piped())
            .spawn();
        let Ok(mut child) = child else {
            failed.push(format!("{phase}: could not start the phase process"));
            break;
        };
        let mut result = None;
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                println!("  [{phase}] {line}");
                if let Some(rest) = line.strip_prefix("SMOKE RESULT: ") {
                    result = Some(rest.to_owned());
                }
                if let Some(rest) = line.strip_prefix("FAIL ") {
                    failed.push(format!("{phase}: {rest}"));
                }
                if line.starts_with("PASS ") {
                    passed += 1;
                }
            }
        }
        let status = child.wait();
        if !matches!(&status, Ok(s) if s.success()) {
            if result.is_none() {
                failed.push(format!(
                    "{phase}: process ended without a result ({status:?})"
                ));
            }
            if !failed.iter().any(|f| f.starts_with(phase)) {
                failed.push(format!("{phase}: exit status {status:?}"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("SMOKE RESULT: {passed} passed, {} failed", failed.len());
    for line in &failed {
        println!("  FAILED {line}");
    }
    let _ = std::io::stdout().flush();
    std::process::exit(if failed.is_empty() { 0 } else { 1 });
}

fn run_phase(phase: String) -> ! {
    std::thread::spawn(|| {
        std::thread::sleep(PHASE_WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });
    let dir = PathBuf::from(std::env::var(DIR_ENV).expect("phase directory"));
    let url = format!("sqlite://{}?mode=rwc", dir.join("dgos.db").display());
    let upstream_hits = Arc::new(AtomicUsize::new(0));
    let first = phase == "first-start";
    let (db, probe, providers, chat, skills) = tauri::async_runtime::block_on(async {
        let db = dgos_db::establish_connection(&url).await.expect("db");
        dgos_db::migrations::Migrator::up(&db, None)
            .await
            .expect("migrations");
        let probe = dgos_db::establish_connection(&url).await.expect("probe");
        let vault = Arc::new(MemoryCredentialVault::default());
        // The loopback Provider is seeded once, by the first process. Later phases only pair.
        let seeded = if first {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("upstream");
            let addr = listener.local_addr().expect("upstream addr");
            tauri::async_runtime::spawn(upstream(listener, upstream_hits.clone()));
            let provider = dgos_providers::test_support::seed_provider(
                db.clone(),
                &vault,
                dgos_types::ProviderConfig {
                    name: PROVIDER_NAME.into(),
                    provider_type: "openai_compatible".into(),
                    base_url: format!("http://{addr}/v1"),
                    api_key: PROVIDER_KEY.into(),
                    enabled: true,
                    config: None,
                },
                crate::host::LOCAL_USER,
            )
            .await
            .expect("seed provider");
            Some(provider.id)
        } else {
            None
        };
        let providers = ProviderManager::with_vault(&url, vault)
            .await
            .expect("providers");
        if let Some(id) = seeded {
            providers
                .add_manual_model(
                    &id,
                    crate::host::LOCAL_USER,
                    MODEL_ID,
                    Some("text-generation"),
                )
                .await
                .expect("model");
        }
        let chat = dgos_workbench::ChatManager::new(db.clone());
        chat.recover_interrupted().await.expect("chat recovery");
        let skills_dir = dir.join("skills");
        std::fs::create_dir_all(&skills_dir).expect("skills dir");
        let skills = dgos_skills::SkillsManager::new(db.clone(), skills_dir);
        (db, probe, providers, chat, skills)
    });

    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let resolver = SessionResolver::starting();

    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(TitleReplies::default())
        .manage(LoadEvents(load_sender))
        .manage(resolver.clone())
        .manage(Fixture {
            dir,
            phase,
            probe,
            upstream_hits,
        })
        .manage(providers)
        .manage(commands::SkillsState {
            manager: Arc::new(tokio::sync::RwLock::new(skills)),
        })
        .manage(chat)
        .manage(AppearanceRepository::new(db.clone()))
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview
                    .app_handle()
                    .state::<LoadEvents>()
                    .0
                    .send((webview.label().to_owned(), payload.url().path().to_owned()));
            }
        })
        .setup(move |app| {
            // The production order (lib.rs): LAN service, account recovery with the LAN revoker, then
            // `on_app_start` reads the switch and starts the service when it is on.
            let handle = app.handle().clone();
            let start = tauri::async_runtime::block_on(async {
                let lan = crate::lan::build(&handle, &db, resolver.clone());
                handle.manage(lan.clone());
                let mcp = Arc::new(McpManager::new(db.clone()));
                let mut ports = AccountPorts::new(
                    &db,
                    Arc::new(HostSessionPort::new(resolver.clone(), mcp, handle.clone())),
                );
                ports.remote = Arc::new(crate::lan::LanSessionRevoker(lan.clone()));
                let accounts = AccountService::new(db.clone(), ports);
                accounts.recover().await.expect("account recovery");
                handle.manage(accounts);
                lan.on_app_start().await
            });
            app.manage(StartStatus(start));
            let events = app.state::<LoadEvents>().0.subscribe();
            WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("settings/lan".into()))
                .inner_size(900.0, 760.0)
                .visible(true)
                .focused(false)
                .build()?;
            tauri::async_runtime::spawn(async move {
                let ready = wait_load(events, MAIN, "/settings/lan").await;
                scenario(handle, ready).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_lan_status,
            commands::set_lan_enabled,
            commands::set_lan_port,
            commands::issue_lan_access_code,
            commands::list_lan_devices,
            commands::disconnect_lan_device,
            commands::disconnect_all_lan_devices,
            commands::list_lan_audit_events,
            commands::get_account_state,
            commands::create_account,
            commands::sign_in,
            commands::sign_out,
            commands::change_password,
            commands::delete_account,
            commands::get_appearance_preferences,
            commands::set_appearance_preferences,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
