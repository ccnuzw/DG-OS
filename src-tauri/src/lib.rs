// DG-OS: A local-first AI application development framework
// Main library entry point

mod capabilities;
mod commands;
mod host;
mod provider_credentials;
#[cfg(feature = "webview-smoke")]
pub mod smoke;

use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_mcp::McpManager;
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Initialize database connection
            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                // Get app data directory
                let app_dir = app_handle
                    .path()
                    .app_data_dir()
                    .expect("Failed to get app data directory");

                // Create directory if it doesn't exist
                std::fs::create_dir_all(&app_dir).expect("Failed to create app data directory");

                // Database path
                let db_path = app_dir.join("dgos.db");
                let database_url = format!("sqlite://{}?mode=rwc", db_path.display());

                tracing::info!("Database URL: {}", database_url);

                // Initialize skills manager
                let skills_dir = app_dir.join("skills");
                std::fs::create_dir_all(&skills_dir).expect("Failed to create skills directory");

                let db = dgos_db::establish_connection(&database_url)
                    .await
                    .expect("Failed to connect to database");

                // Run migrations before registering bundled catalog rows.
                dgos_db::migrations::Migrator::up(&db, None)
                    .await
                    .expect("Failed to run database migrations");

                let manager = ProviderManager::with_vault(
                    &database_url,
                    Arc::new(provider_credentials::OsProviderCredentialVault),
                )
                .await
                .expect("Failed to initialize provider manager");
                manager
                    .recover_credentials(host::LOCAL_USER)
                    .await
                    .expect("Failed to inspect provider credential migration state");

                let skills_manager = SkillsManager::new(db.clone(), skills_dir);
                #[cfg(debug_assertions)]
                let bundled_dir =
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../skills");
                #[cfg(not(debug_assertions))]
                let bundled_dir = app_handle
                    .path()
                    .resource_dir()
                    .expect("Failed to locate application resources")
                    .join("skills");
                skills_manager
                    .register_bundled_catalog(&bundled_dir)
                    .await
                    .expect("Failed to validate bundled skills");
                let skills_state = commands::SkillsState {
                    manager: Arc::new(RwLock::new(skills_manager)),
                };

                let mcp_manager = Arc::new(McpManager::with_policy_and_vault(
                    db.clone(),
                    Arc::new(commands::DesktopMcpPolicy),
                    Arc::new(commands::OsCredentialVault),
                ));
                mcp_manager
                    .recover_runtime_status()
                    .await
                    .expect("Failed to recover MCP runtime status");
                mcp_manager.clone().spawn_monitor();
                let mcp_state = commands::McpState {
                    manager: mcp_manager,
                };

                tracing::info!("Database migrations completed");

                let workbench = dgos_workbench::WorkbenchManager::new(db.clone());
                workbench
                    .recover_interrupted()
                    .await
                    .expect("Failed to recover workbench task audit");

                // Keep module authorization behind a host-owned service. No
                // renderer command receives a module identity.
                let module_permissions = Arc::new(
                    dgos_db::ModulePermissionRepository::new(db.clone()),
                );
                let module_store = app_dir.join("modules");
                std::fs::create_dir_all(&module_store)
                    .expect("Failed to create module package directory");
                // Host-owned local update sources (D034). Nothing outside
                // this directory can be an update source; no URL is accepted.
                let module_sources_dir = app_dir.join("module-sources");
                std::fs::create_dir_all(&module_sources_dir)
                    .expect("Failed to create module source directory");
                // The publisher owns staging, integrity validation, atomic
                // pointer switches and startup recovery.
                let module_publisher = Arc::new(
                    dgos_modules::ModulePublisher::new(module_store, db.clone()),
                );
                let module_release = capabilities::ModuleReleaseService::new(
                    module_publisher.clone(),
                );
                let module_operations = capabilities::ModuleOperationManager::new(
                    module_publisher.clone(),
                    Arc::new(dgos_modules::LocalArtifactLoader),
                    dgos_modules::LocalModuleSources::new(module_sources_dir),
                );
                // Fail-closed (D034): a recovery failure does not abort the
                // desktop app, but start/update stay refused until a later
                // recovery succeeds. No module runtime is started here.
                match module_operations.recover().await {
                    Ok(ready) => tracing::info!(
                        "Module recovery completed: {} module(s) ready",
                        ready.len()
                    ),
                    Err(error) => tracing::error!(
                        "Module recovery failed (code {}); module start/update are disabled",
                        error.code()
                    ),
                }
                let host_capabilities =
                    capabilities::HostCapabilityService::with_publisher(
                        module_permissions,
                        module_publisher.clone(),
                    );
                app_handle.manage(capabilities::ProviderCallService::new(
                    host_capabilities.clone(),
                ));
                app_handle.manage(module_release);
                app_handle.manage(module_operations);
                app_handle.manage(host_capabilities);
                app_handle.manage(module_publisher);

                // Store managers in app state
                app_handle.manage(manager);
                app_handle.manage(skills_state);
                app_handle.manage(mcp_state);
                app_handle.manage(workbench);
            });

            // Open dev tools in debug mode
            #[cfg(debug_assertions)]
            {
                let window = app.get_webview_window("main").unwrap();
                window.open_devtools();
            }

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
            commands::call_provider,
            commands::update_provider_order,
            commands::list_usage_logs,
            commands::migrate_provider_credential,
            // Skills commands
            commands::fetch_skills_registry,
            commands::search_skills,
            commands::get_skill_detail,
            commands::install_skill,
            commands::uninstall_skill,
            commands::list_installed_skills,
            commands::toggle_skill,
            commands::update_skill_config,
            commands::call_skill,
            commands::prepare_content_skill,
            commands::check_skill_updates,
            commands::update_skill,
            commands::check_dependencies,
            commands::list_skill_usage_logs,
            commands::run_workbench_task,
            commands::list_workbench_tasks,
            commands::start_workbench_tool_task,
            commands::advance_workbench_tool_task,
            commands::cancel_workbench_tool_task,
            commands::cancel_workbench_task,
            // MCP commands
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
            // Module lifecycle commands (D034)
            commands::start_module,
            commands::stop_module,
            commands::update_module,
            commands::get_module_operation,
            commands::get_module_runtime_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let mcp = app.state::<commands::McpState>().manager.clone();
                let workbench = app.state::<dgos_workbench::WorkbenchManager>().inner().clone();
                let result = tauri::async_runtime::block_on(async move {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        async move { tokio::join!(workbench.shutdown(), mcp.shutdown()) },
                    ).await
                });
                match result {
                    Ok((Ok(()), Ok(()))) => {}
                    _ => tracing::warn!("Desktop shutdown incomplete; startup recovery will finalize interrupted audits"),
                }
            }
        });
}
