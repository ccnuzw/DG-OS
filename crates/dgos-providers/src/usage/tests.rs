use super::*;
use dgos_db::{BeginGeneration, ChatRepository, ChatTarget, MaintenanceGate};
use dgos_db::migrations::Migrator;
use dgos_db::sea_orm_migration::MigratorTrait;
use dgos_types::ProviderConfig;

struct Fixture {
    _dir: tempfile::TempDir,
    db: DatabaseConnection,
    provider: dgos_types::Provider,
    session: String,
    provenance: ExecutionProvenance,
    gate: MaintenanceGate,
    registry: Arc<UsageRegistry>,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc",dir.path().join("usage.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db,None).await.unwrap();
    let vault = crate::test_support::MemoryCredentialVault::default();
    let provider = crate::test_support::seed_provider(db.clone(),&vault,ProviderConfig {
        name:"usage lifecycle".into(),provider_type:"openai_compatible".into(),
        base_url:"http://127.0.0.1:9".into(),api_key:"synthetic".into(),enabled:true,config:None,
    },"local").await.unwrap();
    let chats = ChatRepository::new(db.clone());
    let session = chats.create_session("local",&ChatTarget { provider_id:Some(provider.id.clone()),
        model_id:Some("m".into()),no_skill:true,..Default::default() }).await.unwrap();
    let run = uuid::Uuid::new_v4().to_string();
    chats.begin_generation(&BeginGeneration {
        user:"local",session_id:&session.id,task_id:&run,
        user_message_id:&uuid::Uuid::new_v4().to_string(),assistant_message_id:&uuid::Uuid::new_v4().to_string(),
        user_content:Some("hello"),regenerate_of:None,provider_id:&provider.id,model_id:"m",
        provider_config_revision:Some(provider.config_revision),endpoint_fingerprint:"fixture",skills:&[],
        generation:&Default::default(),tools:&[],assets:&[],
    }).await.unwrap();
    Fixture { _dir:dir,db,provider,session:session.id,
        provenance:ExecutionProvenance { caller:BuiltinCaller::Workbench,request_id:run },
        gate:MaintenanceGate::new(),registry:Arc::new(UsageRegistry::default()) }
}

impl Fixture {
    fn audit(&self,provenance:Option<&ExecutionProvenance>) -> CallAudit {
        CallAudit::new(&self.db,&self.provider.id,"m","local",provenance,&self.registry,self.gate.admit().unwrap())
    }
    async fn summary(&self) -> dgos_db::ChatUsageSummary {
        ChatUsageRepository::new(self.db.clone()).summary("local",&self.session).await.unwrap()
    }
}

fn measured(prompt:u32,output:u32,total:u32) -> TokenUsage {
    TokenUsage { prompt_tokens:prompt,completion_tokens:output,total_tokens:total,
        prompt_tokens_reported:true,completion_tokens_reported:true,total_tokens_reported:true,
        ..Default::default() }
}

#[test]
fn detail_only_report_keeps_explicit_total_and_overflow_invalidates_a_derived_total() {
    let mut usage = measured(10,1,20);
    merge_usage(&mut usage,&TokenUsage { cache_read_tokens:Some(3),..Default::default() });
    assert_eq!((usage.total_tokens,usage.total_tokens_reported),(20,true));
    let mut usage = measured(u32::MAX-1,0,u32::MAX-1);
    merge_usage(&mut usage,&TokenUsage { completion_tokens:2,completion_tokens_reported:true,..Default::default() });
    assert!(!usage.total_tokens_reported);
    assert_eq!(usage.total_tokens,0);
}

