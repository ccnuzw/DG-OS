//! Release gate entry point (D047, D049).
//!
//! `cargo run -p dgos-modules --example verify_release_bundle -- <bundle-dir>`
//!
//! Recomputes every package digest under `<bundle-dir>/<module_id>/<version>/`
//! and compares it with the pins compiled into this build. Exit code 0 means
//! the directory equals the pin list; 1 means at least one violation; 2 means
//! bad usage. Run it against the unpacked release artifact, not only the
//! source tree.

use dgos_modules::verify_builtin_release_bundle;
use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let (Some(root), None) = (args.next(), args.next()) else {
        eprintln!("usage: verify_release_bundle <bundle-dir>");
        return ExitCode::from(2);
    };
    let violations = verify_builtin_release_bundle(&PathBuf::from(root));
    if violations.is_empty() {
        println!("release bundle matches the compiled-in pins");
        return ExitCode::SUCCESS;
    }
    for violation in &violations {
        eprintln!("{violation:?}");
    }
    eprintln!("{} release gate violation(s)", violations.len());
    ExitCode::from(1)
}
