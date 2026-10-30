//! FR-005 Phase 8: the shipped pages in a real, visible WKWebView at 800x600.
//!
//! The page is the production `dist/`. Every page command is the production handler over a fresh,
//! fully migrated SQLite file in a temp directory (providers use the in-memory vault from
//! `dgos-providers/test-support`; the MCP vault refuses every call, so nothing touches the OS keychain).
//! Theme and density are set through the real `set_appearance_preferences` command and applied by the
//! shipped controller on each page load; the harness never writes `data-theme`/`data-density` itself.
//!
//! What this proves: computed styles, layout and overflow on the real renderer for every route in the
//! theme x density matrix. What it does not prove: VoiceOver output, other platforms' WebViews, a process
//! restart. Keyboard and overlay scenarios are in `ui_keys.rs`.

use super::modules::{build_world_with, ModWorld};

const GALLERY: &str = include_str!("../../../tests/fixtures/modules/p8-gallery.js");
use super::{eval, finish, ipc, wait_load, Checks, LoadEvents, Pending};
use crate::commands;
use crate::module_host::{self, StartupLoad};
use async_trait::async_trait;
use dgos_db::sea_orm::DatabaseConnection;
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_mcp::{CredentialScope, McpCredentialVault, McpError, McpManager, SecretToken};
use dgos_providers::test_support::MemoryCredentialVault;
use dgos_providers::ProviderManager;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::broadcast;

pub(super) const MAIN: &str = "main";

/// A second connection to the smoke database, for fault injection and read-back (never the host's own handle).
pub(super) struct UiProbe(pub(super) DatabaseConnection);

/// Matrix runs ~120 page loads; the shared 240 s watchdog is too short for it.
const UI_WATCHDOG: Duration = Duration::from_secs(1500);

const THEMES: [&str; 3] = ["light", "dark", "auto"];
const DENSITIES: [u32; 5] = [75, 100, 125, 150, 175];
const ROUTES: [&str; 8] = [
    "/",
    "/providers",
    "/skills",
    "/workbench",
    "/mcp",
    "/settings",
    "/settings/modules",
    "/no-such-route",
];

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

/// Wait for the shell, the appearance gate and the page's own loading state, then measure.
const MEASURE: &str = r#"
const root = document.documentElement;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
for (let i = 0; i < 100 && root.hasAttribute('data-appearance-pending'); i++) await wait(50);
await wait(600);
const css = (el, p) => getComputedStyle(el)[p];
const probe = (style) => { const el = document.createElement('div'); el.setAttribute('data-smoke-probe', ''); el.style.cssText = style; document.body.appendChild(el); return el; };
const bgProbe = probe('background-color: var(--color-bg-primary)');
const fgProbe = probe('color: var(--color-text-primary)');
const tokens = { bg: css(bgProbe, 'backgroundColor'), fg: css(fgProbe, 'color') };
bgProbe.remove(); fgProbe.remove();
const vw = root.clientWidth;
const bad = [];
let minFont = 999;
for (const el of document.querySelectorAll('body *')) {
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) continue;
  const own = [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
  if (own) minFont = Math.min(minFont, parseFloat(css(el, 'fontSize')));
  if (r.right > vw + 0.5 || r.left < -0.5) {
    let a = el.parentElement; let scroller = false; let clipped = false;
    while (a && a !== document.body) {
      const ox = css(a, 'overflowX');
      if (ox === 'auto' || ox === 'scroll') { scroller = true; break; }
      if (ox === 'hidden' || ox === 'clip') { clipped = true; break; }
      a = a.parentElement;
    }
    if (!scroller) bad.push({ tag: el.tagName.toLowerCase(), cls: String(el.className).slice(0, 60), text: (el.textContent || '').trim().slice(0, 30), left: Math.round(r.left), right: Math.round(r.right), clipped });
  }
}
return {
  path: location.pathname,
  theme: root.getAttribute('data-theme'), density: root.getAttribute('data-density'),
  osDark: matchMedia('(prefers-color-scheme: dark)').matches,
  rootPx: parseFloat(css(root, 'fontSize')),
  bodyBg: css(document.body, 'backgroundColor'), bodyFg: css(document.body, 'color'), tokens,
  colorScheme: css(root, 'colorScheme'),
  vw, vh: innerHeight, dpr: devicePixelRatio,
  scrollW: root.scrollWidth, bodyScrollW: document.body.scrollWidth,
  minFont: minFont === 999 ? null : minFont,
  offscreen: bad.length, offscreenSample: bad.slice(0, 4),
  hasContent: document.getElementById('root').childElementCount > 0,
  boundary: !!document.querySelector('[data-error-kind]'),
  hasProvider: document.body.innerText.includes('Provider-with-an-extremely-long'),
  hasServer: document.body.innerText.includes('Remote server with a very long'),
};
"#;

