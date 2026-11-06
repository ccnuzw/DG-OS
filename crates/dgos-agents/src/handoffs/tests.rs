use super::*;
use crate::projects::{CreateProjectInput, SaveTaskInput};
use dgos_db::development_repository::{StoredAgentSession, StoredRun};

async fn fixture() -> (std::path::PathBuf, AgentManager, String, String, String) {
    let root = std::env::temp_dir().join(format!("dgos-handoff-recovery-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("code")).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("code/source.txt"), "user code\n").unwrap();
    let manager = crate::test_support::manager(&root).await;
    let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "Handoff".into(), path: root.join("code").to_string_lossy().into_owned(), source: None }, CancellationToken::new()).await.unwrap();
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Continue latest work".into(), requirements: "Retain task and code".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let session = StoredAgentSession { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), engine: "claude".into(), native_id: Some("source-native-history".into()), workspace: project.path, executable: None, model: None, project_id: Some(project.id.clone()), task_id: Some(task.id.clone()) };
    manager.inner.repository.save_session(&session).await.unwrap();
    manager.reload_records().await.unwrap();
    (root, manager, project.id, task.id, session.id)
}

async fn prepare(manager: &AgentManager, task: &str, source: &str) -> StoredHandoff {
    manager.prepare_development_handoff("alice", PrepareHandoffInput { task_id: task.into(), source_session_id: source.into(), target_engine: Engine::Codex, remaining: "Review current work before continuing".into() }, CancellationToken::new()).await.unwrap()
}

fn accept(root: &std::path::Path, handoff: &StoredHandoff) -> AcceptHandoffInput {
    AcceptHandoffInput { handoff_id: handoff.id.clone(), summary: handoff.summary.clone(), confirmed: true, capability_revision: 0, options: SessionOptions { workspace: root.join("code").to_string_lossy().into_owned(), resume_id: None, executable: Some(root.join("missing-cli").to_string_lossy().into_owned()), model: None, trust_workspace: true } }
}

#[tokio::test]
async fn handoff_links_latest_source_run_and_keeps_recent_context_after_restart() {
    let (root, manager, project, task, source) = fixture().await;
    let mut latest = String::new();
    for round in 0..10 {
        let mut run = StoredRun { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), session_id: Some(source.clone()), task_id: task.clone(), project_id: project.clone(), engine: "claude".into(), source: "native_agent".into(), runtime: None, status: "running".into(), started_at: round + 1, finished_at: None, summary: format!("round-{round}-observation"), reason: None, events: "[]".into(), fingerprint: "fixture".into(), start_fingerprint: "fixture".into(), truncated: false, request_fingerprint: format!("round-{round}") };
        manager.inner.repository.start_run(&run).await.unwrap();
        run.status = "completed".into();
        run.finished_at = Some(round + 2);
        assert!(manager.inner.repository.finish_run("alice", &run, Some("review")).await.unwrap());
        latest = run.id;
    }
    let handoff = prepare(&manager, &task, &source).await;
    assert_eq!(handoff.source_run_id.as_deref(), Some(latest.as_str()));
    assert!(handoff.summary.contains("round-9-observation"));
    assert!(handoff.summary.contains("round-2-observation"));
    assert!(!handoff.summary.contains("round-0-observation"));
    assert!(!handoff.summary.contains("round-1-observation"));
    assert!(handoff.summary.find("round-9-observation").unwrap() < handoff.summary.find("round-8-observation").unwrap());
    let restarted = crate::test_support::manager(&root).await;
    let stored = restarted.development_handoff("alice", &handoff.id).await.unwrap();
    assert_eq!(stored.source_run_id, handoff.source_run_id);
    assert_eq!(stored.summary, handoff.summary);
    assert_eq!(std::fs::read_to_string(root.join("code/source.txt")).unwrap(), "user code\n");
    assert!(manager.shutdown().await);
    assert!(restarted.shutdown().await);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn revoked_handoff_and_failed_reserved_launch_never_gain_fresh_authority_on_restart() {
    let (root, manager, project, task, source) = fixture().await;
    let revoked = prepare(&manager, &task, &source).await;
    assert_eq!(manager.inner.repository.revoke_project_capabilities("alice", &project).await.unwrap(), 1);
    assert!(matches!(manager.accept_development_handoff("alice", accept(&root, &revoked), CancellationToken::new()).await, Err(error) if error == "capability_revoked"));
    assert!(manager.development_handoff("alice", &revoked.id).await.unwrap().target_session_id.is_none());
    let current = prepare(&manager, &task, &source).await;
    assert!(manager.accept_development_handoff("alice", accept(&root, &current), CancellationToken::new()).await.is_err());
    let reserved = manager.development_handoff("alice", &current.id).await.unwrap();
    let target = reserved.target_session_id.as_ref().expect("reservation survives launch failure");
    assert!(manager.list("alice").await.iter().all(|s| &s.id != target));
    let restarted = crate::test_support::manager(&root).await;
    assert!(matches!(restarted.accept_development_handoff("alice", accept(&root, &revoked), CancellationToken::new()).await, Err(error) if error == "capability_revoked"));
    assert!(matches!(restarted.accept_development_handoff("alice", accept(&root, &current), CancellationToken::new()).await, Err(error) if error == "handoff_already_accepted"));
    assert_eq!(restarted.development_handoff("alice", &current.id).await.unwrap().target_session_id.as_ref(), Some(target));
    assert_eq!(restarted.inner.repository.task("alice", &task).await.unwrap().goal, "Continue latest work");
    assert_eq!(std::fs::read_to_string(root.join("code/source.txt")).unwrap(), "user code\n");
    assert!(manager.shutdown().await);
    assert!(restarted.shutdown().await);
    std::fs::remove_dir_all(root).unwrap();
}
