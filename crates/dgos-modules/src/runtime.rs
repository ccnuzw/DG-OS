//! Host-owned module runtime handles (D033).
//!
//! A handle is issued by `ModulePublisher` only after the DB pointer and the
//! on-disk artifact agree. Lifecycle transitions are crate-private so only
//! `ModuleRuntimeSupervisor` drives them; capability callers can only ask for
//! request admission, which is decided under the same lock as transitions.

use crate::loader::VerifiedModuleArtifact;
use dgos_types::HostExecutionContext;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleRuntimeState {
    Starting,
    Ready,
    Draining,
    Stopped,
    Failed,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModuleRuntimeError {
    #[error("invalid module runtime state transition")]
    InvalidTransition,
    #[error("module runtime is not accepting requests")]
    NotAccepting(ModuleRuntimeState),
    #[error("module runtime still has in-flight requests")]
    RequestsInFlight(usize),
}

/// Terminal outcome of one admitted request. Each admitted request settles
/// exactly once; nothing is retried or replayed by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleRequestOutcome {
    Succeeded,
    Failed,
    /// Authorization refused the request; the operation never ran.
    Denied,
    /// The request was dropped before it settled. An external side effect
    /// may already have happened, so the result is unknown and not replayed.
    Interrupted,
}

/// Counters for one runtime instance. Invariant:
/// `admitted == in_flight + succeeded + failed + denied + interrupted`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModuleRequestStats {
    pub admitted: u64,
    pub rejected: u64,
    pub in_flight: usize,
    pub succeeded: u64,
    pub failed: u64,
    pub denied: u64,
    pub interrupted: u64,
}

struct RuntimeInner {
    state: ModuleRuntimeState,
    stats: ModuleRequestStats,
}

struct RuntimeShared {
    inner: Mutex<RuntimeInner>,
    // Mirrors `stats.in_flight` so drains can await zero without polling.
    in_flight: tokio::sync::watch::Sender<usize>,
}

impl RuntimeShared {
    fn lock(&self) -> MutexGuard<'_, RuntimeInner> {
        // The guarded data is plain counters/state; a panic elsewhere cannot
        // leave it half-written, so recovering from poison is sound.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Opaque, publisher-issued identity for one runtime instance. Callers cannot
/// construct this handle from renderer input or arbitrary module metadata.
#[derive(Clone)]
pub struct ModuleRuntimeHandle {
    runtime_id: String,
    context: HostExecutionContext,
    artifact: VerifiedModuleArtifact,
    shared: Arc<RuntimeShared>,
}

impl ModuleRuntimeHandle {
    pub(crate) fn issue(context: HostExecutionContext, artifact: VerifiedModuleArtifact) -> Self {
        Self {
            runtime_id: Uuid::new_v4().to_string(),
            context,
            artifact,
            shared: Arc::new(RuntimeShared {
                inner: Mutex::new(RuntimeInner {
                    state: ModuleRuntimeState::Starting,
                    stats: ModuleRequestStats::default(),
                }),
                in_flight: tokio::sync::watch::Sender::new(0),
            }),
        }
    }

    /// Host-generated instance id; a restart of the same version gets a new one.
    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn context(&self) -> &HostExecutionContext {
        &self.context
    }

    pub fn module_identity(&self) -> Option<(&str, &str, &str)> {
        self.context.module_identity()
    }

    pub fn artifact(&self) -> &VerifiedModuleArtifact {
        &self.artifact
    }

    pub fn state(&self) -> ModuleRuntimeState {
        self.shared.lock().state
    }

    pub fn request_stats(&self) -> ModuleRequestStats {
        self.shared.lock().stats
    }