async fn seed(app: &AppHandle, checks: &mut Checks) {
    let server = ipc(
        app,
        MAIN,
        "create_mcp_server",
        json!({"request": {
            "name": "Remote server with a very long display name to test list wrapping",
            "transport": "http",
            "http": {"url": "https://mcp.example.test/a/very/long/path/segment/that/keeps/going/and/going/mcp"},
            "enabled": false, "auto_connect": false
        }}),
    )
    .await;
    checks.check(
        "seed: create_mcp_server through the production command",
        server.0["ok"] == true,
        server.describe(),
    );
}

pub(super) async fn set_appearance(app: &AppHandle, theme: &str, density: u32) -> bool {
    ipc(
        app,
        MAIN,
        "set_appearance_preferences",
        json!({"patch": {"theme": theme, "density": density}}),
    )
    .await
    .success()
}

async fn navigate(app: &AppHandle, path: &str) -> bool {
    super::goto(app, MAIN, path).await
}

fn near(v: &Value, want: f64) -> bool {
    v.as_f64().map(|x| (x - want).abs() < 0.05).unwrap_or(false)
}

async fn layout_matrix(app: &AppHandle, checks: &mut Checks, themes: &[&str], os: &str) {
    let mut cells = 0;
    for &theme in themes {
        for density in DENSITIES {
            let saved = set_appearance(app, theme, density).await;
            checks.check(
                &format!("[os={os}] {theme}/{density}: host accepted the preference"),
                saved,
                "set_appearance_preferences failed",
            );
            for route in ROUTES {
                let name = format!("[os={os}] {theme}/{density}% {route}");
                if !navigate(app, route).await {
                    checks.check(&name, false, "page never finished loading");
                    continue;
                }
                let reply = eval(app, MAIN, MEASURE).await;
                if reply["ok"] != true {
                    checks.check(&name, false, reply["error"].to_string());
                    continue;
                }
                let m = &reply["value"];
                cells += 1;
                println!("CELL {name} {m}");
                let want_theme = if theme == "auto" {
                    if m["osDark"] == true {
                        "dark"
                    } else {
                        "light"
                    }
                } else {
                    theme
                };
                let mut problems = Vec::new();
                if m["osDark"] != (os == "dark") {
                    problems.push(format!(
                        "prefers-color-scheme dark = {} but the window appearance is {os}",
                        m["osDark"]
                    ));
                }
                if m["theme"] != want_theme {
                    problems.push(format!("data-theme {} != {want_theme}", m["theme"]));
                }
                if m["density"] != density.to_string() {
                    problems.push(format!("data-density {} != {density}", m["density"]));
                }
                if !near(&m["rootPx"], 14.0 * density as f64 / 100.0) {
                    problems.push(format!(
                        "root font {} != {}",
                        m["rootPx"],
                        14.0 * density as f64 / 100.0
                    ));
                }
                if m["bodyBg"] != m["tokens"]["bg"] {
                    problems.push(format!(
                        "body bg {} != token {}",
                        m["bodyBg"], m["tokens"]["bg"]
                    ));
                }
                if m["bodyFg"] != m["tokens"]["fg"] {
                    problems.push(format!(
                        "body fg {} != token {}",
                        m["bodyFg"], m["tokens"]["fg"]
                    ));
                }
                if m["hasContent"] != true {
                    problems.push("React root is empty".into());
                }
                if m["scrollW"].as_f64() > m["vw"].as_f64()
                    || m["bodyScrollW"].as_f64() > m["vw"].as_f64()
                {
                    problems.push(format!(
                        "page scrolls horizontally: scrollW {} bodyScrollW {} vw {}",
                        m["scrollW"], m["bodyScrollW"], m["vw"]
                    ));
                }
                if m["offscreen"].as_u64().unwrap_or(0) > 0 {
                    problems.push(format!(
                        "{} element(s) past the viewport edge: {}",
                        m["offscreen"], m["offscreenSample"]
                    ));
                }
                if let Some(min) = m["minFont"].as_f64() {
                    if min < 10.99 {
                        problems.push(format!("text smaller than 11px: {min}"));
                    }
                }
                if m["vw"].as_f64().unwrap_or(0.0) < 700.0 {
                    problems.push(format!(
                        "viewport narrower than the 800px window: {}",
                        m["vw"]
                    ));
                }
                if route == "/providers" && m["hasProvider"] != true {
                    problems.push("seeded Provider is not rendered".into());
                }
                if route == "/mcp" && m["hasServer"] != true {
                    problems.push("seeded MCP server is not rendered".into());
                }
                if (route == "/no-such-route") != (m["boundary"] == true) {
                    problems.push(format!(
                        "error page presence {} is wrong for {route}",
                        m["boundary"]
                    ));
                }
                checks.check(&name, problems.is_empty(), problems.join("; "));
            }
        }
    }
    println!("INFO layout matrix measured {cells} cells");
}

