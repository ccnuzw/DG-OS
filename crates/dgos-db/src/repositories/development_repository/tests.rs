use super::*;
use crate::{migrations::{Migrator, MigratorTrait}, UserDataRepository, UserDataDomain};
use uuid::Uuid;
fn id() -> String { Uuid::new_v4().to_string() }
async fn setup() -> (DatabaseConnection, DevelopmentRepository) { let db=crate::establish_connection("sqlite::memory:").await.unwrap(); Migrator::up(&db,None).await.unwrap(); (db.clone(),DevelopmentRepository::new(db)) }
fn project(owner:&str,path:&str)->Project { Project{id:id(),owner:owner.into(),name:"Code".into(),path:path.into(),directory_identity:"identity".into(),archived:false,created_at:1,last_opened_at:1} }
fn task(p:&Project)->DevelopmentTask { DevelopmentTask{id:id(),owner:p.owner.clone(),project_id:p.id.clone(),goal:"Fix behaviour".into(),requirements:"Actual output matches".into(),status:"todo".into(),created_at:1} }
fn session(owner:&str,path:&str)->StoredAgentSession { StoredAgentSession{id:id(),owner:owner.into(),engine:"codex".into(),native_id:Some("native".into()),workspace:path.into(),executable:None,model:None,project_id:None,task_id:None} }
#[tokio::test]
async fn historical_classification_selection_and_relocation_keep_one_owned_chain() {
 let (_,repo)=setup().await; let p=project("alice","/tmp/code"); let q=project("alice","/tmp/other"); let b=project("bob","/tmp/bob");
 for p in [&p,&q,&b] { repo.insert_project(p).await.unwrap(); }
 let t=task(&p); let other=task(&q); for t in [&t,&other] {repo.save_task(t).await.unwrap();}
 let historical=session("alice",&p.path); repo.initialize_legacy(std::slice::from_ref(&historical)).await.unwrap(); repo.initialize_legacy(std::slice::from_ref(&historical)).await.unwrap();
 assert!(repo.workspace("alice").await.unwrap().sessions[0].task_id.is_none());
 assert!(repo.classify("bob",&historical.id,&t.id).await.is_err()); assert!(repo.classify("alice",&historical.id,&other.id).await.is_err());
 repo.classify("alice",&historical.id,&t.id).await.unwrap();
 let selection=Selection{project_id:Some(p.id.clone()),task_id:Some(t.id.clone()),session_id:Some(historical.id.clone())}; repo.select("alice",&selection).await.unwrap();
 assert!(repo.project("alice",&p.id).await.unwrap().last_opened_at>1);
 assert!(repo.select("bob",&selection).await.is_err());
 assert!(repo.select("alice",&Selection{project_id:Some(q.id.clone()),..selection.clone()}).await.is_err());
 assert_eq!(repo.selection("alice").await.unwrap(),selection);
 let mut moved=repo.project("alice",&p.id).await.unwrap(); moved.path="/tmp/moved".into(); assert_eq!(repo.update_project(&moved).await.unwrap_err(),"workspace_immutable");
 let current=repo.workspace("alice").await.unwrap().sessions.remove(0); assert_eq!(current.workspace,p.path); assert_eq!(current.native_id,historical.native_id);
 let mut stolen=current.clone(); stolen.task_id=Some(other.id); stolen.project_id=Some(q.id); assert!(repo.save_session(&stolen).await.is_err());
 repo.unlink_project("alice",&p.id).await.unwrap(); let after=repo.workspace("alice").await.unwrap(); let row=&after.sessions[0]; assert!(row.task_id.is_none()&&row.project_id.is_none()); assert_eq!(row.native_id,historical.native_id); assert_eq!(after.selection,Selection::default()); assert!(after.tasks.iter().all(|task|task.project_id!=p.id));
}
#[tokio::test]
async fn account_purge_removes_development_metadata_only_for_owner() {
 let (db,repo)=setup().await; let a=project("alice","/tmp/a"); let b=project("bob","/tmp/b");
 for p in [&a,&b] {repo.insert_project(p).await.unwrap();let t=task(p);repo.save_task(&t).await.unwrap();let mut s=session(&p.owner,&p.path);s.project_id=Some(p.id.clone());s.task_id=Some(t.id.clone());repo.save_session(&s).await.unwrap();repo.select(&p.owner,&Selection{project_id:Some(p.id.clone()),task_id:Some(t.id),session_id:Some(s.id)}).await.unwrap();}
 let users=UserDataRepository::new(db); users.purge(UserDataDomain::Workbench,"alice").await.unwrap(); users.purge(UserDataDomain::Workbench,"alice").await.unwrap();
 let removed=repo.workspace("alice").await.unwrap();assert!(removed.projects.is_empty()&&removed.tasks.is_empty()&&removed.sessions.is_empty());assert_eq!(removed.selection,Selection::default());
 let retained=repo.workspace("bob").await.unwrap();assert_eq!(retained.projects[0].id,b.id);assert_eq!(retained.tasks[0].project_id,b.id);assert_eq!(retained.sessions[0].project_id.as_deref(),Some(b.id.as_str()));
}

