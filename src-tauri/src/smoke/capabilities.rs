//! Real module JS calling the five D093 capabilities in a real WebView (V1-FR-009, D093, D050, D055, D086).
//!
//! Sixth smoke target. It wires the production pieces: `PageModuleLoader`, `ModuleHostBridge` with the MCP
//! confirmation broker, the route observer, `ModuleCallRouter`, the typed `module_storage_*`,
//! `module_network_request`, `module_call_mcp_tool`, `module_prepare_skill` and
//! `resolve_module_mcp_confirmation` commands, `ModuleCapabilityDispatchService`, the module management
//! service and a SQLite database with the real repositories. The modules are
//! `tests/fixtures/modules/capability-caller.js`; their `host.storage.*`, `host.network.request`,
//! `host.mcp.callTool` and `host.skills.prepare` go through the real Tauri IPC. The MCP confirmation dialog
//! and the module management page are the shipped `dist/`, driven through their real DOM.
//!
//! Replaced for the run: the network target is a loopback HTTP fixture, the MCP server is the `dgos-mcp`
//! stdio fixture run with `node` (credential vault refuses everything), the Skills catalog is the bundled
//! official catalog in a temp data directory, admission is test pins. Temp directory only; nothing touches
//! the real user's data.
//!
//! What it proves: per-call authorization, denial audit, storage/network/skill/MCP behavior, host-owned MCP
//! confirmation, revocation, audit paging and isolation, audit/usage-log redaction, and the management page
//! flows (enable consent, grant, revoke, permission log, update permission diff) on the platform WebView of
//! the machine that runs it. It does not prove Windows WebView2 or Linux WebKitGTK, real pointer hit-testing
//! (DOM mouse/click events), a release build, or isolation between modules (the realm is shared, D042).

use super::{
    eval, finish, ipc, poll_operation, wait_load, wait_until, Checks, LoadEvents, Pending, WATCHDOG,
};
use crate::commands;
use crate::module_host::{self, ChannelTransport, ModuleHostBridge, RoutePublisher};
use async_trait::async_trait;
use dgos_application::{
    HostCapabilityService, ModuleCapabilityDispatchService, ModuleCrashService,
    ModuleManagementService, ModuleMcpConfirmations, ModuleMcpService, ModuleOperationManager,
    ModuleReleaseService, ModuleStorageService,
};
use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::{ModuleCrashRepository, ModulePermissionRepository, ModuleStorageRepository};
use dgos_mcp::{
    CallMcpToolRequest, CreateMcpServerRequest, CredentialScope, McpCredentialVault, McpError,
    McpManager, SecretToken, StdioConfig, TransportKind,
};
use dgos_modules::{
    LoaderTimeouts, LocalModuleSources, ModuleAdmission, ModuleCallRouter, ModulePublisher,
    ModuleRuntimeSupervisor, PageLoaderConfig, PageModuleLoader,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

const CALLER: &str = include_str!("../../../tests/fixtures/modules/capability-caller.js");

const MAIN: &str = module_host::MODULE_WINDOW;
const USER: &str = "default-user";
const GRANTED: &str = "cap-granted";
const UNGRANTED: &str = "cap-ungranted";
const UI: &str = "cap-ui";
const UPD: &str = "cap-upd";
const ALL_PERMISSIONS: [&str; 6] = [
    "providers.call",
    "skills.execute",
    "mcp.call",
    "storage.read",
    "storage.write",
    "network.local",
];
const SKILL: &str = "dgos.official.writing.summarize";

const DENIED: u64 = 1004004;
const INVALID: u64 = 1004024;
const QUOTA: u64 = 1004025;
const REFUSED: u64 = 1004026;
const TOO_LARGE: u64 = 1004027;
const MCP_NOT_CONFIRMED: u64 = 1003019;

const SECRET_STORE: &str = "SECRET-STORE-7f3a91";
const SECRET_PATH: &str = "SECRET-PATH-2b8c40";
const SECRET_BODY: &str = "SECRET-BODY-c51d9e";
const SECRET_MCP_REJECT: &str = "SECRET-MCP-REJECT-4e7a12";
const SECRET_MCP_OK: &str = "SECRET-MCP-OK-93bb05";
const SECRET_MCP_LATE: &str = "SECRET-MCP-LATE-18d6f7";

// -- Loopback HTTP fixture ---------------------------------------------------------------------------

struct SeenRequest {
    method: String,
    path: String,
    headers: String,
    body: String,
}

#[derive(Default)]
struct HttpStats {
    connections: AtomicUsize,
    requests: Mutex<Vec<SeenRequest>>,
}

impl HttpStats {
    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn serve_http(listener: TcpListener, stats: Arc<HttpStats>) {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        stats.connections.fetch_add(1, Ordering::SeqCst);
        let stats = stats.clone();
        tauri::async_runtime::spawn(async move {
            let mut data = Vec::new();
            let mut buffer = [0u8; 4096];
            let head_end = loop {
                let Ok(read) = socket.read(&mut buffer).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                data.extend_from_slice(&buffer[..read]);
                if let Some(position) = find(&data, b"\r\n\r\n") {
                    break position + 4;
                }
            };
            let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
            let length = head
                .to_lowercase()
                .lines()
                .find_map(|line| line.strip_prefix("content-length:").map(str::to_owned))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while data.len() < head_end + length {
                let Ok(read) = socket.read(&mut buffer).await else {
                    return;
                };
                if read == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..read]);
            }
            let mut line = head.lines().next().unwrap_or("").split_whitespace();
            let method = line.next().unwrap_or("").to_owned();
            let path = line.next().unwrap_or("").to_owned();
            let body = String::from_utf8_lossy(&data[head_end..]).into_owned();
            stats.requests.lock().unwrap().push(SeenRequest {
                method: method.clone(),
                path: path.clone(),
                headers: head.to_lowercase(),
                body: body.clone(),
            });
            let response = if path == "/redirect" {
                "HTTP/1.1 302 Found\r\nLocation: /landed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned()
            } else {
                let payload = if path == "/big" {
                    "a".repeat(1024 * 1024 + 1)
                } else {
                    format!("{method} {path} {body}")
                };
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                )
            };
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
    }
}

// -- World -------------------------------------------------------------------------------------------

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

type Pins = Arc<Mutex<Vec<(String, String, String)>>>;

/// Test pins that can grow while the app runs, like a new build would ship new pins.
struct GrowingAdmission(Pins);

impl ModuleAdmission for GrowingAdmission {
    fn admits(&self, module_id: &str, version: &str, integrity: &str) -> bool {
        self.0.lock().unwrap().iter().any(|(id, v, digest)| {
            id == module_id && v == version && digest.eq_ignore_ascii_case(integrity)
        })
    }
}

/// One-file package declaring `permissions`; returns its v2 integrity.
fn write_cap_package(
    dir: &Path,
    id: &str,
    name: &str,
    version: &str,
    permissions: &[&str],
    body: &str,
) -> String {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("entry.js"), body).unwrap();
    let mut manifest = json!({
        "id": id, "name": name, "version": version, "entry": "entry.js",
        "artifact": {"version": version, "sha256": "0".repeat(64)},
        "permissions": permissions
    });
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    let integrity = dgos_modules::package_integrity(dir).unwrap();
    manifest["artifact"]["sha256"] = integrity.strip_prefix("v2:").unwrap().into();
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    integrity
}

/// State the Tauri builder manages; moved into the application.
struct Managed {
    manager: ModuleOperationManager,
    bridge: ModuleHostBridge,
    router: Arc<ModuleCallRouter>,
    crashes: ModuleCrashService,
    management: ModuleManagementService,
    dispatch: ModuleCapabilityDispatchService,
    storage: ModuleStorageService,
    mcp_service: ModuleMcpService,
    skills: commands::SkillsState,
}

struct CapWorld {
    root: PathBuf,
    sources: PathBuf,
    db: DatabaseConnection,
    pins: Pins,
    manager: ModuleOperationManager,
    mcp: Arc<McpManager>,
    server_id: String,
    http: Arc<HttpStats>,
    http_port: u16,
    maintenance: dgos_db::MaintenanceGate,
}

