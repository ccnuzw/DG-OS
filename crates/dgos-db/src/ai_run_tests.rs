//! Consumer-visible D136 persistence regressions on the complete migrated schema.
use crate::{AiRunRepository, ModulePermission, ModulePermissionRepository, NewAiRun,
    UserDataDomain, UserDataRepository};
use crate::migrations::Migrator;
use dgos_types::{AiFailure, AiRunListInput, AiRunStatus, AiSkillRef, AiTerminal,
    AiToolSelection, DgosError, GenerationParameters};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;

const MODULES: &str = r#"
    INSERT INTO modules(id,module_name,display_name,current_version,manifest,integrity,installed_at,updated_at)
    VALUES('mod','mod','Module','1','{"permissions":["providers.call","skills.execute","mcp.call"]}','hash',1,1),
          ('other','other','Other','1','{"permissions":["providers.call"]}','hash',1,1);
"#;
async fn setup() -> (DatabaseConnection, AiRunRepository, ModulePermissionRepository) {
    let db = crate::establish_connection("sqlite::memory:").await.unwrap();
    Migrator::up(&db,None).await.unwrap();
    db.execute_unprepared(MODULES).await.unwrap();
    db.execute_unprepared("INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) VALUES('alice','Alice','alice','x',1,1),('bob','Bob','bob','x',1,1);").await.unwrap();
    let repo = AiRunRepository::new(db.clone());
    let permissions = ModulePermissionRepository::new(db.clone());
    for user in ["alice","bob"] {
        permissions.grant(user,"mod",ModulePermission::ProvidersCall).await.unwrap();
        permissions.grant(user,"other",ModulePermission::ProvidersCall).await.unwrap();
    }
    (db,repo,permissions)
}
fn input(user: &str, module: &str, id: &str, epoch: i64) -> NewAiRun {
    NewAiRun { run_id:id.into(),user_id:user.into(),module_id:module.into(),module_version:"1".into(),
        module_integrity:"hash".into(),runtime_id:"rt".into(),request_id:format!("request-{id}"),
        authorization_epoch:epoch,provider_id:"provider".into(),model_id:"model".into(),provider_revision:4,
        skill_refs:vec![],tools:vec![],generation:GenerationParameters::default() }
}
fn terminal(status: AiRunStatus) -> AiTerminal {
    AiTerminal { status,content:Some("OUTPUT_SENTINEL".into()),error:None,result_unknown:false,
        audit_warning:false,total_tokens:Some(17) }
}
async fn scalar(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite,sql)).await.unwrap().unwrap()
        .try_get_by_index(0).unwrap()
}

#[tokio::test]
async fn revoke_grant_and_repeated_false_revoke_never_restore_old_admission() {
    let (db,repo,permissions) = setup().await;
    let epoch = repo.authorization_epoch("alice","mod").await.unwrap();
    repo.begin(&input("alice","mod","old",epoch)).await.unwrap();
    repo.admit_step("alice","mod",epoch,"old").await.unwrap();
    assert_eq!(scalar(&db,"SELECT dispatch_count FROM ai_run_bindings WHERE task_id='old'").await,1);
    permissions.revoke("alice","mod",ModulePermission::ProvidersCall).await.unwrap();
    let revoked = repo.authorization_epoch("alice","mod").await.unwrap();
    assert!(revoked > epoch);
    permissions.revoke("alice","mod",ModulePermission::ProvidersCall).await.unwrap();
    let twice = repo.authorization_epoch("alice","mod").await.unwrap();
    assert!(twice > revoked);
    permissions.grant("alice","mod",ModulePermission::ProvidersCall).await.unwrap();
    assert_eq!(repo.authorization_epoch("alice","mod").await.unwrap(),twice);
    assert!(matches!(repo.ensure_authorization_epoch("alice","mod",epoch).await,Err(DgosError::PermissionDenied)));
    assert!(repo.admit_step("alice","mod",epoch,"old").await.is_err());
    assert!(repo.begin(&input("alice","mod","stale",epoch)).await.is_err());
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM workbench_tasks WHERE id='stale'").await,0);
    assert_eq!(repo.authorization_epoch("bob","mod").await.unwrap(),0);
    assert_eq!(repo.authorization_epoch("alice","other").await.unwrap(),0);
    repo.begin(&input("alice","mod","fresh",twice)).await.unwrap();
    repo.admit_step("alice","mod",twice,"fresh").await.unwrap();
}

