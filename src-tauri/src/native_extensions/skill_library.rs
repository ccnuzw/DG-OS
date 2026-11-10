//! Explicit native Skill repository indexing, trusted picker grants and read-only
//! inspection. File publication remains the existing preview/commit transaction.
use super::{dto::{Selection, Source}, fail, Domain, NativeHost, Ticket};
use dgos_agents::{native_extensions as core, AgentManager};
use dgos_db::ActivityPermit;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, HashMap}, path::PathBuf, sync::{Arc, Mutex}, time::{SystemTime, UNIX_EPOCH}};
use tauri_plugin_dialog::DialogExt;

const SOURCE_LIMIT: usize = 64;
const HANDLE_LIMIT: usize = 4;
const GRANT_SECONDS: u64 = 600;
const CACHE_LIMIT: usize = 4 * 1024 * 1024;
fn now() -> Result<u64, String> { SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_secs()).map_err(|_| fail("Unavailable")) }
fn error(error: core::Error) -> String { error.to_string() }
fn valid_id(id: &str) -> Result<(), String> { uuid::Uuid::parse_str(id).map(|_| ()).map_err(|_| fail("InvalidFormat")) }
fn github(input: &str) -> Result<dgos_skills::GithubSource, String> {
    match dgos_skills::parse_source(input).map_err(|_| fail("InvalidFormat"))? {
        dgos_skills::SkillSource::Github(source) => Ok(source), _ => Err(fail("Unsupported")),
    }
}
fn relative(path: &str) -> Result<(), String> {
    if path.is_empty() { return Ok(()); }
    if path.len() > 1024 || path.split('/').count() > core::package::DEPTH_LIMIT || path.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part.len() > 255) || !path.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte)) { return Err(fail("UnsafePath")); }
    Ok(())
}
#[cfg(unix)]
fn directory_identity(path: &std::path::Path) -> Result<(u64, u64), String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| fail("RootChanged"))?;
    if !metadata.is_dir() { return Err(fail("RootChanged")); }
    Ok((metadata.dev(), metadata.ino()))
}
fn validate_repository(repository: &dgos_skills::GithubSource) -> Result<(), String> {
    let input = format!("https://github.com/{}/{}{}", repository.owner, repository.repo, repository.requested_ref.as_ref().map(|reference| format!("/tree/{reference}")).unwrap_or_default());
    let parsed = github(&input)?;
    if parsed.owner != repository.owner || parsed.repo != repository.repo || parsed.requested_ref != repository.requested_ref { return Err(fail("InvalidFormat")); }
    if let Some(path) = &repository.skill_subpath { relative(path)?; }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibrarySkill {
    pub path: String, pub name: String, pub description: String, pub content_hash: String,
    pub file_count: usize, pub script_count: usize, pub executable_files: usize,
    pub dynamic_commands: bool, pub dependencies: core::dependencies::Declarations,
}
impl LibrarySkill {
    fn new(path: String, package: &core::package::NativeSkillPackage) -> Self {
        let (file_count, script_count, executable_files) = package.resource_counts();
        let metadata = package.metadata();
        Self { path, name: metadata.name.clone(), description: metadata.description.clone(), content_hash: package.content_hash().into(), file_count, script_count, executable_files, dynamic_commands: metadata.dynamic_commands, dependencies: metadata.dependencies.clone() }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillSource {
    pub id: String, pub revision: u64, pub repository: dgos_skills::GithubSource,
    pub indexed_at: Option<u64>, pub commit: Option<String>, pub error: Option<String>,
    pub skills: Vec<LibrarySkill>, pub rejected: Vec<(String, String)>,
}
#[derive(Default, Serialize, Deserialize)]
struct Sources { #[serde(default)] version: u8, sources: Vec<SkillSource> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveSource { pub selection: Selection, pub id: Option<String>, pub expected_revision: Option<u64>, pub input: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRequest { pub selection: Selection, pub source_id: String, pub expected_revision: u64 }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchLibrary { pub selection: Selection, pub query: String, pub source_id: Option<String> }
#[derive(Serialize)]
pub struct LibrarySearch { pub sources: Vec<SkillSource>, pub results: Vec<LibraryResult> }
#[derive(Serialize)]
pub struct LibraryResult { pub source_id: String, pub repository_url: String, pub indexed_at: u64, pub commit: String, pub stale: bool, pub skill: LibrarySkill }
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PickerKind { Zip, Directory }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickSource { pub selection: Selection, pub kind: PickerKind }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSource { pub selection: Selection, pub handle: String }
#[derive(Serialize)]
pub struct PickedSource { pub handle: String, pub label: String, pub expires_at: u64, pub skills: Vec<LibrarySkill>, pub rejected: Vec<(String, String)> }
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRequest { pub selection: Selection, pub slug: String, pub expected: core::ResourceInstance }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFile { pub skill: SkillRequest, pub path: String }
#[derive(Serialize)]
pub struct SkillInspection { pub skill: core::ResourceSummary, pub target_path: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewVersion { pub skill: SkillRequest, pub program: String }
#[derive(Clone, Serialize)]
pub struct VersionPlan {
    pub plan_id: String, pub executable: String, pub digest: String, pub argv: Vec<String>,
    pub cwd: String, pub expires_at: u64, pub deadline_seconds: u64,
    pub environment: String, pub requirement: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunVersion { pub selection: Selection, pub plan_id: String, pub approved: bool }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectVersion { pub selection: Selection, pub plan_id: String }
#[derive(Serialize)]
pub struct PreparedUpdate { pub handle: String, pub skill_path: String }

struct PickerGrant {
    ticket: Ticket, selection: Selection, expires_at: u64,
    packages: BTreeMap<String, core::package::NativeSkillPackage>,
    origin: Option<core::ResourceOrigin>, commit: Option<String>,
}
#[cfg(unix)]
struct ProbeGrant { ticket: Ticket, selection: Selection, expires_at: u64, skill: SkillRequest, executable: core::dependencies::VersionExecutable, cwd: PathBuf, cwd_identity: (u64, u64) }
struct CompletedVersion { ticket: Ticket, selection: Selection, result: core::dependencies::VersionResult }
#[derive(Default)]
pub struct LibraryState {
    cache_gate: Arc<Mutex<()>>,
    handles: Mutex<HashMap<String, PickerGrant>>,
    #[cfg(unix)]
    probes: Mutex<HashMap<String, ProbeGrant>>,
    probe_gate: Arc<tokio::sync::Mutex<()>>,
    completed_versions: Arc<Mutex<HashMap<String, CompletedVersion>>>,
    shutdown: tokio_util::sync::CancellationToken,
}

impl LibraryState {
    pub(super) async fn shutdown(&self) -> bool {
        self.shutdown.cancel();
        let Ok(_probe) = tokio::time::timeout(std::time::Duration::from_secs(3), self.probe_gate.lock()).await else { return false; };
        let Ok(mut completed) = self.completed_versions.lock() else { return false; };
        #[cfg(unix)]
        for receipt in completed.values_mut() { core::dependencies::inspect_version_cleanup(&mut receipt.result); }
        completed.values().all(|receipt| receipt.result.cleanup_confirmed)
    }
    #[cfg(unix)]
    fn ensure_version_cleanup(&self, ticket: &Ticket) -> Result<(), String> {
        let mut completed = self.completed_versions.lock().map_err(|_| fail("Unavailable"))?;
        for receipt in completed.values_mut() { core::dependencies::inspect_version_cleanup(&mut receipt.result); }
        completed.retain(|_, receipt| &receipt.ticket == ticket || !receipt.result.cleanup_confirmed);
        if completed.values().any(|receipt| !receipt.result.cleanup_confirmed) { return Err(fail("Busy")); }
        Ok(())
    }
    #[cfg(unix)]
    fn supervise_version<G: Send + 'static>(&self, plan_id: String, grant: ProbeGrant, environment: core::dependencies::Environment, lease: tokio_util::sync::CancellationToken, guards: G) -> tokio::task::JoinHandle<Result<core::dependencies::VersionResult, String>> {
        let completed = self.completed_versions.clone(); let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            let _guards = guards;
            let result = {
                let work = core::dependencies::run_version(&grant.executable, &environment, &grant.cwd, &lease, |pid| {
                    if let Ok(mut completed) = completed.lock() {
                        if let Some(receipt) = completed.get_mut(&plan_id) {
                            receipt.result.status = "unknown".into(); receipt.result.cleanup_confirmed = false; receipt.result.process_group = Some(pid);
                        }
                    }
                });
                tokio::pin!(work);
                tokio::select! { biased; _ = shutdown.cancelled() => { lease.cancel(); work.await }, result = &mut work => result }
            }.map_err(error)?;
            completed.lock().map_err(|_| fail("Unavailable"))?.insert(plan_id, CompletedVersion { ticket: grant.ticket.clone(), selection: grant.selection.clone(), result: result.clone() });
            Ok(result)
        })
    }
}

impl NativeHost {
    pub(super) fn library_lease(&self, ticket: &Ticket) -> Result<tokio_util::sync::CancellationToken, String> {
        let (owner, generation, lease) = self.resolver.native_session()?;
        if owner != ticket.0 || generation != ticket.1 || lease.is_cancelled() { return Err("session_invalid".into()); }
        Ok(lease)
    }
    async fn library_sources<T: Send + 'static>(&self, ticket: &Ticket, permit: Arc<ActivityPermit>, change: impl FnOnce(&mut Sources) -> Result<(T, bool), String> + Send + 'static) -> Result<T, String> {
        let lease = self.library_lease(ticket)?;
        let path = self.roots.journal.join("native-skill-library").join(format!("{:x}", Sha256::digest(ticket.0.as_bytes())));
        let gate = self.skill_library.cache_gate.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit; let _gate = gate.lock().map_err(|_| fail("Unavailable"))?;
            if lease.is_cancelled() { return Err("session_invalid".into()); }
            #[cfg(unix)] {
                let cache = core::package::HostSkillCache::open(&path).map_err(error)?;
                let mut sources: Sources = match cache.read().map_err(error)? { Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| fail("InvalidFormat"))?, None => Sources { version: 1, sources: Vec::new() } };
                if sources.version != 1 || sources.sources.len() > SOURCE_LIMIT { return Err(fail("InvalidFormat")); }
                for source in &sources.sources {
                    valid_id(&source.id)?; validate_repository(&source.repository)?;
                    if source.skills.len() > 128 || source.rejected.len() > 128 || source.revision == 0 || source.commit.as_ref().is_some_and(|commit| commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())) { return Err(fail("InvalidFormat")); }
                    for skill in &source.skills {
                        relative(&skill.path)?;
                        if skill.content_hash.len() != 64 || !skill.content_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) { return Err(fail("InvalidFormat")); }
                    }
                }
                let (value, write) = change(&mut sources)?;
                if lease.is_cancelled() { return Err("session_invalid".into()); }
                if write {
                    let bytes = serde_json::to_vec(&sources).map_err(|_| fail("Unavailable"))?;
                    if bytes.len() > CACHE_LIMIT { return Err(fail("Limit")); }
                    cache.write(&bytes).map_err(error)?;
                }
                Ok(value)
            }
            #[cfg(not(unix))] { let _ = (path, change); Err(fail("Unsupported")) }
        }).await.map_err(|_| fail("Unavailable"))?;
        self.library_lease(ticket)?;
        result
    }
    async fn admit_library(&self, agents: &AgentManager, selection: Selection, manage: bool, permit: Arc<ActivityPermit>, ticket: &Ticket) -> Result<(), String> {
        if selection.scope == core::Scope::ProjectLocal { return Err(fail("Unsupported")); }
        drop(self.admit(agents, selection, Domain::Skills, manage, permit, ticket).await?);
        Ok(())
    }
    pub async fn list_skill_sources(&self, agents: &AgentManager, selection: Selection, permit: ActivityPermit, ticket: Ticket) -> Result<Vec<SkillSource>, String> {
        let permit = Arc::new(permit);
        self.admit_library(agents, selection, false, permit.clone(), &ticket).await?;
        self.library_sources(&ticket, permit, |cache| Ok((cache.sources.clone(), false))).await
    }
    pub async fn save_skill_source(&self, agents: &AgentManager, input: SaveSource, permit: ActivityPermit, ticket: Ticket) -> Result<SkillSource, String> {
        let permit = Arc::new(permit);
        self.admit_library(agents, input.selection, true, permit.clone(), &ticket).await?;
        let repository = github(&input.input)?;
        if input.id.is_none() != input.expected_revision.is_none() { return Err(fail("InvalidFormat")); }
        if let Some(id) = &input.id { valid_id(id)?; }
        self.library_sources(&ticket, permit, move |cache| {
            if cache.sources.iter().any(|source| Some(&source.id) != input.id.as_ref() && source.repository == repository) { return Err(fail("Conflict")); }
            let source = if let Some(id) = input.id {
                let source = cache.sources.iter_mut().find(|source| source.id == id).ok_or_else(|| fail("NotFound"))?;
                if Some(source.revision) != input.expected_revision { return Err(fail("Conflict")); }
                source.revision = source.revision.checked_add(1).ok_or_else(|| fail("Limit"))?;
                if source.repository != repository { source.repository = repository; source.indexed_at = None; source.commit = None; source.skills.clear(); source.rejected.clear(); source.error = None; }
                source.clone()
            } else {
                if cache.sources.len() >= SOURCE_LIMIT { return Err(fail("Limit")); }
                let source = SkillSource { id: uuid::Uuid::new_v4().to_string(), revision: 1, repository, indexed_at: None, commit: None, error: None, skills: Vec::new(), rejected: Vec::new() };
                cache.sources.push(source.clone()); source
            };
            Ok((source, true))
        }).await
    }
    pub async fn remove_skill_source(&self, agents: &AgentManager, input: SourceRequest, permit: ActivityPermit, ticket: Ticket) -> Result<(), String> {
        valid_id(&input.source_id)?;
        let permit = Arc::new(permit);
        self.admit_library(agents, input.selection, true, permit.clone(), &ticket).await?;
        self.library_sources(&ticket, permit, move |cache| {
            let at = cache.sources.iter().position(|source| source.id == input.source_id).ok_or_else(|| fail("NotFound"))?;
            if cache.sources[at].revision != input.expected_revision { return Err(fail("Conflict")); }
            cache.sources.remove(at); Ok(((), true))
        }).await
    }
    async fn fetch_skill_index(&self, repository: dgos_skills::GithubSource, ticket: &Ticket, permit: Arc<ActivityPermit>) -> Result<(String, core::package::PackageIndex), String> {
        let lease = self.library_lease(ticket)?;
        let adapter = dgos_skills::GithubAdapter::new().map_err(|_| fail("Unavailable"))?;
        let mut whole = repository.clone(); whole.skill_subpath = None;
        let resolved = tokio::select! { _ = lease.cancelled() => return Err("session_invalid".into()), value = adapter.fetch(whole) => value.map_err(|_| fail("Unavailable"))? };
        let commit = resolved.resolved_commit.clone();
        let index = tokio::task::spawn_blocking(move || { let _permit = permit; core::package::NativeSkillPackage::index_zip(&resolved.archive).map_err(error) }).await.map_err(|_| fail("Unavailable"))??;
        self.library_lease(ticket)?;
        Ok((commit, index))
    }
    pub async fn refresh_skill_source(&self, agents: &AgentManager, input: SourceRequest, permit: ActivityPermit, ticket: Ticket) -> Result<SkillSource, String> {
        valid_id(&input.source_id)?;
        let permit = Arc::new(permit);
        self.admit_library(agents, input.selection.clone(), true, permit.clone(), &ticket).await?;
        let id = input.source_id.clone();
        let source = self.library_sources(&ticket, permit.clone(), move |cache| {
            let source = cache.sources.iter().find(|source| source.id == id).ok_or_else(|| fail("NotFound"))?;
            if source.revision != input.expected_revision { return Err(fail("Conflict")); }
            Ok((source.clone(), false))
        }).await?;
        let fetched = self.fetch_skill_index(source.repository.clone(), &ticket, permit.clone()).await;
        self.admit_library(agents, input.selection, true, permit.clone(), &ticket).await?;
        self.library_sources(&ticket, permit, move |cache| {
            let current = cache.sources.iter_mut().find(|item| item.id == source.id).ok_or_else(|| fail("NotFound"))?;
            if current.revision != source.revision || current.repository != source.repository { return Err(fail("Conflict")); }
            current.revision = current.revision.checked_add(1).ok_or_else(|| fail("Limit"))?;
            match fetched {
                Ok((commit, index)) => {
                    let prefix = current.repository.skill_subpath.as_deref();
                    current.skills = index.packages.iter().filter(|(path, _)| prefix.is_none_or(|prefix| path.as_str() == prefix || path.starts_with(&format!("{prefix}/")))).map(|(path, package)| LibrarySkill::new(path.clone(), package)).collect();
                    current.rejected = index.rejected;
                    current.commit = Some(commit); current.indexed_at = Some(now()?); current.error = None;
                },
                Err(code) if code == "session_invalid" => return Err(code),
                Err(code) => current.error = Some(code),
            }
            Ok((current.clone(), true))
        }).await
    }
    pub async fn search_skill_library(&self, agents: &AgentManager, input: SearchLibrary, permit: ActivityPermit, ticket: Ticket) -> Result<LibrarySearch, String> {
        if input.query.len() > 256 { return Err(fail("Limit")); }
        if let Some(id) = &input.source_id { valid_id(id)?; }
        let permit = Arc::new(permit);
        self.admit_library(agents, input.selection, false, permit.clone(), &ticket).await?;
        self.library_sources(&ticket, permit, move |cache| {
            let query = input.query.to_lowercase(); let mut results = Vec::new();
            for source in &cache.sources {
                if input.source_id.as_ref().is_some_and(|id| id != &source.id) { continue; }
                let (Some(indexed_at), Some(commit)) = (source.indexed_at, &source.commit) else { continue; };
                for skill in &source.skills {
                    if query.is_empty() || skill.name.to_lowercase().contains(&query) || skill.description.to_lowercase().contains(&query) || skill.path.to_lowercase().contains(&query) {
                        if results.len() >= 2048 { return Err(fail("Limit")); }
                        results.push(LibraryResult { source_id: source.id.clone(), repository_url: source.repository.normalized_source.clone(), indexed_at, commit: commit.clone(), stale: source.error.is_some(), skill: skill.clone() });
                    }
                }
            }
            Ok((LibrarySearch { sources: cache.sources.clone(), results }, false))
        }).await
    }
    fn grant_skill_packages(&self, selection: Selection, ticket: Ticket, index: core::package::PackageIndex, origin: Option<core::ResourceOrigin>, commit: Option<String>, label: String) -> Result<PickedSource, String> {
        self.library_lease(&ticket)?;
        let expires_at = now()? + GRANT_SECONDS;
        let skills = index.packages.iter().map(|(path, package)| LibrarySkill::new(path.clone(), package)).collect();
        let handle = uuid::Uuid::new_v4().to_string();
        let mut handles = self.skill_library.handles.lock().map_err(|_| fail("Unavailable"))?;
        handles.retain(|_, grant| grant.ticket == ticket && grant.expires_at > expires_at - GRANT_SECONDS);
        if handles.len() >= HANDLE_LIMIT { return Err(fail("Limit")); }
        handles.insert(handle.clone(), PickerGrant { ticket, selection, expires_at, packages: index.packages, origin, commit });
        Ok(PickedSource { handle, label, expires_at, skills, rejected: index.rejected })
    }
    pub async fn pick_skill_source(&self, app: &tauri::AppHandle, agents: &AgentManager, input: PickSource, permit: ActivityPermit, ticket: Ticket) -> Result<Option<PickedSource>, String> {
        let permit = Arc::new(permit);
        self.admit_library(agents, input.selection.clone(), true, permit.clone(), &ticket).await?;
        let lease = self.library_lease(&ticket)?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        match input.kind {
            PickerKind::Zip => app.dialog().file().add_filter("Skill ZIP", &["zip"]).pick_file(move |path| { let _ = sender.send(path); }),
            PickerKind::Directory => app.dialog().file().pick_folder(move |path| { let _ = sender.send(path); }),
        }
        let selected = tokio::select! { _ = lease.cancelled() => return Err("session_invalid".into()), selected = receiver => selected.map_err(|_| fail("Unavailable"))? };
        let Some(path) = selected else { return Ok(None); };
        let path = path.into_path().map_err(|_| fail("UnsafePath"))?;
        let label = path.file_name().and_then(|value| value.to_str()).unwrap_or("Skill source").to_owned();
        let activity = permit.clone();
        let index = tokio::task::spawn_blocking(move || {
            let _permit = activity;
            #[cfg(unix)] { match input.kind { PickerKind::Zip => core::package::NativeSkillPackage::index_archive(&path), PickerKind::Directory => core::package::NativeSkillPackage::index_directory(&path) }.map_err(error) }
            #[cfg(not(unix))] { let _ = path; Err(fail("Unsupported")) }
        }).await.map_err(|_| fail("Unavailable"))??;
        self.admit_library(agents, input.selection.clone(), true, permit, &ticket).await?;
        self.grant_skill_packages(input.selection, ticket, index, None, None, label).map(Some)
    }
    pub fn release_skill_source(&self, input: ReleaseSource, permit: ActivityPermit, ticket: Ticket) -> Result<(), String> {
        let _permit = permit; valid_id(&input.handle)?; self.library_lease(&ticket)?;
        let mut handles = self.skill_library.handles.lock().map_err(|_| fail("Unavailable"))?;
        if let Some(grant) = handles.get(&input.handle) {
            if grant.ticket != ticket || grant.selection != input.selection { return Err(fail("AccessDenied")); }
            handles.remove(&input.handle);
        }
        Ok(())
    }
    pub(super) async fn acquire_skill_package(&self, selection: &Selection, source: Source, permit: Arc<ActivityPermit>, ticket: &Ticket) -> Result<(core::package::NativeSkillPackage, core::ResourceSource), String> {
        self.library_lease(ticket)?;
        match source {
            Source::Picker { handle, skill_path } => {
                valid_id(&handle)?; relative(&skill_path)?;
                let mut handles = self.skill_library.handles.lock().map_err(|_| fail("Unavailable"))?;
                let grant = handles.get(&handle).ok_or_else(|| fail("NotFound"))?;
                if &grant.ticket != ticket || &grant.selection != selection { return Err(fail("AccessDenied")); }
                if grant.expires_at <= now()? { return Err(fail("Expired")); }
                if grant.origin.is_some() {
                    if !grant.packages.contains_key(&skill_path) { return Err(fail("NotFound")); }
                    let mut grant = handles.remove(&handle).ok_or_else(|| fail("NotFound"))?;
                    let package = grant.packages.remove(&skill_path).ok_or_else(|| fail("NotFound"))?;
                    let source = core::ResourceSource { source_hash: package.content_hash().into(), commit: grant.commit, origin: grant.origin };
                    return Ok((package, source));
                }
                let package = grant.packages.get(&skill_path).ok_or_else(|| fail("NotFound"))?.clone();
                let source = core::ResourceSource { source_hash: package.content_hash().into(), commit: grant.commit.clone(), origin: grant.origin.clone() };
                Ok((package, source))
            },
            Source::Repository { source_id, skill_path } => {
                valid_id(&source_id)?; relative(&skill_path)?;
                let id = source_id.clone(); let path = skill_path.clone();
                let (repository, pinned_commit, expected_hash) = self.library_sources(ticket, permit.clone(), move |cache| {
                    let source = cache.sources.iter().find(|source| source.id == id).ok_or_else(|| fail("NotFound"))?;
                    let skill = source.skills.iter().find(|skill| skill.path == path).ok_or_else(|| fail("NotFound"))?;
                    Ok(((source.repository.clone(), source.commit.clone().ok_or_else(|| fail("NotFound"))?, skill.content_hash.clone()), false))
                }).await?;
                let mut pinned = repository.clone(); pinned.requested_ref = Some(pinned_commit);
                let (commit, mut index) = self.fetch_skill_index(pinned, ticket, permit).await?;
                let package = index.packages.remove(&skill_path).ok_or_else(|| fail("NotFound"))?;
                if expected_hash != package.content_hash() { return Err(fail("Conflict")); }
                let origin = core::ResourceOrigin { repository, source_id: Some(source_id), skill_path };
                let resource_source = core::ResourceSource { source_hash: package.content_hash().into(), commit: Some(commit), origin: Some(origin) };
                Ok((package, resource_source))
            },
            Source::Github { input } => {
                let repository = github(&input)?;
                let (commit, mut index) = self.fetch_skill_index(repository.clone(), ticket, permit).await?;
                let path = repository.skill_subpath.clone().unwrap_or_default();
                let package = index.packages.remove(&path).ok_or_else(|| fail("InvalidFormat"))?;
                let source = core::ResourceSource { source_hash: package.content_hash().into(), commit: Some(commit), origin: Some(core::ResourceOrigin { repository, source_id: None, skill_path: path }) };
                Ok((package, source))
            },
        }
    }
    pub async fn inspect_skill(&self, agents: &AgentManager, input: SkillRequest, permit: ActivityPermit, ticket: Ticket) -> Result<SkillInspection, String> {
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection.clone(), Domain::Skills, false, permit.clone(), &ticket).await?;
        let target_path = self.roots.skill_path(input.selection.target, admitted.project.as_deref(), &input.slug).parent().ok_or_else(|| fail("UnsafePath"))?.to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let admitted = admitted;
            let skill = admitted.api.resource_summary(input.selection.target, input.selection.scope, &input.slug).map_err(error)?;
            if skill.instance != input.expected { return Err(fail("Conflict")); }
            Ok(SkillInspection { skill, target_path })
        }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn read_skill_file(&self, agents: &AgentManager, input: ReadFile, permit: ActivityPermit, ticket: Ticket) -> Result<core::ResourceContent, String> {
        relative(&input.path)?;
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.skill.selection.clone(), Domain::Skills, false, permit.clone(), &ticket).await?;
        tokio::task::spawn_blocking(move || { let _permit = permit; let admitted = admitted; admitted.api.read_resource_file(input.skill.selection.target, input.skill.selection.scope, &input.skill.slug, &input.skill.expected, &input.path).map_err(error) }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn inspect_skill_dependencies(&self, agents: &AgentManager, input: SkillRequest, permit: ActivityPermit, ticket: Ticket) -> Result<core::dependencies::Report, String> {
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection.clone(), Domain::Skills, false, permit.clone(), &ticket).await?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let admitted = admitted;
            let summary = admitted.api.resource_summary(input.selection.target, input.selection.scope.clone(), &input.slug).map_err(error)?;
            if summary.instance != input.expected { return Err(fail("Conflict")); }
            let environment = core::dependencies::Environment::native_host().map_err(error)?;
            let report = environment.locate(&summary.metadata.dependencies);
            if admitted.api.resource_summary(input.selection.target, input.selection.scope, &input.slug).map_err(error)?.instance != input.expected { return Err(fail("Conflict")); }
            Ok(report)
        }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn prepare_skill_update(&self, agents: &AgentManager, input: SkillRequest, permit: ActivityPermit, ticket: Ticket) -> Result<PreparedUpdate, String> {
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection.clone(), Domain::Skills, true, permit.clone(), &ticket).await?;
        let expected = input.expected.clone(); let slug = input.slug.clone(); let selection = input.selection.clone();
        let activity = permit.clone();
        let origin = tokio::task::spawn_blocking(move || {
            let _permit = activity;
            let admitted = admitted;
            let summary = admitted.api.resource_summary(selection.target, selection.scope, &slug).map_err(error)?;
            if summary.instance != expected { return Err(fail("Conflict")); }
            if summary.owner != core::Owner::Loose { return Err(fail("Unsupported")); }
            summary.source.and_then(|source| source.origin).ok_or_else(|| fail("NotFound"))
        }).await.map_err(|_| fail("Unavailable"))??;
        let (commit, mut index) = self.fetch_skill_index(origin.repository.clone(), &ticket, permit.clone()).await?;
        let package = index.packages.remove(&origin.skill_path).ok_or_else(|| fail("NotFound"))?;
        let mut packages = BTreeMap::new(); packages.insert(origin.skill_path.clone(), package);
        self.admit_library(agents, input.selection.clone(), true, permit, &ticket).await?;
        let skill_path = origin.skill_path.clone();
        let grant = self.grant_skill_packages(input.selection, ticket, core::package::PackageIndex { packages, rejected: Vec::new() }, Some(origin), Some(commit), input.slug)?;
        Ok(PreparedUpdate { handle: grant.handle, skill_path })
    }
    pub async fn preview_skill_version(&self, agents: &AgentManager, input: PreviewVersion, permit: ActivityPermit, ticket: Ticket) -> Result<VersionPlan, String> {
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.skill.selection.clone(), Domain::Skills, true, permit.clone(), &ticket).await?;
        let cwd = admitted.project.clone().unwrap_or_else(|| self.roots.home.clone());
        let skill = input.skill.clone();
        let (executable, requirement) = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let admitted = admitted;
            let summary = admitted.api.resource_summary(input.skill.selection.target, input.skill.selection.scope, &input.skill.slug).map_err(error)?;
            if summary.instance != input.skill.expected { return Err(fail("Conflict")); }
            let report = core::dependencies::Environment::native_host().map_err(error)?.locate(&summary.metadata.dependencies);
            let located = report.tools.into_iter().find(|tool| tool.declaration.program == input.program).ok_or_else(|| fail("NotFound"))?;
            let candidate = located.candidate.ok_or_else(|| fail("NotFound"))?;
            #[cfg(unix)] { Ok((core::dependencies::VersionExecutable::capture(&candidate).map_err(error)?, located.declaration.version_requirement)) }
            #[cfg(not(unix))] { let _ = candidate; Err::<((), Option<String>), String>(fail("Unsupported")) }
        }).await.map_err(|_| fail("Unavailable"))??;
        self.library_lease(&ticket)?;
        #[cfg(unix)] {
            let cwd_identity = directory_identity(&cwd)?;
            let expires_at = now()? + 120; let plan_id = uuid::Uuid::new_v4().to_string();
            let plan = VersionPlan { plan_id: plan_id.clone(), executable: executable.path.to_string_lossy().into_owned(), digest: executable.digest.clone(), argv: vec!["--version".into()], cwd: cwd.to_string_lossy().into_owned(), expires_at, deadline_seconds: 5, environment: "native_host_cli_resolution; no credential forwarding".into(), requirement };
            let mut probes = self.skill_library.probes.lock().map_err(|_| fail("Unavailable"))?;
            probes.retain(|_, grant| grant.ticket == ticket && grant.expires_at > expires_at - 120);
            if probes.len() >= 16 { return Err(fail("Limit")); }
            probes.insert(plan_id, ProbeGrant { ticket, selection: skill.selection.clone(), expires_at, skill, executable, cwd, cwd_identity });
            Ok(plan)
        }
        #[cfg(not(unix))] { let _ = (executable, requirement, cwd, skill); Err(fail("Unsupported")) }
    }
    pub async fn run_skill_version(&self, agents: &AgentManager, input: RunVersion, permit: ActivityPermit, ticket: Ticket) -> Result<core::dependencies::VersionResult, String> {
        valid_id(&input.plan_id)?;
        if !input.approved { return Err(fail("AccessDenied")); }
        let probe_guard = self.skill_library.probe_gate.clone().try_lock_owned().map_err(|_| fail("Busy"))?;
        if self.skill_library.shutdown.is_cancelled() { return Err(fail("Unavailable")); }
        self.library_lease(&ticket)?;
        let permit = Arc::new(permit);
        #[cfg(unix)] {
            self.skill_library.ensure_version_cleanup(&ticket)?;
            if let Some(receipt) = self.skill_library.completed_versions.lock().map_err(|_| fail("Unavailable"))?.get(&input.plan_id) {
                if receipt.ticket != ticket || receipt.selection != input.selection { return Err(fail("AccessDenied")); }
                return Ok(receipt.result.clone());
            }
            let grant = {
                let mut probes = self.skill_library.probes.lock().map_err(|_| fail("Unavailable"))?;
                let grant = probes.get(&input.plan_id).ok_or_else(|| fail("NotFound"))?;
                if grant.ticket != ticket || grant.selection != input.selection { return Err(fail("AccessDenied")); }
                if grant.expires_at <= now()? { return Err(fail("Expired")); }
                probes.remove(&input.plan_id).ok_or_else(|| fail("NotFound"))?
            };
            {
                let mut completed = self.skill_library.completed_versions.lock().map_err(|_| fail("Unavailable"))?;
                if completed.len() >= 64 {
                    let id = completed.iter().find(|(_, receipt)| receipt.result.cleanup_confirmed).map(|(id, _)| id.clone()).ok_or_else(|| fail("Busy"))?;
                    completed.remove(&id);
                }
                completed.insert(input.plan_id.clone(), CompletedVersion { ticket: ticket.clone(), selection: input.selection.clone(), result: core::dependencies::VersionResult { status: "not_started".into(), output: None, exit_code: None, cleanup_confirmed: true, requirement_satisfied: None, process_group: None } });
            }
            let admitted = self.admit(agents, input.selection.clone(), Domain::Skills, true, permit.clone(), &ticket).await?;
            let cwd = admitted.project.clone().unwrap_or_else(|| self.roots.home.clone());
            if cwd != grant.cwd || directory_identity(&cwd)? != grant.cwd_identity { return Err(fail("RootChanged")); }
            let expected = grant.skill.expected.clone(); let slug = grant.skill.slug.clone(); let selection = input.selection.clone();
            let activity = permit.clone();
            let admitted = tokio::task::spawn_blocking(move || {
                let _permit = activity;
                let admitted = admitted;
                if admitted.api.resource_summary(selection.target, selection.scope, &slug).map_err(error)?.instance != expected { return Err(fail("Conflict")); }
                Ok::<_, String>(admitted)
            }).await.map_err(|_| fail("Unavailable"))??;
            let lease = self.library_lease(&ticket)?.child_token();
            let environment = core::dependencies::Environment::native_host().map_err(error)?;
            let result = self.skill_library.supervise_version(input.plan_id.clone(), grant, environment, lease, (probe_guard, permit.clone(), admitted)).await.map_err(|_| fail("Unavailable"))??;
            self.library_lease(&ticket)?;
            self.admit_library(agents, input.selection, true, permit, &ticket).await?;
            Ok(result)
        }
        #[cfg(not(unix))] { let _ = (agents, input, permit, ticket); Err(fail("Unsupported")) }
    }
    pub async fn inspect_skill_version(&self, agents: &AgentManager, input: InspectVersion, permit: ActivityPermit, ticket: Ticket) -> Result<core::dependencies::VersionResult, String> {
        valid_id(&input.plan_id)?;
        self.admit_library(agents, input.selection.clone(), true, Arc::new(permit), &ticket).await?;
        let _probe = self.skill_library.probe_gate.try_lock().map_err(|_| fail("Busy"))?;
        let mut completed = self.skill_library.completed_versions.lock().map_err(|_| fail("Unavailable"))?;
        let Some(receipt) = completed.get_mut(&input.plan_id) else {
            #[cfg(unix)] {
                let probes = self.skill_library.probes.lock().map_err(|_| fail("Unavailable"))?;
                let grant = probes.get(&input.plan_id).ok_or_else(|| fail("NotFound"))?;
                if grant.ticket != ticket || grant.selection != input.selection { return Err(fail("AccessDenied")); }
                return Ok(core::dependencies::VersionResult { status: "not_started".into(), output: None, exit_code: None, cleanup_confirmed: true, requirement_satisfied: None, process_group: None });
            }
            #[cfg(not(unix))] { return Err(fail("Unsupported")); }
        };
        if receipt.ticket != ticket || receipt.selection != input.selection { return Err(fail("AccessDenied")); }
        #[cfg(unix)]
        core::dependencies::inspect_version_cleanup(&mut receipt.result);
        Ok(receipt.result.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_cannot_supply_picker_paths_or_probe_argv() {
        let selection = serde_json::json!({"target":"Codex","scope":"OsUser","project_id":null});
        assert!(serde_json::from_value::<PickSource>(serde_json::json!({"selection":selection,"kind":"directory","path":"/private"})).is_err());
        assert!(serde_json::from_value::<RunVersion>(serde_json::json!({"selection":selection,"plan_id":uuid::Uuid::new_v4(),"approved":true,"argv":["--execute"]})).is_err());
        assert!(serde_json::from_value::<Source>(serde_json::json!({"kind":"zip","bytes":[1,2,3],"subpath":null})).is_err());
    }
    #[test]
    fn repository_grammar_and_relative_paths_fail_closed() {
        for input in ["http://github.com/a/b", "https://evil.example/a/b", "https://user:token@github.com/a/b", "https://github.com/a/b/tree/feature%2Fbranch"] { assert!(github(input).is_err()); }
        for path in ["../escape", "/absolute", "a//b", "a/./b", "a\\b", "a/%2e%2e"] { assert!(relative(path).is_err()); }
        assert!(relative("").is_ok()); assert!(relative("skills/review").is_ok());
    }
    #[tokio::test]
    async fn picker_grants_bind_account_generation_scope_task_and_expiry() {
        let temp = tempfile::tempdir().unwrap();
        let resolver = crate::session::SessionResolver::no_accounts();
        let (owner, generation, _) = resolver.native_session().unwrap();
        let host = NativeHost::from_environment(resolver, temp.path().canonicalize().unwrap()).unwrap();
        let selection = Selection { target: core::Target::Codex, scope: core::Scope::OsUser, project_id: None, task_id: None };
        let ticket = (owner, generation);
        let package = core::package::NativeSkillPackage::from_files(vec![core::package::PackageFile { path: "SKILL.md".into(), bytes: b"---\nname: fixture\n---\nbody".to_vec(), executable: false }]).unwrap();
        let mut packages = BTreeMap::new(); packages.insert("skills/fixture".into(), package);
        let picked = host.grant_skill_packages(selection.clone(), ticket.clone(), core::package::PackageIndex { packages, rejected: Vec::new() }, None, None, "fixture".into()).unwrap();
        let permits = dgos_db::MaintenanceGate::new();
        let source = || Source::Picker { handle: picked.handle.clone(), skill_path: "skills/fixture".into() };
        let (package, _) = host.acquire_skill_package(&selection, source(), Arc::new(permits.admit().unwrap()), &ticket).await.unwrap();
        let mut wrong = selection.clone(); wrong.target = core::Target::Claude;
        assert_eq!(host.acquire_skill_package(&wrong, source(), Arc::new(permits.admit().unwrap()), &ticket).await.err().unwrap(), fail("AccessDenied"));
        wrong = selection.clone(); wrong.task_id = Some("different-worktree".into());
        assert_eq!(host.acquire_skill_package(&wrong, source(), Arc::new(permits.admit().unwrap()), &ticket).await.err().unwrap(), fail("AccessDenied"));
        let foreign = ("another-account".into(), ticket.1);
        assert_eq!(host.acquire_skill_package(&selection, source(), Arc::new(permits.admit().unwrap()), &foreign).await.err().unwrap(), "session_invalid");
        let superseded = (ticket.0.clone(), uuid::Uuid::new_v4());
        assert_eq!(host.acquire_skill_package(&selection, source(), Arc::new(permits.admit().unwrap()), &superseded).await.err().unwrap(), "session_invalid");
        host.skill_library.handles.lock().unwrap().get_mut(&picked.handle).unwrap().expires_at = 0;
        assert_eq!(host.acquire_skill_package(&selection, source(), Arc::new(permits.admit().unwrap()), &ticket).await.err().unwrap(), fail("Expired"));
        assert_eq!(host.release_skill_source(ReleaseSource { selection: wrong, handle: picked.handle.clone() }, permits.admit().unwrap(), ticket.clone()).unwrap_err(), fail("AccessDenied"));
        host.release_skill_source(ReleaseSource { selection: selection.clone(), handle: picked.handle.clone() }, permits.admit().unwrap(), ticket.clone()).unwrap();
        assert_eq!(host.acquire_skill_package(&selection, source(), Arc::new(permits.admit().unwrap()), &ticket).await.err().unwrap(), fail("NotFound"));
        let mut packages = BTreeMap::new(); packages.insert("skills/fixture".into(), package);
        let origin = core::ResourceOrigin { repository: github("https://github.com/example/skills").unwrap(), source_id: None, skill_path: "skills/fixture".into() };
        let update = host.grant_skill_packages(selection.clone(), ticket.clone(), core::package::PackageIndex { packages, rejected: Vec::new() }, Some(origin), Some("a".repeat(40)), "fixture".into()).unwrap();
        let source = || Source::Picker { handle: update.handle.clone(), skill_path: "skills/fixture".into() };
        assert!(host.acquire_skill_package(&selection, source(), Arc::new(permits.admit().unwrap()), &ticket).await.is_ok());
        assert_eq!(host.acquire_skill_package(&selection, source(), Arc::new(permits.admit().unwrap()), &ticket).await.err().unwrap(), fail("NotFound"));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn dropped_dependency_request_remains_supervised_until_shutdown_cleanup() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("version-tool");
        std::fs::write(&path, b"#!/bin/sh\nprintf started > started\n/bin/sleep 30\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let state = LibraryState::default();
        let ticket = ("account".into(), uuid::Uuid::new_v4());
        let selection = Selection { target: core::Target::Codex, scope: core::Scope::OsUser, project_id: None, task_id: None };
        let id = uuid::Uuid::new_v4().to_string();
        let grant = ProbeGrant { ticket: ticket.clone(), selection: selection.clone(), expires_at: now().unwrap() + 120, skill: SkillRequest { selection: selection.clone(), slug: "fixture".into(), expected: core::ResourceInstance { id: "instance".into(), revision: "revision".into() } }, executable: core::dependencies::VersionExecutable::capture(&path).unwrap(), cwd: root.path().to_owned(), cwd_identity: directory_identity(root.path()).unwrap() };
        state.completed_versions.lock().unwrap().insert(id.clone(), CompletedVersion { ticket, selection, result: core::dependencies::VersionResult { status: "not_started".into(), output: None, exit_code: None, cleanup_confirmed: true, requirement_satisfied: None, process_group: None } });
        let guard = state.probe_gate.clone().try_lock_owned().unwrap();
        let worker = state.supervise_version(id.clone(), grant, core::dependencies::Environment::from_host(vec![root.path().to_owned()]).unwrap(), tokio_util::sync::CancellationToken::new(), guard);
        let request = tokio::spawn(async move { worker.await });
        tokio::time::timeout(std::time::Duration::from_secs(2), async { while !root.path().join("started").exists() { tokio::time::sleep(std::time::Duration::from_millis(10)).await; } }).await.unwrap();
        request.abort(); let _ = request.await;
        assert!(state.probe_gate.try_lock().is_err(), "dropping IPC must not release execution ownership");
        assert!(state.shutdown().await);
        let completed = state.completed_versions.lock().unwrap();
        let receipt = &completed[&id].result;
        assert_eq!(receipt.status, "cancelled"); assert!(receipt.cleanup_confirmed);
        assert!(state.probe_gate.try_lock().is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn unknown_dependency_cleanup_blocks_new_accounts_without_exposing_old_results() {
        let state = LibraryState::default();
        let owner = ("previous-account".into(), uuid::Uuid::new_v4());
        let next = ("next-account".into(), uuid::Uuid::new_v4());
        let selection = Selection { target: core::Target::Codex, scope: core::Scope::OsUser, project_id: None, task_id: None };
        state.completed_versions.lock().unwrap().insert("original-plan".into(), CompletedVersion { ticket: owner, selection, result: core::dependencies::VersionResult { status: "unknown".into(), output: None, exit_code: None, cleanup_confirmed: false, requirement_satisfied: None, process_group: None } });
        assert_eq!(state.ensure_version_cleanup(&next).unwrap_err(), fail("Busy"));
        assert!(state.completed_versions.lock().unwrap().contains_key("original-plan"));
        state.completed_versions.lock().unwrap().get_mut("original-plan").unwrap().result.cleanup_confirmed = true;
        state.ensure_version_cleanup(&next).unwrap();
        assert!(state.completed_versions.lock().unwrap().is_empty());
    }
}
