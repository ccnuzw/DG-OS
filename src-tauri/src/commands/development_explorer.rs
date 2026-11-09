//! Owner-scoped explorer APIs. Read-only Git projections never authorize a write.
use crate::{host::{self, Capability}, session::SessionResolver};
use dgos_agents::{AgentManager, explorer::SessionSummary, files::{CreateEntryInput, CreatedEntry, InspectEntryInput, InspectedEntry, MutateEntryInput}, git::{ProjectGitOverview, ProjectDiffInput, ProjectGitHistoryInput, GitHistoryPage}, review::FileDiff};
use dgos_types::BuiltinCaller;
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn read_development_session_summaries(app: tauri::AppHandle, window: WebviewWindow) -> Result<Vec<SessionSummary>, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().read_development_session_summaries(context.user_id()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn create_development_entry(app: tauri::AppHandle, window: WebviewWindow, input: CreateEntryInput) -> Result<CreatedEntry, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().create_development_entry(context.user_id(), input, lease.clone()).await;
    if lease.is_cancelled() { return Err("file_create_unknown".into()); }
    result
}
#[tauri::command]
pub async fn inspect_development_entry(app: tauri::AppHandle, window: WebviewWindow, input: InspectEntryInput) -> Result<InspectedEntry, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().inspect_development_entry(context.user_id(), input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn mutate_development_entry(app: tauri::AppHandle, window: WebviewWindow, input: MutateEntryInput) -> Result<CreatedEntry, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().mutate_development_entry(context.user_id(), input, lease.clone()).await;
    if lease.is_cancelled() { return Err("file_mutation_unknown".into()); }
    result
}
#[tauri::command]
pub async fn open_development_entry_directory(app: tauri::AppHandle, window: WebviewWindow, input: InspectEntryInput) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let manager = app.state::<AgentManager>();
    let admission = manager.admit_development_entry_directory(context.user_id(), input, &lease).await?;
    admission.validate(&lease)?;
    app.opener().open_path(&admission.path, None::<&str>).map_err(|_| "external_open_failed")?;
    admission.validate(&lease)?;
    Ok(())
}
#[tauri::command]
pub async fn read_development_project_git(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<ProjectGitOverview, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().read_development_project_git(context.user_id(), &project_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_project_diff(app: tauri::AppHandle, window: WebviewWindow, input: ProjectDiffInput) -> Result<FileDiff, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().read_development_project_diff(context.user_id(), input).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_project_git_history(app: tauri::AppHandle, window: WebviewWindow, input: ProjectGitHistoryInput) -> Result<GitHistoryPage, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = app.state::<AgentManager>().read_development_project_git_history(context.user_id(), input).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