#[tokio::test]
async fn module_disable_version_switch_and_reinstall_retain_epoch_tombstones_and_history() {
    let (db,repo,_) = setup().await;
    let epoch = repo.authorization_epoch("alice","mod").await.unwrap();
    repo.begin(&input("alice","mod","run",epoch)).await.unwrap();
    db.execute_unprepared("UPDATE modules SET is_enabled=0 WHERE id='mod'").await.unwrap();
    assert!(repo.ensure_authorization_epoch("alice","mod",epoch).await.is_err());
    db.execute_unprepared("UPDATE modules SET is_enabled=1 WHERE id='mod'").await.unwrap();
    let disabled = repo.authorization_epoch("alice","mod").await.unwrap();
    assert!(disabled > epoch);
    db.execute_unprepared("UPDATE modules SET current_version='2' WHERE id='mod'; UPDATE modules SET current_version='1' WHERE id='mod';").await.unwrap();
    let switched = repo.authorization_epoch("alice","mod").await.unwrap();
    assert!(switched > disabled);
    assert!(repo.admit_step("alice","mod",epoch,"run").await.is_err());
    db.execute_unprepared("DELETE FROM module_permissions WHERE module_id='mod'; DELETE FROM modules WHERE id='mod';").await.unwrap();
    let tombstone = repo.authorization_epoch("alice","mod").await.unwrap();
    assert!(tombstone > switched);
    assert!(repo.get("alice","mod","run").await.unwrap().is_some());
    db.execute_unprepared("INSERT INTO modules(id,module_name,display_name,current_version,manifest,integrity,installed_at,updated_at) VALUES('mod','mod','M','1','{}','hash',1,1)").await.unwrap();
    assert_eq!(repo.authorization_epoch("alice","mod").await.unwrap(),tombstone);
    assert!(repo.ensure_authorization_epoch("alice","mod",epoch).await.is_err());
    assert!(repo.authorization_epoch("alice","missing").await.is_err());
}

#[tokio::test]
async fn undeclared_cleanup_delete_replace_and_overflow_are_fail_closed() {
    let (db,repo,permissions) = setup().await;
    let initial = repo.authorization_epoch("alice","mod").await.unwrap();
    use sea_orm::TransactionTrait;
    let tx = db.begin().await.unwrap();
    assert_eq!(permissions.revoke_undeclared_in(&tx,"mod",&serde_json::json!({"permissions":[]})).await.unwrap(),2);
    tx.commit().await.unwrap();
    let cleaned = repo.authorization_epoch("alice","mod").await.unwrap();
    assert!(cleaned > initial);
    db.execute_unprepared("DELETE FROM module_permissions WHERE user_id='alice' AND module_id='mod'").await.unwrap();
    let deleted = repo.authorization_epoch("alice","mod").await.unwrap();
    assert!(deleted > cleaned);
    db.execute_unprepared("INSERT INTO module_permissions(id,user_id,module_id,permission,granted) VALUES('replacement','alice','mod','providers.call',1)").await.unwrap();
    assert_eq!(repo.authorization_epoch("alice","mod").await.unwrap(),deleted);
    db.execute_unprepared("INSERT OR REPLACE INTO module_permissions(id,user_id,module_id,permission,granted) VALUES('replacement','alice','mod','providers.call',1)").await.unwrap();
    assert!(repo.authorization_epoch("alice","mod").await.unwrap() > deleted);
    db.execute_unprepared("UPDATE module_authorization_epochs SET epoch=9223372036854775807 WHERE user_id='alice' AND module_id='mod'").await.unwrap();
    assert!(permissions.revoke("alice","mod",ModulePermission::ProvidersCall).await.is_err());
    assert_eq!(scalar(&db,"SELECT epoch FROM module_authorization_epochs WHERE user_id='alice' AND module_id='mod'").await,i64::MAX);
}

