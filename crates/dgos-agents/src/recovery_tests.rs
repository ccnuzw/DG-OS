//! Deterministic native-protocol/SQLite faults, not installed CLI compatibility evidence.
#![cfg(unix)]
use crate::{AgentManager, Engine, SessionOptions};
use dgos_db::sea_orm::ConnectionTrait;
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

struct Fixture { root: PathBuf, manager: AgentManager, task: String, id: String }
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("code")).unwrap();
        let root = root.canonicalize().unwrap();
        std::fs::write(root.join("code/source.txt"), "user code\n").unwrap();
        let executable = root.join("codex");
        std::fs::write(&executable, r##"#!/bin/sh
printf '%s' "$$" > pid
while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*) printf 'start\n' >> opens; printf '%s\n' '{"id":2,"result":{"thread":{"id":"saved-thread","turns":[]}}}';;
*'"method":"thread/resume"'*)
 printf 'resume\n' >> opens
 if [ -f missing-history ]; then printf '%s\n' '{"id":2,"error":{"code":-1,"message":"native history missing"}}'
 else printf '%s\n' '{"id":2,"result":{"thread":{"id":"saved-thread","turns":[]}}}'; fi;;
*'"method":"turn/start"'*)
 printf 'once\n' >> invocations
 printf 'edited by native fixture\n' > source.txt
 printf '%s\n' '{"id":3,"result":{"turn":{"id":"turn-test"}}}' '{"method":"turn/started","params":{"turn":{"id":"turn-test"}}}' '{"method":"item/agentMessage/delta","params":{"delta":"observed before fault"}}' '{"id":"old-approval","method":"item/commandExecution/requestApproval","params":{"command":"touch unauthorized","availableDecisions":["accept","decline"]}}'
 if [ -f crash ]; then exit 17; fi;;
*'"decision":"accept"'*) touch unauthorized;;
*'"decision":"decline"'*) printf '%s\n' '{"method":"turn/completed","params":{"turn":{"id":"turn-test","status":"completed"}}}';;
esac
done
"##).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let manager = crate::test_support::manager(&root).await;
        let task = crate::test_support::task(&manager, &root.join("code")).await;
        Self { root, manager, task, id: uuid::Uuid::new_v4().to_string() }
    }
    fn options(&self) -> SessionOptions {
        SessionOptions { workspace: self.root.join("code").to_string_lossy().into_owned(), resume_id: None, executable: Some(self.root.join("codex").to_string_lossy().into_owned()), model: None, trust_workspace: true }
    }
    async fn open(&self) {
        self.manager.open("alice", Engine::Codex, self.id.clone(), self.options(), Some(self.task.clone()), CancellationToken::new()).await.unwrap();
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
    let request = uuid::Uuid::new_v4().to_string();
    let _ = f.manager.send("alice", &f.id, request.clone(), "make one change".into()).await;
    let run = f.terminal(&request).await;
    assert_eq!(run.status, "unknown");
    assert!(run.finished_at.is_some());
    assert!(run.events.iter().any(|event| event.text.as_deref() == Some("observed before fault")));
    let view = f.manager.snapshot("alice", &f.id).await.unwrap().view;
    assert_eq!(view.status, "unknown");
    assert!(view.pending.is_empty());
    assert_eq!(f.manager.respond("alice", &f.id, "\"old-approval\"".into(), json!({"decision":"accept"})).await.unwrap_err(), "request_expired");
    assert_eq!(f.manager.send("alice", &f.id, request.clone(), "make one change".into()).await.unwrap_err(), "result_unknown");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(!f.root.join("code/unauthorized").exists());
    assert!(f.manager.shutdown().await);
    std::fs::write(f.root.join("code/missing-history"), "").unwrap();
    let restarted = crate::test_support::manager(&f.root).await;
    assert!(matches!(restarted.open("alice", Engine::Codex, f.id.clone(), f.options(), Some(f.task.clone()), CancellationToken::new()).await, Err(error) if error == "engine_rejected"));
    assert_ne!(unsafe { libc::kill(f.pid(), 0) }, 0, "failed resume must reap its owned child");
    assert_eq!(std::fs::read_to_string(f.root.join("code/opens")).unwrap(), "start\nresume\n");
    assert_eq!(restarted.list("alice").await[0].native_id.as_deref(), Some("saved-thread"));
    assert_eq!(restarted.inner.repository.task("alice", &f.task).await.unwrap().status, "blocked");
    assert_eq!(restarted.list_development_runs("alice", &f.task).await.unwrap()[0].status, "unknown");
    assert_eq!(restarted.send("alice", &f.id, request, "make one change".into()).await.unwrap_err(), "result_unknown");
    assert_eq!(std::fs::read_to_string(f.root.join("code/source.txt")).unwrap(), "edited by native fixture\n");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(restarted.shutdown().await);
}

