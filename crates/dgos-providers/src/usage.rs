use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use dgos_db::sea_orm::{DatabaseConnection, TransactionTrait};
use dgos_db::{ActivityPermit, ChatUsageRepository, ProviderRepository, UsageLogRepository};
use dgos_types::{BuiltinCaller, DgosError, DgosResult, ExecutionProvenance, TokenUsage};
use tokio::sync::{Mutex as AsyncMutex, Notify, OwnedRwLockReadGuard, RwLock};
use tracing::warn;

tokio::task_local! {
    static CURRENT: Arc<UsageMeter>;
}

type RunKey = (String, String);

/// Live chat finalization drains this registry before publishing its terminal state.
/// Non-chat calls are removed as soon as admission proves there is no owned chat run.
#[derive(Default)]
pub(crate) struct UsageRegistry {
    meters: Mutex<HashMap<RunKey, Vec<Arc<UsageMeter>>>>,
    providers: Mutex<HashMap<(String, String), Arc<RwLock<()>>>>,
}

impl UsageRegistry {
    pub(crate) fn provider_gate(&self, user: &str, provider: &str) -> Arc<RwLock<()>> {
        self.providers.lock().unwrap_or_else(|e| e.into_inner())
            .entry((user.to_owned(), provider.to_owned())).or_default().clone()
    }

    pub(crate) fn forget_provider(&self, user: &str, provider: &str) {
        self.providers.lock().unwrap_or_else(|e| e.into_inner())
            .remove(&(user.to_owned(), provider.to_owned()));
    }

    fn register(&self, meter: &Arc<UsageMeter>) {
        if let Some(key) = meter.run_key() {
            self.meters.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_default().push(meter.clone());
        }
    }

    fn forget(&self, meter: &UsageMeter) {
        let Some(key) = meter.run_key() else { return; };
        let mut meters = self.meters.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(run) = meters.get_mut(&key) {
            run.retain(|candidate| !std::ptr::eq(candidate.as_ref(), meter));
            if run.is_empty() { meters.remove(&key); }
        }
    }

    pub(crate) async fn settle(&self, user: &str, run: &str) -> bool {
        let key = (user.to_owned(), run.to_owned());
        // Clone rather than remove before awaiting: cancellation of this barrier cannot orphan
        // cleanup or discard its warning. The host invokes it only after dropping live calls.
        let meters = self.meters.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned().unwrap_or_default();
        let mut saved = true;
        for meter in &meters {
            meter.wait_idle().await;
            saved &= meter.finish(Some(DgosError::StreamInterrupted.error_code()), None).await;
            meter.wait_idle().await;
            saved &= !meter.has_warning();
        }
        let mut registered = self.meters.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(run) = registered.get_mut(&key) {
            run.retain(|candidate| !meters.iter().any(|meter| Arc::ptr_eq(candidate, meter)));
            if run.is_empty() { registered.remove(&key); }
        }
        saved
    }
}

struct Attempt {
    id: Option<String>,
    started: Option<Instant>,
    usage: Option<TokenUsage>,
    // Held from admission through the complete terminal transaction, including owned Drop
    // cleanup. Deletion takes the exclusive side before taking the credential pointer lock.
    _provider: OwnedRwLockReadGuard<()>,
}

/// Admission and terminal writes are owned tasks serialized by one call-local lock. Dropping
/// their waiter never drops an in-progress SQLite operation or its maintenance permit.
pub(crate) struct UsageMeter {
    db: DatabaseConnection,
    provider_id: String,
    model_id: String,
    user_id: String,
    provenance: Option<ExecutionProvenance>,
    registry: Weak<UsageRegistry>,
    current: Mutex<Option<Attempt>>,
    lifecycle: Arc<AsyncMutex<()>>,
    provider_gate: Arc<RwLock<()>>,
    operations: AtomicUsize,
    idle: Notify,
    dispatch_signal: Mutex<Option<Arc<AtomicBool>>>,
    audit_warning: AtomicBool,
    owned_chat: AtomicBool,
    #[cfg(test)]
    admission_started: Notify,
    #[cfg(test)]
    admission_resume: Mutex<Option<Arc<Notify>>>,
}

struct Operation(Arc<UsageMeter>);
impl Drop for Operation {
    fn drop(&mut self) {
        self.0.operations.fetch_sub(1, Ordering::AcqRel);
        self.0.idle.notify_waiters();
    }
}

