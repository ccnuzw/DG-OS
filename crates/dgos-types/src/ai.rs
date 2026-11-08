//! D136 transport-neutral AI DTOs. Identity, leases and approval tokens are never inputs.
use crate::GenerationParameters;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiSkillRef {
    pub skill_id: String,
    pub version: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiToolSelection {
    pub server_id: String,
    pub tool_name: String,
    pub schema_hash: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiRunInput {
    pub provider_id: String,
    pub model_id: String,
    pub provider_revision: i64,
    pub prompt: String,
    #[serde(default)]
    pub skill_refs: Vec<AiSkillRef>,
    #[serde(default)]
    pub tools: Vec<AiToolSelection>,
    #[serde(default)]
    pub generation: GenerationParameters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiExecutionFailure {
    InvalidInput,
    ContextTooLong,
    SelectionChanged,
    ToolCallLimit,
    GenerationLimit,
    InvalidToolProposal,
    EmptyResponse,
    ExecutionFailed,
}
impl AiExecutionFailure {
    pub const fn code(self) -> u32 {
        match self {
            Self::InvalidInput => 1000010,
            Self::ContextTooLong => 1000011,
            Self::SelectionChanged => 1000012,
            Self::ToolCallLimit => 1000013,
            Self::GenerationLimit => 1000014,
            Self::InvalidToolProposal => 1000015,
            Self::EmptyResponse => 1000016,
            Self::ExecutionFailed => 1000017,
        }
    }
    pub const fn reason(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::ContextTooLong => "context_too_long",
            Self::SelectionChanged => "selection_changed",
            Self::ToolCallLimit => "tool_call_limit",
            Self::GenerationLimit => "generation_limit",
            Self::InvalidToolProposal => "invalid_tool_proposal",
            Self::EmptyResponse => "empty_response",
            Self::ExecutionFailed => "execution_failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiRunStatus { Running, Succeeded, Failed, Cancelled, Interrupted }
impl AiRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running", Self::Succeeded => "succeeded", Self::Failed => "failed",
            Self::Cancelled => "cancelled", Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiFailure {
    pub domain: String,
    pub code: u32,
    pub reason: String,
    pub result_unknown: bool,
}
impl AiFailure {
    pub fn new(domain: &str, code: u32, reason: &str, result_unknown: bool) -> Self {
        Self { domain: domain.into(), code, reason: reason.into(), result_unknown }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiRunSummary {
    pub run_id: String,
    pub request_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub status: AiRunStatus,
    pub error: Option<AiFailure>,
    pub result_unknown: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiRunListInput {
    /// A run id from the previous page, resolved only inside the bound owner/module scope.
    pub before: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiRunPage {
    pub items: Vec<AiRunSummary>,
    pub next_before: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiTerminal {
    pub status: AiRunStatus,
    pub content: Option<String>,
    pub error: Option<AiFailure>,
    pub result_unknown: bool,
    pub audit_warning: bool,
    pub total_tokens: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiEvent {
    pub run_id: String,
    pub request_id: String,
    pub seq: u64,
    #[serde(flatten)]
    pub payload: AiEventPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiEventPayload {
    Started { provider_revision: i64, skill_refs: Vec<AiSkillRef>, tools: Vec<AiToolSelection> },
    Delta { text: String },
    ReasoningDelta { text: String },
    Phase { phase: AiPhase },
    Terminal { terminal: AiTerminal },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPhase { Model, AwaitingConfirmation, Tool }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiProviderEntry {
    pub provider_id: String,
    pub display_name: String,
    pub config_revision: i64,
    pub models: Vec<AiModelEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiModelEntry { pub model_id: String, pub display_name: String, pub kind: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiSkillEntry { pub name: String, #[serde(flatten)] pub skill: AiSkillRef }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiToolEntry {
    #[serde(flatten)] pub tool: AiToolSelection,
    pub server_name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}
