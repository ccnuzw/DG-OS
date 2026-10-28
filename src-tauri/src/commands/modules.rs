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
use dgos_application::ModuleOperationManager;
use dgos_types::{
    DgosError, ModuleOperationAccepted, ModuleOperationErrorKind, ModuleOperationView,
    ModuleRuntimeStatusView,
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