impl UsageMeter {
    fn new(db: &DatabaseConnection, provider_id: &str, model_id: &str, user_id: &str,
        provenance: Option<&ExecutionProvenance>, registry: &Arc<UsageRegistry>) -> Arc<Self> {
        let meter = Arc::new(Self { db: db.clone(), provider_id: provider_id.into(), model_id: model_id.into(),
            user_id: user_id.into(), provenance: provenance.cloned(), registry: Arc::downgrade(registry),
            current: Mutex::new(None), lifecycle: Arc::new(AsyncMutex::new(())), operations: AtomicUsize::new(0),
            provider_gate: registry.provider_gate(user_id, provider_id),
            idle: Notify::new(), dispatch_signal: Mutex::new(None), audit_warning: AtomicBool::new(false),
            owned_chat: AtomicBool::new(false), #[cfg(test)] admission_started: Notify::new(),
            #[cfg(test)] admission_resume: Mutex::new(None) });
        registry.register(&meter);
        meter
    }

    fn run_key(&self) -> Option<RunKey> {
        self.provenance.as_ref().filter(|p| p.caller == BuiltinCaller::Workbench)
            .map(|p| (self.user_id.clone(), p.request_id.clone()))
    }

    fn operation(self: &Arc<Self>) -> Operation {
        self.operations.fetch_add(1, Ordering::AcqRel);
        Operation(self.clone())
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.operations.load(Ordering::Acquire) == 0 { return; }
            notified.await;
        }
    }

    pub(crate) async fn scope<F: Future>(self: &Arc<Self>, future: F) -> F::Output {
        CURRENT.scope(self.clone(), future).await
    }

    pub(crate) fn bind_dispatch_signal(&self, signal: Arc<AtomicBool>) {
        *self.dispatch_signal.lock().unwrap_or_else(|e| e.into_inner()) = Some(signal);
    }

    fn mark_dispatched(&self) {
        if let Some(attempt) = self.current.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            attempt.started = Some(Instant::now());
        }
        if let Some(signal) = self.dispatch_signal.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            signal.store(true, Ordering::Release);
        }
    }

    async fn begin(self: &Arc<Self>) -> DgosResult<()> {
        // Acquire before spawning. If cancellation happens during lock acquisition, no SQLite
        // statement exists; after this point the owned admission cannot outlive cleanup.
        let lifecycle = self.lifecycle.clone().lock_owned().await;
        let operation = self.operation();
        let meter = self.clone();
        tokio::spawn(async move {
            let _operation = operation;
            let _lifecycle = lifecycle;
            if meter.current.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
                return Err(DgosError::InternalError("Previous provider attempt was not settled".into()));
            }
            let provider = meter.provider_gate.clone().read_owned().await;
            // Adapter snapshots can predate deletion. Revalidate under the same permit that
            // protects settlement, before admitting a chat fact or sending any network work.
            if ProviderRepository::new(meter.db.clone()).get(&meter.provider_id, &meter.user_id).await?.is_none() {
                return Err(DgosError::ProviderNotFound(meter.provider_id.clone()));
            }
            let id = match meter.run_key() {
                Some((_, run)) => ChatUsageRepository::new(meter.db.clone())
                    .begin(&meter.user_id, &meter.provider_id, &meter.model_id, &run).await?,
                None => None,
            };
            #[cfg(test)] {
                meter.admission_started.notify_one();
                let resume = meter.admission_resume.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(resume) = resume { resume.notified().await; }
            }
            if id.is_some() { meter.owned_chat.store(true, Ordering::Release); }
            *meter.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(Attempt {
                id, started: None, usage: None, _provider: provider,
            });
            if !meter.owned_chat.load(Ordering::Acquire) {
                if let Some(registry) = meter.registry.upgrade() { registry.forget(&meter); }
            }
            Ok(())
        }).await.map_err(|e| DgosError::InternalError(e.to_string()))?
    }

    pub(crate) fn observe(&self, usage: &TokenUsage) {
        if !usage.has_reported_usage() { return; }
        if let Some(attempt) = self.current.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            merge_usage(attempt.usage.get_or_insert_with(TokenUsage::default), usage);
        }
    }

    fn start_finish(self: &Arc<Self>, error_code: Option<u32>, permit: Option<ActivityPermit>) -> tokio::task::JoinHandle<bool> {
        // Registered synchronously, before spawning or awaiting. A concurrent host barrier sees
        // both normal settlement and Drop cleanup, even if their original waiter is cancelled.
        let operation = self.operation();
        let meter = self.clone();
        let ended = Instant::now();
        tokio::spawn(async move {
            let _operation = operation;
            let _permit = permit;
            let _lifecycle = meter.lifecycle.lock().await;
            let attempt = meter.current.lock().unwrap_or_else(|e| e.into_inner()).take();
            let saved = if let Some(attempt) = attempt {
                match attempt.started {
                    Some(started) => {
                        let duration_ms = i64::try_from(ended.saturating_duration_since(started).as_millis()).unwrap_or(i64::MAX);
                        settle(&meter.db, &meter.provider_id, &meter.model_id, &meter.user_id,
                            meter.provenance.as_ref(), attempt, duration_ms, error_code).await
                    }
                    None => {
                        if let Some(id) = attempt.id {
                            match ChatUsageRepository::abort_undispatched(&meter.db, &id).await {
                                Ok(_) => true,
                                Err(error) => { warn!(error = %error, "Undispatched provider admission cleanup failed"); false }
                            }
                        } else { true }
                    }
                }
            } else { true };
            if !saved { meter.audit_warning.store(true, Ordering::Release); }
            if !meter.owned_chat.load(Ordering::Acquire) {
                if let Some(registry) = meter.registry.upgrade() { registry.forget(&meter); }
            }
            !meter.has_warning()
        })
    }

    pub(crate) async fn finish(self: &Arc<Self>, error_code: Option<u32>, permit: Option<ActivityPermit>) -> bool {
        self.start_finish(error_code, permit).await.unwrap_or(false)
    }

    pub(crate) fn has_warning(&self) -> bool { self.audit_warning.load(Ordering::Acquire) }
}

