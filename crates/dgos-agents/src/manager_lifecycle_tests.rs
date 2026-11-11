use super::*;
use std::{path::PathBuf, time::Duration};

#[cfg(unix)]
struct Fixture {
    root: PathBuf,
    manager: AgentManager,
    task: String,
}

#[cfg(unix)]
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-retirement-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        crate::test_support::codex_fixture(&root.join("codex"), &r#"
while IFS= read -r line; do
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"model/list"'*) printf '%s\n' '{"id":4,"result":{"data":[],"nextCursor":null}}';;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":3,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"retired-thread","turns":[]}}}\n' "$PWD";;
esac
done
"#.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root).await;
        Self { root, manager, task }
    }

    async fn open(&self, id: &str) -> OwnedSessionView {
        let options = SessionOptions {
            workspace: self.root.to_string_lossy().into_owned(),
            executable: Some(self.root.join("codex").to_string_lossy().into_owned()),
            resume_id: None, model: None, trust_workspace: true,
        };
        let create = crate::test_support::create(&self.manager, "alice", &self.task, options).await;
        self.manager.open_session("alice", Engine::Codex, id.into(), create, Some(self.task.clone()), CancellationToken::new()).await.unwrap()
    }

    async fn cleanup_finished(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                self.manager.reap_completed_tasks().await;
                if self.manager.inner.cleanup_watchers.lock().await.is_empty() { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
    }
}

#[cfg(unix)]
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

