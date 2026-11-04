//! Real-host smoke for FR-005 Phase 3 (AC02, AC04, AC21 host side).
//!
//! Unlike `appearance.rs` (which registers no appearance command on purpose), this harness registers the
//! production `get_appearance_preferences` / `set_appearance_preferences` handlers over a file-backed,
//! fully migrated SQLite database, loads the shipped `dist/` in a real WebView, and drives the commands with
//! `window.__TAURI_INTERNALS__.invoke`. Page reloads run the shipped appearance controller against the real
//! host, so what the page shows after a reload is what the database returned.
//!
//! The orchestrator starts fresh Tauri processes over one temporary database, proving full snapshot and
//! language persistence across process restarts. Other platforms' WebViews remain outside this harness.

use super::{eval, goto, ipc, wait_load, Checks, LoadEvents, Pending, WATCHDOG};
use crate::commands;
use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::AppearanceRepository;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use tokio::sync::broadcast;

const PHASE_ENV: &str = "DGOS_APPEARANCE_SMOKE_PHASE";
const DIR_ENV: &str = "DGOS_APPEARANCE_SMOKE_DIR";

struct Fixture {
    dir: std::path::PathBuf,
    url: String,
    /// A second connection to the same file: fault injection and read-back, never the host's own handle.
    probe: DatabaseConnection,
}

