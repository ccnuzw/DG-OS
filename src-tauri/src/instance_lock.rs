//! One running application per data directory (D077).
//!
//! Startup recovery marks every `running`/`generating` record as interrupted, which is only
//! correct if no other process is working on the same database. The first process takes an
//! exclusive lock on `dgos.lock` before it opens the database and holds it until it exits; the
//! kernel releases it if the process is killed. A second process gets `WouldBlock` and must exit
//! without touching the database.
//!
//! Limits: the lock is advisory, it does not focus the first window, and its behaviour on network
//! file systems is not guaranteed. Needs Rust 1.89+ (`File::try_lock`).

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

pub const LOCK_FILE: &str = "dgos.lock";

/// Held for the life of the process. Dropping it releases the lock.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

#[derive(Debug)]
pub enum LockError {
    /// Another process holds the lock.
    AlreadyRunning,
    Io(io::Error),
}

impl InstanceLock {
    pub fn acquire(dir: &Path) -> Result<Self, LockError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK_FILE))
            .map_err(LockError::Io)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(LockError::AlreadyRunning),
            Err(TryLockError::Error(error)) => Err(LockError::Io(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_is_refused_until_the_first_releases() {
        let dir = tempfile::tempdir().unwrap();
        let first = InstanceLock::acquire(dir.path()).unwrap();
        assert!(matches!(
            InstanceLock::acquire(dir.path()),
            Err(LockError::AlreadyRunning)
        ));
        drop(first);
        assert!(
            InstanceLock::acquire(dir.path()).is_ok(),
            "released with the first holder"
        );
    }

    #[test]
    fn different_data_directories_do_not_block_each_other() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let _first = InstanceLock::acquire(a.path()).unwrap();
        assert!(InstanceLock::acquire(b.path()).is_ok());
    }

    #[test]
    fn a_missing_directory_is_an_io_error_not_a_lock_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("missing");
        assert!(matches!(
            InstanceLock::acquire(&gone),
            Err(LockError::Io(_))
        ));
    }
}
