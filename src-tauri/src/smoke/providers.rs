//! Real-WebView smoke for Provider CRUD operations (FR-001 AC01, AC03, AC05, AC10, AC12, AC13, AC14).
//!
//! Registers production provider management commands over a migrated SQLite database with the real
//! `ProviderManager`, host session resolver and Settings authorization, and drives the shipped `dist/`.
//!
//! Provider is a loopback fake with OpenAI-compatible model/completion and Anthropic model-list
//! responses (not a real vendor). The credential vault is in memory. Covers:
//! - AC01: Provider creation with connection test
//! - AC03: Model list fetch and caching
//! - AC05: Manual model addition
//! - AC10: Provider deletion
//! - AC12: Provider reordering (up/down buttons)
//! - AC13: Manual refresh models
//! - AC14: Usage record after call
//!
//! Run: `cargo test -p dgos --features provider-smoke --test webview_smoke_providers`.
//! This CRUD/usage entrance complements `provider_stream`'s AC33 stream/cancellation coverage.

use super::{goto, ipc, wait_load, LoadEvents, Pending, WATCHDOG};
use crate::commands;
use dgos_application::{HostCapabilityService, ProviderCallService};
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_db::ModulePermissionRepository;
use dgos_modules::ModulePublisher;
use dgos_providers::test_support::MemoryCredentialVault;
use dgos_providers::ProviderManager;
use serde_json::json;
use std::io::Write;
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// A bound listener makes startup deterministic and avoids a fixed-port collision. The fixture
/// serves model discovery for both protocols and a non-streaming OpenAI completion with usage.
async fn serve(listener: TcpListener) {
    loop {
        let (mut socket, _) = listener.accept().await.expect("accept loopback request");
        tauri::async_runtime::spawn(async move {
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head_end, body_len) = loop {
                let read = socket.read(&mut chunk).await.expect("read loopback request");
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&chunk[..read]);
                if let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..split]).to_ascii_lowercase();
                    let length = head.lines().find_map(|line| {
                        line.strip_prefix("content-length:")?.trim().parse::<usize>().ok()
                    }).unwrap_or(0);
                    break (split + 4, length);
                }
            };
            while request.len() - head_end < body_len {
                let read = socket.read(&mut chunk).await.expect("read loopback body");
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&chunk[..read]);
            }
            let target = String::from_utf8_lossy(&request[..head_end]);
            let route = target.lines().next().unwrap_or_default();
            let (status, body) = if route.starts_with("GET /models ")
                || route.starts_with("GET /v1/models?")
            {
                ("200 OK", json!({
                    "data": [{ "id": "smoke-chat", "type": "model", "display_name": "Smoke Chat" }],
                    "has_more": false
                }))
            } else if route.starts_with("POST /chat/completions ") {
                ("200 OK", json!({
                    "id": "smoke-completion", "object": "chat.completion",
                    "choices": [{ "index": 0, "message": { "role": "assistant", "content": "smoke reply" }, "finish_reason": "stop" }],
                    "usage": { "prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10 }
                }))
            } else {
                ("404 Not Found", json!({ "error": "unsupported smoke route" }))
            };
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.expect("write loopback response");
            socket.shutdown().await.expect("close loopback response");
        });
    }
}

