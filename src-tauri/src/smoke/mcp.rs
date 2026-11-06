//! V1-FR-003 W4: real MCP compatibility and desktop evidence.
//!
//! The acceptance profile seeds two real, separately managed servers in a
//! temporary database: the official filesystem server over stdio and an
//! official SDK Streamable HTTP echo server. Nothing is written to the user's
//! database or credential store. The page is the shipped `dist/` in a visible
//! WKWebView; all management, confirmation, tool and recovery actions go
//! through production Tauri commands.
//!
//! `DGOS_MCP_SMOKE_MODE=page` selects the independent production-page acceptance
//! journey in mcp_page.rs. It retains its fresh /tmp evidence root and does not
//! repeat W4. Omit the variable (or use `w4`) for the original compatibility run.

use super::{eval, finish, ipc, wait_load, Checks, LoadEvents, Pending};
use crate::commands;
use async_trait::async_trait;
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_mcp::{
    CreateMcpServerRequest, CredentialScope, HttpConfig, McpCredentialVault,
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

#[path = "mcp_page.rs"]
mod page;

const USER: &str = "default-user";
const MAIN: &str = "main";
const SMOKE_WATCHDOG: Duration = Duration::from_secs(600);

// Match the shipped accessibility labels without changing the user's locale.
const BROWSER_LABELS: &str = r#"
  const language = document.documentElement.getAttribute('data-locale') || document.documentElement.lang;
  const locale = language === 'en' ? 'en-US' : language === 'zh' ? 'zh-CN' : language;
  const labels = {
    'zh-CN': { inspect: '检查 / 调用', review: '检查工具调用', confirm: '确认并调用工具', completed: '工具已完成', error: '工具返回错误', result: '工具结果', close: '关闭', recovery: 'MCP 连接恢复已停止' },
    'en-US': { inspect: 'Inspect / call', review: 'Review tool call', confirm: 'Confirm and call tool', completed: 'Tool completed', error: 'Tool reported an error', result: 'Tool result', close: 'Close', recovery: 'MCP connection recovery stopped' },
  }[locale];
  if (!labels) throw new Error('unsupported MCP page locale: ' + locale);
"#;


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
    db: dgos_db::sea_orm::DatabaseConnection,
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
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    let bytes = tokio::time::timeout(Duration::from_secs(10), stdout.read_line(&mut line))
        .await
        .expect("official SDK server startup timeout")
        .expect("official SDK server output");
    assert!(bytes > 0, "official SDK server exited before readiness");
    // The SDK can log after readiness; dropping its reader can cause EPIPE.
    tauri::async_runtime::spawn(async move {
        let _ = tokio::io::copy(&mut stdout, &mut tokio::io::sink()).await;
    });
    (child, line.trim().to_owned())
}

async fn build_fixture(maintenance: dgos_db::MaintenanceGate, resolver: Arc<crate::session::SessionResolver>) -> Result<Fixture, String> {
    let filesystem = std::env::var("DGOS_OFFICIAL_FILESYSTEM_SERVER")
        .map(PathBuf::from)
        .map_err(|_| {
            "set DGOS_OFFICIAL_FILESYSTEM_SERVER to the official filesystem Server entrypoint"
                .to_owned()
        })?;
    let prefix = std::env::var("DGOS_OFFICIAL_MCP_SDK_PREFIX").map_err(|_| {
        "set DGOS_OFFICIAL_MCP_SDK_PREFIX to the isolated official SDK install".to_owned()
    })?;
    let root = if page::selected() { page::root()? } else {
        let root = std::env::temp_dir().join(format!("dgos-mcp-w4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        root
    };
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
        Arc::new(commands::DesktopMcpPolicy(resolver)),
        vault,
    ).with_maintenance(maintenance));
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
        .map_err(|e| format!("official filesystem connect: {e}"))?;
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
        .map_err(|e| format!("official SDK HTTP connect: {e}"))?;
    Ok(Fixture {
        root,
        manager,
        filesystem_id: fs_server.id,
        http_id: http_server.id,
        sdk_child: Mutex::new(Some(sdk_child)),
        db,
    })
}