    pub fn is_same_runtime(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    /// Admit one request. Only a ready runtime admits; the admission is
    /// atomic with lifecycle transitions, so a request can never slip in
    /// after draining begins. The returned guard must be settled.
    pub fn begin_request(&self) -> Result<ModuleRequest, ModuleRuntimeError> {
        let mut inner = self.shared.lock();
        if inner.state != ModuleRuntimeState::Ready {
            inner.stats.rejected += 1;
            return Err(ModuleRuntimeError::NotAccepting(inner.state));
        }
        inner.stats.admitted += 1;
        inner.stats.in_flight += 1;
        self.shared.in_flight.send_replace(inner.stats.in_flight);
        drop(inner);
        Ok(ModuleRequest {
            context: self.context.with_new_request_id(),
            shared: Some(self.shared.clone()),
        })
    }

    pub(crate) fn mark_ready(&self) -> Result<(), ModuleRuntimeError> {
        self.transition(ModuleRuntimeState::Starting, ModuleRuntimeState::Ready)
    }

    pub(crate) fn begin_draining(&self) -> Result<(), ModuleRuntimeError> {
        self.transition(ModuleRuntimeState::Ready, ModuleRuntimeState::Draining)
    }

    /// `starting` has no admitted requests; `draining`/`failed` may only stop
    /// once every admitted request has settled.
    pub(crate) fn stop(&self) -> Result<(), ModuleRuntimeError> {
        let mut inner = self.shared.lock();
        match inner.state {
            ModuleRuntimeState::Starting
            | ModuleRuntimeState::Draining
            | ModuleRuntimeState::Failed => {
                if inner.stats.in_flight > 0 {
                    return Err(ModuleRuntimeError::RequestsInFlight(inner.stats.in_flight));
                }
                inner.state = ModuleRuntimeState::Stopped;
                Ok(())
            }
            ModuleRuntimeState::Ready | ModuleRuntimeState::Stopped => {
                Err(ModuleRuntimeError::InvalidTransition)
            }
        }
    }

    /// Stop admitting immediately. In-flight requests keep running to their
    /// own terminal outcome; failure does not cancel or fake their results.
    pub(crate) fn fail(&self) -> Result<(), ModuleRuntimeError> {
        let mut inner = self.shared.lock();
        if inner.state == ModuleRuntimeState::Stopped {
            return Err(ModuleRuntimeError::InvalidTransition);
        }
        inner.state = ModuleRuntimeState::Failed;
        Ok(())
    }

    /// Wait until every admitted request has settled. Returns false on timeout.
    pub(crate) async fn wait_drained(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, self.wait_settled())
            .await
            .is_ok()
    }

    pub(crate) async fn wait_settled(&self) {
        let mut receiver = self.shared.in_flight.subscribe();
        // The sender lives in `shared`, which this handle keeps alive, so the
        // channel cannot close while waiting.
        let _ = receiver.wait_for(|in_flight| *in_flight == 0).await;
    }

    fn transition(
        &self,
        expected: ModuleRuntimeState,
        next: ModuleRuntimeState,
    ) -> Result<(), ModuleRuntimeError> {
        let mut inner = self.shared.lock();
        if inner.state != expected {
            return Err(ModuleRuntimeError::InvalidTransition);
        }
        inner.state = next;
        Ok(())
    }
}

/// One admitted request. Settle it with `finish`; dropping it unsettled
/// records `Interrupted` so drains always reach a stable terminal state.
pub struct ModuleRequest {
    context: HostExecutionContext,
    shared: Option<Arc<RuntimeShared>>,
}

impl ModuleRequest {
    /// Per-request context: same host-bound user/module/version/integrity as
    /// the runtime handle, fresh request id.
    pub fn context(&self) -> &HostExecutionContext {
        &self.context
    }

    pub fn finish(mut self, outcome: ModuleRequestOutcome) {
        self.settle(outcome);
    }

