//! `anthropic` adapter (Anthropic Messages). Also used for third-party services that offer an
//! Anthropic-compatible endpoint. Request shape differences are read from the [`Profile`].
//!
//! Everything here follows the vendors' public documentation (technical design "协议映射",
//! read 2026-10-29). No call to a real service has been made.

use async_trait::async_trait;
use dgos_types::{
    build_model, valid_model_id, ChatChunk, ChatRequest, ChatResponse, DgosError, DgosResult, Hint,
    MessageReasoning, Model, ModelSource, RetryConfig, TokenUsage, ToolCall, ToolChatRequest,
    ToolChatResponse, ToolFunctionCall, ToolMessage, PROTOCOL_ANTHROPIC,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use tracing::debug;

use super::preflight;
use crate::sse::SseEvent;
use crate::stream::{sse_stream, Mapper, Outcome};
use crate::structured;
use crate::traits::{BoxStream, CallFailure, ProviderAdapter};
use crate::transport::{AdapterOptions, BodyState, Transport};
use crate::upstream::{read_limited, ANTHROPIC_DOWNGRADABLE};
use crate::vendor::{
    normalize_anthropic_base, ModelList, Profile, ReasoningParams, StructuredOutput,
    ANTHROPIC_DEFAULT_MAX_TOKENS, ANTHROPIC_GENERIC,
};

/// Page size of the model list and the cap on pages followed (a service that keeps saying
/// `has_more` cannot loop the host).
const MODELS_PAGE_LIMIT: usize = 100;
const MAX_MODEL_PAGES: usize = 50;

pub struct AnthropicAdapter {
    transport: Transport,
}

impl AnthropicAdapter {
    /// Generic profile: only standard fields are sent.
    pub fn new(base_url: String, api_key: String, retry_config: Option<RetryConfig>) -> Self {
        Self::with_options(base_url, api_key, retry_config, AdapterOptions::default())
    }

    pub fn with_options(
        base_url: String,
        api_key: String,
        retry_config: Option<RetryConfig>,
        options: AdapterOptions,
    ) -> Self {
        Self::with_profile(base_url, api_key, retry_config, options, &ANTHROPIC_GENERIC)
    }

    /// The caller resolves the profile from the address (`vendor::resolve_profile`); the adapter
    /// never picks one itself, so a profile cannot change where requests go. A trailing `/v1` on
    /// `base_url` is removed (§2.4.6 #2).
    pub fn with_profile(
        base_url: String,
        api_key: String,
        retry_config: Option<RetryConfig>,
        options: AdapterOptions,
        profile: &'static Profile,
    ) -> Self {
        Self {
            transport: Transport::new(
                normalize_anthropic_base(&base_url),
                api_key,
                retry_config,
                options,
                profile,
                ANTHROPIC_DOWNGRADABLE,
            ),
        }
    }

    fn profile(&self) -> &'static Profile {
        self.transport.profile
    }

    /// All pages, followed with `after_id`. `Ok(None)`: the service has no model list (404 or
    /// 405 on the first page).
    async fn fetch_models(&self) -> DgosResult<Option<Vec<Model>>> {
        if self.profile().models != ModelList::AfterId {
            return Ok(None);
        }
        let options = &self.transport.options;
        let mut models: Vec<Model> = Vec::new();
        let mut seen = HashSet::new();
        let mut after: Option<String> = None;
        for page_number in 0..MAX_MODEL_PAGES {
            let mut path = format!("/v1/models?limit={MODELS_PAGE_LIMIT}");
            if let Some(after) = &after {
                path.push_str(&format!("&after_id={}", encode_cursor(after)));
            }
            let url = self.transport.url(&path)?;
            let Some(bytes) = self.transport.get_models_page(&url).await? else {
                // Missing on the first page: no list. Missing later: the answer is inconsistent.
                return if page_number == 0 {
                    Ok(None)
                } else {
                    Err(DgosError::ProviderResponseInvalid)
                };
            };
            let page: Value =
                serde_json::from_slice(&bytes).map_err(|_| DgosError::ProviderResponseInvalid)?;
            let items = page
                .get("data")
                .and_then(Value::as_array)
                .ok_or(DgosError::ProviderResponseInvalid)?;
            for item in items {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if !valid_model_id(id) || !seen.insert(id.to_owned()) {
                    continue;
                }
                models.push(model_from_item(id, item));
                if models.len() >= options.max_models {
                    return Ok(Some(models));
                }
            }
            let has_more = page
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let last = page
                .get("last_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned);
            match last {
                Some(last) if has_more && after.as_deref() != Some(last.as_str()) => {
                    after = Some(last)
                }
                _ => return Ok(Some(models)),
            }
        }
        Ok(Some(models))
    }

    /// One minimal request to `model_id`; any 2xx proves the endpoint, key and model work.
    async fn probe(&self, model_id: &str) -> DgosResult<()> {
        let url = self.transport.url("/v1/messages")?;
        let state = BodyState::new(json!({
            "model": model_id,
            "max_tokens": 16,
            "messages": [{"role": "user", "content": [{"type": "text", "text": "ping"}]}]
        }));
        self.transport
            .post_json(
                &url,
                &state,
                model_id,
                Some(self.transport.options.request_timeout),
                None,
            )
            .await
            .map(|_| ())
            .map_err(|failure| failure.error)
    }
}

/// Cursor ids are model ids; keep the query well-formed whatever they contain.
fn encode_cursor(id: &str) -> String {
    id.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// Only documented `/v1/models` fields are read: `display_name`, `max_input_tokens`, `max_tokens`
/// and the `capabilities.{thinking, structured_output, image_input}.supported` flags.
fn model_from_item(id: &str, item: &Value) -> Model {
    let mut model = build_model(
        PROTOCOL_ANTHROPIC,
        id,
        item.get("display_name").and_then(Value::as_str),
        ModelSource::Remote,
        None,
    );
    let number = |key: &str| {
        item.get(key)
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .map(|value| value.min(i32::MAX as u64) as i32)
    };
    model.context_window = number("max_input_tokens");
    model.max_output_tokens = number("max_tokens");
    let hint = |path: &str| match item.pointer(path).and_then(Value::as_bool) {
        Some(true) => Some(Hint::Supported),
        Some(false) => Some(Hint::Unsupported),
        None => None,
    };
    if let Some(value) = hint("/capabilities/thinking/supported") {
        model.capabilities.reasoning = value;
    }
    if let Some(value) = hint("/capabilities/structured_outputs/supported") {
        model.capabilities.structured_output = value;
    }
    if let Some(value) = hint("/capabilities/image_input/supported") {
        model.capabilities.vision = value;
    }
    model.supports_vision = model.capabilities.vision == Hint::Supported;
    model
}

#[async_trait]
impl ProviderAdapter for AnthropicAdapter {
    fn provider_type(&self) -> &str {
        PROTOCOL_ANTHROPIC
    }

    fn requires_max_tokens(&self) -> bool {
        true
    }

    async fn list_models(&self) -> DgosResult<Vec<Model>> {
        debug!("Fetching provider model catalog");
        self.fetch_models().await?.ok_or_else(|| {
            DgosError::ProviderUnavailable("The service has no model list endpoint".into())
        })
    }

    async fn call(&self, model_id: &str, request: ChatRequest) -> DgosResult<ChatResponse> {
        self.call_with_usage(model_id, request)
            .await
            .map_err(|failure| failure.error)
    }

    async fn call_with_usage(
        &self,
        model_id: &str,
        request: ChatRequest,
    ) -> Result<ChatResponse, CallFailure> {
        let url = self.transport.url("/v1/messages")?;
        preflight(self.profile(), &request, false)?;
        let state = BodyState::new(chat_body(self.profile(), model_id, &request, false)?);
        let options = &self.transport.options;
        // The upstream answered and was billed even when the host then rejects the answer.
        let billed = std::sync::Mutex::new(None::<TokenUsage>);
        self.transport
            .retry
            .execute_hinted(|| async {
                let response = self
                    .transport
                    .post_json(&url, &state, model_id, Some(options.request_timeout), None)
                    .await?;
                let bytes = read_limited(response, options.max_body_bytes).await?;
                let mut response = parse_chat_response(&bytes)?;
                if let Some(format) = &request.response_format {
                    // A mismatch is final: `StructuredOutputInvalid` is not retryable.
                    match structured::validate(&response.content, format) {
                        Ok(value) => response.structured = Some(value),
                        Err(error) => {
                            *billed.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(response.usage.clone());
                            return Err(error.into());
                        }
                    }
                }
                Ok(response)
            })
            .await
            .map_err(|error| CallFailure {
                usage: matches!(error, DgosError::StructuredOutputInvalid)
                    .then(|| billed.lock().unwrap_or_else(|e| e.into_inner()).take())
                    .flatten(),
                error,
            })
    }

    async fn call_tools(
        &self,
        model_id: &str,
        request: ToolChatRequest,
    ) -> DgosResult<ToolChatResponse> {
        // One attempt: host tool orchestration never relies on provider retries.
        let url = self.transport.url("/v1/messages")?;
        let state = BodyState::new(tool_body(self.profile(), model_id, &request)?);
        let options = &self.transport.options;
        let response = self
            .transport
            .post_json(&url, &state, model_id, Some(options.request_timeout), None)
            .await
            .map_err(|failure| failure.error)?;
        let bytes = read_limited(response, options.max_body_bytes).await?;
        parse_tool_response(&bytes)
    }

    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<BoxStream<ChatChunk>> {
        let url = self.transport.url("/v1/messages")?;
        preflight(self.profile(), &request, true)?;
        let state = BodyState::new(chat_body(self.profile(), model_id, &request, true)?);
        let idle = self.transport.options.stream_idle_timeout;
        // Retries cover connecting and the response headers only. Once the body is handed over
        // nothing is retried or replayed (B01-62).
        let response = self
            .transport
            .retry
            .execute_hinted(|| {
                self.transport
                    .post_json(&url, &state, model_id, None, Some(idle))
            })
            .await?;
        Ok(Box::pin(sse_stream(
            response,
            idle,
            self.transport.options.max_event_bytes,
            AnthropicMapper::default(),
        )))
    }

    async fn test_connection(&self, probe_model: Option<&str>) -> DgosResult<bool> {
        if self.fetch_models().await?.is_some() {
            return Ok(true);
        }
        match probe_model {
            Some(model) if valid_model_id(model) => self.probe(model).await.map(|_| true),
            _ => Err(DgosError::RequestNotSupported),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Request construction
// ---------------------------------------------------------------------------------------------

/// One message in the host's neutral form, so plain chat and tool chat share the conversion.
struct Msg<'a> {
    role: &'a str,
    content: Option<&'a str>,
    tool_call_id: Option<&'a str>,
    tool_calls: &'a [ToolCall],
    reasoning: Option<&'a MessageReasoning>,
}

struct Turn {
    role: &'static str,
    content: Vec<Value>,
}

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn is_tool_result(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("tool_result")
}

/// Leading system messages become the top-level `system`; same-role neighbours are merged into
/// one message with several content blocks; the conversation must open and close with `user`
/// (B01-47). Anything the protocol cannot express is `request_not_supported`.
fn build_turns(messages: Vec<Msg<'_>>) -> DgosResult<(Vec<String>, Vec<Turn>)> {
    let mut system = Vec::new();
    let mut turns: Vec<Turn> = Vec::new();
    for message in messages {
        let (role, blocks) = match message.role {
            "system" => {
                if !turns.is_empty() {
                    return Err(DgosError::RequestNotSupported);
                }
                if let Some(text) = message.content.filter(|text| !text.is_empty()) {
                    system.push(text.to_owned());
                }
                continue;
            }
            "user" => {
                let text = message
                    .content
                    .filter(|text| !text.is_empty())
                    .ok_or(DgosError::RequestNotSupported)?;
                ("user", vec![text_block(text)])
            }
            "assistant" => ("assistant", assistant_blocks(&message)?),
            "tool" => {
                let id = message
                    .tool_call_id
                    .filter(|id| !id.is_empty())
                    .ok_or(DgosError::RequestNotSupported)?;
                let mut block = json!({"type": "tool_result", "tool_use_id": id});
                if let Some(content) = message.content.filter(|text| !text.is_empty()) {
                    block["content"] = json!(content);
                }
                ("user", vec![block])
            }
            _ => return Err(DgosError::RequestNotSupported),
        };
        if blocks.is_empty() {
            return Err(DgosError::RequestNotSupported);
        }
        match turns.last_mut() {
            Some(last) if last.role == role => last.content.extend(blocks),
            _ => turns.push(Turn {
                role,
                content: blocks,
            }),
        }
    }
    if turns.first().map(|turn| turn.role) != Some("user")
        || turns.last().map(|turn| turn.role) != Some("user")
    {
        return Err(DgosError::RequestNotSupported);
    }
    // A user message answering tool calls lists its `tool_result` blocks first.
    for turn in turns.iter_mut().filter(|turn| turn.role == "user") {
        let (results, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut turn.content)
            .into_iter()
            .partition(is_tool_result);
        turn.content = results.into_iter().chain(rest).collect();
    }
    Ok((system, turns))
}

/// Thinking blocks of the previous turn come first and unchanged (signatures included), then the
/// text, then one `tool_use` per call (B01-48, B01-49).
fn assistant_blocks(message: &Msg<'_>) -> DgosResult<Vec<Value>> {
    let mut blocks: Vec<Value> = message
        .reasoning
        .map(|reasoning| reasoning.blocks.clone())
        .unwrap_or_default();
    if let Some(text) = message.content.filter(|text| !text.is_empty()) {
        blocks.push(text_block(text));
    }
    for call in message.tool_calls {
        let arguments = call.function.arguments.trim();
        let input: Value = if arguments.is_empty() {
            json!({})
        } else {
            serde_json::from_str(arguments).map_err(|_| DgosError::RequestNotSupported)?
        };
        if !input.is_object() {
            return Err(DgosError::RequestNotSupported);
        }
        blocks.push(json!({
            "type": "tool_use", "id": call.id, "name": call.function.name, "input": input
        }));
    }
    Ok(blocks)
}

fn finish_body(
    profile: &Profile,
    model_id: &str,
    max_tokens: u32,
    system: Vec<String>,
    turns: Vec<Turn>,
    stream: bool,
) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("model".into(), json!(model_id));
    body.insert("max_tokens".into(), json!(max_tokens));
    if !system.is_empty() {
        body.insert("system".into(), json!(system.join("\n\n")));
    }
    body.insert(
        "messages".into(),
        Value::Array(
            turns
                .into_iter()
                .map(|turn| json!({"role": turn.role, "content": turn.content}))
                .collect(),
        ),
    );
    body.insert("stream".into(), json!(stream));
    let _ = profile;
    body
}

fn chat_body(
    profile: &Profile,
    model_id: &str,
    request: &ChatRequest,
    stream: bool,
) -> DgosResult<Value> {
    let (mut system, turns) = build_turns(
        request
            .messages
            .iter()
            .map(|message| Msg {
                role: &message.role,
                content: Some(&message.content),
                tool_call_id: None,
                tool_calls: &[],
                reasoning: None,
            })
            .collect(),
    )?;
    let max_tokens = request.max_tokens.unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS);
    let mut output_config = Map::new();
    let mut thinking = None;
    if let Some(reasoning) = &request.reasoning {
        let params = profile.reasoning_params(reasoning)?;
        match &params {
            ReasoningParams::AnthropicAdaptive { effort } => {
                output_config.insert("effort".into(), json!(effort));
            }
            // The budget has to leave room for the answer (§5.2).
            ReasoningParams::AnthropicBudget { budget } if *budget >= max_tokens => {
                return Err(DgosError::RequestNotSupported)
            }
            _ => {}
        }
        thinking = params.anthropic_thinking();
    }
    if let Some(format) = &request.response_format {
        match profile.structured {
            StructuredOutput::AnthropicNative => {
                output_config.insert(
                    "format".into(),
                    json!({"type": "json_schema", "schema": format.schema}),
                );
            }
            StructuredOutput::AnthropicInstruction => system.push(structured::instruction(format)),
            StructuredOutput::OpenAiJsonSchema => {
                return Err(DgosError::InternalError("Profile mismatch".into()))
            }
        }
    }
    let mut body = finish_body(profile, model_id, max_tokens, system, turns, stream);
    if let Some(temperature) = request.temperature {
        // Widening f32 to f64 would print 0.7 as 0.699999988...; use the shortest f32 form.
        if let Some(number) = format!("{temperature}")
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
        {
            body.insert("temperature".into(), Value::Number(number));
        }
    }
    // Without a reasoning request nothing is sent: 5.x models think adaptively by default and
    // some reject an explicit `disabled`.
    if let Some(thinking) = thinking {
        body.insert("thinking".into(), thinking);
    }
    if !output_config.is_empty() {
        body.insert("output_config".into(), Value::Object(output_config));
    }
    Ok(Value::Object(body))
}

fn tool_body(profile: &Profile, model_id: &str, request: &ToolChatRequest) -> DgosResult<Value> {
    let (system, turns) = build_turns(
        request
            .messages
            .iter()
            .map(|message| Msg {
                role: &message.role,
                content: message.content.as_deref(),
                tool_call_id: message.tool_call_id.as_deref(),
                tool_calls: &message.tool_calls,
                reasoning: message.reasoning.as_ref(),
            })
            .collect(),
    )?;
    let mut body = finish_body(profile, model_id, request.max_tokens, system, turns, false);
    body.insert(
        "tools".into(),
        Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.function.name,
                        "description": tool.function.description,
                        "input_schema": tool.function.parameters
                    })
                })
                .collect(),
        ),
    );
    if profile.send_tool_choice {
        // One proposal at a time, as the workbench requires (D022).
        body.insert(
            "tool_choice".into(),
            json!({"type": "auto", "disable_parallel_tool_use": true}),
        );
    }
    Ok(Value::Object(body))
}

