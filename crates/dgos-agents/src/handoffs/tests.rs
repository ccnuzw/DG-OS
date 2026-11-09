use super::*;
use crate::projects::{CreateProjectInput, SaveTaskInput};
use dgos_db::development_repository::{CliProviderConfiguration, CliProviderSnapshot, OpeningBinding, StoredAgentSession, StoredRun};

async fn fixture(bound_source: bool) -> (std::path::PathBuf, AgentManager, String, String, String) {
    let root = std::env::temp_dir().join(format!("dgos-handoff-recovery-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("code")).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("code/source.txt"), "user code\n").unwrap();
    let manager = crate::test_support::manager(&root).await;
    let project = manager.create_project("alice", CreateProjectInput { kind: "import".into(), name: "Handoff".into(), path: root.join("code").to_string_lossy().into_owned(), source: None }, CancellationToken::new()).await.unwrap();
    let task = manager.save_development_task("alice", SaveTaskInput { id: None, project_id: project.id.clone(), goal: "Continue latest work".into(), requirements: "Retain task and code".into(), status: "todo".into() }, CancellationToken::new()).await.unwrap();
    let session = StoredAgentSession { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), engine: "claude".into(), native_id: Some("source-native-history".into()), workspace: project.path, executable: None, model: bound_source.then(|| "fixture-model".into()), project_id: Some(project.id.clone()), task_id: Some(task.id.clone()) };
    if bound_source {
        // Seed durable protocol history through the same binding/snapshot chain
        // as native sessions, without launching a CLI or making a model request.
        let configuration = CliProviderConfiguration { engine: session.engine.clone(), provider_id: "fixture-alice".into(), model_id: "fixture-model".into(), provider_revision: 1 };
        let revision = manager.inner.repository.save_cli_provider_revision("alice", &project.id, 0, &configuration).await.unwrap();
        let binding = OpeningBinding { binding_id: uuid::Uuid::new_v4().to_string(), project_id: project.id.clone(), task_id: task.id.clone(), revision: revision.revision, permission_epoch: revision.permission_epoch, provider_id: configuration.provider_id.clone(), model_id: configuration.model_id.clone(), provider_revision: configuration.provider_revision, host_instance_id: "history-fixture".into(), boot_identity: None, request_digest: "source-history-fixture".into() };
        let snapshot = CliProviderSnapshot { binding_id: binding.binding_id.clone(), engine: configuration.engine, provider_id: configuration.provider_id, model_id: configuration.model_id, provider_revision: configuration.provider_revision, revision: revision.revision, permission_epoch: revision.permission_epoch, evidence: Default::default() };
        manager.inner.repository.reserve_cli_session("alice", &session, &binding, &snapshot).await.unwrap();
        manager.inner.repository.activate_cli_session("alice", &session.id, &binding.binding_id, &binding.host_instance_id, session.native_id.as_deref().unwrap(), binding.permission_epoch).await.unwrap();
    } else {
        manager.inner.repository.save_session(&session).await.unwrap();
    }
    manager.reload_records().await.unwrap();
    (root, manager, project.id, task.id, session.id)
}

async fn prepare(manager: &AgentManager, task: &str, source: &str) -> StoredHandoff {
    manager.prepare_development_handoff("alice", PrepareHandoffInput { task_id: task.into(), source_session_id: source.into(), target_engine: Engine::Codex, remaining: "Review current work before continuing".into() }, CancellationToken::new()).await.unwrap()
}

fn accept(root: &std::path::Path, handoff: &StoredHandoff) -> AcceptHandoffInput {
    AcceptHandoffInput { handoff_id: handoff.id.clone(), summary: handoff.summary.clone(), confirmed: true, project_revision: 1, provider: crate::capabilities::CliProviderRef { provider_id: "fixture-alice".into(), model_id: "fixture-model".into(), provider_revision: 1 }, options: SessionOptions { workspace: root.join("code").to_string_lossy().into_owned(), resume_id: None, executable: Some(root.join("missing-cli").to_string_lossy().into_owned()), model: None, trust_workspace: true } }
}

#[tokio::test]
async fn handoff_links_latest_source_run_and_keeps_recent_context_after_restart() {
    let (root, manager, project, task, source) = fixture(true).await;
    let snapshot = manager.inner.repository.read_cli_session_configuration("alice", &source).await.unwrap().snapshot.unwrap();
    let snapshot: CliProviderSnapshot = serde_json::from_str(&snapshot).unwrap();
    let mut latest = String::new();
    for round in 0..10 {
        let mut run = StoredRun { id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), session_id: Some(source.clone()), task_id: task.clone(), project_id: project.clone(), engine: "claude".into(), source: "native_agent".into(), runtime: None, status: "running".into(), started_at: round + 1, finished_at: None, summary: format!("round-{round}-observation"), reason: None, events: "[]".into(), fingerprint: "fixture".into(), start_fingerprint: "fixture".into(), truncated: false, request_fingerprint: format!("round-{round}") };
        manager.inner.repository.begin_cli_run(&run, &snapshot).await.unwrap();
        run.status = "completed".into();
        run.finished_at = Some(round + 2);
        assert!(manager.inner.repository.finish_run("alice", &run, Some("review")).await.unwrap());
        latest = run.id;
    }
    let binding = manager.inner.repository.read_cli_session_configuration("alice", &source).await.unwrap().binding.unwrap();
    manager.inner.repository.record_cli_cleanup("alice", &source, &binding.binding_id, binding.host_instance_id.as_deref().unwrap(), &dgos_db::development_repository::CleanupObservation { binding_id: binding.binding_id.clone(), state: "closed".into(), proof: "no_execution".into(), reason: None }).await.unwrap();
    let handoff = prepare(&manager, &task, &source).await;
    assert_eq!(handoff.source_run_id.as_deref(), Some(latest.as_str()));
    assert!(!handoff.summary.contains(&source));
    assert!(!handoff.summary.contains(&latest));
    assert!(!handoff.summary.contains("source-native-history"));
    assert!(handoff.summary.contains("round-9-observation"));
    assert!(handoff.summary.contains("round-2-observation"));
    assert!(!handoff.summary.contains("round-0-observation"));
    assert!(!handoff.summary.contains("round-1-observation"));
    assert!(handoff.summary.find("round-9-observation").unwrap() < handoff.summary.find("round-8-observation").unwrap());
    let restarted = crate::test_support::manager(&root).await;
    let stored = restarted.development_handoff("alice", &handoff.id).await.unwrap();
    assert_eq!(stored.source_run_id, handoff.source_run_id);
    assert_eq!(stored.summary, handoff.summary);
    let run_snapshot = restarted.inner.repository.run_capabilities("alice", &latest).await.unwrap().unwrap();
    assert_eq!(serde_json::from_str::<CliProviderSnapshot>(&run_snapshot).unwrap(), snapshot);
    assert_eq!(restarted.list("alice").await[0].native_id.as_deref(), Some("source-native-history"));
    assert_eq!(std::fs::read_to_string(root.join("code/source.txt")).unwrap(), "user code\n");
    assert!(manager.shutdown().await);
    assert!(restarted.shutdown().await);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn revoked_handoff_and_failed_reserved_launch_never_gain_fresh_authority_on_restart() {
    let (root, manager, project, task, source) = fixture(false).await;
    let revoked = prepare(&manager, &task, &source).await;
    manager.inner.providers.revoke("alice", &project).await.unwrap();
    assert!(matches!(manager.accept_development_handoff("alice", accept(&root, &revoked), CancellationToken::new()).await, Err(error) if error == "capability_revoked"));
    assert!(manager.development_handoff("alice", &revoked.id).await.unwrap().target_session_id.is_none());
    let current = prepare(&manager, &task, &source).await;
    crate::test_support::create(&manager, "alice", &task, accept(&root, &current).options).await;
    assert!(manager.accept_development_handoff("alice", accept(&root, &current), CancellationToken::new()).await.is_err());
    let reserved = manager.development_handoff("alice", &current.id).await.unwrap();
    let target = reserved.target_session_id.as_ref().expect("reservation survives launch failure");
    assert!(manager.list("alice").await.iter().any(|s| &s.id == target), "failed opening retains durable session evidence");
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
