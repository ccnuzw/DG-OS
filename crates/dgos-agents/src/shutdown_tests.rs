#![cfg(unix)]
use crate::{Engine, SessionOptions};
use std::{path::{Path, PathBuf}, time::Duration};
use tokio_util::sync::CancellationToken;

struct Fixture(PathBuf);
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-shutdown-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("codex");
        crate::test_support::codex_fixture(&executable, &r##"
printf '%s' "$$" > pid
while IFS= read -r line; do
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*)
 if [ -f reject-open ]; then printf '%s\n' '{"id":1,"error":{"code":-1,"message":"credential-SENTINEL"}}'; continue; fi
 if [ -f block-open ]; then touch entered; while :; do sleep 1; done; fi
 printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":3,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"saved-thread","turns":[]}}}\n' "$PWD";;
*'"method":"turn/start"'*) touch entered; while :; do sleep 1; done;;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        Self(root)
    }
    fn options(&self) -> SessionOptions {
        SessionOptions { workspace: self.0.to_string_lossy().into(), executable: Some(self.0.join("codex").to_string_lossy().into()), resume_id: None, model: None, trust_workspace: true }
    }
    fn block(&self) { std::fs::write(self.0.join("block-open"), "").unwrap(); }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
async fn entered(root: &Path) -> i32 {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.join("entered").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    std::fs::read_to_string(root.join("pid")).unwrap().parse().unwrap()
}
async fn opening_shutdown(resume: bool) {
    let fixture = Fixture::new().await;
    let manager = crate::test_support::manager(&fixture.0).await;
    let task_id = crate::test_support::task(&manager, &fixture.0).await;
    let id = uuid::Uuid::new_v4().to_string();
    if resume {
        manager.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&manager, "alice", &task_id, fixture.options()).await, Some(task_id.clone()), CancellationToken::new()).await.unwrap();
        assert!(manager.close_session("alice", &id).await.unwrap());
    }
    fixture.block();
    let worker = manager.clone(); let session_id = id.clone();
    let create = crate::test_support::create(&manager, "alice", &task_id, fixture.options()).await;
    let task_id_for_open = task_id.clone();
    let opening = tokio::spawn(async move {
        if resume { worker.resume_session("alice", &session_id, true, CancellationToken::new()).await }
        else { worker.open_session("alice", Engine::Codex, session_id, create, Some(task_id_for_open), CancellationToken::new()).await }
    });
    let pid = entered(&fixture.0).await;
    let closed = manager.shutdown().await;
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    // Cleanup on assertion failure must not leave a child from the regression itself.
    if alive { unsafe { libc::kill(-pid, libc::SIGKILL); } }
    assert!(closed);
    assert!(!alive, "shutdown returned while handshake child was alive");
    assert!(tokio::time::timeout(Duration::from_secs(2), opening).await.unwrap().unwrap().is_err());
    assert!(manager.snapshot("alice", &id).await.is_err());
    assert_eq!(manager.list("alice").await.len(), 1, "failed opening remains durable history");
    assert!(manager.open_session("alice", Engine::Codex, uuid::Uuid::new_v4().to_string(), crate::test_support::create(&manager, "alice", &task_id, fixture.options()).await, Some(task_id), CancellationToken::new()).await.is_err());
}
#[tokio::test]
async fn shutdown_reaps_an_unregistered_connecting_child() { opening_shutdown(false).await; }
#[tokio::test]
async fn shutdown_reaps_a_resuming_child_without_late_admission() { opening_shutdown(true).await; }
#[tokio::test]
async fn shutdown_rejects_an_inflight_send_and_reaps_its_process() {
    let fixture = Fixture::new().await; let manager = crate::test_support::manager(&fixture.0).await;
    let development_task = crate::test_support::task(&manager, &fixture.0).await;
    let id = uuid::Uuid::new_v4().to_string();
    manager.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&manager, "alice", &development_task, fixture.options()).await, Some(development_task), CancellationToken::new()).await.unwrap();
    let generation = manager.snapshot("alice", &id).await.unwrap().generation;
    let worker = manager.clone(); let task_id = id.clone();
    let send = tokio::spawn(async move { worker.send("alice", &task_id, &generation, uuid::Uuid::new_v4().to_string(), "task".into()).await });
    let pid = entered(&fixture.0).await;
    assert!(manager.shutdown().await);
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    assert!(tokio::time::timeout(Duration::from_secs(2), send).await.unwrap().unwrap().is_err());
    assert!(manager.snapshot("alice", &id).await.is_err());
}