async fn settle(db: &DatabaseConnection, provider_id: &str, model_id: &str, user_id: &str,
    provenance: Option<&ExecutionProvenance>, attempt: Attempt, duration_ms: i64,
    error_code: Option<u32>) -> bool {
    let result = async {
        let txn = db.begin().await.map_err(|e| DgosError::DatabaseError(e.to_string()))?;
        if let Some(id) = &attempt.id {
            let outcome = match error_code {
                None => "success",
                Some(code) if code == DgosError::StreamInterrupted.error_code() => "interrupted",
                Some(_) => "failure",
            };
            if !ChatUsageRepository::settle_in_transaction(&txn, id, attempt.usage.as_ref(),
                Some(duration_ms), outcome).await? {
                return Err(DgosError::DatabaseError("Live provider attempt was already sealed".into()));
            }
        }
        let usage = attempt.usage.as_ref();
        UsageLogRepository::create_with_provenance_on(&txn, provider_id, model_id, user_id,
            usage.map_or(0, |u| u.prompt_tokens), usage.map_or(0, |u| u.completion_tokens),
            usage.is_some_and(TokenUsage::has_reported_usage),
            u32::try_from(duration_ms).unwrap_or(u32::MAX), error_code.is_none(),
            error_code.map(|code| code.to_string()), provenance).await?;
        txn.commit().await.map_err(|e| DgosError::DatabaseError(e.to_string()))
    }.await;
    if let Err(error) = result {
        warn!(provider_id, model_id, error = %error, "Provider usage settlement failed; attempt remains unknown");
        false
    } else { true }
}

pub(crate) struct CallAudit {
    pub(crate) meter: Arc<UsageMeter>,
    permit: Option<ActivityPermit>,
    complete: bool,
}

impl CallAudit {
    pub(crate) fn new(db: &DatabaseConnection, provider_id: &str, model_id: &str, user_id: &str,
        provenance: Option<&ExecutionProvenance>, registry: &Arc<UsageRegistry>, permit: ActivityPermit) -> Self {
        Self { meter: UsageMeter::new(db, provider_id, model_id, user_id, provenance, registry),
            permit: Some(permit), complete: false }
    }

    pub(crate) async fn finish(&mut self, error_code: Option<u32>) -> bool {
        self.complete = true;
        self.meter.finish(error_code, self.permit.take()).await
    }
}

impl Drop for CallAudit {
    fn drop(&mut self) {
        if self.complete { return; }
        if tokio::runtime::Handle::try_current().is_ok() {
            self.meter.start_finish(Some(DgosError::StreamInterrupted.error_code()), self.permit.take());
        }
        // With no runtime a durable dispatched attempt remains pending for exclusive recovery.
    }
}

pub(crate) async fn begin_dispatch() -> DgosResult<()> {
    let meter = CURRENT.try_with(Arc::clone).ok();
    if let Some(meter) = meter { meter.begin().await?; }
    Ok(())
}

pub(crate) fn mark_dispatched() { let _ = CURRENT.try_with(|meter| meter.mark_dispatched()); }

pub(crate) async fn fail_dispatch(error: &DgosError) {
    let meter = CURRENT.try_with(Arc::clone).ok();
    if let Some(meter) = meter { meter.finish(Some(error.error_code()), None).await; }
}

pub(crate) fn observe(usage: &TokenUsage) { let _ = CURRENT.try_with(|meter| meter.observe(usage)); }

/// Replace reported cumulative fields while retaining facts omitted by a later partial report.
pub(crate) fn merge_usage(previous: &mut TokenUsage, next: &TokenUsage) {
    let components_changed = next.prompt_tokens_reported || next.completion_tokens_reported;
    if next.prompt_tokens_reported {
        previous.prompt_tokens = next.prompt_tokens;
        previous.prompt_tokens_reported = true;
    }
    if next.completion_tokens_reported {
        previous.completion_tokens = next.completion_tokens;
        previous.completion_tokens_reported = true;
    }
    if next.total_tokens_reported {
        previous.total_tokens = next.total_tokens;
        previous.total_tokens_reported = true;
    } else if (components_changed || !previous.total_tokens_reported)
        && previous.prompt_tokens_reported && previous.completion_tokens_reported {
        match previous.prompt_tokens.checked_add(previous.completion_tokens) {
            Some(total) => { previous.total_tokens = total; previous.total_tokens_reported = true; }
            None => { previous.total_tokens = 0; previous.total_tokens_reported = false; }
        }
    }
    if next.cache_read_tokens.is_some() { previous.cache_read_tokens = next.cache_read_tokens; }
    if next.cache_write_tokens.is_some() { previous.cache_write_tokens = next.cache_write_tokens; }
    if next.reasoning_tokens.is_some() { previous.reasoning_tokens = next.reasoning_tokens; }
}

#[cfg(test)]
mod tests;
