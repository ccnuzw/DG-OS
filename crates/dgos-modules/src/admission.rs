//! Execution admission for module packages (D041).
//!
//! A package digest only proves the package equals what its own manifest
//! declares, so it says nothing about who published it. Admission is the
//! separate, host-owned decision that a `(module_id, version, integrity)`
//! may run code. It is the only security boundary for module execution: the
//! WebView realm is shared with the host (D042).

use std::collections::HashSet;

/// Host policy deciding whether a verified package may execute.
pub trait ModuleAdmission: Send + Sync {
    fn admits(&self, module_id: &str, version: &str, integrity: &str) -> bool;
}

/// Admission from a fixed list compiled into or injected by the host.
/// Integrity is compared case-insensitively. An empty list admits nothing.
#[derive(Clone, Debug, Default)]
pub struct PinnedModuleAdmission {
    pins: HashSet<(String, String, String)>,
}

impl PinnedModuleAdmission {
    /// The production list (D041). Empty until a release process supplies
    /// real pins; nothing executes in production before that.
    pub fn builtin() -> Self {
        Self::new(BUILTIN_PINS.iter().copied())
    }

    pub fn new<'a>(pins: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>) -> Self {
        Self {
            pins: pins
                .into_iter()
                .map(|(id, version, integrity)| {
                    (
                        id.to_owned(),
                        version.to_owned(),
                        integrity.to_ascii_lowercase(),
                    )
                })
                .collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.pins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
    }
}

impl ModuleAdmission for PinnedModuleAdmission {
    fn admits(&self, module_id: &str, version: &str, integrity: &str) -> bool {
        self.pins.contains(&(
            module_id.to_owned(),
            version.to_owned(),
            integrity.to_ascii_lowercase(),
        ))
    }
}

/// `(module_id, version, sha256)` of packages shipped with this build. Every
/// entry must equal the digest the host recomputes from the shipped bundle
/// (`release_gate`); the shape and artifact gates run in `cargo test`.
pub(crate) const BUILTIN_PINS: &[(&str, &str, &str)] = &[(
    "provider-console",
    "1.0.0",
    "v2:6d50d95a70ebfb98d727891d9961c04c9328173c72467573c1f1ca3eec5abe2a",
)];

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "aabbcc";

    #[test]
    fn only_an_exact_pin_is_admitted() {
        let admission = PinnedModuleAdmission::new([("mod-a", "1.0.0", DIGEST)]);
        assert!(admission.admits("mod-a", "1.0.0", DIGEST));
        assert!(admission.admits("mod-a", "1.0.0", "AABBCC"));
        assert!(!admission.admits("mod-a", "1.0.1", DIGEST));
        assert!(!admission.admits("mod-b", "1.0.0", DIGEST));
        assert!(!admission.admits("mod-a", "1.0.0", "aabbcd"));
    }

    #[test]
    fn production_list_admits_exactly_the_bundled_pins() {
        let admission = PinnedModuleAdmission::builtin();
        assert_eq!(admission.len(), BUILTIN_PINS.len());
        for (id, version, integrity) in BUILTIN_PINS {
            assert!(admission.admits(id, version, integrity), "{id}@{version}");
            // Neither another digest, another version nor another id rides along.
            assert!(!admission.admits(id, version, DIGEST));
            assert!(!admission.admits(id, "0.0.0-unpinned", integrity));
            assert!(!admission.admits("not-bundled", version, integrity));
        }
        assert!(!admission.admits("mod-a", "1.0.0", DIGEST));
    }

    #[test]
    fn a_new_pin_does_not_retire_an_older_version_and_dropping_a_pin_does() {
        // Version replacement is additive, so rollback (D033) stays startable.
        let both =
            PinnedModuleAdmission::new([("mod-a", "1.0.0", DIGEST), ("mod-a", "1.1.0", "ddeeff")]);
        assert!(both.admits("mod-a", "1.0.0", DIGEST));
        assert!(both.admits("mod-a", "1.1.0", "ddeeff"));
        // A version's digest is not interchangeable with another version's.
        assert!(!both.admits("mod-a", "1.1.0", DIGEST));
        // Revocation in V1 is removing the pin from the next build.
        let retired = PinnedModuleAdmission::new([("mod-a", "1.1.0", "ddeeff")]);
        assert!(!retired.admits("mod-a", "1.0.0", DIGEST));
    }

    // The shape and artifact checks for `BUILTIN_PINS` live in
    // `release_gate` so the pin list and the shipped directory are compared
    // by one gate.
}