async fn scenario(app: AppHandle, ready: bool, base_url: String, dir: std::path::PathBuf) {
    assert!(ready, "driver failed: initial Settings page did not load");
    assert!(goto(&app, "main", "/settings/providers").await, "driver failed: Providers page did not load");

    // AC01: Create provider with connection test
    println!("AC01: Creating provider...");
    let create_result = ipc(
        &app,
        "main",
        "create_provider",
        json!({
            "config": {
                "name": "Test OpenAI",
                "provider_type": "openai_compatible",
                "base_url": base_url,
                "api_key": "sk-fake-key-for-smoke-test",
                "enabled": true
            }
        }),
    )
    .await;
    assert!(create_result.success(), "AC01 failed: {}", create_result.describe());
    let provider_id = create_result.data()["id"].as_str().expect("provider_id");
    println!("AC01 PASS: Created provider {provider_id}");

    // AC03: Creation verifies connectivity; discovery is explicitly refreshed into the cache.
    println!("AC03: Listing providers...");
    let list_result = ipc(&app, "main", "list_providers", json!({})).await;
    assert!(list_result.success(), "AC03 failed: {}", list_result.describe());
    let providers = list_result.data().as_array().expect("providers array");
    assert!(!providers.is_empty(), "AC03 failed: no providers");
    let found = providers.iter().find(|p| p["id"] == provider_id);
    assert!(found.is_some(), "AC03 failed: provider not in list");
    println!("AC03: Provider listed; fetching its model catalog...");

    let discovered = ipc(&app, "main", "refresh_models", json!({ "providerId": provider_id })).await;
    assert!(discovered.success(), "AC03 failed: {}", discovered.describe());
    let models_result = ipc(&app, "main", "list_models", json!({ "providerId": provider_id })).await;
    assert!(models_result.success(), "AC03 failed: {}", models_result.describe());
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
        "add_manual_model",
        json!({
            "providerId": provider_id,
            "modelId": "manual-test-model",
            "kind": "text-generation"
        }),
    )
    .await;
    assert!(manual_result.success(), "AC05 failed: {}", manual_result.describe());

    let updated_models = ipc(&app, "main", "list_models", json!({ "providerId": provider_id })).await;
    assert!(updated_models.success(), "AC05 failed: {}", updated_models.describe());
    let all_models = updated_models.data().as_array().expect("models");
    let manual = all_models.iter().find(|m| m["model_id"] == "manual-test-model");
    assert!(manual.is_some_and(|model| model["source"] == "manual" && model["model_type"] == "text-generation"), "AC05 failed: manual model missing or misclassified");
    println!("AC05 PASS: Manual model added");

    // AC12: Test provider reordering
    println!("AC12: Testing provider order...");

    // Create a second provider to enable reordering
    let create2_result = ipc(
        &app,
        "main",
        "create_provider",
        json!({
            "config": {
                "name": "Test Anthropic",
                "provider_type": "anthropic",
                "base_url": base_url,
                "api_key": "sk-ant-fake-for-smoke",
                "enabled": true
            }
        }),
    )
    .await;
    assert!(create2_result.success(), "AC12 setup failed: {}", create2_result.describe());
    let provider2_id = create2_result.data()["id"].as_str().expect("provider2_id");

    let list_before = ipc(&app, "main", "list_providers", json!({})).await;
    assert!(list_before.success(), "AC12 setup failed: {}", list_before.describe());
    let before = list_before.data().as_array().expect("providers");
    let before_ids: Vec<_> = before.iter().map(|p| p["id"].as_str().unwrap()).collect();

    // Reorder: move second provider up
    let reorder_result = ipc(
        &app,
        "main",
        "update_provider_order",
        json!({
            "providerIds": [provider2_id, provider_id]
        }),
    )
    .await;
    assert!(reorder_result.success(), "AC12 failed: {}", reorder_result.describe());

    let list_after = ipc(&app, "main", "list_providers", json!({})).await;
    assert!(list_after.success(), "AC12 failed: {}", list_after.describe());
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
        json!({ "providerId": provider_id }),
    )
    .await;
    assert!(refresh_result.success(), "AC13 failed: {}", refresh_result.describe());
    println!("AC13 PASS: Models refreshed");

    // AC14: Make a call and verify usage record
    println!("AC14: Testing usage record...");

    // Get a model ID for the call
    let models_for_call = ipc(&app, "main", "list_models", json!({ "providerId": provider_id })).await;
    assert!(models_for_call.success(), "AC14 setup failed: {}", models_for_call.describe());
    let model_list = models_for_call.data().as_array().expect("models");
    let test_model = model_list
        .iter()
        .find(|m| m["provider_id"] == provider_id && m["source"] == "remote")
        .expect("test model for call");
    let model_id = test_model["model_id"].as_str().expect("model_id");

    // Make a sync call
    let call_result = ipc(
        &app,
        "main",
        "call_provider",
        json!({
            "providerId": provider_id,
            "modelId": model_id,
            "request": {
                "messages": [{ "role": "user", "content": "test" }],
                "temperature": 0.7,
                "max_tokens": 100,
                "stream": false
            }
        }),
    )
    .await;
    assert!(call_result.success(), "AC14 failed: {}", call_result.describe());
    assert_eq!(call_result.data()["content"], "smoke reply", "AC14 failed: unexpected completion");

    // Check usage logs
    let usage_result = ipc(&app, "main", "list_usage_logs", json!({ "limit": 50 })).await;
    assert!(usage_result.success(), "AC14 failed: {}", usage_result.describe());
    let usage_logs = usage_result.data().as_array().expect("usage logs");
    let found_usage = usage_logs.iter().find(|log| log["model_id"] == model_id);
    assert!(found_usage.is_some(), "AC14 failed: no usage record");

    let log = found_usage.unwrap();
    assert!(
        log["request_tokens"].as_u64().unwrap() > 0,
        "AC14 failed: no input tokens"
    );
    assert!(
        log["response_tokens"].as_u64().unwrap() > 0,
        "AC14 failed: no output tokens"
    );
    assert!(log["success"] == true && log["caller"] == "settings", "AC14 failed: usage was not attributed to Settings: {log}");
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
    assert!(delete1.success(), "AC10 failed: {}", delete1.describe());

    let delete2 = ipc(
        &app,
        "main",
        "delete_provider",
        json!({ "id": provider2_id }),
    )
    .await;
    assert!(delete2.success(), "AC10 failed: {}", delete2.describe());

    let final_list = ipc(&app, "main", "list_providers", json!({})).await;
    assert!(final_list.success(), "AC10 failed: {}", final_list.describe());
    let final_providers = final_list.data().as_array().expect("final providers");
    let remaining = final_providers
        .iter()
        .filter(|p| p["id"] == provider_id || p["id"] == provider2_id)
        .count();
    assert_eq!(remaining, 0, "AC10 failed: providers not deleted");
    println!("AC10 PASS: Providers deleted");
    let _ = std::fs::remove_dir_all(dir);

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
    std::fs::create_dir_all(dir.join("modules")).expect("temp dir");
    let url = format!("sqlite://{}?mode=rwc", dir.join("dgos.db").display());

    let (providers, service, maintenance, base_url) = tauri::async_runtime::block_on(async {
        let maintenance = dgos_db::MaintenanceGate::new();
        let db = dgos_db::establish_connection(&url).await.expect("conn");
        dgos_db::migrations::Migrator::up(&db, None)
            .await
            .expect("migrations");

        let vault = Arc::new(MemoryCredentialVault::default());
        let providers = ProviderManager::with_vault(&url, vault)
            .await
            .expect("ProviderManager")
            .with_maintenance(maintenance.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
        let base_url = format!("http://{}", listener.local_addr().expect("loopback address"));
        tauri::async_runtime::spawn(serve(listener));

        let publisher = Arc::new(ModulePublisher::new(dir.join("modules"), db.clone()));
        let permissions = Arc::new(ModulePermissionRepository::new(db));
        let capabilities = HostCapabilityService::with_publisher(permissions, publisher);
        let service = ProviderCallService::new(capabilities);
        (providers, service, maintenance, base_url)
    });

    let (load_sender, _) = broadcast::channel(64);
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Pending::default())
        // This isolated database has no accounts; use the production resolver's pre-account state,
        // as the other WebView fixtures do. All commands still pass the host Settings gate.
        .manage(crate::session::SessionResolver::no_accounts())
        .manage(LoadEvents(load_sender))
        .manage(maintenance)
        .manage(providers)
        .manage(service)
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview.app_handle().state::<LoadEvents>().0.send((
                    webview.label().to_owned(), payload.url().path().to_owned(),
                ));
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
                let scenario = tauri::async_runtime::spawn(async move {
                    let ready = wait_load(events, "main", "/settings").await;
                    scenario(handle, ready, base_url, dir).await;
                });
                if let Err(error) = scenario.await {
                    println!("SMOKE RESULT: provider scenario failed: {error}");
                    let _ = std::io::stdout().flush();
                    std::process::exit(1);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_providers,
            commands::create_provider,
            commands::create_provider_with_models,
            commands::add_manual_model,
            commands::update_provider,
            commands::toggle_provider,
            commands::delete_provider,
            commands::test_provider_connection,
            commands::list_models,
            commands::refresh_models,
            commands::call_provider,
            commands::update_provider_order,
            commands::list_usage_logs,
            super::smoke_report,
        ])
        .run(context);
    if let Err(error) = result {
        println!("SMOKE RESULT: tauri failed to run: {error}");
    }
    std::process::exit(3);
}
