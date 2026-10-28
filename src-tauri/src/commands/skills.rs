use crate::host::{self, Capability};
use dgos_skills::{
    DependencyStatus, RegistrySnapshot, Skill, SkillDetail, SkillInfo, SkillUpdate, SkillUsageLog,
    SkillsManager,
};
use dgos_types::BuiltinCaller;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use tauri::{State, WebviewWindow};
use tokio::sync::RwLock;

pub struct SkillsState {
    pub manager: Arc<RwLock<SkillsManager>>,
}

// Skills Registry
#[tauri::command]
pub async fn fetch_skills_registry(
    window: WebviewWindow,
    state: State<'_, SkillsState>,
) -> Result<RegistrySnapshot, String> {
    let _context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsRead)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager.fetch_registry().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_skills(
    window: WebviewWindow,
    query: String,
    state: State<'_, SkillsState>,
) -> Result<Vec<SkillInfo>, String> {
    let _context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsRead)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .search_skills(&query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_skill_detail(
    window: WebviewWindow,
    skill_id: String,
    state: State<'_, SkillsState>,
) -> Result<SkillDetail, String> {
    let _context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsRead)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .get_skill_detail(&skill_id)
        .await
        .map_err(|e| e.to_string())
}

// Skills Installation Management
#[tauri::command]
pub async fn install_skill(
    window: WebviewWindow,
    skill_id: String,
    state: State<'_, SkillsState>,
) -> Result<Skill, String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsManage)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .install_skill(&skill_id, context.user_id())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn uninstall_skill(
    window: WebviewWindow,
    skill_id: String,
    state: State<'_, SkillsState>,
) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsManage)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .uninstall_skill(&skill_id, context.user_id())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_installed_skills(
    window: WebviewWindow,
    state: State<'_, SkillsState>,
) -> Result<Vec<Skill>, String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsRead)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .list_installed_skills(context.user_id())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_skill(
    window: WebviewWindow,
    skill_id: String,
    enabled: bool,
    state: State<'_, SkillsState>,
) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsManage)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .toggle_skill(&skill_id, context.user_id(), enabled)
        .await
        .map_err(|e| e.to_string())
}

// Skills Configuration
#[tauri::command]
pub async fn update_skill_config(
    window: WebviewWindow,
    skill_id: String,
    config: JsonValue,
    state: State<'_, SkillsState>,
) -> Result<(), String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsManage)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .update_skill_config(&skill_id, context.user_id(), config)
        .await
        .map_err(|e| e.to_string())
}

// Skills Calling
#[tauri::command]
pub async fn call_skill(
    window: WebviewWindow,
    skill_id: String,
    params: JsonValue,
    state: State<'_, SkillsState>,
) -> Result<JsonValue, String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsManage)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    let skill = manager
        .list_installed_skills(context.user_id())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.skill_id == skill_id)
        .ok_or_else(|| "Skill not found".to_string())?;

    match skill.call_type {
        dgos_skills::CallType::Http => manager
            .call_http_skill(&skill_id, context.user_id(), params)
            .await
            .map_err(|e| e.to_string()),
        dgos_skills::CallType::Cli => manager
            .call_cli_skill(&skill_id, context.user_id(), params)
            .await
            .map_err(|e| e.to_string()),
        dgos_skills::CallType::Content => {
            Err("SKILL.md content is prepared as task context, not executed".to_string())
        }
    }
}

#[tauri::command]
pub async fn prepare_content_skill(
    window: WebviewWindow,
    skill_id: String,
    state: State<'_, SkillsState>,
) -> Result<dgos_skills::PreparedContent, String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsPrepare)
        .map_err(|e| e.to_string())?;
    let provenance = context.provenance();
    let manager = state.manager.read().await;
    manager
        .prepare_content_skill_with_provenance(&skill_id, context.user_id(), Some(&provenance))
        .await
        .map_err(|e| e.to_string())
}

// Skills Updates
#[tauri::command]
pub async fn check_skill_updates(
    window: WebviewWindow,
    state: State<'_, SkillsState>,
) -> Result<Vec<SkillUpdate>, String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsManage)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .check_skill_updates(context.user_id())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_skill(
    window: WebviewWindow,
    skill_id: String,
    state: State<'_, SkillsState>,
) -> Result<Skill, String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsManage)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .update_skill(&skill_id, context.user_id())
        .await
        .map_err(|e| e.to_string())
}

// Dependency Check
#[tauri::command]
pub async fn check_dependencies(
    window: WebviewWindow,
    skill_id: String,
    state: State<'_, SkillsState>,
) -> Result<DependencyStatus, String> {
    let _context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsRead)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    let detail = manager
        .get_skill_detail(&skill_id)
        .await
        .map_err(|e| e.to_string())?;

    if detail.format != "skill_md" {
        return Err("Legacy format dependencies are unavailable until migration".to_string());
    }
    let missing = detail
        .content
        .map(|content| content.required_tools)
        .unwrap_or_default();
    Ok(DependencyStatus {
        satisfied: missing.is_empty(),
        missing,
    })
}

// Usage Logs
#[tauri::command]
pub async fn list_skill_usage_logs(
    window: WebviewWindow,
    limit: Option<usize>,
    state: State<'_, SkillsState>,
) -> Result<Vec<SkillUsageLog>, String> {
    let context = host::context(&window, BuiltinCaller::Settings, Capability::SkillsRead)
        .map_err(|e| e.to_string())?;
    let manager = state.manager.read().await;
    manager
        .list_usage_logs(context.user_id(), limit)
        .await
        .map_err(|e| e.to_string())
}