// ---------------------------------------------------------------------------------------------
// Response reading
// ---------------------------------------------------------------------------------------------

/// B01-50. Values the host has no meaning for (such as `pause_turn`) are an error, not `stop`.
fn map_stop_reason(reason: Option<&str>) -> DgosResult<&'static str> {
    match reason {
        None | Some("end_turn" | "stop_sequence") => Ok("stop"),
        Some("max_tokens" | "model_context_window_exceeded") => Ok("length"),
        Some("tool_use") => Ok("tool_calls"),
        Some("refusal") => Ok("content_filter"),
        Some(_) => Err(DgosError::InternalError("Unsupported stop reason".into())),
    }
}

/// Input is the sum of the three input counters; Anthropic reports no total. Reasoning tokens
/// are already part of `output_tokens`. A missing or empty `usage` is zero and flagged.
fn parse_usage(usage: Option<&Value>) -> (TokenUsage, bool) {
    let Some(usage) = usage.and_then(Value::as_object) else {
        return (TokenUsage::default(), true);
    };
    let field = |name: &str| {
        usage
            .get(name)
            .and_then(Value::as_u64)
            .map(|value| value.min(u32::MAX as u64) as u32)
    };
    let (input, output) = (field("input_tokens"), field("output_tokens"));
    if input.is_none() && output.is_none() {
        return (TokenUsage::default(), true);
    }
    let prompt = input
        .unwrap_or(0)
        .saturating_add(field("cache_creation_input_tokens").unwrap_or(0))
        .saturating_add(field("cache_read_input_tokens").unwrap_or(0));
    let completion = output.unwrap_or(0);
    (
        TokenUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt.saturating_add(completion),
        },
        false,
    )
}

