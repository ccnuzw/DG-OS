//! V1-FR-003 W4: real MCP compatibility and desktop evidence.
//!
//! The acceptance profile seeds two real, separately managed servers in a
//! temporary database: the official filesystem server over stdio and an
//! official SDK Streamable HTTP echo server. Nothing is written to the user's
//! database or credential store. The page is the shipped `dist/` in a visible
//! WKWebView; all management, confirmation, tool and recovery actions go
//! through production Tauri commands.

use super::{eval, finish, goto, wait_load, Checks, LoadEvents, Pending};
use crate::commands;
use async_trait::async_trait;
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_mcp::{
    CreateMcpServerRequest, CredentialScope, HttpConfig, McpCallPolicy, McpCredentialVault,
    McpError, McpManager, SecretToken, StdioConfig, TransportKind,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;

const USER: &str = "default-user";
const MAIN: &str = "main";
const SMOKE_WATCHDOG: Duration = Duration::from_secs(600);

struct AllowCalls;
#[async_trait]
impl McpCallPolicy for AllowCalls {
    async fn allows_call(&self, _: &str, _: &str, _: &str) -> bool {
        true
    }
}

#[derive(Default)]
struct MemoryVault(Mutex<std::collections::HashMap<String, String>>);
#[async_trait]
impl McpCredentialVault for MemoryVault {
    async fn store(&self, scope: &CredentialScope, token: SecretToken) -> dgos_mcp::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(scope.reference.clone(), token.expose().to_owned());
        Ok(())
    }
    async fn load(&self, scope: &CredentialScope) -> dgos_mcp::Result<SecretToken> {
        self.0
            .lock()
            .unwrap()
            .get(&scope.reference)
            .cloned()
            .map(SecretToken::new)
            .transpose()
            .map_err(|_| McpError::CredentialUnavailable)?
            .ok_or(McpError::CredentialUnavailable)
    }
    async fn remove(&self, scope: &CredentialScope) -> dgos_mcp::Result<()> {
        self.0.lock().unwrap().remove(&scope.reference);
        Ok(())
    }
}

struct Fixture {
    root: PathBuf,
    manager: Arc<McpManager>,
    filesystem_id: String,
    http_id: String,
    sdk_child: Mutex<Option<Child>>,
}

async fn official_http(prefix: &str) -> (Child, String) {
    let node = std::env::var("DGOS_OFFICIAL_NODE").unwrap_or_else(|_| "node".into());
    let server_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/dgos-mcp/tests/fixtures/mcp-official-sdk-http.mjs");
    let mut child = Command::new(node)
        .arg(server_script)
        .arg(prefix)
        .arg("streamable")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("start official SDK MCP server");
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
    )
    .await
    .expect("official SDK server startup timeout")
    .expect("official SDK server output");
    (child, line.trim().to_owned())
}