async fn scenario(app: AppHandle, world: ModWorld, ready: bool) {
    let mut checks = Checks::default();
    if !ready {
        checks.check("main window loaded /", false, "page never loaded");
        finish(checks, &world.root);
    }
    let info = eval(
        &app,
        MAIN,
        "return { ua: navigator.userAgent, w: innerWidth, h: innerHeight, dpr: devicePixelRatio };",
    )
    .await;
    println!("INFO webview {}", info["value"]);
    seed(&app, &mut checks).await;
    let window = app.get_webview_window(MAIN).expect("main window");
    // The OS appearance is forced per window (NSWindow.appearance) so `auto` is exercised in both
    // states without touching the user's system setting; WebKit derives prefers-color-scheme from it.
    window.set_theme(Some(tauri::Theme::Light)).ok();
    layout_matrix(&app, &mut checks, &THEMES, "light").await;
    window.set_theme(Some(tauri::Theme::Dark)).ok();
    layout_matrix(&app, &mut checks, &["auto"], "dark").await;
    // Negative controls: the detectors must be able to fail, or a green matrix proves nothing.
    // A: an absolutely positioned 2000px element widens the page (scrollWidth and the edge check).
    // B: the same element inside an overflow:hidden box leaves scrollWidth alone; only the edge check sees it.
    let controls = [
        (
            "A: 2000px element widens the page",
            "const w = document.createElement('div'); w.style.cssText = 'position:absolute;left:0;top:0;width:2000px;height:10px'; w.textContent = 'x'; document.body.appendChild(w);",
            true,
        ),
        (
            "B: 2000px element clipped by overflow:hidden is still reported",
            "const h = document.createElement('div'); h.style.cssText = 'position:absolute;left:0;top:0;width:100px;height:10px;overflow:hidden'; const w = document.createElement('div'); w.style.cssText = 'width:2000px;height:10px'; w.textContent = 'x'; h.appendChild(w); document.body.appendChild(h);",
            false,
        ),
    ];
    for (label, inject, widens) in controls {
        if !navigate(&app, "/").await {
            checks.check(
                &format!("negative control {label}"),
                false,
                "page never loaded",
            );
            continue;
        }
        let reply = eval(&app, MAIN, &format!("{inject}{MEASURE}")).await;
        let m = &reply["value"];
        let edge = m["offscreen"].as_u64().unwrap_or(0) > 0;
        let scroll = m["scrollW"].as_f64() > m["vw"].as_f64();
        checks.check(
            &format!("negative control {label}"),
            reply["ok"] == true && edge && scroll == widens,
            format!("edge={edge} scroll={scroll} {}", reply),
        );
    }
    window.set_theme(None).ok();
    super::ui_keys::run(&app, &mut checks).await;
    window.set_theme(None).ok();
    // Leave the stored preference at its default so a rerun starts from the same state.
    set_appearance(&app, "light", 100).await;
    finish(checks, &world.root);
}

