//! `openai_compatible` adapter (OpenAI Chat Completions). Request shape differences between
//! services are read from the [`Profile`]; nothing in here branches on a vendor name.

use async_trait::async_trait;
use dgos_types::{
    build_model, valid_model_id, ChatChunk, ChatRequest, ChatResponse, DgosError, DgosResult,
    MessageReasoning, Model, ModelSource, RetryConfig, TokenUsage, ToolCall, ToolChatRequest,
    ToolChatResponse, ToolFunctionCall, ToolMessage, ToolStreamEvent, PROTOCOL_OPENAI_COMPATIBLE,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use tracing::debug;

use super::preflight;
use crate::sse::SseEvent;
use crate::stream::{sse_stream, Mapper, Outcome};
use crate::structured;
use crate::traits::{BoxStream, CallFailure, ProviderAdapter};
use crate::transport::{AdapterOptions, BodyState, Transport};
use crate::upstream::{read_limited, MAX_TOOL_BODY_BYTES, OPENAI_DOWNGRADABLE};
use crate::vendor::{Profile, ReasoningParams, StructuredOutput, OPENAI_GENERIC};

/// Reasoning fields an `openai_compatible` upstream may put on an assistant message. Whatever
/// appeared in a response is sent back under the same name on the next turn (B01-51).
pub const REASONING_FIELDS: [&str; 3] = ["reasoning_content", "reasoning", "reasoning_details"];

/// OpenAI compatible adapter
pub struct OpenAIAdapter {
    transport: Transport,
}

impl OpenAIAdapter {
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
        Self::with_profile(base_url, api_key, retry_config, options, &OPENAI_GENERIC)
    }

    /// The caller resolves the profile from the address (`vendor::resolve_profile`); the adapter
    /// never picks one itself, so a profile cannot change where requests go.
    pub fn with_profile(
        base_url: String,
        api_key: String,
        retry_config: Option<RetryConfig>,
        options: AdapterOptions,
        profile: &'static Profile,
    ) -> Self {
        Self {
            transport: Transport::new(
                base_url,
                api_key,
                retry_config,
                options,
                profile,
                OPENAI_DOWNGRADABLE,
            ),
        }
    }

    /// As [`Self::with_profile`], reusing a client the manager built once.
    pub(crate) fn with_client(
        client: Option<reqwest::Client>,
        base_url: String,
        api_key: String,
        retry_config: Option<RetryConfig>,
        options: AdapterOptions,
        profile: &'static Profile,
    ) -> Self {
        Self {
            transport: Transport::with_client(
                client,
                base_url,
                api_key,
                retry_config,
                options,
                profile,
                OPENAI_DOWNGRADABLE,
            ),
        }
    }

    fn profile(&self) -> &'static Profile {
        self.transport.profile
    }

    /// `Ok(None)`: the service has no model list endpoint (404 or 405).
    async fn fetch_models(&self) -> DgosResult<Option<Vec<Model>>> {
        let url = self.transport.url("/models")?;
        match self.transport.get_models_page(&url).await? {
            Some(bytes) => parse_models(&bytes, self.transport.options.max_models).map(Some),
            None => Ok(None),
        }
    }

    /// One minimal request to `model_id`; any 2xx proves the endpoint, key and model work.
    async fn probe(&self, model_id: &str) -> DgosResult<()> {
        let url = self.transport.url("/chat/completions")?;
        let state = BodyState::new(json!({
            "model": model_id,
            "messages": [{"role": "user", "content": "ping"}],
            self.profile().max_tokens_field: 16,
            "stream": false
        }));
        self.transport
            .post_json(
                &url,
                &state,
                model_id,
                Some(self.transport.options.request_timeout),
                None,
                None,
            )
            .await
            .map(|_| ())
            .map_err(|failure| failure.error)
    }
}