async fn build_fixture() -> Result<Fixture, String> {
    let filesystem = std::env::var("DGOS_OFFICIAL_FILESYSTEM_SERVER")
        .map(PathBuf::from)
        .map_err(|_| {
            "set DGOS_OFFICIAL_FILESYSTEM_SERVER to the official filesystem Server entrypoint"
                .to_owned()
        })?;
    let prefix = std::env::var("DGOS_OFFICIAL_MCP_SDK_PREFIX").map_err(|_| {
        "set DGOS_OFFICIAL_MCP_SDK_PREFIX to the isolated official SDK install".to_owned()
    })?;
    let root = std::env::temp_dir().join(format!("dgos-mcp-w4-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    std::fs::write(root.join("acceptance.txt"), "DGOS-FR003-W4\n").map_err(|e| e.to_string())?;
    let db_url = format!("sqlite://{}?mode=rwc", root.join("dgos.db").display());
    let db = dgos_db::establish_connection(&db_url)
        .await
        .map_err(|e| e.to_string())?;
    dgos_db::migrations::Migrator::up(&db, None)
        .await
        .map_err(|e| e.to_string())?;
    let vault = Arc::new(MemoryVault::default());
    let manager = Arc::new(McpManager::with_policy_and_vault(
        db.clone(),
        Arc::new(AllowCalls),
        vault,
    ));
    let fs_server = manager
        .create_server(
            USER,
            CreateMcpServerRequest {
                name: "Official Filesystem".into(),
                transport: TransportKind::Stdio,
                stdio: Some(StdioConfig {
                    executable: std::env::var("DGOS_OFFICIAL_NODE")
                        .unwrap_or_else(|_| "node".into()),
                    args: vec![
                        filesystem.to_string_lossy().into_owned(),
                        root.to_string_lossy().into_owned(),
                    ],
                    cwd: None,
                }),
                http: None,
                enabled: Some(true),
                auto_connect: Some(false),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    manager
        .connect(USER, &fs_server.id)
        .await
        .map_err(|e| e.to_string())?;
    let (sdk_child, url) = official_http(&prefix).await;
    let http_server = manager
        .create_server(
            USER,
            CreateMcpServerRequest {
                name: "Official SDK Echo".into(),
                transport: TransportKind::Http,
                stdio: None,
                http: Some(HttpConfig {
                    url,
                    credential_ref: None,
                    allow_insecure_http: false,
                }),
                enabled: Some(true),
                auto_connect: Some(false),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    manager
        .set_bearer_token(
            USER,
            &http_server.id,
            SecretToken::new("test-only-token-acceptance".into()).unwrap(),
        )
        .await
        .map_err(|e| e.to_string())?;
    manager
        .connect(USER, &http_server.id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Fixture {
        root,
        manager,
        filesystem_id: fs_server.id,
        http_id: http_server.id,
        sdk_child: Mutex::new(Some(sdk_child)),
    })
}

async fn call_in_browser(app: &AppHandle, server_name: &str, tool_name: &str, args: &str) -> Value {
    let result = eval(app, MAIN, &format!(r#"
      const wait = (ms) => new Promise(r => setTimeout(r, ms));
      const until = async (fn, ms=15000) => {{ const t=Date.now(); while(Date.now()-t<ms) {{ const v=fn(); if(v) return v; await wait(50); }} return false; }};
      const txt = e => (e?.textContent || '').trim();
      const buttons = () => [...document.querySelectorAll('button')];
      const select = buttons().find(b => b.getAttribute('aria-label') === 'Select {server_name}');
      if (!select) throw new Error('server row missing'); select.click();
      const tool = await until(() => [...document.querySelectorAll('article')].find(a => a.querySelector('code')?.textContent === {tool_name:?}));
      const ready = tool?.querySelector('button'); if (!ready) throw new Error('tool button missing');
      ready.click();
      const dialog = await until(() => document.querySelector('[role=dialog]')); if (!dialog) throw new Error('tool dialog missing');
      const input = dialog.querySelector('textarea'); if (!input) throw new Error('arguments input missing');
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set; set.call(input, {args:?}); input.dispatchEvent(new Event('input', {{bubbles:true}}));
      [...dialog.querySelectorAll('button')].find(b => txt(b) === 'Review tool call').click();
      const review = await until(() => dialog.querySelector('section[aria-label="Review tool call"]')); if (!review) throw new Error('review surface missing');
      const reviewed = txt(review).includes({server_name:?}) && txt(review).includes({tool_name:?})
        && txt(review).includes(JSON.stringify(JSON.parse({args:?}), null, 2));
      const confirm = await until(() => [...dialog.querySelectorAll('button')].find(b => txt(b) === 'Confirm and call tool')); if (!confirm) throw new Error('confirmation button missing');
      confirm.click();
      const result = await until(() => txt(dialog).includes('Tool completed') || txt(dialog).includes('Tool reported an error'), 20000);
      return {{result: !!result, reviewed, text: txt(dialog)}};
    "#, server_name = server_name, tool_name = tool_name, args = args)).await;
    if result["ok"] == true {
        result["value"].clone()
    } else {
        json!({"__error":result["error"]})
    }
}

async fn scenario(app: AppHandle, fixture: Fixture) {
    let mut checks = Checks::default();
    let loaded = goto(&app, MAIN, "/mcp").await;
    checks.check(
        "W4 real WebView loads MCP management",
        loaded,
        "page did not load",
    );
    let _hydrated = eval(&app, MAIN, r#"
      const wait=(ms)=>new Promise(r=>setTimeout(r,ms));
      const t=Date.now();
      while(Date.now()-t<15000 && !document.querySelector('button[aria-label^="Select "]')) await wait(50);
      return !!document.querySelector('button[aria-label^="Select "]');
    "#).await;
    let seeded = fixture.manager.list_servers(USER).await.unwrap_or_default();
    let rows = eval(&app, MAIN, r#"return {rows:[...document.querySelectorAll('button[aria-label^="Select "]')].map(x => x.getAttribute('aria-label')), text:document.body.innerText, alerts:[...document.querySelectorAll('[role=alert]')].map(x=>x.innerText)}"#).await;
    checks.check(
        "two acceptance MCP servers are managed together",
        seeded.len() == 2
            && rows.to_string().contains("Official Filesystem")
            && rows.to_string().contains("Official SDK Echo"),
        json!({"seeded": seeded.len(), "page": rows}),
    );
    let fs = call_in_browser(
        &app,
        "Official Filesystem",
        "list_directory",
        &json!({"path":fixture.root}).to_string(),
    )
    .await;
    checks.check(
        "real WebView shows exact tool confirmation and filesystem result",
        fs["result"] == true
            && fs["reviewed"] == true
            && fs["text"]
                .as_str()
                .is_some_and(|text| text.contains("acceptance.txt")),
        fs,
    );
    let echo = call_in_browser(
        &app,
        "Official SDK Echo",
        "echo",
        r#"{"text":"DGOS-FR003-W4-ECHO"}"#,
    )
    .await;
    checks.check(
        "real WebView calls the second HTTP MCP",
        echo["result"] == true
            && echo["reviewed"] == true
            && echo["text"]
                .as_str()
                .is_some_and(|text| text.contains("DGOS-FR003-W4-ECHO")),
        echo,
    );

    // Turn the already-successful filesystem server into a deterministic failed
    // recovery. The smoke-only monitor hook advances the production retry state
    // without changing its 30-second release interval.
    let manager = app.state::<commands::McpState>().manager.clone();
    let _ = manager
        .update_server(
            USER,
            &fixture.filesystem_id,
            dgos_mcp::UpdateMcpServerRequest {
                transport: Some(TransportKind::Http),
                http: Some(HttpConfig {
                    url: "http://127.0.0.1:1/mcp".into(),
                    credential_ref: None,
                    allow_insecure_http: false,
                }),
                ..Default::default()
            },
        )
        .await;
    let _ = manager.connect(USER, &fixture.filesystem_id).await;
    for _ in 0..3 {
        manager.monitor_once_for_smoke().await;
        manager
            .make_reconnect_due_for_smoke(&fixture.filesystem_id)
            .await;
    }
    let notice = goto(&app, MAIN, "/workbench").await;
    let recovery = eval(&app, MAIN, r#"
      const t=Date.now();
      while(Date.now()-t<15000 && !document.querySelector('[role="alert"][aria-label="MCP connection recovery stopped"]')) await new Promise(r=>setTimeout(r,50));
      return !!document.querySelector('[role="alert"][aria-label="MCP connection recovery stopped"]');
    "#).await;
    checks.check(
        "real WebView shows bounded recovery failure outside management",
        notice && recovery["ok"] == true && recovery["value"] == true,
        recovery,
    );

    let logs = manager
        .list_usage_logs(USER, Some(&fixture.http_id), Some(50))
        .await
        .unwrap_or_default();
    checks.check(
        "real MCP calls have redacted usage records",
        logs.iter().any(|log| log.outcome == "success")
            && logs
                .iter()
                .all(|log| log.task_ref.as_deref().unwrap_or("").len() < 200),
        format!("{} logs", logs.len()),
    );
    let _ = manager.shutdown().await;
    let sdk_child = { fixture.sdk_child.lock().unwrap().take() };
    if let Some(mut child) = sdk_child {
        let _ = child.kill().await;
    }
    finish(checks, &fixture.root);
}

pub fn run_mcp() {
    std::thread::spawn(|| {
        std::thread::sleep(SMOKE_WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        std::process::exit(2);
    });
    let fixture = match tauri::async_runtime::block_on(build_fixture()) {
        Ok(fixture) => fixture,
        Err(error) => {
            println!("SMOKE RESULT: 0 passed, 1 failed");
            println!("  FAILED W4 setup: {error}");
            std::process::exit(1);
        }
    };
    let manager = fixture.manager.clone();
    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let fixture_cell = Arc::new(Mutex::new(Some(fixture)));
    let fixture_for_setup = fixture_cell.clone();
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(crate::session::SessionResolver::no_accounts())
        .manage(LoadEvents(load_sender))
        .manage(commands::McpState {
            manager: manager.clone(),
        })
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview
                    .app_handle()
                    .state::<LoadEvents>()
                    .0
                    .send((webview.label().into(), payload.url().path().into()));
            }
        })
        .setup(move |app| {
            let events = app.state::<LoadEvents>().0.subscribe();
            // Start at the root asset; BrowserRouter navigation below exercises the
            // same route transition used by the shipped app (and avoids relying
            // on custom-protocol fallback behavior for a deep-link at startup).
            WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("/".into()))
                .inner_size(1000.0, 760.0)
                .visible(true)
                .build()?;
            let handle = app.handle().clone();
            let fixture = fixture_for_setup.lock().unwrap().take().unwrap();
            tauri::async_runtime::spawn(async move {
                let _ = wait_load(events, MAIN, "/").await;
                scenario(handle, fixture).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
