use crate::host::{self, Capability};
use dgos_agents::{capabilities::{ProjectCapabilities, CapabilityRevision, CapabilitySnapshot, CapabilityConfirmation, SaveCapabilitiesInput}, AgentManager};
use dgos_types::BuiltinCaller;
use serde::Deserialize;
use tauri::{Manager, WebviewWindow};

fn caller(app: &tauri::AppHandle, window: &WebviewWindow) -> Result<String, String> {
    let context = host::context(window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok(context.user_id().into())
}
#[tauri::command]
pub async fn read_development_capabilities(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<Option<CapabilityRevision>, String> {
    let owner = caller(&app, &window)?;
    app.state::<ProjectCapabilities>().read(&owner, &project_id).await
}
#[tauri::command]
pub async fn save_development_capabilities(app: tauri::AppHandle, window: WebviewWindow, input: SaveCapabilitiesInput) -> Result<CapabilityRevision, String> {
    let owner = caller(&app, &window)?;
    app.state::<ProjectCapabilities>().save(&owner, input).await
}
#[tauri::command]
pub async fn revoke_development_capabilities(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<i64, String> {
    let owner = caller(&app, &window)?;
    app.state::<ProjectCapabilities>().revoke(&owner, &project_id).await
}
#[tauri::command]
pub async fn read_development_capability_catalog(app: tauri::AppHandle, window: WebviewWindow) -> Result<serde_json::Value, String> {
    let owner = caller(&app, &window)?;
    app.state::<ProjectCapabilities>().catalog(&owner).await
}
#[tauri::command]
pub async fn get_development_session_capabilities(app: tauri::AppHandle, window: WebviewWindow, session_id: String) -> Result<Option<CapabilitySnapshot>, String> {
    let owner = caller(&app, &window)?;
    if !app.state::<AgentManager>().list(&owner).await.iter().any(|s| s.id == session_id) { return Err("session_not_found".into()); }
    app.state::<ProjectCapabilities>().session_snapshot(&owner, &session_id).await
}
#[tauri::command]
pub async fn list_development_capability_confirmations(app: tauri::AppHandle, window: WebviewWindow, session_id: String) -> Result<Vec<CapabilityConfirmation>, String> {
    let owner = caller(&app, &window)?;
    app.state::<ProjectCapabilities>().pending_confirmations(&owner, &session_id).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespondCapabilityInput { session_id: String, confirmation_id: String, approved: bool }
#[tauri::command]
pub async fn respond_development_capability_confirmation(app: tauri::AppHandle, window: WebviewWindow, input: RespondCapabilityInput) -> Result<(), String> {
    let owner = caller(&app, &window)?;
    app.state::<ProjectCapabilities>().respond_confirmation(&owner, &input.session_id, &input.confirmation_id, input.approved).await
}
