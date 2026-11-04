//! Independent native Codex and Claude session backends, supervised by the host.
pub mod cli;
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
pub mod runtime;
pub use manager::{AgentManager, OwnedSessionView, SessionRecord};
pub use executions::RunRecord;
pub use session::{Engine, PendingRequest, SessionEvent, SessionOptions, SessionView};
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod shutdown_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod project_tests;
#[cfg(test)]
mod file_review_tests;
