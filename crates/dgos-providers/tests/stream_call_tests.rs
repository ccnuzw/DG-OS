//! AC06 (stream succeeds), AC20 (error mid-stream) and AC21 (SSE decoding at adapter level).
//! A scripted local server stands in for the upstream; no real provider is called.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::DgosError;
use serde_json::Value;

#[tokio::test]
async fn stream_call_delivers_ordered_deltas_then_one_terminal_chunk_with_usage() {
    let stub = serve(vec![sse_script(&[
        delta("Hel"),
        delta("lo"),
        finish("stop"),
        usage(3, 5),
        DONE.into(),
    ])])
    .await;
    let stream = adapter(&stub.url, 3, 1)
        .stream("m", request())
        .await
        .unwrap();
    let items = drain(stream).await;
    assert_eq!(text_of(&items), "Hello");
    let chunks: Vec<_> = items.iter().map(|item| item.as_ref().unwrap()).collect();
    assert_eq!(chunks.len(), 3, "two deltas and one terminal chunk");
    let last = chunks.last().unwrap();
    assert_eq!(last.finish_reason.as_deref(), Some("stop"));
    let usage = last.usage.as_ref().unwrap();
    assert_eq!((usage.prompt_tokens, usage.completion_tokens), (3, 5));
    assert!(chunks[..2]
        .iter()
        .all(|c| c.finish_reason.is_none() && c.usage.is_none()));

    let request: Value = serde_json::from_str(&stub.requests()[0].body).unwrap();
    assert_eq!(request["stream"], true);
    assert_eq!(request["stream_options"]["include_usage"], true);
    assert!(stub.requests()[0]
        .head
        .to_lowercase()
        .contains(&format!("authorization: bearer {}", API_KEY)));
}

