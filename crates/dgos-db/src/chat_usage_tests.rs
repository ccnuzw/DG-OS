use crate::migrations::Migrator;
use crate::{ChatRepository, ChatStoreError, ChatUsageRepository, ProviderRepository, UserDataDomain, UserDataRepository};
use dgos_types::TokenUsage;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::MigratorTrait;

async fn memory() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

async fn execute(db: &impl ConnectionTrait, sql: &str, values: Vec<sea_orm::Value>) {
    db.execute(Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)).await.unwrap();
}

async fn count(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(DbBackend::Sqlite, sql)).await.unwrap().unwrap()
        .try_get_by_index(0).unwrap()
}

async fn session(db: &DatabaseConnection, user: &str, id: &str) {
    execute(db, "INSERT INTO chat_sessions(id,user_id,title,created_at,updated_at) VALUES(?,?,'fixture',1,1)",
        vec![id.into(), user.into()]).await;
}

async fn run(db: &DatabaseConnection, user: &str, session: &str, id: &str) {
    execute(db, "INSERT INTO chat_runs(task_id,user_id,session_id,assistant_message_id,created_at,updated_at) VALUES(?,?,?,'answer',1,1)",
        vec![id.into(), user.into(), session.into()]).await;
}

async fn provider(db: &DatabaseConnection) {
    db.execute_unprepared("INSERT INTO providers(id,name,provider_type,base_url,api_key,user_id,created_at,updated_at) VALUES('provider','fixture','openai','https://example.invalid','','alice','2026-11-11 00:00:00','2026-11-11 00:00:00')").await.unwrap();
}

fn usage(input: u32, output: u32, cache: Option<u32>) -> TokenUsage {
    TokenUsage {
        prompt_tokens: input,
        completion_tokens: output,
        total_tokens: input.checked_add(output).unwrap(),
        prompt_tokens_reported: true,
        completion_tokens_reported: true,
        total_tokens_reported: true,
        cache_read_tokens: cache,
        ..Default::default()
    }
}

async fn settle(db: &DatabaseConnection, id: &str, usage: Option<&TokenUsage>, duration: Option<i64>, outcome: &str) -> bool {
    let tx = db.begin().await.unwrap();
    let changed = ChatUsageRepository::settle_in_transaction(&tx, id, usage, duration, outcome).await.unwrap();
    tx.commit().await.unwrap();
    changed
}

#[tokio::test]
async fn empty_unknown_and_explicit_zero_are_distinct_with_per_field_coverage() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    let repo = ChatUsageRepository::new(db.clone());
    let empty = repo.summary("alice", "session").await.unwrap();
    assert_eq!(empty.total_tokens, Some(0));
    assert_eq!(empty.input_tokens, None);
    assert_eq!(empty.duration_ms, None);
    assert_eq!(empty.call_count, 0);
    assert!(!empty.partial);
    assert_eq!(empty.latest, None);

    let first = repo.begin("alice", "provider", "m1", "run").await.unwrap().unwrap();
    let pending = repo.summary("alice", "session").await.unwrap();
    assert_eq!(pending.total_tokens, None);
    assert_eq!((pending.call_count, pending.reported_count, pending.pending_count), (1, 0, 1));
    assert_eq!(pending.latest, None);
    assert!(pending.partial);
    assert!(settle(&db, &first, Some(&usage(0, 0, Some(0))), Some(0), "success").await);
    let zero = repo.summary("alice", "session").await.unwrap();
    assert_eq!(zero.total_tokens, Some(0));
    assert_eq!(zero.input_tokens, Some(0));
    assert_eq!(zero.cache_read_tokens, Some(0));
    assert_eq!(zero.duration_ms, Some(0));
    assert_eq!(zero.cache_hit_rate, None, "a zero denominator has no rate");
    assert!(!zero.partial);

    let second = repo.begin("alice", "provider", "m2", "run").await.unwrap().unwrap();
    let absent = TokenUsage { prompt_tokens: 900, completion_tokens: 100, total_tokens: 1000, ..Default::default() };
    assert!(settle(&db, &second, Some(&absent), Some(8), "failure").await);
    let third = repo.begin("alice", "provider", "m2", "run").await.unwrap().unwrap();
    let partial = TokenUsage { prompt_tokens: 100, prompt_tokens_reported: true, cache_read_tokens: Some(50), ..Default::default() };
    assert!(settle(&db, &third, Some(&partial), None, "interrupted").await);
    let summary = repo.summary("alice", "session").await.unwrap();
    assert_eq!((summary.call_count, summary.reported_count, summary.pending_count), (3, 2, 0));
    assert_eq!(summary.total_tokens, Some(0));
    assert_eq!(summary.input_tokens, Some(100));
    assert_eq!(summary.output_tokens, Some(0));
    assert_eq!(summary.cache_read_tokens, Some(50));
    assert_eq!(summary.duration_ms, Some(8));
    assert_eq!(summary.cache_hit_rate, None, "partial cache coverage cannot use all input");
    assert_eq!((summary.coverage.total, summary.coverage.input, summary.coverage.output), (1, 2, 1));
    assert_eq!((summary.coverage.cache_read, summary.coverage.duration), (2, 2));
    assert_eq!((summary.coverage.cache_write, summary.coverage.reasoning), (0, 0));
    assert!(summary.partial);
    assert!(!summary.legacy);
}

