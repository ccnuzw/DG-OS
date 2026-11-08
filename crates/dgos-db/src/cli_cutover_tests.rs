use crate::migrations::{Migrator, MigratorTrait};
use crate::repositories::development_repository::*;
use crate::{UserDataDomain, UserDataRepository};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

async fn database(upgrade: bool) -> (DatabaseConnection, DevelopmentRepository) {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    let steps = if upgrade { None } else {
        Some((Migrator::migrations().iter().position(|migration| migration.name() == "m20261104_000030_task_workspaces").unwrap() + 1) as u32)
    };
    Migrator::up(&db, steps).await.unwrap();
    (db.clone(), DevelopmentRepository::new(db))
}

async fn chain(repo: &DevelopmentRepository, owner: &str) -> (Project, DevelopmentTask, StoredAgentSession) {
    let id = uuid::Uuid::new_v4().to_string();
    let project = Project { id: id.clone(), owner: owner.into(), name: "Cutover".into(), path: format!("/tmp/{id}"), directory_identity: id, archived: false, created_at: 1, last_opened_at: 1 };
    repo.insert_project(&project).await.unwrap();
    let task = DevelopmentTask { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), project_id: project.id.clone(), goal: "Preserve history".into(), requirements: "Retain original execution records".into(), status: "todo".into(), created_at: 1 };
    repo.save_task(&task).await.unwrap();
    let session = StoredAgentSession { id: uuid::Uuid::new_v4().to_string(), owner: owner.into(), engine: "codex".into(), native_id: None, workspace: project.path.clone(), executable: None, model: None, project_id: Some(project.id.clone()), task_id: Some(task.id.clone()) };
    (project, task, session)
}

async fn cutover(repo: &DevelopmentRepository) -> CutoverReceipt {
    repo.migrate_cli_provider_v1(&CutoverContext { boot_identity: None, owned_exit_sessions: Vec::new() }).await.unwrap()
}

fn configuration() -> CliProviderConfiguration {
    CliProviderConfiguration { engine: "codex".into(), provider_id: "provider".into(), model_id: "model".into(), provider_revision: 0 }
}

async fn opening(repo: &DevelopmentRepository, project: &Project, task: &DevelopmentTask) -> (OpeningBinding, CliProviderSnapshot) {
    let revision = repo.save_cli_provider_revision(&project.owner, &project.id, 0, &configuration()).await.unwrap();
    let binding = OpeningBinding { binding_id: uuid::Uuid::new_v4().to_string(), project_id: project.id.clone(), task_id: task.id.clone(), revision: revision.revision, permission_epoch: revision.permission_epoch, provider_id: "provider".into(), model_id: "model".into(), provider_revision: 0, host_instance_id: "host".into(), boot_identity: Some("machine:boot-a".into()), request_digest: "request".into() };
    let snapshot = CliProviderSnapshot { binding_id: binding.binding_id.clone(), engine: "codex".into(), provider_id: binding.provider_id.clone(), model_id: binding.model_id.clone(), provider_revision: 0, revision: binding.revision, permission_epoch: binding.permission_epoch, evidence: Default::default() };
    (binding, snapshot)
}

fn run(task: &DevelopmentTask, session: &StoredAgentSession) -> StoredRun {
    StoredRun { id: uuid::Uuid::new_v4().to_string(), owner: task.owner.clone(), session_id: Some(session.id.clone()), task_id: task.id.clone(), project_id: task.project_id.clone(), engine: session.engine.clone(), status: "running".into(), started_at: 1, finished_at: None, summary: String::new(), reason: None, events: "[]".into(), fingerprint: "before".into(), start_fingerprint: "before".into(), truncated: false, request_fingerprint: "request".into(), source: "native_agent".into(), runtime: None }
}

async fn value(db: &DatabaseConnection, sql: &str) -> String {
    db.query_one(Statement::from_string(DbBackend::Sqlite, sql)).await.unwrap().unwrap().try_get("", "value").unwrap()
}

