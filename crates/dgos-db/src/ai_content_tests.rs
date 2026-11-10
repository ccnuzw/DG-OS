//! Consumer-facing foundation persistence tests. No Provider or application mocks.
use crate::{AiContentRepository, AiRunRepository, ChatRepository, NewAiRun,
    UserDataDomain, UserDataRepository};
use crate::migrations::Migrator;
use dgos_types::{AiContextReceipt, AiRunStatus, AiTerminal, GenerationParameters};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;
use serde_json::json;

const SCOPE: &str = "module:mod";
async fn setup() -> (DatabaseConnection, AiContentRepository) {
    let db=crate::establish_connection("sqlite::memory:").await.unwrap();
    seed(&db).await;
    let repo=AiRunRepository::new(db.clone()).content_repository();
    (db,repo)
}
async fn seed(db: &DatabaseConnection) {
    Migrator::up(db,None).await.unwrap();
    db.execute_unprepared("INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) VALUES('alice','Alice','alice','x',1,1),('bob','Bob','bob','x',1,1); \
        INSERT INTO modules(id,module_name,display_name,current_version,manifest,integrity,installed_at,updated_at) VALUES('mod','mod','Module','1','{}','hash',1,1); \
        INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES('session','alice','chat',1,1);")
        .await.unwrap();
}
async fn count(db: &DatabaseConnection,sql: &str) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite,sql)).await.unwrap().unwrap().try_get_by_index(0).unwrap()
}
async fn start(repo: &AiContentRepository,id: &str,retain: bool) {
    repo.start_run("alice",SCOPE,id,retain,None).await.unwrap();
}
async fn finish(repo: &AiContentRepository,id: &str) {
    repo.finish_run("alice",SCOPE,id,Some("retained result"),"succeeded",true,false).await.unwrap();
}

