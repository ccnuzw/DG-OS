//! Release-artifact gate for the compiled-in admission list (D047, D049).
//!
//! `BUILTIN_PINS` is only meaningful if each pin equals what the build
//! actually ships. This gate takes the directory that ships with the host
//! (`<root>/<module_id>/<version>/`), recomputes every package digest itself
//! and compares `(module_id, version, integrity)` with the pins one by one.
//!
//! Nothing here reads trust from the packages: the manifest's own
//! `artifact.sha256`, the `modules` table, user directories, grants and
//! renderer input are never inputs to the comparison. The manifest is parsed
//! only to prove the directory is the package the pin names.
//!
//! The gate checks a directory. It does not prove that the installer or
//! bundle produced from that directory still contains the same bytes; run it
//! against the unpacked release artifact for that (see `verify_release_bundle`
//! example).

use crate::{
    admission::BUILTIN_PINS, digest, valid_module_id, valid_version_path, ModulePackageError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

/// One expected package: `(module_id, version, integrity)`.
pub type ReleasePin<'a> = (&'a str, &'a str, &'a str);

/// A reason the release must not ship. Variants carry only identity strings,
/// never file contents.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ReleaseViolation {
    /// The pin itself is malformed: empty or unsafe id/version, or integrity
    /// that is not `v2:` followed by 64 lowercase hex digits (D057).
    InvalidPin {
        module_id: String,
        version: String,
        reason: &'static str,
    },
    /// Two pins name the same `(module_id, version)` (ids compare
    /// case-insensitively because bundle roots can live on case-insensitive
    /// file systems).
    DuplicatePin { module_id: String, version: String },
    /// A pin has no package directory in the bundle.
    MissingPackage { module_id: String, version: String },
    /// The bundle holds a package that no pin names. This is the extra-package
    /// case: shipping unpinned code is a release defect even though admission
    /// would refuse to run it.
    UnpinnedPackage { module_id: String, version: String },
    /// The bundle root holds something that is not a plain module directory
    /// (file, symlink, or an unreadable entry).
    UnexpectedEntry { name: String },
    /// The package directory cannot be read as a package.
    UnreadablePackage {
        module_id: String,
        version: String,
        reason: &'static str,
    },
    /// The package's manifest names a different module or version than the
    /// pin, or its entry is not a valid local file.
    ManifestMismatch {
        module_id: String,
        version: String,
        reason: &'static str,
    },
    /// The digest the host recomputes differs from the pin.
    DigestMismatch { module_id: String, version: String },
    /// The recomputed digest matches the pin but the manifest declares a
    /// different one. The publisher would reject this package at install.
    ManifestDigestDisagrees { module_id: String, version: String },
}

/// Check the pins and the bundle directory against each other. An empty list
/// of violations means every pin has exactly one package whose recomputed
/// digest equals the pin, and the bundle holds nothing else.
///
/// A missing `root` is an empty bundle, so an empty pin list passes with no
/// directory and a non-empty list reports every pin as missing.
pub fn verify_release_bundle(pins: &[ReleasePin<'_>], root: &Path) -> Vec<ReleaseViolation> {
    let mut violations = BTreeSet::new();
    let mut valid: BTreeMap<(&str, &str), &str> = BTreeMap::new();
    let mut folded = BTreeSet::new();
    let mut named = BTreeSet::new();

    for &(module_id, version, integrity) in pins {
        let owned = |reason| ReleaseViolation::InvalidPin {
            module_id: module_id.to_owned(),
            version: version.to_owned(),
            reason,
        };
        named.insert((module_id.to_owned(), version.to_owned()));
        if let Some(reason) = pin_defect(module_id, version, integrity) {
            violations.insert(owned(reason));
            continue;
        }
        if !folded.insert((module_id.to_ascii_lowercase(), version.to_owned())) {
            violations.insert(ReleaseViolation::DuplicatePin {
                module_id: module_id.to_owned(),
                version: version.to_owned(),
            });
            continue;
        }
        valid.insert((module_id, version), integrity);
    }

    let found = match scan_bundle(root, &mut violations) {
        Some(found) => found,
        None => BTreeSet::new(),
    };

    for (module_id, version) in &found {
        if !named.contains(&(module_id.clone(), version.clone())) {
            violations.insert(ReleaseViolation::UnpinnedPackage {
                module_id: module_id.clone(),
                version: version.clone(),
            });
        }
    }

    for (&(module_id, version), &integrity) in &valid {
        if !found.contains(&(module_id.to_owned(), version.to_owned())) {
            violations.insert(ReleaseViolation::MissingPackage {
                module_id: module_id.to_owned(),
                version: version.to_owned(),
            });
            continue;
        }
        check_package(
            &root.join(module_id).join(version),
            module_id,
            version,
            integrity,
            &mut violations,
        );
    }

    violations.into_iter().collect()
}

/// The gate for the list compiled into this build. This is what a release
/// pipeline runs against the unpacked artifact.
pub fn verify_builtin_release_bundle(root: &Path) -> Vec<ReleaseViolation> {
    verify_release_bundle(BUILTIN_PINS, root)
}

/// Shape check shared with the compiled-in list: a pin must be a complete,
/// canonical `(module_id, version, sha256)`.
pub(crate) fn pin_defect(module_id: &str, version: &str, integrity: &str) -> Option<&'static str> {
    if !valid_module_id(module_id) {
        return Some("module id is not valid");
    }
    if !valid_version_path(version) {
        return Some("version is not valid");
    }
    if !digest::is_v2_integrity(integrity) {
        return Some("integrity must be v2: followed by 64 lowercase hex digits");
    }
    None
}

