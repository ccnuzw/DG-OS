//! Deterministic native-protocol/SQLite faults, not installed CLI compatibility evidence.
#![cfg(unix)]
use crate::{AgentManager, Engine, SessionOptions};
use dgos_db::sea_orm::ConnectionTrait;
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

struct Fixture { root: PathBuf, manager: AgentManager, task: String, id: String }
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("code")).unwrap();
        let root = root.canonicalize().unwrap();
        std::fs::write(root.join("code/source.txt"), "user code\n").unwrap();
        let executable = root.join("codex");
        crate::test_support::codex_fixture(&executable, &r##"
printf '%s' "$$" > pid
while IFS= read -r line; do
request=${line#*\"id\":}; request=${request%%,*}
case "$line" in
CONFIG_REPLY
*'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
*'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$request";;
*'"method":"thread/start"'*) printf 'start\n' >> opens; printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"saved-thread","turns":[]}}}\n' "$request" "$PWD";;
*'"method":"thread/resume"'*)
 printf 'resume\n' >> opens
 if [ -f missing-history ]; then printf '{"id":%s,"error":{"code":-1,"message":"native history missing"}}\n' "$request"
 else printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"'"$fixture_provider"'","thread":{"id":"saved-thread","turns":[]}}}\n' "$request" "$PWD"; fi;;
*'"method":"turn/start"'*)
 printf 'once\n' >> invocations
 printf 'edited by native fixture\n' > source.txt
 if [ -f delay-ack ]; then touch dispatched; while [ ! -f release-ack ]; do sleep 0.01; done; fi
 printf '{"id":%s,"result":{"turn":{"id":"turn-test"}}}\n' "$request"
 printf '%s\n' '{"method":"turn/started","params":{"threadId":"saved-thread","turn":{"id":"turn-test"}}}' '{"method":"item/agentMessage/delta","params":{"threadId":"saved-thread","turnId":"turn-test","itemId":"message","delta":"observed before fault"}}' '{"id":"old-approval","method":"item/commandExecution/requestApproval","params":{"threadId":"saved-thread","turnId":"turn-test","itemId":"command","command":"touch unauthorized","availableDecisions":["accept","decline"]}}'
 if [ -f crash ]; then exit 17; fi;;
*'"decision":"accept"'*) touch unauthorized;;
*'"decision":"decline"'*)
 printf '%s\n' '{"method":"turn/completed","params":{"threadId":"saved-thread","turn":{"id":"turn-test","status":"completed"}}}'
 if [ -f evict-terminal ]; then
  count=0
  while [ "$count" -lt 2100 ]; do
   printf '%s\n' '{"method":"thread/tokenUsage/updated","params":{"threadId":"saved-thread","tokenUsage":{"total":1}}}'
   count=$((count + 1))
   if [ $((count % 8)) -eq 0 ]; then sleep 0.01; fi
  done
 fi;;