#[async_trait]
impl ProviderAdapter for OpenAIAdapter {
    fn provider_type(&self) -> &str {
        PROTOCOL_OPENAI_COMPATIBLE
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
        let url = self.transport.url("/chat/completions")?;
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
                    .post_json(&url, &state, model_id, Some(options.request_timeout), None, None)
                    .await?;
                let bytes = read_limited(response, options.max_body_bytes).await?;
                let mut response = parse_chat_response(&bytes, &billed)?;
                if let Some(format) = &request.response_format {
                    // A mismatch is final: `StructuredOutputInvalid` is not retryable.
                    match structured::validate(&response.content, format) {
                        Ok(value) => response.structured = Some(value),
                        Err(error) => {
                            return Err(error.into());
                        }
                    }
                }
                Ok(response)
            })
            .await
            .map_err(|error| CallFailure {
                usage: billed.lock().unwrap_or_else(|e| e.into_inner()).take(),
                error,
            })
    }

    async fn call_tools(
        &self,
        model_id: &str,
        request: ToolChatRequest,
    ) -> DgosResult<ToolChatResponse> {
        // One attempt: host tool orchestration never relies on provider retries.
        let url = self.transport.url("/chat/completions")?;
        let state = BodyState::new(tool_body(self.profile(), model_id, &request)?);
        let options = &self.transport.options;
        let response = self
            .transport
            .post_json(&url, &state, model_id, Some(options.request_timeout), None, None)
            .await
            .map_err(|failure| failure.error)?;
        let limit = options.max_body_bytes.min(MAX_TOOL_BODY_BYTES);
        let bytes = read_limited(response, limit).await?;
        parse_tool_response(&bytes)
    }

    async fn stream_tools_with_dispatch(
        &self,
        model_id: &str,
        request: ToolChatRequest,
        dispatched: Option<&AtomicBool>,
    ) -> DgosResult<BoxStream<ToolStreamEvent>> {
        let url = self.transport.url("/chat/completions")?;
        let mut body = tool_body(self.profile(), model_id, &request)?;
        body["stream"] = json!(true);
        if self.profile().send_stream_options {
            body["stream_options"] = json!({"include_usage": true});
        }
        let state = BodyState::new(body);
        let idle = self.transport.options.stream_idle_timeout;
        // One logical transport attempt, never a synchronous fallback or a replay.
        let response = self.transport
            .post_json(&url, &state, model_id, None, Some(idle), dispatched)
            .await
            .map_err(|failure| failure.error)?;
        Ok(Box::pin(sse_stream(
            response,
            idle,
            self.transport.options.max_event_bytes,
            OpenAiToolMapper::default(),
        )))
    }

    async fn stream_with_dispatch(
        &self,
        model_id: &str,
        request: ChatRequest,
        dispatched: Option<&AtomicBool>,
    ) -> DgosResult<BoxStream<ChatChunk>> {
        let url = self.transport.url("/chat/completions")?;
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
                    .post_json(&url, &state, model_id, None, Some(idle), dispatched)
            })
            .await?;
        Ok(Box::pin(sse_stream(
            response,
            idle,
            self.transport.options.max_event_bytes,
            OpenAiMapper::default(),
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

fn chat_body(
    profile: &Profile,
    model_id: &str,
    request: &ChatRequest,
    stream: bool,
) -> DgosResult<Value> {
    let mut body = json!({
        "model": model_id,
        "messages": request.messages.iter().map(chat_message_json).collect::<Vec<_>>(),
        "stream": stream
    });
    let object = body
        .as_object_mut()
        .expect("json! object literal is an object");
    apply_generation(profile, object, request.temperature, request.max_tokens, request.reasoning.as_ref())?;
    if stream && profile.send_stream_options {
        object.insert("stream_options".into(), json!({"include_usage": true}));
    }
    if let Some(format) = &request.response_format {
        if profile.structured != StructuredOutput::OpenAiJsonSchema {
            return Err(DgosError::InternalError("Profile mismatch".into()));
        }
        let mut schema = json!({"name": format.name, "schema": format.schema});
        if let Some(strict) = format.strict {
            schema["strict"] = json!(strict);
        }
        object.insert(
            "response_format".into(),
            json!({"type": "json_schema", "json_schema": schema}),
        );
    }
    Ok(body)
}

/// Wire form of one chat message. Text-only messages keep the plain string content; a message
/// with images becomes a content array of one text part and one `image_url` part per image, each a
/// base64 data URL built from the host-validated bytes.
fn chat_message_json(message: &dgos_types::ChatMessage) -> Value {
    use base64::Engine as _;
    if message.images.is_empty() {
        return json!({"role": message.role, "content": message.content});
    }
    let mut parts = vec![json!({"type": "text", "text": message.content})];
    for image in &message.images {
        let url = format!(
            "data:{};base64,{}",
            image.mime,
            base64::engine::general_purpose::STANDARD.encode(&image.data)
        );
        parts.push(json!({"type": "image_url", "image_url": {"url": url}}));
    }
    json!({"role": message.role, "content": parts})
}

fn tool_body(profile: &Profile, model_id: &str, request: &ToolChatRequest) -> DgosResult<Value> {
    profile.check_sampling(request.temperature, request.reasoning.as_ref())?;
    let mut body = json!({
        "model": model_id,
        "messages": request.messages.iter().map(tool_message_json).collect::<Vec<_>>(),
        "tools": request.tools,
        "stream": false
    });
    let object = body
        .as_object_mut()
        .expect("json! object literal is an object");
    apply_generation(profile, object, request.temperature, Some(request.max_tokens), request.reasoning.as_ref())?;
    if profile.send_tool_choice {
        object.insert("tool_choice".into(), json!("auto"));
    }
    if profile.send_parallel_tool_calls {
        object.insert("parallel_tool_calls".into(), json!(false));
    }
    Ok(body)
}

fn apply_generation(profile: &Profile, object: &mut serde_json::Map<String, Value>, temperature: Option<f32>, max_tokens: Option<u32>, reasoning: Option<&dgos_types::ReasoningConfig>) -> DgosResult<()> {
    if let Some(temperature) = temperature {
        let number = format!("{temperature}").parse::<f64>().ok().and_then(serde_json::Number::from_f64).ok_or(DgosError::RequestNotSupported)?;
        object.insert("temperature".into(), Value::Number(number));
    }
    if let Some(max_tokens) = max_tokens { object.insert(profile.max_tokens_field.into(), json!(max_tokens)); }
    if let Some(reasoning) = reasoning {
        match profile.reasoning_params(reasoning)? {
            ReasoningParams::OpenAiEffort(effort) => { object.insert("reasoning_effort".into(), json!(effort)); }
            _ => return Err(DgosError::InternalError("Profile mismatch".into())),
        }
    }
    Ok(())
}

/// The wire form of one message. `ToolMessage` never serializes its reasoning, so the fields the
/// upstream sent on the previous turn are put back here, under their original names.
fn tool_message_json(message: &ToolMessage) -> Value {
    let mut value = serde_json::to_value(message).unwrap_or(Value::Null);
    if message.role == "assistant" {
        if let (Some(reasoning), Some(object)) = (&message.reasoning, value.as_object_mut()) {
            for (name, field) in &reasoning.fields {
                if REASONING_FIELDS.contains(&name.as_str()) {
                    object.insert(name.clone(), field.clone());
                }
            }
        }
    }
    if !message.images.is_empty() {
        use base64::Engine as _;
        let mut parts = vec![json!({"type": "text", "text": message.content.clone().unwrap_or_default()})];
        for image in &message.images {
            let url = format!(
                "data:{};base64,{}",
                image.mime,
                base64::engine::general_purpose::STANDARD.encode(&image.data)
            );
            parts.push(json!({"type": "image_url", "image_url": {"url": url}}));
        }
        if let Some(object) = value.as_object_mut() {
            object.insert("content".into(), Value::Array(parts));
        }
    }
    value
}

/// Displayable text of a reasoning field: a string, or the text parts of the structured forms
/// (`reasoning_details` items carry `text` or `summary`; encrypted parts show nothing).
fn displayable(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(displayable).collect(),
        Value::Object(map) => ["text", "summary", "content"]
            .iter()
            .find_map(|key| map.get(*key))
            .map(displayable)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// The reasoning an assistant message carries, kept exactly as sent. `None` when the upstream
/// returned no reasoning field, so nothing is added on the way back.
fn extract_reasoning(message: &Value) -> Option<MessageReasoning> {
    let object = message.as_object()?;
    let mut reasoning = MessageReasoning::default();
    for name in REASONING_FIELDS {
        let Some(value) = object.get(name).filter(|value| !value.is_null()) else {
            continue;
        };
        if reasoning.text.is_empty() {
            reasoning.text = displayable(value);
        }
        reasoning.fields.push((name.to_owned(), value.clone()));
    }
    (!reasoning.fields.is_empty()).then_some(reasoning)
}

/// First non-empty reasoning increment of a stream delta.
fn reasoning_delta(delta: Option<&Value>) -> Option<String> {
    let delta = delta?.as_object()?;
    REASONING_FIELDS
        .iter()
        .filter_map(|name| delta.get(*name))
        .map(displayable)
        .find(|text| !text.is_empty())
}

/// §2.4.7 tolerant reading: unknown fields ignored, absent usage is zero and flagged, an empty
/// `choices` gives empty content, an unknown finish reason counts as `stop`.
fn parse_chat_response(bytes: &[u8], billed: &std::sync::Mutex<Option<TokenUsage>>) -> DgosResult<ChatResponse> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| DgosError::ProviderResponseInvalid)?;
    let (usage, usage_unknown) = parse_usage(value.get("usage"));
    *billed.lock().unwrap_or_else(|e| e.into_inner()) = usage.has_reported_usage().then(|| usage.clone());
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .ok_or(DgosError::ProviderResponseInvalid)?;
    let first = choices.first();
    let content = first
        .and_then(|choice| choice.pointer("/message/content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let finish_reason = first
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str)
        .map(normalize_finish_reason);
    let reasoning = first
        .and_then(|choice| choice.get("message"))
        .and_then(extract_reasoning)
        .map(|reasoning| reasoning.text)
        .filter(|text| !text.is_empty());
    Ok(ChatResponse {
        content,
        usage,
        audit_warning: false,
        finish_reason,
        usage_unknown,
        reasoning,
        structured: None,
    })
}

fn parse_tool_response(bytes: &[u8]) -> DgosResult<ToolChatResponse> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| DgosError::ProviderResponseInvalid)?;
    let usage = parse_usage(value.get("usage")).0;
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .ok_or(DgosError::ProviderResponseInvalid)?;
    if choices.len() != 1 {
        return Err(DgosError::InternalError(
            "Expected one tool chat choice".into(),
        ));
    }
    let choice = &choices[0];
    let invalid = || DgosError::InternalError("Invalid or incomplete tool chat response".into());
    let raw = choice.get("message").cloned().unwrap_or(Value::Null);
    let mut message: ToolMessage = serde_json::from_value(raw.clone()).map_err(|_| invalid())?;
    let finish_reason = choice.get("finish_reason").and_then(Value::as_str);
    if message.role != "assistant"
        || !matches!(finish_reason, Some("stop" | "tool_calls"))
        || (message.tool_calls.is_empty() != (finish_reason == Some("stop")))
        || message.tool_call_id.is_some()
    {
        return Err(invalid());
    }
    message.reasoning = extract_reasoning(&raw);
    Ok(ToolChatResponse {
        message,
        usage,
        audit_warning: false,
    })
}

fn parse_models(bytes: &[u8], max_models: usize) -> DgosResult<Vec<Model>> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| DgosError::ProviderResponseInvalid)?;
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or(DgosError::ProviderResponseInvalid)?;
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !valid_model_id(id) || !seen.insert(id.to_owned()) {
            continue;
        }
        models.push(build_model(
            PROTOCOL_OPENAI_COMPATIBLE,
            id,
            None,
            ModelSource::Remote,
            None,
        ));
        if models.len() >= max_models {
            break;
        }
    }
    Ok(models)
}


