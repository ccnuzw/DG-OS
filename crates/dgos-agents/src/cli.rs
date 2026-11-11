//! D133 bounded CLI version probes. Fixed argv, no shell, no session or account side effects.
use crate::diagnostics::ProcessDiagnostic;
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::{Child, Command}};

const OUTPUT_LIMIT: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(5);

/// Reference version used to document and test the App Server adapter, not a startup requirement.
#[derive(Debug, Serialize)]
pub struct CodexProtocolReference { pub version: &'static str }
pub fn codex_protocol_reference() -> &'static CodexProtocolReference {
    static REFERENCE: CodexProtocolReference = CodexProtocolReference { version: "0.167.0" };
    &REFERENCE
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliTool { Codex, Claude, Gemini, Node }

impl CliTool {
    pub fn name(self) -> &'static str { match self { Self::Codex => "codex", Self::Claude => "claude", Self::Gemini => "gemini", Self::Node => "node" } }
    /// Accepts only the single `--version` line that this tool prints. Any other text is an unsupported response.
    fn accepts(self, line: &str) -> bool {
        match self {
            Self::Codex => line.strip_prefix("codex-cli ").is_some_and(is_semver),
            Self::Claude => line.strip_suffix(" (Claude Code)").is_some_and(is_semver),
            Self::Gemini => is_semver(line),
            Self::Node => line.strip_prefix('v').is_some_and(is_semver),
        }
    }
}

fn is_semver(text: &str) -> bool {
    let mut parts = text.split('.');
    let number = |part: Option<&str>| part.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    number(parts.next()) && number(parts.next()) && number(parts.next()) && parts.next().is_none()
}

#[derive(Debug, Serialize)]
pub struct CliProbe {
    pub tool: CliTool, pub status: &'static str, pub path: Option<String>, pub version: Option<String>,
    pub executable: bool, pub managed: bool,
}
impl CliProbe {
    /// Version discovery only qualifies the executable for a startup attempt; the App Server handshake validates protocol compatibility.
    pub fn startup_error(&self) -> Option<&'static str> {
        match self.status {
            "available" => None,
            "not_installed" => Some("cli_not_installed"),
            "not_executable" => Some("cli_not_executable"),
            "timeout" => Some("cli_probe_timeout"),
            "start_failed" => Some("cli_start_failed"),
            _ => Some("cli_probe_failed"),
        }
    }
}

fn finish(tool: CliTool, path: Option<String>, status: &'static str, version: Option<String>) -> CliProbe {
    CliProbe { tool, status, path, version, executable: status == "available", managed: false }
}


// Version wrappers may spawn descendants that retain stdout. Keep the leader
// unreaped until its dedicated group is signalled, including on cancellation:
// reaping first would let the PID/PGID be recycled before cleanup uses it.
#[cfg(unix)]
struct ProbeGroup { pid: i32, armed: bool, diagnostic: ProcessDiagnostic }
#[cfg(unix)]
fn group_kill_outcome(error: Option<i32>) -> (&'static str, &'static str) {
    match error {
        None => ("requested", "none"),
        Some(libc::ESRCH) => ("absent", "none"),
        Some(_) => ("failed", "signal_failed"),
    }
}

// Darwin returns EPERM for a group containing only its exited leader. Keep
// that leader unreaped (pinning the PGID), and require kernel proof that no
// other group member remains. Permission failures without this proof stay failures.
#[cfg(target_os = "macos")]
fn group_has_only_exited_leader(leader: i32) -> bool {
    let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
    if unsafe { libc::waitid(libc::P_PID, leader as libc::id_t, &mut status, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } != 0 || unsafe { status.si_pid() } != leader { return false; }
    let mut pids = [0i32; 2];
    // Unlike proc_listpids, proc_listpgrppids returns a PID count, not bytes.
    let count = unsafe { libc::proc_listpgrppids(leader, pids.as_mut_ptr().cast(), std::mem::size_of_val(&pids) as i32) };
    count == 1 && pids[0] == leader
}
#[cfg(unix)]
impl ProbeGroup {
    fn stop(&mut self) -> bool {
        if !self.armed { return true; }
        self.armed = false;
        let error = if unsafe { libc::kill(-self.pid, libc::SIGKILL) } == 0 { None } else { Some(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)) };
        let (outcome, error_class) = group_kill_outcome(error);
        #[cfg(target_os = "macos")]
        let (outcome, error_class) = if error == Some(libc::EPERM) && group_has_only_exited_leader(self.pid) { ("stopped", "none") } else { (outcome, error_class) };
        self.diagnostic.record("group_kill", Some(self.pid as u32), outcome, error_class);
        error_class == "none"
    }
}
#[cfg(unix)]
impl Drop for ProbeGroup { fn drop(&mut self) { self.stop(); } }