#[test]
fn terminal_history_is_a_bounded_window_without_control_or_receipt_state() {
    let mut view = SessionView::new("/workspace".into());
    for index in 0..300 { view.push("output", Some("x".repeat(4096)), serde_json::json!({"index":index})); }
    view.active_run = Some("run".into());
    let terminal = terminal_view(view);
    assert_eq!(terminal.status, "closed");
    assert!(terminal.history_truncated);
    assert!(terminal.events.len() < 128);
    assert!(serde_json::to_vec(&terminal.events).unwrap().len() <= TERMINAL_EVENT_BYTES);
    assert!(terminal.active_run.is_none());
    assert!(terminal.controls.is_none());
    let mut enormous = SessionView::new("/workspace".into());
    enormous.push("output", Some("x".repeat(TERMINAL_EVENT_BYTES + 1)), Value::Null);
    assert!(terminal_view(enormous).events.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn repeated_close_history_and_resume_release_every_heavy_generation() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let mut generation = fixture.open(&id).await.generation;
    for _ in 0..20 {
        assert!(fixture.manager.close_session("alice", &id).await.unwrap());
        assert!(fixture.manager.close_session("alice", &id).await.unwrap());
        fixture.cleanup_finished().await;
        assert!(fixture.manager.inner.active.lock().await.is_empty());
        assert_eq!(fixture.manager.slot_count().await, 0);
        assert!(fixture.manager.inner.gate.begin_maintenance().is_ok());
        assert_eq!(fixture.manager.inner.terminal.lock().await.len(), 1);
        let closed = fixture.manager.snapshot("alice", &id).await.unwrap();
        assert_eq!(closed.generation, generation);
        assert_eq!(closed.view.status, "closed");
        assert_eq!(closed.view.native_id.as_deref(), Some("retired-thread"));
        fixture.manager.require_generation(&id, &generation).await.unwrap();
        assert!(fixture.manager.snapshot("bob", &id).await.is_err());
        let resumed = fixture.manager.resume_session("alice", &id, true, CancellationToken::new()).await.unwrap();
        assert_ne!(resumed.generation, generation);
        assert_eq!(fixture.manager.require_generation(&id, &generation).await.unwrap_err(), "stale_generation");
        generation = resumed.generation;
    }
    assert!(fixture.manager.close_session("alice", &id).await.unwrap());
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn reboot_cleanup_releases_recovered_slots_without_rewriting_unknown_history() {
    use dgos_db::sea_orm::ConnectionTrait;
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    fixture.open(&id).await;
    assert!(fixture.manager.close_session("alice", &id).await.unwrap());
    fixture.cleanup_finished().await;
    fixture.manager.inner.terminal.lock().await.clear();
    // Legacy records predate durable process ownership evidence.
    let binding = fixture.manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    std::fs::remove_file(fixture.root.join("cli-processes").join(format!("{}.ownership", binding.binding_id))).unwrap();
    let db = dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", fixture.root.join("metadata.sqlite").display())).await.unwrap();
    db.execute_unprepared("UPDATE development_cli_session_bindings SET state='unknown',cleanup_state='unknown',cleanup_proof=NULL,cleanup_reason='host_restart_cleanup_unconfirmed',boot_identity='machine:previous-boot'").await.unwrap();
    fixture.manager.restore_unsettled_slots().await.unwrap();
    assert_eq!(fixture.manager.slot_count().await, 1);
    assert_eq!(fixture.manager.snapshot("alice", &id).await.err().unwrap(), "cleanup_pending");
    let repo = &fixture.manager.inner.repository;
    assert_eq!(repo.recheck_cli_cleanup(&dgos_db::development_repository::HostBootObservation { boot_identity: "machine:previous-boot".into() }).await.unwrap(), 0);
    fixture.manager.reconcile_inactive_slots().await;
    assert_eq!(fixture.manager.slot_count().await, 1);
    assert_eq!(repo.recheck_cli_cleanup(&dgos_db::development_repository::HostBootObservation { boot_identity: "machine:next-boot".into() }).await.unwrap(), 1);
    fixture.manager.reconcile_inactive_slots().await;
    assert_eq!(fixture.manager.slot_count().await, 0);
    assert_eq!(fixture.manager.snapshot("alice", &id).await.unwrap().view.status, "closed");
    let binding = repo.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(binding.state, "unknown");
    assert_eq!(binding.cleanup_state, "stopped");
    assert_eq!(binding.cleanup_proof.as_deref(), Some("boot_changed"));
    fixture.manager.restore_unsettled_slots().await.unwrap();
    assert_eq!(fixture.manager.slot_count().await, 0);
    let fresh = Uuid::new_v4().to_string();
    assert_eq!(fixture.open(&fresh).await.view.status, "idle");
    assert!(fixture.manager.close_session("alice", &fresh).await.unwrap());
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_cache_eviction_keeps_durable_history_close_and_resume() {
    let fixture = Fixture::new().await;
    let mut ids = Vec::new();
    for _ in 0..TERMINAL_SESSION_LIMIT + 3 {
        let id = Uuid::new_v4().to_string();
        fixture.open(&id).await;
        assert!(fixture.manager.close_session("alice", &id).await.unwrap());
        fixture.cleanup_finished().await;
        ids.push(id);
    }
    assert_eq!(fixture.manager.inner.terminal.lock().await.len(), TERMINAL_SESSION_LIMIT);
    assert!(fixture.manager.inner.active.lock().await.is_empty());
    assert_eq!(fixture.manager.list("alice").await.len(), ids.len());
    let evicted = fixture.manager.snapshot("alice", &ids[0]).await.unwrap();
    assert_eq!(evicted.view.status, "closed");
    assert!(evicted.view.history_truncated);
    assert!(evicted.event_sync.unwrap().reset);
    assert!(fixture.manager.close_session("alice", &ids[0]).await.unwrap());
    fixture.manager.resume_session("alice", &ids[0], true, CancellationToken::new()).await.unwrap();
    assert!(fixture.manager.close_session("alice", &ids[0]).await.unwrap());
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn unsettled_run_projection_keeps_backend_slot_and_maintenance_permit() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let generation = fixture.open(&id).await.generation;
    let key = (id.clone(), generation.clone());
    fixture.manager.inner.run_projections.lock().await.insert(key.clone(), RunProjection { run_id: Uuid::new_v4().to_string(), persistence_failed: true });
    assert!(fixture.manager.close_session("alice", &id).await.unwrap());
    fixture.cleanup_finished().await;
    assert!(fixture.manager.inner.active.lock().await.get(&id).unwrap().lease.is_cancelled(), "execution is revoked while read-only persistence state remains observable");
    assert!(fixture.manager.inner.active.lock().await.contains_key(&id));
    assert_eq!(fixture.manager.slot_count().await, 1);
    assert!(fixture.manager.inner.gate.begin_maintenance().is_err());
    assert_eq!(fixture.manager.snapshot("alice", &id).await.unwrap().view.persistence_error.as_deref(), Some("storage_failed"));
    assert_eq!(fixture.manager.resume_session("alice", &id, true, CancellationToken::new()).await.err().unwrap(), "cleanup_pending");
    fixture.manager.inner.run_projections.lock().await.remove(&key);
    fixture.manager.retire_settled_session(&id).await;
    assert!(fixture.manager.inner.active.lock().await.is_empty());
    assert_eq!(fixture.manager.slot_count().await, 0);
    assert!(fixture.manager.inner.gate.begin_maintenance().is_ok());
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn failed_cleanup_persistence_keeps_resources_and_latches_supervision_failure() {
    use dgos_db::sea_orm::ConnectionTrait;
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    fixture.open(&id).await;
    let db = dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", fixture.root.join("metadata.sqlite").display())).await.unwrap();
    db.execute_unprepared("CREATE TRIGGER deny_cleanup BEFORE UPDATE ON development_cli_session_bindings WHEN NEW.cleanup_state='stopped' BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    assert!(fixture.manager.close_session("alice", &id).await.is_err());
    fixture.cleanup_finished().await;
    assert!(fixture.manager.inner.active.lock().await.contains_key(&id));
    assert_eq!(fixture.manager.slot_count().await, 1);
    assert!(fixture.manager.inner.gate.begin_maintenance().is_err());
    assert!(fixture.manager.inner.supervision_failed.load(std::sync::atomic::Ordering::Acquire));
    db.execute_unprepared("DROP TRIGGER deny_cleanup").await.unwrap();
    assert!(fixture.manager.close_session("alice", &id).await.unwrap());
    assert!(fixture.manager.inner.active.lock().await.is_empty());
    assert!(fixture.manager.inner.gate.begin_maintenance().is_ok());
    assert!(!fixture.manager.shutdown().await, "joining a failed cleanup cannot erase its failure");
}

#[cfg(unix)]
#[tokio::test]
async fn late_cleanup_cannot_settle_or_release_a_new_generation() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    fixture.open(&id).await;
    let (old_slot, binding) = fixture.manager.inner.active.lock().await.get(&id).map(|entry| (entry.slot, entry.binding.clone())).unwrap();
    assert!(fixture.manager.close_session("alice", &id).await.unwrap());
    fixture.manager.resume_session("alice", &id, true, CancellationToken::new()).await.unwrap();
    fixture.manager.inner.providers.cleanup("alice", &id, &binding, true, true).await.unwrap();
    fixture.manager.mark_cleanup_settled(&id, old_slot).await;
    fixture.manager.release_slot(&id, old_slot).await;
    fixture.manager.retire_settled_session(&id).await;
    let active = fixture.manager.inner.active.lock().await;
    let current = active.get(&id).unwrap();
    assert_ne!(current.slot, old_slot);
    assert!(!current.cleanup_settled);
    drop(active);
    assert_eq!(fixture.manager.slot_count().await, 1);
    let binding = fixture.manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(binding.state, "active");
    assert!(fixture.manager.close_session("alice", &id).await.unwrap());
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn reaping_completed_handles_bounds_storage_without_losing_false_or_panic() {
    let fixture = Fixture::new().await;
    for _ in 0..64 {
        fixture.manager.inner.watchers.lock().await.push(tokio::spawn(async { true }));
        fixture.manager.inner.cleanup_watchers.lock().await.push(tokio::spawn(async { true }));
    }
    fixture.manager.inner.watchers.lock().await.push(tokio::spawn(async { false }));
    fixture.manager.inner.cleanup_watchers.lock().await.push(tokio::spawn(async { panic!("injected cleanup panic"); }));
    fixture.cleanup_finished().await;
    fixture.manager.reap_completed_tasks().await;
    assert!(fixture.manager.inner.watchers.lock().await.is_empty());
    assert!(fixture.manager.inner.cleanup_watchers.lock().await.is_empty());
    assert!(!fixture.manager.shutdown().await);
    assert!(!fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_timeout_keeps_the_only_cleanup_handle_until_it_can_settle() {
    let fixture = Fixture::new().await;
    let (send, receive) = tokio::sync::oneshot::channel();
    fixture.manager.inner.cleanup_watchers.lock().await.push(tokio::spawn(async move { receive.await.unwrap_or(false) }));
    let started = tokio::time::Instant::now();
    assert!(!fixture.manager.shutdown().await);
    assert!(started.elapsed() < Duration::from_secs(7));
    assert_eq!(fixture.manager.inner.cleanup_watchers.lock().await.len(), 1);
    send.send(true).unwrap();
    fixture.cleanup_finished().await;
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn unowned_unknown_cleanup_cannot_claim_close_or_replace_its_slot_on_resume() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    fixture.open(&id).await;
    // Recover while the old host still owns its child: no fabricated downgrade
    // of a durable stopped receipt, which the repository correctly rejects.
    let recovered = crate::test_support::manager(&fixture.root).await;
    let unresolved_slot = *recovered.inner.slots.lock().await.get(&id).unwrap();
    assert_eq!(recovered.close_session("alice", &id).await.unwrap_err(), "cleanup_pending");
    assert_eq!(recovered.snapshot("alice", &id).await.err().unwrap(), "cleanup_pending");
    assert_eq!(recovered.resume_session("alice", &id, true, CancellationToken::new()).await.err().unwrap(), "cleanup_pending");
    assert_eq!(recovered.inner.slots.lock().await.get(&id), Some(&unresolved_slot));
    assert!(recovered.inner.active.lock().await.is_empty());
    assert!(!recovered.shutdown().await);
    assert!(fixture.manager.shutdown().await);
    assert!(recovered.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn stopped_generation_invalidates_only_its_child_lease_and_never_a_stale_slot_or_account() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let account = CancellationToken::new();
    let options = SessionOptions { workspace: fixture.root.to_string_lossy().into_owned(), executable: Some(fixture.root.join("codex").to_string_lossy().into_owned()), resume_id: None, model: None, trust_workspace: true };
    let create = crate::test_support::create(&fixture.manager, "alice", &fixture.task, options).await;
    fixture.manager.open_session("alice", Engine::Codex, id.clone(), create, Some(fixture.task.clone()), account.clone()).await.unwrap();
    let (slot, generation_lease) = {
        let active = fixture.manager.inner.active.lock().await;
        let entry = active.get(&id).unwrap();
        (entry.slot, entry.lease.clone())
    };
    fixture.manager.cancel_stopped_generation(&id, Uuid::new_v4()).await;
    assert!(!generation_lease.is_cancelled());
    fixture.manager.cancel_stopped_generation(&id, slot).await;
    assert!(generation_lease.is_cancelled());
    assert!(!account.is_cancelled());
    assert!(fixture.manager.close_session("alice", &id).await.unwrap());
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn absent_start_reconciliation_fences_late_create_without_launching_or_sending() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let lease = CancellationToken::new();
    assert!(fixture.manager.reconcile_start("bob", &id, &fixture.task, &lease).await.is_err());
    let revoked = CancellationToken::new(); revoked.cancel();
    assert_eq!(fixture.manager.reconcile_start("alice", &id, &fixture.task, &revoked).await.err().unwrap(), "session_invalid");
    assert_eq!(fixture.manager.reconcile_start("alice", &id, &fixture.task, &lease).await.unwrap().status, "not_started");
    assert_eq!(fixture.manager.reconcile_start("alice", &id, &fixture.task, &lease).await.unwrap().status, "not_started");
    let options = SessionOptions { workspace: fixture.root.to_string_lossy().into_owned(), executable: Some(fixture.root.join("codex").to_string_lossy().into_owned()), resume_id: None, model: None, trust_workspace: true };
    let create = crate::test_support::create(&fixture.manager, "alice", &fixture.task, options).await;
    assert_eq!(fixture.manager.open_session("alice", Engine::Codex, id.clone(), create, Some(fixture.task.clone()), lease).await.err().unwrap(), "cancelled");
    assert!(fixture.manager.list("alice").await.is_empty());
    assert_eq!(fixture.manager.slot_count().await, 0);
    let new_id = Uuid::new_v4().to_string();
    assert_eq!(fixture.open(&new_id).await.view.status, "idle");
    assert_eq!(fixture.manager.reconcile_start("alice", &new_id, &fixture.task, &CancellationToken::new()).await.unwrap().status, "ready");
    assert_eq!(fixture.manager.snapshot("alice", &new_id).await.unwrap().view.status, "idle");
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn durable_start_without_memory_projection_is_observed_not_cancelled() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let options = SessionOptions { workspace: fixture.root.to_string_lossy().into_owned(), executable: None, resume_id: None, model: None, trust_workspace: true };
    let create = crate::test_support::create(&fixture.manager, "alice", &fixture.task, options.clone()).await;
    let project = fixture.manager.task_project("alice", &fixture.task).await.unwrap();
    let launch = fixture.manager.inner.providers.prepare("alice", &id, &fixture.task, &project, Engine::Codex, &options, create.project_revision, create.provider, &CancellationToken::new()).await.unwrap();
    assert!(!fixture.manager.inner.records.lock().await.iter().any(|record| record.id == id));
    assert_eq!(fixture.manager.reconcile_start("alice", &id, &fixture.task, &CancellationToken::new()).await.unwrap().status, "recovering");
    assert_eq!(fixture.manager.snapshot("alice", &id).await.err().unwrap(), "cleanup_pending");
    assert!(!fixture.manager.inner.cancelled_starts.lock().await.get("alice").is_some_and(|ids| ids.contains(&Uuid::parse_str(&id).unwrap())));
    fixture.manager.inner.providers.cleanup("alice", &id, &launch.binding, true, false).await.unwrap();
    assert_eq!(fixture.manager.reconcile_start("alice", &id, &fixture.task, &CancellationToken::new()).await.unwrap().status, "closed");
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn failed_unstarted_cleanup_persistence_recovers_without_losing_the_reservation() {
    use dgos_db::sea_orm::ConnectionTrait;
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let db = dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", fixture.root.join("metadata.sqlite").display())).await.unwrap();
    db.execute_unprepared("CREATE TRIGGER deny_start_cleanup BEFORE UPDATE ON development_cli_session_bindings WHEN NEW.cleanup_state='stopped' BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let options = SessionOptions { workspace: fixture.root.to_string_lossy().into_owned(), executable: Some(fixture.root.join("missing-cli").to_string_lossy().into_owned()), resume_id: None, model: None, trust_workspace: true };
    let create = crate::test_support::create(&fixture.manager, "alice", &fixture.task, options).await;
    assert_eq!(fixture.manager.open_session("alice", Engine::Codex, id.clone(), create, Some(fixture.task.clone()), CancellationToken::new()).await.err().unwrap(), "cleanup_pending");
    assert!(fixture.manager.inner.records.lock().await.iter().any(|record| record.id == id));
    assert_eq!(fixture.manager.slot_count().await, 1);
    assert_eq!(fixture.manager.workspace_available(&fixture.root.canonicalize().unwrap().to_string_lossy(), None).await.err().unwrap(), "cleanup_pending");
    db.execute_unprepared("DROP TRIGGER deny_start_cleanup").await.unwrap();
    assert_eq!(fixture.manager.reconcile_start("alice", &id, &fixture.task, &CancellationToken::new()).await.unwrap().status, "closed");
    assert_eq!(fixture.manager.slot_count().await, 0);
    assert!(fixture.manager.inner.retained.lock().await.is_empty());
    let binding = fixture.manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(binding.cleanup_proof.as_deref(), Some("no_execution"));
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn unlink_automatically_closes_owned_session_without_deleting_project_files() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    fixture.open(&id).await;
    std::fs::write(fixture.root.join("user-code.txt"), "keep user's code").unwrap();
    let project = fixture.manager.task_project("alice", &fixture.task).await.unwrap();
    fixture.manager.update_project("alice", crate::projects::UpdateProjectInput { project_id: project.id.clone(), action: "unlink".into(), value: None }, CancellationToken::new()).await.unwrap();
    assert_eq!(fixture.manager.inner.repository.project("alice", &project.id).await.err().unwrap(), "project_not_found");
    assert_eq!(std::fs::read_to_string(fixture.root.join("user-code.txt")).unwrap(), "keep user's code");
    let history = fixture.manager.snapshot("alice", &id).await.unwrap();
    assert_eq!(history.view.status, "closed");
    assert_eq!(history.view.native_id.as_deref(), Some("retired-thread"));
    assert!(matches!(history.configuration, crate::capabilities::SessionConfigurationView::LegacyHistory { .. }));
    assert!(fixture.manager.inner.active.lock().await.is_empty());
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn task_preparation_is_idempotent_and_cannot_overwrite_a_different_request() {
    let fixture = Fixture::new().await;
    let project = fixture.manager.task_project("alice", &fixture.task).await.unwrap();
    let id = Uuid::new_v4().to_string();
    let lease = CancellationToken::new();
    let first = fixture.manager.prepare_agent_task("alice", &id, &project.id, "First conversation", &lease).await.unwrap();
    let again = fixture.manager.prepare_agent_task("alice", &id, &project.id, "First conversation", &lease).await.unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(first.created_at, again.created_at);
    assert_eq!(fixture.manager.prepare_agent_task("alice", &id, &project.id, "Different conversation", &lease).await.err().unwrap(), "request_conflict");
    assert!(fixture.manager.prepare_agent_task("bob", &id, &project.id, "First conversation", &lease).await.is_err());
    assert_eq!(fixture.manager.inner.repository.task("alice", &id).await.unwrap().goal, "First conversation");
    assert!(fixture.manager.shutdown().await);
}

#[cfg(unix)]
#[tokio::test]
async fn unlink_keeps_untraceable_legacy_cleanup_quarantined_to_its_workspace() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let options = SessionOptions { workspace: fixture.root.canonicalize().unwrap().to_string_lossy().into_owned(), executable: None, resume_id: None, model: None, trust_workspace: true };
    let create = crate::test_support::create(&fixture.manager, "alice", &fixture.task, options.clone()).await;
    let project = fixture.manager.task_project("alice", &fixture.task).await.unwrap();
    let launch = fixture.manager.inner.providers.prepare("alice", &id, &fixture.task, &project, Engine::Codex, &options, create.project_revision, create.provider, &CancellationToken::new()).await.unwrap();
    let binding_id = launch.binding.binding_id.clone();
    drop(launch);
    std::fs::remove_file(fixture.root.join("cli-processes").join(format!("{binding_id}.ownership"))).unwrap();
    let recovered = crate::test_support::manager(&fixture.root).await;
    recovered.update_project("alice", crate::projects::UpdateProjectInput { project_id: project.id.clone(), action: "unlink".into(), value: None }, CancellationToken::new()).await.unwrap();
    assert!(recovered.inner.repository.project("alice", &project.id).await.is_err());
    let binding = recovered.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(binding.cleanup_state, "unknown");
    assert_eq!(recovered.workspace_available(&project.path, None).await.err().unwrap(), "cleanup_pending");
    assert!(recovered.workspace_available("/an/unrelated/workspace", None).await.is_ok());
    assert_eq!(recovered.occupied_slots().await, 0, "unowned history must not freeze unrelated projects");
    assert!(fixture.manager.shutdown().await);
    assert!(!recovered.shutdown().await, "unknown cleanup is not a fabricated stopped receipt");
}

#[cfg(unix)]
#[tokio::test]
async fn no_execution_owner_can_retry_the_same_uuid_after_its_slot_is_released() {
    let fixture = Fixture::new().await;
    let id = Uuid::new_v4().to_string();
    let executable = fixture.root.join("retry-codex");
    let options = SessionOptions { workspace: fixture.root.to_string_lossy().into_owned(), executable: Some(executable.to_string_lossy().into_owned()), resume_id: None, model: None, trust_workspace: true };
    let create = crate::test_support::create(&fixture.manager, "alice", &fixture.task, options.clone()).await;
    assert!(fixture.manager.open_session("alice", Engine::Codex, id.clone(), create, Some(fixture.task.clone()), CancellationToken::new()).await.is_err());
    let failed = fixture.manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_eq!(failed.cleanup_proof.as_deref(), Some("no_execution"));
    assert_eq!(fixture.manager.slot_count().await, 0);

    // Keep the exact request and move the already-admitted fixture executable
    // into place; the original owner's safe retry is not a UUID collision.
    std::fs::rename(fixture.root.join("codex"), &executable).unwrap();
    let create = crate::test_support::create(&fixture.manager, "alice", &fixture.task, options).await;
    let reopened = fixture.manager.open_session("alice", Engine::Codex, id.clone(), create, Some(fixture.task.clone()), CancellationToken::new()).await.unwrap();
    assert_eq!(reopened.view.status, "idle");
    assert_eq!(fixture.manager.slot_count().await, 1);
    let retried = fixture.manager.inner.repository.read_cli_session_configuration("alice", &id).await.unwrap().binding.unwrap();
    assert_ne!(retried.binding_id, failed.binding_id);
    assert_eq!(retried.state, "active");
    assert!(fixture.manager.close_session("alice", &id).await.unwrap());
    assert_eq!(fixture.manager.slot_count().await, 0);
    assert!(fixture.manager.shutdown().await);
}