async fn call_in_browser(app: &AppHandle, server_name: &str, tool_name: &str, args: &str) -> Value {
    let select = eval(app, MAIN, &format!(r#"
      {BROWSER_LABELS}
      const txt = e => (e?.textContent || '').trim();
      const row = [...document.querySelectorAll('.mcp-server-row button[aria-pressed]')].find(b => [...b.querySelectorAll('span')].some(s => txt(s) === {server_name:?}));
      if (!row) throw new Error('server row missing');
      row.click();
      const t = Date.now();
      while (Date.now()-t < 15000) {{
        const tool = [...document.querySelectorAll('.mcp-server-detail article')].find(a => txt(a.querySelector('code')) === {tool_name:?});
        const button = [...(tool?.querySelectorAll('button') || [])].find(b => txt(b) === labels.inspect);
        if (button) {{ button.click(); return true; }}
        await new Promise(r => setTimeout(r, 50));
      }}
      throw new Error('tool button missing');
    "#)).await;
    if select["ok"] != true { return json!({"phase":"select tool", "failure":select}); }
    let review = eval(app, MAIN, &format!(r#"
      {BROWSER_LABELS}
      const txt = e => (e?.textContent || '').trim();
      const t = Date.now();
      let dialog;
      while (Date.now()-t < 15000) {{
        dialog = document.querySelector('[role=dialog]');
        if (dialog?.querySelector('textarea')) break;
        await new Promise(r => setTimeout(r, 50));
      }}
      const input = dialog?.querySelector('textarea');
      if (!input) throw new Error('arguments input missing');
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
      set.call(input, {args:?}); input.dispatchEvent(new Event('input', {{bubbles:true}}));
      return true;
    "#)).await;
    if review["ok"] != true { return json!({"phase":"enter arguments", "failure":review}); }
    let review = eval(app, MAIN, &format!(r#"
      {BROWSER_LABELS}
      const txt = e => (e?.textContent || '').trim();
      const dialog = document.querySelector('[role=dialog]');
      const button = [...dialog.querySelectorAll('button')].find(b => txt(b) === labels.review);
      if (!button) throw new Error('review button missing'); button.click();
      const t = Date.now();
      while (Date.now()-t < 15000) {{
        const review = [...dialog.querySelectorAll('section[aria-label]')].find(s => s.getAttribute('aria-label') === labels.review);
        if (review) return txt(review.querySelector('strong')) === {server_name:?}
          && txt(review.querySelector('code')) === {tool_name:?}
          && txt(review.querySelector('pre')) === JSON.stringify(JSON.parse({args:?}), null, 2);
        await new Promise(r => setTimeout(r, 50));
      }}
      throw new Error('review surface missing: '+txt(dialog));
    "#)).await;
    if review["ok"] != true || review["value"] != true { return json!({"phase":"review arguments", "failure":review}); }
    let result = eval(app, MAIN, &format!(r#"
      {BROWSER_LABELS}
      const txt = e => (e?.textContent || '').trim();
      const dialog = document.querySelector('[role=dialog]');
      const confirm = [...dialog.querySelectorAll('button')].find(b => txt(b) === labels.confirm);
      if (!confirm) throw new Error('confirmation button missing'); confirm.click();
      const t = Date.now();
      let result;
      while (Date.now()-t < 20000) {{
        result = [...dialog.querySelectorAll('section[aria-label]')].find(s => s.getAttribute('aria-label') === labels.result && [labels.completed, labels.error].includes(txt(s.querySelector('h3'))));
        if (result) break;
        await new Promise(r => setTimeout(r, 50));
      }}
      const text = txt(dialog);
      const close = [...dialog.querySelectorAll('button')].find(b => txt(b) === labels.close);
      if (!close) throw new Error('tool close button missing'); close.click();
      return {{result: !!result, reviewed: true, text}};
    "#)).await;
    if result["ok"] == true { result["value"].clone() }
    else { json!({"phase":"call tool", "failure":result}) }
}

async fn scenario(app: AppHandle, fixture: Fixture) {
    let mut checks = Checks::default();
    let loaded = eval(&app, MAIN, r#"
      const state = {...history.state, idx:(history.state?.idx??0)+1, key:crypto.randomUUID()};
      history.pushState(state, '', '/mcp');
      dispatchEvent(new PopStateEvent('popstate', {state}));
      return location.pathname === '/mcp';
    "#).await;
    checks.check(
        "W4 real WebView loads MCP management",
        loaded["ok"] == true && loaded["value"] == true,
        "page did not load",
    );
    let _hydrated = eval(&app, MAIN, r#"
      const wait=(ms)=>new Promise(r=>setTimeout(r,ms));
      const t=Date.now();
      while(Date.now()-t<15000 && !document.querySelector('.mcp-server-row button[aria-pressed]')) await wait(50);
      return !!document.querySelector('.mcp-server-row button[aria-pressed]');
    "#).await;
    let seeded = fixture.manager.list_servers(USER).await.unwrap_or_default();
    let rows = eval(&app, MAIN, r#"return {rows:[...document.querySelectorAll('.mcp-server-row button[aria-pressed]')].map(x => x.textContent), text:document.body.innerText, alerts:[...document.querySelectorAll('[role=alert]')].map(x=>x.innerText)}"#).await;
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
    let changed = manager
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
    checks.check("recovery configuration is accepted", changed.is_ok(), format!("{changed:?}"));
    let disconnected = manager.connect(USER, &fixture.filesystem_id).await;
    checks.check("recovery endpoint fails explicitly", disconnected.is_err(), format!("{disconnected:?}"));
    for _ in 0..3 {
        manager.monitor_once_for_smoke().await;
        manager
            .make_reconnect_due_for_smoke(&fixture.filesystem_id)
            .await;
    }
    let recovery_state = manager.list_servers(USER).await.unwrap_or_default();
    checks.check("three recovery attempts exhaust the failed connection", recovery_state.iter().any(|s| s.id == fixture.filesystem_id && s.reconnect_exhausted && s.status == "error"), format!("{recovery_state:?}"));
    let notice = eval(&app, MAIN, r#"
      const state = {...history.state, idx:(history.state?.idx??0)+1, key:crypto.randomUUID()};
      history.pushState(state, '', '/workbench');
      dispatchEvent(new PopStateEvent('popstate', {state}));
      return location.pathname === '/workbench';
    "#).await;
    let recovery = eval(&app, MAIN, r#"
      const stopped = () => {
        const locale = document.documentElement.getAttribute('data-locale');
        const label = locale === 'zh-CN' ? 'MCP 连接恢复已停止' : locale === 'en-US' ? 'MCP connection recovery stopped' : null;
        return label !== null && [...document.querySelectorAll('[role="alert"][aria-label]')].some(e => e.getAttribute('aria-label') === label);
      };
      const t=Date.now();
      while(Date.now()-t<15000 && !stopped()) await new Promise(r=>setTimeout(r,50));
      return {shown: stopped(), locale: document.documentElement.getAttribute('data-locale'), alerts: [...document.querySelectorAll('[role="alert"]')].map(e=>({label:e.getAttribute('aria-label'),text:e.textContent}))};
    "#).await;
    checks.check(
        "real WebView shows bounded recovery failure outside management",
        notice["ok"] == true && notice["value"] == true && recovery["ok"] == true && recovery["value"]["shown"] == true,
        recovery,
    );

    let logs = manager
        .list_usage_logs(USER, Some(&fixture.http_id), Some(50))
        .await
        .unwrap_or_default();
    checks.check(
        "real MCP calls have redacted usage records",
        logs.len() == 1 && logs[0].outcome == "success" && logs[0].tool_name == "echo"
            && !serde_json::to_string(&logs).unwrap().contains("DGOS-FR003-W4-ECHO")
            && !serde_json::to_string(&logs).unwrap().contains("test-only-token-acceptance"),
        format!("{} terminal echo logs, no result or credential markers", logs.len()),
    );

    // Restore through the real page command gate, then verify the global notice
    // disappears and a newly confirmed call works without replaying the old one.
    let restored = eval(&app, MAIN, r#"
      const state = {...history.state, idx:(history.state?.idx??0)+1, key:crypto.randomUUID()};
      history.pushState(state, '', '/mcp');
      dispatchEvent(new PopStateEvent('popstate', {state}));
      const t=Date.now();
      while(Date.now()-t<15000 && !document.querySelector('.mcp-server-row')) await new Promise(r=>setTimeout(r,50));
      return !!document.querySelector('.mcp-server-row');
    "#).await;
    let configured = ipc(&app, MAIN, "update_mcp_server", json!({"serverId":fixture.filesystem_id,"request":{
        "transport":"stdio", "stdio":{
            "executable":std::env::var("DGOS_OFFICIAL_NODE").unwrap_or_else(|_|"node".into()),
            "args":[std::env::var("DGOS_OFFICIAL_FILESYSTEM_SERVER").unwrap(),fixture.root], "cwd":null
        }
    }})).await;
    let connected = ipc(&app, MAIN, "connect_mcp_server", json!({"serverId":fixture.filesystem_id})).await;
    checks.check("manual recovery uses production configuration and connection commands",
        restored["ok"] == true && !configured.rejected() && !connected.rejected()
            && connected.0["value"]["status"] == "ready", format!("{} {}",configured.describe(),connected.describe()));
    let cleared = eval(&app, MAIN, r#"
      const state = {...history.state, idx:(history.state?.idx??0)+1, key:crypto.randomUUID()};
      history.pushState(state, '', '/workbench');
      dispatchEvent(new PopStateEvent('popstate', {state}));
      const t=Date.now();
      const stopped=()=>document.querySelector('[role=alert][aria-label]');
      while(Date.now()-t<15000 && stopped()) await new Promise(r=>setTimeout(r,50));
      const cleared = !stopped() && location.pathname === '/workbench';
      const back = {...history.state, idx:(history.state?.idx??0)+1, key:crypto.randomUUID()};
      history.pushState(back, '', '/mcp');
      dispatchEvent(new PopStateEvent('popstate', {state:back}));
      while(Date.now()-t<15000 && !document.querySelector('.mcp-server-row')) await new Promise(r=>setTimeout(r,50));
      return cleared;
    "#).await;
    checks.check("successful manual recovery clears the cross-page warning", cleared["ok"] == true && cleared["value"] == true, cleared);
    let resumed = call_in_browser(&app, "Official Filesystem", "list_directory", &json!({"path":fixture.root}).to_string()).await;
    let fs_logs = manager.list_usage_logs(USER, Some(&fixture.filesystem_id), Some(50)).await.unwrap_or_default();
    checks.check("recovered filesystem executes one new confirmed call with terminal audit",
        resumed["result"] == true && resumed["text"].as_str().is_some_and(|text|text.contains("acceptance.txt"))
            && fs_logs.len() == 2 && fs_logs.iter().all(|log|log.tool_name == "list_directory" && log.outcome == "success"),
        json!({"call":resumed,"audit":fs_logs}));
    let _ = manager.shutdown().await;
    let sdk_child = { fixture.sdk_child.lock().unwrap().take() };
    if let Some(mut child) = sdk_child {
        let _ = child.kill().await;
    }
    finish(checks, &fixture.root);
}

pub fn run_mcp() {
    if std::env::var("DGOS_MCP_SMOKE_MODE").is_ok_and(|mode| mode != "page" && mode != "w4") {
        eprintln!("DGOS_MCP_SMOKE_MODE must be page or w4");
        std::process::exit(1);
    }
    std::thread::spawn(|| {
        std::thread::sleep(SMOKE_WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        std::process::exit(2);
    });
    let maintenance = dgos_db::MaintenanceGate::new();
    let resolver = crate::session::SessionResolver::no_accounts();
    let fixture = match tauri::async_runtime::block_on(build_fixture(maintenance.clone(), resolver.clone())) {
        Ok(fixture) => fixture,
        Err(error) => {
            println!("SMOKE RESULT: 0 passed, 1 failed");
            println!("  FAILED W4 setup: {error}");
            std::process::exit(1);
        }
    };
    let manager = fixture.manager.clone();
    let db = fixture.db.clone();
    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let fixture_cell = Arc::new(Mutex::new(Some(fixture)));
    let fixture_for_setup = fixture_cell.clone();
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(page::Control::default())
        .manage(dgos_db::AppearanceRepository::new(db.clone()))
        .manage(dgos_db::HostSettingsRepository::new(db.clone()))
        .manage(dgos_db::DesktopRepository::new(db.clone()))
        .manage(dgos_db::CliRepository::new(db))
        .manage(maintenance)
        .manage(resolver)
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
            let fixture = fixture_for_setup.lock().unwrap().take().unwrap();
            if page::selected() { tauri::async_runtime::block_on(page::install(app, &fixture)); }
            let events = app.state::<LoadEvents>().0.subscribe();
            // Start at the root asset; BrowserRouter navigation below exercises the
            // same route transition used by the shipped app (and avoids relying
            // on custom-protocol fallback behavior for a deep-link at startup).
            WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("/".into()))
                .inner_size(if page::selected() {1440.0}else{1000.0}, if page::selected() {900.0}else{760.0})
                .visible(true)
                .build()?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = wait_load(events, MAIN, "/").await;
                if page::selected() { page::run(handle, fixture).await; }
                else { scenario(handle, fixture).await; }
            });
            Ok(())
        })
        .invoke_handler(|invoke| {
            if page::selected() && matches!(invoke.message.command(), "read_workspace_app_catalog" | "attach_module_host" | "report_module_host" | "report_module_crash") {
                let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![commands::read_workspace_app_catalog, commands::attach_module_host, commands::report_module_host, commands::report_module_crash];
                return handler(invoke);
            }
            if page::selected() && matches!(invoke.message.command(), "list_mcp_servers" | "get_mcp_server_config" | "update_mcp_server" | "refresh_mcp_tools" | "list_mcp_tools" | "list_mcp_usage_logs" | "list_mcp_connection_logs") {
                return page::handle(invoke);
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
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
            commands::get_appearance_preferences, commands::set_appearance_preferences,
            commands::recover_appearance_preferences,
            commands::get_language_preferences, commands::set_language_preferences,
            commands::get_desktop_preferences, commands::set_desktop_preferences,
            commands::get_cli_preferences, commands::set_cli_preferences,
            super::smoke_report,
            ];
            handler(invoke)
        })
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