#[tokio::test]
async fn lost_started_is_recovered_by_scoped_keyset_list_and_foreign_cursor_is_rejected() {
    let (_,repo,_) = setup().await;
    for (user,module,id) in [("alice","mod","a"),("alice","mod","b"),("alice","mod","c"),("bob","mod","foreign-user"),("alice","other","foreign-module")] {
        repo.begin(&input(user,module,id,0)).await.unwrap();
    }
    let page = repo.list("alice","mod",AiRunListInput { before:None,limit:Some(2) }).await.unwrap();
    assert_eq!(page.items.len(),2);
    let next = repo.list("alice","mod",AiRunListInput { before:page.next_before,limit:Some(2) }).await.unwrap();
    assert_eq!(next.items.len(),1);
    assert!(next.next_before.is_none());
    let ids: std::collections::BTreeSet<_> = page.items.into_iter().chain(next.items).map(|run|run.run_id).collect();
    assert_eq!(ids,std::collections::BTreeSet::from(["a".into(),"b".into(),"c".into()]));
    assert!(repo.get("bob","mod","a").await.unwrap().is_none());
    assert!(repo.get("alice","other","a").await.unwrap().is_none());
    for cursor in ["foreign-user","foreign-module","missing"] {
        assert!(matches!(repo.list("alice","mod",AiRunListInput { before:Some(cursor.into()),limit:Some(2) }).await,Err(DgosError::ModuleCapabilityInputInvalid)));
    }
    assert_eq!(repo.list("alice","mod",AiRunListInput{before:None,limit:Some(u32::MAX)}).await.unwrap().items.len(),3);
}

#[tokio::test]
async fn terminal_cas_preserves_failure_unknown_and_never_stores_output_or_guidance() {
    let (db,repo,_) = setup().await;
    let mut run = input("alice","mod","run",0);
    run.generation.instruction="GUIDANCE_SENTINEL".into();
    repo.begin(&run).await.unwrap();
    assert!(!repo.finish("bob","mod","run",&terminal(AiRunStatus::Succeeded)).await.unwrap());
    let mut failed = terminal(AiRunStatus::Cancelled);
    failed.error=Some(AiFailure::new("module",1004028,"cancelled",true));
    failed.audit_warning=true;
    let other = terminal(AiRunStatus::Succeeded);
    let (first,second) = tokio::join!(repo.finish("alice","mod","run",&failed),repo.finish("alice","mod","run",&other));
    assert_eq!(u8::from(first.unwrap())+u8::from(second.unwrap()),1);
    let stored = repo.get("alice","mod","run").await.unwrap().unwrap();
    if stored.status == AiRunStatus::Cancelled {
        assert!(stored.result_unknown);
        assert_eq!(stored.error.unwrap().reason,"cancelled");
        assert_eq!(scalar(&db,"SELECT audit_warning FROM ai_run_bindings WHERE task_id='run'").await,1);
        assert_eq!(scalar(&db,"SELECT COUNT(*) FROM workbench_tasks WHERE id='run' AND status='interrupted'").await,1);
    } else { assert_eq!(stored.status,AiRunStatus::Succeeded); }
    assert!(!repo.finish("alice","mod","run",&failed).await.unwrap());
    assert!(repo.admit_step("alice","mod",0,"run").await.is_err());
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM workbench_tasks WHERE generation_parameters LIKE '%SENTINEL%'").await,0);
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM pragma_table_info('ai_run_bindings') WHERE name IN ('prompt','content','output','instruction')").await,0);
    assert!(db.execute_unprepared("UPDATE ai_run_bindings SET user_id='bob' WHERE task_id='run'").await.is_err());
    assert!(db.execute_unprepared("UPDATE workbench_tasks SET app_id='other' WHERE id='run'").await.is_err());
    repo.begin(&input("alice","mod","cancelled",0)).await.unwrap();
    assert!(repo.finish("alice","mod","cancelled",&failed).await.unwrap());
    let cancelled=repo.get("alice","mod","cancelled").await.unwrap().unwrap();
    assert_eq!(cancelled.status,AiRunStatus::Cancelled);
    assert_eq!(cancelled.error.unwrap().code,1004028);
    assert!(cancelled.result_unknown);
}

