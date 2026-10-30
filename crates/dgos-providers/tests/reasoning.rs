//! AC23: reasoning requests and reasoning output on both protocols, synchronous and streaming.
//! The effort string reaches the upstream in the form the profile maps it to, a level without a
//! mapping is refused locally, and reasoning text never ends up in `content` / `delta`.
mod common;

use common::*;
use dgos_providers::{AdapterOptions, ProviderAdapter};
use dgos_types::{ChatChunk, ChatRequest, ChatResponse, DgosError, DgosResult, ReasoningConfig};
use serde_json::{json, Value};
use std::time::Duration;

// ---------------------------------------------------------------------------------------------
// Helpers local to this file
// ---------------------------------------------------------------------------------------------

fn options() -> AdapterOptions {
    AdapterOptions {
        request_timeout: Duration::from_secs(5),
        stream_idle_timeout: Duration::from_secs(2),
        ..Default::default()
    }
}

fn body_of(stub: &Stub, index: usize) -> Value {
    serde_json::from_str(&stub.requests()[index].body).unwrap()
}

/// `official = false`: generic profile (no retries); `true`: the official OpenAI profile.
fn openai_boxed(official: bool, url: &str) -> Box<dyn ProviderAdapter> {
    if official {
        Box::new(openai_official(url, options()))
    } else {
        Box::new(adapter_with(url, 0, 1, options()))
    }
}

/// `official = false`: generic Anthropic profile; `true`: the official one.
fn anthropic_boxed(official: bool, url: &str) -> Box<dyn ProviderAdapter> {
    if official {
        Box::new(anthropic_official(url, options()))
    } else {
        Box::new(anthropic_with(url, 0, 1, options()))
    }
}

fn reasoning_request(reasoning: ReasoningConfig, max_tokens: u32) -> ChatRequest {
    ChatRequest {
        reasoning: Some(reasoning),
        max_tokens: Some(max_tokens),
        ..request()
    }
}

/// A request that carries only an effort string and leaves room for every fixed budget.
fn effort(level: &str) -> ChatRequest {
    reasoning_request(
        ReasoningConfig {
            effort: Some(level.into()),
            budget_tokens: None,
        },
        32_000,
    )
}

fn error_of<T>(result: DgosResult<T>) -> DgosError {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(error) => error,
    }
}

fn assert_not_supported(error: &DgosError, label: &str) {
    assert_eq!(error.error_code(), 1001020, "{label}: {error}");
    assert!(!error.to_string().contains(API_KEY), "{label}");
}

/// The request is refused before anything is sent, on the synchronous and the streaming path.
async fn assert_refused_locally(
    adapter: &dyn ProviderAdapter,
    stub: &Stub,
    request: ChatRequest,
    label: &str,
) {
    let error = error_of(adapter.call("m", request.clone()).await);
    assert_not_supported(&error, label);
    let error = error_of(adapter.stream("m", request).await);
    assert_not_supported(&error, label);
    assert_eq!(stub.hits(), 0, "{label}: nothing may reach the upstream");
}

fn openai_message(message: Value) -> String {
    json!({"choices": [{"message": message, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 50, "total_tokens": 60,
            "completion_tokens_details": {"reasoning_tokens": 40}}})
    .to_string()
}

fn anthropic_message(blocks: Value) -> String {
    json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "m",
        "content": blocks, "stop_reason": "end_turn",
        "usage": {"input_tokens": 3, "output_tokens": 40}})
    .to_string()
}

async fn openai_call(message: Value) -> ChatResponse {
    let stub = serve(vec![response(200, &[], &openai_message(message))]).await;
    openai_boxed(false, &stub.url)
        .call("m", request())
        .await
        .unwrap()
}

async fn anthropic_call(official: bool, blocks: Value) -> ChatResponse {
    let stub = serve(vec![response(200, &[], &anthropic_message(blocks))]).await;
    anthropic_boxed(official, &stub.url)
        .call("m", request())
        .await
        .unwrap()
}

/// An OpenAI stream event with the given `delta` object.
fn openai_event(delta: Value) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices": [{"delta": delta, "finish_reason": null}]})
    )
}

