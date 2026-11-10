//! Static declarations and host-environment lookup. No subprocess or environment expansion.
use super::Error;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Declarations { pub declared: bool, pub tools: Vec<Tool>, pub packages: Vec<String>, pub required_tools: Vec<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tool { pub program: String, pub version_requirement: Option<String> }
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub enum Availability { FoundUnprobed, Missing, Unreadable, Unknown }
#[derive(Clone, Debug, Serialize)]
pub struct LocatedTool { pub declaration: Tool, pub availability: Availability, pub candidate: Option<PathBuf>, pub executable: bool }
#[derive(Clone, Debug, Serialize)]
pub struct Report { pub declarations: Declarations, pub tools: Vec<LocatedTool>, pub unknown: bool }
/// Host-only PATH snapshot: never Deserialize or populated from arbitrary renderer environment.
pub struct Environment { paths: Vec<PathBuf> }
fn text(v: &str) -> Result<String, Error> { if v.len() > 256 || v.chars().any(char::is_control) { Err(Error::InvalidFormat) } else { Ok(v.to_owned()) } }
fn program(v: &str) -> Result<String, Error> { if v.is_empty() || v.len() > 128 || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) { Err(Error::InvalidFormat) } else { Ok(v.into()) } }
fn list(value: Option<&serde_yaml::Value>) -> Result<Vec<String>, Error> {
    let Some(value) = value else { return Ok(Vec::new()); };
    let sequence = value.as_sequence().ok_or(Error::InvalidFormat)?;
    if sequence.len() > 128 { return Err(Error::Limit); }
    sequence.iter().map(|v| text(v.as_str().ok_or(Error::InvalidFormat)?)).collect()
}
impl Declarations {
    pub(super) fn parse(value: &serde_yaml::Value) -> Result<Self, Error> {
        let dependencies = value.get("dependencies");
        let mut result = Self { declared: dependencies.is_some() || value.get("required_tools").is_some(), ..Default::default() };
        if let Some(deps) = dependencies {
            let map = deps.as_mapping().ok_or(Error::InvalidFormat)?;
            for (key, value) in map { match key.as_str() {
                Some("python" | "node") => { let key = key.as_str().ok_or(Error::InvalidFormat)?; result.tools.push(Tool { program: if key == "python" { "python3" } else { "node" }.into(), version_requirement: Some(text(value.as_str().ok_or(Error::InvalidFormat)?)?) }); },
                Some("tools") => for tool in list(Some(value))? { result.tools.push(Tool { program: program(&tool)?, version_requirement: None }); },
                Some("packages") => result.packages = list(Some(value))?,
                _ => {},
            } }
        }
        result.required_tools = list(value.get("required_tools"))?;
        Ok(result)
    }
}
impl Environment {
    /// The same inherited PATH and trusted fallback directories used by
    /// session::resolve_executable. Does not invoke a shell or inspect secrets.
    pub fn native_host() -> Result<Self, Error> {
        let mut paths: Vec<PathBuf> = std::env::var_os("PATH").map(|value| std::env::split_paths(&value).filter(|path| path.is_absolute()).collect()).unwrap_or_default();
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            paths.extend([home.join(".local/bin"), home.join(".npm-global/bin"), home.join(".bun/bin")]);
        }
        paths.extend([PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]);
        let mut unique = std::collections::HashSet::new();
        paths.retain(|path| unique.insert(path.clone()));
        Self::from_host(paths)
    }
    pub fn path_value(&self) -> Result<std::ffi::OsString, Error> { std::env::join_paths(&self.paths).map_err(|_| Error::UnsafePath) }
    pub fn from_host(paths: Vec<PathBuf>) -> Result<Self, Error> {
        if paths.len() > 128 || paths.iter().any(|p| !p.is_absolute()) { return Err(Error::UnsafePath); }
        Ok(Self { paths })
    }
    pub fn locate(&self, declarations: &Declarations) -> Report {
        let tools = declarations.tools.iter().map(|tool| {
            let mut availability = Availability::Missing; let mut candidate = None; let mut executable = false;
            for root in &self.paths {
                let path = root.join(&tool.program);
                match std::fs::metadata(&path) {
                    Ok(meta) if meta.is_file() => {
                        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; executable = meta.permissions().mode() & 0o111 != 0; }
                        if executable { availability = Availability::FoundUnprobed; candidate = Some(path); break; }
                        availability = Availability::Unreadable;
                    },
                    Ok(_) => {},
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                    Err(_) => availability = Availability::Unreadable,
                }
            }
            LocatedTool { declaration: tool.clone(), availability, candidate, executable }
        }).collect();
        // Location is never proof of a declared version/package/API requirement.
        Report { declarations: declarations.clone(), tools, unknown: !declarations.declared || !declarations.tools.is_empty() || !declarations.packages.is_empty() || !declarations.required_tools.is_empty() }
    }
}