#[tokio::test]
async fn begin_rolls_back_task_binding_and_refs_together_and_task_delete_cascades() {
    let (db,repo,_) = setup().await;
    db.execute_unprepared("INSERT INTO skill_versions(id,skill_id,version,content_hash,install_path,verification_status,file_manifest,installed_at,metadata) VALUES('v','skill','1','sha','/unused','verified','[]',1,'{}'); INSERT INTO skill_user_states(user_id,skill_id,version_id,enabled,created_at) VALUES('alice','skill','v',1,1)").await.unwrap();
    let mut run=input("alice","mod","run",0);
    run.skill_refs.push(AiSkillRef{skill_id:"skill".into(),version:"1".into(),content_hash:"sha".into()});
    run.tools.push(AiToolSelection{server_id:"s".into(),tool_name:"t".into(),schema_hash:"h".into()});
    repo.begin(&run).await.unwrap();
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM workbench_task_skills WHERE task_id='run'").await,1);
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM workbench_task_tools WHERE task_id='run'").await,1);
    let mut duplicate=input("alice","mod","duplicate",0);
    duplicate.request_id=run.request_id.clone();
    assert!(repo.begin(&duplicate).await.is_err());
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM workbench_tasks WHERE id='duplicate'").await,0);
    run.run_id="bad-pin".into(); run.request_id="bad-pin-request".into(); run.skill_refs[0].content_hash="changed".into();
    assert!(repo.begin(&run).await.is_err());
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM workbench_tasks WHERE id='bad-pin'").await,0);
    assert!(db.execute_unprepared("INSERT INTO ai_run_bindings(task_id,user_id,module_id,module_version,module_integrity,runtime_id,request_id,authorization_epoch,skill_refs,tool_refs) VALUES('absent','alice','mod','1','hash','rt','absent',0,'[]','[]')").await.is_err());
    assert!(scalar(&db,"SELECT COUNT(*) FROM pragma_foreign_key_list('ai_run_bindings') WHERE \"table\"='workbench_tasks' AND on_delete='CASCADE'").await > 0);
    db.execute_unprepared("DELETE FROM workbench_tasks WHERE id='run'").await.unwrap();
    for table in ["ai_run_bindings","workbench_task_skills","workbench_task_tools"] {
        assert_eq!(scalar(&db,&format!("SELECT COUNT(*) FROM {table}")).await,0);
    }
    assert_eq!(repo.authorization_epoch("alice","mod").await.unwrap(),0);
    assert!(!repo.finish("alice","mod","run",&terminal(AiRunStatus::Succeeded)).await.unwrap());
}

#[tokio::test]
async fn restart_and_legacy_recovery_converge_binding_without_replay() {
    let (db,repo,_) = setup().await;
    repo.begin(&input("alice","mod","restart",0)).await.unwrap();
    repo.admit_step("alice","mod",0,"restart").await.unwrap();
    assert_eq!(repo.recover_interrupted().await.unwrap(),1);
    assert_eq!(repo.recover_interrupted().await.unwrap(),0);
    let stored=repo.get("alice","mod","restart").await.unwrap().unwrap();
    assert_eq!(stored.status,AiRunStatus::Interrupted);
    assert!(stored.result_unknown);
    assert!(stored.error.is_none());
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM ai_run_bindings WHERE terminal_reason='process_interrupted' AND dispatch_count=1").await,1);
    repo.begin(&input("alice","mod","legacy",0)).await.unwrap();
    db.execute_unprepared("UPDATE workbench_tasks SET status='interrupted',finished_at=2 WHERE id='legacy' AND status='running'").await.unwrap();
    assert!(repo.get("alice","mod","legacy").await.unwrap().unwrap().result_unknown);
    assert!(!repo.finish("alice","mod","restart",&terminal(AiRunStatus::Succeeded)).await.unwrap());
}