/// Runs `<executable> --version` with a fixed argv, stdin closed, 5 s limit and 4 KiB stdout limit.
/// Managed installers do not exist, so the result is never `managed`.
pub async fn probe(tool: CliTool, explicit: Option<&str>) -> CliProbe {
    let diagnostic = ProcessDiagnostic::new("version_probe", tool.name());
    let operation = diagnostic.phase("probe", None);
    let discovery = diagnostic.phase("discovery", None);
    let explicit = explicit.filter(|p| !p.is_empty());
    let path = match crate::session::resolve_executable(tool.name(), explicit) {
        Ok(path) => path,
        Err(_) => {
            let exists = explicit.is_some_and(|p| Path::new(p).exists());
            let status = if exists { "not_executable" } else { "not_installed" };
            discovery.finish(Some(status));
            operation.finish(Some(status));
            return finish(tool, explicit.map(str::to_owned), status, None);
        }
    };
    discovery.finish(None);
    let shown = Some(path.to_string_lossy().into_owned());
    let mut command = Command::new(&path);
    command.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    // Finding a CLI outside the inherited GUI PATH does not make its env-node
    // shebang runnable. Expose the host-discovered runtime to this child only.
    if !matches!(tool, CliTool::Node) {
        if let Ok(node) = crate::session::resolve_executable("node", None) {
            if let Some(directory) = node.parent() {
                let inherited = std::env::var_os("PATH").unwrap_or_default();
                if !std::env::split_paths(&inherited).any(|path| path == directory) {
                    if let Ok(path) = std::env::join_paths(std::env::split_paths(&inherited).chain(std::iter::once(directory.to_path_buf()))) {
                        command.env("PATH", path);
                    }
                }
            }
        }
    }
    #[cfg(unix)]
    command.process_group(0);
    let spawn = diagnostic.phase("spawn", None);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            spawn.finish(Some("start_failed"));
            operation.finish(Some("start_failed"));
            return finish(tool, shown, "start_failed", None);
        }
    };
    spawn.finish(None);
    let pid = child.id();
    diagnostic.record("spawned", pid, "completed", "none");
    #[cfg(unix)]
    let mut group = child.id().map(|pid| ProbeGroup { pid: pid as i32, armed: true, diagnostic: diagnostic.clone() });
    let outcome = tokio::time::timeout(TIMEOUT, read_version(tool, &mut child, &diagnostic)).await;
    let (status, version) = match outcome {
        Ok(Ok(version)) => ("available", Some(version)),
        Ok(Err(status)) => (status, None),
        Err(_) => ("timeout", None),
    };
    if status == "timeout" { diagnostic.record("deadline", pid, "failed", "timeout"); }
    let cleanup = diagnostic.phase("cleanup", pid);
    #[cfg(unix)]
    let cleanup_error = {
        let signalled = group.as_mut().is_some_and(ProbeGroup::stop);
        let inactive = if let Some(pid) = pid {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match crate::process_ownership::group_executable(pid as i32) {
                        Ok(false) => break true,
                        Ok(true) => tokio::time::sleep(Duration::from_millis(10)).await,
                        Err(_) => break false,
                    }
                }
            }).await.unwrap_or(false)
        } else { false };
        let reaped = tokio::time::timeout(Duration::from_secs(2), child.wait()).await.is_ok_and(|result| result.is_ok());
        (!signalled || !inactive || !reaped).then_some("cleanup_pending")
    };
    #[cfg(not(unix))]
    let cleanup_error = if status != "available" && child.kill().await.is_err() { Some("cleanup_pending") } else { None };
    cleanup.finish(cleanup_error);
    let (status, version) = if let Some(error) = cleanup_error { (error, None) } else { (status, version) };
    operation.finish((status != "available").then_some(status));
    finish(tool, shown, status, version)
}
pub async fn probe_node_directory(directory: &str) -> CliProbe {
    let root = Path::new(directory);
    if !root.is_absolute() { return finish(CliTool::Node, Some(directory.to_owned()), "not_installed", None); }
    let executable = if cfg!(windows) { "node.exe" } else { "node" };
    let mut failure = finish(CliTool::Node, Some(directory.to_owned()), "not_installed", None);
    for path in [root.join(executable), root.join("bin").join(executable)] {
        if !path.is_file() { continue; }
        let result = probe(CliTool::Node, path.to_str()).await;
        if result.status == "available" { return result; }
        failure = result;
    }
    failure
}

