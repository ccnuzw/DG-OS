use super::*;
use crate::{migrations::{Migrator, MigratorTrait}, UserDataDomain, UserDataRepository};
use serde_json::json;

async fn setup() -> (DatabaseConnection, DevelopmentRepository) {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    (db.clone(), DevelopmentRepository::new(db))
}
fn project(owner: &str) -> Project {
    let id = uuid::Uuid::new_v4().to_string();
    Project { path: format!("/tmp/{owner}/{id}"), id, owner: owner.into(), name: "Desktop".into(), directory_identity: "original".into(), archived: false, created_at: 1, last_opened_at: 1 }
}
#[tokio::test]
async fn desktop_cas_quota_private_content_lease_and_account_purge() {
    let (db, repo) = setup().await;
    assert_eq!(repo.desktop_state("alice").await.unwrap().revision, 0);
    let original = json!({"drafts":{"text":"private draft", "configuration":{"provider":{"provider_id":"reference"},"permission_mode":"restricted"}},"operation":{"requestId":"unknown-request","submittedGeneration":"old-generation","status":"unknown"}});
    assert_eq!(repo.save_desktop_state("alice", 0, original.clone(), || true).await.unwrap().revision, 1);
    assert_eq!(repo.desktop_state("bob").await.unwrap().value, json!({}));
    assert_eq!(repo.save_desktop_state("alice", 0, json!({}), || true).await.unwrap_err(), "desktop_state_conflict");
    assert_eq!(repo.save_desktop_state("alice", 1, json!({}), || false).await.unwrap_err(), "session_invalid");
    assert_eq!(repo.save_desktop_state("alice", 1, json!({"draft":"x".repeat(8*1024*1024)}), || true).await.unwrap_err(), "desktop_state_limit");
    for value in [json!([]), json!({"configuration":{"api_key":"secret"}}), json!({"approvals":[]}), json!({"runtime":{}}), json!({"workspaceTrust":true})] {
        assert_eq!(repo.save_desktop_state("alice", 1, value, || true).await.unwrap_err(), "invalid_desktop_state");
    }
    // Cancellation between the mutation and commit must roll the write back.
    let checks = std::cell::Cell::new(0);
    assert_eq!(repo.save_desktop_state("alice", 1, json!({"draft":"not committed"}), || { let n=checks.get(); checks.set(n+1); n==0 }).await.unwrap_err(), "session_invalid");
    assert_eq!(repo.desktop_state("alice").await.unwrap().value, original);
    repo.save_desktop_state("bob", 0, json!({"draft":"retained"}), || true).await.unwrap();
    UserDataRepository::new(db).purge(UserDataDomain::Workbench, "alice").await.unwrap();
    assert_eq!(repo.desktop_state("alice").await.unwrap().revision, 0);
    assert_eq!(repo.desktop_state("bob").await.unwrap().value, json!({"draft":"retained"}));
}
#[tokio::test]
async fn trust_is_exact_scope_owner_directory_cas_and_relocation_never_revives() {
    let (db, repo) = setup().await;
    let mut p = project("alice"); repo.insert_project(&p).await.unwrap();
    let b = project("bob"); repo.insert_project(&b).await.unwrap();
    assert_eq!(repo.workspace_trust("alice", &p).await.unwrap().revision, 0);
    assert_eq!(repo.workspace_trust("bob", &p).await.unwrap_err(), "project_not_found");
    let a = repo.set_workspace_trust("alice", &p, 0, "agent", true, || true).await.unwrap();
    assert!(a.agent && !a.terminal && !a.service);
    assert_eq!(repo.set_workspace_trust("alice", &p, 0, "terminal", true, || true).await.unwrap_err(), "workspace_trust_conflict");
    let service = repo.set_workspace_trust("alice", &p, 1, "service", true, || true).await.unwrap();
    assert!(service.agent && service.service && !service.terminal);
    let mut worktree = p.clone(); worktree.path.push_str("-worktree"); worktree.directory_identity = "worktree".into();
    assert!(!repo.workspace_trust("alice", &worktree).await.unwrap().agent);
    let mut replaced = p.clone(); replaced.directory_identity = "replacement".into();
    assert!(!repo.workspace_trust("alice", &replaced).await.unwrap().agent);
    let checks = std::cell::Cell::new(0);
    assert_eq!(repo.set_workspace_trust("alice", &p, 2, "terminal", true, || {let n=checks.get();checks.set(n+1);n==0}).await.unwrap_err(), "session_invalid");
    assert_eq!(repo.workspace_trust("alice", &p).await.unwrap().revision, 2);
    let revoked = repo.set_workspace_trust("alice", &p, 2, "agent", false, || true).await.unwrap();
    assert!(!revoked.agent && revoked.service);
    let original = p.path.clone(); p.path.push_str("-moved"); repo.update_project(&p).await.unwrap();
    p.path = original; repo.update_project(&p).await.unwrap();
    assert_eq!(repo.workspace_trust("alice", &p).await.unwrap().revision, 0);
    repo.set_workspace_trust("bob", &b, 0, "terminal", true, || true).await.unwrap();
    UserDataRepository::new(db).purge(UserDataDomain::Workbench, "alice").await.unwrap();
    assert!(repo.workspace_trust("bob", &b).await.unwrap().terminal);
}
#[tokio::test]
async fn migration_keeps_preexisting_desktop_unrelated_data_and_is_idempotent() {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    let count = Migrator::migrations().iter().position(|m|m.name()=="m20261110_000039_development_desktop").unwrap();
    Migrator::up(&db, Some(count as u32)).await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    let p = project("alice"); repo.insert_project(&p).await.unwrap();
    Migrator::up(&db, None).await.unwrap(); Migrator::up(&db, None).await.unwrap();
    assert_eq!(repo.project("alice", &p.id).await.unwrap(), p);
    assert!(fetch(&db, "PRAGMA foreign_key_check", vec![]).await.unwrap().is_empty());
}

