//! Host version compatibility for module manifests (D055).
//!
//! A manifest may declare `requires: { "api": <int>, "react": "<semver range>" }`.
//! Absent fields mean "no constraint". This is a compatibility notice, not a
//! security boundary: it only compares declared values.

use semver::{Version, VersionReq};
use serde::{Deserialize, Deserializer, Serialize};

/// The module host API version (`host.apiVersion`) this build provides.
pub const HOST_API_VERSION: u32 = 1;

/// The React version this host injects as `host.React` (D060). `build.rs`
/// reads it from the `node_modules/react` entry of the root
/// `package-lock.json`, so it is the exact version the frontend bundles.
pub const HOST_REACT_VERSION: &str = env!("DGOS_HOST_REACT_VERSION");

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<u32>,
    /// A semver range, validated when the manifest is parsed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_react_range"
    )]
    pub react: Option<String>,
}

fn deserialize_react_range<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if let Some(range) = &value {
        VersionReq::parse(range).map_err(<D::Error as serde::de::Error>::custom)?;
    }
    Ok(value)
}

/// `Ok` when the declared requirements hold for this host. Absent or empty
/// requirements are compatible.
pub(crate) fn check(requires: Option<&Requires>) -> Result<(), ()> {
    let host = Version::parse(HOST_REACT_VERSION).map_err(|_| ())?;
    check_against(requires, &host)
}

/// `check` with an explicit host React version, so range semantics at minor
/// and patch level can be tested without depending on the locked version.
pub(crate) fn check_against(requires: Option<&Requires>, host_react: &Version) -> Result<(), ()> {
    let Some(requires) = requires else {
        return Ok(());
    };
    if requires.api.is_some_and(|api| api != HOST_API_VERSION) {
        return Err(());
    }
    if let Some(range) = &requires.react {
        let range = VersionReq::parse(range).map_err(|_| ())?;
        if !range.matches(host_react) {
            return Err(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requires(api: Option<u32>, react: Option<&str>) -> Requires {
        Requires {
            api,
            react: react.map(str::to_owned),
        }
    }

    #[test]
    fn absent_and_empty_requirements_are_compatible() {
        assert!(check(None).is_ok());
        assert!(check(Some(&Requires::default())).is_ok());
    }

    #[test]
    fn react_range_is_matched_against_the_host_version() {
        assert!(check(Some(&requires(None, Some("^19")))).is_ok());
        assert!(check(Some(&requires(None, Some(">=19.0.0, <20.0.0")))).is_ok());
        // AC02: a module built for React 20 is refused on a React 19 host.
        assert!(check(Some(&requires(None, Some("^20")))).is_err());
        assert!(check(Some(&requires(None, Some("^18")))).is_err());
    }

    #[test]
    fn api_must_equal_the_host_api_version() {
        assert!(check(Some(&requires(Some(HOST_API_VERSION), None))).is_ok());
        assert!(check(Some(&requires(Some(HOST_API_VERSION + 1), None))).is_err());
        assert!(check(Some(&requires(Some(HOST_API_VERSION + 1), Some("^19")))).is_err());
    }

    #[test]
    fn invalid_semver_range_fails_to_parse() {
        let parsed =
            serde_json::from_value::<Requires>(serde_json::json!({"react": "not a range"}));
        assert!(parsed.is_err());
        let parsed = serde_json::from_value::<Requires>(serde_json::json!({"react": "^19"}));
        assert_eq!(parsed.unwrap().react.as_deref(), Some("^19"));
    }

    #[test]
    fn unknown_requirement_fields_are_rejected() {
        let parsed = serde_json::from_value::<Requires>(serde_json::json!({"node": "20"}));
        assert!(parsed.is_err());
    }

    fn host(version: &str) -> Version {
        Version::parse(version).unwrap()
    }

    fn accepts(host_version: &str, range: &str) -> bool {
        check_against(Some(&requires(None, Some(range))), &host(host_version)).is_ok()
    }

    #[test]
    fn host_react_version_is_the_exact_locked_version() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let read = |path: &str| -> serde_json::Value {
            let bytes = std::fs::read(format!("{root}/{path}"))
                .unwrap_or_else(|error| panic!("{path}: {error}"));
            serde_json::from_slice(&bytes).unwrap()
        };
        let lock = read("package-lock.json");
        let locked = lock["packages"]["node_modules/react"]["version"]
            .as_str()
            .expect("package-lock.json has no node_modules/react version");
        assert_eq!(HOST_REACT_VERSION, locked);
        let installed = read("node_modules/react/package.json");
        assert_eq!(
            installed["version"].as_str(),
            Some(HOST_REACT_VERSION),
            "node_modules/react differs from package-lock.json; run `npm ci`"
        );
        assert!(Version::parse(HOST_REACT_VERSION).unwrap().pre.is_empty());
    }

    #[test]
    fn minor_and_patch_drift_changes_the_outcome() {
        // The old hand-written 19.1.0 against the locked 19.3.1.
        assert!(!accepts("19.1.0", ">=19.2"));
        assert!(accepts("19.3.1", ">=19.2"));
        assert!(accepts("19.1.0", "<19.2"));
        assert!(!accepts("19.3.1", "<19.2"));
        assert!(accepts("19.1.0", "^19.1"));
        assert!(accepts("19.3.1", "^19.1"));
        assert!(!accepts("19.1.0", "~19.3.0"));
        assert!(accepts("19.3.1", "~19.3.0"));
        assert!(!accepts("19.3.1", "~19.1.0"));
        assert!(!accepts("19.1.0", "19.3.1"));
        assert!(accepts("19.3.1", "19.3.1"));
        assert!(accepts("19.1.0", "=19.1.0"));
        assert!(!accepts("19.3.1", "=19.1.0"));
        assert!(!accepts("19.3.1", "^20"));
        assert!(!accepts("19.3.1", ">19.3.1"));
        assert!(accepts("19.3.1", ">=19.3.1, <19.4.0"));
    }

    #[test]
    fn prerelease_hosts_only_match_ranges_that_name_the_same_triple() {
        // semver: a prerelease satisfies a comparator only when that comparator
        // carries a prerelease on the same major.minor.patch.
        assert!(!accepts("19.2.0-rc.1", ">=19.1"));
        assert!(!accepts("19.2.0-rc.1", "^19"));
        assert!(accepts("19.2.0-rc.1", ">=19.2.0-rc.0"));
        assert!(accepts("19.2.0-rc.1", "=19.2.0-rc.1"));
        assert!(!accepts("19.2.0-rc.1", ">=19.2.0"));
        assert!(accepts("19.2.0", ">=19.2.0-rc.0"));
    }

    #[test]
    fn api_check_still_applies_with_an_injected_host_version() {
        let wrong_api = requires(Some(HOST_API_VERSION + 1), Some("^19"));
        assert!(check_against(Some(&wrong_api), &host("19.3.1")).is_err());
        let ok = requires(Some(HOST_API_VERSION), Some("^19"));
        assert!(check_against(Some(&ok), &host("19.3.1")).is_ok());
        assert!(check_against(None, &host("19.3.1")).is_ok());
    }
}
