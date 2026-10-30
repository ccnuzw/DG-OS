//! FR-001 AC26, `openai_compatible` side: what the host refuses locally under a profile that
//! declares a constraint, and what it sends unchanged otherwise (the upstream decides). The
//! profile is chosen by the test because a loopback address can never be an official host.
mod common;

use common::*;
use dgos_providers::{AdapterOptions, OpenAIAdapter, ProviderAdapter};
use dgos_types::{
    ChatRequest, DgosError, ReasoningConfig, ResponseFormat, ToolChatRequest, ToolDefinition,
    ToolFunction, ToolMessage,
};
use serde_json::{json, Value};

const MARKER: &str = "UPSTREAM-MARKER-4410";

fn ok_response() -> Vec<Step> {
    response(200, &[("content-type", "application/json")], &ok_chat())
}

fn body_of(stub: &Stub, index: usize) -> Value {
    serde_json::from_str(&stub.requests()[index].body).unwrap()
}

fn assert_clean(error: &DgosError) {
    let text = error.to_string();
    assert!(!text.contains(API_KEY), "key leaked: {text}");
    assert!(!text.contains(MARKER), "upstream text leaked: {text}");
}

fn official(url: &str) -> OpenAIAdapter {
    openai_official(url, AdapterOptions::default())
}

fn reasoning(effort: &str) -> Option<ReasoningConfig> {
    Some(ReasoningConfig {
        effort: Some(effort.into()),
        budget_tokens: None,
    })
}

fn sampled(temperature: f32, effort: Option<&str>) -> ChatRequest {
    ChatRequest {
        temperature: Some(temperature),
        reasoning: effort.and_then(reasoning),
        ..request()
    }
}

fn structured_request() -> ChatRequest {
    ChatRequest {
        response_format: Some(ResponseFormat {
            kind: "json_schema".into(),
            name: "answer".into(),
            schema: json!({"type": "object", "properties": {"n": {"type": "integer"}}}),
            strict: None,
        }),
        ..request()
    }
}

fn tool_request() -> ToolChatRequest {
    ToolChatRequest {
        messages: vec![ToolMessage {
            role: "user".into(),
            content: Some("hi".into()),
            tool_call_id: None,
            tool_calls: vec![],
            reasoning: None,
        }],
        tools: vec![ToolDefinition {
            kind: "function".into(),
            function: ToolFunction {
                name: "read_file".into(),
                description: "Selected tool".into(),
                parameters: json!({"type": "object"}),
            },
        }],
        max_tokens: 321,
    }
}

async fn refused_call(adapter: &OpenAIAdapter, stub: &Stub, request: ChatRequest) {
    let error = adapter
        .call("m", request)
        .await
        .expect_err("call must be refused locally");
    assert_eq!(error.error_code(), 1001020, "{error}");
    assert_clean(&error);
    assert_eq!(
        stub.hits(),
        0,
        "a refused request must not reach the upstream"
    );
}

#[tokio::test]
async fn passthrough_generic_profile_sends_temperature_and_reasoning_effort_unchanged() {
    let stub = serve(vec![ok_response()]).await;
    let response = adapter(&stub.url, 0, 1)
        .call("m", sampled(0.5, Some("high")))
        .await
        .unwrap();
    assert_eq!(response.content, "ok");
    assert_eq!(stub.hits(), 1);
    let body = body_of(&stub, 0);
    assert_eq!(body["temperature"], json!(0.5));
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["max_tokens"], 16);
    assert!(body.get("max_completion_tokens").is_none(), "{body}");
}

#[tokio::test]
async fn passthrough_generic_profile_hands_any_effort_string_to_the_upstream() {
    // The host keeps no list of valid levels; the upstream rejects what it does not know.
    let stub = serve(vec![ok_response()]).await;
    let client = adapter(&stub.url, 0, 1);
    for effort in ["none", "minimal", "xhigh", "custom-level"] {
        client.call("m", sampled(0.2, Some(effort))).await.unwrap();
    }
    assert_eq!(stub.hits(), 4);
    for (index, effort) in ["none", "minimal", "xhigh", "custom-level"]
        .iter()
        .enumerate()
    {
        assert_eq!(body_of(&stub, index)["reasoning_effort"], *effort);
        assert_eq!(body_of(&stub, index)["temperature"], json!(0.2));
    }
}

#[tokio::test]
async fn passthrough_official_profile_uses_max_completion_tokens() {
    let stub = serve(vec![ok_response()]).await;
    official(&stub.url).call("m", request()).await.unwrap();
    let body = body_of(&stub, 0);
    assert_eq!(body["max_completion_tokens"], 16);
    assert!(body.get("max_tokens").is_none(), "{body}");
}

#[tokio::test]
async fn passthrough_official_profile_sends_temperature_without_reasoning() {
    let stub = serve(vec![ok_response()]).await;
    official(&stub.url)
        .call("m", sampled(0.5, None))
        .await
        .unwrap();
    assert_eq!(stub.hits(), 1);
    let body = body_of(&stub, 0);
    assert_eq!(body["temperature"], json!(0.5));
    assert!(body.get("reasoning_effort").is_none(), "{body}");
}

