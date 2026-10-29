//! Real-host smoke for FR-005 Phase 3 (AC02, AC04, AC21 host side).
//!
//! Unlike `appearance.rs` (which registers no appearance command on purpose), this harness registers the
//! production `get_appearance_preferences` / `set_appearance_preferences` handlers over a file-backed,
//! fully migrated SQLite database, loads the shipped `dist/` in a real WebView, and drives the commands with
//! `window.__TAURI_INTERNALS__.invoke`. Page reloads run the shipped appearance controller against the real
//! host, so what the page shows after a reload is what the database returned.
//!
//! What it does not prove: a full process restart (the database file is read back through a second,
//! independent connection and repository, and the page is reloaded, but the process and its first
//! connection stay alive), the controller's rollback after a failed write (the shipped UI has no control
//! that calls it yet; tests/appearance.test.tsx covers that state machine against a mocked host), or any
//! WebView other than the one on this machine.

use super::{eval, goto, ipc, wait_load, Checks, LoadEvents, Pending, WATCHDOG};
use crate::commands;
use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::AppearanceRepository;
use serde_json::{json, Value};
use std::io::Write;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::broadcast;

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

    // Empty database: nothing stored, so defaults, through the real host.
    let first = page(&app).await;
    checks.check(
        "empty database -> shipped controller shows light/100 with the gate lifted",
        first["lifted"] == true && first["theme"] == "light" && first["density"] == "100",
        first.to_string(),
    );
    let read = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
    checks.check(
        "get_appearance_preferences returns the defaults",
        read.success() && read.data() == &json!({"theme": "light", "density": 100}),
        read.describe(),
    );

    // AC02: theme persists across a reload.
    let set_theme = ipc(
        &app,
        "main",
        "set_appearance_preferences",
        json!({"patch": {"theme": "dark"}}),
    )
    .await;
    checks.check(
        "set theme=dark returns the complete preferences",
        set_theme.success() && set_theme.data() == &json!({"theme": "dark", "density": 100}),
        set_theme.describe(),
    );
    let after = reload(&app).await;
    checks.check(
        "AC02: after a reload the page applies the stored dark theme",
        after["lifted"] == true && after["theme"] == "dark" && after["density"] == "100",
        after.to_string(),
    );

    // AC04: density persists, and a density-only patch leaves the theme alone.
    let set_density = ipc(
        &app,
        "main",
        "set_appearance_preferences",
        json!({"patch": {"density": 150}}),
    )
    .await;
    checks.check(
        "set density=150 keeps theme=dark",
        set_density.success() && set_density.data() == &json!({"theme": "dark", "density": 150}),
        set_density.describe(),
    );
    let after = reload(&app).await;
    checks.check(
        "AC04: after a reload the page applies density 150 and the dark theme",
        after["lifted"] == true && after["theme"] == "dark" && after["density"] == "150",
        after.to_string(),
    );

    // A fresh connection and repository read the same values from the file.
    let reopened = dgos_db::establish_connection(&fixture.url).await;
    let reopened_prefs = match reopened {
        Ok(db) => AppearanceRepository::new(db)
            .get("default-user")
            .await
            .map(|p| serde_json::to_value(p).unwrap()),
        Err(error) => Err(dgos_types::DgosError::DatabaseError(error.to_string())),
    };
    checks.check(
        "a second connection and repository read dark/150 from the database file",
        matches!(&reopened_prefs, Ok(value) if value == &json!({"theme": "dark", "density": 150})),
        format!("{reopened_prefs:?}"),
    );

    // Identity and keys: exactly two rows, both for the host-derived user.
    checks.check(
        "stored rows are exactly the two appearance keys of default-user",
        count(&probe, "SELECT COUNT(*) AS n FROM system_settings").await == 2
            && count(
                &probe,
                "SELECT COUNT(*) AS n FROM system_settings WHERE user_id='default-user' \
                 AND setting_key IN ('appearance.theme','appearance.density')",
            )
            .await
                == 2,
        "unexpected rows",
    );

    // Illegal input: precise codes, nothing written.
    for (name, patch, code) in [
        ("unknown theme", json!({"theme": "neon"}), 1005001u64),
        ("density outside the set", json!({"density": 90}), 1005006),
        ("string density", json!({"density": "125"}), 1005006),
        ("empty patch", json!({}), 1005002),
        (
            "renderer-supplied user id",
            json!({"user_id": "mallory", "theme": "light"}),
            1005002,
        ),
        (
            "arbitrary settings key",
            json!({"http_port": 1, "theme": "light"}),
            1005002,
        ),
    ] {
        let reply = ipc(
            &app,
            "main",
            "set_appearance_preferences",
            json!({"patch": patch}),
        )
        .await;
        checks.check(
            &format!("{name} -> {code}"),
            !reply.success() && reply.code() == Some(code),
            reply.describe(),
        );
    }
    let unchanged = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
    checks.check(
        "rejected patches left dark/150 and exactly two rows",
        unchanged.data() == &json!({"theme": "dark", "density": 150})
            && count(&probe, "SELECT COUNT(*) AS n FROM system_settings").await == 2,
        unchanged.describe(),
    );

    // The renderer cannot name a user: an extra top-level argument never changes whose data is read.
    let spoof = ipc(
        &app,
        "main",
        "set_appearance_preferences",
        json!({"patch": {"theme": "auto"}, "userId": "mallory", "user_id": "mallory"}),
    )
    .await;
    checks.check(
        "an extra userId argument is ignored: default-user is updated, no mallory rows",
        spoof.success()
            && count(
                &probe,
                "SELECT COUNT(*) AS n FROM system_settings WHERE user_id <> 'default-user'",
            )
            .await
                == 0,
        spoof.describe(),
    );
    let _ = ipc(
        &app,
        "main",
        "set_appearance_preferences",
        json!({"patch": {"theme": "dark"}}),
    )
    .await;

    // AC21 host side: a failing pair write rolls back as a unit and the error carries no driver text.
    let trigger = sql(
        &probe,
        "CREATE TRIGGER fail_density BEFORE UPDATE ON system_settings WHEN NEW.setting_key='appearance.density' \
         BEGIN SELECT RAISE(ABORT, 'scripted failure'); END;",
    )
    .await;
    checks.check(
        "fault injected: density updates abort",
        trigger,
        "trigger not created",
    );
    let failed = ipc(
        &app,
        "main",
        "set_appearance_preferences",
        json!({"patch": {"theme": "light", "density": 175}}),
    )
    .await;
    let message = failed.0["value"]["error"]["message"].as_str().unwrap_or("");
    checks.check(
        "failing pair write -> success=false, code 1000000, fixed message without driver text",
        !failed.success()
            && failed.code() == Some(1000000)
            && !message.contains("system_settings")
            && !message.contains("scripted"),
        failed.describe(),
    );
    let _ = sql(&probe, "DROP TRIGGER fail_density").await;
    let rolled_back = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
    checks.check(
        "neither half of the failed pair was stored (still dark/150)",
        rolled_back.data() == &json!({"theme": "dark", "density": 150}),
        rolled_back.describe(),
    );

    // AC21 read failure on the real host: storage unavailable -> error envelope; the page falls back to defaults.
    let broken = sql(
        &probe,
        "ALTER TABLE system_settings RENAME TO system_settings_off",
    )
    .await;
    checks.check(
        "fault injected: settings table unavailable",
        broken,
        "rename failed",
    );
    let unreadable = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
    checks.check(
        "storage failure on read -> success=false, code 1000000 (not silently defaults)",
        !unreadable.success() && unreadable.code() == Some(1000000),
        unreadable.describe(),
    );
    let degraded = reload(&app).await;
    checks.check(
        "page still comes up with the gate lifted and light/100 when the read fails",
        degraded["lifted"] == true && degraded["theme"] == "light" && degraded["density"] == "100",
        degraded.to_string(),
    );
    let restored = sql(
        &probe,
        "ALTER TABLE system_settings_off RENAME TO system_settings",
    )
    .await;
    checks.check("fault removed", restored, "rename back failed");
    let recovered = reload(&app).await;
    checks.check(
        "after the fault is removed the stored dark/150 is applied again",
        recovered["lifted"] == true
            && recovered["theme"] == "dark"
            && recovered["density"] == "150",
        recovered.to_string(),
    );

    // Gate: only the trusted main window may write.
    let events = app.state::<LoadEvents>().0.subscribe();
    let other = WebviewWindowBuilder::new(&app, "other", WebviewUrl::App("settings".into()))
        .visible(false)
        .build();
    if other.is_ok() && wait_load(events, "other", "/settings").await {
        let denied = ipc(
            &app,
            "other",
            "set_appearance_preferences",
            json!({"patch": {"theme": "light"}}),
        )
        .await;
        let denied_read = ipc(&app, "other", "get_appearance_preferences", json!({})).await;
        let still = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
        checks.check(
            "a non-main window is denied (1004004) for write and read, and nothing changed",
            !denied.success()
                && denied.code() == Some(1004004)
                && !denied_read.success()
                && denied_read.code() == Some(1004004)
                && still.data() == &json!({"theme": "dark", "density": 150}),
            format!(
                "{} / {} / {}",
                denied.describe(),
                denied_read.describe(),
                still.describe()
            ),
        );
    } else {
        checks.check(
            "secondary window for the gate check",
            false,
            "window did not load",
        );
    }

    finish(checks, &fixture.dir);
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

pub fn run_appearance_host() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });

    let dir = std::env::temp_dir().join(format!(
        "dgos-appearance-host-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
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
        .manage(LoadEvents(load_sender))
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
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
