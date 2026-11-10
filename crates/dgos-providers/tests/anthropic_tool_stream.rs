//! Genuine Messages SSE tool streams: no synchronous fixtures or external provider calls.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::{DgosResult, ToolChatRequest, ToolChatResponse, ToolDefinition, ToolFunction, ToolMessage, ToolStreamEvent};
use futures::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;

fn tool_request() -> ToolChatRequest {
    ToolChatRequest {
        messages: vec![ToolMessage {
            role: "user".into(), content: Some("Find the answer".into()), tool_call_id: None,
            tool_calls: vec![], reasoning: None, images: vec![],
        }],
        tools: vec![ToolDefinition {
            kind: "function".into(),
            function: ToolFunction {
                name: "lookup".into(), description: "Lookup".into(),
                parameters: json!({"type":"object","properties":{"query":{"type":"string"}}}),
            },
        }],
        temperature: None, reasoning: None, max_tokens: 1024,
    }
}

fn start(index: u64, block: Value) -> String {
    event("content_block_start", json!({"type":"content_block_start","index":index,"content_block":block}))
}

fn stop(index: u64) -> String {
    event("content_block_stop", json!({"type":"content_block_stop","index":index}))
}

fn piece(index: u64, delta: Value) -> String {
    event("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":delta}))
}

fn tool_start(index: u64, id: &str) -> String {
    start(index, json!({"type":"tool_use","id":id,"name":"lookup","input":{}}))
}

fn arguments(index: u64, fragment: &str) -> String {
    piece(index, json!({"type":"input_json_delta","partial_json":fragment}))
}

async fn play(events: Vec<String>) -> Vec<DgosResult<ToolStreamEvent>> {
    let stub = serve(vec![sse_script(&events)]).await;
    let items = anthropic(&stub.url, 3, 1).stream_tools("m", tool_request())
        .await.unwrap().collect::<Vec<_>>().await;
    assert_eq!(stub.hits(), 1, "tool streams never retry");
    let body: Value = serde_json::from_str(&stub.requests()[0].body).unwrap();
    assert_eq!(body["stream"], true);
    items
}

fn assert_failed(items: &[DgosResult<ToolStreamEvent>]) {
    assert!(items.last().is_some_and(Result::is_err), "must end with an error");
    assert_eq!(items.iter().filter(|item| item.is_err()).count(), 1);
    assert!(!items.iter().any(|item| matches!(item, Ok(ToolStreamEvent::Complete { .. }))),
        "an invalid stream must never expose a proposal");
}

fn complete(items: Vec<DgosResult<ToolStreamEvent>>) -> ToolChatResponse {
    let mut result = None;
    for item in items {
        if let ToolStreamEvent::Complete { response, usage_unknown } = item.unwrap() {
            assert!(result.is_none(), "one terminal response");
            assert!(!usage_unknown);
            result = Some(response);
        }
    }
    result.expect("validated terminal response")
}

#[tokio::test]
async fn indexed_tool_json_is_private_until_complete_message_and_usage() {
    let items = play(vec![
        message_start(7),
        start(0, json!({"type":"text","text":"Searching "})),
        piece(0, json!({"type":"text_delta","text":"now"})), stop(0),
        tool_start(1, "tool_1"), arguments(1, "{\"query\":"),
        arguments(1, "\"你好\"}"), stop(1),
        message_delta("tool_use", 9), message_stop(),
    ]).await;
    assert_eq!(items.len(), 3, "only text deltas and one complete proposal");
    assert!(matches!(&items[0], Ok(ToolStreamEvent::Delta(text)) if text == "Searching "));
    assert!(matches!(&items[1], Ok(ToolStreamEvent::Delta(text)) if text == "now"));
    let response = complete(items);
    assert_eq!(response.message.content.as_deref(), Some("Searching now"));
    assert_eq!(response.message.tool_calls.len(), 1);
    assert_eq!(response.message.tool_calls[0].id, "tool_1");
    assert_eq!(response.message.tool_calls[0].function.name, "lookup");
    let input: Value = serde_json::from_str(&response.message.tool_calls[0].function.arguments).unwrap();
    assert_eq!(input, json!({"query":"你好"}));
    assert_eq!(response.usage.total_tokens, 16);
}

#[tokio::test]
async fn malformed_partial_and_non_object_arguments_never_become_proposals() {
    for fragment in ["{\"query\":", "{broken}", "[]", "null", "{}{}", ""] {
        let items = play(vec![message_start(1), tool_start(0, "tool_1"), arguments(0, fragment),
            stop(0), message_delta("tool_use", 2), message_stop()]).await;
        assert_failed(&items);
    }
    assert_failed(&play(vec![message_start(1), tool_start(0, "tool_1"),
        "event: content_block_delta\ndata: {malformed}\n\n".into(),
        stop(0), message_delta("tool_use", 2), message_stop()]).await);
}