fn usage_counts_valid(usage: Option<&Value>) -> bool {
    let Some(usage) = usage else { return true; };
    ["/prompt_tokens", "/completion_tokens", "/total_tokens", "/prompt_cache_hit_tokens",
        "/prompt_tokens_details/cached_tokens", "/prompt_tokens_details/cache_creation_tokens",
        "/completion_tokens_details/reasoning_tokens"].into_iter().all(|path| {
        usage.pointer(path).filter(|value| !value.is_null()).is_none_or(|value|
            value.as_u64().is_some_and(|value| u32::try_from(value).is_ok()))
    })
}
/// OpenAI prompt/completion already include cache/reasoning subsets. Missing fields stay unknown.
pub(crate) fn parse_usage(usage: Option<&Value>) -> (TokenUsage, bool) {
    // Reject an unrepresentable report as a whole: deriving totals from a truncated subset
    // would manufacture a known count. Valid earlier stream reports remain partial evidence.
    if !usage_counts_valid(usage) { return (TokenUsage::default(), true); }
    let Some(usage) = usage.and_then(Value::as_object) else {
        return (TokenUsage::default(), true);
    };
    let count = |value: Option<&Value>| value.and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let field = |name: &str| count(usage.get(name));
    let prompt = field("prompt_tokens");
    let completion = field("completion_tokens");
    let total = field("total_tokens").or_else(|| prompt.zip(completion).and_then(|(p, c)| p.checked_add(c)));
    let usage = TokenUsage {
        prompt_tokens: prompt.unwrap_or(0),
        completion_tokens: completion.unwrap_or(0),
        total_tokens: total.unwrap_or_else(|| prompt.unwrap_or(0).saturating_add(completion.unwrap_or(0))),
        prompt_tokens_reported: prompt.is_some(),
        completion_tokens_reported: completion.is_some(),
        total_tokens_reported: total.is_some(),
        cache_read_tokens: count(usage.get("prompt_tokens_details").and_then(|v| v.get("cached_tokens")))
            .or_else(|| field("prompt_cache_hit_tokens")),
        cache_write_tokens: count(usage.get("prompt_tokens_details").and_then(|v| v.get("cache_creation_tokens"))),
        reasoning_tokens: count(usage.get("completion_tokens_details").and_then(|v| v.get("reasoning_tokens"))),
    };
    let unknown = !usage.has_reported_usage();
    crate::usage::observe(&usage);
    (usage, unknown)
}