/// List `<root>/<id>/<version>` directories. Anything else in the tree is a
/// violation; a missing root is empty.
fn scan_bundle(
    root: &Path,
    violations: &mut BTreeSet<ReleaseViolation>,
) -> Option<BTreeSet<(String, String)>> {
    let mut found = BTreeSet::new();
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Some(found),
        Err(_) => {
            violations.insert(ReleaseViolation::UnexpectedEntry { name: ".".into() });
            return None;
        }
        Ok(meta) if !meta.is_dir() => {
            violations.insert(ReleaseViolation::UnexpectedEntry { name: ".".into() });
            return None;
        }
        Ok(_) => {}
    }
    for (name, path) in children(root, violations, "") {
        if !is_plain_dir(&path) {
            violations.insert(ReleaseViolation::UnexpectedEntry { name });
            continue;
        }
        for (version, version_path) in children(&path, violations, &format!("{name}/")) {
            if is_plain_dir(&version_path) {
                found.insert((name.clone(), version));
            } else {
                violations.insert(ReleaseViolation::UnexpectedEntry {
                    name: format!("{name}/{version}"),
                });
            }
        }
    }
    Some(found)
}

fn children(
    dir: &Path,
    violations: &mut BTreeSet<ReleaseViolation>,
    prefix: &str,
) -> Vec<(String, std::path::PathBuf)> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => {
            violations.insert(ReleaseViolation::UnexpectedEntry {
                name: prefix.trim_end_matches('/').to_owned(),
            });
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => out.push((
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )),
            Err(_) => {
                violations.insert(ReleaseViolation::UnexpectedEntry {
                    name: prefix.trim_end_matches('/').to_owned(),
                });
            }
        }
    }
    out.sort();
    out
}

fn is_plain_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
}