#[derive(Default)]
struct Parsed {
    text: String,
    thinking: String,
    /// `thinking` and `redacted_thinking` blocks, verbatim.
    blocks: Vec<Value>,
    calls: Vec<ToolCall>,
}

fn parse_content(value: &Value) -> DgosResult<Parsed> {
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or(DgosError::ProviderResponseInvalid)?;
    let mut parsed = Parsed::default();
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    parsed.text.push_str(text);
                }
            }
            Some("thinking") => {
                if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                    parsed.thinking.push_str(text);
                }
                parsed.blocks.push(block.clone());
            }
            Some("redacted_thinking") => parsed.blocks.push(block.clone()),
            Some("tool_use") => {
                let id = block.get("id").and_then(Value::as_str);
                let name = block.get("name").and_then(Value::as_str);
                let (Some(id), Some(name)) = (id, name) else {
                    return Err(DgosError::ProviderResponseInvalid);
                };
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                parsed.calls.push(ToolCall {
                    id: id.to_owned(),
                    kind: "function".into(),
                    function: ToolFunctionCall {
                        name: name.to_owned(),
                        arguments: input.to_string(),
                    },
                });
            }
            // Block types the host does not know (server tools, fallbacks) are skipped.
            other => {
                debug!(block_type = ?other.unwrap_or("none"), "Ignoring unknown content block")
            }
        }
    }
    Ok(parsed)
}