fn normalize_finish_reason(reason: &str) -> String {
    match reason {
        "stop" | "length" | "tool_calls" | "content_filter" => reason.to_owned(),
        "function_call" => "tool_calls".to_owned(),
        other => {
            // Only the category name is kept, and only after stripping anything unusual.
            let name: String = other
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(32)
                .collect();
            debug!(finish_reason = %name, "Unknown finish reason treated as stop");
            "stop".to_owned()
        }
    }
}

/// Maps `chat.completions` stream events to chunks. Text and reasoning increments are forwarded
/// as they arrive; the final chunk carries `finish_reason` and the usage, if the upstream sent any.
#[derive(Default)]
struct OpenAiMapper {
    finish: Option<String>,
    usage: Option<TokenUsage>,
}

impl Mapper for OpenAiMapper {
    fn event(&mut self, event: &SseEvent, out: &mut Vec<ChatChunk>) -> Outcome {
        let data = event.data.trim();
        if data == "[DONE]" {
            return Outcome::Complete;
        }
        // Unparseable data is an interruption, never a silently dropped chunk.
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Outcome::Fail(DgosError::StreamInterrupted);
        };
        if let Some(usage) = value.get("usage") {
            if !usage_counts_valid(Some(usage)) { return Outcome::Fail(DgosError::ProviderResponseInvalid); }
            let (usage, unknown) = parse_usage(Some(usage));
            if !unknown { crate::usage::merge_usage(self.usage.get_or_insert_with(TokenUsage::default), &usage); }
        }
        if event.event.as_deref() == Some("error") || value.get("error").is_some_and(|error| !error.is_null()) {
            return Outcome::Fail(DgosError::StreamInterrupted);
        }
        if let Some(choice) = value.pointer("/choices/0") {
            let text = choice
                .pointer("/delta/content")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty());
            let reasoning = reasoning_delta(choice.get("delta"));
            if text.is_some() || reasoning.is_some() {
                out.push(ChatChunk {
                    delta: text.unwrap_or_default().to_owned(),
                    reasoning_delta: reasoning,
                    ..Default::default()
                });
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish = Some(normalize_finish_reason(reason));
            }
        }
        Outcome::Continue
    }

    fn terminal(&mut self) -> ChatChunk {
        ChatChunk {
            finish_reason: Some(self.finish.take().unwrap_or_else(|| "stop".into())),
            usage: self.usage.take(),
            ..Default::default()
        }
    }

    /// A stream that already reported a finish reason completed even if the `[DONE]` marker is
    /// missing; anything else was cut off.
    fn at_eof(&mut self) -> Outcome {
        if self.finish.is_some() {
            Outcome::Complete
        } else {
            Outcome::Fail(DgosError::StreamInterrupted)
        }
    }
}

