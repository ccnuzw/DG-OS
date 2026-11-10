use crate::{native_extensions::{mcp_catalog::*, Domain, NativeHost}, commands::native_extensions::authorize};
use dgos_agents::AgentManager;
use tauri::{AppHandle, Manager, WebviewWindow};

#[tauri::command]
pub async fn search_native_mcp_registry(app: AppHandle, window: WebviewWindow, input: RegistrySearch) -> Result<RegistryPage, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.search_mcp_registry(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn project_native_mcp_config(app: AppHandle, window: WebviewWindow, input: ProjectionInput) -> Result<dgos_agents::native_extensions::mcp::ConfigProjection, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.project_mcp_config(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn preview_native_mcp_probe(app: AppHandle, window: WebviewWindow, input: ProbeInput) -> Result<ProbeSnapshot, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.preview_mcp_probe(&app.state::<AgentManager>(), &app.state::<dgos_db::CliRepository>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn run_native_mcp_probe(app: AppHandle, window: WebviewWindow, input: ProbeApproval) -> Result<ProbeSnapshot, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.run_mcp_probe(&app.state::<AgentManager>(), &app.state::<dgos_db::CliRepository>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn cancel_native_mcp_probe(app: AppHandle, window: WebviewWindow, input: ProbeOperation) -> Result<ProbeSnapshot, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.inspect_mcp_probe(input, true, permit, ticket).await
}
#[tauri::command]
pub async fn inspect_native_mcp_probe(app: AppHandle, window: WebviewWindow, input: ProbeOperation) -> Result<ProbeSnapshot, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.inspect_mcp_probe(input, false, permit, ticket).await
}

#[tauri::command]
pub async fn resolve_native_mcp_registry(app: AppHandle, window: WebviewWindow, input: RegistryResolve) -> Result<serde_json::Value, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.resolve_mcp_registry(&app.state::<AgentManager>(), input, permit, ticket).await
}
