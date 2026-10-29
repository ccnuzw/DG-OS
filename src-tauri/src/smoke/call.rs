//! Real module JS calling `providers.call` in a real WebView (D050, D051).
//!
//! Third smoke target. It wires the production pieces: `PageModuleLoader` and
//! `ModuleHostBridge`, the route observer, `ModuleCallRouter`, the typed
//! `module_call_provider` command, `ModuleCapabilityDispatchService`, the real
//! `ProviderManager` and a SQLite database with the real permission
//! repository. The module is `tests/fixtures/modules/provider-caller.js`; its
//! `host.providers.call` goes through the real Tauri IPC.
//!
//! Replaced or shortened for the run: the Provider is a loopback fake server
//! (not a real vendor), the credential vault is in memory, and the host
//! deadline is 1 second instead of the production 120 seconds.
//!
//! What it proves: admission, per-call authorization, revocation, replaced
//! runtimes, page reload, the host deadline, the in-flight limit, error
//! mapping and audit redaction on the platform WebView of the machine that
//! runs it. What it does not: Windows WebView2 or Linux WebKitGTK, a real
//! vendor, a release build with real pins, or any isolation between modules
//! (the realm is shared, D042).

use super::{
    eval, finish, goto, ipc, poll_operation, wait_load, wait_until, write_package_with, Checks,
    LoadEvents, Pending, WATCHDOG,
};
use crate::commands;
use crate::module_host::{self, ChannelTransport, ModuleHostBridge, RoutePublisher};
use dgos_application::{
    HostCapabilityService, ModuleCapabilityDispatchService, ModuleOperationManager,
    ModuleReleaseService,
};
use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::{ModulePermission, ModulePermissionRepository};
use dgos_modules::{
    LoaderTimeouts, LocalModuleSources, ModuleAdmission, ModuleCallRouter, ModulePublisher,
    ModuleRuntimeSupervisor, PageLoaderConfig, PageModuleLoader, PinnedModuleAdmission,
};

const CONSOLE: &str = "provider-console";

/// Test pins for the fixture modules, plus the real compiled-in list. The
/// shipped module is therefore admitted only by its real pin.
struct TestAndBuiltinAdmission {
    test: PinnedModuleAdmission,
    builtin: PinnedModuleAdmission,
}

impl ModuleAdmission for TestAndBuiltinAdmission {
    fn admits(&self, module_id: &str, version: &str, integrity: &str) -> bool {
        self.test.admits(module_id, version, integrity)
            || self.builtin.admits(module_id, version, integrity)
    }
}

/// Fill in and submit the shipped console through its real DOM, then report
/// what it shows. React inputs only see the native value setter plus an
/// `input` event.
async fn drive_console(app: &AppHandle, provider: &str, prompt: &str) -> Value {
    let body = format!(
        "const link = document.querySelector('a[href=\"/settings/modules/provider-console\"]'); if (link && !document.querySelector('[data-module-id=\"provider-console\"]')) link.click();\
         for (let i = 0; i < 50 && !document.querySelector('[data-module-id=\"provider-console\"]'); i++) await new Promise((r) => setTimeout(r, 100));\
         const root = document.querySelector('[data-module-id=\"provider-console\"]');\
         if (!root) return {{missing: true}};\
         const set = (el, v) => {{ const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;\
           Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v); el.dispatchEvent(new Event('input', {{bubbles: true}})); }};\
         const inputs = root.querySelectorAll('input');\
         set(inputs[0], {provider:?}); set(inputs[1], 'fixture-chat'); set(root.querySelector('textarea'), {prompt:?});\
         await new Promise((r) => setTimeout(r, 80));\
         const stale = root.querySelector('[role=\"alert\"]');\
         const staleReply = root.querySelector('[data-testid=\"provider-console-reply\"]');\
         root.querySelector('button').click();\
         for (let i = 0; i < 100; i++) {{\
           const reply = root.querySelector('[data-testid=\"provider-console-reply\"]');\
           const alert = root.querySelector('[role=\"alert\"]');\
           if ((reply && reply !== staleReply) || (alert && alert !== stale)) return {{reply: reply && reply.textContent, alert: alert && alert.textContent}};\
           await new Promise((r) => setTimeout(r, 50));\
         }}\
         return {{timeout: true}};"
    );
    eval(app, MAIN, &body).await["value"].clone()
}
use dgos_providers::test_support::{seed_provider, MemoryCredentialVault};
use dgos_providers::ProviderManager;
use dgos_types::ProviderConfig;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

