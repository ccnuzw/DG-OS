#![cfg(unix)]
use super::*;
use std::{os::unix::fs::{symlink, PermissionsExt}, sync::atomic::{AtomicU64, Ordering}};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-cli-updates-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&root).unwrap();
        Self(std::fs::canonicalize(root).unwrap())
    }
    fn path(&self, name: &str) -> PathBuf { self.0.join(name) }
    fn executable(&self, path: &Path, text: &str) {
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).unwrap(); }
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn context(&self) -> Context { Context { home: Some(self.path("home")), npm: None, brew: None, releases: Some(self.0.clone()) } }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn version_script(version: &str) -> String { format!("#!/bin/sh\n[ \"$1\" = --version ] || exit 2\nprintf '%s\\n' 'codex-cli {version}'\n") }

#[tokio::test]
async fn full_check_deadline_cancels_a_slow_version_probe() {
    let f = Fixture::new();
    let launcher = f.path("codex");
    f.executable(&launcher, "#!/bin/sh\nexec sleep 10\n");
    let started = tokio::time::Instant::now();
    let (checked, candidate) = check_with_deadline(&f.context(), CliTool::Codex, launcher.to_str(), Duration::from_millis(50)).await;
    assert_eq!(checked.status, "check_failed");
    assert!(candidate.is_none());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn version_ordering_handles_prereleases_and_current_targets() {
    assert!(semver("1.2.0").unwrap() > semver("1.2.0-rc.1").unwrap());
    assert!(semver("1.2.0-rc.2").unwrap() > semver("1.2.0-rc.1").unwrap());
    assert_eq!(semver("v2.0.0").unwrap(), semver("2.0.0").unwrap());
    assert!(semver("1.2").is_none());
    assert!(semver("latest").is_none());
}

#[test]
fn scoped_npm_bin_ownership_requires_official_name_and_exact_bin_target() {
    let f = Fixture::new();
    let root = f.path("prefix/lib/node_modules/@openai/codex");
    let bin = root.join("bin/codex.js");
    f.executable(&bin, "#!/bin/sh\nexit 0\n");
    let launcher = f.path("prefix/bin/codex");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    symlink(&bin, &launcher).unwrap();
    let manifest: Value = serde_json::from_str(r#"{"name":"@openai/codex","version":"1.0.0","bin":{"codex":"bin/codex.js"}}"#).unwrap();
    let real_root = std::fs::canonicalize(&root).unwrap();
    let real_bin = std::fs::canonicalize(&bin).unwrap();
    assert!(package_bin_matches(&manifest, CliTool::Codex, &real_root, &real_bin));
    let wrong_name: Value = serde_json::from_str(r#"{"name":"attacker/codex","bin":{"codex":"bin/codex.js"}}"#).unwrap();
    assert!(!package_bin_matches(&wrong_name, CliTool::Codex, &real_root, &real_bin));
    let traversal: Value = serde_json::from_str(r#"{"name":"@openai/codex","bin":{"codex":"../../outside"}}"#).unwrap();
    assert!(!package_bin_matches(&traversal, CliTool::Codex, &real_root, &real_bin));
}

#[test]
fn brew_ownership_requires_official_package_metadata_and_stable_launcher() {
    let f = Fixture::new();
    let prefix = f.path("brew");
    let root = prefix.join("Caskroom/codex");
    let installed = root.join("1.0.0/bin/codex");
    f.executable(&installed, "fixture");
    let launcher = prefix.join("bin/codex");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    symlink(&installed, &launcher).unwrap();
    let metadata: Value = serde_json::from_str(r#"{"token":"codex","full_token":"codex","tap":"homebrew/cask","installed":"1.0.0"}"#).unwrap();
    let real = std::fs::canonicalize(&installed).unwrap();
    assert!(brew_owned(&prefix, BrewKind::Cask, "codex", CliTool::Codex, &launcher, &real, &metadata).is_some());
    assert!(brew_owned(&prefix, BrewKind::Cask, "codex", CliTool::Codex, &installed, &real, &metadata).is_none());
    let untrusted: Value = serde_json::from_str(r#"{"token":"codex","full_token":"codex","tap":"third-party/tap","installed":"1.0.0"}"#).unwrap();
    assert!(brew_owned(&prefix, BrewKind::Cask, "codex", CliTool::Codex, &launcher, &real, &untrusted).is_none());
}

#[test]
fn brew_formula_ownership_requires_cellar_version_and_stable_launcher() {
    let f = Fixture::new();
    let prefix = f.path("brew");
    let root = prefix.join("Cellar/gemini-cli");
    let installed = root.join("1.0.0/bin/gemini");
    f.executable(&installed, "fixture");
    let launcher = prefix.join("bin/gemini");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    symlink(&installed, &launcher).unwrap();
    let metadata: Value = serde_json::from_str(r#"{"name":"gemini-cli","full_name":"gemini-cli","tap":"homebrew/core","installed":[{"version":"1.0.0"}]}"#).unwrap();
    let real = std::fs::canonicalize(&installed).unwrap();
    assert!(brew_owned(&prefix, BrewKind::Formula, "gemini-cli", CliTool::Gemini, &launcher, &real, &metadata).is_some());
    let wrong_tap: Value = serde_json::from_str(r#"{"name":"gemini-cli","full_name":"gemini-cli","tap":"third-party/tap","installed":[{"version":"1.0.0"}]}"#).unwrap();
    assert!(brew_owned(&prefix, BrewKind::Formula, "gemini-cli", CliTool::Gemini, &launcher, &real, &wrong_tap).is_none());
}

#[test]
fn native_claude_requires_launcher_symlink_and_preserves_release_channel() {
    let f = Fixture::new();
    let home = f.path("home");
    let versioned = home.join(".local/share/claude/versions/2.1.0");
    f.executable(&versioned, "native fixture");
    let launcher = home.join(".local/bin/claude");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    symlink(&versioned, &launcher).unwrap();
    let real = std::fs::canonicalize(&launcher).unwrap();
    assert_eq!(native_root(&home, &launcher, &real), Some(std::fs::canonicalize(home.join(".local/share/claude/versions")).unwrap()));
    assert_eq!(native_channel(&home).unwrap(), "latest");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(home.join(".claude/settings.json"), r#"{"autoUpdatesChannel":"stable","env":{"token":"must not be logged"}}"#).unwrap();
    assert_eq!(native_channel(&home).unwrap(), "stable");
    assert!(native_root(&home, &versioned, &real).is_none());
}

async fn npm_fixture(f: &Fixture, initial: &str, latest: &str, fails: bool) -> (Context, PathBuf) {
    let context = f.context();
    let home = context.home.as_ref().unwrap();
    std::fs::create_dir_all(home).unwrap();
    let prefix = f.path("npm-prefix");
    let root = prefix.join("lib/node_modules");
    let package = root.join("@openai/codex");
    let binary = package.join("bin/codex.js");
    f.executable(&binary, &version_script(initial));
    std::fs::write(package.join("package.json"), format!(r#"{{"name":"@openai/codex","version":"{initial}","bin":{{"codex":"bin/codex.js"}}}}"#)).unwrap();
    let launcher = prefix.join("bin/codex");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    symlink(&binary, &launcher).unwrap();
    let npm = f.path("npm");
    let fail_arg = if fails { "exit 9".to_owned() } else {
        let replacement = f.path("replacement-codex");
        f.executable(&replacement, &version_script(latest));
        let manifest = f.path("replacement-manifest");
        std::fs::write(&manifest, format!(r#"{{"name":"@openai/codex","version":"{latest}","bin":{{"codex":"bin/codex.js"}}}}"#)).unwrap();
        format!("cp '{}' '{}'; cp '{}' '{}'", replacement.display(), binary.display(), manifest.display(), package.join("package.json").display())
    };
    let body = format!(r#"#!/bin/sh
case "$*" in
  "prefix --global") printf '%s\n' '{}';;
  "root --global") printf '%s\n' '{}';;
  "install --global --prefix {} --registry=https://registry.npmjs.org -- @openai/codex@{}") {};;
  *) exit 8;;
esac
"#, prefix.display(), root.display(), prefix.display(), latest, fail_arg);
    f.executable(&npm, &body);
    let mut context = context;
    context.npm = Some(npm);
    std::fs::write(f.path("npm-codex"), format!(r#"{{"name":"@openai/codex","version":"{latest}"}}"#)).unwrap();
    (context, launcher)
}

#[tokio::test]
async fn isolated_npm_update_replaces_version_and_postverifies() {
    let f = Fixture::new();
    let (context, launcher) = npm_fixture(&f, "1.0.0", "1.1.0", false).await;
    let (checked, candidate) = check_with(&context, CliTool::Codex, launcher.to_str()).await;
    assert_eq!(checked.status, "available");
    let candidate = candidate.unwrap();
    std::fs::write(f.path("npm-codex"), r#"{"name":"@openai/codex","version":"1.1.0"}"#).unwrap();
    assert_eq!(update(&candidate).await.unwrap().version.as_deref(), Some("codex-cli 1.1.0"));
    assert_eq!(update(&candidate).await.unwrap_err(), "ticket_consumed");
}

#[tokio::test]
async fn failed_install_is_unknown_and_confirmation_cannot_be_replayed() {
    let f = Fixture::new();
    let (context, launcher) = npm_fixture(&f, "1.0.0", "1.1.0", true).await;
    let (_, candidate) = check_with(&context, CliTool::Codex, launcher.to_str()).await;
    let candidate = candidate.unwrap();
    assert_eq!(update(&candidate).await.unwrap_err(), "outcome_unknown");
    assert_eq!(update(&candidate).await.unwrap_err(), "ticket_consumed");
    assert_eq!(probe(CliTool::Codex, launcher.to_str()).await.version.as_deref(), Some("codex-cli 1.0.0"));
}

#[tokio::test]
async fn current_official_version_has_no_candidate() {
    let f = Fixture::new();
    let (context, launcher) = npm_fixture(&f, "1.1.0", "1.1.0", false).await;
    let (checked, candidate) = check_with(&context, CliTool::Codex, launcher.to_str()).await;
    assert_eq!(checked.status, "up_to_date");
    assert!(candidate.is_none());
}

#[tokio::test]
async fn chatty_installer_stdout_is_drained_without_exceeding_capture_limit() {
    let f = Fixture::new();
    let program = f.path("chatty");
    f.executable(&program, "#!/bin/sh\ni=0; while [ $i -lt 20000 ]; do printf x; i=$((i+1)); done\n");
    let output = command_with_deadline(&program, &[], true, None, Duration::from_secs(5)).await.unwrap();
    assert_eq!(output.len(), INSTALL_OUTPUT_LIMIT);
}

#[tokio::test]
async fn native_update_preserves_channel_and_never_claims_a_raced_target() {
    for installed in ["2.1.0", "2.2.0"] {
        let f = Fixture::new();
        let context = f.context();
        let home = context.home.as_ref().unwrap();
        let root = home.join(".local/share/claude/versions");
        let old = root.join("2.0.0");
        let new = root.join(installed);
        let launcher = home.join(".local/bin/claude");
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let settings = r#"{"autoUpdatesChannel":"stable"}"#;
        std::fs::write(home.join(".claude/settings.json"), settings).unwrap();
        f.executable(&new, &format!("#!/bin/sh\n[ \"$1\" = --version ] || exit 2\nprintf '{installed} (Claude Code)\\n'\n"));
        f.executable(&old, &format!("#!/bin/sh\ncase \"$1\" in\n--version) printf '2.0.0 (Claude Code)\\n';;\nupdate) rm '{}' && ln -s '{}' '{}';;\n*) exit 2;;\nesac\n", launcher.display(), new.display(), launcher.display()));
        symlink(&old, &launcher).unwrap();
        std::fs::write(f.path("claude-stable"), "2.1.0\n").unwrap();
        let (checked, candidate) = check_with(&context, CliTool::Claude, launcher.to_str()).await;
        assert_eq!(checked.status, "available");
        assert_eq!(checked.method, Some("native".into()));
        let candidate = candidate.unwrap();
        let result = update(&candidate).await;
        if installed == "2.1.0" {
            assert_eq!(result.unwrap().version.as_deref(), Some("2.1.0 (Claude Code)"));
        } else {
            assert_eq!(result.unwrap_err(), "outcome_unknown");
        }
        assert_eq!(std::fs::read_to_string(home.join(".claude/settings.json")).unwrap(), settings);
        assert_eq!(update(&candidate).await.unwrap_err(), "ticket_consumed");
    }
}

#[tokio::test]
async fn homebrew_update_follows_the_stable_launcher_and_rechecks_ownership() {
    let f = Fixture::new();
    let mut context = f.context();
    let prefix = f.path("brew-prefix");
    let old = prefix.join("Caskroom/codex/1.0.0/bin/codex");
    let new = prefix.join("Caskroom/codex/1.1.0/bin/codex");
    let launcher = prefix.join("bin/codex");
    f.executable(&old, &version_script("1.0.0"));
    f.executable(&new, &version_script("1.1.0"));
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    symlink(&old, &launcher).unwrap();
    let before = r#"{"token":"codex","full_token":"codex","tap":"homebrew/cask","version":"1.1.0","installed":"1.0.0"}"#;
    let after = before.replace("\"installed\":\"1.0.0\"", "\"installed\":\"1.1.0\"");
    let metadata = f.path("brew-metadata");
    let replacement = f.path("brew-metadata-after");
    std::fs::write(&metadata, format!("{{\"casks\":[{before}]}}")).unwrap();
    std::fs::write(&replacement, format!("{{\"casks\":[{after}]}}")).unwrap();
    std::fs::write(f.path("brew-cask-codex"), before).unwrap();
    let brew = f.path("brew");
    f.executable(&brew, &format!("#!/bin/sh\ncase \"$*\" in\n--prefix) printf '%s\\n' '{}';;\n'info --json=v2 --cask codex') cat '{}';;\n'upgrade --cask codex') cp '{}' '{}' && rm '{}' && ln -s '{}' '{}';;\n*) exit 2;;\nesac\n", prefix.display(), metadata.display(), replacement.display(), metadata.display(), launcher.display(), new.display(), launcher.display()));
    context.brew = Some(brew);
    let (checked, candidate) = check_with(&context, CliTool::Codex, launcher.to_str()).await;
    assert_eq!(checked.status, "available");
    assert_eq!(checked.method, Some("homebrew".into()));
    let candidate = candidate.unwrap();
    let updated = update(&candidate).await.unwrap();
    assert_eq!(updated.version.as_deref(), Some("codex-cli 1.1.0"));
    assert_eq!(updated.path.as_deref(), launcher.to_str());
    assert_eq!(update(&candidate).await.unwrap_err(), "ticket_consumed");
}