#[tokio::test]
async fn account_purge_cleans_workspace_children_and_counts_orphans_without_touching_files() {
 let (db,repo)=setup().await;
 let dir=tempfile::tempdir().unwrap();
 let sentinel=dir.path().join("retained.txt");
 std::fs::write(&sentinel,b"user workspace").unwrap();
 let users=UserDataRepository::new(db.clone());
 for owner in ["alice","bob","default-user"] {
  let p=project(owner,dir.path().to_str().unwrap());
  repo.insert_project(&p).await.unwrap();
  let t=task(&p);repo.save_task(&t).await.unwrap();
  let w=StoredWorkspace{id:id(),owner:owner.into(),project_id:p.id.clone(),path:p.path.clone(),directory_identity:"owned-directory".into(),common_dir:None,common_identity:None,kind:"original".into(),created_operation:None,base_commit:None};
  repo.insert_workspace(&w).await.unwrap();repo.bind_task_workspace(owner,&t.id,&w.id).await.unwrap();
  let mut s=session(owner,&p.path);s.project_id=Some(p.id.clone());s.task_id=Some(t.id.clone());repo.save_session(&s).await.unwrap();
  repo.save_review_baseline(owner,&t.id,&serde_json::json!({"files":{"retained.txt":{"hash":"before"}}})).await.unwrap();
  repo.save_host_file_ownership(owner,&t.id,"retained.txt","before","after").await.unwrap();
  execute(&db,"INSERT INTO development_handoffs(id,owner,project_id,task_id,source_session_id,target_engine,permission_epoch,summary,fingerprint,created_at) VALUES(?,?,?,?,?,'claude',1,'handoff','fingerprint',1)",vec![v(id()),v(owner),v(p.id),v(t.id),v(s.id)]).await.unwrap();
 }
 let tables=["development_workspaces","development_task_workspaces","development_file_writes","development_handoffs"];
 let before=users.orphan_count().await.unwrap();
 for table in tables {
  let row=fetch_one(&db,&format!("SELECT COUNT(*) AS n FROM {table} WHERE owner='alice'"),vec![]).await.unwrap().unwrap();
  assert_eq!(row.try_get::<i64>("","n").unwrap(),1,"{table}");
 }
 assert_eq!(before,16,"eight metadata rows per non-default owner, including the review baseline");
 crate::UserRepository::new(db.clone()).insert(crate::NewUser{id:"bob",username:"bob",username_key:"bob",password_hash:"$argon2id$x",now:1}).await.unwrap();
 assert_eq!(users.orphan_count().await.unwrap(),16,"Alice and now-orphaned default-user rows");
 users.purge(UserDataDomain::Workbench,"alice").await.unwrap();
 assert_eq!(users.purge(UserDataDomain::Workbench,"alice").await.unwrap(),0);
 for table in tables {
  let rows=fetch(&db,&format!("SELECT owner FROM {table}"),vec![]).await.unwrap();
  assert_eq!(rows.len(),2,"{table}");
  let mut owners=rows.iter().map(|r|r.try_get::<String>("","owner").unwrap()).collect::<Vec<_>>();owners.sort();
  assert_eq!(owners,vec!["bob","default-user"],"{table}");
 }
 assert_eq!(users.orphan_count().await.unwrap(),8);
 users.purge(UserDataDomain::Workbench,"default-user").await.unwrap();
 assert_eq!(users.orphan_count().await.unwrap(),0);
 assert!(fetch(&db,"PRAGMA foreign_key_check",vec![]).await.unwrap().is_empty());
 assert_eq!(std::fs::read(&sentinel).unwrap(),b"user workspace");
}