static NEXT_CALL: AtomicUsize = AtomicUsize::new(1);
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

const CALLER: &str = include_str!("../../../tests/fixtures/modules/provider-caller.js");

const GRANTED: &str = "call-module";
const UNGRANTED: &str = "call-other";
const USER: &str = "default-user";
const MAIN: &str = module_host::MODULE_WINDOW;

const SECRET_PROMPT: &str = "SECRET-PROMPT-9c2e";
const SECRET_KEY: &str = "synthetic-call-key-5d1e";
const SECRET_REPLY: &str = "SECRET-REPLY-77af";

const DENIED: u64 = 1004004;
const RATE_LIMITED: u64 = 1001005;
const TIMED_OUT: u64 = 1001006;

/// The fake Provider's behavior is chosen by a marker in the prompt.
const SLOW_MS: u64 = 500;
const HANG_MS: u64 = 5000;

#[derive(Default)]
struct ProviderStats {
    /// Authenticated requests that reached the fake Provider.
    hits: AtomicUsize,
    unauthenticated: AtomicUsize,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    /// Marker class of every authenticated request, in arrival order. Never
    /// the prompt text itself.
    trail: Mutex<Vec<&'static str>>,
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn serve_provider(listener: TcpListener, stats: Arc<ProviderStats>) {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let stats = stats.clone();
        tauri::async_runtime::spawn(async move {
            let mut data = Vec::new();
            let mut buffer = [0u8; 4096];
            let (head_end, length) = loop {
                let Ok(read) = socket.read(&mut buffer).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                data.extend_from_slice(&buffer[..read]);
                if let Some(end) = find(&data, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..end]).to_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (end + 4, length);
                }
            };
            while data.len() < head_end + length {
                let Ok(read) = socket.read(&mut buffer).await else {
                    return;
                };
                if read == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..read]);
            }
            let head = String::from_utf8_lossy(&data[..head_end]).to_lowercase();
            let body = String::from_utf8_lossy(&data[head_end..]).into_owned();
            let authorized = head.contains(&format!("authorization: bearer {SECRET_KEY}"));

            let (status, payload) = if !authorized {
                stats.unauthenticated.fetch_add(1, Ordering::SeqCst);
                (
                    "401 Unauthorized",
                    r#"{"error":{"message":"rejected"}}"#.to_owned(),
                )
            } else {
                stats.hits.fetch_add(1, Ordering::SeqCst);
                let marker = ["HANG", "SLOW", "FAIL401", "route-probe", "ungranted"]
                    .into_iter()
                    .find(|m| body.contains(m))
                    .unwrap_or("other");
                stats.trail.lock().unwrap().push(marker);
                let now = stats.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                stats.peak.fetch_max(now, Ordering::SeqCst);
                let result = if body.contains("FAIL401") {
                    (
                        "401 Unauthorized",
                        r#"{"error":{"message":"rejected"}}"#.to_owned(),
                    )
                } else {
                    if body.contains("HANG") {
                        tokio::time::sleep(Duration::from_millis(HANG_MS)).await;
                    } else if body.contains("SLOW") {
                        tokio::time::sleep(Duration::from_millis(SLOW_MS)).await;
                    }
                    (
                        "200 OK",
                        format!(
                            r#"{{"choices":[{{"message":{{"content":"{SECRET_REPLY}"}}}}],"usage":{{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}}}"#
                        ),
                    )
                };
                stats.in_flight.fetch_sub(1, Ordering::SeqCst);
                result
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
    }
}

struct CallWorld {
    root: PathBuf,
    db: DatabaseConnection,
    permissions: Arc<ModulePermissionRepository>,
    stats: Arc<ProviderStats>,
    provider_id: String,
    routes: Arc<RoutePublisher>,
    router: Arc<ModuleCallRouter>,
    dispatch: ModuleCapabilityDispatchService,
    manager: ModuleOperationManager,
}