#[tokio::test]
async fn aggregate_is_uncapped_wide_and_cache_rate_uses_matching_full_scope() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    let repo = ChatUsageRepository::new(db.clone());
    let facts = usage(4_000_000_000, 0, Some(2_000_000_000));
    for _ in 0..151 {
        let id = repo.begin("alice", "deleted-provider", "any-model", "run").await.unwrap().unwrap();
        assert!(settle(&db, &id, Some(&facts), Some(7), "success").await);
    }
    let summary = repo.summary("alice", "session").await.unwrap();
    assert_eq!((summary.call_count, summary.reported_count), (151, 151));
    assert_eq!(summary.total_tokens, Some(604_000_000_000));
    assert_eq!(summary.input_tokens, Some(604_000_000_000));
    assert_eq!(summary.cache_read_tokens, Some(302_000_000_000));
    assert_eq!(summary.duration_ms, Some(1057));
    assert_eq!(summary.cache_hit_rate, Some(0.5));
    assert!(!summary.partial);
    assert_eq!(summary.coverage.total, 151);
}

#[tokio::test]
async fn owner_run_isolation_and_missing_session_are_indistinguishable() {
    let db = memory().await;
    session(&db, "alice", "alice-session").await;
    session(&db, "bob", "bob-session").await;
    run(&db, "alice", "alice-session", "alice-run").await;
    let repo = ChatUsageRepository::new(db.clone());
    assert_eq!(repo.begin("bob", "p", "m", "alice-run").await.unwrap(), None);
    assert_eq!(repo.begin("alice", "p", "m", "not-a-chat-run").await.unwrap(), None);
    for id in ["alice-session", "missing"] {
        assert_eq!(repo.summary("bob", id).await.unwrap_err(), ChatStoreError::SessionNotFound);
    }
    let cross = db.execute_unprepared("INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,created_at) VALUES('cross','bob','bob-session','alice-run','p','m','measured','pending',1)").await;
    assert!(cross.is_err());
    let id = repo.begin("alice", "p", "m", "alice-run").await.unwrap().unwrap();
    assert!(settle(&db, &id, Some(&usage(3, 2, None)), Some(1), "success").await);
    assert_eq!(repo.summary("bob", "bob-session").await.unwrap().call_count, 0);
}

