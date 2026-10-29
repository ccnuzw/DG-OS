//! Real-WebView smoke harness for the module lifecycle commands (D035).
//!
//! Compiled only with the `webview-smoke` feature and run through
//! `npm run smoke:tauri`. It builds a real Tauri application (embedded
//! `dist/`, the production command handlers and host gate, a temp SQLite
//! database, the real publisher and manager) and drives a real WebView by
//! evaluating JavaScript that calls `window.__TAURI_INTERNALS__.invoke`.
//!
//! What it is not: the module loader is a scripted test double (no module code
//! runs), the OS keychain-backed Provider/MCP/Skills stack is not started, and
//! only the platform WebView of the machine that runs it is exercised.

use crate::commands;
use dgos_application::{ModuleOperationManager, ModuleReleaseService};
use dgos_db::sea_orm::ConnectionTrait;
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_modules::{
    LocalArtifactLoader, LocalModuleSources, ModuleLoadError, ModuleLoader, ModulePublisher,
    VerifiedModuleArtifact,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot, watch};

const WATCHDOG: Duration = Duration::from_secs(240);

mod appearance;
mod appearance_host;
mod call;
mod modules;
mod real;
pub use appearance::run_appearance;
pub use appearance_host::run_appearance_host;
pub use call::run_call;
pub use modules::run_modules;
pub use real::run_real;
const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// Scripted loader: optional start gate and scripted stop failures around the
/// real local artifact checks.
#[derive(Default)]
struct SmokeLoader {
    gate: Mutex<Option<watch::Receiver<bool>>>,
    entered: AtomicUsize,
    failing_stops: AtomicUsize,
}

impl SmokeLoader {
    fn close_gate(&self) -> watch::Sender<bool> {
        let (sender, receiver) = watch::channel(false);
        *self.gate.lock().unwrap() = Some(receiver);
        sender
    }

    fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ModuleLoader for SmokeLoader {
    async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
        LocalArtifactLoader.smoke(artifact).await
    }

    async fn start(
        &self,
        runtime_id: &str,
        artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().clone();
        if let Some(mut gate) = gate {
            let _ = gate.wait_for(|open| *open).await;
        }
        LocalArtifactLoader.start(runtime_id, artifact).await
    }