#[tokio::test]
async fn d136_file_database_migrates_legacy_data_and_archives_mixed_capabilities() {
    let mut builder = tempfile::Builder::new();
    builder.prefix("d136-legacy-database-");
    let mut directory = match std::env::var_os("DGOS_D136_EVIDENCE_DIR") {
        Some(root) => builder.tempdir_in(root).unwrap(),
        None => builder.tempdir().unwrap(),
    };
    if std::env::var_os("DGOS_D136_EVIDENCE_DIR").is_some() { directory.disable_cleanup(true); }
    eprintln!("D136 legacy SQLite evidence: {}", directory.path().display());
    let path = directory.path().join("d136-before-cutover.sqlite");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let db = crate::establish_connection(&url).await.unwrap();
    let boundary = Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m20261104_000030_task_workspaces")
        .unwrap()
        + 1;
    Migrator::up(&db, Some(boundary as u32)).await.unwrap();
    db.execute_unprepared("INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) VALUES('alice','Alice','alice','legacy-hash',1,1)").await.unwrap();
    let repo = DevelopmentRepository::new(db.clone());
    let (project, task, session) = chain(&repo, "alice").await;
    repo.save_session(&session).await.unwrap();
    let mixed = r#"{"providers":["providers.call"],"skills":["skills.execute"],"mcp":["mcp.call"]}"#;
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO development_capability_revisions(owner,project_id,revision,configuration,created_at) VALUES('alice',?,1,?,1)",
        [project.id.clone().into(), mixed.into()])).await.unwrap();
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO development_capability_heads(owner,project_id,revision,permission_epoch) VALUES('alice',?,1,0)",
        [project.id.clone().into()])).await.unwrap();
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO development_session_capabilities(owner,session_id,project_id,revision,permission_epoch,snapshot) VALUES('alice',?,?,1,0,?)",
        [session.id.clone().into(), project.id.clone().into(), mixed.into()])).await.unwrap();
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO development_runs(id,owner,session_id,task_id,project_id,engine,source,status,started_at,summary,events,fingerprint,start_fingerprint,truncated,request_fingerprint) VALUES('legacy-running','alice',?,?,?,'codex','native_agent','running',1,'original summary','[]','after','before',0,'old-request')",
        [session.id.clone().into(), task.id.clone().into(), project.id.clone().into()])).await.unwrap();
    drop(repo);
    db.close().await.unwrap();
    let backup = directory.path().join("old-000030.sqlite");
    std::fs::copy(&path, &backup).unwrap();
    let db = crate::establish_connection(&url).await.unwrap();
    db.execute_unprepared("CREATE TRIGGER reject_d136_ai_registration BEFORE INSERT ON seaql_migrations WHEN NEW.version='m20261107_000031_ai_runs' BEGIN SELECT RAISE(ABORT,'injected D136 schema registration failure'); END").await.unwrap();
    let failure = Migrator::up(&db, None).await.unwrap_err();
    std::fs::write(directory.path().join("schema-upgrade-failure.txt"), failure.to_string()).unwrap();
    assert_eq!(value(&db, "SELECT username AS value FROM users WHERE id='alice'").await, "Alice");
    db.close().await.unwrap();
    // Retain the failed file, restoring the offline baseline into a separate file.
    let restored = directory.path().join("restored.sqlite");
    std::fs::copy(&backup, &restored).unwrap();
    let restored_url = format!("sqlite://{}?mode=rwc", restored.display());
    let db = crate::establish_connection(&restored_url).await.unwrap();
    assert_eq!(value(&db, "SELECT version AS value FROM seaql_migrations ORDER BY version DESC LIMIT 1").await, "m20261104_000030_task_workspaces");
    assert_eq!(value(&db, "SELECT CAST(COUNT(*) AS TEXT) AS value FROM sqlite_master WHERE type='table' AND name='development_cli_session_bindings'").await, "0");
    let repo = DevelopmentRepository::new(db.clone());

    Migrator::up(&db, None).await.unwrap();
    db.execute_unprepared("CREATE TRIGGER reject_d136_archive BEFORE INSERT ON development_cli_legacy_history BEGIN SELECT RAISE(ABORT,'injected D136 archive failure'); END").await.unwrap();
    let archive_failure = repo.migrate_cli_provider_v1(&CutoverContext { boot_identity: None, owned_exit_sessions: vec![] }).await.unwrap_err();
    std::fs::write(directory.path().join("archive-upgrade-failure.txt"), archive_failure).unwrap();
    assert_eq!(value(&db, "SELECT CAST(COUNT(*) AS TEXT) AS value FROM development_cli_legacy_history").await, "0");
    assert!(!repo.read_cli_cutover("alice").await.unwrap().complete);
    assert_eq!(repo.capability_revision("alice", &project.id, None).await.unwrap().unwrap().configuration, mixed);
    db.execute_unprepared("DROP TRIGGER reject_d136_archive").await.unwrap();
    let receipt = cutover(&repo).await;
    assert_eq!(receipt.archived_sessions, 1);
    assert_eq!(value(&db, "SELECT username AS value FROM users WHERE id='alice'").await, "Alice");
    assert_eq!(repo.workspace("alice").await.unwrap().projects[0].id, project.id);
    assert_eq!(repo.task("alice", &task.id).await.unwrap(), task);
    assert!(repo.sessions().await.unwrap().iter().any(|retained| retained.id == session.id && retained.owner == session.owner));
    let retained_runs = repo.runs("alice", &task.id).await.unwrap();
    assert_eq!(retained_runs.len(), 1);
    assert_eq!(retained_runs[0].id, "legacy-running");
    assert_eq!(retained_runs[0].summary, "original summary");
    assert_eq!(retained_runs[0].status, "unknown");
    assert_eq!(repo.capability_epoch("alice", &project.id).await.unwrap(), 1);
    assert!(repo.read_cli_session_configuration("alice", &session.id).await.unwrap().legacy_read_only);
    assert_eq!(value(&db, "SELECT capability_snapshot AS value FROM development_cli_legacy_history").await, mixed);
    assert_eq!(repo.capability_revision("alice", &project.id, None).await.unwrap(), None);
    assert!(repo.save_capability_revision("alice", &project.id, 1, mixed).await.is_err());
    assert!(repo.cli_provider_revision("alice", &project.id, None).await.unwrap().is_none());
    assert_eq!(repo.read_session_capabilities("alice", &session.id).await.unwrap().unwrap().snapshot, mixed);
    assert!(repo.start_run(&run(&task, &session)).await.is_err());
    assert!(cutover(&repo).await.already_complete);
    assert!(db.query_all(Statement::from_string(DbBackend::Sqlite, "PRAGMA foreign_key_check")).await.unwrap().is_empty());
    drop(repo);
    db.close().await.unwrap();
    let reopened = crate::establish_connection(&restored_url).await.unwrap();
    let repo = DevelopmentRepository::new(reopened.clone());
    assert!(repo.read_cli_session_configuration("alice", &session.id).await.unwrap().legacy_read_only);
    assert!(cutover(&repo).await.already_complete);
    assert_eq!(repo.runs("alice", &task.id).await.unwrap()[0].status, "unknown");
    drop(repo);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn upgrade_archives_bad_json_and_external_unknown_without_local_blocker() {
    let (db, repo) = database(false).await;
    let (p, t, s) = chain(&repo, "alice").await;
    repo.save_session(&s).await.unwrap();
    // These APIs predate the format column, so seed the pre-upgrade database directly.
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO development_capability_revisions(owner,project_id,revision,configuration,created_at) VALUES('alice',?,7,'{}',1)", [p.id.clone().into()])).await.unwrap();
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO development_capability_heads(owner,project_id,revision,permission_epoch) VALUES('alice',?,7,3)", [p.id.clone().into()])).await.unwrap();
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO development_session_capabilities(owner,session_id,project_id,revision,permission_epoch,snapshot) VALUES('alice',?,?,7,3,'{}')", [s.id.clone().into(),p.id.clone().into()])).await.unwrap();
    db.execute_unprepared("PRAGMA ignore_check_constraints=ON; UPDATE development_session_capabilities SET snapshot='{broken'; PRAGMA ignore_check_constraints=OFF;").await.unwrap();
    Migrator::up(&db,None).await.unwrap();
    repo.start_run(&run(&t,&s)).await.unwrap();
    let first = cutover(&repo).await;
    assert_eq!(first.archived_sessions,1);
    let view = repo.read_cli_session_configuration("alice",&s.id).await.unwrap();
    assert!(view.legacy_read_only);
    let binding = view.binding.unwrap();
    assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("unknown","not_required"));
    assert_eq!(value(&db,"SELECT capability_snapshot AS value FROM development_cli_legacy_history").await,"{broken");
    assert_eq!(repo.runs("alice",&t.id).await.unwrap()[0].status,"unknown");
    assert_eq!(repo.read_cli_cutover("alice").await.unwrap().local_cleanup_pending,0);
    assert!(cutover(&repo).await.already_complete);
    assert_eq!(repo.capability_epoch("alice",&p.id).await.unwrap(),4);
    let next = repo.save_cli_provider_revision("alice",&p.id,0,&configuration()).await.unwrap();
    assert_eq!((next.revision,next.permission_epoch),(8,4));
    assert_eq!(repo.cli_provider_revision("alice",&p.id,None).await.unwrap(),Some(next.clone()));
    assert_eq!(repo.cli_provider_revision("alice",&p.id,Some(8)).await.unwrap(),Some(next.clone()));
    assert!(repo.cli_provider_revision("bob",&p.id,None).await.is_err());
    assert!(repo.capability_revision("alice",&p.id,None).await.unwrap().is_none());
    assert!(next.created_at>0);
    assert!(repo.read_cli_session_configuration("bob",&s.id).await.is_err());
    assert!(repo.save_capability_revision("alice",&p.id,8,"{}").await.is_err());
}

