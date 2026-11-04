//! FR-010 host commands (interface contract §14.8). Eight typed commands for
//! the FR-007 LAN tab. Every one enters through `host::context` with the
//! Settings caller and `Capability::LanManage`; none takes a user id, caller
//! or session token. The access code's plaintext leaves only through
//! `issue_lan_access_code`, once.

use super::CommandResponse;
use crate::host::{self, Capability};
use dgos_db::LanAuditPage;
use dgos_lan::{LanDevice, LanIssuedCode, LanService, LanStatus};
use dgos_types::BuiltinCaller;
use serde::Deserialize;
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListLanAuditInput {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

fn lan(app: &tauri::AppHandle) -> Arc<LanService> {
    app.state::<Arc<LanService>>().inner().clone()
}

#[tauri::command]
pub async fn get_lan_status(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<LanStatus> {
    if let Err(error) = host::context(&window, BuiltinCaller::Settings, Capability::LanManage) {
        return CommandResponse::from(Err(error));
    }
    CommandResponse::from(Ok(lan(&app).status().await))
}

#[tauri::command]
pub async fn set_lan_enabled(
    app: tauri::AppHandle,
    window: WebviewWindow,
    enabled: bool,
) -> CommandResponse<LanStatus> {
    let _context = match host::context(&window, BuiltinCaller::Settings, Capability::LanManage) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    CommandResponse::from(lan(&app).set_enabled(enabled).await)
}

/// `port` is read as a wide integer so out-of-range values reach the
/// `1010012` check instead of failing deserialization.
#[tauri::command]
pub async fn set_lan_port(
    app: tauri::AppHandle,
    window: WebviewWindow,
    port: i64,
) -> CommandResponse<LanStatus> {
    let _context = match host::context(&window, BuiltinCaller::Settings, Capability::LanManage) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    CommandResponse::from(lan(&app).set_port(port).await)
}

#[tauri::command]
pub async fn issue_lan_access_code(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<Option<LanIssuedCode>> {
    if let Err(error) = host::context(&window, BuiltinCaller::Settings, Capability::LanManage) {
        return CommandResponse::from(Err(error));
    }
    CommandResponse::from(Ok(lan(&app).issue_code().await))
}

#[tauri::command]
pub async fn list_lan_devices(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<Vec<LanDevice>> {
    if let Err(error) = host::context(&window, BuiltinCaller::Settings, Capability::LanManage) {
        return CommandResponse::from(Err(error));
    }
    CommandResponse::from(Ok(lan(&app).list_devices()))
}

#[tauri::command]
pub async fn disconnect_lan_device(
    app: tauri::AppHandle,
    window: WebviewWindow,
    session_id: String,
) -> CommandResponse<bool> {
    if let Err(error) = host::context(&window, BuiltinCaller::Settings, Capability::LanManage) {
        return CommandResponse::from(Err(error));
    }
    CommandResponse::from(Ok(lan(&app).disconnect_device(&session_id).await))
}

#[tauri::command]
pub async fn disconnect_all_lan_devices(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> CommandResponse<usize> {
    if let Err(error) = host::context(&window, BuiltinCaller::Settings, Capability::LanManage) {
        return CommandResponse::from(Err(error));
    }
    CommandResponse::from(Ok(lan(&app).disconnect_all().await))
}

/// Rows of the host-derived user and of unauthenticated sources, newest first.
#[tauri::command]
pub async fn list_lan_audit_events(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: ListLanAuditInput,
) -> CommandResponse<LanAuditPage> {
    let context = match host::context(&window, BuiltinCaller::Settings, Capability::LanManage) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    CommandResponse::from(
        lan(&app)
            .list_audit(context.user_id(), input.cursor.as_deref(), input.limit)
            .await,
    )
}
