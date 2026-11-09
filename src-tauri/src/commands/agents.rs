use crate::host::{self, Capability};
use dgos_agents::{AgentManager, Engine, OwnedSessionView, SessionOptions, SessionRecord};
use dgos_types::BuiltinCaller;
use serde::Deserialize;
use serde_json::Value;
use tauri::{Manager, WebviewWindow};
use dgos_agents::projects::{CreateProjectInput, UpdateProjectInput, SaveTaskInput, ClassifySessionInput, ProjectInspection};
use dgos_db::development_repository::{DevelopmentWorkspace, DevelopmentTask, Project, Selection};

#[derive(Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpenAgentSessionInput {
    Create { id: String, engine: Engine, options: SessionOptions, task_id: String, project_revision: i64, provider: dgos_agents::capabilities::CliProviderRef },
    Resume { session_id: String, trust_workspace: bool },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendAgentMessageInput { session_id: String, generation: String, request_id: String, text: String, settings: Option<dgos_agents::session::AgentTurnSettings>, #[serde(default)] references: Vec<dgos_agents::references::ConversationReference>, #[serde(default)] skills: Vec<dgos_agents::session::AgentSkillInput> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespondAgentRequestInput { session_id: String, generation: String, request_id: String, response: Value }

#[tauri::command]
pub async fn list_agent_sessions(app: tauri::AppHandle, window: WebviewWindow) -> Result<Vec<SessionRecord>, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    Ok(app.state::<AgentManager>().list(context.user_id()).await)
}
#[tauri::command]
pub async fn open_agent_session(app: tauri::AppHandle, window: WebviewWindow, input: OpenAgentSessionInput) -> Result<OwnedSessionView, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    let manager = app.state::<AgentManager>();
    match input {
        OpenAgentSessionInput::Create { id, engine, mut options, task_id, project_revision, provider } => {
            let preferences = app.state::<dgos_db::CliRepository>().get(context.user_id()).await?;
            options.executable = match engine { Engine::Codex => preferences.paths.codex, Engine::Claude => preferences.paths.claude };
            manager.open_session(context.user_id(), engine, id, dgos_agents::capabilities::SessionCreate { options, project_revision, provider }, Some(task_id), lease).await
        }
        OpenAgentSessionInput::Resume { session_id, trust_workspace } => manager.resume_session(context.user_id(), &session_id, trust_workspace, lease).await,
    }
}
#[tauri::command]
pub async fn get_agent_session(app: tauri::AppHandle, window: WebviewWindow, session_id: String, cursor: Option<dgos_agents::session::SessionCursor>) -> Result<OwnedSessionView, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    app.state::<AgentManager>().snapshot_since(context.user_id(), &session_id, cursor).await
}
#[tauri::command]
pub async fn send_agent_message(app: tauri::AppHandle, window: WebviewWindow, input: SendAgentMessageInput) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    app.state::<AgentManager>().send(context.user_id(), &input.session_id, &input.generation, input.request_id, input.text, input.settings, input.references, input.skills).await
}
#[tauri::command]
pub async fn respond_agent_request(app: tauri::AppHandle, window: WebviewWindow, input: RespondAgentRequestInput) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    app.state::<AgentManager>().respond(context.user_id(), &input.session_id, &input.generation, input.request_id, input.response).await
}
#[tauri::command]
pub async fn interrupt_agent_session(app: tauri::AppHandle, window: WebviewWindow, session_id: String) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    app.state::<AgentManager>().interrupt(context.user_id(), &session_id).await
}
#[tauri::command]
pub async fn close_agent_session(app: tauri::AppHandle, window: WebviewWindow, session_id: String) -> Result<bool, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    app.state::<AgentManager>().close_session(context.user_id(), &session_id).await
}

#[tauri::command]
pub async fn read_development_workspace(app: tauri::AppHandle, window: WebviewWindow) -> Result<DevelopmentWorkspace, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    app.state::<AgentManager>().development_workspace(context.user_id()).await
}
#[tauri::command]
pub async fn create_development_project(app: tauri::AppHandle, window: WebviewWindow, input: CreateProjectInput) -> Result<Project, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    app.state::<AgentManager>().create_project(context.user_id(), input, lease).await
}
#[tauri::command]
pub async fn update_development_project(app: tauri::AppHandle, window: WebviewWindow, input: UpdateProjectInput) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    app.state::<AgentManager>().update_project(context.user_id(), input, lease).await
}
#[tauri::command]
pub async fn inspect_development_project(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<ProjectInspection, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    app.state::<AgentManager>().inspect_project(context.user_id(), &project_id).await
}
#[tauri::command]
pub async fn save_development_task(app: tauri::AppHandle, window: WebviewWindow, input: SaveTaskInput) -> Result<DevelopmentTask, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    app.state::<AgentManager>().save_development_task(context.user_id(), input, lease).await
}
#[tauri::command]
pub async fn select_development_context(app: tauri::AppHandle, window: WebviewWindow, input: Selection) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    app.state::<AgentManager>().select_development_context(context.user_id(), input, lease).await
}
#[tauri::command]
pub async fn classify_agent_session(app: tauri::AppHandle, window: WebviewWindow, input: ClassifySessionInput) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    app.state::<AgentManager>().classify_session(context.user_id(), input, lease).await
}