#[tokio::test]
async fn archive_failure_rolls_back_marker_heads_and_prior_archives() {
    let (db,repo)=database(true).await;
    for _ in 0..2 { let (_,_,s)=chain(&repo,"alice").await; repo.save_session(&s).await.unwrap(); }
    let project=repo.workspace("alice").await.unwrap().projects[0].clone();
    repo.save_capability_revision("alice",&project.id,0,"{}").await.unwrap();
    db.execute_unprepared("CREATE TRIGGER fail_archive BEFORE INSERT ON development_cli_legacy_history WHEN (SELECT COUNT(*) FROM development_cli_legacy_history)>0 BEGIN SELECT RAISE(ABORT,'fixture'); END;").await.unwrap();
    assert!(repo.migrate_cli_provider_v1(&CutoverContext { boot_identity: None, owned_exit_sessions: vec![] }).await.is_err());
    assert_eq!(value(&db,"SELECT CAST(COUNT(*) AS TEXT) AS value FROM development_cli_legacy_history").await,"0");
    assert_eq!(value(&db,"SELECT CAST(COUNT(*) AS TEXT) AS value FROM development_cli_session_bindings").await,"0");
    assert!(!repo.read_cli_cutover("alice").await.unwrap().complete);
    assert_eq!(repo.capability_epoch("alice",&project.id).await.unwrap(),0);
    assert_eq!(repo.capability_revision("alice",&project.id,None).await.unwrap().unwrap().revision,1);
    db.execute_unprepared("DROP TRIGGER fail_archive").await.unwrap();
    assert_eq!(cutover(&repo).await.archived_sessions,2);
}

