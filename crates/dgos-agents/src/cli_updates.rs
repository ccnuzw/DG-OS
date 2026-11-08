//! Read-only source checks and single-use, source-preserving CLI updates.
use crate::cli::{probe, CliProbe, CliTool};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{atomic::{AtomicBool, Ordering}, Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::OwnedMutexGuard};

const CHECK_DEADLINE: Duration = Duration::from_secs(25);
const UPDATE_DEADLINE: Duration = Duration::from_secs(180);
const JSON_LIMIT: usize = 2 * 1024 * 1024;
const MANIFEST_LIMIT: usize = 256 * 1024;
const INSTALL_OUTPUT_LIMIT: usize = 8192;
static INSTALL_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn try_installation_lock(key: &Path) -> Option<OwnedMutexGuard<()>> {
    let lock = INSTALL_LOCKS.lock().unwrap_or_else(|p| p.into_inner()).entry(key.to_owned()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone();
    lock.try_lock_owned().ok()
}


#[derive(Clone, Debug, Serialize)]
pub struct CheckResult {
    pub status: &'static str,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub method: Option<String>,
    pub ticket: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub tool: CliTool,
    pub path: PathBuf,
    pub current: String,
    pub latest: String,
    pub package: &'static str,
    installation: Installation,
    context: Context,
    used: Arc<AtomicBool>,
}

// Explicit paths keep fixtures isolated: no process-global PATH/HOME changes and
// no test can accidentally fall through to a workstation's package manager.
#[derive(Clone, Debug)]
struct Context {
    home: Option<PathBuf>,
    npm: Option<PathBuf>,
    brew: Option<PathBuf>,
    #[cfg(test)]
    releases: Option<PathBuf>,
}
impl Context {
    fn discover() -> Self {
        Self {
            home: std::env::var_os("HOME").map(PathBuf::from),
            npm: crate::session::resolve_executable("npm", None).ok(),
            brew: crate::session::resolve_executable("brew", None).ok(),
            #[cfg(test)]
            releases: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    real: PathBuf,
    stamp: String,
    launcher: String,
    digest: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Manager {
    path: PathBuf,
    identity: FileIdentity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrewKind { Cask, Formula }
impl BrewKind {
    fn flag(self) -> &'static str { match self { Self::Cask => "--cask", Self::Formula => "--formula" } }
    fn collection(self) -> &'static str { match self { Self::Cask => "casks", Self::Formula => "formulae" } }
    fn directory(self) -> &'static str { match self { Self::Cask => "Caskroom", Self::Formula => "Cellar" } }
    fn api(self) -> &'static str { match self { Self::Cask => "cask", Self::Formula => "formula" } }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    Npm { manager: Manager, prefix: PathBuf, manifest: Vec<u8> },
    Brew { manager: Manager, prefix: PathBuf, kind: BrewKind, package: &'static str, installed: String },
    Native { channel: &'static str },
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Installation {
    root: PathBuf,
    executable: FileIdentity,
    source: Source,
}
impl Installation {
    fn method(&self) -> &'static str { match &self.source { Source::Npm { .. } => "npm", Source::Brew { .. } => "homebrew", Source::Native { .. } => "native" } }
    fn package(&self, tool: CliTool) -> &'static str { match &self.source { Source::Brew { package, .. } => package, Source::Native { .. } => "claude", Source::Npm { .. } => npm_package(tool).unwrap_or("") } }
    fn lock_root(&self) -> &Path { match &self.source { Source::Npm { prefix, .. } | Source::Brew { prefix, .. } => prefix, Source::Native { .. } => &self.root } }
}

fn npm_package(tool: CliTool) -> Option<&'static str> {
    match tool { CliTool::Codex => Some("@openai/codex"), CliTool::Claude => Some("@anthropic-ai/claude-code"), CliTool::Gemini => Some("@google/gemini-cli"), CliTool::Node => None }
}
fn semver(value: &str) -> Option<semver::Version> { semver::Version::parse(value.strip_prefix('v').unwrap_or(value)).ok() }
fn extract_version(tool: CliTool, displayed: &str) -> &str {
    match tool { CliTool::Codex => displayed.strip_prefix("codex-cli ").unwrap_or(""), CliTool::Claude => displayed.strip_suffix(" (Claude Code)").unwrap_or(""), CliTool::Gemini => displayed, CliTool::Node => "" }
}

#[cfg(unix)]
struct ProcessGroup(i32);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) { unsafe { libc::kill(-self.0, libc::SIGKILL); } }
}
#[derive(Debug, PartialEq, Eq)]
enum CommandError { Start, Failed }

// Capture the process-group id before wait clears Child::id. Both child and group
// are owned by the future, including while draining stdout, so cancellation kills
// descendants too. Drain all stdout, retain only the cap; installers can be chatty.
async fn command(program: &Path, args: &[&str], install: bool, home: Option<&Path>) -> Result<Vec<u8>, CommandError> {
    command_with_deadline(program, args, install, home, if install { UPDATE_DEADLINE } else { CHECK_DEADLINE }).await
}
async fn command_with_deadline(program: &Path, args: &[&str], install: bool, home: Option<&Path>, deadline: Duration) -> Result<Vec<u8>, CommandError> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    // No manager auto-refresh, analytics or unrelated post-upgrade cleanup.
    cmd.env("HOMEBREW_NO_AUTO_UPDATE", "1").env("HOMEBREW_NO_ANALYTICS", "1")
        .env("HOMEBREW_NO_INSTALL_CLEANUP", "1").env("HOMEBREW_API_AUTO_UPDATE_SECS", "0")
        .env("npm_config_update_notifier", "false");
    if let Some(home) = home { cmd.env("HOME", home); }
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|_| CommandError::Start)?;
    #[cfg(unix)]
    let group = ProcessGroup(child.id().ok_or(CommandError::Start)? as i32);
    let mut stdout = child.stdout.take().ok_or(CommandError::Start)?;
    let limit = if install { INSTALL_OUTPUT_LIMIT } else { JSON_LIMIT };
    let operation = async move {
        let drain = async move {
            let mut bytes = Vec::new();
            let mut truncated = false;
            let mut buffer = [0u8; 16384];
            loop {
                let count = stdout.read(&mut buffer).await.map_err(|_| CommandError::Failed)?;
                if count == 0 { break; }
                let retained = count.min(limit.saturating_sub(bytes.len()));
                bytes.extend_from_slice(&buffer[..retained]);
                truncated |= retained < count;
            }
            if truncated && !install { return Err(CommandError::Failed); }
            Ok(bytes)
        };
        let wait = async move {
            let status = child.wait().await;
            #[cfg(unix)]
            drop(group);
            if status.is_ok_and(|s| s.success()) { Ok(()) } else { Err(CommandError::Failed) }
        };
        let (bytes, ()) = tokio::try_join!(drain, wait)?;
        Ok(bytes)
    };
    tokio::time::timeout(deadline, operation).await.map_err(|_| CommandError::Failed)?
}

fn stamp(metadata: &std::fs::Metadata) -> String {
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}:{}:{}:{}:{}:{}:{}", metadata.dev(), metadata.ino(), metadata.mode(), metadata.len(), metadata.mtime(), metadata.mtime_nsec(), metadata.ctime(), metadata.ctime_nsec())
    }
    #[cfg(not(unix))] { format!("{}:{:?}:{:?}", metadata.len(), metadata.modified(), metadata.permissions()) }
}
fn file_identity(path: &Path) -> Result<FileIdentity, ()> {
    let launcher_metadata = std::fs::symlink_metadata(path).map_err(|_| ())?;
    let launcher = format!("{}:{:?}", stamp(&launcher_metadata), std::fs::read_link(path).ok());
    let real = std::fs::canonicalize(path).map_err(|_| ())?;
    let mut file = std::fs::File::open(&real).map_err(|_| ())?;
    let before = file.metadata().map_err(|_| ())?;
    if !before.is_file() || before.len() > 512 * 1024 * 1024 { return Err(()); }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| ())?;
        if count == 0 { break; }
        total += count as u64;
        if total > 512 * 1024 * 1024 { return Err(()); }
        digest.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|_| ())?;
    if stamp(&before) != stamp(&after) || total != after.len() { return Err(()); }
    Ok(FileIdentity { real, stamp: stamp(&after), launcher, digest: digest.finalize().to_vec() })
}
async fn file_identity_async(path: &Path) -> Result<FileIdentity, ()> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || file_identity(&path)).await.map_err(|_| ())?
}
async fn manager(path: &Path) -> Result<Manager, ()> { Ok(Manager { path: path.to_owned(), identity: file_identity_async(path).await? }) }
fn bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>, ()> {
    let file = std::fs::File::open(path).map_err(|_| ())?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|_| ())?;
    if bytes.len() > limit { return Err(()); }
    Ok(bytes)
}
fn package_bin_matches(manifest: &Value, tool: CliTool, package_root: &Path, executable: &Path) -> bool {
    if manifest.get("name").and_then(Value::as_str) != npm_package(tool) { return false; }
    let target = match manifest.get("bin") {
        Some(Value::String(path)) => path.as_str(),
        Some(Value::Object(map)) => map.get(tool.name()).and_then(Value::as_str).unwrap_or(""),
        _ => "",
    };
    let relative = Path::new(target);
    if target.is_empty() || relative.components().any(|part| !matches!(part, Component::Normal(_) | Component::CurDir)) { return false; }
    let Ok(target) = std::fs::canonicalize(package_root.join(relative)) else { return false; };
    target.starts_with(package_root) && target == executable
}
fn native_channel(home: &Path) -> Result<&'static str, ()> {
    let settings = home.join(".claude/settings.json");
    if !settings.try_exists().map_err(|_| ())? { return Ok("latest"); }
    let settings: Value = serde_json::from_slice(&bounded_file(&settings, MANIFEST_LIMIT)?).map_err(|_| ())?;
    match settings.get("autoUpdatesChannel").and_then(Value::as_str) {
        None if settings.get("autoUpdatesChannel").is_none() => Ok("latest"),
        Some("latest") => Ok("latest"), Some("stable") => Ok("stable"), _ => Err(()),
    }
}
fn native_root(home: &Path, path: &Path, real: &Path) -> Option<PathBuf> {
    if path != home.join(".local/bin/claude") || !std::fs::symlink_metadata(path).ok()?.file_type().is_symlink() { return None; }
    let root = std::fs::canonicalize(home.join(".local/share/claude/versions")).ok()?;
    if real.parent()? != root || semver(real.file_name()?.to_str()?).is_none() { return None; }
    Some(root)
}