fn parse_chat_response(bytes: &[u8]) -> DgosResult<ChatResponse> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| DgosError::ProviderResponseInvalid)?;
    let parsed = parse_content(&value)?;
    let finish = map_stop_reason(value.get("stop_reason").and_then(Value::as_str))?;
    let (usage, usage_unknown) = parse_usage(value.get("usage"));
    Ok(ChatResponse {
        content: parsed.text,
        usage,
        audit_warning: false,
        finish_reason: Some(finish.to_owned()),
        usage_unknown,
        reasoning: (!parsed.thinking.is_empty()).then_some(parsed.thinking),
        structured: None,
    })
}

fn parse_tool_response(bytes: &[u8]) -> DgosResult<ToolChatResponse> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| DgosError::ProviderResponseInvalid)?;
    let invalid = || DgosError::InternalError("Invalid or incomplete tool chat response".into());
    let parsed = parse_content(&value)?;
    let finish =
        map_stop_reason(value.get("stop_reason").and_then(Value::as_str)).map_err(|_| invalid())?;
    if !matches!(finish, "stop" | "tool_calls") || (parsed.calls.is_empty() != (finish == "stop")) {
        return Err(invalid());
    }
    let reasoning = (!parsed.blocks.is_empty()).then(|| MessageReasoning {
        text: parsed.thinking,
        fields: vec![],
        blocks: parsed.blocks,
    });
    Ok(ToolChatResponse {
        message: ToolMessage {
            role: "assistant".into(),
            content: (!parsed.text.is_empty()).then_some(parsed.text),
            tool_call_id: None,
            tool_calls: parsed.calls,
            reasoning,
        },
        usage: parse_usage(value.get("usage")).0,
        audit_warning: false,
    })
}

