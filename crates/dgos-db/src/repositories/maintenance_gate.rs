//! Process-wide admission gate for shared-database maintenance (D133 item 5).
//!
//! Every long-running writer or runner (chat generation, single task, Agent
//! run, skill install or publish, module storage write, provider and MCP
//! calls) holds an `ActivityPermit` for its whole lifetime. VACUUM takes the
//! `MaintenanceLease` with a non-blocking `try_write`, so it either sees no
//! activity at all or fails at once with `MaintenanceBusy`. Admission never
//! waits behind VACUUM, so a sample taken earlier cannot be acted on later:
//! the check and the VACUUM happen under the same lock.
//!
//! The instance lock keeps one app process per database, so this gate covers
//! every writer that can reach the database. SQLite locking still applies
//! inside the database as a second barrier; a SQLite busy error during VACUUM
//! is reported as busy, never retried.

use std::sync::Arc;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaintenanceBusy;

impl std::fmt::Display for MaintenanceBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("database maintenance is running or activity is in progress")
    }
}

#[derive(Clone, Default)]
pub struct MaintenanceGate {
    inner: Arc<RwLock<()>>,
}

/// Held by one unit of activity. Dropping it releases the admission.
pub struct ActivityPermit {
    _guard: OwnedRwLockReadGuard<()>,
}

/// Held by VACUUM. While it exists, no `ActivityPermit` can be granted.
pub struct MaintenanceLease {
    _guard: OwnedRwLockWriteGuard<()>,
}

impl MaintenanceGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Admits one unit of activity, or fails at once during maintenance.
    pub fn admit(&self) -> Result<ActivityPermit, MaintenanceBusy> {
        self.inner
            .clone()
            .try_read_owned()
            .map(|guard| ActivityPermit { _guard: guard })
            .map_err(|_| MaintenanceBusy)
    }

    /// Takes exclusive maintenance. Fails at once when any activity holds a
    /// permit or another maintenance is running.
    pub fn begin_maintenance(&self) -> Result<MaintenanceLease, MaintenanceBusy> {
        self.inner
            .clone()
            .try_write_owned()
            .map(|guard| MaintenanceLease { _guard: guard })
            .map_err(|_| MaintenanceBusy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_blocks_maintenance_until_its_permit_drops() {
        let gate = MaintenanceGate::new();
        let permit = gate.admit().unwrap();
        assert!(gate.begin_maintenance().is_err());
        drop(permit);
        assert!(gate.begin_maintenance().is_ok());
    }

    #[test]
    fn maintenance_refuses_new_activity_until_its_lease_drops() {
        let gate = MaintenanceGate::new();
        let lease = gate.begin_maintenance().unwrap();
        assert!(gate.admit().is_err());
        assert!(gate.begin_maintenance().is_err());
        drop(lease);
        assert!(gate.admit().is_ok());
    }

    #[test]
    fn many_permits_are_all_counted_before_maintenance() {
        let gate = MaintenanceGate::new();
        let first = gate.admit().unwrap();
        let second = gate.admit().unwrap();
        drop(first);
        assert!(gate.begin_maintenance().is_err());
        drop(second);
        assert!(gate.begin_maintenance().is_ok());
    }
}
