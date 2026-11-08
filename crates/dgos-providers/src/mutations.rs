//! Manager-owned mutation completion. Dropping an RPC waiter never aborts a commit tail.
use super::{cli_bindings::CliChangeDelivery, ProviderManager};
use crate::CliProviderChangeKind;
use dgos_types::{DgosError, DgosResult, Provider};
use std::{future::Future, sync::Mutex};
use tokio_util::task::TaskTracker;

#[derive(Default)]
pub(super) struct ProviderMutations {
    // Serializes task registration against close; never held across an await.
    closed: Mutex<bool>,
    tasks: TaskTracker,
    #[cfg(test)]
    pub(super) barrier: Mutex<Option<std::sync::Arc<CommitBarrier>>>,
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct CommitBarrier {
    pub(super) reached: tokio::sync::Notify,
    pub(super) release: tokio::sync::Notify,
}

impl ProviderManager {
    pub(super) async fn own_mutation<T, F, Fut>(&self, operation: F) -> DgosResult<T>
    where
        T: Send + 'static,
        F: FnOnce(ProviderManager) -> Fut + Send + 'static,
        Fut: Future<Output = DgosResult<T>> + Send + 'static,
    {
        let task = {
            let closed = self.mutations.closed.lock().map_err(|_| DgosError::ProviderRequestRejected)?;
            if *closed { return Err(DgosError::ProviderRequestRejected); }
            let permit = self.gate.admit().map_err(|_| DgosError::ProviderRequestRejected)?;
            let manager = self.clone();
            self.mutations.tasks.spawn(async move {
                // Held through DB commit, lease invalidation and lock-free delivery,
                // even when nobody is waiting for this task's result any more.
                let _permit = permit;
                operation(manager).await
            })
        };
        task.await.map_err(|_| DgosError::InternalError("Provider mutation task failed".into()))?
    }

    /// Close mutation admission permanently and wait for every accepted mutation,
    /// including detached callers' commit tails and notifications. Call before
    /// shutting down the runtime/database. Cancellation of this wait does not
    /// cancel mutations; the next call continues draining the same tracker.
    /// Maintenance uses the shared ActivityPermit and cannot start mid-tail.
    pub async fn shutdown_provider_mutations(&self) -> DgosResult<()> {
        {
            let mut closed = self.mutations.closed.lock().map_err(|_| DgosError::ProviderRequestRejected)?;
            *closed = true;
            self.mutations.tasks.close();
        }
        self.mutations.tasks.wait().await;
        Ok(())
    }

    /// Classify an uncertain repository return under credentials_lock. A database
    /// error is not proof that COMMIT rolled back. Re-read the same worker/DB; if
    /// changed or unreadable, cancel conservatively and report uncertainty rather
    /// than publishing a successful mutation. Never retry the mutation itself.
    pub(super) async fn cli_mutation_outcome<T: Sync>(&self, before: &Provider,
        result: &DgosResult<T>, kind: CliProviderChangeKind,
    ) -> Option<CliChangeDelivery> {
        let uncertain = matches!(result, Err(DgosError::DatabaseError(_)));
        if result.is_err() && !uncertain { return None; }
        #[cfg(test)]
        self.pause_after_cli_commit(result.is_ok()).await;
        let observed = self.provider_repo.get(&before.id, &before.user_id).await;
        match observed {
            Ok(Some(after)) if after.config_revision == before.config_revision
                && after.enabled == before.enabled && after.credential_ref == before.credential_ref
                && after.base_url == before.base_url && after.provider_type == before.provider_type
                && after.config == before.config => None,
            Ok(after) => Some(self.cli_provider_committed(before, after.map(|value| value.config_revision),
                if uncertain { CliProviderChangeKind::CommitUncertain } else { kind })),
            Err(_) => Some(self.cli_provider_committed(before, None, CliProviderChangeKind::CommitUncertain)),
        }
    }

    /// A model-cache change only invalidates bindings whose selected model is no
    /// longer eligible. The exact targets are captured while credentials_lock is held.
    pub(super) async fn cli_model_mutation_outcome<T: Sync>(&self, before: &Provider, result: &DgosResult<T>) -> Option<CliChangeDelivery> {
        if result.is_err() {
            return self.cli_mutation_outcome(before, result, CliProviderChangeKind::ExecutionInvalidated).await;
        }
        #[cfg(test)]
        self.pause_after_cli_commit(true).await;
        let after = self.provider_repo.get(&before.id, &before.user_id).await;
        let models = self.model_repo.list(&before.id).await;
        match (after, models) {
            (Ok(Some(after)), Ok(models)) => Some(self.cli_bindings.committed_models(crate::CliProviderChange {
                owner: before.user_id.clone(), provider_id: before.id.clone(), previous_revision: before.config_revision,
                current_revision: Some(after.config_revision), kind: CliProviderChangeKind::ExecutionInvalidated,
            }, &models)),
            _ => Some(self.cli_provider_committed(before, None, CliProviderChangeKind::CommitUncertain)),
        }
    }

    #[cfg(test)]
    async fn pause_after_cli_commit(&self, committed: bool) {
        if !committed { return; }
        let barrier = self.mutations.barrier.lock().unwrap().take();
        if let Some(barrier) = barrier {
            barrier.reached.notify_one();
            barrier.release.notified().await;
        }
    }
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;