#[tokio::test]
async fn activation_only_fills_undecided_agent_scope_and_never_reverses_durable_revoke() {
    let (db, repo) = setup().await;
    let p = project("alice"); repo.insert_project(&p).await.unwrap();
    let read = repo.workspace_trust("alice", &p).await.unwrap();
    assert!(!read.agent && !read.agent_revoked && read.revision == 0);
    assert!(fetch(&db, "SELECT * FROM development_workspace_trust", vec![]).await.unwrap().is_empty());
    let active = repo.activate_workspace_agent("alice", &p, 0, || true).await.unwrap();
    assert!(active.agent && !active.agent_revoked && !active.terminal && !active.service);
    assert_eq!(repo.activate_workspace_agent("alice", &p, 1, || true).await.unwrap().revision, 1);
    let revoked = repo.set_workspace_trust("alice", &p, 1, "agent", false, || true).await.unwrap();
    assert!(revoked.agent_revoked && !revoked.agent);
    // A new repository instance cannot lose the explicit refusal.
    let reopened = DevelopmentRepository::new(db.clone());
    assert_eq!(reopened.activate_workspace_agent("alice", &p, 2, || true).await.unwrap().revision, 2);
    assert_eq!(reopened.activate_workspace_agent("alice", &p, 1, || true).await.unwrap_err(), "workspace_trust_conflict");
    assert!(reopened.workspace_trust("alice", &p).await.unwrap().agent_revoked);
    let terminal = reopened.set_workspace_trust("alice", &p, 2, "terminal", true, || true).await.unwrap();
    assert!(terminal.agent_revoked && terminal.terminal && !terminal.agent);
    let enabled = reopened.set_workspace_trust("alice", &p, 3, "agent", true, || true).await.unwrap();
    assert!(enabled.agent && !enabled.agent_revoked && enabled.terminal);
    let other = project("alice"); repo.insert_project(&other).await.unwrap();
    let service = repo.set_workspace_trust("alice", &other, 0, "service", true, || true).await.unwrap();
    assert!(!service.agent_revoked);
    let checks = std::cell::Cell::new(0);
    assert_eq!(repo.activate_workspace_agent("alice", &other, 1, || { let n=checks.get(); checks.set(n+1); n<2 }).await.unwrap_err(), "session_invalid");
    assert_eq!(repo.workspace_trust("alice", &other).await.unwrap().revision, 1);
    let activated = repo.activate_workspace_agent("alice", &other, 1, || true).await.unwrap();
    assert!(activated.agent && activated.service && !activated.terminal);
    assert_eq!(repo.activate_workspace_agent("bob", &other, 2, || true).await.unwrap_err(), "project_not_found");
}

#[tokio::test]
async fn activation_migration_preserves_legacy_denials_without_inventing_new_grants() {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    let count = Migrator::migrations().iter().position(|m|m.name()=="m20261110_000040_workspace_agent_activation").unwrap();
    Migrator::up(&db, Some(count as u32)).await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    let denied = project("alice"); let granted = project("alice"); let untouched = project("alice");
    for p in [&denied, &granted, &untouched] { repo.insert_project(p).await.unwrap(); }
    for (p, agent) in [(&denied, 0_i64), (&granted, 1)] {
        execute(&db, "INSERT INTO development_workspace_trust(owner,project_id,path,directory_identity,agent,terminal,revision) VALUES(?,?,?,?,?,1,1)", vec![v("alice"),v(p.id.clone()),v(p.path.clone()),v(p.directory_identity.clone()),v(agent)]).await.unwrap();
    }
    Migrator::up(&db, None).await.unwrap(); Migrator::up(&db, None).await.unwrap();
    let denied = repo.activate_workspace_agent("alice", &denied, 1, || true).await.unwrap();
    assert!(!denied.agent && denied.agent_revoked && denied.terminal && denied.revision == 1);
    let granted = repo.activate_workspace_agent("alice", &granted, 1, || true).await.unwrap();
    assert!(granted.agent && !granted.agent_revoked && granted.terminal && granted.revision == 1);
    let active = repo.activate_workspace_agent("alice", &untouched, 0, || true).await.unwrap();
    assert!(active.agent && !active.agent_revoked && !active.terminal);
    assert!(fetch(&db, "PRAGMA foreign_key_check", vec![]).await.unwrap().is_empty());
}
