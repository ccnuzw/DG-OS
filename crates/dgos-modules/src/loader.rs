//! Loader port for verified local module artifacts (D033).
//!
//! Loaders never receive URLs, renderer input or arbitrary paths: the only
//! input is a `VerifiedModuleArtifact`, which only `ModulePublisher` can
//! construct after manifest, entry-path, symlink and digest checks. A future
//! Module Federation adapter implements this port for trusted same-realm
//! packages; it is a loading mechanism, not a security sandbox.

use async_trait::async_trait;
use std::{fmt, io, sync::Arc};
use thiserror::Error;

/// A package whose manifest, identity and v2 digest were checked by the
/// publisher from a single read (D057). It carries the entry bytes exactly
/// as they were hashed: loaders execute these bytes and never go back to the
/// disk, so a file changed after verification cannot be what runs. Fields are
/// private and there is no public constructor.
#[derive(Clone)]
pub struct VerifiedModuleArtifact {
    module_id: String,
    version: String,
    integrity: String,
    entry: Arc<[u8]>,
}

impl fmt::Debug for VerifiedModuleArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedModuleArtifact")
            .field("module_id", &self.module_id)
            .field("version", &self.version)
            .field("integrity", &self.integrity)
            .field("entry_bytes", &self.entry.len())
            .finish()
    }
}

impl VerifiedModuleArtifact {
    pub(crate) fn new(
        module_id: String,
        version: String,
        integrity: String,
        entry: Arc<[u8]>,
    ) -> Self {
        Self {
            module_id,
            version,
            integrity,
            entry,
        }
    }

    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn integrity(&self) -> &str {
        &self.integrity
    }

    /// The verified entry file, byte for byte as it was hashed.
    pub fn entry_bytes(&self) -> &[u8] {
        &self.entry
    }
}

/// Which loader call exceeded its supervisor deadline (D044).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadPhase {
    Smoke,
    Start,
    Stop,
}

/// Why the page refused or aborted a real load. A coarse category only: the
/// page never reports messages, stacks or module-supplied text (D044).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageFailureStage {
    Import,
    Contract,
    Render,
    Hook,
    Timeout,
    Cancelled,
}

#[derive(Debug, Error)]
pub enum ModuleLoadError {
    #[error("module entry is not loadable: {0}")]
    Rejected(String),
    #[error("module loader I/O error: {0}")]
    Io(#[from] io::Error),
    /// The supervisor deadline for one loader call elapsed. Only the loader
    /// future was dropped; loaders must be cancel-safe (D044).
    #[error("module loader call timed out: {0:?}")]
    Timeout(LoadPhase),
    /// An executing loader has no page to run the module in. It never falls
    /// back to a loader that does not execute code (D043).
    #[error("no module host page is attached")]
    HostNotAttached,
    /// The page that held (or was to hold) the module was replaced, reloaded
    /// or destroyed, or its channel is gone.
    #[error("module host page was lost")]
    PageLost,
    #[error("module host page rejected the load: {0:?}")]
    PageRejected(PageFailureStage),
}

/// Loader port. Every method must be cancel-safe: the supervisor enforces
/// per-call deadlines (D044) by dropping the loader future only, never its
/// own, and then runs its normal failure cleanup.
#[async_trait]
pub trait ModuleLoader: Send + Sync {
    /// Whether `start` runs module code. Executing loaders are only usable
    /// behind an admission policy (D041); the status view reports this so a
    /// `ready` runtime can be told apart from one that never executed (D043).
    fn executes_module_code(&self) -> bool {
        false
    }

    /// Host-side preflight without side effects (D043): nothing is imported
    /// or executed and nothing is activated. A successful preflight does not
    /// prove the module can run. Runs before the current-version pointer
    /// moves.
    async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError>;

    /// Really load one runtime instance of the artifact. Returns only once
    /// the instance is active; a failure leaves nothing the supervisor must
    /// not release through `stop`.
    async fn start(
        &self,
        runtime_id: &str,
        artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError>;

    /// Release loader resources of one runtime instance. Called only after
    /// every admitted request of that instance has settled. Must be safe to
    /// call for an instance whose `start` failed, timed out or was dropped.
    async fn stop(
        &self,
        runtime_id: &str,
        artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError>;
}

/// Largest entry source a loader accepts.
pub const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;

/// The entry of a verified artifact as source text: 1..=16 MiB of UTF-8.
/// The bytes come from the verification read (D057); nothing is read from
/// the disk here.
pub(crate) fn verified_entry_source(
    artifact: &VerifiedModuleArtifact,
) -> Result<String, ModuleLoadError> {
    let bytes = artifact.entry_bytes();
    if bytes.is_empty() || bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(ModuleLoadError::Rejected("entry size out of bounds".into()));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ModuleLoadError::Rejected("entry is not UTF-8".into()))
}

/// Host-side validator for trusted local packages. Smoke and start check
/// that the verified entry is a bounded, non-empty UTF-8 file. It does not
/// execute JavaScript (`executes_module_code() == false`), so a `ready`
/// runtime started through it has not run any module code (D043).
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalArtifactLoader;

impl LocalArtifactLoader {
    pub const MAX_ENTRY_BYTES: u64 = MAX_ENTRY_BYTES;
}

#[async_trait]
impl ModuleLoader for LocalArtifactLoader {
    async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
        verified_entry_source(artifact).map(drop)
    }

    async fn start(
        &self,
        _runtime_id: &str,
        artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError> {
        verified_entry_source(artifact).map(drop)
    }

    /// The local loader keeps no per-instance resources.
    async fn stop(
        &self,
        _runtime_id: &str,
        _artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError> {
        Ok(())
    }
}