#[tokio::test]
async fn immutable_resources_bind_live_owner_scope_hash_and_fixed_lease() {
    let (db,repo)=setup().await;
    let r=repo.import_resource("alice",SCOPE,"note.md","text/markdown",b"abc").await.unwrap();
    assert_eq!(r.sha256,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(r.bytes_len,3);
    assert!(r.expires_at>chrono::Utc::now().timestamp()+86_390);
    assert_eq!(repo.resource_metadata("alice",SCOPE,&r.id).await.unwrap().unwrap().sha256,r.sha256);
    assert_eq!(repo.resource("alice",SCOPE,&r.id).await.unwrap().unwrap().1,b"abc");
    assert!(repo.resource("bob",SCOPE,&r.id).await.unwrap().is_none());
    assert!(repo.resource("alice","module:other",&r.id).await.unwrap().is_none());
    assert!(!repo.delete_resource("bob",SCOPE,&r.id).await.unwrap());
    assert!(repo.delete_resource("alice",SCOPE,&r.id).await.unwrap());
    assert!(repo.resource_metadata("alice",SCOPE,&r.id).await.unwrap().is_none());
    assert!(!repo.delete_resource("alice",SCOPE,&r.id).await.unwrap());
    db.execute_unprepared("UPDATE users SET deleting_since=2 WHERE id='alice'").await.unwrap();
    assert!(repo.import_resource("alice",SCOPE,"n","text/plain",b"x").await.is_err());
    assert!(repo.resource_metadata("alice",SCOPE,&r.id).await.is_err());
    assert!(repo.start_run("missing",SCOPE,"run",true,None).await.is_err());
}

#[tokio::test]
async fn resource_validation_and_quota_include_expiry_without_cross_scope_charge() {
    let (db,repo)=setup().await;
    for (mime,bytes) in [("text/html",b"x".as_slice()),("text/plain",b"\xff".as_slice()),("image/png",b"fake".as_slice()),("image/jpeg",b"fake".as_slice()),("image/webp",b"fake".as_slice())] {
        assert!(repo.import_resource("alice",SCOPE,"n",mime,bytes).await.is_err());
    }
    assert!(repo.import_resource("alice",SCOPE,"bad\nname","text/plain",b"x").await.is_err());
    assert!(repo.import_resource("alice",SCOPE,"n","text/plain",&vec![b'a';1024*1024+1]).await.is_err());
    let mut png=vec![0;8*1024*1024];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    for i in 0..3 { repo.import_resource("alice",SCOPE,&format!("{i}.png"),"image/png",&png).await.unwrap(); }
    assert!(repo.import_resource("alice",SCOPE,"extra","text/plain",b"x").await.is_err());
    repo.import_resource("bob",SCOPE,"other-owner","text/plain",b"x").await.unwrap();
    repo.import_resource("alice","module:other","other-scope","text/plain",b"x").await.unwrap();
    db.execute_unprepared("UPDATE ai_content_resources SET expires_at=0 WHERE user_id='alice' AND scope='module:mod'").await.unwrap();
    repo.import_resource("alice",SCOPE,"after-expiry","text/plain",b"x").await.unwrap();
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_resources WHERE user_id='alice' AND scope='module:mod'").await,1);
}

#[tokio::test]
async fn resource_names_use_the_public_unicode_character_limit() {
    let (_,repo)=setup().await;
    let name="资".repeat(255);
    let resource=repo.import_resource("alice",SCOPE,&name,"text/plain",b"x").await.unwrap();
    assert_eq!(resource.name,name);
    assert_eq!(repo.resource_metadata("alice",SCOPE,&resource.id).await.unwrap().unwrap().name,name);
    assert!(repo.import_resource("alice",SCOPE,&"资".repeat(256),"text/plain",b"x").await.is_err());
}

#[tokio::test]
async fn resource_expiry_is_fail_closed_before_cleanup() {
    let (db,repo)=setup().await;
    let r=repo.import_resource("alice",SCOPE,"n","text/plain",b"x").await.unwrap();
    db.execute_unprepared("UPDATE ai_content_resources SET expires_at=0").await.unwrap();
    assert!(repo.resource("alice",SCOPE,&r.id).await.unwrap().is_none());
    assert!(repo.resource_metadata("alice",SCOPE,&r.id).await.unwrap().is_none());
    repo.recover_interrupted().await.unwrap();
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_resources").await,0);
}

#[tokio::test]
async fn opt_out_never_persists_checkpoints_or_results_but_keeps_run_fact() {
    let (db,repo)=setup().await;
    start(&repo,"private",false).await;
    repo.checkpoint("alice",SCOPE,"private","PRIVATE_CHECKPOINT",&[],&json!([])).await.unwrap();
    repo.finish_run("alice",SCOPE,"private",Some("PRIVATE_RESULT"),"succeeded",true,false).await.unwrap();
    let got=repo.get_run("alice",SCOPE,"private").await.unwrap().unwrap();
    assert!(got.content.is_none());
    assert_eq!(got.status,"succeeded");
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_runs WHERE content IS NOT NULL").await,0);
    assert!(repo.continuation("alice",SCOPE,"private").await.is_err());
    assert!(repo.start_run("alice",SCOPE,"private",true,None).await.is_err());
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_run_bindings").await,0);
}

#[tokio::test]
async fn checkpoints_are_bounded_and_terminal_is_immutable() {
    let (_,repo)=setup().await;
    start(&repo,"r",true).await;
    let receipt=AiContextReceipt { resource_id:"resource".into(),sha256:"a".repeat(64),start:1,bytes:3,total_bytes:4 };
    repo.checkpoint("alice",SCOPE,"r","progress",&[receipt],&json!([{"skill_id":"s","version":"1"}])).await.unwrap();
    let got=repo.get_run("alice",SCOPE,"r").await.unwrap().unwrap();
    assert_eq!(got.content.as_deref(),Some("progress"));
    assert_eq!(got.context_receipts[0].bytes,3);
    assert!(repo.checkpoint("alice",SCOPE,"r",&"é".repeat(32769),&[],&json!([])).await.is_err());
    assert!(repo.checkpoint("alice",SCOPE,"r","x",&[],&json!("x".repeat(16384))).await.is_err());
    finish(&repo,"r").await;
    assert!(repo.checkpoint("alice",SCOPE,"r","overwrite",&[],&json!([])).await.is_err());
    assert!(repo.finish_run("alice",SCOPE,"r",Some("overwrite"),"failed",false,false).await.is_err());
    assert_eq!(repo.get_run("alice",SCOPE,"r").await.unwrap().unwrap().content.as_deref(),Some("retained result"));
}

#[tokio::test]
async fn acknowledgement_is_idempotent_owned_and_does_not_delete_content() {
    let (_,repo)=setup().await;
    start(&repo,"r",true).await;
    finish(&repo,"r").await;
    assert!(!repo.acknowledge("bob",SCOPE,"r").await.unwrap());
    assert!(!repo.acknowledge("alice","module:other","r").await.unwrap());
    assert!(!repo.acknowledge("alice",SCOPE,"absent").await.unwrap());
    assert!(repo.acknowledge("alice",SCOPE,"r").await.unwrap());
    let first=repo.get_run("alice",SCOPE,"r").await.unwrap().unwrap();
    assert!(first.acknowledged_at.is_some());
    assert!(repo.acknowledge("alice",SCOPE,"r").await.unwrap());
    let second=repo.get_run("alice",SCOPE,"r").await.unwrap().unwrap();
    assert_eq!(first.acknowledged_at,second.acknowledged_at);
    assert_eq!(first.content,second.content);
}

#[tokio::test]
async fn expiry_and_body_eviction_keep_metadata_tombstones_and_cannot_resurrect() {
    let (db,repo)=setup().await;
    for i in 0..33 {
        let id=format!("r{i}"); start(&repo,&id,true).await; finish(&repo,&id).await;
    }
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_runs").await,33);
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_runs WHERE content IS NOT NULL").await,32);
    let old=repo.get_run("alice",SCOPE,"r0").await.unwrap().unwrap();
    assert_eq!(old.status,"expired"); assert!(old.content.is_none());
    assert!(repo.start_run("alice",SCOPE,"r0",true,None).await.is_err());
    assert!(repo.continuation("alice",SCOPE,"r0").await.is_err());
    db.execute_unprepared("UPDATE ai_content_runs SET expires_at=0 WHERE run_id='r32'").await.unwrap();
    let expired=repo.get_run("alice",SCOPE,"r32").await.unwrap().unwrap();
    assert_eq!(expired.status,"expired"); assert!(expired.content.is_none());
    assert!(expired.output_valid);
    assert!(repo.acknowledge("alice",SCOPE,"r32").await.unwrap());
    repo.recover_interrupted().await.unwrap();
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_runs WHERE run_id='r32' AND content IS NULL").await,1);
}

#[tokio::test]
async fn evicted_running_checkpoint_cannot_regain_a_body() {
    let (_,repo)=setup().await;
    start(&repo,"old",true).await;
    repo.checkpoint("alice",SCOPE,"old","partial",&[],&json!([])).await.unwrap();
    for i in 0..32 { let id=format!("new{i}");start(&repo,&id,true).await;finish(&repo,&id).await; }
    repo.checkpoint("alice",SCOPE,"old","later",&[],&json!([])).await.unwrap();
    finish(&repo,"old").await;
    assert_eq!(repo.get_run("alice",SCOPE,"old").await.unwrap().unwrap().status,"expired");
}

#[tokio::test]
async fn continuation_inherits_fingerprints_and_allows_new_effect_with_reused_provider_call_id() {
    let (_,repo)=setup().await;
    start(&repo,"source",true).await;
    repo.tool_dispatched("alice",SCOPE,"source","call1",&"a".repeat(64)).await.unwrap();
    assert!(repo.tool_settled("alice",SCOPE,"source","missing","succeeded",false).await.is_err());
    repo.tool_settled("alice",SCOPE,"source","call1","succeeded",false).await.unwrap();
    assert!(repo.tool_settled("alice",SCOPE,"source","call1","failed",false).await.is_err());
    finish(&repo,"source").await;
    let source=repo.continuation("alice",SCOPE,"source").await.unwrap();
    assert_eq!(source.tool_receipts[0]["fingerprint"],"a".repeat(64));
    assert!(repo.start_run("bob",SCOPE,"foreign",true,Some("source")).await.is_err());
    assert!(repo.start_run("alice","module:other","foreign-scope",true,Some("source")).await.is_err());
    repo.start_run("alice",SCOPE,"next",true,Some("source")).await.unwrap();
    assert!(repo.tool_dispatched("alice",SCOPE,"next","different-id",&"A".repeat(64)).await.is_err());
    repo.tool_dispatched("alice",SCOPE,"next","call1",&"b".repeat(64)).await.unwrap();
    repo.tool_settled("alice",SCOPE,"next","call1","succeeded",false).await.unwrap();
    finish(&repo,"next").await;
    let next=repo.continuation("alice",SCOPE,"next").await.unwrap();
    assert_eq!(next.source_run_id.as_deref(),Some("source"));
    assert_eq!(next.tool_receipts.as_array().unwrap().len(),2);
    assert_eq!(next.tool_receipts.as_array().unwrap().iter().filter(|r|r["inherited"]==true).count(),1);
    repo.start_run("alice",SCOPE,"third",true,Some("next")).await.unwrap();
    assert!(repo.tool_dispatched("alice",SCOPE,"third","new-id",&"b".repeat(64)).await.is_err());
}

#[tokio::test]
async fn unresolved_unknown_and_audit_warning_prevent_continuation_and_false_success() {
    let (_,repo)=setup().await;
    for (id,unknown,warning) in [("unresolved",false,false),("unknown",true,false),("warning",false,true)] {
        start(&repo,id,true).await;
        repo.checkpoint("alice",SCOPE,id,"partial",&[],&json!([])).await.unwrap();
        if !warning {
            repo.tool_dispatched("alice",SCOPE,id,"call",&"c".repeat(64)).await.unwrap();
            if unknown { repo.tool_settled("alice",SCOPE,id,"call","unknown",true).await.unwrap(); }
        }
        assert!(repo.finish_run("alice",SCOPE,id,Some("result"),"succeeded",true,warning).await.is_err());
        repo.finish_run("alice",SCOPE,id,None,"failed",false,warning).await.unwrap();
        assert!(repo.continuation("alice",SCOPE,id).await.is_err());
        assert!(repo.start_run("alice",SCOPE,&format!("from-{id}"),true,Some(id)).await.is_err());
    }
}

#[tokio::test]
async fn receipt_limits_and_content_free_outcomes_are_enforced() {
    let (_,repo)=setup().await;
    start(&repo,"r",false).await;
    assert!(repo.tool_dispatched("alice",SCOPE,"r","call","raw-arguments").await.is_err());
    for i in 0..128 { repo.tool_dispatched("alice",SCOPE,"r",&format!("call{i}"),&format!("{i:064x}")).await.unwrap(); }
    assert!(repo.tool_dispatched("alice",SCOPE,"r","overflow",&format!("{:064x}",128)).await.is_err());
    assert!(repo.tool_settled("alice",SCOPE,"r","call0","RAW_TOOL_OUTPUT",false).await.is_err());
    assert!(repo.tool_settled("alice",SCOPE,"r","call0","unknown",false).await.is_err());
}

#[tokio::test]
async fn crash_reopen_marks_running_interrupted_and_unsettled_unknown_without_replay() {
    let dir=tempfile::tempdir().unwrap();
    let url=format!("sqlite://{}?mode=rwc",dir.path().join("content.db").display());
    let db=crate::establish_connection(&url).await.unwrap();seed(&db).await;
    let repo=AiContentRepository::new(db.clone());start(&repo,"crashed",true).await;
    repo.checkpoint("alice",SCOPE,"crashed","partial",&[],&json!([])).await.unwrap();
    repo.tool_dispatched("alice",SCOPE,"crashed","call",&"d".repeat(64)).await.unwrap();
    drop(repo);db.close().await.unwrap();
    let db=crate::establish_connection(&url).await.unwrap();
    let repo=AiContentRepository::new(db.clone());
    assert_eq!(repo.recover_interrupted().await.unwrap(),1);
    assert_eq!(repo.recover_interrupted().await.unwrap(),0);
    let got=repo.get_run("alice",SCOPE,"crashed").await.unwrap().unwrap();
    assert_eq!(got.status,"interrupted");assert_eq!(got.content.as_deref(),Some("partial"));
    assert_eq!(got.tool_receipts[0]["outcome"],"unknown");assert_eq!(got.tool_receipts[0]["unknown"],true);
    assert!(repo.continuation("alice",SCOPE,"crashed").await.is_err());
    assert!(repo.tool_settled("alice",SCOPE,"crashed","call","succeeded",false).await.is_err());
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_tool_receipts").await,1);
}

#[tokio::test]
async fn chat_content_needs_no_module_task_and_session_delete_cleans_only_its_scope() {
    let (db,repo)=setup().await;
    let r=repo.import_resource("alice","chat:session","n","text/plain",b"x").await.unwrap();
    repo.start_run("alice","chat:session","chat-run",true,None).await.unwrap();
    repo.finish_run("alice","chat:session","chat-run",Some("chat"),"succeeded",true,false).await.unwrap();
    assert_eq!(count(&db,"SELECT COUNT(*) FROM workbench_tasks").await,0);
    assert!(repo.resource("bob","chat:session",&r.id).await.is_err());
    assert!(repo.start_run("alice","chat:missing","missing-session",true,None).await.is_err());
    start(&repo,"module-run",true).await;
    ChatRepository::new(db.clone()).delete_session("alice","session").await.unwrap();
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_resources WHERE scope='chat:session'").await,0);
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_runs WHERE scope='chat:session'").await,0);
    assert!(repo.get_run("alice",SCOPE,"module-run").await.unwrap().is_some());
    assert!(repo.resource_metadata("alice","chat:session",&r.id).await.is_err());
}

#[tokio::test]
async fn account_purge_and_account_cascade_clear_all_content_and_receipts() {
    let (db,repo)=setup().await;
    start(&repo,"r",true).await;
    repo.tool_dispatched("alice",SCOPE,"r","call",&"e".repeat(64)).await.unwrap();
    repo.import_resource("alice",SCOPE,"n","text/plain",b"x").await.unwrap();
    repo.start_run("bob",SCOPE,"bob-run",true,None).await.unwrap();
    UserDataRepository::new(db.clone()).purge(UserDataDomain::Workbench,"alice").await.unwrap();
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_runs WHERE user_id='alice'").await,0);
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_resources WHERE user_id='alice'").await,0);
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_tool_receipts").await,0);
    assert!(repo.get_run("bob",SCOPE,"bob-run").await.unwrap().is_some());
    db.execute_unprepared("DELETE FROM users WHERE id='bob'").await.unwrap();
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_content_runs").await,0);
}

#[tokio::test]
async fn content_failure_rolls_back_both_audit_and_task_terminal() {
    let (db,content)=setup().await;
    let audit=AiRunRepository::new(db.clone());
    let epoch=audit.authorization_epoch("alice","mod").await.unwrap();
    audit.begin(&NewAiRun { run_id:"r".into(),user_id:"alice".into(),module_id:"mod".into(),
        module_version:"1".into(),module_integrity:"hash".into(),runtime_id:"rt".into(),request_id:"request".into(),
        authorization_epoch:epoch,provider_id:"p".into(),model_id:"m".into(),provider_revision:1,
        skill_refs:vec![],tools:vec![],generation:GenerationParameters::default() }).await.unwrap();
    let mut terminal=AiTerminal { status:AiRunStatus::Succeeded,content:Some("result".into()),error:None,
        result_unknown:false,audit_warning:false,total_tokens:Some(1) };
    assert!(audit.finish_with_content("alice","mod","r",&terminal,true).await.is_err());
    assert_eq!(audit.get("alice","mod","r").await.unwrap().unwrap().status,AiRunStatus::Running);
    start(&content,"r",true).await;
    db.execute_unprepared("CREATE TRIGGER reject_ai_body BEFORE UPDATE OF content ON ai_content_runs BEGIN SELECT RAISE(ABORT,'storage unavailable'); END;").await.unwrap();
    assert!(audit.finish_with_content("alice","mod","r",&terminal,true).await.is_err());
    assert_eq!(audit.get("alice","mod","r").await.unwrap().unwrap().status,AiRunStatus::Running);
    assert_eq!(content.get_run("alice",SCOPE,"r").await.unwrap().unwrap().status,"running");
    db.execute_unprepared("DROP TRIGGER reject_ai_body").await.unwrap();
    terminal.content=Some("x".repeat(65537));
    assert!(audit.finish_with_content("alice","mod","r",&terminal,true).await.is_err());
    assert_eq!(audit.get("alice","mod","r").await.unwrap().unwrap().status,AiRunStatus::Running);
    terminal.content=Some("RESULT_CONTENT_SENTINEL".into());
    assert!(audit.finish_with_content("alice","mod","r",&terminal,true).await.unwrap());
    assert!(!audit.finish_with_content("alice","mod","r",&terminal,true).await.unwrap());
    assert_eq!(content.get_run("alice",SCOPE,"r").await.unwrap().unwrap().content,terminal.content);
    let row=db.query_one(Statement::from_string(DbBackend::Sqlite,"SELECT * FROM ai_run_bindings WHERE task_id='r'"))
        .await.unwrap().unwrap();
    assert!(!format!("{row:?}").contains("RESULT_CONTENT_SENTINEL"));
}

#[tokio::test]
async fn current_epoch_is_read_only_missing_disabled_deleting_and_overflow_fail_closed() {
    let (db,_)=setup().await;
    let repo=AiRunRepository::new(db.clone());
    assert!(repo.current_authorization_epoch("alice","mod").await.is_err());
    assert_eq!(count(&db,"SELECT COUNT(*) FROM module_authorization_epochs").await,0);
    let epoch=repo.authorization_epoch("alice","mod").await.unwrap();
    assert_eq!(repo.current_authorization_epoch("alice","mod").await.unwrap(),epoch);
    db.execute_unprepared("UPDATE modules SET is_enabled=0 WHERE id='mod'").await.unwrap();
    assert!(repo.current_authorization_epoch("alice","mod").await.is_err());
    db.execute_unprepared("UPDATE modules SET is_enabled=1 WHERE id='mod'; UPDATE module_authorization_epochs SET epoch=9223372036854775807").await.unwrap();
    assert!(repo.current_authorization_epoch("alice","mod").await.is_err());
    db.execute_unprepared("UPDATE module_authorization_epochs SET epoch=0; UPDATE users SET deleting_since=1 WHERE id='alice'").await.unwrap();
    assert!(repo.current_authorization_epoch("alice","mod").await.is_err());
    assert_eq!(count(&db,"SELECT COUNT(*) FROM ai_run_bindings").await,0);
}

fn context_receipt(id: &str) -> AiContextReceipt {
    AiContextReceipt { resource_id:id.into(),sha256:"a".repeat(64),start:0,bytes:1,total_bytes:1 }
}

#[tokio::test]
async fn ancestral_context_metadata_survives_expired_bodies_without_becoming_current_delivery() {
    let (db,repo)=setup().await;
    let mut previous=None;
    for i in 0..3 {
        let id=format!("generation{i}");
        repo.start_run("alice",SCOPE,&id,true,previous.as_deref()).await.unwrap();
        repo.checkpoint("alice",SCOPE,&id,"progress",&[context_receipt(&format!("resource{i}"))],&json!([])).await.unwrap();
        finish(&repo,&id).await;
        previous=Some(id);
    }
    db.execute_unprepared("UPDATE ai_content_runs SET expires_at=0,content=NULL,content_expired=1 WHERE run_id='generation0'").await.unwrap();
    let receipts=repo.continuation_context_receipts("alice",SCOPE,"generation2").await.unwrap();
    assert_eq!(receipts.iter().map(|r|r.resource_id.as_str()).collect::<Vec<_>>(),vec!["resource2","resource1","resource0"]);
    let actual=repo.get_run("alice",SCOPE,"generation2").await.unwrap().unwrap();
    assert_eq!(actual.context_receipts.len(),1);
    assert_eq!(actual.context_receipts[0].resource_id,"resource2");
    assert_eq!(repo.get_run("alice",SCOPE,"generation0").await.unwrap().unwrap().status,"expired");
    assert!(repo.continuation_context_receipts("bob",SCOPE,"generation2").await.is_err());
    assert!(repo.continuation_context_receipts("alice","module:other","generation2").await.is_err());
}

#[tokio::test]
async fn ancestral_context_metadata_rejects_missing_cross_owner_cross_scope_and_cyclic_sources() {
    let (db,repo)=setup().await;
    start(&repo,"child",true).await;
    repo.start_run("alice","module:other","other-scope",true,None).await.unwrap();
    repo.start_run("bob",SCOPE,"other-owner",true,None).await.unwrap();
    for source in ["absent","other-scope","other-owner","child"] {
        db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "UPDATE ai_content_runs SET source_run_id=?1 WHERE run_id='child'",[source.into()])).await.unwrap();
        assert!(repo.continuation_context_receipts("alice",SCOPE,"child").await.is_err());
    }
}