async fn read_version(tool: CliTool, child: &mut Child, diagnostic: &ProcessDiagnostic) -> Result<String, &'static str> {
    let pid = child.id();
    let read = diagnostic.phase("version_stdout", pid);
    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => { read.finish(Some("start_failed")); return Err("start_failed"); }
    };
    let mut bytes = Vec::new();
    if (&mut stdout).take(OUTPUT_LIMIT as u64 + 1).read_to_end(&mut bytes).await.is_err() {
        read.finish(Some("execution_failed"));
        return Err("execution_failed");
    }
    if bytes.len() > OUTPUT_LIMIT { read.finish(Some("output_limit")); return Err("output_limit"); }
    read.finish(None);
    let exit = diagnostic.phase("exit_wait", pid);
    #[cfg(unix)]
    let successful = match child.id() {
        Some(pid) => loop {
            let observed = {
                let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
                if unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut status, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } != 0 { Some(false) }
                else if unsafe { status.si_pid() } != 0 { Some(status.si_code == libc::CLD_EXITED && unsafe { status.si_status() } == 0) }
                else { None }
            };
            if let Some(successful) = observed { break successful; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        },
        None => false,
    };
    #[cfg(not(unix))]
    let successful = child.wait().await.is_ok_and(|status| status.success());
    if !successful {
        exit.finish(Some("execution_failed"));
        return Err("execution_failed");
    }
    exit.finish(None);
    let validation = diagnostic.phase("version_validate", pid);
    let result = String::from_utf8(bytes).map_err(|_| "protocol_unsupported").and_then(|text| {
        let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
        match (lines.next(), lines.next()) {
            (Some(line), None) if tool.accepts(line) => Ok(line.to_owned()),
            _ => Err("protocol_unsupported"),
        }
    });
    validation.finish(result.as_ref().err().copied());
    result
}

#[cfg(test)]
mod tests {
    use super::CliTool;

    #[cfg(unix)]
    #[test]
    fn group_kill_distinguishes_absence_from_signal_failure() {
        assert_eq!(super::group_kill_outcome(None), ("requested", "none"));
        assert_eq!(super::group_kill_outcome(Some(libc::ESRCH)), ("absent", "none"));
        assert_eq!(super::group_kill_outcome(Some(libc::EPERM)), ("failed", "signal_failed"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn version_read_keeps_the_group_leader_waitable_until_cleanup() {
        use std::process::Stdio;
        use tokio::process::Command;
        for (output, code, expected) in [
            ("codex-cli 0.167.0", 0, Ok("codex-cli 0.167.0".to_owned())),
            ("not-a-version", 0, Err("protocol_unsupported")),
            ("codex-cli 0.167.0", 7, Err("execution_failed")),
        ] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", &format!("printf '%s\\n' '{output}'; exit {code}")])
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
                .kill_on_drop(true).process_group(0).spawn().unwrap();
            let pid = child.id().unwrap();
            let diagnostic = crate::diagnostics::ProcessDiagnostic::new("version_probe", "codex");
            let mut group = super::ProbeGroup { pid: pid as i32, armed: true, diagnostic: diagnostic.clone() };
            let result = tokio::time::timeout(super::TIMEOUT, super::read_version(CliTool::Codex, &mut child, &diagnostic)).await.unwrap();
            // Both successful and rejected output must preserve ownership of the
            // original PID until the group guard has signalled its descendants.
            let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let observed = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut status, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) };
            let owned = observed == 0 && unsafe { status.si_pid() } == pid as i32;
            let signalled = group.stop();
            let reaped = child.wait().await.unwrap();
            assert_eq!(result, expected);
            assert!(owned, "version read reaped the group leader before cleanup");
            assert!(signalled);
            assert!(!group.armed, "cleanup must never signal the PGID after reaping");
            assert_eq!(reaped.code(), Some(code));
        }
    }

