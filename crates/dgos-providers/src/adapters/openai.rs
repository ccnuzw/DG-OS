//! `openai_compatible` adapter (OpenAI Chat Completions). Request shape differences between
//! services are read from the [`Profile`]; nothing in here branches on a vendor name.

use async_trait::async_trait;
use dgos_types::{
    build_model, valid_model_id, ChatChunk, ChatRequest, ChatResponse, DgosError, DgosResult,
    MessageReasoning, Model, ModelSource, RetryConfig, TokenUsage, ToolChatRequest,
    ToolChatResponse, ToolMessage, PROTOCOL_OPENAI_COMPATIBLE,
};
use serde_json::{json, Value};
use std::collections::HashSet;
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
        let url = self.transport.url("/chat/completions")?;
        let state = BodyState::new(tool_body(self.profile(), model_id, &request));
        let options = &self.transport.options;
        let response = self
            .transport
            .post_json(&url, &state, model_id, Some(options.request_timeout), None)
            .await
            .map_err(|failure| failure.error)?;
        let limit = options.max_body_bytes.min(MAX_TOOL_BODY_BYTES);
        let bytes = read_limited(response, limit).await?;
        parse_tool_response(&bytes)
    }

    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest,
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
                    .post_json(&url, &state, model_id, None, Some(idle))
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
        "messages": request.messages,
        "stream": stream
    });
    let object = body
        .as_object_mut()
        .expect("json! object literal is an object");
    if let Some(temperature) = request.temperature {
        // Widening f32 to f64 would print 0.7 as 0.699999988...; use the shortest f32 form.
        if let Some(number) = format!("{temperature}")
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
        {
            object.insert("temperature".into(), Value::Number(number));
        }
    }
    if let Some(max_tokens) = request.max_tokens {
        object.insert(profile.max_tokens_field.into(), json!(max_tokens));
    }
    if stream && profile.send_stream_options {
        object.insert("stream_options".into(), json!({"include_usage": true}));
    }
    if let Some(reasoning) = &request.reasoning {
        match profile.reasoning_params(reasoning)? {
            ReasoningParams::OpenAiEffort(effort) => {
                object.insert("reasoning_effort".into(), json!(effort));
            }
            _ => return Err(DgosError::InternalError("Profile mismatch".into())),
        }
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

fn tool_body(profile: &Profile, model_id: &str, request: &ToolChatRequest) -> Value {
    let mut body = json!({
        "model": model_id,
        "messages": request.messages.iter().map(tool_message_json).collect::<Vec<_>>(),
        "tools": request.tools,
        "stream": false
    });
    let object = body
        .as_object_mut()
        .expect("json! object literal is an object");
    object.insert(profile.max_tokens_field.into(), json!(request.max_tokens));
    if profile.send_tool_choice {
        object.insert("tool_choice".into(), json!("auto"));
    }
    if profile.send_parallel_tool_calls {
        object.insert("parallel_tool_calls".into(), json!(false));
    }
    body
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
fn parse_chat_response(bytes: &[u8]) -> DgosResult<ChatResponse> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| DgosError::ProviderResponseInvalid)?;
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
    let (usage, usage_unknown) = parse_usage(value.get("usage"));
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
        usage: parse_usage(value.get("usage")).0,
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

/// A missing or empty `usage` is `(0, 0, 0)` with the unknown flag set.
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
    let (prompt, completion, total) = (
        field("prompt_tokens"),
        field("completion_tokens"),
        field("total_tokens"),
    );
    if prompt.is_none() && completion.is_none() && total.is_none() {
        return (TokenUsage::default(), true);
    }
    let prompt_tokens = prompt.unwrap_or(0);
    let completion_tokens = completion.unwrap_or(0);
    (
        TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: total.unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens)),
        },
        false,
    )
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
        if event.event.as_deref() == Some("error") {
            return Outcome::Fail(DgosError::StreamInterrupted);
        }
        // Unparseable data is an interruption, never a silently dropped chunk.
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Outcome::Fail(DgosError::StreamInterrupted);
        };
        if value.get("error").is_some_and(|error| !error.is_null()) {
            return Outcome::Fail(DgosError::StreamInterrupted);
        }
        if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
            let (usage, unknown) = parse_usage(Some(usage));
            if !unknown {
                self.usage = Some(usage);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::OPENAI_OFFICIAL;

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
    fn usage_without_counts_is_zero_and_flagged_but_partial_usage_is_kept() {
        let (usage, unknown) = parse_usage(None);
        assert!(unknown && usage.total_tokens == 0);
        let (_, unknown) = parse_usage(Some(&json!({})));
        assert!(unknown);
        let (usage, unknown) =
            parse_usage(Some(&json!({"prompt_tokens": 4, "completion_tokens": 6})));
        assert!(!unknown);
        assert_eq!(usage.total_tokens, 10);
        let (usage, _) = parse_usage(Some(&json!({"total_tokens": 99999999999u64})));
        assert_eq!(usage.total_tokens, u32::MAX);
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
