//! Trusted host runtime commands. The renderer selects project/task IDs, but never owner,
//! environment values, executable ownership, or a persisted PID.
use crate::{host::{self, Capability}, session::SessionResolver};
use dgos_agents::{runtime::{CommandCatalog, CommandConfiguration, ProcessSnapshot, StartProcessInput}, AgentManager};
use dgos_types::BuiltinCaller;
use serde::Deserialize;
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;

fn caller(app: &tauri::AppHandle, window: &WebviewWindow) -> Result<(String, CancellationToken), String> {
    let context = host::context(window, BuiltinCaller::Workbench, Capability::TasksRun).map_err(|e| e.to_string())?;
    let (session, lease) = app.state::<Arc<SessionResolver>>().agent_session().map_err(|e| e.to_string())?;
    if session.user_id() != context.user_id() || lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok((context.user_id().into(), lease))
}

#[tauri::command]
pub async fn save_development_file(app: tauri::AppHandle, window: WebviewWindow, input: dgos_agents::files::SaveFileInput) -> Result<dgos_agents::files::FileView, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().save_development_file(&owner, input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[tauri::command]
pub async fn read_development_commands(app: tauri::AppHandle, window: WebviewWindow, task_id: String) -> Result<CommandCatalog, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().read_development_commands(&owner, &task_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveDevelopmentCommandsInput { task_id: String, commands: Vec<CommandConfiguration> }
#[tauri::command]
pub async fn save_development_commands(app: tauri::AppHandle, window: WebviewWindow, input: SaveDevelopmentCommandsInput) -> Result<CommandCatalog, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().save_development_commands(&owner, &input.task_id, input.commands).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[tauri::command]
pub async fn start_development_process(app: tauri::AppHandle, window: WebviewWindow, input: StartProcessInput) -> Result<ProcessSnapshot, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().start_development_process(&owner, input, lease.clone()).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn read_development_process(app: tauri::AppHandle, window: WebviewWindow, id: String) -> Result<ProcessSnapshot, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().read_development_process(&owner, &id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn list_development_processes(app: tauri::AppHandle, window: WebviewWindow, task_id: String) -> Result<Vec<ProcessSnapshot>, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().list_development_processes(&owner, &task_id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[tauri::command]
pub async fn stop_development_process(app: tauri::AppHandle, window: WebviewWindow, id: String) -> Result<ProcessSnapshot, String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().stop_development_process(&owner, &id).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentTerminalInput { id: String, data: String }
#[tauri::command]
pub async fn write_development_terminal(app: tauri::AppHandle, window: WebviewWindow, input: DevelopmentTerminalInput) -> Result<(), String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().write_development_terminal(&owner, &input.id, input.data).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResizeDevelopmentTerminalInput { id: String, rows: u16, cols: u16 }
#[tauri::command]
pub async fn resize_development_terminal(app: tauri::AppHandle, window: WebviewWindow, input: ResizeDevelopmentTerminalInput) -> Result<(), String> {
    let (owner, lease) = caller(&app, &window)?;
    let result = app.state::<AgentManager>().resize_development_terminal(&owner, &input.id, input.cols, input.rows).await;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDevelopmentExternalInput { project_id: String, task_id: String, process_id: Option<String>, target: String, editor: Option<String> }
#[tauri::command]
pub async fn open_development_external(app: tauri::AppHandle, window: WebviewWindow, input: OpenDevelopmentExternalInput) -> Result<(), String> {
    let (owner, lease) = caller(&app, &window)?;
    let manager = app.state::<AgentManager>();
    let task = manager.task_project(&owner, &input.task_id).await?;
    if task.id != input.project_id { return Err("task_not_found".into()); }
    if task.archived { return Err("project_archived".into()); }
    let target = match input.target.as_str() {
        "editor" => {
            let editor = input.editor.as_deref().filter(|value| !value.trim().is_empty() && value.len() <= 4096 && !value.contains(['\0', '\n', '\r'])).ok_or("invalid_request")?;
            if lease.is_cancelled() { return Err("session_invalid".into()); }
            app.opener().open_path(&task.path, Some(editor)).map_err(|_| "external_open_failed")?;
            return if lease.is_cancelled() { Err("session_invalid".into()) } else { Ok(()) };
        }
        "preview" => {
            let id = input.process_id.ok_or("process_not_found")?;
            let snapshot = manager.read_development_process(&owner, &id).await?;
            if snapshot.task_id != input.task_id || snapshot.project_id != task.id || snapshot.status != "ready" { return Err("service_not_ready".into()); }
            snapshot.preview_url.ok_or("service_not_ready")?
        }
        _ => return Err("invalid_request".into()),
    };
    if !target.starts_with("http://127.0.0.1:") && !target.starts_with("http://[::1]:") && !target.starts_with("http://localhost:") { return Err("service_not_ready".into()); }
    app.opener().open_url(target, None::<&str>).map_err(|_| "external_open_failed")?;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProjectExternalTarget { Terminal, Reveal }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDevelopmentProjectExternalInput { project_id: String, target: ProjectExternalTarget }

#[tauri::command]
pub async fn open_development_project_external(app: tauri::AppHandle, window: WebviewWindow, input: OpenDevelopmentProjectExternalInput) -> Result<(), String> {
    let (owner, lease) = caller(&app, &window)?;
    let manager = app.state::<AgentManager>();
    // Keep admission until dispatch completes so archive, relocation and removal cannot race it.
    let admission = manager.admit_native_project(&owner, &input.project_id, &lease).await?;
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    let result = match input.target {
        ProjectExternalTarget::Terminal => open_project_terminal(std::path::Path::new(&admission.project.path), &lease).await,
        ProjectExternalTarget::Reveal => app.opener().reveal_item_in_dir(&admission.project.path).map_err(|_| "external_open_failed".into()),
    };
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    result
}

fn project_terminal_command(root: &std::path::Path) -> Result<tokio::process::Command, String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = tokio::process::Command::new("/usr/bin/open");
        command.args(["-a", "/System/Applications/Utilities/Terminal.app", "--"]).arg(root);
        Ok(command)
    }
    #[cfg(target_os = "linux")]
    {
        // Use the desktop helper directly: never search project cwd/PATH or interpolate a shell.
        let mut command = tokio::process::Command::new("/usr/bin/gnome-terminal");
        command.arg("--working-directory").arg(root);
        Ok(command)
    }
    #[cfg(target_os = "windows")]
    {
        let system_root = std::env::var_os("SystemRoot").ok_or("terminal_unavailable")?;
        let executable = std::path::PathBuf::from(system_root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
        if !executable.is_absolute() { return Err("terminal_unavailable".into()); }
        let mut command = tokio::process::Command::new(executable);
        // No profile scripts or command text; cwd also supports UNC project roots.
        command.args(["-NoLogo", "-NoProfile", "-NoExit"]).current_dir(root).creation_flags(0x00000010); // CREATE_NEW_CONSOLE
        Ok(command)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    { let _ = root; Err("terminal_unsupported".into()) }
}

async fn open_project_terminal(root: &std::path::Path, lease: &CancellationToken) -> Result<(), String> {
    if lease.is_cancelled() { return Err("session_invalid".into()); }
    let mut command = project_terminal_command(root)?;
    #[cfg(not(target_os = "windows"))]
    command.kill_on_drop(true).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let mut child = command.spawn().map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { "terminal_unavailable" } else { "external_open_failed" })?;
    // Windows starts the interactive console itself, not a short-lived launcher.
    #[cfg(target_os = "windows")]
    { let _ = child; Ok(()) }
    #[cfg(not(target_os = "windows"))]
    {
        tokio::select! {
            biased;
            _ = lease.cancelled() => Err("session_invalid".into()),
            result = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait()) => match result {
                Ok(Ok(status)) if status.success() => Ok(()),
                Ok(_) => Err("external_open_failed".into()),
                Err(_) => Err("external_open_timeout".into()),
            },
        }
    }
}

#[cfg(test)]
mod project_external_tests {
    use super::*;

    #[test]
    fn project_external_input_rejects_renderer_paths_programs_and_unknown_targets() {
        for extra in ["path", "program", "owner", "lease", "editor", "task_id"] {
            let mut input = serde_json::json!({"project_id":"project", "target":"terminal"});
            input[extra] = serde_json::json!("untrusted");
            assert!(serde_json::from_value::<OpenDevelopmentProjectExternalInput>(input).is_err());
        }
        for target in ["editor", "preview", "/bin/sh", "Terminal", ""] {
            assert!(serde_json::from_value::<OpenDevelopmentProjectExternalInput>(serde_json::json!({"project_id":"project", "target":target})).is_err());
        }
        for target in ["terminal", "reveal"] {
            assert!(serde_json::from_value::<OpenDevelopmentProjectExternalInput>(serde_json::json!({"project_id":"project", "target":target})).is_ok());
        }
    }

    #[test]
    fn project_terminal_passes_metacharacters_as_one_directory_argument() {
        let root = std::path::Path::new("/tmp/project 'quoted' $(touch forbidden); &\n-next");
        let command = project_terminal_command(root).unwrap();
        let command = command.as_std();
        let args: Vec<_> = command.get_args().collect();
        #[cfg(target_os = "macos")]
        {
            assert_eq!(command.get_program(), "/usr/bin/open");
            assert_eq!(args, [std::ffi::OsStr::new("-a"), std::ffi::OsStr::new("/System/Applications/Utilities/Terminal.app"), std::ffi::OsStr::new("--"), root.as_os_str()]);
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(command.get_program(), "/usr/bin/gnome-terminal");
            assert_eq!(args, [std::ffi::OsStr::new("--working-directory"), root.as_os_str()]);
        }
        #[cfg(target_os = "windows")]
        {
            assert!(std::path::Path::new(command.get_program()).is_absolute());
            assert_eq!(args, [std::ffi::OsStr::new("-NoLogo"), std::ffi::OsStr::new("-NoProfile"), std::ffi::OsStr::new("-NoExit")]);
            assert_eq!(command.get_current_dir(), Some(root));
        }
    }

    #[tokio::test]
    async fn project_terminal_rejects_revoked_lease_before_spawning() {
        let lease = CancellationToken::new(); lease.cancel();
        assert_eq!(open_project_terminal(std::path::Path::new("/missing"), &lease).await.unwrap_err(), "session_invalid");
    }
}
