// DG-OS: A local-first AI application development framework
// Main library entry point

mod capabilities;
mod commands;
mod host;
mod module_host;
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

    module_host::install(tauri::Builder::default())
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
                // Modules shipped with the host (D052). The shipped directory
                // is checked against the compiled-in pins first; a mismatch
                // publishes nothing and the host runs without bundled modules.
                #[cfg(debug_assertions)]
                let bundled_modules_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../bundled-modules");
                #[cfg(not(debug_assertions))]
                let bundled_modules_dir = app_handle
                    .path()
                    .resource_dir()
                    .expect("Failed to locate application resources")
                    .join("bundled-modules");
                match module_publisher
                    .provision_builtin(&bundled_modules_dir)
                    .await
                {
                    Ok(report) => tracing::info!(
                        "Bundled modules: {} published, {} already present, {} kept at another release",
                        report.published.len(),
                        report.present.len(),
                        report.kept_other_release.len()
                    ),
                    Err(dgos_modules::ProvisionError::Bundle(violations)) => tracing::error!(
                        "Bundled modules rejected, none published: {} violation(s)",
                        violations.len()
                    ),
                    Err(error) => tracing::error!("Bundled modules could not be published: {error}"),
                }
                // Real module execution runs in the main WebView page (D042).
                // Admission is the host's pinned list (D041/D052). With no
                // page attached `start` fails closed (D043); there is no
                // fallback to a loader that does not execute code.
                let module_transport = Arc::new(module_host::ChannelTransport::default());
                let (module_loader, module_lost) = dgos_modules::PageModuleLoader::new(
                    module_transport.clone(),
                    dgos_modules::PageLoaderConfig::default(),
                );
                let module_router =
                    Arc::new(dgos_modules::ModuleCallRouter::new(module_loader.clone()));
                let module_supervisor = dgos_modules::ModuleRuntimeSupervisor::new(
                    module_publisher.clone(),
                    Arc::new(module_loader.clone()),
                )
                .with_admission(Arc::new(dgos_modules::PinnedModuleAdmission::builtin()))
                .with_ready_observer(Arc::new(module_host::RoutePublisher::new(
                    module_router.clone(),
                    module_transport.clone(),
                )));
                let module_operations = capabilities::ModuleOperationManager::with_supervisor(
                    module_publisher.clone(),
                    module_supervisor,
                    dgos_modules::LocalModuleSources::new(module_sources_dir),
                );
                module_operations
                    .spawn_lost_runtime_reporter(module_lost)
                    .expect("Failed to start the module runtime loss reporter");
                let module_host_bridge =
                    module_host::ModuleHostBridge::new(module_loader, module_transport);
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
                let module_permissions_repo = module_permissions.clone();
                let host_capabilities =
                    capabilities::HostCapabilityService::with_publisher(
                        module_permissions,
                        module_publisher.clone(),
                    );
                let module_crashes = dgos_application::ModuleCrashService::new(
                    dgos_db::ModuleCrashRepository::new(db.clone()),
                    module_operations.clone(),
                );
                // D059: a module that already crashed too often inside the
                // window is disabled before any auto-load can start it.
                match module_crashes
                    .enforce_on_startup(&dgos_types::HostSession::local_default())
                    .await
                {
                    Ok(disabled) if disabled.is_empty() => {}
                    Ok(disabled) => tracing::warn!(
                        "Startup crash evaluation disabled {} module(s): {}",
                        disabled.len(),
                        disabled.join(", ")
                    ),
                    Err(error) => {
                        tracing::error!("Startup crash evaluation failed: {error}")
                    }
                }
                app_handle.manage(dgos_application::ModuleManagementService::new(
                    module_permissions_repo,
                    module_operations.clone(),
                    module_crashes.clone(),
                ));
                app_handle.manage(module_crashes);
                app_handle.manage(module_host::StartupLoad);
                app_handle.manage(capabilities::ProviderCallService::new(
                    host_capabilities.clone(),
                ));
                app_handle.manage(capabilities::ModuleCapabilityDispatchService::new(
                    host_capabilities.clone(),
                ));
                app_handle.manage(module_router);
                app_handle.manage(module_release);
                app_handle.manage(module_operations);
                app_handle.manage(host_capabilities);
                app_handle.manage(module_publisher);
                app_handle.manage(module_host_bridge);

                // Store managers in app state
                app_handle.manage(manager);
                app_handle.manage(skills_state);
                app_handle.manage(dgos_db::AppearanceRepository::new(db.clone()));
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
            // Module host bridge (D046): page attach and receipt only
            commands::attach_module_host,
            commands::report_module_host,
            commands::report_module_crash,
            // Appearance preferences (D065): typed, no generic settings access
            commands::get_appearance_preferences,
            commands::set_appearance_preferences,
            // Module capability call (D051): typed, one command per capability
            commands::module_call_provider,
            // Module management (D055)
            commands::enable_module,
            commands::disable_module,
            commands::uninstall_module,
            commands::list_modules,
            commands::grant_module_permission,
            commands::revoke_module_permission,
            commands::list_module_crashes,
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
