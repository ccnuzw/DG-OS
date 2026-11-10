mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::{DgosResult, ToolChatRequest, ToolDefinition, ToolFunction, ToolMessage, ToolStreamEvent};
use futures::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;

fn tools_request() -> ToolChatRequest {
    ToolChatRequest {
        messages: vec![ToolMessage {
            role: "user".into(), content: Some("Read the selected item".into()),
            tool_call_id: None, tool_calls: vec![], reasoning: None, images: vec![],
        }],
        tools: vec![ToolDefinition {
            kind: "function".into(),
            function: ToolFunction {
                name: "read_item".into(), description: "Read one selected item".into(),
                parameters: json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
            },
        }],
        max_tokens: 512, temperature: None, reasoning: None,
    }
}

fn chunk(delta: Value, finish: Option<&str>) -> String {
    format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":delta,"finish_reason":finish}]}))
}

fn start_call() -> String {
    chunk(json!({"role":"assistant","tool_calls":[{"index":0,"id":"call_1","type":"function",
        "function":{"name":"read_item","arguments":"{\"id\":"}}]}), None)
}

fn arguments(value: &str) -> String {
    chunk(json!({"tool_calls":[{"index":0,"function":{"arguments":value}}]}), None)
}

fn end(reason: &str) -> String { chunk(json!({}), Some(reason)) }

async fn collect(events: Vec<String>) -> (Vec<DgosResult<ToolStreamEvent>>, usize) {
    let stub = serve(vec![sse_script(&events)]).await;
    let items = adapter(&stub.url, 3, 1).stream_tools("m", tools_request()).await.unwrap()
        .collect::<Vec<_>>().await;
    (items, stub.hits())
}

fn assert_failed(items: &[DgosResult<ToolStreamEvent>]) {
    assert!(items.last().unwrap().is_err(), "must end with an error");
    assert_eq!(items.iter().filter(|item| item.is_err()).count(), 1);
    assert!(!items.iter().any(|item| matches!(item, Ok(ToolStreamEvent::Complete { .. }))));
}