async fn command_path(manager: &Manager, args: &[&str]) -> Result<PathBuf, ()> {
    let bytes = command(&manager.identity.real, args, false, None).await.map_err(|_| ())?;
    let value = std::str::from_utf8(&bytes).map_err(|_| ())?.trim();
    let path = Path::new(value);
    if !path.is_absolute() || value.contains('\n') || value.contains('\r') { return Err(()); }
    std::fs::canonicalize(path).map_err(|_| ())
}
async fn npm_installation(context: &Context, tool: CliTool, path: &Path, executable: &FileIdentity) -> Result<Option<Installation>, ()> {
    let (Some(program), Some(package)) = (&context.npm, npm_package(tool)) else { return Ok(None); };
    let manager = manager(program).await?;
    let prefix = command_path(&manager, &["prefix", "--global"]).await?;
    let root = command_path(&manager, &["root", "--global"]).await?;
    let expected_root = if cfg!(windows) { prefix.join("node_modules") } else { prefix.join("lib/node_modules") };
    if std::fs::canonicalize(expected_root).ok().as_ref() != Some(&root) { return Ok(None); }
    let launcher = if cfg!(windows) { prefix.join(format!("{}.cmd", tool.name())) } else { prefix.join("bin").join(tool.name()) };
    if path != launcher { return Ok(None); }
    let Ok(package_root) = std::fs::canonicalize(root.join(package)) else { return Ok(None); };
    if !package_root.starts_with(&root) || !executable.real.starts_with(&package_root) { return Ok(None); }
    let manifest = bounded_file(&package_root.join("package.json"), MANIFEST_LIMIT)?;
    let parsed: Value = serde_json::from_slice(&manifest).map_err(|_| ())?;
    if parsed.get("name").and_then(Value::as_str) != Some(package) || !package_bin_matches(&parsed, tool, &package_root, &executable.real) { return Ok(None); }
    Ok(Some(Installation { root: package_root, executable: executable.clone(), source: Source::Npm { manager, prefix, manifest } }))
}