#[tokio::test]
async fn settlement_and_audit_rollback_together_and_replay_is_idempotent() {
    let db = memory().await;
    provider(&db).await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    let repo = ChatUsageRepository::new(db.clone());
    let id = repo.begin("alice", "provider", "model", "run").await.unwrap().unwrap();
    let facts = usage(10, 5, Some(2));
    let tx = db.begin().await.unwrap();
    assert!(ChatUsageRepository::settle_in_transaction(&tx, &id, Some(&facts), Some(4), "success").await.unwrap());
    let audit_failure = tx.execute_unprepared("INSERT INTO provider_usage_logs(id,provider_id,model_id,user_id,success,created_at) VALUES('audit','absent','model','alice',1,'t')").await;
    assert!(audit_failure.is_err());
    tx.rollback().await.unwrap();
    assert_eq!(repo.summary("alice", "session").await.unwrap().pending_count, 1);
    assert_eq!(repo.summary("alice", "session").await.unwrap().total_tokens, None);

    for _ in 0..2 {
        let tx = db.begin().await.unwrap();
        if ChatUsageRepository::settle_in_transaction(&tx, &id, Some(&facts), Some(4), "success").await.unwrap() {
            tx.execute_unprepared("INSERT INTO provider_usage_logs(id,provider_id,model_id,user_id,success,created_at) VALUES('audit','provider','model','alice',1,'t')").await.unwrap();
        }
        tx.commit().await.unwrap();
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM provider_usage_logs").await, 1);
    assert_eq!(repo.summary("alice", "session").await.unwrap().total_tokens, Some(15));
    assert!(!settle(&db, &id, Some(&usage(100, 50, None)), Some(90), "failure").await);
    assert_eq!(repo.summary("alice", "session").await.unwrap().total_tokens, Some(15));
}

#[tokio::test]
async fn crash_recovery_makes_pending_unknown_without_replay_or_fake_zero() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    let repo = ChatUsageRepository::new(db.clone());
    let id = repo.begin("alice", "p", "m", "run").await.unwrap().unwrap();
    assert!(count(&db, "SELECT created_at FROM chat_usage_attempts").await > 1_000_000_000_000);
    let before_recovery = chrono::Utc::now().timestamp_millis();
    ChatRepository::new(db.clone()).recover_runs(Some("alice")).await.unwrap();
    let after_recovery = chrono::Utc::now().timestamp_millis();
    let recovered = repo.summary("alice", "session").await.unwrap();
    assert_eq!((recovered.call_count, recovered.pending_count, recovered.reported_count), (1, 0, 0));
    assert_eq!(recovered.total_tokens, None);
    assert_eq!(recovered.duration_ms, None);
    assert!(recovered.partial);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts WHERE status='unknown'").await, 1);
    let recovered_at = count(&db, "SELECT settled_at FROM chat_usage_attempts").await;
    assert!((before_recovery..=after_recovery).contains(&recovered_at));
    assert!(!settle(&db, &id, Some(&usage(1, 2, None)), Some(3), "success").await);
    assert_eq!(repo.begin("alice", "p", "m", "run").await.unwrap(), None);
}

#[tokio::test]
async fn provider_delete_retains_facts_but_session_and_account_purge_remove_them() {
    let db = memory().await;
    provider(&db).await;
    for (user, sid, rid) in [("alice", "alice-session", "alice-run"), ("bob", "bob-session", "bob-run")] {
        session(&db, user, sid).await;
        run(&db, user, sid, rid).await;
        let id = ChatUsageRepository::new(db.clone()).begin(user, "provider", "m", rid).await.unwrap().unwrap();
        assert!(settle(&db, &id, Some(&usage(8, 4, None)), Some(2), "success").await);
    }
    ProviderRepository::new(db.clone()).delete("provider", "alice").await.unwrap();
    let repo = ChatUsageRepository::new(db.clone());
    assert_eq!(repo.summary("alice", "alice-session").await.unwrap().total_tokens, Some(12));
    ChatRepository::new(db.clone()).recover_runs(Some("alice")).await.unwrap();
    ChatRepository::new(db.clone()).delete_session("alice", "alice-session").await.unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts WHERE user_id='alice'").await, 0);
    assert_eq!(repo.summary("bob", "bob-session").await.unwrap().total_tokens, Some(12));
    ChatRepository::new(db.clone()).recover_runs(Some("bob")).await.unwrap();
    UserDataRepository::new(db.clone()).purge(UserDataDomain::Chat, "bob").await.unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts").await, 0);
}

#[tokio::test]
async fn direct_account_delete_cleans_only_owned_usage_facts() {
    let db = memory().await;
    db.execute_unprepared("INSERT INTO users(id,username,username_key,password_hash,created_at,updated_at) VALUES('alice','alice','alice','x',1,1),('bob','bob','bob','x',1,1)").await.unwrap();
    for user in ["alice", "bob"] {
        session(&db, user, user).await;
        run(&db, user, user, user).await;
        ChatUsageRepository::new(db.clone()).begin(user, "p", "m", user).await.unwrap().unwrap();
    }
    db.execute_unprepared("DELETE FROM users WHERE id='alice'").await.unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts WHERE user_id='alice'").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts WHERE user_id='bob'").await, 1);
}

