//! Explicit opt-in smoke against real OpenAI-compatible and Anthropic-compatible services
//! (FR-001 §2.4.6, "inferred" values). It talks to the adapters directly: no database, no
//! vault, nothing written to disk.
//!
//! Each service runs only when all three of its variables are set:
//!   DGOS_SMOKE_OPENAI_BASE_URL / DGOS_SMOKE_OPENAI_KEY / DGOS_SMOKE_OPENAI_MODEL
//!   DGOS_SMOKE_ANTHROPIC_BASE_URL / DGOS_SMOKE_ANTHROPIC_KEY / DGOS_SMOKE_ANTHROPIC_MODEL
//! A service with no variables is reported as "NOT RUN", never as a pass. The key is read
//! from the environment only and is never printed; neither are request or response bodies.
//!
//! Run: cargo run -p dgos-providers --example real_provider_smoke
//!
//! What a pass shows: the model list parses and classifies, a synchronous call answers, and a
//! stream ends with exactly one finish reason. It does not calibrate the thinking budgets or the
//! 4096 `max_tokens` default; read the printed status codes to judge those.

use dgos_providers::{
    AnthropicAdapter, ChatMessage, ChatRequest, OpenAIAdapter, ProviderAdapter, RetryConfig,
    TokenUsage,
};
use dgos_types::{
    ReasoningConfig, ResponseFormat, ToolChatRequest, ToolDefinition, ToolFunction, ToolMessage,
};
use futures::StreamExt;
use serde_json::{json, Value};
use std::process::ExitCode;
use std::time::Duration;

struct Service {
    name: &'static str,
    /// Anthropic Messages instead of OpenAI Chat Completions; only the raw usage request needs it.
    anthropic: bool,
    base_url: String,
    /// Used only to authenticate the raw usage request. Never printed.
    key: String,
    model: String,
    adapter: Box<dyn ProviderAdapter>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn service(
    name: &'static str,
    prefix: &str,
    anthropic: bool,
    build: fn(String, String) -> Box<dyn ProviderAdapter>,
) -> Option<Service> {
    let base_url = env(&format!("DGOS_SMOKE_{prefix}_BASE_URL"));
    let key = env(&format!("DGOS_SMOKE_{prefix}_KEY"));
    let model = env(&format!("DGOS_SMOKE_{prefix}_MODEL"));
    match (base_url, key, model) {
        (Some(base_url), Some(key), Some(model)) => Some(Service {
            name,
            anthropic,
            adapter: build(base_url.clone(), key.clone()),
            base_url,
            key,
            model,
        }),
        _ => {
            println!("NOT RUN {name}: set all three DGOS_SMOKE_{prefix}_* variables");
            None
        }
    }
}

fn request(stream: bool) -> ChatRequest {
    ChatRequest {
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "Reply with the single word: ok".into(),
        }],
        max_tokens: Some(64),
        stream: Some(stream),
        ..Default::default()
    }
}

/// Numeric leaves of a JSON value as `path=value`. Strings and booleans are skipped, so response
/// text cannot reach the output; the path segments are the upstream's own field names.
fn numeric_leaves(value: &Value, path: &str, out: &mut Vec<String>) {
    match value {
        Value::Number(number) => out.push(format!("{path}={number}")),
        Value::Object(map) => {
            for (key, inner) in map {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                numeric_leaves(inner, &next, out);
            }
        }
        Value::Array(items) => {
            for (index, inner) in items.iter().enumerate() {
                numeric_leaves(inner, &format!("{path}[{index}]"), out);
            }
        }
        _ => {}
    }
}

