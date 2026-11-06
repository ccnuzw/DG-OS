use crate::host::{self, Capability};
use dgos_agents::{capabilities::{ProjectCapabilities, CapabilityRevision, CapabilitySnapshot, CapabilityConfirmation, SaveCapabilitiesInput}, AgentManager};
use dgos_types::{BuiltinCaller, HostExecutionContext};
use serde::Deserialize;
use tauri::{Manager, WebviewWindow};
use tokio_util::sync::CancellationToken;

// Return the admission itself: copying the owner would release maintenance protection
// before the service future starts. Commands drain admitted work, then reject stale results.
fn caller(app: &tauri::AppHandle, window: &WebviewWindow) -> Result<(host::Admission<HostExecutionContext>, CancellationToken), String> {
    let context = host::context(window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = resolver.agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok((context, lease))
}
#[tauri::command]
pub async fn read_development_capabilities(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<Option<CapabilityRevision>, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<ProjectCapabilities>().read(context.user_id(), &project_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn save_development_capabilities(app: tauri::AppHandle, window: WebviewWindow, input: SaveCapabilitiesInput) -> Result<CapabilityRevision, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<ProjectCapabilities>().save(context.user_id(), input).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn revoke_development_capabilities(app: tauri::AppHandle, window: WebviewWindow, project_id: String) -> Result<i64, String> {
    let (context, lease) = caller(&app, &window)?;
    // Drain revoke through binding/confirmation cleanup after its durable epoch update.
    let result = app.state::<ProjectCapabilities>().revoke(context.user_id(), &project_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_capability_catalog(app: tauri::AppHandle, window: WebviewWindow) -> Result<serde_json::Value, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<ProjectCapabilities>().catalog(context.user_id()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn get_development_session_capabilities(app: tauri::AppHandle, window: WebviewWindow, session_id: String) -> Result<Option<CapabilitySnapshot>, String> {
    let (context, lease) = caller(&app, &window)?;
    let sessions = app.state::<AgentManager>().list(context.user_id()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    if !sessions.iter().any(|s| s.id == session_id) { return Err("session_not_found".into()); }
    let result = app.state::<ProjectCapabilities>().session_snapshot(context.user_id(), &session_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn list_development_capability_confirmations(app: tauri::AppHandle, window: WebviewWindow, session_id: String) -> Result<Vec<CapabilityConfirmation>, String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<ProjectCapabilities>().pending_confirmations(context.user_id(), &session_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespondCapabilityInput { session_id: String, confirmation_id: String, approved: bool }
#[tauri::command]
pub async fn respond_development_capability_confirmation(app: tauri::AppHandle, window: WebviewWindow, input: RespondCapabilityInput) -> Result<(), String> {
    let (context, lease) = caller(&app, &window)?;
    let result = app.state::<ProjectCapabilities>().respond_confirmation(context.user_id(), &input.session_id, &input.confirmation_id, input.approved).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

/// Exercises the production command on a real host, with the installed skills lock as a
/// deterministic await barrier. The harness must renew the account lease afterward.
#[cfg(feature = "webview-smoke")]
pub async fn capability_admission_regression(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    skills: &std::sync::Arc<tokio::sync::RwLock<dgos_skills::SkillsManager>>,
) -> Result<serde_json::Value, String> {
    let gate = app.state::<dgos_db::MaintenanceGate>();
    drop(gate.begin_maintenance().map_err(|_| "capability_regression_activity_busy")?);
    let blocked_skills = skills.write().await;
    let request = read_development_capability_catalog(app.clone(), window.clone());
    tokio::pin!(request);
    // Even if every database read completes in this poll, the real catalog cannot
    // complete while the installed skills manager is exclusively locked.
    if !matches!(futures::poll!(request.as_mut()), std::task::Poll::Pending) {
        return Err("capability_regression_catalog_did_not_wait".into());
    }
    let blocked_maintenance = gate.begin_maintenance().is_err();
    if !blocked_maintenance { return Err("capability_regression_admission_dropped".into()); }

    app.state::<std::sync::Arc<crate::session::SessionResolver>>().close_agents();
    // Revoking the lease must not drop admitted work before it can finish. This is
    // also the drain policy used by revoke's epoch-write and binding cleanup.
    let drains_after_revoke = matches!(futures::poll!(request.as_mut()), std::task::Poll::Pending);
    let protected_after_revoke = gate.begin_maintenance().is_err();
    if !drains_after_revoke || !protected_after_revoke {
        return Err("capability_regression_cancelled_admitted_work".into());
    }
    drop(blocked_skills);
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), request.as_mut()).await
        .map_err(|_| "capability_regression_catalog_did_not_finish")?;
    if !matches!(&response, Err(error) if error == "session_invalid") {
        return Err(format!("capability_regression_stale_response: {response:?}"));
    }
    drop(gate.begin_maintenance().map_err(|_| "capability_regression_admission_not_released")?);
    Ok(serde_json::json!({
        "command": "read_development_capability_catalog",
        "blocked_maintenance_while_awaiting": blocked_maintenance,
        "drains_after_lease_revocation": drains_after_revoke,
        "blocked_maintenance_after_lease_revocation": protected_after_revoke,
        "revoked_session_response": "session_invalid",
        "maintenance_available_after_completion": true,
        "execution_source": "production_command_with_installed_skills_lock"
    }))
}