fn brew_packages(tool: CliTool) -> &'static [(BrewKind, &'static str)] {
    match tool {
        CliTool::Codex => &[(BrewKind::Cask, "codex"), (BrewKind::Formula, "codex")],
        CliTool::Claude => &[(BrewKind::Cask, "claude-code")],
        CliTool::Gemini => &[(BrewKind::Formula, "gemini-cli")],
        CliTool::Node => &[],
    }
}
fn official_brew(value: &Value, kind: BrewKind, package: &str) -> bool {
    let (name, full, tap) = match kind { BrewKind::Cask => ("token", "full_token", "homebrew/cask"), BrewKind::Formula => ("name", "full_name", "homebrew/core") };
    value.get(name).and_then(Value::as_str) == Some(package)
        && value.get("tap").and_then(Value::as_str) == Some(tap)
        && value.get(full).and_then(Value::as_str).is_some_and(|name| name == package || name == format!("{tap}/{package}"))
}
fn brew_version(value: &Value, kind: BrewKind) -> Option<&str> {
    match kind { BrewKind::Cask => value.get("version")?.as_str(), BrewKind::Formula => value.get("versions")?.get("stable")?.as_str() }
}
fn brew_owned(prefix: &Path, kind: BrewKind, package: &str, tool: CliTool, path: &Path, real: &Path, metadata: &Value) -> Option<(PathBuf, String)> {
    if !official_brew(metadata, kind, package) { return None; }
    // A Cellar/Caskroom versioned launcher disappears during upgrade. Only the
    // stable prefix/bin or opt/<formula>/bin entry point is confirmable.
    if path != prefix.join("bin").join(tool.name()) && !(kind == BrewKind::Formula && path == prefix.join("opt").join(package).join("bin").join(tool.name())) { return None; }
    let root = std::fs::canonicalize(prefix.join(kind.directory()).join(package)).ok()?;
    let relative = real.strip_prefix(&root).ok()?;
    let installed = relative.components().next()?.as_os_str().to_str()?;
    if semver(installed).is_none() { return None; }
    let listed = match kind {
        BrewKind::Cask => metadata.get("installed").and_then(Value::as_str) == Some(installed)
            || metadata.get("installed").and_then(Value::as_array).is_some_and(|items| items.iter().any(|v| v.as_str() == Some(installed))),
        BrewKind::Formula => metadata.get("installed").and_then(Value::as_array).is_some_and(|items| items.iter().any(|v| v.get("version").and_then(Value::as_str) == Some(installed))),
    };
    listed.then(|| (root, installed.to_owned()))
}
async fn brew_info(manager: &Manager, kind: BrewKind, package: &str) -> Result<Value, ()> {
    let bytes = command(&manager.identity.real, &["info", "--json=v2", kind.flag(), package], false, None).await.map_err(|_| ())?;
    let parsed: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    let collection = parsed.get(kind.collection()).and_then(Value::as_array).ok_or(())?;
    if collection.len() != 1 || !official_brew(&collection[0], kind, package) { return Err(()); }
    Ok(collection[0].clone())
}
async fn brew_installation(context: &Context, tool: CliTool, path: &Path, executable: &FileIdentity) -> Result<Option<Installation>, ()> {
    let Some(program) = &context.brew else { return Ok(None); };
    let manager = manager(program).await?;
    let prefix = command_path(&manager, &["--prefix"]).await?;
    for &(kind, package) in brew_packages(tool) {
        let package_root = prefix.join(kind.directory()).join(package);
        if !executable.real.starts_with(&package_root) { continue; }
        let metadata = brew_info(&manager, kind, package).await?;
        if let Some((root, installed)) = brew_owned(&prefix, kind, package, tool, path, &executable.real, &metadata) {
            return Ok(Some(Installation { root, executable: executable.clone(), source: Source::Brew { manager, prefix, kind, package, installed } }));
        }
    }
    Ok(None)
}
async fn detect(context: &Context, tool: CliTool, path: &Path) -> Result<Option<Installation>, ()> {
    let executable = file_identity_async(path).await?;
    if matches!(tool, CliTool::Claude) {
        if let Some(home) = &context.home {
            if let Some(root) = native_root(home, path, &executable.real) {
                return Ok(Some(Installation { root, executable, source: Source::Native { channel: native_channel(home)? } }));
            }
        }
    }
    // Managers unrelated to this path must not prevent classifying the actual
    // owner (a broken brew does not disable a valid npm installation).
    let brew = brew_installation(context, tool, path, &executable).await;
    if let Ok(Some(installation)) = &brew { return Ok(Some(installation.clone())); }
    let npm = npm_installation(context, tool, path, &executable).await;
    if let Ok(Some(installation)) = npm { return Ok(Some(installation)); }
    if brew.is_err() || npm.is_err() { Err(()) } else { Ok(None) }
}