fn anthropic_event(name: &str, data: Value) -> String {
    event(name, data)
}

fn block_start(kind: &str) -> String {
    let block = if kind == "thinking" {
        json!({"type": "thinking", "thinking": ""})
    } else {
        json!({"type": "text", "text": ""})
    };
    anthropic_event(
        "content_block_start",
        json!({"type": "content_block_start", "index": 0, "content_block": block}),
    )
}

fn block_stop() -> String {
    anthropic_event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 0}),
    )
}

fn signature_delta(signature: &str) -> String {
    anthropic_event(
        "content_block_delta",
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "signature_delta", "signature": signature}}),
    )
}

/// The event sent in two packets cut at byte `at`, with a pause between them.
fn split_at_byte(event: &str, at: usize) -> Vec<Step> {
    let bytes = event.as_bytes();
    vec![
        Step::Send(bytes[..at].to_vec()),
        sleep(10),
        Step::Send(bytes[at..].to_vec()),
    ]
}

/// A script made of whole events and events delivered in two packets.
fn script(parts: Vec<Vec<Step>>) -> Vec<Step> {
    let mut steps = vec![sse_head()];
    for part in parts {
        steps.extend(part);
    }
    steps
}

fn whole(event: &str) -> Vec<Step> {
    vec![text(event)]
}

fn anthropic_minimal_stream() -> Vec<Step> {
    sse_script(&[
        message_start(1),
        text_delta("ok"),
        message_delta("end_turn", 1),
        message_stop(),
    ])
}

/// `(delta, reasoning_delta)` of every chunk before the terminal one. Asserts the stream ended
/// without error, that no chunk is empty and that only the last chunk carries finish / usage.
fn body_chunks(items: &[DgosResult<ChatChunk>]) -> (Vec<(String, Option<String>)>, ChatChunk) {
    let chunks: Vec<ChatChunk> = items
        .iter()
        .map(|item| item.as_ref().expect("stream item must be Ok").clone())
        .collect();
    let (terminal, body) = chunks.split_last().expect("a terminal chunk");
    assert!(terminal.delta.is_empty(), "terminal chunk carries no text");
    assert!(terminal.reasoning_delta.is_none());
    assert!(
        terminal.finish_reason.is_some(),
        "terminal chunk finish_reason"
    );
    let mut pairs = Vec::new();
    for chunk in body {
        assert!(chunk.finish_reason.is_none() && chunk.usage.is_none());
        assert!(
            !chunk.delta.is_empty()
                || chunk
                    .reasoning_delta
                    .as_deref()
                    .is_some_and(|r| !r.is_empty()),
            "no empty chunks: {chunk:?}"
        );
        pairs.push((chunk.delta.clone(), chunk.reasoning_delta.clone()));
    }
    (pairs, terminal.clone())
}

fn pair(delta: &str, reasoning: Option<&str>) -> (String, Option<String>) {
    (delta.to_owned(), reasoning.map(str::to_owned))
}

// ---------------------------------------------------------------------------------------------
// Request mapping
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn openai_effort_goes_upstream_unchanged_without_a_host_side_list() {
    let levels = [
        "minimal", "low", "medium", "high", "xhigh", "max", "none", "turbo",
    ];
    for official in [false, true] {
        let stub = serve(vec![response(200, &[], &ok_chat())]).await;
        let adapter = openai_boxed(official, &stub.url);
        for level in levels {
            adapter.call("m", effort(level)).await.unwrap();
        }
        assert_eq!(stub.hits(), levels.len());
        for (index, level) in levels.iter().enumerate() {
            let body = body_of(&stub, index);
            assert_eq!(body["reasoning_effort"], *level, "official={official}");
            assert!(body.get("thinking").is_none());
            assert!(body.get("output_config").is_none());
        }
    }
}

#[tokio::test]
async fn openai_without_reasoning_sends_no_reasoning_fields() {
    for official in [false, true] {
        let stub = serve(vec![response(200, &[], &ok_chat())]).await;
        openai_boxed(official, &stub.url)
            .call("m", request())
            .await
            .unwrap();
        let body = body_of(&stub, 0);
        for key in ["reasoning_effort", "reasoning", "thinking", "output_config"] {
            assert!(
                body.get(key).is_none(),
                "official={official}: {key} must be absent"
            );
        }
    }
}