    #[test]
    fn version_line_must_match_the_tool_shape() {
        assert!(CliTool::Codex.accepts("codex-cli 0.46.0"));
        assert!(CliTool::Claude.accepts("2.0.14 (Claude Code)"));
        assert!(CliTool::Gemini.accepts("0.1.11"));
        assert!(CliTool::Node.accepts("v22.1.0"));
        for (tool, line) in [
            (CliTool::Codex, "1"),
            (CliTool::Codex, "codex 0.1.0"),
            (CliTool::Claude, "2.0.14"),
            (CliTool::Gemini, "gemini 0.1.0"),
            (CliTool::Node, "22.1.0"),
            (CliTool::Node, "v22.1"),
        ] {
            assert!(!tool.accepts(line), "{line}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn node_directory_verifies_only_selected_candidates_in_priority_order() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("dgos-node-directory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let executable = root.join("node");
        let nested = root.join("bin/node");
        for (path, version) in [(&executable, "v20.0.0"), (&nested, "v22.1.0")] {
            std::fs::write(path, format!("#!/bin/sh\nprintf '{version}\\n'\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let selected = super::probe_node_directory(root.to_str().unwrap()).await;
        assert_eq!(selected.path.as_deref(), executable.to_str());
        assert_eq!(selected.version.as_deref(), Some("v20.0.0"));
        std::fs::write(&executable, "#!/bin/sh\nprintf 'not-node\\n'\n").unwrap();
        let fallback = super::probe_node_directory(root.to_str().unwrap()).await;
        assert_eq!(fallback.path.as_deref(), nested.to_str());
        assert_eq!(fallback.version.as_deref(), Some("v22.1.0"));
        std::fs::remove_file(executable).unwrap();
        std::fs::remove_file(nested).unwrap();
        assert_eq!(super::probe_node_directory(root.to_str().unwrap()).await.status, "not_installed");
        assert_eq!(super::probe_node_directory("relative/path").await.status, "not_installed");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn gui_path_resolves_node_for_version_scripts() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        const CHILD: &str = "DGOS_TEST_CLI_GUI_PATH";
        if std::env::var_os(CHILD).is_some() {
            let result = super::probe(CliTool::Gemini, None).await;
            assert_eq!(result.status, "available");
            assert_eq!(result.version.as_deref(), Some("0.1.11"));
            return;
        }
        let root = std::env::temp_dir().join(format!("dgos gui probe-{}", uuid::Uuid::new_v4()));
        let bin = root.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        // Exercise the real env shebang without depending on an installed Node:
        // /bin/sh supplies the interpreter under the discovered runtime name.
        symlink("/bin/sh", bin.join("node")).unwrap();
        let cli = bin.join("gemini");
        std::fs::write(&cli, "#!/usr/bin/env node\n[ \"$1\" = --version ] || exit 2\nprintf '0.1.11\\n'\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Only the fresh test subprocess receives the synthetic HOME/PATH.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cli::tests::gui_path_resolves_node_for_version_scripts", "--nocapture"])
            .env(CHILD, "1").env("HOME", &root).env("PATH", root.join("empty-path"))
            .output().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_executable_probe_distinguishes_failures_and_bounds_output() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("dgos-cli-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("tool with spaces;literal");
        for (body, expected, expected_version, startup_error) in [
            ("[ \"$#\" = 1 ] && [ \"$1\" = --version ] || exit 2\nprintf '%s\\n' 'codex-cli 0.0.0'", "available", Some("codex-cli 0.0.0"), None),
            ("printf '%s\\n' 'codex-cli 0.166.0'", "available", Some("codex-cli 0.166.0"), None),
            ("printf '%s\\n' 'codex-cli 0.167.0'", "available", Some("codex-cli 0.167.0"), None),
            ("printf '%s\\n' 'codex-cli 0.168.0'", "available", Some("codex-cli 0.168.0"), None),
            ("printf '%s\\n' 'codex-cli 0.167.0-preview'", "protocol_unsupported", None, Some("cli_probe_failed")),
            ("printf '%s\\n' 42", "protocol_unsupported", None, Some("cli_probe_failed")),
            ("exit 7", "execution_failed", None, Some("cli_probe_failed")),
            ("i=0; while [ $i -lt 4100 ]; do printf x; i=$((i+1)); done", "output_limit", None, Some("cli_probe_failed")),
            ("exec sleep 10", "timeout", None, Some("cli_probe_timeout")),
        ] {
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let result = super::probe(CliTool::Codex, path.to_str()).await;
            assert_eq!(result.status, expected);
            assert!(!result.managed);
            assert_eq!(result.executable, expected == "available");
            assert_eq!(result.startup_error(), startup_error);
            assert_eq!(result.version.as_deref(), expected_version);
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(super::probe(CliTool::Codex, path.to_str()).await.status, "not_executable");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(super::probe(CliTool::Codex, path.to_str()).await.status, "not_installed");
        std::fs::remove_dir(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn version_probe_cleans_process_group_on_timeout_cancellation_and_success() {
        use std::time::Duration;
        for outcome in ["timeout", "cancel", "available"] {
            let root = std::env::temp_dir().join(format!("dgos-cli-descendants-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let path = root.join("codex");
            let pids = root.join("pids");
            let body = if outcome == "available" {
                "/bin/sleep 30 >/dev/null &\nprintf 'codex-cli 0.0.0\\n'"
            } else {
                "/bin/sleep 30 &"
            };
            let tail = if outcome == "available" { "exit 0" } else { "wait" };
            crate::test_support::prepare_shell_fixture(&path, &format!("{body}\nprintf '%s %s\\n' \"$$\" \"$!\" > '{}'\n{tail}\n", pids.display())).await;
            let task = tokio::spawn(async move { super::probe(CliTool::Codex, path.to_str()).await });
            let ids: Vec<i32> = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Ok(text) = std::fs::read_to_string(&pids) {
                        let ids: Vec<i32> = text.split_whitespace().map(|s| s.parse().unwrap()).collect();
                        if ids.len() == 2 { break ids; }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            if outcome == "cancel" {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(task.await.unwrap().status, outcome);
            }
            let stopped = tokio::time::timeout(Duration::from_secs(3), async {
                while ids.iter().any(|pid| unsafe { libc::kill(*pid, 0) } == 0) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.is_ok();
            // Do not leave fixture processes alive if the regression fails.
            if !stopped { for pid in ids { unsafe { libc::kill(pid, libc::SIGKILL); } } }
            std::fs::remove_dir_all(root).unwrap();
            assert!(stopped, "probe left its child or descendant alive ({outcome})");
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn probe_diagnostics_distinguish_stdout_exit_and_spawn_failures() {
        use std::os::unix::fs::PermissionsExt;
        use tracing::instrument::WithSubscriber;
        let (log, subscriber) = crate::test_support::DiagnosticLog::capture();
        let root = std::env::temp_dir().join(format!("dgos-probe-private-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("credential-SENTINEL");
        for (body, expected) in [
            ("#!/bin/sh\nprintf 'codex-cli 0.0.0\\n'\n", "available"),
            ("#!/bin/sh\nprintf 'codex-cli 0.0.0\\n'\nexec /bin/sleep 10\n", "timeout"),
            ("#!/bin/sh\nprintf 'codex-cli 0.0.0\\n'\nexec 1>&-\nexec /bin/sleep 10\n", "timeout"),
            ("#!/dgos-nonexistent-interpreter\n", "start_failed"),
            ("#!/bin/sh\nprintf 'credential-SENTINEL\\n'\n", "protocol_unsupported"),
        ] {
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(super::probe(CliTool::Codex, path.to_str()).with_subscriber(subscriber.clone()).await.status, expected);
        }
        std::fs::remove_dir_all(&root).unwrap();
        let text = log.text();
        assert!(text.lines().any(|line| line.contains("stage=\"version_stdout\"") && line.contains("outcome=\"cancelled\"")), "{text}");
        assert!(text.lines().any(|line| line.contains("stage=\"exit_wait\"") && line.contains("outcome=\"cancelled\"")), "{text}");
        assert!(text.contains("error_class=\"spawn_failed\""), "{text}");
        assert!(text.contains("error_class=\"protocol_unsupported\""), "{text}");
        assert!(text.lines().any(|line| line.contains("stage=\"group_kill\"") && line.contains("outcome=\"requested\"") && line.contains("error_class=\"none\"")), "{text}");
        assert!(text.lines().any(|line| line.contains("stage=\"cleanup\"") && line.contains("outcome=\"completed\"") && line.contains("error_class=\"none\"")), "{text}");
        assert!(!text.contains("group_absent_or_signal_failed") && !text.contains("error_class=\"signal_failed\""), "{text}");
        assert!(text.contains("correlation_id=") && text.contains("elapsed_ms=") && text.contains("child_pid=Some("), "{text}");
        assert!(!text.contains("credential-SENTINEL") && !text.contains(&root.to_string_lossy().to_string()), "{text}");
        eprint!("{text}");
    }

    /// Opt-in installed CLI version check only: no model request, account or database writes.
    #[tokio::test]
    #[ignore = "requires installed Node and Codex; emits fixed-field phase diagnostics"]
    async fn installed_node_codex_probe_diagnostic() {
        use tracing::instrument::WithSubscriber;
        let (log, subscriber) = crate::test_support::DiagnosticLog::capture();
        let node = super::probe(CliTool::Node, None).with_subscriber(subscriber.clone()).await;
        let codex = super::probe(CliTool::Codex, None).with_subscriber(subscriber).await;
        eprint!("{}", log.text());
        assert_eq!(node.status, "available", "Node probe: {}", node.status);
        assert_eq!(codex.status, "available", "Codex probe: {}", codex.status);
    }
}
