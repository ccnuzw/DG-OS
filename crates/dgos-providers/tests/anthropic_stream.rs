//! AC19: `AnthropicAdapter::stream` turns Messages stream events into ordered chunks with one
//! terminal chunk (finish reason and usage). A scripted local server controls packetisation and
//! pacing; no real provider is called.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::{ChatChunk, ChatMessage, ChatRequest, DgosError, DgosResult};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Distinctive text placed in upstream error bodies; it must never reach an error message.
const MARKER: &str = "UPSTREAM-MARKER-4d2b";

/// Runs one scripted stream with the generic profile and no retries.
async fn play(script: Vec<Step>) -> (Vec<DgosResult<ChatChunk>>, usize) {
    let stub = serve(vec![script]).await;
    let items = drain(
        anthropic(&stub.url, 0, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    (items, stub.hits())
}

fn ok_chunks(items: &[DgosResult<ChatChunk>]) -> Vec<&ChatChunk> {
    items
        .iter()
        .map(|item| item.as_ref().expect("no error expected"))
        .collect()
}

/// Comparable form of a result list (`ChatChunk` has no `PartialEq`).
fn shape(items: &[DgosResult<ChatChunk>]) -> Vec<Value> {
    items
        .iter()
        .map(|item| match item {
            Ok(chunk) => serde_json::to_value(chunk).unwrap(),
            Err(error) => json!({"error": error.error_code()}),
        })
        .collect()
}

fn block_start() -> String {
    event(
        "content_block_start",
        json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
    )
}

fn block_stop() -> String {
    event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 0}),
    )
}

/// The head, then each piece as its own write, optionally spaced so the client sees separate
/// network reads.
fn paced(pieces: Vec<Vec<u8>>, gap_ms: u64) -> Vec<Step> {
    let mut script = vec![sse_head()];
    for piece in pieces {
        script.push(Step::Send(piece));
        if gap_ms > 0 {
            script.push(sleep(gap_ms));
        }
    }
    script
}

fn find(haystack: &[u8], needle: &str) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle.as_bytes())
        .unwrap_or_else(|| panic!("{needle:?} not in the wire text"))
}

fn error_body(kind: &str) -> String {
    json!({"type": "error", "error": {"type": kind, "message": MARKER}}).to_string()
}

fn assert_no_leak(error: &DgosError) {
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(API_KEY), "{shown}");
    assert!(!shown.contains(MARKER), "{shown}");
}

#[tokio::test]
async fn full_exchange_yields_ordered_text_then_one_terminal_chunk_with_the_last_usage() {
    let events = [
        message_start(25),
        block_start(),
        ping(),
        text_delta("Hel"),
        text_delta("lo "),
        text_delta("你好"),
        block_stop(),
        // `message_delta.usage` is cumulative: the second value replaces the first.
        message_delta("end_turn", 5),
        message_delta("end_turn", 12),
        message_stop(),
    ];
    let (items, hits) = play(sse_script(&events)).await;
    assert_eq!(hits, 1);
    let chunks = ok_chunks(&items);
    assert_eq!(chunks.len(), 4, "three text chunks and one terminal chunk");

    let deltas: Vec<&str> = chunks[..3]
        .iter()
        .map(|chunk| chunk.delta.as_str())
        .collect();
    assert_eq!(deltas, ["Hel", "lo ", "你好"]);
    for chunk in &chunks[..3] {
        assert!(chunk.finish_reason.is_none(), "{chunk:?}");
        assert!(chunk.usage.is_none(), "{chunk:?}");
        assert!(chunk.reasoning_delta.is_none(), "{chunk:?}");
    }

    let terminal = chunks[3];
    assert_eq!(terminal.delta, "");
    assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
    let usage = terminal
        .usage
        .as_ref()
        .expect("usage on the terminal chunk");
    assert_eq!(usage.prompt_tokens, 25, "input from message_start");
    assert_eq!(
        usage.completion_tokens, 12,
        "the last cumulative output value wins"
    );
    assert_eq!(usage.total_tokens, 37);
}

