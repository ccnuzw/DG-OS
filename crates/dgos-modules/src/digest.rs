//! Package digest v2 and the single-read package snapshot (D057).
//!
//! The digest binds the manifest's security-relevant fields and every file of
//! the package. A package is read exactly once: the same pass parses the
//! manifest, hashes every file and keeps the entry bytes, so the bytes that
//! were verified are the bytes a loader executes.
//!
//! ```text
//! SHA-256( "dgos-module-digest\0v2\0"
//!          || frame(manifest_canon)
//!          || u64be(file_count)
//!          || for each file, sorted by relative UTF-8 path bytes:
//!               frame(path) || frame(content) )
//! frame(x) = u64be(len(x)) || x
//! ```
//!
//! Files are every regular file except the *root* `manifest.json`; a
//! `manifest.json` in a subdirectory is an ordinary file. The manifest's own
//! `artifact.sha256` cannot be part of its own digest and is not read at run
//! time. `scripts/bundled-modules.mjs` implements the same algorithm; a Rust
//! test compares the two on the shipped bundle.

use crate::{valid_relative_entry, ModuleManifest, ModulePackageError};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Prefix of a v2 integrity string. A bare 64-digit hex string is a v1
/// (legacy) value and is never executable (D057).
pub(crate) const INTEGRITY_PREFIX: &str = "v2:";
/// Upper bound for all files of one package together.
pub(crate) const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const DOMAIN: &[u8] = b"dgos-module-digest\0v2\0";

/// One consistent read of a package directory.
pub(crate) struct PackageSnapshot {
    pub(crate) manifest: ModuleManifest,
    /// `v2:` + lowercase SHA-256 hex.
    pub(crate) integrity: String,
    /// The v1 digest of the same bytes. Only computed on request: it is the
    /// input of the one-time migration and nothing else.
    pub(crate) legacy_digest: Option<String>,
    /// The entry file exactly as hashed.
    pub(crate) entry: Arc<[u8]>,
}

/// `v2:` followed by exactly 64 lowercase hex digits.
pub(crate) fn is_v2_integrity(value: &str) -> bool {
    value.strip_prefix(INTEGRITY_PREFIX).is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// The 64 hex digits of a v2 integrity.
pub(crate) fn v2_hex(integrity: &str) -> Option<&str> {
    is_v2_integrity(integrity).then(|| &integrity[INTEGRITY_PREFIX.len()..])
}

/// The v2 integrity of the package at `root`, read once. This is what a
/// package author writes (without the `v2:` prefix) into `artifact.sha256`.
pub fn package_integrity(root: &Path) -> Result<String, ModulePackageError> {
    read_package(root, false).map(|snapshot| snapshot.integrity)
}

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend((bytes.len() as u64).to_be_bytes());
    out.extend(bytes);
}

/// Fixed-order encoding of every manifest field except `artifact.sha256`.
/// `ModuleManifest` denies unknown fields, so nothing else can be present.
pub(crate) fn manifest_canon(manifest: &ModuleManifest) -> Vec<u8> {
    let mut out = Vec::new();
    for value in [
        &manifest.id,
        &manifest.name,
        &manifest.version,
        &manifest.entry,
        &manifest.artifact.version,
    ] {
        put(&mut out, value.as_bytes());
    }
    out.extend((manifest.permissions.len() as u64).to_be_bytes());
    for permission in &manifest.permissions {
        put(&mut out, permission.as_bytes());
    }
    match &manifest.requires {
        None => out.push(0),
        Some(requires) => {
            out.push(1);
            match requires.api {
                None => out.push(0),
                Some(api) => {
                    out.push(1);
                    out.extend(api.to_be_bytes());
                }
            }
            match &requires.react {
                None => out.push(0),
                Some(range) => {
                    out.push(1);
                    put(&mut out, range.as_bytes());
                }
            }
        }
    }
    out
}

