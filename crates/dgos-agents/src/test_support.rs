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
