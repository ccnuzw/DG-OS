//! Module capability call commands for the five non-Provider capabilities
//! (D093; same channel as `module_call_provider`, D050/D051/D054).
//!
//! Every command takes only the page's host-issued `attachment_id`, `route`
//! and per-call `call_id`, plus that capability's business fields. The
//! module, runtime, version, digest and user all come from the host-side
//! binding. The order is fixed: window gate and channel admission (layer 4,
//! `admit`) → per-call authorization of layers 1–3 inside
//! `ModuleCapabilityDispatchService` → the capability → settle once.
//! Admission failures answer `1004004` without a reason; a runtime at its
//! in-flight limit answers the retryable `1001005`. `contracts:check` holds
//! each command to an exact parameter allow-list.

use super::{CommandResponse, SkillsState};
use crate::host;
use dgos_application::ModuleCapabilityDispatchService;
use dgos_db::ModulePermission;
use dgos_modules::{BoundModuleCall, CallRejection, ModuleCallRouter};
use dgos_types::DgosError;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Manager, WebviewWindow};

/// Host-owned deadline of `skills.execute` (D093).
pub const SKILL_PREPARE_DEADLINE: Duration = Duration::from_secs(10);

/// Window gate plus channel admission, shared by every command here. Not a
/// command itself.
fn admit(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    attachment_id: &str,
    route: &str,
    call_id: &str,
) -> Result<BoundModuleCall, DgosError> {
    host::module_host_window(window)?;
    let router = app.state::<Arc<ModuleCallRouter>>();
    router
        .admit_once(route, attachment_id, call_id)
        .map_err(|rejection| match rejection {
            CallRejection::Busy => DgosError::RateLimitExceeded,
            rejection => {
                tracing::debug!(?rejection, "module call was not admitted");
                DgosError::PermissionDenied
            }
        })
}

/// What a module receives from `skills.prepare`: the pinned content of one
/// installed, enabled skill. No local path, no execution (D016).
#[derive(serde::Serialize)]
pub struct ModuleSkillContent {
    pub skill_id: String,
    pub version: String,
    pub content_hash: String,
    pub body: String,
}

