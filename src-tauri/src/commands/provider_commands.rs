use crate::host::{self, Capability};
use dgos_providers::ProviderManager;
use dgos_types::BuiltinCaller;
use dgos_types::{ChatRequest, ChatResponse, DgosError, Model, Provider, ProviderConfig, UsageLog};
use serde::Serialize;
use tauri::{Manager, WebviewWindow};

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
    window: WebviewWindow,
) -> CommandResponse<Vec<Provider>> {
    let context = match host::context(&window, BuiltinCaller::Settings, Capability::ProvidersRead) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager.list_providers(context.user_id()).await;
    CommandResponse::from(result)
}

/// Create a new provider
#[tauri::command]
pub async fn create_provider(
    app: tauri::AppHandle,
    window: WebviewWindow,
    config: ProviderConfig,
) -> CommandResponse<Provider> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager
        .create_provider(config, context.user_id().into())
        .await;
    CommandResponse::from(result)
}

/// Create a new provider with models
#[tauri::command]
pub async fn create_provider_with_models(
    app: tauri::AppHandle,
    window: WebviewWindow,
    config: ProviderConfig,
    models: Vec<Model>,
) -> CommandResponse<Provider> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager
        .create_provider_with_models(config, context.user_id().into(), models)
        .await;
    CommandResponse::from(result)
}

/// Update a provider
#[tauri::command]
pub async fn update_provider(
    app: tauri::AppHandle,
    window: WebviewWindow,
    id: String,
    config: ProviderConfig,
) -> CommandResponse<Provider> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager
        .update_provider(&id, config, context.user_id())
        .await;
    CommandResponse::from(result)
}

/// Toggle a provider without requiring the API key or other editable fields
#[tauri::command]
pub async fn toggle_provider(
    app: tauri::AppHandle,
    window: WebviewWindow,
    id: String,
    enabled: bool,
) -> CommandResponse<Provider> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager
        .toggle_provider(&id, enabled, context.user_id())
        .await;
    CommandResponse::from(result)
}

/// Delete a provider
#[tauri::command]
pub async fn delete_provider(
    app: tauri::AppHandle,
    window: WebviewWindow,
    id: String,
) -> CommandResponse<()> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager.delete_provider(&id, context.user_id()).await;
    CommandResponse::from(result)
}

/// Test provider connection
#[tauri::command]
pub async fn test_provider_connection(
    app: tauri::AppHandle,
    window: WebviewWindow,
    config: ProviderConfig,
) -> CommandResponse<bool> {
    let _context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager.test_connection(&config).await;
    CommandResponse::from(result)
}

/// List models for an unsaved provider configuration
#[tauri::command]
pub async fn list_models_for_config(
    app: tauri::AppHandle,
    window: WebviewWindow,
    config: ProviderConfig,
) -> CommandResponse<Vec<Model>> {
    let _context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager.list_models_for_config(&config).await;
    CommandResponse::from(result)
}

/// Test provider connection by ID (for edit mode)
#[tauri::command]
pub async fn test_provider_by_id(
    app: tauri::AppHandle,
    window: WebviewWindow,
    provider_id: String,
) -> CommandResponse<bool> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();

    let result = manager
        .test_provider_by_id(&provider_id, context.user_id())
        .await;
    CommandResponse::from(result)
}

#[tauri::command]
pub async fn migrate_provider_credential(
    app: tauri::AppHandle,
    window: WebviewWindow,
    provider_id: String,
) -> CommandResponse<Provider> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    CommandResponse::from(
        manager
            .migrate_provider_credential(&provider_id, context.user_id())
            .await,
    )
}

/// List models for a provider
#[tauri::command]
pub async fn list_models(
    app: tauri::AppHandle,
    window: WebviewWindow,
    provider_id: String,
) -> CommandResponse<Vec<Model>> {
    let context = match host::context(&window, BuiltinCaller::Settings, Capability::ProvidersRead) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager.list_models(&provider_id, context.user_id()).await;
    CommandResponse::from(result)
}

/// Refresh models for a provider
#[tauri::command]
pub async fn refresh_models(
    app: tauri::AppHandle,
    window: WebviewWindow,
    provider_id: String,
) -> CommandResponse<Vec<Model>> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager
        .refresh_models(&provider_id, context.user_id())
        .await;
    CommandResponse::from(result)
}

/// Call a provider (synchronous)
#[tauri::command]
pub async fn call_provider(
    app: tauri::AppHandle,
    window: WebviewWindow,
    provider_id: String,
    model_id: String,
    request: ChatRequest,
) -> CommandResponse<ChatResponse> {
    let context = match host::context(&window, BuiltinCaller::Settings, Capability::ProvidersCall) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let provenance = context.provenance();
    let result = manager
        .call_provider_bound_with_provenance(
            &provider_id,
            &model_id,
            request,
            context.user_id(),
            None,
            Some(&provenance),
        )
        .await;
    CommandResponse::from(result)
}

/// Update provider sort order
#[tauri::command]
pub async fn update_provider_order(
    app: tauri::AppHandle,
    window: WebviewWindow,
    provider_ids: Vec<String>,
) -> CommandResponse<()> {
    let context = match host::context(
        &window,
        BuiltinCaller::Settings,
        Capability::ProvidersManage,
    ) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager
        .update_provider_order(provider_ids, context.user_id())
        .await;
    CommandResponse::from(result)
}

/// List usage logs
#[tauri::command]
pub async fn list_usage_logs(
    app: tauri::AppHandle,
    window: WebviewWindow,
    limit: Option<usize>,
) -> CommandResponse<Vec<UsageLog>> {
    let context = match host::context(&window, BuiltinCaller::Settings, Capability::ProvidersRead) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ProviderManager>();
    let result = manager.list_usage_logs(context.user_id(), limit).await;
    CommandResponse::from(result)
}