#[tokio::test]
async fn stop_reasons_map_to_the_host_finish_reasons() {
    let cases = [
        ("end_turn", "stop"),
        ("stop_sequence", "stop"),
        ("max_tokens", "length"),
        ("tool_use", "tool_calls"),
        ("refusal", "content_filter"),
    ];
    for (reason, expected) in cases {
        let events = [
            message_start(2),
            text_delta("x"),
            message_delta(reason, 3),
            message_stop(),
        ];
        let (items, _) = play(sse_script(&events)).await;
        let chunks = ok_chunks(&items);
        assert_eq!(chunks.len(), 2, "{reason}");
        assert_eq!(chunks[0].delta, "x", "{reason}");
        assert_eq!(
            chunks[1].finish_reason.as_deref(),
            Some(expected),
            "{reason}"
        );
        assert_eq!(
            chunks[1].usage.as_ref().unwrap().total_tokens,
            5,
            "{reason}"
        );
    }
}

#[tokio::test]
async fn stream_request_sets_stream_true_lifts_system_and_defaults_max_tokens() {
    let script = || {
        sse_script(&[
            message_start(1),
            text_delta("x"),
            message_delta("end_turn", 1),
            message_stop(),
        ])
    };
    let stub = serve(vec![script()]).await;
    let chat = |max_tokens| ChatRequest {
        messages: vec![
            ChatMessage {
                role: "system".into(),
                content: "be brief".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "hello".into(),
            },
        ],
        max_tokens,
        ..Default::default()
    };
    let items = drain(
        anthropic(&stub.url, 0, 1)
            .stream("claude-x", chat(None))
            .await
            .unwrap(),
    )
    .await;
    assert!(items.iter().all(|item| item.is_ok()), "{items:?}");

    let requests = stub.requests();
    assert_eq!(
        requests[0].head.lines().next(),
        Some("POST /v1/messages HTTP/1.1")
    );
    let body: Value = serde_json::from_str(&requests[0].body).unwrap();
    assert_eq!(body["model"], "claude-x");
    assert_eq!(body["stream"], true);
    assert!(body.get("stream_options").is_none(), "{body}");
    assert_eq!(
        body["max_tokens"], 4096,
        "default when the request has none"
    );
    assert_eq!(body["system"], "be brief");
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [{"type": "text", "text": "hello"}]}])
    );
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["max_tokens", "messages", "model", "stream", "system"]
    );

    // An explicit value is passed through unchanged.
    let stub = serve(vec![script()]).await;
    drain(
        anthropic(&stub.url, 0, 1)
            .stream("claude-x", chat(Some(777)))
            .await
            .unwrap(),
    )
    .await;
    let body: Value = serde_json::from_str(&stub.requests()[0].body).unwrap();
    assert_eq!(body["max_tokens"], 777);
    assert_eq!(body["stream"], true);
}

