//! Process leases for the trusted CLI host. No process control or Agent dependency lives here.
use crate::{CliProviderLaunch, CliProviderProtocol, ProviderManager};
use dgos_types::{DgosError, DgosResult};
use std::sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliProviderChangeKind {
    MetadataChanged,
    ExecutionInvalidated,
    /// Commit could not be established reliably; targets are cancelled conservatively,
    /// never reported as a successfully applied configuration change.
    CommitUncertain,
}

/// Non-secret commit notification; never a renderer authorization token.
#[derive(Clone, Debug)]
pub struct CliProviderChange {
    pub owner: String,
    pub provider_id: String,
    pub previous_revision: i64,
    pub current_revision: Option<i64>,
    pub kind: CliProviderChangeKind,
}

/// Host-derived identity. A fresh binding_id is required for every process opening,
/// including a resume, so a late cleanup cannot affect a replacement process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliProviderBinding {
    pub binding_id: String,
    pub owner: String,
    pub session_id: String,
    pub host_instance_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub provider_revision: i64,
}

/// The host persists these exact targets and waits for its own children outside Provider locks.
/// The callback is synchronous, must only enqueue work, and must not panic or reenter the manager.
pub trait CliProviderChangeSink: Send + Sync {
    fn changed(&self, change: &CliProviderChange, invalidated: &[CliProviderBinding]);
}

struct LeaseInner {
    binding: CliProviderBinding,
    cancellation: CancellationToken,
    spawned: AtomicBool,
}

/// Retain until the child has actually exited. Dropping a lease does not prove exit.
pub struct CliProviderLease {
    gate: Arc<CliProviderBindingGate>,
    inner: Arc<LeaseInner>,
}

impl CliProviderLease {
    pub fn binding(&self) -> &CliProviderBinding { &self.inner.binding }
    pub fn cancellation_token(&self) -> CancellationToken { self.inner.cancellation.clone() }
    pub fn is_cancelled(&self) -> bool { self.inner.cancellation.is_cancelled() }
    /// Local project/account revocation. Never cancels the caller's parent token.
    pub fn cancel(&self) { self.inner.cancellation.cancel(); }
}

#[derive(Default)]
struct Bindings {
    sink: Option<Arc<dyn CliProviderChangeSink>>,
    leases: Vec<Weak<LeaseInner>>,
}

/// Shared by every clone of a ProviderManager. credentials_lock is the asynchronous
/// commit/spawn gate; this short mutex only protects lease registration and cancellation.
#[derive(Default)]
pub struct CliProviderBindingGate {
    bindings: Mutex<Bindings>,
}

pub(crate) struct CliChangeDelivery {
    sink: Option<Arc<dyn CliProviderChangeSink>>,
    change: CliProviderChange,
    targets: Vec<CliProviderBinding>,
}

impl CliChangeDelivery {
    pub(crate) fn deliver(self) {
        if let Some(sink) = self.sink {
            sink.changed(&self.change, &self.targets);
        }
    }
}

impl CliProviderBindingGate {
    fn install(&self, sink: Arc<dyn CliProviderChangeSink>) -> DgosResult<()> {
        let mut bindings = self.bindings.lock().map_err(|_| DgosError::ProviderRequestRejected)?;
        if bindings.sink.is_some() { return Err(DgosError::ProviderRequestRejected); }
        bindings.sink = Some(sink);
        Ok(())
    }

    fn register(self: &Arc<Self>, binding: CliProviderBinding, parent: &CancellationToken) -> DgosResult<CliProviderLease> {
        let mut bindings = self.bindings.lock().map_err(|_| DgosError::ProviderRequestRejected)?;
        if bindings.sink.is_none() || parent.is_cancelled() {
            return Err(DgosError::ProviderRequestRejected);
        }
        bindings.leases.retain(|lease| lease.strong_count() > 0);
        if bindings.leases.iter().filter_map(Weak::upgrade).any(|lease| lease.binding.binding_id == binding.binding_id) {
            return Err(DgosError::ProviderRequestRejected);
        }
        let inner = Arc::new(LeaseInner { binding, cancellation: parent.child_token(), spawned: AtomicBool::new(false) });
        bindings.leases.push(Arc::downgrade(&inner));
        Ok(CliProviderLease { gate: self.clone(), inner })
    }

    pub(crate) fn committed_models(&self, change: CliProviderChange, eligible_models: &[dgos_types::Model]) -> CliChangeDelivery {
        self.committed_where(change, |binding| !eligible_models.iter().any(|model| {
            model.model_id == binding.model_id && matches!(model.model_type.as_str(), "text-generation" | "chat")
        }))
    }

    /// Called after commit, before releasing credentials_lock. Even if the host
    /// callback is delayed, opening/active leases are already cancelled here.
    pub(crate) fn committed(&self, change: CliProviderChange) -> CliChangeDelivery {
        self.committed_where(change, |_| true)
    }