#[tokio::test]
async fn migration_prefers_known_totals_backfills_exact_logs_and_keeps_old_message_bindings() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, Some((Migrator::migrations().len() - 1) as u32)).await.unwrap();
    provider(&db).await;
    db.execute_unprepared("INSERT INTO providers(id,name,provider_type,base_url,api_key,user_id,created_at,updated_at) VALUES('provider-bob','fixture','openai','https://example.invalid','','bob','t','t')").await.unwrap();
    session(&db, "alice", "history").await;
    session(&db, "bob", "other").await;
    for rid in ["known-total", "log-only", "unknown-log", "summary", "no-evidence"] {
        run(&db, "alice", "history", rid).await;
    }
    db.execute_unprepared("UPDATE chat_runs SET status='completed',dispatched=1; UPDATE chat_runs SET total_tokens=100 WHERE task_id='known-total'; UPDATE chat_runs SET kind='summary',total_tokens=40 WHERE task_id='summary'; UPDATE chat_runs SET dispatched=0 WHERE task_id='no-evidence';
        INSERT INTO provider_usage_logs(id,provider_id,model_id,user_id,request_tokens,response_tokens,total_tokens,latency_ms,success,caller,request_id,usage_reported,created_at) VALUES
        ('retained-subset','provider','m','alice',20,10,30,9,1,'workbench','known-total',1,'t'),
        ('exact','provider','m','alice',10,5,15,3,1,'workbench','log-only',1,'1970-01-01T00:00:01+00:00'),
        ('unreported','provider','m','alice',0,0,0,5,0,'workbench','unknown-log',0,'t'),
        ('foreign-owner','provider-bob','m','bob',999,0,999,1,1,'workbench','log-only',1,'t'),
        ('wrong-caller','provider','m','alice',999,0,999,1,1,'settings','log-only',1,'t');
        INSERT INTO workbench_tasks(id,user_id,app_id,provider_id,model_id,status,created_at,kind,session_id,total_tokens) VALUES('old-message','alice','workbench','provider','m','succeeded',1,'chat','history',25);
        INSERT INTO chat_messages(id,user_id,session_id,seq,role,status,task_id,created_at,updated_at) VALUES('old-answer','alice','history',1,'assistant','completed','old-message',1,2);").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repo = ChatUsageRepository::new(db.clone());
    let summary = repo.summary("alice", "history").await.unwrap();
    assert_eq!(summary.total_tokens, Some(180), "100 run + 15 logs + 40 summary + 25 old message, never both sources");
    assert_eq!(summary.input_tokens, Some(10), "incomplete audit breakdown is not added to authoritative totals");
    assert_eq!((summary.call_count, summary.reported_count), (2, 1));
    assert_eq!(summary.coverage.total, 1);
    assert!(summary.legacy && summary.partial);
    assert_eq!(summary.cache_hit_rate, None);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts").await, 5);
    assert_eq!(count(&db, "SELECT created_at FROM chat_usage_attempts WHERE id='legacy-log:exact'").await, 1000);
    assert_eq!(count(&db, "SELECT settled_at FROM chat_usage_attempts WHERE id='legacy-log:exact'").await, 1000);
    assert_eq!(count(&db, "SELECT created_at FROM chat_usage_attempts WHERE id='legacy-run:old-message'").await, 1000);
    assert_eq!(count(&db, "SELECT settled_at FROM chat_usage_attempts WHERE id='legacy-run:old-message'").await, 2000);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_runs WHERE task_id='old-message'").await, 0, "migration must not manufacture newly executed runs");
    assert_eq!(repo.summary("bob", "other").await.unwrap().total_tokens, Some(0));
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(repo.summary("alice", "history").await.unwrap(), summary, "migration replay is stable");
    ProviderRepository::new(db.clone()).delete("provider", "alice").await.unwrap();
    assert_eq!(repo.summary("alice", "history").await.unwrap(), summary, "backfill is independent of deleted Provider logs");
}