#[tokio::test]
async fn openai_stream_request_carries_the_effort_too() {
    let stub = serve(vec![sse_script(&[
        delta("ok"),
        finish("stop"),
        DONE.into(),
    ])])
    .await;
    let items = drain(
        openai_boxed(false, &stub.url)
            .stream("m", effort("xhigh"))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "ok");
    let body = body_of(&stub, 0);
    assert_eq!(body["reasoning_effort"], "xhigh");
    assert_eq!(body["stream"], true);
}

#[tokio::test]
async fn generic_anthropic_maps_effort_to_a_fixed_thinking_budget() {
    let cases = [("low", 2048), ("medium", 8192), ("high", 16384)];
    let stub = serve(vec![response(200, &[], &ok_message())]).await;
    let adapter = anthropic_boxed(false, &stub.url);
    for (level, _) in cases {
        adapter.call("m", effort(level)).await.unwrap();
    }
    for (index, (level, budget)) in cases.iter().enumerate() {
        let body = body_of(&stub, index);
        assert_eq!(
            body["thinking"],
            json!({"type": "enabled", "budget_tokens": budget}),
            "{level}"
        );
        assert!(body.get("output_config").is_none(), "{level}");
        assert_eq!(body["max_tokens"], 32_000);
    }
}

#[tokio::test]
async fn generic_anthropic_explicit_budget_wins_over_effort() {
    let stub = serve(vec![response(200, &[], &ok_message())]).await;
    let adapter = anthropic_boxed(false, &stub.url);
    adapter
        .call(
            "m",
            reasoning_request(
                ReasoningConfig {
                    effort: Some("high".into()),
                    budget_tokens: Some(3000),
                },
                32_000,
            ),
        )
        .await
        .unwrap();
    // A budget without any effort is enough on its own.
    adapter
        .call(
            "m",
            reasoning_request(
                ReasoningConfig {
                    effort: None,
                    budget_tokens: Some(1500),
                },
                32_000,
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        body_of(&stub, 0)["thinking"],
        json!({"type": "enabled", "budget_tokens": 3000})
    );
    assert_eq!(
        body_of(&stub, 1)["thinking"],
        json!({"type": "enabled", "budget_tokens": 1500})
    );
    assert!(body_of(&stub, 0).get("output_config").is_none());
    assert!(body_of(&stub, 1).get("output_config").is_none());
}

#[tokio::test]
async fn official_anthropic_uses_adaptive_thinking_and_output_config_effort() {
    let levels = ["low", "medium", "high", "xhigh", "max"];
    let stub = serve(vec![response(200, &[], &ok_message())]).await;
    let adapter = anthropic_boxed(true, &stub.url);
    for level in levels {
        adapter.call("m", effort(level)).await.unwrap();
    }
    for (index, level) in levels.iter().enumerate() {
        let body = body_of(&stub, index);
        assert_eq!(
            body["thinking"],
            json!({"type": "adaptive", "display": "summarized"}),
            "{level}"
        );
        assert_eq!(body["output_config"], json!({"effort": level}), "{level}");
    }
}

#[tokio::test]
async fn anthropic_without_reasoning_sends_no_thinking_or_output_config() {
    for official in [false, true] {
        let stub = serve(vec![response(200, &[], &ok_message())]).await;
        anthropic_boxed(official, &stub.url)
            .call("m", request())
            .await
            .unwrap();
        let body = body_of(&stub, 0);
        for key in ["thinking", "output_config", "reasoning_effort", "reasoning"] {
            assert!(
                body.get(key).is_none(),
                "official={official}: {key} must be absent"
            );
        }
    }
}

#[tokio::test]
async fn anthropic_stream_requests_carry_the_reasoning_mapping_too() {
    let stub = serve(vec![anthropic_minimal_stream()]).await;
    drain(
        anthropic_boxed(false, &stub.url)
            .stream("m", effort("medium"))
            .await
            .unwrap(),
    )
    .await;
    let stub_official = serve(vec![anthropic_minimal_stream()]).await;
    drain(
        anthropic_boxed(true, &stub_official.url)
            .stream("m", effort("max"))
            .await
            .unwrap(),
    )
    .await;
    let generic = body_of(&stub, 0);
    assert_eq!(generic["stream"], true);
    assert_eq!(
        generic["thinking"],
        json!({"type": "enabled", "budget_tokens": 8192})
    );
    assert!(generic.get("output_config").is_none());
    let official = body_of(&stub_official, 0);
    assert_eq!(official["stream"], true);
    assert_eq!(
        official["thinking"],
        json!({"type": "adaptive", "display": "summarized"})
    );
    assert_eq!(official["output_config"], json!({"effort": "max"}));
}

// ---------------------------------------------------------------------------------------------
// Levels without a mapping are refused locally
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn generic_anthropic_refuses_levels_it_cannot_map_without_a_request() {
    let stub = serve(vec![response(200, &[], &ok_message())]).await;
    let adapter = anthropic_boxed(false, &stub.url);
    for level in ["none", "minimal", "xhigh", "max", "turbo"] {
        assert_refused_locally(adapter.as_ref(), &stub, effort(level), level).await;
    }
    assert_refused_locally(
        adapter.as_ref(),
        &stub,
        reasoning_request(ReasoningConfig::default(), 32_000),
        "no effort and no budget",
    )
    .await;
}

#[tokio::test]
async fn generic_anthropic_refuses_a_budget_that_leaves_no_room_for_the_answer() {
    let stub = serve(vec![response(200, &[], &ok_message())]).await;
    let adapter = anthropic_boxed(false, &stub.url);
    let budget = |budget: u32, max_tokens: u32| {
        reasoning_request(
            ReasoningConfig {
                effort: None,
                budget_tokens: Some(budget),
            },
            max_tokens,
        )
    };
    assert_refused_locally(
        adapter.as_ref(),
        &stub,
        budget(4096, 4096),
        "budget == max_tokens",
    )
    .await;
    assert_refused_locally(
        adapter.as_ref(),
        &stub,
        budget(5000, 4096),
        "budget > max_tokens",
    )
    .await;
    // The fixed budget of `high` (16384) does not fit in 8192 either.
    assert_refused_locally(
        adapter.as_ref(),
        &stub,
        reasoning_request(
            ReasoningConfig {
                effort: Some("high".into()),
                budget_tokens: None,
            },
            8192,
        ),
        "high effort budget >= max_tokens",
    )
    .await;
    // One token of room is enough: the same request below the limit goes through.
    adapter.call("m", budget(4095, 4096)).await.unwrap();
    assert_eq!(stub.hits(), 1);
    assert_eq!(
        body_of(&stub, 0)["thinking"],
        json!({"type": "enabled", "budget_tokens": 4095})
    );
}

#[tokio::test]
async fn official_anthropic_refuses_levels_without_an_effort_mapping() {
    let stub = serve(vec![response(200, &[], &ok_message())]).await;
    let adapter = anthropic_boxed(true, &stub.url);
    for level in ["none", "minimal", "turbo"] {
        assert_refused_locally(adapter.as_ref(), &stub, effort(level), level).await;
    }
    assert_refused_locally(
        adapter.as_ref(),
        &stub,
        reasoning_request(
            ReasoningConfig {
                effort: None,
                budget_tokens: Some(4000),
            },
            32_000,
        ),
        "only budget_tokens",
    )
    .await;
}

#[tokio::test]
async fn openai_refuses_a_reasoning_request_without_an_effort() {
    for official in [false, true] {
        let stub = serve(vec![response(200, &[], &ok_chat())]).await;
        let adapter = openai_boxed(official, &stub.url);
        assert_refused_locally(
            adapter.as_ref(),
            &stub,
            reasoning_request(ReasoningConfig::default(), 32_000),
            "default reasoning config",
        )
        .await;
        // `budget_tokens` alone is not an effort on this protocol either.
        assert_refused_locally(
            adapter.as_ref(),
            &stub,
            reasoning_request(
                ReasoningConfig {
                    effort: None,
                    budget_tokens: Some(2000),
                },
                32_000,
            ),
            "budget only",
        )
        .await;
    }
}

// ---------------------------------------------------------------------------------------------
// Reading reasoning: synchronous
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn openai_reads_reasoning_content_and_keeps_it_out_of_content() {
    let response = openai_call(json!({"role": "assistant", "content": "final answer",
        "reasoning_content": "step one, step two"}))
    .await;
    assert_eq!(response.reasoning.as_deref(), Some("step one, step two"));
    assert_eq!(response.content, "final answer");
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    // Reasoning tokens are part of the completion tokens; the numbers are carried through.
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 50);
    assert_eq!(response.usage.total_tokens, 60);
    assert!(!response.usage_unknown);
}