#[tokio::test]
async fn split_packets_and_grouped_events_give_the_same_result_as_one_packet() {
    let events = [
        message_start(7),
        text_delta("你好"),
        text_delta("世界!"),
        ping(),
        text_delta("end"),
        message_delta("end_turn", 9),
        message_stop(),
    ];
    let wire = events.concat();
    let bytes = wire.as_bytes();

    // Cut inside the `data:` JSON, inside a field name, between the `event:` and `data:` lines,
    // inside two multi-byte characters, and between the two newlines that end an event.
    let mut cuts = vec![
        find(bytes, "\"usage\"") + 3,
        find(bytes, "event: ping") + 3,
        find(bytes, "event: content_block_delta\n") + "event: content_block_delta\n".len(),
        find(bytes, "你") + 1,
        find(bytes, "世") + 2,
        find(bytes, "event: message_stop") - 1,
    ];
    cuts.sort_unstable();
    cuts.dedup();
    assert!(!wire.is_char_boundary(find(bytes, "你") + 1));
    assert!(!wire.is_char_boundary(find(bytes, "世") + 2));

    let mut split = Vec::new();
    let mut from = 0;
    for cut in cuts {
        split.push(bytes[from..cut].to_vec());
        from = cut;
    }
    split.push(bytes[from..].to_vec());
    assert!(split.len() >= 7);

    let grouped: Vec<Vec<u8>> = events
        .chunks(3)
        .map(|group| group.concat().into_bytes())
        .collect();
    let single_bytes: Vec<Vec<u8>> = bytes.iter().map(|byte| vec![*byte]).collect();

    let (baseline, _) = play(paced(vec![bytes.to_vec()], 0)).await;
    assert_eq!(text_of(&baseline), "你好世界!end");
    let chunks = ok_chunks(&baseline);
    assert_eq!(chunks.len(), 4);
    assert_eq!(chunks[3].finish_reason.as_deref(), Some("stop"));
    let usage = chunks[3].usage.as_ref().unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (7, 9, 16)
    );
    let expected = shape(&baseline);

    for (label, script) in [
        ("cut at awkward offsets", paced(split, 15)),
        ("several events per packet", paced(grouped, 15)),
        ("one byte per write", paced(single_bytes, 0)),
    ] {
        let (items, hits) = play(script).await;
        assert_eq!(hits, 1, "{label}");
        assert_eq!(shape(&items), expected, "{label}");
    }
}

#[tokio::test]
async fn comments_unknown_events_and_unknown_delta_types_are_ignored_without_chunks() {
    let wire = [
        message_start(3),
        ": keepalive\n\n".to_string(),
        ": another comment\n".to_string(),
        event(
            "future_event",
            json!({"type": "future_event", "payload": {"text": "must not appear"}}),
        ),
        event(
            "content_block_start",
            json!({"type": "content_block_start", "index": 1,
                "content_block": {"type": "server_tool_use", "id": "srv_1", "name": "search"}}),
        ),
        text_delta("a"),
        event(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "signature_delta", "signature": "sig-must-not-appear"}}),
        ),
        event(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1,
                "delta": {"type": "input_json_delta", "partial_json": "{\"q\":\"nope\"}"}}),
        ),
        event(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "citations_delta", "citation": {"cited_text": "nope"}}}),
        ),
        event(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": ""}}),
        ),
        text_delta("b"),
        block_stop(),
        message_delta("end_turn", 4),
        message_stop(),
    ];
    let (items, _) = play(sse_script(&wire)).await;
    let chunks = ok_chunks(&items);
    assert_eq!(chunks.len(), 3, "{items:?}");
    assert_eq!(chunks[0].delta, "a");
    assert_eq!(chunks[1].delta, "b");
    assert_eq!(chunks[2].finish_reason.as_deref(), Some("stop"));
    for chunk in &chunks[..2] {
        assert!(chunk.reasoning_delta.is_none(), "{chunk:?}");
    }
    assert_eq!(chunks[2].usage.as_ref().unwrap().total_tokens, 7);
}

#[tokio::test]
async fn a_stream_without_any_usage_ends_with_usage_none() {
    let events = [
        event(
            "message_start",
            json!({"type": "message_start", "message": {"id": "m1", "type": "message",
                "role": "assistant", "content": []}}),
        ),
        text_delta("hi"),
        event(
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
        ),
        message_stop(),
    ];
    let (items, _) = play(sse_script(&events)).await;
    let chunks = ok_chunks(&items);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].delta, "hi");
    assert_eq!(chunks[1].finish_reason.as_deref(), Some("stop"));
    assert!(
        chunks[1].usage.is_none(),
        "no usage reported, none invented"
    );
}