#[tokio::test]
async fn relocation_failure_rolls_back_project_and_session_paths() {
 let (db,repo)=setup().await; let p=project("alice","/tmp/before"); repo.insert_project(&p).await.unwrap(); let t=task(&p); repo.save_task(&t).await.unwrap(); let mut s=session("alice",&p.path); s.project_id=Some(p.id.clone()); s.task_id=Some(t.id); repo.save_session(&s).await.unwrap();
 db.execute_unprepared("CREATE TRIGGER fail_relocation BEFORE UPDATE OF workspace ON development_agent_sessions BEGIN SELECT RAISE(ABORT, 'injected'); END;").await.unwrap();
 let mut moved=p.clone(); moved.path="/tmp/after".into(); assert!(repo.update_project(&moved).await.is_err());
 assert_eq!(repo.project("alice",&p.id).await.unwrap().path,p.path); assert_eq!(repo.workspace("alice").await.unwrap().sessions[0].workspace,p.path);
}

fn run(t:&DevelopmentTask,s:&StoredAgentSession)->StoredRun {
 StoredRun{id:id(),owner:t.owner.clone(),session_id:Some(s.id.clone()),task_id:t.id.clone(),project_id:t.project_id.clone(),engine:s.engine.clone(),source:"native_agent".into(),runtime:None,status:"running".into(),started_at:1,finished_at:None,summary:String::new(),reason:None,events:"[]".into(),fingerprint:"start".into(),start_fingerprint:"start".into(),truncated:false,request_fingerprint:"prompt-hash".into()}
}

#[tokio::test]
async fn execution_recovery_baseline_immutability_and_unlink_are_owned() {
 let (_,repo)=setup().await;let p=project("alice","/tmp/stage2");repo.insert_project(&p).await.unwrap();let t=task(&p);repo.save_task(&t).await.unwrap();let mut s=session("alice",&p.path);s.project_id=Some(p.id.clone());s.task_id=Some(t.id.clone());repo.save_session(&s).await.unwrap();
 let baseline=serde_json::json!({"files":{},"fingerprint":"original"});repo.save_review_baseline("alice",&t.id,&baseline).await.unwrap();repo.save_review_baseline("alice",&t.id,&serde_json::json!({"fingerprint":"replacement"})).await.unwrap();assert_eq!(repo.review_baseline("alice",&t.id).await.unwrap(),Some(baseline));assert!(repo.review_baseline("bob",&t.id).await.unwrap().is_none());
 repo.save_review_observation("alice",&t.id,&serde_json::json!({"fingerprint":"later"})).await.unwrap();let r=run(&t,&s);repo.start_run(&r).await.unwrap();assert_eq!(repo.task("alice",&t.id).await.unwrap().status,"in_progress");assert!(repo.run("bob",&r.id).await.unwrap().is_none());
 repo.recover_runs().await.unwrap();let recovered=repo.run("alice",&r.id).await.unwrap().unwrap();assert_eq!(recovered.status,"unknown");assert_eq!(recovered.reason.as_deref(),Some("interrupted_by_restart"));assert_eq!(repo.task("alice",&t.id).await.unwrap().status,"blocked");
 repo.unlink_project("alice",&p.id).await.unwrap();assert!(repo.run("alice",&r.id).await.unwrap().is_none());assert!(repo.review_baseline("alice",&t.id).await.unwrap().is_none());assert!(repo.review_observation("alice",&t.id).await.unwrap().is_none());assert!(repo.sessions().await.unwrap()[0].task_id.is_none());
}

