use super::{McpState, SkillsState};
use crate::host::{self, Capability};
use dgos_providers::ProviderManager;
use dgos_types::BuiltinCaller;
use dgos_workbench::{
    TaskInput, TaskOutcome, TaskRecord, ToolAction, ToolTaskInput, ToolTaskUpdate, WorkbenchManager,
};
use tauri::{State, WebviewWindow};

#[tauri::command]
pub async fn run_workbench_task(
    window: WebviewWindow,
    input: TaskInput,
    workbench: State<'_, WorkbenchManager>,
    providers: State<'_, ProviderManager>,
    skills: State<'_, SkillsState>,
) -> Result<TaskOutcome, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun)
        .map_err(|e| e.to_string())?;
    let manager = skills.manager.read().await;
    workbench
        .run(&manager, &providers, input, context.user_id())
        .await
}

#[tauri::command]
pub async fn list_workbench_tasks(
    window: WebviewWindow,
    workbench: State<'_, WorkbenchManager>,
) -> Result<Vec<TaskRecord>, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun)
        .map_err(|e| e.to_string())?;
    workbench.list(context.user_id()).await
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
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun)
        .map_err(|e| e.to_string())?;
    let skills = skills.manager.read().await;
    workbench
        .start_tool_task(&skills, &providers, &mcp.manager, input, context.user_id())
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
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun)
        .map_err(|e| e.to_string())?;
    workbench
        .advance_tool_task(
            &providers,
            &mcp.manager,
            &task_id,
            context.user_id(),
            action,
        )
        .await
}

#[tauri::command]
pub async fn cancel_workbench_tool_task(
    window: WebviewWindow,
    task_id: String,
    workbench: State<'_, WorkbenchManager>,
) -> Result<ToolTaskUpdate, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun)
        .map_err(|e| e.to_string())?;
    workbench
        .cancel_tool_task(&task_id, context.user_id())
        .await
}

#[tauri::command]
pub async fn cancel_workbench_task(
    window: WebviewWindow,
    task_id: String,
    workbench: State<'_, WorkbenchManager>,
) -> Result<bool, String> {
    let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun)
        .map_err(|e| e.to_string())?;
    workbench
        .cancel_text_task(&task_id, context.user_id())
        .await
}