/// Read the package at `root` once and derive everything from those bytes.
/// Symbolic links and non-regular files are refused.
pub(crate) fn read_package(
    root: &Path,
    with_legacy: bool,
) -> Result<PackageSnapshot, ModulePackageError> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(ModulePackageError::Manifest(
            "package root is not a directory".into(),
        ));
    }
    let manifest_bytes = read_regular(&root.join("manifest.json"), MAX_MANIFEST_BYTES)?;
    let manifest: ModuleManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| ModulePackageError::Manifest(error.to_string()))?;
    if !valid_relative_entry(&manifest.entry) {
        return Err(ModulePackageError::Manifest(
            "invalid identity, version or local entry".into(),
        ));
    }

    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut v2 = Sha256::new();
    v2.update(DOMAIN);
    let mut canon = Vec::new();
    put(&mut canon, &manifest_canon(&manifest));
    v2.update(&canon);
    v2.update((files.len() as u64).to_be_bytes());
    let mut v1 = with_legacy.then(Sha256::new);

    let mut budget = MAX_PACKAGE_BYTES;
    let mut entry: Option<Arc<[u8]>> = None;
    for (relative, path) in &files {
        let content = read_regular(path, budget)?;
        budget -= content.len() as u64;
        v2.update((relative.len() as u64).to_be_bytes());
        v2.update(relative.as_bytes());
        v2.update((content.len() as u64).to_be_bytes());
        v2.update(&content);
        if let Some(v1) = v1.as_mut() {
            if path.file_name().is_some_and(|name| name != "manifest.json") {
                v1.update(relative.as_bytes());
                v1.update([0]);
                v1.update(&content);
            }
        }
        if *relative == manifest.entry {
            entry = Some(Arc::from(content));
        }
    }
    let entry = entry.ok_or_else(|| {
        ModulePackageError::Manifest("invalid identity, version or local entry".into())
    })?;
    Ok(PackageSnapshot {
        manifest,
        integrity: format!("{INTEGRITY_PREFIX}{:x}", v2.finalize()),
        legacy_digest: v1.map(|hasher| format!("{:x}", hasher.finalize())),
        entry,
    })
}

/// Read a regular file through one open handle, at most `limit` bytes.
fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, ModulePackageError> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(ModulePackageError::Manifest(
            "symbolic links are not allowed".into(),
        ));
    }
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(ModulePackageError::Manifest(
            "package contains a file that is not a regular file".into(),
        ));
    }
    let too_large = || ModulePackageError::Manifest("package is too large".into());
    if metadata.len() > limit {
        return Err(too_large());
    }
    let mut content = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1).read_to_end(&mut content)?;
    if content.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(content)
}