#[tokio::test]
async fn sqlite_run_admission_failure_sends_nothing_and_keeps_task_and_code() {
    let f = Fixture::new().await;
    f.open().await;
    let db = f.db().await;
    db.execute_unprepared("CREATE TRIGGER deny_run BEFORE INSERT ON development_runs BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    assert_eq!(f.manager.send("alice", &f.id, request.clone(), "make one change".into()).await.unwrap_err(), "storage_failed");
    assert!(f.manager.inner.repository.run("alice", &request).await.unwrap().is_none());
    assert_eq!(f.manager.inner.repository.task("alice", &f.task).await.unwrap().status, "todo");
    assert!(!f.root.join("code/invocations").exists());
    assert_eq!(std::fs::read_to_string(f.root.join("code/source.txt")).unwrap(), "user code\n");
    db.execute_unprepared("DROP TRIGGER deny_run").await.unwrap();
    f.manager.send("alice", &f.id, request.clone(), "make one change".into()).await.unwrap();
    let pending = f.waiting().await;
    f.manager.respond("alice", &f.id, pending.id, json!({"decision":"decline"})).await.unwrap();
    assert_eq!(f.terminal(&request).await.status, "completed");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(f.manager.shutdown().await);
}

#[tokio::test]
async fn sqlite_terminal_write_failure_recovers_unknown_without_replay_or_old_approval() {
    let f = Fixture::new().await;
    f.open().await;
    let db = f.db().await;
    db.execute_unprepared("CREATE TRIGGER deny_terminal BEFORE UPDATE ON development_runs WHEN NEW.status NOT IN ('running','waiting') BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    f.manager.send("alice", &f.id, request.clone(), "make one change".into()).await.unwrap();
    let pending = f.waiting().await;
    // Let the host checkpoint the observed output before failing the terminal transaction.
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let run = f.manager.inner.repository.run("alice", &request).await.unwrap().unwrap();
            if run.status == "waiting" && run.events.contains("observed before fault") { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    f.manager.respond("alice", &f.id, pending.id.clone(), json!({"decision":"decline"})).await.unwrap();
    let watchers = std::mem::take(&mut *f.manager.inner.watchers.lock().await);
    assert_eq!(watchers.len(), 1);
    for watcher in watchers { assert!(!tokio::time::timeout(Duration::from_secs(4), watcher).await.unwrap().unwrap()); }
    assert_eq!(f.manager.inner.repository.run("alice", &request).await.unwrap().unwrap().status, "waiting");
    assert_eq!(f.manager.inner.repository.task("alice", &f.task).await.unwrap().status, "in_progress");
    assert!(f.manager.close_session("alice", &f.id).await.unwrap());
    db.execute_unprepared("DROP TRIGGER deny_terminal").await.unwrap();
    assert!(f.manager.shutdown().await);
    let restarted = crate::test_support::manager(&f.root).await;
    let run = restarted.list_development_runs("alice", &f.task).await.unwrap().remove(0);
    assert_eq!(run.status, "unknown");
    assert_eq!(run.reason.as_deref(), Some("interrupted_by_restart"));
    assert!(run.events.iter().any(|event| event.text.as_deref() == Some("observed before fault")));
    assert_eq!(restarted.inner.repository.task("alice", &f.task).await.unwrap().status, "blocked");
    assert_eq!(restarted.send("alice", &f.id, request.clone(), "make one change".into()).await.unwrap_err(), "result_unknown");
    restarted.open("alice", Engine::Codex, f.id.clone(), f.options(), Some(f.task.clone()), CancellationToken::new()).await.unwrap();
    assert!(restarted.snapshot("alice", &f.id).await.unwrap().view.pending.is_empty());
    assert_eq!(restarted.respond("alice", &f.id, pending.id, json!({"decision":"accept"})).await.unwrap_err(), "request_expired");
    assert_eq!(std::fs::read_to_string(f.root.join("code/invocations")).unwrap(), "once\n");
    assert!(!f.root.join("code/unauthorized").exists());
    assert_eq!(std::fs::read_to_string(f.root.join("code/source.txt")).unwrap(), "edited by native fixture\n");
    assert!(restarted.shutdown().await);
}