#[tokio::test]
async fn openai_reads_the_reasoning_field_variant() {
    let response = openai_call(json!({"role": "assistant", "content": "final answer",
        "reasoning": "thinking via the reasoning field"}))
    .await;
    assert_eq!(
        response.reasoning.as_deref(),
        Some("thinking via the reasoning field")
    );
    assert_eq!(response.content, "final answer");
}

#[tokio::test]
async fn openai_reasoning_details_show_text_and_summary_but_never_encrypted_parts() {
    let response = openai_call(json!({"role": "assistant", "content": "final answer",
    "reasoning_details": [
        {"type": "reasoning.text", "text": "first "},
        {"type": "reasoning.summary", "summary": "second"},
        {"type": "reasoning.encrypted", "data": "ENCRYPTEDBLOB"}
    ]}))
    .await;
    assert_eq!(response.reasoning.as_deref(), Some("first second"));
    assert_eq!(response.content, "final answer");
    assert!(!response.reasoning.unwrap().contains("ENCRYPTEDBLOB"));
}

#[tokio::test]
async fn openai_without_reasoning_fields_has_no_reasoning() {
    let messages = [
        json!({"role": "assistant", "content": "plain"}),
        json!({"role": "assistant", "content": "plain", "reasoning_content": null}),
        json!({"role": "assistant", "content": "plain", "reasoning_content": ""}),
        json!({"role": "assistant", "content": "plain",
            "reasoning_details": [{"type": "reasoning.encrypted", "data": "ENCRYPTEDBLOB"}]}),
    ];
    for message in messages {
        let label = message.to_string();
        let response = openai_call(message).await;
        assert_eq!(response.reasoning, None, "{label}");
        assert_eq!(response.content, "plain", "{label}");
    }
}