#[tokio::test]
async fn cancelled_committed_admission_is_owned_until_undispatched_row_is_removed() {
    let f = fixture().await;
    let audit = f.audit(Some(&f.provenance));
    let meter = audit.meter.clone();
    let resume = Arc::new(Notify::new());
    *meter.admission_resume.lock().unwrap() = Some(resume.clone());
    let network = Arc::new(AtomicBool::new(false));
    let pending = {
        let meter = meter.clone();
        let network = network.clone();
        tokio::spawn(async move {
            let _audit = audit;
            meter.begin().await.unwrap();
            meter.mark_dispatched();
            network.store(true,Ordering::Release);
        })
    };
    meter.admission_started.notified().await;
    assert_eq!(f.summary().await.pending_count,1,"insert committed before its caller resumes");
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert!(!network.load(Ordering::Acquire));
    assert!(f.gate.begin_maintenance().is_err(),"cleanup owns the maintenance permit");
    let barrier = {
        let registry = f.registry.clone();
        let run = f.provenance.request_id.clone();
        tokio::spawn(async move { registry.settle("local",&run).await })
    };
    assert!(!barrier.is_finished());
    resume.notify_one();
    assert!(barrier.await.unwrap());
    assert_eq!((f.summary().await.call_count,f.summary().await.pending_count),(0,0));
    assert!(UsageLogRepository::new(f.db.clone()).list("local",None).await.unwrap().is_empty());
    assert!(f.gate.begin_maintenance().is_ok());
    assert!(!network.load(Ordering::Acquire));
    assert!(f.registry.meters.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dropped_nonchat_call_keeps_maintenance_permit_through_settlement() {
    let f = fixture().await;
    let foreign = ExecutionProvenance { caller:BuiltinCaller::Module,request_id:f.provenance.request_id.clone() };
    let audit = f.audit(Some(&foreign));
    let meter = audit.meter.clone();
    meter.begin().await.unwrap();
    meter.mark_dispatched();
    meter.observe(&measured(7,2,9));
    let lock = meter.lifecycle.clone().lock_owned().await;
    drop(audit);
    assert!(meter.operations.load(Ordering::Acquire)>0,"Drop registers cleanup before returning");
    assert!(f.gate.begin_maintenance().is_err());
    drop(lock);
    meter.wait_idle().await;
    let rows = UsageLogRepository::new(f.db.clone()).list("local",None).await.unwrap();
    assert_eq!(rows.len(),1);
    assert_eq!((rows[0].request_tokens,rows[0].response_tokens),(7,2));
    assert_eq!(f.summary().await.call_count,0,"module provenance cannot claim a chat run");
    assert!(f.registry.meters.lock().unwrap().is_empty());
    assert!(f.gate.begin_maintenance().is_ok());
}

#[tokio::test]
async fn cancelled_normal_finish_is_still_owned_by_the_run_barrier() {
    let f = fixture().await;
    let mut audit = f.audit(Some(&f.provenance));
    let meter = audit.meter.clone();
    meter.begin().await.unwrap();
    meter.mark_dispatched();
    meter.observe(&measured(6,3,9));
    let lock = meter.lifecycle.clone().lock_owned().await;
    let finishing = tokio::spawn(async move { audit.finish(None).await });
    while meter.operations.load(Ordering::Acquire)==0 { tokio::task::yield_now().await; }
    finishing.abort();
    assert!(finishing.await.unwrap_err().is_cancelled());
    assert!(f.gate.begin_maintenance().is_err());
    drop(lock);
    assert!(f.registry.settle("local",&f.provenance.request_id).await);
    let summary = f.summary().await;
    assert_eq!((summary.pending_count,summary.reported_count,summary.total_tokens),(0,1,Some(9)));
    let rows = UsageLogRepository::new(f.db.clone()).list("local",None).await.unwrap();
    assert_eq!(rows.len(),1);
    assert!(rows[0].success);
    assert!(f.gate.begin_maintenance().is_ok());
}

#[tokio::test]
async fn body_read_failure_settles_actual_error_before_retry_backoff() {
    let f = fixture().await;
    let mut audit = f.audit(Some(&f.provenance));
    let meter = audit.meter.clone();
    let policy = crate::retry::RetryPolicy::new(dgos_types::RetryConfig {
        max_retries:1,initial_delay_ms:30_000,max_delay_ms:30_000,
    });
    let (failed_tx,failed_rx) = tokio::sync::oneshot::channel();
    let mut failed_tx = Some(failed_tx);
    let retries = Arc::new(AtomicUsize::new(0));
    let running = {
        let meter = meter.clone();
        let retries = retries.clone();
        tokio::spawn(async move {
            meter.scope(policy.execute_hinted(|| {
                let failed_tx = failed_tx.take();
                let meter = meter.clone();
                let attempt = retries.fetch_add(1,Ordering::AcqRel);
                async move {
                    meter.begin().await?;
                    meter.mark_dispatched();
                    if attempt==0 {
                        // HTTP headers were accepted; the body read then timed out.
                        if let Some(tx) = failed_tx { let _ = tx.send(()); }
                        Err(crate::retry::AttemptError::new(DgosError::Timeout))
                    } else { Ok(()) }
                }
            })).await
        })
    };
    failed_rx.await.unwrap();
    meter.wait_idle().await;
    assert!(meter.current.lock().unwrap().is_none(),"body failure closed before the retry sleep");
    assert_eq!(retries.load(Ordering::Acquire),1);
    let summary = f.summary().await;
    assert_eq!((summary.call_count,summary.pending_count),(1,0));
    let rows = UsageLogRepository::new(f.db.clone()).list("local",None).await.unwrap();
    assert_eq!(rows[0].error_code.as_deref(),Some("1001006"));
    assert!(rows[0].latency_ms<30_000,"backoff is excluded from request execution time");
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert!(audit.finish(Some(DgosError::StreamInterrupted.error_code())).await);
    assert!(f.registry.settle("local",&f.provenance.request_id).await);
    assert_eq!(f.summary().await.call_count,1);
}

#[tokio::test]
async fn completed_workbench_call_without_owned_chat_does_not_leak_registry_entries() {
    let f = fixture().await;
    let provenance = ExecutionProvenance { caller:BuiltinCaller::Workbench,request_id:uuid::Uuid::new_v4().to_string() };
    let mut audit = f.audit(Some(&provenance));
    audit.meter.begin().await.unwrap();
    audit.meter.mark_dispatched();
    assert!(audit.finish(None).await);
    assert!(f.registry.meters.lock().unwrap().is_empty());
}

async fn wait_for_delete(gate: &RwLock<()>) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while gate.try_read().is_ok() { tokio::task::yield_now().await; }
    }).await.expect("delete must queue behind the active attempt");
}

