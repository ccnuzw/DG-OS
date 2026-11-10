//! Typed, owner-scoped Codex conversation operations. Native identifiers are observations, not authority.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ReviewTarget {
    #[serde(rename = "uncommittedChanges")] UncommittedChanges,
    #[serde(rename = "baseBranch")] BaseBranch { branch: String },
    #[serde(rename = "commit")] Commit { sha: String, title: Option<String> },
    #[serde(rename = "custom")] Custom { instructions: String },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodexAction {
    Review { target: ReviewTarget }, Compact {},
    Fork { new_session_id: String, last_turn_id: Option<String> },
    Revert { before_turn_id: String },
    InterruptChild { child_thread_id: String, turn_id: String },
    SendChild { child_thread_id: String, text: String },
    ResumeChild { child_thread_id: String }, CloseChild { child_thread_id: String },
    TerminateTerminal { process_id: String },
    QueueAdd { text: String }, QueueUpdate { submission_id: String, text: String },
    QueueDelete { submission_id: String }, QueueReorder { submission_ids: Vec<String> },
    QueueStart { submission_id: Option<String> },
}
impl CodexAction {
    pub(crate) fn starts_run(&self) -> bool { matches!(self, Self::Review { .. } | Self::Compact {} | Self::QueueStart { .. }) }
    pub(crate) fn validate(&self) -> Result<(), String> {
        fn text(value: &str) -> bool { !value.trim().is_empty() && value.len() <= 65536 }
        fn id(value: &str) -> bool { !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control) }
        let valid = match self {
            Self::Review { target } => match target {
                ReviewTarget::UncommittedChanges => true,
                ReviewTarget::BaseBranch { branch } => id(branch),
                ReviewTarget::Commit { sha, title } => (7..=64).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit()) && title.as_ref().is_none_or(|s| s.len() <= 1024),
                ReviewTarget::Custom { instructions } => text(instructions),
            },
            Self::Fork { new_session_id, last_turn_id } => uuid::Uuid::parse_str(new_session_id).is_ok() && last_turn_id.as_ref().is_none_or(|s| id(s)),
            Self::Revert { before_turn_id } => id(before_turn_id),
            Self::InterruptChild { child_thread_id, turn_id } => id(child_thread_id) && id(turn_id),
            Self::SendChild { child_thread_id, text: input } => id(child_thread_id) && text(input),
            Self::ResumeChild { child_thread_id } | Self::CloseChild { child_thread_id } => id(child_thread_id),
            Self::TerminateTerminal { process_id } => id(process_id),
            Self::QueueAdd { text: input } => text(input),
            Self::QueueUpdate { submission_id, text: input } => id(submission_id) && text(input),
            Self::QueueDelete { submission_id } => id(submission_id),
            Self::QueueReorder { submission_ids } => submission_ids.len() <= 256 && submission_ids.iter().all(|s| id(s)) && submission_ids.iter().collect::<std::collections::HashSet<_>>().len() == submission_ids.len(),
            Self::QueueStart { submission_id } => submission_id.as_ref().is_none_or(|s| id(s)),
            Self::Compact {} => true,
        };
        if valid { Ok(()) } else { Err("invalid_codex_operation".into()) }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexScope { pub session_id: String, pub generation: String }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCodexOperation { pub session_id: String, pub generation: String, pub request_id: String, pub action: CodexAction }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectCodexOperation { pub session_id: String, pub generation: String, pub request_id: String }
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadCodexChildHistory { pub session_id: String, pub generation: String, pub child_thread_id: String }
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexOperationState { Applied, Unknown }
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodexOperationReceipt {
    pub session_id: String, pub generation: String, pub request_id: String,
    pub state: CodexOperationState,
    #[serde(skip_serializing_if = "Option::is_none")] pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub fork_session_id: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CodexOperationsSnapshot {
    pub session_id: String, pub generation: String, pub children: Vec<CodexChildAgent>,
    pub terminals: Vec<CodexBackgroundTerminal>, pub queue: Vec<CodexQueueEntry>,
    pub turns: Vec<CodexHistoryTurn>, pub unsupported: Vec<String>, pub incomplete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodexChildAgent { pub thread_id: String, pub parent_thread_id: String, pub status: String, pub turn_id: Option<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodexBackgroundTerminal { pub process_id: String, pub command: String, pub status: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodexQueueEntry { pub id: String, pub text: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodexHistoryTurn { pub id: String, pub status: String, pub items: Vec<CodexHistoryItem> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodexHistoryItem { pub id: String, #[serde(rename = "type")] pub kind: String, pub text: Option<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodexChildHistory { pub thread_id: String, pub turns: Vec<CodexHistoryTurn>, pub incomplete: bool }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn reorder_rejects_duplicates_and_excess() {
        assert!(CodexAction::QueueReorder { submission_ids: vec!["a".into(), "a".into()] }.validate().is_err());
        assert!(CodexAction::QueueReorder { submission_ids: vec!["a".into()] }.validate().is_ok());
    }
    #[test] fn caller_cannot_smuggle_rpc_or_paths() {
        assert!(serde_json::from_str::<CodexAction>(r#"{"kind":"fork","new_session_id":"x","last_turn_id":null,"path":"/tmp/other"}"#).is_err());
        assert!(serde_json::from_str::<CodexAction>(r#"{"kind":"rpc","method":"thread/read"}"#).is_err());
        assert!(CodexAction::Review { target: ReviewTarget::Commit { sha: "--exec=sh".into(), title: None } }.validate().is_err());
    }
}
