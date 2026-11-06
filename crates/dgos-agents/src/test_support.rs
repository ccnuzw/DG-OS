use crate::{AgentManager, projects::{CreateProjectInput, SaveTaskInput}};
use dgos_db::{DevelopmentRepository, migrations::{Migrator, MigratorTrait}};
use std::path::Path;
use tokio_util::sync::CancellationToken;
pub async fn repository(root: &Path) -> DevelopmentRepository {
    let db = dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", root.join("metadata.sqlite").display())).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    DevelopmentRepository::new(db)
}
pub async fn manager(root: &Path) -> AgentManager {
    let manager = AgentManager::new(root.join("index.json"), repository(root).await);
    manager.load().await.unwrap();
    manager
}
pub async fn task(manager: &AgentManager, root: &Path) -> String {
    let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "fixture".into(), path: root.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
    manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id, goal: "Exercise native lifecycle".into(), requirements: "Keep ownership and shutdown semantics".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap().id
}

/// Admit a newly written script before timing its protocol or process cleanup.
/// macOS can hold the first shebang exec before userspace for several seconds
/// when parallel tests create executables. The readiness branch must exit before
/// starting descendants, writing markers, or consuming protocol input.
#[cfg(unix)]
pub(crate) async fn prepare_shell_fixture(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\nif [ \"${{1-}}\" = --dgos-fixture-ready ]; then exit 0; fi\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = tokio::process::Command::new(path)
        .arg("--dgos-fixture-ready")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .kill_on_drop(true).spawn().unwrap();
    match tokio::time::timeout(std::time::Duration::from_secs(10), child.wait()).await {
        Ok(Ok(status)) => assert!(status.success(), "shell fixture failed executable admission"),
        failed => {
            let _ = child.start_kill();
            let reaped = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
            panic!("shell fixture admission failed: {failed:?}; cleanup: {reaped:?}");
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shell_fixture_preparation_does_not_run_its_protocol_or_side_effects() {
    let root = std::env::temp_dir().join(format!("dgos-fixture-preparation-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let executable = root.join("fixture");
    let marker = root.join("started");
    prepare_shell_fixture(&executable, &format!("printf started > '{}'\nprintf 'codex-cli 0.0.0\\n'\n", marker.display())).await;
    assert!(!marker.exists());
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), crate::cli::probe(crate::cli::CliTool::Codex, executable.to_str())).await.unwrap();
    assert_eq!(result.status, "available");
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "started");
    std::fs::remove_dir_all(root).unwrap();
}

#[derive(Clone, Default)]
pub(crate) struct DiagnosticLog(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for DiagnosticLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for DiagnosticLog {
    type Writer = Self;
    fn make_writer(&'a self) -> Self { self.clone() }
}
impl DiagnosticLog {
    pub(crate) fn capture() -> (Self, tracing::subscriber::DefaultGuard) {
        let log = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false).without_time().with_writer(log.clone()).finish();
        let guard = tracing::subscriber::set_default(subscriber);
        (log, guard)
    }
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}
