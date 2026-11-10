use super::*;
use crate::migrations::{Migrator, MigratorTrait};
use crate::{UserDataDomain, UserDataRepository};

async fn database() -> (DatabaseConnection, DevelopmentRepository) {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    (db.clone(), DevelopmentRepository::new(db))
}

async fn session(repo: &DevelopmentRepository, owner: &str) -> StoredAgentSession {
    let session = StoredAgentSession {
        id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), engine: "codex".into(),
        native_id: Some(uuid::Uuid::new_v4().to_string()), workspace: "/tmp/codex-journal".into(),
        executable: None, model: None, project_id: None, task_id: None,
    };
    repo.save_session(&session).await.unwrap();
    session
}

async fn source(repo: &DevelopmentRepository) -> (StoredAgentSession, OpeningBinding, CliProviderSnapshot) {
    repo.migrate_cli_provider_v1(&CutoverContext { boot_identity: None, owned_exit_sessions: vec![] }).await.unwrap();
    let project = Project {
        id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), name: "Fork history".into(),
        path: "/tmp/codex-fork".into(), directory_identity: "directory-identity".into(),
        archived: false, created_at: 1, last_opened_at: 1,
    };
    repo.insert_project(&project).await.unwrap();
    let task = DevelopmentTask {
        id: uuid::Uuid::new_v4().to_string(), owner: project.owner.clone(), project_id: project.id.clone(),
        goal: "Keep native conversation history".into(), requirements: String::new(), status: "todo".into(), created_at: 1,
    };
    repo.save_task(&task).await.unwrap();
    let revision = repo.save_cli_provider_revision("alice", &project.id, 0, &CliProviderConfiguration {
        engine: "codex".into(), provider_id: "provider".into(), model_id: "model".into(), provider_revision: 7,
    }).await.unwrap();
    let session = StoredAgentSession {
        id: uuid::Uuid::new_v4().to_string(), owner: "alice".into(), engine: "codex".into(), native_id: None,
        workspace: project.path.clone(), executable: Some("/usr/local/bin/codex".into()), model: Some("model".into()),
        project_id: Some(project.id.clone()), task_id: Some(task.id.clone()),
    };
    let opening = OpeningBinding {
        binding_id: uuid::Uuid::new_v4().to_string(), project_id: project.id, task_id: task.id,
        revision: revision.revision, permission_epoch: revision.permission_epoch,
        provider_id: "provider".into(), model_id: "model".into(), provider_revision: 7,
        host_instance_id: "source-host".into(), boot_identity: Some("machine:boot".into()), request_digest: "source-request".into(),
    };
    let snapshot = CliProviderSnapshot {
        binding_id: opening.binding_id.clone(), engine: "codex".into(), provider_id: opening.provider_id.clone(),
        model_id: opening.model_id.clone(), provider_revision: opening.provider_revision,
        revision: opening.revision, permission_epoch: opening.permission_epoch,
        evidence: serde_json::Map::from_iter([("protocol".into(), serde_json::json!("app-server"))]),
    };
    repo.reserve_cli_session("alice", &session, &opening, &snapshot).await.unwrap();
    repo.activate_cli_session("alice", &session.id, &opening.binding_id, &opening.host_instance_id, "source-native", opening.permission_epoch).await.unwrap();
    let mut session = session;
    session.native_id = Some("source-native".into());
    (session, opening, snapshot)
}

async fn scalar(db: &DatabaseConnection, sql: &str) -> String {
    fetch_one(db, sql, vec![]).await.unwrap().unwrap().try_get("", "value").unwrap()
}

