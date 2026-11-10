//! Cleanup is restricted to one randomly named/labeled probe-owned container.
//! Neither configuration nor arbitrary Docker commands enter this interface.
use crate::{McpError, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{io::Read, process::Stdio, sync::atomic::{AtomicBool, Ordering}};
use tokio::{io::AsyncReadExt, process::{Child, Command}};

#[derive(Clone, Serialize)]
pub struct ContainerCleanupPlan {
    pub executable: String, pub name: String, pub label: String,
    pub inspect_argv: Vec<String>, pub remove_argv_prefix: Vec<String>,
    pub remove_target: &'static str,
}
#[derive(Clone)]
pub struct ContainerCleanupSpec { pub plan: ContainerCleanupPlan, pub binary_sha256: String }
pub(super) struct OwnedContainerCleanup {
    spec: ContainerCleanupSpec, environment: Vec<(String, String)>, cwd: String, started: AtomicBool,
}
struct OwnedCommand { child: Child, group: Option<u32> }
impl Drop for OwnedCommand {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.group { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
        let _ = self.child.start_kill();
    }
}
impl OwnedContainerCleanup {
    pub(super) fn new(spec: ContainerCleanupSpec, environment: Vec<(String, String)>, cwd: String) -> Self { Self { spec, environment, cwd, started: AtomicBool::new(false) } }
    pub(super) fn acknowledge_start(&self) { self.started.store(true, Ordering::Release); }
    async fn command(&self, args: &[String]) -> Result<Vec<u8>> {
        let executable = self.spec.plan.executable.clone(); let expected = self.spec.binary_sha256.clone();
        let valid = tokio::task::spawn_blocking(move || {
            let mut options = std::fs::OpenOptions::new(); options.read(true);
            #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC); }
            let mut file = options.open(&executable).ok()?;
            let before = file.metadata().ok()?;
            if !before.is_file() || before.len() > 128 * 1024 * 1024 { return None; }
            let mut hasher = Sha256::new(); let mut bytes = [0_u8; 32 * 1024]; let mut total = 0_u64;
            loop { let count = file.read(&mut bytes).ok()?; if count == 0 { break; } total += count as u64; if total > 128 * 1024 * 1024 { return None; } hasher.update(&bytes[..count]); }
            let after = file.metadata().ok()?; let named = std::fs::symlink_metadata(&executable).ok()?;
            if before.len() != total || before.len() != after.len() || before.modified().ok()? != after.modified().ok()? || named.file_type().is_symlink() { return None; }
            #[cfg(unix)] {
                use std::os::unix::fs::MetadataExt;
                if before.dev() != after.dev() || before.ino() != after.ino() || named.dev() != after.dev() || named.ino() != after.ino() || before.mode() != after.mode() || before.ctime() != after.ctime() || before.ctime_nsec() != after.ctime_nsec() { return None; }
            }
            Some(format!("{:x}", hasher.finalize()) == expected)
        }).await.map_err(|_| McpError::ConnectionFailed("cleanup binary unavailable".into()))?;
        if valid != Some(true) { return Err(McpError::InvalidConfig("cleanup binary changed".into())); }
        let mut command = Command::new(&self.spec.plan.executable);
        command.args(args).current_dir(&self.cwd).env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        for name in ["PATH", "HOME", "USERPROFILE", "SystemRoot", "TMPDIR", "TEMP", "LANG"] { if let Some(value) = std::env::var_os(name) { command.env(name, value); } }
        for (name, value) in &self.environment { command.env(name, value); }
        #[cfg(unix)]
        command.process_group(0);
        let child = command.spawn().map_err(|_| McpError::ConnectionFailed("cleanup executable unavailable".into()))?;
        let mut owned = OwnedCommand { group: child.id(), child };
        let stdout = owned.child.stdout.take().ok_or_else(|| McpError::Transport("cleanup output unavailable".into()))?;
        let mut bytes = Vec::new();
        stdout.take(4097).read_to_end(&mut bytes).await.map_err(|_| McpError::Transport("cleanup read failed".into()))?;
        if bytes.len() > 4096 { return Err(McpError::Protocol("cleanup output limit".into())); }
        if !owned.child.wait().await.map_err(|_| McpError::Transport("cleanup wait failed".into()))?.success() { return Err(McpError::Transport("daemon cleanup unconfirmed".into())); }
        owned.group = None;
        Ok(bytes)
    }
    pub(super) async fn cleanup(&self) -> bool {
        let Ok(bytes) = self.command(&self.spec.plan.inspect_argv).await else { return false; };
        let Ok(output) = std::str::from_utf8(&bytes) else { return false; };
        let ids: Vec<_> = output.lines().filter(|line| !line.is_empty()).collect();
        if ids.is_empty() { return self.started.load(Ordering::Acquire); }
        if ids.len() != 1 || ids[0].len() != 64 || !ids[0].bytes().all(|byte| byte.is_ascii_hexdigit()) { return false; }
        // Name AND random label were checked by the exact approved query. Remove
        // the resulting immutable container ID, never an arbitrary user name.
        self.acknowledge_start();
        let mut remove = self.spec.plan.remove_argv_prefix.clone(); remove.push(ids[0].to_owned());
        let _ = self.command(&remove).await;
        matches!(self.command(&self.spec.plan.inspect_argv).await, Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace))
    }
}

