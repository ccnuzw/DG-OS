use crate::{host::{self, Capability}, native_extensions::{dto::*, Domain, NativeHost, OperationAction, Ticket}, session::SessionResolver};
use dgos_agents::{native_extensions as core, AgentManager};
use dgos_types::BuiltinCaller;
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewWindow};

fn authorize(app: &AppHandle, window: &WebviewWindow, domain: Domain, write: bool) -> Result<(dgos_db::ActivityPermit, Ticket), String> {
    let capability = match (domain, write) { (Domain::Skills, false) => Capability::SkillsRead, (Domain::Skills, true) => Capability::SkillsManage, (Domain::Mcp, false) => Capability::McpRead, (Domain::Mcp, true) => Capability::McpManage };
    let admission = host::context(window, BuiltinCaller::Settings, capability).map_err(|_| "native_extension_AccessDenied".to_owned())?;
    let (owner, generation, _) = app.try_state::<Arc<SessionResolver>>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.native_session()?;
    if owner != admission.user_id() { return Err("session_invalid".into()); }
    Ok((host::admit_activity(app).map_err(|_| "native_extension_Busy".to_owned())?, (owner, generation)))
}
#[tauri::command]
pub async fn discover_native_skills(app: AppHandle, window: WebviewWindow, input: DiscoverSkills) -> Result<core::ResourceDiscovery, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.discover_skills(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn get_native_skill(app: AppHandle, window: WebviewWindow, input: GetSkill) -> Result<core::ResourceSummary, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.get_skill(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn preview_native_skill(app: AppHandle, window: WebviewWindow, input: PreviewSkill) -> Result<SkillPreview, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.preview_skill(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn preview_native_skill_visibility(app: AppHandle, window: WebviewWindow, input: Visibility) -> Result<VisibilityPreview, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.preview_visibility(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn discover_native_mcp(app: AppHandle, window: WebviewWindow, input: Selection) -> Result<core::mcp::Discovery, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.discover_mcp(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn preview_native_mcp(app: AppHandle, window: WebviewWindow, input: PreviewMcp) -> Result<core::Preview, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Mcp, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.preview_mcp(&app.state::<AgentManager>(), input, permit, ticket).await
}
async fn operation(app: AppHandle, window: WebviewWindow, input: Operation, action: OperationAction) -> Result<Receipt, String> {
    // First establish trusted main and a current account before reading host index metadata.
    let (initial, ticket) = authorize(&app, &window, Domain::Skills, false)?;
    let native = app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?;
    let domain = native.operation_domain(&input.operation_id, Arc::new(initial), &ticket).await?;
    // Inspection may finalize journal state, so all file operations need management authority.
    let (permit, current) = authorize(&app, &window, domain, true)?;
    if current != ticket { return Err("session_invalid".into()); }
    native.operation(&app.state::<AgentManager>(), input, action, domain, permit, ticket).await
}
#[tauri::command]
pub async fn commit_native_operation(app: AppHandle, window: WebviewWindow, input: Operation) -> Result<Receipt, String> { operation(app, window, input, OperationAction::Commit).await }
#[tauri::command]
pub async fn cancel_native_operation(app: AppHandle, window: WebviewWindow, input: Operation) -> Result<Receipt, String> { operation(app, window, input, OperationAction::Cancel).await }
#[tauri::command]
pub async fn inspect_native_operation(app: AppHandle, window: WebviewWindow, input: Operation) -> Result<Receipt, String> { operation(app, window, input, OperationAction::Inspect).await }

fn session_lease(app: &AppHandle, ticket: &Ticket) -> Result<tokio_util::sync::CancellationToken, String> {
    let (owner, generation, lease) = app.try_state::<Arc<SessionResolver>>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.native_session()?;
    if owner != ticket.0 || generation != ticket.1 { return Err("session_invalid".into()); }
    Ok(lease)
}
fn session_domain(domain: dgos_agents::native_session::SessionDomain) -> Domain {
    match domain { dgos_agents::native_session::SessionDomain::Skills => Domain::Skills, dgos_agents::native_session::SessionDomain::Mcp => Domain::Mcp }
}
#[tauri::command]
pub async fn list_native_sessions(app: AppHandle, window: WebviewWindow, input: ListNativeSessions) -> Result<Vec<dgos_agents::native_session::NativeSessionHandle>, String> {
    let (_permit, ticket) = authorize(&app, &window, session_domain(input.domain), false)?;
    let lease = session_lease(&app, &ticket)?;
    let result = app.state::<AgentManager>().native_session_handles(&ticket.0, &input.project_id, &lease).await?;
    session_lease(&app, &ticket)?;
    Ok(result)
}
#[tauri::command]
pub async fn observe_native_session(app: AppHandle, window: WebviewWindow, input: ObserveNativeSession) -> Result<dgos_agents::native_session::NativeSessionObservation, String> {
    let (_permit, ticket) = authorize(&app, &window, session_domain(input.domain), false)?;
    let lease = session_lease(&app, &ticket)?;
    let result = app.state::<AgentManager>().observe_native_session(&ticket.0, &input.session_id, &input.generation, input.domain, &lease).await?;
    session_lease(&app, &ticket)?;
    Ok(result)
}
#[tauri::command]
pub async fn native_session_action(app: AppHandle, window: WebviewWindow, input: NativeSessionControl) -> Result<dgos_agents::native_session::NativeSessionReceipt, String> {
    let (_permit, ticket) = authorize(&app, &window, session_domain(input.action.domain()), true)?;
    let lease = session_lease(&app, &ticket)?;
    let result = app.state::<AgentManager>().native_session_action(&ticket.0, &input.session_id, &input.generation, &input.request_id, input.action, &lease).await?;
    session_lease(&app, &ticket)?;
    Ok(result)
}
#[tauri::command]
pub async fn inspect_native_session_request(app: AppHandle, window: WebviewWindow, input: InspectNativeSessionRequest) -> Result<Option<dgos_agents::native_session::NativeSessionReceipt>, String> {
    let (_permit, ticket) = authorize(&app, &window, session_domain(input.domain), false)?;
    let lease = session_lease(&app, &ticket)?;
    let result = app.state::<AgentManager>().inspect_native_session_request(&ticket.0, &input.session_id, &input.generation, &input.request_id, input.domain, &lease).await?;
    session_lease(&app, &ticket)?;
    Ok(result)
}