#[tokio::test]
async fn reserve_is_atomic_and_rejects_foreign_or_mismatched_chains() {
    let (db,repo)=database(true).await;
    cutover(&repo).await;
    let (p,t,s)=chain(&repo,"alice").await;
    let (q,u,_)=chain(&repo,"alice").await;
    let (b,snapshot)=opening(&repo,&p,&t).await;
    assert!(repo.reserve_cli_session("bob",&s,&b,&snapshot).await.is_err());
    let mut wrong=b.clone(); wrong.task_id=u.id; wrong.project_id=q.id;
    assert!(repo.reserve_cli_session("alice",&s,&wrong,&snapshot).await.is_err());
    db.execute_unprepared("CREATE TRIGGER fail_snapshot BEFORE INSERT ON development_session_capabilities BEGIN SELECT RAISE(ABORT,'fixture'); END;").await.unwrap();
    assert!(repo.reserve_cli_session("alice",&s,&b,&snapshot).await.is_err());
    assert!(repo.sessions().await.unwrap().is_empty());
    db.execute_unprepared("DROP TRIGGER fail_snapshot").await.unwrap();
    repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
    assert!(repo.activate_cli_session("alice",&s.id,"stale","host","native",b.permission_epoch).await.is_err());
    assert_eq!(repo.sessions().await.unwrap()[0].native_id,None);
    repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","native",b.permission_epoch).await.unwrap();
    let r=run(&t,&s);
    let mut wrong_snapshot=snapshot.clone(); wrong_snapshot.model_id="other".into();
    assert!(repo.begin_cli_run(&r,&wrong_snapshot).await.is_err());
    assert!(repo.run("alice",&r.id).await.unwrap().is_none());
    db.execute_unprepared("CREATE TRIGGER fail_run_snapshot BEFORE INSERT ON development_run_capabilities BEGIN SELECT RAISE(ABORT,'fixture'); END;").await.unwrap();
    assert!(repo.begin_cli_run(&r,&snapshot).await.is_err());
    assert!(repo.run("alice",&r.id).await.unwrap().is_none());
    assert_eq!(repo.task("alice",&t.id).await.unwrap().status,"todo");
    db.execute_unprepared("DROP TRIGGER fail_run_snapshot").await.unwrap();
    repo.begin_cli_run(&r,&snapshot).await.unwrap();
    assert_eq!(repo.run("alice",&r.id).await.unwrap().unwrap().source,"native_agent");
    assert_eq!(serde_json::from_str::<CliProviderSnapshot>(&repo.run_capabilities("alice",&r.id).await.unwrap().unwrap()).unwrap(),snapshot);
    assert_eq!(repo.begin_cli_run(&run(&t,&s),&snapshot).await.unwrap_err(),"busy");
}