fn check_package(
    package: &Path,
    module_id: &str,
    version: &str,
    integrity: &str,
    violations: &mut BTreeSet<ReleaseViolation>,
) {
    let identity = || (module_id.to_owned(), version.to_owned());
    let unreadable = |reason| {
        let (module_id, version) = identity();
        ReleaseViolation::UnreadablePackage {
            module_id,
            version,
            reason,
        }
    };
    let mismatch = |reason| {
        let (module_id, version) = identity();
        ReleaseViolation::ManifestMismatch {
            module_id,
            version,
            reason,
        }
    };

    // One read: the manifest, every file and the digest come from the same
    // bytes, exactly as the host reads them (D057). Every file, including a
    // `manifest.json` in a subdirectory, is part of the digest.
    let snapshot = match digest::read_package(package, false) {
        Ok(snapshot) => snapshot,
        Err(ModulePackageError::Io(_)) => {
            violations.insert(unreadable("package contents cannot be read"));
            return;
        }
        Err(_) => {
            violations.insert(unreadable("package is not a readable module package"));
            return;
        }
    };
    let manifest = &snapshot.manifest;
    if manifest.id != module_id {
        violations.insert(mismatch("manifest id differs from the pin"));
    }
    if manifest.version != version || manifest.artifact.version != version {
        violations.insert(mismatch("manifest version differs from the pin"));
    }
    if snapshot.integrity != integrity {
        let (module_id, version) = identity();
        violations.insert(ReleaseViolation::DigestMismatch { module_id, version });
    } else if !digest::v2_hex(integrity)
        .is_some_and(|hex| manifest.artifact.sha256.eq_ignore_ascii_case(hex))
    {
        let (module_id, version) = identity();
        violations.insert(ReleaseViolation::ManifestDigestDisagrees { module_id, version });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{tempdir, TempDir};

    /// Write `<root>/<id>/<version>` with one entry file and a consistent
    /// manifest; returns the recomputed v2 integrity.
    fn publish(root: &Path, id: &str, version: &str, source: &str) -> String {
        let dir = root.join(id).join(version);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("entry.js"), source).unwrap();
        let write_manifest = |sha256: &str| {
            fs::write(
                dir.join("manifest.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id": id,
                    "name": "Example",
                    "version": version,
                    "entry": "entry.js",
                    "artifact": {"version": version, "sha256": sha256},
                    "permissions": [],
                }))
                .unwrap(),
            )
            .unwrap();
        };
        write_manifest(&"0".repeat(64));
        let integrity = digest::package_integrity(&dir).unwrap();
        write_manifest(digest::v2_hex(&integrity).unwrap());
        integrity
    }

    fn bundle() -> TempDir {
        tempdir().unwrap()
    }

    fn kinds(violations: &[ReleaseViolation]) -> Vec<&'static str> {
        violations
            .iter()
            .map(|violation| match violation {
                ReleaseViolation::InvalidPin { .. } => "invalid_pin",
                ReleaseViolation::DuplicatePin { .. } => "duplicate_pin",
                ReleaseViolation::MissingPackage { .. } => "missing",
                ReleaseViolation::UnpinnedPackage { .. } => "unpinned",
                ReleaseViolation::UnexpectedEntry { .. } => "unexpected_entry",
                ReleaseViolation::UnreadablePackage { .. } => "unreadable",
                ReleaseViolation::ManifestMismatch { .. } => "manifest_mismatch",
                ReleaseViolation::DigestMismatch { .. } => "digest_mismatch",
                ReleaseViolation::ManifestDigestDisagrees { .. } => "manifest_digest",
            })
            .collect()
    }

    #[test]
    fn a_bundle_that_equals_the_pins_passes() {
        let root = bundle();
        let a = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        let b = publish(root.path(), "mod-a", "1.1.0", "export default 2;");
        let c = publish(root.path(), "mod-b", "1.0.0", "export default 3;");
        let pins = [
            ("mod-a", "1.0.0", a.as_str()),
            ("mod-a", "1.1.0", b.as_str()),
            ("mod-b", "1.0.0", c.as_str()),
        ];
        assert_eq!(verify_release_bundle(&pins, root.path()), vec![]);
    }

    #[test]
    fn empty_pins_pass_only_an_empty_bundle() {
        let root = bundle();
        assert_eq!(verify_release_bundle(&[], root.path()), vec![]);
        assert_eq!(
            verify_release_bundle(&[], &root.path().join("absent")),
            vec![]
        );
        publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        assert_eq!(
            kinds(&verify_release_bundle(&[], root.path())),
            ["unpinned"]
        );
    }

    #[test]
    fn a_pin_without_a_package_is_missing_and_a_missing_root_is_empty() {
        let root = bundle();
        let digest = format!("v2:{}", "a".repeat(64));
        let pins = [("mod-a", "1.0.0", digest.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["missing"]
        );
        assert_eq!(
            kinds(&verify_release_bundle(&pins, &root.path().join("absent"))),
            ["missing"]
        );
    }

    #[test]
    fn an_extra_package_or_extra_version_is_reported() {
        let root = bundle();
        let a = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        publish(root.path(), "mod-a", "2.0.0", "export default 2;");
        publish(root.path(), "mod-z", "1.0.0", "export default 3;");
        let pins = [("mod-a", "1.0.0", a.as_str())];
        let violations = verify_release_bundle(&pins, root.path());
        assert_eq!(kinds(&violations), ["unpinned", "unpinned"]);
        assert!(violations.contains(&ReleaseViolation::UnpinnedPackage {
            module_id: "mod-z".into(),
            version: "1.0.0".into()
        }));
    }

    #[test]
    fn a_pin_for_the_wrong_version_leaves_one_missing_and_one_unpinned() {
        let root = bundle();
        let digest = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        let pins = [("mod-a", "1.0.1", digest.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["missing", "unpinned"]
        );
    }

    #[test]
    fn a_directory_whose_manifest_names_another_version_is_a_mismatch() {
        let root = bundle();
        let digest = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        // The directory is named 1.1.0 but still holds the 1.0.0 package.
        fs::rename(
            root.path().join("mod-a").join("1.0.0"),
            root.path().join("mod-a").join("1.1.0"),
        )
        .unwrap();
        let pins = [("mod-a", "1.1.0", digest.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["manifest_mismatch"]
        );
    }

    #[test]
    fn a_directory_whose_manifest_names_another_module_is_a_mismatch() {
        let root = bundle();
        let digest = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        fs::rename(root.path().join("mod-a"), root.path().join("mod-b")).unwrap();
        let pins = [("mod-b", "1.0.0", digest.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["manifest_mismatch"]
        );
    }

    #[test]
    fn changed_bytes_fail_even_when_the_manifest_digest_is_refreshed() {
        let root = bundle();
        let pinned = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        // A tampered package whose manifest was regenerated to be
        // self-consistent: the package digest check alone would accept it.
        let tampered = publish(root.path(), "mod-a", "1.0.0", "export default 666;");
        assert_ne!(pinned, tampered);
        let pins = [("mod-a", "1.0.0", pinned.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["digest_mismatch"]
        );
    }

    #[test]
    fn an_added_file_changes_the_digest_and_fails() {
        let root = bundle();
        let pinned = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        fs::write(
            root.path().join("mod-a").join("1.0.0").join("extra.js"),
            "x",
        )
        .unwrap();
        let pins = [("mod-a", "1.0.0", pinned.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["digest_mismatch"]
        );
    }

    #[test]
    fn the_manifest_cannot_vouch_for_a_pin() {
        let root = bundle();
        let digest = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        // The pin is wrong; the manifest still says what the files hash to.
        let wrong = format!("v2:{}", "b".repeat(64));
        let pins = [("mod-a", "1.0.0", wrong.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["digest_mismatch"]
        );
        // The pin is right but the manifest claims another digest.
        let path = root
            .path()
            .join("mod-a")
            .join("1.0.0")
            .join("manifest.json");
        let manifest = fs::read_to_string(&path).unwrap().replace(
            digest.strip_prefix("v2:").unwrap(),
            wrong.strip_prefix("v2:").unwrap(),
        );
        fs::write(&path, manifest).unwrap();
        let pins = [("mod-a", "1.0.0", digest.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["manifest_digest"]
        );
    }

    #[test]
    fn uppercase_integrity_is_not_a_canonical_pin() {
        let root = bundle();
        let digest = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        let upper = digest.to_ascii_uppercase();
        let pins = [("mod-a", "1.0.0", upper.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["invalid_pin"]
        );
    }

    #[test]
    fn duplicate_pins_are_reported_whatever_their_digest() {
        let root = bundle();
        let digest = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        let other = format!("v2:{}", "c".repeat(64));
        let pins = [
            ("mod-a", "1.0.0", digest.as_str()),
            // Exact repeat.
            ("mod-a", "1.0.0", digest.as_str()),
            // Same module and version with another digest is still a duplicate.
            ("mod-a", "1.0.0", other.as_str()),
            // Case-folded id collides on case-insensitive file systems.
            ("MOD-A", "1.0.0", digest.as_str()),
        ];
        let duplicate = |module_id: &str| ReleaseViolation::DuplicatePin {
            module_id: module_id.into(),
            version: "1.0.0".into(),
        };
        // The first pin stays authoritative, so the package itself passes.
        assert_eq!(
            verify_release_bundle(&pins, root.path()),
            vec![duplicate("MOD-A"), duplicate("mod-a")]
        );
    }

    #[test]
    fn malformed_pins_are_rejected_without_touching_the_bundle() {
        let root = bundle();
        let ok = format!("v2:{}", "d".repeat(64));
        let short = format!("v2:{}", "d".repeat(63));
        let non_hex = format!("v2:{}g", "d".repeat(63));
        let upper = format!("v2:{}", "D".repeat(64));
        let legacy = "d".repeat(64);
        let cases: [ReleasePin<'_>; 11] = [
            ("", "1.0.0", ok.as_str()),
            ("mod-a", "", ok.as_str()),
            ("..", "1.0.0", ok.as_str()),
            ("mod/a", "1.0.0", ok.as_str()),
            ("mod-a", "1/0", ok.as_str()),
            ("mod-a", "1.0.0", short.as_str()),
            ("mod-a", "1.0.0", non_hex.as_str()),
            ("mod-a", "1.0.0", upper.as_str()),
            // A bare digest is the legacy (v1) form and can never be a pin.
            ("mod-a", "1.0.0", legacy.as_str()),
            ("mod-a", "1.0.0", "v2:"),
            ("mod-a", "1.0.0", ""),
        ];
        for pin in cases {
            let violations = verify_release_bundle(&[pin], root.path());
            assert_eq!(kinds(&violations), ["invalid_pin"], "{pin:?}");
        }
    }

    #[test]
    fn a_nested_manifest_is_an_ordinary_file_covered_by_the_digest() {
        let root = bundle();
        let dir = root.path().join("mod-a").join("1.0.0");
        publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("manifest.json"), "{\"v\":1}").unwrap();
        let pinned = digest::package_integrity(&dir).unwrap();
        let pins = [("mod-a", "1.0.0", pinned.as_str())];
        // The manifest declares the pre-nested digest; the pin is the new one.
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["manifest_digest"]
        );
        // Once the declared digest is refreshed the bundle passes, and any
        // later change to the nested file fails (v1 ignored it).
        let manifest_path = dir.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["artifact"]["sha256"] = digest::v2_hex(&pinned).unwrap().into();
        fs::write(&manifest_path, manifest.to_string()).unwrap();
        assert_eq!(verify_release_bundle(&pins, root.path()), vec![]);
        fs::write(dir.join("sub").join("manifest.json"), "{\"v\":2}").unwrap();
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["digest_mismatch"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_stray_files_are_not_packages() {
        let root = bundle();
        let digest = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        fs::write(root.path().join("README"), "x").unwrap();
        std::os::unix::fs::symlink(root.path().join("mod-a"), root.path().join("alias")).unwrap();
        let pins = [("mod-a", "1.0.0", digest.as_str())];
        let violations = verify_release_bundle(&pins, root.path());
        assert_eq!(kinds(&violations), ["unexpected_entry", "unexpected_entry"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_a_package_cannot_be_digested() {
        let root = bundle();
        let digest = publish(root.path(), "mod-a", "1.0.0", "export default 1;");
        let dir = root.path().join("mod-a").join("1.0.0");
        std::os::unix::fs::symlink("/etc/hosts", dir.join("link.js")).unwrap();
        let pins = [("mod-a", "1.0.0", digest.as_str())];
        assert_eq!(
            kinds(&verify_release_bundle(&pins, root.path())),
            ["unreadable"]
        );
    }

    #[test]
    fn violations_are_deterministic() {
        let root = bundle();
        publish(root.path(), "mod-z", "1.0.0", "export default 1;");
        publish(root.path(), "mod-b", "1.0.0", "export default 2;");
        let first = verify_release_bundle(&[], root.path());
        let second = verify_release_bundle(&[], root.path());
        assert_eq!(first, second);
        assert!(first.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    /// The executable release gate for the real list. While `BUILTIN_PINS`
    /// is empty it requires the shipped directory to hold no package; the
    /// first real pin turns it into the artifact comparison (D047, D049).
    #[test]
    fn the_shipped_bundle_equals_the_builtin_pins() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-modules");
        let violations = verify_builtin_release_bundle(&root);
        assert_eq!(violations, vec![], "bundle: {}", root.display());
    }

    #[test]
    fn every_builtin_pin_is_a_canonical_binding() {
        let mut seen = BTreeSet::new();
        for (id, version, integrity) in BUILTIN_PINS {
            assert_eq!(pin_defect(id, version, integrity), None, "{id}@{version}");
            assert!(
                seen.insert((id.to_ascii_lowercase(), *version)),
                "duplicate pin {id}@{version}"
            );
        }
    }
}