#[tokio::test]
async fn pending_admission_survives_restart_and_never_authorizes_replay() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", directory.path().join("journal.sqlite").display());
    let db = crate::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    let session = session(&repo, "alice").await;
    repo.begin_codex_operation("alice", &session.id, "generation-1", "request", "fingerprint").await.unwrap();
    assert_eq!(scalar(&db, "SELECT state AS value FROM development_codex_operations").await, "pending");
    drop(repo);
    db.close().await.unwrap();

    let db = crate::establish_connection(&url).await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    let receipt = repo.codex_operation("alice", "request").await.unwrap().unwrap();
    assert_eq!(receipt, CodexOperationJournal {
        session_id: session.id.clone(), generation: "generation-1".into(), fingerprint: "fingerprint".into(),
        state: "unknown".into(), result: "null".into(),
    });
    assert_eq!(repo.begin_codex_operation("alice", &session.id, "generation-1", "request", "fingerprint").await.unwrap_err(), "codex_operation_conflict");
    assert_eq!(repo.begin_codex_operation("alice", &session.id, "generation-2", "request", "different").await.unwrap_err(), "codex_operation_conflict");
    repo.finish_codex_operation("alice", "request", "unknown", r#"{"state":"unknown"}"#).await.unwrap();
    assert_eq!(repo.finish_codex_operation("alice", "request", "applied", r#"{"state":"applied"}"#).await.unwrap_err(), "codex_operation_conflict");
    drop(repo);
    db.close().await.unwrap();
}

#[tokio::test]
async fn owner_scoped_request_ids_cannot_read_settle_or_conflict_other_accounts() {
    let (db, repo) = database().await;
    let alice = session(&repo, "alice").await;
    let bob = session(&repo, "bob").await;
    repo.begin_codex_operation("alice", &alice.id, "alice-generation", "shared-id", "alice-fingerprint").await.unwrap();
    assert!(repo.codex_operation("bob", "shared-id").await.unwrap().is_none());
    assert_eq!(repo.finish_codex_operation("bob", "shared-id", "applied", "{}").await.unwrap_err(), "codex_operation_not_found");
    assert_eq!(repo.begin_codex_operation("bob", &alice.id, "alice-generation", "foreign-session", "fp").await.unwrap_err(), "session_not_found");
    repo.begin_codex_operation("bob", &bob.id, "bob-generation", "shared-id", "bob-fingerprint").await.unwrap();
    repo.finish_codex_operation("bob", "shared-id", "applied", r#"{"owner":"bob"}"#).await.unwrap();
    assert_eq!(repo.codex_operation("alice", "shared-id").await.unwrap().unwrap().state, "unknown");
    assert_eq!(repo.codex_operation("bob", "shared-id").await.unwrap().unwrap().fingerprint, "bob-fingerprint");
    assert!(db.execute_unprepared(&format!(
        "INSERT INTO development_codex_operations(owner,request_id,session_id,generation,fingerprint,state,result) VALUES('bob','bypass','{}','g','f','pending','null')", alice.id,
    )).await.is_err());
}

#[tokio::test]
async fn concurrent_admission_has_exactly_one_dispatch_winner() {
    let (_, repo) = database().await;
    let session = session(&repo, "alice").await;
    let (left, right) = tokio::join!(
        repo.begin_codex_operation("alice", &session.id, "g", "request", "f"),
        repo.begin_codex_operation("alice", &session.id, "g", "request", "f"),
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert_eq!(left.err().or_else(|| right.err()).unwrap(), "codex_operation_conflict");
}

#[tokio::test]
async fn settled_receipts_are_immutable_at_repository_and_database_boundaries() {
    let (db, repo) = database().await;
    let session = session(&repo, "alice").await;
    repo.begin_codex_operation("alice", &session.id, "g", "request", "f").await.unwrap();
    let result = r#"{"state":"applied","queue_id":"native-item"}"#;
    repo.finish_codex_operation("alice", "request", "applied", result).await.unwrap();
    repo.finish_codex_operation("alice", "request", "applied", result).await.unwrap();
    for (state, value) in [("unknown", result), ("applied", "{}"), ("pending", result), ("applied", "not JSON")] {
        assert!(repo.finish_codex_operation("alice", "request", state, value).await.is_err());
    }
    assert!(db.execute_unprepared("UPDATE development_codex_operations SET state='unknown',result='{}'").await.is_err());
    assert!(db.execute_unprepared("UPDATE development_codex_operations SET generation='replacement'").await.is_err());
    assert_eq!(repo.codex_operation("alice", "request").await.unwrap().unwrap().result, result);
    assert_eq!(repo.begin_codex_operation("alice", &session.id, "g", "request", "f").await.unwrap_err(), "codex_operation_conflict");
}

#[tokio::test]
async fn invalid_admission_and_receipts_do_not_destroy_pending_evidence() {
    let (_, repo) = database().await;
    let mut session = session(&repo, "alice").await;
    for (generation, request, fingerprint) in [("", "r", "f"), ("g", "", "f"), ("g", "r", "")] {
        assert_eq!(repo.begin_codex_operation("alice", &session.id, generation, request, fingerprint).await.unwrap_err(), "invalid_codex_operation");
    }
    session.id = uuid::Uuid::new_v4().to_string();
    session.engine = "claude".into();
    repo.save_session(&session).await.unwrap();
    assert!(repo.begin_codex_operation("alice", &session.id, "g", "claude", "f").await.is_err());
    let codex = super::tests::session(&repo, "alice").await;
    repo.begin_codex_operation("alice", &codex.id, "g", "pending", "f").await.unwrap();
    assert_eq!(repo.finish_codex_operation("alice", "pending", "applied", "{broken").await.unwrap_err(), "invalid_codex_operation");
    assert_eq!(repo.codex_operation("alice", "pending").await.unwrap().unwrap().result, "null");
}

#[tokio::test]
async fn fork_persists_independent_closed_binding_and_can_resume_with_fresh_lease_identity() {
    let (db, repo) = database().await;
    let (source, opening, _) = source(&repo).await;
    repo.begin_codex_operation("alice", &source.id, "live-generation", "fork-request", "fork-fingerprint").await.unwrap();
    let fork_id = uuid::Uuid::new_v4().to_string();
    let result = serde_json::json!({"state":"applied","fork_session_id":fork_id}).to_string();
    let fork = repo.persist_codex_fork("alice", &source.id, &opening.binding_id, "fork-request", &fork_id, "fork-native", &result).await.unwrap();
    assert_eq!(fork.native_id.as_deref(), Some("fork-native"));
    assert_eq!(fork.project_id, source.project_id);
    assert_eq!(fork.task_id, source.task_id);
    assert_eq!(fork.workspace, source.workspace);
    assert_eq!(fork.executable, source.executable);
    let config = repo.read_cli_session_configuration("alice", &fork_id).await.unwrap();
    let binding = config.binding.unwrap();
    assert_eq!(binding.state, "closed");
    assert_eq!(binding.cleanup_state, "not_required");
    assert_eq!(binding.cleanup_proof.as_deref(), Some("no_execution"));
    assert!(binding.host_instance_id.is_none());
    assert_ne!(binding.binding_id, opening.binding_id);
    let mut snapshot: CliProviderSnapshot = serde_json::from_str(&config.snapshot.unwrap()).unwrap();
    assert_eq!(snapshot.binding_id, binding.binding_id);
    assert_eq!(snapshot.evidence["protocol"], "app-server");
    assert_eq!(repo.codex_operation("alice", "fork-request").await.unwrap().unwrap().result, result);
    // A fresh resume uses the ordinary reservation/activation path, not the
    // source's process identity, and cannot change the fork's native history ID.
    let mut resumed = opening.clone();
    resumed.binding_id = uuid::Uuid::new_v4().to_string();
    resumed.host_instance_id = "fresh-resume-host".into();
    resumed.request_digest = "explicit-resume".into();
    snapshot.binding_id = resumed.binding_id.clone();
    repo.reserve_cli_session("alice", &fork, &resumed, &snapshot).await.unwrap();
    repo.activate_cli_session("alice", &fork_id, &resumed.binding_id, &resumed.host_instance_id, "fork-native", resumed.permission_epoch).await.unwrap();
    let original = repo.read_cli_session_configuration("alice", &source.id).await.unwrap().binding.unwrap();
    assert_eq!(original.binding_id, opening.binding_id);
    assert_eq!(original.host_instance_id.as_deref(), Some("source-host"));
    assert_eq!(original.state, "active");
    assert_eq!(scalar(&db, "SELECT COUNT(*) || '' AS value FROM development_agent_sessions").await, "2");
    assert!(repo.persist_codex_fork("alice", &source.id, &opening.binding_id, "fork-request", &uuid::Uuid::new_v4().to_string(), "another-native", "{}").await.is_err());
}

#[tokio::test]
async fn fork_snapshot_failure_rolls_back_session_binding_and_receipt_together() {
    let (db, repo) = database().await;
    let (source, opening, _) = source(&repo).await;
    repo.begin_codex_operation("alice", &source.id, "g", "fork", "f").await.unwrap();
    let fork_id = uuid::Uuid::new_v4().to_string();
    db.execute_unprepared(&format!(
        "CREATE TRIGGER reject_fork_snapshot BEFORE INSERT ON development_session_capabilities WHEN NEW.session_id='{fork_id}' BEGIN SELECT RAISE(ABORT,'injected snapshot failure'); END",
    )).await.unwrap();
    assert_eq!(repo.persist_codex_fork("alice", &source.id, &opening.binding_id, "fork", &fork_id, "fork-native", "{}").await.unwrap_err(), "storage_failed");
    assert_eq!(repo.sessions().await.unwrap(), vec![source.clone()]);
    assert!(repo.read_cli_session_configuration("alice", &fork_id).await.is_err());
    assert_eq!(scalar(&db, "SELECT state AS value FROM development_codex_operations").await, "pending");
    assert_eq!(repo.codex_operation("alice", "fork").await.unwrap().unwrap().result, "null");
    // Retrying only the local transaction with the already observed native fork
    // is safe; it does not authorize another native thread/fork request.
    db.execute_unprepared("DROP TRIGGER reject_fork_snapshot").await.unwrap();
    repo.persist_codex_fork("alice", &source.id, &opening.binding_id, "fork", &fork_id, "fork-native", "{}").await.unwrap();
}

#[tokio::test]
async fn fork_rejects_revocation_wrong_binding_foreign_owner_and_native_aliases() {
    let (_, repo) = database().await;
    let (source, opening, _) = source(&repo).await;
    repo.begin_codex_operation("alice", &source.id, "g", "fork", "f").await.unwrap();
    let fork_id = uuid::Uuid::new_v4().to_string();
    for (owner, binding, native) in [("bob", opening.binding_id.as_str(), "fork-native"), ("alice", "wrong-binding", "fork-native"), ("alice", opening.binding_id.as_str(), "source-native")] {
        assert!(repo.persist_codex_fork(owner, &source.id, binding, "fork", &fork_id, native, "{}").await.is_err());
    }
    let other = session(&repo, "alice").await;
    assert!(repo.persist_codex_fork("alice", &source.id, &opening.binding_id, "fork", &fork_id, other.native_id.as_deref().unwrap(), "{}").await.is_err());
    repo.revoke_cli_bindings("alice", &CliRevocationScope::Project { project_id: opening.project_id.clone() }).await.unwrap();
    assert_eq!(repo.persist_codex_fork("alice", &source.id, &opening.binding_id, "fork", &fork_id, "fork-native", "{}").await.unwrap_err(), "capability_revoked");
    assert_eq!(repo.sessions().await.unwrap().len(), 2);
    assert_eq!(repo.codex_operation("alice", "fork").await.unwrap().unwrap().state, "unknown");
}

#[tokio::test]
async fn workbench_purge_removes_only_the_owners_journal() {
    let (db, repo) = database().await;
    for owner in ["alice", "bob"] {
        let session = session(&repo, owner).await;
        repo.begin_codex_operation(owner, &session.id, "g", "request", "f").await.unwrap();
    }
    UserDataRepository::new(db.clone()).purge(UserDataDomain::Workbench, "alice").await.unwrap();
    assert!(repo.codex_operation("alice", "request").await.unwrap().is_none());
    assert!(repo.codex_operation("bob", "request").await.unwrap().is_some());
    assert_eq!(repo.sessions().await.unwrap().len(), 1);
    assert!(UserDataDomain::Workbench.owner_tables().contains(&"development_codex_operations"));
}

#[tokio::test]
async fn upgrade_adds_journal_without_reclassifying_existing_session_history() {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    let previous = Migrator::migrations().iter().position(|migration| migration.name() == "m20261110_000038_codex_operations").unwrap();
    Migrator::up(&db, Some(previous as u32)).await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    let existing = session(&repo, "alice").await;
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(repo.sessions().await.unwrap(), vec![existing.clone()]);
    assert!(repo.codex_operation("alice", "new-request").await.unwrap().is_none());
    repo.begin_codex_operation("alice", &existing.id, "generation", "new-request", "fingerprint").await.unwrap();
    assert_eq!(repo.codex_operation("alice", "new-request").await.unwrap().unwrap().state, "unknown");
    assert_eq!(repo.sessions().await.unwrap()[0].native_id, existing.native_id);
}

#[tokio::test]
async fn fork_history_and_settled_receipt_survive_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", directory.path().join("fork.sqlite").display());
    let db = crate::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    let (source, opening, _) = source(&repo).await;
    repo.begin_codex_operation("alice", &source.id, "generation", "fork", "fingerprint").await.unwrap();
    let fork_id = uuid::Uuid::new_v4().to_string();
    let result = serde_json::json!({"state":"applied","fork_session_id":fork_id}).to_string();
    let stored = repo.persist_codex_fork("alice", &source.id, &opening.binding_id, "fork", &fork_id, "durable-native-fork", &result).await.unwrap();
    drop(repo);
    db.close().await.unwrap();

    let db = crate::establish_connection(&url).await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    assert!(repo.sessions().await.unwrap().contains(&stored));
    // Recovery sees no child process to clean up and does not turn the closed
    // fork into an unknown generation inherited from the source process.
    repo.recover_cli_bindings("new-host", Some("machine:boot")).await.unwrap();
    let binding = repo.read_cli_session_configuration("alice", &fork_id).await.unwrap().binding.unwrap();
    assert_eq!(binding.state, "closed");
    assert_eq!(binding.cleanup_state, "not_required");
    assert!(binding.host_instance_id.is_none());
    let receipt = repo.codex_operation("alice", "fork").await.unwrap().unwrap();
    assert_eq!(receipt.state, "applied");
    assert_eq!(receipt.result, result);
    assert_eq!(repo.begin_codex_operation("alice", &source.id, "new-generation", "fork", "fingerprint").await.unwrap_err(), "codex_operation_conflict");
    drop(repo);
    db.close().await.unwrap();
}

#[tokio::test]
async fn explicit_refusal_is_durable_idempotent_and_never_replayed_or_replaced() {
    let (db, repo) = database().await;
    let session = session(&repo, "alice").await;
    repo.begin_codex_operation("alice", &session.id, "generation", "refused", "fingerprint").await.unwrap();
    let error = r#"{"error":"unsupported_native_operation"}"#;
    repo.finish_codex_operation("alice", "refused", "failed", error).await.unwrap();
    repo.finish_codex_operation("alice", "refused", "failed", error).await.unwrap();
    let receipt = repo.codex_operation("alice", "refused").await.unwrap().unwrap();
    assert_eq!(receipt.state, "failed");
    assert_eq!(receipt.result, error);
    assert_eq!(repo.begin_codex_operation("alice", &session.id, "generation", "refused", "fingerprint").await.unwrap_err(), "codex_operation_conflict");
    assert_eq!(repo.finish_codex_operation("alice", "refused", "applied", "{}").await.unwrap_err(), "codex_operation_conflict");
    assert_eq!(repo.finish_codex_operation("alice", "refused", "failed", r#"{"error":"different"}"#).await.unwrap_err(), "codex_operation_conflict");
    assert!(db.execute_unprepared("UPDATE development_codex_operations SET state='pending',result='null' WHERE request_id='refused'").await.is_err());
}

fn observed_run(session: &StoredAgentSession) -> StoredRun {
    StoredRun {
        id: uuid::Uuid::new_v4().to_string(), owner: session.owner.clone(), session_id: Some(session.id.clone()),
        task_id: session.task_id.clone().unwrap(), project_id: session.project_id.clone().unwrap(),
        engine: "codex".into(), status: "running".into(), started_at: 2, finished_at: None,
        summary: String::new(), reason: None,
        events: r#"[{"kind":"turn/started","data":{"turn":{"id":"observed-native-turn"}}}]"#.into(),
        fingerprint: "workspace-before".into(), start_fingerprint: "workspace-before".into(), truncated: false,
        request_fingerprint: "observed-native-turn".into(), source: "native_agent".into(), runtime: None,
    }
}

#[tokio::test]
async fn observed_codex_audit_survives_close_revocation_and_stale_head_without_execution_authority() {
    for lost_authority in ["closed", "revoked", "stale_head"] {
        let (db, repo) = database().await;
        let (source, opening, snapshot) = source(&repo).await;
        match lost_authority {
            "closed" => {
                assert!(repo.record_cli_cleanup("alice", &source.id, &opening.binding_id, &opening.host_instance_id, &CleanupObservation {
                    binding_id: opening.binding_id.clone(), state: "closed".into(), proof: "owned_exit".into(), reason: None,
                }).await.unwrap().matched);
            }
            "revoked" => {
                repo.revoke_cli_bindings("alice", &CliRevocationScope::Project { project_id: opening.project_id.clone() }).await.unwrap();
            }
            "stale_head" => {
                repo.save_cli_provider_revision("alice", &opening.project_id, opening.revision, &CliProviderConfiguration {
                    engine: "codex".into(), provider_id: opening.provider_id.clone(), model_id: opening.model_id.clone(), provider_revision: opening.provider_revision,
                }).await.unwrap();
            }
            _ => unreachable!(),
        }
        let mut run = observed_run(&source);
        // The audit path must not silently relax normal execution admission.
        assert_eq!(repo.begin_cli_run(&run, &snapshot).await.unwrap_err(), "capability_revoked");
        repo.record_observed_codex_run(&run, &snapshot).await.unwrap();
        assert_eq!(repo.run("alice", &run.id).await.unwrap().unwrap().request_fingerprint, "observed-native-turn");
        let stored: CliProviderSnapshot = serde_json::from_str(&repo.run_capabilities("alice", &run.id).await.unwrap().unwrap()).unwrap();
        assert_eq!(stored, snapshot);
        assert_eq!(scalar(&db, "SELECT format AS value FROM development_run_capabilities").await, "observed_codex_v1");
        assert!(db.execute_unprepared("UPDATE development_run_capabilities SET snapshot='{}'").await.is_err());
        assert!(db.execute_unprepared("UPDATE development_run_capabilities SET format='legacy_mixed_v1'").await.is_err());
        assert_eq!(repo.record_observed_codex_run(&observed_run(&source), &snapshot).await.unwrap_err(), "busy");
        run.status = "completed".into();
        run.finished_at = Some(3);
        assert!(repo.finish_run("alice", &run, None).await.unwrap());
        assert_eq!(repo.run("alice", &run.id).await.unwrap().unwrap().status, "completed");
        assert_eq!(repo.read_cli_session_configuration("alice", &source.id).await.unwrap().binding.unwrap().state, if lost_authority == "stale_head" { "active" } else { lost_authority });
    }
}

#[tokio::test]
async fn observed_codex_audit_rejects_cross_binding_scope_and_snapshot_evidence() {
    let (_, repo) = database().await;
    let (source, _, snapshot) = source(&repo).await;
    for mismatch in ["binding", "epoch", "revision", "provider", "evidence"] {
        let mut forged = snapshot.clone();
        match mismatch {
            "binding" => forged.binding_id = uuid::Uuid::new_v4().to_string(),
            "epoch" => forged.permission_epoch += 1,
            "revision" => forged.revision += 1,
            "provider" => forged.provider_id = "other-provider".into(),
            "evidence" => { forged.evidence.insert("protocol".into(), serde_json::json!("forged")); }
            _ => unreachable!(),
        }
        assert_eq!(repo.record_observed_codex_run(&observed_run(&source), &forged).await.unwrap_err(), "capability_revoked");
    }
    for mismatch in ["owner", "session", "project", "task", "engine", "source"] {
        let mut forged = observed_run(&source);
        match mismatch {
            "owner" => forged.owner = "bob".into(),
            "session" => forged.session_id = Some(uuid::Uuid::new_v4().to_string()),
            "project" => forged.project_id = uuid::Uuid::new_v4().to_string(),
            "task" => forged.task_id = uuid::Uuid::new_v4().to_string(),
            "engine" => forged.engine = "claude".into(),
            "source" => forged.source = "host".into(),
            _ => unreachable!(),
        }
        assert!(repo.record_observed_codex_run(&forged, &snapshot).await.is_err());
    }
    assert!(repo.runs("alice", source.task_id.as_deref().unwrap()).await.unwrap().is_empty());
    repo.record_observed_codex_run(&observed_run(&source), &snapshot).await.unwrap();
}

#[tokio::test]
async fn observed_codex_snapshot_failure_rolls_back_run_and_task_status() {
    let (db, repo) = database().await;
    let (source, _, snapshot) = source(&repo).await;
    let run = observed_run(&source);
    db.execute_unprepared("CREATE TRIGGER reject_observed_snapshot BEFORE INSERT ON development_run_capabilities WHEN NEW.format='observed_codex_v1' BEGIN SELECT RAISE(ABORT,'injected audit failure'); END").await.unwrap();
    assert_eq!(repo.record_observed_codex_run(&run, &snapshot).await.unwrap_err(), "storage_failed");
    assert!(repo.run("alice", &run.id).await.unwrap().is_none());
    assert!(repo.run_capabilities("alice", &run.id).await.unwrap().is_none());
    assert_eq!(repo.task("alice", &run.task_id).await.unwrap().status, "todo");
    db.execute_unprepared("DROP TRIGGER reject_observed_snapshot").await.unwrap();
    repo.record_observed_codex_run(&run, &snapshot).await.unwrap();
    assert_eq!(repo.task("alice", &run.task_id).await.unwrap().status, "in_progress");
}

#[tokio::test]
async fn observed_codex_audit_never_attaches_a_prior_generation_to_resumed_binding() {
    let (_, repo) = database().await;
    let (source, mut opening, snapshot) = source(&repo).await;
    repo.record_cli_cleanup("alice", &source.id, &opening.binding_id, &opening.host_instance_id, &CleanupObservation {
        binding_id: opening.binding_id.clone(), state: "closed".into(), proof: "owned_exit".into(), reason: None,
    }).await.unwrap();
    opening.binding_id = uuid::Uuid::new_v4().to_string();
    opening.host_instance_id = "resumed-host".into();
    opening.request_digest = "resumed-request".into();
    let mut resumed = snapshot.clone();
    resumed.binding_id = opening.binding_id.clone();
    repo.reserve_cli_session("alice", &source, &opening, &resumed).await.unwrap();
    repo.activate_cli_session("alice", &source.id, &opening.binding_id, &opening.host_instance_id, "source-native", opening.permission_epoch).await.unwrap();
    let run = observed_run(&source);
    assert_eq!(repo.record_observed_codex_run(&run, &snapshot).await.unwrap_err(), "capability_revoked");
    assert!(repo.run("alice", &run.id).await.unwrap().is_none());
    repo.record_observed_codex_run(&run, &resumed).await.unwrap();
}
