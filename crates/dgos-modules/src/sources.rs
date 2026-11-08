//! Controlled local module sources (D034).
//!
//! Lifecycle commands receive only a module id. Non-official sources resolve
//! inside a host-owned local root; official sources require the entire shipped
//! bundle to match compiled pins. Staging rechecks copied identity and digest,
//! so neither renderer paths nor local replacements authorize official code.

use crate::valid_module_id;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;

/// Host-only bundled update source. Production pins cannot be supplied by a renderer.
#[derive(Clone, Debug)]
pub struct BuiltinModuleSources {
    root: PathBuf,
    pins: Vec<(String, String, String)>,
    bundle_pins: Vec<(String, String, String)>,
}

impl BuiltinModuleSources {
    /// Assemble from the host's shipped resource directory, never renderer input.
    /// Validation failure must leave official updates unavailable; no local fallback.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, ModuleSourceError> {
        Self::verified(root.into(), crate::admission::BUILTIN_PINS, &crate::release_gate::builtin_bundle_pins())
    }

    pub fn is_builtin(module_id: &str) -> bool {
        crate::admission::BUILTIN_PINS.iter().any(|(id,_,_)|*id==module_id)
    }

    fn verified(root: PathBuf, pins: &[crate::ReleasePin<'_>], bundle_pins: &[crate::ReleasePin<'_>]) -> Result<Self, ModuleSourceError> {
        let mut ids = std::collections::HashSet::new();
        if pins.iter().any(|(id, _, _)| !ids.insert(id.to_ascii_lowercase())) { return Err(ModuleSourceError::Rejected); }
        if !crate::verify_release_bundle(bundle_pins, &root).is_empty() { return Err(ModuleSourceError::Rejected); }
        let owned = |pins: &[crate::ReleasePin<'_>]| pins.iter().map(|(id, version, digest)| (id.to_string(), version.to_string(), digest.to_string())).collect();
        Ok(Self { root, pins: owned(pins), bundle_pins: owned(bundle_pins) })
    }

    pub fn contains(&self, module_id: &str) -> bool {
        self.pins.iter().any(|(id,_,_)|id==module_id)
    }

    pub(crate) fn resolve(&self, module_id: &str) -> Result<(PathBuf, &str, &str), ModuleSourceError> {
        if !valid_module_id(module_id) { return Err(ModuleSourceError::InvalidModuleId); }
        let pins: Vec<_> = self.bundle_pins.iter().map(|(id,version,digest)|(id.as_str(),version.as_str(),digest.as_str())).collect();
        // Recheck the complete bundle for each explicit update, not only at startup.
        if !crate::verify_release_bundle(&pins,&self.root).is_empty() { return Err(ModuleSourceError::Rejected); }
        let mut matches=self.pins.iter().filter(|(id,_,_)|id==module_id);
        let (_,version,digest)=matches.next().ok_or(ModuleSourceError::NotFound)?;
        if matches.next().is_some() { return Err(ModuleSourceError::Rejected); }
        Ok((self.root.join(module_id).join(version),version,digest))
    }

    #[cfg(test)]
    pub(crate) fn for_test(root: PathBuf, pins: &[crate::ReleasePin<'_>]) -> Result<Self, ModuleSourceError> {
        Self::verified(root, pins, pins)
    }
}

#[derive(Debug, Error)]
pub enum ModuleSourceError {
    #[error("module id is not valid")]
    InvalidModuleId,
    #[error("no local source is available for the module")]
    NotFound,
    #[error("module source is not an allowed local directory")]
    Rejected,
    #[error("module source I/O error: {0}")]
    Io(#[source] io::Error),
}

#[derive(Clone, Debug)]
pub struct LocalModuleSources {
    root: PathBuf,
}

impl LocalModuleSources {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve the canonical package directory for `module_id`.
    pub fn resolve(&self, module_id: &str) -> Result<PathBuf, ModuleSourceError> {
        if !valid_module_id(module_id) {
            return Err(ModuleSourceError::InvalidModuleId);
        }
        let root = canonical(&self.root)?;
        let candidate = root.join(module_id);
        let metadata = match fs::symlink_metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ModuleSourceError::NotFound)
            }
            Err(error) => return Err(ModuleSourceError::Io(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ModuleSourceError::Rejected);
        }
        let resolved = canonical(&candidate)?;
        if !is_inside(&resolved, &root) {
            return Err(ModuleSourceError::Rejected);
        }
        Ok(resolved)
    }
}

fn canonical(path: &Path) -> Result<PathBuf, ModuleSourceError> {
    fs::canonicalize(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ModuleSourceError::NotFound
        } else {
            ModuleSourceError::Io(error)
        }
    })
}

fn is_inside(path: &Path, root: &Path) -> bool {
    path != root && path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_only_a_real_directory_named_like_the_module_inside_the_root() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("module-1")).unwrap();
        let sources = LocalModuleSources::new(root.path());

        let resolved = sources.resolve("module-1").unwrap();
        assert_eq!(
            resolved,
            fs::canonicalize(root.path().join("module-1")).unwrap()
        );
        assert!(matches!(
            sources.resolve("module-2"),
            Err(ModuleSourceError::NotFound)
        ));
    }

    #[test]
    fn rejects_traversal_separators_urls_and_non_directories() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("module-1")).unwrap();
        fs::write(root.path().join("plain-file"), "x").unwrap();
        let sources = LocalModuleSources::new(root.path());

        for id in [
            "",
            ".",
            "..",
            "../module-1",
            "module-1/..",
            "nested/module",
            "/etc",
            "https://example.invalid/module",
            "file:///tmp/module",
            "C:\\module",
        ] {
            assert!(
                matches!(sources.resolve(id), Err(ModuleSourceError::InvalidModuleId)),
                "{id:?} must be rejected"
            );
        }
        assert!(matches!(
            sources.resolve("plain-file"),
            Err(ModuleSourceError::Rejected)
        ));
    }

    #[test]
    fn missing_root_is_not_found() {
        let root = tempdir().unwrap();
        let sources = LocalModuleSources::new(root.path().join("absent"));
        assert!(matches!(
            sources.resolve("module-1"),
            Err(ModuleSourceError::NotFound)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_source_even_when_it_points_inside_the_root() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("alias")).unwrap();
        let sources = LocalModuleSources::new(root.path());

        assert!(matches!(
            sources.resolve("escape"),
            Err(ModuleSourceError::Rejected)
        ));
        assert!(matches!(
            sources.resolve("alias"),
            Err(ModuleSourceError::Rejected)
        ));
    }
}