#[tokio::test]
async fn summary_state_is_uncapped_account_busy_and_session_effect_scoped() {
    let db = memory().await;
    session(&db, "alice", "history").await;
    session(&db, "alice", "another").await;
    session(&db, "bob", "foreign").await;
    run(&db, "alice", "history", "old-unresolved").await;
    db.execute_unprepared("UPDATE chat_runs SET status='completed'; INSERT INTO chat_run_steps(id,task_id,ordinal,kind,phase,status,result_unknown,created_at,updated_at) VALUES('effect','old-unresolved',0,'tool','tool','unknown',1,1,1)").await.unwrap();
    for index in 0..105 {
        let id = format!("recent-{index}");
        run(&db, "alice", "history", &id).await;
        execute(&db, "UPDATE chat_runs SET status='completed' WHERE task_id=?", vec![id.into()]).await;
    }
    run(&db, "alice", "another", "active").await;
    let chat = ChatRepository::new(db.clone());
    assert_eq!(chat.summary_state("alice", "history", None).await.unwrap(), (true, true));
    assert_eq!(chat.summary_state("alice", "history", Some("active")).await.unwrap(), (false, true));
    assert_eq!(chat.summary_state("alice", "another", Some("active")).await.unwrap(), (false, false));
    assert_eq!(chat.summary_state("bob", "foreign", None).await.unwrap(), (false, false));
    assert_eq!(chat.summary_state("bob", "history", None).await.unwrap_err(), ChatStoreError::SessionNotFound);
}

#[tokio::test]
async fn aggregate_rejects_javascript_unsafe_integer_instead_of_rounding() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    db.execute_unprepared("INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,total_tokens,created_at,settled_at) VALUES('wide','alice','session','run','p','m','legacy_total','unknown',9007199254740992,1,1)").await.unwrap();
    assert!(matches!(ChatUsageRepository::new(db).summary("alice", "session").await, Err(ChatStoreError::Db(_))));
}

#[tokio::test]
async fn cache_hit_rate_is_ratio_of_sums_not_average_of_call_percentages() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    let repo = ChatUsageRepository::new(db.clone());
    for facts in [usage(100, 10, Some(100)), usage(900, 20, Some(0))] {
        let id = repo.begin("alice", "p", "m", "run").await.unwrap().unwrap();
        assert!(settle(&db, &id, Some(&facts), Some(1), "success").await);
    }
    let summary = repo.summary("alice", "session").await.unwrap();
    assert_eq!(summary.cache_hit_rate, Some(0.1));
    assert_eq!(summary.total_tokens, Some(1030), "cache tokens are already included in input");
}

#[tokio::test]
async fn latest_selects_newest_settled_fact_and_preserves_unknown_instead_of_older_usage() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    session(&db, "bob", "other").await;
    run(&db, "alice", "session", "run").await;
    run(&db, "bob", "other", "foreign-run").await;
    db.execute_unprepared("INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,input_tokens,output_tokens,total_tokens,cache_read_tokens,duration_ms,created_at,settled_at) VALUES
        ('older-known','alice','session','run','p','old-model','measured','success',10,5,15,2,4,1000,1100),
        ('a-tied-known','alice','session','run','p','tied-model','measured','success',20,10,30,3,8,2000,2100),
        ('z-tied-unknown','alice','session','run','p','unknown-model','measured','unknown',NULL,NULL,NULL,NULL,NULL,2000,2200),
        ('newer-pending','alice','session','run','p','pending-model','measured','pending',NULL,NULL,NULL,NULL,NULL,4000,NULL),
        ('foreign-latest','bob','other','foreign-run','p','foreign-model','measured','success',900,100,1000,50,99,9000,9100)").await.unwrap();
    let repo = ChatUsageRepository::new(db.clone());
    let latest = repo.summary("alice", "session").await.unwrap().latest.unwrap();
    assert_eq!(latest.model_id, "unknown-model", "created_at ties are resolved by attempt ID");
    assert_eq!(latest.created_at, Some(2000));
    assert_eq!((latest.total_tokens, latest.input_tokens, latest.output_tokens), (None, None, None));
    assert_eq!((latest.cache_read_tokens, latest.duration_ms), (None, None));
    assert!(latest.partial);

    db.execute_unprepared("INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,input_tokens,output_tokens,total_tokens,cache_read_tokens,duration_ms,created_at,settled_at) VALUES('newest-settled','alice','session','run','p','latest-model','measured','success',70,30,100,10,50,3000,3100)").await.unwrap();
    let latest = repo.summary("alice", "session").await.unwrap().latest.unwrap();
    assert_eq!(latest.model_id, "latest-model", "newer pending and foreign facts are excluded");
    assert_eq!(latest.created_at, Some(3000));
    assert_eq!((latest.total_tokens, latest.input_tokens, latest.output_tokens), (Some(100), Some(70), Some(30)));
    assert_eq!((latest.cache_read_tokens, latest.duration_ms), (Some(10), Some(50)));
    assert!(!latest.partial);
}

