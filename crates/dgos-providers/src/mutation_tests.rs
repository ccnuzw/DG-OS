use super::*;
use crate::{test_support::{MemoryCredentialVault, seed_provider}, CliProviderBinding,
    CliProviderChange, CliProviderChangeSink, ProviderConfig};
use dgos_db::{migrations::Migrator, sea_orm::{ConnectionTrait, DbBackend, Statement},
    sea_orm_migration::MigratorTrait};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Sink(mpsc::UnboundedSender<(CliProviderChange, Vec<CliProviderBinding>)>);
impl CliProviderChangeSink for Sink {
    fn changed(&self, change: &CliProviderChange, targets: &[CliProviderBinding]) {
        self.0.send((change.clone(), targets.to_vec())).unwrap();
    }
}

fn config(key: &str) -> ProviderConfig {
    ProviderConfig { name: "fixture".into(), provider_type: "openai_compatible".into(),
        base_url: "https://example.invalid/v1".into(), api_key: key.into(), enabled: true, config: None }
}

async fn setup() -> (tempfile::TempDir, ProviderManager, Provider,
    mpsc::UnboundedReceiver<(CliProviderChange, Vec<CliProviderBinding>)>) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("mutation.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(db, &vault, config("synthetic-old"), "local").await.unwrap();
    let manager = ProviderManager::with_vault(&url, vault).await.unwrap();
    manager.add_manual_model(&provider.id, "local", "fixture-chat", None).await.unwrap();
    let provider = manager.get_provider(&provider.id, "local").await.unwrap().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    manager.install_cli_change_sink(Arc::new(Sink(tx))).unwrap();
    (dir, manager, provider, rx)
}

fn binding(provider: &Provider) -> CliProviderBinding {
    CliProviderBinding { binding_id: uuid::Uuid::new_v4().to_string(), owner: "local".into(),
        session_id: uuid::Uuid::new_v4().to_string(), host_instance_id: "host".into(),
        provider_id: provider.id.clone(), model_id: "fixture-chat".into(), provider_revision: provider.config_revision }
}

#[derive(Clone, Copy, Debug)]
enum Mutation { Toggle, Update, Delete, RemoveModel }