#[tokio::test]
async fn duplicate_reassigned_parallel_or_wrong_index_calls_fail_closed() {
    let mut cases = Vec::new();
    for id in ["tool_1", "tool_2"] {
        cases.push(vec![message_start(1), tool_start(0, "tool_1"), arguments(0, "{}"), stop(0),
            tool_start(1, id), arguments(1, "{}"), stop(1), message_delta("tool_use", 2), message_stop()]);
    }
    cases.push(vec![message_start(1), tool_start(0, "tool_1"), tool_start(0, "tool_2")]);
    cases.push(vec![message_start(1), tool_start(0, "tool_1"), arguments(1, "{}")]);
    cases.push(vec![message_start(1), tool_start(0, "tool_1"),
        piece(0, json!({"type":"input_json_delta","partial_json":"{}","id":"tool_2"}))]);
    cases.push(vec![message_start(1), tool_start(0, "tool_1"), arguments(0, "{}"), stop(0),
        tool_start(0, "tool_1")]);
    for events in cases { assert_failed(&play(events).await); }
}

#[tokio::test]
async fn missing_stops_and_incompatible_stop_reasons_never_complete() {
    let mut cases = vec![
        vec![message_start(1), tool_start(0, "tool_1"), arguments(0, "{}"), message_delta("tool_use", 2), message_stop()],
        vec![message_start(1), tool_start(0, "tool_1"), arguments(0, "{}"), stop(0), message_delta("tool_use", 2)],
        vec![message_start(1), tool_start(0, "tool_1"), arguments(0, "{}"), stop(0), message_stop()],
        vec![message_start(1), message_delta("tool_use", 2), message_stop()],
        vec![tool_start(0, "tool_1"), arguments(0, "{}"), stop(0), message_delta("tool_use", 2), message_stop()],
        vec![message_start(1), message_start(1), message_delta("end_turn", 2), message_stop()],
    ];
    for reason in ["end_turn", "stop_sequence", "max_tokens", "pause_turn", "refusal"] {
        cases.push(vec![message_start(1), tool_start(0, "tool_1"), arguments(0, "{}"), stop(0),
            message_delta(reason, 2), message_stop()]);
    }
    for events in cases { assert_failed(&play(events).await); }
}

#[tokio::test]
async fn thinking_signatures_redacted_blocks_and_opaque_fields_roundtrip_unchanged() {
    let events = vec![message_start(5),
        start(0, json!({"type":"thinking","thinking":"Plan ","signature":"","opaque":{"vendor":"kept"}})),
        piece(0, json!({"type":"thinking_delta","thinking":"carefully"})),
        piece(0, json!({"type":"signature_delta","signature":"signed-"})),
        piece(0, json!({"type":"signature_delta","signature":"bytes"})), stop(0),
        start(1, json!({"type":"redacted_thinking","data":"opaque-ciphertext","extra":"kept"})), stop(1),
        tool_start(2, "tool_1"), arguments(2, "{}"), stop(2), message_delta("tool_use", 6), message_stop()];
    let stub = serve(vec![sse_script(&events), sse_script(&[
        message_start(2), start(0, json!({"type":"text","text":"Done"})), stop(0),
        message_delta("end_turn", 1), message_stop(),
    ])]).await;
    let adapter = anthropic(&stub.url, 0, 1);
    let items = adapter.stream_tools("m", tool_request()).await.unwrap().collect::<Vec<_>>().await;
    assert!(matches!(&items[0], Ok(ToolStreamEvent::ReasoningDelta(text)) if text == "Plan "));
    assert!(matches!(&items[1], Ok(ToolStreamEvent::ReasoningDelta(text)) if text == "carefully"));
    assert_eq!(items.len(), 3, "signatures and redacted data are not display deltas");
    let response = complete(items);
    let expected = json!([
        {"type":"thinking","thinking":"Plan carefully","signature":"signed-bytes","opaque":{"vendor":"kept"}},
        {"type":"redacted_thinking","data":"opaque-ciphertext","extra":"kept"},
    ]);
    let reasoning = response.message.reasoning.as_ref().unwrap();
    assert_eq!(reasoning.text, "Plan carefully");
    assert_eq!(serde_json::to_value(&reasoning.blocks).unwrap(), expected);
    let mut next = tool_request();
    next.messages.push(response.message);
    next.messages.push(ToolMessage {
        role: "tool".into(), content: Some("Found".into()), tool_call_id: Some("tool_1".into()),
        tool_calls: vec![], reasoning: None, images: vec![],
    });
    complete(adapter.stream_tools("m", next).await.unwrap().collect::<Vec<_>>().await);
    let wire: Value = serde_json::from_str(&stub.requests()[1].body).unwrap();
    let blocks = wire["messages"][1]["content"].as_array().unwrap();
    assert_eq!(&blocks[..2], expected.as_array().unwrap().as_slice());
    assert_eq!(blocks[2]["type"], "tool_use");
}