async fn release_bytes(context: &Context, key: &str, url: &str) -> Result<Vec<u8>, ()> {
    #[cfg(test)]
    if let Some(root) = &context.releases { return bounded_file(&root.join(key), JSON_LIMIT); }
    #[cfg(not(test))]
    let _ = (context, key);
    // No redirects: every release request has a fixed, official HTTPS origin.
    let client = reqwest::Client::builder().https_only(true).redirect(reqwest::redirect::Policy::none()).timeout(CHECK_DEADLINE).build().map_err(|_| ())?;
    tokio::time::timeout(CHECK_DEADLINE, async {
        let mut response = client.get(url).send().await.map_err(|_| ())?.error_for_status().map_err(|_| ())?;
        if response.content_length().is_some_and(|length| length > JSON_LIMIT as u64) { return Err(()); }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if bytes.len().saturating_add(chunk.len()) > JSON_LIMIT { return Err(()); }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }).await.map_err(|_| ())?
}
async fn latest(context: &Context, tool: CliTool, installation: &Installation) -> Result<String, ()> {
    let version = match &installation.source {
        Source::Npm { .. } => {
            let package = npm_package(tool).ok_or(())?;
            let key = format!("npm-{}", tool.name());
            let url = format!("https://registry.npmjs.org/{}/latest", package.replace('/', "%2f"));
            let metadata: Value = serde_json::from_slice(&release_bytes(context, &key, &url).await?).map_err(|_| ())?;
            if metadata.get("name").and_then(Value::as_str) != Some(package) { return Err(()); }
            metadata.get("version").and_then(Value::as_str).ok_or(())?.to_owned()
        }
        Source::Brew { kind, package, .. } => {
            let key = format!("brew-{}-{package}", kind.api());
            let url = format!("https://formulae.brew.sh/api/{}/{package}.json", kind.api());
            let metadata: Value = serde_json::from_slice(&release_bytes(context, &key, &url).await?).map_err(|_| ())?;
            if !official_brew(&metadata, *kind, package) || metadata.get("disabled").and_then(Value::as_bool) == Some(true) { return Err(()); }
            brew_version(&metadata, *kind).ok_or(())?.to_owned()
        }
        Source::Native { channel } => {
            // Official bootstrap.sh's DOWNLOAD_BASE_URL; these endpoints contain
            // a plain version, not npm dist-tags (stable can intentionally lag).
            let key = format!("claude-{channel}");
            let url = format!("https://downloads.claude.ai/claude-code-releases/{channel}");
            String::from_utf8(release_bytes(context, &key, &url).await?).map_err(|_| ())?.trim().to_owned()
        }
    };
    if semver(&version).is_none() { return Err(()); }
    Ok(version)
}
fn result(status: &'static str, current_version: Option<String>, latest_version: Option<String>, method: Option<&str>) -> CheckResult {
    CheckResult { status, current_version, latest_version, method: method.map(str::to_owned), ticket: None }
}