#[tokio::test]
async fn tool_stream_keeps_call_private_until_done_and_preserves_wire() {
    let stub = serve(vec![sse_script(&[
        chunk(json!({"reasoning_content":"Inspect "}), None),
        start_call(),
        arguments("\"selected\"}"),
        end("tool_calls"), usage(7, 4), DONE.into(),
    ])]).await;
    let items = adapter(&stub.url, 3, 1).stream_tools("m", tools_request()).await.unwrap()
        .collect::<Vec<_>>().await;
    assert_eq!(items.len(), 2, "one reasoning delta, one fully validated terminal");
    assert!(matches!(&items[0], Ok(ToolStreamEvent::ReasoningDelta(text)) if text == "Inspect "));
    let Ok(ToolStreamEvent::Complete { response, usage_unknown }) = &items[1] else { panic!("missing complete") };
    assert!(!usage_unknown);
    assert_eq!(response.usage.total_tokens, 11);
    assert_eq!(response.message.tool_calls.len(), 1);
    let call = &response.message.tool_calls[0];
    assert_eq!(call.id, "call_1");
    assert_eq!(call.function.name, "read_item");
    assert_eq!(call.function.arguments, "{\"id\":\"selected\"}");
    let wire: Value = serde_json::from_str(&stub.requests()[0].body).unwrap();
    assert_eq!(wire["stream"], true);
    assert_eq!(wire["stream_options"]["include_usage"], true);
    assert_eq!(wire["parallel_tool_calls"], false);
    assert_eq!(wire["tool_choice"], "auto");
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn final_answer_text_is_incremental_and_unknown_usage_is_explicit() {
    let (items, hits) = collect(vec![
        chunk(json!({"role":"assistant","content":"Hel"}), None),
        chunk(json!({"content":"lo"}), None), end("stop"), DONE.into(),
    ]).await;
    assert!(matches!(&items[0], Ok(ToolStreamEvent::Delta(text)) if text == "Hel"));
    assert!(matches!(&items[1], Ok(ToolStreamEvent::Delta(text)) if text == "lo"));
    let Ok(ToolStreamEvent::Complete { response, usage_unknown }) = &items[2] else { panic!("missing complete") };
    assert!(*usage_unknown);
    assert_eq!(response.message.content.as_deref(), Some("Hello"));
    assert!(response.message.tool_calls.is_empty());
    assert_eq!(hits, 1);
}

#[tokio::test]
async fn malformed_partial_or_non_object_arguments_never_produce_a_proposal() {
    for tail in ["\"selected\"", "oops}", "\"selected\"} trailing"] {
        let (items, hits) = collect(vec![start_call(), arguments(tail), end("tool_calls"), DONE.into()]).await;
        assert_failed(&items);
        assert_eq!(hits, 1);
    }
    for value in ["null", "[]", "42"] {
        let (items, _) = collect(vec![chunk(json!({"tool_calls":[{"index":0,"id":"c","type":"function",
            "function":{"name":"read_item","arguments":value}}]}), None), end("tool_calls"), DONE.into()]).await;
        assert_failed(&items);
    }
}

#[tokio::test]
async fn duplicate_reassigned_parallel_and_incomplete_identities_fail_closed() {
    for delta in [
        json!({"tool_calls":[{"index":0,"id":"call_1"}]}),
        json!({"tool_calls":[{"index":0,"id":"call_2"}]}),
        json!({"tool_calls":[{"index":0,"type":"function"}]}),
        json!({"tool_calls":[{"index":0,"function":{"name":"read_item"}}]}),
        json!({"tool_calls":[{"index":0,"function":{"name":"write_item"}}]}),
        json!({"tool_calls":[{"index":1,"id":"call_2"}]}),
        json!({"tool_calls":[{"index":0},{"index":0}]}),
        json!({"tool_calls":[{"function":{"arguments":"{}"}}]}),
        json!({"tool_calls":"invalid"}),
        json!({"tool_calls":[{"index":0,"function":{"arguments":{}}}]}),
    ] {
        let (items, _) = collect(vec![start_call(), chunk(delta, None), arguments("\"selected\"}"), end("tool_calls"), DONE.into()]).await;
        assert_failed(&items);
    }
    for call in [
        json!({"index":0,"type":"function","function":{"name":"read_item","arguments":"{}"}}),
        json!({"index":0,"id":"c","function":{"name":"read_item","arguments":"{}"}}),
        json!({"index":0,"id":"c","type":"function","function":{"arguments":"{}"}}),
    ] {
        let (items, _) = collect(vec![chunk(json!({"tool_calls":[call]}), None), end("tool_calls"), DONE.into()]).await;
        assert_failed(&items);
    }
}

#[tokio::test]
async fn finish_and_done_are_both_required_and_must_match_the_call() {
    let valid = vec![start_call(), arguments("\"selected\"}")];
    for suffix in [vec![], vec![DONE.into()], vec![end("tool_calls")],
        vec![end("stop"), DONE.into()], vec![end("length"), DONE.into()],
        vec![end("content_filter"), DONE.into()], vec![end("unexpected"), DONE.into()],
        vec![end("tool_calls"), arguments(" "), DONE.into()],
        vec![end("tool_calls"), end("tool_calls"), DONE.into()],
        vec!["data: {broken\n\n".into(), DONE.into()],
    ] {
        let (items, _) = collect(valid.iter().cloned().chain(suffix).collect()).await;
        assert_failed(&items);
    }
    let (items, _) = collect(vec![end("tool_calls"), DONE.into()]).await;
    assert_failed(&items);
}

#[tokio::test]
async fn accumulated_text_reasoning_opaque_data_and_arguments_are_bounded() {
    for events in [
        vec![chunk(json!({"content":"x".repeat(32768)}), None), chunk(json!({"content":"x".repeat(32769)}), None)],
        vec![chunk(json!({"reasoning":"x".repeat(32768)}), None), chunk(json!({"reasoning":"x".repeat(32768)}), None)],
        vec![chunk(json!({"reasoning_details":[{"index":0,"type":"reasoning.encrypted","data":"x".repeat(32768)}]}), None),
             chunk(json!({"reasoning_details":[{"index":0,"data":"x".repeat(32768)}]}), None)],
        vec![start_call(), arguments(&" ".repeat(8192)), arguments(&" ".repeat(8192))],
    ] {
        let (items, _) = collect(events).await;
        assert_failed(&items);
    }
}

#[tokio::test]
async fn original_reasoning_fields_and_indexed_signatures_roundtrip() {
    let details = json!([
        {"index":0,"id":"r0","type":"reasoning.text","format":"vendor-v1","text":"inspect item","signature":"signed"},
        {"index":1,"type":"reasoning.encrypted","data":"opaque"},
    ]);
    let stub = serve(vec![sse_script(&[
        chunk(json!({"reasoning_content":"inspect ","reasoning":"alternate ",
            "reasoning_details":[{"index":0,"id":"r0","type":"reasoning.text","format":"vendor-v1","text":"inspect ","signature":"sig"}]}), None),
        chunk(json!({"reasoning_content":"item","reasoning":"field","reasoning_details":[
            {"index":0,"text":"item","signature":"ned"},{"index":1,"type":"reasoning.encrypted","data":"opaque"}]}), None),
        start_call(), arguments("\"selected\"}"), end("tool_calls"), DONE.into(),
    ]), sse_script(&[chunk(json!({"content":"done"}), None), end("stop"), DONE.into()])]).await;
    let adapter = adapter(&stub.url, 0, 1);
    let items = adapter.stream_tools("m", tools_request()).await.unwrap().collect::<Vec<_>>().await;
    let Ok(ToolStreamEvent::Complete { response, .. }) = items.into_iter().last().unwrap() else { panic!("missing complete") };
    let reasoning = response.message.reasoning.as_ref().unwrap();
    assert_eq!(reasoning.text, "inspect item");
    assert_eq!(reasoning.fields.iter().find(|(name, _)| name == "reasoning_details").unwrap().1, details);
    let mut next = tools_request();
    next.messages.push(response.message);
    next.messages.push(ToolMessage { role:"tool".into(), content:Some("result".into()),
        tool_call_id:Some("call_1".into()), tool_calls:vec![], reasoning:None, images:vec![] });
    let _ = adapter.stream_tools("m", next).await.unwrap().collect::<Vec<_>>().await;
    let wire: Value = serde_json::from_str(&stub.requests()[1].body).unwrap();
    assert_eq!(wire["messages"][1]["reasoning_content"], "inspect item");
    assert_eq!(wire["messages"][1]["reasoning"], "alternate field");
    assert_eq!(wire["messages"][1]["reasoning_details"], details);
}

#[tokio::test]
async fn dropping_stream_during_partial_call_never_waits_for_a_terminal_proposal() {
    let stub = serve(vec![vec![sse_head(), text(&chunk(json!({"content":"Working"}), None)),
        text(&start_call()), sleep(60_000), text(&arguments("\"selected\"}")), text(&end("tool_calls")), text(DONE)]]).await;
    let mut stream = adapter(&stub.url, 3, 1).stream_tools("m", tools_request()).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), stream.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(first, ToolStreamEvent::Delta(text) if text == "Working"));
    // Cancellation is dropping the owned response stream: no proposal can escape later.
    drop(stream);
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn unsupported_tool_stream_has_no_retry_or_sync_fallback() {
    let stub = serve(vec![response(500, &[], "")]).await;
    assert!(adapter(&stub.url, 3, 1).stream_tools("m", tools_request()).await.is_err());
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn oversized_multibyte_reply_emits_bounded_prefix_then_typed_limit() {
    let text = "中".repeat(64 * 1024 / 3 + 100);
    let (items, _) = collect(vec![chunk(json!({"content":text}), Some("stop")), DONE.into()]).await;
    assert_failed(&items);
    assert!(matches!(items.last(), Some(Err(dgos_types::DgosError::ProviderOutputLimit))));
    let output: String = items.iter().filter_map(|item| match item { Ok(ToolStreamEvent::Delta(text)) => Some(text.as_str()), _ => None }).collect();
    assert!(output.len() <= 64 * 1024 && output.len() > 64 * 1024 - 3);
    assert!(text.starts_with(&output));
}

#[tokio::test]
async fn semantic_argument_and_call_limits_keep_typed_rejection_details() {
    let (items, _) = collect(vec![start_call(), arguments("oops}"), end("tool_calls"), DONE.into()]).await;
    assert!(matches!(items.last(), Some(Err(dgos_types::DgosError::ProviderToolArgumentsInvalid))));
    let (items, _) = collect(vec![chunk(json!({"tool_calls":[{"index":0},{"index":1}]}), None)]).await;
    assert!(matches!(items.last(), Some(Err(dgos_types::DgosError::ProviderToolCallLimit))));
}