#[tokio::test]
async fn provider_delete_waits_for_dropped_call_cleanup_through_the_terminal_transaction() {
    let f = fixture().await;
    let audit = f.audit(Some(&f.provenance));
    let meter = audit.meter.clone();
    meter.begin().await.unwrap();
    meter.mark_dispatched();
    meter.observe(&measured(4, 3, 7));
    let lifecycle = meter.lifecycle.clone().lock_owned().await;
    drop(audit);
    let gate = f.registry.provider_gate("local", &f.provider.id);
    let deleting = {
        let gate = gate.clone();
        let db = f.db.clone();
        let id = f.provider.id.clone();
        tokio::spawn(async move {
            let _deleting = gate.write_owned().await;
            ProviderRepository::new(db).delete(&id, "local").await
        })
    };
    wait_for_delete(&gate).await;
    assert!(!deleting.is_finished());
    assert_eq!(f.summary().await.pending_count, 1);
    drop(lifecycle);
    deleting.await.unwrap().unwrap();
    assert!(f.registry.settle("local", &f.provenance.request_id).await);
    let summary = f.summary().await;
    assert_eq!((summary.total_tokens, summary.call_count, summary.reported_count, summary.pending_count),
        (Some(7), 1, 1, 0));
    assert!(summary.partial, "dropped calls preserve observed facts without claiming a complete bill");
    assert!(UsageLogRepository::new(f.db.clone()).list("local", None).await.unwrap().is_empty());
    assert!(f.gate.begin_maintenance().is_ok());
}