// ---------------------------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------------------------

/// Maps Messages stream events (B01-61, B01-62). Tool use is not streamed (V1 streams text
/// only), signature and JSON deltas are dropped, unknown events are ignored.
#[derive(Default)]
struct AnthropicMapper {
    input: Option<u32>,
    output: Option<u32>,
    stop_reason: Option<String>,
    stopped: bool,
}

impl AnthropicMapper {
    fn absorb_usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage.and_then(Value::as_object) else {
            return;
        };
        let field = |name: &str| {
            usage
                .get(name)
                .and_then(Value::as_u64)
                .map(|value| value.min(u32::MAX as u64) as u32)
        };
        // `message_delta.usage` is cumulative: the latest value replaces the earlier one.
        if let Some(input) = field("input_tokens") {
            let cached = field("cache_creation_input_tokens")
                .unwrap_or(0)
                .saturating_add(field("cache_read_input_tokens").unwrap_or(0));
            self.input = Some(input.saturating_add(cached));
        }
        if let Some(output) = field("output_tokens") {
            self.output = Some(output);
        }
    }

    fn finish(&mut self) -> Outcome {
        match map_stop_reason(self.stop_reason.as_deref()) {
            Ok(_) => Outcome::Complete,
            Err(error) => Outcome::Fail(error),
        }
    }
}

