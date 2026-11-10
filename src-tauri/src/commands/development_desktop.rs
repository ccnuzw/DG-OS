use crate::host::{self, Capability};
use dgos_agents::{AgentManager, desktop::{DesktopState, WorkspaceTrust, SaveDesktopStateInput, ReadWorkspaceTrustInput, SetWorkspaceTrustInput, ActivateWorkspaceAgentInput}};
use dgos_types::{BuiltinCaller, HostExecutionContext};
use tauri::{Manager, WebviewWindow};
use tokio_util::sync::CancellationToken;

fn caller(app: &tauri::AppHandle, window: &WebviewWindow) -> Result<(host::Admission<HostExecutionContext>, CancellationToken), String> {
    let context = host::context(window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok((context, lease))
}
#[tauri::command]
pub async fn read_development_desktop_state(app: tauri::AppHandle, window: WebviewWindow) -> Result<DesktopState, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().read_development_desktop_state(context.user_id(), lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn save_development_desktop_state(app: tauri::AppHandle, window: WebviewWindow, input: SaveDesktopStateInput) -> Result<DesktopState, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().save_development_desktop_state(context.user_id(), input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_workspace_trust(app: tauri::AppHandle, window: WebviewWindow, input: ReadWorkspaceTrustInput) -> Result<WorkspaceTrust, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().read_development_workspace_trust(context.user_id(), input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn activate_development_workspace_agent(app: tauri::AppHandle, window: WebviewWindow, input: ActivateWorkspaceAgentInput) -> Result<WorkspaceTrust, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().activate_development_workspace_agent(context.user_id(), input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn set_development_workspace_trust(app: tauri::AppHandle, window: WebviewWindow, input: SetWorkspaceTrustInput) -> Result<WorkspaceTrust, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().set_development_workspace_trust(context.user_id(), input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyDevelopmentExitInput { request_id: String, proceed: bool }
#[tauri::command]
pub async fn reply_development_exit(app: tauri::AppHandle, window: WebviewWindow, input: ReplyDevelopmentExitInput) -> Result<(), String> {
    // Quit must also work on the signed-out account page. Authority is the
    // trusted native window plus the one-shot host-issued pending request.
    host::account_window(&window).map_err(|e| e.to_string())?;
    let coordinator = app.try_state::<crate::desktop_exit::ExitCoordinator>().ok_or("exit_not_pending")?;
    if coordinator.complete(window.label(), &input.request_id, input.proceed)? { app.exit(0); }
    Ok(())
}
