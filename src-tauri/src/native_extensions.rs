//! Native file management. Explicit session controls use AgentManager via separate commands.
pub mod dto;
mod roots;
mod index;
#[cfg(test)]
mod tests;
use dgos_agents::{native_extensions as core, AgentManager};
use dgos_db::ActivityPermit;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::{Arc, Mutex}};
use tokio::sync::OwnedMutexGuard;
use crate::session::SessionResolver;
use dto::*;

pub(crate) fn fail(kind: &str) -> String { format!("native_extension_{kind}") }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Domain { Skills, Mcp }
#[derive(Clone, Copy)]
pub enum OperationAction { Commit, Cancel, Inspect }
pub type Ticket = (String, uuid::Uuid);
pub struct NativeHost {
    resolver: Arc<SessionResolver>,
    roots: roots::Roots,
    contexts: Mutex<HashMap<String, Arc<core::NativeExtensions>>>,
    context_init: tokio::sync::Mutex<()>,
}
struct Admitted {
    api: Arc<core::NativeExtensions>,
    owner: String,
    generation: uuid::Uuid,
    selection: Selection,
    project: Option<PathBuf>,
    _project_lock: Option<OwnedMutexGuard<()>>,
    _project_admission: Option<dgos_agents::NativeProjectAdmission>,
    _permit: Arc<ActivityPermit>,
}
fn validate_selection(selection: &Selection, domain: Domain) -> Result<(), String> {
    match (&selection.scope, &selection.project_id) {
        (Scope::OsUser, None) => {},
        (Scope::ProjectShared | Scope::ProjectLocal, Some(id)) if !id.is_empty() && id.len() <= 128 => {},
        _ => return Err(fail("InvalidFormat")),
    }
    if selection.scope == Scope::ProjectLocal && selection.target == Target::Codex { return Err(fail("Unsupported")); }
    let _ = domain;
    Ok(())
}
impl NativeHost {
    /// Called once with the app-private root. Does not touch native configuration.
    pub fn from_environment(resolver: Arc<SessionResolver>, private_root: PathBuf) -> Result<Self, String> {
        Ok(Self { resolver, roots: roots::Roots::from_environment(private_root).map_err(|e| e.to_string())?, contexts: Mutex::new(HashMap::new()), context_init: tokio::sync::Mutex::new(()) })
    }
    async fn admit(&self, agents: &AgentManager, selection: Selection, domain: Domain, write: bool, permit: Arc<ActivityPermit>, ticket: &Ticket) -> Result<Admitted, String> {
        validate_selection(&selection, domain)?;
        let (owner, generation, lease) = self.resolver.native_session()?;
        if owner != ticket.0 || generation != ticket.1 { return Err("session_invalid".into()); }
        let (project, project_admission) = if let Some(id) = &selection.project_id {
            let admission = agents.admit_native_project(&owner, id, &lease).await.map_err(|error| match error.as_str() {
                "project_not_found" => "project_not_found".to_owned(),
                "project_archived" => "project_archived".to_owned(),
                "workspace_changed" => fail("RootChanged"),
                "session_invalid" => "session_invalid".to_owned(),
                _ => "storage_failed".to_owned(),
            })?;
            let activity = permit.clone();
            let record = admission.project.clone();
            let project = tokio::task::spawn_blocking(move || { let _activity = activity; roots::validate_project(&record) }).await.map_err(|_| fail("Unavailable"))??;
            (Some(project), Some(admission))
        } else { (None, None) };
        let project_lock = if project_admission.is_none() { Some(agents.admit_native_extensions(&lease).await?) } else { None };
        let _init = self.context_init.lock().await;
        let prefix = format!("{generation}:");
        let root_binding = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                project.as_ref().map(|path| {
                    let metadata = std::fs::metadata(path).map_err(|_| fail("RootChanged"))?;
                    Ok::<_, String>((path, metadata.dev(), metadata.ino()))
                }).transpose()?
            }
            #[cfg(not(unix))]
            { project.as_ref().map(|path| path.as_path()) }
        };
        let selection_key = serde_json::to_string(&selection).map_err(|_| fail("InvalidFormat"))?;
        let root_binding = serde_json::to_string(&root_binding).map_err(|_| fail("InvalidFormat"))?;
        let key = format!("{prefix}{write}:{selection_key}:{root_binding}");
        let cached = {
            let mut contexts = self.contexts.lock().map_err(|_| fail("Unavailable"))?;
            let current = self.resolver.native_session()?;
            if current.0 != owner || current.1 != generation || lease.is_cancelled() { return Err("session_invalid".into()); }
            let selection_prefix = format!("{prefix}{write}:{}:", serde_json::to_string(&selection).map_err(|_| fail("InvalidFormat"))?);
            let stale: Vec<_> = contexts.keys().filter(|existing| existing.starts_with(&selection_prefix) && *existing != &key).cloned().collect();
            for old in stale { if let Some(api) = contexts.remove(&old) { let _ = api.revoke(); } }
            contexts.retain(|key, _| key.starts_with(&prefix));
            contexts.get(&key).cloned()
        };
        let api = if let Some(api) = cached { api } else {
            let resolver = self.resolver.clone();
            let roots = self.roots.trusted(project.clone());
            let registration_owner = owner.clone();
            let activity = permit.clone();
            let (api, context) = tokio::task::spawn_blocking(move || {
                let _activity = activity;
                resolver.register_native(&registration_owner, generation, roots, if write { core::Access::Manage } else { core::Access::Read })
            }).await.map_err(|_| fail("Unavailable"))??;
            let api = Arc::new(api);
            let mut contexts = self.contexts.lock().map_err(|_| fail("Unavailable"))?;
            let current = self.resolver.native_session()?;
            if current.0 != owner || current.1 != generation || lease.is_cancelled() {
                if let Ok(current) = self.resolver.native_session() {
                    contexts.retain(|key, _| key.starts_with(&format!("{}:", current.1)));
                }
                let _ = context.revoke();
                return Err("session_invalid".into());
            }
            contexts.retain(|key, _| key.starts_with(&format!("{generation}:")));
            if let Some(existing) = contexts.get(&key).cloned() {
                let _ = context.revoke();
                existing
            } else {
                if contexts.len() >= 4096 {
                    let _ = context.revoke();
                    return Err(fail("Limit"));
                }
                contexts.insert(key, api.clone());
                api
            }
        };
        Ok(Admitted { api, owner, generation, selection, project, _project_lock: project_lock, _project_admission: project_admission, _permit: permit })
    }
    pub async fn discover_skills(&self, agents: &AgentManager, input: DiscoverSkills, permit: ActivityPermit, ticket: Ticket) -> Result<core::ResourceDiscovery, String> {
        if input.scope == Scope::ProjectLocal { return Err(fail("Unsupported")); }
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection(), Domain::Skills, false, permit.clone(), &ticket).await?;
        tokio::task::spawn_blocking(move || { let _permit = permit; let result = admitted.api.discover_resources(admitted.selection.target, admitted.selection.scope.clone(), &input.query).map_err(|e| e.to_string()); drop(admitted); result }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn get_skill(&self, agents: &AgentManager, input: GetSkill, permit: ActivityPermit, ticket: Ticket) -> Result<core::ResourceSummary, String> {
        if input.scope == Scope::ProjectLocal { return Err(fail("Unsupported")); }
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection(), Domain::Skills, false, permit.clone(), &ticket).await?;
        tokio::task::spawn_blocking(move || { let _permit = permit; let result = admitted.api.resource_summary(admitted.selection.target, admitted.selection.scope.clone(), &input.slug).map_err(|e| e.to_string()); drop(admitted); result }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn discover_mcp(&self, agents: &AgentManager, selection: Selection, permit: ActivityPermit, ticket: Ticket) -> Result<core::mcp::Discovery, String> {
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, selection, Domain::Mcp, false, permit.clone(), &ticket).await?;
        tokio::task::spawn_blocking(move || { let _permit = permit; let result = admitted.api.discover_mcp(admitted.selection.target, admitted.selection.scope.clone()).map_err(|e| e.to_string()); drop(admitted); result }).await.map_err(|_| fail("Unavailable"))?
    }
    fn save_binding(root: &std::path::Path, admitted: &Admitted, receipt: &Receipt, domain: Domain, resource: bool) -> Result<(), String> {
        let binding = index::Binding { version: 1, id: receipt.operation_id.as_str().into(), owner: admitted.owner.clone(), generation: admitted.generation.to_string(), domain, selection: admitted.selection.clone(), resource };
        index::Index::open(root, true)?.save(&binding)
    }
    pub async fn preview_mcp(&self, agents: &AgentManager, input: PreviewMcp, permit: ActivityPermit, ticket: Ticket) -> Result<core::Preview, String> {
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection(), Domain::Mcp, true, permit.clone(), &ticket).await?;
        let journal = self.roots.journal.clone();
        tokio::task::spawn_blocking(move || {
            let admitted = admitted;
            let _permit = permit;
            let preview = admitted.api.preview_mcp(admitted.selection.target, admitted.selection.scope.clone(), input.change.map()).map_err(|e| e.to_string())?;
            if Self::save_binding(&journal, &admitted, &preview.receipt, Domain::Mcp, false).is_err() { let _ = admitted.api.cancel(&preview.receipt.operation_id); return Err(fail("RecoveryRequired")); }
            Ok(preview)
        }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn discover_skill_rules(&self, agents: &AgentManager, selection: Selection, permit: ActivityPermit, ticket: Ticket) -> Result<core::skills::Rules, String> {
        if selection.target != Target::Claude { return Err(fail("Unsupported")); }
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, selection, Domain::Skills, false, permit.clone(), &ticket).await?;
        tokio::task::spawn_blocking(move || { let _permit = permit; let result = admitted.api.discover_skill_rules(admitted.selection.target, admitted.selection.scope.clone()).map_err(|e| e.to_string()); drop(admitted); result }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn preview_skill(&self, agents: &AgentManager, input: PreviewSkill, permit: ActivityPermit, ticket: Ticket) -> Result<SkillPreview, String> {
        if input.scope == Scope::ProjectLocal { return Err(fail("Unsupported")); }
        let selection = input.selection();
        let permit = Arc::new(permit);
        if (input.action == SkillAction::Install && input.expected.is_some()) || (input.action != SkillAction::Install && input.expected.is_none()) || (input.action == SkillAction::Remove && input.source.is_some()) || (input.action != SkillAction::Remove && input.source.is_none()) { return Err(fail("InvalidFormat")); }
        // Acquire management authorization before parsing or fetching user-selected content.
        let admitted = self.admit(agents, selection.clone(), Domain::Skills, true, permit.clone(), &ticket).await?;
        let generation = admitted.generation;
        let (current_owner, current_generation, lease) = self.resolver.native_session()?;
        if current_owner != admitted.owner || current_generation != generation || lease.is_cancelled() { return Err("session_invalid".into()); }
        drop(admitted);
        let package = match input.source {
            None => None,
            Some(Source::Zip { bytes, subpath }) => {
                let activity = permit.clone();
                Some(tokio::task::spawn_blocking(move || { let _activity = activity; core::package::NativeSkillPackage::from_zip(&bytes, subpath.as_deref()).map(|p| (p, None)) }).await.map_err(|_| fail("Unavailable"))?.map_err(|e| e.to_string())?)
            },
            Some(Source::Github { input }) => {
                let source = match dgos_skills::parse_source(&input).map_err(|_| fail("InvalidFormat"))? { dgos_skills::SkillSource::Github(source) => source, _ => return Err(fail("Unsupported")) };
                let adapter = dgos_skills::GithubAdapter::new().map_err(|_| fail("Unavailable"))?;
                let resolved = tokio::select! { _ = lease.cancelled() => return Err("session_invalid".into()), result = adapter.fetch(source) => result.map_err(|_| fail("Unavailable"))? };
                let activity = permit.clone();
                Some(tokio::task::spawn_blocking(move || { let _activity = activity; core::package::NativeSkillPackage::from_github(&resolved).map(|p| (p, Some(resolved.resolved_commit.clone()))) }).await.map_err(|_| fail("Unavailable"))?.map_err(|e| e.to_string())?)
            }
        };
        let (current_owner, current_generation, current_lease) = self.resolver.native_session()?;
        if current_owner != ticket.0 || current_generation != generation || current_lease.is_cancelled() || lease.is_cancelled() { return Err("session_invalid".into()); }
        // Re-authorize project/root after acquisition; never hold manager locks over network I/O.
        let admitted = self.admit(agents, selection, Domain::Skills, true, permit.clone(), &ticket).await?;
        if admitted.generation != generation || admitted.owner != current_owner { return Err("session_invalid".into()); }
        let journal = self.roots.journal.clone();
        tokio::task::spawn_blocking(move || {
            let admitted = admitted;
            let _permit = permit;
            let action = match (input.action, input.expected, package) {
                (SkillAction::Install, None, Some((package, commit))) => { let source = core::ResourceSource { source_hash: package.content_hash().into(), commit }; core::ResourceAction::Install { package, source } },
                (SkillAction::Update, Some(expected), Some((package, commit))) => { let source = core::ResourceSource { source_hash: package.content_hash().into(), commit }; core::ResourceAction::Update { expected, package, source } },
                (SkillAction::Remove, Some(expected), None) => core::ResourceAction::Remove { expected },
                _ => return Err(fail("InvalidFormat")),
            };
            let preview = admitted.api.preview_resource(admitted.selection.target, admitted.selection.scope.clone(), &input.slug, action).map_err(|e| e.to_string())?;
            if Self::save_binding(&journal, &admitted, &preview.receipt, Domain::Skills, true).is_err() { let _ = admitted.api.cancel_resource(&preview.receipt.operation_id); return Err(fail("RecoveryRequired")); }
            Ok(SkillPreview { receipt: preview.receipt, target: preview.target, scope: preview.scope, project_id: admitted.selection.project_id.clone(), slug: preview.slug, before_hash: preview.before_hash, after_hash: preview.after_hash, source: preview.source, resources: preview.resources, metadata: preview.metadata, impact: preview.impact, expires_at: preview.expires_at })
        }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn preview_visibility(&self, agents: &AgentManager, input: Visibility, permit: ActivityPermit, ticket: Ticket) -> Result<VisibilityPreview, String> {
        let permit = Arc::new(permit);
        let admitted = self.admit(agents, input.selection(), Domain::Skills, true, permit.clone(), &ticket).await?;
        let journal = self.roots.journal.clone(); let roots = self.roots.clone();
        tokio::task::spawn_blocking(move || {
            let admitted = admitted;
            let _permit = permit;
            let (change, affected, coverage) = match (admitted.selection.target, input.intent) {
                (Target::Codex, VisibilityIntent::CodexPath { instance, visible }) => {
                    let found = admitted.api.discover_resources(Target::Codex, admitted.selection.scope.clone(), "").map_err(|e| e.to_string())?;
                    if found.incomplete { return Err(fail("Limit")); }
                    let selected = found.instances.into_iter().find(|s| s.instance == instance).ok_or_else(|| fail("Conflict"))?;
                    if selected.owner != core::Owner::Loose { return Err(fail("Unsupported")); }
                    let selector = roots.skill_path(Target::Codex, admitted.project.as_deref(), &selected.slug).to_str().ok_or_else(|| fail("UnsafePath"))?.to_owned();
                    (core::skills::VisibilityChange::CodexPath { selector, visible }, vec![VisibilityEffect { selection: admitted.selection.clone(), instance: selected.instance, slug: selected.slug }], "exact_path")
                },
                (Target::Claude, VisibilityIntent::ClaudeName { name, state }) => {
                    // This is a scoped name rule, independent of resource discovery.
                    // Empty observations do not narrow its current/future impact.
                    (core::skills::VisibilityChange::ClaudeName { name, state }, Vec::new(), "current_observations_only")
                },
                _ => return Err(fail("InvalidFormat")),
            };
            let preview = admitted.api.preview_skill_visibility(admitted.selection.target, admitted.selection.scope.clone(), change).map_err(|e| e.to_string())?;
            if Self::save_binding(&journal, &admitted, &preview.receipt, Domain::Skills, false).is_err() { let _ = admitted.api.cancel(&preview.receipt.operation_id); return Err(fail("RecoveryRequired")); }
            let observation_coverage = if admitted.selection.target == Target::Claude { "not_captured" } else { "not_applicable" }.into();
            Ok(VisibilityPreview { rule: preview.rule.clone(), preview, affected, coverage: coverage.into(), observation_coverage })
        }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn operation_domain(&self, id: &str, permit: Arc<ActivityPermit>, ticket: &Ticket) -> Result<Domain, String> {
        let (owner, generation, _) = self.resolver.native_session()?;
        if owner != ticket.0 || generation != ticket.1 { return Err("session_invalid".into()); }
        let journal = self.roots.journal.clone(); let id = id.to_owned();
        tokio::task::spawn_blocking(move || { let _permit = permit; let binding = index::Index::open(&journal, false)?.load(&id)?; if binding.owner != owner { return Err(fail("NotFound")); } Ok(binding.domain) }).await.map_err(|_| fail("Unavailable"))?
    }
    pub async fn operation(&self, agents: &AgentManager, input: Operation, action: OperationAction, domain: Domain, permit: ActivityPermit, ticket: Ticket) -> Result<Receipt, String> {
        let permit = Arc::new(permit);
        let (owner, generation, _) = self.resolver.native_session()?;
        if owner != ticket.0 || generation != ticket.1 { return Err("session_invalid".into()); }
        let journal = self.roots.journal.clone(); let id = input.operation_id;
        let activity = permit.clone();
        let binding = tokio::task::spawn_blocking(move || { let _activity = activity; index::Index::open(&journal, false)?.load(&id) }).await.map_err(|_| fail("Unavailable"))??;
        if binding.owner != owner || binding.domain != domain { return Err(fail("NotFound")); }
        if !matches!(action, OperationAction::Inspect) && binding.generation != generation.to_string() { return Err("session_invalid".into()); }
        // Inspection is a reauthorized read; it may persist a verified receipt but never publishes.
        let admitted = self.admit(agents, binding.selection, domain, true, permit.clone(), &ticket).await?;
        if admitted.owner != owner || admitted.generation != generation { return Err("session_invalid".into()); }
        tokio::task::spawn_blocking(move || {
            let admitted = admitted;
            let _permit = permit;
            let id = core::OperationId::parse(&binding.id).map_err(|e| e.to_string())?;
            let receipt = match (binding.resource, action) {
                (true, OperationAction::Commit) => admitted.api.commit_resource(&id),
                (true, OperationAction::Cancel) => admitted.api.cancel_resource(&id),
                (true, OperationAction::Inspect) => admitted.api.inspect_resource(&id),
                (false, OperationAction::Commit) => admitted.api.commit(&id),
                (false, OperationAction::Cancel) => admitted.api.cancel(&id),
                (false, OperationAction::Inspect) => admitted.api.inspect(&id),
            }.map_err(|e| e.to_string());
            drop(admitted); receipt
        }).await.map_err(|_| fail("Unavailable"))?
    }
}
