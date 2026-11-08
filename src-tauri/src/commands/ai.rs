//! Runtime-bound AI commands. Identity and both leases are always host-derived.
use super::CommandResponse;
use crate::host;
use dgos_application::{AiInvocation, AiService};
use dgos_modules::{CallRejection, ModuleCallRouter, ModuleRequestOutcome};
use dgos_types::*;
use std::sync::Arc;
use tauri::{ipc::Channel, Manager, WebviewWindow};

fn invocation(app: &tauri::AppHandle, window: &WebviewWindow, attachment_id: &str, route: &str, call_id: &str) -> DgosResult<AiInvocation> {
    host::module_host_window(window)?;
    let call = app.state::<Arc<ModuleCallRouter>>().admit_once(route, attachment_id, call_id)
        .map_err(|e| if matches!(e, CallRejection::Busy) { DgosError::RateLimitExceeded } else { DgosError::PermissionDenied })?;
    let (session, account, attachment) = match host::module_call_leases(window, &call) {
        Ok(leases) => leases,
        Err(error) => { call.finish(ModuleRequestOutcome::Denied); return Err(error); }
    };
    AiInvocation::bind(call, &session, account, attachment)
}

#[tauri::command]
pub async fn module_run_ai(app: tauri::AppHandle, window: WebviewWindow, attachment_id: String, route: String, call_id: String, input: AiRunInput, on_event: Channel<AiEvent>) -> CommandResponse<AiTerminal> {
    let invocation = match invocation(&app, &window, &attachment_id, &route, &call_id) { Ok(v) => v, Err(e) => return CommandResponse::from(Err(e)) };
    let service = app.state::<AiService>().inner().clone();
    // The runtime owns the driver even when the IPC receiver goes away. Failed
    // channel sends cancel the run; the same future still completes settlement.
    let driver = tauri::async_runtime::spawn(async move {
        service.run(invocation, input, Arc::new(move |event| on_event.send(event).is_ok())).await
    });
    CommandResponse::from(driver.await.unwrap_or(Err(DgosError::StreamInterrupted)))
}

#[tauri::command]
pub async fn module_list_ai_providers(app: tauri::AppHandle, window: WebviewWindow, attachment_id: String, route: String, call_id: String) -> CommandResponse<Vec<AiProviderEntry>> {
    let invocation = match invocation(&app, &window, &attachment_id, &route, &call_id) { Ok(v) => v, Err(e) => return CommandResponse::from(Err(e)) };
    CommandResponse::from(app.state::<AiService>().list_providers(invocation).await)
}

#[tauri::command]
pub async fn module_list_ai_skills(app: tauri::AppHandle, window: WebviewWindow, attachment_id: String, route: String, call_id: String) -> CommandResponse<Vec<AiSkillEntry>> {
    let invocation = match invocation(&app, &window, &attachment_id, &route, &call_id) { Ok(v) => v, Err(e) => return CommandResponse::from(Err(e)) };
    CommandResponse::from(app.state::<AiService>().list_skills(invocation).await)
}

#[tauri::command]
pub async fn module_list_ai_tools(app: tauri::AppHandle, window: WebviewWindow, attachment_id: String, route: String, call_id: String) -> CommandResponse<Vec<AiToolEntry>> {
    let invocation = match invocation(&app, &window, &attachment_id, &route, &call_id) { Ok(v) => v, Err(e) => return CommandResponse::from(Err(e)) };
    CommandResponse::from(app.state::<AiService>().list_tools(invocation).await)
}

#[tauri::command]
pub async fn module_list_ai_runs(app: tauri::AppHandle, window: WebviewWindow, attachment_id: String, route: String, call_id: String, input: AiRunListInput) -> CommandResponse<AiRunPage> {
    let invocation = match invocation(&app, &window, &attachment_id, &route, &call_id) { Ok(v) => v, Err(e) => return CommandResponse::from(Err(e)) };
    CommandResponse::from(app.state::<AiService>().list_runs(invocation, input).await)
}

#[tauri::command]
pub async fn module_get_ai_run(app: tauri::AppHandle, window: WebviewWindow, attachment_id: String, route: String, call_id: String, run_id: String) -> CommandResponse<Option<AiRunSummary>> {
    let invocation = match invocation(&app, &window, &attachment_id, &route, &call_id) { Ok(v) => v, Err(e) => return CommandResponse::from(Err(e)) };
    CommandResponse::from(app.state::<AiService>().get_run(invocation, &run_id).await)
}

#[tauri::command]
pub async fn module_cancel_ai_run(app: tauri::AppHandle, window: WebviewWindow, attachment_id: String, route: String, call_id: String, run_id: String) -> CommandResponse<bool> {
    let invocation = match invocation(&app, &window, &attachment_id, &route, &call_id) { Ok(v) => v, Err(e) => return CommandResponse::from(Err(e)) };
    CommandResponse::from(app.state::<AiService>().cancel(invocation, &run_id).await)
}