const TOOL_TEXT_LIMIT: usize = 64 * 1024;
const TOOL_REASONING_LIMIT: usize = 64 * 1024;
const TOOL_ARGUMENT_LIMIT: usize = 16 * 1024;

#[derive(Default)]
struct StreamToolCall {
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Default)]
struct OpenAiToolMapper {
    content: Option<String>,
    reasoning: Vec<(String, Value)>,
    reasoning_bytes: usize,
    call: Option<StreamToolCall>,
    finish: Option<String>,
    usage: Option<TokenUsage>,
}

fn invalid_tool_stream() -> DgosError {
    DgosError::ProviderResponseInvalid
}

fn append_tool_text(target: &mut String, text: &str, limit: usize) -> DgosResult<()> {
    if text.len() > limit.saturating_sub(target.len()) {
        return Err(invalid_tool_stream());
    }
    target.push_str(text);
    Ok(())
}

// Count the encoded size without allocating a second copy of opaque reasoning/signatures.
fn reasoning_wire_len<T: serde::Serialize + ?Sized>(value: &T) -> DgosResult<usize> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| invalid_tool_stream())?;
    Ok(counter.0)
}

/// Merge structured reasoning by its wire index, retaining encrypted data and signatures.
/// Identity metadata is immutable; only actual incremental string fields are concatenated.
/// Returns the exact encoded-size increase, so each fragment is counted only once.
fn merge_stream_reasoning(target: &mut Value, next: &Value, available: usize) -> DgosResult<usize> {
    match (target, next) {
        (Value::String(old), Value::String(part)) => {
            let growth = reasoning_wire_len(next)?.saturating_sub(2);
            if growth > available { return Err(invalid_tool_stream()); }
            append_tool_text(old, part, TOOL_REASONING_LIMIT)?;
            Ok(growth)
        }
        (Value::Object(old), Value::Object(part)) => {
            let mut growth = 0;
            for (key, value) in part {
                if let Some(previous) = old.get_mut(key) {
                    if matches!(key.as_str(), "index" | "id" | "type" | "format") {
                        if *previous != *value { return Err(invalid_tool_stream()); }
                    } else {
                        growth += merge_stream_reasoning(previous, value, available.saturating_sub(growth))?;
                    }
                } else {
                    growth += usize::from(!old.is_empty())
                        + reasoning_wire_len(key)? + 1 + reasoning_wire_len(value)?;
                    if growth > available { return Err(invalid_tool_stream()); }
                    old.insert(key.clone(), value.clone());
                }
            }
            Ok(growth)
        }
        (Value::Array(old), Value::Array(parts)) => {
            let mut growth = 0;
            let mut indices = HashSet::new();
            for part in parts {
                let index = part.get("index");
                if let Some(index) = index {
                    let index = index.as_u64().ok_or_else(invalid_tool_stream)?;
                    if !indices.insert(index) { return Err(invalid_tool_stream()); }
                }
                let existing = index.and_then(|index| old.iter_mut().find(|item| item.get("index") == Some(index)));
                if let Some(existing) = existing {
                    growth += merge_stream_reasoning(existing, part, available.saturating_sub(growth))?;
                } else {
                    growth += usize::from(!old.is_empty()) + reasoning_wire_len(part)?;
                    if growth > available { return Err(invalid_tool_stream()); }
                    old.push(part.clone());
                }
            }
            Ok(growth)
        }
        (old, new) if *old == *new => Ok(0),
        _ => Err(invalid_tool_stream()),
    }
}

