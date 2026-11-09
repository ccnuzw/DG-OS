use super::CommandResponse;
use crate::host;
use dgos_db::{CliDetection, CliUpdateMetadata, CliPaths, CliRepository, CliSave, CliSnapshot};
use dgos_agents::cli::{CliTool, CliProbe};
use dgos_types::DgosError;
use tauri::{Manager, WebviewWindow};
fn failure(error: String) -> DgosError {
    if error == "session_invalid" { DgosError::SessionInvalid } else { DgosError::InternalError("CLI preferences unavailable".into()) }
}
fn detection(probe: &CliProbe) -> CliDetection {
    CliDetection { status: probe.status.into(), path: probe.path.clone(), version: probe.version.clone(), update: None }
}
#[derive(serde::Serialize)]
pub struct CliPreferencesView {
    #[serde(flatten)]
    pub snapshot: CliSnapshot,
    pub codex_protocol_reference: &'static dgos_agents::cli::CodexProtocolReference,
}
#[tauri::command]
pub async fn get_cli_preferences(app: tauri::AppHandle, window: WebviewWindow) -> CommandResponse<CliPreferencesView> {
    let session = match host::workspace_catalog_session(&window) { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    CommandResponse::from(app.state::<CliRepository>().get(session.user_id()).await.map(|snapshot| {
        CliPreferencesView { snapshot, codex_protocol_reference: dgos_agents::cli::codex_protocol_reference() }
    }).map_err(failure))
}
#[tauri::command]
pub async fn set_cli_preferences(app: tauri::AppHandle, window: WebviewWindow, paths: CliPaths, expected_revision: u64) -> CommandResponse<CliSave> {
    let session = match host::workspace_catalog_session(&window) { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    let _permit = match host::admit_activity(&app) { Ok(permit) => permit, Err(error) => return CommandResponse::from(Err(error)) };
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (current, lease) = match resolver.agent_session() { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    if current.user_id() != session.user_id() || lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    let result = async {
        let repository = app.state::<CliRepository>();
        let stored = repository.get(session.user_id()).await.map_err(failure)?;
        if stored.revision != expected_revision { return Ok(CliSave { saved: false, snapshot: stored }); }
        let mut paths = paths;
        let node = if paths.node != stored.paths.node {
            if let Some(path) = paths.node.as_deref() {
                let probe = tokio::select! {
                    biased;
                    _ = lease.cancelled() => return Err(DgosError::SessionInvalid),
                    probe = dgos_agents::cli::probe(CliTool::Node, Some(path)) => probe,
                };
                if probe.status != "available" { return Err(DgosError::InternalError("Selected Node executable could not be verified".into())); }
                paths.node = probe.path.clone();
                Some(detection(&probe))
            } else { None }
        } else { None };
        repository.save_verified(session.user_id(), paths, expected_revision, node, || !lease.is_cancelled()).await.map_err(failure)
    }.await;
    CommandResponse::from(result)
}
#[tauri::command]
pub async fn probe_cli(app: tauri::AppHandle, window: WebviewWindow, tool: CliTool, path: Option<String>) -> CommandResponse<CliProbe> {
    let session = match host::workspace_catalog_session(&window) { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (current, lease) = match resolver.agent_session() { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    // A probe is valid only for the account that issued it. Account switches cancel the lease.
    if current.user_id() != session.user_id() || lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    let result = tokio::select! {
        biased;
        _ = lease.cancelled() => return CommandResponse::from(Err(DgosError::SessionInvalid)),
        result = dgos_agents::cli::probe(tool, path.as_deref()) => result,
    };
    if let Err(error) = app.state::<CliRepository>().record_detection(session.user_id(), tool.name(), detection(&result), || !lease.is_cancelled()).await {
        return CommandResponse::from(Err(failure(error)));
    }
    CommandResponse::from(Ok(result))
}
const TICKET_LIFETIME: std::time::Duration = std::time::Duration::from_secs(120);

struct UpdateTicket {
    candidate: dgos_agents::cli_updates::Candidate,
    owner: String,
    lease: tokio_util::sync::CancellationToken,
    expires: std::time::Instant,
}

#[derive(Default)]
pub struct CliUpdateTickets {
    entries: std::sync::Mutex<std::collections::HashMap<String, UpdateTicket>>,
}

impl CliUpdateTickets {
    fn issue(&self, candidate: dgos_agents::cli_updates::Candidate, owner: String, lease: tokio_util::sync::CancellationToken) -> String {
        let now = std::time::Instant::now();
        let mut entries = self.entries.lock().unwrap_or_else(|poison| poison.into_inner());
        // Retire old checks and superseded offers; repeated discovery must not grow the book.
        entries.retain(|_, entry| entry.expires > now && !entry.lease.is_cancelled()
            && !(entry.owner == owner && entry.candidate.tool.name() == candidate.tool.name()));
        if entries.len() >= 64 { entries.clear(); }
        let id = uuid::Uuid::new_v4().to_string();
        entries.insert(id.clone(), UpdateTicket { candidate, owner, lease, expires: now + TICKET_LIFETIME });
        id
    }

    fn consume(&self, id: &str, owner: &str) -> Option<UpdateTicket> {
        let mut entries = self.entries.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = entries.get(id)?;
        if entry.owner != owner { return None; }
        let entry = entries.remove(id)?;
        (!entry.lease.is_cancelled() && entry.expires > std::time::Instant::now()).then_some(entry)
    }
}

#[derive(serde::Serialize)]
pub struct CliUpdateOutcome { pub status: &'static str, pub probe: Option<CliProbe> }

#[tauri::command]
pub async fn probe_node_directory(app: tauri::AppHandle, window: WebviewWindow, directory: String) -> CommandResponse<CliProbe> {
    let session = match host::workspace_catalog_session(&window) { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (current, lease) = match resolver.agent_session() { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    if current.user_id() != session.user_id() || lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    let result = tokio::select! {
        biased;
        _ = lease.cancelled() => Err(DgosError::SessionInvalid),
        result = dgos_agents::cli::probe_node_directory(&directory) => Ok(result),
    };
    CommandResponse::from(result)
}

#[tauri::command]
pub async fn check_cli_update(app: tauri::AppHandle, window: WebviewWindow, tool: CliTool, path: Option<String>) -> CommandResponse<dgos_agents::cli_updates::CheckResult> {
    let context = match host::workspace_catalog_session(&window) { Ok(c) => c, Err(e) => return CommandResponse::from(Err(e)) };
    let resolver = app.state::<std::sync::Arc<crate::session::SessionResolver>>();
    let (session, lease) = match resolver.agent_session() { Ok(s) => s, Err(e) => return CommandResponse::from(Err(e)) };
    if session.user_id() != context.user_id() || lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    let stored = match app.state::<CliRepository>().get(context.user_id()).await {
        Ok(stored) => stored,
        Err(error) => return CommandResponse::from(Err(failure(error))),
    };
    let recorded = stored.detections.get(tool.name());
    let requested_path = path.as_deref().or_else(|| recorded.and_then(|record| record.path.as_deref()));
    let (mut result, candidate) = tokio::select! {
        biased;
        _ = lease.cancelled() => return CommandResponse::from(Err(DgosError::SessionInvalid)),
        result = dgos_agents::cli_updates::check(tool, requested_path) => result,
    };
    if lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    if result.status == "available" && candidate.is_none() { result.status = "check_failed"; }
    if let Some(record) = recorded.filter(|record| record.path.as_deref() == requested_path && (record.version == result.current_version || result.status == "check_failed" && result.current_version.is_none())) {
        let metadata = CliUpdateMetadata {
            status: result.status.into(), current_version: result.current_version.clone().or_else(|| record.version.clone()),
            latest_version: result.latest_version.clone(), method: result.method.clone(),
        };
        if let Err(error) = app.state::<CliRepository>().record_update(context.user_id(), tool.name(), record.path.as_deref(), record.version.as_deref(), metadata, || !lease.is_cancelled()).await {
            return CommandResponse::from(Err(failure(error)));
        }
    }
    if lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    result.ticket = candidate.filter(|_| result.status == "available").map(|candidate|
        app.state::<CliUpdateTickets>().issue(candidate, context.user_id().to_owned(), lease));
    CommandResponse::from(Ok(result))
}

#[tauri::command]
pub async fn update_cli(app: tauri::AppHandle, window: WebviewWindow, ticket: String) -> CommandResponse<CliUpdateOutcome> {
    let context = match host::workspace_catalog_session(&window) { Ok(c) => c, Err(e) => return CommandResponse::from(Err(e)) };
    let owner = context.user_id().to_owned();
    let Some(issued) = app.state::<CliUpdateTickets>().consume(&ticket, &owner) else {
        return CommandResponse::from(Ok(CliUpdateOutcome { status: "stale", probe: None }));
    };
    // Release this command's read admission before taking the existing exclusive gate.
    // It atomically refuses active work and blocks new work until the installer settles.
    drop(context);
    let _maintenance = match app.state::<dgos_db::repositories::maintenance_gate::MaintenanceGate>().begin_maintenance() {
        Ok(lease) => lease,
        Err(_) => return CommandResponse::from(Ok(CliUpdateOutcome { status: "busy", probe: None })),
    };
    let outcome = tokio::select! {
        biased;
        _ = issued.lease.cancelled() => CliUpdateOutcome { status: "unknown", probe: None },
        result = dgos_agents::cli_updates::update(&issued.candidate) => match result {
            Ok(probe) => {
                // Installation already happened: a failed durable write is not a safe
                // installer failure and must never invite an automatic retry.
                let persisted = app.state::<CliRepository>().record_detection(&owner, probe.tool.name(), detection(&probe), || !issued.lease.is_cancelled()).await;
                CliUpdateOutcome { status: if persisted.is_ok() { "updated" } else { "unknown" }, probe: Some(probe) }
            },
            Err("outcome_unknown" | "verification_failed") => CliUpdateOutcome { status: "unknown", probe: None },
            Err("identity_changed" | "target_changed") => CliUpdateOutcome { status: "stale", probe: None },
            Err("busy") => CliUpdateOutcome { status: "busy", probe: None },
            Err(_) => CliUpdateOutcome { status: "failed", probe: None },
        },
    };
    CommandResponse::from(Ok(outcome))
}