#[tokio::test]
async fn resumed_generation_ignores_stale_cleanup_and_revocation() {
    let (db,repo)=database(true).await;
    cutover(&repo).await;
    let (p,t,mut s)=chain(&repo,"alice").await;
    let (mut b,mut snapshot)=opening(&repo,&p,&t).await;
    repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
    repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","native",b.permission_epoch).await.unwrap();
    let old=CleanupTarget { owner:"alice".into(),session_id:s.id.clone(),binding_id:b.binding_id.clone(),host_instance_id:"host".into() };
    let exit=CleanupObservation { binding_id:b.binding_id.clone(),state:"closed".into(),proof:"owned_exit".into(),reason:None };
    assert!(repo.record_cli_cleanup("alice",&s.id,&old.binding_id,"host",&exit).await.unwrap().matched);
    s.native_id=Some("native".into());
    let mut replaced=s.clone(); replaced.native_id=Some("other".into());
    assert!(repo.save_session(&replaced).await.is_err());
    replaced=s.clone(); replaced.executable=Some("/tmp/other-cli".into());
    assert!(repo.save_session(&replaced).await.is_err());
    b.binding_id=uuid::Uuid::new_v4().to_string(); snapshot.binding_id=b.binding_id.clone();
    repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
    assert!(repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","different-native",b.permission_epoch).await.is_err());
    assert_eq!(repo.sessions().await.unwrap()[0].native_id.as_deref(),Some("native"));
    assert!(!repo.revoke_cli_binding(&old).await.unwrap());
    assert!(!repo.record_cli_cleanup("alice",&s.id,&old.binding_id,"host",&exit).await.unwrap().matched);
    assert_eq!(repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap().state,"opening");
    let targets=repo.revoke_cli_bindings("alice",&CliRevocationScope::Project { project_id:p.id.clone() }).await.unwrap();
    assert_eq!(targets[0].binding_id,b.binding_id);
    assert!(repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","native",b.permission_epoch).await.is_err());
    let exit=CleanupObservation { binding_id:b.binding_id.clone(),state:"closed".into(),proof:"owned_exit".into(),reason:None };
    assert!(repo.record_cli_cleanup("alice",&s.id,&b.binding_id,"host",&exit).await.unwrap().matched);
    let binding=repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap();
    assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("revoked","stopped"));
    assert!(db.execute_unprepared("UPDATE development_cli_session_bindings SET state='active' WHERE state='revoked'").await.is_err());
    assert!(repo.reserve_cli_session("alice",&s,&b,&snapshot).await.is_err());
}

#[tokio::test]
async fn boot_recheck_converges_cleanup_without_restoring_authority() {
    let (_,repo)=database(true).await;
    cutover(&repo).await;
    let (p,t,s)=chain(&repo,"alice").await;
    let (b,snapshot)=opening(&repo,&p,&t).await;
    repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
    repo.revoke_cli_bindings("alice",&CliRevocationScope::Project { project_id:p.id }).await.unwrap();
    assert!(repo.recheck_cli_cleanup(&HostBootObservation { boot_identity:"boot-b".into() }).await.is_err());
    assert_eq!(repo.recheck_cli_cleanup(&HostBootObservation { boot_identity:"machine:boot-a".into() }).await.unwrap(),0);
    assert_eq!(repo.recheck_cli_cleanup(&HostBootObservation { boot_identity:"other-machine:boot-b".into() }).await.unwrap(),0);
    assert_eq!(repo.recheck_cli_cleanup(&HostBootObservation { boot_identity:"machine:boot-b".into() }).await.unwrap(),1);
    let binding=repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap();
    assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("revoked","stopped"));
}

