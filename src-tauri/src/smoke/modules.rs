//! Module management, hot update, crash handling and startup load in a real
//! WebView (D055).
//!
//! Fourth smoke target. It wires the production pieces for these flows: the
//! startup load that follows a page attach (`StartupLoad`), the real
//! `PageModuleLoader`/`ModuleHostBridge`, the route observer, the crash
//! service, the management service, the management commands and the real
//! frontend from `dist/` (sidebar links, the module panel, the management
//! page). Fixtures are the files the vitest suite uses.
//!
//! Replaced for the run: admission is the real compiled-in pin list plus test
//! pins for the fixtures, and update sources are a temp directory. Not
//! covered: Windows WebView2, Linux WebKitGTK, a real process restart (a page
//! reload stands in for it), a release build, or isolation between modules
//! (the realm is shared, D042).

use super::{
    eval, finish, ipc, poll_operation, wait_load, wait_until, Checks, LoadEvents, Pending, WATCHDOG,
};
use crate::commands;
use crate::module_host::{self, ChannelTransport, ModuleHostBridge, RoutePublisher, StartupLoad};
use dgos_application::{
    ModuleCrashService, ModuleManagementService, ModuleOperationManager, ModuleReleaseService,
};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::{ModuleCrashRepository, ModulePermissionRepository};
use dgos_modules::{
    LoaderTimeouts, LocalModuleSources, ModuleAdmission, ModuleCallRouter, ModulePublisher,
    ModuleRuntimeSupervisor, PageLoaderConfig, PageModuleLoader, PinnedModuleAdmission,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::broadcast;

const COUNTER: &str = include_str!("../../../tests/fixtures/modules/counter.js");
const HOOKS: &str = include_str!("../../../tests/fixtures/modules/hooks-update.js");
const CRASH: &str = include_str!("../../../tests/fixtures/modules/crash-after-mount.js");
const REACT_IDENTITY: &str =
    include_str!("../../../tests/fixtures/modules/hooks-react-identity.js");
const BARE_IMPORT: &str = include_str!("../../../tests/fixtures/modules/hooks-bare-import.js");

const MAIN: &str = module_host::MODULE_WINDOW;
const COUNTERS: [&str; 5] = ["mc-1", "mc-2", "mc-3", "mc-4", "mc-5"];
const HOOKS_ID: &str = "mh-hooks";
const CRASH_ID: &str = "mx-crash";
const REACT_ID: &str = "mr-react";
const BARE_ID: &str = "mr-bare";
const CONSOLE: &str = "provider-console";

const DISABLED: u64 = 1004009;
const UPDATE_FAILED: u64 = 1004013;
const INCOMPATIBLE: u64 = 1004022;
const OFFICIAL: u64 = 1004023;

/// One-file package; returns its v2 integrity. `requires` goes into the manifest.
fn write_pkg(dir: &Path, id: &str, version: &str, body: &str, requires: Option<Value>) -> String {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("entry.js"), body).unwrap();
    let mut manifest = json!({
        "id": id, "name": id, "version": version, "entry": "entry.js",
        "artifact": {"version": version, "sha256": "0".repeat(64)},
        "permissions": ["providers.call"]
    });
    if let Some(requires) = requires {
        manifest["requires"] = requires;
    }
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    let integrity = dgos_modules::package_integrity(dir).unwrap();
    manifest["artifact"]["sha256"] = integrity.strip_prefix("v2:").unwrap().into();
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    integrity
}

pub(super) struct ModWorld {
    pub(super) root: PathBuf,
    sources: PathBuf,
    store: PathBuf,
    pub(super) manager: ModuleOperationManager,
    pub(super) router: Arc<ModuleCallRouter>,
    pub(super) crashes: ModuleCrashService,
    pub(super) management: ModuleManagementService,
    /// Test pins for versions that arrive later through the update source.
    pins: Arc<Mutex<Vec<(String, String, String)>>>,
}

/// Admission whose test pins can grow while the app runs, like a new build
/// would ship new pins.
struct GrowingAdmission {
    pins: Arc<Mutex<Vec<(String, String, String)>>>,
    builtin: PinnedModuleAdmission,
}

