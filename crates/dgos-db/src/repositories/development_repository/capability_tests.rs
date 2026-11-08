use super::*;
use crate::{migrations::{Migrator, MigratorTrait}, UserDataDomain, UserDataRepository};

async fn setup() -> (DatabaseConnection, DevelopmentRepository) {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    (db.clone(), DevelopmentRepository::new(db))
}

async fn chain(repo: &DevelopmentRepository, owner: &str, path: &str) -> (Project, DevelopmentTask, StoredAgentSession) {
    let project = Project { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), name: "Capabilities".into(), path: path.into(), directory_identity: path.into(), archived: false, created_at: 1, last_opened_at: 1 };
    repo.insert_project(&project).await.unwrap();
    let task = DevelopmentTask { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), project_id: project.id.clone(), goal: "Exercise storage".into(), requirements: "Keep owner and project boundaries".into(), status: "todo".into(), created_at: 1 };
    repo.save_task(&task).await.unwrap();
    let session = StoredAgentSession { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), engine: "codex".into(), native_id: None, workspace: path.into(), executable: None, model: None, project_id: Some(project.id.clone()), task_id: Some(task.id.clone()) };
    repo.save_session(&session).await.unwrap();
    (project, task, session)
}

fn run(task: &DevelopmentTask, session: &StoredAgentSession, started_at: i64) -> StoredRun {
    StoredRun { id: uuid::Uuid::new_v4().to_string(), owner: task.owner.clone(), session_id: Some(session.id.clone()), task_id: task.id.clone(), project_id: task.project_id.clone(), engine: session.engine.clone(), status: "running".into(), started_at, finished_at: None, summary: String::new(), reason: None, events: "[]".into(), fingerprint: "before".into(), start_fingerprint: "before".into(), truncated: false, request_fingerprint: "request".into(), source: "native_agent".into(), runtime: None }
}

#[tokio::test]
async fn capability_revisions_compare_swap_and_preserve_live_epoch() {
    let (db, repo) = setup().await;
    let (p, _, _) = chain(&repo, "alice", "/tmp/capability-cas").await;
    assert_eq!(repo.capability_epoch("alice", &p.id).await.unwrap(), 0);
    assert!(repo.capability_revision("alice", &p.id, None).await.unwrap().is_none());
    assert_eq!(repo.revoke_project_capabilities("alice", &p.id).await.unwrap(), 1);
    assert!(repo.capability_revision("alice", &p.id, None).await.unwrap().is_none());
    assert_eq!(repo.save_capability_revision("alice", &p.id, 1, "{}").await.unwrap_err(), "capability_conflict");
    let first = repo.save_capability_revision("alice", &p.id, 0, "{\"provider\":null}").await.unwrap();
    assert_eq!((first.revision, first.permission_epoch), (1, 1));
    assert!(first.created_at > 0);
    assert_eq!(repo.save_capability_revision("alice", &p.id, 0, "{}").await.unwrap_err(), "capability_conflict");
    assert_eq!(repo.revoke_project_capabilities("alice", &p.id).await.unwrap(), 2);
    let second = repo.save_capability_revision("alice", &p.id, 1, "{}").await.unwrap();
    assert_eq!((second.revision, second.permission_epoch), (2, 2));
    let historical = repo.capability_revision("alice", &p.id, Some(1)).await.unwrap().unwrap();
    assert_eq!(historical.configuration, first.configuration);
    assert_eq!(historical.permission_epoch, 2);
    assert_eq!(repo.capability_revision("alice", &p.id, None).await.unwrap(), Some(second));
    assert!(execute(&db, "UPDATE development_capability_heads SET permission_epoch=1 WHERE owner=? AND project_id=?", vec![v("alice"), v(p.id.clone())]).await.is_err());
    assert!(repo.capability_revision("bob", &p.id, None).await.is_err());
    assert!(repo.save_capability_revision("bob", &p.id, 2, "{}").await.is_err());
    let mut archived = p.clone(); archived.archived = true; repo.update_project(&archived).await.unwrap();
    assert_eq!(repo.save_capability_revision("alice", &p.id, 2, "{}").await.unwrap_err(), "project_archived");
    assert_eq!(repo.revoke_project_capabilities("alice", &p.id).await.unwrap(), 3);
}

