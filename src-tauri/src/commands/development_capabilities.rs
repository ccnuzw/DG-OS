use crate::host::{self, Capability};
use dgos_agents::{capabilities::{CliProviders, CliProviderRevision, SessionConfigurationView, SaveCliProviderInput}, Engine};
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
pub async fn read_development_cli_provider(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<Option<CliProviderRevision>, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<CliProviders>().read(context.user_id(), &project_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn save_development_cli_provider(app: tauri::AppHandle, window: WebviewWindow, input: SaveCliProviderInput) -> Result<CliProviderRevision, String> {
    let (context, lease) = caller(&app, &window)?;
    let _guard = app.state::<dgos_agents::AgentManager>().admit_native_extensions(&lease).await?;
    let result = app.state::<CliProviders>().save(context.user_id(), input).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn revoke_development_cli_provider(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<i64, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<CliProviders>().revoke(context.user_id(), &project_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_cli_provider_catalog(app: tauri::AppHandle, window: WebviewWindow, project_id: String, engine: Engine) -> Result<serde_json::Value, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<CliProviders>().catalog(context.user_id(), &project_id, engine).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn get_development_session_configuration(app: tauri::AppHandle, window: WebviewWindow, session_id: String) -> Result<SessionConfigurationView, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<CliProviders>().read_session(context.user_id(), &session_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn recheck_development_cli_cleanup(app: tauri::AppHandle, window: WebviewWindow) -> Result<(), String> {
    let (_context, lease) = caller(&app, &window)?;
    let _guard = app.state::<dgos_agents::AgentManager>().admit_native_extensions(&lease).await?;
    let result = app.state::<CliProviders>().recheck_cleanup().await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result.map(|_| ())
}
