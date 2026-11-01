//! Audit sink (B10-65, B10-66). Recording never blocks a request and never
//! changes its outcome: events go through a bounded queue to one writer task;
//! a full queue or a failed write is only a `tracing::warn!` with the event kind.

use dgos_db::{LanAuditRecord, LanAuditRepository};
use tokio::sync::mpsc;

pub trait AuditSink: Send + Sync + 'static {
    fn record(&self, record: LanAuditRecord);
}

const QUEUE: usize = 1024;

/// SQLite sink. One writer keeps merges and trimming in order.
pub struct DbAuditSink {
    tx: mpsc::Sender<LanAuditRecord>,
}

impl DbAuditSink {
    /// Must be called inside a Tokio runtime.
    pub fn spawn(repository: LanAuditRepository) -> Self {
        let (tx, mut rx) = mpsc::channel::<LanAuditRecord>(QUEUE);
        tokio::spawn(async move {
            while let Some(record) = rx.recv().await {
                let kind = record.event.as_str();
                if repository.append(record).await.is_err() {
                    tracing::warn!(event = kind, "LAN audit write failed");
                }
            }
        });
        Self { tx }
    }
}

impl AuditSink for DbAuditSink {
    fn record(&self, record: LanAuditRecord) {
        let kind = record.event.as_str();
        if self.tx.try_send(record).is_err() {
            tracing::warn!(event = kind, "LAN audit queue full; event dropped");
        }
    }
}

/// Collects records in memory (tests and hosts without storage).
#[derive(Default)]
pub struct MemoryAudit(pub std::sync::Mutex<Vec<LanAuditRecord>>);

impl MemoryAudit {
    pub fn take(&self) -> Vec<LanAuditRecord> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl AuditSink for MemoryAudit {
    fn record(&self, record: LanAuditRecord) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(record);
    }
}

impl<T: AuditSink> AuditSink for std::sync::Arc<T> {
    fn record(&self, record: LanAuditRecord) {
        (**self).record(record)
    }
}
