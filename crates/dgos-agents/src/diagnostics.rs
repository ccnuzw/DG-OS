//! Fixed-field process diagnostics. Never record executable paths, argv, IO or account data.
use std::{sync::{Arc, atomic::{AtomicU64, Ordering}}, time::Instant};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(crate) struct ProcessDiagnostic(Arc<Context>);
struct Context {
    id: u64,
    started: Instant,
    operation: &'static str,
    tool: &'static str,
}
impl ProcessDiagnostic {
    pub(crate) fn new(operation: &'static str, tool: &'static str) -> Self {
        Self(Arc::new(Context {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
            operation,
            tool,
        }))
    }
    pub(crate) fn phase(&self, stage: &'static str, pid: Option<u32>) -> Phase {
        let phase = Phase { diagnostic: self.clone(), stage, pid, finished: false };
        self.record(stage, pid, "waiting", "none");
        phase
    }
    pub(crate) fn record(&self, stage: &'static str, pid: Option<u32>, outcome: &'static str, error_class: &'static str) {
        tracing::info!(target: "dgos_agents::process",
            correlation_id = self.0.id, host_pid = std::process::id(), child_pid = ?pid,
            elapsed_ms = self.0.started.elapsed().as_millis() as u64,
            operation = self.0.operation, tool = self.0.tool, stage, outcome, error_class,
            "agent process phase");
    }
}

/// Logs cancellation when a future is dropped while a phase is pending.
pub(crate) struct Phase {
    diagnostic: ProcessDiagnostic,
    stage: &'static str,
    pid: Option<u32>,
    finished: bool,
}
impl Phase {
    pub(crate) fn finish(mut self, error: Option<&str>) {
        self.finished = true;
        self.diagnostic.record(self.stage, self.pid, if error.is_some() { "failed" } else { "completed" }, error.map_or("none", error_class));
    }
}
impl Drop for Phase {
    fn drop(&mut self) {
        if !self.finished {
            self.diagnostic.record(self.stage, self.pid, "cancelled", "cancelled");
        }
    }
}

// Return only compile-time literals, even when a caller supplies an engine error.
fn error_class(error: &str) -> &'static str {
    match error {
        "not_executable" => "not_executable",
        "not_installed" => "not_installed",
        "invalid_executable" => "invalid_executable",
        "adapter_unavailable" => "adapter_unavailable",
        "bridge_unavailable" => "bridge_unavailable",
        "spawn_failed" | "start_failed" => "spawn_failed",
        "execution_failed" => "execution_failed",
        "output_limit" => "output_limit",
        "protocol_unsupported" => "protocol_unsupported",
        "invalid_output" => "invalid_output",
        "timeout" => "timeout",
        "result_unknown" => "result_unknown",
        "connection_lost" => "connection_lost",
        "engine_rejected" => "engine_rejected",
        "closed" => "closed",
        "cancelled" => "cancelled",
        "cleanup_pending" => "cleanup_pending",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unknown_errors_never_become_log_fields() {
        assert_eq!(super::error_class("/private/token=secret"), "other");
        assert_eq!(super::error_class("engine_rejected"), "engine_rejected");
    }
}