#[tokio::test]
async fn codex_handshake_diagnostics_cover_success_rejection_and_lease_cancellation() {
    use tracing::instrument::WithSubscriber;
    let (log, subscriber) = crate::test_support::DiagnosticLog::capture();
    let fixture = Fixture::new().await;
    let manager = crate::test_support::manager(&fixture.0).await;
    let task = crate::test_support::task(&manager, &fixture.0).await;
    let id = uuid::Uuid::new_v4().to_string();
    manager.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&manager, "alice", &task, fixture.options()).await, Some(task.clone()), CancellationToken::new()).with_subscriber(subscriber.clone()).await.unwrap();
    assert!(manager.close_session("alice", &id).await.unwrap());
    std::fs::write(fixture.0.join("reject-open"), "").unwrap();
    let error = manager.resume_session("alice", &id, true, CancellationToken::new()).with_subscriber(subscriber.clone()).await.err().unwrap();
    assert_eq!(error, "engine_rejected");
    let rejected_pid: i32 = std::fs::read_to_string(fixture.0.join("pid")).unwrap().parse().unwrap();
    assert_ne!(unsafe { libc::kill(rejected_pid, 0) }, 0, "failed handshake returned before reaping");
    std::fs::remove_file(fixture.0.join("reject-open")).unwrap();
    fixture.block();
    let lease = CancellationToken::new();
    let opening_lease = lease.clone();
    let worker = manager.clone();
    let opening = tokio::spawn(async move { worker.resume_session("alice", &id, true, opening_lease).await }.with_subscriber(subscriber));
    let pid = entered(&fixture.0).await;
    lease.cancel();
    let error = tokio::time::timeout(Duration::from_secs(4), opening).await.unwrap().unwrap().err().unwrap();
    assert!(matches!(error.as_str(), "connection_lost" | "closed"));
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "cancelled handshake returned before reaping");
    let text = log.text();
    assert!(text.contains("stage=\"handshake_initialize\"") && text.contains("stage=\"handshake_thread\""), "{text}");
    assert!(text.contains("error_class=\"engine_rejected\""), "{text}");
    assert!(text.contains("outcome=\"cancelled\"") && text.contains("stage=\"cleanup\"") && text.contains("stage=\"exit_wait\""), "{text}");
    assert!(!text.contains("credential-SENTINEL") && !text.contains(&fixture.0.to_string_lossy().to_string()), "{text}");
    eprint!("{text}");
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn startup_failure_distinguishes_no_execution_from_owned_exit() {
    let fixture = Fixture::new().await;
    let manager = crate::test_support::manager(&fixture.0).await;
    let task = crate::test_support::task(&manager, &fixture.0).await;
    let missing = uuid::Uuid::new_v4().to_string();
    let mut options = fixture.options();
    options.executable = Some(fixture.0.join("missing").to_string_lossy().into());
    let create = crate::test_support::create(&manager, "alice", &task, options).await;
    assert!(manager.open_session("alice", Engine::Codex, missing.clone(), create, Some(task.clone()), CancellationToken::new()).await.is_err());
    let binding = manager.inner.repository.read_cli_session_configuration("alice", &missing).await.unwrap().binding.unwrap();
    assert_eq!(binding.cleanup_proof.as_deref(), Some("no_execution"));
    std::fs::write(fixture.0.join("reject-open"), "").unwrap();
    let rejected = uuid::Uuid::new_v4().to_string();
    let create = crate::test_support::create(&manager, "alice", &task, fixture.options()).await;
    assert!(manager.open_session("alice", Engine::Codex, rejected.clone(), create, Some(task), CancellationToken::new()).await.is_err());
    assert!(manager.shutdown().await);
    let binding = manager.inner.repository.read_cli_session_configuration("alice", &rejected).await.unwrap().binding.unwrap();
    assert_eq!(binding.cleanup_proof.as_deref(), Some("owned_exit"));
    assert_eq!(binding.cleanup_state, "stopped");
}

