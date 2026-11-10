use crate::{native_extensions::{dto::Selection, skill_library::*, Domain, NativeHost}, commands::native_extensions::authorize};
use dgos_agents::{native_extensions as core, AgentManager};
use tauri::{AppHandle, Manager, WebviewWindow};

#[tauri::command]
pub async fn list_native_skill_sources(app: AppHandle, window: WebviewWindow, input: Selection) -> Result<Vec<SkillSource>, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.list_skill_sources(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn save_native_skill_source(app: AppHandle, window: WebviewWindow, input: SaveSource) -> Result<SkillSource, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.save_skill_source(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn remove_native_skill_source(app: AppHandle, window: WebviewWindow, input: SourceRequest) -> Result<(), String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.remove_skill_source(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn refresh_native_skill_source(app: AppHandle, window: WebviewWindow, input: SourceRequest) -> Result<SkillSource, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.refresh_skill_source(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn search_native_skill_library(app: AppHandle, window: WebviewWindow, input: SearchLibrary) -> Result<LibrarySearch, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.search_skill_library(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn pick_native_skill_source(app: AppHandle, window: WebviewWindow, input: PickSource) -> Result<Option<PickedSource>, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.pick_skill_source(&app, &app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn inspect_native_skill(app: AppHandle, window: WebviewWindow, input: SkillRequest) -> Result<SkillInspection, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.inspect_skill(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn read_native_skill_file(app: AppHandle, window: WebviewWindow, input: ReadFile) -> Result<core::ResourceContent, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.read_skill_file(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn inspect_native_skill_dependencies(app: AppHandle, window: WebviewWindow, input: SkillRequest) -> Result<core::dependencies::Report, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, false)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.inspect_skill_dependencies(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn preview_native_skill_version(app: AppHandle, window: WebviewWindow, input: PreviewVersion) -> Result<VersionPlan, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.preview_skill_version(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn run_native_skill_version(app: AppHandle, window: WebviewWindow, input: RunVersion) -> Result<core::dependencies::VersionResult, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.run_skill_version(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn prepare_native_skill_update(app: AppHandle, window: WebviewWindow, input: SkillRequest) -> Result<PreparedUpdate, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.prepare_skill_update(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn inspect_native_skill_version(app: AppHandle, window: WebviewWindow, input: InspectVersion) -> Result<core::dependencies::VersionResult, String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.inspect_skill_version(&app.state::<AgentManager>(), input, permit, ticket).await
}
#[tauri::command]
pub async fn release_native_skill_source(app: AppHandle, window: WebviewWindow, input: ReleaseSource) -> Result<(), String> {
    let (permit, ticket) = authorize(&app, &window, Domain::Skills, true)?;
    app.try_state::<NativeHost>().ok_or_else(|| "native_extension_Unavailable".to_owned())?.release_skill_source(input, permit, ticket)
}