fn tokens(usage: &TokenUsage) -> String {
    format!(
        "tokens {}/{}/{}",
        usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
    )
}
/// One small request sent without the adapter, to see the usage numbers the service itself
/// reports. Only numeric usage fields are printed, never the reply.
async fn raw_usage(service: &Service, prompt: &str) -> String {
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .build()
    else {
        return "client could not be built".into();
    };
    let body = json!({
        "model": service.model,
        "max_tokens": 64,
        "messages": [{"role": "user", "content": prompt}],
    });
    let base = service.base_url.trim_end_matches('/');
    let request = if service.anthropic {
        let base = base.strip_suffix("/v1").unwrap_or(base);
        client
            .post(format!("{base}/v1/messages"))
            .header("authorization", format!("Bearer {}", service.key))
            .header("x-api-key", &service.key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
    } else {
        client
            .post(format!("{base}/chat/completions"))
            .bearer_auth(&service.key)
            .json(&body)
    };
    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let reply: Value = response.json().await.unwrap_or(Value::Null);
            let mut leaves = Vec::new();
            numeric_leaves(&reply["usage"], "usage", &mut leaves);
            if leaves.is_empty() {
                format!("status {status}, no numeric usage fields")
            } else {
                format!("status {status}, {}", leaves.join(" "))
            }
        }
        Err(error) => format!(
            "request failed (timeout={}, connect={})",
            error.is_timeout(),
            error.is_connect()
        ),
    }
}
/// Optional probes (`DGOS_SMOKE_PROBES=1`). They report what the service did; none of them counts
/// as a failed step, because a rejection is itself the finding.
async fn probe_reasoning_and_structured(service: &Service) {
    let name = service.name;
    let user = |text: &str| ChatMessage {
        role: "user".into(),
        content: text.into(),
    };
    let reasoning = ChatRequest {
        messages: vec![user("What is 17 * 23? Answer with the number only.")],
        max_tokens: Some(4096),
        reasoning: Some(ReasoningConfig {
            effort: Some("low".into()),
            budget_tokens: None,
        }),
        ..Default::default()
    };
    match service.adapter.call(&service.model, reasoning).await {
        Ok(reply) => println!(
            "PROBE {name} reasoning(low): accepted, reasoning text {} chars, content {} chars, {}",
            reply.reasoning.as_deref().map_or(0, |t| t.chars().count()),
            reply.content.chars().count(),
            tokens(&reply.usage),
        ),
        Err(error) => println!(
            "PROBE {name} reasoning(low): rejected, code {}",
            error.error_code()
        ),
    }
    let schema = json!({
        "type": "object",
        "properties": {"answer": {"type": "integer"}},
        "required": ["answer"],
        "additionalProperties": false,
    });
    let structured = ChatRequest {
        messages: vec![user("What is 17 * 23? Put it in the answer field.")],
        max_tokens: Some(256),
        response_format: Some(ResponseFormat {
            kind: "json_schema".into(),
            name: "answer".into(),
            schema,
            strict: Some(true),
        }),
        ..Default::default()
    };
    match service.adapter.call(&service.model, structured).await {
        Ok(reply) => println!(
            "PROBE {name} structured output: accepted, parsed object present={}",
            reply.structured.is_some()
        ),
        Err(error) => println!(
            "PROBE {name} structured output: rejected, code {}",
            error.error_code()
        ),
    }
}
fn weather_tools() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        kind: "function".into(),
        function: ToolFunction {
            name: "get_weather".into(),
            description: "Get the current weather for a city".into(),
            parameters: json!({
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"],
                "additionalProperties": false,
            }),
        },
    }]
}

fn tool_message(role: &str, content: Option<String>, tool_call_id: Option<String>) -> ToolMessage {
    ToolMessage {
        role: role.into(),
        content,
        tool_call_id,
        tool_calls: vec![],
        reasoning: None,
    }
}

