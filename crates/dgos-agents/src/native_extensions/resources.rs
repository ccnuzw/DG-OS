//! Loose resource directory operations. Resource and visibility transactions are independent.
use super::{dependencies, package::{self, NativeSkillPackage, ResourceFile}, Error, NativeExtensions, OperationId, Owner, Receipt, Scope, State, Target};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use super::{paths::{self, Directory, Identity}, resource_tree::{self, Tree}};
use std::path::PathBuf;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceSource { pub source_hash: String, pub commit: Option<String> }
impl ResourceSource {
    fn validate(&self) -> Result<(), Error> {
        if self.source_hash.len() != 64 || !self.source_hash.bytes().all(|b| b.is_ascii_hexdigit()) || self.commit.as_ref().is_some_and(|c| c.len() != 40 || !c.bytes().all(|b| b.is_ascii_hexdigit())) { return Err(Error::InvalidFormat); }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceInstance { pub id: String, pub revision: String }
#[derive(Clone, Debug, Serialize)]
pub struct ResourceSummary {
    pub instance: ResourceInstance, pub target: Target, pub scope: Scope, pub slug: String, pub owner: Owner,
    pub metadata: package::Metadata, pub resources: Vec<ResourceFile>, pub source: Option<ResourceSource>, pub can_update: bool,
}
/// Packages are already bounded and validated; no renderer path is accepted.
pub enum ResourceAction {
    Install { package: NativeSkillPackage, source: ResourceSource },
    Update { expected: ResourceInstance, package: NativeSkillPackage, source: ResourceSource },
    Remove { expected: ResourceInstance },
}
/// File evidence only: never an identity mapping to registered native commands.
#[derive(Clone, Debug, Serialize)]
pub struct ResourceNameMatch { pub scope: Scope, pub slug: String, pub native_name: String, pub owner: Owner }
#[derive(Clone, Debug, Serialize)]
pub struct ResourceImpact {
    pub native_name: String,
    pub previous_name: Option<String>,
    pub same_name: Vec<ResourceNameMatch>,
    pub coverage: &'static str,
    pub incomplete: bool,
    pub visibility_writable: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ResourcePreview {
    pub receipt: Receipt, pub target: Target, pub scope: Scope, pub slug: String,
    /// Only shown in the explicitly authorized preview, never a general log.
    pub destination: PathBuf, pub before_hash: Option<String>, pub after_hash: Option<String>,
    pub source: Option<ResourceSource>, pub resources: Vec<ResourceFile>, pub metadata: Option<package::Metadata>, pub expires_at: u64,
    pub impact: Option<ResourceImpact>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ResourceDiscovery { pub instances: Vec<ResourceSummary>, pub readonly_entries: Vec<ResourceLink>, pub missing: bool, pub incomplete: bool }

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLinkStatus { WithinAuthorizedRoot, OutsideAuthorizedRoot, Unresolved }
#[derive(Clone, Debug, Serialize)]
pub struct ResourceLink {
    pub id: String,
    pub slug: String,
    pub link_path: String,
    pub target: String,
    pub resolved_target: Option<String>,
    pub status: ResourceLinkStatus,
}
#[cfg(unix)]
#[derive(Serialize, Deserialize)]
struct ResourceProjectBinding { root: PathBuf, identity: Identity }
#[cfg(unix)]
#[derive(Serialize, Deserialize)]
struct ResourceJournal {
    version: u8, id: OperationId, principal: String, lease: String, target: Target, scope: Scope, slug: String,
    #[serde(default)] project: Option<ResourceProjectBinding>,
    anchors: Vec<(PathBuf, Identity)>, parent: Identity, before: Option<Tree>, after: Option<Tree>,
    source: Option<ResourceSource>, expires_at: u64, state: State,
}
#[cfg(unix)]
impl ResourceJournal {
    fn receipt(&self) -> Receipt { Receipt { operation_id: self.id.clone(), state: self.state, pending_activation: self.state == State::Committed, recovery_material_retained: true } }
}
#[cfg(unix)]
fn now() -> Result<u64, Error> { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|v| v.as_secs()).map_err(|_| Error::Unavailable) }
#[cfg(unix)]
pub(super) fn root_path(context: &super::HostContext, target: Target, scope: &Scope) -> Result<PathBuf, Error> {
    let base = match scope { Scope::OsUser => if target == Target::Codex { &context.roots.os_home } else { &context.roots.claude_home }, Scope::ProjectShared => context.roots.project.as_ref().ok_or(Error::AccessDenied)?, Scope::ProjectLocal => return Err(Error::Unsupported) };
    Ok(base.join(match (target, scope) { (Target::Codex, _) => ".agents/skills", (Target::Claude, Scope::OsUser) => "skills", _ => ".claude/skills" }))
}
#[cfg(unix)]
fn normalize_link_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let mut output = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => output.push(component.as_os_str()),
            Component::CurDir => {},
            Component::ParentDir => { if !output.pop() { return None; } },
            Component::Normal(part) => output.push(part),
        }
    }
    Some(output)
}
#[cfg(unix)]
fn readonly_link(root: &Directory, root_path: &std::path::Path, slug: &str) -> Result<ResourceLink, Error> {
    let (target, link_identity) = root.read_link(slug, 4096)?;
    let source = std::path::Path::new(&target);
    let candidate = if source.is_absolute() { normalize_link_path(source) } else { normalize_link_path(&root_path.join(source)) };
    let root_id = root.identity()?;
    let (status, resolved_target) = match candidate {
        Some(path) if path.starts_with(root_path) => {
            let relative = path.strip_prefix(root_path).map_err(|_| Error::UnsafePath)?;
            match Directory::open(&path) {
                Ok(_) => (ResourceLinkStatus::WithinAuthorizedRoot, Some(relative.to_string_lossy().into_owned())),
                Err(_) => (ResourceLinkStatus::Unresolved, None),
            }
        },
        _ => (ResourceLinkStatus::OutsideAuthorizedRoot, None),
    };
    let id = paths::hash(&serde_json::to_vec(&(root_id, slug, link_identity, &target)).map_err(|_| Error::Unavailable)?);
    Ok(ResourceLink { id, slug: slug.to_owned(), link_path: slug.to_owned(), target, resolved_target, status })
}
#[cfg(unix)]
fn tree(parent: &Directory, slug: &str) -> Result<Option<Tree>, Error> {
    match parent.kind(slug)? {
        "missing" => Ok(None),
        "directory" => {
            let child = parent.child(slug.as_ref(), false)?;
            let snapshot = resource_tree::scan(&child)?.0;
            if parent.child(slug.as_ref(), false)?.identity()? != snapshot.identity { return Err(Error::Conflict); }
            Ok(Some(snapshot))
        },
        _ => Err(Error::UnsafePath),
    }
}
#[cfg(unix)]
pub(super) fn owner(root: &Directory, slug: &str) -> Result<Owner, Error> {
    if slug.eq_ignore_ascii_case("synced") { return Ok(Owner::Synced); }
    if ["managed", ".system"].iter().any(|marker| slug.eq_ignore_ascii_case(marker)) { return Ok(Owner::Managed); }
    let (names, incomplete) = root.names(package::ENTRY_LIMIT * 2)?;
    if incomplete { return Err(Error::Limit); }
    for name in names {
        if [".claude-plugin", "plugin.json"].iter().any(|marker| name.eq_ignore_ascii_case(marker)) { return Ok(Owner::Plugin); }
        if ["managed-policy.json", ".managed"].iter().any(|marker| name.eq_ignore_ascii_case(marker)) { return Ok(Owner::Managed); }
    }
    Ok(Owner::Loose)
}
#[cfg(unix)]
fn stage(parent: &Directory, id: &OperationId, create: bool) -> Result<Directory, Error> {
    let recovery = parent.child(".dgos-native-recovery".as_ref(), create)?; recovery.private()?;
    let stage = recovery.child(id.as_str().as_ref(), create)?; stage.private()?; Ok(stage)
}
#[cfg(unix)]
fn save(dir: &Directory, record: &ResourceJournal) -> Result<(), Error> { dir.replace_journal(&format!("resource-{}.json", record.id.as_str()), &serde_json::to_vec(record).map_err(|_| Error::Unavailable)?) }
#[cfg(unix)]
fn validate_tree_snapshot(snapshot: Option<&Tree>) -> Result<(), Error> {
    if let Some(snapshot) = snapshot {
        for path in snapshot.directories.keys().chain(snapshot.files.keys()) { package::loose_path(path)?; }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_payload(stage: &Directory, expected: Option<&Tree>) -> Result<(), Error> {
    let snapshot = tree(stage, "payload")?;
    validate_tree_snapshot(snapshot.as_ref())?;
    if snapshot.as_ref() != expected { return Err(Error::Conflict); }
    Ok(())
}
impl NativeExtensions {
    /// Search reads only host-derived roots; missing roots never create directories.
    pub fn discover_resources(&self, target: Target, scope: Scope, query: &str) -> Result<ResourceDiscovery, Error> {
        #[cfg(unix)] {
            self.validate(false)?;
            if query.len() > 256 { return Err(Error::Limit); }
            let root = match self.resource_root(target, &scope, false) {
                Ok(root) => root,
                Err(Error::NotFound) => return Ok(ResourceDiscovery { instances: Vec::new(), readonly_entries: Vec::new(), missing: true, incomplete: false }),
                Err(e) => return Err(e),
            };
            let identity = root.identity()?;
            let root_path = root_path(&self.context, target, &scope)?;
            let (names, mut incomplete) = root.names(2048)?;
            let mut instances = Vec::new(); let mut readonly_entries = Vec::new(); let query = query.to_lowercase();
            for name in names.into_iter().filter(|n| !n.starts_with('.')) {
                match root.kind(&name)? {
                    "symlink" => {
                        match readonly_link(&root, &root_path, &name) {
                            Ok(entry) => {
                                if query.is_empty() || entry.slug.to_lowercase().contains(&query) || entry.target.to_lowercase().contains(&query) { readonly_entries.push(entry); }
                            },
                            Err(_) => incomplete = true,
                        }
                        continue;
                    },
                    "directory" => {},
                    _ => continue,
                }
                match self.resource_summary(target, scope.clone(), &name) {
                    Ok(summary) => {
                        if query.is_empty() || name.to_lowercase().contains(&query) || summary.metadata.name.to_lowercase().contains(&query) || summary.metadata.description.to_lowercase().contains(&query) { instances.push(summary); }
                    },
                    Err(Error::Unsupported) => { continue; },
                    Err(_) => { incomplete = true; },
                }
            }
            if self.resource_root(target, &scope, false)?.identity()? != identity { return Err(Error::RootChanged); }
            self.validate(false)?;
            Ok(ResourceDiscovery { instances, readonly_entries, missing: false, incomplete })
        }
        #[cfg(not(unix))] { let _ = (target, scope, query); Err(Error::Unsupported) }
    }
    #[cfg(unix)]
    fn resource_root(&self, target: Target, scope: &Scope, create: bool) -> Result<Directory, Error> {
        let path = root_path(&self.context, target, scope)?; self.validate_path(&path)?;
        let dir = if create { Directory::ensure(&path)? } else { Directory::open(&path)? };
        if create { dir.writable_owner()?; }
        self.validate_path(&path)?;
        if Directory::open(&path)?.identity()? != dir.identity()? { return Err(Error::RootChanged); }
        // A host-authorized loose root cannot bypass a plugin marker in its parent.
        let parent = Directory::open(path.parent().ok_or(Error::UnsafePath)?)?;
        if owner(&parent, "")? != Owner::Loose { return Err(Error::Unsupported); }
        Ok(dir)
    }
    #[cfg(unix)]
    fn resource_journals(&self, create: bool) -> Result<Directory, Error> {
        let root_path = &self.context.roots.journal;
        self.validate_path(root_path)?;
        let root = if create { Directory::ensure(root_path)? } else { Directory::open(root_path)? };
        root.private()?;
        let path = root_path.join("resources");
        let dir = if create {
            Directory::ensure(&path)?
        } else {
            match Directory::open(&path) { Ok(dir) => dir, Err(Error::NotFound) => return Ok(root), Err(error) => return Err(error) }
        };
        dir.private()?;
        self.validate_path(root_path)?;
        if Directory::open(root_path)?.identity()? != root.identity()? || Directory::open(&path)?.identity()? != dir.identity()? { return Err(Error::RootChanged); }
        Ok(dir)
    }
    #[cfg(unix)]
    fn resource_load(&self, dir: &Directory, id: &OperationId) -> Result<ResourceJournal, Error> {
        OperationId::parse(id.as_str())?;
        let filename = format!("resource-{}.json", id.as_str());
        let text = match dir.read(&filename)? {
            Some((text, _)) => text,
            None => {
                self.validate_path(&self.context.roots.journal)?;
                let legacy = Directory::open(&self.context.roots.journal)?; legacy.private()?;
                legacy.read(&filename)?.ok_or(Error::NotFound)?.0
            },
        };
        let record: ResourceJournal = serde_json::from_str(&text).map_err(|_| Error::RecoveryRequired)?;
        if record.version != 1 || record.id != *id || record.principal != paths::hash(self.context.principal.as_bytes()) { return Err(Error::NotFound); }
        if record.scope == Scope::ProjectShared {
            let project = self.context.roots.project.as_ref().ok_or(Error::AccessDenied)?;
            let binding = record.project.as_ref().ok_or(Error::AccessDenied)?;
            if binding.root != *project { return Err(Error::AccessDenied); }
            if Directory::open(project)?.identity()? != binding.identity { return Err(Error::RootChanged); }
        } else if record.project.is_some() {
            return Err(Error::AccessDenied);
        }
        package::slug(&record.slug)?;
        for (path, identity) in &record.anchors { if Directory::open(path)?.identity()? != *identity { return Err(Error::RootChanged); } }
        Ok(record)
    }
    #[cfg(unix)]
    fn instance(&self, target: Target, scope: &Scope, slug: &str, root: &Directory, tree: &Tree) -> Result<ResourceInstance, Error> {
        let binding = serde_json::to_vec(&(self.context.principal.as_str(), target, scope, slug, root.identity()?, &tree.identity)).map_err(|_| Error::Unavailable)?;
        Ok(ResourceInstance { id: paths::hash(&binding), revision: paths::hash(&serde_json::to_vec(tree).map_err(|_| Error::Unavailable)?) })
    }
    #[cfg(unix)]
    fn known_source(&self, root: &Directory, target: Target, scope: &Scope, slug: &str, snapshot: &Tree) -> Result<Option<ResourceSource>, Error> {
        let namespace = self.context.roots.journal.join("resources");
        let dir = match Directory::open(&namespace) {
            Ok(dir) => { dir.private()?; Some(dir) },
            Err(Error::NotFound) => None,
            Err(error) => return Err(error),
        };
        if let Some(dir) = dir {
            let (names, incomplete) = dir.names(4096)?; if incomplete { return Err(Error::Limit); }
            let mut candidates = 0usize;
            for name in names.into_iter().filter(|n| n.starts_with("resource-") && n.ends_with(".json")) {
                candidates += 1; if candidates > 4096 { return Err(Error::Limit); }
                let text = dir.read(&name)?.ok_or(Error::Conflict)?.0;
                let Ok(record) = serde_json::from_str::<ResourceJournal>(&text) else { continue; };
                if record.principal == paths::hash(self.context.principal.as_bytes()) && record.state == State::Committed && record.target == target && &record.scope == scope && record.slug == slug && record.parent == root.identity()? && record.after.as_ref() == Some(snapshot) { return Ok(record.source); }
            }
        }
        let (legacy_names, legacy_incomplete) = match Directory::open(&self.context.roots.journal) {
            Ok(legacy) => { legacy.private()?; legacy.names(65_536)? },
            Err(Error::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        if legacy_incomplete { return Err(Error::Limit); }
        let mut legacy_candidates = 0usize;
        let legacy = Directory::open(&self.context.roots.journal)?;
        for name in legacy_names.into_iter().filter(|n| n.starts_with("resource-") && n.ends_with(".json")) {
            legacy_candidates += 1; if legacy_candidates > 4096 { return Err(Error::Limit); }
            let text = legacy.read(&name)?.ok_or(Error::Conflict)?.0;
            let Ok(record) = serde_json::from_str::<ResourceJournal>(&text) else { continue; };
            if record.principal == paths::hash(self.context.principal.as_bytes()) && record.state == State::Committed && record.target == target && &record.scope == scope && record.slug == slug && record.parent == root.identity()? && record.after.as_ref() == Some(snapshot) { return Ok(record.source); }
        }
        Ok(None)
    }
    pub fn resource_summary(&self, target: Target, scope: Scope, slug: &str) -> Result<ResourceSummary, Error> {
        #[cfg(unix)] {
            self.validate(false)?; package::slug(slug)?;
            let root = self.resource_root(target, &scope, false)?;
            let child = root.child(slug.as_ref(), false)?;
            let (snapshot, files) = resource_tree::scan(&child)?;
            let root_owner = owner(&root, "")?;
            let mut owner = if root_owner == Owner::Loose { owner(&child, slug)? } else { root_owner };
            if owner == Owner::Loose && validate_tree_snapshot(Some(&snapshot)).is_err() { owner = Owner::Unknown; }
            let body = files.iter().find(|f| f.path == "SKILL.md").ok_or(Error::InvalidFormat)?;
            let metadata = package::metadata(&body.bytes)?;
            let resources = files.iter().map(|f| ResourceFile { path: f.path.clone(), bytes: f.bytes.len(), executable: f.executable, hash: paths::hash(&f.bytes) }).collect();
            let source = self.known_source(&root, target, &scope, slug, &snapshot)?;
            let instance = self.instance(target, &scope, slug, &root, &snapshot)?;
            if tree(&root, slug)?.as_ref() != Some(&snapshot) { return Err(Error::Conflict); }
            self.validate(false)?; self.validate_path(&root_path(&self.context, target, &scope)?)?;
            Ok(ResourceSummary { instance, target, scope, slug: slug.into(), owner, metadata, resources, can_update: owner == Owner::Loose && source.is_some(), source })
        }
        #[cfg(not(unix))] { let _ = (target, scope, slug); Err(Error::Unsupported) }
    }
    pub fn scan_resource_dependencies(&self, target: Target, scope: Scope, slug: &str, environment: &dependencies::Environment) -> Result<dependencies::Report, Error> {
        let summary = self.resource_summary(target, scope, slug)?;
        self.context.admit(false)?;
        Ok(environment.locate(&summary.metadata.dependencies))
    }
    #[cfg(unix)]
    fn resource_impact(&self, target: Target, native_name: &str, previous_name: Option<String>) -> Option<ResourceImpact> {
        if target != Target::Claude { return None; }
        let mut same_name = Vec::new(); let mut incomplete = false;
        let scopes = [Scope::OsUser].into_iter().chain(self.context.roots.project.as_ref().map(|_| Scope::ProjectShared));
        for scope in scopes {
            match self.discover_resources(target, scope, "") {
                Ok(discovery) => {
                    incomplete |= discovery.incomplete || !discovery.readonly_entries.is_empty();
                    for item in discovery.instances {
                        if item.metadata.name == native_name || previous_name.as_deref() == Some(item.metadata.name.as_str()) {
                            same_name.push(ResourceNameMatch { scope: item.scope, slug: item.slug, native_name: item.metadata.name, owner: item.owner });
                        }
                    }
                },
                Err(_) => incomplete = true,
            }
        }
        Some(ResourceImpact { native_name: native_name.into(), previous_name, same_name, coverage: "authorized_loose_roots", incomplete, visibility_writable: false })
    }
    pub fn preview_resource(&self, target: Target, scope: Scope, slug: &str, action: ResourceAction) -> Result<ResourcePreview, Error> {
        #[cfg(unix)] {
            if !cfg!(target_os = "macos") { return Err(Error::Unsupported); }
            let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?;
            self.validate(true)?; package::slug(slug)?;
            package::loose_path(slug)?;
            let root = self.resource_root(target, &scope, true)?; let _lock = root.lock()?;
            if owner(&root, "")? != Owner::Loose { return Err(Error::Unsupported); }
            let journals = self.resource_journals(true)?; let _journal_lock = journals.lock()?;
            let before = tree(&root, slug)?;
            validate_tree_snapshot(before.as_ref())?;
            if before.is_some() && owner(&root.child(slug.as_ref(), false)?, slug)? != Owner::Loose { return Err(Error::Unsupported); }
            let (package, source) = match action {
                ResourceAction::Install { package, source } => { if before.is_some() { return Err(Error::Conflict); } (Some(package), Some(source)) },
                ResourceAction::Update { expected, package, source } => {
                    let old = before.as_ref().ok_or(Error::NotFound)?;
                    if self.instance(target, &scope, slug, &root, old)? != expected || self.known_source(&root, target, &scope, slug, old)?.is_none() { return Err(Error::Conflict); }
                    (Some(package), Some(source))
                },
                ResourceAction::Remove { expected } => {
                    if self.instance(target, &scope, slug, &root, before.as_ref().ok_or(Error::NotFound)?)? != expected { return Err(Error::Conflict); }
                    (None, None)
                },
            };
            if let Some(source) = &source {
                source.validate()?;
                if package.as_ref().is_some_and(|p| p.content_hash() != source.source_hash) { return Err(Error::Conflict); }
            }
            let id = OperationId::parse(&uuid::Uuid::new_v4().to_string())?;
            let stage = stage(&root, &id, true)?;
            let (after, resources, metadata) = match &package {
                Some(package) => {
                    let payload = stage.child("payload".as_ref(), true)?;
                    let after = resource_tree::write(&payload, package)?;
                    (Some(after), package.summary(), Some(package.metadata.clone()))
                }, None => {
                    // Removal previews describe the exact tree being removed,
                    // including scripts and independent resources, not an empty after image.
                    let child = root.child(slug.as_ref(), false)?;
                    let (snapshot, files) = resource_tree::scan(&child)?;
                    if Some(&snapshot) != before.as_ref() { return Err(Error::Conflict); }
                    let body = files.iter().find(|file| file.path == "SKILL.md").ok_or(Error::InvalidFormat)?;
                    let metadata = package::metadata(&body.bytes)?;
                    let resources = files.iter().map(|file| ResourceFile { path: file.path.clone(), bytes: file.bytes.len(), executable: file.executable, hash: paths::hash(&file.bytes) }).collect();
                    (None, resources, Some(metadata))
                },
            };
            let previous_name = if before.is_some() && after.is_some() && target == Target::Claude {
                let (snapshot, files) = resource_tree::scan(&root.child(slug.as_ref(), false)?)?;
                if Some(&snapshot) != before.as_ref() { return Err(Error::Conflict); }
                let body = files.iter().find(|file| file.path == "SKILL.md").ok_or(Error::InvalidFormat)?;
                let name = package::metadata(&body.bytes)?.name;
                metadata.as_ref().filter(|metadata| metadata.name != name).map(|_| name)
            } else { None };
            let impact = metadata.as_ref().and_then(|metadata| self.resource_impact(target, &metadata.name, previous_name));
            let project = if scope == Scope::ProjectShared {
                let root = self.context.roots.project.as_ref().ok_or(Error::AccessDenied)?;
                Some(ResourceProjectBinding { root: root.clone(), identity: Directory::open(root)?.identity()? })
            } else { None };
            let actual_root = root_path(&self.context, target, &scope)?;
            let mut anchors: Vec<_> = self.anchors.iter().filter(|(p, _)| actual_root.starts_with(p) || self.context.roots.journal.starts_with(p)).cloned().collect();
            for path in [&actual_root, &self.context.roots.journal] {
                if !anchors.iter().any(|(p, _)| p == path) { anchors.push((path.clone(), Directory::open(path)?.identity()?)); }
            }
            let record = ResourceJournal { version: 1, id, principal: paths::hash(self.context.principal.as_bytes()), lease: paths::hash(self.context.lease_id.as_bytes()), target, scope: scope.clone(), slug: slug.into(), project, anchors, parent: root.identity()?, before, after, source: source.clone(), expires_at: now()?.checked_add(600).ok_or(Error::Unavailable)?, state: State::Previewed };
            self.validate(true)?; self.validate_path(&root_path(&self.context, target, &scope)?)?;
            save(&journals, &record)?;
            Ok(ResourcePreview { receipt: record.receipt(), target, scope: scope.clone(), slug: slug.into(), destination: root_path(&self.context, target, &scope)?.join(slug), before_hash: record.before.as_ref().map(|t| t.hash.clone()), after_hash: record.after.as_ref().map(|t| t.hash.clone()), source, resources, metadata, impact, expires_at: record.expires_at })
        }
        #[cfg(not(unix))] { let _ = (target, scope, slug, action); Err(Error::Unsupported) }
    }
    pub fn commit_resource(&self, id: &OperationId) -> Result<Receipt, Error> {
        #[cfg(unix)] {
            if !cfg!(target_os = "macos") { return Err(Error::Unsupported); }
            let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?; self.validate(true)?;
            let journals = self.resource_journals(false)?; let _jlock = journals.lock()?;
            let mut record = self.resource_load(&journals, id)?;
            if record.lease != paths::hash(self.context.lease_id.as_bytes()) { return Err(Error::AccessDenied); }
            package::loose_path(&record.slug)?;
            let root = self.resource_root(record.target, &record.scope, false)?; let _lock = root.lock()?;
            if owner(&root, "")? != Owner::Loose { return Err(Error::Unsupported); }
            if root.identity()? != record.parent { return Err(Error::RootChanged); }
            if !matches!(record.state, State::Previewed | State::Prepared) { return self.inspect_resource_locked(&journals, &root, &mut record); }
            if now()? > record.expires_at { return Err(Error::Expired); }
            let current = tree(&root, &record.slug)?;
            validate_tree_snapshot(current.as_ref())?;
            if current != record.before { record.state = State::Conflict; save(&journals, &record)?; return Ok(record.receipt()); }
            if record.before.is_some() && owner(&root.child(record.slug.as_ref(), false)?, &record.slug)? != Owner::Loose { return Err(Error::Unsupported); }
            let stage = stage(&root, id, false)?;
            validate_payload(&stage, record.after.as_ref())?;
            record.state = State::Prepared; save(&journals, &record)?;
            self.validate(true)?; self.validate_path(&root_path(&self.context, record.target, &record.scope)?)?;
            if self.resource_root(record.target, &record.scope, false)?.identity()? != record.parent || tree(&root, &record.slug)? != record.before { record.state = State::Conflict; save(&journals, &record)?; return Ok(record.receipt()); }
            if owner(&root, "")? != Owner::Loose { return Err(Error::Unsupported); }
            validate_payload(&stage, record.after.as_ref())?;
            record.state = State::Publishing; save(&journals, &record)?;
            let publication = if record.after.is_some() { root.publish(&stage, "payload", &record.slug, record.before.is_some()) } else { root.move_to(&record.slug, &stage, "payload") };
            if publication.is_err() { record.state = State::Unknown; save(&journals, &record)?; }
            self.inspect_resource_locked(&journals, &root, &mut record)
        }
        #[cfg(not(unix))] { let _ = id; Err(Error::Unsupported) }
    }
    pub fn inspect_resource(&self, id: &OperationId) -> Result<Receipt, Error> {
        #[cfg(unix)] {
            let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?;
            self.validate(true)?;
            let journals = self.resource_journals(false)?; let _jlock = journals.lock()?;
            let mut record = self.resource_load(&journals, id)?;
            let root = self.resource_root(record.target, &record.scope, false)?; let _lock = root.lock()?;
            if root.identity()? != record.parent { return Err(Error::RootChanged); }
            self.inspect_resource_locked(&journals, &root, &mut record)
        }
        #[cfg(not(unix))] { let _ = id; Err(Error::Unsupported) }
    }
    #[cfg(unix)]
    fn inspect_resource_locked(&self, journals: &Directory, root: &Directory, record: &mut ResourceJournal) -> Result<Receipt, Error> {
        if matches!(record.state, State::Committed | State::Conflict | State::Cancelled) { return Ok(record.receipt()); }
        let stage = stage(root, &record.id, false)?;
        let observation = (tree(root, &record.slug), tree(&stage, "payload"));
        let observed = match observation { (Ok(current), Ok(payload)) => Some((current, payload)), _ => None };
        if self.resource_root(record.target, &record.scope, false).and_then(|r| r.identity()).ok() != Some(record.parent.clone()) { record.state = State::RecoveryRequired; }
        else if let Some((current, payload)) = observed {
            if current == record.after && payload == record.before {
                root.sync()?; stage.sync()?; record.state = State::Committed;
            } else if current == record.before && payload == record.after {
                if matches!(record.state, State::Publishing | State::Unknown) { record.state = State::Conflict; }
            } else { record.state = State::RecoveryRequired; }
        } else { record.state = State::RecoveryRequired; }
        save(journals, record)?; Ok(record.receipt())
    }
    pub fn cancel_resource(&self, id: &OperationId) -> Result<Receipt, Error> {
        #[cfg(unix)] {
            let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?; self.validate(true)?;
            let dir = self.resource_journals(false)?; let _lock = dir.lock()?;
            let mut record = self.resource_load(&dir, id)?;
            if record.lease != paths::hash(self.context.lease_id.as_bytes()) { return Err(Error::AccessDenied); }
            let root = self.resource_root(record.target, &record.scope, false)?; let _root_lock = root.lock()?;
            if root.identity()? != record.parent { return Err(Error::RootChanged); }
            if matches!(record.state, State::Previewed | State::Prepared) {
                if tree(&root, &record.slug)? != record.before { return Err(Error::Conflict); }
                record.state = State::Cancelled; save(&dir, &record)?;
            }
            Ok(record.receipt())
        }
        #[cfg(not(unix))] { let _ = id; Err(Error::Unsupported) }
    }
    pub fn resource_recovery_material(&self, id: &OperationId) -> Result<super::RecoveryMaterial, Error> {
        #[cfg(unix)] {
            let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?;
            self.validate(true)?; let dir = self.resource_journals(false)?; let _lock = dir.lock()?;
            let record = self.resource_load(&dir, id)?;
            let root = self.resource_root(record.target, &record.scope, false)?;
            let _root_lock = root.lock()?;
            if root.identity()? != record.parent { return Err(Error::RootChanged); }
            let _stage = stage(&root, id, false)?;
            // Return the retained location even when an external writer changed its contents.
            Ok(super::RecoveryMaterial { directory: root_path(&self.context, record.target, &record.scope)?.join(".dgos-native-recovery").join(id.as_str()) })
        }
        #[cfg(not(unix))] { let _ = id; Err(Error::Unsupported) }
    }
}