#[tokio::test]
async fn capability_json_bounds_are_bytes_and_enforced_in_sql() {
    let (db, repo) = setup().await;
    let (p, _, s) = chain(&repo, "alice", "/tmp/capability-bounds").await;
    let bounded = format!("\"{}\"", "x".repeat(65_534));
    repo.save_capability_revision("alice", &p.id, 0, &bounded).await.unwrap();
    let oversized = format!("\"{}\"", "x".repeat(65_535));
    assert_eq!(repo.save_capability_revision("alice", &p.id, 1, &oversized).await.unwrap_err(), "invalid_capabilities");
    assert!(repo.save_capability_revision("alice", &p.id, 1, "{invalid}").await.is_err());
    let multibyte = format!("\"{}\"", "界".repeat(21_845));
    assert!(repo.save_capability_revision("alice", &p.id, 1, &multibyte).await.is_err());
    assert!(execute(&db, "INSERT INTO development_capability_revisions(owner,project_id,revision,configuration,created_at) VALUES(?,?,2,?,1)", vec![v("alice"), v(p.id.clone()), v(oversized)]).await.is_err());
    assert!(execute(&db, "INSERT INTO development_capability_revisions(owner,project_id,revision,configuration,created_at) VALUES(?,?,0,'{}',1)", vec![v("alice"), v(p.id.clone())]).await.is_err());
    let snapshot = format!("\"{}\"", "x".repeat(1_048_574));
    repo.save_session_capabilities("alice", &s.id, &p.id, 0, 0, &snapshot).await.unwrap();
    assert!(repo.save_session_capabilities("alice", &s.id, &p.id, 0, 0, &(snapshot + " ")).await.is_err());
    assert!(repo.save_session_capabilities("alice", &s.id, &p.id, -1, 0, "{}").await.is_err());
    assert!(repo.save_session_capabilities("alice", &s.id, &p.id, 0, -1, "{}").await.is_err());
}

#[tokio::test]
async fn capability_session_chains_native_zero_and_unlink_are_scoped() {
    let (db, repo) = setup().await;
    let (p, _, s) = chain(&repo, "alice", "/tmp/capability-session-a").await;
    let (q, _, other) = chain(&repo, "alice", "/tmp/capability-session-q").await;
    let (b, _, foreign) = chain(&repo, "bob", "/tmp/capability-session-b").await;
    repo.save_capability_revision("alice", &q.id, 0, "{}").await.unwrap();
    repo.save_session_capabilities("alice", &s.id, &p.id, 0, 0, "{\"native\":true}").await.unwrap();
    let native = repo.read_session_capabilities("alice", &s.id).await.unwrap().unwrap();
    assert_eq!((native.project_id.as_str(), native.revision), (p.id.as_str(), 0));
    assert!(repo.read_session_capabilities("bob", &s.id).await.unwrap().is_none());
    for (owner, session, project, revision) in [("alice", &s.id, &q.id, 1), ("bob", &s.id, &b.id, 0), ("alice", &foreign.id, &p.id, 0), ("alice", &s.id, &p.id, 1)] {
        assert!(repo.save_session_capabilities(owner, session, project, revision, 0, "{}").await.is_err());
    }
    assert!(execute(&db, "INSERT INTO development_session_capabilities(owner,session_id,project_id,revision,permission_epoch,snapshot) VALUES('alice',?,?,0,0,'{}')", vec![v(other.id.clone()), v(p.id.clone())]).await.is_err());
    repo.save_capability_revision("alice", &p.id, 0, "{}").await.unwrap();
    repo.save_session_capabilities("alice", &s.id, &p.id, 1, 0, "{}").await.unwrap();
    repo.revoke_project_capabilities("alice", &p.id).await.unwrap();
    assert_eq!(repo.read_session_capabilities("alice", &s.id).await.unwrap().unwrap().permission_epoch, 0);
    repo.unlink_project("alice", &p.id).await.unwrap();
    assert!(repo.read_session_capabilities("alice", &s.id).await.unwrap().is_none());
    assert!(repo.sessions().await.unwrap().iter().find(|row| row.id == s.id).unwrap().project_id.is_none());
    for table in ["development_session_capabilities", "development_capability_heads", "development_capability_revisions"] {
        assert!(fetch(&db, &format!("SELECT owner FROM {table} WHERE owner=? AND project_id=?"), vec![v("alice"), v(p.id.clone())]).await.unwrap().is_empty());
    }
    repo.save_session_capabilities("bob", &foreign.id, &b.id, 0, 0, "{}").await.unwrap();
    execute(&db, "DELETE FROM development_agent_sessions WHERE owner=? AND id=?", vec![v("bob"), v(foreign.id.clone())]).await.unwrap();
    assert!(fetch(&db, "SELECT session_id FROM development_session_capabilities WHERE owner='bob'", vec![]).await.unwrap().is_empty());
}

