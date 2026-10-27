use dgos_providers::ProviderManager;
use dgos_types::{ChatRequest, ChatResponse, DgosError, Model, Provider, ProviderConfig, UsageLog};
use serde::Serialize;
use tauri::Manager;

/// Response wrapper for Tauri commands
#[derive(Serialize)]
pub struct CommandResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<CommandError>,
}

#[derive(Serialize)]
pub struct CommandError {
    pub code: u32,
    pub message: String,
}

impl<T> From<Result<T, DgosError>> for CommandResponse<T>
where
    T: Serialize,
{
    fn from(result: Result<T, DgosError>) -> Self {
        match result {
            Ok(data) => CommandResponse {
                success: true,
                data: Some(data),
                error: None,
            },
            Err(e) => CommandResponse {
                success: false,
                data: None,
                error: Some(CommandError {
                    code: e.error_code(),
                    message: e.to_string(),
                }),
            },
        }
    }
}

/// List all providers for a user
#[tauri::command]
pub async fn list_providers(
    app: tauri::AppHandle,
    user_id: String,
) -> CommandResponse<Vec<Provider>> {
    let manager = app.state::<ProviderManager>();
    let result = manager.list_providers(&user_id).await;
    CommandResponse::from(result)
}

/// Create a new provider
#[tauri::command]
pub async fn create_provider(
    app: tauri::AppHandle,
    config: ProviderConfig,
    user_id: String,
) -> CommandResponse<Provider> {
    let manager = app.state::<ProviderManager>();
    let result = manager.create_provider(config, user_id).await;
    CommandResponse::from(result)
}

/// Create a new provider with models
#[tauri::command]
pub async fn create_provider_with_models(
    app: tauri::AppHandle,
    config: ProviderConfig,
    user_id: String,
    models: Vec<Model>,
) -> CommandResponse<Provider> {
    let manager = app.state::<ProviderManager>();
    let result = manager.create_provider_with_models(config, user_id, models).await;
    CommandResponse::from(result)
}

/// Update a provider
#[tauri::command]
pub async fn update_provider(
    app: tauri::AppHandle,
    id: String,
    config: ProviderConfig,
) -> CommandResponse<Provider> {
    let manager = app.state::<ProviderManager>();
    let result = manager.update_provider(&id, config).await;
    CommandResponse::from(result)
}

/// Delete a provider
#[tauri::command]
pub async fn delete_provider(
    app: tauri::AppHandle,
    id: String,
) -> CommandResponse<()> {
    let manager = app.state::<ProviderManager>();
    let result = manager.delete_provider(&id).await;
    CommandResponse::from(result)
}

/// Test provider connection
#[tauri::command]
pub async fn test_provider_connection(
    app: tauri::AppHandle,
    config: ProviderConfig,
) -> CommandResponse<bool> {
    let manager = app.state::<ProviderManager>();
    let result = manager.test_connection(&config).await;
    CommandResponse::from(result)
}

/// List models for a provider
#[tauri::command]
pub async fn list_models(
    app: tauri::AppHandle,
    provider_id: String,
) -> CommandResponse<Vec<Model>> {
    let manager = app.state::<ProviderManager>();
    let result = manager.list_models(&provider_id).await;
    CommandResponse::from(result)
}

/// Refresh models for a provider
#[tauri::command]
pub async fn refresh_models(
    app: tauri::AppHandle,
    provider_id: String,
) -> CommandResponse<Vec<Model>> {
    let manager = app.state::<ProviderManager>();
    let result = manager.refresh_models(&provider_id).await;
    CommandResponse::from(result)
}

/// Call a provider (synchronous)
#[tauri::command]
pub async fn call_provider(
    app: tauri::AppHandle,
    provider_id: String,
    model_id: String,
    request: ChatRequest,
    user_id: String,
) -> CommandResponse<ChatResponse> {
    let manager = app.state::<ProviderManager>();
    let result = manager
        .call_provider(&provider_id, &model_id, request, &user_id)
        .await;
    CommandResponse::from(result)
}

/// Update provider sort order
#[tauri::command]
pub async fn update_provider_order(
    app: tauri::AppHandle,
    provider_ids: Vec<String>,
) -> CommandResponse<()> {
    let manager = app.state::<ProviderManager>();
    let result = manager.update_provider_order(provider_ids).await;
    CommandResponse::from(result)
}

/// List usage logs
#[tauri::command]
pub async fn list_usage_logs(
    app: tauri::AppHandle,
    user_id: String,
    limit: Option<usize>,
) -> CommandResponse<Vec<UsageLog>> {
    let manager = app.state::<ProviderManager>();
    let result = manager.list_usage_logs(&user_id, limit).await;
    CommandResponse::from(result)
}
