use dgos_providers::{OpenAIAdapter, ProviderAdapter};
use dgos_types::{
    ToolCall, ToolChatRequest, ToolDefinition, ToolFunction, ToolFunctionCall, ToolMessage,
};
use serde_json::json;

fn request() -> ToolChatRequest {
    ToolChatRequest {
        messages: vec![
            ToolMessage {
                role: "assistant".into(),
                content: None,
                tool_call_id: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    kind: "function".into(),
                    function: ToolFunctionCall {
                        name: "dgos_tool_0".into(),
                        arguments: "{}".into(),
                    },
                }],
            },
            ToolMessage {
                role: "tool".into(),
                content: Some("untrusted result".into()),
                tool_call_id: Some("call_1".into()),
                tool_calls: vec![],
            },
        ],
        tools: vec![ToolDefinition {
            kind: "function".into(),
            function: ToolFunction {
                name: "dgos_tool_0".into(),
                description: "Selected tool".into(),
                parameters: json!({"type":"object"}),
            },
        }],
        max_tokens: 1024,
    }
}

#[tokio::test]
async fn tool_wire_preserves_correlation_and_nullable_assistant_content() {
    let mut server = mockito::Server::new_async().await;
    let wire = server.mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer synthetic-key")
        .match_body(mockito::Matcher::PartialJson(json!({
            "model":"test", "stream":false, "parallel_tool_calls":false, "tool_choice":"auto",
            "messages":[
                {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"dgos_tool_0","arguments":"{}"}}]},
                {"role":"tool","content":"untrusted result","tool_call_id":"call_1"}
            ], "tools":[{"type":"function","function":{"name":"dgos_tool_0","description":"Selected tool","parameters":{"type":"object"}}}]
        })))
        .with_status(200).with_header("content-type", "application/json")
        .with_body(json!({"choices":[{"message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"call_2","type":"function","function":{"name":"dgos_tool_0","arguments":"{}"}}]},
            "finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13}}).to_string())
        .expect(1).create_async().await;
    let adapter = OpenAIAdapter::new(server.url(), "synthetic-key".into(), None);
    let result = adapter.call_tools("test", request()).await.unwrap();
    assert_eq!(result.message.tool_calls[0].id, "call_2");
    assert!(result.message.content.is_none());
    assert_eq!(result.usage.total_tokens, 13);
    wire.assert_async().await;
}

#[tokio::test]
async fn incomplete_tool_response_is_rejected_without_retry() {
    let mut server = mockito::Server::new_async().await;
    let wire = server.mock("POST", "/chat/completions").with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"choices":[{"message":{"role":"assistant","content":"partial"},"finish_reason":"length"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}).to_string())
        .expect(1).create_async().await;
    let adapter = OpenAIAdapter::new(server.url(), "synthetic-key".into(), None);
    assert!(adapter.call_tools("test", request()).await.is_err());
    wire.assert_async().await;
}

#[tokio::test]
async fn unsupported_tool_api_fails_without_text_fallback_or_retry() {
    let mut server = mockito::Server::new_async().await;
    let wire = server
        .mock("POST", "/chat/completions")
        .with_status(400)
        .expect(1)
        .create_async()
        .await;
    let adapter = OpenAIAdapter::new(server.url(), "synthetic-key".into(), None);
    assert!(adapter.call_tools("test", request()).await.is_err());
    wire.assert_async().await;
}