impl ModuleAdmission for GrowingAdmission {
    fn admits(&self, module_id: &str, version: &str, integrity: &str) -> bool {
        self.builtin.admits(module_id, version, integrity)
            || self.pins.lock().unwrap().iter().any(|(id, v, digest)| {
                id == module_id && v == version && digest.eq_ignore_ascii_case(integrity)
            })
    }
}

pub(super) async fn build_world() -> (ModWorld, ModuleHostBridge) {
    build_world_with(&[]).await
}

/// `extra` are additional one-file fixture modules (id, source) published and pinned like the others.
pub(super) async fn build_world_with(extra: &[(&str, &str)]) -> (ModWorld, ModuleHostBridge) {
    let root =
        std::env::temp_dir().join(format!("dgos-webview-smoke-modules-{}", std::process::id()));
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

    let publisher = Arc::new(ModulePublisher::new(store.clone(), db.clone()));
    let release = ModuleReleaseService::new(publisher.clone());
    let pins = Arc::new(Mutex::new(Vec::new()));
    let mut published: Vec<(String, &str)> = COUNTERS
        .iter()
        .map(|id| ((*id).to_owned(), COUNTER))
        .collect();
    published.push((HOOKS_ID.into(), HOOKS));
    published.push((CRASH_ID.into(), CRASH));
    published.push((REACT_ID.into(), REACT_IDENTITY));
    published.push((BARE_ID.into(), BARE_IMPORT));
    for (id, body) in extra {
        published.push(((*id).to_owned(), *body));
    }
    for (id, body) in &published {
        let dir = root.join(format!("pkg-{id}"));
        let digest = write_pkg(&dir, id, "1.0.0", body, None);
        release.publish(&dir).await.unwrap();
        pins.lock()
            .unwrap()
            .push((id.clone(), "1.0.0".into(), digest));
    }
    // The shipped first-party module is provisioned as the host does at startup.
    publisher
        .provision_builtin(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bundled-modules"))
        .await
        .unwrap();

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
        .with_admission(Arc::new(GrowingAdmission {
            pins: pins.clone(),
            builtin: PinnedModuleAdmission::builtin(),
        }))
        .with_ready_observer(routes)
        .with_loader_timeouts(LoaderTimeouts {
            smoke: Duration::from_secs(5),
            start: Duration::from_secs(4),
            stop: Duration::from_secs(5),
        });
    let manager = ModuleOperationManager::with_supervisor(
        publisher,
        supervisor,
        LocalModuleSources::new(sources.clone()),
    );
    manager.spawn_lost_runtime_reporter(lost).unwrap();
    manager.recover().await.unwrap();
    let crashes = ModuleCrashService::new(ModuleCrashRepository::new(db.clone()), manager.clone());
    let management = ModuleManagementService::new(
        Arc::new(ModulePermissionRepository::new(db)),
        manager.clone(),
        crashes.clone(),
    );
    (
        ModWorld {
            root,
            sources,
            store,
            manager,
            router,
            crashes,
            management,
            pins,
        },
        ModuleHostBridge::new(loader, transport),
    )
}

async fn js(app: &AppHandle, body: &str) -> Value {
    eval(app, MAIN, body).await["value"].clone()
}

async fn list(app: &AppHandle) -> Vec<Value> {
    ipc(app, MAIN, "list_modules", json!({}))
        .await
        .data()
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn module<'a>(all: &'a [Value], id: &str) -> Option<&'a Value> {
    all.iter().find(|m| m["module_id"] == id)
}

async fn run_op(app: &AppHandle, command: &str, module: &str) -> Value {
    let reply = ipc(app, MAIN, command, json!({"moduleId": module})).await;
    match reply.data()["operation_id"].as_str() {
        Some(operation) if reply.success() => poll_operation(app, MAIN, operation).await,
        _ => json!({"state": "rejected", "reply": reply.describe(), "code": reply.code()}),
    }
}

fn failure_code(view: &Value) -> Option<u64> {
    view["failure"]["code"].as_u64()
}

async fn status(app: &AppHandle, id: &str) -> Value {
    ipc(
        app,
        MAIN,
        "get_module_runtime_status",
        json!({"moduleId": id}),
    )
    .await
    .data()
    .clone()
}