#[tokio::test]
async fn anthropic_thinking_block_becomes_reasoning_and_text_blocks_become_content() {
    for official in [false, true] {
        let response = anthropic_call(
            official,
            json!([
                {"type": "thinking", "thinking": "plan A", "signature": "SIGBLOB"},
                {"type": "text", "text": "the "},
                {"type": "text", "text": "answer"}
            ]),
        )
        .await;
        assert_eq!(
            response.reasoning.as_deref(),
            Some("plan A"),
            "official={official}"
        );
        assert_eq!(response.content, "the answer");
        assert!(!response.reasoning.unwrap().contains("SIGBLOB"));
        // `output_tokens` already includes the thinking tokens.
        assert_eq!(response.usage.prompt_tokens, 3);
        assert_eq!(response.usage.completion_tokens, 40);
        assert_eq!(response.usage.total_tokens, 43);
        assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    }
}

#[tokio::test]
async fn anthropic_redacted_thinking_gives_no_reasoning_and_keeps_the_content() {
    for official in [false, true] {
        let response = anthropic_call(
            official,
            json!([
                {"type": "redacted_thinking", "data": "REDACTEDBLOB"},
                {"type": "text", "text": "the answer"}
            ]),
        )
        .await;
        assert_eq!(response.reasoning, None, "official={official}");
        assert_eq!(response.content, "the answer");
    }
}

