use dgos_skills::{DependencyStatus, RegistrySnapshot, Skill, SkillDetail, SkillInfo, SkillUpdate, SkillUsageLog, SkillsManager};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

pub struct SkillsState {
    pub manager: Arc<RwLock<SkillsManager>>,
}

// Skills Registry
#[tauri::command]
pub async fn fetch_skills_registry(
    state: State<'_, SkillsState>,
) -> Result<RegistrySnapshot, String> {
    let manager = state.manager.read().await;
    manager
        .fetch_registry()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_skills(
    query: String,
    state: State<'_, SkillsState>,
) -> Result<Vec<SkillInfo>, String> {
    let manager = state.manager.read().await;
    manager
        .search_skills(&query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_skill_detail(
    skill_id: String,
    state: State<'_, SkillsState>,
) -> Result<SkillDetail, String> {
    let manager = state.manager.read().await;
    manager
        .get_skill_detail(&skill_id)
        .await
        .map_err(|e| e.to_string())
}

// Skills Installation Management
#[tauri::command]
pub async fn install_skill(
    skill_id: String,
    user_id: String,
    state: State<'_, SkillsState>,
) -> Result<Skill, String> {
    let manager = state.manager.read().await;
    manager
        .install_skill(&skill_id, &user_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn uninstall_skill(
    skill_id: String,
    user_id: String,
    state: State<'_, SkillsState>,
) -> Result<(), String> {
    let manager = state.manager.read().await;
    manager
        .uninstall_skill(&skill_id, &user_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_installed_skills(
    user_id: String,
    state: State<'_, SkillsState>,
) -> Result<Vec<Skill>, String> {
    let manager = state.manager.read().await;
    manager
        .list_installed_skills(&user_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_skill(
    skill_id: String,
    user_id: String,
    enabled: bool,
    state: State<'_, SkillsState>,
) -> Result<(), String> {
    let manager = state.manager.read().await;
    manager
        .toggle_skill(&skill_id, &user_id, enabled)
        .await
        .map_err(|e| e.to_string())
}

// Skills Configuration
#[tauri::command]
pub async fn update_skill_config(
    skill_id: String,
    user_id: String,
    config: JsonValue,
    state: State<'_, SkillsState>,
) -> Result<(), String> {
    let manager = state.manager.read().await;
    manager
        .update_skill_config(&skill_id, &user_id, config)
        .await
        .map_err(|e| e.to_string())
}

// Skills Calling
#[tauri::command]
pub async fn call_skill(
    skill_id: String,
    user_id: String,
    params: JsonValue,
    state: State<'_, SkillsState>,
) -> Result<JsonValue, String> {
    let manager = state.manager.read().await;
    let skill = manager
        .list_installed_skills(&user_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.skill_id == skill_id)
        .ok_or_else(|| "Skill not found".to_string())?;

    match skill.call_type {
        dgos_skills::CallType::Http => {
            manager
                .call_http_skill(&skill_id, &user_id, params)
                .await
                .map_err(|e| e.to_string())
        }
        dgos_skills::CallType::Cli => {
            manager
                .call_cli_skill(&skill_id, &user_id, params)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

// Skills Updates
#[tauri::command]
pub async fn check_skill_updates(
    user_id: String,
    state: State<'_, SkillsState>,
) -> Result<Vec<SkillUpdate>, String> {
    let manager = state.manager.read().await;
    manager
        .check_skill_updates(&user_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_skill(
    skill_id: String,
    user_id: String,
    state: State<'_, SkillsState>,
) -> Result<Skill, String> {
    let manager = state.manager.read().await;
    manager
        .update_skill(&skill_id, &user_id)
        .await
        .map_err(|e| e.to_string())
}

// Dependency Check
#[tauri::command]
pub async fn check_dependencies(
    skill_id: String,
    state: State<'_, SkillsState>,
) -> Result<DependencyStatus, String> {
    let manager = state.manager.read().await;
    let detail = manager
        .get_skill_detail(&skill_id)
        .await
        .map_err(|e| e.to_string())?;

    dgos_skills::DependencyChecker::check_dependencies(&detail.manifest.dependencies)
        .map_err(|e| e.to_string())
}

// Usage Logs
#[tauri::command]
pub async fn list_skill_usage_logs(
    user_id: String,
    limit: Option<usize>,
    state: State<'_, SkillsState>,
) -> Result<Vec<SkillUsageLog>, String> {
    let manager = state.manager.read().await;
    manager
        .list_usage_logs(&user_id, limit)
        .await
        .map_err(|e| e.to_string())
}