/// Every regular file below `current` except the root `manifest.json`, as
/// `(relative path with '/', absolute path)`.
fn collect_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), ModulePackageError> {
    for entry in fs::read_dir(current)? {
        let path = entry?.path();
        let file_type = fs::symlink_metadata(&path)?.file_type();
        if file_type.is_symlink() {
            return Err(ModulePackageError::Manifest(
                "symbolic links are not allowed".into(),
            ));
        }
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("entries are below the root")
                .to_string_lossy()
                .replace('\\', "/");
            if relative != "manifest.json" {
                files.push((relative, path));
            }
        } else {
            return Err(ModulePackageError::Manifest(
                "package contains a file that is not a regular file".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn manifest(permissions: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "id": "mod-a", "name": "A", "version": "1.0.0", "entry": "entry.js",
            "artifact": {"version": "1.0.0", "sha256": "0".repeat(64)},
            "permissions": permissions,
        })
    }

    fn package(dir: &Path, manifest: &serde_json::Value, files: &[(&str, &[u8])]) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        for (name, body) in files {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
    }

    #[test]
    fn integrity_shape_is_v2_prefix_and_lowercase_hex() {
        let dir = tempdir().unwrap();
        package(dir.path(), &manifest(&[]), &[("entry.js", b"x")]);
        let integrity = package_integrity(dir.path()).unwrap();
        assert!(is_v2_integrity(&integrity));
        assert!(!is_v2_integrity(v2_hex(&integrity).unwrap()));
        assert!(!is_v2_integrity(&integrity.to_uppercase()));
    }

    #[test]
    fn every_security_relevant_manifest_field_changes_the_digest() {
        let base = tempdir().unwrap();
        package(
            base.path(),
            &manifest(&["providers.call"]),
            &[("entry.js", b"x")],
        );
        let reference = package_integrity(base.path()).unwrap();

        let mut variants = Vec::new();
        let mut m = manifest(&["providers.call"]);
        m["permissions"] = serde_json::json!(["providers.call", "providers.list"]);
        variants.push(("permissions added", m));
        let mut m = manifest(&[]);
        m["permissions"] = serde_json::json!([]);
        variants.push(("permissions removed", m));
        let mut m = manifest(&["providers.call"]);
        m["requires"] = serde_json::json!({"react": "^19"});
        variants.push(("requires", m));
        let mut m = manifest(&["providers.call"]);
        m["requires"] = serde_json::json!({});
        variants.push(("empty requires is not absent", m));
        let mut m = manifest(&["providers.call"]);
        m["name"] = "B".into();
        variants.push(("name", m));
        let mut m = manifest(&["providers.call"]);
        m["id"] = "mod-b".into();
        variants.push(("id", m));

        for (label, changed) in variants {
            let dir = tempdir().unwrap();
            package(dir.path(), &changed, &[("entry.js", b"x")]);
            assert_ne!(package_integrity(dir.path()).unwrap(), reference, "{label}");
        }
    }

    #[test]
    fn the_manifests_own_digest_field_is_not_part_of_the_digest() {
        let a = tempdir().unwrap();
        package(a.path(), &manifest(&[]), &[("entry.js", b"x")]);
        let b = tempdir().unwrap();
        let mut other = manifest(&[]);
        other["artifact"]["sha256"] = "f".repeat(64).into();
        package(b.path(), &other, &[("entry.js", b"x")]);
        assert_eq!(
            package_integrity(a.path()).unwrap(),
            package_integrity(b.path()).unwrap()
        );
    }

    #[test]
    fn a_nested_manifest_json_is_an_ordinary_file_and_is_covered() {
        let a = tempdir().unwrap();
        package(
            a.path(),
            &manifest(&[]),
            &[("entry.js", b"x"), ("sub/manifest.json", b"{\"a\":1}")],
        );
        let b = tempdir().unwrap();
        package(
            b.path(),
            &manifest(&[]),
            &[("entry.js", b"x"), ("sub/manifest.json", b"{\"a\":2}")],
        );
        assert_ne!(
            package_integrity(a.path()).unwrap(),
            package_integrity(b.path()).unwrap()
        );
    }

    #[test]
    fn length_framing_removes_the_v1_boundary_ambiguity() {
        // v1 hashed `path NUL content` back to back with no lengths. Sorted,
        // both packages feed it the stream `a\0cd\0eentry.js\0x`.
        let split = tempdir().unwrap();
        package(
            split.path(),
            &manifest(&[]),
            &[("a", b"c"), ("d", b"e"), ("entry.js", b"x")],
        );
        let merged = tempdir().unwrap();
        package(
            merged.path(),
            &manifest(&[]),
            &[("a", b"cd\0e"), ("entry.js", b"x")],
        );
        let legacy = |dir: &Path| read_package(dir, true).unwrap().legacy_digest.unwrap();
        assert_eq!(
            legacy(split.path()),
            legacy(merged.path()),
            "v1 collides: a real ambiguity, not a theoretical one"
        );
        assert_ne!(
            package_integrity(split.path()).unwrap(),
            package_integrity(merged.path()).unwrap(),
            "v2 separates them"
        );
    }

    #[test]
    fn the_entry_bytes_are_the_hashed_bytes_even_if_the_disk_changes_later() {
        let dir = tempdir().unwrap();
        package(
            dir.path(),
            &manifest(&[]),
            &[("entry.js", b"export default 1;")],
        );
        let snapshot = read_package(dir.path(), false).unwrap();
        fs::write(dir.path().join("entry.js"), b"export default 'evil';").unwrap();
        assert_eq!(&*snapshot.entry, b"export default 1;");
        assert_ne!(
            package_integrity(dir.path()).unwrap(),
            snapshot.integrity,
            "the next read sees the change"
        );
    }

    #[test]
    fn legacy_digest_matches_the_v1_algorithm() {
        let dir = tempdir().unwrap();
        package(
            dir.path(),
            &manifest(&[]),
            &[("entry.js", b"x"), ("sub/manifest.json", b"n")],
        );
        let mut hasher = Sha256::new();
        hasher.update(b"entry.js\0x");
        let expected = format!("{:x}", hasher.finalize());
        // v1 skipped every file named manifest.json, nested ones included.
        assert_eq!(
            read_package(dir.path(), true)
                .unwrap()
                .legacy_digest
                .unwrap(),
            expected
        );
    }

    #[test]
    fn symlinks_missing_entries_and_oversize_files_are_refused() {
        let dir = tempdir().unwrap();
        package(dir.path(), &manifest(&[]), &[("entry.js", b"x")]);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", dir.path().join("link")).unwrap();
            assert!(read_package(dir.path(), false).is_err());
            fs::remove_file(dir.path().join("link")).unwrap();
        }
        fs::remove_file(dir.path().join("entry.js")).unwrap();
        assert!(matches!(
            read_package(dir.path(), false),
            Err(ModulePackageError::Manifest(_))
        ));
        let big = tempdir().unwrap();
        package(big.path(), &manifest(&[]), &[("entry.js", b"x")]);
        let file = fs::File::create(big.path().join("blob")).unwrap();
        file.set_len(MAX_PACKAGE_BYTES + 1).unwrap();
        assert!(matches!(
            read_package(big.path(), false),
            Err(ModulePackageError::Manifest(reason)) if reason.contains("too large")
        ));
    }
}
