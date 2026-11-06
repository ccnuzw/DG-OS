use crate::{claude_session::ClaudeSession, codex_session::CodexSession, session::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use dgos_db::development_repository::{DevelopmentRepository, StoredAgentSession};
use dgos_db::{ActivityPermit, MaintenanceGate};

#[derive(Clone)]
pub(crate) enum Backend {
    Codex(CodexSession),
    Claude(ClaudeSession),
}
impl Backend {
    pub(crate) async fn snapshot(&self) -> SessionView {
        match self {
            Self::Codex(s) => s.snapshot().await,
            Self::Claude(s) => s.snapshot().await,
        }
    }
    async fn close(&self) -> bool {
        match self {
            Self::Codex(s) => s.close().await,
            Self::Claude(s) => s.close().await,
        }
    }
    async fn wait_stopped(&self) -> bool {
        match self {
            Self::Codex(s) => s.wait_stopped().await,
            Self::Claude(s) => s.wait_stopped().await,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub owner: String,
    pub engine: Engine,
    pub native_id: Option<String>,
    pub workspace: String,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}
impl SessionRecord {
    fn stored(&self) -> StoredAgentSession {
        StoredAgentSession { id: self.id.clone(), owner: self.owner.clone(), engine: match self.engine { Engine::Codex => "codex", Engine::Claude => "claude" }.into(), native_id: self.native_id.clone(), workspace: self.workspace.clone(), executable: self.executable.clone(), model: self.model.clone(), project_id: self.project_id.clone(), task_id: self.task_id.clone() }
    }
    fn from_stored(r: StoredAgentSession) -> Result<Self, String> {
        if Uuid::parse_str(&r.id).is_err() || r.owner.is_empty() || !std::path::Path::new(&r.workspace).is_absolute()
            || r.project_id.is_some() != r.task_id.is_some() { return Err("session_index_invalid".into()); }
        Ok(Self { id: r.id, owner: r.owner, engine: match r.engine.as_str() { "codex" => Engine::Codex, "claude" => Engine::Claude, _ => return Err("session_index_invalid".into()) }, native_id: r.native_id, workspace: r.workspace, executable: r.executable, model: r.model, project_id: r.project_id, task_id: r.task_id })
    }
}
#[derive(Serialize)]
pub struct OwnedSessionView {
    pub id: String,
    pub engine: Engine,
    pub capabilities: Option<crate::capabilities::CapabilitySnapshot>,
    #[serde(flatten)]
    pub view: SessionView,
}
struct Active {
    backend: Backend,
    lease: CancellationToken,
    generation: Uuid,
}
#[derive(Clone)]
pub struct AgentManager {
    pub(crate) inner: Arc<Inner>,
}
pub(crate) struct Inner {
    records: Mutex<Vec<SessionRecord>>,
    load_error: Mutex<Option<String>>,
    active: Mutex<HashMap<String, Active>>,
    pub(crate) admission: Mutex<()>,
    path: PathBuf,
    pub(crate) repository: DevelopmentRepository,
    pub(crate) stop: CancellationToken,
    pub(crate) gate: MaintenanceGate,
    /// Permits whose backend failed to close. Kept so VACUUM stays blocked while the child may still run.
    retained: Mutex<Vec<ActivityPermit>>,
    pub(crate) watchers: Mutex<Vec<tokio::task::JoinHandle<bool>>>,
    pub(crate) host: Mutex<HashMap<String, Arc<crate::runtime::HostProcess>>>,
    #[cfg(test)]
    pub(crate) fail_takeover_close: std::sync::atomic::AtomicBool,
    pub(crate) capabilities: std::sync::RwLock<Option<crate::capabilities::ProjectCapabilities>>,
}
impl AgentManager {
    pub fn new(path: PathBuf, repository: DevelopmentRepository) -> Self {
        Self::with_maintenance(path, repository, MaintenanceGate::new())
    }
    /// Production constructor. `gate` must be the process-wide gate that VACUUM uses.
    pub fn with_maintenance(path: PathBuf, repository: DevelopmentRepository, gate: MaintenanceGate) -> Self {
        Self {
            inner: Arc::new(Inner {
                records: Mutex::new(Vec::new()),
                active: Mutex::new(HashMap::new()),
                admission: Mutex::new(()),
                path,
                repository,
                stop: CancellationToken::new(),
                load_error: Mutex::new(Some("session_index_unavailable".into())),
                gate,
                retained: Mutex::new(Vec::new()),
                watchers: Mutex::new(Vec::new()),
                host: Mutex::new(HashMap::new()),
                #[cfg(test)]
                fail_takeover_close: std::sync::atomic::AtomicBool::new(false),
                capabilities: std::sync::RwLock::new(None),
            }),
        }
    }
    pub fn install_capabilities(&self, service: crate::capabilities::ProjectCapabilities) {
        *self.inner.capabilities.write().unwrap_or_else(|e| e.into_inner()) = Some(service);
    }
    pub(crate) fn capabilities(&self) -> Option<crate::capabilities::ProjectCapabilities> {
        self.inner.capabilities.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub async fn load(&self) -> Result<(), String> {
        let result = async {
            if !self.inner.repository.legacy_imported().await? {
                let records: Vec<SessionRecord> = match tokio::fs::read(&self.inner.path).await {
                    Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "session_index_invalid")?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(_) => return Err("session_index_unavailable".into()),
                };
                // Legacy files never grant project/task ownership, even if extra keys exist.
                let stored: Vec<_> = records.into_iter().map(|mut r| { r.project_id = None; r.task_id = None; r.stored() }).collect();
                self.inner.repository.initialize_legacy(&stored).await?;
            }
            self.inner.repository.recover_operations().await?;
            self.inner.repository.recover_runs().await?;
            self.reload_records().await
        }.await;
        *self.inner.load_error.lock().await = result.as_ref().err().cloned();
        result
    }
    pub(crate) async fn ready(&self) -> Result<(), String> {
        if let Some(error) = self.inner.load_error.lock().await.clone() { return Err(error); }
        if self.inner.stop.is_cancelled() { return Err("cancelled".into()); }
        Ok(())
    }
    pub(crate) async fn reload_records(&self) -> Result<(), String> {
        let records = self.inner.repository.sessions().await?.into_iter().map(SessionRecord::from_stored).collect::<Result<Vec<_>, _>>()?;
        *self.inner.records.lock().await = records;
        Ok(())
    }
    pub async fn list(&self, owner: &str) -> Vec<SessionRecord> {
        self.inner
            .records
            .lock()
            .await
            .iter()
            .filter(|r| r.owner == owner)
            .cloned()
            .collect()
    }
    pub async fn open(
        &self,
        owner: &str,
        engine: Engine,
        id: String,
        options: SessionOptions,
        task_id: Option<String>,
        lease: CancellationToken,
    ) -> Result<OwnedSessionView, String> {
        self.open_with_capabilities(owner, engine, id, crate::capabilities::SessionCapabilities { options, revision: None }, task_id, lease).await
    }
    pub async fn open_with_capabilities(
        &self, owner: &str, engine: Engine, id: String, capabilities: crate::capabilities::SessionCapabilities,
        task_id: Option<String>, lease: CancellationToken,
    ) -> Result<OwnedSessionView, String> {
        let crate::capabilities::SessionCapabilities { mut options, revision: capability_revision } = capabilities;
        Uuid::parse_str(&id).map_err(|_| "invalid_request_id")?;
        self.ready().await?;
        let _admission = self.inner.admission.lock().await;
        self.validate_handoff_target(owner, &id).await?;
        if self.inner.stop.is_cancelled() || lease.is_cancelled() {
            return Err("cancelled".into());
        }
        if !options.trust_workspace {
            return Err("workspace_untrusted".into());
        }
        let existing = self
            .inner
            .records
            .lock()
            .await
            .iter()
            .find(|r| r.id == id)
            .cloned();
        if let Some(record) = &existing {
            if record.owner != owner || record.engine != engine { return Err("session_not_found".into()); }
            if task_id.as_ref().is_some_and(|id| Some(id) != record.task_id.as_ref()) { return Err("task_not_found".into()); }
        }
        let task_id = match &existing { Some(r) => r.task_id.clone(), None => task_id };
        let project = if let Some(task_id) = &task_id {
            let project = self.ensure_task_workspace(owner, task_id).await?;
            if project.archived { return Err("project_archived".into()); }
            crate::projects::validate_directory(&project).await?;
            Some(project)
        } else if existing.is_none() { return Err("task_not_found".into()); } else { None };
        if let Some(record) = &existing {
            if record.owner != owner || record.engine != engine {
                return Err("session_not_found".into());
            }
            if project.as_ref().is_some_and(|p| p.path != record.workspace) { return Err("workspace_changed".into()); }
            options.workspace = record.workspace.clone();
            options.resume_id = record.native_id.clone();
            options.executable = record.executable.clone();
            options.model = record.model.clone();
            if let Ok(view) = self.snapshot(owner, &id).await {
                if !matches!(view.view.status.as_str(), "closed" | "error" | "unknown") {
                    return Ok(view);
                }
            }
            if options.resume_id.is_none() {
                return Err("session_not_saved".into());
            }
        } else if options.resume_id.is_some() {
            return Err("session_not_found".into());
        }
        if let Some(project) = &project { options.workspace = project.path.clone(); }
        let workspace = tokio::fs::canonicalize(&options.workspace)
            .await
            .map_err(|_| "invalid_workspace")?;
        if !workspace.is_dir() {
            return Err("invalid_workspace".into());
        }
        options.workspace = workspace.to_string_lossy().into_owned();
        self.workspace_available(&options.workspace, Some(&id)).await?;
        self.host_workspace_available(&options.workspace).await?;
        if existing.is_none() {
            let backends: Vec<_> = self
                .inner
                .active
                .lock()
                .await
                .values()
                .map(|a| a.backend.clone())
                .collect();
            let mut live = 0;
            for backend in backends {
                if backend.snapshot().await.status != "closed" {
                    live += 1;
                }
            }
            if live >= 8 {
                return Err("session_capacity".into());
            }
        }
        // Admission covers the whole session. The permit moves into `Active` once the backend is live.
        let permit = self.inner.gate.admit().map_err(|_| "maintenance_active")?;
        let old = self.inner.active.lock().await.get(&id).map(|entry| entry.backend.clone());
        if let Some(old) = old {
            if !old.close().await {
                return Err("cleanup_pending".into());
            }
            self.inner.active.lock().await.remove(&id);
        }
        let service = self.capabilities();
        let (capability_snapshot, launch) = match (&service, &project, &task_id) {
            (Some(service), Some(project), Some(task)) => {
                let prepared = service.prepare(crate::capabilities::CapabilityPreparation { owner, session: &id, task, project, engine, options: &options, revision: capability_revision, restoring: existing.is_some(), lease: lease.clone() }).await?;
                (Some(prepared.0), prepared.1)
            }
            _ if capability_revision.is_some_and(|r| r > 0) => return Err("capability_unavailable".into()),
            _ => (None, crate::capabilities::CapabilityLaunch::default()),
        };
        if let Some(model) = &launch.model { options.model = Some(model.clone()); }
        let backend = {
            let opening_lease = lease.child_token();
            let opening = async {
                match engine {
                    Engine::Codex => CodexSession::open_with_capabilities(options.clone(), opening_lease.clone(), launch).await.map(Backend::Codex),
                    Engine::Claude => ClaudeSession::open_with_capabilities(options.clone(), opening_lease.clone(), launch).await.map(Backend::Claude),
                }
            };
            tokio::pin!(opening);
            tokio::select! {
                result = &mut opening => match result { Ok(backend) => backend, Err(error) => { if let Some(service) = &service { service.close_session(&id).await; } return Err(error); } },
                _ = self.inner.stop.cancelled() => {
                    opening_lease.cancel();
                    // Keep polling open so its failure path reaps the child before admission is released.
                    if let Ok(backend) = opening.await { self.retire(backend, permit).await; }
                    if let Some(service) = &service { service.close_session(&id).await; }
                    return Err("cancelled".into());
                }
            }
        };
        if lease.is_cancelled() || self.inner.stop.is_cancelled() {
            if let Some(service) = &service { service.close_session(&id).await; }
            self.retire(backend, permit).await;
            return Err("cancelled".into());
        }
        let view = backend.snapshot().await;
        let record = SessionRecord {
            id: id.clone(),
            owner: owner.into(),
            engine,
            native_id: view.native_id.clone(),
            workspace: options.workspace,
            executable: options.executable,
            model: options.model,
            project_id: project.as_ref().map(|p| p.id.clone()),
            task_id,
        };
        if let Err(error) = self.inner.repository.save_session(&record.stored()).await {
            if let Some(service) = &service { service.close_session(&id).await; }
            self.retire(backend, permit).await;
            return Err(error);
        }
        if let Some(snapshot) = &capability_snapshot {
            let encoded = serde_json::to_string(snapshot).map_err(|_| "storage_failed")?;
            if let Err(error) = self.inner.repository.save_session_capabilities(owner, &id, record.project_id.as_deref().ok_or("project_not_found")?, snapshot.revision, snapshot.permission_epoch, &encoded).await {
                if let Some(service) = &service { service.close_session(&id).await; }
                self.retire(backend, permit).await;
                return Err(error);
            }
        }
        let mut records = self.inner.records.lock().await;
        records.retain(|r| r.id != id);
        records.push(record);
        drop(records);
        if lease.is_cancelled() || self.inner.stop.is_cancelled() {
            if let Some(service) = &service { service.close_session(&id).await; }
            self.retire(backend, permit).await;
            return Err("cancelled".into());
        }
        self.hold(backend.clone(), permit);
        self.inner
            .active
            .lock()
            .await
            .insert(id.clone(), Active { backend, lease, generation: Uuid::new_v4() });
        Ok(OwnedSessionView { id, engine, view, capabilities: capability_snapshot })
    }
    pub(crate) async fn workspace_available(&self, workspace: &str, except: Option<&str>) -> Result<(), String> {
        let records = self.inner.records.lock().await.clone();
        let backends: Vec<_> = self.inner.active.lock().await.iter().filter(|(id, _)| Some(id.as_str()) != except).filter_map(|(id, a)| records.iter().find(|r| &r.id == id && r.workspace == workspace).map(|_| a.backend.clone())).collect();
        for backend in backends { if backend.snapshot().await.status != "closed" { return Err("workspace_busy".into()); } }
        Ok(())
    }
    /// Exact actual directory, including foreign occupancy; callers must redact ownership.
    pub(crate) async fn native_workspace_entries(&self, workspace: &str) -> Vec<(SessionRecord, Backend, Uuid)> {
        let records = self.inner.records.lock().await.clone();
        let entries: Vec<_> = self.inner.active.lock().await.iter().filter_map(|(id, active)| {
            records.iter().find(|record| record.id == *id && record.workspace == workspace)
                .map(|record| (record.clone(), active.backend.clone(), active.generation))
        }).collect();
        let mut live = Vec::new();
        for entry in entries {
            if entry.1.snapshot().await.status != "closed" { live.push(entry); }
        }
        live.sort_by(|left, right| left.0.id.cmp(&right.0.id));
        live
    }
    pub(crate) async fn project_available(&self, project_id: &str) -> Result<(), String> {
        if self.inner.host.lock().await.values().any(|p| p.project_id == project_id) { return Err("project_busy".into()); }
        let ids: Vec<_> = self.inner.records.lock().await.iter().filter(|r| r.project_id.as_deref() == Some(project_id)).map(|r| r.id.clone()).collect();
        let backends: Vec<_> = self.inner.active.lock().await.iter().filter(|(id, _)| ids.contains(id)).map(|(_, a)| a.backend.clone()).collect();
        for backend in backends { if backend.snapshot().await.status != "closed" { return Err("project_busy".into()); } }
        Ok(())
    }
    pub(crate) async fn owned(&self, owner: &str, id: &str) -> Result<(Engine, Backend), String> {
        let engine = self
            .inner
            .records
            .lock()
            .await
            .iter()
            .find(|r| r.id == id && r.owner == owner)
            .map(|r| r.engine)
            .ok_or("session_not_found")?;
        let active = self.inner.active.lock().await;
        let entry = active.get(id).ok_or("session_closed")?;
        if entry.lease.is_cancelled() {
            return Err("session_closed".into());
        }
        Ok((engine, entry.backend.clone()))
    }
    pub async fn snapshot(&self, owner: &str, id: &str) -> Result<OwnedSessionView, String> {
        let (engine, backend) = self.owned(owner, id).await?;
        let view = backend.snapshot().await;
        self.owned(owner, id).await?;
        Ok(OwnedSessionView {
            id: id.into(),
            engine,
            view,
            capabilities: match self.capabilities() { Some(service) => service.session_snapshot(owner, id).await?, None => None },
        })
    }
    pub async fn respond(
        &self,
        owner: &str,
        id: &str,
        request_id: String,
        response: Value,
    ) -> Result<(), String> {
        let (_, backend) = self.owned(owner, id).await?;
        match backend {
            Backend::Codex(s) => s.respond(request_id, response).await,
            Backend::Claude(s) => s.respond(request_id, response).await,
        }
    }
    pub async fn interrupt(&self, owner: &str, id: &str) -> Result<(), String> {
        let (_, backend) = self.owned(owner, id).await?;
        if let Some(service) = self.capabilities() { service.end_run(id).await; }
        match backend {
            Backend::Codex(s) => s.interrupt().await,
            Backend::Claude(s) => s.interrupt().await,
        }
    }
    pub async fn close_session(&self, owner: &str, id: &str) -> Result<bool, String> {
        let (_, backend) = self.owned(owner, id).await?;
        if let Some(service) = self.capabilities() { service.close_session(id).await; }
        Ok(backend.close().await)
    }
    /// Same close lifecycle, with authority rechecked after capability cleanup and just
    /// before touching the native process. Caller holds manager admission.
    pub(crate) async fn close_takeover_session(&self, owner: &str, id: &str, project_id: &str, epoch: i64, lease: &CancellationToken) -> Result<bool, String> {
        let (_, backend) = self.owned(owner, id).await?;
        if let Some(service) = self.capabilities() { service.close_session(id).await; }
        if self.inner.repository.capability_epoch(owner, project_id).await? != epoch { return Err("capability_revoked".into()); }
        self.ready().await?;
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        Ok(backend.close().await)
    }
    /// Closes a backend that never entered `active`, then hands its permit to a watcher.
    async fn retire(&self, backend: Backend, permit: ActivityPermit) {
        backend.close().await;
        self.hold(backend, permit);
    }
    /// Keeps the permit until the child has stopped and been reaped. A failed reap parks the
    /// permit in `retained`, so VACUUM stays blocked while the child may still run.
    fn hold(&self, backend: Backend, permit: ActivityPermit) {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            if backend.wait_stopped().await {
                drop(permit);
            } else {
                inner.retained.lock().await.push(permit);
            }
        });
    }
    pub fn close(&self) {
        self.inner.stop.cancel();
    }
    pub async fn cancel_user(&self, owner: &str) -> bool {
        let ids: Vec<_> = self.list(owner).await.into_iter().map(|record| record.id).collect();
        let entries: Vec<_> = {
            let active = self.inner.active.lock().await;
            ids.iter().filter_map(|id| active.get(id).map(|entry| (id.clone(), entry.lease.clone(), entry.backend.clone()))).collect()
        };
        let mut settled = true;
        for (id, lease, backend) in entries {
            if let Some(service) = self.capabilities() { service.close_session(&id).await; }
            lease.cancel();
            if backend.close().await {
                self.inner.active.lock().await.remove(&id);
            } else {
                settled = false;
            }
        }
        settled &= self.stop_host_processes(Some(owner)).await;
        settled
    }
    pub async fn shutdown(&self) -> bool {
        self.close();
        if let Some(service) = self.capabilities() { service.shutdown().await; }
        // A connecting or resuming child is not in active yet. Its cancellation and
        // cleanup must finish before draining active or reporting shutdown success.
        let Ok(_admission) = tokio::time::timeout(
            std::time::Duration::from_secs(8), self.inner.admission.lock(),
        ).await else { return false; };
        let entries: Vec<_> = self
            .inner
            .active
            .lock()
            .await
            .drain()
            .map(|(_, a)| a)
            .collect();
        for entry in &entries {
            entry.lease.cancel();
        }
        let mut ok = true;
        ok &= self.stop_host_processes(None).await;
        for entry in entries {
            ok &= entry.backend.close().await;
        }
        drop(_admission);
        let watchers = std::mem::take(&mut *self.inner.watchers.lock().await);
        for watcher in watchers {
            ok &= matches!(tokio::time::timeout(std::time::Duration::from_secs(15), watcher).await, Ok(Ok(true)));
        }
        ok
    }
}

impl AgentManager {
    /// Caller holds admission. External Git/editors remain outside this host lock.
    pub(crate) async fn repository_git_available(&self, path: &str, common_dir: &str) -> Result<(), String> {
        self.workspace_available(path, None).await?;
        self.host_workspace_available(path).await?;
        let records = self.inner.records.lock().await.clone();
        let active: Vec<_> = self.inner.active.lock().await.iter().filter_map(|(id, active)| {
            records.iter().find(|r| r.id == *id).map(|r| (r.workspace.clone(), active.backend.clone()))
        }).collect();
        let mut candidates = Vec::new();
        for (workspace, backend) in active {
            if matches!(backend.snapshot().await.status.as_str(), "running" | "waiting" | "starting" | "unknown") {
                candidates.push(workspace);
            }
        }
        candidates.extend(self.inner.host.lock().await.values().map(|p| p.workspace.clone()));
        candidates.sort(); candidates.dedup();
        for workspace in candidates {
            let mut command = crate::projects::git_command(std::path::Path::new(&workspace));
            command.args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
            let lease = CancellationToken::new();
            // Failure cannot establish that a live writer belongs to another repository.
            let (bytes, truncated) = crate::projects::git_output(command, std::time::Duration::from_secs(10), &lease, &self.inner.stop).await.map_err(|_| "repository_busy")?;
            if truncated { return Err("repository_busy".into()); }
            let common = std::str::from_utf8(&bytes).map_err(|_| "repository_busy")?.trim();
            let common = tokio::fs::canonicalize(common).await.map_err(|_| "repository_busy")?;
            if common == std::path::Path::new(common_dir) { return Err("repository_busy".into()); }
        }
        Ok(())
    }
}