async fn count(probe: &DatabaseConnection, sql: &str) -> i64 {
    probe
        .query_one(Statement::from_string(
            probe.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<i64>("", "n").ok())
        .unwrap_or(-1)
}

async fn sql(probe: &DatabaseConnection, statement: &str) -> bool {
    probe.execute_unprepared(statement).await.is_ok()
}

/// Wait for the first-frame gate to lift, then report what the shipped controller applied.
async fn page(app: &AppHandle) -> Value {
    let reply = eval(
        app,
        "main",
        "const root = document.documentElement; let lifted = false; \
         for (let i = 0; i < 100; i++) { if (!root.hasAttribute('data-appearance-pending')) { lifted = true; break; } \
         await new Promise(r => setTimeout(r, 50)); } \
         return { lifted, theme: root.getAttribute('data-theme'), density: root.getAttribute('data-density') };",
    )
    .await;
    if reply["ok"] == true {
        reply["value"].clone()
    } else {
        json!({"__error": reply["error"].clone()})
    }
}

async fn reload(app: &AppHandle) -> Value {
    if !goto(app, "main", "/settings").await {
        return json!({"__error": "reload never finished"});
    }
    page(app).await
}

async fn scenario(app: AppHandle, ready: bool) {
    let mut checks = Checks::default();
    let fixture = app.state::<Fixture>();
    let probe = fixture.probe.clone();
    if !ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks, &fixture.dir);
    }

    if std::env::var(PHASE_ENV).as_deref() != Ok("save") {
        let expected: Value = serde_json::from_slice(&std::fs::read(fixture.dir.join("expected.json")).unwrap()).unwrap();
        let read = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
        checks.check("fresh process reads the complete saved snapshot and revision", read.data()["snapshot"] == expected, read.describe());
        let first = page(&app).await;
        checks.check("fresh process applies saved dark/150 before lifting the gate", first["lifted"] == true && first["theme"] == "dark" && first["density"] == "150", first.to_string());
        let expected_language = if std::env::var(PHASE_ENV).as_deref() == Ok("restart-en") { "en-US" } else { "zh-CN" };
        let first = eval(&app, "main", "for(let i=0;i<100&&!window.__appearanceFirstFrame;i++)await new Promise(r=>setTimeout(r,50));return window.__appearanceFirstFrame || null;").await;
        let titles = if expected_language == "en-US" { ["Settings", "Reading account status"] } else { ["设置", "正在读取账户状态"] };
        checks.check("fresh process first visible frame uses persisted language", first["value"]["lang"] == expected_language && titles.iter().any(|title| first["value"]["text"].as_str().unwrap_or("").contains(title)), first.to_string());
        checks.check("fresh renderer applies saved preset, independent typography and effects", first["value"]["preset"] == "forest" && first["value"]["reading"] == "20px" && first["value"]["code"] == "18px" && first["value"]["motion"] == "reduce", first.to_string());
        if expected_language == "en-US" {
            let saved = ipc(&app, "main", "set_language_preferences", json!({"language":"zh-CN"})).await;
            checks.check("save Chinese for the next fresh process", saved.data()["locale"] == "zh-CN", saved.describe());
        }
        finish(checks, &fixture.dir);
    }
    let first = page(&app).await;
    checks.check("empty database loads studio light/100", first["lifted"] == true && first["theme"] == "light" && first["density"] == "100", first.to_string());
    let read = ipc(&app,"main","get_appearance_preferences",json!({})).await;
    checks.check("new snapshot revision zero",read.data()["kind"]=="ready" && read.data()["snapshot"]["revision"]==0,read.describe());
    let mut values=serde_json::to_value(dgos_types::AppearanceValues::default()).unwrap();
    values["mode"]=json!("dark");values["density"]=json!(150);
    values["variants"]["dark"]["presetId"] = json!("forest");
    values["typography"]["readingSize"] = json!(20);
    values["typography"]["codeSize"] = json!(18);
    values["effects"]["reducedMotion"] = json!("reduce");
    let saved=ipc(&app,"main","set_appearance_preferences",json!({"snapshot":values,"expectedRevision":0})).await;
    checks.check("full snapshot commits once",saved.data()["kind"]=="saved" && saved.data()["snapshot"]["revision"]==1,saved.describe());
    std::fs::write(fixture.dir.join("expected.json"), serde_json::to_vec(&saved.data()["snapshot"]).unwrap()).unwrap();
    let after=reload(&app).await;
    checks.check("reload applies confirmed dark/150",after["theme"]=="dark"&&after["density"]=="150",after.to_string());
    let reopened=dgos_db::establish_connection(&fixture.url).await.unwrap();
    let reopened=AppearanceRepository::new(reopened).get("default-user").await.unwrap();
    checks.check("independent file connection reads complete snapshot",serde_json::to_value(reopened).unwrap()["snapshot"]==saved.data()["snapshot"],saved.describe());
    let conflict=ipc(&app,"main","set_appearance_preferences",json!({"snapshot":values,"expectedRevision":0})).await;
    checks.check("stale revision returns current snapshot without writing",conflict.data()["kind"]=="conflict"&&conflict.data()["snapshot"]["revision"]==1,conflict.describe());
    let trigger=sql(&probe,"CREATE TRIGGER fail_snapshot BEFORE UPDATE ON system_settings BEGIN SELECT RAISE(ABORT,'injected'); END").await;
    checks.check("storage fault installed",trigger,"trigger");
    let failed=ipc(&app,"main","set_appearance_preferences",json!({"snapshot":values,"expectedRevision":1})).await;
    checks.check("storage failure reported without driver details",failed.code()==Some(1000000)&&!failed.describe().contains("injected"),failed.describe());
    sql(&probe,"DROP TRIGGER fail_snapshot").await;
    let after=ipc(&app,"main","get_appearance_preferences",json!({})).await;
    checks.check("failed write leaves full snapshot unchanged",after.data()["snapshot"]==saved.data()["snapshot"],after.describe());
    // Faults touch only this harness's temporary database. Preserve the restart fixture below.
    checks.check("corrupt fixture installed", sql(&probe,"UPDATE system_settings SET setting_value='broken' WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'").await, "fixture SQL");
    let corrupt=ipc(&app,"main","get_appearance_preferences",json!({})).await;
    let token=corrupt.data()["recoveryToken"].as_str().unwrap_or("").to_owned();
    let rejected=ipc(&app,"main","set_appearance_preferences",json!({"snapshot":values,"expectedRevision":0})).await;
    checks.check("corrupt read keeps raw row and rejects ordinary save",corrupt.data()["kind"]=="corrupt" && !token.is_empty() && rejected.code()==Some(1005008) && count(&probe,"SELECT COUNT(*) AS n FROM system_settings WHERE setting_value='broken'").await==1,format!("{} {}",corrupt.describe(),rejected.describe()));
    let maintenance=app.state::<dgos_db::MaintenanceGate>().begin_maintenance().unwrap();
    let blocked=ipc(&app,"main","recover_appearance_preferences",json!({"recoveryToken":token})).await;
    drop(maintenance);
    checks.check("maintenance refuses recovery without consuming its token",blocked.code()==Some(dgos_types::DgosError::ProviderRequestRejected.error_code() as u64),blocked.describe());
    let recovered=ipc(&app,"main","recover_appearance_preferences",json!({"recoveryToken":token})).await;
    let reused=ipc(&app,"main","recover_appearance_preferences",json!({"recoveryToken":token})).await;
    checks.check("explicit recovery writes full defaults at revision zero exactly once",recovered.data()["revision"]==0 && recovered.data()["mode"]=="light" && reused.code()==Some(1005008),format!("{} {}",recovered.describe(),reused.describe()));
    sql(&probe,"UPDATE system_settings SET setting_value='broken' WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'").await;
    let stale=ipc(&app,"main","get_appearance_preferences",json!({})).await;
    let future=r#"{"schemaVersion":3,"revision":9}"#;
    probe.execute(Statement::from_sql_and_values(probe.get_database_backend(),"UPDATE system_settings SET setting_value=? WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'",[future.into()])).await.unwrap();
    let read_future=ipc(&app,"main","get_appearance_preferences",json!({})).await;
    let write_future=ipc(&app,"main","set_appearance_preferences",json!({"snapshot":values,"expectedRevision":9})).await;
    let recover_future=ipc(&app,"main","recover_appearance_preferences",json!({"recoveryToken":stale.data()["recoveryToken"]})).await;
    checks.check("future schema rejects save and stale recovery without rewriting",read_future.data()["kind"]=="unsupported_schema" && write_future.code()==Some(1005009) && recover_future.code()==Some(1005008) && count(&probe,"SELECT COUNT(*) AS n FROM system_settings WHERE json_extract(setting_value,'$.schemaVersion')=3").await==1,format!("{} {} {}",read_future.describe(),write_future.describe(),recover_future.describe()));
    probe.execute(Statement::from_sql_and_values(probe.get_database_backend(),"UPDATE system_settings SET setting_value=? WHERE user_id='default-user' AND setting_key='appearance.snapshot.v2'",[saved.data()["snapshot"].to_string().into()])).await.unwrap();
    let language=ipc(&app,"main","set_language_preferences",json!({"language":"en-US"})).await;
    checks.check("language saved by real host",language.data()["locale"]=="en-US",language.describe());
    reload(&app).await;
    let english=eval(&app,"main","return {lang:document.documentElement.lang,text:document.body.textContent};").await;
    checks.check("reload first frame uses saved English",english["value"]["lang"]=="en-US"&&english["value"]["text"].as_str().unwrap_or("").contains("Settings"),english.to_string());
    let language=ipc(&app,"main","set_language_preferences",json!({"language":"zh-CN"})).await;
    checks.check("Chinese saved independently",language.data()["locale"]=="zh-CN",language.describe());
    reload(&app).await;
    let chinese=eval(&app,"main","return document.documentElement.lang;").await;
    checks.check("reload restores Chinese",chinese["value"]=="zh-CN",chinese.to_string());
    checks.check("single fixed snapshot key",count(&probe,"SELECT COUNT(*) AS n FROM system_settings WHERE user_id='default-user'").await==1,"expected one full snapshot");
    let language = ipc(&app,"main","set_language_preferences",json!({"language":"en-US"})).await;
    checks.check("save English for a fresh process", language.data()["locale"] == "en-US", language.describe());
    finish(checks, &fixture.dir);
}