#[tokio::test]
async fn ancestral_context_chain_accepts_32_nodes_and_rejects_33() {
    let (_,repo)=setup().await;
    let mut previous=None;
    for i in 0..33 {
        let id=format!("chain{i}");
        repo.start_run("alice",SCOPE,&id,true,previous.as_deref()).await.unwrap();
        finish(&repo,&id).await;
        previous=Some(id);
    }
    assert!(repo.continuation_context_receipts("alice",SCOPE,"chain31").await.unwrap().is_empty());
    assert!(repo.continuation_context_receipts("alice",SCOPE,"chain32").await.is_err());
}

#[tokio::test]
async fn ancestral_context_receipts_accept_128_and_reject_129_across_generations() {
    let (_,repo)=setup().await;
    let receipts:Vec<_>=(0..64).map(|i|context_receipt(&format!("resource{i}"))).collect();
    start(&repo,"root",true).await;
    repo.checkpoint("alice",SCOPE,"root","progress",&receipts,&json!([])).await.unwrap();
    finish(&repo,"root").await;
    repo.start_run("alice",SCOPE,"middle",true,Some("root")).await.unwrap();
    repo.checkpoint("alice",SCOPE,"middle","progress",&receipts,&json!([])).await.unwrap();
    finish(&repo,"middle").await;
    assert_eq!(repo.continuation_context_receipts("alice",SCOPE,"middle").await.unwrap().len(),128);
    repo.start_run("alice",SCOPE,"leaf",true,Some("middle")).await.unwrap();
    repo.checkpoint("alice",SCOPE,"leaf","progress",&[context_receipt("additional")],&json!([])).await.unwrap();
    finish(&repo,"leaf").await;
    assert!(repo.continuation_context_receipts("alice",SCOPE,"leaf").await.is_err());
}