/// Poll `body` (a JS function body) until it returns something truthy.
async fn until_js(app: &AppHandle, body: &str, seconds: u64) -> bool {
    for _ in 0..(seconds * 10) {
        let value = js(app, body).await;
        if value
            .as_bool()
            .unwrap_or(!value.is_null() && value != json!(0) && value != json!(""))
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn until_modules(app: &AppHandle, seconds: u64, ok: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    let mut all = Vec::new();
    for _ in 0..(seconds * 10) {
        all = list(app).await;
        if ok(&all) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    all
}

fn ready_ids(all: &[Value]) -> Vec<String> {
    let mut ids: Vec<String> = all
        .iter()
        .filter(|m| m["runtime"]["state"] == "ready")
        .filter_map(|m| m["module_id"].as_str().map(str::to_owned))
        .collect();
    ids.sort();
    ids
}

async fn click_link(app: &AppHandle, href: &str) -> bool {
    js(
        app,
        &format!("const a = document.querySelector('a[href=\"{href}\"]'); if (!a) return false; a.click(); return true;"),
    )
    .await
        == json!(true)
}

fn probe_events(probe: &Value) -> Vec<String> {
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

async fn probe(app: &AppHandle) -> Value {
    js(
        app,
        "return JSON.parse(JSON.stringify(globalThis.__dgosModuleProbe ?? {evaluated: 0, events: []}));",
    )
    .await
}

fn index_of(events: &[String], needle: &str) -> Option<usize> {
    events.iter().position(|e| e == needle)
}

fn replace_source(
    sources: &Path,
    id: &str,
    version: &str,
    body: &str,
    requires: Option<Value>,
) -> String {
    let dir = sources.join(id);
    let _ = fs::remove_dir_all(&dir);
    write_pkg(&dir, id, version, body, requires)
}

async fn scenario(app: AppHandle, world: ModWorld, ready: bool) {
    let mut checks = Checks::default();
    if !ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks, &world.root);
    }
    let bridge = app.state::<ModuleHostBridge>();
    let ModWorld {
        root,
        sources,
        store,
        manager,
        router,
        crashes,
        management,
        pins,
    } = &world;
    let _ = (router, crashes, management, manager);

    // -- AC01/AC14: startup load of every published, enabled module ---------------
    let expected: Vec<String> = {
        let mut ids: Vec<String> = COUNTERS.iter().map(|s| (*s).to_owned()).collect();
        ids.extend([HOOKS_ID, CRASH_ID, REACT_ID].map(str::to_owned));
        ids.sort();
        ids
    };
    let all = until_modules(&app, 30, |all| ready_ids(all) == expected).await;
    checks.check(
        "after the page attaches the host starts every enabled module on its own (5 counters + 3 others)",
        ready_ids(&all) == expected,
        format!("ready {:?}", ready_ids(&all)),
    );
    let bare = status(&app, BARE_ID).await;
    checks.check(
        "a module whose entry imports a bare specifier cannot load: no second React can be pulled in (AC03)",
        bare["state"] == "failed",
        bare.to_string(),
    );
    let console = module(&all, CONSOLE);
    checks.check(
        "the bundled module is listed as official and stays off until the user enables it",
        console.is_some_and(|m| {
            m["official"] == true && m["enabled"] == false && m["runtime"]["state"] == "not_loaded"
        }),
        format!("{console:?}"),
    );
    let p = probe(&app).await;
    let events = probe_events(&p);
    checks.check(
        "each started module's onLoad ran once and the page evaluated 8 modules (AC12)",
        COUNTERS.iter().all(|id| {
            events
                .iter()
                .filter(|e| **e == format!("load:{id}"))
                .count()
                == 1
        }) && p["evaluated"] == 8,
        p.to_string(),
    );
    let react = js(&app, "const r = globalThis.__dgosHooks?.seenReact; return r ? {version: r.version, hooks: typeof r.useState} : null;").await;
    checks.check(
        "a module receives the host's React (same major, hooks usable), not its own copy (AC03)",
        react["version"]
            .as_str()
            .is_some_and(|v| v.starts_with("19."))
            && react["hooks"] == "function",
        react.to_string(),
    );

    // -- Navigation entries and the module panel ---------------------------------
    let links = js(
        &app,
        "return document.querySelectorAll('a[href^=\"/settings/modules/\"]').length;",
    )
    .await;
    checks.check(
        "the sidebar has one link per ready module",
        links == 8,
        links.to_string(),
    );
    let opened = click_link(&app, "/settings/modules/mc-1").await
        && until_js(&app, "const n = document.querySelector('[data-module-id=\"mc-1\"]'); return !!n && n.isConnected;", 5).await;
    checks.check(
        "a module's panel shows its live component",
        opened,
        "panel did not render",
    );

    // -- AC14: five modules updated at once ---------------------------------------
    for id in COUNTERS {
        let digest = replace_source(
            sources,
            id,
            "2.0.0",
            &format!("{COUNTER}\n// release 2.0.0\n"),
            None,
        );
        pins.lock()
            .unwrap()
            .push((id.to_owned(), "2.0.0".into(), digest));
    }
    let submitted = js(
        &app,
        &format!(
            "const ids = {};\
             const out = await Promise.all(ids.map((id) => window.__TAURI_INTERNALS__.invoke('update_module', {{moduleId: id}})));\
             return out.map((r) => r.data && r.data.operation_id);",
            json!(COUNTERS)
        ),
    )
    .await;
    let mut views = Vec::new();
    for operation in submitted.as_array().cloned().unwrap_or_default() {
        if let Some(operation) = operation.as_str() {
            views.push(poll_operation(&app, MAIN, operation).await);
        }
    }
    let all = list(&app).await;
    checks.check(
        "5 modules updated at once all succeed and end up ready on 2.0.0",
        views.len() == 5
            && views
                .iter()
                .all(|v| v["state"] == "succeeded" && v["outcome"] == "updated")
            && COUNTERS.iter().all(|id| {
                module(&all, id)
                    .is_some_and(|m| m["version"] == "2.0.0" && m["runtime"]["state"] == "ready")
            }),
        format!("{views:?}"),
    );
    let nodes = js(
        &app,
        "return document.querySelectorAll('[data-module-runtime]').length;",
    )
    .await;
    checks.check(
        "after the updates no old instance lingers: one live instance per module (AC11 runtime cleanup)",
        bridge.loader().tracked_instances() == 8 && nodes.as_u64().unwrap_or(99) <= 8,
        format!("tracked={} nodes={nodes}", bridge.loader().tracked_instances()),
    );

    // -- AC06/AC07: hot update hands state to the new version ---------------------
    js(&app, "globalThis.__dgosHooks = globalThis.__dgosHooks ?? {}; delete globalThis.__dgosHooks.after; delete globalThis.__dgosHooks.before; return true;").await;
    let generation = bridge.loader().generation(MAIN);
    let digest = replace_source(
        sources,
        HOOKS_ID,
        "2.0.0",
        &format!("{HOOKS}\n// release 2.0.0\n"),
        None,
    );
    pins.lock()
        .unwrap()
        .push((HOOKS_ID.into(), "2.0.0".into(), digest));
    let view = run_op(&app, "update_module", HOOKS_ID).await;
    let p = probe(&app).await;
    let events = probe_events(&p);
    let before = index_of(&events, "before:1.0.0");
    let load = index_of(&events, "load:2.0.0");
    let after = index_of(&events, "after:2.0.0");
    let unload = index_of(&events, "unload:1.0.0");
    checks.check(
        "hot update without a restart: old onBeforeUpdate, new onLoad, new onAfterUpdate, then the old instance unloads",
        view["outcome"] == "updated"
            && matches!((before, load, after, unload), (Some(b), Some(l), Some(a), Some(u)) if b < l && l < a && a < u)
            && bridge.loader().generation(MAIN) == generation,
        format!("{view} {events:?}"),
    );
    let restored = js(
        &app,
        "return JSON.stringify(globalThis.__dgosHooks.restored);",
    )
    .await;
    checks.check(
        "the user state survives the update (JSON round trip)",
        restored == json!("{\"count\":3,\"list\":[1,2],\"when\":\"1970-01-01T00:00:00.000Z\"}"),
        restored.to_string(),
    );

    // -- AC08: a failing restore rolls back and keeps the old version running -----
    js(&app, "globalThis.__dgosHooks.after = 'throw'; return true;").await;
    let digest = replace_source(
        sources,
        HOOKS_ID,
        "3.0.0",
        &format!("{HOOKS}\n// release 3.0.0\n"),
        None,
    );
    pins.lock()
        .unwrap()
        .push((HOOKS_ID.into(), "3.0.0".into(), digest));
    let tracked = bridge.loader().tracked_instances();
    let view = run_op(&app, "update_module", HOOKS_ID).await;
    let all = list(&app).await;
    let events = probe_events(&probe(&app).await);
    checks.check(
        "a throwing onAfterUpdate fails the update (1004013) and 2.0.0 keeps running, not unloaded",
        view["state"] == "failed"
            && failure_code(&view) == Some(UPDATE_FAILED)
            && module(&all, HOOKS_ID)
                .is_some_and(|m| m["version"] == "2.0.0" && m["runtime"]["state"] == "ready")
            && index_of(&events, "after:3.0.0").is_some()
            && index_of(&events, "unload:2.0.0").is_none()
            && bridge.loader().tracked_instances() == tracked,
        format!("{view} {events:?}"),
    );

    // -- AC02: an incompatible release is refused before it can load -------------
    js(&app, "delete globalThis.__dgosHooks.after; return true;").await;
    replace_source(
        sources,
        HOOKS_ID,
        "4.0.0",
        &format!("{HOOKS}\n// release 4.0.0\n"),
        Some(json!({"react": "^20"})),
    );
    let view = run_op(&app, "update_module", HOOKS_ID).await;
    let all = list(&app).await;
    checks.check(
        "a release that needs React 20 on a React 19 host is refused with 1004022 and nothing changes",
        view["state"] == "failed"
            && failure_code(&view) == Some(INCOMPATIBLE)
            && module(&all, HOOKS_ID).is_some_and(|m| m["version"] == "2.0.0" && m["runtime"]["state"] == "ready"),
        format!("{view}"),
    );

    // -- AC09/AC10/AC15: crash isolation, log, reload, auto-disable ---------------
    let opened = click_link(&app, "/settings/modules/mx-crash").await
        && until_js(
            &app,
            "return !!document.querySelector('[data-testid=\"fixture-crash\"]');",
            5,
        )
        .await;
    checks.check("the crash fixture renders in its panel", opened, "no panel");
    let mut first_runtime = String::new();
    for round in 1..=3 {
        let before = status(&app, CRASH_ID).await;
        if round == 1 {
            first_runtime = before["runtime_id"].as_str().unwrap_or("").to_owned();
        }
        js(
            &app,
            "document.querySelector('[data-testid=\"fixture-crash\"]').click(); return true;",
        )
        .await;
        let banner = until_js(
            &app,
            "const a = [...document.querySelectorAll('[role=\"alert\"]')].find((n) => n.textContent.includes('crashed')); return !!a;",
            8,
        )
        .await;
        let alive = js(
            &app,
            "return document.body.children.length > 0 && !!document.querySelector('nav, a');",
        )
        .await;
        let others = status(&app, "mc-1").await;
        if round == 1 {
            checks.check(
                "a render crash after mount shows the module's error page while the app and other modules keep running (AC09)",
                banner && alive == json!(true) && others["state"] == "ready",
                format!("banner={banner} alive={alive} other={others}"),
            );
        }
        if round < 3 {
            let failed = status(&app, CRASH_ID).await;
            let clicked = js(
                &app,
                "for (let i = 0; i < 80; i++) { const b = [...document.querySelectorAll('button')].find((n) => n.textContent.trim() === 'Reload module' && !n.disabled); if (b) { b.click(); return 'clicked'; } await new Promise((r) => setTimeout(r, 100)); } return 'no-button';",
            )
            .await;
            let reloaded = clicked == json!("clicked")
                && until_js(
                    &app,
                    "return !!document.querySelector('[data-testid=\"fixture-crash\"]');",
                    10,
                )
                .await;
            let now = status(&app, CRASH_ID).await;
            let alerts = js(
                &app,
                "return {path: location.pathname, text: document.body.innerText.slice(0, 300), containers: document.querySelectorAll('[data-module-runtime]').length};",
            )
            .await;
            checks.check(
                &format!("crash {round}: the host knows the runtime failed, and Reload brings a fresh working instance (AC10)"),
                failed["state"] == "failed" && reloaded && now["state"] == "ready" && now["runtime_id"] != before["runtime_id"],
                format!(
                    "clicked={clicked} failed={} now={} alerts={alerts}",
                    failed["state"], now["state"]
                ),
            );
        }
    }
    let records = ipc(
        &app,
        MAIN,
        "list_module_crashes",
        json!({"moduleId": CRASH_ID}),
    )
    .await;
    let rows = records.data().as_array().cloned().unwrap_or_default();
    let now = chrono_now();
    checks.check(
        "the crash log has an entry per crash with a stack and a recent time (AC15)",
        rows.len() == 3
            && rows.iter().all(|r| {
                r["stack"]
                    .as_str()
                    .is_some_and(|s| s.contains("frame@module:1:1") && s.len() <= 8 * 1024)
                    && r["created_at"]
                        .as_i64()
                        .is_some_and(|t| (now - t).abs() < 300)
                    && r["error_name"] == "FixtureCrash"
                    && r["source"] == "render"
            }),
        format!("{} rows", rows.len()),
    );
    let dump = serde_json::to_string(&rows).unwrap_or_default();
    checks.check(
        "the crash log keeps no message text, URL or path",
        !dump.contains("secret")
            && !dump.contains("http")
            && !dump.contains("blob")
            && !dump.contains("fixture secret message"),
        dump.chars().take(200).collect::<String>(),
    );
    let all = until_modules(&app, 15, |all| {
        module(all, CRASH_ID).is_some_and(|m| m["enabled"] == false)
    })
    .await;
    let crashed = module(&all, CRASH_ID);
    checks.check(
        "three crashes of one version within ten minutes disable the module and stop its runtime",
        crashed.is_some_and(|m| {
            m["enabled"] == false && m["recent_crashes"] == 3 && m["runtime"]["state"] != "ready"
        }),
        format!("{crashed:?}"),
    );
    let faulty = until_js(
        &app,
        "return document.body.textContent.includes('may be faulty');",
        8,
    )
    .await;
    checks.check(
        "the panel tells the user the module may be faulty",
        faulty,
        "no notice",
    );
    let _ = first_runtime;

    // -- AC04: enabling a bundled module through the real UI ----------------------
    let on_page = click_link(&app, "/settings/modules").await
        && until_js(
            &app,
            &format!("return !!document.querySelector('[data-testid=\"module-{CONSOLE}\"]');"),
            8,
        )
        .await;
    let opened_dialog = on_page
        && js(
            &app,
            &format!(
                "const card = document.querySelector('[data-testid=\"module-{CONSOLE}\"]');\
                 const b = [...card.querySelectorAll('button')].find((n) => n.textContent.trim() === 'Enable'); if (!b) return false; b.click(); return true;"
            ),
        )
        .await
            == json!(true);
    let dialog = until_js(
        &app,
        "return !!document.querySelector('[role=\"dialog\"]');",
        5,
    )
    .await;
    let listed = js(
        &app,
        "const d = document.querySelector('[role=\"dialog\"]'); return d ? d.textContent : '';",
    )
    .await;
    checks.check(
        "the enable dialog lists the permission the module asks for",
        opened_dialog
            && dialog
            && listed
                .as_str()
                .is_some_and(|t| t.contains("providers.call")),
        listed.to_string(),
    );
    let before = list(&app).await;
    checks.check(
        "nothing is granted or running before the user confirms",
        module(&before, CONSOLE)
            .is_some_and(|m| m["enabled"] == false && m["permissions"][0]["granted"] == false),
        format!("{:?}", module(&before, CONSOLE)),
    );
    js(
        &app,
        "const d = document.querySelector('[role=\"dialog\"]'); d.querySelector('[role=\"checkbox\"]').click(); return true;",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    js(
        &app,
        "const d = document.querySelector('[role=\"dialog\"]'); const b = [...d.querySelectorAll('button')].find((n) => n.textContent.trim() === 'Confirm enable'); b.click(); return true;",
    )
    .await;
    let all = until_modules(&app, 15, |all| {
        module(all, CONSOLE)
            .is_some_and(|m| m["enabled"] == true && m["runtime"]["state"] == "ready")
    })
    .await;
    let console = module(&all, CONSOLE);
    checks.check(
        "after the user confirms, the bundled module is granted its permission, enabled and running",
        console.is_some_and(|m| m["permissions"][0]["granted"] == true && m["enabled"] == true && m["runtime"]["state"] == "ready"),
        format!("{console:?}"),
    );
    let view = run_op(&app, "uninstall_module", CONSOLE).await;
    checks.check(
        "a bundled module cannot be uninstalled (1004023) and keeps running",
        view["state"] == "failed"
            && failure_code(&view) == Some(OFFICIAL)
            && status(&app, CONSOLE).await["state"] == "ready",
        view.to_string(),
    );

    // -- AC13: disable and enable -------------------------------------------------
    let view = run_op(&app, "disable_module", "mc-4").await;
    let all = list(&app).await;
    let gone = js(&app, "return !document.querySelector('a[href=\"/settings/modules/mc-4\"]') && !document.querySelector('[data-module-id=\"mc-4\"]');").await;
    checks.check(
        "a disabled module stops, leaves the sidebar and the DOM, and is listed disabled",
        view["outcome"] == "disabled"
            && module(&all, "mc-4")
                .is_some_and(|m| m["enabled"] == false && m["runtime"]["state"] != "ready")
            && gone == json!(true),
        format!("{view} {gone}"),
    );
    let view = run_op(&app, "start_module", "mc-4").await;
    checks.check(
        "starting a disabled module is refused with 1004009",
        view["state"] == "failed" && failure_code(&view) == Some(DISABLED),
        view.to_string(),
    );
    let enabled = run_op(&app, "enable_module", "mc-4").await;
    let started = run_op(&app, "start_module", "mc-4").await;
    checks.check(
        "enabling then starting brings it back",
        enabled["outcome"] == "enabled"
            && started["outcome"] == "started"
            && status(&app, "mc-4").await["state"] == "ready",
        format!("{enabled} {started}"),
    );

    // -- AC11: uninstall cleans routes, sidebar, DOM, runtime and files -----------
    let tracked = bridge.loader().tracked_instances();
    let installed = store.join("versions").join("mc-5").exists();
    let view = run_op(&app, "uninstall_module", "mc-5").await;
    let all = list(&app).await;
    let cleaned = js(&app, "return !document.querySelector('a[href=\"/settings/modules/mc-5\"]') && !document.querySelector('[data-module-id=\"mc-5\"]');").await;
    checks.check(
        "uninstall removes the module from the list, sidebar and DOM and releases its runtime",
        installed
            && view["outcome"] == "uninstalled"
            && module(&all, "mc-5").is_none()
            && cleaned == json!(true)
            && bridge.loader().tracked_instances() == tracked - 1,
        format!(
            "{view} tracked {} -> {}",
            tracked,
            bridge.loader().tracked_instances()
        ),
    );
    checks.check(
        "uninstall deletes the module's files",
        !store.join("versions").join("mc-5").exists()
            && !store.join("current").join("mc-5").exists(),
        "files remain",
    );
    let panel = click_link(&app, "/settings/modules").await;
    let _ = panel;

    // -- AC01: a restart (page reload) loads the enabled modules again ---------------
    let generation = bridge.loader().generation(MAIN);
    let reloaded = super::goto(&app, MAIN, "/settings").await;
    let expected_after: Vec<String> = {
        let mut ids: Vec<String> = ["mc-1", "mc-2", "mc-3", "mc-4", HOOKS_ID, REACT_ID, CONSOLE]
            .map(str::to_owned)
            .to_vec();
        ids.sort();
        ids
    };
    let all = until_modules(&app, 40, |all| ready_ids(all) == expected_after).await;
    let fresh = probe(&app).await;
    checks.check(
        "after a reload every enabled module is started again and the disabled ones stay off",
        reloaded
            && bridge.loader().generation(MAIN) > generation
            && ready_ids(&all) == expected_after
            && module(&all, CRASH_ID).is_some_and(|m| m["runtime"]["state"] != "ready")
            && fresh["evaluated"] == 6,
        format!(
            "ready {:?} evaluated {}",
            ready_ids(&all),
            fresh["evaluated"]
        ),
    );

    let _ = wait_until(|| true, "done").await;
    finish(checks, root);
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn run_modules() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        std::process::exit(2);
    });

    let (world, bridge) = tauri::async_runtime::block_on(build_world());
    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();

    let manager = world.manager.clone();
    let router = world.router.clone();
    let crashes = world.crashes.clone();
    let management = world.management.clone();
    let scenario_world = Mutex::new(Some(world));
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(LoadEvents(load_sender))
        .manage(manager)
        .manage(bridge)
        .manage(router)
        .manage(crashes)
        .manage(management)
        .manage(StartupLoad)
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
            commands::attach_module_host,
            commands::report_module_host,
            commands::report_module_crash,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