#[cfg(unix)]
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct VersionExecutable {
    pub path: PathBuf, pub digest: String,
    device: u64, inode: u64, bytes: u64, modified_seconds: i64, modified_nanos: i64, mode: u32,
}
#[cfg(unix)]
impl VersionExecutable {
    pub fn capture(candidate: &std::path::Path) -> Result<Self, Error> {
        use std::{io::Read, os::unix::fs::{MetadataExt, OpenOptionsExt}};
        use sha2::{Digest, Sha256};
        let path = std::fs::canonicalize(candidate).map_err(|_| Error::NotFound)?;
        let mut file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(&path).map_err(|_| Error::AccessDenied)?;
        let before = file.metadata().map_err(|_| Error::Unavailable)?;
        if !before.is_file() || before.mode() & 0o111 == 0 || before.mode() & 0o022 != 0 { return Err(Error::AccessDenied); }
        if before.len() > 128 * 1024 * 1024 { return Err(Error::Limit); }
        let mut digest = Sha256::new(); let mut buffer = [0; 65536]; let mut total = 0u64;
        loop {
            let read = file.read(&mut buffer).map_err(|_| Error::Unavailable)?;
            if read == 0 { break; }
            total += read as u64; if total > 128 * 1024 * 1024 { return Err(Error::Limit); }
            digest.update(&buffer[..read]);
        }
        let after = file.metadata().map_err(|_| Error::Unavailable)?;
        let named = std::fs::metadata(&path).map_err(|_| Error::Conflict)?;
        if before.dev() != after.dev() || before.ino() != after.ino() || before.len() != total || before.mtime() != after.mtime() || before.mtime_nsec() != after.mtime_nsec() || before.mode() != after.mode() || before.ctime() != after.ctime() || before.ctime_nsec() != after.ctime_nsec() || named.dev() != after.dev() || named.ino() != after.ino() || named.mode() != after.mode() { return Err(Error::Conflict); }
        Ok(Self { path, digest: format!("{:x}", digest.finalize()), device: after.dev(), inode: after.ino(), bytes: after.len(), modified_seconds: after.mtime(), modified_nanos: after.mtime_nsec(), mode: after.mode() })
    }
    pub fn validate(&self) -> Result<(), Error> {
        if &Self::capture(&self.path)? != self { return Err(Error::Conflict); }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VersionResult {
    pub status: String, pub output: Option<String>, pub exit_code: Option<i32>,
    pub cleanup_confirmed: bool, pub requirement_satisfied: Option<bool>,
    #[serde(skip)]
    pub process_group: Option<i32>,
}

#[cfg(unix)]
struct VersionGroup { pid: i32, armed: bool }
#[cfg(unix)]
impl Drop for VersionGroup { fn drop(&mut self) { if self.armed { unsafe { libc::kill(-self.pid, libc::SIGKILL); } } } }

/// Explicitly approved host-only execution. No caller supplies argv, cwd, shell,
/// environment values or credentials. Output is evidence, not version matching.
#[cfg(unix)]
pub async fn run_version(executable: &VersionExecutable, environment: &Environment, cwd: &std::path::Path, lease: &tokio_util::sync::CancellationToken, on_spawn: impl FnOnce(i32)) -> Result<VersionResult, Error> {
    use tokio::io::AsyncReadExt;
    use std::{process::Stdio, time::Duration};
    let pinned = executable.clone();
    tokio::task::spawn_blocking(move || pinned.validate()).await.map_err(|_| Error::Unavailable)??;
    if lease.is_cancelled() { return Err(Error::AccessDenied); }
    let mut command = tokio::process::Command::new(&executable.path);
    command.arg("--version").current_dir(cwd).env_clear().env("PATH", environment.path_value()?).env("NO_COLOR", "1")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).process_group(0);
    let mut child = command.spawn().map_err(|_| Error::Unavailable)?;
    let mut group = VersionGroup { pid: child.id().ok_or(Error::Unavailable)? as i32, armed: true };
    on_spawn(group.pid);
    let mut stdout = child.stdout.take().ok_or(Error::Unavailable)?;
    let mut bytes = Vec::new();
    let work = async {
        (&mut stdout).take(4097).read_to_end(&mut bytes).await.map_err(|_| Error::Unavailable)?;
        if bytes.len() > 4096 { return Ok::<_, Error>(("output_limit", None)); }
        // Observe exit without reaping: the unreaped leader reserves its PID
        // until cleanup has signalled the owned group, so it cannot be recycled.
        loop {
            let (pid, exit_code) = {
                let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
                if unsafe { libc::waitid(libc::P_PID, group.pid as libc::id_t, &mut status, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } != 0 { return Err(Error::Unavailable); }
                (unsafe { status.si_pid() }, (status.si_code == libc::CLD_EXITED).then(|| unsafe { status.si_status() }))
            };
            if pid != 0 { break Ok((if exit_code == Some(0) { "observed" } else { "failed" }, exit_code)); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let outcome = tokio::select! {
        _ = lease.cancelled() => Ok(("cancelled", None)),
        value = tokio::time::timeout(Duration::from_secs(5), work) => match value { Ok(value) => value, Err(_) => Ok(("timeout", None)) },
    };
    // Even a successful launcher can leave descendants or inherited stdout open.
    unsafe { libc::kill(-group.pid, libc::SIGKILL); }
    group.armed = false;
    let cleanup_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let reaped = tokio::time::timeout_at(cleanup_deadline, child.wait()).await.is_ok_and(|result| result.is_ok());
    let gone = loop {
        let gone = unsafe { libc::kill(-group.pid, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if gone || tokio::time::Instant::now() >= cleanup_deadline { break gone; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let (status, exit_code) = outcome.unwrap_or(("failed", None));
    let output = if status == "observed" { String::from_utf8(bytes).ok().map(|text| text.chars().filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t').collect()) } else { None };
    Ok(VersionResult { status: status.into(), output, exit_code, cleanup_confirmed: reaped && gone, requirement_satisfied: None, process_group: Some(group.pid) })
}

/// Re-observe only a group created by run_version. Never signal a retained PID:
/// once its original child was reaped a recycled numeric PID grants no authority.
#[cfg(unix)]
pub fn inspect_version_cleanup(result: &mut VersionResult) {
    if result.cleanup_confirmed { return; }
    if let Some(group) = result.process_group {
        if unsafe { libc::kill(-group, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) { result.cleanup_confirmed = true; }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};
    fn executable(root: &std::path::Path, body: &str) -> PathBuf {
        let path = root.join("fixture-version-tool");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    #[tokio::test]
    async fn static_scan_and_preview_do_not_execute_but_approved_probe_uses_fixed_argv() {
        let temp = tempfile::tempdir().unwrap();
        let path = executable(temp.path(), "[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 91\nprintf '%s' \"$1\" > invoked\nprintf 'fixture 1.2.3\\n'");
        let declarations = Declarations { declared: true, tools: vec![Tool { program: "fixture-version-tool".into(), version_requirement: Some(">=1".into()) }], ..Default::default() };
        let environment = Environment::from_host(vec![temp.path().to_owned()]).unwrap();
        let report = environment.locate(&declarations);
        assert_eq!(report.tools[0].availability, Availability::FoundUnprobed);
        let pinned = VersionExecutable::capture(&path).unwrap();
        assert!(!temp.path().join("invoked").exists());
        let result = run_version(&pinned, &environment, temp.path(), &tokio_util::sync::CancellationToken::new(), |_| {}).await.unwrap();
        assert_eq!(result.status, "observed"); assert_eq!(result.output.as_deref(), Some("fixture 1.2.3\n"));
        assert_eq!(fs::read_to_string(temp.path().join("invoked")).unwrap(), "--version");
        assert!(result.cleanup_confirmed); assert_eq!(result.requirement_satisfied, None);
        fs::write(&path, b"#!/bin/sh\nprintf changed").unwrap();
        assert_eq!(pinned.validate().unwrap_err(), Error::Conflict);
    }
    #[tokio::test]
    async fn approved_probe_bounds_output_and_cancels_its_owned_group() {
        let temp = tempfile::tempdir().unwrap();
        let path = executable(temp.path(), "i=0; while [ \"$i\" -lt 5000 ]; do printf x; i=$((i+1)); done");
        let environment = Environment::from_host(vec![temp.path().to_owned()]).unwrap();
        let result = run_version(&VersionExecutable::capture(&path).unwrap(), &environment, temp.path(), &tokio_util::sync::CancellationToken::new(), |_| {}).await.unwrap();
        assert_eq!(result.status, "output_limit"); assert!(result.output.is_none()); assert!(result.cleanup_confirmed);
        executable(temp.path(), "printf started > started; /bin/sleep 20");
        let pinned = VersionExecutable::capture(&path).unwrap();
        let lease = tokio_util::sync::CancellationToken::new();
        let cancellation = async { tokio::time::sleep(std::time::Duration::from_millis(50)).await; lease.cancel(); };
        let (result, _) = tokio::join!(run_version(&pinned, &environment, temp.path(), &lease, |_| {}), cancellation);
        let result = result.unwrap();
        assert_eq!(result.status, "cancelled"); assert!(result.cleanup_confirmed);
    }
    #[tokio::test]
    async fn revoked_probe_never_starts_a_child() {
        let temp = tempfile::tempdir().unwrap(); let path = executable(temp.path(), "printf forbidden > forbidden");
        let lease = tokio_util::sync::CancellationToken::new(); lease.cancel();
        let result = run_version(&VersionExecutable::capture(&path).unwrap(), &Environment::from_host(vec![temp.path().to_owned()]).unwrap(), temp.path(), &lease, |_| {}).await;
        assert_eq!(result.unwrap_err(), Error::AccessDenied); assert!(!temp.path().join("forbidden").exists());
    }
    #[test]
    fn executable_capture_rejects_special_files_without_waiting_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap(); let fifo = root.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o700) }, 0);
        assert_eq!(VersionExecutable::capture(&fifo).unwrap_err(), Error::AccessDenied);
    }
}