pub async fn check(tool: CliTool, requested_path: Option<&str>) -> (CheckResult, Option<Candidate>) {
    check_with(&Context::discover(), tool, requested_path).await
}
async fn check_with(context: &Context, tool: CliTool, requested_path: Option<&str>) -> (CheckResult, Option<Candidate>) {
    check_with_deadline(context, tool, requested_path, CHECK_DEADLINE).await
}
async fn check_with_deadline(context: &Context, tool: CliTool, requested_path: Option<&str>, deadline: Duration) -> (CheckResult, Option<Candidate>) {
    match tokio::time::timeout(deadline, check_with_inner(context, tool, requested_path)).await {
        Ok(result) => result,
        Err(_) => (result("check_failed", None, None, None), None),
    }
}
async fn check_with_inner(context: &Context, tool: CliTool, requested_path: Option<&str>) -> (CheckResult, Option<Candidate>) {
    if npm_package(tool).is_none() { return (result("unsupported", None, None, None), None); }
    let before = probe(tool, requested_path).await;
    if before.status != "available" { return (result("not_installed", before.version, None, None), None); }
    let Some(path) = before.path.as_deref().map(PathBuf::from) else { return (result("check_failed", before.version, None, None), None); };
    let installation = match detect(context, tool, &path).await {
        Ok(Some(installation)) => installation,
        Ok(None) => return (result("unsupported", before.version, None, None), None),
        Err(()) => return (result("check_failed", before.version, None, None), None),
    };
    let method = installation.method();
    let Ok(latest) = latest(context, tool, &installation).await else { return (result("check_failed", before.version, None, Some(method)), None); };
    let Some(current) = before.version.as_deref().and_then(|value| semver(extract_version(tool, value))) else { return (result("check_failed", before.version, None, Some(method)), None); };
    if semver(&latest).is_none_or(|latest| latest <= current) { return (result("up_to_date", before.version, Some(latest), Some(method)), None); }
    let candidate = Candidate {
        tool, path, current: before.version.clone().unwrap_or_default(), latest,
        package: installation.package(tool),
        installation, context: context.clone(), used: Arc::new(AtomicBool::new(false)),
    };
    (result("available", before.version, Some(candidate.latest.clone()), Some(method)), Some(candidate))
}


