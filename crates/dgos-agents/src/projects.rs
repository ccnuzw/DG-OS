//! D134: project metadata orchestration; file operations never remove user code.
use crate::AgentManager;
use dgos_db::development_repository::{DevelopmentTask, DevelopmentWorkspace, Project, ProjectOperation, Selection};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::{Duration, SystemTime, UNIX_EPOCH}};
use tokio::sync::OwnedMutexGuard;
use tokio_util::sync::CancellationToken;
use tokio::process::Command;
use tokio::io::AsyncReadExt;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectInput { pub kind: String, pub name: String, pub path: String, pub source: Option<String> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateProjectInput { pub project_id: String, pub action: String, pub value: Option<String> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveTaskInput { pub id: Option<String>, pub project_id: String, pub goal: String, pub requirements: String, pub status: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifySessionInput { pub session_id: String, pub task_id: String }
#[derive(Serialize, Debug)]
pub struct ProjectInspection { pub path_state: String, pub kind: String, pub repository_root: Option<String>, pub branch: Option<String>, pub changes: Vec<String>, pub truncated: bool, pub error: Option<String> }

/// Holds project admission through native extension filesystem work.
pub struct NativeProjectAdmission {
    pub project: Project,
    _admission: OwnedMutexGuard<()>,
}
fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }
fn check_lease(lease: &CancellationToken) -> Result<(), String> { if lease.is_cancelled() { Err("session_invalid".into()) } else { Ok(()) } }
pub(crate) async fn directory(path: &str) -> Result<(String, String), String> {
    if !Path::new(path).is_absolute() { return Err("invalid_workspace".into()); }
    let path = tokio::fs::canonicalize(path).await.map_err(|_| "invalid_workspace")?;
    let metadata = tokio::fs::metadata(&path).await.map_err(|_| "invalid_workspace")?;
    if !metadata.is_dir() { return Err("invalid_workspace".into()); }
    #[cfg(unix)]
    let identity = { use std::os::unix::fs::MetadataExt; format!("{}:{}", metadata.dev(), metadata.ino()) };
    #[cfg(windows)]
    let identity = { use std::os::windows::fs::MetadataExt; format!("{:?}:{:?}", metadata.volume_serial_number(), metadata.file_index()) };
    #[cfg(not(any(unix, windows)))]
    let identity = format!("{:?}", metadata.created().map_err(|_| "invalid_workspace")?);
    Ok((path.to_string_lossy().into_owned(), identity))
}
pub(crate) async fn validate_directory(project: &Project) -> Result<(), String> {
    let (path, identity) = directory(&project.path).await?;
    if path != project.path || identity != project.directory_identity { return Err("workspace_changed".into()); }
    Ok(())
}
impl AgentManager {
    /// Serializes project-scoped native extension work with project updates.
    /// Hold the returned permit until the filesystem operation completes.
    pub async fn admit_native_project(&self, owner: &str, project_id: &str, lease: &CancellationToken) -> Result<NativeProjectAdmission, String> {
        let admission = tokio::select! {
            biased;
            _ = lease.cancelled() => return Err("session_invalid".into()),
            guard = self.inner.admission.clone().lock_owned() => guard,
        };
        self.ready().await?;
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        let project = self.inner.repository.project(owner, project_id).await?;
        if project.archived { return Err("project_archived".into()); }
        validate_directory(&project).await?;
        if lease.is_cancelled() { return Err("session_invalid".into()); }
        Ok(NativeProjectAdmission { project, _admission: admission })
    }
    pub async fn development_workspace(&self, owner: &str) -> Result<DevelopmentWorkspace, String> {
        self.ready().await?;
        self.inner.repository.workspace(owner).await
    }
    pub async fn create_project(&self, owner: &str, input: CreateProjectInput, lease: CancellationToken) -> Result<Project, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?; check_lease(&lease)?;
        if input.name.trim().is_empty() || input.name.chars().count() > 200 { return Err("invalid_project".into()); }
        if !matches!(input.kind.as_str(), "import" | "create" | "clone") { return Err("invalid_project".into()); }
        let mut operation = None;
        let path = if input.kind == "import" { input.path.clone() } else {
            let target = Path::new(&input.path);
            if !target.is_absolute() { return Err("invalid_workspace".into()); }
            let name = target.file_name().filter(|s| !s.is_empty()).ok_or("invalid_workspace")?;
            let parent = tokio::fs::canonicalize(target.parent().ok_or("invalid_workspace")?).await.map_err(|_| "invalid_workspace")?;
            let target = parent.join(name);
            match tokio::fs::symlink_metadata(&target).await {
                Ok(_) => return Err("invalid_workspace".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => return Err("invalid_workspace".into()),
            }
            if input.kind == "clone" { validate_source(input.source.as_deref().ok_or("invalid_project")?)?; }
            let op = ProjectOperation { id: Uuid::new_v4().to_string(), owner: owner.into(), kind: input.kind.clone(), path: target.to_string_lossy().into_owned(), status: "pending".into(), project_id: None };
            self.inner.repository.start_operation(&op).await?;
            // Reserve the destination exclusively before Git sees it. Never let
            // clone reuse a pre-existing user directory after a check/create race.
            let result = async {
                check_lease(&lease)?; self.ready().await?;
                tokio::fs::create_dir(&target).await.map_err(|_| "invalid_workspace".to_string())?;
                if input.kind == "clone" {
                    let mut command = git_command(&parent);
                    command.args(["clone", "--no-recurse-submodules", "--template=", "--", input.source.as_deref().unwrap_or_default()]).arg(&target);
                    git_output(command, Duration::from_secs(120), &lease, &self.inner.stop).await?;
                }
                Ok::<_, String>(())
            }.await;
            if let Err(error) = result {
                self.inner.repository.finish_operation(owner, &op.id, if matches!(error.as_str(), "cancelled" | "git_timeout") { "unknown" } else { "failed" }, None).await?;
                return Err(error);
            }
            operation = Some(op.id);
            target.to_string_lossy().into_owned()
        };
        let result = async {
            check_lease(&lease)?; self.ready().await?;
            let (path, directory_identity) = directory(&path).await?;
            check_lease(&lease)?;
            let project = Project { id: Uuid::new_v4().to_string(), owner: owner.into(), name: input.name.trim().into(), path, directory_identity, archived: false, created_at: now(), last_opened_at: now() };
            self.inner.repository.insert_project(&project).await?;
            Ok::<_, String>(project)
        }.await;
        if let Some(op) = operation { self.inner.repository.finish_operation(owner, &op, if result.is_ok() { "succeeded" } else { "unknown" }, result.as_ref().ok().map(|p| p.id.as_str())).await?; }
        result
    }
    pub async fn update_project(&self, owner: &str, input: UpdateProjectInput, lease: CancellationToken) -> Result<(), String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?; check_lease(&lease)?;
        let mut project = self.inner.repository.project(owner, &input.project_id).await?;
        match input.action.as_str() {
            "rename" => { let name = input.value.ok_or("invalid_project")?; if name.trim().is_empty() || name.chars().count() > 200 { return Err("invalid_project".into()); } project.name = name.trim().into(); },
            "archive" | "unlink" | "relocate" => {
                self.project_available(&project.id).await?;
                self.workspace_available(&project.path, None).await?;
                self.host_workspace_available(&project.path).await?;
                if input.action == "unlink" {
                    check_lease(&lease)?;
                    self.inner.repository.unlink_project(owner, &project.id).await?;
                    return self.reload_records().await;
                }
                if input.action == "archive" { project.archived = true; } else {
                    let (path, identity) = directory(input.value.as_deref().ok_or("invalid_workspace")?).await?;
                    self.workspace_available(&path, None).await?;
                    self.host_workspace_available(&path).await?;
                    project.path = path; project.directory_identity = identity;
                }
            },
            "restore" => project.archived = false,
            _ => return Err("invalid_project".into()),
        }
        check_lease(&lease)?;
        self.inner.repository.update_project(&project).await?;
        self.reload_records().await
    }
    pub async fn save_development_task(&self, owner: &str, input: SaveTaskInput, lease: CancellationToken) -> Result<DevelopmentTask, String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?; check_lease(&lease)?;
        let project = self.inner.repository.project(owner, &input.project_id).await?;
        if project.archived { return Err("project_archived".into()); }
        let created_at = if let Some(id) = &input.id { let old = self.inner.repository.task(owner, id).await?; if old.project_id != project.id { return Err("task_not_found".into()); } old.created_at } else { now() };
        let task = DevelopmentTask { id: input.id.unwrap_or_else(|| Uuid::new_v4().to_string()), owner: owner.into(), project_id: project.id, goal: input.goal.trim().into(), requirements: input.requirements.trim().into(), status: input.status, created_at };
        check_lease(&lease)?;
        self.inner.repository.save_task(&task).await?;
        Ok(task)
    }
    pub async fn select_development_context(&self, owner: &str, input: Selection, lease: CancellationToken) -> Result<(), String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?; check_lease(&lease)?;
        self.inner.repository.select(owner, &input).await
    }
    pub async fn classify_session(&self, owner: &str, input: ClassifySessionInput, lease: CancellationToken) -> Result<(), String> {
        let _admission = self.inner.admission.lock().await;
        self.ready().await?; check_lease(&lease)?;
        let record = self.list(owner).await.into_iter().find(|r| r.id == input.session_id).ok_or("session_not_found")?;
        let project = self.task_project(owner, &input.task_id).await?;
        if project.archived { return Err("project_archived".into()); }
        validate_directory(&project).await?;
        self.workspace_available(&record.workspace, None).await?;
        check_lease(&lease)?;
        self.inner.repository.classify(owner, &input.session_id, &input.task_id).await?;
        self.reload_records().await
    }
    pub async fn inspect_project(&self, owner: &str, id: &str) -> Result<ProjectInspection, String> {
        self.ready().await?;
        let project = self.inner.repository.project(owner, id).await?;
        self.inspect_directory(project).await
    }
    pub(crate) async fn inspect_task_project(&self, owner: &str, task_id: &str) -> Result<ProjectInspection, String> {
        self.inspect_directory(self.task_project(owner, task_id).await?).await
    }
    async fn inspect_directory(&self, project: Project) -> Result<ProjectInspection, String> {
        let mut view = ProjectInspection { path_state: "available".into(), kind: "unknown".into(), repository_root: None, branch: None, changes: vec![], truncated: false, error: None };
        let state = match tokio::fs::metadata(&project.path).await {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => "missing",
            Err(_) => "inaccessible",
            Ok(_) => match validate_directory(&project).await { Ok(()) => "available", Err(e) if e == "workspace_changed" => "changed", Err(_) => "inaccessible" },
        };
        view.path_state = state.into();
        if state != "available" { return Ok(view); }
        let lease = CancellationToken::new();
        let mut command = git_command(Path::new(&project.path)); command.args(["rev-parse", "--show-toplevel"]);
        let root = match git_output(command, Duration::from_secs(10), &lease, &self.inner.stop).await {
            Ok((bytes, _)) => String::from_utf8_lossy(&bytes).trim().to_string(),
            Err(error) => {
                // Check directory ancestry rather than disguising repository errors as clean directories.
                let mut path = Some(Path::new(&project.path)); let mut has_git = false;
                while let Some(p) = path { if tokio::fs::symlink_metadata(p.join(".git")).await.is_ok() { has_git = true; break; } path = p.parent(); }
                view.kind = if has_git { "unknown" } else { "directory" }.into();
                if has_git || error != "git_failed" { view.error = Some(error); }
                return Ok(view);
            }
        };
        view.kind = "git".into(); view.repository_root = Some(root);
        // Status may invoke repository-defined clean/process filters while hashing
        // changed files. Override every configured filter before inspecting code.
        let mut config = git_command(Path::new(&project.path)); config.args(["config", "--name-only", "--list"]);
        let filters = match git_output(config, Duration::from_secs(10), &lease, &self.inner.stop).await {
            Ok((bytes, _)) => String::from_utf8(bytes).map_err(|_| "git_failed")?,
            Err(error) => { view.error = Some(error); return Ok(view); }
        };
        let mut command = git_command(Path::new(&project.path));
        for key in filters.lines().filter(|key| key.starts_with("filter.") && [".clean", ".smudge", ".process", ".required"].iter().any(|suffix| key.ends_with(suffix))) {
            if key.contains(['\n', '\r', '\0', '=']) { view.error = Some("git_failed".into()); return Ok(view); }
            command.arg("-c").arg(format!("{key}={}", if key.ends_with(".required") { "false" } else { "" }));
        }
        command.args(["status", "--porcelain=v1", "-z", "--untracked-files=normal"]);
        match git_output(command, Duration::from_secs(10), &lease, &self.inner.stop).await {
            Ok((bytes, truncated)) => { view.truncated = truncated; view.changes = bytes.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect(); },
            Err(error) => view.error = Some(error),
        }
        let mut command = git_command(Path::new(&project.path)); command.args(["symbolic-ref", "--quiet", "--short", "HEAD"]);
        match git_output(command, Duration::from_secs(10), &lease, &self.inner.stop).await {
            Ok((bytes, _)) => view.branch = Some(String::from_utf8_lossy(&bytes).trim().into()),
            Err(_) => {
                let mut command = git_command(Path::new(&project.path)); command.args(["rev-parse", "--short", "HEAD"]);
                match git_output(command, Duration::from_secs(10), &lease, &self.inner.stop).await { Ok((bytes, _)) => view.branch = Some(format!("HEAD {}", String::from_utf8_lossy(&bytes).trim())), Err(error) => view.error = Some(error) }
            }
        }
        Ok(view)
    }
}
fn validate_source(source: &str) -> Result<(), String> {
    if source.is_empty() || source.starts_with('-') || source.contains('\0') || source.contains('\n') || source.starts_with("ext::") { return Err("invalid_project".into()); }
    if source.contains("://") && !["https://", "http://", "ssh://", "file://"].iter().any(|prefix| source.starts_with(prefix)) { return Err("invalid_project".into()); }
    // URL userinfo may contain credentials; never persist or echo them.
    if source.starts_with("http") && source.split("://").nth(1).and_then(|s| s.split('/').next()).is_some_and(|s| s.contains('@')) { return Err("invalid_project".into()); }
    Ok(())
}
pub(crate) fn git_command(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(cwd).args(["-c", "core.hooksPath=", "-c", "core.fsmonitor=false", "-c", "protocol.allow=never", "-c", "protocol.https.allow=always", "-c", "protocol.http.allow=always", "-c", "protocol.ssh.allow=always", "-c", "protocol.file.allow=always", "-c", "core.sshCommand=ssh -oBatchMode=yes"])
        .env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0").env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env_remove("GIT_CONFIG_COUNT").env_remove("GIT_CONFIG_PARAMETERS").env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_INDEX_FILE").env_remove("GIT_SSH_COMMAND").env_remove("GIT_SSH");
    command
}
pub(crate) async fn git_output(mut command: Command, deadline: Duration, lease: &CancellationToken, stop: &CancellationToken) -> Result<(Vec<u8>, bool), String> {
    const LIMIT: u64 = 1024 * 1024;
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(unix)] command.process_group(0);
    let mut child = command.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "git_unavailable" } else { "git_failed" })?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or("git_failed")?;
    let io = async {
        let mut bytes = Vec::new();
        stdout.take(LIMIT + 1).read_to_end(&mut bytes).await.map_err(|_| "git_failed")?;
        if bytes.len() > LIMIT as usize { return Err("git_output_limit"); }
        let status = child.wait().await.map_err(|_| "git_failed")?;
        if !status.success() { return Err("git_failed"); }
        Ok((bytes, false))
    };
    let result = tokio::select! { value = tokio::time::timeout(deadline, io) => value.unwrap_or(Err("git_timeout")), _ = lease.cancelled() => Err("cancelled"), _ = stop.cancelled() => Err("cancelled") };
    if result.is_err() {
        #[cfg(unix)] if let Some(pid) = pid { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    }
    result.map_err(String::from)
}
