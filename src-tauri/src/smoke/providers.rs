//! Real-WebView smoke for Provider CRUD operations (FR-001 AC01, AC03, AC05, AC10, AC12, AC13, AC14).
//!
//! Registers production provider management commands over a migrated SQLite database with the real
//! `ProviderManager` and drives them from the shipped `dist/` in a real WebView.
//!
//! Provider is a loopback fake that speaks OpenAI-style (not a real vendor). The credential vault
//! is in memory. Covers:
//! - AC01: Provider creation with connection test
//! - AC03: Model list fetch and caching
//! - AC05: Manual model addition
//! - AC10: Provider deletion
//! - AC12: Provider reordering (up/down buttons)
//! - AC13: Manual refresh models
//! - AC14: Usage record after call

use super::{eval, goto, ipc, smoke_report, wait_load, LoadEvents, WATCHDOG};
use crate::commands;
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_providers::{loopback, MemoryVault, ProviderCallService, ProviderManager};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

async fn scenario(app: AppHandle, _ready: ()) {
    goto(&app, "main", "/settings/providers").await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // AC01: Create provider with connection test
    println!("AC01: Creating provider...");
    let create_result = ipc(
        &app,
        "main",
        "create_provider",
        json!({
            "name": "Test OpenAI",
            "kind": "openai",
            "base_url": "http://127.0.0.1:9999",
            "api_key": "sk-fake-key-for-smoke-test"
        }),
    )
    .await;
    assert!(create_result.is_ok(), "AC01 failed: create_provider");
    let provider_id = create_result.data()["id"].as_str().expect("provider_id");
    println!("AC01 PASS: Created provider {provider_id}");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // AC03: List providers (should show the created one with models cached)
    println!("AC03: Listing providers...");
    let list_result = ipc(&app, "main", "list_providers", json!({})).await;
    assert!(list_result.is_ok(), "AC03 failed: list_providers");
    let providers = list_result.data().as_array().expect("providers array");
    assert!(!providers.is_empty(), "AC03 failed: no providers");
    let found = providers.iter().find(|p| p["id"] == provider_id);
    assert!(found.is_some(), "AC03 failed: provider not in list");
    println!("AC03 PASS: Provider listed with cached models");

    // AC03: Check models were fetched
    let models_result = ipc(&app, "main", "list_models", json!({})).await;
    assert!(models_result.is_ok(), "AC03 failed: list_models");
    let models = models_result.data().as_array().expect("models array");
    let provider_models: Vec<_> = models
        .iter()
        .filter(|m| m["provider_id"] == provider_id)
        .collect();
    assert!(!provider_models.is_empty(), "AC03 failed: no cached models");
    println!("AC03 PASS: {} models cached", provider_models.len());

    // AC05: Add manual model
    println!("AC05: Adding manual model...");
    let manual_result = ipc(
        &app,
        "main",
        "create_provider_with_models",
        json!({
            "provider_id": provider_id,
            "models": [{
                "id": "manual-test-model",
                "name": "Manual Test Model",
                "kind": "text-generation"
            }]
        }),
    )
    .await;
    assert!(manual_result.is_ok(), "AC05 failed: add manual model");

    let updated_models = ipc(&app, "main", "list_models", json!({})).await;
    let all_models = updated_models.data().as_array().expect("models");
    let manual = all_models.iter().find(|m| m["id"] == "manual-test-model");
    assert!(manual.is_some(), "AC05 failed: manual model not found");
    println!("AC05 PASS: Manual model added");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // AC12: Test provider reordering
    println!("AC12: Testing provider order...");

    // Create a second provider to enable reordering
    let create2_result = ipc(
        &app,
        "main",
        "create_provider",
        json!({
            "name": "Test Anthropic",
            "kind": "anthropic",
            "base_url": "http://127.0.0.1:9999",
            "api_key": "sk-ant-fake-for-smoke"
        }),
    )
    .await;
    assert!(create2_result.is_ok(), "AC12 setup failed: second provider");
    let provider2_id = create2_result.data()["id"].as_str().expect("provider2_id");

    tokio::time::sleep(Duration::from_millis(200)).await;

    let list_before = ipc(&app, "main", "list_providers", json!({})).await;
    let before = list_before.data().as_array().expect("providers");
    let before_ids: Vec<_> = before.iter().map(|p| p["id"].as_str().unwrap()).collect();

    // Reorder: move second provider up
    let reorder_result = ipc(
        &app,
        "main",
        "update_provider_order",
        json!({
            "provider_ids": [provider2_id, provider_id]
        }),
    )
    .await;
    assert!(reorder_result.is_ok(), "AC12 failed: update_provider_order");

    let list_after = ipc(&app, "main", "list_providers", json!({})).await;
    let after = list_after.data().as_array().expect("providers");
    let after_ids: Vec<_> = after.iter().map(|p| p["id"].as_str().unwrap()).collect();

    assert_ne!(before_ids, after_ids, "AC12 failed: order unchanged");
    assert_eq!(after_ids[0], provider2_id, "AC12 failed: wrong order");
    println!("AC12 PASS: Provider order updated");

    // AC13: Refresh models
    println!("AC13: Refreshing models...");
    let refresh_result = ipc(
        &app,
        "main",
        "refresh_models",
        json!({ "provider_id": provider_id }),
    )
    .await;
    assert!(refresh_result.is_ok(), "AC13 failed: refresh_models");
    println!("AC13 PASS: Models refreshed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // AC14: Make a call and verify usage record
    println!("AC14: Testing usage record...");

    // Get a model ID for the call
    let models_for_call = ipc(&app, "main", "list_models", json!({})).await;
    let model_list = models_for_call.data().as_array().expect("models");
    let test_model = model_list
        .iter()
        .find(|m| m["provider_id"] == provider_id && m["source"] == "remote")
        .expect("test model for call");
    let model_id = test_model["id"].as_str().expect("model_id");

    // Make a sync call
    let call_result = ipc(
        &app,
        "main",
        "call_provider",
        json!({
            "model_id": model_id,
            "messages": [{
                "role": "user",
                "content": "test"
            }],
            "temperature": 0.7,
            "max_tokens": 100,
            "stream": false
        }),
    )
    .await;
    assert!(call_result.is_ok(), "AC14 failed: call_provider");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check usage logs
    let usage_result = ipc(&app, "main", "list_usage_logs", json!({ "limit": 50 })).await;
    assert!(usage_result.is_ok(), "AC14 failed: list_usage_logs");
    let usage_logs = usage_result.data().as_array().expect("usage logs");
    let found_usage = usage_logs.iter().find(|log| log["model_id"] == model_id);
    assert!(found_usage.is_some(), "AC14 failed: no usage record");

    let log = found_usage.unwrap();
    assert!(
        log["input_tokens"].as_u64().unwrap() > 0,
        "AC14 failed: no input tokens"
    );
    assert!(
        log["output_tokens"].as_u64().unwrap() > 0,
        "AC14 failed: no output tokens"
    );
    println!("AC14 PASS: Usage record saved");

    // AC10: Delete providers (cleanup)
    println!("AC10: Deleting providers...");
    let delete1 = ipc(
        &app,
        "main",
        "delete_provider",
        json!({ "id": provider_id }),
    )
    .await;
    assert!(delete1.is_ok(), "AC10 failed: delete first provider");

    let delete2 = ipc(
        &app,
        "main",
        "delete_provider",
        json!({ "id": provider2_id }),
    )
    .await;
    assert!(delete2.is_ok(), "AC10 failed: delete second provider");

    let final_list = ipc(&app, "main", "list_providers", json!({})).await;
    let final_providers = final_list.data().as_array().expect("final providers");
    let remaining = final_providers
        .iter()
        .filter(|p| p["id"] == provider_id || p["id"] == provider2_id)
        .count();
    assert_eq!(remaining, 0, "AC10 failed: providers not deleted");
    println!("AC10 PASS: Providers deleted");

    println!("SMOKE RESULT: 7 passed, 0 failed");
    let _ = std::io::stdout().flush();
    std::process::exit(0);
}

pub fn run_providers() {
    std::thread::spawn(|| {
        std::thread::sleep(WATCHDOG);
        println!("SMOKE RESULT: watchdog expired");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });

    let dir = std::env::temp_dir().join(format!(
        "dgos-providers-smoke-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let url = format!("sqlite://{}?mode=rwc", dir.join("dgos.db").display());

    let (providers, service) = tauri::async_runtime::block_on(async {
        let db = dgos_db::establish_connection(&url).await.expect("conn");
        dgos_db::migrations::Migrator::up(&db, None)
            .await
            .expect("migrations");

        let vault = Arc::new(MemoryVault::default());
        let providers = ProviderManager::with_vault(db.clone(), vault.clone())
            .await
            .expect("ProviderManager");

        // Start loopback server
        let loopback_addr = "127.0.0.1:9999".parse().unwrap();
        tokio::spawn(loopback::serve(loopback_addr));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let service = ProviderCallService::new(providers.clone());
        (providers, service)
    });

    let context = tauri::generate_context!();
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(LoadEvents::default())
        .manage(providers)
        .manage(service)
        .setup(|app| {
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
            commands::list_providers,
            commands::create_provider,
            commands::create_provider_with_models,
            commands::update_provider,
            commands::toggle_provider,
            commands::delete_provider,
            commands::test_provider_connection,
            commands::list_models,
            commands::refresh_models,
            commands::call_provider,
            commands::update_provider_order,
            commands::list_usage_logs,
            smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