#[tokio::test]
async fn latest_legacy_aggregate_keeps_breakdown_unknown_and_checks_timestamp_range() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    db.execute_unprepared("INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,total_tokens,created_at,settled_at) VALUES('legacy','alice','session','run','p','historical-model','legacy_total','unknown',42,1000,1100)").await.unwrap();
    let repo = ChatUsageRepository::new(db.clone());
    let latest = repo.summary("alice", "session").await.unwrap().latest.unwrap();
    assert_eq!(latest.model_id, "historical-model");
    assert_eq!(latest.total_tokens, Some(42));
    assert_eq!((latest.input_tokens, latest.output_tokens, latest.cache_read_tokens, latest.duration_ms), (None, None, None, None));
    assert!(latest.partial);
    db.execute_unprepared("INSERT INTO chat_usage_attempts(id,user_id,session_id,run_id,provider_id,model_id,source,status,created_at,settled_at) VALUES('unsafe-time','alice','session','run','p','m','measured','unknown',9007199254740992,9007199254740992)").await.unwrap();
    assert!(matches!(repo.summary("alice", "session").await, Err(ChatStoreError::Db(_))));
}

#[tokio::test]
async fn summary_candidate_state_counts_utf8_bytes_excludes_replacements_and_keeps_legacy_generating_busy() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    session(&db, "bob", "other").await;
    db.execute_unprepared("INSERT INTO chat_messages(id,user_id,session_id,seq,role,status,content,created_at,updated_at) VALUES('answer','alice','session',1,'assistant','completed','x',1,1)").await.unwrap();
    execute(&db, "INSERT INTO chat_messages(id,user_id,session_id,seq,role,status,content,created_at,updated_at) VALUES('multibyte','alice','session',2,'user','completed',?,1,1)", vec!["界".repeat(349_525).into()]).await;
    execute(&db, "INSERT INTO chat_messages(id,user_id,session_id,seq,role,status,content,replaces_id,created_at,updated_at) VALUES('replacement','alice','session',3,'assistant','completed',?,'answer',1,1)", vec!["x".repeat(2 * 1024 * 1024).into()]).await;
    let chat = ChatRepository::new(db.clone());
    assert_eq!(chat.summary_candidate_state("alice", "session", 0).await.unwrap(), (true, false, false));
    assert_eq!(chat.summary_candidate_state("alice", "session", 1).await.unwrap(), (false, false, false));
    let source = chat.summary_source("alice", "session").await.unwrap();
    assert_eq!(source.len(), 2, "replacement drafts are excluded from both metadata and source");
    assert_eq!(source.iter().map(|row| row.content.len()).sum::<usize>(), 1024 * 1024, "exactly 1MiB is allowed");

    db.execute_unprepared("UPDATE chat_messages SET content=content||'x' WHERE id='multibyte'").await.unwrap();
    assert_eq!(chat.summary_candidate_state("alice", "session", 0).await.unwrap(), (true, true, false));
    assert!(matches!(chat.summary_source("alice", "session").await, Err(ChatStoreError::Db(_))), "oversized text is rejected before hydration");
    db.execute_unprepared("INSERT INTO chat_messages(id,user_id,session_id,seq,role,status,created_at,updated_at) VALUES('legacy-generating','alice','session',4,'assistant','generating',1,1)").await.unwrap();
    assert_eq!(chat.summary_candidate_state("alice", "session", 1).await.unwrap(), (true, true, true));
    assert_eq!(chat.summary_candidate_state("bob", "other", 0).await.unwrap(), (false, false, false));
    for id in ["session", "missing"] {
        assert_eq!(chat.summary_candidate_state("bob", id, 0).await.unwrap_err(), ChatStoreError::SessionNotFound);
    }
}