    fn settle(&mut self, outcome: ModuleRequestOutcome) {
        let Some(shared) = self.shared.take() else {
            return;
        };
        let mut inner = shared.lock();
        inner.stats.in_flight -= 1;
        match outcome {
            ModuleRequestOutcome::Succeeded => inner.stats.succeeded += 1,
            ModuleRequestOutcome::Failed => inner.stats.failed += 1,
            ModuleRequestOutcome::Denied => inner.stats.denied += 1,
            ModuleRequestOutcome::Interrupted => inner.stats.interrupted += 1,
        }
        shared.in_flight.send_replace(inner.stats.in_flight);
    }
}

impl Drop for ModuleRequest {
    fn drop(&mut self) {
        self.settle(ModuleRequestOutcome::Interrupted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_types::HostSession;

    fn handle() -> ModuleRuntimeHandle {
        let context = HostExecutionContext::for_module_session(
            &HostSession::local_default(),
            "module-1",
            "1.0.0",
            &"a".repeat(64),
        );
        ModuleRuntimeHandle::issue(
            context,
            VerifiedModuleArtifact::new(
                "module-1".into(),
                "1.0.0".into(),
                "a".repeat(64),
                "/nonexistent".into(),
                "/nonexistent/entry.js".into(),
            ),
        )
    }

    #[test]
    fn only_ready_admits_and_draining_refuses_new_requests() {
        let handle = handle();
        assert_eq!(
            handle.begin_request().err(),
            Some(ModuleRuntimeError::NotAccepting(
                ModuleRuntimeState::Starting
            ))
        );
        assert_eq!(
            handle.begin_draining(),
            Err(ModuleRuntimeError::InvalidTransition)
        );
        handle.mark_ready().unwrap();
        let request = handle.begin_request().unwrap();
        assert_eq!(
            request.context().module_identity(),
            handle.module_identity()
        );
        assert_eq!(request.context().user_id(), handle.context().user_id());
        assert_ne!(
            request.context().request_id(),
            handle.context().request_id()
        );

        handle.begin_draining().unwrap();
        assert_eq!(
            handle.begin_request().err(),
            Some(ModuleRuntimeError::NotAccepting(
                ModuleRuntimeState::Draining
            ))
        );
        assert_eq!(handle.stop(), Err(ModuleRuntimeError::RequestsInFlight(1)));
        request.finish(ModuleRequestOutcome::Succeeded);
        handle.stop().unwrap();
        assert_eq!(handle.state(), ModuleRuntimeState::Stopped);
        assert_eq!(handle.fail(), Err(ModuleRuntimeError::InvalidTransition));
        assert_eq!(
            handle.mark_ready(),
            Err(ModuleRuntimeError::InvalidTransition)
        );
        assert_eq!(
            handle.request_stats(),
            ModuleRequestStats {
                admitted: 1,
                rejected: 2,
                in_flight: 0,
                succeeded: 1,
                failed: 0,
                denied: 0,
                interrupted: 0,
            }
        );
    }

    #[test]
    fn failed_runtime_refuses_requests_and_stops_only_after_settle() {
        let handle = handle();
        handle.mark_ready().unwrap();
        let request = handle.begin_request().unwrap();
        handle.fail().unwrap();
        assert_eq!(
            handle.begin_request().err(),
            Some(ModuleRuntimeError::NotAccepting(ModuleRuntimeState::Failed))
        );
        assert_eq!(handle.stop(), Err(ModuleRuntimeError::RequestsInFlight(1)));
        request.finish(ModuleRequestOutcome::Denied);
        handle.stop().unwrap();
        assert_eq!(handle.request_stats().denied, 1);
    }

    #[test]
    fn every_admitted_request_settles_exactly_once() {
        let handle = handle();
        handle.mark_ready().unwrap();
        let finished = handle.begin_request().unwrap();
        let dropped = handle.begin_request().unwrap();
        let failed = handle.begin_request().unwrap();
        finished.finish(ModuleRequestOutcome::Succeeded);
        failed.finish(ModuleRequestOutcome::Failed);
        drop(dropped);
        let stats = handle.request_stats();
        assert_eq!(
            (
                stats.admitted,
                stats.in_flight,
                stats.succeeded,
                stats.failed,
                stats.interrupted
            ),
            (3, 0, 1, 1, 1)
        );
    }

    #[tokio::test]
    async fn drain_wait_times_out_while_requests_are_in_flight() {
        let handle = handle();
        handle.mark_ready().unwrap();
        let request = handle.begin_request().unwrap();
        handle.begin_draining().unwrap();
        assert!(!handle.wait_drained(Duration::from_millis(10)).await);
        let waiter = handle.clone();
        let drained =
            tokio::spawn(async move { waiter.wait_drained(Duration::from_secs(5)).await });
        tokio::task::yield_now().await;
        drop(request);
        assert!(drained.await.unwrap());
        assert_eq!(handle.request_stats().interrupted, 1);
    }
}