#[tokio::test]
async fn ending_after_the_stop_reason_without_message_stop_still_completes() {
    let events = [
        message_start(4),
        text_delta("hi"),
        message_delta("end_turn", 6),
    ];
    let (items, hits) = play(sse_script(&events)).await;
    assert_eq!(hits, 1);
    let chunks = ok_chunks(&items);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].delta, "hi");
    assert_eq!(chunks[1].finish_reason.as_deref(), Some("stop"));
    let usage = chunks[1].usage.as_ref().unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (4, 6, 10)
    );
}

#[tokio::test]
async fn ending_before_any_stop_reason_is_exactly_one_interruption() {
    let cases: [(&str, Vec<String>, &str); 3] = [
        ("after text", vec![message_start(4), text_delta("hi")], "hi"),
        ("after message_start only", vec![message_start(4)], ""),
        (
            "after a block stop",
            vec![message_start(4), text_delta("hi"), block_stop()],
            "hi",
        ),
    ];
    for (label, events, text) in cases {
        let (items, hits) = play(sse_script(&events)).await;
        assert_eq!(hits, 1, "{label}");
        assert_eq!(text_of(&items), text, "{label}");
        assert_eq!(
            items.iter().filter(|item| item.is_err()).count(),
            1,
            "{label}"
        );
        let error = items.last().unwrap().as_ref().unwrap_err();
        assert_eq!(error.error_code(), 1001007, "{label}");
        assert!(
            items
                .iter()
                .filter_map(|item| item.as_ref().ok())
                .all(|chunk| chunk.finish_reason.is_none() && chunk.usage.is_none()),
            "{label}: no terminal chunk before an interruption"
        );
    }
}

#[tokio::test]
async fn unauthorized_before_the_stream_starts_is_an_error_not_a_chunk() {
    let stub = serve(vec![response(
        401,
        &[],
        &error_body("authentication_error"),
    )])
    .await;
    let adapter = anthropic(&stub.url, 3, 1);
    let started = Instant::now();
    let error = adapter
        .stream("m", request())
        .await
        .err()
        .expect("the call itself fails");
    assert_eq!(error.error_code(), 1001002);
    assert_eq!(stub.hits(), 1, "a bad key is not retried");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_no_leak(&error);
}

#[tokio::test]
async fn bad_request_before_the_stream_starts_is_rejected_without_a_retry() {
    let stub = serve(vec![response(
        400,
        &[],
        &error_body("invalid_request_error"),
    )])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .stream("m", request())
        .await
        .err()
        .expect("the call itself fails");
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);
    assert_no_leak(&error);
}

#[tokio::test]
async fn rate_limit_before_the_stream_starts_is_retried_once_then_streams() {
    let stub = serve(vec![
        response(
            429,
            &[("retry-after", "30")],
            &error_body("rate_limit_error"),
        ),
        sse_script(&[
            message_start(2),
            text_delta("ok"),
            message_delta("end_turn", 1),
            message_stop(),
        ]),
    ])
    .await;
    // max_delay is 40 ms here, so the 30 s the upstream asked for must be capped.
    let adapter = anthropic(&stub.url, 1, 10);
    let started = Instant::now();
    let items = drain(adapter.stream("m", request()).await.unwrap()).await;
    assert!(items.iter().all(|item| item.is_ok()), "{items:?}");
    assert_eq!(text_of(&items), "ok");
    assert_eq!(stub.hits(), 2, "one call and one retry");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let requests = stub.requests();
    assert_eq!(
        requests[0].body, requests[1].body,
        "the retry sends the same body"
    );
}

#[tokio::test]
async fn overloaded_before_the_stream_starts_reports_max_retries_after_a_failed_retry() {
    let stub = serve(vec![response(529, &[], &error_body("overloaded_error"))]).await;
    let error = anthropic(&stub.url, 1, 1)
        .stream("m", request())
        .await
        .err()
        .expect("the call itself fails");
    assert_eq!(error.error_code(), 1001008);
    assert_eq!(stub.hits(), 2);
    assert_no_leak(&error);
}