impl Mapper for AnthropicMapper {
    fn event(&mut self, event: &SseEvent, out: &mut Vec<ChatChunk>) -> Outcome {
        // Unparseable data is an interruption, never a silently dropped chunk.
        let Ok(value) = serde_json::from_str::<Value>(event.data.trim()) else {
            return Outcome::Fail(DgosError::StreamInterrupted);
        };
        // The SSE event name `error` decides, whatever shape the payload has: after HTTP 200 the
        // upstream may send `{"type":"error",...}` or just `{"type":"overloaded_error",...}`.
        if event.event.as_deref() == Some("error") {
            return Outcome::Fail(DgosError::StreamInterrupted);
        }
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .or(event.event.as_deref())
            .unwrap_or_default()
            .to_owned();
        match kind.as_str() {
            // Only resets the idle timer, which the driver already did by receiving bytes.
            "ping" => {}
            "message_start" => self.absorb_usage(value.pointer("/message/usage")),
            "content_block_start" => {
                if let Some(text) = value
                    .pointer("/content_block/text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    out.push(ChatChunk {
                        delta: text.to_owned(),
                        ..Default::default()
                    });
                }
            }
            "content_block_delta" => {
                let delta = value.get("delta");
                let text = |key: &str| {
                    delta
                        .and_then(|delta| delta.get(key))
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(str::to_owned)
                };
                match delta
                    .and_then(|delta| delta.get("type"))
                    .and_then(Value::as_str)
                {
                    Some("text_delta") => {
                        if let Some(text) = text("text") {
                            out.push(ChatChunk {
                                delta: text,
                                ..Default::default()
                            });
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = text("thinking") {
                            out.push(ChatChunk {
                                reasoning_delta: Some(text),
                                ..Default::default()
                            });
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(reason.to_owned());
                    self.stopped = true;
                }
                self.absorb_usage(value.get("usage"));
            }
            "message_stop" => {
                self.stopped = true;
                return self.finish();
            }
            // After HTTP 200 the upstream may still fail (`overloaded_error`, ...).
            "error" => return Outcome::Fail(DgosError::StreamInterrupted),
            other => debug!(event = other, "Ignoring unknown stream event"),
        }
        Outcome::Continue
    }

    fn terminal(&mut self) -> ChatChunk {
        let usage = (self.input.is_some() || self.output.is_some()).then(|| {
            let prompt = self.input.unwrap_or(0);
            let completion = self.output.unwrap_or(0);
            TokenUsage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: prompt.saturating_add(completion),
            }
        });
        ChatChunk {
            finish_reason: Some(
                map_stop_reason(self.stop_reason.as_deref())
                    .unwrap_or("stop")
                    .to_owned(),
            ),
            usage,
            ..Default::default()
        }
    }

    /// `message_stop` always follows `message_delta`; a body that ends after the stop reason
    /// but before it completed anyway. Anything earlier was cut off.
    fn at_eof(&mut self) -> Outcome {
        if self.stopped && self.stop_reason.is_some() {
            self.finish()
        } else {
            Outcome::Fail(DgosError::StreamInterrupted)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::ANTHROPIC_OFFICIAL;
    use dgos_types::{ChatMessage, ReasoningConfig, ResponseFormat, ToolDefinition, ToolFunction};

    fn message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: content.into(),
        }
    }

    fn request(messages: Vec<ChatMessage>) -> ChatRequest {
        ChatRequest {
            messages,
            ..Default::default()
        }
    }

