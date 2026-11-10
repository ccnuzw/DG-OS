//! Account-private desktop content and revocable, exact-directory execution authority.
use crate::AgentManager;
use dgos_db::development_repository::Project;
pub use dgos_db::development_repository::{DesktopState, WorkspaceTrust};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveDesktopStateInput { pub expected_revision: i64, pub value: serde_json::Value }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadWorkspaceTrustInput { pub project_id: String, pub task_id: Option<String> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetWorkspaceTrustInput {
    pub project_id: String, pub task_id: Option<String>,
    pub expected_directory_identity: String, pub expected_revision: i64,
    pub scope: String, pub granted: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivateWorkspaceAgentInput {
    pub project_id: String, pub task_id: Option<String>,
    pub expected_path: String, pub expected_directory_identity: String, pub expected_revision: i64,
}
fn live(lease: &CancellationToken) -> Result<(), String> {
    if lease.is_cancelled() { Err("session_invalid".into()) } else { Ok(()) }
}
impl AgentManager {
    /// Caller retains project admission through dispatch. An external terminal
    /// remains an external writer after launch, never an owned resumable process.
    pub async fn authorize_development_external_terminal(&self, owner: &str, project: &Project, lease: &CancellationToken) -> Result<(), String> {
        self.ready().await?; live(lease)?;
        self.require_workspace_trust(owner, project, "terminal", false).await?;
        self.workspace_available(&project.path, None).await?;
        self.host_workspace_available(&project.path).await?;
        live(lease)
    }
    pub async fn read_development_desktop_state(&self, owner: &str, lease: CancellationToken) -> Result<DesktopState, String> {
        self.ready().await?; live(&lease)?;
        let result = self.inner.repository.desktop_state(owner).await;
        live(&lease)?; result
    }
    pub async fn save_development_desktop_state(&self, owner: &str, input: SaveDesktopStateInput, lease: CancellationToken) -> Result<DesktopState, String> {
        let _admission = self.admit_native_extensions(&lease).await?;
        let _permit = self.inner.gate.admit().map_err(|_| "maintenance_active")?;
        let result = self.inner.repository.save_desktop_state(owner, input.expected_revision, input.value, || !lease.is_cancelled() && !self.inner.stop.is_cancelled()).await;
        live(&lease)?; result
    }
    async fn trust_project(&self, owner: &str, project_id: &str, task_id: Option<&str>, require_directory: bool) -> Result<Project, String> {
        let mut project = self.inner.repository.project(owner, project_id).await?;
        if let Some(task_id) = task_id {
            let task = self.inner.repository.task(owner, task_id).await?;
            if task.project_id != project.id { return Err("task_not_found".into()); }
            if require_directory { project = self.task_project(owner, task_id).await?; }
            else if let Some(workspace) = self.inner.repository.task_workspace(owner, task_id).await? {
                project.path = workspace.path;
                project.directory_identity = workspace.directory_identity;
            }
        }
        if require_directory {
            if project.archived { return Err("project_archived".into()); }
            crate::projects::validate_directory(&project).await?;
        }
        Ok(project)
    }
    pub async fn read_development_workspace_trust(&self, owner: &str, input: ReadWorkspaceTrustInput, lease: CancellationToken) -> Result<WorkspaceTrust, String> {
        let _admission = self.admit_native_extensions(&lease).await?;
        let project = self.trust_project(owner, &input.project_id, input.task_id.as_deref(), true).await?;
        let result = self.inner.repository.workspace_trust(owner, &project).await;
        live(&lease)?; result
    }
    /// Entering an opened project's selected workspace is the trusted host
    /// action; reading grants or restoring hidden tabs never creates authority.
    pub async fn activate_development_workspace_agent(&self, owner: &str, input: ActivateWorkspaceAgentInput, lease: CancellationToken) -> Result<WorkspaceTrust, String> {
        let _admission = self.admit_native_extensions(&lease).await?;
        let _permit = self.inner.gate.admit().map_err(|_| "maintenance_active")?;
        let project = self.trust_project(owner, &input.project_id, input.task_id.as_deref(), true).await?;
        let selected = self.inner.repository.selection(owner).await?;
        if selected.project_id.as_deref() != Some(input.project_id.as_str()) || selected.task_id != input.task_id { return Err("invalid_selection".into()); }
        if project.path != input.expected_path || project.directory_identity != input.expected_directory_identity { return Err("workspace_changed".into()); }
        live(&lease)?;
        let result = self.inner.repository.activate_workspace_agent(owner, &project, input.expected_revision, || !lease.is_cancelled() && !self.inner.stop.is_cancelled()).await?;
        crate::projects::validate_directory(&project).await?;
        live(&lease)?;
        Ok(result)
    }
    pub async fn set_development_workspace_trust(&self, owner: &str, input: SetWorkspaceTrustInput, lease: CancellationToken) -> Result<WorkspaceTrust, String> {
        let _admission = self.admit_native_extensions(&lease).await?;
        let _permit = self.inner.gate.admit().map_err(|_| "maintenance_active")?;
        // Revocation acts on the recorded identity, even when its directory is
        // missing/replaced. Owned processes may still be executing in the old cwd.
        let project = self.trust_project(owner, &input.project_id, input.task_id.as_deref(), input.granted).await?;
        if project.directory_identity != input.expected_directory_identity { return Err("workspace_changed".into()); }
        live(&lease)?;
        let result = self.inner.repository.set_workspace_trust(owner, &project, input.expected_revision, &input.scope, input.granted, || !lease.is_cancelled() && !self.inner.stop.is_cancelled()).await?;
        // Persist revocation before touching only the host's matching owned processes.
        // A cleanup error leaves the revoked revision durable and the safety locks held.
        if !input.granted {
            if input.scope == "agent" { self.revoke_directory_agents(owner, &project.id, &project.path).await?; }
            else { self.revoke_directory_processes(owner, &project.id, &project.path, &input.scope).await?; }
        }
        if input.granted { crate::projects::validate_directory(&project).await?; }
        live(&lease)?;
        Ok(result)
    }
    pub(crate) async fn require_workspace_trust(&self, owner: &str, project: &Project, scope: &str, explicit: bool) -> Result<(), String> {
        crate::projects::validate_directory(project).await?;
        if explicit || self.inner.repository.workspace_trust(owner, project).await?.permits(scope) { Ok(()) } else { Err("workspace_untrusted".into()) }
    }
}