/// The shape the workbench sends: one system message, then the selected skill's guidance and the
/// user's prompt as two consecutive user messages. The Anthropic adapter lifts the system message
/// and merges the two user turns; this checks a real service accepts the result.
async fn probe_workbench_shape(service: &Service) {
    let name = service.name;
    let message = |role: &str, content: &str| ChatMessage {
        role: role.into(),
        content: content.into(),
    };
    let shaped = ChatRequest {
        messages: vec![
            message("system", "You are a terse assistant."),
            message("user", "Selected skill guidance: answer in one word."),
            message("user", "What colour is the sky on a clear day?"),
        ],
        max_tokens: Some(64),
        ..Default::default()
    };
    match service.adapter.call(&service.model, shaped).await {
        Ok(reply) => println!(
            "PROBE {name} workbench shape (system + two user messages): accepted, content {} chars, finish_reason {:?}, {}",
            reply.content.chars().count(),
            reply.finish_reason,
            tokens(&reply.usage)
        ),
        Err(error) => println!(
            "PROBE {name} workbench shape (system + two user messages): rejected, code {}",
            error.error_code()
        ),
    }
}

/// Two rounds: the model asks for the tool, then the tool result goes back with the assistant
/// message that asked for it. This is the conversion the workbench tool loop depends on.
async fn probe_tool_round_trip(service: &Service) {
    let name = service.name;
    let request = |messages: &[ToolMessage]| ToolChatRequest {
        messages: messages.to_vec(),
        tools: weather_tools(),
        max_tokens: 256,
    };
    let mut messages = vec![tool_message(
        "user",
        Some("What is the weather in Paris? Use the tool.".into()),
        None,
    )];
    let asked = match service
        .adapter
        .call_tools(&service.model, request(&messages))
        .await
    {
        Ok(reply) => reply,
        Err(error) => {
            println!(
                "PROBE {name} tool round trip: first call rejected, code {}",
                error.error_code()
            );
            return;
        }
    };
    let Some(call) = asked.message.tool_calls.first().cloned() else {
        println!("PROBE {name} tool round trip: the model made no tool call, nothing to continue");
        return;
    };
    messages.push(asked.message);
    let result = json!({"temperature_c": 18, "sky": "clear"}).to_string();
    messages.push(tool_message("tool", Some(result), Some(call.id)));
    match service.adapter.call_tools(&service.model, request(&messages)).await {
        Ok(reply) => println!(
            "PROBE {name} tool round trip: accepted, second turn made {} further tool call(s), answer {} chars, {}",
            reply.message.tool_calls.len(),
            reply.message.content.as_deref().map_or(0, |text| text.chars().count()),
            tokens(&reply.usage)
        ),
        Err(error) => println!(
            "PROBE {name} tool round trip: second call rejected, code {}",
            error.error_code()
        ),
    }
}

async fn probe_tools_and_bare(service: &Service) {
    let name = service.name;
    let tools = ToolChatRequest {
        messages: vec![tool_message(
            "user",
            Some("What is the weather in Paris? Use the tool.".into()),
            None,
        )],
        tools: weather_tools(),
        max_tokens: 256,
    };
    match service.adapter.call_tools(&service.model, tools).await {
        Ok(reply) => {
            let calls = &reply.message.tool_calls;
            let valid = calls
                .iter()
                .all(|call| serde_json::from_str::<Value>(&call.function.arguments).is_ok());
            println!(
                "PROBE {name} tools: accepted, {} tool call(s), arguments are valid JSON={valid}, {}",
                calls.len(),
                tokens(&reply.usage)
            );
        }
        Err(error) => println!("PROBE {name} tools: rejected, code {}", error.error_code()),
    }
    // No `max_tokens`: the product's own default applies (4096 for the generic Anthropic profile).
    let bare = ChatRequest {
        max_tokens: None,
        ..request(false)
    };
    match service.adapter.call(&service.model, bare).await {
        Ok(reply) => println!(
            "PROBE {name} no max_tokens: accepted, finish_reason {:?}, {}",
            reply.finish_reason,
            tokens(&reply.usage)
        ),
        Err(error) => println!(
            "PROBE {name} no max_tokens: rejected, code {}",
            error.error_code()
        ),
    }
}