#[tokio::test]
async fn terminal_execution_keeps_manual_completion_and_account_purge_removes_evidence() {
 let (db,repo)=setup().await;let p=project("alice","/tmp/stage2-purge");repo.insert_project(&p).await.unwrap();let mut t=task(&p);repo.save_task(&t).await.unwrap();let mut s=session("alice",&p.path);s.project_id=Some(p.id.clone());s.task_id=Some(t.id.clone());repo.save_session(&s).await.unwrap();let mut r=run(&t,&s);repo.start_run(&r).await.unwrap();
 t.status="completed".into();repo.save_task(&t).await.unwrap();r.status="failed".into();r.finished_at=Some(2);r.reason=Some("native_error".into());assert!(repo.finish_run("alice",&r,Some("blocked")).await.unwrap());assert_eq!(repo.task("alice",&t.id).await.unwrap().status,"completed");assert!(!repo.finish_run("alice",&r,Some("review")).await.unwrap());
 repo.save_review_baseline("alice",&t.id,&serde_json::json!({})).await.unwrap();repo.save_review_observation("alice",&t.id,&serde_json::json!({})).await.unwrap();UserDataRepository::new(db).purge(UserDataDomain::Workbench,"alice").await.unwrap();assert!(repo.run("alice",&r.id).await.unwrap().is_none());assert!(repo.review_baseline("alice",&t.id).await.unwrap().is_none());
}

#[tokio::test]
async fn host_receipts_and_command_configuration_remain_owner_scoped_and_cascade() {
 let (db,repo)=setup().await;let a=project("alice","/tmp/host-a");let b=project("bob","/tmp/host-b");for p in [&a,&b] {repo.insert_project(p).await.unwrap();}
 let t=task(&a);repo.save_task(&t).await.unwrap();
 let config=vec![("check".into(),serde_json::json!({"program":"cargo","args":["test"],"env_refs":{"ACCESS_TOKEN":"HOST_TOKEN"}}).to_string())];
 repo.save_development_commands("alice",&a.id,&config).await.unwrap();repo.save_development_commands("bob",&b.id,&config).await.unwrap();assert!(repo.development_commands("bob",&a.id).await.is_err());
 let s=session("alice",&a.path);let mut r=run(&t,&s);r.engine="host".into();r.source="host_command".into();r.session_id=None;r.runtime=Some("{}".into());repo.start_run(&r).await.unwrap();assert!(repo.start_run(&r).await.is_err());
 repo.recover_runs().await.unwrap();repo.recover_runs().await.unwrap();assert_eq!(repo.run("alice",&r.id).await.unwrap().unwrap().status,"unknown");assert_eq!(repo.runs("alice",&t.id).await.unwrap().len(),1);assert!(repo.run("bob",&r.id).await.unwrap().is_none());
 repo.unlink_project("alice",&a.id).await.unwrap();let rows=fetch(&db,"SELECT id FROM development_commands WHERE owner='alice'",vec![]).await.unwrap();assert!(rows.is_empty());assert_eq!(repo.development_commands("bob",&b.id).await.unwrap().len(),1);
 UserDataRepository::new(db.clone()).purge(UserDataDomain::Workbench,"bob").await.unwrap();assert!(fetch(&db,"SELECT id FROM development_commands",vec![]).await.unwrap().is_empty());
}