#[tokio::test]
async fn passthrough_official_profile_rejects_temperature_with_active_reasoning() {
    for effort in ["low", "medium", "high", "xhigh"] {
        let stub = serve(vec![ok_response()]).await;
        refused_call(&official(&stub.url), &stub, sampled(0.5, Some(effort))).await;
    }
}

#[tokio::test]
async fn passthrough_official_profile_sends_temperature_with_effort_none() {
    let stub = serve(vec![ok_response()]).await;
    official(&stub.url)
        .call("m", sampled(0.5, Some("none")))
        .await
        .unwrap();
    assert_eq!(stub.hits(), 1);
    let body = body_of(&stub, 0);
    assert_eq!(body["temperature"], json!(0.5));
    assert_eq!(body["reasoning_effort"], "none");
}

#[tokio::test]
async fn passthrough_official_profile_sends_reasoning_without_temperature() {
    // The refused combination is temperature plus reasoning; reasoning alone is fine.
    let stub = serve(vec![ok_response()]).await;
    official(&stub.url)
        .call(
            "m",
            ChatRequest {
                reasoning: reasoning("low"),
                ..request()
            },
        )
        .await
        .unwrap();
    let body = body_of(&stub, 0);
    assert_eq!(body["reasoning_effort"], "low");
    assert!(body.get("temperature").is_none(), "{body}");
}

#[tokio::test]
async fn passthrough_streaming_with_response_format_is_refused_on_both_profiles() {
    for is_official in [false, true] {
        let stub = serve(vec![ok_response()]).await;
        let client = if is_official {
            official(&stub.url)
        } else {
            adapter(&stub.url, 0, 1)
        };
        let error = match client.stream("m", structured_request()).await {
            Err(error) => error,
            Ok(_) => panic!("official={is_official}: stream must be refused"),
        };
        assert_eq!(
            error.error_code(),
            1001020,
            "official={is_official}: {error}"
        );
        assert_clean(&error);
        assert_eq!(stub.hits(), 0, "official={is_official}");
    }
}

#[tokio::test]
async fn passthrough_non_streaming_response_format_is_sent_on_both_profiles() {
    let structured_ok = response(
        200,
        &[("content-type", "application/json")],
        &json!({"choices":[{"message":{"role":"assistant","content":"{\"n\":3}"},
            "finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
        .to_string(),
    );
    for is_official in [false, true] {
        let stub = serve(vec![structured_ok.clone()]).await;
        let client = if is_official {
            official(&stub.url)
        } else {
            adapter(&stub.url, 0, 1)
        };
        let result = client.call("m", structured_request()).await.unwrap();
        assert_eq!(
            result.structured,
            Some(json!({"n": 3})),
            "official={is_official}"
        );
        let body = body_of(&stub, 0);
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["name"], "answer");
    }
}

#[tokio::test]
async fn passthrough_non_chat_model_id_is_not_blocked_by_the_host() {
    // The host classifies models for display but does not gate calls by kind.
    for is_official in [false, true] {
        let stub = serve(vec![ok_response()]).await;
        let client = if is_official {
            official(&stub.url)
        } else {
            adapter(&stub.url, 0, 1)
        };
        client
            .call("text-embedding-3-small", request())
            .await
            .unwrap_or_else(|error| panic!("official={is_official}: {error}"));
        assert_eq!(stub.hits(), 1, "official={is_official}");
        assert_eq!(body_of(&stub, 0)["model"], "text-embedding-3-small");
    }
}

#[tokio::test]
async fn passthrough_tool_body_generic_profile() {
    let stub = serve(vec![ok_response()]).await;
    adapter(&stub.url, 0, 1)
        .call_tools("m", tool_request())
        .await
        .unwrap();
    let body = body_of(&stub, 0);
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], json!(false));
    assert_eq!(body["max_tokens"], 321);
    assert!(body.get("max_completion_tokens").is_none(), "{body}");
    assert_eq!(body["stream"], json!(false));
    assert_eq!(body["tools"][0]["function"]["name"], "read_file");
}

#[tokio::test]
async fn passthrough_tool_body_official_profile() {
    let stub = serve(vec![ok_response()]).await;
    official(&stub.url)
        .call_tools("m", tool_request())
        .await
        .unwrap();
    let body = body_of(&stub, 0);
    assert_eq!(body["max_completion_tokens"], 321);
    assert!(body.get("max_tokens").is_none(), "{body}");
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], json!(false));
}

#[tokio::test]
async fn passthrough_upstream_error_text_never_reaches_the_error_message() {
    let body = format!("{{\"error\":{{\"message\":\"{MARKER} {API_KEY}\"}}}}");
    let stub = serve(vec![response(400, &[], &body)]).await;
    let error = adapter(&stub.url, 2, 1)
        .call("m", sampled(0.5, Some("high")))
        .await
        .expect_err("a 400 is final");
    assert_clean(&error);
    assert_eq!(stub.hits(), 1);
}
