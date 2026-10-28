//! Loader port for verified local module artifacts (D033).
//!
//! Loaders never receive URLs, renderer input or arbitrary paths: the only
//! input is a `VerifiedModuleArtifact`, which only `ModulePublisher` can
//! construct after manifest, entry-path, symlink and digest checks. A future
//! Module Federation adapter implements this port for trusted same-realm
//! packages; it is a loading mechanism, not a security sandbox.

use async_trait::async_trait;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;

/// A package directory whose manifest, canonical local entry and SHA-256
/// digest were checked by the publisher. Fields are private and there is no
/// public constructor.
#[derive(Clone, Debug)]
pub struct VerifiedModuleArtifact {
    module_id: String,
    version: String,
    integrity: String,
    package_root: PathBuf,
    entry_path: PathBuf,
}

impl VerifiedModuleArtifact {
    pub(crate) fn new(
        module_id: String,
        version: String,
        integrity: String,
        package_root: PathBuf,
        entry_path: PathBuf,
    ) -> Self {
        Self {
            module_id,
            version,
            integrity,
            package_root,
            entry_path,
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

    /// Canonical package directory.
    pub fn package_root(&self) -> &Path {
        &self.package_root
    }

    /// Canonical entry file inside `package_root`.
    pub fn entry_path(&self) -> &Path {
        &self.entry_path
    }
}

#[derive(Debug, Error)]
pub enum ModuleLoadError {
    #[error("module entry is not loadable: {0}")]
    Rejected(String),
    #[error("module artifact changed after verification")]
    ArtifactChanged,
    #[error("module loader I/O error: {0}")]
    Io(#[from] io::Error),
}

#[async_trait]
pub trait ModuleLoader: Send + Sync {
    /// Prove the artifact can be loaded without activating it. Must not have
    /// external side effects; runs before the current-version pointer moves.
    async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError>;

    /// Activate one runtime instance of the artifact.
    async fn start(
        &self,
        runtime_id: &str,
        artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError>;

    /// Release loader resources of one runtime instance. Called only after
    /// every admitted request of that instance has settled.
    async fn stop(
        &self,
        runtime_id: &str,
        artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError>;
}

/// Host-side loader for trusted local packages. Smoke checks that the entry
/// is a bounded, non-empty UTF-8 regular file that still resolves inside the
/// package; start re-checks the package digest. It does not execute
/// JavaScript and does not prove a Federation remote can be imported.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalArtifactLoader;

impl LocalArtifactLoader {
    pub const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;

    fn check_entry(artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
        let entry = artifact.entry_path().canonicalize()?;
        if !entry.starts_with(artifact.package_root()) {
            return Err(ModuleLoadError::Rejected("entry escapes package".into()));
        }
        let metadata = fs::symlink_metadata(&entry)?;
        if !metadata.file_type().is_file() {
            return Err(ModuleLoadError::Rejected("entry is not a file".into()));
        }
        if metadata.len() == 0 || metadata.len() > Self::MAX_ENTRY_BYTES {
            return Err(ModuleLoadError::Rejected("entry size out of bounds".into()));
        }
        if std::str::from_utf8(&fs::read(&entry)?).is_err() {
            return Err(ModuleLoadError::Rejected("entry is not UTF-8".into()));
        }
        Ok(())
    }
}

#[async_trait]
impl ModuleLoader for LocalArtifactLoader {
    async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
        Self::check_entry(artifact)
    }

    async fn start(
        &self,
        _runtime_id: &str,
        artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError> {
        Self::check_entry(artifact)?;
        let digest = crate::package_digest(artifact.package_root())
            .map_err(|_| ModuleLoadError::ArtifactChanged)?;
        if !digest.eq_ignore_ascii_case(artifact.integrity()) {
            return Err(ModuleLoadError::ArtifactChanged);
        }
        Ok(())
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