    #[test]
    fn system_is_lifted_and_same_role_messages_merge() {
        let body = chat_body(
            &ANTHROPIC_GENERIC,
            "m",
            &request(vec![
                message("system", "be brief"),
                message("system", "be kind"),
                message("user", "one"),
                message("user", "two"),
            ]),
            false,
        )
        .unwrap();
        assert_eq!(body["system"], "be brief\n\nbe kind");
        assert_eq!(
            body["messages"],
            json!([{"role": "user", "content": [
                {"type": "text", "text": "one"}, {"type": "text", "text": "two"}]}])
        );
        // Not given by the caller: the profile default applies.
        assert_eq!(body["max_tokens"], ANTHROPIC_DEFAULT_MAX_TOKENS);
        assert!(body.get("thinking").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn requests_that_cannot_be_expressed_are_refused_locally() {
        let refused = |messages: Vec<ChatMessage>| {
            chat_body(&ANTHROPIC_GENERIC, "m", &request(messages), false).is_err()
        };
        // System after the conversation started.
        assert!(refused(vec![
            message("user", "a"),
            message("system", "late"),
            message("user", "b")
        ]));
        // Ends with assistant (prefill), or starts with assistant, or has no user turn at all.
        assert!(refused(vec![
            message("user", "a"),
            message("assistant", "b")
        ]));
        assert!(refused(vec![
            message("assistant", "a"),
            message("user", "b")
        ]));
        assert!(refused(vec![message("system", "only")]));
        assert!(refused(vec![]));
        assert!(refused(vec![message("user", "")]));
        assert!(refused(vec![message("tool", "x"), message("user", "b")]));
        assert!(refused(vec![message("developer", "x")]));
        assert!(!refused(vec![
            message("user", "a"),
            message("assistant", "b"),
            message("user", "c")
        ]));
    }

    #[test]
    fn reasoning_is_mapped_per_profile_and_budget_must_leave_room() {
        let with = |profile: &'static Profile, effort: &str, max_tokens: Option<u32>| {
            chat_body(
                profile,
                "m",
                &ChatRequest {
                    messages: vec![message("user", "q")],
                    max_tokens,
                    reasoning: Some(ReasoningConfig {
                        effort: Some(effort.into()),
                        budget_tokens: None,
                    }),
                    ..Default::default()
                },
                false,
            )
        };
        let official = with(&ANTHROPIC_OFFICIAL, "high", Some(4096)).unwrap();
        assert_eq!(
            official["thinking"],
            json!({"type": "adaptive", "display": "summarized"})
        );
        assert_eq!(official["output_config"], json!({"effort": "high"}));
        let generic = with(&ANTHROPIC_GENERIC, "medium", Some(16000)).unwrap();
        assert_eq!(
            generic["thinking"],
            json!({"type": "enabled", "budget_tokens": 8192})
        );
        assert!(generic.get("output_config").is_none());
        // The budget 8192 does not fit under max_tokens 8192.
        assert!(with(&ANTHROPIC_GENERIC, "medium", Some(8192)).is_err());
        // Levels the profile has no mapping for.
        assert!(with(&ANTHROPIC_GENERIC, "xhigh", Some(16000)).is_err());
        assert!(with(&ANTHROPIC_OFFICIAL, "minimal", Some(4096)).is_err());
    }

    #[test]
    fn structured_output_uses_the_native_format_or_a_system_instruction() {
        let format = ResponseFormat {
            kind: "json_schema".into(),
            name: "answer".into(),
            schema: json!({"type": "object"}),
            strict: None,
        };
        let req = ChatRequest {
            messages: vec![message("system", "sys"), message("user", "q")],
            response_format: Some(format),
            ..Default::default()
        };
        let native = chat_body(&ANTHROPIC_OFFICIAL, "m", &req, false).unwrap();
        assert_eq!(
            native["output_config"]["format"],
            json!({"type": "json_schema", "schema": {"type": "object"}})
        );
        assert_eq!(native["system"], "sys");
        let generic = chat_body(&ANTHROPIC_GENERIC, "m", &req, false).unwrap();
        assert!(generic.get("output_config").is_none());
        let system = generic["system"].as_str().unwrap();
        assert!(system.starts_with("sys\n\n") && system.contains("JSON Schema"));
    }

    fn tool_request(messages: Vec<ToolMessage>) -> ToolChatRequest {
        ToolChatRequest {
            messages,
            tools: vec![ToolDefinition {
                kind: "function".into(),
                function: ToolFunction {
                    name: "dgos_tool_0".into(),
                    description: "d".into(),
                    parameters: json!({"type": "object"}),
                },
            }],
            max_tokens: 512,
        }
    }

    fn tm(role: &str, content: Option<&str>) -> ToolMessage {
        ToolMessage {
            role: role.into(),
            content: content.map(str::to_owned),
            tool_call_id: None,
            tool_calls: vec![],
            reasoning: None,
        }
    }

    #[test]
    fn tool_round_places_thinking_first_then_text_then_tool_use_then_the_result() {
        let thinking = json!({"type": "thinking", "thinking": "hm", "signature": "sig=="});
        let redacted = json!({"type": "redacted_thinking", "data": "opaque"});
        let mut assistant = tm("assistant", Some("let me check"));
        assistant.tool_calls = vec![ToolCall {
            id: "toolu_1".into(),
            kind: "function".into(),
            function: ToolFunctionCall {
                name: "dgos_tool_0".into(),
                arguments: r#"{"text":"x"}"#.into(),
            },
        }];
        assistant.reasoning = Some(MessageReasoning {
            text: "hm".into(),
            fields: vec![],
            blocks: vec![thinking.clone(), redacted.clone()],
        });
        let mut result = tm("tool", Some("done"));
        result.tool_call_id = Some("toolu_1".into());
        let body = tool_body(
            &ANTHROPIC_GENERIC,
            "m",
            &tool_request(vec![
                tm("system", Some("policy")),
                tm("user", Some("task")),
                assistant,
                result,
            ]),
        )
        .unwrap();
        assert_eq!(body["system"], "policy");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1]["content"],
            json!([thinking, redacted,
                {"type": "text", "text": "let me check"},
                {"type": "tool_use", "id": "toolu_1", "name": "dgos_tool_0", "input": {"text": "x"}}])
        );
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(
            messages[2]["content"],
            json!([{"type": "tool_result", "tool_use_id": "toolu_1", "content": "done"}])
        );
        assert_eq!(body["tools"][0]["input_schema"], json!({"type": "object"}));
        assert_eq!(
            body["tool_choice"],
            json!({"type": "auto", "disable_parallel_tool_use": true})
        );
    }

    #[test]
    fn malformed_tool_arguments_are_refused_instead_of_sent() {
        for arguments in ["not json", "[1]", "\"s\""] {
            let mut assistant = tm("assistant", None);
            assistant.tool_calls = vec![ToolCall {
                id: "t".into(),
                kind: "function".into(),
                function: ToolFunctionCall {
                    name: "n".into(),
                    arguments: arguments.into(),
                },
            }];
            let mut result = tm("tool", Some("r"));
            result.tool_call_id = Some("t".into());
            assert!(
                tool_body(
                    &ANTHROPIC_GENERIC,
                    "m",
                    &tool_request(vec![tm("user", Some("q")), assistant, result])
                )
                .is_err(),
                "{arguments}"
            );
        }
    }

    #[test]
    fn tool_response_keeps_thinking_blocks_and_stringifies_the_input() {
        let body = json!({"content": [
            {"type": "thinking", "thinking": "plan", "signature": "s"},
            {"type": "redacted_thinking", "data": "x"},
            {"type": "text", "text": "calling"},
            {"type": "tool_use", "id": "toolu_9", "name": "dgos_tool_0", "input": {"a": 1}}],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 5, "output_tokens": 7}});
        let response = parse_tool_response(body.to_string().as_bytes()).unwrap();
        let message = response.message;
        assert_eq!(message.content.as_deref(), Some("calling"));
        assert_eq!(message.tool_calls[0].function.arguments, r#"{"a":1}"#);
        let reasoning = message.reasoning.unwrap();
        assert_eq!(reasoning.text, "plan");
        assert_eq!(reasoning.blocks.len(), 2);
        assert_eq!(reasoning.blocks[0]["signature"], "s");
        assert_eq!(response.usage.total_tokens, 12);
    }

    #[test]
    fn tool_response_must_agree_with_its_stop_reason() {
        let bad = |value: Value| parse_tool_response(value.to_string().as_bytes()).is_err();
        // tool_use without a tool_use block, a block without the stop reason, truncation.
        assert!(bad(
            json!({"content": [{"type": "text", "text": "x"}], "stop_reason": "tool_use"})
        ));
        assert!(bad(json!({"content": [
            {"type": "tool_use", "id": "t", "name": "n", "input": {}}], "stop_reason": "end_turn"})));
        assert!(bad(
            json!({"content": [{"type": "text", "text": "x"}], "stop_reason": "max_tokens"})
        ));
        assert!(bad(
            json!({"content": [{"type": "text", "text": "x"}], "stop_reason": "pause_turn"})
        ));
        assert!(!bad(
            json!({"content": [{"type": "text", "text": "x"}], "stop_reason": "end_turn"})
        ));
    }

    #[test]
    fn stop_reasons_and_usage_are_normalised() {
        for (reason, expected) in [
            ("end_turn", "stop"),
            ("stop_sequence", "stop"),
            ("max_tokens", "length"),
            ("model_context_window_exceeded", "length"),
            ("tool_use", "tool_calls"),
            ("refusal", "content_filter"),
        ] {
            assert_eq!(map_stop_reason(Some(reason)).unwrap(), expected);
        }
        assert!(map_stop_reason(Some("pause_turn")).is_err());
        let (usage, unknown) = parse_usage(Some(&json!({
            "input_tokens": 10, "cache_creation_input_tokens": 3,
            "cache_read_input_tokens": 2, "output_tokens": 4})));
        assert!(!unknown);
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (15, 4, 19)
        );
        assert!(parse_usage(None).1 && parse_usage(Some(&json!({}))).1);
    }

    #[test]
    fn model_items_use_only_documented_fields() {
        let item = json!({"id": "claude-x", "display_name": "Claude X", "max_input_tokens": 200000,
            "max_tokens": 64000, "undocumented": 1,
            "capabilities": {"thinking": {"supported": true},
                "structured_outputs": {"supported": false}, "image_input": {"supported": true}}});
        let model = model_from_item("claude-x", &item);
        assert_eq!(model.model_name, "Claude X");
        assert_eq!(model.model_type, "text-generation");
        assert_eq!(model.context_window, Some(200000));
        assert_eq!(model.max_output_tokens, Some(64000));
        assert_eq!(model.capabilities.reasoning, Hint::Supported);
        assert_eq!(model.capabilities.structured_output, Hint::Unsupported);
        assert!(model.supports_vision && model.supports_streaming);
        let bare = model_from_item("claude-y", &json!({"id": "claude-y"}));
        assert_eq!((bare.context_window, bare.max_output_tokens), (None, None));
        assert_eq!(bare.capabilities.reasoning, Hint::Unknown);
    }

    #[test]
    fn cursor_is_percent_encoded() {
        assert_eq!(encode_cursor("claude-3.5_x~"), "claude-3.5_x~");
        assert_eq!(encode_cursor("a b&c"), "a%20b%26c");
    }
}