// This barrier is compiled only for tests and is consumed immediately after the
// repository has completed COMMIT, before the manager can cancel the lease.
// The same observable window exists while sqlx's worker finishes a submitted
// COMMIT after its original waiter has gone away. No sleep chooses the schedule.
#[tokio::test]
async fn dropped_commit_waiters_still_cancel_exact_leases_and_shutdown_drains_them() {
    for mutation in [Mutation::Toggle, Mutation::Update, Mutation::Delete, Mutation::RemoveModel] {
        let (_dir, manager, provider, mut events) = setup().await;
        let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
        let barrier = Arc::new(CommitBarrier::default());
        *manager.mutations.barrier.lock().unwrap() = Some(barrier.clone());
        let writer = manager.clone();
        let id = provider.id.clone();
        let waiter = tokio::spawn(async move {
            match mutation {
                Mutation::Toggle => writer.toggle_provider(&id, false, "local").await.map(|_| ()),
                Mutation::Update => writer.update_provider(&id, config("synthetic-new"), "local").await.map(|_| ()),
                Mutation::Delete => writer.delete_provider(&id, "local").await,
                Mutation::RemoveModel => writer.remove_manual_model(&id, "local", "fixture-chat").await,
            }
        });
        tokio::time::timeout(Duration::from_secs(5), barrier.reached.notified()).await.unwrap();
        // This independent connection observes committed data before releasing the tail.
        let observer = dgos_db::establish_connection(&format!("sqlite://{}?mode=rwc", _dir.path().join("mutation.db").display())).await.unwrap();
        let row = observer.query_one(Statement::from_sql_and_values(DbBackend::Sqlite,
            "SELECT enabled,config_revision,credential_ref FROM providers WHERE id=?", [provider.id.clone().into()])).await.unwrap();
        match mutation {
            Mutation::Delete => assert!(row.is_none()),
            Mutation::Toggle => assert!(!row.unwrap().try_get::<bool>("", "enabled").unwrap()),
            Mutation::Update => assert_ne!(row.unwrap().try_get::<Option<String>>("", "credential_ref").unwrap(), provider.credential_ref),
            Mutation::RemoveModel => {
                assert!(row.unwrap().try_get::<i64>("", "config_revision").unwrap() > provider.config_revision);
                assert!(dgos_db::ProviderRepository::new(observer).model_snapshot(&provider.id, "local", "fixture-chat").await.is_err());
            }
        }
        assert!(!lease.is_cancelled());
        assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        assert!(manager.gate.begin_maintenance().is_err());
        let mut shutdown = Box::pin(manager.shutdown_provider_mutations());
        assert!(matches!(futures::poll!(&mut shutdown), std::task::Poll::Pending));
        assert!(matches!(manager.toggle_provider(&provider.id, true, "local").await, Err(DgosError::ProviderRequestRejected)));
        barrier.release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), shutdown).await.unwrap().unwrap();
        assert!(lease.is_cancelled());
        let (change, targets) = events.try_recv().unwrap();
        assert_eq!(change.kind, CliProviderChangeKind::ExecutionInvalidated);
        assert_eq!(targets, vec![lease.binding().clone()]);
        assert!(manager.gate.begin_maintenance().is_ok());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn dropped_mutation_waiter_still_terminates_and_reaps_a_real_child() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let (_dir, manager, provider, mut events) = setup().await;
    let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    let launch = manager.resolve_cli_provider_for_lease(&lease).await.unwrap();
    let mut child = manager.with_cli_spawn(&lease, &launch, |_| {
        let child = tokio::process::Command::new("/bin/sh").args(["-c", "printf 'ready\\n'; read value"])
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
            .kill_on_drop(true).spawn().map_err(|_| DgosError::InternalError("fixture spawn failed".into()))?;
        Ok(child)
    }).await.unwrap();
    let mut ready = String::new();
    tokio::time::timeout(Duration::from_secs(5), BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready)).await.unwrap().unwrap();
    assert_eq!(ready, "ready\n");
    assert!(child.try_wait().unwrap().is_none());
    let cancellation = lease.cancellation_token();
    let reaper = tokio::spawn(async move {
        cancellation.cancelled().await;
        child.start_kill().unwrap();
        child.wait().await.unwrap()
    });
    let barrier = Arc::new(CommitBarrier::default());
    *manager.mutations.barrier.lock().unwrap() = Some(barrier.clone());
    let writer = manager.clone();
    let id = provider.id.clone();
    let waiter = tokio::spawn(async move { writer.toggle_provider(&id, false, "local").await });
    tokio::time::timeout(Duration::from_secs(5), barrier.reached.notified()).await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    barrier.release.notify_one();
    manager.shutdown_provider_mutations().await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), reaper).await.unwrap().unwrap();
    assert!(!status.success());
    assert_eq!(events.try_recv().unwrap().1, vec![lease.binding().clone()]);
}

#[tokio::test]
async fn definitely_rolled_back_database_failure_does_not_claim_success_or_cancel() {
    let (_dir, manager, provider, mut events) = setup().await;
    let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    manager.db.execute(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER fixture_reject BEFORE UPDATE ON providers BEGIN SELECT RAISE(ABORT,'fixture rollback'); END;")).await.unwrap();
    assert!(manager.toggle_provider(&provider.id, false, "local").await.is_err());
    assert_eq!(manager.get_provider(&provider.id, "local").await.unwrap().unwrap().config_revision, provider.config_revision);
    assert!(!lease.is_cancelled());
    assert!(matches!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
    manager.shutdown_provider_mutations().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_commit_cancels_conservatively_without_a_success_event() {
    let (_dir, manager, provider, mut events) = setup().await;
    let lease = manager.register_cli_opening(binding(&provider), &CancellationToken::new()).await.unwrap();
    let guard = manager.credentials_lock.lock().await;
    manager.provider_repo.toggle_enabled(&provider.id, false, "local").await.unwrap();
    let uncertain: DgosResult<()> = Err(DgosError::DatabaseError("synthetic commit response lost".into()));
    let delivery = manager.cli_mutation_outcome(&provider, &uncertain, CliProviderChangeKind::ExecutionInvalidated).await.unwrap();
    drop(guard);
    delivery.deliver();
    assert!(lease.is_cancelled());
    let (change, targets) = events.try_recv().unwrap();
    assert_eq!(change.kind, CliProviderChangeKind::CommitUncertain);
    assert_eq!(targets, vec![lease.binding().clone()]);
    manager.shutdown_provider_mutations().await.unwrap();
}