#[tokio::test]
async fn active_provider_disable_and_reenable_never_resurrects_and_shutdown_drains_sink() {
    let fixture = Fixture::new().await;
    let manager = crate::test_support::manager(&fixture.0).await;
    let task = crate::test_support::task(&manager, &fixture.0).await;
    let id = uuid::Uuid::new_v4().to_string();
    let create = crate::test_support::create(&manager, "alice", &task, fixture.options()).await;
    manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task), CancellationToken::new()).await.unwrap();
    let generation = manager.snapshot("alice", &id).await.unwrap().generation;
    let providers = manager.inner.providers.providers();
    let disabled = providers.toggle_provider("fixture-alice", false, "alice").await.unwrap();
    // Do not wait for the asynchronous sink: reenable must serialize its write
    // against cleanup from the separate DevelopmentRepository pool.
    let enabled = providers.toggle_provider("fixture-alice", true, "alice").await.unwrap();
    assert!(!disabled.enabled && enabled.enabled);
    assert_eq!(enabled.config_revision, disabled.config_revision + 1);
    assert!(manager.send("alice", &id, &generation, uuid::Uuid::new_v4().to_string(), "must never run".into()).await.is_err());
    assert!(manager.shutdown().await);
    let binding = manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(binding.state, "revoked");
    assert_eq!(binding.cleanup_state, "stopped");
    assert_eq!(binding.cleanup_proof.as_deref(), Some("owned_exit"));
    let pid: i32 = std::fs::read_to_string(fixture.0.join("pid")).unwrap().parse().unwrap();
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "shutdown must reap the invalidated child");
    assert!(manager.resume_session("alice", &id, true, CancellationToken::new()).await.is_err());
}

#[tokio::test]
async fn recovered_unknown_binding_blocks_only_its_actual_directory() {
    let fixture = Fixture::new().await;
    let original = crate::test_support::manager(&fixture.0).await;
    let task = crate::test_support::task(&original, &fixture.0).await;
    let id = uuid::Uuid::new_v4().to_string();
    original.open_session("alice", Engine::Codex, id.clone(), crate::test_support::create(&original, "alice", &task, fixture.options()).await, Some(task), CancellationToken::new()).await.unwrap();
    // A new host cannot claim ownership or exit proof for the previous host's child.
    let restarted = crate::test_support::manager(&fixture.0).await;
    let binding = restarted.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(binding.state, "unknown");
    assert!(binding.cleanup_proof.is_none());
    let directory = fixture.0.canonicalize().unwrap();
    assert_eq!(restarted.workspace_available(directory.to_str().unwrap(), None).await.unwrap_err(), "cleanup_pending");
    let other = fixture.0.join("other"); std::fs::create_dir(&other).unwrap();
    assert!(restarted.workspace_available(other.to_str().unwrap(), None).await.is_ok());
    assert!(original.shutdown().await);
    assert!(restarted.shutdown().await);
}

