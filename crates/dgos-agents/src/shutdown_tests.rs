#![cfg(unix)]
use crate::{Engine, SessionOptions};
use std::{os::unix::fs::PermissionsExt, path::{Path, PathBuf}, time::Duration};
use tokio_util::sync::CancellationToken;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dgos-shutdown-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("codex");
        std::fs::write(&executable, r##"#!/bin/sh
printf '%s' "$$" > pid
while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*)
 if [ -f reject-open ]; then printf '%s\n' '{"id":1,"error":{"code":-1,"message":"credential-SENTINEL"}}'; continue; fi
 if [ -f block-open ]; then touch entered; while :; do sleep 1; done; fi
 printf '%s\n' '{"id":1,"result":{}}';;
*'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '%s\n' '{"id":2,"result":{"thread":{"id":"saved-thread","turns":[]}}}';;
*'"method":"turn/start"'*) touch entered; while :; do sleep 1; done;;
esac
done
"##).unwrap();
        std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o700)).unwrap();
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
    let fixture = Fixture::new();
    let manager = crate::test_support::manager(&fixture.0).await;
    let task_id = crate::test_support::task(&manager, &fixture.0).await;
    let id = uuid::Uuid::new_v4().to_string();
    if resume {
        manager.open("alice", Engine::Codex, id.clone(), fixture.options(), Some(task_id.clone()), CancellationToken::new()).await.unwrap();
        assert!(manager.close_session("alice", &id).await.unwrap());
    }
    fixture.block();
    let worker = manager.clone(); let options = fixture.options(); let session_id = id.clone();
    let task_id_for_open = task_id.clone();
    let opening = tokio::spawn(async move { worker.open("alice", Engine::Codex, session_id, options, Some(task_id_for_open), CancellationToken::new()).await });
    let pid = entered(&fixture.0).await;
    let closed = manager.shutdown().await;
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    // Cleanup on assertion failure must not leave a child from the regression itself.
    if alive { unsafe { libc::kill(-pid, libc::SIGKILL); } }
    assert!(closed);
    assert!(!alive, "shutdown returned while handshake child was alive");
    assert!(tokio::time::timeout(Duration::from_secs(2), opening).await.unwrap().unwrap().is_err());
    assert!(manager.snapshot("alice", &id).await.is_err());
    assert_eq!(manager.list("alice").await.len(), usize::from(resume));
    assert!(manager.open("alice", Engine::Codex, uuid::Uuid::new_v4().to_string(), fixture.options(), Some(task_id), CancellationToken::new()).await.is_err());
}
#[tokio::test]
async fn shutdown_reaps_an_unregistered_connecting_child() { opening_shutdown(false).await; }
#[tokio::test]
async fn shutdown_reaps_a_resuming_child_without_late_admission() { opening_shutdown(true).await; }
#[tokio::test]
async fn shutdown_rejects_an_inflight_send_and_reaps_its_process() {
    let fixture = Fixture::new(); let manager = crate::test_support::manager(&fixture.0).await;
    let development_task = crate::test_support::task(&manager, &fixture.0).await;
    let id = uuid::Uuid::new_v4().to_string();
    manager.open("alice", Engine::Codex, id.clone(), fixture.options(), Some(development_task), CancellationToken::new()).await.unwrap();
    let worker = manager.clone(); let task_id = id.clone();
    let send = tokio::spawn(async move { worker.send("alice", &task_id, uuid::Uuid::new_v4().to_string(), "task".into()).await });
    let pid = entered(&fixture.0).await;
    assert!(manager.shutdown().await);
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    assert!(tokio::time::timeout(Duration::from_secs(2), send).await.unwrap().unwrap().is_err());
    assert!(manager.snapshot("alice", &id).await.is_err());
}

#[tokio::test]
async fn codex_handshake_diagnostics_cover_success_rejection_and_lease_cancellation() {
    let (log, _guard) = crate::test_support::DiagnosticLog::capture();
    let fixture = Fixture::new();
    let session = crate::codex_session::CodexSession::open(fixture.options(), CancellationToken::new()).await.unwrap();
    assert!(session.close().await);
    std::fs::write(fixture.0.join("reject-open"), "").unwrap();
    let error = crate::codex_session::CodexSession::open(fixture.options(), CancellationToken::new()).await.err().unwrap();
    assert_eq!(error, "engine_rejected");
    let rejected_pid: i32 = std::fs::read_to_string(fixture.0.join("pid")).unwrap().parse().unwrap();
    assert_ne!(unsafe { libc::kill(rejected_pid, 0) }, 0, "failed handshake returned before reaping");
    std::fs::remove_file(fixture.0.join("reject-open")).unwrap();
    fixture.block();
    let lease = CancellationToken::new();
    let options = fixture.options();
    let opening_lease = lease.clone();
    let opening = tokio::spawn(async move { crate::codex_session::CodexSession::open(options, opening_lease).await });
    let pid = entered(&fixture.0).await;
    lease.cancel();
    let error = tokio::time::timeout(Duration::from_secs(4), opening).await.unwrap().unwrap().err().unwrap();
    assert!(matches!(error.as_str(), "connection_lost" | "closed"));
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "cancelled handshake returned before reaping");
    let text = log.text();
    assert!(text.contains("stage=\"handshake_initialize\"") && text.contains("stage=\"handshake_thread_start\""), "{text}");
    assert!(text.contains("error_class=\"engine_rejected\""), "{text}");
    assert!(text.contains("outcome=\"cancelled\"") && text.contains("stage=\"cleanup\"") && text.contains("stage=\"exit_wait\""), "{text}");
    assert!(!text.contains("credential-SENTINEL") && !text.contains(&fixture.0.to_string_lossy().to_string()), "{text}");
    eprint!("{text}");
}
