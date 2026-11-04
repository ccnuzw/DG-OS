use super::CommandResponse;
use crate::host;
use dgos_db::{CliPaths, CliRepository, CliSave, CliSnapshot};
use dgos_agents::cli::{CliTool, CliProbe};
use dgos_types::DgosError;
use tauri::{Manager, WebviewWindow};
fn failure(_: String) -> DgosError { DgosError::InternalError("CLI preferences unavailable".into()) }
#[tauri::command]
pub async fn get_cli_preferences(app: tauri::AppHandle, window: WebviewWindow) -> CommandResponse<CliSnapshot> {
    let session = match host::workspace_catalog_session(&window) { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    CommandResponse::from(app.state::<CliRepository>().get(session.user_id()).await.map_err(failure))
}
#[tauri::command]
pub async fn set_cli_preferences(app: tauri::AppHandle, window: WebviewWindow, paths: CliPaths, expected_revision: u64) -> CommandResponse<CliSave> {
    let session = match host::workspace_catalog_session(&window) { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return CommandResponse::from(Err(error)) };
    CommandResponse::from(app.state::<CliRepository>().save(session.user_id(), paths, expected_revision).await.map_err(failure))
}
#[tauri::command]
pub async fn probe_cli(app: tauri::AppHandle, window: WebviewWindow, tool: CliTool, path: Option<String>) -> CommandResponse<CliProbe> {
    let session = match host::workspace_catalog_session(&window) { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (current, lease) = match resolver.agent_session() { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    // A probe is valid only for the account that issued it. Account switches cancel the lease.
    if current.user_id() != session.user_id() || lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    let result = tokio::select! { _ = lease.cancelled() => Err(DgosError::SessionInvalid), result = dgos_agents::cli::probe(tool, path.as_deref()) => Ok(result) };
    CommandResponse::from(result)
}