/// `skills.execute` (D093): prepares the pinned content of an installed and
/// enabled skill of the bound user, the same path the Workbench uses. Runs
/// no script or tool; the content grants nothing.
#[tauri::command]
pub async fn module_prepare_skill(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
    skill_id: String,
) -> CommandResponse<ModuleSkillContent> {
    let call = match admit(&app, &window, &attachment_id, &route, &call_id) {
        Ok(call) => call,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let dispatch = app.state::<ModuleCapabilityDispatchService>();
    let skills = app.state::<SkillsState>();
    let result = dispatch
        .run_bound(
            call,
            ModulePermission::SkillsExecute,
            SKILL_PREPARE_DEADLINE,
            |grant| async move {
                let provenance = grant.provenance();
                let manager = skills.manager.read().await;
                manager
                    .prepare_content_skill_with_provenance(
                        &skill_id,
                        &grant.user_id,
                        Some(&provenance),
                    )
                    .await
                    .map(|content| ModuleSkillContent {
                        skill_id: content.skill_id,
                        version: content.version,
                        content_hash: content.content_hash,
                        body: content.body,
                    })
                    .map_err(|error| DgosError::CapabilityFailed {
                        code: error.error_code().parse().unwrap_or(1002999),
                        message: "Skill content is unavailable".to_owned(),
                    })
            },
        )
        .await;
    CommandResponse::from(result)
}

/// `network.local` (D093): HTTP(S) GET/POST to the loopback interface only.
/// The host validates and resolves the target, pins the connection, follows
/// no redirect and sends no credential; the 30 s deadline is host-owned.
#[tauri::command]
pub async fn module_network_request(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
    request: dgos_application::ModuleNetworkRequestInput,
) -> CommandResponse<dgos_application::ModuleNetworkResponse> {
    let call = match admit(&app, &window, &attachment_id, &route, &call_id) {
        Ok(call) => call,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let dispatch = app.state::<ModuleCapabilityDispatchService>();
    let result = dispatch
        .run_bound(
            call,
            ModulePermission::NetworkLocal,
            dgos_application::NETWORK_LOCAL_DEADLINE,
            |_grant| dgos_application::execute_network_request(request),
        )
        .await;
    CommandResponse::from(result)
}

/// `storage.read` (D093): the stored JSON value of one key, `null` when the
/// key does not exist. The scope is the host-bound user and module.
#[tauri::command]
pub async fn module_storage_get(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
    key: String,
) -> CommandResponse<Option<serde_json::Value>> {
    let call = match admit(&app, &window, &attachment_id, &route, &call_id) {
        Ok(call) => call,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let dispatch = app.state::<ModuleCapabilityDispatchService>();
    let storage = app.state::<dgos_application::ModuleStorageService>();
    let result = dispatch
        .run_bound(
            call,
            ModulePermission::StorageRead,
            dgos_application::MODULE_STORAGE_DEADLINE,
            |grant| async move { storage.get(&grant, &key).await },
        )
        .await;
    CommandResponse::from(result)
}

/// `storage.write` (D093): insert or replace one key (key 1-128 characters,
/// value JSON <= 64 KiB, 5 MiB per module and user).
#[tauri::command]
pub async fn module_storage_set(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
    key: String,
    value: serde_json::Value,
) -> CommandResponse<()> {
    let call = match admit(&app, &window, &attachment_id, &route, &call_id) {
        Ok(call) => call,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let dispatch = app.state::<ModuleCapabilityDispatchService>();
    let storage = app.state::<dgos_application::ModuleStorageService>();
    let result = dispatch
        .run_bound(
            call,
            ModulePermission::StorageWrite,
            dgos_application::MODULE_STORAGE_DEADLINE,
            |grant| async move { storage.set(&grant, &key, &value).await },
        )
        .await;
    CommandResponse::from(result)
}

/// `storage.write` (D093): idempotent delete of one key.
#[tauri::command]
pub async fn module_storage_delete(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
    key: String,
) -> CommandResponse<()> {
    let call = match admit(&app, &window, &attachment_id, &route, &call_id) {
        Ok(call) => call,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let dispatch = app.state::<ModuleCapabilityDispatchService>();
    let storage = app.state::<dgos_application::ModuleStorageService>();
    let result = dispatch
        .run_bound(
            call,
            ModulePermission::StorageWrite,
            dgos_application::MODULE_STORAGE_DEADLINE,
            |grant| async move { storage.delete(&grant, &key).await },
        )
        .await;
    CommandResponse::from(result)
}

/// `storage.read` (D093): keys ascending with `size_bytes`, never values.
#[tauri::command]
pub async fn module_storage_list(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
) -> CommandResponse<Vec<dgos_db::ModuleStorageEntry>> {
    let call = match admit(&app, &window, &attachment_id, &route, &call_id) {
        Ok(call) => call,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let dispatch = app.state::<ModuleCapabilityDispatchService>();
    let storage = app.state::<dgos_application::ModuleStorageService>();
    let result = dispatch
        .run_bound(
            call,
            ModulePermission::StorageRead,
            dgos_application::MODULE_STORAGE_DEADLINE,
            |grant| async move { storage.list(&grant).await },
        )
        .await;
    CommandResponse::from(result)
}

/// `mcp.call` (D093): a module-initiated MCP tool call. Layers 1–3 are
/// checked first; then the user confirms the exact arguments in a host
/// dialog (never module UI) and the host itself mints and consumes the
/// one-time MCP token, so the module cannot obtain or supply one. The 120 s
/// deadline runs from admission and includes the wait for the user.
#[tauri::command]
pub async fn module_call_mcp_tool(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
    server_id: String,
    tool_name: String,
    arguments: serde_json::Value,
) -> CommandResponse<dgos_application::ModuleMcpResult> {
    let call = match admit(&app, &window, &attachment_id, &route, &call_id) {
        Ok(call) => call,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let admitted_at = std::time::Instant::now();
    let dispatch = app.state::<ModuleCapabilityDispatchService>();
    let mcp = app.state::<dgos_application::ModuleMcpService>();
    let result = match dispatch
        .authorize_bound(call, ModulePermission::McpCall)
        .await
    {
        Ok((call, grant)) => {
            mcp.call_tool(call, grant, admitted_at, server_id, tool_name, arguments)
                .await
        }
        Err(error) => Err(error),
    };
    CommandResponse::from(result)
}

/// The user's answer to a module's MCP confirmation (D093). Main-window and
/// trusted-origin gate on every route, because the dialog and the calling
/// module both survive SPA navigation; the user is host-derived. The only
/// inputs are the host-issued confirmation id and the decision. An unknown,
/// expired, already answered or foreign id is `PermissionDenied`; an id
/// answers exactly once.
///
/// Scripts sharing the page's realm can call this command too (D050
/// residual risk): the dialog stops a module from calling without the user
/// knowing, it is not isolation.
#[tauri::command]
pub async fn resolve_module_mcp_confirmation(
    app: tauri::AppHandle,
    window: WebviewWindow,
    confirmation_id: String,
    approved: bool,
) -> CommandResponse<()> {
    let session = match host::module_confirmation_session(&window) {
        Ok(session) => session,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let mcp = app.state::<dgos_application::ModuleMcpService>();
    CommandResponse::from(mcp.confirmations().resolve(
        session.user_id(),
        &confirmation_id,
        approved,
    ))
}
