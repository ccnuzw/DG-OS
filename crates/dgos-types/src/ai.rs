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

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiSkillMode { #[default] Manual, OnDemand }

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiContextMode { #[default] Inline, OnDemand }

fn default_context_bytes() -> u32 { 8192 }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiContextRef {
    pub resource_id: String,
    pub sha256: String,
    #[serde(default)] pub start: u32,
    #[serde(default = "default_context_bytes")] pub max_bytes: u32,
    #[serde(default)] pub mode: AiContextMode,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiResourceImport { pub name: String, pub mime: String, pub base64: String }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiResource {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub bytes_len: u64,
    pub sha256: String,
    pub expires_at: i64,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewChatResourceInput {
    pub session_id: String,
    pub resource_id: String,
    pub sha256: String,
}

/// Full, bounded resource content for its owning chat; deliberately not `Debug`.
#[derive(Clone, Serialize)]
pub struct ChatResourcePreview {
    pub base64: String,
    pub mime: String,
    pub start: u32,
    pub bytes: u32,
    pub total_bytes: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AiOutputContract {
    #[default] Text,
    JsonSchema { schema: serde_json::Value },
}

impl<'de> Deserialize<'de> for AiOutputContract {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde ignores extra fields on internally tagged unit variants even with
        // deny_unknown_fields. An empty struct variant enforces the same strict
        // wire shape while preserving the public AiOutputContract::Text API.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Text {},
            JsonSchema { schema: serde_json::Value },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Text {} => Self::Text,
            Wire::JsonSchema { schema } => Self::JsonSchema { schema },
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiOperation { #[default] Generate, Summarize }

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AiFoundationOptions {
    pub skill_mode: AiSkillMode,
    pub context_refs: Vec<AiContextRef>,
    pub output: AiOutputContract,
    pub retain_result: bool,
    pub source_run_id: Option<String>,
    pub operation: AiOperation,
    pub confirmed: bool,
}

/// Coverage of bytes actually supplied, not a claim that the model understood them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiContextReceipt {
    pub resource_id: String,
    pub sha256: String,
    pub start: u32,
    pub bytes: u32,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiRunContent {
    pub run_id: String,
    pub content: Option<String>,
    pub status: String,
    pub output_valid: bool,
    pub expires_at: i64,
    pub acknowledged_at: Option<i64>,
    pub source_run_id: Option<String>,
    pub context_receipts: Vec<AiContextReceipt>,
    pub tool_receipts: serde_json::Value,
    pub skill_receipts: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiContextBudget {
    pub text_bytes: usize,
    pub image_reserve_tokens: usize,
    pub tool_schema_bytes: usize,
    pub host_text_limit_bytes: usize,
    pub model_window_tokens: Option<i32>,
    pub reserved_output_tokens: u32,
    pub estimated_input_tokens: usize,
    pub can_send: bool,
    pub rejection: Option<String>,
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
    #[serde(default)]
    pub foundation: AiFoundationOptions,
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
    ModelSegment { round: u32 },
    Phase { phase: AiPhase },
    Terminal { terminal: AiTerminal },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPhase { Model, AwaitingConfirmation, Tool, ProcessingResult, FinalAnswer }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiProviderEntry {
    pub provider_id: String,
    pub display_name: String,
    pub config_revision: i64,
    pub models: Vec<AiModelEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiModelEntry {
    pub model_id: String,
    pub display_name: String,
    pub kind: String,
    pub capabilities: crate::ModelCapabilities,
    pub context_window: Option<i32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiSkillEntry { pub name: String, #[serde(flatten)] pub skill: AiSkillRef }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiToolEntry {
    #[serde(flatten)] pub tool: AiToolSelection,
    pub server_name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

#[cfg(test)]
mod foundation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_inputs_default_to_explicit_no_extra_work() {
        let input: AiRunInput = serde_json::from_value(json!({"provider_id":"p","model_id":"m","provider_revision":1,"prompt":"text"})).unwrap();
        assert_eq!(input.foundation, AiFoundationOptions::default());
        assert_eq!(input.foundation.skill_mode, AiSkillMode::Manual);
        assert!(!input.foundation.retain_result);
        assert!(!input.foundation.confirmed);
        assert_eq!(input.foundation.operation, AiOperation::Generate);
        let reference: AiContextRef = serde_json::from_value(json!({"resource_id":"r","sha256":"h"})).unwrap();
        assert_eq!(reference.start, 0);
        assert_eq!(reference.max_bytes, 8192);
        assert_eq!(reference.mode, AiContextMode::Inline);
    }

    #[test]
    fn foundation_inputs_do_not_accept_authority_fields_or_unknown_contract_fields() {
        assert!(serde_json::from_value::<AiContextRef>(json!({"resource_id":"r","sha256":"h","scope":"module:other"})).is_err());
        assert!(serde_json::from_value::<AiFoundationOptions>(json!({"user":"other"})).is_err());
        assert!(serde_json::from_value::<AiOutputContract>(json!({"kind":"text","repair":true})).is_err());
        assert!(serde_json::from_value::<AiOutputContract>(json!({"kind":"text","schema":{}})).is_err());
        assert!(serde_json::from_value::<AiOutputContract>(json!({"kind":"json_schema","schema":{},"repair":true})).is_err());
        assert_eq!(serde_json::from_value::<AiOutputContract>(json!({"kind":"text"})).unwrap(), AiOutputContract::Text);
        assert_eq!(serde_json::to_value(AiOutputContract::Text).unwrap(), json!({"kind":"text"}));
        let contract = AiOutputContract::JsonSchema { schema: json!({"type":"object"}) };
        assert_eq!(serde_json::to_value(contract).unwrap(), json!({"kind":"json_schema","schema":{"type":"object"}}));
    }
}