#[tokio::test]
async fn missing_thinking_signature_or_delta_after_signature_is_rejected() {
    assert_failed(&play(vec![message_start(1), start(0, json!({"type":"thinking","thinking":"x"})),
        stop(0), message_delta("end_turn", 1), message_stop()]).await);
    assert_failed(&play(vec![message_start(1), start(0, json!({"type":"thinking","thinking":"x","signature":"s"})),
        piece(0, json!({"type":"thinking_delta","thinking":"late"})), stop(0),
        message_delta("end_turn", 1), message_stop()]).await);
}

#[tokio::test]
async fn text_arguments_signature_and_redacted_accumulations_are_bounded() {
    let chunk = "x".repeat(8192);
    let mut text_events = vec![message_start(1), start(0, json!({"type":"text","text":""}))];
    for _ in 0..9 { text_events.push(piece(0, json!({"type":"text_delta","text":chunk}))); }
    text_events.extend([stop(0), message_delta("end_turn", 1), message_stop()]);
    assert_failed(&play(text_events).await);
    let mut args = vec![message_start(1), tool_start(0, "tool_1")];
    for _ in 0..3 { args.push(arguments(0, &chunk)); }
    assert_failed(&play(args).await);
    let mut signatures = vec![message_start(1), start(0, json!({"type":"thinking","thinking":"","signature":""}))];
    for _ in 0..9 { signatures.push(piece(0, json!({"type":"signature_delta","signature":chunk}))); }
    assert_failed(&play(signatures).await);
    assert_failed(&play(vec![message_start(1),
        start(0, json!({"type":"redacted_thinking","data":"x".repeat(65536)})), stop(0),
        message_delta("end_turn", 1), message_stop()]).await);
    assert_failed(&play(vec![message_start(1),
        start(0, json!({"type":"thinking","thinking":"","signature":"s","opaque":"x".repeat(65536)})),
        stop(0), message_delta("end_turn", 1), message_stop()]).await);
}

#[tokio::test]
async fn usage_is_unknown_when_an_input_or_output_counter_is_missing() {
    let items = play(vec![
        event("message_start", json!({"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[]}})),
        start(0, json!({"type":"text","text":"ok"})), stop(0),
        message_delta("end_turn", 2), message_stop(),
    ]).await;
    assert!(matches!(items.last(), Some(Ok(ToolStreamEvent::Complete { usage_unknown: true, .. }))));
    let items = play(vec![message_start(1),
        start(0, json!({"type":"text","text":"ok"})), stop(0),
        event("message_delta", json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}})),
        message_stop(),
    ]).await;
    assert!(matches!(items.last(), Some(Ok(ToolStreamEvent::Complete { usage_unknown: true, .. }))),
        "message_start's output counter is not the completed output usage");
}

#[tokio::test]
async fn text_arrives_before_terminal_and_dropping_pending_tools_does_not_retry() {
    let mut script = sse_script(&[message_start(1), start(0, json!({"type":"text","text":"Working"})),
        stop(0), tool_start(1, "tool_1"), arguments(1, "{\"query\":")]);
    script.push(sleep(10_000));
    script.extend([text(&arguments(1, "\"late\"}")), text(&stop(1)),
        text(&message_delta("tool_use", 2)), text(&message_stop())]);
    let stub = serve(vec![script]).await;
    let adapter = anthropic(&stub.url, 3, 1);
    let mut stream = adapter.stream_tools("m", tool_request()).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), stream.next()).await.unwrap();
    assert!(matches!(first, Some(Ok(ToolStreamEvent::Delta(text))) if text == "Working"));
    assert!(tokio::time::timeout(Duration::from_millis(20), stream.next()).await.is_err(),
        "partial tool arguments must not produce any event");
    drop(stream);
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn oversized_multibyte_reply_emits_bounded_prefix_then_typed_limit() {
    let text = "中".repeat(64 * 1024 / 3 + 100);
    let items = play(vec![message_start(1), start(0, json!({"type":"text","text":text}))]).await;
    assert_failed(&items);
    assert!(matches!(items.last(), Some(Err(dgos_types::DgosError::ProviderOutputLimit))));
    let output: String = items.iter().filter_map(|item| match item { Ok(ToolStreamEvent::Delta(text)) => Some(text.as_str()), _ => None }).collect();
    assert!(output.len() <= 64 * 1024 && output.len() > 64 * 1024 - 3);
    assert!(text.starts_with(&output));
}

#[tokio::test]
async fn semantic_argument_and_call_limits_keep_typed_rejection_details() {
    let items = play(vec![message_start(1), start(0, json!({"type":"tool_use","id":"tool_1","name":"lookup","input":"invalid"}))]).await;
    assert!(matches!(items.last(), Some(Err(dgos_types::DgosError::ProviderToolArgumentsInvalid))));
    let items = play(vec![message_start(1), tool_start(0, "tool_1"), stop(0), tool_start(1, "tool_2")]).await;
    assert!(matches!(items.last(), Some(Err(dgos_types::DgosError::ProviderToolCallLimit))));
}
