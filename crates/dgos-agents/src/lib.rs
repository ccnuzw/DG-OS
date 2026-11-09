//! Independent native Codex and Claude session backends, supervised by the host.
pub mod cli;
pub mod cli_updates;
pub mod capabilities;
mod claude_session;
mod codex_session;
mod manager;
mod executions;
pub mod projects;
pub mod session;
mod transport;
mod diagnostics;
pub mod files;
pub mod review;
pub mod references;
pub mod runtime;
pub mod workspaces;
pub mod git;
pub mod handoffs;
pub mod takeover;
pub mod native_extensions;
pub mod native_session;
pub use git::{GitHistoryEntry, GitHistoryInput, GitHistoryPage};
pub use manager::{AgentManager, OwnedSessionView, SessionRecord};
pub use projects::NativeProjectAdmission;
pub use executions::RunRecord;
pub use session::{Engine, PendingRequest, SessionEvent, SessionOptions, SessionView};
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod native_session_tests;
#[cfg(test)]
mod shutdown_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod project_tests;
#[cfg(test)]
mod file_review_tests;
#[cfg(test)]
mod recovery_tests;