#[tokio::test]
async fn capability_run_snapshots_share_native_ids_and_cascade_on_retention() {
    let (db, repo) = setup().await;
    let (p, t, s) = chain(&repo, "alice", "/tmp/capability-retention").await;
    let mut first = run(&t, &s, 1);
    repo.start_run(&first).await.unwrap();
    repo.save_run_capabilities("alice", &first.id, "{\"native\":true}").await.unwrap();
    assert_eq!(repo.run("alice", &first.id).await.unwrap().unwrap().id, first.id);
    assert!(repo.save_run_capabilities("bob", &first.id, "{}").await.is_err());
    assert!(repo.run_capabilities("bob", &first.id).await.unwrap().is_none());
    assert!(repo.save_run_capabilities("alice", &first.id, "invalid").await.is_err());
    assert!(repo.save_run_capabilities("alice", &first.id, &format!("\"{}\"", "x".repeat(1_048_575))).await.is_err());
    first.status = "completed".into(); first.finished_at = Some(2);
    repo.finish_run("alice", &first, None).await.unwrap();
    for index in 2..=51 {
        let mut next = run(&t, &s, index);
        repo.start_run(&next).await.unwrap();
        repo.save_run_capabilities("alice", &next.id, "{}").await.unwrap();
        next.status = "completed".into(); next.finished_at = Some(index + 1);
        repo.finish_run("alice", &next, None).await.unwrap();
    }
    assert!(repo.run("alice", &first.id).await.unwrap().is_none());
    assert!(repo.run_capabilities("alice", &first.id).await.unwrap().is_none());
    assert_eq!(fetch(&db, "SELECT run_id FROM development_run_capabilities WHERE owner='alice'", vec![]).await.unwrap().len(), 50);
    repo.unlink_project("alice", &p.id).await.unwrap();
    assert!(fetch(&db, "SELECT run_id FROM development_run_capabilities", vec![]).await.unwrap().is_empty());
}

