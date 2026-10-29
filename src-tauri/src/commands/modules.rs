//! Module lifecycle commands (D034).
//!
//! Commands only submit and query host operations. They accept a `module_id`
//! selector and an `operation_id` for status; they never accept a user id,
//! runtime handle, execution context, version, digest, filesystem path or
//! URL. Identity comes from `host::module_session`, update sources from the
//! host-owned local source root, and failures leave only as
//! `ModuleOperationErrorKind` codes with fixed messages.

use super::CommandResponse;
use crate::host::{self, Capability};
use dgos_application::{ModuleCrashService, ModuleManagementService, ModuleOperationManager};
use dgos_types::{
    DgosError, ModuleCrashView, ModuleOperationAccepted, ModuleOperationErrorKind,
    ModuleOperationView, ModuleRuntimeStatusView, ModuleSummaryView,
};
use tauri::{Manager, WebviewWindow};

fn into_response<T: serde::Serialize>(
    result: Result<T, ModuleOperationErrorKind>,
) -> CommandResponse<T> {
    CommandResponse::from(result.map_err(DgosError::ModuleOperation))
}

#[tauri::command]
pub async fn start_module(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
) -> CommandResponse<ModuleOperationAccepted> {
    let session = match host::module_session(&window, Capability::ModulesManage) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ModuleOperationManager>();
    into_response(manager.start(&session, &module_id))
}

#[tauri::command]
pub async fn stop_module(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
) -> CommandResponse<ModuleOperationAccepted> {
    let session = match host::module_session(&window, Capability::ModulesManage) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ModuleOperationManager>();
    into_response(manager.stop(&session, &module_id))
}

/// Updates from the host-owned local source directory of `module_id`.
#[tauri::command]
pub async fn update_module(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
) -> CommandResponse<ModuleOperationAccepted> {
    let session = match host::module_session(&window, Capability::ModulesManage) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ModuleOperationManager>();
    into_response(manager.update(&session, &module_id))
}

#[tauri::command]
pub async fn get_module_operation(
    app: tauri::AppHandle,
    window: WebviewWindow,
    operation_id: String,
) -> CommandResponse<ModuleOperationView> {
    if let Err(error) = host::module_session(&window, Capability::ModulesRead) {
        return CommandResponse::from(Err(error));
    }
    let manager = app.state::<ModuleOperationManager>();
    into_response(manager.operation(&operation_id))
}

#[tauri::command]
pub async fn get_module_runtime_status(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
) -> CommandResponse<ModuleRuntimeStatusView> {
    if let Err(error) = host::module_session(&window, Capability::ModulesRead) {
        return CommandResponse::from(Err(error));
    }
    let manager = app.state::<ModuleOperationManager>();
    into_response(manager.runtime_status(&module_id))
}

#[tauri::command]
pub async fn enable_module(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
) -> CommandResponse<ModuleOperationAccepted> {
    let session = match host::module_session(&window, Capability::ModulesManage) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ModuleOperationManager>();
    into_response(manager.enable(&session, &module_id))
}

/// Stops the runtime, then disables the module.
#[tauri::command]
pub async fn disable_module(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
) -> CommandResponse<ModuleOperationAccepted> {
    let session = match host::module_session(&window, Capability::ModulesManage) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ModuleOperationManager>();
    into_response(manager.disable(&session, &module_id))
}

/// Stops the runtime, then removes the module. Refused for bundled modules.
#[tauri::command]
pub async fn uninstall_module(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
) -> CommandResponse<ModuleOperationAccepted> {
    let session = match host::module_session(&window, Capability::ModulesManage) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let manager = app.state::<ModuleOperationManager>();
    into_response(manager.uninstall(&session, &module_id))
}

#[tauri::command]
pub async fn list_modules(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<Vec<ModuleSummaryView>> {
    let session = match host::module_session(&window, Capability::ModulesRead) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let service = app.state::<ModuleManagementService>();
    CommandResponse::from(service.list(&session).await)
}

/// Grants one permission the module's manifest declares. The permission is a
/// D006 string; the host re-reads the manifest and refuses anything else.
#[tauri::command]
pub async fn grant_module_permission(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
    permission: String,
) -> CommandResponse<ModuleSummaryView> {
    let session = match host::module_session(&window, Capability::ModulesManage) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let service = app.state::<ModuleManagementService>();
    CommandResponse::from(service.grant(&session, &module_id, &permission).await)
}

/// Revocation applies to the next call and never cancels one in flight.
#[tauri::command]
pub async fn revoke_module_permission(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
    permission: String,
) -> CommandResponse<ModuleSummaryView> {
    let session = match host::module_session(&window, Capability::ModulesManage) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let service = app.state::<ModuleManagementService>();
    CommandResponse::from(service.revoke(&session, &module_id, &permission).await)
}

#[tauri::command]
pub async fn list_module_crashes(
    app: tauri::AppHandle,
    window: WebviewWindow,
    module_id: String,
) -> CommandResponse<Vec<ModuleCrashView>> {
    let session = match host::module_session(&window, Capability::ModulesRead) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let service = app.state::<ModuleCrashService>();
    CommandResponse::from(service.list(&session, &module_id).await)
}
