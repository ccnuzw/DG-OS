//! Explicit native-session closure for one task's validated actual directory.
//! A projection grants no write permission: every subsequent save/run rechecks admission.
use crate::{AgentManager, Engine, projects::validate_directory};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadTakeoverInput { pub project_id: String, pub task_id: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseTakeoverInput {
    pub project_id: String, pub task_id: String, pub token: String, pub confirmed: bool,
}
#[derive(Debug, Serialize)]
pub struct TakeoverSession {
    pub id: String, pub task_id: Option<String>, pub engine: Engine, pub status: String,
}
#[derive(Debug, Serialize)]
pub struct TakeoverView {
    pub project_id: String, pub task_id: String, pub directory: String,
    pub sessions: Vec<TakeoverSession>, pub restricted: bool, pub host_busy: bool,
    pub can_close: bool, pub token: String,
}
fn live(lease: &CancellationToken) -> Result<(), String> {
    if lease.is_cancelled() { Err("session_invalid".into()) } else { Ok(()) }
}
impl AgentManager {
    // Caller holds admission. No foreign session IDs, tasks, engines or statuses escape.
    async fn takeover_projection(&self, owner: &str, input: &ReadTakeoverInput, lease: &CancellationToken) -> Result<TakeoverView, String> {
        self.ready().await?;
        live(lease)?;
        let project = self.task_project(owner, &input.task_id).await?;
        if project.id != input.project_id { return Err("task_not_found".into()); }
        if project.archived { return Err("project_archived".into()); }
        let epoch = self.inner.repository.capability_epoch(owner, &project.id).await?;
        let entries = self.native_workspace_entries(&project.path).await;
        let host_busy = self.host_workspace_available(&project.path).await.is_err();
        let mut sessions = Vec::new();
        let mut restricted = false;
        let mut occupancy = Vec::new();
        for (record, backend, generation) in entries {
            occupancy.push((record.id.clone(), generation));
            if record.owner != owner { restricted = true; continue; }
            let status = backend.snapshot().await.status;
            sessions.push(TakeoverSession { id: record.id, task_id: record.task_id, engine: record.engine, status });
        }
        let bytes = serde_json::to_vec(&(owner, &input.project_id, &input.task_id, &project.path, &project.directory_identity, epoch, &occupancy, host_busy)).map_err(|_| "invalid_request")?;
        live(lease)?;
        Ok(TakeoverView { project_id: project.id, task_id: input.task_id.clone(), directory: project.path,
            can_close: !sessions.is_empty() && !restricted && !host_busy, sessions, restricted, host_busy,
            token: format!("{:x}", Sha256::digest(bytes)) })
    }
    pub async fn read_development_takeover(&self, owner: &str, input: ReadTakeoverInput, lease: CancellationToken) -> Result<TakeoverView, String> {
        let _admission = self.inner.admission.lock().await;
        self.takeover_projection(owner, &input, &lease).await
    }
    pub async fn close_development_takeover(&self, owner: &str, input: CloseTakeoverInput, lease: CancellationToken) -> Result<TakeoverView, String> {
        let _admission = self.inner.admission.lock().await;
        if !input.confirmed { return Err("takeover_confirmation_required".into()); }
        let scope = ReadTakeoverInput { project_id: input.project_id, task_id: input.task_id };
        let epoch = self.inner.repository.capability_epoch(owner, &scope.project_id).await?;
        let before = self.takeover_projection(owner, &scope, &lease).await?;
        if before.token != input.token { return Err("takeover_stale".into()); }
        if !before.can_close { return Err("workspace_busy".into()); }
        for session in &before.sessions {
            // Revalidate at each effect boundary, including changes made by capability service
            // outside manager admission. Never close a saved session by task/project alone.
            self.ready().await?;
            live(&lease)?;
            let project = self.task_project(owner, &scope.task_id).await?;
            if project.id != scope.project_id || project.path != before.directory || project.archived { return Err("takeover_stale".into()); }
            validate_directory(&project).await?;
            if self.inner.repository.capability_epoch(owner, &scope.project_id).await? != epoch { return Err("capability_revoked".into()); }
            let current = self.native_workspace_entries(&project.path).await;
            if !current.iter().any(|(record, _, _)| record.owner == owner && record.id == session.id) { return Err("takeover_stale".into()); }
            live(&lease)?;
            #[cfg(test)]
            if self.inner.fail_takeover_close.load(std::sync::atomic::Ordering::SeqCst) { return Err("cleanup_pending".into()); }
            if !self.close_takeover_session(owner, &session.id, &scope.project_id, epoch, &lease).await? { return Err("cleanup_pending".into()); }
            live(&lease)?;
        }
        let after = self.takeover_projection(owner, &scope, &lease).await?;
        if self.inner.repository.capability_epoch(owner, &scope.project_id).await? != epoch { return Err("capability_revoked".into()); }
        self.workspace_available(&after.directory, None).await?;
        self.host_workspace_available(&after.directory).await?;
        live(&lease)?;
        Ok(after)
    }
}

#[cfg(all(test, unix))]
mod tests;