impl OpenAiToolMapper {
    fn delta(&mut self, delta: &Value, out: &mut Vec<ToolStreamEvent>) -> DgosResult<()> {
        let delta = delta.as_object().ok_or_else(invalid_tool_stream)?;
        if let Some(role) = delta.get("role") {
            if role.as_str() != Some("assistant") {
                return Err(invalid_tool_stream());
            }
        }
        if delta.contains_key("function_call") || delta.contains_key("tool_call_id") {
            return Err(invalid_tool_stream());
        }
        if let Some(content) = delta.get("content").filter(|value| !value.is_null()) {
            let text = content.as_str().ok_or_else(invalid_tool_stream)?;
            let content = self.content.get_or_insert_with(String::new);
            let room = TOOL_TEXT_LIMIT.saturating_sub(content.len());
            let mut end = room.min(text.len());
            while !text.is_char_boundary(end) { end -= 1; }
            if end > 0 {
                content.push_str(&text[..end]);
                out.push(ToolStreamEvent::Delta(text[..end].to_owned()));
            }
            if end < text.len() { return Err(DgosError::ProviderOutputLimit); }
        }
        // Emit one display field per event, as the existing plain-chat mapper does, while
        // retaining *all* original wire fields for the next tool turn.
        let mut shown = false;
        for name in REASONING_FIELDS {
            let Some(value) = delta.get(name).filter(|value| !value.is_null()) else { continue; };
            if !matches!(value, Value::String(_) | Value::Array(_) | Value::Object(_)) {
                return Err(invalid_tool_stream());
            }
            let available = TOOL_REASONING_LIMIT.saturating_sub(self.reasoning_bytes);
            let growth = if let Some((_, old)) = self.reasoning.iter_mut().find(|(key, _)| key == name) {
                merge_stream_reasoning(old, value, available)?
            } else {
                let growth = name.len() + reasoning_wire_len(value)?;
                if growth > available { return Err(invalid_tool_stream()); }
                self.reasoning.push((name.to_owned(), value.clone()));
                growth
            };
            self.reasoning_bytes += growth;
            if !shown {
                let text = displayable(value);
                if !text.is_empty() {
                    out.push(ToolStreamEvent::ReasoningDelta(text));
                    shown = true;
                }
            }
        }
        if let Some(calls) = delta.get("tool_calls").filter(|value| !value.is_null()) {
            let calls = calls.as_array().ok_or_else(invalid_tool_stream)?;
            if calls.len() > 1 { return Err(DgosError::ProviderToolCallLimit); }
            for raw in calls {
                let raw = raw.as_object().ok_or_else(invalid_tool_stream)?;
                let index = raw.get("index").and_then(Value::as_u64).ok_or_else(invalid_tool_stream)?;
                if index != 0 { return Err(DgosError::ProviderToolCallLimit); }
                let call = self.call.get_or_insert_with(StreamToolCall::default);
                for (key, slot) in [("id", &mut call.id), ("type", &mut call.kind)] {
                    if let Some(value) = raw.get(key) {
                        let value = value.as_str().filter(|value| !value.is_empty() && value.len() <= TOOL_ARGUMENT_LIMIT)
                            .ok_or_else(invalid_tool_stream)?;
                        if slot.is_some() { return Err(invalid_tool_stream()); }
                        *slot = Some(value.to_owned());
                    }
                }
                if let Some(function) = raw.get("function") {
                    let function = function.as_object().ok_or_else(invalid_tool_stream)?;
                    if let Some(name) = function.get("name") {
                        let name = name.as_str().filter(|name| !name.is_empty() && name.len() <= TOOL_ARGUMENT_LIMIT)
                            .ok_or_else(invalid_tool_stream)?;
                        if call.name.is_some() { return Err(invalid_tool_stream()); }
                        call.name = Some(name.to_owned());
                    }
                    if let Some(arguments) = function.get("arguments") {
                        let arguments = arguments.as_str().ok_or(DgosError::ProviderToolArgumentsInvalid)?;
                        if arguments.len() > TOOL_ARGUMENT_LIMIT.saturating_sub(call.arguments.len()) { return Err(DgosError::ProviderToolArgumentsInvalid); }
                        call.arguments.push_str(arguments);
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_complete(&self) -> DgosResult<()> {
        match (self.finish.as_deref(), &self.call) {
            (Some("stop"), None) => Ok(()),
            (Some("tool_calls"), Some(call)) if call.id.is_some() && call.name.is_some()
                && call.kind.as_deref() == Some("function") => {
                let arguments: Value = serde_json::from_str(&call.arguments).map_err(|_| DgosError::ProviderToolArgumentsInvalid)?;
                if arguments.is_object() { Ok(()) } else { Err(DgosError::ProviderToolArgumentsInvalid) }
            }
            _ => Err(invalid_tool_stream()),
        }
    }

    fn map_event(&mut self, event: &SseEvent, out: &mut Vec<ToolStreamEvent>) -> DgosResult<bool> {
        let data = event.data.trim();
        if data == "[DONE]" {
            self.validate_complete()?;
            return Ok(true);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| invalid_tool_stream())?;
        if let Some(usage) = value.get("usage") {
            if !usage_counts_valid(Some(usage)) { return Err(DgosError::ProviderResponseInvalid); }
            let (usage, unknown) = parse_usage(Some(usage));
            if !unknown { crate::usage::merge_usage(self.usage.get_or_insert_with(TokenUsage::default), &usage); }
        }
        if event.event.as_deref() == Some("error") || value.get("error").is_some_and(|error| !error.is_null()) {
            return Err(DgosError::StreamInterrupted);
        }
        let choices = value.get("choices").and_then(Value::as_array).ok_or_else(invalid_tool_stream)?;
        if choices.len() > 1 { return Err(invalid_tool_stream()); }
        if let Some(choice) = choices.first() {
            if self.finish.is_some() || choice.get("index").and_then(Value::as_u64) != Some(0) {
                return Err(invalid_tool_stream());
            }
            self.delta(choice.get("delta").ok_or_else(invalid_tool_stream)?, out)?;
            if let Some(finish) = choice.get("finish_reason").filter(|value| !value.is_null()) {
                let finish = finish.as_str().filter(|reason| matches!(*reason, "stop" | "tool_calls"))
                    .ok_or_else(invalid_tool_stream)?;
                self.finish = Some(finish.to_owned());
                self.validate_complete()?;
            }
        }
        Ok(false)
    }
}

impl Mapper<ToolStreamEvent> for OpenAiToolMapper {
    fn event(&mut self, event: &SseEvent, out: &mut Vec<ToolStreamEvent>) -> Outcome {
        match self.map_event(event, out) {
            Ok(true) => Outcome::Complete,
            Ok(false) => Outcome::Continue,
            Err(error) => Outcome::Fail(error),
        }
    }

    fn terminal(&mut self) -> ToolStreamEvent {
        let tool_calls = self.call.take().map(|call| ToolCall {
            id: call.id.expect("validated tool id"),
            kind: call.kind.expect("validated tool type"),
            function: ToolFunctionCall {
                name: call.name.expect("validated tool name"),
                arguments: call.arguments,
            },
        }).into_iter().collect();
        let reasoning = if self.reasoning.is_empty() { None } else {
            Some(MessageReasoning {
                text: self.reasoning.iter().map(|(_, value)| displayable(value)).find(|text| !text.is_empty()).unwrap_or_default(),
                fields: std::mem::take(&mut self.reasoning),
                blocks: Vec::new(),
            })
        };
        ToolStreamEvent::Complete {
            usage_unknown: self.usage.as_ref().is_none_or(|usage| !usage.total_tokens_reported),
            response: ToolChatResponse {
                message: ToolMessage {
                    role: "assistant".into(), content: self.content.take(), tool_call_id: None,
                    tool_calls, reasoning, images: Vec::new(),
                },
                usage: self.usage.take().unwrap_or_default(),
                audit_warning: false,
            },
        }
    }

    fn at_eof(&mut self) -> Outcome {
        // Unlike tolerant text streaming, tool proposals require both finish and [DONE].
        Outcome::Fail(DgosError::StreamInterrupted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::OPENAI_OFFICIAL;

    #[test]
    fn tool_terminal_keeps_partial_facts_but_reports_unknown_total() {
        let mut mapper = OpenAiToolMapper { usage:Some(TokenUsage { completion_tokens:7,
            completion_tokens_reported:true,..Default::default() }),..Default::default() };
        let ToolStreamEvent::Complete { response,usage_unknown } = mapper.terminal() else { unreachable!() };
        assert!(usage_unknown && response.usage.completion_tokens_reported);
        assert_eq!(response.usage.completion_tokens,7);
        assert!(!response.usage.total_tokens_reported);
    }

    #[test]
    fn usage_presence_cache_and_reasoning_are_independent_subsets() {
        let (zero, unknown) = parse_usage(Some(&json!({"prompt_tokens":0,"completion_tokens":0,
            "prompt_tokens_details":{"cached_tokens":0},
            "completion_tokens_details":{"reasoning_tokens":0}})));
        assert!(!unknown);
        assert!(zero.prompt_tokens_reported && zero.completion_tokens_reported && zero.total_tokens_reported);
        assert_eq!((zero.total_tokens, zero.cache_read_tokens, zero.reasoning_tokens), (0, Some(0), Some(0)));
        assert_eq!(zero.cache_write_tokens, None);
        let (partial, unknown) = parse_usage(Some(&json!({"completion_tokens":7})));
        assert!(!unknown && partial.completion_tokens_reported);
        assert!(!partial.prompt_tokens_reported && !partial.total_tokens_reported);
        let (usage, _) = parse_usage(Some(&json!({"prompt_tokens":20,"completion_tokens":8,
            "prompt_tokens_details":{"cached_tokens":12,"cache_creation_tokens":3},
            "completion_tokens_details":{"reasoning_tokens":5}})));
        assert_eq!(usage.total_tokens, 28, "cache/reasoning are not extra tokens");
        assert_eq!((usage.cache_read_tokens, usage.cache_write_tokens, usage.reasoning_tokens),
            (Some(12), Some(3), Some(5)));
    }

    #[test]
    fn cumulative_stream_usage_replaces_counts_and_keeps_omitted_fields() {
        let mut mapper = OpenAiMapper::default();
        let mut out = Vec::new();
        for usage in [json!({"prompt_tokens":20,"completion_tokens":1,
            "prompt_tokens_details":{"cached_tokens":12}}), json!({"completion_tokens":8})] {
            let event = SseEvent { event: None, data: json!({"choices":[],"usage":usage}).to_string() };
            assert!(matches!(mapper.event(&event, &mut out), Outcome::Continue));
        }
        let usage = mapper.terminal().usage.unwrap();
        assert_eq!((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens), (20,8,28));
        assert_eq!(usage.cache_read_tokens, Some(12));
        assert!(usage.total_tokens_reported);
    }

    #[tokio::test]
    async fn test_classify_model() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_body(r#"{"data":[{"id":"gpt-4"},{"id":"dall-e-3"},{"id":"whisper-1"}]}"#)
            .create_async()
            .await;
        let adapter = OpenAIAdapter::new(server.url(), "test-key".to_string(), None);
        let models = adapter.list_models().await.unwrap();
        let kind = |id: &str| {
            models
                .iter()
                .find(|model| model.model_id == id)
                .unwrap()
                .model_type
                .clone()
        };
        assert_eq!(kind("gpt-4"), "text-generation");
        assert_eq!(kind("dall-e-3"), "image-generation");
        assert_eq!(kind("whisper-1"), "speech-transcription");
    }

    #[test]
    fn token_boundaries_never_clamp_reported_facts() {
        let max = u32::MAX as u64;
        let (usage,unknown) = parse_usage(Some(&json!({"prompt_tokens":max,"completion_tokens":0})));
        assert!(!unknown && usage.total_tokens_reported);
        assert_eq!(usage.total_tokens,u32::MAX);
        for usage in [json!({"prompt_tokens":max+1,"completion_tokens":0}),
            json!({"prompt_tokens":1,"completion_tokens":0,"prompt_tokens_details":{"cached_tokens":max+1}}),
            json!({"completion_tokens_details":{"reasoning_tokens":max+1}})] {
            let (usage,unknown) = parse_usage(Some(&usage));
            assert!(unknown && !usage.has_reported_usage());
        }
        let (usage,_) = parse_usage(Some(&json!({"prompt_tokens":max,"completion_tokens":1})));
        assert!(usage.prompt_tokens_reported && usage.completion_tokens_reported);
        assert!(!usage.total_tokens_reported,"an overflowing sum is unknown, not a saturated total");
        let mut mapper = OpenAiMapper::default();
        let event = SseEvent { event:None,data:json!({"choices":[],"usage":{"completion_tokens":max+1}}).to_string() };
        assert!(matches!(mapper.event(&event,&mut Vec::new()),Outcome::Fail(DgosError::ProviderResponseInvalid)));
    }

    #[test]
    fn usage_without_counts_is_zero_and_flagged_but_partial_usage_is_kept() {
        let (usage, unknown) = parse_usage(None);
        assert!(unknown && usage.total_tokens == 0);
        let (_, unknown) = parse_usage(Some(&json!({})));
        assert!(unknown);
        let (usage, unknown) =
            parse_usage(Some(&json!({"prompt_tokens": 4, "completion_tokens": 6})));
        assert!(!unknown);
        assert_eq!(usage.total_tokens, 10);
        let (usage, unknown) = parse_usage(Some(&json!({"total_tokens": 99999999999u64})));
        assert!(unknown && !usage.has_reported_usage());
        assert_eq!(usage.total_tokens, 0);
    }

    #[test]
    fn temperature_is_serialised_in_its_shortest_form() {
        let request = ChatRequest {
            temperature: Some(0.7),
            max_tokens: Some(8),
            ..Default::default()
        };
        let body = chat_body(&OPENAI_GENERIC, "m", &request, false)
            .unwrap()
            .to_string();
        assert!(body.contains("\"temperature\":0.7"), "{body}");
        assert!(!body.contains("stream_options"));
        assert!(chat_body(&OPENAI_GENERIC, "m", &request, true)
            .unwrap()
            .to_string()
            .contains("\"stream_options\":{\"include_usage\":true}"));
    }

    #[test]
    fn profiles_choose_the_token_limit_name() {
        let request = ChatRequest {
            max_tokens: Some(8),
            ..Default::default()
        };
        let generic = chat_body(&OPENAI_GENERIC, "m", &request, false).unwrap();
        assert_eq!(generic["max_tokens"], 8);
        assert!(generic.get("max_completion_tokens").is_none());
        let official = chat_body(&OPENAI_OFFICIAL, "m", &request, false).unwrap();
        assert_eq!(official["max_completion_tokens"], 8);
        assert!(official.get("max_tokens").is_none());
    }

    #[test]
    fn reasoning_fields_are_kept_by_name_and_displayed_without_encrypted_parts() {
        let message = json!({"role": "assistant", "content": "x",
            "reasoning_content": "step one",
            "reasoning_details": [
                {"type": "reasoning.text", "text": " and two"},
                {"type": "reasoning.encrypted", "data": "opaque"}]});
        let reasoning = extract_reasoning(&message).unwrap();
        assert_eq!(reasoning.text, "step one");
        let names: Vec<_> = reasoning
            .fields
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["reasoning_content", "reasoning_details"]);
        let details_only = json!({"reasoning_details": [
            {"type": "reasoning.text", "text": "a"}, {"type": "reasoning.summary", "summary": "b"},
            {"type": "reasoning.encrypted", "data": "opaque"}]});
        assert_eq!(extract_reasoning(&details_only).unwrap().text, "ab");
        // No reasoning field (or only a null) means nothing to send back.
        assert!(extract_reasoning(&json!({"content": "x"})).is_none());
        assert!(extract_reasoning(&json!({"reasoning_content": null})).is_none());
    }

    #[test]
    fn only_the_fields_the_upstream_sent_go_back_on_the_assistant_message() {
        let plain = ToolMessage {
            role: "assistant".into(),
            content: Some("hi".into()),
            tool_call_id: None,
            tool_calls: vec![],
            reasoning: None,
            images: Vec::new(),
        };
        let value = tool_message_json(&plain);
        for name in REASONING_FIELDS {
            assert!(value.get(name).is_none(), "{name}");
        }
        let with = ToolMessage {
            reasoning: Some(MessageReasoning {
                text: "shown".into(),
                fields: vec![
                    ("reasoning".into(), json!("kept as is")),
                    ("not_a_reasoning_field".into(), json!("dropped")),
                ],
                blocks: vec![],
            }),
            ..plain
        };
        let value = tool_message_json(&with);
        assert_eq!(value["reasoning"], "kept as is");
        assert!(value.get("not_a_reasoning_field").is_none());
        assert!(value.get("reasoning_content").is_none());
        // A tool-role message never carries reasoning.
        let tool = ToolMessage {
            role: "tool".into(),
            ..with
        };
        assert!(tool_message_json(&tool).get("reasoning").is_none());
    }
}
