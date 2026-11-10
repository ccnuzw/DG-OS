use super::{mcp, skills, Error, HostContext, OperationId, Receipt, Scope, State, Target};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use super::paths::{Directory, Identity, Snapshot};

const TTL_SECONDS: u64 = 600;
fn now() -> Result<u64, Error> { SystemTime::now().duration_since(UNIX_EPOCH).map(|v| v.as_secs()).map_err(|_| Error::Unavailable) }

#[derive(Clone, Debug, Serialize)]
pub struct Preview { pub receipt: Receipt, pub target: Target, pub scope: Scope, pub target_path: String, pub expires_at: u64, pub rule: Option<skills::RuleEvidence> }

/// Construct only after the host authorizes its account and project. Roots are
/// pinned here; all later filesystem opens revalidate their identities.
pub struct NativeExtensions {
    pub(super) context: HostContext,
    #[cfg(unix)]
    pub(super) anchors: Vec<(std::path::PathBuf, Identity)>,
    #[cfg(unix)]
    observed_roots: std::sync::Mutex<std::collections::BTreeMap<std::path::PathBuf, Identity>>,
}
#[cfg(unix)]
#[derive(Serialize, Deserialize)]
struct ProjectBinding {
    native_key: String,
    root: std::path::PathBuf,
    identity: Identity,
}
#[cfg(unix)]
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u8,
    id: OperationId,
    principal_hash: String,
    lease_hash: String,
    target: Target,
    scope: Scope,
    #[serde(default)]
    project: Option<ProjectBinding>,
    visibility: bool,
    #[serde(default)]
    rule: Option<skills::RuleEvidence>,
    filename: String,
    anchors: Vec<(std::path::PathBuf, Identity)>,
    parent: Identity,
    before: Option<Snapshot>,
    after: Snapshot,
    expires_at: u64,
    state: State,
    retained: bool,
}
#[cfg(unix)]
impl Journal {
    fn receipt(&self) -> Receipt {
        Receipt { operation_id: self.id.clone(), state: self.state, pending_activation: self.state == State::Committed, recovery_material_retained: self.retained }
    }
}
impl NativeExtensions {
    /// Invalidates outstanding previews when the host retires this root binding.
    pub fn revoke(&self) -> Result<(), Error> { self.context.revoke() }
    pub fn new(context: HostContext) -> Result<Self, Error> {
        context.admit(false)?;
        #[cfg(unix)] {
            let mut anchors = Vec::new();
            for path in [&context.roots.os_home].into_iter().chain(context.roots.project.iter()) {
                anchors.push((path.clone(), Directory::open(path)?.identity()?));
            }
            // Optional target roots must not prevent another engine being read.
            // Remember existing identities, but check only the root being used.
            for path in [&context.roots.codex_home, &context.roots.claude_home, &context.roots.journal] {
                if let Ok((base, dir)) = Directory::nearest(path) {
                    if !anchors.iter().any(|(p, _)| p == &base) { anchors.push((base, dir.identity()?)); }
                }
            }
            if let Some(path) = context.roots.claude_user_file.parent() {
                if let Ok((base, dir)) = Directory::nearest(path) {
                    if !anchors.iter().any(|(p, _)| p == &base) { anchors.push((base, dir.identity()?)); }
                }
            }
            Ok(Self { context, anchors, observed_roots: std::sync::Mutex::new(std::collections::BTreeMap::new()) })
        }
        #[cfg(not(unix))] { let _ = context; Err(Error::Unsupported) }
    }
    #[cfg(unix)]
    pub(super) fn validate(&self, write: bool) -> Result<(), Error> {
        self.context.admit(write)?;
        for (path, identity) in &self.anchors {
            if (path == &self.context.roots.os_home || self.context.roots.project.as_ref() == Some(path)) && Directory::open(path)?.identity()? != *identity { return Err(Error::RootChanged); }
        }
        Ok(())
    }
    #[cfg(unix)]
    pub(super) fn validate_path(&self, path: &std::path::Path) -> Result<(), Error> {
        self.validate(false)?;
        for (anchor, identity) in &self.anchors {
            if path.starts_with(anchor) && Directory::open(anchor)?.identity()? != *identity { return Err(Error::RootChanged); }
        }
        let mut observed = self.observed_roots.lock().map_err(|_| Error::Unavailable)?;
        for (root, identity) in observed.iter() {
            if path.starts_with(root) && Directory::open(root)?.identity()? != *identity { return Err(Error::RootChanged); }
        }
        match Directory::open(path) {
            Ok(dir) => { observed.entry(path.to_path_buf()).or_insert(dir.identity()?); },
            Err(Error::NotFound) => {},
            Err(error) => return Err(error),
        }
        Ok(())
    }
    #[cfg(unix)]
    fn operation_anchors(&self, target_path: &std::path::Path) -> Result<Vec<(std::path::PathBuf, Identity)>, Error> {
        let journal = &self.context.roots.journal;
        let mut anchors: Vec<_> = self.anchors.iter().filter(|(p, _)| target_path.starts_with(p) || journal.starts_with(p)).cloned().collect();
        for path in [target_path, journal.as_path()] {
            if !anchors.iter().any(|(p, _)| p == path) { anchors.push((path.to_path_buf(), Directory::open(path)?.identity()?)); }
        }
        Ok(anchors)
    }
    #[cfg(unix)]
    fn project_binding(&self, scope: &Scope) -> Result<Option<ProjectBinding>, Error> {
        if *scope == Scope::OsUser { return Ok(None); }
        let root = self.context.roots.project.as_ref().ok_or(Error::AccessDenied)?;
        Ok(Some(ProjectBinding { native_key: self.context.project_key()?, root: root.clone(), identity: Directory::open(root)?.identity()? }))
    }
    #[cfg(unix)]
    fn validate_project(&self, journal: &Journal) -> Result<(), Error> {
        if journal.scope == Scope::OsUser { return if journal.project.is_none() { Ok(()) } else { Err(Error::AccessDenied) }; }
        let binding = journal.project.as_ref().ok_or(Error::AccessDenied)?;
        let root = self.context.roots.project.as_ref().ok_or(Error::AccessDenied)?;
        if root != &binding.root || self.context.project_key()? != binding.native_key { return Err(Error::AccessDenied); }
        if Directory::open(root)?.identity()? != binding.identity { return Err(Error::RootChanged); }
        Ok(())
    }
    fn target_path(&self, target: Target, scope: &Scope, visibility: bool) -> Result<std::path::PathBuf, Error> {
        if visibility && target == Target::Claude {
            match scope {
                Scope::OsUser => Ok(self.context.roots.claude_home.join("settings.json")),
                Scope::ProjectShared => Ok(self.context.roots.project.as_ref().ok_or(Error::AccessDenied)?.join(".claude/settings.json")),
                Scope::ProjectLocal => Ok(self.context.roots.project.as_ref().ok_or(Error::AccessDenied)?.join(".claude/settings.local.json")),
            }
        } else { self.context.config_path(target, scope) }
    }
    #[cfg(unix)]
    fn parent(&self, target: Target, scope: &Scope, visibility: bool, create: bool) -> Result<(Directory, String), Error> {
        let path = self.target_path(target, scope, visibility)?;
        let parent_path = path.parent().ok_or(Error::UnsafePath)?;
        self.validate_path(parent_path)?;
        let parent = match Directory::open(parent_path) {
            Ok(dir) => dir,
            Err(Error::NotFound) if create => Directory::ensure(parent_path)?,
            Err(error) => return Err(error),
        };
        if create { parent.writable_owner()?; }
        self.validate_path(parent_path)?;
        let filename = path.file_name().and_then(|p| p.to_str()).ok_or(Error::UnsafePath)?.to_owned();
        Ok((parent, filename))
    }
    #[cfg(unix)]
    fn journal_dir(&self) -> Result<Directory, Error> {
        self.validate_path(&self.context.roots.journal)?;
        let dir = Directory::open(&self.context.roots.journal)?; dir.private()?; Ok(dir)
    }
    #[cfg(unix)]
    fn stage(parent: &Directory, id: &OperationId, create: bool) -> Result<Directory, Error> {
        let recovery = parent.child(std::ffi::OsStr::new(".dgos-native-recovery"), create)?;
        recovery.private()?;
        let operation = recovery.child(std::ffi::OsStr::new(id.as_str()), create)?;
        operation.private()?;
        Ok(operation)
    }
    #[cfg(unix)]
    fn load(&self, dir: &Directory, id: &OperationId) -> Result<Journal, Error> {
        OperationId::parse(id.as_str())?;
        let (text, _) = dir.read(&format!("{}.json", id.as_str()))?.ok_or(Error::NotFound)?;
        let journal: Journal = serde_json::from_str(&text).map_err(|_| Error::RecoveryRequired)?;
        if journal.version != 1 || journal.id != *id || journal.principal_hash != super::paths::hash(self.context.principal.as_bytes()) { return Err(Error::NotFound); }
        self.validate_project(&journal)?;
        for (path, identity) in &journal.anchors { if Directory::open(path)?.identity()? != *identity { return Err(Error::RootChanged); } }
        Ok(journal)
    }
    #[cfg(unix)]
    fn save(dir: &Directory, journal: &Journal) -> Result<(), Error> {
        let bytes = serde_json::to_vec(journal).map_err(|_| Error::Unavailable)?;
        dir.replace_journal(&format!("{}.json", journal.id.as_str()), &bytes)
    }
    /// Reads only the safe native MCP projection. It does not health-check.
    pub fn discover_mcp(&self, target: Target, scope: Scope) -> Result<mcp::Discovery, Error> {
        #[cfg(unix)] {
            self.validate(false)?;
            let (parent, filename) = match self.parent(target, &scope, false, false) {
                Err(Error::NotFound) => return Ok(mcp::Discovery { instances: Vec::new(), incomplete: false }),
                other => other?,
            };
            let original = parent.read(&filename)?;
            if original.is_none() { return Ok(mcp::Discovery { instances: Vec::new(), incomplete: false }); }
            if target == Target::Claude && original.as_ref().is_some_and(|v| v.0.is_empty()) { return Err(Error::InvalidFormat); }
            let text = original.as_ref().map_or("", |v| v.0.as_str());
            let found = mcp::discover(text, target, &scope, self.context.roots.project.as_ref().and_then(|p| p.to_str()))?;
            self.validate(false)?;
            Ok(found)
        }
        #[cfg(not(unix))] { let _ = (target, scope); Err(Error::Unsupported) }
    }
    /// Reads only rules explicitly stored in the selected Claude settings layer.
    pub fn discover_skill_rules(&self, target: Target, scope: Scope) -> Result<skills::Rules, Error> {
        if target != Target::Claude { return Err(Error::Unsupported); }
        #[cfg(unix)] {
            self.validate(false)?;
            let original = match self.parent(target, &scope, true, false) {
                Ok((parent, filename)) => parent.read(&filename)?,
                Err(Error::NotFound) => None,
                Err(error) => return Err(error),
            };
            if original.as_ref().is_some_and(|v| v.0.is_empty()) { return Err(Error::InvalidFormat); }
            let rules = skills::read_rules(original.as_ref().map_or("", |v| v.0.as_str()))?;
            self.validate(false)?;
            Ok(skills::Rules { rules, coverage: "selected_settings_layer" })
        }
        #[cfg(not(unix))] { let _ = scope; Err(Error::Unsupported) }
    }
    pub fn discover_skills(&self, target: Target, scope: Scope) -> Result<skills::Discovery, Error> {
        #[cfg(unix)] {
            self.validate(false)?;
            self.validate_path(&super::resources::root_path(&self.context, target, &scope)?)?;
            let found = skills::discover(&self.context, target, scope)?;
            self.validate(false)?;
            Ok(found)
        }
        #[cfg(not(unix))] { let _ = (target, scope); Err(Error::Unsupported) }
    }
    pub fn preview_mcp(&self, target: Target, scope: Scope, change: mcp::Change) -> Result<Preview, Error> {
        let project = if scope == Scope::ProjectLocal { Some(self.context.project_key()?) } else { None };
        let patches = mcp::patches(target, &scope, project.as_deref(), change.clone())?;
        let validation_scope = scope.clone();
        self.preview(target, scope, false, move |text| {
            mcp::validate_original(text, target, &validation_scope, project.as_deref(), &change)?;
            let output = mcp::apply(text, target, &validation_scope, project.as_deref(), &change, &patches)?;
            mcp::validate_edited(&output, target, &validation_scope, project.as_deref(), &change)?;
            Ok((output, None))
        })
    }
    pub fn preview_skill_visibility(&self, target: Target, scope: Scope, change: skills::VisibilityChange) -> Result<Preview, Error> {
        // Claude visibility uses settings files, NOT the MCP ~/.claude.json.
        // A separate settings target is deliberately handled by this operation.
        self.preview_visibility(target, scope, change)
    }
    #[cfg(unix)]
    pub fn recovery_material(&self, id: &OperationId) -> Result<super::RecoveryMaterial, Error> {
        let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?;
        self.validate(true)?;
        let dir = self.journal_dir()?; let _lock = dir.lock()?;
        let journal = self.load(&dir, id)?;
        let (parent, filename) = self.parent(journal.target, &journal.scope, journal.visibility, false)?;
        let _target_lock = parent.lock()?;
        if parent.identity()? != journal.parent || filename != journal.filename { return Err(Error::RootChanged); }
        let _stage = Self::stage(&parent, id, false)?;
        Ok(super::RecoveryMaterial { directory: self.target_path(journal.target, &journal.scope, journal.visibility)?.parent().ok_or(Error::UnsafePath)?.join(".dgos-native-recovery").join(id.as_str()) })
    }
    #[cfg(unix)]
    fn preview(&self, target: Target, scope: Scope, visibility: bool, edit: impl FnOnce(&str) -> Result<(String, Option<skills::RuleEvidence>), Error>) -> Result<Preview, Error> {
        if !cfg!(target_os = "macos") { return Err(Error::Unsupported); }
        let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?;
        self.validate(true)?;
        self.validate_path(&self.context.roots.journal)?;
        Directory::ensure(&self.context.roots.journal)?.private()?;
        let journal_dir = self.journal_dir()?; let _journal_lock = journal_dir.lock()?;
        let (parent, filename) = self.parent(target, &scope, visibility, true)?; let _target_lock = parent.lock()?;
        let before = parent.read(&filename)?;
        if before.as_ref().is_some_and(|v| v.1.mode & 0o022 != 0) { return Err(Error::AccessDenied); }
        if target == Target::Claude && before.as_ref().is_some_and(|v| v.0.is_empty()) { return Err(Error::InvalidFormat); }
        let text = before.as_ref().map_or("", |v| v.0.as_str());
        let (output, rule) = edit(text)?;
        let id = OperationId(uuid::Uuid::new_v4().to_string());
        let stage = Self::stage(&parent, &id, true)?;
        // Configuration recovery material may contain existing native secrets.
        // It remains exclusively in this private target-side recovery directory.
        let mode = before.as_ref().map_or(0o600, |v| v.1.mode);
        stage.write_new("payload", output.as_bytes(), mode)?;
        let after = stage.read("payload")?.ok_or(Error::Unavailable)?.1;
        let expires_at = now()?.checked_add(TTL_SECONDS).ok_or(Error::Unavailable)?;
        let project = self.project_binding(&scope)?;
        let target_path = self.target_path(target, &scope, visibility)?;
        let mut anchors = self.operation_anchors(target_path.parent().ok_or(Error::UnsafePath)?)?;
        if let Some(binding) = &project {
            if !anchors.iter().any(|(path, _)| path == &binding.root) { anchors.push((binding.root.clone(), binding.identity.clone())); }
        }
        let journal = Journal { version: 1, id, principal_hash: super::paths::hash(self.context.principal.as_bytes()), lease_hash: super::paths::hash(self.context.lease_id.as_bytes()), target, scope: scope.clone(), project, visibility, rule, filename, anchors, parent: parent.identity()?, before: before.map(|v| v.1), after, expires_at, state: State::Previewed, retained: true };
        self.validate(true)?;
        self.validate_project(&journal)?;
        Self::save(&journal_dir, &journal)?;
        Ok(Preview { receipt: journal.receipt(), target, scope, target_path: target_path.to_string_lossy().into_owned(), expires_at, rule: journal.rule })
    }
    #[cfg(not(unix))]
    fn preview(&self, _target: Target, _scope: Scope, _visibility: bool, _edit: impl FnOnce(&str) -> Result<(String, Option<skills::RuleEvidence>), Error>) -> Result<Preview, Error> { Err(Error::Unsupported) }
    fn preview_visibility(&self, target: Target, scope: Scope, change: skills::VisibilityChange) -> Result<Preview, Error> {
        let evidence_scope = scope.clone();
        self.preview(target, scope, true, move |text| {
            let output = match target {
                Target::Codex => skills::edit_codex_visibility(text, &change)?,
                Target::Claude => skills::edit_claude_visibility(text, &change)?,
            };
            Ok((output, skills::rule_evidence(text, evidence_scope, &change)?))
        })
    }
    pub fn commit(&self, id: &OperationId) -> Result<Receipt, Error> {
        #[cfg(unix)] {
            if !cfg!(target_os = "macos") { return Err(Error::Unsupported); }
            let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?;
            self.validate(true)?;
            let dir = self.journal_dir()?; let _journal_lock = dir.lock()?;
            let mut journal = self.load(&dir, id)?;
            if journal.lease_hash != super::paths::hash(self.context.lease_id.as_bytes()) { return Err(Error::AccessDenied); }
            // Only D142 name-rule previews can publish Claude settings changes.
            // Historical journals have no versioned rule evidence and stay rejected.
            if journal.visibility && journal.target == Target::Claude {
                let rule = journal.rule.as_ref().ok_or(Error::Unsupported)?;
                if rule.evidence_version != 1 || rule.scope != journal.scope || !rule.current_and_future || !rule.plugins_excluded || !rule.config_change_hook_possible || rule.higher_precedence != "not_evaluated" { return Err(Error::Unsupported); }
                skills::validate_name(&rule.name)?;
            }
            let (parent, filename) = self.parent(journal.target, &journal.scope, journal.visibility, false)?; let _target_lock = parent.lock()?;
            if parent.identity()? != journal.parent || filename != journal.filename { return Err(Error::RootChanged); }
            if journal.state != State::Previewed && journal.state != State::Prepared { return self.inspect_locked(&dir, &mut journal, &parent, &filename); }
            if now()? > journal.expires_at { return Err(Error::Expired); }
            let current = parent.read(&filename)?;
            if current.as_ref().map(|v| &v.1) != journal.before.as_ref() {
                journal.state = State::Conflict; Self::save(&dir, &journal)?; return Ok(journal.receipt());
            }
            parent.writable_owner()?;
            let stage = Self::stage(&parent, id, false)?;
            let payload = stage.read("payload")?.ok_or(Error::Conflict)?;
            if payload.1 != journal.after { return Err(Error::Conflict); }
            if let Some(rule) = &journal.rule {
                if !journal.visibility || journal.target != Target::Claude { return Err(Error::Unsupported); }
                let original = current.as_ref().map_or("", |v| v.0.as_str());
                let change = skills::VisibilityChange::ClaudeName { name: rule.name.clone(), state: rule.state };
                if skills::rule_evidence(original, journal.scope.clone(), &change)?.as_ref() != Some(rule)
                    || skills::edit_claude_visibility(original, &change)? != payload.0 { return Err(Error::Conflict); }
            }
            journal.state = State::Prepared; Self::save(&dir, &journal)?;
            self.validate(true)?;
            self.validate_project(&journal)?;
            if parent.identity()? != self.parent(journal.target, &journal.scope, journal.visibility, false)?.0.identity()? || parent.read(&filename)?.map(|v| v.1) != journal.before { journal.state = State::Conflict; Self::save(&dir, &journal)?; return Ok(journal.receipt()); }
            journal.state = State::Publishing; Self::save(&dir, &journal)?;
            if parent.publish(&stage, "payload", &filename, journal.before.is_some()).is_err() {
                // Includes post-rename fsync failures: inspect the original
                // operation rather than replaying a possibly completed rename.
                journal.state = State::Unknown; Self::save(&dir, &journal)?;
            }
            self.inspect_locked(&dir, &mut journal, &parent, &filename)
        }
        #[cfg(not(unix))] { let _ = id; Err(Error::Unsupported) }
    }
    pub fn cancel(&self, id: &OperationId) -> Result<Receipt, Error> {
        #[cfg(unix)] {
            let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?;
            self.validate(true)?;
            let dir = self.journal_dir()?; let _lock = dir.lock()?;
            let mut journal = self.load(&dir, id)?;
            if journal.lease_hash != super::paths::hash(self.context.lease_id.as_bytes()) { return Err(Error::AccessDenied); }
            let (parent, filename) = self.parent(journal.target, &journal.scope, journal.visibility, false)?;
            let _target_lock = parent.lock()?;
            if parent.identity()? != journal.parent || filename != journal.filename { return Err(Error::RootChanged); }
            if matches!(journal.state, State::Previewed | State::Prepared) {
                if parent.read(&filename)?.map(|v| v.1) != journal.before { return Err(Error::Conflict); }
                journal.state = State::Cancelled;
                Self::save(&dir, &journal)?;
            }
            Ok(journal.receipt())
        }
        #[cfg(not(unix))] { let _ = id; Err(Error::Unsupported) }
    }
    pub fn inspect(&self, id: &OperationId) -> Result<Receipt, Error> {
        #[cfg(unix)] {
            let _gate = self.context.gate.lock().map_err(|_| Error::Unavailable)?;
            self.validate(true)?;
            let dir = self.journal_dir()?; let _journal_lock = dir.lock()?;
            let mut journal = self.load(&dir, id)?;
            let (parent, filename) = self.parent(journal.target, &journal.scope, journal.visibility, false)?; let _target_lock = parent.lock()?;
            if parent.identity()? != journal.parent || filename != journal.filename { return Err(Error::RootChanged); }
            self.inspect_locked(&dir, &mut journal, &parent, &filename)
        }
        #[cfg(not(unix))] { let _ = id; Err(Error::Unsupported) }
    }
    #[cfg(unix)]
    fn inspect_locked(&self, dir: &Directory, journal: &mut Journal, parent: &Directory, filename: &str) -> Result<Receipt, Error> {
        if matches!(journal.state, State::Committed | State::Conflict | State::Cancelled) { return Ok(journal.receipt()); }
        let current = parent.read(filename)?.map(|v| v.1);
        let stage = Self::stage(parent, &journal.id, false)?;
        let payload = stage.read("payload")?.map(|v| v.1);
        if self.validate(false).is_err() || self.parent(journal.target, &journal.scope, journal.visibility, false).map(|v| v.0.identity()).ok() != Some(Ok(journal.parent.clone())) {
            journal.state = State::RecoveryRequired;
            journal.retained = true;
            Self::save(dir, journal)?;
            return Ok(journal.receipt());
        }
        if current.as_ref() == Some(&journal.after) && payload == journal.before {
            parent.sync()?; stage.sync()?;
            journal.state = State::Committed;
            // Keep every displaced inode, even after a successful commit. There
            // is no observable proof that external processes closed old FDs.
            journal.retained = journal.before.is_some();
        } else if current == journal.before && payload.as_ref() == Some(&journal.after) {
            if matches!(journal.state, State::Publishing | State::Unknown) { journal.state = State::Conflict; }
        } else { journal.state = State::RecoveryRequired; journal.retained = true; }
        Self::save(dir, journal)?;
        Ok(journal.receipt())
    }
}
