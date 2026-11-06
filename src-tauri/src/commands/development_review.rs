//! Stage2 read-only project review. Caller identity and lease are derived by the host.
use crate::{host::{self, Capability}, session::SessionResolver};
use dgos_agents::{AgentManager, RunRecord, files::{FileList, FileView, ListFilesInput, ReadFileInput}, review::{TaskReview, FileDiff, DiffInput}};
use dgos_types::BuiltinCaller;
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub async fn list_development_files(app: tauri::AppHandle, window: WebviewWindow, input: ListFilesInput) -> Result<FileList, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().list_development_files(context.user_id(), input).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_file(app: tauri::AppHandle, window: WebviewWindow, input: ReadFileInput) -> Result<FileView, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().read_development_file(context.user_id(), input).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn resolve_development_file_reference(app: tauri::AppHandle, window: WebviewWindow, input: dgos_agents::files::FileReferenceInput) -> Result<dgos_agents::files::FileReference, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().resolve_development_file_reference(context.user_id(), input).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_review(app: tauri::AppHandle, window: WebviewWindow, task_id: String) -> Result<TaskReview, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().read_development_review(context.user_id(), &task_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_diff(app: tauri::AppHandle, window: WebviewWindow, input: DiffInput) -> Result<FileDiff, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().read_development_diff(context.user_id(), input).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn list_development_runs(app: tauri::AppHandle, window: WebviewWindow, task_id: String) -> Result<Vec<RunRecord>, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().list_development_runs(context.user_id(), &task_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
