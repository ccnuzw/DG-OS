//! Renderer-visible module lifecycle contract (D034).
//!
//! These types describe what a host command may return about a module
//! lifecycle operation. They carry no runtime handle, execution context,
//! user identity, filesystem path or raw error text: the renderer only ever
//! sees an operation id, a state, stable error codes and fixed messages.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleOperationKind {
    Start,
    Stop,
    Update,
}

/// `queued → running → succeeded | failed`. A terminal state never changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleOperationState {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl ModuleOperationState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// Successful result of an operation. `UpdatedPendingRelease` means the new
/// release is active but the previous runtime's loader release is still
/// pending; a later `stop` retries it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleOperationOutcome {
    Started,
    Stopped,
    Updated,
    UpdatedPendingRelease,
    Unchanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleRuntimeStatusState {
    /// The host holds no runtime for the module.
    NotLoaded,
    Starting,
    Ready,
    Draining,
    Stopped,
    Failed,
}

/// Failure classes of module lifecycle operations. Codes live in the FR-004
/// range (`1004xxx`); messages are fixed strings so no path, URL, SQL text
/// or loader detail reaches the renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleOperationErrorKind {
    ModuleNotFound,
    AlreadyRunning,
    LoadFailed,
    VersionConflict,
    StopFailed,
    Disabled,
    UpdateFailed,
    PackageInvalid,
    NotRunning,
    OperationInProgress,
    OperationNotFound,
    HostUnavailable,
    Internal,
}

impl ModuleOperationErrorKind {
    pub fn code(self) -> u32 {
        match self {
            Self::ModuleNotFound => 1004001,
            Self::AlreadyRunning => 1004002,
            Self::LoadFailed => 1004003,
            Self::VersionConflict => 1004006,
            Self::StopFailed => 1004007,
            Self::Disabled => 1004009,
            Self::UpdateFailed => 1004013,
            Self::PackageInvalid => 1004015,
            Self::NotRunning => 1004016,
            Self::OperationInProgress => 1004017,
            Self::OperationNotFound => 1004018,
            Self::HostUnavailable => 1004019,
            Self::Internal => 1004020,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::ModuleNotFound => "Module not found",
            Self::AlreadyRunning => "Module is already running",
            Self::LoadFailed => "Module failed to load",
            Self::VersionConflict => "Module release does not match the running version",
            Self::StopFailed => "Module runtime could not be released; retry stop",
            Self::Disabled => "Module was disabled after a failed update",
            Self::UpdateFailed => "Module update failed and the previous release was restored",
            Self::PackageInvalid => "Module package is invalid or unavailable",
            Self::NotRunning => "Module is not running",
            Self::OperationInProgress => "Another operation is in progress for this module",
            Self::OperationNotFound => "Module operation not found",
            Self::HostUnavailable => "Module host is not available",
            Self::Internal => "Internal module host error",
        }
    }

    /// Whether repeating the same request later can succeed without any
    /// change by the user.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::StopFailed | Self::OperationInProgress | Self::Internal
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ModuleOperationFailure {
    pub kind: ModuleOperationErrorKind,
    pub code: u32,
    pub retryable: bool,
}

impl From<ModuleOperationErrorKind> for ModuleOperationFailure {
    fn from(kind: ModuleOperationErrorKind) -> Self {
        Self {
            kind,
            code: kind.code(),
            retryable: kind.retryable(),
        }
    }
}

/// Snapshot of one lifecycle operation. Timestamps are Unix seconds.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModuleOperationView {
    pub operation_id: String,
    pub module_id: String,
    pub kind: ModuleOperationKind,
    pub state: ModuleOperationState,
    pub outcome: Option<ModuleOperationOutcome>,
    pub failure: Option<ModuleOperationFailure>,
    pub submitted_at: i64,
    pub finished_at: Option<i64>,
}

/// Host-side view of a module runtime. `runtime_id` is an opaque label; it
/// grants nothing and no command accepts it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModuleRuntimeStatusView {
    pub module_id: String,
    pub state: ModuleRuntimeStatusState,
    pub runtime_id: Option<String>,
    pub version: Option<String>,
    pub integrity: Option<String>,
    pub in_flight: usize,
    /// Retired runtimes whose loader release is unconfirmed; `stop` retries.
    pub pending_releases: usize,
    pub active_operation_id: Option<String>,
}

/// Returned by lifecycle commands once the host accepted the operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModuleOperationAccepted {
    pub operation_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ModuleOperationErrorKind; 13] = [
        ModuleOperationErrorKind::ModuleNotFound,
        ModuleOperationErrorKind::AlreadyRunning,
        ModuleOperationErrorKind::LoadFailed,
        ModuleOperationErrorKind::VersionConflict,
        ModuleOperationErrorKind::StopFailed,
        ModuleOperationErrorKind::Disabled,
        ModuleOperationErrorKind::UpdateFailed,
        ModuleOperationErrorKind::PackageInvalid,
        ModuleOperationErrorKind::NotRunning,
        ModuleOperationErrorKind::OperationInProgress,
        ModuleOperationErrorKind::OperationNotFound,
        ModuleOperationErrorKind::HostUnavailable,
        ModuleOperationErrorKind::Internal,
    ];

    #[test]
    fn error_codes_are_unique_and_inside_the_module_range() {
        let mut codes: Vec<u32> = ALL.iter().map(|kind| kind.code()).collect();
        assert!(codes.iter().all(|code| (1004001..=1004999).contains(code)));
        // 1004004 is the PermissionDenied wire code; it must never be reused.
        assert!(!codes.contains(&1004004));
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), ALL.len());
    }

    #[test]
    fn messages_are_fixed_and_leak_no_local_detail() {
        for kind in ALL {
            let message = kind.message();
            assert!(!message.is_empty());
            assert!(!message.contains('/') && !message.contains('\\'));
        }
    }

    #[test]
    fn only_release_and_contention_failures_are_retryable() {
        let retryable: Vec<_> = ALL.iter().filter(|kind| kind.retryable()).collect();
        assert_eq!(
            retryable,
            [
                &ModuleOperationErrorKind::StopFailed,
                &ModuleOperationErrorKind::OperationInProgress,
                &ModuleOperationErrorKind::Internal,
            ]
        );
    }

    #[test]
    fn only_succeeded_and_failed_are_terminal() {
        assert!(!ModuleOperationState::Queued.is_terminal());
        assert!(!ModuleOperationState::Running.is_terminal());
        assert!(ModuleOperationState::Succeeded.is_terminal());
        assert!(ModuleOperationState::Failed.is_terminal());
    }
}