#[tokio::test]
async fn deleting_account_purge_retry_and_late_workers_cannot_recreate_data() {
    let (db,repo,_) = setup().await;
    repo.begin(&input("alice","mod","alice-run",0)).await.unwrap();
    repo.begin(&input("bob","mod","bob-run",0)).await.unwrap();
    db.execute_unprepared("UPDATE users SET deleting_since=2 WHERE id='alice'").await.unwrap();
    assert!(repo.authorization_epoch("alice","mod").await.is_err());
    assert!(repo.admit_step("alice","mod",0,"alice-run").await.is_err());
    let data=UserDataRepository::new(db.clone());
    db.execute_unprepared("CREATE TRIGGER injected_purge_failure BEFORE DELETE ON ai_run_bindings WHEN OLD.user_id='alice' BEGIN SELECT RAISE(ABORT,'injected'); END").await.unwrap();
    assert!(data.purge(UserDataDomain::Workbench,"alice").await.is_err());
    assert!(repo.get("alice","mod","alice-run").await.unwrap().is_some());
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM users WHERE id='alice' AND deleting_since IS NOT NULL").await,1);
    db.execute_unprepared("DROP TRIGGER injected_purge_failure").await.unwrap();
    data.purge(UserDataDomain::Workbench,"alice").await.unwrap();
    data.purge(UserDataDomain::Modules,"alice").await.unwrap();
    assert_eq!(data.purge(UserDataDomain::Workbench,"alice").await.unwrap(),0);
    assert_eq!(data.purge(UserDataDomain::Modules,"alice").await.unwrap(),0);
    db.execute_unprepared("DELETE FROM users WHERE id='alice'").await.unwrap();
    assert!(!repo.finish("alice","mod","alice-run",&terminal(AiRunStatus::Succeeded)).await.unwrap());
    assert!(repo.begin(&input("alice","mod","late",0)).await.is_err());
    assert!(repo.ensure_authorization_epoch("alice","mod",0).await.is_err());
    assert!(repo.authorization_epoch("alice","mod").await.is_err());
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM module_authorization_epochs WHERE user_id='alice'").await,0);
    assert!(repo.get("alice","mod","alice-run").await.unwrap().is_none());
    assert!(repo.get("bob","mod","bob-run").await.unwrap().is_some());
    assert_eq!(data.orphan_count().await.unwrap(),0);
}

#[tokio::test]
async fn direct_capability_admission_shares_revocation_order_without_ai_task() {
    let (db,repo,permissions)=setup().await;
    permissions.grant("alice","mod",ModulePermission::McpCall).await.unwrap();
    let epoch=repo.authorization_epoch("alice","mod").await.unwrap();
    repo.admit_capability_step("alice","mod",epoch,ModulePermission::McpCall).await.unwrap();
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM workbench_tasks").await,0);
    permissions.revoke("alice","mod",ModulePermission::McpCall).await.unwrap();
    permissions.grant("alice","mod",ModulePermission::McpCall).await.unwrap();
    assert!(repo.admit_capability_step("alice","mod",epoch,ModulePermission::McpCall).await.is_err());
    let fresh=repo.authorization_epoch("alice","mod").await.unwrap();
    repo.admit_capability_step("alice","mod",fresh,ModulePermission::McpCall).await.unwrap();
    assert!(repo.admit_capability_step("alice","mod",fresh,ModulePermission::SkillsExecute).await.is_err());
    db.execute_unprepared("UPDATE modules SET manifest='{}' WHERE id='mod'").await.unwrap();
    let undeclared=repo.authorization_epoch("alice","mod").await.unwrap();
    assert!(repo.admit_capability_step("alice","mod",undeclared,ModulePermission::McpCall).await.is_err());
}

