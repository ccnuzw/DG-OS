use crate::{AgentManager, projects::{CreateProjectInput, UpdateProjectInput, SaveTaskInput}};
use dgos_db::sea_orm::{ConnectionTrait, DbBackend, Statement};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
fn root() -> std::path::PathBuf { let p = std::env::temp_dir().join(format!("dgos-project-boundary-{}", Uuid::new_v4())); std::fs::create_dir_all(&p).unwrap(); p.canonicalize().unwrap() }
fn import(path: &std::path::Path) -> CreateProjectInput { CreateProjectInput { kind: "import".into(), name: "Example".into(), path: path.to_string_lossy().into(), source: None } }
#[tokio::test]
async fn legacy_migration_failure_preserves_bytes_and_rejects_admission() {
    let root = root();
    let old = root.join("index.json");
    let id = Uuid::new_v4().to_string();
    let bytes = serde_json::to_vec(&serde_json::json!([
        {"id":id,"owner":"alice","engine":"codex","native_id":"native-a","workspace":root},
        {"id":id,"owner":"bob","engine":"claude","native_id":"native-b","workspace":root}
    ])).unwrap();
    std::fs::write(&old, &bytes).unwrap();
    let repository = crate::test_support::repository(&root).await;
    let manager = AgentManager::new(old.clone(), repository.clone(), crate::test_support::providers(&root, repository.clone()).await);
    assert!(manager.load().await.is_err());
    assert_eq!(std::fs::read(&old).unwrap(), bytes);
    assert!(!repository.legacy_imported().await.unwrap());
    assert!(repository.sessions().await.unwrap().is_empty());
    assert!(manager.create_project("alice", import(&root), CancellationToken::new()).await.is_err());
    std::fs::write(&old, serde_json::to_vec(&serde_json::json!([{ "id":id,"owner":"alice","engine":"codex","native_id":"native-a","workspace":root }])).unwrap()).unwrap();
    let reloaded = AgentManager::new(old.clone(), repository.clone(), crate::test_support::providers(&root, repository.clone()).await);
    reloaded.load().await.unwrap();
    let workspace = reloaded.development_workspace("alice").await.unwrap();
    assert!(workspace.projects.is_empty() && workspace.tasks.is_empty());
    assert_eq!(workspace.sessions[0].task_id, None);
    assert_eq!(workspace.sessions[0].native_id.as_deref(), Some("native-a"));
    // A successful mark makes the retired JSON non-authoritative, even if later damaged.
    std::fs::write(&old, "retired file changed outside app").unwrap();
    let next = AgentManager::new(old.clone(), repository.clone(), crate::test_support::providers(&root, repository).await);
    next.load().await.unwrap();
    assert_eq!(next.list("alice").await[0].native_id.as_deref(), Some("native-a"));
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn database_commit_failure_leaves_legacy_and_all_rows_unimported() {
    use dgos_db::migrations::{Migrator, MigratorTrait};
    let root = root();
    let db = dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", root.join("metadata.sqlite").display())).await.unwrap(); Migrator::up(&db, None).await.unwrap();
    db.execute(Statement::from_string(DbBackend::Sqlite, "CREATE TRIGGER deny_import BEFORE INSERT ON development_migrations BEGIN SELECT RAISE(ABORT, 'injected'); END".to_string())).await.unwrap();
    let old = root.join("index.json");
    let bytes = serde_json::to_vec(&serde_json::json!([{ "id":Uuid::new_v4(),"owner":"alice","engine":"codex","native_id":"saved","workspace":root }])).unwrap();
    std::fs::write(&old, &bytes).unwrap();
    let repository = dgos_db::DevelopmentRepository::new(db.clone());
    let manager = AgentManager::new(old.clone(), repository.clone(), crate::test_support::providers(&root, repository.clone()).await);
    assert!(manager.load().await.is_err());
    assert_eq!(std::fs::read(&old).unwrap(), bytes);
    assert!(repository.sessions().await.unwrap().is_empty());
    assert!(!repository.legacy_imported().await.unwrap());
    db.execute_unprepared("DROP TRIGGER deny_import").await.unwrap();
    let next = AgentManager::new(old, repository.clone(), crate::test_support::providers(&root, repository).await); next.load().await.unwrap();
    assert_eq!(next.list("alice").await[0].native_id.as_deref(), Some("saved"));
    assert!(manager.shutdown().await);
    assert!(next.shutdown().await);
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn directory_replacement_requires_relocation_and_unlink_preserves_code() {
    let root = root(); let path = root.join("code"); std::fs::create_dir(&path).unwrap(); std::fs::write(path.join("user.txt"), "keep").unwrap();
    let manager = crate::test_support::manager(&root).await;
    let project = manager.create_project("alice", import(&path), CancellationToken::new()).await.unwrap();
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Fix".into(), requirements: "Tests pass".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    assert_eq!(manager.inspect_project("alice", &project.id).await.unwrap().kind, "directory");
    assert!(manager.inspect_project("bob", &project.id).await.is_err());
    assert!(manager.save_development_task("bob", SaveTaskInput { id: Some(task.id.clone()), project_id: project.id.clone(), goal: "Steal".into(), requirements: "No".into(), status: "completed".into() }, CancellationToken::new()).await.is_err());
    let moved = root.join("moved"); std::fs::rename(&path, &moved).unwrap();
    assert_eq!(manager.inspect_project("alice", &project.id).await.unwrap().path_state, "missing");
    std::fs::create_dir(&path).unwrap();
    assert_eq!(manager.inspect_project("alice", &project.id).await.unwrap().path_state, "changed");
    manager.update_project("alice", UpdateProjectInput { project_id: project.id.clone(), action: "relocate".into(), value: Some(moved.to_string_lossy().into()) }, CancellationToken::new()).await.unwrap();
    assert_eq!(manager.inspect_project("alice", &project.id).await.unwrap().path_state, "available");
    let restarted = crate::test_support::manager(&root).await;
    let retained = restarted.inner.repository.task("alice", &task.id).await.unwrap();
    assert_eq!(retained.goal, task.goal);
    assert_eq!(retained.requirements, task.requirements);
    assert_eq!(retained.status, task.status);
    assert_eq!(restarted.inner.repository.project("alice", &project.id).await.unwrap().path, moved.canonicalize().unwrap().to_string_lossy());
    assert_eq!(std::fs::read_to_string(moved.join("user.txt")).unwrap(), "keep");
    assert!(restarted.shutdown().await);
    manager.update_project("alice", UpdateProjectInput { project_id: project.id, action: "unlink".into(), value: None }, CancellationToken::new()).await.unwrap();
    assert_eq!(std::fs::read_to_string(moved.join("user.txt")).unwrap(), "keep");
    assert!(manager.development_workspace("alice").await.unwrap().tasks.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn git_import_keeps_dirty_state_and_clone_preserves_source() {
    fn git(path: &std::path::Path, args: &[&str]) { assert!(std::process::Command::new("git").current_dir(path).args(args).status().unwrap().success()); }
    let root = root(); let source = root.join("source"); std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    std::fs::write(source.join("tracked.txt"), "base").unwrap();
    git(&source, &["add", "tracked.txt"]);
    git(&source, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "base"]);
    std::fs::write(source.join("tracked.txt"), "user change").unwrap();
    std::fs::write(source.join("untracked.txt"), "user new").unwrap();
    std::fs::write(source.join(".gitattributes"), "tracked.txt filter=untrusted\n").unwrap();
    git(&source, &["config", "filter.untrusted.clean", "touch filter-ran; cat"]);
    git(&source, &["config", "core.fsmonitor", "touch monitor-ran"]);
    let manager = crate::test_support::manager(&root).await;
    let project = manager.create_project("alice", import(&source), CancellationToken::new()).await.unwrap();
    let state = manager.inspect_project("alice", &project.id).await.unwrap();
    assert_eq!(state.kind, "git"); assert_eq!(state.branch.as_deref(), Some("main")); assert!(state.error.is_none());
    assert!(state.changes.iter().any(|c| c == " M tracked.txt"));
    assert!(state.changes.iter().any(|c| c == "?? untracked.txt"));
    assert!(!source.join("filter-ran").exists());
    assert!(!source.join("monitor-ran").exists());
    let target = root.join("clone");
    let cloned = manager.create_project("alice", CreateProjectInput { kind: "clone".into(), name: "Clone".into(), path: target.to_string_lossy().into(), source: Some(source.to_string_lossy().into()) }, CancellationToken::new()).await.unwrap();
    assert_eq!(std::fs::read_to_string(target.join("tracked.txt")).unwrap(), "base");
    assert_eq!(std::fs::read_to_string(source.join("tracked.txt")).unwrap(), "user change");
    assert_eq!(manager.inspect_project("alice", &cloned.id).await.unwrap().changes, Vec::<String>::new());
    let empty = root.join("empty");
    let created = manager.create_project("alice", CreateProjectInput { kind: "create".into(), name: "Empty".into(), path: empty.to_string_lossy().into(), source: None }, CancellationToken::new()).await.unwrap();
    std::fs::write(empty.join("keep.txt"), "keep").unwrap();
    manager.update_project("alice", UpdateProjectInput { project_id: created.id.clone(), action: "archive".into(), value: None }, CancellationToken::new()).await.unwrap();
    assert_eq!(std::fs::read_to_string(empty.join("keep.txt")).unwrap(), "keep");
    let invalid = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: created.id.clone(), goal: "No".into(), requirements: "Archived".into(), status: "todo".into() }, CancellationToken::new()).await;
    assert!(matches!(invalid, Err(e) if e == "project_archived"));
    manager.update_project("alice", UpdateProjectInput { project_id: created.id.clone(), action: "restore".into(), value: None }, CancellationToken::new()).await.unwrap();
    manager.update_project("alice", UpdateProjectInput { project_id: created.id.clone(), action: "rename".into(), value: Some("Renamed".into()) }, CancellationToken::new()).await.unwrap();
    assert!(manager.development_workspace("alice").await.unwrap().projects.iter().any(|p| p.id == created.id && p.name == "Renamed" && !p.archived));
    assert!(manager.development_workspace("bob").await.unwrap().projects.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn failed_clone_and_revoked_create_preserve_disk_and_report_operations() {
    let root = root(); let manager = crate::test_support::manager(&root).await;
    let target = root.join("failed-clone");
    assert!(manager.create_project("alice", CreateProjectInput { kind: "clone".into(), name: "Missing source".into(), path: target.to_string_lossy().into(), source: Some(root.join("missing-source").to_string_lossy().into()) }, CancellationToken::new()).await.is_err());
    assert!(target.is_dir());
    let workspace = manager.development_workspace("alice").await.unwrap();
    assert!(workspace.projects.is_empty());
    assert!(workspace.operations.iter().any(|op| op.path == target.to_string_lossy() && op.status == "failed"));
    let revoked = CancellationToken::new(); revoked.cancel();
    let cancelled = root.join("cancelled");
    assert!(manager.create_project("alice", CreateProjectInput { kind: "create".into(), name: "Cancelled".into(), path: cancelled.to_string_lossy().into(), source: None }, revoked).await.is_err());
    assert!(!cancelled.exists());
    std::fs::remove_dir_all(root).unwrap();
}
async fn admission_manager(root: &std::path::Path) -> AgentManager {
    let repository = crate::test_support::repository(root).await;
    let database_url = format!("sqlite://{}?mode=rwc", root.join("metadata.sqlite").display());
    let provider_manager = dgos_providers::ProviderManager::new(&database_url).await.unwrap();
    let providers = crate::capabilities::CliProviders::new(repository.clone(), provider_manager).unwrap();
    let manager = AgentManager::new(root.join("index.json"), repository, providers);
    manager.load().await.unwrap();
    manager
}

#[tokio::test]
async fn project_admission_checks_owner_archive_and_directory_identity() {
    let root = root();
    let path = root.join("project");
    std::fs::create_dir(&path).unwrap();
    let manager = admission_manager(&root).await;
    let project = manager.create_project("alice", import(&path), CancellationToken::new()).await.unwrap();

    assert!(manager.admit_native_project("bob", &project.id, &CancellationToken::new()).await.is_err());
    let admission = manager.admit_native_project("alice", &project.id, &CancellationToken::new()).await.unwrap();
    assert_eq!(admission.project.owner, "alice");
    drop(admission);

    manager.update_project("alice", UpdateProjectInput { project_id: project.id.clone(), action: "archive".into(), value: None }, CancellationToken::new()).await.unwrap();
    assert!(matches!(manager.admit_native_project("alice", &project.id, &CancellationToken::new()).await, Err(error) if error == "project_archived"));
    manager.update_project("alice", UpdateProjectInput { project_id: project.id.clone(), action: "restore".into(), value: None }, CancellationToken::new()).await.unwrap();

    std::fs::remove_dir_all(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(manager.admit_native_project("alice", &project.id, &CancellationToken::new()).await, Err(error) if error == "workspace_changed"));
    manager.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn project_admission_permit_serializes_archive_updates() {
    let root = root();
    let path = root.join("project");
    std::fs::create_dir(&path).unwrap();
    let manager = admission_manager(&root).await;
    let project = manager.create_project("alice", import(&path), CancellationToken::new()).await.unwrap();
    let admission = manager.admit_native_project("alice", &project.id, &CancellationToken::new()).await.unwrap();

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let update_manager = manager.clone();
    let update_id = project.id.clone();
    let update = tokio::spawn(async move {
        let _ = started_tx.send(());
        update_manager.update_project("alice", UpdateProjectInput { project_id: update_id, action: "archive".into(), value: None }, CancellationToken::new()).await
    });
    started_rx.await.unwrap();
    let mut update = update;
    assert!(tokio::time::timeout(std::time::Duration::from_millis(50), &mut update).await.is_err());

    drop(admission);
    assert!(update.await.unwrap().is_ok());
    assert!(matches!(manager.admit_native_project("alice", &project.id, &CancellationToken::new()).await, Err(error) if error == "project_archived"));
    manager.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}
