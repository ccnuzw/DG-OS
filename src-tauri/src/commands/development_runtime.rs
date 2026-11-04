//! Trusted host runtime commands. The renderer selects project/task IDs, but never owner,
//! environment values, executable ownership, or a persisted PID.
use crate::{host::{self, Capability}, session::SessionResolver};
use dgos_agents::{runtime::{CommandCatalog, CommandConfiguration, ProcessSnapshot, StartProcessInput}, AgentManager};
use dgos_types::BuiltinCaller;
use serde::Deserialize;
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;

fn caller(app: &tauri::AppHandle, window: &WebviewWindow) -> Result<(String, CancellationToken), String> {
    let context = host::context(window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok((context.user_id().into(), lease))
}

#[tauri::command]
pub async fn save_development_file(app: tauri::AppHandle, window: WebviewWindow, input: dgos_agents::files::SaveFileInput) -> Result<dgos_agents::files::FileView, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().save_development_file(&owner, input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[tauri::command]
pub async fn read_development_commands(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<CommandCatalog, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().read_development_commands(&owner, &project_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveDevelopmentCommandsInput { project_id: String, commands: Vec<CommandConfiguration> }
#[tauri::command]
pub async fn save_development_commands(app: tauri::AppHandle, window: WebviewWindow, input: SaveDevelopmentCommandsInput) -> Result<CommandCatalog, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().save_development_commands(&owner, &input.project_id, input.commands).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[tauri::command]
pub async fn start_development_process(app: tauri::AppHandle, window: WebviewWindow, input: StartProcessInput) -> Result<ProcessSnapshot, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().start_development_process(&owner, input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_process(app: tauri::AppHandle, window: WebviewWindow, id: String) -> Result<ProcessSnapshot, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().read_development_process(&owner, &id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn list_development_processes(app: tauri::AppHandle, window: WebviewWindow, task_id: String) -> Result<Vec<ProcessSnapshot>, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().list_development_processes(&owner, &task_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn stop_development_process(app: tauri::AppHandle, window: WebviewWindow, id: String) -> Result<ProcessSnapshot, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().stop_development_process(&owner, &id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentTerminalInput { id: String, data: String }
#[tauri::command]
pub async fn write_development_terminal(app: tauri::AppHandle, window: WebviewWindow, input: DevelopmentTerminalInput) -> Result<(), String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().write_development_terminal(&owner, &input.id, input.data).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResizeDevelopmentTerminalInput { id: String, rows: u16, cols: u16 }
#[tauri::command]
pub async fn resize_development_terminal(app: tauri::AppHandle, window: WebviewWindow, input: ResizeDevelopmentTerminalInput) -> Result<(), String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().resize_development_terminal(&owner, &input.id, input.cols, input.rows).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDevelopmentExternalInput { project_id: String, process_id: Option<String>, target: String, editor: Option<String> }
#[tauri::command]
pub async fn open_development_external(app: tauri::AppHandle, window: WebviewWindow, input: OpenDevelopmentExternalInput) -> Result<(), String> {
    let (owner, lease) = caller(&app, &window)?;
    let manager = app.state::<AgentManager>();
    let workspace = manager.development_workspace(&owner).await?;
    let project = workspace.projects.iter().find(|project| project.id == input.project_id && !project.archived).ok_or("project_not_found")?;
    let target = match input.target.as_str() {
        "editor" => {
            if manager.inspect_project(&owner, &project.id).await?.path_state != "available" { return Err("workspace_changed".into()); }
            let editor = input.editor.as_deref().filter(|value| !value.trim().is_empty() && value.len() <= 4096 && !value.contains(['\0', '\n', '\r'])).ok_or("invalid_request")?;
            if lease.is_cancelled() { return Err("session_invalid".into()); }
            app.opener().open_path(&project.path, Some(editor)).map_err(|_| "external_open_failed")?;
            return if lease.is_cancelled() { Err("session_invalid".into()) } else { Ok(()) };
        }
        "preview" => {
            let id = input.process_id.ok_or("process_not_found")?;
            let snapshot = manager.read_development_process(&owner, &id).await?;
            if snapshot.project_id != project.id || snapshot.status != "ready" { return Err("service_not_ready".into()); }
            snapshot.preview_url.ok_or("service_not_ready")?
        }
        _ => return Err("invalid_request".into()),
    };
    if !target.starts_with("http://127.0.0.1:") && !target.starts_with("http://[::1]:") && !target.starts_with("http://localhost:") { return Err("service_not_ready".into()); }
    app.opener().open_url(target, None::<&str>).map_err(|_| "external_open_failed")?;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok(())
}