#[tokio::test]
async fn unknown_cleanup_requires_exit_evidence_before_stopping() {
    let (_,repo)=database(true).await;
    cutover(&repo).await;
    let (p,t,s)=chain(&repo,"alice").await;
    let (b,snapshot)=opening(&repo,&p,&t).await;
    repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
    let mut observation=CleanupObservation { binding_id:b.binding_id.clone(),state:"unknown".into(),proof:String::new(),reason:Some("cleanup_pending".into()) };
    assert!(repo.record_cli_cleanup("alice",&s.id,&b.binding_id,"host",&observation).await.unwrap().matched);
    let binding=repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap();
    assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("unknown","unknown"));
    assert_eq!(binding.cleanup_proof,None);
    observation.state="closed".into();
    assert_eq!(repo.record_cli_cleanup("alice",&s.id,&b.binding_id,"host",&observation).await.unwrap_err(),"invalid_cli_binding");
    observation.proof="owned_exit".into();
    assert!(repo.record_cli_cleanup("alice",&s.id,&b.binding_id,"host",&observation).await.unwrap().matched);
    let binding=repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap();
    assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("closed","stopped"));
    assert_eq!(binding.cleanup_proof.as_deref(),Some("owned_exit"));
}

#[tokio::test]
async fn host_recovery_preserves_history_and_does_not_claim_exit() {
    let (_,repo)=database(true).await;
    let (_,_,legacy)=chain(&repo,"alice").await;
    repo.save_session(&legacy).await.unwrap();
    cutover(&repo).await;
    let mut cases=Vec::new();
    for state in ["opening","active","closed","current"] {
        let (p,t,s)=chain(&repo,"alice").await;
        let (mut b,snapshot)=opening(&repo,&p,&t).await;
        if state=="opening" { b.boot_identity=None; }
        if state=="current" { b.host_instance_id="new-host".into(); }
        repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
        if state=="active" { repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","native",b.permission_epoch).await.unwrap(); }
        if state=="closed" {
            let exit=CleanupObservation { binding_id:b.binding_id.clone(),state:"closed".into(),proof:"no_execution".into(),reason:None };
            repo.record_cli_cleanup("alice",&s.id,&b.binding_id,"host",&exit).await.unwrap();
        }
        cases.push((state,s,b));
    }
    assert_eq!(repo.recover_cli_bindings("new-host",Some("machine:boot-b")).await.unwrap(),2);
    assert_eq!(repo.recover_cli_bindings("new-host",Some("machine:boot-b")).await.unwrap(),0);
    for (state,s,b) in &cases {
        let binding=repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap();
        match *state {
            "opening" | "active" => {
                assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("unknown","unknown"));
                assert_eq!(binding.cleanup_proof,None);
                assert_eq!(binding.cleanup_reason.as_deref(),Some("host_restart_cleanup_unconfirmed"));
                assert_eq!(binding.boot_identity.as_deref(), if *state == "opening" { None } else { Some("machine:boot-a") });
                assert!(repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","native",b.permission_epoch).await.is_err());
            }
            "closed" => assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("closed","stopped")),
            "current" => assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("opening","pending")),
            _ => unreachable!(),
        }
    }
    let historical=repo.read_cli_session_configuration("alice",&legacy.id).await.unwrap().binding.unwrap();
    assert_eq!((historical.state.as_str(),historical.cleanup_state.as_str()),("closed","not_required"));
    assert_eq!(repo.recheck_cli_cleanup(&HostBootObservation { boot_identity:"machine:boot-b".into() }).await.unwrap(),1);
    let recovered=repo.read_cli_session_configuration("alice",&cases[1].1.id).await.unwrap().binding.unwrap();
    assert_eq!((recovered.state.as_str(),recovered.cleanup_state.as_str()),("unknown","stopped"));
    let no_boot=repo.read_cli_session_configuration("alice",&cases[0].1.id).await.unwrap().binding.unwrap();
    assert_eq!((no_boot.state.as_str(),no_boot.cleanup_state.as_str(),no_boot.boot_identity.as_deref()),("unknown","unknown",None));
}

