use super::*;

pub fn run_agents() {
    if super::phase6_local::real_codex_enabled() {
        assert!(std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| phase == "phase6-local"), "real Codex requires isolated phase6-local");
    }
    let root = std::env::var_os("DGOS_PROJECT_ACCEPTANCE_ROOT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("dgos-project-desktop-{}", uuid::Uuid::new_v4())));
    if std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| phase == "phase6-local") {
        super::phase6_local::isolate(&root);
    }
    let fx = tauri::async_runtime::block_on(open_fx(&root));
    let (manager, cli_providers, providers, vault) = tauri::async_runtime::block_on(load_agents(&fx));
    let watchdog = if std::env::var_os("DGOS_PROJECT_ACCEPTANCE_ROOT").is_some() { ACCEPTANCE_WATCHDOG } else { DEFAULT_WATCHDOG };
    std::thread::spawn(move || { std::thread::sleep(watchdog); eprintln!("SMOKE RESULT: watchdog expired"); std::process::exit(2); });
    let (sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Pending::default())
        .manage(SessionResolver::starting())
        .manage(LoadEvents(sender))
        .manage(dgos_db::MaintenanceGate::new())
        .manage(dgos_db::AppearanceRepository::new(fx.db.clone()))
        .manage(dgos_db::HostSettingsRepository::new(fx.db.clone()))
        .manage(dgos_db::DesktopRepository::new(fx.db.clone()))
        .manage(CliRepository::new(fx.db.clone()))
        .manage(manager)
        .manage(cli_providers)
        .manage(providers)
        .manage(vault)
        .manage(fx.clone())
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview.app_handle().state::<LoadEvents>().0.send((webview.label().into(), payload.url().path().into()));
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let db = app.state::<Fx>().db.clone();
            let resolver = app.state::<Arc<SessionResolver>>().inner().clone();
            let stage4_ui = std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| phase == "stage4-ui");
            let mcp = if stage4_ui { tauri::async_runtime::block_on(super::stage4::install(&handle, app.state::<Fx>().inner())) } else { Arc::new(McpManager::new(db.clone())) };
            if std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| phase.starts_with("stage6-") || matches!(phase.as_str(), "phase6-local" | "terminal-activity" | "worktree-runtime" | "layout-round2" | "terminal-round3" | "workflows-round3" | "round4-native" | "round4-pending-fixture")) || std::env::var_os("DGOS_AGENT_CLOSE_ONLY").is_some() {
                let fx = app.state::<Fx>();
                let skills = Arc::new(tokio::sync::RwLock::new(dgos_skills::SkillsManager::new(db.clone(), fx.root.join("skills"))));
                app.manage(skills.clone());
            }
            if std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| matches!(phase.as_str(), "phase6-local" | "layout-round2" | "terminal-round3" | "terminal-activity" | "workflows-round3" | "round4-native" | "round4-pending-fixture")) {
                tauri::async_runtime::block_on(super::layout_round2::install(app));
            }
            let accounts = tauri::async_runtime::block_on(async {
                let ports = AccountPorts::new(&db, Arc::new(HostSessionPort::new(resolver, mcp, handle.clone())));
                let service = AccountService::new(db, ports);
                service.recover().await.expect("account recovery");
                service
            });
            app.manage(accounts);
            let events = app.state::<LoadEvents>().0.subscribe();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("workbench".into())).incognito(std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| phase == "phase6-local")).inner_size(1440.0,1000.0).visible(true).build()?;
            tauri::async_runtime::spawn(async move {
                let loaded = wait_load(events, "main", "/workbench").await;
                if stage4_ui { super::stage4::run(handle, loaded).await; } else { scenario(handle, loaded).await; }
            });
            Ok(())
        })
        .invoke_handler(|invoke| {
            if std::env::var("DGOS_PROJECT_ACCEPTANCE_PHASE").is_ok_and(|phase| matches!(phase.as_str(), "phase6-local" | "layout-round2" | "terminal-round3" | "terminal-activity" | "workflows-round3" | "round4-native" | "round4-pending-fixture"))
                && matches!(invoke.message.command(), "set_appearance_preferences" | "recover_appearance_preferences" | "read_workspace_app_catalog") {
                let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![commands::set_appearance_preferences, commands::recover_appearance_preferences, commands::read_workspace_app_catalog];
                return handler(invoke);
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
            commands::get_account_state, commands::create_account, commands::sign_in, commands::sign_out,
            commands::get_appearance_preferences, commands::get_language_preferences, commands::set_language_preferences,
            commands::list_local_fonts,
            commands::get_desktop_preferences, commands::set_desktop_preferences,
            commands::get_cli_preferences, commands::set_cli_preferences,
            commands::read_development_workspace, commands::create_development_project,
            commands::update_development_project, commands::inspect_development_project,
            commands::save_development_task, commands::select_development_context, commands::classify_agent_session,
            commands::list_development_files, commands::read_development_file,
            commands::resolve_development_file_reference,
            commands::read_development_takeover, commands::close_development_takeover,
            commands::read_development_review, commands::read_development_diff, commands::list_development_runs,
            commands::read_development_session_summaries, commands::create_development_entry,
            commands::inspect_development_entry, commands::mutate_development_entry,
            commands::open_development_entry_directory,
            commands::read_development_project_git, commands::read_development_project_diff,
            commands::read_development_project_git_history,
            commands::save_development_file, commands::read_development_commands, commands::save_development_commands,
            commands::start_development_process, commands::read_development_process, commands::list_development_processes,
            commands::stop_development_process, commands::write_development_terminal, commands::resize_development_terminal,
            commands::open_development_external,
            commands::open_development_project_external,
            commands::read_development_cli_provider, commands::save_development_cli_provider,
            commands::generate_development_commit_message, commands::cancel_development_commit_message,
            commands::revoke_development_cli_provider, commands::read_development_cli_provider_catalog,
            commands::get_development_session_configuration,
            commands::read_development_workspaces, commands::save_development_workspaces,
            commands::read_development_git, commands::execute_development_git, commands::reconcile_development_git,
            commands::read_development_git_details,
            commands::read_development_git_commit, commands::read_development_git_commit_diff,
            commands::preview_development_git_pull, commands::read_development_git_history,
            commands::prepare_development_handoff, commands::list_development_handoffs, commands::accept_development_handoff,
            commands::list_agent_sessions, commands::open_agent_session, commands::get_agent_session,
            commands::send_agent_message, commands::steer_agent_turn, commands::respond_agent_request, commands::interrupt_agent_session,
            commands::observe_codex_operations, commands::control_codex_operation,
            commands::inspect_codex_operation, commands::read_codex_child_history,
            commands::close_agent_session, super::super::smoke_report,
            ];
            handler(invoke)
        })
        .build(context).expect("acceptance app");
    let exit_code = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(3));
    let captured = exit_code.clone();
    app.run(move |handle,event| {
        if let tauri::RunEvent::ExitRequested { code, .. } = event {
            if let Some(code)=code { captured.store(code,Ordering::SeqCst); }
            if let Some(resolver)=handle.try_state::<Arc<SessionResolver>>() { resolver.close_agents(); }
            let agents=handle.state::<AgentManager>().inner().clone();
            agents.close();
            let clean=tauri::async_runtime::block_on(agents.shutdown());
            if !super::stage3::exit_evidence(handle.state::<Fx>().inner(),clean) { captured.store(1,Ordering::SeqCst); }
        }
    });
    std::process::exit(exit_code.load(Ordering::SeqCst));
}