    async fn stop(
        &self,
        _runtime_id: &str,
        _artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError> {
        let scripted = self
            .failing_stops
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if scripted {
            return Err(ModuleLoadError::Rejected("scripted stop failure".into()));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct Pending(Mutex<HashMap<String, oneshot::Sender<String>>>);

struct LoadEvents(broadcast::Sender<(String, String)>);

/// The only harness-specific command: the evaluated script reports its result.
#[tauri::command]
pub(crate) fn smoke_report(pending: tauri::State<'_, Pending>, id: String, payload: String) {
    if let Some(sender) = pending.0.lock().unwrap().remove(&id) {
        let _ = sender.send(payload);
    }
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// Run `body` (an async function body that may `return`) inside window
/// `label` and return `{ok, value|error}`.
async fn eval(app: &AppHandle, label: &str, body: &str) -> Value {
    let Some(window) = app.get_webview_window(label) else {
        return json!({"ok": false, "error": "no such window"});
    };
    let id = format!("r{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
    let (sender, receiver) = oneshot::channel();
    app.state::<Pending>()
        .0
        .lock()
        .unwrap()
        .insert(id.clone(), sender);
    let script = format!(
        "(async()=>{{let __r;try{{__r={{ok:true,value:await (async()=>{{{body}}})()}};}}\
         catch(e){{__r={{ok:false,error:String((e&&e.message)||e)}};}}\
         await window.__TAURI_INTERNALS__.invoke('smoke_report',{{id:{id:?},payload:JSON.stringify(__r)}});}})()"
    );
    if window.eval(script).is_err() {
        return json!({"ok": false, "error": "eval failed"});
    }
    match tokio::time::timeout(STEP_TIMEOUT, receiver).await {
        Ok(Ok(payload)) => serde_json::from_str(&payload).unwrap_or(Value::Null),
        _ => {
            app.state::<Pending>().0.lock().unwrap().remove(&id);
            json!({"ok": false, "error": "harness timeout"})
        }
    }
}

/// One Tauri command call from the page.
struct Reply(Value);

impl Reply {
    /// Rejected by Tauri before reaching the command (ACL, missing bridge).
    fn rejected(&self) -> bool {
        self.0["ok"] == false
    }
    fn success(&self) -> bool {
        self.0["ok"] == true && self.0["value"]["success"] == true
    }
    fn code(&self) -> Option<u64> {
        self.0["value"]["error"]["code"].as_u64()
    }
    fn data(&self) -> &Value {
        &self.0["value"]["data"]
    }
    fn describe(&self) -> String {
        self.0.to_string()
    }
}

async fn ipc(app: &AppHandle, label: &str, command: &str, args: Value) -> Reply {
    let body = format!("return await window.__TAURI_INTERNALS__.invoke({command:?}, {args});");
    Reply(eval(app, label, &body).await)
}

fn looks_like_uuid(value: &str) -> bool {
    value.len() == 36 && value.matches('-').count() == 4
}

async fn poll_operation(app: &AppHandle, label: &str, operation_id: &str) -> Value {
    for _ in 0..200 {
        let reply = ipc(
            app,
            label,
            "get_module_operation",
            json!({"operationId": operation_id}),
        )
        .await;
        let state = reply.data()["state"].as_str().unwrap_or("");
        if reply.success() && (state == "succeeded" || state == "failed") {
            return reply.data().clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    json!({"state": "poll timeout"})
}

async fn wait_load(
    mut events: broadcast::Receiver<(String, String)>,
    label: &str,
    path: &str,
) -> bool {
    let wait = async {
        loop {
            match events.recv().await {
                Ok((l, p)) if l == label && p == path => return true,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return false,
            }
        }
    };
    tokio::time::timeout(STEP_TIMEOUT, wait)
        .await
        .unwrap_or(false)
}

async fn goto(app: &AppHandle, label: &str, path: &str) -> bool {
    let events = app.state::<LoadEvents>().0.subscribe();
    let Some(window) = app.get_webview_window(label) else {
        return false;
    };
    let Ok(mut url) = window.url() else {
        return false;
    };
    url.set_path(path);
    url.set_query(None);
    if window.navigate(url).is_err() {
        return false;
    }
    wait_load(events, label, path).await
}

async fn wait_until(mut condition: impl FnMut() -> bool, what: &str) -> Result<(), String> {
    for _ in 0..600 {
        if condition() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("timed out waiting for {what}"))
}

fn write_package(dir: &Path, module_id: &str, version: &str) -> String {
    let body = format!("export default '{module_id}@{version}';");
    write_package_with(dir, module_id, version, &body)
}

/// Write a one-file package whose entry is `body`; returns the package's v2 integrity.
fn write_package_with(dir: &Path, module_id: &str, version: &str, body: &str) -> String {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("entry.js"), body).unwrap();
    let write_manifest = |sha256: &str| {
        fs::write(
            dir.join("manifest.json"),
            json!({
                "id": module_id, "name": "Smoke", "version": version, "entry": "entry.js",
                "artifact": {"version": version, "sha256": sha256},
                "permissions": ["providers.call"]
            })
            .to_string(),
        )
        .unwrap();
    };
    // The manifest is part of the v2 digest, the declared value is not.
    write_manifest(&"0".repeat(64));
    let integrity = dgos_modules::package_integrity(dir).unwrap();
    write_manifest(integrity.strip_prefix("v2:").unwrap());
    integrity
}

#[derive(Default)]
struct Checks {
    passed: usize,
    failed: Vec<String>,
}

impl Checks {
    fn check(&mut self, name: &str, ok: bool, detail: impl std::fmt::Display) {
        if ok {
            self.passed += 1;
            println!("PASS {name}");
        } else {
            let line = format!("{name}: {detail}");
            println!("FAIL {line}");
            self.failed.push(line);
        }
    }
}

const REMOTE_PAGE: &str = "<!doctype html><title>remote</title><script>(async()=>{let r;\
try{const i=window.__TAURI_INTERNALS__;if(!i){r={ok:false,error:'no tauri bridge'};}\
else{r={ok:true,value:await i.invoke('start_module',{moduleId:'smoke-d'})};}}\
catch(e){r={ok:false,error:String((e&&e.message)||e)};}\
await fetch('/report?d='+encodeURIComponent(JSON.stringify(r)));})();</script>";

async fn serve(listener: TcpListener, hits: Arc<Mutex<Vec<String>>>) {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let hits = hits.clone();
        tauri::async_runtime::spawn(async move {
            let mut buffer = vec![0u8; 8192];
            let Ok(read) = socket.read(&mut buffer).await else {
                return;
            };
            let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
            let target = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (content_type, body) = if let Some(query) = target.strip_prefix("/report?") {
                if let Ok(url) = tauri::Url::parse(&format!("http://x/?{query}")) {
                    if let Some((_, value)) = url.query_pairs().find(|(key, _)| key == "d") {
                        hits.lock().unwrap().push(value.into_owned());
                    }
                }
                ("text/plain", "ok".to_owned())
            } else {
                ("text/html", REMOTE_PAGE.to_owned())
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
    }
}

struct World {
    root: PathBuf,
    store: PathBuf,
    sources: PathBuf,
    decoy: PathBuf,
    db: dgos_db::sea_orm::DatabaseConnection,
    loader: Arc<SmokeLoader>,
    manager: ModuleOperationManager,
    digest_a2: String,
}

async fn build_world() -> World {
    let root = std::env::temp_dir().join(format!("dgos-webview-smoke-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    // The publisher canonicalises package roots; avoid /var -> /private/var skew.
    let root = root.canonicalize().unwrap();
    let store = root.join("modules");
    let sources = root.join("module-sources");
    let decoy = root.join("decoy");
    for dir in [&store, &sources, &decoy] {
        fs::create_dir_all(dir).unwrap();
    }

    let url = format!("sqlite://{}?mode=rwc", root.join("dgos.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    dgos_db::migrations::Migrator::up(&db, None).await.unwrap();

    let publisher = Arc::new(ModulePublisher::new(store.clone(), db.clone()));
    let release = ModuleReleaseService::new(publisher.clone());
    for module in ["smoke-a", "smoke-b", "smoke-c", "smoke-d"] {
        let package = root.join(format!("pkg-{module}"));
        write_package(&package, module, "1.0.0");
        release.publish(&package).await.unwrap();
    }
    // smoke-b is tampered on disk after publication: recovery must fail closed.
    fs::write(
        store.join("versions/smoke-b/1.0.0/entry.js"),
        "export default 'tampered';",
    )
    .unwrap();

    // Host-owned update sources: a real 2.0.0 for a and d, none for c.
    let digest_a2 = write_package(&sources.join("smoke-a"), "smoke-a", "2.0.0");
    write_package(&sources.join("smoke-d"), "smoke-d", "2.0.0");
    // A decoy outside the source root that a renderer might try to point at.
    write_package(&decoy.join("smoke-a"), "smoke-a", "3.0.0");

    let loader = Arc::new(SmokeLoader::default());
    let manager = ModuleOperationManager::new(
        publisher,
        loader.clone(),
        LocalModuleSources::new(sources.clone()),
    );
    World {
        root,
        store,
        sources,
        decoy,
        db,
        loader,
        manager,
        digest_a2,
    }
}

const FORBIDDEN: &str = r#"{"operationId":"fake-op","userId":"attacker","user_id":"attacker","caller":"workbench","runtimeId":"forged-runtime","runtime":"x","handle":"x","context":{"x":1},"session":"x","version":"9.9.9","integrity":"00","sha256":"00","path":"/etc","url":"https://evil.example/pkg","source":"/tmp","manifest":{"id":"x"},"permissions":["providers.call"]}"#;

fn with_forbidden(module_id: &str) -> Value {
    let mut args: Value = serde_json::from_str(FORBIDDEN).unwrap();
    args["moduleId"] = json!(module_id);
    args
}

async fn scenario(app: AppHandle, hits: Arc<Mutex<Vec<String>>>, world: World, ready: bool) {
    let mut checks = Checks::default();
    let World {
        root,
        store,
        sources,
        decoy,
        db,
        loader,
        manager,
        digest_a2,
    } = world;
    let main = "main";

    if !ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks, &root);
    }

    // -- Startup recovery: unavailable, failure, retry, fail-closed -----------
    let r = ipc(&app, main, "start_module", json!({"moduleId": "smoke-a"})).await;
    checks.check(
        "start before recovery is refused with 1004019",
        !r.success() && r.code() == Some(1004019) && loader.entered() == 0,
        r.describe(),
    );
    let r = ipc(&app, main, "stop_module", json!({"moduleId": "smoke-a"})).await;
    let accepted = r.success() && looks_like_uuid(r.data()["operation_id"].as_str().unwrap_or(""));
    checks.check(
        "stop is still accepted while the host is unavailable",
        accepted,
        r.describe(),
    );
    if accepted {
        let view = poll_operation(&app, main, r.data()["operation_id"].as_str().unwrap()).await;
        checks.check(
            "that stop reaches a terminal state",
            matches!(view["state"].as_str(), Some("succeeded" | "failed")),
            view.to_string(),
        );
    }

    db.execute_unprepared("ALTER TABLE module_release_locks RENAME TO module_release_locks_x")
        .await
        .unwrap();
    checks.check(
        "recovery fails while the publisher lock table is missing",
        manager.recover().await.is_err() && !manager.is_available(),
        "recover() unexpectedly succeeded",
    );
    let r = ipc(&app, main, "start_module", json!({"moduleId": "smoke-a"})).await;
    checks.check(
        "start after a failed recovery stays refused with 1004019",
        r.code() == Some(1004019) && loader.entered() == 0,
        r.describe(),
    );
    db.execute_unprepared("ALTER TABLE module_release_locks_x RENAME TO module_release_locks")
        .await
        .unwrap();
    let recovered = manager.recover().await;
    checks.check(
        "recovery retry succeeds and excludes the tampered module",
        matches!(&recovered, Ok(ready) if ready.iter().any(|m| m == "smoke-a")
            && ready.iter().any(|m| m == "smoke-c")
            && ready.iter().any(|m| m == "smoke-d")
            && !ready.iter().any(|m| m == "smoke-b"))
            && manager.is_available(),
        format!("{recovered:?}"),
    );
    let r = ipc(&app, main, "start_module", json!({"moduleId": "smoke-b"})).await;
    let op = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
    let view = poll_operation(&app, main, &op).await;
    checks.check(
        "tampered module fails closed with 1004009 and never reaches the loader",
        view["state"] == "failed" && view["failure"]["code"] == 1004009 && loader.entered() == 0,
        view.to_string(),
    );

    // -- Gate, operation ids, forbidden fields --------------------------------
    let r = ipc(&app, main, "start_module", with_forbidden("smoke-a")).await;
    let op_a = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
    checks.check(
        "Settings route on main returns a host-generated operation_id",
        r.success() && looks_like_uuid(&op_a) && op_a != "fake-op",
        r.describe(),
    );
    let view = poll_operation(&app, main, &op_a).await;
    checks.check(
        "start operation succeeds and is queryable",
        view["state"] == "succeeded" && view["outcome"] == "started" && loader.entered() == 1,
        view.to_string(),
    );
    let r = ipc(
        &app,
        main,
        "get_module_operation",
        json!({"operationId": "fake-op"}),
    )
    .await;
    checks.check(
        "renderer-supplied operationId was ignored (1004018)",
        r.code() == Some(1004018),
        r.describe(),
    );
    let mut status_args = with_forbidden("smoke-a");
    status_args["operationId"] = json!("fake-op");
    let r = ipc(&app, main, "get_module_runtime_status", status_args).await;
    checks.check(
        "runtime status ignores forged identity and version fields",
        r.success()
            && r.data()["state"] == "ready"
            && r.data()["version"] == "1.0.0"
            && r.data()["runtime_id"] != "forged-runtime"
            && r.data()["integrity"] != "00",
        r.describe(),
    );

    // Non-Settings route on the main window.
    let on_workbench = goto(&app, main, "/workbench").await;
    checks.check(
        "main navigated to /workbench",
        on_workbench,
        "no load event",
    );
    let before = loader.entered();
    let r = ipc(&app, main, "start_module", json!({"moduleId": "smoke-d"})).await;
    checks.check(
        "non-Settings route is denied for start (1004004)",
        r.code() == Some(1004004) && loader.entered() == before,
        r.describe(),
    );
    let r = ipc(&app, main, "stop_module", json!({"moduleId": "smoke-a"})).await;
    checks.check(
        "non-Settings route is denied for stop (1004004)",
        r.code() == Some(1004004),
        r.describe(),
    );
    let r = ipc(&app, main, "update_module", json!({"moduleId": "smoke-a"})).await;
    checks.check(
        "non-Settings route is denied for update (1004004)",
        r.code() == Some(1004004),
        r.describe(),
    );
    let r = ipc(
        &app,
        main,
        "get_module_runtime_status",
        json!({"moduleId": "smoke-a"}),
    )
    .await;
    checks.check(
        "non-Settings route is denied for status reads (1004004)",
        r.code() == Some(1004004),
        r.describe(),
    );
    let r = ipc(
        &app,
        main,
        "get_module_operation",
        json!({"operationId": op_a}),
    )
    .await;
    checks.check(
        "non-Settings route is denied for operation reads (1004004)",
        r.code() == Some(1004004),
        r.describe(),
    );
    checks.check(
        "denied calls created no operation for smoke-d",
        manager
            .runtime_status("smoke-d")
            .map(|s| s.active_operation_id.is_none())
            .unwrap_or(false),
        "smoke-d has an active operation",
    );
    checks.check(
        "main navigated back to /settings",
        goto(&app, main, "/settings").await,
        "no load event",
    );

    // Second local window on /settings: not "main".
    let events = app.state::<LoadEvents>().0.subscribe();
    let other = WebviewWindowBuilder::new(&app, "other", WebviewUrl::App("settings".into()))
        .visible(false)
        .build();
    if other.is_ok() && wait_load(events, "other", "/settings").await {
        let before = loader.entered();
        let r = ipc(
            &app,
            "other",
            "start_module",
            json!({"moduleId": "smoke-d"}),
        )
        .await;
        let layer = if r.rejected() {
            "Tauri ACL (window not in any capability)"
        } else {
            "host gate (label != main)"
        };
        checks.check(
            "non-main window on a trusted origin and /settings cannot start a module",
            !r.success() && loader.entered() == before,
            r.describe(),
        );
        println!(
            "INFO non-main window was stopped by: {layer}; reply {}",
            r.describe()
        );
        if let Some(window) = app.get_webview_window("other") {
            let _ = window.destroy();
        }
    } else {
        checks.check("second local window opened", false, format!("{other:?}"));
    }

    // Remote origin window.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tauri::async_runtime::spawn(serve(listener, hits.clone()));
    let remote_url = tauri::Url::parse(&format!("http://127.0.0.1:{port}/page")).unwrap();
    let before = loader.entered();
    let remote = WebviewWindowBuilder::new(&app, "remote", WebviewUrl::External(remote_url))
        .visible(false)
        .build();
    let _ = wait_until(|| !hits.lock().unwrap().is_empty(), "remote page report").await;
    let report = hits.lock().unwrap().first().cloned().unwrap_or_default();
    let report: Value = serde_json::from_str(&report).unwrap_or(Value::Null);
    checks.check(
        "remote-origin window cannot start a module",
        remote.is_ok()
            && !report.is_null()
            && report["value"]["success"] != true
            && loader.entered() == before,
        report.to_string(),
    );
    println!("INFO remote-origin window reply: {report}");
    if let Some(window) = app.get_webview_window("remote") {
        let _ = window.destroy();
    }

    // Path-, URL- and traversal-shaped module ids.
    for id in [
        "../smoke-a",
        "..",
        ".",
        "a/b",
        "https://evil.example/x",
        "C:\\x",
        "smoke-a/..",
        "",
    ] {
        let r = ipc(&app, main, "start_module", json!({"moduleId": id})).await;
        checks.check(
            &format!("module id {id:?} is rejected with 1004001 and no operation"),
            r.code() == Some(1004001) && r.data().is_null(),
            r.describe(),
        );
    }

    // -- Update reads only the host-owned source -------------------------------
    let mut args = with_forbidden("smoke-a");
    args["path"] = json!(decoy.join("smoke-a").display().to_string());
    args["source"] = json!(decoy.join("smoke-a").display().to_string());
    args["version"] = json!("3.0.0");
    let r = ipc(&app, main, "update_module", args).await;
    let op = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
    let view = poll_operation(&app, main, &op).await;
    let r = ipc(
        &app,
        main,
        "get_module_runtime_status",
        json!({"moduleId": "smoke-a"}),
    )
    .await;
    checks.check(
        "update uses module-sources/<id> (2.0.0), not the renderer-supplied decoy",
        view["state"] == "succeeded"
            && matches!(
                view["outcome"].as_str(),
                Some("updated" | "updated_pending_release")
            )
            && r.data()["version"] == "2.0.0"
            && r.data()["integrity"] == digest_a2.as_str()
            && store.join("versions/smoke-a/2.0.0").is_dir()
            && !store.join("versions/smoke-a/3.0.0").exists()
            && decoy.join("smoke-a/manifest.json").is_file(),
        format!("{view} / {}", r.describe()),
    );
    let r = ipc(&app, main, "update_module", json!({"moduleId": "smoke-c"})).await;
    let op = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
    let view = poll_operation(&app, main, &op).await;
    let leaked = [
        root.display().to_string(),
        sources.display().to_string(),
        "module-sources".to_owned(),
        "No such file".to_owned(),
    ]
    .iter()
    .any(|needle| view.to_string().contains(needle.as_str()));
    checks.check(
        "update without a host source fails with 1004015 and leaks no path",
        view["state"] == "failed" && view["failure"]["code"] == 1004015 && !leaked,
        view.to_string(),
    );

    // -- A page destroyed while an accepted operation is running ---------------
    let gate = loader.close_gate();
    let before = loader.entered();
    let dispatch = eval(
        &app,
        main,
        "window.__TAURI_INTERNALS__.invoke('start_module',{moduleId:'smoke-c'}); \
         await new Promise(r=>setTimeout(r,200)); return 'dispatched';",
    )
    .await;
    let entered = wait_until(|| loader.entered() > before, "loader to enter smoke-c").await;
    checks.check(
        "start_module dispatched and the operation is running behind the gate",
        entered.is_ok(),
        format!("{dispatch} {entered:?}"),
    );
    let left = goto(&app, main, "/workbench").await;
    let op_c = manager
        .runtime_status("smoke-c")
        .ok()
        .and_then(|s| s.active_operation_id);
    gate.send(true).unwrap();
    let finished = match &op_c {
        Some(id) => tokio::time::timeout(STEP_TIMEOUT, manager.wait(id))
            .await
            .ok(),
        None => None,
    };
    checks.check(
        "operation keeps running after its page was navigated away and completes",
        left && matches!(&finished, Some(Ok(v)) if v.state == dgos_types::ModuleOperationState::Succeeded),
        format!("left={left} op={op_c:?} finished={finished:?}"),
    );
    goto(&app, main, "/settings").await;
    if let Some(id) = &op_c {
        let r = ipc(
            &app,
            main,
            "get_module_operation",
            json!({"operationId": id}),
        )
        .await;
        checks.check(
            "a new page can query the abandoned operation's result",
            r.data()["state"] == "succeeded",
            r.describe(),
        );
    }

    // -- Release failure after hot update, retried through stop ---------------
    let r = ipc(&app, main, "start_module", json!({"moduleId": "smoke-d"})).await;
    let op = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
    poll_operation(&app, main, &op).await;
    loader.failing_stops.store(3, Ordering::SeqCst);
    let r = ipc(&app, main, "update_module", json!({"moduleId": "smoke-d"})).await;
    let op = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
    let view = poll_operation(&app, main, &op).await;
    let r = ipc(
        &app,
        main,
        "get_module_runtime_status",
        json!({"moduleId": "smoke-d"}),
    )
    .await;
    checks.check(
        "update with a failing old-runtime release reports updated_pending_release",
        view["state"] == "succeeded"
            && view["outcome"] == "updated_pending_release"
            && r.data()["version"] == "2.0.0"
            && r.data()["pending_releases"].as_u64().unwrap_or(0) >= 1,
        format!("{view} / {}", r.describe()),
    );
    let r = ipc(&app, main, "stop_module", json!({"moduleId": "smoke-d"})).await;
    let op = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
    let view = poll_operation(&app, main, &op).await;
    let r = ipc(
        &app,
        main,
        "get_module_runtime_status",
        json!({"moduleId": "smoke-d"}),
    )
    .await;
    checks.check(
        "stop fails with retryable 1004007 and the release stays pending",
        view["state"] == "failed"
            && view["failure"]["code"] == 1004007
            && view["failure"]["retryable"] == true
            && r.data()["pending_releases"].as_u64().unwrap_or(0) >= 1,
        format!("{view} / {}", r.describe()),
    );
    loader.failing_stops.store(0, Ordering::SeqCst);
    let r = ipc(&app, main, "stop_module", json!({"moduleId": "smoke-d"})).await;
    let op = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
    let view = poll_operation(&app, main, &op).await;
    let r = ipc(
        &app,
        main,
        "get_module_runtime_status",
        json!({"moduleId": "smoke-d"}),
    )
    .await;
    checks.check(
        "a later stop retries and clears every pending release",
        view["state"] == "succeeded"
            && r.data()["pending_releases"] == 0
            && r.data()["state"] != "ready",
        format!("{view} / {}", r.describe()),
    );

    // -- Cleanup ---------------------------------------------------------------
    for module in ["smoke-a", "smoke-c"] {
        let r = ipc(&app, main, "stop_module", json!({"moduleId": module})).await;
        let op = r.data()["operation_id"].as_str().unwrap_or("").to_owned();
        let view = poll_operation(&app, main, &op).await;
        checks.check(
            &format!("{module} stops cleanly"),
            view["state"] == "succeeded",
            view.to_string(),
        );
    }
    finish(checks, &root);
}

fn finish(checks: Checks, root: &Path) -> ! {
    let _ = fs::remove_dir_all(root);
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

pub fn run() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });

    let world = tauri::async_runtime::block_on(build_world());
    let (load_sender, _) = broadcast::channel(64);
    let hits = Arc::new(Mutex::new(Vec::new()));

    let mut context = tauri::generate_context!();
    // Windows are created below so the harness controls the first URL.
    context.config_mut().app.windows.clear();

    let manager = world.manager.clone();
    let world_cell = Mutex::new(Some(world));
    let hits_for_setup = hits.clone();
    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(LoadEvents(load_sender))
        .manage(manager)
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
            let events = app.state::<LoadEvents>().0.subscribe();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("settings".into()))
                .visible(false)
                .build()?;
            let handle = app.handle().clone();
            let world = world_cell.lock().unwrap().take().unwrap();
            let hits = hits_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                let ready = wait_load(events, "main", "/settings").await;
                scenario(handle, hits, world, ready).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_module,
            commands::stop_module,
            commands::update_module,
            commands::get_module_operation,
            commands::get_module_runtime_status,
            smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
