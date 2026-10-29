//! Real module execution in a real WebView (D042-D046).
//!
//! A second smoke target, because a Tauri app can run its event loop once per
//! process and the lifecycle smoke needs a scripted loader. This one wires
//! the production pieces instead: `PageModuleLoader` behind the real
//! `ModuleHostBridge`, host-pinned admission, supervisor deadlines, the real
//! Settings route gate and the real frontend from `dist/`. Fixture sources are
//! the same files the vitest suite uses.
//!
//! What it proves: module code is imported through a Blob URL by the platform
//! WebView, evaluated, rendered with the host's React, started, stopped,
//! failed, cleaned up and loaded again. What it does not: Windows WebView2 or
//! Linux WebKitGTK, a release build with real pins, the dev-server origin, any
//! sandboxing (the realm is shared by design, D042), or a visible window.

use super::{
    eval, finish, goto, ipc, looks_like_uuid, poll_operation, wait_load, wait_until,
    write_package_with, Checks, LoadEvents, Pending, WATCHDOG,
};
use crate::commands;
use crate::module_host::{self, ChannelTransport, ModuleHostBridge};
use dgos_application::{ModuleOperationManager, ModuleReleaseService};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_modules::{
    LoaderTimeouts, LocalModuleSources, ModulePageTransport, ModulePublisher,
    ModuleRuntimeSupervisor, PageInstruction, PageLoaderConfig, PageModuleLoader, PageSendError,
    PinnedModuleAdmission,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::broadcast;

const COUNTER: &str = include_str!("../../../tests/fixtures/modules/counter.js");
const BAD_CONTRACT: &str = include_str!("../../../tests/fixtures/modules/bad-contract.js");
const SYNTAX_ERROR: &str = include_str!("../../../tests/fixtures/modules/syntax-error.js");
const RENDER_THROWS: &str = include_str!("../../../tests/fixtures/modules/render-throws.js");
const ONLOAD_THROWS: &str = include_str!("../../../tests/fixtures/modules/onload-throws.js");
const ONLOAD_HANGS: &str = include_str!("../../../tests/fixtures/modules/onload-hangs.js");
const IMPORT_HANGS: &str = include_str!("../../../tests/fixtures/modules/import-hangs.js");
const ONUNLOAD_THROWS: &str = include_str!("../../../tests/fixtures/modules/onunload-throws.js");

/// `(module id, entry source, pinned)`. `real-unpinned` is a package whose
/// digest matches its own manifest and is nevertheless not admitted.
const PACKAGES: [(&str, &str, bool); 9] = [
    ("real-counter", COUNTER, true),
    ("real-bad-contract", BAD_CONTRACT, true),
    ("real-syntax-error", SYNTAX_ERROR, true),
    ("real-render-throws", RENDER_THROWS, true),
    ("real-onload-throws", ONLOAD_THROWS, true),
    ("real-onload-hangs", ONLOAD_HANGS, true),
    ("real-import-hangs", IMPORT_HANGS, true),
    ("real-unload-throws", ONUNLOAD_THROWS, true),
    ("real-unpinned", COUNTER, false),
];

const LOAD_FAILED: u64 = 1004003;
const NOT_ADMITTED: u64 = 1004021;
const HOST_DENIED: u64 = 1004004;

/// Records every instruction handed to the page, then delivers it through the
/// real channel transport.
struct RecordingTransport {
    inner: Arc<ChannelTransport>,
    sent: Mutex<Vec<(String, PageInstruction)>>,
}

impl RecordingTransport {
    fn log(&self) -> Vec<(String, PageInstruction)> {
        self.sent.lock().unwrap().clone()
    }
}

impl ModulePageTransport for RecordingTransport {
    fn send(&self, attachment_id: &str, instruction: PageInstruction) -> Result<(), PageSendError> {
        self.sent
            .lock()
            .unwrap()
            .push((attachment_id.to_owned(), instruction.clone()));
        self.inner.send(attachment_id, instruction)
    }
}

struct LoadRec {
    index: usize,
    attachment: String,
    ticket: String,
    runtime: String,
    version: String,
    source: String,
}

fn loads(log: &[(String, PageInstruction)], module: &str) -> Vec<LoadRec> {
    log.iter()
        .enumerate()
        .filter_map(|(index, (attachment, instruction))| match instruction {
            PageInstruction::Load {
                ticket,
                runtime_id,
                module_id,
                version,
                source,
                ..
            } if module_id == module => Some(LoadRec {
                index,
                attachment: attachment.clone(),
                ticket: ticket.clone(),
                runtime: runtime_id.clone(),
                version: version.clone(),
                source: source.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Positions of `Unload` instructions addressed to `runtime`.
fn unloads(log: &[(String, PageInstruction)], runtime: &str) -> Vec<usize> {
    log.iter()
        .enumerate()
        .filter_map(|(index, (_, instruction))| match instruction {
            PageInstruction::Unload { runtime_id, .. } if runtime_id == runtime => Some(index),
            _ => None,
        })
        .collect()
}

struct RealWorld {
    root: PathBuf,
    sources: PathBuf,
    manager: ModuleOperationManager,
    recording: Arc<RecordingTransport>,
}

async fn build_world() -> (RealWorld, ModuleHostBridge) {
    let root = std::env::temp_dir().join(format!("dgos-webview-smoke-real-{}", std::process::id()));
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

    let publisher = Arc::new(ModulePublisher::new(store, db));
    let release = ModuleReleaseService::new(publisher.clone());
    let mut pins: Vec<(String, String, String)> = Vec::new();
    for (module, body, pinned) in PACKAGES {
        let package = root.join(format!("pkg-{module}"));
        let digest = write_package_with(&package, module, "1.0.0", body);
        release.publish(&package).await.unwrap();
        if pinned {
            pins.push((module.into(), "1.0.0".into(), digest));
        }
    }
    // A pinned 2.0.0 of the counter waits in the host-owned source root.
    let v2 = format!("{COUNTER}\n// release 2.0.0\n");
    let digest = write_package_with(&sources.join("real-counter"), "real-counter", "2.0.0", &v2);
    pins.push(("real-counter".into(), "2.0.0".into(), digest));

    let transport = Arc::new(ChannelTransport::default());
    let recording = Arc::new(RecordingTransport {
        inner: transport.clone(),
        sent: Mutex::default(),
    });
    let (loader, lost) = PageModuleLoader::new(
        recording.clone(),
        PageLoaderConfig {
            window_label: module_host::MODULE_WINDOW.into(),
            // Shorter than the start deadline below, so a hanging onLoad is
            // ended by the page and a hanging import only by the host.
            hook_timeout: Duration::from_secs(1),
        },
    );
    let admission = PinnedModuleAdmission::new(
        pins.iter()
            .map(|(id, version, digest)| (id.as_str(), version.as_str(), digest.as_str())),
    );
    let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), Arc::new(loader.clone()))
        .with_admission(Arc::new(admission))
        .with_loader_timeouts(LoaderTimeouts {
            smoke: Duration::from_secs(5),
            start: Duration::from_secs(3),
            stop: Duration::from_secs(5),
        });
    let manager = ModuleOperationManager::with_supervisor(
        publisher,
        supervisor,
        LocalModuleSources::new(sources.clone()),
    );
    manager.spawn_lost_runtime_reporter(lost).unwrap();
    (
        RealWorld {
            root,
            sources,
            manager,
            recording,
        },
        ModuleHostBridge::new(loader, transport),
    )
}

async fn run_op(app: &AppHandle, command: &str, module: &str) -> Value {
    let reply = ipc(app, "main", command, json!({"moduleId": module})).await;
    match reply.data()["operation_id"].as_str() {
        Some(operation) if reply.success() => poll_operation(app, "main", operation).await,
        _ => json!({"state": "not accepted", "reply": reply.describe()}),
    }
}

async fn status(app: &AppHandle, module: &str) -> Value {
    ipc(
        app,
        "main",
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
            return last;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last
}

async fn dom(app: &AppHandle) -> Value {
    eval(
        app,
        "main",
        "return Array.from(document.querySelectorAll('[data-module-runtime]')).map(e=>({\
         runtime:e.getAttribute('data-module-runtime'),module:e.getAttribute('data-module-id'),\
         text:e.textContent,connected:e.isConnected}))",
    )
    .await["value"]
        .clone()
}

async fn probe(app: &AppHandle) -> Value {
    eval(
        app,
        "main",
        "return globalThis.__dgosModuleProbe ?? {evaluated:0,events:[]}",
    )
    .await["value"]
        .clone()
}

fn evaluated(probe: &Value) -> u64 {
    probe["evaluated"].as_u64().unwrap_or(u64::MAX)
}

fn events(probe: &Value) -> Vec<String> {
    probe["events"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The module's DOM lives in its panel (D055); open it from the sidebar like a user.
async fn open_panel(app: &AppHandle, module: &str) {
    eval(
        app,
        "main",
        &format!(
            "const sel='[data-module-id=\"{module}\"]';\
             if(!document.querySelector(sel)){{const a=document.querySelector('a[href=\"/settings/modules/{module}\"]');if(a)a.click();}}\
             for(let i=0;i<50&&!document.querySelector(sel);i++)await new Promise(r=>setTimeout(r,100));\
             return !!document.querySelector(sel)"
        ),
    )
    .await;
}

fn module_nodes(dom: &Value, module: &str) -> Vec<Value> {
    dom.as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| item["module"] == module)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

async fn report(app: &AppHandle, attachment: &str, ticket: &str, receipt: Value) -> String {
    let reply = ipc(
        app,
        "main",
        "report_module_host",
        json!({"attachmentId": attachment, "ticket": ticket, "receipt": receipt}),
    )
    .await;
    reply.data()["disposition"]
        .as_str()
        .unwrap_or("<none>")
        .to_owned()
}

fn failed_with(view: &Value, code: u64) -> bool {
    view["state"] == "failed" && view["failure"]["code"] == code
}

async fn scenario(app: AppHandle, world: RealWorld, ready: bool) {
    let mut checks = Checks::default();
    let RealWorld {
        root,
        sources,
        manager,
        recording,
    } = world;
    let main = "main";
    if !ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks, &root);
    }
    let bridge = app.state::<ModuleHostBridge>();
    let attached = wait_until(
        || bridge.loader().has_attachment(main),
        "the page to attach to the module host",
    )
    .await;
    checks.check(
        "the page attaches to the module host at application start",
        attached.is_ok(),
        format!("{attached:?}"),
    );
    if attached.is_err() {
        finish(checks, &root);
    }
    checks.check(
        "host recovery succeeds",
        manager.recover().await.is_ok(),
        "recover() failed",
    );
    let before = probe(&app).await;
    checks.check(
        "a fresh page has evaluated no module code",
        evaluated(&before) == 0 && events(&before).is_empty(),
        before.to_string(),
    );

    // -- Admission: a self-consistent but unpinned package never runs ---------
    let view = run_op(&app, "start_module", "real-unpinned").await;
    checks.check(
        "an unpinned package is refused with 1004021",
        failed_with(&view, NOT_ADMITTED),
        view.to_string(),
    );
    let log = recording.log();
    let after = probe(&app).await;
    checks.check(
        "the unpinned package reached neither the page nor the loader",
        loads(&log, "real-unpinned").is_empty()
            && evaluated(&after) == 0
            && status(&app, "real-unpinned").await["state"] == "not_loaded",
        format!("{} instructions, {after}", log.len()),
    );

    // -- Real load: code evaluated, rendered, started -------------------------
    let view = run_op(&app, "start_module", "real-counter").await;
    checks.check(
        "a pinned module starts through the real page loader",
        view["state"] == "succeeded" && view["outcome"] == "started",
        view.to_string(),
    );
    let st = status(&app, "real-counter").await;
    checks.check(
        "status is ready and says the loader executes module code",
        st["state"] == "ready" && st["executes_module_code"] == true && st["version"] == "1.0.0",
        st.to_string(),
    );
    let log = recording.log();
    let counter_loads = loads(&log, "real-counter");
    checks.check(
        "exactly one load instruction carried the verified entry bytes",
        counter_loads.len() == 1
            && counter_loads[0].source == COUNTER
            && counter_loads[0].version == "1.0.0"
            && Some(counter_loads[0].runtime.as_str()) == st["runtime_id"].as_str(),
        format!("{} loads", counter_loads.len()),
    );
    let p = probe(&app).await;
    checks.check(
        "the module code really ran in the page (evaluated once, onLoad hook ran)",
        evaluated(&p) == 1 && events(&p) == ["load:real-counter"],
        p.to_string(),
    );
    open_panel(&app, "real-counter").await;
    let d = dom(&app).await;
    let nodes = module_nodes(&d, "real-counter");
    checks.check(
        "the component is rendered in the document with the host's React",
        nodes.len() == 1 && nodes[0]["text"] == "count:0" && nodes[0]["connected"] == true,
        d.to_string(),
    );
    let clicked = eval(
        &app,
        main,
        "const b=document.querySelector('[data-module-id=\"real-counter\"] button');\
         b.click();await new Promise(r=>setTimeout(r,100));return b.textContent",
    )
    .await;
    checks.check(
        "a click updates module state (hooks work on the shared React)",
        clicked["value"] == "count:1",
        clicked.to_string(),
    );
    let counter = &counter_loads[0];
    let runtime_one = counter.runtime.clone();

    // -- Duplicate, unknown and wrong receipts are inert ----------------------
    let duplicate = report(
        &app,
        &counter.attachment,
        &counter.ticket,
        json!({"kind": "loaded"}),
    )
    .await;
    let forged_failure = report(
        &app,
        &counter.attachment,
        &counter.ticket,
        json!({"kind": "loadFailed", "stage": "render"}),
    )
    .await;
    let unknown = report(
        &app,
        &counter.attachment,
        "never-issued",
        json!({"kind": "loaded"}),
    )
    .await;
    let st = status(&app, "real-counter").await;
    let d = dom(&app).await;
    checks.check(
        "duplicate and unknown receipts are classified and change nothing",
        duplicate == "duplicate"
            && forged_failure == "duplicate"
            && unknown == "unknown"
            && st["state"] == "ready"
            && module_nodes(&d, "real-counter")[0]["text"] == "count:1",
        format!("{duplicate} {forged_failure} {unknown} {st}"),
    );

    // -- SPA navigation: gate parity and survival -----------------------------
    let generation = bridge.loader().generation(main);
    let marked = eval(
        &app,
        main,
        "const e=document.querySelector('[data-module-id=\"real-counter\"]');\
         e.__smokeMark='kept';return e.isConnected",
    )
    .await;
    let away = eval(
        &app,
        main,
        "document.querySelector('a[href=\"/\"]').click();\
         await new Promise(r=>setTimeout(r,300));return location.pathname",
    )
    .await;
    checks.check(
        "a BrowserRouter Link changes the path by pushState",
        marked["value"] == true && away["value"] == "/",
        format!("{marked} {away}"),
    );
    let denied_status = ipc(
        &app,
        main,
        "get_module_runtime_status",
        json!({"moduleId": "real-counter"}),
    )
    .await;
    let denied_start = ipc(
        &app,
        main,
        "start_module",
        json!({"moduleId": "real-counter"}),
    )
    .await;
    checks.check(
        "after the SPA navigation the host gate denies module commands (1004004)",
        denied_status.code() == Some(HOST_DENIED) && denied_start.code() == Some(HOST_DENIED),
        format!("{} / {}", denied_status.describe(), denied_start.describe()),
    );
    let bridge_off_route = report(
        &app,
        &counter.attachment,
        "never-issued",
        json!({"kind": "loaded"}),
    )
    .await;
    checks.check(
        "the bridge report command is not tied to the Settings route",
        bridge_off_route == "unknown",
        bridge_off_route,
    );
    checks.check(
        "the module stays ready on the host and its DOM is detached while away",
        matches!(
            manager.runtime_status("real-counter"),
            Ok(view) if view.state == dgos_types::ModuleRuntimeStatusState::Ready
        ) && dom(&app).await.as_array().is_some_and(Vec::is_empty),
        "module did not survive route change",
    );
    let back = eval(
        &app,
        main,
        "document.querySelector('a[href=\"/settings/modules/real-counter\"]').click();\
         await new Promise(r=>setTimeout(r,300));\
         const e=document.querySelector('[data-module-id=\"real-counter\"]');\
         return {path:location.pathname,mark:e&&e.__smokeMark,connected:e&&e.isConnected,text:e&&e.textContent}",
    )
    .await;
    let readmitted = ipc(
        &app,
        main,
        "get_module_runtime_status",
        json!({"moduleId": "real-counter"}),
    )
    .await;
    checks.check(
        "returning to Settings re-attaches the same DOM node with its state, and the gate allows again",
        back["value"]["path"] == "/settings/modules/real-counter"
            && back["value"]["mark"] == "kept"
            && back["value"]["connected"] == true
            && back["value"]["text"] == "count:1"
            && readmitted.success(),
        format!("{back} {}", readmitted.describe()),
    );
    checks.check(
        "SPA navigation never changed the page generation or reloaded the module",
        bridge.loader().generation(main) == generation
            && evaluated(&probe(&app).await) == 1
            && status(&app, "real-counter").await["runtime_id"] == runtime_one.as_str(),
        "generation, evaluation count or runtime changed",
    );

    // UI route table against the host's URL gate, in the real WebView.
    let mut parity_failures = Vec::new();
    for path in [
        "/settings",
        "/settings/",
        "/settings/sub",
        "/Settings",
        "/settings2",
        "/%73ettings",
        "/settings%2Fx",
        "/",
        "/workbench-missing",
    ] {
        let shown = eval(
            &app,
            main,
            &format!(
                "history.pushState({{}},'',{path:?});\
                 window.dispatchEvent(new PopStateEvent('popstate',{{state:history.state}}));\
                 await new Promise(r=>setTimeout(r,200));\
                 return {{path:location.pathname,shown:Array.from(document.querySelectorAll('a')).some(a=>a.textContent==='Module management')}}"
            ),
        )
        .await;
        let gate = ipc(
            &app,
            main,
            "get_module_runtime_status",
            json!({"moduleId": "real-counter"}),
        )
        .await;
        let expected = path == "/settings" || path.starts_with("/settings/");
        let ui = shown["value"]["shown"] == true;
        if ui != gate.success() || gate.success() != expected {
            parity_failures.push(format!(
                "{path}: ui={ui} gate={} expected={expected}",
                gate.success()
            ));
        }
    }
    checks.check(
        "the UI shows the Settings page exactly where the host gate allows (path, case, encoding)",
        parity_failures.is_empty(),
        parity_failures.join("; "),
    );
    eval(
        &app,
        main,
        "history.pushState({},'','/settings');\
         window.dispatchEvent(new PopStateEvent('popstate',{state:history.state}));\
         await new Promise(r=>setTimeout(r,200));return location.pathname",
    )
    .await;

    // -- Failures leave nothing behind and can be retried ---------------------
    for (module, delta) in [
        ("real-bad-contract", 1u64),
        ("real-syntax-error", 0),
        ("real-render-throws", 1),
        ("real-onload-throws", 1),
        ("real-onload-hangs", 1),
    ] {
        let evaluated_before = evaluated(&probe(&app).await);
        let view = run_op(&app, "start_module", module).await;
        let st = status(&app, module).await;
        let log = recording.log();
        let recs = loads(&log, module);
        let d = dom(&app).await;
        let p = probe(&app).await;
        checks.check(
            &format!("{module}: start fails with 1004003 and the runtime is failed"),
            failed_with(&view, LOAD_FAILED) && st["state"] == "failed",
            format!("{view} {st}"),
        );
        checks.check(
            &format!("{module}: the page evaluated it {delta} time(s) and left no DOM or tracked instance"),
            evaluated(&p) == evaluated_before + delta
                && module_nodes(&d, module).is_empty()
                && bridge.loader().tracked_instances() == 1
                && recs.len() == 1
                && unloads(&log, &recs[0].runtime).is_empty(),
            format!("{p} {d} tracked={}", bridge.loader().tracked_instances()),
        );
        let stopped = run_op(&app, "stop_module", module).await;
        checks.check(
            &format!("{module}: stop completes and the status is stopped"),
            stopped["state"] == "succeeded" && status(&app, module).await["state"] == "stopped",
            stopped.to_string(),
        );
    }
    let first = status(&app, "real-render-throws").await;
    let again = run_op(&app, "start_module", "real-render-throws").await;
    let second = status(&app, "real-render-throws").await;
    checks.check(
        "a retry after a failed load is a new runtime with a new real load",
        failed_with(&again, LOAD_FAILED)
            && second["state"] == "failed"
            && second["runtime_id"] != first["runtime_id"],
        format!("{first} -> {second}"),
    );
    run_op(&app, "stop_module", "real-render-throws").await;

    // A hanging import is ended only by the host's own deadline.
    let evaluated_before = evaluated(&probe(&app).await);
    let view = run_op(&app, "start_module", "real-import-hangs").await;
    let log = recording.log();
    let hung = loads(&log, "real-import-hangs");
    let hung_runtime = hung
        .first()
        .map(|rec| rec.runtime.clone())
        .unwrap_or_default();
    let hung_unloads = unloads(&log, &hung_runtime);
    checks.check(
        "a hanging import ends at the host deadline with 1004003 and the page is told to unload",
        failed_with(&view, LOAD_FAILED)
            && hung.len() == 1
            && hung_unloads.len() == 1
            && hung_unloads[0] > hung[0].index
            && evaluated(&probe(&app).await) == evaluated_before + 1,
        format!("{view} loads={} unloads={}", hung.len(), hung_unloads.len()),
    );
    checks.check(
        "after that cleanup only the running counter is tracked",
        bridge.loader().tracked_instances() == 1,
        format!("tracked={}", bridge.loader().tracked_instances()),
    );
    if let Some(rec) = hung.first() {
        let late = report(
            &app,
            &rec.attachment,
            &rec.ticket,
            json!({"kind": "loaded"}),
        )
        .await;
        let st = status(&app, "real-import-hangs").await;
        checks.check(
            "a late loaded receipt for the timed-out load is abandoned and revives nothing",
            late == "abandoned" && st["state"] == "failed",
            format!("{late} {st}"),
        );
    }
    run_op(&app, "stop_module", "real-import-hangs").await;

    // A throwing onUnload still releases the instance.
    run_op(&app, "start_module", "real-unload-throws").await;
    let stopped = run_op(&app, "stop_module", "real-unload-throws").await;
    let p = probe(&app).await;
    checks.check(
        "a failing onUnload hook is logged but the instance is released",
        stopped["state"] == "succeeded"
            && events(&p).contains(&"unload:real-unload-throws".to_owned())
            && module_nodes(&dom(&app).await, "real-unload-throws").is_empty()
            && bridge.loader().tracked_instances() == 1,
        format!("{stopped} {p}"),
    );

    // -- Hot update: new code runs before the old one is unloaded -------------
    let events_before = events(&probe(&app).await).len();
    let old = status(&app, "real-counter").await;
    let view = run_op(&app, "update_module", "real-counter").await;
    let st = status(&app, "real-counter").await;
    let log = recording.log();
    let v2 = loads(&log, "real-counter")
        .into_iter()
        .find(|rec| rec.version == "2.0.0");
    let old_unloads = unloads(&log, old["runtime_id"].as_str().unwrap_or(""));
    open_panel(&app, "real-counter").await;
    let d = dom(&app).await;
    let nodes = module_nodes(&d, "real-counter");
    let p = probe(&app).await;
    let new_events: Vec<String> = events(&p).into_iter().skip(events_before).collect();
    checks.check(
        "update switches to 2.0.0: new load first, old unload second, one node, fresh runtime",
        view["state"] == "succeeded"
            && view["outcome"] == "updated"
            && st["version"] == "2.0.0"
            && st["runtime_id"] != old["runtime_id"]
            && v2
                .as_ref()
                .is_some_and(|rec| old_unloads.len() == 1 && rec.index < old_unloads[0])
            && nodes.len() == 1
            && nodes[0]["runtime"] == st["runtime_id"]
            && nodes[0]["text"] == "count:0"
            && new_events == ["load:real-counter", "unload:real-counter"],
        format!("{view} {st} {d} {new_events:?}"),
    );
    let loads_before = recording.log().len();
    let _ = fs::remove_dir_all(sources.join("real-counter"));
    write_package_with(
        &sources.join("real-counter"),
        "real-counter",
        "3.0.0",
        &format!("{COUNTER}\n// release 3.0.0 (not pinned)\n"),
    );
    let view = run_op(&app, "update_module", "real-counter").await;
    let st_after = status(&app, "real-counter").await;
    checks.check(
        "an update to an unpinned release is refused with 1004021; the 2.0.0 runtime keeps running",
        failed_with(&view, NOT_ADMITTED)
            && st_after["version"] == "2.0.0"
            && st_after["state"] == "ready"
            && st_after["runtime_id"] == st["runtime_id"]
            && recording.log().len() == loads_before,
        format!("{view} {st_after}"),
    );

    // -- Reload with a load in flight and a live instance ---------------------
    let generation = bridge.loader().generation(main);
    let live = status(&app, "real-counter").await;
    let accepted = ipc(
        &app,
        main,
        "start_module",
        json!({"moduleId": "real-import-hangs"}),
    )
    .await;
    let operation = accepted.data()["operation_id"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let pending_sent = wait_until(
        || loads(&recording.log(), "real-import-hangs").len() == 2,
        "the in-flight load instruction",
    )
    .await;
    let inflight = loads(&recording.log(), "real-import-hangs").pop();
    let navigated = goto(&app, main, "/settings").await;
    checks.check(
        "the page reloads while a load is in flight",
        accepted.success() && looks_like_uuid(&operation) && pending_sent.is_ok() && navigated,
        format!("{} {pending_sent:?} {navigated}", accepted.describe()),
    );
    let reattached = wait_until(
        || bridge.loader().has_attachment(main),
        "re-attach after reload",
    )
    .await;
    let view = poll_operation(&app, main, &operation).await;
    checks.check(
        "the reload advances the page generation, fails the in-flight load, and the new page attaches",
        bridge.loader().generation(main) > generation && reattached.is_ok() && failed_with(&view, LOAD_FAILED),
        view.to_string(),
    );
    let lost = wait_status(&app, "real-counter", "failed").await;
    checks.check(
        "the live instance of the old page is reported lost and its runtime is failed",
        lost["state"] == "failed" && lost["runtime_id"] == live["runtime_id"],
        lost.to_string(),
    );
    if let Some(rec) = inflight {
        let replay = report(
            &app,
            &rec.attachment,
            &rec.ticket,
            json!({"kind": "loaded"}),
        )
        .await;
        checks.check(
            "a receipt replayed from the old page's attachment is stale",
            replay == "stale",
            replay,
        );
    }
    let fresh = probe(&app).await;
    checks.check(
        "the new realm starts clean: nothing evaluated, nothing carried over",
        evaluated(&fresh) == 0
            && events(&fresh).is_empty()
            && dom(&app).await.as_array().is_some_and(Vec::is_empty),
        fresh.to_string(),
    );
    let log = recording.log();
    let lost_unloads = unloads(&log, live["runtime_id"].as_str().unwrap_or("")).len();
    checks.check(
        "no unload was sent to the new page for the vanished instances",
        lost_unloads == 0,
        format!("{lost_unloads} unload instruction(s) targeted the lost runtime"),
    );
    let stopped_hung = run_op(&app, "stop_module", "real-import-hangs").await;
    let stopped_lost = run_op(&app, "stop_module", "real-counter").await;
    checks.check(
        "stopping the lost runtimes completes without touching the new page",
        stopped_hung["state"] == "succeeded"
            && stopped_lost["state"] == "succeeded"
            && bridge.loader().tracked_instances() == 0
            && recording.log().len() == log.len(),
        format!("{stopped_hung} {stopped_lost}"),
    );

    // -- Load again in the new realm (2.0.0 is current) -----------------------
    let view = run_op(&app, "start_module", "real-counter").await;
    let st = status(&app, "real-counter").await;
    open_panel(&app, "real-counter").await;
    let d = dom(&app).await;
    let p = probe(&app).await;
    checks.check(
        "loading again after a reload is a new runtime evaluated once in the new realm",
        view["outcome"] == "started"
            && st["version"] == "2.0.0"
            && st["runtime_id"] != live["runtime_id"]
            && evaluated(&p) == 1
            && module_nodes(&d, "real-counter").len() == 1
            && module_nodes(&d, "real-counter")[0]["text"] == "count:0",
        format!("{view} {st} {d} {p}"),
    );

    // -- No silent fallback when the host has no page -------------------------
    run_op(&app, "stop_module", "real-counter").await;
    let evaluated_before = evaluated(&probe(&app).await);
    let sent_before = recording.log().len();
    bridge.page_started(main);
    let view = run_op(&app, "start_module", "real-counter").await;
    let st = status(&app, "real-counter").await;
    checks.check(
        "without an attached page start fails 1004003, executes nothing and never reports ready",
        failed_with(&view, LOAD_FAILED)
            && st["state"] == "failed"
            && st["executes_module_code"] == true
            && evaluated(&probe(&app).await) == evaluated_before
            && recording.log().len() == sent_before,
        format!("{view} {st}"),
    );
    run_op(&app, "stop_module", "real-counter").await;
    let navigated = goto(&app, main, "/settings").await;
    let reattached = wait_until(|| bridge.loader().has_attachment(main), "re-attach").await;
    let view = run_op(&app, "start_module", "real-counter").await;
    checks.check(
        "after the page attaches again the same module loads for real",
        navigated && reattached.is_ok() && view["outcome"] == "started",
        view.to_string(),
    );

    // -- Final stop -----------------------------------------------------------
    let stopped = run_op(&app, "stop_module", "real-counter").await;
    let d = dom(&app).await;
    checks.check(
        "the final stop unloads the module and leaves no DOM or tracked instance",
        stopped["state"] == "succeeded"
            && module_nodes(&d, "real-counter").is_empty()
            && bridge.loader().tracked_instances() == 0
            && status(&app, "real-counter").await["state"] == "stopped",
        format!("{stopped} {d}"),
    );
    finish(checks, &root);
}

pub fn run_real() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        std::process::exit(2);
    });

    let mut world = Some(tauri::async_runtime::block_on(build_world()));
    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    // The window is created below so the harness controls the first URL.
    context.config_mut().app.windows.clear();

    let (world, bridge) = world.take().unwrap();
    let manager = world.manager.clone();
    let scenario_world = Mutex::new(Some(world));
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(LoadEvents(load_sender))
        .manage(manager)
        .manage(bridge)
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
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
