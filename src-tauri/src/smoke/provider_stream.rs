//! Real-WebView smoke for the provider stream commands (FR-001 AC33).
//!
//! Registers the production `call_provider_stream`, `cancel_provider_stream` and `list_usage_logs`
//! handlers over a migrated SQLite database, the real `ProviderManager`, `ProviderCallService` and
//! `ProviderStreams`, and drives them from the shipped `dist/` in a real WebView. The page builds a
//! real IPC callback (`transformCallback` plus the `__CHANNEL__:<id>` form that the
//! `@tauri-apps/api` `Channel` serializes to), so the events it records are what Tauri delivered.
//!
//! Replaced for the run: the Provider is a loopback fake that speaks OpenAI-style SSE (not a real
//! vendor) and the credential vault is in memory. What it does not prove: a real vendor, WebView2
//! or WebKitGTK, or the window-destroyed hook (destroying `main` ends the app; the page-reload
//! hook, which shares `cancel_owner`, is exercised instead).

use super::{eval, goto, ipc, wait_load, wait_until, Checks, LoadEvents, Pending, Reply, WATCHDOG};
use crate::commands;
use crate::host::LOCAL_USER;
use crate::provider_streams::ProviderStreams;
use dgos_application::{HostCapabilityService, ProviderCallService};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::ModulePermissionRepository;
use dgos_modules::ModulePublisher;
use dgos_providers::test_support::{seed_provider, MemoryCredentialVault};
use dgos_providers::ProviderManager;
use dgos_types::ProviderConfig;
use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

const DELTA: &str = r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":null}]}"#;
const FINISH: &str = r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#;
const USAGE: &str =
    r#"{"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;

#[derive(Default)]
struct Upstream {
    opened: AtomicUsize,
    closed: AtomicUsize,
}

struct Fixture {
    dir: std::path::PathBuf,
    provider_id: String,
    upstream: Arc<Upstream>,
}

fn sse(events: &[&str]) -> String {
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

/// Reads one HTTP request and returns the `model` of its JSON body.
async fn read_model(socket: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let Some(split) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buffer[..split]).to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let body = &buffer[split + 4..];
        if body.len() >= length {
            let value: Value = serde_json::from_slice(&body[..length]).ok()?;
            return value["model"].as_str().map(str::to_owned);
        }
    }
}
/// A model name starting with `slow` gets one delta and then silence until the client goes away;
/// anything else gets a complete stream. `closed` counts connections the client ended.
async fn serve(listener: TcpListener, upstream: Arc<Upstream>) {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let upstream = upstream.clone();
        tauri::async_runtime::spawn(async move {
            let Some(model) = read_model(&mut socket).await else {
                return;
            };
            upstream.opened.fetch_add(1, Ordering::SeqCst);
            let head =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            if model.starts_with("slow") {
                let first = format!("{head}{}", sse(&[DELTA]));
                let mut gone = socket.write_all(first.as_bytes()).await.is_err();
                let mut sink = [0u8; 64];
                while !gone {
                    let read = socket.read(&mut sink);
                    match tokio::time::timeout(Duration::from_secs(60), read).await {
                        Ok(Ok(0) | Err(_)) => gone = true,
                        Ok(Ok(_)) => {}
                        Err(_) => return,
                    }
                }
                upstream.closed.fetch_add(1, Ordering::SeqCst);
            } else {
                let body = format!("{head}{}", sse(&[DELTA, FINISH, USAGE, "[DONE]"]));
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
    }
}
/// Builds a real IPC callback that records every raw message under `window.__streams[name]`.
/// Its `toJSON` is the form `@tauri-apps/api`'s `Channel` serializes to.
fn channel_prelude(name: &str) -> String {
    format!(
        "window.__streams = window.__streams || {{}}; window.__streams[{name:?}] = []; \
         const __id = window.__TAURI_INTERNALS__.transformCallback((raw) => {{ window.__streams[{name:?}].push(raw); }}); \
         const __channel = {{ id: __id, toJSON() {{ return '__CHANNEL__:' + __id; }} }}; "
    )
}

async fn start(app: &AppHandle, label: &str, name: &str, provider: &str, model: &str) -> Reply {
    let body = format!(
        "{} return await window.__TAURI_INTERNALS__.invoke('call_provider_stream', \
         {{ providerId: {provider:?}, modelId: {model:?}, onEvent: __channel, \
         request: {{ messages: [{{ role: 'user', content: 'hello' }}], max_tokens: 16, stream: true }} }});",
        channel_prelude(name)
    );
    Reply(eval(app, label, &body).await)
}

async fn cancel(app: &AppHandle, label: &str, stream_id: &str) -> Reply {
    let args = json!({ "streamId": stream_id });
    ipc(app, label, "cancel_provider_stream", args).await
}

/// The messages (not the end marker) the page's callback received, in channel order.
async fn messages(app: &AppHandle, label: &str, name: &str) -> Vec<Value> {
    let body = format!("return (window.__streams && window.__streams[{name:?}]) || [];");
    let reply = eval(app, label, &body).await;
    let mut items = reply["value"].as_array().cloned().unwrap_or_default();
    items.sort_by_key(|item| item["index"].as_u64().unwrap_or(u64::MAX));
    items
        .into_iter()
        .filter(|item| item.get("message").is_some())
        .collect()
}
fn kinds(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|item| item["message"]["kind"].as_str().unwrap_or("?").to_owned())
        .collect()
}

fn terminals(items: &[Value]) -> usize {
    items
        .iter()
        .filter(|item| matches!(item["message"]["kind"].as_str(), Some("done" | "error")))
        .count()
}

fn filled(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.is_empty())
}

