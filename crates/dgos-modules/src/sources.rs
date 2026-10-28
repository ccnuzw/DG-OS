//! Controlled local module sources (D034).
//!
//! A lifecycle operation never receives a path or URL from the renderer. The
//! only update source is a directory named exactly like the module id inside
//! a host-owned root; the id must pass the publisher's id check, the entry
//! must be a real directory (not a symlink) and must still resolve inside the
//! root. The publisher then re-validates manifest, entry, symlinks and digest
//! when the package is staged.

use crate::valid_module_id;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;

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
