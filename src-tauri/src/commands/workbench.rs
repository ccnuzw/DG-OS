use super::{require_local_main_window, McpState, SkillsState, LOCAL_MCP_USER};
use dgos_providers::ProviderManager;
use dgos_workbench::{
    TaskInput, TaskOutcome, TaskRecord, ToolAction, ToolTaskInput, ToolTaskUpdate, WorkbenchManager,
};
use tauri::{State, WebviewWindow};

#[tauri::command]
pub async fn run_workbench_task(
    window: WebviewWindow,
    input: TaskInput,
    user_id: String,
    workbench: State<'_, WorkbenchManager>,
    providers: State<'_, ProviderManager>,
    skills: State<'_, SkillsState>,
) -> Result<TaskOutcome, String> {
    require_local_main_window(&window)?;
    if user_id != LOCAL_MCP_USER {
        return Err("Local desktop identity required".into());
    }
    let manager = skills.manager.read().await;
    workbench.run(&manager, &providers, input, &user_id).await
}

#[tauri::command]
pub async fn list_workbench_tasks(
    window: WebviewWindow,
    user_id: String,
    workbench: State<'_, WorkbenchManager>,
) -> Result<Vec<TaskRecord>, String> {
    require_local_main_window(&window)?;
    if user_id != LOCAL_MCP_USER {
        return Err("Local desktop identity required".into());
    }
    workbench.list(&user_id).await
}

#[tauri::command]
pub async fn start_workbench_tool_task(
    window: WebviewWindow,
    input: ToolTaskInput,
    workbench: State<'_, WorkbenchManager>,
    providers: State<'_, ProviderManager>,
    skills: State<'_, SkillsState>,
    mcp: State<'_, McpState>,
) -> Result<ToolTaskUpdate, String> {
    require_local_main_window(&window)?;
    let skills = skills.manager.read().await;
    workbench
        .start_tool_task(&skills, &providers, &mcp.manager, input, LOCAL_MCP_USER)
        .await
}

#[tauri::command]
pub async fn advance_workbench_tool_task(
    window: WebviewWindow,
    task_id: String,
    action: ToolAction,
    workbench: State<'_, WorkbenchManager>,
    providers: State<'_, ProviderManager>,
    mcp: State<'_, McpState>,
) -> Result<ToolTaskUpdate, String> {
    require_local_main_window(&window)?;
    workbench
        .advance_tool_task(&providers, &mcp.manager, &task_id, LOCAL_MCP_USER, action)
        .await
}

#[tauri::command]
pub async fn cancel_workbench_tool_task(
    window: WebviewWindow,
    task_id: String,
    workbench: State<'_, WorkbenchManager>,
) -> Result<ToolTaskUpdate, String> {
    require_local_main_window(&window)?;
    workbench.cancel_tool_task(&task_id, LOCAL_MCP_USER).await
}