#[tokio::test]
async fn legacy_global_report_flag_does_not_invent_zero_field_presence() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, Some((Migrator::migrations().len() - 1) as u32)).await.unwrap();
    provider(&db).await;
    session(&db, "alice", "history").await;
    for id in ["partial", "total-only", "positive", "authoritative-zero"] {
        run(&db, "alice", "history", id).await;
    }
    db.execute_unprepared("UPDATE chat_runs SET status='completed',dispatched=1;
        UPDATE chat_runs SET total_tokens=0 WHERE task_id='authoritative-zero';
        INSERT INTO provider_usage_logs(id,provider_id,model_id,user_id,request_tokens,response_tokens,total_tokens,latency_ms,success,caller,request_id,usage_reported,created_at) VALUES
        ('partial','provider','partial-model','alice',10,0,10,1,1,'workbench','partial',1,'2026-11-11T00:00:00+00:00'),
        ('total-only','provider','total-only-model','alice',0,0,0,1,1,'workbench','total-only',1,'2026-11-11T00:00:01+00:00'),
        ('positive','provider','positive-model','alice',3,2,5,1,1,'workbench','positive',NULL,'2026-11-11T00:00:02+00:00')").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let summary = ChatUsageRepository::new(db.clone()).summary("alice", "history").await.unwrap();
    assert_eq!((summary.total_tokens, summary.input_tokens, summary.output_tokens), (Some(15), Some(13), Some(2)));
    assert_eq!((summary.coverage.total, summary.coverage.input, summary.coverage.output), (2, 2, 1));
    assert_eq!((summary.call_count, summary.reported_count), (3, 2));
    assert!(summary.partial && summary.legacy);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts WHERE id='legacy-log:partial' AND input_tokens=10 AND output_tokens IS NULL").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts WHERE id='legacy-log:total-only' AND input_tokens IS NULL AND output_tokens IS NULL AND total_tokens IS NULL").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_usage_attempts WHERE id='legacy-run:authoritative-zero' AND total_tokens=0").await, 1, "a separately known run total is retained");
}

#[tokio::test]
async fn legacy_latest_uses_each_audit_timestamp_and_marks_malformed_dates_unavailable() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, Some((Migrator::migrations().len() - 1) as u32)).await.unwrap();
    provider(&db).await;
    session(&db, "alice", "history").await;
    session(&db, "alice", "malformed").await;
    run(&db, "alice", "history", "tool-loop").await;
    run(&db, "alice", "malformed", "malformed-run").await;
    db.execute_unprepared("UPDATE chat_runs SET status='completed',dispatched=1,created_at=500,updated_at=999;
        INSERT INTO provider_usage_logs(id,provider_id,model_id,user_id,request_tokens,response_tokens,total_tokens,latency_ms,success,caller,request_id,usage_reported,created_at) VALUES
        ('z-old','provider','old-model','alice',10,5,15,1,1,'workbench','tool-loop',1,'2026-11-11T00:00:00.125+00:00'),
        ('a-new','provider','new-model','alice',20,10,30,2,1,'workbench','tool-loop',1,'2026-11-11 08:00:00.375 +08:00'),
        ('zz-malformed','provider','bad-date-model','alice',30,15,45,3,1,'workbench','tool-loop',1,'not-a-date'),
        ('only-malformed','provider','unknown-time-model','alice',40,20,60,4,1,'workbench','malformed-run',1,'broken')").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repo = ChatUsageRepository::new(db.clone());
    let summary = repo.summary("alice", "history").await.unwrap();
    let latest = summary.latest.unwrap();
    assert_eq!(latest.model_id, "new-model", "audit time wins over UUID order and shared run time");
    assert_eq!(latest.created_at, Some(chrono::DateTime::parse_from_rfc3339("2026-11-11T00:00:00.375Z").unwrap().timestamp_millis()));
    assert_eq!(latest.total_tokens, Some(30));
    assert!(latest.partial, "legacy audit coverage is never claimed complete");
    assert_eq!(count(&db, "SELECT created_at FROM chat_usage_attempts WHERE id='legacy-log:z-old'").await,
        chrono::DateTime::parse_from_rfc3339("2026-11-11T00:00:00.125Z").unwrap().timestamp_millis());
    assert_eq!(count(&db, "SELECT created_at FROM chat_usage_attempts WHERE id='legacy-log:zz-malformed'").await, 0);
    let unknown_time = repo.summary("alice", "malformed").await.unwrap().latest.unwrap();
    assert_eq!(unknown_time.created_at, None, "malformed dates do not inherit a guessed run timestamp");
    assert_eq!(unknown_time.total_tokens, Some(60));
    assert!(unknown_time.partial);
}

