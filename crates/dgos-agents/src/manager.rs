use crate::{claude_session::ClaudeSession, codex_session::CodexSession, session::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::{SystemTime, UNIX_EPOCH}};
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
    pub generation: String,
    pub configuration: crate::capabilities::SessionConfigurationView,
    #[serde(flatten)]
    pub view: SessionView,
}
struct Active {
    backend: Backend,
    lease: CancellationToken,
    generation: Uuid,
    provider_lease: Arc<dgos_providers::CliProviderLease>,
    binding: dgos_db::development_repository::OpeningBinding,
    native_receipts: Arc<Mutex<HashMap<String, (crate::native_session::NativeSessionAction, crate::native_session::NativeSessionReceipt)>>>,
    control: Arc<Mutex<()>>,
    native_uncertain: Arc<std::sync::atomic::AtomicBool>,
    slot: Uuid,
}
#[derive(Clone)]
pub struct AgentManager {
    pub(crate) inner: Arc<Inner>,
}
pub(crate) struct Inner {
    records: Mutex<Vec<SessionRecord>>,
    load_error: Mutex<Option<String>>,
    active: Mutex<HashMap<String, Active>>,
    pub(crate) slots: Mutex<HashMap<String, Uuid>>,
    pub(crate) admission: Arc<Mutex<()>>,
    path: PathBuf,
    pub(crate) repository: DevelopmentRepository,
    pub(crate) stop: CancellationToken,
    pub(crate) gate: MaintenanceGate,
    /// Permits whose backend failed to close. Kept so VACUUM stays blocked while the child may still run.
    retained: Mutex<Vec<ActivityPermit>>,
    pub(crate) watchers: Mutex<Vec<tokio::task::JoinHandle<bool>>>,
    cleanup_watchers: std::sync::Mutex<Vec<tokio::task::JoinHandle<bool>>>,
    pub(crate) host: Mutex<HashMap<String, Arc<crate::runtime::HostProcess>>>,
    #[cfg(test)]
    pub(crate) fail_takeover_close: std::sync::atomic::AtomicBool,
    pub(crate) providers: crate::capabilities::CliProviders,
}
impl AgentManager {
    pub fn new(path: PathBuf, repository: DevelopmentRepository, providers: crate::capabilities::CliProviders) -> Self {
        Self::with_maintenance(path, repository, MaintenanceGate::new(), providers)
    }
    /// The process-wide maintenance gate and mandatory Provider service share the host lifetime.
    pub fn with_maintenance(path: PathBuf, repository: DevelopmentRepository, gate: MaintenanceGate, providers: crate::capabilities::CliProviders) -> Self {
        Self {
            inner: Arc::new(Inner {
                records: Mutex::new(Vec::new()),
                active: Mutex::new(HashMap::new()),
                slots: Mutex::new(HashMap::new()),
                admission: Arc::new(Mutex::new(())),
                path,
                repository,
                stop: CancellationToken::new(),
                load_error: Mutex::new(Some("session_index_unavailable".into())),
                gate,
                retained: Mutex::new(Vec::new()),
                watchers: Mutex::new(Vec::new()),
                cleanup_watchers: std::sync::Mutex::new(Vec::new()),
                host: Mutex::new(HashMap::new()),
                #[cfg(test)]
                fail_takeover_close: std::sync::atomic::AtomicBool::new(false),
                providers,
            }),
        }
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
            self.inner.repository.migrate_cli_provider_v1(&dgos_db::development_repository::CutoverContext { boot_identity: self.inner.providers.boot_identity(), owned_exit_sessions: Vec::new() }).await?;
            self.inner.providers.recover().await?;
            self.inner.repository.recover_operations().await?;
            self.inner.repository.recover_runs().await?;
            self.reload_records().await?;
            self.restore_unsettled_slots().await
        }.await;
        *self.inner.load_error.lock().await = result.as_ref().err().cloned();
        result
    }
    pub(crate) async fn ready(&self) -> Result<(), String> {
        if let Some(error) = self.inner.load_error.lock().await.clone() { return Err(error); }
        if self.inner.stop.is_cancelled() { return Err("cancelled".into()); }
        Ok(())
    }
    /// Serializes resource file operations with project archive and relocation.
    /// Revalidate project ownership and directory identity while holding the guard.
    pub async fn admit_native_extensions(&self, lease: &CancellationToken) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        let guard = tokio::select! {
            biased;
            _ = lease.cancelled() => return Err("session_invalid".into()),
            guard = self.inner.admission.clone().lock_owned() => guard,
        };
        self.ready().await?;
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        Ok(guard)
    }
    pub(crate) async fn reload_records(&self) -> Result<(), String> {
        let records = self.inner.repository.sessions().await?.into_iter().map(SessionRecord::from_stored).collect::<Result<Vec<_>, _>>()?;
        *self.inner.records.lock().await = records;
        Ok(())
    }

    async fn restore_unsettled_slots(&self) -> Result<(), String> {
        let records = self.inner.records.lock().await.clone();
        let mut slots = HashMap::new();
        for record in records {
            let configuration = match self.inner.repository.read_cli_session_configuration(&record.owner, &record.id).await {
                Ok(configuration) => configuration,
                Err(error) if error == "session_not_found" => continue,
                Err(error) => return Err(error),
            };
            let settled = configuration.legacy_read_only || configuration.binding.is_some_and(|binding| {
                matches!(binding.state.as_str(), "closed" | "revoked") && matches!(binding.cleanup_state.as_str(), "stopped" | "not_required")
            });
            if !settled { slots.insert(record.id, Uuid::new_v4()); }
        }
        *self.inner.slots.lock().await = slots;
        Ok(())
    }

    pub(crate) async fn slot_count(&self) -> usize { self.inner.slots.lock().await.len() }

    async fn release_slot(&self, id: &str, slot: Uuid) {
        let mut slots = self.inner.slots.lock().await;
        if slots.get(id) == Some(&slot) { slots.remove(id); }
    }

    async fn slot_is_settled(&self, owner: &str, id: &str) -> bool {
        match self.inner.repository.read_cli_session_configuration(owner, id).await {
            Ok(configuration) => configuration.legacy_read_only || configuration.binding.is_some_and(|binding| {
                matches!(binding.state.as_str(), "closed" | "revoked") && matches!(binding.cleanup_state.as_str(), "stopped" | "not_required")
            }),
            Err(error) => error == "session_not_found",
        }
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
    pub async fn open_session(
        &self, owner: &str, engine: Engine, id: String, create: crate::capabilities::SessionCreate,
        task_id: Option<String>, lease: CancellationToken,
    ) -> Result<OwnedSessionView, String> {
        Uuid::parse_str(&id).map_err(|_| "invalid_request_id")?;
        self.ready().await?;
        let _admission = self.inner.admission.lock().await;
        let existing = self.inner.records.lock().await.iter().find(|record| record.id == id).cloned();
        if let Some(record) = existing {
            if record.owner != owner || !self.retryable_no_execution(owner, engine, &id, &record, &create, task_id.as_deref()).await? { return Err("session_exists".into()); }
        } else if create.options.resume_id.is_some() { return Err("session_exists".into()); }
        self.open_bound(owner, engine, id, create, task_id, lease).await
    }

    async fn retryable_no_execution(&self, owner: &str, engine: Engine, id: &str, record: &SessionRecord, create: &crate::capabilities::SessionCreate, task_id: Option<&str>) -> Result<bool, String> {
        let Some(task_id) = task_id else { return Ok(false); };
        let configuration = self.inner.repository.read_cli_session_configuration(owner, id).await?;
        let Some(binding) = configuration.binding.clone() else { return Ok(false); };
        if binding.owner != owner || binding.session_id != id || binding.execution_policy != "provider_bound" || binding.state != "closed" || binding.cleanup_state != "stopped" || binding.cleanup_proof.as_deref() != Some("no_execution") { return Ok(false); }
        let Some(project_id) = binding.project_id.as_deref() else { return Ok(false); };
        let project = self.inner.repository.project(owner, project_id).await?;
        let task = self.inner.repository.task(owner, task_id).await?;
        if task.project_id != project.id || project.archived { return Ok(false); }
        crate::projects::validate_directory(&project).await?;
        let workspace = tokio::fs::canonicalize(&project.path).await.map_err(|_| "invalid_workspace")?.to_string_lossy().into_owned();
        let options = &create.options;
        if record.engine != engine || record.task_id.as_deref() != Some(task_id) || record.project_id.as_deref() != Some(project_id) || record.workspace != workspace || record.executable != options.executable || record.native_id != options.resume_id || options.resume_id.is_some() { return Ok(false); }
        let Some(snapshot_raw) = configuration.snapshot else { return Ok(false); };
        let snapshot: dgos_db::development_repository::CliProviderSnapshot = serde_json::from_str(&snapshot_raw).map_err(|_| "storage_failed")?;
        if binding.task_id.as_deref() != Some(task_id) || binding.revision != Some(create.project_revision) || binding.provider_id.as_deref() != Some(create.provider.provider_id.as_str()) || binding.model_id.as_deref() != Some(create.provider.model_id.as_str()) || binding.provider_revision != Some(create.provider.provider_revision) { return Ok(false); }
        let mut normalized = options.clone();
        normalized.workspace = workspace;
        let digest = crate::capabilities::request_digest(id, engine, task_id, project_id, &normalized, create.project_revision, &create.provider)?;
        Ok(binding.request_digest.as_deref() == Some(digest.as_str()) && snapshot.provider_id == create.provider.provider_id && snapshot.model_id == create.provider.model_id && snapshot.provider_revision == create.provider.provider_revision)
    }

    async fn occupied_slots(&self) -> usize { self.slot_count().await }

    pub async fn resume_session(&self, owner: &str, id: &str, trust_workspace: bool, lease: CancellationToken) -> Result<OwnedSessionView, String> {
        if !trust_workspace { return Err("workspace_untrusted".into()); }
        self.ready().await?;
        let _admission = self.inner.admission.lock().await;
        let record = self.list(owner).await.into_iter().find(|r| r.id == id).ok_or("session_not_found")?;
        let configuration = self.inner.providers.read_session(owner, id).await?;
        let crate::capabilities::SessionConfigurationView::ProviderBound { snapshot, .. } = configuration else { return Err("legacy_session_read_only".into()); };
        let old = self.inner.active.lock().await.get(id).map(|active| (active.backend.clone(), active.binding.clone(), active.slot));
        if let Some((backend, binding, slot)) = old {
            let control = self.session_control(id).await?;
            let _control = control.try_lock().map_err(|_| "session_busy")?;
            let reaped = backend.close().await;
            self.inner.providers.cleanup(owner, id, &binding, reaped, true).await?;
            if !reaped { return Err("cleanup_pending".into()); }
            self.release_slot(id, slot).await;
        }
        self.inner.active.lock().await.remove(id);
        let create = crate::capabilities::SessionCreate {
            options: SessionOptions { workspace: record.workspace, resume_id: Some(record.native_id.ok_or("session_not_saved")?), executable: record.executable, model: record.model, trust_workspace: true },
            project_revision: snapshot.revision,
            provider: crate::capabilities::CliProviderRef { provider_id: snapshot.provider_id, model_id: snapshot.model_id, provider_revision: snapshot.provider_revision },
        };
        self.open_bound(owner, record.engine, id.into(), create, record.task_id, lease).await
    }

    async fn open_bound(&self, owner: &str, engine: Engine, id: String, create: crate::capabilities::SessionCreate, task_id: Option<String>, lease: CancellationToken) -> Result<OwnedSessionView, String> {
        self.validate_handoff_target(owner, &id).await?;
        if self.inner.stop.is_cancelled() || lease.is_cancelled() { return Err("cancelled".into()); }
        let mut options = create.options;
        if !options.trust_workspace { return Err("workspace_untrusted".into()); }
        let task_id = task_id.ok_or("task_not_found")?;
        let project = self.ensure_task_workspace(owner, &task_id).await?;
        if project.archived { return Err("project_archived".into()); }
        crate::projects::validate_directory(&project).await?;
        options.workspace = tokio::fs::canonicalize(&project.path).await.map_err(|_| "invalid_workspace")?.to_string_lossy().into_owned();
        self.workspace_available(&options.workspace, Some(&id)).await?;
        self.host_workspace_available(&options.workspace).await?;
        if self.occupied_slots().await >= 8 { return Err("session_capacity".into()); }
        let permit = self.inner.gate.admit().map_err(|_| "maintenance_active")?;
        let slot = Uuid::new_v4();
        self.inner.slots.lock().await.insert(id.clone(), slot);
        let service = &self.inner.providers;
        let launch = match service.prepare(owner, &id, &task_id, &project, engine, &options, create.project_revision, create.provider, &lease).await {
            Ok(launch) => launch,
            Err(error) => {
                if self.slot_is_settled(owner, &id).await { self.release_slot(&id, slot).await; }
                let _ = self.reload_records().await;
                return Err(error);
            }
        };
        options.model = Some(launch.model.clone());
        let backend = {
            let opening = async {
                match engine {
                    Engine::Codex => CodexSession::open_provider(options.clone(), service.providers(), &launch).await.map(Backend::Codex),
                    Engine::Claude => ClaudeSession::open_provider(options.clone(), service.providers(), &launch).await.map(Backend::Claude),
                }
            };
            tokio::pin!(opening);
            tokio::select! {
                result = &mut opening => result,
                _ = self.inner.stop.cancelled() => { launch.lease.cancel(); opening.await },
            }
        };
        let backend = match backend {
            Ok(backend) => backend,
            Err(failure) => {
                if let Some(child) = failure.child {
                    // A spawned child is potentially executed until its owned exit is confirmed.
                    let reaped = child.stopped();
                    self.hold_failed_open(owner.into(), id.clone(), child, permit, launch.binding.clone(), slot);
                    let cleanup_persisted = service.cleanup(owner, &id, &launch.binding, reaped, true).await.is_ok();
                    if !reaped || !cleanup_persisted {
                        let _ = self.reload_records().await;
                        return Err("cleanup_pending".into());
                    }
                    self.release_slot(&id, slot).await;
                } else {
                    // No transport means spawn never completed; this is the only no_execution path.
                    if service.cleanup(owner, &id, &launch.binding, true, false).await.is_err() { return Err("cleanup_pending".into()); }
                    drop(permit);
                    self.release_slot(&id, slot).await;
                }
                if self.reload_records().await.is_err() { return Err("cleanup_pending".into()); }
                return Err(failure.error);
            }
        };
        let view = backend.snapshot().await;
        let activated = async {
            if lease.is_cancelled() || self.inner.stop.is_cancelled() { return Err("cancelled".into()); }
            service.activate(owner, &id, &launch, view.native_id.as_deref().ok_or("invalid_output")?).await?;
            service.providers().validate_cli_lease(&launch.lease).await.map_err(|_| "provider_revision_changed".to_owned())?;
            self.reload_records().await
        }.await;
        let provider_lease = launch.lease.clone();
        if let Err(error) = activated {
            launch.lease.cancel();
            let reaped = backend.close().await;
            let cleanup_persisted = if reaped {
                let persisted = service.cleanup(owner, &id, &launch.binding, true, true).await.is_ok();
                drop(permit);
                persisted
            } else {
                self.hold_bound(owner.into(), id.clone(), backend, permit, launch.binding.clone(), slot);
                service.cleanup(owner, &id, &launch.binding, false, true).await.is_ok()
            };
            if self.reload_records().await.is_err() || !reaped || !cleanup_persisted { return Err("cleanup_pending".into()); }
            self.release_slot(&id, slot).await;
            return Err(error);
        }
        self.hold_bound(owner.into(), id.clone(), backend.clone(), permit, launch.binding.clone(), slot);
        let generation = Uuid::new_v4();
        self.inner.active.lock().await.insert(id.clone(), Active { backend, lease, generation, provider_lease, binding: launch.binding.clone(), native_receipts: Arc::default(), control: Arc::default(), native_uncertain: Arc::default(), slot });
        Ok(OwnedSessionView { id, engine, generation: generation.to_string(), view, configuration: crate::capabilities::SessionConfigurationView::ProviderBound { snapshot: launch.snapshot, read_only: false } })
    }
    pub(crate) async fn workspace_available(&self, workspace: &str, except: Option<&str>) -> Result<(), String> {
        let records = self.inner.records.lock().await.clone();
        for record in records.iter().filter(|record| record.workspace == workspace) {
            let configuration = match self.inner.repository.read_cli_session_configuration(&record.owner, &record.id).await {
                Ok(configuration) => configuration,
                Err(error) if error == "session_not_found" => continue,
                Err(error) => return Err(error),
            };
            if configuration.binding.is_some_and(|binding| binding.execution_policy == "provider_bound" && binding.cleanup_state == "unknown") {
                return Err("cleanup_pending".into());
            }
        }
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
        let result = self.owned_backend(owner, id).await?;
        let lease = self.inner.active.lock().await.get(id).map(|entry| entry.provider_lease.clone()).ok_or("session_closed")?;
        self.inner.providers.providers().validate_cli_lease(&lease).await.map_err(|_| "provider_revision_changed".to_owned())?;
        self.inner.providers.validate(owner, id).await?;
        Ok(result)
    }
    async fn owned_backend(&self, owner: &str, id: &str) -> Result<(Engine, Backend), String> {
        let engine = self.inner.records.lock().await.iter().find(|r| r.id == id && r.owner == owner).map(|r| r.engine).ok_or("session_not_found")?;
        let active = self.inner.active.lock().await;
        let entry = active.get(id).ok_or("session_closed")?;
        if entry.lease.is_cancelled() { entry.provider_lease.cancel(); return Err("session_closed".into()); }
        Ok((engine, entry.backend.clone()))
    }
    pub async fn snapshot(&self, owner: &str, id: &str) -> Result<OwnedSessionView, String> {
        let (engine, _) = self.owned_backend(owner, id).await?;
        let (backend, generation) = {
            let active = self.inner.active.lock().await;
            let entry = active.get(id).ok_or("session_closed")?;
            (entry.backend.clone(), entry.generation)
        };
        let view = backend.snapshot().await;
        let configuration = self.inner.providers.read_session(owner, id).await?;
        self.owned_backend(owner, id).await?;
        self.require_generation(id, &generation.to_string()).await?;
        Ok(OwnedSessionView {
            id: id.into(),
            engine,
            generation: generation.to_string(),
            view,
            configuration,
        })
    }
    pub(crate) async fn require_generation(&self, id: &str, generation: &str) -> Result<(), String> {
        let expected = Uuid::parse_str(generation).map_err(|_| "stale_generation")?;
        let current = self.inner.active.lock().await.get(id).map(|active| active.generation).ok_or("session_closed")?;
        if current != expected { return Err("stale_generation".into()); }
        Ok(())
    }
    pub async fn respond(
        &self,
        owner: &str,
        id: &str,
        generation: &str,
        request_id: String,
        response: Value,
    ) -> Result<(), String> {
        let admission = self.inner.admission.lock().await;
        self.ready().await?;
        let (_, backend) = self.owned(owner, id).await?;
        self.require_generation(id, generation).await?;
        let record = self.list(owner).await.into_iter().find(|record| record.id == id).ok_or("session_not_found")?;
        let project = self.task_project(owner, record.task_id.as_deref().ok_or("task_not_found")?).await?;
        if project.archived { return Err("project_archived".into()); }
        crate::projects::validate_directory(&project).await?;
        let control = self.session_control(id).await?;
        let _control = control.try_lock().map_err(|_| "session_busy")?;
        drop(admission);
        match backend {
            Backend::Codex(s) => s.respond(request_id, response).await,
            Backend::Claude(s) => s.respond(request_id, response).await,
        }
    }
    pub async fn interrupt(&self, owner: &str, id: &str) -> Result<(), String> {
        let (_, backend) = self.owned(owner, id).await?;
        match backend {
            Backend::Codex(s) => s.interrupt().await,
            Backend::Claude(s) => s.interrupt().await,
        }
    }
    pub async fn close_session(&self, owner: &str, id: &str) -> Result<bool, String> {
        let (_, backend) = self.owned_backend(owner, id).await?;
        let control = self.session_control(id).await?;
        let _control = control.try_lock().map_err(|_| "session_busy")?;
        let (binding, slot) = self.inner.active.lock().await.get(id).map(|entry| (entry.binding.clone(), entry.slot)).ok_or("session_closed")?;
        let reaped = backend.close().await;
        self.inner.providers.cleanup(owner, id, &binding, reaped, true).await?;
        if reaped { self.release_slot(id, slot).await; }
        Ok(reaped)
    }
    /// Same close lifecycle, with authority rechecked after capability cleanup and just
    /// before touching the native process. Caller holds manager admission.
    pub(crate) async fn close_takeover_session(&self, owner: &str, id: &str, project_id: &str, epoch: i64, lease: &CancellationToken) -> Result<bool, String> {
        let (_, backend) = self.owned_backend(owner, id).await?;
        let control = self.session_control(id).await?;
        let _control = control.try_lock().map_err(|_| "session_busy")?;
        if self.inner.repository.capability_epoch(owner, project_id).await? != epoch { return Err("capability_revoked".into()); }
        self.ready().await?;
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        Ok(backend.close().await)
    }
    fn hold_failed_open(&self, owner: String, id: String, child: crate::transport::Transport, permit: ActivityPermit, binding: dgos_db::development_repository::OpeningBinding, slot: Uuid) {
        let inner = self.inner.clone();
        let watcher = tokio::spawn(async move {
            let reaped = child.wait_stopped().await;
            let persisted = inner.providers.cleanup(&owner, &id, &binding, reaped, true).await.is_ok();
            if reaped { drop(permit); } else { inner.retained.lock().await.push(permit); }
            if reaped && persisted {
                let mut slots = inner.slots.lock().await;
                if slots.get(&id) == Some(&slot) { slots.remove(&id); }
            }
            reaped && persisted
        });
        self.inner.cleanup_watchers.lock().unwrap_or_else(|e| e.into_inner()).push(watcher);
    }
    fn hold_bound(&self, owner: String, id: String, backend: Backend, permit: ActivityPermit, binding: dgos_db::development_repository::OpeningBinding, slot: Uuid) {
        let inner = self.inner.clone();
        let watcher = tokio::spawn(async move {
            let reaped = backend.wait_stopped().await;
            let persisted = inner.providers.cleanup(&owner, &id, &binding, reaped, true).await.is_ok();
            if reaped { drop(permit); } else { inner.retained.lock().await.push(permit); }
            if reaped && persisted {
                let mut slots = inner.slots.lock().await;
                if slots.get(&id) == Some(&slot) { slots.remove(&id); }
            }
            reaped && persisted
        });
        self.inner.cleanup_watchers.lock().unwrap_or_else(|e| e.into_inner()).push(watcher);
    }
    pub fn close(&self) {
        self.inner.stop.cancel();
        self.inner.providers.cancel_all();
    }
    pub async fn cancel_user(&self, owner: &str) -> bool {
        let ids: Vec<_> = self.list(owner).await.into_iter().map(|record| record.id).collect();
        let entries: Vec<_> = {
            let active = self.inner.active.lock().await;
            ids.iter().filter_map(|id| active.get(id).map(|entry| (id.clone(), entry.lease.clone(), entry.backend.clone()))).collect()
        };
        let mut settled = true;
        for (id, lease, backend) in entries {
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
        self.inner.providers.cancel_all();
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
        let cleanup = std::mem::take(&mut *self.inner.cleanup_watchers.lock().unwrap_or_else(|e| e.into_inner()));
        for mut watcher in cleanup {
            if let Ok(result) = tokio::time::timeout(std::time::Duration::from_secs(8), &mut watcher).await {
                ok &= matches!(result, Ok(true));
            } else {
                self.inner.cleanup_watchers.lock().unwrap_or_else(|e| e.into_inner()).push(watcher);
                ok = false;
            }
        }
        ok &= self.inner.providers.shutdown().await;
        ok
    }
}

impl Backend {
    fn native_available(&self) -> bool {
        match self { Self::Codex(s) => s.native_available(), Self::Claude(s) => s.native_available() }
    }
    fn native_capabilities(&self) -> crate::native_session::NativeSessionCapabilities {
        match self { Self::Codex(s) => s.native_capabilities(), Self::Claude(s) => s.native_capabilities() }
    }
}
impl AgentManager {
    pub(crate) async fn session_control(&self, id: &str) -> Result<Arc<Mutex<()>>, String> {
        self.inner.active.lock().await.get(id).map(|entry| entry.control.clone()).ok_or_else(|| "session_closed".into())
    }
    pub(crate) async fn native_result_known(&self, id: &str) -> Result<(), String> {
        let active = self.inner.active.lock().await;
        let entry = active.get(id).ok_or("session_closed")?;
        if entry.native_uncertain.load(std::sync::atomic::Ordering::Acquire) { return Err("result_unknown".into()); }
        Ok(())
    }
    async fn native_owned(&self, owner: &str, id: &str, generation: Option<&str>, lease: &CancellationToken) -> Result<(SessionRecord, Backend, Uuid), String> {
        self.ready().await?;
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        let (_, backend) = self.owned(owner, id).await?;
        let record = self.inner.records.lock().await.iter().find(|r| r.id == id && r.owner == owner).cloned().ok_or("session_not_found")?;
        let current = self.inner.active.lock().await.get(id).map(|a| a.generation).ok_or("session_closed")?;
        if generation.is_some_and(|g| g != current.to_string()) { return Err("native_extension_StaleGeneration".into()); }
        let project = self.inner.repository.project(owner, record.project_id.as_deref().ok_or("project_not_found")?).await?;
        if project.archived { return Err("project_archived".into()); }
        crate::projects::validate_directory(&project).await?;
        if !backend.native_available() { return Err("session_closed".into()); }
        if matches!(backend.snapshot().await.status.as_str(), "closed" | "unknown" | "starting") { return Err("session_closed".into()); }
        self.owned(owner, id).await?;
        if lease.is_cancelled() || !backend.native_available() { return Err("session_invalid".into()); }
        Ok((record, backend, current))
    }
    /// Enumerates only active processes; never opens or resumes a native session.
    pub async fn native_session_handles(&self, owner: &str, project_id: &str, lease: &CancellationToken) -> Result<Vec<crate::native_session::NativeSessionHandle>, String> {
        let _admission = self.admit_native_extensions(lease).await?;
        let project = self.inner.repository.project(owner, project_id).await?;
        if project.archived { return Err("project_archived".into()); }
        crate::projects::validate_directory(&project).await?;
        let mut result = Vec::new();
        for record in self.list(owner).await.into_iter().filter(|r| r.project_id.as_deref() == Some(project_id)) {
            if !self.inner.active.lock().await.contains_key(&record.id) { continue; }
            let status = { let active = self.inner.active.lock().await; active.get(&record.id).map(|a| a.backend.clone()) };
            if let Some(backend) = status { if !backend.native_available() || matches!(backend.snapshot().await.status.as_str(), "closed" | "unknown" | "starting") { continue; } }
            let (record, backend, generation) = self.native_owned(owner, &record.id, None, lease).await?;
            if result.len() >= 256 { return Err("native_extension_Limit".into()); }
            result.push(crate::native_session::NativeSessionHandle { session_id: record.id, generation: generation.to_string(), project_id: project_id.into(), engine: record.engine, capabilities: backend.native_capabilities() });
        }
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        Ok(result)
    }
    pub async fn observe_native_session(&self, owner: &str, id: &str, generation: &str, domain: crate::native_session::SessionDomain, lease: &CancellationToken) -> Result<crate::native_session::NativeSessionObservation, String> {
        let _admission = self.admit_native_extensions(lease).await?;
        let (record, backend, _) = self.native_owned(owner, id, Some(generation), lease).await?;
        let control = self.session_control(id).await?;
        let _control = control.try_lock().map_err(|_| "session_busy")?;
        let capabilities = backend.native_capabilities();
        if !match domain { crate::native_session::SessionDomain::Skills => capabilities.observe_skills, crate::native_session::SessionDomain::Mcp => capabilities.observe_mcp } { return Err("native_extension_Unsupported".into()); }
        drop(_admission);
        let read = async { match &backend { Backend::Codex(s) => s.observe_native(domain).await, Backend::Claude(s) => s.observe_native(domain).await } };
        let projection = tokio::select! { biased; _ = lease.cancelled() => return Err("session_invalid".into()), result = read => result.map_err(|_| "native_extension_Unavailable".to_owned()) }?;
        let _admission = self.admit_native_extensions(lease).await?;
        self.native_owned(owner, id, Some(generation), lease).await?;
        Ok(crate::native_session::NativeSessionObservation { session_id: id.into(), generation: generation.into(), engine: record.engine,
            observed_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64, domain, capabilities: backend.native_capabilities(), skills: projection.skills, mcp_servers: projection.mcp_servers, incomplete: projection.incomplete })
    }
    /// Reads the reserved receipt only; never takes the control lock or dispatches IO.
    pub async fn inspect_native_session_request(&self, owner: &str, id: &str, generation: &str, request_id: &str, domain: crate::native_session::SessionDomain, lease: &CancellationToken) -> Result<Option<crate::native_session::NativeSessionReceipt>, String> {
        Uuid::parse_str(request_id).map_err(|_| "invalid_request_id")?;
        let _admission = self.admit_native_extensions(lease).await?;
        self.native_owned(owner, id, Some(generation), lease).await?;
        let receipts = self.inner.active.lock().await.get(id).ok_or("session_closed")?.native_receipts.clone();
        let receipt = receipts.lock().await.get(request_id)
            .filter(|(action, _)| action.domain() == domain)
            .map(|(_, receipt)| receipt.clone());
        self.native_owned(owner, id, Some(generation), lease).await?;
        Ok(receipt)
    }
    pub async fn native_session_action(&self, owner: &str, id: &str, generation: &str, request_id: &str, action: crate::native_session::NativeSessionAction, lease: &CancellationToken) -> Result<crate::native_session::NativeSessionReceipt, String> {
        use crate::native_session::{NativeActionState, NativeSessionAction as Action, NativeSessionReceipt};
        Uuid::parse_str(request_id).map_err(|_| "invalid_request_id")?;
        action.validate()?;
        let _admission = self.admit_native_extensions(lease).await?;
        let (_, backend, _) = self.native_owned(owner, id, Some(generation), lease).await?;
        let receipts = self.inner.active.lock().await.get(id).ok_or("session_closed")?.native_receipts.clone();
        if let Some((previous, receipt)) = receipts.lock().await.get(request_id) {
            return if previous == &action { Ok(receipt.clone()) } else { Err("request_conflict".into()) };
        }
        let control = self.session_control(id).await?;
        let _control = control.try_lock().map_err(|_| "session_busy")?;
        let capabilities = backend.native_capabilities();
        let supported = match &action { Action::ReloadSkills {} => capabilities.reload_skills, Action::ReloadPlugins { .. } => capabilities.reload_plugins,
            Action::ReconnectMcp { .. } => capabilities.reconnect_mcp, Action::ToggleMcp { .. } => capabilities.toggle_mcp,
            Action::NativeLogin { .. } => capabilities.native_login, Action::NativeLogout { .. } => capabilities.native_logout };
        if !supported { return Err("native_extension_Unsupported".into()); }
        self.native_result_known(id).await?;
        if !matches!(backend.snapshot().await.status.as_str(), "idle" | "error") { return Err("session_busy".into()); }
        drop(_admission);
        if let Backend::Claude(session) = &backend {
            tokio::select! { biased; _ = lease.cancelled() => return Err("session_invalid".into()), result = session.admit_native_action(&action) => result }?;
        }
        let _admission = self.admit_native_extensions(lease).await?;
        self.native_owned(owner, id, Some(generation), lease).await?;
        let mut receipt = NativeSessionReceipt { session_id: id.into(), generation: generation.into(), request_id: request_id.into(), state: NativeActionState::Unknown };
        {
            let mut saved = receipts.lock().await;
            if saved.len() >= 256 { return Err("request_capacity".into()); }
            // Reserve unknown BEFORE transport IO. Dropped/cancelled futures cannot replay.
            saved.insert(request_id.into(), (action.clone(), receipt.clone()));
            self.inner.active.lock().await.get(id).ok_or("session_closed")?.native_uncertain.store(true, std::sync::atomic::Ordering::Release);
        }
        drop(_admission);
        let execute = async { match &backend { Backend::Codex(s) => s.native_action(&action).await, Backend::Claude(s) => s.native_action(&action).await } };
        let state = tokio::select! { biased; _ = lease.cancelled() => return Err("session_invalid".into()), result = execute => result };
        let _admission = self.admit_native_extensions(lease).await?;
        self.native_owned(owner, id, Some(generation), lease).await?;
        // Even a native rejection can occur after partial effects. Never replay it.
        receipt.state = state.unwrap_or(NativeActionState::Unknown);
        receipts.lock().await.insert(request_id.into(), (action, receipt.clone()));
        if receipt.state != NativeActionState::Unknown {
            self.inner.active.lock().await.get(id).ok_or("session_closed")?.native_uncertain.store(false, std::sync::atomic::Ordering::Release);
        }
        Ok(receipt)
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