#[tokio::test]
async fn capability_upgrade_preserves_runs_and_account_purge_removes_only_owned_rows() {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    let count = Migrator::migrations().iter().position(|m| m.name() == "m20261104_000028_development_runtime").unwrap() + 1;
    Migrator::up(&db, Some(count as u32)).await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    let (a, t, s) = chain(&repo, "alice", "/tmp/capability-upgrade").await;
    let native = run(&t, &s, 1);
    execute(&db, "INSERT INTO development_runs(id,owner,session_id,task_id,project_id,engine,source,status,started_at,summary,events,fingerprint,start_fingerprint,truncated,request_fingerprint) VALUES(?,?,?,?,?,'codex','native_agent','running',1,'','[]','before','before',0,'request')", vec![v(native.id.clone()),v(native.owner.clone()),v(s.id.clone()),v(t.id.clone()),v(a.id.clone())]).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(repo.run("alice", &native.id).await.unwrap().unwrap().source, "native_agent");
    assert!(repo.capability_revision("alice", &a.id, None).await.unwrap().is_none());
    repo.save_run_capabilities("alice", &native.id, "{}").await.unwrap();
    let (b, _, foreign) = chain(&repo, "bob", "/tmp/capability-purge-b").await;
    for (owner, p, session) in [("alice", &a, &s), ("bob", &b, &foreign)] {
        repo.save_capability_revision(owner, &p.id, 0, "{}").await.unwrap();
        repo.save_session_capabilities(owner, &session.id, &p.id, 1, 0, "{}").await.unwrap();
    }
    UserDataRepository::new(db.clone()).purge(UserDataDomain::Workbench, "alice").await.unwrap();
    for table in ["development_run_capabilities", "development_session_capabilities", "development_capability_heads", "development_capability_revisions"] {
        assert!(fetch(&db, &format!("SELECT owner FROM {table} WHERE owner='alice'"), vec![]).await.unwrap().is_empty());
    }
    assert!(repo.capability_revision("bob", &b.id, None).await.unwrap().is_some());
    assert!(repo.read_session_capabilities("bob", &foreign.id).await.unwrap().is_some());
    assert!(fetch(&db, "PRAGMA foreign_key_check", vec![]).await.unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_capability_writes_have_one_cas_winner() {
    let (_, repo) = setup().await;
    let (p, _, _) = chain(&repo, "alice", "/tmp/capability-concurrent").await;
    let (left, right) = tokio::join!(
        repo.save_capability_revision("alice", &p.id, 0, "{\"left\":true}"),
        repo.save_capability_revision("alice", &p.id, 0, "{\"right\":true}")
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let error = if let Err(error) = left { error } else { right.unwrap_err() };
    assert_eq!(error, "capability_conflict");
    assert_eq!(repo.capability_revision("alice", &p.id, None).await.unwrap().unwrap().revision, 1);
}

#[tokio::test]
async fn run_snapshot_rejects_a_native_session_from_another_project() {
    let (db, repo) = setup().await;
    let (p, t, _) = chain(&repo, "alice", "/tmp/capability-run-chain-a").await;
    let (_, _, foreign_session) = chain(&repo, "alice", "/tmp/capability-run-chain-b").await;
    let invalid = run(&t, &foreign_session, 1);
    execute(&db, "INSERT INTO development_runs(id,owner,session_id,task_id,project_id,engine,status,started_at,finished_at,summary,reason,events,fingerprint,start_fingerprint,truncated,request_fingerprint,source,runtime) VALUES(?,?,?,?,?,'codex','running',1,NULL,'',NULL,'[]','before','before',0,'request','native_agent',NULL)", vec![v(invalid.id.clone()), v("alice"), v(foreign_session.id.clone()), v(t.id.clone()), v(p.id.clone())]).await.unwrap();
    assert_eq!(repo.save_run_capabilities("alice", &invalid.id, "{}").await.unwrap_err(), "run_not_found");
    assert!(execute(&db, "INSERT INTO development_run_capabilities(owner,run_id,snapshot) VALUES('alice',?,'{}')", vec![v(invalid.id)]).await.is_err());
}

#[tokio::test]
async fn session_detachment_clears_capability_snapshot_without_deleting_native_history() {
    let (db, repo) = setup().await;
    let (p, _, s) = chain(&repo, "alice", "/tmp/capability-detachment").await;
    repo.save_session_capabilities("alice", &s.id, &p.id, 0, 0, "{}").await.unwrap();
    execute(&db, "UPDATE development_agent_sessions SET project_id=NULL,task_id=NULL WHERE owner=? AND id=?", vec![v("alice"), v(s.id.clone())]).await.unwrap();
    assert!(repo.read_session_capabilities("alice", &s.id).await.unwrap().is_none());
    assert!(fetch(&db, "SELECT session_id FROM development_session_capabilities WHERE owner=? AND session_id=?", vec![v("alice"), v(s.id.clone())]).await.unwrap().is_empty());
    assert!(repo.sessions().await.unwrap().iter().any(|row| row.id == s.id));
}
