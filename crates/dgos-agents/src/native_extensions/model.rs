use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::{Arc, Mutex}};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Target { Codex, Claude }
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Scope { OsUser, ProjectShared, ProjectLocal }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access { Read, Manage }
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Owner { Loose, Plugin, Managed, Synced, Symlink, Unknown }

/// These paths MUST come from the host's OS-user configuration and an already
/// authorized project record. Not Deserialize: this is not an IPC input.
/// The host authorizes the journal path; explicit previews may create it privately.
#[derive(Clone)]
pub struct TrustedRoots {
    pub os_home: PathBuf,
    pub codex_home: PathBuf,
    pub claude_home: PathBuf,
    pub claude_user_file: PathBuf,
    pub project: Option<PathBuf>,
    pub journal: PathBuf,
}

/// One context per admitted host lease. The same publication gate must also be
/// used by the host when ending that lease (call revoke, not token.cancel).
#[derive(Clone)]
pub struct HostContext {
    pub(super) principal: String,
    pub(super) lease_id: String,
    pub(super) roots: TrustedRoots,
    pub(super) access: Access,
    pub(super) lease: CancellationToken,
    pub(super) gate: Arc<Mutex<()>>,
}
impl HostContext {
    pub fn from_host(principal: String, lease_id: String, roots: TrustedRoots, access: Access) -> Result<Self, Error> {
        if principal.is_empty() || lease_id.is_empty() || !roots.os_home.is_absolute()
            || !roots.codex_home.is_absolute() || !roots.claude_home.is_absolute()
            || !roots.claude_user_file.is_absolute() || !roots.journal.is_absolute()
            || roots.project.as_ref().is_some_and(|p| !p.is_absolute()) { return Err(Error::AccessDenied); }
        Ok(Self { principal, lease_id, roots, access, lease: CancellationToken::new(), gate: Arc::new(Mutex::new(())) })
    }
    pub fn revoke(&self) -> Result<(), Error> {
        let _guard = self.gate.lock().map_err(|_| Error::Unavailable)?;
        self.lease.cancel();
        Ok(())
    }
    pub(super) fn admit(&self, write: bool) -> Result<(), Error> {
        if self.lease.is_cancelled() || (write && self.access != Access::Manage) { Err(Error::AccessDenied) } else { Ok(()) }
    }
    pub(super) fn config_path(&self, target: Target, scope: &Scope) -> Result<PathBuf, Error> {
        self.admit(false)?;
        match (target, scope) {
            (Target::Codex, Scope::OsUser) => Ok(self.roots.codex_home.join("config.toml")),
            (Target::Codex, Scope::ProjectShared) => Ok(self.roots.project.as_ref().ok_or(Error::AccessDenied)?.join(".codex/config.toml")),
            (Target::Codex, Scope::ProjectLocal) => Err(Error::Unsupported),
            (Target::Claude, Scope::OsUser | Scope::ProjectLocal) => {
                if *scope == Scope::ProjectLocal && self.roots.project.is_none() { return Err(Error::AccessDenied); }
                Ok(self.roots.claude_user_file.clone())
            }
            (Target::Claude, Scope::ProjectShared) => Ok(self.roots.project.as_ref().ok_or(Error::AccessDenied)?.join(".mcp.json")),
        }
    }
    pub(super) fn project_key(&self) -> Result<String, Error> {
        self.roots.project.as_ref().and_then(|p| p.to_str()).map(str::to_owned).ok_or(Error::AccessDenied)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationId(pub(super) String);
impl OperationId {
    pub fn as_str(&self) -> &str { &self.0 }
    pub fn parse(value: &str) -> Result<Self, Error> {
        uuid::Uuid::parse_str(value).map_err(|_| Error::NotFound)?;
        Ok(Self(value.into()))
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum State { Previewed, Prepared, Publishing, Committed, Conflict, Unknown, RecoveryRequired, Cancelled }
/// Safe projection: never contains raw configuration, field values, native
/// error output, paths to credentials, or a displaced configuration backup.
#[derive(Clone, Debug, Serialize)]
pub struct Receipt {
    pub operation_id: OperationId,
    pub state: State,
    pub pending_activation: bool,
    pub recovery_material_retained: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    AccessDenied, UnsafePath, RootChanged, InvalidFormat, Limit, Unsupported,
    Conflict, Busy, Expired, NotFound, Unavailable, RecoveryRequired,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "native_extension_{self:?}") }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::PermissionDenied => Self::AccessDenied,
            std::io::ErrorKind::AlreadyExists => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}