#[tokio::test]
async fn unlink_preserves_archive_and_account_purge_is_scoped() {
    let (db,repo)=database(true).await;
    let (p,_,s)=chain(&repo,"alice").await;
    let (_,_,other)=chain(&repo,"bob").await;
    repo.save_session(&s).await.unwrap(); repo.save_session(&other).await.unwrap();
    repo.save_session_capabilities("alice",&s.id,&p.id,0,0,"{\"native\":true}").await.unwrap();
    cutover(&repo).await;
    repo.unlink_project("alice",&p.id).await.unwrap();
    assert_eq!(value(&db,"SELECT capability_snapshot AS value FROM development_cli_legacy_history WHERE owner='alice'").await,"{\"native\":true}");
    assert!(repo.read_cli_session_configuration("alice",&s.id).await.unwrap().legacy_read_only);
    let data=UserDataRepository::new(db.clone());
    let orphans_before=data.orphan_count().await.unwrap();
    data.purge(UserDataDomain::Workbench,"alice").await.unwrap();
    assert!(repo.read_cli_session_configuration("alice",&s.id).await.is_err());
    assert!(repo.read_cli_session_configuration("bob",&other.id).await.unwrap().legacy_read_only);
    assert!(data.orphan_count().await.unwrap()<orphans_before);
    assert_eq!(data.purge(UserDataDomain::Workbench,"alice").await.unwrap(),0);
}

#[tokio::test]
async fn unlink_revokes_opening_and_preserves_new_snapshot_before_detaching() {
    let (db,repo)=database(true).await;
    cutover(&repo).await;
    let (p,t,s)=chain(&repo,"alice").await;
    let (b,snapshot)=opening(&repo,&p,&t).await;
    repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
    repo.unlink_project("alice",&p.id).await.unwrap();
    let binding=repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap();
    assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("revoked","pending"));
    let archived=value(&db,"SELECT capability_snapshot AS value FROM development_cli_legacy_history WHERE owner='alice'").await;
    assert_eq!(serde_json::from_str::<CliProviderSnapshot>(&archived).unwrap(),snapshot);
    assert!(repo.sessions().await.unwrap()[0].project_id.is_none());
    assert!(repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","native",b.permission_epoch).await.is_err());
}

