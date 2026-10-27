use super::SkillsState;
use dgos_providers::ProviderManager;
use dgos_workbench::{TaskInput, TaskOutcome, TaskRecord, WorkbenchManager};
use tauri::State;

#[tauri::command]
pub async fn run_workbench_task(
    input: TaskInput,
    user_id: String,
    workbench: State<'_, WorkbenchManager>,
    providers: State<'_, ProviderManager>,
    skills: State<'_, SkillsState>,
) -> Result<TaskOutcome, String> {
    let manager = skills.manager.read().await;
    workbench.run(&manager, &providers, input, &user_id).await
}

#[tauri::command]
pub async fn list_workbench_tasks(
    user_id: String,
    workbench: State<'_, WorkbenchManager>,
) -> Result<Vec<TaskRecord>, String> {
    workbench.list(&user_id).await
}