pub fn run_ui() {
    std::thread::spawn(|| {
        std::thread::sleep(UI_WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        std::process::exit(2);
    });

    let (world, bridge) =
        tauri::async_runtime::block_on(build_world_with(&[("p8-gallery", GALLERY)]));
    let db_url = format!("sqlite://{}?mode=rwc", world.root.join("dgos.db").display());
    let (provider_manager, skills, mcp, workbench, appearance, probe) =
        tauri::async_runtime::block_on(async {
            let db = dgos_db::establish_connection(&db_url).await.expect("db");
            dgos_db::migrations::Migrator::up(&db, None)
                .await
                .expect("migrate");
            let vault = Arc::new(MemoryCredentialVault::default());
            // create_provider probes the upstream's /models over the network (production behaviour), so the
            // row is seeded with the crate's own fixture instead. The name is long and unbroken on purpose.
            dgos_providers::test_support::seed_provider(
                db.clone(),
                &vault,
                dgos_types::ProviderConfig {
                    name:
                        "Provider-with-an-extremely-long-unbroken-name-that-must-wrap-not-overflow"
                            .into(),
                    provider_type: "openai_compatible".into(),
                    base_url: "https://api.example.test/v1".into(),
                    api_key: "sk-smoke-not-a-real-key".into(),
                    enabled: true,
                    config: None,
                },
                crate::host::LOCAL_USER,
            )
            .await
            .expect("seed provider");
            let providers = ProviderManager::with_vault(&db_url, vault)
                .await
                .expect("providers");
            let skills_dir = world.root.join("skills");
            std::fs::create_dir_all(&skills_dir).unwrap();
            let skills = dgos_skills::SkillsManager::new(db.clone(), skills_dir);
            skills
                .register_bundled_catalog(
                    &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../skills"),
                )
                .await
                .expect("bundled skills");
            let mcp = Arc::new(McpManager::with_policy_and_vault(
                db.clone(),
                Arc::new(commands::DesktopMcpPolicy),
                Arc::new(RefusingMcpVault),
            ));
            let workbench = dgos_workbench::WorkbenchManager::new(db.clone());
            let probe = dgos_db::establish_connection(&db_url)
                .await
                .expect("probe db");
            let appearance = dgos_db::AppearanceRepository::new(db);
            (providers, skills, mcp, workbench, appearance, probe)
        });

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
        .manage(UiProbe(probe))
        .manage(manager)
        .manage(bridge)
        .manage(router)
        .manage(crashes)
        .manage(management)
        .manage(StartupLoad)
        .manage(provider_manager)
        .manage(commands::SkillsState {
            manager: Arc::new(tokio::sync::RwLock::new(skills)),
        })
        .manage(commands::McpState { manager: mcp })
        .manage(workbench)
        .manage(appearance)
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
            // 800x600 logical: the minimum window size in tauri.conf.json. Visible, so the renderer
            // lays out and paints as it does for a user.
            WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("/".into()))
                .inner_size(800.0, 600.0)
                .min_inner_size(800.0, 600.0)
                .visible(true)
                .focused(false)
                .build()?;
            let handle = app.handle().clone();
            let world = scenario_world.lock().unwrap().take().unwrap();
            tauri::async_runtime::spawn(async move {
                let ready = wait_load(events, MAIN, "/").await;
                scenario(handle, world, ready).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_providers,
            commands::create_provider,
            commands::create_provider_with_models,
            commands::update_provider,
            commands::toggle_provider,
            commands::delete_provider,
            commands::test_provider_connection,
            commands::list_models_for_config,
            commands::test_provider_by_id,
            commands::list_models,
            commands::refresh_models,
            commands::update_provider_order,
            commands::list_usage_logs,
            commands::migrate_provider_credential,
            commands::fetch_skills_registry,
            commands::search_skills,
            commands::get_skill_detail,
            commands::install_skill,
            commands::uninstall_skill,
            commands::list_installed_skills,
            commands::toggle_skill,
            commands::update_skill_config,
            commands::check_skill_updates,
            commands::update_skill,
            commands::check_dependencies,
            commands::list_skill_usage_logs,
            commands::list_workbench_tasks,
            commands::run_workbench_task,
            commands::start_workbench_tool_task,
            commands::advance_workbench_tool_task,
            commands::cancel_workbench_tool_task,
            commands::cancel_workbench_task,
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
            commands::start_module,
            commands::stop_module,
            commands::update_module,
            commands::get_module_operation,
            commands::get_module_runtime_status,
            commands::attach_module_host,
            commands::report_module_host,
            commands::report_module_crash,
            commands::get_appearance_preferences,
            commands::set_appearance_preferences,
            commands::enable_module,
            commands::disable_module,
            commands::uninstall_module,
            commands::list_modules,
            commands::grant_module_permission,
            commands::revoke_module_permission,
            commands::list_module_crashes,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