/// Runs the three steps; returns the number of failed steps.
async fn run(service: &Service) -> usize {
    let mut failed = 0;
    let name = service.name;
    println!("-- {name}: {}", service.base_url);

    match service.adapter.list_models().await {
        Ok(models) => {
            let kinds: std::collections::BTreeSet<_> =
                models.iter().map(|m| m.model_type.as_str()).collect();
            let listed = models.iter().any(|m| m.model_id == service.model);
            println!(
                "PASS {name} list_models: {} models, kinds {kinds:?}, requested model listed={listed}",
                models.len()
            );
        }
        Err(error) => {
            // A service without a model list answers 404 or 405; the product falls back to a probe.
            println!("NOTE {name} list_models: code {}", error.error_code());
        }
    }

    match service.adapter.call(&service.model, request(false)).await {
        Ok(reply) => println!(
            "PASS {name} call: {} chars, tokens {}/{}/{}, usage_unknown={}",
            reply.content.chars().count(),
            reply.usage.prompt_tokens,
            reply.usage.completion_tokens,
            reply.usage.total_tokens,
            reply.usage_unknown,
        ),
        Err(error) => {
            failed += 1;
            println!("FAIL {name} call: code {}", error.error_code());
        }
    }

    match service.adapter.stream(&service.model, request(true)).await {
        Ok(mut stream) => {
            let (mut chunks, mut finishes, mut broken) = (0, 0, false);
            let mut last_usage = None;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(chunk) => {
                        chunks += 1;
                        finishes += usize::from(chunk.finish_reason.is_some());
                        if chunk.usage.is_some() {
                            last_usage = chunk.usage;
                        }
                    }
                    Err(error) => {
                        broken = true;
                        println!("FAIL {name} stream: code {}", error.error_code());
                        break;
                    }
                }
            }
            if broken || finishes != 1 {
                failed += 1;
                if !broken {
                    println!("FAIL {name} stream: {finishes} finish reasons in {chunks} chunks");
                }
            } else {
                let usage = last_usage
                    .as_ref()
                    .map_or_else(|| "the terminal chunk carried no usage".to_owned(), tokens);
                println!("PASS {name} stream: {chunks} chunks, one finish reason, {usage}");
            }
        }
        Err(error) => {
            failed += 1;
            println!("FAIL {name} stream: code {}", error.error_code());
        }
    }

    // What the service itself reports for a normal prompt and a one-character prompt. If the
    // input count barely changes, the service adds a fixed amount in front of the request.
    println!(
        "RAW {name} usage, normal prompt: {}",
        raw_usage(service, "Reply with the single word: ok").await
    );
    println!(
        "RAW {name} usage, one-character prompt: {}",
        raw_usage(service, "x").await
    );

    if env("DGOS_SMOKE_PROBES").as_deref() == Some("1") {
        probe_reasoning_and_structured(service).await;
        probe_tools_and_bare(service).await;
        probe_workbench_shape(service).await;
        probe_tool_round_trip(service).await;
    } else {
        println!("NOTE {name} probes skipped: set DGOS_SMOKE_PROBES=1 for reasoning, structured output, tools and no-max_tokens");
    }
    failed
}

#[tokio::main]
async fn main() -> ExitCode {
    let openai = service("openai_compatible", "OPENAI", false, |base_url, key| {
        Box::new(OpenAIAdapter::new(base_url, key, None::<RetryConfig>))
    });
    let anthropic = service("anthropic", "ANTHROPIC", true, |base_url, key| {
        Box::new(AnthropicAdapter::new(base_url, key, None::<RetryConfig>))
    });
    let services: Vec<Service> = [openai, anthropic].into_iter().flatten().collect();
    if services.is_empty() {
        println!("SMOKE RESULT: not run (no service configured)");
        return ExitCode::SUCCESS;
    }
    let mut failed = 0;
    for service in &services {
        failed += run(service).await;
    }
    println!(
        "SMOKE RESULT: {} service(s) run, {failed} failed step(s)",
        services.len()
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