#[tokio::test]
async fn non_success_snapshots_remain_subtotals_even_when_all_fields_present() {
    let db = memory().await;
    let repo = ChatUsageRepository::new(db.clone());
    for outcome in ["interrupted", "unknown", "failure", "success"] {
        session(&db, "alice", outcome).await;
        run(&db, "alice", outcome, outcome).await;
        let id = repo.begin("alice", "p", "m", outcome).await.unwrap().unwrap();
        let snapshot = TokenUsage {
            cache_write_tokens: Some(4), reasoning_tokens: Some(5),
            ..usage(100, 20, Some(25))
        };
        assert!(settle(&db, &id, Some(&snapshot), Some(8), outcome).await);
        let summary = repo.summary("alice", outcome).await.unwrap();
        assert_eq!((summary.total_tokens, summary.input_tokens, summary.output_tokens), (Some(120), Some(100), Some(20)));
        assert_eq!((summary.cache_read_tokens, summary.cache_write_tokens, summary.reasoning_tokens, summary.duration_ms), (Some(25), Some(4), Some(5), Some(8)));
        assert_eq!((summary.call_count, summary.reported_count, summary.pending_count), (1, 1, 0));
        assert_eq!((summary.coverage.total, summary.coverage.input, summary.coverage.output, summary.coverage.cache_read), (1, 1, 1, 1), "coverage counts observed field presence, not terminal bill completeness");
        let incomplete = !matches!(outcome, "success");
        assert_eq!(summary.partial, incomplete);
        assert_eq!(summary.cache_hit_rate, if incomplete { None } else { Some(0.25) });
        let latest = summary.latest.unwrap();
        assert_eq!(latest.total_tokens, Some(120));
        assert_eq!(latest.partial, incomplete);
    }
}

#[tokio::test]
async fn live_run_terminal_does_not_seal_a_pending_provider_receipt() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    let repo = ChatUsageRepository::new(db.clone());
    let id = repo.begin("alice", "p", "m", "run").await.unwrap().unwrap();
    ChatRepository::new(db.clone()).seal_run("alice", "run", "interrupted", true, true, None, None).await.unwrap();
    assert_eq!(repo.summary("alice", "session").await.unwrap().pending_count, 1, "live finalization must not race an owned async settlement");
    assert!(settle(&db, &id, Some(&usage(9, 3, Some(1))), Some(8), "interrupted").await);
    let summary = repo.summary("alice", "session").await.unwrap();
    assert_eq!((summary.total_tokens, summary.pending_count), (Some(12), 0));
    assert!(summary.partial);
}

#[tokio::test]
async fn undispatched_admission_abort_removes_no_request_but_never_removes_settled_facts() {
    let db = memory().await;
    session(&db, "alice", "session").await;
    run(&db, "alice", "session", "run").await;
    let repo = ChatUsageRepository::new(db.clone());
    let id = repo.begin("alice", "p", "m", "run").await.unwrap().unwrap();
    assert!(ChatUsageRepository::abort_undispatched(&db, &id).await.unwrap());
    assert!(!ChatUsageRepository::abort_undispatched(&db, &id).await.unwrap());
    let empty = repo.summary("alice", "session").await.unwrap();
    assert_eq!((empty.total_tokens, empty.call_count, empty.pending_count, empty.latest), (Some(0), 0, 0, None));
    let settled = repo.begin("alice", "p", "m", "run").await.unwrap().unwrap();
    assert!(settle(&db, &settled, Some(&usage(2, 1, None)), Some(1), "success").await);
    assert!(!ChatUsageRepository::abort_undispatched(&db, &settled).await.unwrap());
    let recovered = repo.begin("alice", "p", "m", "run").await.unwrap().unwrap();
    ChatRepository::new(db.clone()).recover_runs(Some("alice")).await.unwrap();
    assert!(!ChatUsageRepository::abort_undispatched(&db, &recovered).await.unwrap());
    let summary = repo.summary("alice", "session").await.unwrap();
    assert_eq!((summary.total_tokens, summary.call_count), (Some(3), 2));
}