#[tokio::test]
async fn stream_call_reports_no_usage_as_none_rather_than_inventing_zeros() {
    let stub = serve(vec![sse_script(&[
        delta("hi"),
        finish("length"),
        DONE.into(),
    ])])
    .await;
    let items = drain(
        adapter(&stub.url, 0, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    let last = items.last().unwrap().as_ref().unwrap();
    assert_eq!(last.finish_reason.as_deref(), Some("length"));
    assert!(last.usage.is_none());
}

#[tokio::test]
async fn stream_call_without_a_done_marker_completes_when_a_finish_reason_was_seen() {
    let stub = serve(vec![sse_script(&[
        delta("hi"),
        finish("stop"),
        usage(1, 1),
    ])])
    .await;
    let items = drain(
        adapter(&stub.url, 0, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert!(items.iter().all(|item| item.is_ok()));
    assert_eq!(text_of(&items), "hi");
    assert_eq!(
        items
            .last()
            .unwrap()
            .as_ref()
            .unwrap()
            .finish_reason
            .as_deref(),
        Some("stop")
    );
}

#[tokio::test]
async fn stream_call_fails_before_streaming_with_the_normalised_error() {
    let stub = serve(vec![response(401, &[], r#"{"error":{"message":"nope"}}"#)]).await;
    let error = adapter(&stub.url, 3, 1)
        .stream("m", request())
        .await
        .err()
        .unwrap();
    assert!(matches!(error, DgosError::InvalidApiKey));
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn stream_call_html_instead_of_events_is_an_interruption_not_empty_success() {
    let stub = serve(vec![response(
        200,
        &[("content-type", "text/html")],
        "<html>hi</html>",
    )])
    .await;
    let items = drain(
        adapter(&stub.url, 0, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], Err(DgosError::StreamInterrupted)));
}

#[tokio::test]
async fn sse_decoding_handles_split_packets_comments_and_empty_choice_usage_blocks() {
    let first = delta("Hel");
    let second = delta("lo ");
    let third = delta("world");
    let (a, b) = first.split_at(18);
    let (c, d) = third.split_at(30);
    let stub = serve(vec![vec![
        sse_head(),
        // One event cut in two, then a comment and a whole second event in the same packet.
        text(a),
        sleep(30),
        text(&format!("{b}: keep-alive\n\n{second}")),
        sleep(30),
        // Third event split again, followed by the finish and an empty-choices usage block.
        text(c),
        sleep(30),
        text(&format!("{d}{}{}{DONE}", finish("stop"), usage(7, 9))),
    ]])
    .await;
    let items = drain(
        adapter(&stub.url, 0, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert!(items.iter().all(|item| item.is_ok()), "{items:?}");
    assert_eq!(text_of(&items), "Hello world");
    let deltas: Vec<_> = items
        .iter()
        .map(|item| item.as_ref().unwrap())
        .filter(|chunk| !chunk.delta.is_empty())
        .collect();
    assert_eq!(
        deltas.len(),
        3,
        "no empty chunks from comments or the usage block"
    );
    let usage = items
        .last()
        .unwrap()
        .as_ref()
        .unwrap()
        .usage
        .as_ref()
        .unwrap();
    assert_eq!((usage.prompt_tokens, usage.completion_tokens), (7, 9));
}

#[tokio::test]
async fn sse_decoding_keeps_a_multibyte_character_split_between_packets() {
    let event = delta("你好");
    let bytes = event.as_bytes();
    // Cut inside the first three-byte character.
    let cut = event.find('你').unwrap() + 1;
    let stub = serve(vec![vec![
        sse_head(),
        Step::Send(bytes[..cut].to_vec()),
        sleep(30),
        Step::Send(bytes[cut..].to_vec()),
        text(&format!("{}{DONE}", finish("stop"))),
    ]])
    .await;
    let items = drain(
        adapter(&stub.url, 0, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "你好");
}

#[tokio::test]
async fn sse_decoding_ends_on_an_unparseable_event_without_dropping_earlier_text() {
    let stub = serve(vec![sse_script(&[
        delta("kept"),
        "data: {this is not json}\n\n".into(),
        delta("never delivered"),
        DONE.into(),
    ])])
    .await;
    let items = drain(
        adapter(&stub.url, 3, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "kept");
    assert!(matches!(
        items.last(),
        Some(Err(DgosError::StreamInterrupted))
    ));
    assert_eq!(items.iter().filter(|item| item.is_err()).count(), 1);
}

#[tokio::test]
async fn mid_stream_error_events_end_the_stream_once_with_stream_interrupted() {
    let error_events = [
        "event: error\ndata: {\"type\":\"overloaded_error\"}\n\n".to_string(),
        "data: {\"error\":{\"message\":\"upstream failed\",\"type\":\"server_error\"}}\n\n".into(),
    ];
    for error_event in error_events {
        let stub = serve(vec![sse_script(&[
            delta("par"),
            delta("tial"),
            error_event,
            delta("ignored"),
        ])])
        .await;
        let items = drain(
            adapter(&stub.url, 3, 1)
                .stream("m", request())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(text_of(&items), "partial");
        assert_eq!(items.iter().filter(|item| item.is_err()).count(), 1);
        let error = items.last().unwrap().as_ref().unwrap_err();
        assert_eq!(error.error_code(), 1001007);
        // Not retried and not replayed.
        assert_eq!(stub.hits(), 1);
        assert!(!format!("{error:?}").contains("upstream failed"));
    }
}

#[tokio::test]
async fn mid_stream_disconnect_keeps_received_text_and_reports_one_interruption() {
    let cases: [(&str, Vec<String>); 3] = [
        ("after whole events", vec![delta("a"), delta("b")]),
        (
            "inside an event",
            vec![delta("a"), delta("b"), "data: {\"choi".into()],
        ),
        (
            "after a finish reason but inside the next event",
            vec![delta("a"), delta("b"), finish("stop"), "data: [DO".into()],
        ),
    ];
    for (label, events) in cases {
        let stub = serve(vec![sse_script(&events)]).await;
        let items = drain(
            adapter(&stub.url, 3, 1)
                .stream("m", request())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(text_of(&items), "ab", "{label}");
        assert_eq!(
            items.iter().filter(|item| item.is_err()).count(),
            1,
            "{label}"
        );
        assert!(
            matches!(items.last(), Some(Err(DgosError::StreamInterrupted))),
            "{label}"
        );
        assert_eq!(stub.hits(), 1, "{label}: no retry, no replay");
    }
}

#[tokio::test]
async fn mid_stream_event_over_the_size_limit_is_an_interruption() {
    let options = dgos_providers::AdapterOptions {
        max_event_bytes: 256,
        ..Default::default()
    };
    let huge = delta(&"x".repeat(1000));
    let stub = serve(vec![sse_script(&[delta("ok"), huge, DONE.into()])]).await;
    let items = drain(
        adapter_with(&stub.url, 0, 1, options)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "ok");
    assert!(matches!(
        items.last(),
        Some(Err(DgosError::StreamInterrupted))
    ));
}

// ---------------------------------------------------------------------------------------------
// Anthropic twins of the `mid_stream_*` tests (AC20). The stream ends with exactly one
// `stream_interrupted`, already received text is kept, and the upstream is asked exactly once even
// when the retry count is 3.
// ---------------------------------------------------------------------------------------------

/// Distinctive text in upstream payloads; it must never reach an error message.
const ANTHROPIC_MARKER: &str = "UPSTREAM-MARKER-9f31";

fn anthropic_block_stop() -> String {
    event(
        "content_block_stop",
        serde_json::json!({"type": "content_block_stop", "index": 0}),
    )
}

/// Plays one Anthropic stream with retry_count 3 and checks the AC20 invariants shared by every
/// case: kept text, one trailing `stream_interrupted`, no terminal chunk, one upstream request.
async fn assert_anthropic_interrupted_once(
    label: &str,
    max_event_bytes: Option<usize>,
    script: Vec<Step>,
    expected_text: &str,
) {
    let stub = serve(vec![script]).await;
    let options = dgos_providers::AdapterOptions {
        max_event_bytes: max_event_bytes
            .unwrap_or(dgos_providers::AdapterOptions::default().max_event_bytes),
        ..Default::default()
    };
    let items = drain(
        anthropic_with(&stub.url, 3, 1, options)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), expected_text, "{label}");
    assert_eq!(
        items.iter().filter(|item| item.is_err()).count(),
        1,
        "{label}: {items:?}"
    );
    let error = items.last().unwrap().as_ref().unwrap_err();
    assert_eq!(error.error_code(), 1001007, "{label}");
    assert!(
        items
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .all(|chunk| chunk.finish_reason.is_none() && chunk.usage.is_none()),
        "{label}: no terminal chunk on an interrupted stream"
    );
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(API_KEY), "{label}: {shown}");
    assert!(!shown.contains(ANTHROPIC_MARKER), "{label}: {shown}");
    assert_eq!(stub.hits(), 1, "{label}: no retry, no replay");
}

#[tokio::test]
async fn mid_stream_anthropic_error_event_after_partial_output_is_one_interruption() {
    // The documented shape: `{"type":"error","error":{"type":"overloaded_error",...}}`.
    let error_event = event(
        "error",
        serde_json::json!({"type": "error",
            "error": {"type": "overloaded_error", "message": ANTHROPIC_MARKER}}),
    );
    assert_anthropic_interrupted_once(
        "documented error event",
        None,
        sse_script(&[
            message_start(3),
            text_delta("par"),
            text_delta("tial"),
            error_event,
            text_delta("ignored"),
            message_delta("end_turn", 2),
            message_stop(),
        ]),
        "partial",
    )
    .await;
}

/// The SSE event name is `error`, so the stream is interrupted whatever the payload looks like
/// (design pseudo-code: `error` -> stream_interrupted, B01-62). The payload below is the one
/// the existing OpenAI-side twin uses: a bare error kind with no outer `type: error`.
#[tokio::test]
async fn mid_stream_anthropic_error_event_with_a_bare_kind_payload_is_an_interruption() {
    let payload = format!("{{\"type\":\"overloaded_error\",\"message\":\"{ANTHROPIC_MARKER}\"}}");
    assert_anthropic_interrupted_once(
        "bare kind payload",
        None,
        sse_script(&[
            message_start(3),
            text_delta("par"),
            text_delta("tial"),
            format!("event: error\ndata: {payload}\n\n"),
            text_delta("ignored"),
            message_delta("end_turn", 2),
            message_stop(),
        ]),
        "partial",
    )
    .await;
}

#[tokio::test]
async fn mid_stream_anthropic_error_event_with_only_an_error_object_is_an_interruption() {
    let payload = format!(
        "{{\"error\":{{\"type\":\"overloaded_error\",\"message\":\"{ANTHROPIC_MARKER}\"}}}}"
    );
    assert_anthropic_interrupted_once(
        "error object without outer type",
        None,
        sse_script(&[
            message_start(3),
            text_delta("par"),
            text_delta("tial"),
            format!("event: error\ndata: {payload}\n\n"),
            text_delta("ignored"),
            message_delta("end_turn", 2),
            message_stop(),
        ]),
        "partial",
    )
    .await;
}

#[tokio::test]
async fn mid_stream_anthropic_disconnect_keeps_received_text_and_reports_one_interruption() {
    let cases: [(&str, Vec<String>); 4] = [
        (
            "between events",
            vec![message_start(3), text_delta("a"), text_delta("b")],
        ),
        (
            "inside the data JSON of an event",
            vec![
                message_start(3),
                text_delta("a"),
                text_delta("b"),
                "event: content_block_delta\ndata: {\"type\":\"content_block_".into(),
            ],
        ),
        (
            "between the event and data lines",
            vec![
                message_start(3),
                text_delta("a"),
                text_delta("b"),
                "event: content_block_delta\n".into(),
            ],
        ),
        (
            "after the stop reason but inside message_stop",
            vec![
                message_start(3),
                text_delta("a"),
                text_delta("b"),
                message_delta("end_turn", 2),
                anthropic_block_stop(),
                "event: message_stop\ndata: {\"type\":\"mess".into(),
            ],
        ),
    ];
    for (label, events) in cases {
        assert_anthropic_interrupted_once(label, None, sse_script(&events), "ab").await;
    }
}

#[tokio::test]
async fn mid_stream_anthropic_event_over_the_size_limit_is_an_interruption() {
    let huge = text_delta(&"x".repeat(2000));
    assert_anthropic_interrupted_once(
        "oversized event",
        Some(512),
        sse_script(&[
            message_start(3),
            text_delta("ok"),
            huge,
            message_delta("end_turn", 2),
            message_stop(),
        ]),
        "ok",
    )
    .await;
}

#[tokio::test]
async fn mid_stream_anthropic_non_json_data_is_an_interruption_not_a_dropped_event() {
    let bad_events = [
        "event: content_block_delta\ndata: {this is not json}\n\n".to_string(),
        "event: content_block_delta\ndata: <html>gateway timeout</html>\n\n".to_string(),
        format!("event: message_delta\ndata: {ANTHROPIC_MARKER}\n\n"),
    ];
    for bad in bad_events {
        assert_anthropic_interrupted_once(
            &bad,
            None,
            sse_script(&[
                message_start(3),
                text_delta("kept"),
                bad.clone(),
                text_delta("never delivered"),
                message_delta("end_turn", 2),
                message_stop(),
            ]),
            "kept",
        )
        .await;
    }
}
