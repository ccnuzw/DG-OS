//! Exposes the locked React version to the crate (D060).
//!
//! `package-lock.json` is the single source of truth for the React that is
//! actually bundled into the frontend. Reading it here means the version the
//! host uses to judge `requires.react` can never drift from the shipped one.

use std::{env, fs, path::PathBuf, process::exit};

fn fail(message: &str) -> ! {
    eprintln!("error: dgos-modules build script: {message}");
    exit(1);
}

fn main() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").unwrap_or_else(|| fail("CARGO_MANIFEST_DIR is not set")),
    );
    let lockfile = manifest_dir.join("../../package-lock.json");
    println!("cargo:rerun-if-changed={}", lockfile.display());
    println!("cargo:rerun-if-changed=build.rs");

    let bytes = fs::read(&lockfile).unwrap_or_else(|error| {
        fail(&format!(
            "cannot read {} ({error}); run `npm install` to create the lockfile",
            lockfile.display()
        ))
    });
    let lock: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        fail(&format!(
            "{} is not valid JSON: {error}",
            lockfile.display()
        ))
    });
    let version = lock
        .get("packages")
        .and_then(|packages| packages.get("node_modules/react"))
        .and_then(|entry| entry.get("version"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| {
            fail(&format!(
                "{} has no packages[\"node_modules/react\"].version",
                lockfile.display()
            ))
        });
    // An exact version only: a range, tag or empty value would silently
    // change what `requires.react` is checked against.
    let parsed = semver::Version::parse(version).unwrap_or_else(|error| {
        fail(&format!(
            "react version {version:?} in {} is not an exact semver version: {error}",
            lockfile.display()
        ))
    });
    if parsed.to_string() != version {
        fail(&format!(
            "react version {version:?} in {} is not canonical semver",
            lockfile.display()
        ));
    }
    println!("cargo:rustc-env=DGOS_HOST_REACT_VERSION={version}");
}