async fn build_world() -> (CallWorld, ModuleHostBridge, ProviderManager) {
    let root = std::env::temp_dir().join(format!("dgos-webview-smoke-call-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let store = root.join("modules");
    let sources = root.join("module-sources");
    fs::create_dir_all(&store).unwrap();
    fs::create_dir_all(&sources).unwrap();

    let url = format!("sqlite://{}?mode=rwc", root.join("dgos.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();

    // Fake Provider on loopback.
    let stats = Arc::new(ProviderStats::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tauri::async_runtime::spawn(serve_provider(listener, stats.clone()));
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: "Synthetic smoke".into(),
            provider_type: "openai_compatible".into(),
            base_url: format!("http://{address}"),
            api_key: SECRET_KEY.into(),
            enabled: true,
            config: None,
        },
        USER,
    )
    .await
    .unwrap();
    let providers = ProviderManager::with_vault(&url, vault).await.unwrap();

    // Both modules carry the same source; only the first is ever granted.
    let publisher = Arc::new(ModulePublisher::new(store, db.clone()));
    let release = ModuleReleaseService::new(publisher.clone());
    let mut pins = Vec::new();
    for module in [GRANTED, UNGRANTED] {
        let package = root.join(format!("pkg-{module}"));
        let digest = write_package_with(&package, module, "1.0.0", CALLER);
        release.publish(&package).await.unwrap();
        pins.push((module, digest));
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
    // The shipped bundle is provisioned exactly as the host does at startup.
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bundled-modules");
    publisher.provision_builtin(&bundled).await.unwrap();
    // Provisioning leaves a bundled module disabled (D055); this scenario is about calls,
    // so it takes the step the user takes in the management page. The management smoke
    // (`webview_smoke_modules`) drives that page itself.
    publisher.set_enabled(CONSOLE, true).await.unwrap();
    let admission = TestAndBuiltinAdmission {
        test: PinnedModuleAdmission::new(
            pins.iter()
                .map(|(id, digest)| (*id, "1.0.0", digest.as_str())),
        ),
        builtin: PinnedModuleAdmission::builtin(),
    };
    let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), Arc::new(loader.clone()))
        .with_admission(Arc::new(admission))
        .with_ready_observer(routes.clone())
        .with_loader_timeouts(LoaderTimeouts {
            smoke: Duration::from_secs(5),
            start: Duration::from_secs(3),
            stop: Duration::from_secs(5),
        });
    let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
    let dispatch = ModuleCapabilityDispatchService::new(HostCapabilityService::with_publisher(
        permissions.clone(),
        publisher.clone(),
    ))
    .with_provider_deadline(Duration::from_secs(1));
    let manager = ModuleOperationManager::with_supervisor(
        publisher,
        supervisor,
        LocalModuleSources::new(sources),
    );
    manager.spawn_lost_runtime_reporter(lost).unwrap();
    (
        CallWorld {
            root,
            db,
            permissions,
            stats,
            provider_id: provider.id,
            routes,
            router,
            dispatch,
            manager,
        },
        ModuleHostBridge::new(loader, transport),
        providers,
    )
}

#[derive(Debug)]
struct Outcome {
    ok: bool,
    code: Option<u64>,
    content: String,
}

impl Outcome {
    fn parse(inner: &Value) -> Self {
        Self {
            ok: inner["ok"] == true,
            code: inner["code"].as_u64(),
            content: inner["content"].as_str().unwrap_or("").to_owned(),
        }
    }

    fn denied(&self) -> bool {
        !self.ok && self.code == Some(DENIED)
    }
}

/// One call from a module's own closure in the page.
async fn module_call(app: &AppHandle, module: &str, provider: &str, prompt: &str) -> Outcome {
    let body =
        format!("return await globalThis.__dgosCallers[{module:?}]({provider:?}, {prompt:?});");
    let reply = eval(app, MAIN, &body).await;
    if reply["ok"] != true {
        return Outcome {
            ok: false,
            code: None,
            content: reply.to_string(),
        };
    }
    Outcome::parse(&reply["value"])
}

/// The typed command called directly, with a captured route.
async fn replay(
    app: &AppHandle,
    attachment: &str,
    route: &str,
    provider: &str,
    prompt: &str,
    extra: Value,
) -> (bool, Option<u64>, String) {
    let mut args = json!({
        "attachmentId": attachment,
        "callId": format!("replay-{}", NEXT_CALL.fetch_add(1, Ordering::SeqCst)),
        "route": route,
        "providerId": provider,
        "modelId": "fixture-chat",
        "request": {"messages": [{"role": "user", "content": prompt}], "max_tokens": 16, "stream": false},
    });
    if let (Some(args), Some(extra)) = (args.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            args.insert(key.clone(), value.clone());
        }
    }
    let reply = ipc(app, MAIN, "module_call_provider", args).await;
    (
        reply.success(),
        reply.code(),
        reply.data()["content"].as_str().unwrap_or("").to_owned(),
    )
}

async fn run_op(app: &AppHandle, command: &str, module: &str) -> Value {
    let reply = ipc(app, MAIN, command, json!({"moduleId": module})).await;
    match reply.data()["operation_id"].as_str() {
        Some(operation) if reply.success() => poll_operation(app, MAIN, operation).await,
        _ => json!({"state": "rejected", "reply": reply.describe()}),
    }
}

async fn status(app: &AppHandle, module: &str) -> Value {
    ipc(
        app,
        MAIN,
        "get_module_runtime_status",
        json!({"moduleId": module}),
    )
    .await
    .data()
    .clone()
}

async fn wait_status(app: &AppHandle, module: &str, state: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..100 {
        last = status(app, module).await;
        if last["state"] == state {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last
}

async fn probe(app: &AppHandle) -> Value {
    eval(
        app,
        MAIN,
        "return JSON.parse(JSON.stringify(globalThis.__dgosModuleProbe ?? {evaluated: 0, events: []}));",
    )
    .await["value"]
        .clone()
}

fn events(probe: &Value) -> Vec<String> {
    probe["events"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

async fn scalar(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(db.get_database_backend(), sql))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "n")
        .unwrap()
}

async fn denied_audits(db: &DatabaseConnection, module: &str) -> i64 {
    scalar(
        db,
        &format!(
            "SELECT COUNT(*) AS n FROM module_permission_audits WHERE action = 'denied' AND module_id = '{module}'"
        ),
    )
    .await
}

async fn audit_dump(db: &DatabaseConnection) -> String {
    db.query_all(Statement::from_string(
        db.get_database_backend(),
        "SELECT user_id, module_id, permission, action, resource_id, request_id FROM module_permission_audits",
    ))
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        let text = |name: &str| row.try_get::<Option<String>>("", name).unwrap();
        format!(
            "{:?}",
            [
                text("user_id"),
                text("module_id"),
                text("permission"),
                text("action"),
                text("resource_id"),
                text("request_id"),
            ]
        )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

async fn usage(app: &AppHandle) -> Vec<dgos_types::UsageLog> {
    app.state::<ProviderManager>()
        .list_usage_logs(USER, Some(1000))
        .await
        .unwrap()
}

/// Start `module` and wait until the page holds a route for it, proven by an
/// ungranted-or-granted call reaching host authorization or the Provider.
async fn start_and_wait_route(app: &AppHandle, world: &CallWorld, module: &str) -> Value {
    let started = run_op(app, "start_module", module).await;
    let provider = world.provider_id.clone();
    for _ in 0..80 {
        let before_denied = denied_audits(&world.db, module).await;
        let before_hits = world.stats.hits.load(Ordering::SeqCst);
        let _ = module_call(app, module, &provider, "route-probe").await;
        if denied_audits(&world.db, module).await > before_denied
            || world.stats.hits.load(Ordering::SeqCst) > before_hits
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    started
}

async fn scenario(app: AppHandle, world: CallWorld, bridge_ready: bool) {
    let mut checks = Checks::default();
    if !bridge_ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks, &world.root);
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
        world.manager.recover().await.is_ok(),
        "recover() failed",
    );
    let provider = world.provider_id.clone();
    let stats = world.stats.clone();
    let hits = || stats.hits.load(Ordering::SeqCst);

    // -- Start: a call before the runtime is ready has no route -----------------
    let started = run_op(&app, "start_module", GRANTED).await;
    checks.check(
        "the granted module starts through the real loader",
        started["state"] == "succeeded" && started["outcome"] == "started",
        started.to_string(),
    );
    let p = probe(&app).await;
    checks.check(
        "a call made inside onLoad is refused with 1004004 and never reaches the Provider",
        events(&p).contains(&"load:call-module".to_owned())
            && events(&p).contains(&"early:1004004".to_owned())
            && hits() == 0,
        p.to_string(),
    );

    // -- Admission and authorization -------------------------------------------
    let mut reached_authorization = false;
    for _ in 0..80 {
        let o = module_call(&app, GRANTED, &provider, "ungranted").await;
        if denied_audits(&world.db, GRANTED).await > 0 {
            checks.check(
                "an ungranted call reaches host authorization and is denied with 1004004",
                o.denied(),
                format!("{o:?}"),
            );
            reached_authorization = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    checks.check(
        "the page received its route once the runtime was ready",
        reached_authorization,
        "no call ever reached authorization",
    );
    checks.check(
        "the denied call never reached the Provider",
        hits() == 0,
        format!("hits={}", hits()),
    );
    let issued = world.routes.issued();
    checks.check(
        "the host issued exactly one route for the ready runtime",
        issued.len() == 1,
        format!("{}", issued.len()),
    );

    world
        .permissions
        .grant(USER, GRANTED, ModulePermission::ProvidersCall)
        .await
        .unwrap();
    let o = module_call(&app, GRANTED, &provider, &format!("hello {SECRET_PROMPT}")).await;
    checks.check(
        "after a grant the module's call succeeds through the real Provider adapter",
        o.ok && o.content == SECRET_REPLY && hits() == 1,
        format!("{o:?} hits={}", hits()),
    );
    let logs = usage(&app).await;
    checks.check(
        "the Provider usage log attributes the call to the module caller",
        logs.len() == 1 && logs[0].caller.as_deref() == Some("module") && logs[0].success,
        format!("{logs:?}"),
    );

    // -- Another module with the same bytes has no grant -------------------------
    let other = start_and_wait_route(&app, &world, UNGRANTED).await;
    let o = module_call(&app, UNGRANTED, &provider, "other").await;
    checks.check(
        "a second module running the same source is denied: grants are per module",
        other["outcome"] == "started" && o.denied() && hits() == 1,
        format!("{other} {o:?} hits={}", hits()),
    );
    checks.check(
        "the denial is audited against the module that called",
        denied_audits(&world.db, UNGRANTED).await >= 1,
        "no denied audit row for the second module",
    );

    // -- Forged and replayed channel input --------------------------------------
    let (runtime, attachment, route) = issued[0].clone();
    let status_now = status(&app, GRANTED).await;
    checks.check(
        "the captured route belongs to the running runtime",
        status_now["runtime_id"] == runtime.as_str(),
        status_now.to_string(),
    );
    let (ok, code, _) = replay(&app, "forged-attachment", &route, &provider, "x", json!({})).await;
    checks.check(
        "a real route with a forged attachment is refused with 1004004",
        !ok && code == Some(DENIED) && hits() == 1,
        format!("{ok} {code:?}"),
    );
    let (ok, code, _) = replay(&app, &attachment, "forged-route", &provider, "x", json!({})).await;
    checks.check(
        "an unknown route is refused with 1004004",
        !ok && code == Some(DENIED) && hits() == 1,
        format!("{ok} {code:?}"),
    );
    let forged = json!({
        "moduleId": UNGRANTED, "module_id": UNGRANTED, "userId": "attacker", "runtimeId": "forged",
        "permissions": ["providers.call"],
    });
    let (ok, _, content) = replay(&app, &attachment, &route, &provider, "replayed", forged).await;
    checks.check(
        "KNOWN LIMIT (D050): any script in the page can replay a route it can read; extra identity fields are ignored and the call is still attributed to the route's own module",
        ok && content == SECRET_REPLY && hits() == 2,
        format!("{ok} hits={}", hits()),
    );

    // -- Revocation --------------------------------------------------------------
    world
        .permissions
        .revoke(USER, GRANTED, ModulePermission::ProvidersCall)
        .await
        .unwrap();
    let o = module_call(&app, GRANTED, &provider, "after revoke").await;
    checks.check(
        "revocation applies to the next call, which never reaches the Provider",
        o.denied() && hits() == 2,
        format!("{o:?} hits={}", hits()),
    );
    world
        .permissions
        .grant(USER, GRANTED, ModulePermission::ProvidersCall)
        .await
        .unwrap();
    let o = module_call(&app, GRANTED, &provider, "granted again").await;
    checks.check(
        "a new grant applies to the next call with the same runtime",
        o.ok && hits() == 3,
        format!("{o:?} hits={}", hits()),
    );

    // -- Error mapping -----------------------------------------------------------
    let o = module_call(&app, GRANTED, &provider, "FAIL401").await;
    checks.check(
        "a Provider failure keeps its own 1001xxx code and is not a permission error",
        !o.ok && o.code.is_some_and(|c| c != DENIED && c / 1000 == 1001) && hits() == 4,
        format!("{o:?}"),
    );

    // -- Concurrency: the in-flight limit ---------------------------------------
    let peak_before = stats.peak.load(Ordering::SeqCst);
    let hits_before = hits();
    let body = format!(
        "const call = globalThis.__dgosCallers['call-module'];\
         return await Promise.all(Array.from({{length: 12}}, () => call({provider:?}, 'SLOW')));"
    );
    let reply = eval(&app, MAIN, &body).await;
    let results: Vec<Outcome> = reply["value"]
        .as_array()
        .map(|items| items.iter().map(Outcome::parse).collect())
        .unwrap_or_default();
    let served = results.iter().filter(|o| o.ok).count();
    let busy = results
        .iter()
        .filter(|o| !o.ok && o.code == Some(RATE_LIMITED))
        .count();
    checks.check(
        "12 concurrent calls: 8 are served and 4 get the retryable 1001005",
        results.len() == 12 && served == 8 && busy == 4,
        format!("served={served} busy={busy} {reply}"),
    );
    checks.check(
        "only the 8 admitted calls reached the Provider, concurrently",
        hits() - hits_before == 8 && stats.peak.load(Ordering::SeqCst) >= 2.max(peak_before),
        format!(
            "hits+={} peak={}",
            hits() - hits_before,
            stats.peak.load(Ordering::SeqCst)
        ),
    );
    let st = status(&app, GRANTED).await;
    checks.check(
        "after the burst the runtime has nothing in flight",
        st["in_flight"] == 0 && st["state"] == "ready",
        st.to_string(),
    );
    let o = module_call(&app, GRANTED, &provider, "after burst").await;
    checks.check(
        "a refused call is retryable: the next call succeeds",
        o.ok,
        format!("{o:?}"),
    );

    // -- Host deadline -----------------------------------------------------------
    let body = format!(
        "const t = performance.now(); const r = await globalThis.__dgosCallers['call-module']({provider:?}, 'HANG'); \
         return {{r, ms: performance.now() - t}};"
    );
    let hits_before = hits();
    let reply = eval(&app, MAIN, &body).await;
    let ms = reply["value"]["ms"].as_f64().unwrap_or(0.0);
    let o = Outcome::parse(&reply["value"]["r"]);
    checks.check(
        "a hanging Provider ends at the host deadline with 1001006, not at the Provider's pace",
        !o.ok && o.code == Some(TIMED_OUT) && (900.0..4000.0).contains(&ms),
        format!("{o:?} {ms}ms"),
    );
    checks.check(
        "the external call was already dispatched and may still complete (result unknown, not a safe retry)",
        hits() == hits_before + 1 && stats.in_flight.load(Ordering::SeqCst) == 1,
        format!("hits+={} provider_in_flight={}", hits() - hits_before, stats.in_flight.load(Ordering::SeqCst)),
    );
    let st = status(&app, GRANTED).await;
    checks.check(
        "the timed-out request is settled on the host: nothing stays in flight",
        st["in_flight"] == 0,
        st.to_string(),
    );
    let _ = wait_until(
        || stats.in_flight.load(Ordering::SeqCst) == 0,
        "provider drain",
    )
    .await;

    // -- Stopped runtime and replaced runtime -------------------------------------
    let stopped = run_op(&app, "stop_module", GRANTED).await;
    let hits_before = hits();
    let o = module_call(&app, GRANTED, &provider, "stale closure").await;
    let (ok, code, _) = replay(
        &app,
        &attachment,
        &route,
        &provider,
        "stale route",
        json!({}),
    )
    .await;
    checks.check(
        "after stop the old closure and the old route are both refused with 1004004",
        stopped["state"] == "succeeded"
            && o.denied()
            && !ok
            && code == Some(DENIED)
            && hits() == hits_before,
        format!("{stopped} {o:?} {ok} {code:?}"),
    );
    let _ = start_and_wait_route(&app, &world, GRANTED).await;
    let o = module_call(&app, GRANTED, &provider, "restarted").await;
    let (ok, code, _) = replay(
        &app,
        &attachment,
        &route,
        &provider,
        "old route again",
        json!({}),
    )
    .await;
    let issued = world.routes.issued();
    checks.check(
        "a restart is a new runtime with a new route; the old route never reaches it",
        o.ok && !ok
            && code == Some(DENIED)
            && issued.len() >= 3
            && issued.last().unwrap().2 != route,
        format!("{o:?} {ok} {code:?} routes={}", issued.len()),
    );

    // -- Page reload with a call in flight ----------------------------------------
    let (_, old_attachment, old_route) = issued.last().unwrap().clone();
    let logs_before = usage(&app).await.len();
    let hits_before = hits();
    let generation = bridge.loader().generation(MAIN);
    let fired = eval(
        &app,
        MAIN,
        &format!(
            "sessionStorage.setItem('ipcTrace', '[]');\
             const note = (kind, target) => {{ if (String(target).includes('module_call_provider')) {{ const t = JSON.parse(sessionStorage.getItem('ipcTrace')); t.push(kind); sessionStorage.setItem('ipcTrace', JSON.stringify(t)); }} }};\
             const realFetch = window.fetch; window.fetch = function (url, ...rest) {{ note('fetch', url); return realFetch.call(this, url, ...rest); }};\
             let hooked = 'no'; try {{ const realPost = window.ipc.postMessage.bind(window.ipc); window.ipc.postMessage = (data) => {{ note('postMessage', data); return realPost(data); }}; hooked = window.ipc.postMessage.toString().includes('note(') ? 'yes' : 'assign-ignored'; }} catch (e) {{ hooked = 'throws'; }} \
             const t0 = JSON.parse(sessionStorage.getItem('ipcTrace')); t0.push('hook:' + hooked); sessionStorage.setItem('ipcTrace', JSON.stringify(t0));\
             const realWarn = console.warn.bind(console); console.warn = (...args) => {{ const t1 = JSON.parse(sessionStorage.getItem('ipcTrace')); t1.push('warn:' + String(args[0]).slice(0, 40)); sessionStorage.setItem('ipcTrace', JSON.stringify(t1)); return realWarn(...args); }};\
             globalThis.__pending = globalThis.__dgosCallers['call-module']({provider:?}, 'SLOW'); return true;"
        ),
    )
    .await;
    let reached = wait_until(
        || hits() > hits_before,
        "the in-flight call reaches the Provider",
    )
    .await;
    let navigated = goto(&app, MAIN, "/settings").await;
    checks.check(
        "the page reloads while a module call is in flight",
        fired["ok"] == true
            && reached.is_ok()
            && navigated
            && bridge.loader().generation(MAIN) > generation,
        format!("{fired} {reached:?} {navigated}"),
    );
    let _ = wait_until(
        || bridge.loader().has_attachment(MAIN),
        "re-attach after reload",
    )
    .await;
    let lost = wait_status(&app, GRANTED, "failed").await;
    checks.check(
        "the runtime of the vanished page is reported lost",
        lost["state"] == "failed",
        lost.to_string(),
    );
    let mut settled = false;
    let mut last = String::new();
    for _ in 0..60 {
        let st = status(&app, GRANTED).await;
        let logged = usage(&app).await.len();
        last = format!(
            "usage {logged} (expected {}), in_flight {}, provider hits+={}, provider in_flight {}",
            logs_before + 1,
            st["in_flight"],
            hits() - hits_before,
            stats.in_flight.load(Ordering::SeqCst)
        );
        if logged == logs_before + 1 && st["in_flight"] == 0 {
            settled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let ipc_trace =
        eval(&app, MAIN, "return sessionStorage.getItem('ipcTrace');").await["value"].clone();
    let trail: Vec<&str> = stats
        .trail
        .lock()
        .unwrap()
        .iter()
        .rev()
        .take(4)
        .copied()
        .collect();
    let recent: Vec<_> = usage(&app)
        .await
        .iter()
        .take(3)
        .map(|l| {
            (
                l.request_id.clone(),
                l.latency_ms,
                l.success,
                l.error_code.clone(),
            )
        })
        .collect();
    checks.check(
        "the host-side call ran to its own terminal state: settled and recorded though no page can receive it",
        settled,
        format!("{last}; recent hits {trail:?}; recent logs {recent:?}; page ipc trace {ipc_trace}"),
    );
    let (ok, code, _) = replay(
        &app,
        &old_attachment,
        &old_route,
        &provider,
        "after reload",
        json!({}),
    )
    .await;
    checks.check(
        "a route from before the reload is refused with 1004004 and nothing is replayed to the new page",
        !ok && code == Some(DENIED) && hits() == hits_before + 1,
        format!("{ok} {code:?} hits+={}", hits() - hits_before),
    );
    let fresh = probe(&app).await;
    checks.check(
        "the reloaded realm starts clean",
        fresh["evaluated"] == 0 && events(&fresh).is_empty(),
        fresh.to_string(),
    );

    let _ = run_op(&app, "stop_module", GRANTED).await;
    let _ = run_op(&app, "stop_module", UNGRANTED).await;
    let started = start_and_wait_route(&app, &world, GRANTED).await;
    let o = module_call(&app, GRANTED, &provider, "after reload start").await;
    checks.check(
        "after a reload a fresh start gets a fresh route and calls work again",
        started["outcome"] == "started" && o.ok,
        format!("{started} {o:?}"),
    );

    // -- The shipped first-party module under its real pin (D052) -----------------
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../bundled-modules/provider-console/1.0.0/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    // The manifest declares the bare v2 hex; the host reports the `v2:` integrity (D057).
    let pinned_digest = format!(
        "v2:{}",
        manifest["artifact"]["sha256"].as_str().unwrap_or("")
    );
    let started = run_op(&app, "start_module", CONSOLE).await;
    let st = status(&app, CONSOLE).await;
    checks.check(
        "the provisioned bundled module starts under the compiled-in pin",
        started["outcome"] == "started"
            && st["state"] == "ready"
            && st["version"] == "1.0.0"
            && st["integrity"] == pinned_digest.as_str(),
        format!("{started} {st}"),
    );
    let hits_before = hits();
    let mut shown = Value::Null;
    for _ in 0..40 {
        shown = drive_console(&app, &provider, "ungranted console").await;
        if denied_audits(&world.db, CONSOLE).await > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    checks.check(
        "its real UI shows a refusal before any grant, audited, with the Provider untouched",
        shown["alert"]
            .as_str()
            .is_some_and(|text| text.contains("1004004"))
            && denied_audits(&world.db, CONSOLE).await >= 1
            && hits() == hits_before,
        format!("{shown} hits+={}", hits() - hits_before),
    );
    world
        .permissions
        .grant(USER, CONSOLE, ModulePermission::ProvidersCall)
        .await
        .unwrap();
    let shown = drive_console(&app, &provider, "hello console").await;
    checks.check(
        "after a grant the real UI sends the prompt and renders the Provider's reply",
        shown["reply"] == SECRET_REPLY && hits() == hits_before + 1,
        format!("{shown} hits+={}", hits() - hits_before),
    );
    let _ = run_op(&app, "stop_module", CONSOLE).await;

    // -- Attribution and redaction ------------------------------------------------
    let logs = usage(&app).await;
    let module_logs: Vec<_> = logs
        .iter()
        .filter(|l| l.caller.as_deref() == Some("module"))
        .collect();
    let mut ids: Vec<_> = module_logs
        .iter()
        .filter_map(|l| l.request_id.clone())
        .collect();
    let count = ids.len();
    ids.sort();
    ids.dedup();
    checks.check(
        "every module call has a host-generated request id and they are all distinct",
        count == module_logs.len() && ids.len() == count && count >= 12,
        format!("{count} logs, {} distinct", ids.len()),
    );
    let mut haystack = format!("{logs:?}\n{}", audit_dump(&world.db).await);
    haystack.push_str(&format!("{:?}", world.router.len()));
    let mut leaked = Vec::new();
    for secret in [SECRET_PROMPT, SECRET_KEY, SECRET_REPLY, "attacker"] {
        if haystack.contains(secret) {
            leaked.push(secret.to_owned());
        }
    }
    for (_, _, route) in world.routes.issued() {
        if haystack.contains(&route) {
            leaked.push("a route key".to_owned());
        }
    }
    checks.check(
        "usage logs and permission audits hold no prompt, key, reply, forged identity or route key",
        leaked.is_empty(),
        format!("{leaked:?}"),
    );
    checks.check(
        "the fake Provider only ever saw authenticated requests",
        stats.unauthenticated.load(Ordering::SeqCst) == 0,
        format!("{}", stats.unauthenticated.load(Ordering::SeqCst)),
    );
    let _ = run_op(&app, "stop_module", GRANTED).await;
    finish(checks, &world.root);
}

pub fn run_call() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        std::process::exit(2);
    });

    let (world, bridge, providers) = tauri::async_runtime::block_on(build_world());
    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    // The window is created below so the harness controls the first URL.
    context.config_mut().app.windows.clear();

    let manager = world.manager.clone();
    let router = world.router.clone();
    let dispatch = world.dispatch.clone();
    let scenario_world = Mutex::new(Some(world));
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(LoadEvents(load_sender))
        .manage(manager)
        .manage(bridge)
        .manage(router)
        .manage(dispatch)
        .manage(providers)
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
            commands::attach_module_host,
            commands::report_module_host,
            commands::module_call_provider,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