    fn committed_where(&self, change: CliProviderChange, affected: impl Fn(&CliProviderBinding) -> bool) -> CliChangeDelivery {
        // A poisoned registry must still cancel every previously registered lease.
        let mut bindings = self.bindings.lock().unwrap_or_else(|error| error.into_inner());
        let mut targets = Vec::new();
        bindings.leases.retain(|weak| {
            let Some(lease) = weak.upgrade() else { return false; };
            let bound = &lease.binding;
            if change.kind != CliProviderChangeKind::MetadataChanged
                && bound.owner == change.owner && bound.provider_id == change.provider_id
                && bound.provider_revision <= change.previous_revision && !lease.cancellation.is_cancelled() && affected(bound)
            {
                lease.cancellation.cancel();
                targets.push(bound.clone());
            }
            true
        });
        CliChangeDelivery { sink: bindings.sink.clone(), change, targets }
    }
}

impl ProviderManager {
    /// Install once before declaring CLI execution available. Ordinary AI calls do
    /// not require this sink; every process lease does. All manager clones share it.
    pub fn install_cli_change_sink(&self, sink: Arc<dyn CliProviderChangeSink>) -> DgosResult<()> {
        self.cli_bindings.install(sink)
    }

    /// Register before resolving credentials. The parent is the host's account or
    /// process token; revoking one binding cannot cancel other processes or accounts.
    pub async fn register_cli_opening(&self, binding: CliProviderBinding, parent: &CancellationToken) -> DgosResult<CliProviderLease> {
        if binding.provider_revision < 0 || [&binding.binding_id, &binding.owner, &binding.session_id,
            &binding.host_instance_id, &binding.provider_id, &binding.model_id].iter().any(|value| value.is_empty())
        {
            return Err(DgosError::ProviderRequestRejected);
        }
        let _permit = self.gate.admit().map_err(|_| DgosError::ProviderRequestRejected)?;
        let _guard = self.credentials_lock.lock().await;
        self.ensure_revision(&binding.provider_id, &binding.owner, binding.provider_revision).await?;
        self.provider_repo.model_snapshot(&binding.provider_id, &binding.owner, &binding.model_id).await?;
        self.cli_bindings.register(binding, parent)
    }

    fn check_cli_lease(&self, lease: &CliProviderLease) -> DgosResult<()> {
        if !Arc::ptr_eq(&self.cli_bindings, &lease.gate) || lease.is_cancelled() {
            return Err(DgosError::ProviderConfigurationChanged);
        }
        Ok(())
    }

    pub async fn resolve_cli_provider_for_lease(&self, lease: &CliProviderLease, protocol: CliProviderProtocol) -> DgosResult<CliProviderLaunch> {
        self.check_cli_lease(lease)?;
        let binding = lease.binding();
        let launch = self.resolve_cli_provider_launch(&binding.provider_id, &binding.model_id,
            &binding.owner, binding.provider_revision, protocol).await?;
        self.check_cli_lease(lease)?;
        Ok(launch)
    }

    /// The final spawn boundary. Rechecks the exact lease/model/revision under the
    /// same lock as mutation commits. The closure MUST synchronously create/register
    /// the child, never return a future or perform network/handshake/DB work.
    /// Keep the lease and watch cancellation immediately, including while opening.
    pub async fn with_cli_spawn<T>(&self, lease: &CliProviderLease, launch: &CliProviderLaunch,
        spawn: impl FnOnce(&CliProviderLaunch) -> DgosResult<T>) -> DgosResult<T>
    {
        let _permit = self.gate.admit().map_err(|_| DgosError::ProviderRequestRejected)?;
        let _guard = self.credentials_lock.lock().await;
        self.check_cli_lease(lease)?;
        let binding = lease.binding();
        let metadata = launch.metadata();
        if metadata.user_id != binding.owner || metadata.provider_id != binding.provider_id
            || metadata.model.model_id != binding.model_id || metadata.config_revision != binding.provider_revision
        { return Err(DgosError::ProviderConfigurationChanged); }
        self.ensure_revision(&binding.provider_id, &binding.owner, binding.provider_revision).await?;
        self.provider_repo.model_snapshot(&binding.provider_id, &binding.owner, &binding.model_id).await?;
        self.check_cli_lease(lease)?;
        if lease.inner.spawned.swap(true, Ordering::AcqRel) {
            return Err(DgosError::ProviderRequestRejected);
        }
        spawn(launch)
    }

    /// Use before accepting a late ready, sending a message or approving a tool.
    /// Cancellation remains the process-level authority between these checkpoints.
    pub async fn validate_cli_lease(&self, lease: &CliProviderLease) -> DgosResult<()> {
        let _guard = self.credentials_lock.lock().await;
        self.check_cli_lease(lease)?;
        let binding = lease.binding();
        self.ensure_revision(&binding.provider_id, &binding.owner, binding.provider_revision).await?;
        self.provider_repo.model_snapshot(&binding.provider_id, &binding.owner, &binding.model_id).await?;
        self.check_cli_lease(lease)
    }

    pub(crate) fn cli_provider_committed(&self, before: &dgos_types::Provider,
        current_revision: Option<i64>, kind: CliProviderChangeKind) -> CliChangeDelivery
    {
        self.cli_bindings.committed(CliProviderChange { owner: before.user_id.clone(),
            provider_id: before.id.clone(), previous_revision: before.config_revision,
            current_revision, kind })
    }
}