#[tokio::test]
async fn durable_started_loss_and_restart_survive_database_reopen() {
    let dir=tempfile::tempdir().unwrap();
    let url=format!("sqlite://{}?mode=rwc",dir.path().join("ai.sqlite").display());
    let db=crate::establish_connection(&url).await.unwrap();
    Migrator::up(&db,None).await.unwrap();
    db.execute_unprepared(MODULES).await.unwrap();
    db.execute_unprepared("INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) VALUES('alice','Alice','alice','x',1,1)").await.unwrap();
    let repo=AiRunRepository::new(db.clone());
    let epoch=repo.authorization_epoch("alice","mod").await.unwrap();
    repo.begin(&input("alice","mod","lost-started",epoch)).await.unwrap();
    repo.admit_step("alice","mod",epoch,"lost-started").await.unwrap();
    drop(repo);
    db.close().await.unwrap();
    let reopened=crate::establish_connection(&url).await.unwrap();
    let repo=AiRunRepository::new(reopened.clone());
    assert_eq!(repo.recover_interrupted().await.unwrap(),1);
    let page=repo.list("alice","mod",AiRunListInput::default()).await.unwrap();
    assert_eq!(page.items.len(),1);
    assert_eq!(page.items[0].run_id,"lost-started");
    assert_eq!(page.items[0].status,AiRunStatus::Interrupted);
    assert!(page.items[0].result_unknown);
    assert_eq!(scalar(&reopened,"SELECT dispatch_count FROM ai_run_bindings").await,1);
}

#[tokio::test]
async fn orphan_accounting_includes_bindings_and_epoch_tombstones() {
    let (db,repo,_)=setup().await;
    repo.begin(&input("alice","mod","orphan",0)).await.unwrap();
    db.execute_unprepared("DELETE FROM users WHERE id='alice'").await.unwrap();
    // Task, binding, two grants, two grant audits and two epoch tombstones.
    assert_eq!(UserDataRepository::new(db.clone()).orphan_count().await.unwrap(),8);
    assert!(repo.authorization_epoch("alice","mod").await.is_err());
    assert!(repo.admit_step("alice","mod",0,"orphan").await.is_err());
}

#[tokio::test]
async fn terminal_error_fields_and_composite_owner_binding_are_durable() {
    let (db,repo,_)=setup().await;
    repo.begin(&input("alice","mod","failure",0)).await.unwrap();
    let mut failed=terminal(AiRunStatus::Failed);
    failed.error=Some(AiFailure::new("provider",1001006,"timeout",true));
    failed.result_unknown=true;
    failed.audit_warning=true;
    assert!(repo.finish("alice","mod","failure",&failed).await.unwrap());
    let stored=repo.get("alice","mod","failure").await.unwrap().unwrap();
    assert_eq!(stored.status,AiRunStatus::Failed);
    let error=stored.error.unwrap();
    assert_eq!((error.domain.as_str(),error.code,error.reason.as_str()),("provider",1001006,"timeout"));
    assert!(stored.result_unknown);
    assert!(stored.finished_at.is_some());
    assert_eq!(scalar(&db,"SELECT audit_warning FROM ai_run_bindings WHERE task_id='failure'").await,1);
    assert_eq!(scalar(&db,"SELECT total_tokens FROM workbench_tasks WHERE id='failure'").await,17);
    db.execute_unprepared("INSERT INTO workbench_tasks(id,user_id,app_id,provider_id,model_id,status,created_at) VALUES('unbound','alice','mod','p','m','running',1)").await.unwrap();
    assert!(db.execute_unprepared("INSERT INTO ai_run_bindings(task_id,user_id,module_id,module_version,module_integrity,runtime_id,request_id,authorization_epoch,skill_refs,tool_refs) VALUES('unbound','bob','mod','1','hash','rt','foreign-owner',0,'[]','[]')").await.is_err());
    assert_eq!(scalar(&db,"SELECT COUNT(*) FROM pragma_foreign_key_list('ai_run_bindings') WHERE \"from\"='user_id' AND \"to\"='user_id' AND on_delete='CASCADE'").await,1);
}