#[tokio::test]
async fn opening_project_revoke_stops_child_before_late_ready() {
    let fixture = Fixture::new().await; fixture.block();
    let manager = crate::test_support::manager(&fixture.0).await;
    let task = crate::test_support::task(&manager, &fixture.0).await;
    let project = manager.inner.repository.task("alice", &task).await.unwrap().project_id;
    let id = uuid::Uuid::new_v4().to_string();
    let create = crate::test_support::create(&manager, "alice", &task, fixture.options()).await;
    let worker = manager.clone(); let session = id.clone();
    let opening = tokio::spawn(async move { worker.open_session("alice", Engine::Codex, session, create, Some(task), CancellationToken::new()).await });
    let pid = entered(&fixture.0).await;
    manager.inner.providers.revoke("alice", &project).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(8), opening).await.unwrap().unwrap().is_err());
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    assert!(manager.shutdown().await);
    let binding = manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(binding.state, "revoked");
    assert_eq!(binding.cleanup_proof.as_deref(), Some("owned_exit"));
}

#[tokio::test]
async fn create_close_cycles_release_capacity_after_cleanup_converges() {
    let fixture = Fixture::new().await;
    let manager = crate::test_support::manager(&fixture.0).await;
    let task = crate::test_support::task(&manager, &fixture.0).await;
    for _ in 0..10 {
        let id = uuid::Uuid::new_v4().to_string();
        let create = crate::test_support::create(&manager, "alice", &task, fixture.options()).await;
        manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task.clone()), CancellationToken::new()).await.unwrap();
        assert!(manager.close_session("alice", &id).await.unwrap());
    }
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn no_execution_session_id_can_retry_only_the_same_binding_input() {
    let fixture = Fixture::new().await;
    let manager = crate::test_support::manager(&fixture.0).await;
    let task = crate::test_support::task(&manager, &fixture.0).await;
    let id = uuid::Uuid::new_v4().to_string();
    let retry_executable = fixture.0.join("retry-codex");
    let mut options = fixture.options();
    options.executable = Some(retry_executable.to_string_lossy().into_owned());
    let create = crate::test_support::create(&manager, "alice", &task, options.clone()).await;
    assert!(manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task.clone()), CancellationToken::new()).await.is_err());
    let binding = manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(binding.cleanup_proof.as_deref(), Some("no_execution"));
    let mut changed = options.clone();
    changed.executable = Some(fixture.0.join("different-codex").to_string_lossy().into_owned());
    let changed_create = crate::test_support::create(&manager, "alice", &task, changed).await;
    assert_eq!(manager.open_session("alice", Engine::Codex, id.clone(), changed_create, Some(task.clone()), CancellationToken::new()).await.err().unwrap(), "session_exists");
    crate::test_support::codex_fixture(&retry_executable, &r##"
while IFS= read -r line; do
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*) printf '{"id":3,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"saved-thread","turns":[]}}}\n' "$PWD";;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
    let create = crate::test_support::create(&manager, "alice", &task, options).await;
    manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task), CancellationToken::new()).await.unwrap();
    assert!(manager.close_session("alice", &id).await.unwrap());
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn unknown_cleanup_keeps_its_capacity_slot() {
    let fixture = Fixture::new().await;
    let manager = crate::test_support::manager(&fixture.0).await;
    let mut tasks = Vec::new();
    for index in 0..8 {
        let workspace = fixture.0.join(format!("workspace-{index}"));
        std::fs::create_dir_all(&workspace).unwrap();
        let project = manager.create_project("alice", crate::projects::CreateProjectInput { kind: "import".into(), name: format!("Project {index}"), path: workspace.to_string_lossy().into_owned(), source: None }, CancellationToken::new()).await.unwrap();
        tasks.push(manager.save_development_task("alice", crate::projects::SaveTaskInput { id: None, project_id: project.id, goal: "Occupy a session slot".into(), requirements: "Retain uncertain sessions".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap().id);
    }
    let uncertain_id = uuid::Uuid::new_v4().to_string();
    let mut missing = fixture.options();
    missing.executable = Some(fixture.0.join("absent").to_string_lossy().into_owned());
    let create = crate::test_support::create(&manager, "alice", &tasks[0], missing).await;
    assert!(manager.open_session("alice", Engine::Codex, uncertain_id.clone(), create, Some(tasks[0].clone()), CancellationToken::new()).await.is_err());
    let binding = manager.inner.repository.read_cli_session_configuration("alice", &uncertain_id).await.unwrap().binding.unwrap();
    manager.inner.repository.record_cli_cleanup("alice", &uncertain_id, &binding.binding_id, binding.host_instance_id.as_deref().unwrap(), &dgos_db::development_repository::CleanupObservation { binding_id: binding.binding_id.clone(), state: "unknown".into(), proof: String::new(), reason: Some("cleanup_pending".into()) }).await.unwrap();
    manager.inner.slots.lock().await.insert(uncertain_id.clone(), uuid::Uuid::new_v4());
    for task in tasks.iter().skip(1).take(7) {
        let id = uuid::Uuid::new_v4().to_string();
        let create = crate::test_support::create(&manager, "alice", task, fixture.options()).await;
        manager.open_session("alice", Engine::Codex, id, create, Some(task.clone()), CancellationToken::new()).await.unwrap();
    }
    let unused = fixture.0.join("unused");
    std::fs::create_dir_all(&unused).unwrap();
    let extra_task = crate::test_support::task(&manager, &unused).await;
    let create = crate::test_support::create(&manager, "alice", &extra_task, fixture.options()).await;
    assert_eq!(manager.open_session("alice", Engine::Codex, uuid::Uuid::new_v4().to_string(), create, Some(extra_task), CancellationToken::new()).await.err().unwrap(), "session_capacity");
    assert!(manager.shutdown().await);
}

#[tokio::test]
async fn natural_exit_releases_capacity_only_after_cleanup_converges() {
    let fixture = Fixture::new().await;
    crate::test_support::codex_fixture(&fixture.0.join("codex"), &r##"
while IFS= read -r line; do
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*)
 printf '{"id":3,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","thread":{"id":"saved-thread","turns":[]}}}\n' "$PWD"
 while [ ! -f "$PWD/exit" ]; do sleep 0.01; done
 exit 0;;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
    let manager = crate::test_support::manager(&fixture.0).await;
    let mut tasks = Vec::new();
    for index in 0..9 {
        let workspace = fixture.0.join(format!("natural-{index}"));
        std::fs::create_dir_all(&workspace).unwrap();
        let project = manager.create_project("alice", crate::projects::CreateProjectInput { kind: "import".into(), name: format!("Natural {index}"), path: workspace.to_string_lossy().into_owned(), source: None }, CancellationToken::new()).await.unwrap();
        tasks.push(manager.save_development_task("alice", crate::projects::SaveTaskInput { id: None, project_id: project.id, goal: "Test natural exit".into(), requirements: "Release capacity after cleanup".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap().id);
    }
    let mut ids = Vec::new();
    for task in tasks.iter().take(8) {
        let id = uuid::Uuid::new_v4().to_string();
        let create = crate::test_support::create(&manager, "alice", task, fixture.options()).await;
        manager.open_session("alice", Engine::Codex, id.clone(), create, Some(task.clone()), CancellationToken::new()).await.unwrap();
        ids.push(id);
    }
    let first_workspace = fixture.0.join("natural-0");
    std::fs::write(first_workspace.join("exit"), "").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let binding = manager.inner.repository.read_cli_session_configuration("alice", &ids[0]).await.unwrap().binding.unwrap();
            if binding.cleanup_state == "stopped" && manager.slot_count().await == 7 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let create = crate::test_support::create(&manager, "alice", &tasks[8], fixture.options()).await;
    manager.open_session("alice", Engine::Codex, id.clone(), create, Some(tasks[8].clone()), CancellationToken::new()).await.unwrap();
    assert!(manager.close_session("alice", &id).await.unwrap());
    assert!(manager.shutdown().await);
}