/// Revalidates the launcher, manager, install root, version and remote target while
/// holding the installation lock. A consumed confirmation is never replayable,
/// including when an installer fails or its final state is unknown.
pub async fn update(candidate: &Candidate) -> Result<CliProbe, &'static str> {
    if candidate.used.swap(true, Ordering::AcqRel) { return Err("ticket_consumed"); }
    let _serial = try_installation_lock(candidate.installation.lock_root()).ok_or("busy")?;
    let actual = detect(&candidate.context, candidate.tool, &candidate.path).await.map_err(|_| "identity_changed")?.ok_or("identity_changed")?;
    if actual != candidate.installation { return Err("identity_changed"); }
    let before = probe(candidate.tool, candidate.path.to_str()).await;
    if before.status != "available" || before.version.as_deref() != Some(&candidate.current) { return Err("identity_changed"); }
    if latest(&candidate.context, candidate.tool, &actual).await.map_err(|_| "check_failed")? != candidate.latest { return Err("target_changed"); }
    if let Source::Brew { manager, kind, package, .. } = &actual.source {
        let metadata = brew_info(manager, *kind, package).await.map_err(|_| "check_failed")?;
        if brew_version(&metadata, *kind) != Some(&candidate.latest) || metadata.get("pinned").and_then(Value::as_bool) == Some(true) { return Err("target_changed"); }
    }
    // Close the metadata-check window: no command executes after a launcher or
    // manager change, even if its displayed version remained unchanged.
    if file_identity_async(&candidate.path).await.map_err(|_| "identity_changed")? != actual.executable { return Err("identity_changed"); }
    let outcome = match &actual.source {
        Source::Npm { manager, prefix, .. } => {
            if file_identity_async(&manager.path).await.map_err(|_| "identity_changed")? != manager.identity { return Err("identity_changed"); }
            let spec = format!("{}@{}", candidate.package, candidate.latest);
            let prefix = prefix.to_str().ok_or("identity_changed")?;
            command(&manager.identity.real, &["install", "--global", "--prefix", prefix, "--registry=https://registry.npmjs.org", "--", &spec], true, None).await
        }
        Source::Brew { manager, kind, package, .. } => {
            if file_identity_async(&manager.path).await.map_err(|_| "identity_changed")? != manager.identity { return Err("identity_changed"); }
            command(&manager.identity.real, &["upgrade", kind.flag(), package], true, None).await
        }
        Source::Native { .. } => {
            // `claude update` respects autoUpdatesChannel without rewriting any
            // settings. `claude install <version>` is official too, but changes
            // install/channel semantics and is deliberately not used here.
            command(&actual.executable.real, &["update"], true, candidate.context.home.as_deref()).await
        }
    };
    match outcome { Err(CommandError::Start) => return Err("install_failed"), Err(CommandError::Failed) => return Err("outcome_unknown"), Ok(_) => {} }
    let after = probe(candidate.tool, candidate.path.to_str()).await;
    let version = after.version.as_deref().map(|v| extract_version(candidate.tool, v));
    // A native updater can race the endpoint. Never claim the confirmed version
    // was installed when it selected a different target.
    if after.status != "available" || version != Some(candidate.latest.as_str()) { return Err("outcome_unknown"); }
    let installed = detect(&candidate.context, candidate.tool, &candidate.path).await.map_err(|_| "outcome_unknown")?.ok_or("outcome_unknown")?;
    if installed.root != actual.root || installed.method() != actual.method() { return Err("outcome_unknown"); }
    let same_source = match (&actual.source, &installed.source) {
        (Source::Npm { manager: a, prefix: ap, .. }, Source::Npm { manager: b, prefix: bp, .. }) => a == b && ap == bp,
        (Source::Brew { manager: a, prefix: ap, kind: ak, package: an, .. }, Source::Brew { manager: b, prefix: bp, kind: bk, package: bn, .. }) => a == b && ap == bp && ak == bk && an == bn,
        (Source::Native { channel: a }, Source::Native { channel: b }) => a == b,
        _ => false,
    };
    if !same_source { return Err("outcome_unknown"); }
    if let Source::Npm { manifest, .. } = &installed.source {
        let metadata: Value = serde_json::from_slice(manifest).map_err(|_| "outcome_unknown")?;
        if metadata.get("version").and_then(Value::as_str) != Some(candidate.latest.as_str()) { return Err("outcome_unknown"); }
    }
    Ok(after)
}

#[cfg(test)]
#[path = "cli_update_tests.rs"]
mod tests;
