use super::CommandResponse;
use crate::host;
use dgos_db::{DesktopRepository, DesktopSave, DesktopSnapshot, DesktopValues};
use dgos_types::DgosError;
use tauri::{Manager, WebviewWindow};
fn failure(_: String) -> DgosError { DgosError::InternalError("desktop preferences unavailable".into()) }
#[tauri::command]
pub async fn get_desktop_preferences(app: tauri::AppHandle, window: WebviewWindow) -> CommandResponse<DesktopSnapshot> {
    let session = match host::workspace_catalog_session(&window) { Ok(session) => session, Err(error) => return CommandResponse::from(Err(error)) };
    CommandResponse::from(app.state::<DesktopRepository>().get(session.user_id()).await.map_err(failure))
}
#[tauri::command]
pub async fn set_desktop_preferences(app: tauri::AppHandle, window: WebviewWindow, values: DesktopValues, expected_revision: u64) -> CommandResponse<DesktopSave> {
    let session = match host::workspace_catalog_session(&window) { Ok(session) => session, Err(error) => return CommandResponse::from(Err(error)) };
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return CommandResponse::from(Err(error)) };
    CommandResponse::from(app.state::<DesktopRepository>().save(session.user_id(), values, expected_revision).await.map_err(failure))
}