#[tokio::test]
async fn anthropic_omitted_or_empty_thinking_text_gives_no_reasoning() {
    let variants = [
        json!({"type": "thinking", "thinking": "", "signature": "SIG"}),
        json!({"type": "thinking", "signature": "SIG"}),
    ];
    for block in variants {
        let label = block.to_string();
        let response =
            anthropic_call(true, json!([block, {"type": "text", "text": "the answer"}])).await;
        assert_eq!(response.reasoning, None, "{label}");
        assert_eq!(response.content, "the answer", "{label}");
    }
}

// ---------------------------------------------------------------------------------------------
// Reading reasoning: streaming
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn openai_stream_separates_reasoning_from_content_without_empty_chunks() {
    let stub = serve(vec![sse_script(&[
        // Role-only opener and empty fields produce nothing.
        openai_event(json!({"role": "assistant", "content": ""})),
        openai_event(json!({"reasoning_content": "Let me "})),
        openai_event(json!({"reasoning_content": "think. "})),
        openai_event(json!({"content": null, "reasoning_content": ""})),
        openai_event(json!({"reasoning": "More. "})),
        // Both in one delta: ONE chunk with both.
        openai_event(json!({"content": "The ", "reasoning_content": "Wrap. "})),
        openai_event(json!({"content": "answer"})),
        finish("stop"),
        usage(10, 50),
        DONE.into(),
    ])])
    .await;
    let items = drain(
        openai_boxed(false, &stub.url)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    let (chunks, terminal) = body_chunks(&items);
    assert_eq!(
        chunks,
        vec![
            pair("", Some("Let me ")),
            pair("", Some("think. ")),
            pair("", Some("More. ")),
            pair("The ", Some("Wrap. ")),
            pair("answer", None),
        ]
    );
    assert_eq!(text_of(&items), "The answer");
    assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
    let usage = terminal.usage.expect("usage on the terminal chunk");
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (10, 50, 60)
    );
}