fn finish(checks: Checks, _dir: &std::path::Path) -> ! {
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

pub fn run_appearance_host() {
    if std::env::var_os(PHASE_ENV).is_some() {
        run_phase();
    }
    let dir = std::env::temp_dir().join(format!("dgos-appearance-restart-{}", std::process::id()));
    std::fs::create_dir(&dir).expect("fresh temporary directory");
    let exe = std::env::current_exe().expect("current executable");
    let mut failed = false;
    for phase in ["save", "restart-en", "restart-zh"] {
        println!("== appearance phase {phase} (new process)");
        let mut child = std::process::Command::new(&exe)
            .env(PHASE_ENV, phase).env(DIR_ENV, &dir)
            .stdout(std::process::Stdio::piped()).spawn().expect("phase process");
        let mut result = false;
        for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
            println!("[{phase}] {line}");
            result |= line.starts_with("SMOKE RESULT: ") && line.ends_with(", 0 failed");
        }
        let status = child.wait().expect("phase completion");
        if !status.success() || !result {
            failed = true;
            break;
        }
    }
    std::fs::remove_dir_all(dir).expect("remove temporary database");
    println!("APPEARANCE RESTART RESULT: {}", if failed { "failed" } else { "three fresh processes passed" });
    std::process::exit(i32::from(failed));
}

fn run_phase() -> ! {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });

    let dir = std::path::PathBuf::from(std::env::var_os(DIR_ENV).expect("shared fixture directory"));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let url = format!("sqlite://{}?mode=rwc", dir.join("dgos.db").display());
    let (db, probe) = tauri::async_runtime::block_on(async {
        let db = dgos_db::establish_connection(&url)
            .await
            .expect("host connection");
        dgos_db::migrations::Migrator::up(&db, None)
            .await
            .expect("migrations");
        let probe = dgos_db::establish_connection(&url)
            .await
            .expect("probe connection");
        (db, probe)
    });

    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();

    let result = tauri::Builder::default()
        .manage(Pending::default())
        .manage(dgos_db::MaintenanceGate::new())
        .manage(crate::session::SessionResolver::no_accounts())
        .manage(LoadEvents(load_sender))
        .manage(dgos_db::HostSettingsRepository::new(db.clone()))
        .manage(AppearanceRepository::new(db))
        .manage(Fixture { dir, url, probe })
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
                .inner_size(800.0, 600.0)
                .initialization_script(r#"
                    const observer=new MutationObserver(() => {
                        const r=document.documentElement;
                        const content=document.getElementById('root');
                        if (!content?.textContent?.trim() || r.hasAttribute('data-appearance-pending')) return;
                        const s=getComputedStyle(r);
                        window.__appearanceFirstFrame={lang:r.lang,text:content.textContent,preset:r.dataset.preset,
                            reading:s.getPropertyValue('--font-size-reading').trim(),
                            code:s.getPropertyValue('--font-size-code').trim(),motion:r.dataset.reducedMotion};
                        observer.disconnect();
                    });
                    observer.observe(document,{subtree:true,childList:true,attributes:true,characterData:true});
                "#)
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
            commands::get_appearance_preferences,
            commands::set_appearance_preferences,
            commands::recover_appearance_preferences,
            commands::get_language_preferences,
            commands::set_language_preferences,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