#[tokio::test]
async fn queued_provider_delete_revalidates_prepared_admission_before_any_new_dispatch() {
    let f = fixture().await;
    let mut live = f.audit(Some(&f.provenance));
    live.meter.begin().await.unwrap();
    live.meter.mark_dispatched();
    live.meter.observe(&measured(4, 3, 7));
    let mut prepared = f.audit(Some(&f.provenance));
    let gate = f.registry.provider_gate("local", &f.provider.id);
    let deleting = {
        let gate = gate.clone();
        let db = f.db.clone();
        let id = f.provider.id.clone();
        tokio::spawn(async move {
            let _deleting = gate.write_owned().await;
            ProviderRepository::new(db).delete(&id, "local").await
        })
    };
    wait_for_delete(&gate).await;
    let admission = {
        let meter = prepared.meter.clone();
        tokio::spawn(async move { meter.begin().await })
    };
    assert!(live.finish(None).await);
    deleting.await.unwrap().unwrap();
    assert!(matches!(admission.await.unwrap(), Err(DgosError::ProviderNotFound(_))));
    assert!(prepared.finish(Some(DgosError::ProviderNotFound(f.provider.id.clone()).error_code())).await);
    let summary = f.summary().await;
    assert_eq!((summary.total_tokens, summary.call_count, summary.pending_count), (Some(7), 1, 0));
    assert!(UsageLogRepository::new(f.db.clone()).list("local", None).await.unwrap().is_empty());
}

#[tokio::test]
async fn provider_delete_can_complete_between_retries_and_blocks_a_stale_adapter_dispatch() {
    let f = fixture().await;
    let mut audit = f.audit(Some(&f.provenance));
    let meter = audit.meter.clone();
    let waiting = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let dispatched = Arc::new(AtomicUsize::new(0));
    let running = {
        let meter = meter.clone();
        let waiting = waiting.clone();
        let resume = resume.clone();
        let dispatched = dispatched.clone();
        tokio::spawn(async move {
            let policy = crate::retry::RetryPolicy::new(dgos_types::RetryConfig {
                max_retries: 1, initial_delay_ms: 0, max_delay_ms: 0,
            });
            let mut first = true;
            let result: DgosResult<()> = meter.scope(policy.execute_hinted(|| {
                let is_first = std::mem::replace(&mut first, false);
                let meter = meter.clone();
                let waiting = waiting.clone();
                let resume = resume.clone();
                let dispatched = dispatched.clone();
                async move {
                    if !is_first { waiting.notify_one(); resume.notified().await; }
                    meter.begin().await?;
                    meter.mark_dispatched();
                    dispatched.fetch_add(1, Ordering::AcqRel);
                    meter.observe(&measured(4, 3, 7));
                    Err(crate::retry::AttemptError::new(DgosError::UpstreamServerError))
                }
            })).await;
            let saved = audit.finish(result.as_ref().err().map(DgosError::error_code)).await;
            (result, saved)
        })
    };
    waiting.notified().await;
    assert!(meter.current.lock().unwrap().is_none(), "retry does not retain a provider read permit");
    let gate = f.registry.provider_gate("local", &f.provider.id);
    let deleting = gate.write().await;
    ProviderRepository::new(f.db.clone()).delete(&f.provider.id, "local").await.unwrap();
    drop(deleting);
    resume.notify_one();
    let (result, saved) = running.await.unwrap();
    assert!(matches!(result, Err(DgosError::ProviderNotFound(_))));
    assert!(saved);
    assert_eq!(dispatched.load(Ordering::Acquire), 1);
    let summary = f.summary().await;
    assert_eq!((summary.total_tokens, summary.call_count, summary.pending_count), (Some(7), 1, 0));
    assert!(UsageLogRepository::new(f.db.clone()).list("local", None).await.unwrap().is_empty());
}
