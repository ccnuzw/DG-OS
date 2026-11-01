//! Real-host smoke for V1-FR-008 local accounts (interface contract §9).
//!
//! Registers the production account commands and one ordinary command
//! (`get_appearance_preferences`) over a file-backed, migrated SQLite
//! database, with the production `SessionResolver` and `HostSessionPort`
//! (the account service builds and recovers as in `lib.rs`), loads the
//! shipped `dist/` in a real WebView and drives the commands with
//! `window.__TAURI_INTERNALS__.invoke`. Reloads run the shipped `AccountGate`
//! against the real host, so the page that appears is what the host decided.
//!
//! Proven: IPC deserialization of the typed inputs (unknown fields refused),
//! the window gate, first account as `default-user`, signed-out refusal of an
//! ordinary command with `1008007` and of account creation, wrong password
//! `1008005`, back-off `1008011` with its detail, sign-in, restart recovery
//! with preselection, delete back to first run, the gate pages after reload.
//! Not proven: module runtimes, chat or MCP being stopped (not registered
//! here; covered by crate tests), the OS vault (none configured), real
//! mouse or keyboard input, any WebView other than this machine's.

use super::{eval, goto, ipc, wait_load, Checks, LoadEvents, Pending, WATCHDOG};
use crate::commands;
use crate::session::{HostSessionPort, SessionResolver};
use dgos_application::{AccountPorts, AccountService};
use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::AppearanceRepository;
use dgos_mcp::McpManager;
use serde_json::json;
use std::io::Write;
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::broadcast;

struct Fixture {
    dir: std::path::PathBuf,
    db: DatabaseConnection,
    /// A second connection to the same file, for read-back only.
    probe: DatabaseConnection,
}