async fn build_world() -> (CapWorld, Managed) {
    let root = std::env::temp_dir().join(format!(
        "dgos-webview-smoke-capabilities-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let store = root.join("modules");
    let sources = root.join("module-sources");
    fs::create_dir_all(&store).unwrap();
    fs::create_dir_all(&sources).unwrap();

    let url = format!("sqlite://{}?mode=rwc", root.join("dgos.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    let maintenance = dgos_db::MaintenanceGate::new();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();

    // Loopback HTTP fixture for network.local.
    let http = Arc::new(HttpStats::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = listener.local_addr().unwrap().port();
    tauri::async_runtime::spawn(serve_http(listener, http.clone()));

    // Modules: granted / ungranted / management-page fixtures; all but `cap-ui` start enabled.
    let publisher = Arc::new(ModulePublisher::new(store, db.clone()));
    let release = ModuleReleaseService::new(publisher.clone());
    let pins: Pins = Arc::new(Mutex::new(Vec::new()));
    let fixtures: [(&str, &str, Vec<&str>, String); 4] = [
        (
            GRANTED,
            "Capability Granted",
            ALL_PERMISSIONS.to_vec(),
            CALLER.to_owned(),
        ),
        (
            UNGRANTED,
            "Capability Ungranted",
            ALL_PERMISSIONS.to_vec(),
            CALLER.to_owned(),
        ),
        (
            UI,
            "Capability UI",
            ALL_PERMISSIONS.to_vec(),
            CALLER.to_owned(),
        ),
        (
            UPD,
            "Capability Update",
            vec!["storage.read", "storage.write"],
            format!("{CALLER}\n// release 1.0.0\n"),
        ),
    ];
    for (id, name, permissions, body) in &fixtures {
        let dir = root.join(format!("pkg-{id}"));
        let digest = write_cap_package(&dir, id, name, "1.0.0", permissions, body);
        release.publish(&dir).await.unwrap();
        publisher.set_enabled(id, *id != UI).await.unwrap();
        pins.lock()
            .unwrap()
            .push(((*id).to_owned(), "1.0.0".to_owned(), digest));
    }

    let transport = Arc::new(ChannelTransport::default());
    let (loader, lost) = PageModuleLoader::new(
        transport.clone(),
        PageLoaderConfig {
            window_label: MAIN.into(),
            hook_timeout: Duration::from_secs(2),
        },
    );
    let router = Arc::new(ModuleCallRouter::new(loader.clone()));
    let routes = Arc::new(RoutePublisher::new(router.clone(), transport.clone()));
    let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), Arc::new(loader.clone()))
        .with_admission(Arc::new(GrowingAdmission(pins.clone())))
        .with_ready_observer(routes)
        .with_loader_timeouts(LoaderTimeouts {
            smoke: Duration::from_secs(5),
            start: Duration::from_secs(4),
            stop: Duration::from_secs(5),
        });
    let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
    let capabilities =
        HostCapabilityService::with_publisher(permissions.clone(), publisher.clone());
    let dispatch = ModuleCapabilityDispatchService::new(capabilities);
    let manager = ModuleOperationManager::with_supervisor(
        publisher,
        supervisor,
        LocalModuleSources::new(sources.clone()),
    )
    .with_maintenance(maintenance.clone());
    manager.spawn_lost_runtime_reporter(lost).unwrap();
    let crashes = ModuleCrashService::new(ModuleCrashRepository::new(db.clone()), manager.clone());
    let management =
        ModuleManagementService::new(permissions.clone(), manager.clone(), crashes.clone());

    // MCP: the dgos-mcp stdio fixture in its call-counting mode.
    let mcp = Arc::new(McpManager::with_policy_and_vault(
        db.clone(),
        Arc::new(commands::DesktopMcpPolicy(
            crate::session::SessionResolver::no_accounts(),
        )),
        Arc::new(RefusingMcpVault),
    ).with_maintenance(maintenance.clone()));
    let server = mcp
        .create_server(
            USER,
            CreateMcpServerRequest {
                name: "fixture-server".into(),
                transport: TransportKind::Stdio,
                stdio: Some(StdioConfig {
                    executable: "node".into(),
                    args: vec![
                        concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/../crates/dgos-mcp/tests/fixtures/mcp-server.mjs"
                        )
                        .into(),
                        "cancellation-observable".into(),
                    ],
                    cwd: None,
                }),
                http: None,
                enabled: Some(true),
                auto_connect: None,
            },
        )
        .await
        .expect("create the MCP fixture server");
    mcp.connect(USER, &server.id)
        .await
        .expect("connect the MCP fixture server (needs `node`)");
    let confirmations = Arc::new(ModuleMcpConfirmations::new(transport.clone()));
    let mcp_service = ModuleMcpService::new(confirmations.clone(), mcp.clone(), db.clone());
    let bridge = ModuleHostBridge::new(loader, transport).with_mcp_confirmations(confirmations);

    // Skills: the bundled official catalog, one skill installed for the local user.
    let skills_manager = dgos_skills::SkillsManager::new(db.clone(), root.join("skills"));
    skills_manager
        .register_bundled_catalog(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../skills"))
        .await
        .expect("register the bundled skills catalog");
    skills_manager
        .install_skill(SKILL, USER)
        .await
        .expect("install the official skill");
    let skills = commands::SkillsState {
        manager: Arc::new(tokio::sync::RwLock::new(skills_manager)),
    };

    let storage = ModuleStorageService::new(ModuleStorageRepository::new(db.clone()));
    (
        CapWorld {
            root,
            sources,
            db,
            pins,
            manager: manager.clone(),
            mcp,
            server_id: server.id,
            http,
            http_port,
            maintenance,
        },
        Managed {
            manager,
            bridge,
            router,
            crashes,
            management,
            dispatch,
            storage,
            mcp_service,
            skills,
        },
    )
}

// -- Helpers -----------------------------------------------------------------------------------------

async fn js(app: &AppHandle, body: &str) -> Value {
    eval(app, MAIN, body).await["value"].clone()
}

/// Poll `body` (a JS function body) until it returns something truthy.
async fn until(app: &AppHandle, body: &str, seconds: u64) -> bool {
    for _ in 0..(seconds * 10) {
        let value = js(app, body).await;
        let falsy =
            value.is_null() || value == json!(false) || value == json!(0) || value == json!("");
        if !falsy {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[derive(Debug)]
struct Out {
    ok: bool,
    code: Option<u64>,
    value: Value,
}

impl Out {
    fn parse(reply: &Value) -> Self {
        if reply["ok"] != true {
            return Self {
                ok: false,
                code: None,
                value: reply.clone(),
            };
        }
        let inner = &reply["value"];
        Self {
            ok: inner["ok"] == true,
            code: inner["code"].as_u64(),
            value: inner["value"].clone(),
        }
    }

    fn denied(&self) -> bool {
        !self.ok && self.code == Some(DENIED)
    }

    fn failed_with(&self, code: u64) -> bool {
        !self.ok && self.code == Some(code)
    }
}

/// One call from a module's own closure in the page.
async fn cap(app: &AppHandle, module: &str, method: &str, args: Value) -> Out {
    let body = format!(
        "const c = globalThis.__dgosCapabilityCallers && globalThis.__dgosCapabilityCallers[{module:?}];\
         if (!c) return {{ok: false, code: -1}};\
         return await c[{method:?}](...{args});"
    );
    Out::parse(&eval(app, MAIN, &body).await)
}

/// Start a call without waiting for it; `finished` collects it by `tag`.
async fn begin(app: &AppHandle, tag: &str, module: &str, method: &str, args: Value) {
    let body = format!(
        "const c = globalThis.__dgosCapabilityCallers[{module:?}];\
         globalThis.__capPending ??= {{}}; globalThis.__capPending[{tag:?}] = null;\
         c[{method:?}](...{args}).then((r) => {{ globalThis.__capPending[{tag:?}] = r; }});\
         return true;"
    );
    let _ = eval(app, MAIN, &body).await;
}

async fn finished(app: &AppHandle, tag: &str) -> Option<Out> {
    let body =
        format!("return (globalThis.__capPending && globalThis.__capPending[{tag:?}]) || null;");
    for _ in 0..100 {
        let value = js(app, &body).await;
        if !value.is_null() {
            return Some(Out::parse(&json!({"ok": true, "value": value})));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

const DIALOG_FIND: &str = "const find = () => [...document.querySelectorAll('[role=\"dialog\"]')].find((n) => n.textContent.includes('模块请求调用 MCP 工具'));";

/// Wait for the host MCP confirmation dialog and describe it.
async fn wait_dialog(app: &AppHandle) -> Value {
    let body = [
        DIALOG_FIND,
        "for (let i = 0; i < 100; i++) { const d = find(); if (d) { const pre = d.querySelector('pre');\
         return { text: d.textContent, args: pre ? pre.textContent : null,\
         insideModule: !!d.closest('[data-module-runtime],[data-module-id]'),\
         buttons: [...d.querySelectorAll('button')].map((b) => b.textContent.trim()) }; }\
         await new Promise((r) => setTimeout(r, 100)); } return null;",
    ]
    .concat();
    js(app, &body).await
}

async fn dialog_open(app: &AppHandle) -> bool {
    let body = [DIALOG_FIND, "return !!find();"].concat();
    js(app, &body).await == json!(true)
}

async fn click_dialog(app: &AppHandle, label: &str) -> bool {
    let body = [
        DIALOG_FIND,
        &format!(
            "const d = find(); if (!d) return false;\
             const b = [...d.querySelectorAll('button')].find((n) => n.textContent.trim() === {label:?});\
             if (!b) return false; b.click(); return true;"
        ),
    ]
    .concat();
    js(app, &body).await == json!(true)
}

async fn dialog_gone(app: &AppHandle) -> bool {
    let body = [DIALOG_FIND, "return !find();"].concat();
    until(app, &body, 5).await
}

/// Click the shipped rail link without reloading the module runtime. Settings opens its Provider category.
async fn navigate(app: &AppHandle, path: &str) -> bool {
    let destination = if path == "/settings" {
        "/settings/providers"
    } else {
        path
    };
    let body = format!(
        "const a = [...document.querySelectorAll('.workspace-rail a')].find((n) => n.getAttribute('href') === {path:?});\
         if (!a) return false; a.click();\
         let selected = false;\
         for (let i = 0; i < 50; i++) {{\
           if ({path:?} === '/settings' && location.pathname === '/settings' && !selected) {{\
             const tab = [...document.querySelectorAll('[data-settings-shell] [role=\"tab\"]')].find((n) => n.textContent.trim() === 'Provider');\
             if (tab) {{ tab.dispatchEvent(new MouseEvent('mousedown', {{ bubbles: true, button: 0 }})); tab.click(); selected = true; }}\
           }}\
           if (location.pathname === {destination:?}) return true;\
           await new Promise((r) => setTimeout(r, 100));\
         }}\
         return false;"
    );
    js(app, &body).await == json!(true)
}

/// Settings categories activate on Radix's primary mousedown; click alone does not select a tab.
async fn select_settings(app: &AppHandle, section: &str, label: &str) -> bool {
    let path = format!("/settings/{section}");
    let body = format!(
        "const tab = [...document.querySelectorAll('[data-settings-shell] [role=\"tab\"]')].find((n) => n.textContent.trim() === {label:?});\
         if (!tab || tab.disabled) return false;\
         tab.dispatchEvent(new MouseEvent('mousedown', {{ bubbles: true, button: 0 }})); tab.click();\
         for (let i = 0; i < 50; i++) {{\
           if (location.pathname === {path:?} && tab.getAttribute('aria-selected') === 'true') return true;\
           await new Promise((r) => setTimeout(r, 100));\
         }}\
         return false;"
    );
    js(app, &body).await == json!(true)
}

async fn scalar(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(db.get_database_backend(), sql))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "n")
        .unwrap()
}

async fn denied_count(db: &DatabaseConnection, module: &str, permission: &str) -> i64 {
    scalar(
        db,
        &format!(
            "SELECT COUNT(*) AS n FROM module_permission_audits WHERE action = 'denied' AND module_id = '{module}' AND permission = '{permission}'"
        ),
    )
    .await
}

async fn action_count(db: &DatabaseConnection, module: &str, action: &str) -> i64 {
    scalar(
        db,
        &format!(
            "SELECT COUNT(*) AS n FROM module_permission_audits WHERE action = '{action}' AND module_id = '{module}' AND user_id = '{USER}'"
        ),
    )
    .await
}

async fn storage_rows(db: &DatabaseConnection, module: &str) -> i64 {
    scalar(
        db,
        &format!("SELECT COUNT(*) AS n FROM module_storage WHERE module_id = '{module}'"),
    )
    .await
}

/// (module_id, permission, action) of the local user's audit rows, newest first.
async fn audit_rows(
    db: &DatabaseConnection,
    module: Option<&str>,
) -> Vec<(String, String, String)> {
    let filter = module
        .map(|m| format!(" AND module_id = '{m}'"))
        .unwrap_or_default();
    db.query_all(Statement::from_string(
        db.get_database_backend(),
        format!(
            "SELECT module_id, permission, action FROM module_permission_audits WHERE user_id = '{USER}'{filter} ORDER BY rowid DESC"
        ),
    ))
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        let text = |name: &str| row.try_get::<String>("", name).unwrap();
        (text("module_id"), text("permission"), text("action"))
    })
    .collect()
}

/// Every table's rows as `table\tcolumn|column|...` lines, except the tables named in `skip`.
async fn dump_tables(db: &DatabaseConnection, skip: &[&str]) -> String {
    let backend = db.get_database_backend();
    let tables = db
        .query_all(Statement::from_string(
            backend,
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        ))
        .await
        .unwrap();
    let mut out = String::new();
    for row in tables {
        let table: String = row.try_get("", "name").unwrap();
        if skip.contains(&table.as_str()) {
            continue;
        }
        let Ok(columns) = db
            .query_all(Statement::from_string(
                backend,
                format!("SELECT name FROM pragma_table_info('{table}')"),
            ))
            .await
        else {
            continue;
        };
        let expression = columns
            .iter()
            .map(|c| {
                let name: String = c.try_get("", "name").unwrap();
                format!("COALESCE(CAST(\"{name}\" AS TEXT), '')")
            })
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        if expression.is_empty() {
            continue;
        }
        let Ok(rows) = db
            .query_all(Statement::from_string(
                backend,
                format!("SELECT {expression} AS line FROM \"{table}\""),
            ))
            .await
        else {
            continue;
        };
        for r in rows {
            let line: String = r.try_get("", "line").unwrap_or_default();
            out.push_str(&format!("{table}\t{line}\n"));
        }
    }
    out
}

async fn run_op(app: &AppHandle, command: &str, module: &str) -> Value {
    let reply = ipc(app, MAIN, command, json!({"moduleId": module})).await;
    match reply.data()["operation_id"].as_str() {
        Some(operation) if reply.success() => poll_operation(app, MAIN, operation).await,
        _ => json!({"state": "rejected", "reply": reply.describe()}),
    }
}

async fn listed(app: &AppHandle) -> Vec<Value> {
    ipc(app, MAIN, "list_modules", json!({}))
        .await
        .data()
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn module_view<'a>(all: &'a [Value], id: &str) -> Option<&'a Value> {
    all.iter().find(|m| m["module_id"] == id)
}

fn granted_permissions(view: &Value) -> Vec<String> {
    let mut granted: Vec<String> = view["permissions"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e["granted"] == true)
                .filter_map(|e| e["permission"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    granted.sort();
    granted
}

async fn grant(app: &AppHandle, module: &str, permission: &str) -> bool {
    ipc(
        app,
        MAIN,
        "grant_module_permission",
        json!({"moduleId": module, "permission": permission}),
    )
    .await
    .success()
}

async fn revoke(app: &AppHandle, module: &str, permission: &str) -> bool {
    ipc(
        app,
        MAIN,
        "revoke_module_permission",
        json!({"moduleId": module, "permission": permission}),
    )
    .await
    .success()
}

/// Calls from `module` until one reaches host authorization (a denied storage.read row exists).
async fn wait_route(app: &AppHandle, db: &DatabaseConnection, module: &str) -> bool {
    for _ in 0..100 {
        let _ = cap(app, module, "storageGet", json!(["probe"])).await;
        if denied_count(db, module, "storage.read").await > 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Tool calls the MCP fixture server actually started, read through a counting call that is not itself
/// counted (`cancellation-observable` mode).
async fn tool_started(world: &CapWorld) -> i64 {
    let mut request = CallMcpToolRequest {
        server_id: world.server_id.clone(),
        tool_name: "echo".into(),
        arguments: json!({"text": "fixture-counts"}),
        call_id: Some(uuid::Uuid::new_v4().to_string()),
        task_ref: None,
        timeout_ms: Some(10_000),
        confirmation_token: None,
    };
    request.confirmation_token = Some(world.mcp.confirm_call(USER, &request).await.unwrap());
    let response = world.mcp.call_tool(USER, request).await.unwrap();
    let text = response.content[0]["text"].as_str().unwrap_or("{}");
    serde_json::from_str::<Value>(text).unwrap()["started"]
        .as_i64()
        .unwrap_or(-1)
}

async fn module_mcp_logs(world: &CapWorld) -> usize {
    world
        .mcp
        .list_usage_logs(USER, Some(&world.server_id), Some(100))
        .await
        .unwrap()
        .iter()
        .filter(|l| l.caller.as_deref() == Some("module"))
        .count()
}

/// Walk the audit command page by page.
async fn walk(app: &AppHandle, module: Option<&str>, limit: u32) -> (Vec<Value>, Vec<usize>, bool) {
    let mut all = Vec::new();
    let mut sizes = Vec::new();
    let mut before = Value::Null;
    for _ in 0..1000 {
        let reply = ipc(
            app,
            MAIN,
            "list_module_permission_audits",
            json!({"moduleId": module, "before": before, "limit": limit}),
        )
        .await;
        if !reply.success() {
            return (all, sizes, false);
        }
        let entries = reply.data()["entries"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        sizes.push(entries.len());
        all.extend(entries);
        match reply.data()["next_before"].as_str() {
            Some(cursor) => before = json!(cursor),
            None => return (all, sizes, true),
        }
    }
    (all, sizes, false)
}

fn entry_triples(entries: &[Value]) -> Vec<(String, String, String)> {
    entries
        .iter()
        .map(|e| {
            let text = |name: &str| e[name].as_str().unwrap_or("").to_owned();
            (text("module_id"), text("permission"), text("action"))
        })
        .collect()
}

/// Click the enabled button `label` inside the module's card, waiting for it: the page disables its
/// buttons while an action is still running.
async fn click_in_card(app: &AppHandle, module: &str, label: &str) -> bool {
    let body = format!(
        "for (let i = 0; i < 80; i++) {{\
           const card = document.querySelector('[data-testid=\"module-{module}\"]');\
           const b = card && [...card.querySelectorAll('button')].find((n) => n.textContent.trim() === {label:?} && !n.disabled);\
           if (b) {{ b.click(); return true; }}\
           await new Promise((r) => setTimeout(r, 100));\
         }}\
         return false;"
    );
    js(app, &body).await == json!(true)
}

/// Same for a button found by its `aria-label`.
async fn click_labelled(app: &AppHandle, module: &str, aria_label: &str) -> bool {
    let body = format!(
        "for (let i = 0; i < 80; i++) {{\
           const card = document.querySelector('[data-testid=\"module-{module}\"]');\
           const b = card && [...card.querySelectorAll('button')].find((n) => n.getAttribute('aria-label') === {aria_label:?} && !n.disabled);\
           if (b) {{ b.click(); return true; }}\
           await new Promise((r) => setTimeout(r, 100));\
         }}\
         return false;"
    );
    js(app, &body).await == json!(true)
}

fn card_js(module: &str) -> String {
    format!("const card = document.querySelector('[data-testid=\"module-{module}\"]');")
}

// -- Scenario ----------------------------------------------------------------------------------------

async fn scenario(app: AppHandle, w: CapWorld, ready: bool) {
    let mut checks = Checks::default();
    if !ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks, &w.root);
    }
    let bridge = app.state::<ModuleHostBridge>();
    let attached = wait_until(|| bridge.loader().has_attachment(MAIN), "page attach").await;
    checks.check(
        "the page attaches to the module host at application start",
        attached.is_ok(),
        format!("{attached:?}"),
    );
    checks.check(
        "host recovery succeeds",
        w.manager.recover().await.is_ok(),
        "recover() failed",
    );
    let base = format!("http://127.0.0.1:{}", w.http_port);

    // -- Start both modules through start_module; wait for their routes ----------------------------
    let started_granted = run_op(&app, "start_module", GRANTED).await;
    let started_ungranted = run_op(&app, "start_module", UNGRANTED).await;
    checks.check(
        "both fixture modules start through the real loader",
        started_granted["outcome"] == "started" && started_ungranted["outcome"] == "started",
        format!("{started_granted} {started_ungranted}"),
    );
    let route_granted = wait_route(&app, &w.db, GRANTED).await;
    let route_ungranted = wait_route(&app, &w.db, UNGRANTED).await;
    checks.check(
        "both modules receive a route: a call reaches host authorization",
        route_granted && route_ungranted,
        format!("granted={route_granted} ungranted={route_ungranted}"),
    );

    // -- [1] Every capability from the ungranted module is denied and audited -----------------------
    let started0 = tool_started(&w).await;
    let caps: Vec<(&str, Value, &str)> = vec![
        ("storageGet", json!(["k"]), "storage.read"),
        ("storageList", json!([]), "storage.read"),
        ("storageSet", json!(["k", 1]), "storage.write"),
        ("storageDelete", json!(["k"]), "storage.write"),
        (
            "networkRequest",
            json!([{"url": format!("{base}/ping")}]),
            "network.local",
        ),
        (
            "mcpCallTool",
            json!([w.server_id, "echo", {"text": "x"}]),
            "mcp.call",
        ),
        ("skillsPrepare", json!([SKILL]), "skills.execute"),
    ];
    for (method, args, permission) in &caps {
        let before = denied_count(&w.db, UNGRANTED, permission).await;
        let out = cap(&app, UNGRANTED, method, args.clone()).await;
        let after = denied_count(&w.db, UNGRANTED, permission).await;
        checks.check(
            &format!("[1] ungranted {method} answers 1004004 and writes a denied {permission} audit row (AC04/AC05)"),
            out.denied() && after == before + 1,
            format!("{out:?} before={before} after={after}"),
        );
    }
    checks.check(
        "[1] storage.read denial: the ungranted module reads nothing and stores nothing (AC12)",
        storage_rows(&w.db, UNGRANTED).await == 0,
        "rows exist",
    );
    checks.check(
        "[1] denied calls had no side effects: no HTTP connection, no MCP tool started, no dialog, no skill log",
        w.http.connections() == 0
            && tool_started(&w).await == started0
            && module_mcp_logs(&w).await == 0
            && !dialog_open(&app).await
            && app
                .state::<commands::SkillsState>()
                .manager
                .read()
                .await
                .list_usage_logs(USER, Some(100))
                .await
                .unwrap()
                .iter()
                .all(|l| l.caller.as_deref() != Some("module")),
        format!("connections={}", w.http.connections()),
    );

    // -- [2] Storage: grants, round trip, validation, quota ----------------------------------------
    let before_audits = action_count(&w.db, GRANTED, "granted").await;
    checks.check(
        "[2] grant_module_permission(storage.read) succeeds from the settings page and audits a granted row",
        grant(&app, GRANTED, "storage.read").await
            && action_count(&w.db, GRANTED, "granted").await == before_audits + 1,
        "grant failed",
    );
    let get = cap(&app, GRANTED, "storageGet", json!(["a"])).await;
    let list = cap(&app, GRANTED, "storageList", json!([])).await;
    let set_denied = cap(
        &app,
        GRANTED,
        "storageSet",
        json!(["a", {"n": 1, "s": SECRET_STORE}]),
    )
    .await;
    let get_after = cap(&app, GRANTED, "storageGet", json!(["a"])).await;
    checks.check(
        "[2] with storage.read only: get/list succeed, set is denied and stores nothing (AC03)",
        get.ok
            && get.value.is_null()
            && list.ok
            && list.value == json!([])
            && set_denied.denied()
            && get_after.ok
            && get_after.value.is_null()
            && storage_rows(&w.db, GRANTED).await == 0,
        format!("{get:?} {list:?} {set_denied:?} {get_after:?}"),
    );
    checks.check(
        "[2] grant_module_permission(storage.write) succeeds",
        grant(&app, GRANTED, "storage.write").await,
        "grant failed",
    );
    let value_a = json!({"n": 1, "s": SECRET_STORE});
    let set_b = cap(&app, GRANTED, "storageSet", json!(["b", [1, 2]])).await;
    let set_a = cap(&app, GRANTED, "storageSet", json!(["a", value_a])).await;
    let get_a = cap(&app, GRANTED, "storageGet", json!(["a"])).await;
    let list_two = cap(&app, GRANTED, "storageList", json!([])).await;
    let size_a = serde_json::to_string(&value_a).unwrap().len();
    checks.check(
        "[2] storage set/get/list round trip: value returned exactly, list is key-ascending with sizes and no values (AC03)",
        set_a.ok
            && set_b.ok
            && get_a.value == value_a
            && list_two.value == json!([{"key": "a", "size_bytes": size_a}, {"key": "b", "size_bytes": 5}])
            && !list_two.value.to_string().contains(SECRET_STORE),
        format!("{set_a:?} {set_b:?} {get_a:?} {list_two:?}"),
    );
    checks.check(
        "[2] the stored value belongs to the calling module only: the other module reads nothing",
        storage_rows(&w.db, UNGRANTED).await == 0 && storage_rows(&w.db, GRANTED).await == 2,
        "storage not scoped per module",
    );
    let delete_a = cap(&app, GRANTED, "storageDelete", json!(["a"])).await;
    let delete_again = cap(&app, GRANTED, "storageDelete", json!(["a"])).await;
    let get_gone = cap(&app, GRANTED, "storageGet", json!(["a"])).await;
    let list_one = cap(&app, GRANTED, "storageList", json!([])).await;
    checks.check(
        "[2] storage delete is idempotent and removes the key",
        delete_a.ok
            && delete_again.ok
            && get_gone.ok
            && get_gone.value.is_null()
            && list_one.value == json!([{"key": "b", "size_bytes": 5}]),
        format!("{delete_a:?} {delete_again:?} {get_gone:?} {list_one:?}"),
    );
    let empty_key = cap(&app, GRANTED, "storageSet", json!(["", 1])).await;
    let long_key = cap(&app, GRANTED, "storageSet", json!(["k".repeat(129), 1])).await;
    let big_value = cap(&app, GRANTED, "storageFill", json!(["big", 70_000, 1])).await;
    checks.check(
        "[2] an empty key, a 129-character key and a value over 64 KiB answer 1004024",
        empty_key.failed_with(INVALID)
            && long_key.failed_with(INVALID)
            && big_value.ok
            && big_value.value["written"] == 0
            && big_value.value["code"] == INVALID,
        format!("{empty_key:?} {long_key:?} {big_value:?}"),
    );
    let fill = cap(&app, GRANTED, "storageFill", json!(["q", 60_000, 100])).await;
    let written = fill.value["written"].as_u64().unwrap_or(0);
    checks.check(
        "[2] the 5 MiB per-module quota answers 1004025 after about 87 values of 60 KB",
        fill.ok && fill.value["code"] == QUOTA && (80..100).contains(&written),
        format!("{fill:?}"),
    );
    let cleanup = js(
        &app,
        &format!(
            "const c = globalThis.__dgosCapabilityCallers[{GRANTED:?}];\
             for (let i = 0; i < {written}; i++) await c.storageDelete('q-' + i);\
             return (await c.storageList()).value;"
        ),
    )
    .await;
    let set_after_quota = cap(&app, GRANTED, "storageSet", json!(["c", "ok"])).await;
    let _ = cap(&app, GRANTED, "storageDelete", json!(["c"])).await;
    checks.check(
        "[2] deleting the fill frees the quota: only key b remains and a new set succeeds",
        cleanup == json!([{"key": "b", "size_bytes": 5}]) && set_after_quota.ok,
        format!("{cleanup} {set_after_quota:?}"),
    );

    // -- [3] network.local ---------------------------------------------------------------------------
    checks.check(
        "[3] grant_module_permission(network.local) succeeds",
        grant(&app, GRANTED, "network.local").await,
        "grant failed",
    );
    let connections0 = w.http.connections();
    let ping = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": format!("{base}/ping/{SECRET_PATH}")}]),
    )
    .await;
    checks.check(
        "[3] a loopback GET succeeds with status, content type and a text body",
        ping.ok
            && ping.value["status"] == 200
            && ping.value["content_type"] == "text/plain"
            && ping.value["body"]["encoding"] == "text"
            && ping.value["body"]["data"] == format!("GET /ping/{SECRET_PATH} ")
            && w.http.connections() == connections0 + 1,
        format!("{ping:?}"),
    );
    let post = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": format!("{base}/post"), "method": "POST", "headers": {"content-type": "text/plain"}, "body": SECRET_BODY}]),
    )
    .await;
    {
        let seen = w.http.requests.lock().unwrap();
        let last = seen.last();
        checks.check(
            "[3] a loopback POST delivers the body and sends no cookie, authorization or host credential",
            post.ok
                && last.is_some_and(|r| {
                    r.method == "POST"
                        && r.path == "/post"
                        && r.body == SECRET_BODY
                        && !r.headers.contains("cookie:")
                        && !r.headers.contains("authorization:")
                }),
            format!("{post:?}"),
        );
    }
    let redirect = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": format!("{base}/redirect")}]),
    )
    .await;
    checks.check(
        "[3] a redirect is returned as is and not followed",
        redirect.ok
            && redirect.value["status"] == 302
            && !w
                .http
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.path == "/landed"),
        format!("{redirect:?}"),
    );
    let big = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": format!("{base}/big")}]),
    )
    .await;
    checks.check(
        "[3] a response over 1 MiB answers 1004027",
        big.failed_with(TOO_LARGE),
        format!("{big:?}"),
    );
    let connections_before_refused = w.http.connections();
    let started_at = Instant::now();
    let lan = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": "http://10.255.255.1/"}]),
    )
    .await;
    let metadata = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": "http://169.254.169.254/latest"}]),
    )
    .await;
    checks.check(
        "[3] non-loopback targets answer 1004026 at once and the fixture saw no extra connection",
        lan.failed_with(REFUSED)
            && metadata.failed_with(REFUSED)
            && started_at.elapsed() < Duration::from_secs(5)
            && w.http.connections() == connections_before_refused,
        format!("{lan:?} {metadata:?} {:?}", started_at.elapsed()),
    );
    let ftp = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": format!("ftp://127.0.0.1:{}/", w.http_port)}]),
    )
    .await;
    let put = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": format!("{base}/put"), "method": "PUT"}]),
    )
    .await;
    let header = cap(
        &app,
        GRANTED,
        "networkRequest",
        json!([{"url": format!("{base}/hdr"), "headers": {"authorization": "Bearer x"}}]),
    )
    .await;
    checks.check(
        "[3] a non-HTTP scheme, a PUT and a forbidden header answer 1004024 without a connection",
        ftp.failed_with(INVALID)
            && put.failed_with(INVALID)
            && header.failed_with(INVALID)
            && w.http.connections() == connections_before_refused,
        format!("{ftp:?} {put:?} {header:?}"),
    );

    // -- [4] skills.prepare --------------------------------------------------------------------------
    checks.check(
        "[4] grant_module_permission(skills.execute) succeeds",
        grant(&app, GRANTED, "skills.execute").await,
        "grant failed",
    );
    let skill = cap(&app, GRANTED, "skillsPrepare", json!([SKILL])).await;
    let skill_text = skill.value.to_string();
    checks.check(
        "[4] skills.prepare returns the installed official skill's pinned content without any local path",
        skill.ok
            && skill.value["skill_id"] == SKILL
            && skill.value["version"].as_str().is_some_and(|v| !v.is_empty())
            && skill.value["content_hash"].as_str().is_some_and(|v| !v.is_empty())
            && skill.value["body"].as_str().is_some_and(|v| !v.is_empty())
            && !skill_text.contains(env!("CARGO_MANIFEST_DIR"))
            && !skill_text.contains("/Users/"),
        format!("{:?}", skill.value["skill_id"]),
    );
    let unknown = cap(
        &app,
        GRANTED,
        "skillsPrepare",
        json!(["dgos.official.not-installed"]),
    )
    .await;
    checks.check(
        "[4] an unknown skill keeps a 1002xxx code",
        !unknown.ok
            && unknown
                .code
                .is_some_and(|c| (1002000..1003000).contains(&c)),
        format!("{unknown:?}"),
    );
    let skill_logs = app
        .state::<commands::SkillsState>()
        .manager
        .read()
        .await
        .list_usage_logs(USER, Some(100))
        .await
        .unwrap();
    checks.check(
        "[4] the skill usage log attributes the call to the module caller",
        skill_logs
            .iter()
            .any(|l| l.caller.as_deref() == Some("module") && l.skill_id == SKILL),
        format!("{} logs", skill_logs.len()),
    );

    // -- [5] mcp.callTool through the host dialog -----------------------------------------------------
    checks.check(
        "[5] grant_module_permission(mcp.call) succeeds",
        grant(&app, GRANTED, "mcp.call").await,
        "grant failed",
    );
    // The call and the confirmation run on the Home route: the module and the dialog both outlive SPA
    // navigation, so neither may depend on the Settings route (D093). A Settings-gated command is refused
    // there, which shows the window really left Settings.
    let on_home = navigate(&app, "/").await;
    let settings_gate = ipc(&app, MAIN, "list_modules", json!({})).await;
    checks.check(
        "[5] SPA navigation to / leaves Settings: list_modules is refused with 1004004 there",
        on_home && !settings_gate.success() && settings_gate.code() == Some(1_004_004),
        format!("on_home={on_home} {}", settings_gate.describe()),
    );
    let started_before = tool_started(&w).await;
    let logs_before = module_mcp_logs(&w).await;
    begin(
        &app,
        "reject",
        GRANTED,
        "mcpCallTool",
        json!([w.server_id, "echo", {"text": SECRET_MCP_REJECT}]),
    )
    .await;
    let dialog = wait_dialog(&app).await;
    let shown_args: Value = dialog["args"]
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);
    let dialog_text = dialog["text"].as_str().unwrap_or("").to_owned();
    checks.check(
        "[5] on /, the confirm dialog appears in the real DOM outside every module container, naming module, server and tool and showing the exact arguments",
        !dialog.is_null()
            && dialog["insideModule"] == false
            && dialog_text.contains("Capability Granted")
            && dialog_text.contains(&w.server_id)
            && dialog_text.contains("echo")
            && shown_args == json!({"text": SECRET_MCP_REJECT})
            && dialog["buttons"]
                .as_array()
                .is_some_and(|b| b.contains(&json!("拒绝")) && b.contains(&json!("批准"))),
        dialog.to_string(),
    );
    let clicked = click_dialog(&app, "拒绝").await;
    let rejected = finished(&app, "reject").await;
    checks.check(
        "[5] on /, clicking Reject answers 1003019 and closes the dialog",
        clicked
            && rejected
                .as_ref()
                .is_some_and(|o| o.failed_with(MCP_NOT_CONFIRMED))
            && dialog_gone(&app).await,
        format!("{rejected:?}"),
    );
    checks.check(
        "[5] a rejected call never reached the tool and left no MCP usage log",
        tool_started(&w).await == started_before && module_mcp_logs(&w).await == logs_before,
        "the fixture tool was called",
    );
    begin(
        &app,
        "approve",
        GRANTED,
        "mcpCallTool",
        json!([w.server_id, "echo", {"text": SECRET_MCP_OK}]),
    )
    .await;
    let dialog = wait_dialog(&app).await;
    let clicked = click_dialog(&app, "批准").await;
    let approved = finished(&app, "approve").await;
    checks.check(
        "[5] on /, clicking Approve returns the tool result",
        !dialog.is_null()
            && clicked
            && approved.as_ref().is_some_and(|o| {
                o.ok && o.value["is_error"] == false
                    && o.value["content"][0]["text"] == SECRET_MCP_OK
            }),
        format!("{approved:?}"),
    );
    checks.check(
        "[5] the approved call started the tool once and is logged with caller=module",
        tool_started(&w).await == started_before + 1
            && module_mcp_logs(&w).await == logs_before + 1,
        "tool or log count wrong",
    );
    let back = navigate(&app, "/settings").await;
    checks.check(
        "[5] SPA navigation back through Settings to /settings/providers for the management steps",
        back,
        "navigation failed",
    );

    // -- [6] Revocation (AC07) ------------------------------------------------------------------------
    let revocations: Vec<(&str, Vec<(&str, Value, &str)>)> = vec![
        (
            "storage.read",
            vec![
                ("storageGet", json!(["b"]), "storage.read"),
                ("storageList", json!([]), "storage.read"),
            ],
        ),
        (
            "storage.write",
            vec![
                ("storageSet", json!(["z", 1]), "storage.write"),
                ("storageDelete", json!(["b"]), "storage.write"),
            ],
        ),
        (
            "network.local",
            vec![(
                "networkRequest",
                json!([{"url": format!("{base}/ping")}]),
                "network.local",
            )],
        ),
        (
            "skills.execute",
            vec![("skillsPrepare", json!([SKILL]), "skills.execute")],
        ),
    ];
    let connections_before_revoked = w.http.connections();
    for (permission, calls) in &revocations {
        let revoked = revoke(&app, GRANTED, permission).await;
        for (method, args, needed) in calls {
            let before = denied_count(&w.db, GRANTED, needed).await;
            let out = cap(&app, GRANTED, method, args.clone()).await;
            let after = denied_count(&w.db, GRANTED, needed).await;
            checks.check(
                &format!("[6] after revoking {permission} the next {method} is denied and audited (AC07)"),
                revoked && out.denied() && after == before + 1,
                format!("{out:?} before={before} after={after}"),
            );
        }
    }
    checks.check(
        "[6] the revoked network call made no connection and the revoked delete removed nothing",
        w.http.connections() == connections_before_revoked
            && storage_rows(&w.db, GRANTED).await == 1,
        "side effect after revoke",
    );
    // MCP: revoke while the dialog is open; approving still completes that call, the next one is denied.
    let started_before = tool_started(&w).await;
    begin(
        &app,
        "late",
        GRANTED,
        "mcpCallTool",
        json!([w.server_id, "echo", {"text": SECRET_MCP_LATE}]),
    )
    .await;
    let dialog = wait_dialog(&app).await;
    let revoked = revoke(&app, GRANTED, "mcp.call").await;
    let clicked = click_dialog(&app, "批准").await;
    let late = finished(&app, "late").await;
    checks.check(
        "[6] revoking mcp.call while the dialog is open does not cancel that call: approving completes it",
        !dialog.is_null()
            && revoked
            && clicked
            && late.as_ref().is_some_and(|o| o.ok && o.value["content"][0]["text"] == SECRET_MCP_LATE)
            && tool_started(&w).await == started_before + 1,
        format!("{late:?}"),
    );
    let before = denied_count(&w.db, GRANTED, "mcp.call").await;
    let next = cap(
        &app,
        GRANTED,
        "mcpCallTool",
        json!([w.server_id, "echo", {"text": "after"}]),
    )
    .await;
    let after = denied_count(&w.db, GRANTED, "mcp.call").await;
    checks.check(
        "[6] the next mcp.callTool after the revoke is denied, audited, opens no dialog and starts no tool",
        next.denied()
            && after == before + 1
            && !dialog_open(&app).await
            && tool_started(&w).await == started_before + 1,
        format!("{next:?}"),
    );
    checks.check(
        "[6] the audit holds one granted and one revoked row per granted/revoked permission",
        action_count(&w.db, GRANTED, "granted").await == 5
            && action_count(&w.db, GRANTED, "revoked").await == 5,
        format!(
            "granted={} revoked={}",
            action_count(&w.db, GRANTED, "granted").await,
            action_count(&w.db, GRANTED, "revoked").await
        ),
    );

    // -- [7] list_module_permission_audits (AC05/AC08) -------------------------------------------------
    let foreign = w
        .db
        .execute(Statement::from_string(
            w.db.get_database_backend(),
            format!(
                "INSERT INTO module_permission_audits (id, user_id, module_id, permission, action, created_at) VALUES ('foreign-audit-1', 'other-user', '{GRANTED}', 'storage.read', 'denied', 4102444800)"
            ),
        ))
        .await
        .is_ok();
    let expected = audit_rows(&w.db, Some(GRANTED)).await;
    let (entries, _, complete) = walk(&app, Some(GRANTED), 200).await;
    let actions: std::collections::BTreeSet<String> = entries
        .iter()
        .filter_map(|e| e["action"].as_str().map(str::to_owned))
        .collect();
    checks.check(
        "[7] the command returns only this user's rows for the module, newest first, with granted/revoked/denied entries and no request or resource id",
        foreign
            && complete
            && entry_triples(&entries) == expected
            && expected.len() > 15
            && actions.contains("granted")
            && actions.contains("revoked")
            && actions.contains("denied")
            && entries.iter().all(|e| {
                e["module_id"] == GRANTED
                    && e.as_object().is_some_and(|o| {
                        let mut keys: Vec<&str> = o.keys().map(String::as_str).collect();
                        keys.sort_unstable();
                        keys == ["action", "created_at", "module_id", "permission"]
                    })
            })
            && entries.windows(2).all(|p| p[0]["created_at"].as_i64() >= p[1]["created_at"].as_i64()),
        format!("complete={complete} rows={} expected={}", entries.len(), expected.len()),
    );
    let (paged, page_sizes, complete) = walk(&app, Some(GRANTED), 2).await;
    checks.check(
        "[7] next_before paging with limit=2 walks every row exactly once, in order, and ends with no cursor",
        complete
            && entry_triples(&paged) == expected
            && page_sizes.len() == expected.len().div_ceil(2)
            && page_sizes.iter().all(|n| *n <= 2)
            && page_sizes[..page_sizes.len() - 1].iter().all(|n| *n == 2),
        format!("pages={page_sizes:?} rows={} expected={}", paged.len(), expected.len()),
    );
    let all_expected = audit_rows(&w.db, None).await;
    let (everything, _, complete) = walk(&app, None, 200).await;
    checks.check(
        "[7] without a module filter the command returns the local user's rows of every module and none of another user's",
        complete
            && entry_triples(&everything) == all_expected
            && everything.iter().any(|e| e["module_id"] == UNGRANTED)
            && entries.len() == expected.len(),
        format!("rows={} expected={}", everything.len(), all_expected.len()),
    );

    // -- [8] Redaction ----------------------------------------------------------------------------------
    let dump = dump_tables(&w.db, &["module_storage"]).await;
    let mut leaked = Vec::new();
    for secret in [
        SECRET_STORE,
        SECRET_PATH,
        SECRET_BODY,
        SECRET_MCP_REJECT,
        SECRET_MCP_OK,
        SECRET_MCP_LATE,
    ] {
        if dump.contains(secret) {
            leaked.push(secret);
        }
    }
    checks.check(
        "[8] audit rows, MCP usage logs and skill usage logs (every table except module_storage) hold no storage value, network body or URL path, or MCP argument",
        leaked.is_empty()
            && dump.contains("module_permission_audits\t")
            && dump.contains("mcp_usage_logs\t")
            && dump.contains("skill_usage_logs\t"),
        format!("leaked={leaked:?}"),
    );
    let mcp_logs = w
        .mcp
        .list_usage_logs(USER, Some(&w.server_id), Some(100))
        .await
        .unwrap();
    let skill_logs = app
        .state::<commands::SkillsState>()
        .manager
        .read()
        .await
        .list_usage_logs(USER, Some(100))
        .await
        .unwrap();
    let typed = format!("{mcp_logs:?}{skill_logs:?}");
    checks.check(
        "[8] the typed MCP and skill usage log views hold none of the secret markers either",
        ![
            SECRET_MCP_OK,
            SECRET_MCP_LATE,
            SECRET_MCP_REJECT,
            SECRET_STORE,
            SECRET_PATH,
            SECRET_BODY,
        ]
        .iter()
        .any(|s| typed.contains(s)),
        "marker found in a usage log view",
    );

    // -- [9] Management page ------------------------------------------------------------------------------
    // This host does not register locale commands, so the shipped UI uses its deterministic zh-CN default.
    let opened = select_settings(&app, "modules", "模块").await;
    let page = until(
        &app,
        "return !!document.querySelector('[data-testid=\"module-cap-ui\"]') && !!document.querySelector('[data-testid=\"module-cap-upd\"]');",
        10,
    )
    .await;
    checks.check(
        "[9] the Modules category tab opens the management page listing the fixture modules",
        opened && page,
        format!("opened={opened} page={page}"),
    );

    // Enable consent (AC01/AC02).
    let ui_card = card_js(UI);
    let enable_clicked = click_in_card(&app, UI, "启用").await;
    let consent = until(
        &app,
        &format!("{ui_card} return !!(card && card.querySelector('[role=\"dialog\"]'));"),
        5,
    )
    .await;
    let consent_text = js(
        &app,
        &format!("{ui_card} const d = card && card.querySelector('[role=\"dialog\"]'); return d ? d.textContent : '';"),
    )
    .await;
    let consent_text = consent_text.as_str().unwrap_or("").to_owned();
    checks.check(
        "[9] Enable on a disabled module shows the full permission list before anything is granted (AC01)",
        enable_clicked
            && consent
            && ALL_PERMISSIONS.iter().all(|p| consent_text.contains(p))
            && module_view(&listed(&app).await, UI).is_some_and(|m| {
                m["enabled"] == false && granted_permissions(m).is_empty()
            }),
        consent_text.clone(),
    );
    let cancel = click_in_card(&app, UI, "取消").await;
    let consent_closed = until(
        &app,
        &format!("{ui_card} return !!card && !card.querySelector('[role=\"dialog\"]');"),
        5,
    )
    .await;
    let after_cancel = listed(&app).await;
    checks.check(
        "[9] Cancel closes the consent panel and leaves the module disabled with no grant and no audit row (AC02)",
        cancel
            && consent_closed
            && module_view(&after_cancel, UI).is_some_and(|m| {
                m["enabled"] == false && granted_permissions(m).is_empty() && m["runtime"]["state"] != "ready"
            })
            && action_count(&w.db, UI, "granted").await == 0,
        format!("{:?}", module_view(&after_cancel, UI)),
    );
    let again = click_in_card(&app, UI, "启用").await;
    let boxed = until(
        &app,
        &format!("{ui_card} return !!(card && card.querySelector('[role=\"dialog\"] [role=\"checkbox\"]'));"),
        5,
    )
    .await;
    let ticked = js(
        &app,
        &format!("{ui_card} const checkbox = card && card.querySelector('[role=\"dialog\"] [role=\"checkbox\"]'); if (!checkbox || checkbox.disabled) return false; checkbox.click(); return true;"),
    )
    .await
        == json!(true);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let confirmed = click_in_card(&app, UI, "确认启用").await;
    let mut view = Value::Null;
    for _ in 0..150 {
        let all = listed(&app).await;
        if let Some(m) = module_view(&all, UI) {
            view = m.clone();
            if m["enabled"] == true
                && m["runtime"]["state"] == "ready"
                && granted_permissions(m).len() == ALL_PERMISSIONS.len()
            {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    checks.check(
        "[9] Confirm enable enables the module, grants all six permissions and starts the runtime (AC01)",
        again && boxed && ticked && confirmed
            && view["enabled"] == true
            && view["runtime"]["state"] == "ready"
            && granted_permissions(&view).len() == ALL_PERMISSIONS.len()
            && action_count(&w.db, UI, "granted").await == 6,
        view.to_string(),
    );

    // Revoke and permission log (AC06/AC08).
    let revoke_clicked = click_labelled(&app, UI, "撤销 Capability UI 的 storage.read").await;
    let flipped = until(
        &app,
        &format!(
            "{ui_card} if (!card) return false;\
             const li = [...card.querySelectorAll('li')].find((n) => (n.querySelector('code') || {{}}).textContent === 'storage.read');\
             return !!li && li.textContent.includes('未授予') && !!card.querySelector('button[aria-label=\"向 Capability UI 授予 storage.read\"]');"
        ),
        5,
    )
    .await;
    let after_revoke = listed(&app).await;
    checks.check(
        "[9] Revoke flips the permission to Not granted (Grant appears) and the host agrees (AC06)",
        revoke_clicked
            && flipped
            && module_view(&after_revoke, UI).is_some_and(|m| {
                let granted = granted_permissions(m);
                granted.len() == 5 && !granted.contains(&"storage.read".to_owned())
            })
            && action_count(&w.db, UI, "revoked").await == 1,
        format!("{:?}", module_view(&after_revoke, UI)),
    );
    let log_clicked = click_in_card(&app, UI, "权限日志").await;
    let log_shown = until(
        &app,
        &format!(
            "{ui_card} const r = card && card.querySelector('[role=\"region\"][aria-label=\"权限日志\"]');\
             return !!r && r.textContent.includes('granted') && r.textContent.includes('revoked') && r.textContent.includes('storage.read');"
        ),
        5,
    )
    .await;
    checks.check(
        "[9] the Permission log lists the granted and revoked entries of the module (AC08)",
        log_clicked && log_shown,
        "permission log missing entries",
    );
    // The enabled module's own call works through the page it was started from (AC10 end to end).
    let route_ui = wait_route(&app, &w.db, UI).await;
    let ui_set = cap(&app, UI, "storageSet", json!(["ui", 1])).await;
    checks.check(
        "[9] the module enabled through the page is running: storage.write works, the revoked storage.read is denied (AC10)",
        route_ui && ui_set.ok && cap(&app, UI, "storageGet", json!(["ui"])).await.denied(),
        format!("{ui_set:?}"),
    );

    // Update permission diff (AC09): v1 declares storage.read/write, v2 declares storage.read/network.local.
    let granted_both =
        grant(&app, UPD, "storage.read").await && grant(&app, UPD, "storage.write").await;
    // An update needs a ready runtime to replace.
    let started_upd = run_op(&app, "start_module", UPD).await;
    checks.check(
        "[9] the module to be updated is running before the update",
        started_upd["outcome"] == "started",
        started_upd.to_string(),
    );
    let v2_dir = w.sources.join(UPD);
    let _ = fs::remove_dir_all(&v2_dir);
    let digest = write_cap_package(
        &v2_dir,
        UPD,
        "Capability Update",
        "2.0.0",
        &["storage.read", "network.local"],
        &format!("{CALLER}\n// release 2.0.0\n"),
    );
    w.pins
        .lock()
        .unwrap()
        .push((UPD.to_owned(), "2.0.0".to_owned(), digest));
    // Modules is not kept alive: selecting another category and returning remounts its list through the SPA.
    let left_modules = select_settings(&app, "providers", "Provider").await;
    let reopened_modules = select_settings(&app, "modules", "模块").await;
    let card_ready = until(
        &app,
        "const c = document.querySelector('[data-testid=\"module-cap-upd\"]'); return !!c && c.textContent.includes('storage.write') && c.textContent.includes('已授予');",
        10,
    )
    .await;
    let update_clicked = click_in_card(&app, UPD, "更新").await;
    // The first rendered notice must already list both sections; the page's module list may still be
    // the pre-update one at that moment, and that must not hide the newly declared permission.
    let first_notice_js = format!(
        "{} const n = card && [...card.querySelectorAll('[role=\"status\"]')].find((x) => x.textContent.includes('此更新修改了权限。')); return n ? n.textContent : '';",
        card_js(UPD)
    );
    let mut notice_text = String::new();
    for _ in 0..600 {
        notice_text = js(&app, &first_notice_js)
            .await
            .as_str()
            .unwrap_or("")
            .to_owned();
        if !notice_text.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let notice = !notice_text.is_empty();
    let added_at = notice_text.find("新声明但尚未授予：");
    let withdrawn_at = notice_text.find("不再声明，因此已撤销你的授权：");
    let after_update = listed(&app).await;
    checks.check(
        "[9] Update shows a notice listing the newly declared permission (with Grant) and the withdrawn one (AC09)",
        granted_both
            && left_modules
            && reopened_modules
            && card_ready
            && update_clicked
            && notice
            && added_at.is_some_and(|a| notice_text[a..].contains("network.local"))
            && withdrawn_at.is_some_and(|b| notice_text[b..].contains("storage.write"))
            && module_view(&after_update, UPD).is_some_and(|m| {
                m["version"] == "2.0.0" && granted_permissions(m) == ["storage.read"]
            })
            && action_count(&w.db, UPD, "revoked").await == 1,
        format!("{notice_text} {:?}", module_view(&after_update, UPD)),
    );
    let grant_new = click_labelled(
        &app,
        UPD,
        "向 Capability Update 授予新权限 network.local",
    )
    .await;
    let mut granted_now = false;
    for _ in 0..50 {
        granted_now = module_view(&listed(&app).await, UPD)
            .is_some_and(|m| granted_permissions(m) == ["network.local", "storage.read"]);
        if granted_now {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    checks.check(
        "[9] the Grant button beside the new permission grants it",
        grant_new && granted_now,
        "network.local was not granted",
    );

    for module in [GRANTED, UNGRANTED, UI] {
        let _ = run_op(&app, "stop_module", module).await;
    }
    finish(checks, &w.root);
}

pub fn run_capabilities() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        std::process::exit(2);
    });

    let (world, managed) = tauri::async_runtime::block_on(build_world());
    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    // The window is created below so the harness controls the first URL.
    context.config_mut().app.windows.clear();

    let maintenance = world.maintenance.clone();
    let Managed {
        manager,
        bridge,
        router,
        crashes,
        management,
        dispatch,
        storage,
        mcp_service,
        skills,
    } = managed;
    let scenario_world = Mutex::new(Some(world));
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(crate::session::SessionResolver::no_accounts())
        .manage(maintenance)
        .manage(LoadEvents(load_sender))
        .manage(manager)
        .manage(bridge)
        .manage(router)
        .manage(crashes)
        .manage(management)
        .manage(dispatch)
        .manage(storage)
        .manage(mcp_service)
        .manage(skills)
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
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("settings".into()))
                .visible(false)
                .build()?;
            let handle = app.handle().clone();
            let world = scenario_world.lock().unwrap().take().unwrap();
            tauri::async_runtime::spawn(async move {
                let ready = wait_load(events, "main", "/settings").await;
                scenario(handle, world, ready).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_module,
            commands::stop_module,
            commands::update_module,
            commands::get_module_operation,
            commands::get_module_runtime_status,
            commands::enable_module,
            commands::disable_module,
            commands::uninstall_module,
            commands::list_modules,
            commands::grant_module_permission,
            commands::revoke_module_permission,
            commands::list_module_crashes,
            commands::list_module_permission_audits,
            commands::attach_module_host,
            commands::report_module_host,
            commands::report_module_crash,
            commands::module_storage_get,
            commands::module_storage_set,
            commands::module_storage_delete,
            commands::module_storage_list,
            commands::module_network_request,
            commands::module_call_mcp_tool,
            commands::module_prepare_skill,
            commands::resolve_module_mcp_confirmation,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