#[tokio::test]
async fn host_checkpoint_is_atomic_and_cannot_change_another_task_or_finished_run() {
 let (db,repo)=setup().await;let p=project("alice","/tmp/host-checkpoint");repo.insert_project(&p).await.unwrap();let t=task(&p);let other=task(&p);repo.save_task(&t).await.unwrap();repo.save_task(&other).await.unwrap();
 let mut r=run(&t,&session("alice",&p.path));r.engine="host".into();r.source="host_command".into();r.session_id=None;r.runtime=Some("{}".into());repo.start_run(&r).await.unwrap();r.status="completed".into();r.finished_at=Some(2);
 let mut mismatched=r.clone();mismatched.task_id=other.id.clone();assert!(repo.checkpoint_host_run(&mismatched).await.is_err());assert_eq!(repo.task("alice",&other.id).await.unwrap().status,"todo");assert_eq!(repo.run("alice",&r.id).await.unwrap().unwrap().status,"running");
 db.execute_unprepared("CREATE TRIGGER fail_host_task BEFORE UPDATE OF status ON development_tasks BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
 assert!(repo.checkpoint_host_run(&r).await.is_err());assert_eq!(repo.run("alice",&r.id).await.unwrap().unwrap().status,"running");assert_eq!(repo.task("alice",&t.id).await.unwrap().status,"in_progress");
 db.execute_unprepared("DROP TRIGGER fail_host_task").await.unwrap();repo.checkpoint_host_run(&r).await.unwrap();assert_eq!(repo.task("alice",&t.id).await.unwrap().status,"review");
 r.status="failed".into();assert!(repo.checkpoint_host_run(&r).await.is_err());assert_eq!(repo.run("alice",&r.id).await.unwrap().unwrap().status,"completed");assert_eq!(repo.task("alice",&t.id).await.unwrap().status,"review");
}

#[tokio::test]
async fn runtime_migration_preserves_existing_native_runs_and_ownership() {
 let db=crate::establish_connection("sqlite::memory:").await.unwrap();let stage2_count=Migrator::migrations().iter().position(|m|m.name()=="m20261104_000027_executions_review").unwrap()+1;Migrator::up(&db,Some(stage2_count as u32)).await.unwrap();
 let repo=DevelopmentRepository::new(db.clone());let p=project("alice","/tmp/stage2-upgrade");repo.insert_project(&p).await.unwrap();let t=task(&p);repo.save_task(&t).await.unwrap();let mut s=session("alice",&p.path);s.project_id=Some(p.id.clone());s.task_id=Some(t.id.clone());repo.save_session(&s).await.unwrap();
 let r=run(&t,&s);
 execute(&db,"INSERT INTO development_runs(id,owner,session_id,task_id,project_id,engine,status,started_at,finished_at,summary,reason,events,fingerprint,start_fingerprint,truncated,request_fingerprint) VALUES(?,?,?,?,?,?,'completed',1,2,'native summary',NULL,'[]','after','before',0,'request-hash')",vec![v(r.id.clone()),v("alice"),v(s.id.clone()),v(t.id.clone()),v(p.id.clone()),v("codex")]).await.unwrap();
 Migrator::up(&db,None).await.unwrap();
 let retained=repo.run("alice",&r.id).await.unwrap().unwrap();assert_eq!(retained.source,"native_agent");assert_eq!(retained.session_id,Some(s.id));assert_eq!(retained.status,"completed");assert_eq!(retained.summary,"native summary");assert_eq!(retained.fingerprint,"after");assert_eq!(retained.start_fingerprint,"before");assert!(retained.runtime.is_none());assert!(repo.run("bob",&r.id).await.unwrap().is_none());
 let mut host=run(&t,&session("alice",&p.path));host.engine="host".into();host.source="host_command".into();host.session_id=None;host.runtime=Some("{}".into());repo.start_run(&host).await.unwrap();assert_eq!(repo.runs("alice",&t.id).await.unwrap().len(),2);
 assert!(fetch(&db,"PRAGMA foreign_key_check",vec![]).await.unwrap().is_empty());
}

#[tokio::test]
async fn workspace_bindings_are_owned_and_freeze_after_baseline_or_session() {
 let (_,repo)=setup().await;let p=project("alice","/tmp/workspace-original");let other=project("bob","/tmp/workspace-foreign");repo.insert_project(&p).await.unwrap();repo.insert_project(&other).await.unwrap();let t=task(&p);repo.save_task(&t).await.unwrap();
 let w=StoredWorkspace{id:id(),owner:"alice".into(),project_id:p.id.clone(),path:"/tmp/workspace-one".into(),directory_identity:"one".into(),common_dir:Some("/tmp/common".into()),common_identity:Some("common".into()),kind:"imported".into(),created_operation:None,base_commit:None};let second=StoredWorkspace{id:id(),path:"/tmp/workspace-two".into(),..w.clone()};for w in [&w,&second] {repo.insert_workspace(w).await.unwrap();}
 assert!(repo.bind_task_workspace("bob",&t.id,&w.id).await.is_err());repo.bind_task_workspace("alice",&t.id,&w.id).await.unwrap();assert_eq!(repo.task_workspace("alice",&t.id).await.unwrap(),Some(w.clone()));
 repo.save_review_baseline("alice",&t.id,&serde_json::json!({"files":{}})).await.unwrap();assert_eq!(repo.bind_task_workspace("alice",&t.id,&second.id).await.unwrap_err(),"workspace_immutable");repo.bind_task_workspace("alice",&t.id,&w.id).await.unwrap();
 let fresh=task(&p);repo.save_task(&fresh).await.unwrap();let mut s=session("alice",&p.path);s.project_id=Some(p.id.clone());s.task_id=Some(fresh.id.clone());repo.save_session(&s).await.unwrap();assert_eq!(repo.bind_task_workspace("alice",&fresh.id,&w.id).await.unwrap_err(),"workspace_immutable");
 repo.unlink_project("alice",&p.id).await.unwrap();assert!(repo.workspace_by_id("alice",&w.id).await.unwrap().is_none());
}

#[tokio::test]
async fn host_file_ownership_requires_baseline_continuity_and_never_recovers_after_external_change() {
 let (_,repo)=setup().await;let p=project("alice","/tmp/ownership");repo.insert_project(&p).await.unwrap();let t=task(&p);repo.save_task(&t).await.unwrap();repo.save_review_baseline("alice",&t.id,&serde_json::json!({"files":{"code":{"hash":"base"},"dirty":{"hash":"dirty-base"}},"initial_changes":[" M dirty"]})).await.unwrap();
 repo.save_host_file_ownership("alice",&t.id,"code","base","saved-1").await.unwrap();assert_eq!(repo.host_file_ownership("alice",&t.id,"code").await.unwrap().unwrap()["valid"],true);
 repo.save_host_file_ownership("alice",&t.id,"code","saved-1","saved-2").await.unwrap();assert_eq!(repo.host_file_ownership("alice",&t.id,"code").await.unwrap().unwrap()["valid"],true);
 repo.save_host_file_ownership("alice",&t.id,"code","external","saved-3").await.unwrap();repo.save_host_file_ownership("alice",&t.id,"code","saved-3","saved-4").await.unwrap();assert_eq!(repo.host_file_ownership("alice",&t.id,"code").await.unwrap().unwrap()["valid"],false);
 repo.save_host_file_ownership("alice",&t.id,"dirty","dirty-base","changed").await.unwrap();assert_eq!(repo.host_file_ownership("alice",&t.id,"dirty").await.unwrap().unwrap()["valid"],false);assert!(repo.host_file_ownership("bob",&t.id,"code").await.is_err());
}

#[tokio::test]
async fn workspace_migration_preserves_legacy_receipts_and_recovers_git_intent() {
 let db=crate::establish_connection("sqlite::memory:").await.unwrap();let count=Migrator::migrations().iter().position(|m|m.name()=="m20261104_000029_project_capabilities").unwrap()+1;Migrator::up(&db,Some(count as u32)).await.unwrap();let repo=DevelopmentRepository::new(db.clone());
 let op=ProjectOperation{id:id(),owner:"alice".into(),kind:"create".into(),path:"/tmp/old-receipt".into(),status:"pending".into(),project_id:None};repo.start_operation(&op).await.unwrap();Migrator::up(&db,None).await.unwrap();assert_eq!(repo.git_operation("alice",&op.id).await.unwrap().unwrap()["kind"],"create");repo.recover_operations().await.unwrap();assert_eq!(repo.git_operation("alice",&op.id).await.unwrap().unwrap()["status"],"unknown");assert!(fetch(&db,"PRAGMA foreign_key_check",vec![]).await.unwrap().is_empty());
}

#[tokio::test]
async fn session_summary_batch_is_owned_bounded_and_excludes_tool_payloads() {
    let (db, repo) = setup().await;
    let empty = session("alice", "/tmp/no-message");
    repo.save_session(&empty).await.unwrap();
    let foreign = session("bob", "/tmp/foreign-message");
    repo.save_session(&foreign).await.unwrap();
    let p = project("alice", "/tmp/messages"); repo.insert_project(&p).await.unwrap();
    let t = task(&p); repo.save_task(&t).await.unwrap();
    let mut s = session("alice", &p.path);
    s.project_id = Some(p.id.clone()); s.task_id = Some(t.id.clone()); repo.save_session(&s).await.unwrap();
    let events = serde_json::json!([
        {"kind":"userMessage","text":"initial question"},
        {"kind":"agentMessage","text":"答".repeat(2000)},
        {"kind":"tool_result","text":"never display tool output","data":{"secret":"not returned"}},
        {"kind":"permission","text":"never display permission input"}
    ]).to_string();
    execute(&db, "INSERT INTO development_runs(id,owner,session_id,task_id,project_id,engine,source,status,started_at,finished_at,summary,events,fingerprint,start_fingerprint,truncated,request_fingerprint) VALUES(?,?,?,?,?,'codex','native_agent','completed',10,20,'not a trusted message',?,'a','b',0,'fixture')", vec![v(id()),v("alice"),v(s.id.clone()),v(t.id),v(p.id),v(events)]).await.unwrap();
    let rows = repo.session_message_summaries("alice").await.unwrap();
    assert_eq!(rows.len(), 2);
    let summary = rows.iter().find(|row| row.session_id == s.id).unwrap();
    assert_eq!(summary.preview.as_deref(), Some("答".repeat(1024).as_str()));
    assert_eq!(summary.last_activity_at, Some(20));
    let absent = rows.iter().find(|row| row.session_id == empty.id).unwrap();
    assert!(absent.preview.is_none() && absent.last_activity_at.is_none());
    assert!(!rows.iter().any(|row| row.session_id == foreign.id));
}

#[tokio::test]
async fn session_summary_uses_complete_messages_or_the_latest_message_stream() {
    use serde_json::json;
    let (db, repo) = setup().await;
    let p = project("alice", "/tmp/message-shapes"); repo.insert_project(&p).await.unwrap();
    let t = task(&p); repo.save_task(&t).await.unwrap();
    let cases = vec![
        ("claude", json!([
            {"kind":"userMessage","text":"question"},
            {"kind":"text_delta","text":"partial ","data":{"request_id":"request"}},
            {"kind":"text_delta","text":"answer","data":{"request_id":"request"}},
            {"kind":"message","text":null,"data":{"type":"assistant","native_message_id":"message","content":[
                {"type":"text","text":"Complete "},
                {"type":"tool_use","name":"Read","input":{"text":"secret input"}},
                {"type":"thinking","text":"private reasoning"},
                {"type":"text","text":"answer"}
            ]}},
            {"kind":"message","data":{"type":"user","content":[{"type":"tool_result","content":[{"type":"text","text":"secret output"}]}]}},
            {"kind":"message","data":{"type":"assistant","parent_tool_use_id":"child","content":[{"type":"text","text":"child tool output"}]}},
            {"kind":"result","text":"not a message"}
        ]), Some("Complete answer".to_owned())),
        ("claude", json!([
            {"kind":"text_delta","text":"old stream"},
            {"kind":"message","data":{"type":"assistant","content":[{"type":"text","text":"old complete"}]}},
            {"kind":"message","data":{"type":"user","content":[{"type":"tool_result","content":"secret"}]}},
            {"kind":"text_delta","text":"Latest "},
            {"kind":"text_delta","text":" ","data":{"parent_tool_use_id":null}},
            {"kind":"tool_progress","text":"secret"},
            {"kind":"text_delta","text":"answer"},
            {"kind":"text_delta","text":"child output","data":{"parent_tool_use_id":"child"}}
        ]), Some("Latest  answer".to_owned())),
        ("codex", json!([
            {"kind":"item/agentMessage/delta","text":"partial","data":{"itemId":"answer"}},
            {"kind":"agentMessage","text":"Complete Codex answer","data":{"id":"answer","type":"agentMessage"}},
            {"kind":"item/agentMessage/delta","text":"late fragment","data":{"itemId":"answer"}}
        ]), Some("Complete Codex answer".to_owned())),
        ("codex", json!([
            {"kind":"agentMessage","text":"Earlier answer","data":{"id":"answer","type":"agentMessage"}},
            {"kind":"userMessage","text":"later prompt"}
        ]), Some("later prompt".to_owned())),
        ("claude", json!([
            {"kind":"message","data":{"type":"assistant","content":[{"type":"text","text":"Earlier answer"}]}},
            {"kind":"userMessage","text":"later prompt"}
        ]), Some("later prompt".to_owned())),
        ("codex", json!([
            {"kind":"item/agentMessage/delta","text":"older","data":{"itemId":"old"}},
            {"kind":"item/agentMessage/delta","text":"new ","data":{"itemId":"new"}},
            {"kind":"item/commandExecution/outputDelta","text":"secret"},
            {"kind":"item/agentMessage/delta","text":"message","data":{"itemId":"new"}}
        ]), Some("new message".to_owned())),
        ("claude", json!([
            {"kind":"text_delta","text":"答".repeat(800)},
            {"kind":"text_delta","text":"文".repeat(800)},
            {"kind":"text_delta","text":"tail"}
        ]), Some(format!("{}{}", "答".repeat(800), "文".repeat(224)))),
        ("claude", json!([
            {"kind":"message","data":{"type":"assistant","content":[{"type":"text","text":"答".repeat(800)},{"type":"text","text":"文".repeat(800)}]}}
        ]), Some(format!("{}{}", "答".repeat(800), "文".repeat(224)))),
        ("claude", json!([
            {"kind":"message","text":"ignored outer text","data":{"type":"assistant","content":[{"type":"tool_use","text":"secret"}]}},
            {"kind":"message","data":{"type":"user","content":[{"type":"tool_result","text":"secret"}]}},
            {"kind":"text_delta","text":"child output","data":{"parent_tool_use_id":"child"}},
            {"kind":"text_delta","text":" "},
            {"kind":"permission","text":"secret"}
        ]), None),
        ("codex", json!([{"kind":"userMessage","text":"Only prompt"}]), Some("Only prompt".to_owned())),
    ];
    for (engine, events, expected) in cases {
        let mut s = session("alice", &p.path); s.engine = engine.into();
        s.project_id = Some(p.id.clone()); s.task_id = Some(t.id.clone());
        repo.save_session(&s).await.unwrap();
        execute(&db, "INSERT INTO development_runs(id,owner,session_id,task_id,project_id,engine,source,status,started_at,finished_at,summary,events,fingerprint,start_fingerprint,truncated,request_fingerprint) VALUES(?,?,?,?,?,?,'native_agent','completed',10,20,'untrusted summary',?,'a','b',0,'fixture')", vec![v(id()),v("alice"),v(s.id.clone()),v(t.id.clone()),v(p.id.clone()),v(engine),v(events.to_string())]).await.unwrap();
        let rows = repo.session_message_summaries("alice").await.unwrap();
        let summary = rows.iter().find(|row| row.session_id == s.id).unwrap();
        assert_eq!(summary.preview, expected, "{engine}: {events}");
        assert_eq!(summary.last_activity_at, summary.preview.as_ref().map(|_| 20));
    }
}
