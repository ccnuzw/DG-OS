use crate::{host::{self, Capability}, session::SessionResolver};
use dgos_agents::{takeover::{CloseTakeoverInput, ReadTakeoverInput, TakeoverView}, AgentManager};
use dgos_types::BuiltinCaller;
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};
use tokio_util::sync::CancellationToken;

fn caller(app: &tauri::AppHandle, window: &WebviewWindow) -> Result<(String, CancellationToken), String> {
    let context = host::context(window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok((context.user_id().into(), lease))
}
#[tauri::command]
pub async fn read_development_takeover(app: tauri::AppHandle, window: WebviewWindow, input: ReadTakeoverInput) -> Result<TakeoverView, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().read_development_takeover(&owner, input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn close_development_takeover(app: tauri::AppHandle, window: WebviewWindow, input: CloseTakeoverInput) -> Result<TakeoverView, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().close_development_takeover(&owner, input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