/// Pure plan derivation; no daemon contact or process execution occurs here.
pub fn container_plan(executable: &str, args: &[String], nonce: &str, binary_sha256: String) -> Result<(Vec<String>, ContainerCleanupSpec)> {
    if args.first().map(String::as_str) != Some("run") || !args.iter().any(|argument| argument == "--rm")
        || args.iter().any(|argument| ["--name", "--label", "--label-file", "--cidfile", "--detach", "--restart", "--rm=false", "--rm=0"].iter().any(|flag| argument.strip_prefix(*flag).is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('='))) || argument.starts_with('-') && !argument.starts_with("--") && argument[1..].contains(['d', 'l']))
        || nonce.len() != 36 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit() || byte == b'-') {
        return Err(McpError::InvalidConfig("docker probe requires unowned, non-detached docker run --rm".into()));
    }
    let name = format!("dgos-native-probe-{nonce}"); let label = format!("io.dgos.native.probe={nonce}");
    let mut effective = vec!["run".into(), "--name".into(), name.clone(), "--label".into(), label.clone()];
    if !args.iter().any(|argument| matches!(argument.as_str(), "-i" | "--interactive")) { effective.push("-i".into()); }
    effective.extend(args[1..].iter().cloned());
    let inspect_argv = vec!["container".into(), "ls".into(), "--all".into(), "--no-trunc".into(), "--filter".into(), format!("name=^/{name}$"), "--filter".into(), format!("label={label}"), "--format".into(), "{{.ID}}".into()];
    Ok((effective, ContainerCleanupSpec { plan: ContainerCleanupPlan { executable: executable.into(), name, label, inspect_argv, remove_argv_prefix: vec!["rm".into(), "--force".into(), "--volumes".into()], remove_target: "immutable_id_verified_by_exact_name_and_label" }, binary_sha256 }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_derives_owned_non_detached_run_plans_without_execution() {
        let nonce = "00000000-0000-4000-8000-000000000001";
        for args in [vec!["ps"], vec!["run", "image"], vec!["run", "--rm", "--name=external", "image"], vec!["run", "--rm", "-d", "image"], vec!["run", "--rm", "--label", "external", "image"], vec!["run", "--rm", "-lio.dgos.native.probe=external", "image"], vec!["run", "--rm", "-itlother=external", "image"]] {
            assert!(container_plan("/not-executed/docker", &args.into_iter().map(str::to_owned).collect::<Vec<_>>(), nonce, "hash".into()).is_err());
        }
        let (args, cleanup) = container_plan("/not-executed/docker", &["run".into(), "--rm".into(), "-i".into(), "image:1".into()], nonce, "hash".into()).unwrap();
        assert_eq!(&args[..5], ["run", "--name", "dgos-native-probe-00000000-0000-4000-8000-000000000001", "--label", "io.dgos.native.probe=00000000-0000-4000-8000-000000000001"]);
        assert!(cleanup.plan.inspect_argv.iter().any(|argument| argument == "label=io.dgos.native.probe=00000000-0000-4000-8000-000000000001"));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cleanup_removes_only_the_verified_immutable_id_and_requires_daemon_confirmation() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("docker");
        let nonce = "00000000-0000-4000-8000-000000000001";
        let id = "a".repeat(64);
        let script = format!("#!/bin/sh\ncase \"$1\" in\ncontainer) [ \"$#\" -eq 10 ] && [ \"$6\" = 'name=^/dgos-native-probe-{nonce}$' ] && [ \"$8\" = 'label=io.dgos.native.probe={nonce}' ] || exit 91; if [ -f active ]; then printf '%s\\n' '{id}'; fi;;\nrm) [ \"$#\" -eq 4 ] && [ \"$2\" = --force ] && [ \"$3\" = --volumes ] && [ \"$4\" = '{id}' ] || exit 92; printf '%s' \"$4\" > removed; /bin/rm active;;\n*) exit 93;;\nesac\n");
        std::fs::write(&executable, &script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(root.path().join("active"), b"owned").unwrap();
        let (_, spec) = container_plan(executable.to_str().unwrap(), &["run".into(), "--rm".into(), "image".into()], nonce, format!("{:x}", Sha256::digest(script.as_bytes()))).unwrap();
        let cleanup = OwnedContainerCleanup::new(spec, vec![], root.path().to_string_lossy().into_owned());
        assert!(tokio::time::timeout(std::time::Duration::from_secs(2), cleanup.cleanup()).await.unwrap());
        assert_eq!(std::fs::read_to_string(root.path().join("removed")).unwrap(), id);
        assert!(cleanup.cleanup().await);
        std::fs::write(&executable, b"#!/bin/sh\nexit 1\n").unwrap();
        assert!(!cleanup.cleanup().await, "a changed executable cannot assert daemon cleanup");
    }
}
