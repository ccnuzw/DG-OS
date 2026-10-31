use serde::{Deserialize, Serialize};

/// Provider capability types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    TextGeneration,
    ImageGeneration,
    VideoGeneration,
    AudioGeneration,
    Streaming,
    Vision,
}

/// Model information
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub id: String,                     // UUID for database
    pub provider_id: String,            // Provider UUID
    pub model_id: String,               // API identifier (e.g., "gpt-4")
    pub model_name: String,             // Display name (e.g., "GPT-4")
    pub model_type: String, // Carries the kind (`text-generation`, ...); legacy `chat` reads as text-generation
    pub context_window: Option<i32>, // Max context tokens
    pub max_output_tokens: Option<i32>, // Max output tokens
    pub supports_streaming: bool, // Supports streaming
    pub supports_vision: bool, // Supports vision/images
    pub cached_at: String,  // Last fetch timestamp
    /// How `model_type` was decided. Host-derived; a submitted value is never trusted.
    #[serde(default)]
    pub kind_source: crate::model_catalog::KindSource,
    #[serde(default)]
    pub source: crate::model_catalog::ModelSource,
    #[serde(default)]
    pub capabilities: crate::model_catalog::ModelCapabilities,
}

/// Chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Reasoning request (FR-001 §6.3). The effort string goes to the upstream as written; the host
/// keeps no per-model list of valid levels. A profile without a mapping for the level rejects
/// the request locally.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Only for profiles that control thinking by token budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

/// Structured output request (`type` is always `json_schema`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// Reasoning content of one assistant message, exactly as the upstream produced it.
///
/// Held in memory for one tool loop only. It is never serialized (not to the renderer, not to
/// SQLite) and `Debug` prints sizes, not content, so it cannot reach a log line.
#[derive(Clone, Default, PartialEq)]
pub struct MessageReasoning {
    /// Displayable text.
    pub text: String,
    /// `openai_compatible`: the reasoning fields the upstream sent (`reasoning_content`,
    /// `reasoning`, `reasoning_details`), in the order found, to be sent back under the same names.
    pub fields: Vec<(String, serde_json::Value)>,
    /// `anthropic`: `thinking` and `redacted_thinking` blocks, signatures included, to be sent
    /// back verbatim at the start of the assistant turn.
    pub blocks: Vec<serde_json::Value>,
}

impl MessageReasoning {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.fields.is_empty() && self.blocks.is_empty()
    }

    /// Size of what would be sent back upstream, for context budgeting.
    pub fn encoded_len(&self) -> usize {
        self.fields
            .iter()
            .map(|(name, value)| name.len() + value.to_string().len())
            .chain(self.blocks.iter().map(|block| block.to_string().len()))
            .sum()
    }
}

impl std::fmt::Debug for MessageReasoning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageReasoning")
            .field("text_bytes", &self.text.len())
            .field("fields", &self.fields.len())
            .field("blocks", &self.blocks.len())
            .finish()
    }
}

/// Chat request parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    /// Non-streaming calls only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

/// Chat response
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub usage: TokenUsage,
    #[serde(default)]
    pub audit_warning: bool,
    /// Normalised to `stop`, `length`, `tool_calls` or `content_filter`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// The upstream sent no usage; `usage` is zero rather than measured (§2.4.7).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub usage_unknown: bool,
    /// Displayable reasoning text; never mixed into `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// The parsed object, present only when it passed the requested schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
}

/// Structured tool chat stays separate from the existing text-only contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Rust-internal (FR-001 §6.3): never serialized, so it cannot reach the renderer, SQLite
    /// or a request log. Adapters send it back to the upstream explicitly.
    #[serde(skip)]
    pub reasoning: Option<MessageReasoning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChatRequest {
    pub messages: Vec<ToolMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChatResponse {
    pub message: ToolMessage,
    pub usage: TokenUsage,
    #[serde(default)]
    pub audit_warning: bool,
}

/// Token usage information
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Stream chunk for streaming responses
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatChunk {
    pub delta: String,
    pub finish_reason: Option<String>,
    /// Present only on the terminal chunk; `None` there means the upstream reported no usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /// Displayable reasoning text increment; never mixed into `delta`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_delta: Option<String>,
}

/// What `call_provider_stream` pushes to the caller's Channel (FR-001 §6.3). Exactly one `Done`
/// or `Error` ends a stream; nothing follows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderStreamEvent {
    Delta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    Done {
        finish_reason: String,
        usage: TokenUsage,
    },
    Error {
        code: u32,
        message: String,
    },
}