#[tokio::test]
async fn cutover_rejects_unbound_native_runs_but_allows_host_runs() {
    let (_,repo)=database(true).await;
    cutover(&repo).await;
    let (_,t,s)=chain(&repo,"alice").await;
    repo.save_session(&s).await.unwrap();
    let native=run(&t,&s);
    assert_eq!(repo.start_run(&native).await.unwrap_err(),"legacy_session_read_only");
    assert!(repo.run("alice",&native.id).await.unwrap().is_none());
    let mut host=run(&t,&s); host.session_id=None; host.engine="host".into(); host.source="host_command".into();
    repo.start_run(&host).await.unwrap();
    assert_eq!(repo.run("alice",&host.id).await.unwrap().unwrap().source,"host_command");
}

#[tokio::test]
async fn provider_revocation_prevents_closed_session_resume_without_new_cleanup() {
    let (_,repo)=database(true).await;
    cutover(&repo).await;
    let (p,t,mut s)=chain(&repo,"alice").await;
    let (mut b,mut snapshot)=opening(&repo,&p,&t).await;
    repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
    repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","native",b.permission_epoch).await.unwrap();
    let exit=CleanupObservation { binding_id:b.binding_id.clone(),state:"closed".into(),proof:"owned_exit".into(),reason:None };
    repo.record_cli_cleanup("alice",&s.id,&b.binding_id,"host",&exit).await.unwrap();
    assert!(repo.revoke_cli_bindings("alice",&CliRevocationScope::Provider { provider_id:b.provider_id.clone() }).await.unwrap().is_empty());
    let binding=repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap();
    assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str()),("revoked","stopped"));
    s.native_id=Some("native".into()); b.binding_id=uuid::Uuid::new_v4().to_string(); snapshot.binding_id=b.binding_id.clone();
    assert!(repo.reserve_cli_session("alice",&s,&b,&snapshot).await.is_err());
}

#[tokio::test]
async fn delayed_provider_notification_revokes_closed_generation_without_losing_exit_proof() {
    let (_,repo)=database(true).await;
    cutover(&repo).await;
    let (p,t,mut s)=chain(&repo,"alice").await;
    let (mut b,mut snapshot)=opening(&repo,&p,&t).await;
    repo.reserve_cli_session("alice",&s,&b,&snapshot).await.unwrap();
    repo.activate_cli_session("alice",&s.id,&b.binding_id,"host","native",b.permission_epoch).await.unwrap();
    let exit=CleanupObservation { binding_id:b.binding_id.clone(),state:"closed".into(),proof:"owned_exit".into(),reason:None };
    repo.record_cli_cleanup("alice",&s.id,&b.binding_id,"host",&exit).await.unwrap();
    let target=CleanupTarget { owner:"alice".into(),session_id:s.id.clone(),binding_id:b.binding_id.clone(),host_instance_id:"host".into() };
    assert!(repo.revoke_cli_binding(&target).await.unwrap());
    let binding=repo.read_cli_session_configuration("alice",&s.id).await.unwrap().binding.unwrap();
    assert_eq!((binding.state.as_str(),binding.cleanup_state.as_str(),binding.cleanup_proof.as_deref()),("revoked","stopped",Some("owned_exit")));
    assert!(!repo.revoke_cli_binding(&target).await.unwrap());
    s.native_id=Some("native".into()); b.binding_id=uuid::Uuid::new_v4().to_string(); snapshot.binding_id=b.binding_id.clone();
    assert!(repo.reserve_cli_session("alice",&s,&b,&snapshot).await.is_err());
}

#[tokio::test]
async fn unlink_before_cutover_preserves_original_archive() {
    let (db,repo)=database(true).await;
    let (p,_,s)=chain(&repo,"alice").await;
    repo.save_session(&s).await.unwrap();
    repo.save_session_capabilities("alice",&s.id,&p.id,0,0,"{\"original\":true}").await.unwrap();
    repo.unlink_project("alice",&p.id).await.unwrap();
    let original=value(&db,"SELECT session_record AS value FROM development_cli_legacy_history").await;
    cutover(&repo).await;
    assert_eq!(value(&db,"SELECT session_record AS value FROM development_cli_legacy_history").await,original);
    assert_eq!(value(&db,"SELECT capability_snapshot AS value FROM development_cli_legacy_history").await,"{\"original\":true}");
    assert!(repo.read_cli_session_configuration("alice",&s.id).await.unwrap().legacy_read_only);
}