#[tokio::test]
async fn openai_stream_reasoning_details_show_text_only() {
    let stub = serve(vec![sse_script(&[
        openai_event(json!({"reasoning_details": [
            {"type": "reasoning.text", "text": "detail one"},
            {"type": "reasoning.encrypted", "data": "ENCRYPTEDBLOB"}
        ]})),
        // A delta that only carries an encrypted part has nothing to show.
        openai_event(json!({"reasoning_details": [
            {"type": "reasoning.encrypted", "data": "ENCRYPTEDBLOB"}
        ]})),
        openai_event(json!({"content": "done"})),
        finish("stop"),
        DONE.into(),
    ])])
    .await;
    let items = drain(
        openai_boxed(true, &stub.url)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    let (chunks, terminal) = body_chunks(&items);
    assert_eq!(
        chunks,
        vec![pair("", Some("detail one")), pair("done", None)]
    );
    assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
    assert!(terminal.usage.is_none(), "no usage was sent");
}

#[tokio::test]
async fn openai_stream_split_packets_keep_order_and_lose_nothing() {
    let reasoning_one = openai_event(json!({"reasoning_content": "alpha "}));
    let content_one = openai_event(json!({"content": "beta "}));
    let reasoning_two = openai_event(json!({"reasoning": "gamma "}));
    let both = openai_event(json!({"content": "delta", "reasoning_content": "epsilon"}));
    let stub = serve(vec![script(vec![
        // Cut in the middle of the JSON.
        split_at_byte(&reasoning_one, reasoning_one.len() / 2),
        // Cut inside the `data:` prefix.
        split_at_byte(&content_one, 3),
        // Cut between the two newlines that end the event.
        split_at_byte(&reasoning_two, reasoning_two.len() - 1),
        split_at_byte(&both, both.len() / 3),
        whole(&finish("stop")),
        whole(&usage(10, 50)),
        whole(DONE),
    ])])
    .await;
    let items = drain(
        openai_boxed(false, &stub.url)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    let (chunks, terminal) = body_chunks(&items);
    assert_eq!(
        chunks,
        vec![
            pair("", Some("alpha ")),
            pair("beta ", None),
            pair("", Some("gamma ")),
            pair("delta", Some("epsilon")),
        ]
    );
    assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
    assert_eq!(terminal.usage.unwrap().completion_tokens, 50);
}

#[tokio::test]
async fn openai_stream_reasoning_split_inside_a_multibyte_character() {
    let one = openai_event(json!({"reasoning_content": "思考中"}));
    // One byte into the first three-byte character of the reasoning text.
    let at = one.find('思').unwrap() + 1;
    let stub = serve(vec![script(vec![
        split_at_byte(&one, at),
        whole(&openai_event(json!({"content": "完成"}))),
        whole(&finish("stop")),
        whole(DONE),
    ])])
    .await;
    let items = drain(
        openai_boxed(false, &stub.url)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    let (chunks, _) = body_chunks(&items);
    assert_eq!(chunks, vec![pair("", Some("思考中")), pair("完成", None)]);
}

#[tokio::test]
async fn anthropic_stream_maps_thinking_to_reasoning_and_drops_signatures() {
    for official in [false, true] {
        let stub = serve(vec![sse_script(&[
            message_start(3),
            block_start("thinking"),
            ping(),
            thinking_delta("Plan: "),
            thinking_delta("step."),
            signature_delta("SIGBLOB"),
            block_stop(),
            block_start("text"),
            text_delta("The "),
            text_delta("answer"),
            block_stop(),
            message_delta("end_turn", 40),
            message_stop(),
        ])])
        .await;
        let items = drain(
            anthropic_boxed(official, &stub.url)
                .stream("m", request())
                .await
                .unwrap(),
        )
        .await;
        let (chunks, terminal) = body_chunks(&items);
        assert_eq!(
            chunks,
            vec![
                pair("", Some("Plan: ")),
                pair("", Some("step.")),
                pair("The ", None),
                pair("answer", None),
            ],
            "official={official}"
        );
        assert_eq!(text_of(&items), "The answer");
        assert!(!format!("{items:?}").contains("SIGBLOB"));
        assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
        let usage = terminal.usage.expect("usage on the terminal chunk");
        // Thinking tokens are inside `output_tokens`; nothing is added or subtracted.
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (3, 40, 43)
        );
    }
}

#[tokio::test]
async fn anthropic_stream_split_packets_keep_order_and_lose_nothing() {
    let think_one = thinking_delta("first thought. ");
    let think_two = thinking_delta("second thought.");
    let answer = text_delta("the answer");
    let signature = signature_delta("SIGBLOB");
    let stub = serve(vec![script(vec![
        whole(&message_start(3)),
        // Cut between the `event:` line and the `data:` line.
        split_at_byte(&think_one, think_one.find("data:").unwrap()),
        // Cut in the middle of the JSON.
        split_at_byte(&think_two, think_two.len() / 2),
        split_at_byte(&signature, signature.len() / 2),
        // Cut inside the event name.
        split_at_byte(&answer, 8),
        whole(&message_delta("end_turn", 40)),
        // Cut between the two newlines that end the last event.
        split_at_byte(&message_stop(), message_stop().len() - 1),
    ])])
    .await;
    let items = drain(
        anthropic_boxed(false, &stub.url)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    let (chunks, terminal) = body_chunks(&items);
    assert_eq!(
        chunks,
        vec![
            pair("", Some("first thought. ")),
            pair("", Some("second thought.")),
            pair("the answer", None),
        ]
    );
    assert!(!format!("{items:?}").contains("SIGBLOB"));
    assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
    assert_eq!(terminal.usage.unwrap().total_tokens, 43);
}

#[tokio::test]
async fn anthropic_stream_reasoning_split_inside_a_multibyte_character() {
    let think = thinking_delta("思考中");
    let at = think.find('思').unwrap() + 2;
    let stub = serve(vec![script(vec![
        whole(&message_start(1)),
        split_at_byte(&think, at),
        whole(&text_delta("完成")),
        whole(&message_delta("end_turn", 4)),
        whole(&message_stop()),
    ])])
    .await;
    let items = drain(
        anthropic_boxed(true, &stub.url)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    let (chunks, _) = body_chunks(&items);
    assert_eq!(chunks, vec![pair("", Some("思考中")), pair("完成", None)]);
}