impl ProviderStreamEvent {
    /// The events one upstream chunk stands for, in order. Empty text produces no event; a chunk
    /// with a finish reason ends the stream and carries the upstream's usage (0 when it gave
    /// none).
    pub fn from_chunk(chunk: ChatChunk) -> Vec<Self> {
        let mut events = Vec::new();
        if let Some(text) = chunk.reasoning_delta.filter(|text| !text.is_empty()) {
            events.push(Self::ReasoningDelta { text });
        }
        if !chunk.delta.is_empty() {
            events.push(Self::Delta { text: chunk.delta });
        }
        if let Some(finish_reason) = chunk.finish_reason {
            events.push(Self::Done {
                finish_reason,
                usage: chunk.usage.unwrap_or_default(),
            });
        }
        events
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Error { .. })
    }
}

#[cfg(test)]
mod stream_event_tests {
    use super::*;

    #[test]
    fn a_chunk_maps_to_ordered_events_and_only_the_finish_reason_ends_the_stream() {
        let events = ProviderStreamEvent::from_chunk(ChatChunk {
            delta: "hi".into(),
            reasoning_delta: Some("think".into()),
            finish_reason: Some("stop".into()),
            usage: Some(TokenUsage {
                prompt_tokens: 3,
                completion_tokens: 2,
                total_tokens: 5,
            }),
        });
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0],
            ProviderStreamEvent::ReasoningDelta {
                text: "think".into()
            }
        );
        assert_eq!(events[1], ProviderStreamEvent::Delta { text: "hi".into() });
        assert!(events[2].is_terminal());
        assert!(!events[0].is_terminal() && !events[1].is_terminal());
        assert!(ProviderStreamEvent::from_chunk(ChatChunk::default()).is_empty());
        let json = serde_json::to_value(&events[2]).unwrap();
        assert_eq!(json["kind"], "done");
        assert_eq!(json["usage"]["total_tokens"], 5);
    }
}

/// Provider configuration
#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("name", &self.name)
            .field("provider_type", &self.provider_type)
            .field("api_key", &"[redacted]")
            .finish_non_exhaustive()
    }
}

/// Provider entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    /// Host vault reference; the secret itself is never serialized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    pub credential_status: CredentialStatus,
    pub config_revision: i64,
    pub enabled: bool,
    pub sort_order: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Stored,
    MigrationRequired,
    Missing,
}

/// Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 8000,
        }
    }
}

/// Usage log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageLog {
    pub id: String,
    pub provider_id: String,
    pub model_id: String,
    pub user_id: String,
    pub request_tokens: u32,
    pub response_tokens: u32,
    pub total_tokens: u32,
    pub latency_ms: u32,
    pub success: bool,
    pub error_code: Option<String>,
    pub caller: Option<String>,
    pub request_id: Option<String>,
    /// D081: whether the upstream reported usage. `Some(false)` means the token counts are
    /// unknown (the call may still have been billed); `None` is a row older than the field.
    pub usage_reported: Option<bool>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One endpoint of a vendor: the adapter it uses and where it lives by default (§6.3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderVendorEndpoint {
    /// Adapter: `openai_compatible` or `anthropic`.
    pub provider_type: String,
    pub display_name: String,
    /// `None` for the `custom` vendor: the user must supply the address.
    pub default_base_url: Option<String>,
}

/// Public fields of a built-in vendor entry. The vendor table itself lives in `dgos-providers`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderVendor {
    pub vendor_id: String,
    pub display_name: String,
    pub endpoints: Vec<ProviderVendorEndpoint>,
}
