// DG-OS: A local-first AI application development framework
// Main library entry point

mod chat_media;
mod capabilities;
mod commands;
mod host;
mod instance_lock;
mod lan;
mod logging;
mod module_host;
mod os_credentials;
mod provider_credentials;
mod provider_streams;
mod session;
#[cfg(feature = "webview-smoke")]
pub mod smoke;

use dgos_mcp::McpManager;
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging
    logging::init();

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


                // One process per data directory (D077): recovery below assumes nobody else is
                // generating. Exit before touching the database if another instance holds it.
                match instance_lock::InstanceLock::acquire(&app_dir) {
                    Ok(lock) => {
                        app_handle.manage(lock);
                    }
                    Err(instance_lock::LockError::AlreadyRunning) => {
                        tracing::error!("Another DG-OS instance is already running for this data directory");
                        std::process::exit(1);
                    }
                    Err(instance_lock::LockError::Io(error)) => {
                        tracing::error!("Unable to take the instance lock: {error}");
                        std::process::exit(1);
                    }
                }

                // Database path
                let db_path = app_dir.join("dgos.db");
                let database_url = dgos_db::startup::database_url(&db_path);

                tracing::info!("Database URL: {}", database_url);

                // Initialize skills manager
                let skills_dir = app_dir.join("skills");
                std::fs::create_dir_all(&skills_dir).expect("Failed to create skills directory");

                // Open and migrate before registering bundled catalog rows. A failure is shown
                // with the file location and nothing is repaired (FR-008 §10.3, D098 item 9,
                // D116): no other state is managed, so every command except the account gate
                // answers as unavailable, and the window shows the failure page.
                let db = match dgos_db::startup::open_database(&db_path).await {
                    Ok(db) => db,
                    Err(failure) => {
                        tracing::error!(
                            stage = failure.stage.as_str(),
                            reason = %failure.reason,
                            path = %db_path.display(),
                            "The local database could not be opened; nothing was repaired"
                        );
                        app_handle.manage(commands::DatabaseUnavailable {
                            stage: failure.stage.as_str(),
                            reason: failure.reason,
                            path: db_path.display().to_string(),
                        });
                        return;
                    }
                };

                let maintenance = dgos_db::MaintenanceGate::new();
                let manager = ProviderManager::with_vault(
                    &database_url,
                    Arc::new(provider_credentials::OsProviderCredentialVault),
                )
                .await
                .expect("Failed to initialize provider manager").with_maintenance(maintenance.clone());
                // Provider credential migration and outbox recovery run per account
                // after account recovery, below.

                let skills_manager = SkillsManager::new(db.clone(), skills_dir);
                if let Err(error) = skills_manager.recover_external_installs().await {
                    tracing::warn!(error = %error, "Skill install recovery did not complete");
                }
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

                // FR-008: the only source of the current user. Closed (`1008007`)
                // until account recovery below settles it.
                let resolver = session::SessionResolver::starting();
                app_handle.manage(resolver.clone());
                let mcp_manager = Arc::new(McpManager::with_policy_and_vault(
                    db.clone(),
                    Arc::new(commands::DesktopMcpPolicy(resolver.clone())),
                    Arc::new(commands::OsCredentialVault),
                ).with_maintenance(maintenance.clone()));
                mcp_manager
                    .recover_runtime_status()
                    .await
                    .expect("Failed to recover MCP runtime status");
                // Nobody's servers auto-connect until the session is known.
                mcp_manager.set_connection_scope(dgos_mcp::ConnectionScope::Nobody);
                mcp_manager.clone().spawn_monitor();
                let mcp_state = commands::McpState {
                    manager: mcp_manager,
                };

                tracing::info!("Database migrations completed");

                let workbench = dgos_workbench::WorkbenchManager::new(db.clone()).with_maintenance(maintenance.clone());
                workbench
                    .recover_interrupted()
                    .await
                    .expect("Failed to recover workbench task audit");

                // Chat generations left `generating` by a previous run end as interrupted; nothing
                // is resent (D074). Safe because the instance lock above is held.
                let chat = dgos_workbench::ChatManager::new(db.clone(), maintenance.clone());
                chat.recover_interrupted()
                    .await
                    .expect("Failed to recover chat generations");

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
                ).with_maintenance(maintenance.clone());
                module_operations
                    .spawn_lost_runtime_reporter(module_lost)
                    .expect("Failed to start the module runtime loss reporter");
                // Prompts of a module's `mcp.call` go to the attachment of the
                // call; a lost attachment ends its pending confirmations (D093).
                let module_mcp_confirmations =
                    Arc::new(dgos_application::ModuleMcpConfirmations::new(
                        module_transport.clone(),
                    ));
                let module_host_bridge =
                    module_host::ModuleHostBridge::new(module_loader, module_transport)
                        .with_mcp_confirmations(module_mcp_confirmations.clone());
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
                // D059's startup crash evaluation runs after account recovery, below.
                app_handle.manage(dgos_application::ModuleManagementService::new(
                    module_permissions_repo,
                    module_operations.clone(),
                    module_crashes.clone(),
                ));
                app_handle.manage(module_crashes);
                app_handle.manage(module_host::StartupLoad);
                app_handle.manage(provider_streams::ProviderStreams::default());
                app_handle.manage(capabilities::ProviderCallService::new(
                    host_capabilities.clone(),
                ));
                app_handle.manage(capabilities::ModuleCapabilityDispatchService::new(
                    host_capabilities.clone(),
                ));
                app_handle.manage(dgos_application::ModuleStorageService::new(
                    dgos_db::ModuleStorageRepository::new(db.clone()),
                ));
                app_handle.manage(module_mcp_confirmations.clone());
                app_handle.manage(dgos_application::ModuleMcpService::new(
                    module_mcp_confirmations,
                    mcp_state.manager.clone(),
                    db.clone(),
                ));
                app_handle.manage(module_router);
                app_handle.manage(module_release);
                app_handle.manage(module_operations);
                app_handle.manage(host_capabilities);
                app_handle.manage(module_publisher);
                app_handle.manage(module_host_bridge);

                let project_capabilities = dgos_agents::capabilities::ProjectCapabilities::new(dgos_db::DevelopmentRepository::new(db.clone()), manager.clone(), skills_state.manager.clone(), mcp_state.manager.clone(), app_dir.join("project-capabilities"));
                app_handle.manage(project_capabilities.clone());
                // Store managers in app state
                app_handle.manage(manager);
                app_handle.manage(skills_state);
                app_handle.manage(dgos_db::AppearanceRepository::new(db.clone()));
                app_handle.manage(dgos_db::HostSettingsRepository::new(db.clone()));
                app_handle.manage(dgos_db::DesktopRepository::new(db.clone()));
                app_handle.manage(dgos_db::CliRepository::new(db.clone()));
                app_handle.manage(maintenance.clone());
                app_handle.manage(dgos_workbench::ChatAssets::new(db.clone()));
                app_handle.manage(chat_media::Transcriptions::default());
                app_handle.manage(Arc::new(dgos_application::StorageService::new(
                    dgos_db::StorageRepository::new(db.clone()),
                    dgos_db::AssetRepository::new(db.clone()),
                    app_dir.join("skills"),
                    maintenance.clone(),
                )));
                let mcp_for_accounts = mcp_state.manager.clone();
                app_handle.manage(mcp_state);
                app_handle.manage(workbench);
                let agents = dgos_agents::AgentManager::with_maintenance(app_dir.join("agent-sessions.json"), dgos_db::DevelopmentRepository::new(db.clone()), maintenance.clone());
                agents.install_capabilities(project_capabilities);
                if agents.load().await.is_err() { tracing::error!("Agent session index unavailable; Agent admission remains closed"); }
                app_handle.manage(agents);
                app_handle.manage(chat);

                // FR-008 startup (technical design `recover`): after chat and task
                // recovery, before any command is served or module started. Finish
                // interrupted account deletions, retry both credential outboxes,
                // then settle the session (signed out after a restart when
                // accounts exist; `no_accounts` runs as `default-user`).
                // FR-010: the LAN service. Its revoker replaces FR-008's
                // `NoRemoteSessions`; it starts after account recovery so the
                // resolver is settled.
                let lan_service = lan::build(&app_handle, &db, resolver.clone());
                app_handle.manage(lan_service.clone());
                let mut ports = dgos_application::AccountPorts::new(
                    &db,
                    Arc::new(session::HostSessionPort::new(
                        resolver.clone(),
                        mcp_for_accounts.clone(),
                        app_handle.clone(),
                    )),
                );
                ports.remote = Arc::new(lan::LanSessionRevoker(lan_service.clone()));
                ports.outboxes = vec![
                    Arc::new(session::ProviderOutbox(app_handle.clone())),
                    Arc::new(session::McpOutbox(mcp_for_accounts)),
                ];
                let accounts = dgos_application::AccountService::new(db.clone(), ports);
                match accounts.recover().await {
                    Ok(_) => {}
                    // Fail closed: the resolver stays switching, every command
                    // answers 1008007 until a restart recovers.
                    Err(error) => tracing::error!(
                        code = error.error_code(),
                        "Account recovery failed; commands stay closed"
                    ),
                }
                let users = accounts.known_user_ids().await.unwrap_or_default();
                let providers = app_handle.state::<ProviderManager>();
                for user in &users {
                    if providers.recover_credentials(user).await.is_err() {
                        tracing::warn!("Provider credential migration state could not be read");
                    }
                }
                // D059: a module that already crashed too often inside the window
                // is disabled before any auto-load (which waits for a page attach
                // and a session) can start it. Module enablement is global, so
                // every account's crash log counts.
                let crashes = app_handle.state::<dgos_application::ModuleCrashService>();
                for user in &users {
                    match crashes
                        .enforce_on_startup(&dgos_types::HostSession::for_user(user))
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
                }
                app_handle.manage(accounts);
                // D039: on by default; a start failure is only shown in Settings.
                let status = lan_service.on_app_start().await;
                if status.failure_code.is_some() {
                    tracing::warn!(code = status.failure_code, "LAN service did not start");
                }
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
            commands::import_chat_asset,
            commands::preview_chat_asset,
            commands::remove_chat_asset,
            commands::transcribe_chat_audio,
            commands::cancel_chat_transcription,
            commands::get_cli_preferences,
            commands::set_cli_preferences,
            commands::probe_cli,
            commands::get_storage_overview,
            commands::preview_storage_cleanup,
            commands::execute_storage_cleanup,
            commands::preview_vacuum,
            commands::execute_vacuum,
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
            commands::list_provider_vendors,
            commands::add_manual_model,
            commands::remove_manual_model,
            commands::call_provider_stream,
            commands::cancel_provider_stream,
            // Skills commands
            commands::fetch_skills_registry,
            commands::search_skills,
            commands::get_skill_detail,
            commands::parse_skill_source,
            commands::preview_skill_install,
            commands::preview_skill_package,
            commands::confirm_skill_install,
            commands::cancel_skill_install,
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
            commands::create_chat_session,
            commands::list_chat_sessions,
            commands::rename_chat_session,
            commands::update_chat_session_target,
            commands::delete_chat_session,
            commands::search_chat_sessions,
            commands::list_chat_messages,
            commands::send_chat_message,
            commands::regenerate_chat_message,
            commands::cancel_chat_generation,
            commands::list_agent_sessions,
            commands::open_agent_session,
            commands::get_agent_session,
            commands::send_agent_message,
            commands::respond_agent_request,
            commands::interrupt_agent_session,
            commands::close_agent_session,
            commands::read_development_workspace,
            commands::create_development_project,
            commands::update_development_project,
            commands::inspect_development_project,
            commands::save_development_task,
            commands::select_development_context,
            commands::classify_agent_session,
            commands::list_development_files,
            commands::read_development_file,
            commands::read_development_review,
            commands::read_development_diff,
            commands::save_development_file,
            commands::read_development_commands,
            commands::save_development_commands,
            commands::start_development_process,
            commands::read_development_process,
            commands::list_development_processes,
            commands::stop_development_process,
            commands::write_development_terminal,
            commands::resize_development_terminal,
            commands::open_development_external,
            commands::list_development_runs,
            commands::read_development_capabilities,
            commands::save_development_capabilities,
            commands::revoke_development_capabilities,
            commands::read_development_capability_catalog,
            commands::get_development_session_capabilities,
            commands::list_development_capability_confirmations,
            commands::respond_development_capability_confirmation,
            commands::advance_chat_tool_turn,
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
             commands::recover_appearance_preferences,
             commands::get_desktop_preferences,
             commands::set_desktop_preferences,
             // Host language preference (D118): available before account sign-in
             commands::get_language_preferences,
             commands::set_language_preferences,
            // Module capability call (D051): typed, one command per capability
            commands::module_call_provider,
            commands::module_list_provider_catalog,
            commands::cancel_module_call,
            // Module capability calls (D093): typed, one command per capability
            commands::module_prepare_skill,
            commands::module_network_request,
            commands::module_storage_get,
            commands::module_storage_set,
            commands::module_storage_delete,
            commands::module_storage_list,
            commands::module_call_mcp_tool,
            commands::resolve_module_mcp_confirmation,
            // Module management (D055)
            commands::enable_module,
            commands::disable_module,
            commands::uninstall_module,
            commands::list_modules,
            commands::read_workspace_app_catalog,
            commands::grant_module_permission,
            commands::revoke_module_permission,
            commands::list_module_crashes,
            commands::list_module_permission_audits,
            // Local accounts (FR-008, contract §9): window gate only
            commands::get_account_state,
            commands::create_account,
            commands::sign_in,
            commands::sign_out,
            commands::change_password,
            commands::delete_account,
            // LAN collaboration host commands (FR-010, contract §14.8): Settings caller only
            commands::get_lan_status,
            commands::set_lan_enabled,
            commands::set_lan_port,
            commands::issue_lan_access_code,
            commands::list_lan_devices,
            commands::disconnect_lan_device,
            commands::disconnect_all_lan_devices,
            commands::list_lan_audit_events,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(resolver) = app.try_state::<Arc<session::SessionResolver>>() {
                    resolver.close_agents();
                }
                let agents = app.try_state::<dgos_agents::AgentManager>().map(|m| m.inner().clone());
                if let Some(agents) = &agents { agents.close(); }
                // Agent cleanup is independent of unrelated managers being present.
                let agent_cleanup = async move {
                    match agents { Some(agents) => agents.shutdown().await, None => true }
                };
                // Absent when startup stopped at a database failure: nothing to shut down.
                let (Some(mcp), Some(workbench), Some(chat), Some(lan)) = (
                    app.try_state::<commands::McpState>(),
                    app.try_state::<dgos_workbench::WorkbenchManager>(),
                    app.try_state::<dgos_workbench::ChatManager>(),
                    app.try_state::<Arc<dgos_lan::LanService>>(),
                ) else {
                    if !tauri::async_runtime::block_on(agent_cleanup) {
                        tracing::warn!("Agent shutdown incomplete");
                    }
                    return;
                };
                let mcp = mcp.manager.clone();
                let workbench = workbench.inner().clone();
                let chat = chat.inner().clone();
                let lan = lan.inner().clone();
                let result = tauri::async_runtime::block_on(async move {
                    // Owned process cleanup must finish even if unrelated managers
                    // exceed their shutdown deadline.
                    let other_cleanup = async {
                        tokio::time::timeout(std::time::Duration::from_secs(5), async {
                            lan.shutdown().await;
                            tokio::join!(workbench.shutdown(), mcp.shutdown(), chat.shutdown())
                        }).await
                    };
                    tokio::join!(other_cleanup, agent_cleanup)
                });
                match result {
                    (Ok((Ok(()), Ok(()), true)), true) => {}
                    _ => tracing::warn!("Desktop shutdown incomplete; startup recovery will finalize interrupted audits"),
                }
            }
        });
}