esac
done
"##.replace("CONFIG_REPLY", crate::test_support::CODEX_CONFIG_REPLY)).await;
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root.join("code")).await;
        Self { root, manager, task, id: uuid::Uuid::new_v4().to_string() }
    }
    fn options(&self) -> SessionOptions {
        SessionOptions { workspace: self.root.join("code").to_string_lossy().into_owned(), resume_id: None, executable: Some(self.root.join("codex").to_string_lossy().into_owned()), model: None, trust_workspace: true }
    }
    async fn open(&self) {
        self.manager.open_session("alice", Engine::Codex, self.id.clone(), crate::test_support::create(&self.manager, "alice", &self.task, self.options()).await, Some(self.task.clone()), CancellationToken::new()).await.unwrap();
    }
    async fn db(&self) -> dgos_db::sea_orm::DatabaseConnection {
        dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", self.root.join("metadata.sqlite").display())).await.unwrap()
    }
    async fn waiting(&self) -> crate::PendingRequest {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Some(pending) = self.manager.snapshot("alice", &self.id).await.unwrap().view.pending.into_iter().next() { return pending; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap()
    }
    async fn terminal(&self, request: &str) -> crate::RunRecord {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(run) = self.manager.list_development_runs("alice", &self.task).await.unwrap().into_iter().find(|r| r.id == request && !matches!(r.status.as_str(), "running" | "waiting")) { return run; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap()
    }
    async fn persistence_failed(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self.manager.snapshot("alice", &self.id).await.unwrap().view.persistence_error.as_deref() == Some("storage_failed") { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap();
    }
    fn pid(&self) -> i32 { std::fs::read_to_string(self.root.join("code/pid")).unwrap().parse().unwrap() }
}
impl Drop for Fixture {
    fn drop(&mut self) { self.manager.close(); let _ = std::fs::remove_dir_all(&self.root); }
}

#[tokio::test]
async fn native_crash_and_missing_history_preserve_task_code_receipt_and_expire_approval() {
    let f = Fixture::new().await;
    std::fs::write(f.root.join("code/crash"), "").unwrap();
    f.open().await;
    let generation = f.manager.snapshot("alice", &f.id).await.unwrap().generation;
    let request = uuid::Uuid::new_v4().to_string();
    let _ = f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await;
    let run = f.terminal(&request).await;
    assert_eq!(run.status, "unknown");
    assert!(run.finished_at.is_some());
    assert!(run.events.iter().any(|event| event.text.as_deref() == Some("observed before fault")));
    let view = f.manager.snapshot("alice", &f.id).await.unwrap().view;
    assert!(view.pending.is_empty());
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let binding = f.manager.inner.repository.read_cli_session_configuration("alice", &f.id).await.unwrap().binding.unwrap();
            if binding.state == "closed" {
                assert_eq!(binding.cleanup_proof.as_deref(), Some("owned_exit"));
                break;
            }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    // Retirement may release the backend after the durable unknown run was
    // recorded. Neither retained nor retired state can revive an old approval.
    assert!(f.manager.respond("alice", &f.id, &generation, "\"old-approval\"".into(), json!({"decision":"accept"})).await.is_err());
    assert_eq!(f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap_err(), "result_unknown");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(!f.root.join("code/unauthorized").exists());
    assert!(f.manager.shutdown().await);
    std::fs::write(f.root.join("code/missing-history"), "").unwrap();
    let restarted = crate::test_support::manager(&f.root).await;
    assert!(matches!(restarted.resume_session("alice", &f.id, true, CancellationToken::new()).await, Err(error) if error == "cli_handshake_failed"));
    assert_ne!(unsafe { libc::kill(f.pid(), 0) }, 0, "failed resume must reap its owned child");
    assert_eq!(std::fs::read_to_string(f.root.join("code/opens")).unwrap(), "start\nresume\n");
    assert_eq!(restarted.list("alice").await[0].native_id.as_deref(), Some("saved-thread"));
    assert_eq!(restarted.inner.repository.task("alice", &f.task).await.unwrap().status, "blocked");
    assert_eq!(restarted.list_development_runs("alice", &f.task).await.unwrap()[0].status, "unknown");
    assert_eq!(restarted.send("alice", &f.id, &generation, request, "make one change".into(), None, vec![], vec![], vec![]).await.unwrap_err(), "result_unknown");
    assert_eq!(std::fs::read_to_string(f.root.join("code/source.txt")).unwrap(), "edited by native fixture\n");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(restarted.shutdown().await);
}

#[tokio::test]
async fn sqlite_run_admission_failure_sends_nothing_and_keeps_task_and_code() {
    let f = Fixture::new().await;
    f.open().await;
    let generation = f.manager.snapshot("alice", &f.id).await.unwrap().generation;
    let db = f.db().await;
    db.execute_unprepared("CREATE TRIGGER deny_run BEFORE INSERT ON development_runs BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    assert_eq!(f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap_err(), "storage_failed");
    assert!(f.manager.inner.repository.run("alice", &request).await.unwrap().is_none());
    assert_eq!(f.manager.inner.repository.task("alice", &f.task).await.unwrap().status, "todo");
    assert!(!f.root.join("code/invocations").exists());
    assert_eq!(std::fs::read_to_string(f.root.join("code/source.txt")).unwrap(), "user code\n");
    db.execute_unprepared("DROP TRIGGER deny_run").await.unwrap();
    f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap();
    let pending = f.waiting().await;
    f.manager.respond("alice", &f.id, &generation, pending.id, json!({"decision":"decline"})).await.unwrap();
    assert_eq!(f.terminal(&request).await.status, "completed");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn sqlite_terminal_write_failure_reconciles_exact_terminal_without_restart_or_replay() {
    let f = Fixture::new().await;
    f.open().await;
    let generation = f.manager.snapshot("alice", &f.id).await.unwrap().generation;
    let db = f.db().await;
    db.execute_unprepared("CREATE TRIGGER deny_terminal BEFORE UPDATE ON development_runs WHEN NEW.status NOT IN ('running','waiting') BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap();
    let pending = f.waiting().await;
    // Let the host checkpoint the observed output before failing the terminal transaction.
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let run = f.manager.inner.repository.run("alice", &request).await.unwrap().unwrap();
            if run.status == "waiting" && run.events.contains("observed before fault") { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    f.manager.respond("alice", &f.id, &generation, pending.id.clone(), json!({"decision":"decline"})).await.unwrap();
    f.persistence_failed().await;
    let projections = f.manager.inner.run_projections.lock().await;
    let projection = projections.get(&(f.id.clone(), generation.clone())).unwrap();
    assert_eq!(projection.run_id, request);
    assert!(projection.persistence_failed);
    drop(projections);
    assert_eq!(f.manager.list_development_runs("alice", &f.task).await.unwrap().len(), 1);
    assert_eq!(f.manager.inner.repository.run("alice", &request).await.unwrap().unwrap().status, "waiting");
    assert_eq!(f.manager.inner.repository.task("alice", &f.task).await.unwrap().status, "in_progress");
    assert_eq!(f.manager.send("alice", &f.id, &generation, uuid::Uuid::new_v4().to_string(), "another turn".into(), None, vec![], vec![], vec![]).await.unwrap_err(), "storage_failed");
    db.execute_unprepared("DROP TRIGGER deny_terminal").await.unwrap();
    let run = f.terminal(&request).await;
    assert_eq!(run.status, "completed");
    assert!(run.finished_at.is_some());
    assert!(run.events.iter().any(|event| event.text.as_deref() == Some("observed before fault")));
    assert_eq!(f.manager.inner.repository.task("alice", &f.task).await.unwrap().status, "review");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if f.manager.snapshot("alice", &f.id).await.unwrap().view.persistence_error.is_none() { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap();
    assert!(f.manager.snapshot("alice", &f.id).await.unwrap().view.pending.is_empty());
    assert_eq!(f.manager.respond("alice", &f.id, &generation, pending.id, json!({"decision":"accept"})).await.unwrap_err(), "request_expired");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(!f.root.join("code/unauthorized").exists());
    assert_eq!(std::fs::read_to_string(f.root.join("code/source.txt")).unwrap(), "edited by native fixture\n");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn caller_abort_after_dispatch_still_installs_turn_and_finishes_once() {
    let f = Fixture::new().await;
    std::fs::write(f.root.join("code/delay-ack"), "").unwrap();
    f.open().await;
    let generation = f.manager.snapshot("alice", &f.id).await.unwrap().generation;
    let db = f.db().await;
    db.execute_unprepared("CREATE TABLE terminal_writes (run_id TEXT NOT NULL)").await.unwrap();
    db.execute_unprepared("CREATE TRIGGER count_terminal AFTER UPDATE ON development_runs WHEN NEW.status NOT IN ('running','waiting') BEGIN INSERT INTO terminal_writes VALUES (NEW.id); END").await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    let manager = f.manager.clone();
    let id = f.id.clone();
    let run = request.clone();
    let current = generation.clone();
    let caller = tokio::spawn(async move { manager.send("alice", &id, &current, run, "make one change".into(), None, vec![], vec![], vec![]).await });
    tokio::time::timeout(Duration::from_secs(4), async {
        while !f.root.join("code/dispatched").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let control = f.manager.session_control(&f.id).await.unwrap();
    assert!(control.try_lock().is_err(), "dropping the caller released native acknowledgement ownership");
    std::fs::write(f.root.join("code/release-ack"), "").unwrap();
    let pending = f.waiting().await;
    assert_eq!(f.manager.snapshot("alice", &f.id).await.unwrap().view.active_run.as_deref(), Some(request.as_str()));
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if control.try_lock().is_ok() { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    f.manager.respond("alice", &f.id, &generation, pending.id, json!({"decision":"decline"})).await.unwrap();
    assert_eq!(f.terminal(&request).await.status, "completed");
    f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap();
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    let row = db.query_one(dgos_db::sea_orm::Statement::from_string(dgos_db::sea_orm::DatabaseBackend::Sqlite, "SELECT COUNT(*) AS count FROM terminal_writes")).await.unwrap().unwrap();
    assert_eq!(row.try_get::<i64>("", "count").unwrap(), 1);
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn sqlite_checkpoint_failure_keeps_observing_and_recovers_without_restart() {
    let f = Fixture::new().await;
    f.open().await;
    let generation = f.manager.snapshot("alice", &f.id).await.unwrap().generation;
    let db = f.db().await;
    db.execute_unprepared("CREATE TRIGGER deny_checkpoint BEFORE UPDATE ON development_runs WHEN NEW.status IN ('running','waiting') BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap();
    let pending = f.waiting().await;
    f.persistence_failed().await;
    assert_eq!(f.manager.inner.repository.run("alice", &request).await.unwrap().unwrap().events, "[]");
    assert_eq!(f.manager.send("alice", &f.id, &generation, uuid::Uuid::new_v4().to_string(), "next".into(), None, vec![], vec![], vec![]).await.unwrap_err(), "storage_failed");
    db.execute_unprepared("DROP TRIGGER deny_checkpoint").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let run = f.manager.inner.repository.run("alice", &request).await.unwrap().unwrap();
            let view = f.manager.snapshot("alice", &f.id).await.unwrap().view;
            if run.status == "waiting" && run.events.contains("observed before fault") && view.persistence_error.is_none() { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    f.manager.respond("alice", &f.id, &generation, pending.id, json!({"decision":"decline"})).await.unwrap();
    assert_eq!(f.terminal(&request).await.status, "completed");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn shutdown_retains_failed_terminal_supervision_until_reconciliation() {
    let f = Fixture::new().await;
    f.open().await;
    let generation = f.manager.snapshot("alice", &f.id).await.unwrap().generation;
    let db = f.db().await;
    db.execute_unprepared("CREATE TRIGGER deny_terminal BEFORE UPDATE ON development_runs WHEN NEW.status NOT IN ('running','waiting') BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap();
    let pending = f.waiting().await;
    f.manager.respond("alice", &f.id, &generation, pending.id, json!({"decision":"decline"})).await.unwrap();
    f.persistence_failed().await;
    assert!(!f.manager.shutdown().await, "unsettled terminal storage cannot report successful shutdown");
    assert_eq!(f.manager.inner.watchers.lock().await.len(), 1);
    assert!(!f.manager.inner.watchers.lock().await[0].is_finished());
    assert_eq!(f.manager.inner.run_projections.lock().await.len(), 1);
    assert_eq!(f.manager.slot_count().await, 1, "reaping alone cannot retire an undurable run");
    assert!(f.manager.inner.gate.begin_maintenance().is_err());
    db.execute_unprepared("DROP TRIGGER deny_terminal").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let run = f.manager.inner.repository.run("alice", &request).await.unwrap().unwrap();
            if run.status == "completed" && f.manager.inner.run_projections.lock().await.is_empty() && f.manager.slot_count().await == 0 { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert!(f.manager.inner.gate.begin_maintenance().is_ok());
    assert!(f.manager.shutdown().await);
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
}

#[tokio::test]
async fn terminal_evicted_before_watcher_snapshot_still_finishes_durably() {
    let f = Fixture::new().await;
    std::fs::write(f.root.join("code/evict-terminal"), "").unwrap();
    f.open().await;
    let generation = f.manager.snapshot("alice", &f.id).await.unwrap().generation;
    let (_, backend) = f.manager.owned("alice", &f.id).await.unwrap();
    let db = f.db().await;
    db.execute_unprepared("CREATE TRIGGER deny_checkpoint BEFORE UPDATE ON development_runs WHEN NEW.status IN ('running','waiting') BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    f.manager.send("alice", &f.id, &generation, request.clone(), "make one change".into(), None, vec![], vec![], vec![]).await.unwrap();
    let pending = f.waiting().await;
    f.persistence_failed().await;
    // Pause the watcher while it clears a recovered checkpoint failure, before
    // its next snapshot. Native event consumption remains independent.
    let projections = f.manager.inner.run_projections.lock().await;
    db.execute_unprepared("DROP TRIGGER deny_checkpoint").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if f.manager.inner.repository.run("alice", &request).await.unwrap().unwrap().status == "waiting" { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    f.manager.respond("alice", &f.id, &generation, pending.id, json!({"decision":"decline"})).await.unwrap();
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let view = backend.snapshot().await;
            if view.history_truncated && view.events.len() == 2000 && view.events.iter().all(|event| event.kind == "thread/tokenUsage/updated") {
                let terminal = view.terminal_run.as_ref().unwrap();
                assert_eq!(terminal.run_id, request);
                assert_eq!(terminal.status, "completed");
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    drop(projections);
    let run = f.terminal(&request).await;
    assert_eq!(run.status, "completed");
    assert!(run.finished_at.is_some());
    assert!(run.truncated);
    assert!(run.events.iter().all(|event| event.data.get("run_status").and_then(serde_json::Value::as_str) != Some("completed")));
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn another_owner_cannot_replace_a_recovered_slot_with_a_failed_start() {
    let f = Fixture::new().await;
    f.open().await;
    let pid = f.pid();
    let recovered = crate::test_support::manager(&f.root).await;
    let unresolved_slot = *recovered.inner.slots.lock().await.get(&f.id).unwrap();
    let binding = recovered.inner.repository.read_cli_session_configuration("alice", &f.id).await.unwrap().binding.unwrap();
    assert_eq!(binding.state, "unknown");
    assert_eq!(binding.cleanup_state, "unknown");
    assert!(binding.cleanup_proof.is_none());
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the original host still owns a live child");

    let directory = f.root.join("bob-code");
    std::fs::create_dir(&directory).unwrap();
    let project = recovered.create_project("bob", crate::projects::CreateProjectInput { kind: "import".into(), name: "Bob's project".into(), path: directory.to_string_lossy().into_owned(), source: None }, CancellationToken::new()).await.unwrap();
    let task = recovered.save_development_task("bob", crate::projects::SaveTaskInput { id: None, project_id: project.id, goal: "Start a different task".into(), requirements: "Do not replace another account's reservation".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let mut options = f.options();
    options.workspace = directory.to_string_lossy().into_owned();
    let mut create = crate::test_support::create(&recovered, "bob", &task.id, options).await;
    // This would fail before creating Bob's durable binding. It must not first
    // replace Alice's global slot and then release that slot as "not found".
    create.project_revision = 0;
    let error = recovered.open_session("bob", Engine::Codex, f.id.clone(), create, Some(task.id), CancellationToken::new()).await.err().unwrap();
    assert_eq!(error, "session_exists");
    assert_eq!(recovered.inner.slots.lock().await.get(&f.id), Some(&unresolved_slot));
    assert_eq!(recovered.slot_count().await, 1);
    assert!(recovered.list("bob").await.is_empty());
    assert_eq!(recovered.snapshot("bob", &f.id).await.err().unwrap(), "session_not_found");
    assert!(recovered.inner.repository.agent_session("bob", &f.id).await.unwrap().is_none());
    assert!(!directory.join("pid").exists(), "the conflicting request must not launch a child");
    let unchanged = recovered.inner.repository.read_cli_session_configuration("alice", &f.id).await.unwrap().binding.unwrap();
    assert_eq!(unchanged.binding_id, binding.binding_id);
    assert_eq!(unchanged.host_instance_id, binding.host_instance_id);
    assert_eq!(unchanged.cleanup_state, "unknown");
    assert!(unchanged.cleanup_proof.is_none());
    assert!(!recovered.shutdown().await, "another account's failed start cannot prove the old child stopped");
    assert_eq!(recovered.inner.slots.lock().await.get(&f.id), Some(&unresolved_slot));
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the recovered host must not signal another host's child");
    assert!(f.manager.shutdown().await);
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    assert!(recovered.shutdown().await, "the original host's durable exit proof releases the reservation");
    assert_eq!(recovered.slot_count().await, 0);
}
