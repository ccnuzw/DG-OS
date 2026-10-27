// DG-OS: A local-first AI application development framework
// Main library entry point

mod commands;

use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use std::sync::Arc;
use tokio::sync::RwLock;
use tauri::Manager;

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

                // Initialize provider manager
                let manager = ProviderManager::new(&database_url)
                    .await
                    .expect("Failed to initialize provider manager");

                // Initialize skills manager
                let skills_dir = app_dir.join("skills");
                std::fs::create_dir_all(&skills_dir).expect("Failed to create skills directory");

                let db = dgos_db::establish_connection(&database_url)
                    .await
                    .expect("Failed to connect to database");

                let skills_manager = SkillsManager::new(db.clone(), skills_dir);
                let skills_state = commands::SkillsState {
                    manager: Arc::new(RwLock::new(skills_manager)),
                };

                // Run migrations
                dgos_db::migrations::Migrator::up(&db, None)
                    .await
                    .expect("Failed to run migrations");

                tracing::info!("Database migrations completed");

                // Store managers in app state
                app_handle.manage(manager);
                app_handle.manage(skills_state);
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
            commands::check_skill_updates,
            commands::update_skill,
            commands::check_dependencies,
            commands::list_skill_usage_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