async fn wait_messages(
    app: &AppHandle,
    label: &str,
    name: &str,
    ready: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let mut last = Vec::new();
    for _ in 0..200 {
        last = messages(app, label, name).await;
        if ready(&last) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last
}

async fn fetch_usage(app: &AppHandle, model: &str) -> Vec<Value> {
    let reply = ipc(app, "main", "list_usage_logs", json!({ "limit": 50 })).await;
    let rows = reply.data().as_array().cloned().unwrap_or_default();
    rows.into_iter()
        .filter(|row| row["model_id"] == model)
        .collect()
}

/// Usage rows for `model`: waits for the first, then a moment more to catch a duplicate.
async fn usage_rows(app: &AppHandle, model: &str) -> Vec<Value> {
    for _ in 0..100 {
        if !fetch_usage(app, model).await.is_empty() {
            tokio::time::sleep(Duration::from_millis(300)).await;
            return fetch_usage(app, model).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Vec::new()
}

fn finish(checks: Checks, dir: &std::path::Path) -> ! {
    let _ = std::fs::remove_dir_all(dir);
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
async fn scenario(app: AppHandle, ready: bool) {
    let mut checks = Checks::default();
    let fixture = app.state::<Fixture>();
    let provider = fixture.provider_id.clone();
    let upstream = fixture.upstream.clone();
    let dir = fixture.dir.clone();
    if !ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks, &dir);
    }

    // A complete stream: ordered events over the real Channel, one terminal event, one usage row.
    let started = start(&app, "main", "fast", &provider, "fast").await;
    let stream_id = started.data()["stream_id"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    checks.check(
        "call_provider_stream answers with a host-issued stream id",
        started.success() && super::looks_like_uuid(&stream_id),
        started.describe(),
    );
    let events = wait_messages(&app, "main", "fast", |m| terminals(m) >= 1).await;
    let seen = kinds(&events);
    checks.check(
        "events arrive in order: a delta first, exactly one done last",
        seen.first().map(String::as_str) == Some("delta")
            && seen.last().map(String::as_str) == Some("done")
            && terminals(&events) == 1,
        format!("{seen:?}"),
    );
    let text = events
        .first()
        .map_or(Value::Null, |e| e["message"]["text"].clone());
    let done = events.last().map_or(Value::Null, |e| e["message"].clone());
    checks.check(
        "the delta text, finish reason and usage are the upstream's",
        text == "hi"
            && done["finish_reason"] == "stop"
            && done["usage"]["prompt_tokens"] == 7
            && done["usage"]["completion_tokens"] == 3
            && done["usage"]["total_tokens"] == 10,
        format!("{text} {done}"),
    );
    checks.check(
        "the Channel's message indices are contiguous from 0",
        events.iter().enumerate().all(|(i, e)| e["index"] == i),
        format!("{events:?}"),
    );
    let rows = usage_rows(&app, "fast").await;
    let row = rows.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "exactly one usage record: success, the upstream's tokens, caller and request id set",
        rows.len() == 1
            && row["success"] == true
            && row["request_tokens"] == 7
            && row["response_tokens"] == 3
            && filled(&row["caller"])
            && filled(&row["request_id"]),
        format!("{rows:?}"),
    );
    let finished = cancel(&app, "main", &stream_id).await;
    checks.check(
        "cancelling a finished stream answers false",
        finished.success() && finished.data() == &json!(false),
        finished.describe(),
    );

    let events_rx = app.state::<LoadEvents>().0.subscribe();
    let other = WebviewWindowBuilder::new(&app, "other", WebviewUrl::App("settings".into()))
        .visible(false)
        .build();
    let other_ready = other.is_ok() && wait_load(events_rx, "other", "/settings").await;
    checks.check(
        "a second window loaded /settings",
        other_ready,
        "it did not load",
    );

    scenario_cancel(&app, &mut checks, &provider, &upstream, other_ready).await;
    scenario_reload(&app, &mut checks, &provider, &upstream).await;
    finish(checks, &dir);
}
async fn scenario_cancel(
    app: &AppHandle,
    checks: &mut Checks,
    provider: &str,
    upstream: &Upstream,
    other_ready: bool,
) {
    let opened = upstream.opened.load(Ordering::SeqCst);
    let closed = upstream.closed.load(Ordering::SeqCst);
    let started = start(app, "main", "slow-cancel", provider, "slow-cancel").await;
    let stream_id = started.data()["stream_id"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let first = wait_messages(app, "main", "slow-cancel", |m| !m.is_empty()).await;
    checks.check(
        "a slow stream delivers its first delta while the upstream stays open",
        started.success()
            && kinds(&first) == ["delta"]
            && upstream.opened.load(Ordering::SeqCst) == opened + 1
            && upstream.closed.load(Ordering::SeqCst) == closed,
        format!("{} {first:?}", started.describe()),
    );

    if other_ready {
        let denied_start = start(app, "other", "denied", provider, "fast").await;
        let denied_cancel = cancel(app, "other", &stream_id).await;
        let alive = messages(app, "main", "slow-cancel").await;
        checks.check(
            "a non-main window can neither start nor cancel a stream (1004004)",
            !denied_start.success()
                && denied_start.code() == Some(1004004)
                && !denied_cancel.success()
                && denied_cancel.code() == Some(1004004),
            format!("{} / {}", denied_start.describe(), denied_cancel.describe()),
        );
        checks.check(
            "the denied calls reached no upstream and left the stream running",
            upstream.opened.load(Ordering::SeqCst) == opened + 1
                && upstream.closed.load(Ordering::SeqCst) == closed
                && terminals(&alive) == 0,
            format!("{alive:?}"),
        );
    }
    let cancelled = cancel(app, "main", &stream_id).await;
    let ended = wait_messages(app, "main", "slow-cancel", |m| terminals(m) >= 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let settled = messages(app, "main", "slow-cancel").await;
    let last = ended.last().map_or(Value::Null, |e| e["message"].clone());
    checks.check(
        "cancelling from the owning window answers true and ends the stream with one stream_interrupted",
        cancelled.success()
            && cancelled.data() == &json!(true)
            && kinds(&settled) == ["delta", "error"]
            && last["code"] == 1001007,
        format!("{} {settled:?}", cancelled.describe()),
    );
    let seen_close = wait_until(
        || upstream.closed.load(Ordering::SeqCst) == closed + 1,
        "the upstream to see the close",
    )
    .await;
    checks.check(
        "cancelling closes the upstream connection",
        seen_close.is_ok(),
        format!("{seen_close:?}"),
    );
    let rows = usage_rows(app, "slow-cancel").await;
    let row = rows.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "one failed usage record with the interrupt code",
        rows.len() == 1
            && row["success"] == false
            && row["error_code"].to_string().trim_matches('"') == "1001007",
        format!("{rows:?}"),
    );
    let again = cancel(app, "main", &stream_id).await;
    let unknown = cancel(app, "main", "00000000-0000-4000-8000-000000000000").await;
    checks.check(
        "cancelling it again, or an unknown id, answers false",
        again.data() == &json!(false) && unknown.data() == &json!(false),
        format!("{} / {}", again.describe(), unknown.describe()),
    );
}

/// A page reload drops the page's Channel; the host must cancel the streams that page started.
async fn scenario_reload(
    app: &AppHandle,
    checks: &mut Checks,
    provider: &str,
    upstream: &Upstream,
) {
    let closed = upstream.closed.load(Ordering::SeqCst);
    let started = start(app, "main", "slow-reload", provider, "slow-reload").await;
    let old_id = started.data()["stream_id"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    wait_messages(app, "main", "slow-reload", |m| !m.is_empty()).await;
    let reloaded = goto(app, "main", "/settings").await;
    let ended = wait_until(
        || upstream.closed.load(Ordering::SeqCst) == closed + 1,
        "the upstream to see the reload",
    )
    .await;
    checks.check(
        "reloading the page cancels its stream and the upstream connection closes",
        started.success() && reloaded && ended.is_ok(),
        format!("{} reloaded={reloaded} {ended:?}", started.describe()),
    );
    let rows = usage_rows(app, "slow-reload").await;
    let row = rows.first().cloned().unwrap_or(Value::Null);
    checks.check(
        "the reloaded stream settles as one failed usage record with the interrupt code",
        rows.len() == 1
            && row["success"] == false
            && row["error_code"].to_string().trim_matches('"') == "1001007",
        format!("{rows:?}"),
    );
    let stale = cancel(app, "main", &old_id).await;
    checks.check(
        "the old stream id answers false from the new page",
        stale.success() && stale.data() == &json!(false),
        stale.describe(),
    );
    let after = start(app, "main", "after", provider, "fast-after").await;
    let events = wait_messages(app, "main", "after", |m| terminals(m) >= 1).await;
    checks.check(
        "after the reload the page streams again",
        after.success() && kinds(&events).last().map(String::as_str) == Some("done"),
        format!("{} {events:?}", after.describe()),
    );
}

pub fn run_provider_stream() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });

    let dir = std::env::temp_dir().join(format!(
        "dgos-provider-stream-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(dir.join("modules")).expect("temp dir");
    let url = format!("sqlite://{}?mode=rwc", dir.join("dgos.db").display());
    let upstream = Arc::new(Upstream::default());
    let (providers, provider_id, service) = tauri::async_runtime::block_on(async {
        let db = dgos_db::establish_connection(&url)
            .await
            .expect("connection");
        dgos_db::migrations::Migrator::up(&db, None)
            .await
            .expect("migrations");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        tauri::async_runtime::spawn(serve(listener, upstream.clone()));
        let vault = Arc::new(MemoryCredentialVault::default());
        let config = ProviderConfig {
            name: "Synthetic stream smoke".into(),
            provider_type: "openai_compatible".into(),
            base_url: format!("http://{address}"),
            api_key: "synthetic-stream-smoke-key".into(),
            enabled: true,
            config: None,
        };
        let provider = seed_provider(db.clone(), &vault, config, LOCAL_USER)
            .await
            .expect("provider");
        let providers = ProviderManager::with_vault(&url, vault)
            .await
            .expect("manager");
        let publisher = Arc::new(ModulePublisher::new(dir.join("modules"), db.clone()));
        let permissions = Arc::new(ModulePermissionRepository::new(db));
        let capabilities = HostCapabilityService::with_publisher(permissions, publisher);
        (
            providers,
            provider.id,
            ProviderCallService::new(capabilities),
        )
    });
    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();

    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(LoadEvents(load_sender))
        .manage(ProviderStreams::default())
        .manage(providers)
        .manage(service)
        .manage(Fixture {
            dir,
            provider_id,
            upstream,
        })
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview
                    .app_handle()
                    .state::<LoadEvents>()
                    .0
                    .send((webview.label().to_owned(), payload.url().path().to_owned()));
            }
            // The production hook: a started load cancels the streams its page owned.
            crate::module_host::handle_page_load(webview, payload);
        })
        .on_window_event(crate::module_host::handle_window_event)
        .setup(|app| {
            let events = app.state::<LoadEvents>().0.subscribe();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("settings".into()))
                .inner_size(800.0, 600.0)
                .visible(false)
                .build()?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let ready = wait_load(events, "main", "/settings").await;
                scenario(handle, ready).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::call_provider_stream,
            commands::cancel_provider_stream,
            commands::list_usage_logs,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