async fn scalar(probe: &DatabaseConnection, sql: &str) -> String {
    probe
        .query_one(Statement::from_string(
            probe.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get_by_index::<String>(0).ok())
        .unwrap_or_default()
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
        .and_then(|row| row.try_get_by_index::<i64>(0).ok())
        .unwrap_or(-1)
}

/// The account service as the production host builds it, over the managed
/// resolver. Called again to model a restart.
async fn recover(app: &AppHandle) -> AccountService {
    let fixture = app.state::<Fixture>();
    let resolver = app.state::<Arc<SessionResolver>>().inner().clone();
    let mcp = Arc::new(McpManager::new(fixture.db.clone()));
    let ports = AccountPorts::new(
        &fixture.db,
        Arc::new(HostSessionPort::new(resolver, mcp, app.clone())),
    );
    let service = AccountService::new(fixture.db.clone(), ports);
    service.recover().await.expect("account recovery");
    service
}

/// Heading of the page the shipped gate shows after a reload.
async fn heading(app: &AppHandle) -> String {
    if !goto(app, "main", "/").await {
        return "reload never finished".into();
    }
    let reply = eval(
        app,
        "main",
        "for (let i = 0; i < 100; i++) { const h = document.querySelector('h1'); \
         const nav = document.querySelector('nav'); \
         if (h || nav) return h ? h.textContent : 'APP'; \
         await new Promise(r => setTimeout(r, 50)); } return 'NOTHING';",
    )
    .await;
    reply["value"].as_str().unwrap_or("eval failed").to_owned()
}

async fn scenario(app: AppHandle, ready: bool) {
    let mut checks = Checks::default();
    if !ready {
        checks.check("main window loaded /settings", false, "page never loaded");
        finish(checks, &app.state::<Fixture>().dir);
    }
    let probe = app.state::<Fixture>().probe.clone();
    let accounts = recover(&app).await;
    app.manage(accounts);

    let state = ipc(&app, "main", "get_account_state", json!({})).await;
    checks.check(
        "upgraded/empty database starts as no_accounts",
        state.success() && state.data()["mode"] == "no_accounts",
        state.describe(),
    );
    let prefs = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
    checks.check(
        "no_accounts: ordinary commands run as default-user without a gate",
        prefs.success(),
        prefs.describe(),
    );
    let page = heading(&app).await;
    checks.check(
        "AC01: the shipped gate shows the welcome page",
        page == "欢迎使用 DGOS",
        &page,
    );

    let smuggled = ipc(
        &app,
        "main",
        "create_account",
        json!({"input": {"username": "alice", "password": "alice-password", "user_id": "x"}}),
    )
    .await;
    checks.check(
        "an input with an unknown field (user_id) is refused by the host",
        !smuggled.success(),
        smuggled.describe(),
    );
    let created = ipc(
        &app,
        "main",
        "create_account",
        json!({"input": {"username": "alice", "password": "alice-password"}}),
    )
    .await;
    checks.check(
        "AC02: the first account is created and signed in",
        created.success()
            && created.data()["mode"] == "signed_in"
            && created.data()["current"]["username"] == "alice",
        created.describe(),
    );
    let stored_id = scalar(&probe, "SELECT id FROM users").await;
    let hash = scalar(&probe, "SELECT password_hash FROM users").await;
    checks.check(
        "D087: the first account is default-user; AC03: only an Argon2id hash is stored",
        stored_id == "default-user"
            && hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$")
            && !hash.contains("alice-password"),
        format!("{stored_id} / {}", &hash[..hash.len().min(32)]),
    );
    let leaked = created.describe();
    checks.check(
        "no DTO carries the user id or hash",
        !leaked.contains("default-user") && !leaked.contains("argon2"),
        &leaked,
    );
    let duplicate = ipc(
        &app,
        "main",
        "create_account",
        json!({"input": {"username": "ALICE", "password": "other-password"}}),
    )
    .await;
    checks.check(
        "AC12: a case-insensitive duplicate is 1008001",
        duplicate.code() == Some(1008001),
        duplicate.describe(),
    );
    let bob = ipc(
        &app,
        "main",
        "create_account",
        json!({"input": {"username": "bob", "password": "bob-password"}}),
    )
    .await;
    checks.check(
        "signed in: a second account is created, the session stays alice",
        bob.success() && bob.data()["current"]["username"] == "alice",
        bob.describe(),
    );

    let out = ipc(&app, "main", "sign_out", json!({})).await;
    checks.check(
        "sign_out ends the session",
        out.success() && out.data()["mode"] == "signed_out",
        out.describe(),
    );
    let refused = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
    checks.check(
        "signed out: an ordinary command answers 1008007",
        refused.code() == Some(1008007),
        refused.describe(),
    );
    let no_create = ipc(
        &app,
        "main",
        "create_account",
        json!({"input": {"username": "mallory", "password": "mallory-pw"}}),
    )
    .await;
    checks.check(
        "D090: the sign-in page cannot create accounts (1008007)",
        no_create.code() == Some(1008007),
        no_create.describe(),
    );
    let page = heading(&app).await;
    checks.check(
        "the shipped gate shows the sign-in page",
        page == "登录 DGOS",
        &page,
    );

    let wrong = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "alice", "password": "not-her-password"}}),
    )
    .await;
    let unknown = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "nobody", "password": "whatever-pw"}}),
    )
    .await;
    checks.check(
        "AC05: wrong password and unknown name are the same 1008005",
        wrong.code() == Some(1008005)
            && unknown.code() == Some(1008005)
            && wrong.0["value"]["error"]["message"] == unknown.0["value"]["error"]["message"],
        format!("{} / {}", wrong.describe(), unknown.describe()),
    );
    for _ in 0..4 {
        ipc(
            &app,
            "main",
            "sign_in",
            json!({"input": {"username": "bob", "password": "wrong-password"}}),
        )
        .await;
    }
    let throttled = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "bob", "password": "wrong-password"}}),
    )
    .await;
    let still = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "bob", "password": "bob-password"}}),
    )
    .await;
    checks.check(
        "D087: after five failures even the right password waits (1008011, retry_after_seconds)",
        throttled.code() == Some(1008005)
            && still.code() == Some(1008011)
            && still.0["value"]["error"]["detail"]["retry_after_seconds"]
                .as_u64()
                .is_some_and(|seconds| (1..=30).contains(&seconds)),
        format!("{} / {}", throttled.describe(), still.describe()),
    );

    let signed_in = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "Alice", "password": "alice-password"}}),
    )
    .await;
    let prefs = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
    checks.check(
        "AC04: sign-in works case-insensitively and ordinary commands run again",
        signed_in.success() && signed_in.data()["mode"] == "signed_in" && prefs.success(),
        format!("{} / {}", signed_in.describe(), prefs.describe()),
    );

    // AC11: a restart signs out and only preselects.
    let restarted = recover(&app).await;
    app.manage(restarted.clone());
    let state = restarted
        .state()
        .await
        .map(|state| serde_json::to_value(state).unwrap());
    let refused = ipc(&app, "main", "get_appearance_preferences", json!({})).await;
    checks.check(
        "AC11: after a restart the host is signed out and alice is preselected",
        state
            .as_ref()
            .is_ok_and(|state| state["mode"] == "signed_out" && state["preselected"] == "alice")
            && refused.code() == Some(1008007),
        format!("{state:?} / {}", refused.describe()),
    );

    let signed_in = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "alice", "password": "alice-password"}}),
    )
    .await;
    let changed = ipc(
        &app,
        "main",
        "change_password",
        json!({"input": {"current_password": "alice-password", "new_password": "alice-new-password"}}),
    )
    .await;
    let old = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "alice", "password": "alice-password"}}),
    )
    .await;
    let new = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "alice", "password": "alice-new-password"}}),
    )
    .await;
    checks.check(
        "AC08: change password signs out; the old password fails, the new one works",
        signed_in.success()
            && changed.data()["mode"] == "signed_out"
            && old.code() == Some(1008005)
            && new.success(),
        format!(
            "{} / {} / {}",
            changed.describe(),
            old.describe(),
            new.describe()
        ),
    );

    let wrong_delete = ipc(
        &app,
        "main",
        "delete_account",
        json!({"input": {"password": "nope-nope"}}),
    )
    .await;
    let deleted = ipc(
        &app,
        "main",
        "delete_account",
        json!({"input": {"password": "alice-new-password"}}),
    )
    .await;
    checks.check(
        "AC09: deleting needs the password; then the other account remains, signed out",
        wrong_delete.code() == Some(1008005)
            && deleted.success()
            && deleted.data()["state"]["mode"] == "signed_out"
            && deleted.data()["credential_cleanup_pending"] == false
            && count(&probe, "SELECT COUNT(*) FROM users").await == 1
            && count(
                &probe,
                "SELECT COUNT(*) FROM system_settings WHERE user_id = 'default-user'",
            )
            .await
                == 0,
        format!("{} / {}", wrong_delete.describe(), deleted.describe()),
    );

    // Bob's back-off from above has expired only after 30 s; wait it out.
    tokio::time::sleep(std::time::Duration::from_secs(31)).await;
    let bob_in = ipc(
        &app,
        "main",
        "sign_in",
        json!({"input": {"username": "bob", "password": "bob-password"}}),
    )
    .await;
    let last = ipc(
        &app,
        "main",
        "delete_account",
        json!({"input": {"password": "bob-password"}}),
    )
    .await;
    checks.check(
        "deleting the last account returns to first run",
        bob_in.success()
            && last.success()
            && last.data()["state"]["mode"] == "no_accounts"
            && count(&probe, "SELECT COUNT(*) FROM users").await == 0,
        format!("{} / {}", bob_in.describe(), last.describe()),
    );
    let page = heading(&app).await;
    checks.check(
        "the shipped gate shows the welcome page again",
        page == "欢迎使用 DGOS",
        &page,
    );

    // Gate: only the trusted main window.
    let events = app.state::<LoadEvents>().0.subscribe();
    let other = WebviewWindowBuilder::new(&app, "other", WebviewUrl::App("settings".into()))
        .visible(false)
        .build();
    if other.is_ok() && wait_load(events, "other", "/settings").await {
        let denied = ipc(&app, "other", "get_account_state", json!({})).await;
        checks.check(
            "a non-main window is denied (1004004)",
            denied.code() == Some(1004004),
            denied.describe(),
        );
    } else {
        checks.check(
            "secondary window for the gate check",
            false,
            "window did not load",
        );
    }

    finish(checks, &app.state::<Fixture>().dir);
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

pub fn run_accounts() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });

    let dir = std::env::temp_dir().join(format!(
        "dgos-accounts-{}-{}",
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
        .manage(SessionResolver::starting())
        .manage(LoadEvents(load_sender))
        .manage(AppearanceRepository::new(db.clone()))
        .manage(Fixture { dir, db, probe })
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
            commands::get_account_state,
            commands::create_account,
            commands::sign_in,
            commands::sign_out,
            commands::change_password,
            commands::delete_account,
            commands::get_appearance_preferences,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
