//! Wire contract of the FR-001 §6.3 additions: old JSON still parses, new fields are optional,
//! and reasoning content cannot leave the process through serialization or `Debug`.

use dgos_types::{
    ChatChunk, ChatRequest, ChatResponse, MessageReasoning, ProviderVendor, ProviderVendorEndpoint,
    ReasoningConfig, ResponseFormat, ToolMessage,
};
use serde_json::json;

#[test]
fn requests_and_responses_without_the_new_fields_still_parse() {
    let request: ChatRequest = serde_json::from_value(json!({
        "messages": [{"role": "user", "content": "hi"}],
        "temperature": 0.5, "max_tokens": 8, "stream": false
    }))
    .unwrap();
    assert!(request.reasoning.is_none() && request.response_format.is_none());

    let response: ChatResponse = serde_json::from_value(json!({
        "content": "x",
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
    }))
    .unwrap();
    assert!(response.reasoning.is_none() && response.structured.is_none());

    let chunk: ChatChunk =
        serde_json::from_value(json!({"delta": "a", "finish_reason": null})).unwrap();
    assert!(chunk.reasoning_delta.is_none());
}

#[test]
fn unset_additions_do_not_appear_on_the_wire() {
    let request = ChatRequest {
        messages: vec![],
        ..Default::default()
    };
    let value = serde_json::to_value(&request).unwrap();
    assert!(value.get("reasoning").is_none() && value.get("response_format").is_none());
    let chunk = serde_json::to_value(ChatChunk::default()).unwrap();
    assert!(chunk.get("reasoning_delta").is_none());
    let response = serde_json::to_value(ChatResponse::default()).unwrap();
    assert!(response.get("reasoning").is_none() && response.get("structured").is_none());
}

#[test]
fn reasoning_and_response_format_round_trip() {
    let request: ChatRequest = serde_json::from_value(json!({
        "messages": [],
        "reasoning": {"effort": "xhigh"},
        "response_format": {"type": "json_schema", "name": "a", "schema": {"type": "object"}}
    }))
    .unwrap();
    assert_eq!(
        request.reasoning,
        Some(ReasoningConfig {
            effort: Some("xhigh".into()),
            budget_tokens: None
        })
    );
    let format: ResponseFormat = request.response_format.unwrap();
    assert_eq!(format.kind, "json_schema");
    assert_eq!(format.strict, None);
    let back = serde_json::to_value(&format).unwrap();
    assert_eq!(back["type"], "json_schema");
    assert!(back.get("strict").is_none());
}

#[test]
fn tool_message_reasoning_is_never_serialized_or_deserialized() {
    let message = ToolMessage {
        role: "assistant".into(),
        content: Some("text".into()),
        tool_call_id: None,
        tool_calls: vec![],
        reasoning: Some(MessageReasoning {
            text: "PRIVATE-THOUGHT".into(),
            fields: vec![("reasoning_content".into(), json!("PRIVATE-THOUGHT"))],
            blocks: vec![json!({"type": "thinking", "thinking": "PRIVATE-THOUGHT",
                "signature": "SECRET-SIG"})],
        }),
    };
    let wire = serde_json::to_string(&message).unwrap();
    assert!(
        !wire.contains("PRIVATE-THOUGHT") && !wire.contains("SECRET-SIG"),
        "{wire}"
    );
    assert!(!wire.contains("reasoning"), "{wire}");

    // A renderer cannot inject reasoning either: the key is ignored on input.
    let parsed: ToolMessage = serde_json::from_value(json!({
        "role": "assistant", "content": "x",
        "reasoning": {"text": "injected", "fields": [], "blocks": []}
    }))
    .unwrap();
    assert!(parsed.reasoning.is_none());
}

#[test]
fn message_reasoning_debug_shows_sizes_not_content() {
    let reasoning = MessageReasoning {
        text: "PRIVATE-THOUGHT".into(),
        fields: vec![("reasoning".into(), json!("PRIVATE-THOUGHT"))],
        blocks: vec![json!({"signature": "SECRET-SIG"})],
    };
    let shown = format!("{reasoning:?}");
    assert!(
        !shown.contains("PRIVATE-THOUGHT") && !shown.contains("SECRET-SIG"),
        "{shown}"
    );
    let message = ToolMessage {
        role: "assistant".into(),
        content: None,
        tool_call_id: None,
        tool_calls: vec![],
        reasoning: Some(reasoning),
    };
    let shown = format!("{message:?}");
    assert!(
        !shown.contains("PRIVATE-THOUGHT") && !shown.contains("SECRET-SIG"),
        "{shown}"
    );
}

#[test]
fn vendor_shape_matches_the_published_contract() {
    let vendor = ProviderVendor {
        vendor_id: "custom".into(),
        display_name: "自定义".into(),
        endpoints: vec![ProviderVendorEndpoint {
            provider_type: "anthropic".into(),
            display_name: "Anthropic 兼容".into(),
            default_base_url: None,
        }],
    };
    assert_eq!(
        serde_json::to_value(&vendor).unwrap(),
        json!({"vendor_id": "custom", "display_name": "自定义", "endpoints": [
            {"provider_type": "anthropic", "display_name": "Anthropic 兼容",
             "default_base_url": null}]})
    );
}
