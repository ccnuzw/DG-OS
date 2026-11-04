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
 let historical=session("alice",&p.path); repo.initialize_legacy(&[historical.clone()]).await.unwrap(); repo.initialize_legacy(&[historical.clone()]).await.unwrap();
 assert!(repo.workspace("alice").await.unwrap().sessions[0].task_id.is_none());
 assert!(repo.classify("bob",&historical.id,&t.id).await.is_err()); assert!(repo.classify("alice",&historical.id,&other.id).await.is_err());
 repo.classify("alice",&historical.id,&t.id).await.unwrap();
 let selection=Selection{project_id:Some(p.id.clone()),task_id:Some(t.id.clone()),session_id:Some(historical.id.clone())}; repo.select("alice",&selection).await.unwrap();
 assert!(repo.project("alice",&p.id).await.unwrap().last_opened_at>1);
 assert!(repo.select("bob",&selection).await.is_err());
 assert!(repo.select("alice",&Selection{project_id:Some(q.id.clone()),..selection.clone()}).await.is_err());
 assert_eq!(repo.selection("alice").await.unwrap(),selection);
 let mut moved=repo.project("alice",&p.id).await.unwrap(); moved.path="/tmp/moved".into(); repo.update_project(&moved).await.unwrap();
 let current=repo.workspace("alice").await.unwrap().sessions.remove(0); assert_eq!(current.workspace,moved.path); assert_eq!(current.native_id,historical.native_id);
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
