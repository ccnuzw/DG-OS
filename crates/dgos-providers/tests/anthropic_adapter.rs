//! FR-001 AC15, AC18, AC22, AC26 for the `anthropic` adapter: profile headers, model paging,
//! request/response normalisation, tool use mapping and local refusals. Everything runs against
//! the scripted local stub; the profile is chosen by the test because a loopback address can
//! never be an official host.
mod common;

use common::*;
use dgos_providers::{AdapterOptions, AnthropicAdapter, AuthStyle, ProviderAdapter};
use dgos_types::{
    ChatMessage, ChatRequest, DgosError, KindSource, MessageReasoning, ModelSource,
    ReasoningConfig, ResponseFormat, ToolCall, ToolChatRequest, ToolDefinition, ToolFunction,
    ToolFunctionCall, ToolMessage,
};
use serde_json::{json, Value};
use std::time::Duration;

const MARKER: &str = "UPSTREAM-MARKER-7731";

// ---------------------------------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------------------------------

fn msg(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: role.into(),
        content: content.into(),
    }
}

fn chat(messages: Vec<ChatMessage>) -> ChatRequest {
    ChatRequest {
        messages,
        max_tokens: Some(64),
        ..Default::default()
    }
}

fn json_ok(body: &Value) -> Vec<Step> {
    response(
        200,
        &[("content-type", "application/json")],
        &body.to_string(),
    )
}

fn message_body(stop_reason: &str, content: Value, usage: Value) -> Value {
    json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "m",
        "content": content, "stop_reason": stop_reason, "usage": usage})
}

fn body_of(stub: &Stub, index: usize) -> Value {
    serde_json::from_str(&stub.requests()[index].body).unwrap()
}

fn request_line(stub: &Stub, index: usize) -> String {
    stub.requests()[index]
        .head
        .lines()
        .next()
        .unwrap()
        .to_owned()
}

/// Value of one header (name compared case-insensitively), if the request carried it.
fn header(captured: &Captured, name: &str) -> Option<String> {
    captured.head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

fn assert_clean(error: &DgosError) {
    let text = error.to_string();
    assert!(!text.contains(API_KEY), "key leaked: {text}");
    assert!(!text.contains(MARKER), "upstream text leaked: {text}");
}

fn models_page(models: &[(&str, u64, u64)], has_more: bool, last_id: Option<&str>) -> Vec<Step> {
    let data: Vec<Value> = models
        .iter()
        .map(|(id, input, output)| {
            json!({"id": id, "type": "model", "display_name": format!("Name of {id}"),
                "created_at": "2026-01-01T00:00:00Z",
                "max_input_tokens": input, "max_tokens": output})
        })
        .collect();
    let mut page = json!({"data": data, "has_more": has_more});
    if let Some(last) = last_id {
        page["last_id"] = json!(last);
    }
    json_ok(&page)
}

fn options_with(auth_style: Option<AuthStyle>, workspace_id: Option<&str>) -> AdapterOptions {
    AdapterOptions {
        auth_style,
        workspace_id: workspace_id.map(str::to_owned),
        ..Default::default()
    }
}

fn tool_message(
    role: &str,
    content: Option<&str>,
    tool_call_id: Option<&str>,
    tool_calls: Vec<ToolCall>,
    reasoning: Option<MessageReasoning>,
) -> ToolMessage {
    ToolMessage {
        role: role.into(),
        content: content.map(str::to_owned),
        tool_call_id: tool_call_id.map(str::to_owned),
        tool_calls,
        reasoning,
    }
}

fn call_of(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        kind: "function".into(),
        function: ToolFunctionCall {
            name: name.into(),
            arguments: arguments.into(),
        },
    }
}

fn tool_request(messages: Vec<ToolMessage>) -> ToolChatRequest {
    ToolChatRequest {
        messages,
        tools: vec![ToolDefinition {
            kind: "function".into(),
            function: ToolFunction {
                name: "read_file".into(),
                description: "Selected tool".into(),
                parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            },
        }],
        max_tokens: 256,
    }
}

fn user_tool_message(text: &str) -> ToolMessage {
    tool_message("user", Some(text), None, vec![], None)
}

fn tool_use_stop(content: Value) -> Vec<Step> {
    json_ok(&message_body(
        "tool_use",
        content,
        json!({"input_tokens": 10, "output_tokens": 5}),
    ))
}

/// A stub that must never be reached.
async fn silent_stub() -> Stub {
    serve(vec![json_ok(&json!({"unused": true}))]).await
}

async fn rejected_call(adapter: &AnthropicAdapter, stub: &Stub, request: ChatRequest) {
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

async fn rejected_stream(adapter: &AnthropicAdapter, stub: &Stub, request: ChatRequest) {
    let error = match adapter.stream("m", request).await {
        Err(error) => error,
        Ok(_) => panic!("stream must be refused locally"),
    };
    assert_eq!(error.error_code(), 1001020, "{error}");
    assert_clean(&error);
    assert_eq!(
        stub.hits(),
        0,
        "a refused request must not reach the upstream"
    );
}

async fn rejected_both(adapter: &AnthropicAdapter, stub: &Stub, request: ChatRequest) {
    rejected_call(adapter, stub, request.clone()).await;
    rejected_stream(adapter, stub, request).await;
}

// ---------------------------------------------------------------------------------------------
// AC15: headers, base_url, model paging
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn headers_generic_profile_sends_bearer_x_api_key_and_old_version() {
    let stub = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    anthropic(&stub.url, 0, 1)
        .call("m", request())
        .await
        .unwrap();
    let sent = &stub.requests()[0];
    assert_eq!(
        header(sent, "authorization").as_deref(),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(header(sent, "x-api-key").as_deref(), Some(API_KEY));
    assert_eq!(
        header(sent, "anthropic-version").as_deref(),
        Some("2023-06-01")
    );
    assert!(header(sent, "anthropic-workspace-id").is_none());
}

#[tokio::test]
async fn headers_official_profile_sends_only_bearer_and_new_version() {
    let stub = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    anthropic_official(&stub.url, AdapterOptions::default())
        .call("m", request())
        .await
        .unwrap();
    let sent = &stub.requests()[0];
    assert_eq!(
        header(sent, "authorization").as_deref(),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert!(header(sent, "x-api-key").is_none(), "{}", sent.head);
    assert_eq!(
        header(sent, "anthropic-version").as_deref(),
        Some("2026-10-21")
    );
}

#[tokio::test]
async fn headers_are_sent_on_model_list_requests() {
    let generic = serve(vec![models_page(&[("a", 10, 5)], false, Some("a"))]).await;
    anthropic(&generic.url, 0, 1).list_models().await.unwrap();
    let sent = &generic.requests()[0];
    assert!(sent.head.starts_with("GET "), "{}", sent.head);
    assert_eq!(header(sent, "x-api-key").as_deref(), Some(API_KEY));
    assert_eq!(
        header(sent, "authorization").as_deref(),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(
        header(sent, "anthropic-version").as_deref(),
        Some("2023-06-01")
    );

    let official = serve(vec![models_page(&[("a", 10, 5)], false, Some("a"))]).await;
    anthropic_official(&official.url, AdapterOptions::default())
        .list_models()
        .await
        .unwrap();
    let sent = &official.requests()[0];
    assert!(header(sent, "x-api-key").is_none(), "{}", sent.head);
    assert_eq!(
        header(sent, "anthropic-version").as_deref(),
        Some("2026-10-21")
    );
}

#[tokio::test]
async fn headers_auth_style_override_replaces_the_profile_default() {
    // (style, sends Authorization, sends x-api-key)
    let cases = [
        (AuthStyle::XApiKey, false, true),
        (AuthStyle::Both, true, true),
        (AuthStyle::Bearer, true, false),
    ];
    for official in [false, true] {
        for (style, bearer, x_api_key) in cases {
            let stub = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
            let options = options_with(Some(style), None);
            let adapter = if official {
                anthropic_official(&stub.url, options)
            } else {
                anthropic_with(&stub.url, 0, 1, options)
            };
            adapter.call("m", request()).await.unwrap();
            let sent = &stub.requests()[0];
            let label = format!("official={official} style={style:?}");
            assert_eq!(header(sent, "authorization").is_some(), bearer, "{label}");
            assert_eq!(header(sent, "x-api-key").is_some(), x_api_key, "{label}");
            if x_api_key {
                assert_eq!(
                    header(sent, "x-api-key").as_deref(),
                    Some(API_KEY),
                    "{label}"
                );
            }
            // The version header belongs to the profile, not to the auth override.
            let version = if official { "2026-10-21" } else { "2023-06-01" };
            assert_eq!(
                header(sent, "anthropic-version").as_deref(),
                Some(version),
                "{label}"
            );
        }
    }
}

#[tokio::test]
async fn headers_workspace_id_is_sent_only_under_the_official_profile() {
    let official = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    anthropic_official(&official.url, options_with(None, Some("wrkspc_01ABC")))
        .call("m", request())
        .await
        .unwrap();
    assert_eq!(
        header(&official.requests()[0], "anthropic-workspace-id").as_deref(),
        Some("wrkspc_01ABC")
    );

    // The same option on a model list request of the official profile.
    let listing = serve(vec![models_page(&[("a", 10, 5)], false, Some("a"))]).await;
    anthropic_official(&listing.url, options_with(None, Some("wrkspc_01ABC")))
        .list_models()
        .await
        .unwrap();
    assert_eq!(
        header(&listing.requests()[0], "anthropic-workspace-id").as_deref(),
        Some("wrkspc_01ABC")
    );

    let unset = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    anthropic_official(&unset.url, AdapterOptions::default())
        .call("m", request())
        .await
        .unwrap();
    assert!(header(&unset.requests()[0], "anthropic-workspace-id").is_none());

    let generic = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    anthropic_with(&generic.url, 0, 1, options_with(None, Some("wrkspc_01ABC")))
        .call("m", request())
        .await
        .unwrap();
    assert!(
        header(&generic.requests()[0], "anthropic-workspace-id").is_none(),
        "{}",
        generic.requests()[0].head
    );
}

#[tokio::test]
async fn base_url_trailing_v1_is_normalised_for_messages_and_models() {
    for suffix in ["", "/", "/v1", "/v1/"] {
        let calls = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
        anthropic(&format!("{}{suffix}", calls.url), 0, 1)
            .call("m", request())
            .await
            .unwrap();
        assert_eq!(
            request_line(&calls, 0),
            "POST /v1/messages HTTP/1.1",
            "suffix {suffix:?}"
        );

        let listing = serve(vec![models_page(&[("a", 10, 5)], false, Some("a"))]).await;
        anthropic(&format!("{}{suffix}", listing.url), 0, 1)
            .list_models()
            .await
            .unwrap();
        let line = request_line(&listing, 0);
        assert!(
            line.starts_with("GET /v1/models?"),
            "suffix {suffix:?}: {line}"
        );
        assert!(!line.contains("/v1/v1"), "suffix {suffix:?}: {line}");
    }

    let official = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    anthropic_official(&format!("{}/v1", official.url), AdapterOptions::default())
        .call("m", request())
        .await
        .unwrap();
    assert_eq!(request_line(&official, 0), "POST /v1/messages HTTP/1.1");
}

#[tokio::test]
async fn models_two_pages_are_followed_with_after_id_and_merged() {
    let stub = serve(vec![
        models_page(
            &[("model-a", 200_000, 64_000), ("model-b", 100_000, 32_000)],
            true,
            Some("model-b"),
        ),
        // `model-b` is repeated on the second page and must be kept once.
        models_page(
            &[
                ("model-b", 100_000, 32_000),
                ("model-c", 1_000_000, 128_000),
            ],
            false,
            Some("model-c"),
        ),
    ])
    .await;
    let models = anthropic(&stub.url, 0, 1).list_models().await.unwrap();

    assert_eq!(stub.hits(), 2);
    let first = request_line(&stub, 0);
    assert!(first.contains("limit="), "{first}");
    assert!(!first.contains("after_id"), "{first}");
    let second = request_line(&stub, 1);
    assert!(second.contains("after_id=model-b"), "{second}");
    assert!(second.contains("limit="), "{second}");

    let ids: Vec<&str> = models.iter().map(|m| m.model_id.as_str()).collect();
    assert_eq!(ids, ["model-a", "model-b", "model-c"]);
    for model in &models {
        assert_eq!(model.model_type, "text-generation", "{}", model.model_id);
        assert_eq!(
            model.kind_source,
            KindSource::Protocol,
            "{}",
            model.model_id
        );
        assert_eq!(model.source, ModelSource::Remote, "{}", model.model_id);
    }
    assert_eq!(models[0].context_window, Some(200_000));
    assert_eq!(models[0].max_output_tokens, Some(64_000));
    assert_eq!(models[1].context_window, Some(100_000));
    assert_eq!(models[1].max_output_tokens, Some(32_000));
    assert_eq!(models[2].context_window, Some(1_000_000));
    assert_eq!(models[2].max_output_tokens, Some(128_000));
    assert_eq!(models[0].model_name, "Name of model-a");
}

#[tokio::test]
async fn models_repeated_last_id_stops_the_paging() {
    // Every page says "more, continue after m1": the second page repeats the cursor.
    let stub = serve(vec![models_page(&[("m1", 10, 5)], true, Some("m1"))]).await;
    let models = anthropic(&stub.url, 0, 1).list_models().await.unwrap();
    assert_eq!(stub.hits(), 2, "the repeated cursor must end the loop");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model_id, "m1");
}

#[tokio::test]
async fn models_missing_or_empty_last_id_stops_the_paging() {
    for last in [None, Some("")] {
        let stub = serve(vec![models_page(&[("m1", 10, 5)], true, last)]).await;
        let models = anthropic(&stub.url, 0, 1).list_models().await.unwrap();
        assert_eq!(stub.hits(), 1, "last_id {last:?}");
        assert_eq!(models.len(), 1, "last_id {last:?}");
    }
}

#[tokio::test]
async fn models_cursor_cycle_is_bounded() {
    // Two pages that point at each other would loop forever without a page cap.
    let stub = serve(vec![
        models_page(&[("x1", 10, 5)], true, Some("p1")),
        models_page(&[("x2", 10, 5)], true, Some("p2")),
    ])
    .await;
    let adapter = anthropic(&stub.url, 0, 1);
    let models = tokio::time::timeout(Duration::from_secs(30), adapter.list_models())
        .await
        .expect("paging must end")
        .unwrap();
    assert!(stub.hits() <= 50, "hits {}", stub.hits());
    assert_eq!(models.len(), 2, "duplicates are removed");
}

#[tokio::test]
async fn models_404_on_first_page_is_an_error_and_probe_without_model_is_not_supported() {
    let listing = serve(vec![response(
        404,
        &[],
        &format!("{{\"error\":\"{MARKER}\"}}"),
    )])
    .await;
    let error = anthropic(&listing.url, 0, 1)
        .list_models()
        .await
        .expect_err("a missing model list is an error for list_models");
    assert_eq!(error.error_code(), 1001003, "{error}");
    assert!(
        matches!(error, DgosError::ProviderUnavailable(_)),
        "{error}"
    );
    assert_clean(&error);
    assert_eq!(listing.hits(), 1);

    let probe = serve(vec![response(
        404,
        &[],
        &format!("{{\"error\":\"{MARKER}\"}}"),
    )])
    .await;
    let error = anthropic(&probe.url, 0, 1)
        .test_connection(None)
        .await
        .expect_err("no list and no probe model cannot prove the connection");
    assert_eq!(error.error_code(), 1001020, "{error}");
    assert_clean(&error);
    assert_eq!(probe.hits(), 1, "no probe request without a probe model");
}

#[tokio::test]
async fn models_404_on_a_later_page_is_an_invalid_response() {
    let stub = serve(vec![
        models_page(&[("m1", 10, 5)], true, Some("m1")),
        response(404, &[], "{}"),
    ])
    .await;
    let error = anthropic(&stub.url, 0, 1)
        .list_models()
        .await
        .expect_err("a page that disappears mid-list is inconsistent");
    assert_eq!(error.error_code(), 1001025, "{error}");
    assert_eq!(stub.hits(), 2);
}

#[tokio::test]
async fn models_401_is_invalid_api_key_without_echoing_the_body() {
    let body = format!("{{\"error\":{{\"message\":\"bad {API_KEY} {MARKER}\"}}}}");
    let stub = serve(vec![response(401, &[], &body)]).await;
    let error = anthropic(&stub.url, 3, 1)
        .list_models()
        .await
        .expect_err("401 is final");
    assert_eq!(error.error_code(), 1001002, "{error}");
    assert_clean(&error);
    assert_eq!(stub.hits(), 1);
}

// ---------------------------------------------------------------------------------------------
// AC18: synchronous call normalisation
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn sync_request_golden_system_on_top_and_user_messages_merged() {
    let stub = serve(vec![json_ok(&message_body(
        "end_turn",
        json!([{"type": "text", "text": "hi"}]),
        json!({"input_tokens": 11, "output_tokens": 7}),
    ))])
    .await;
    let request = ChatRequest {
        messages: vec![
            msg("system", "You are terse."),
            msg("user", "first"),
            msg("user", "second"),
        ],
        max_tokens: None,
        ..Default::default()
    };
    let response = anthropic(&stub.url, 0, 1)
        .call("claude-test", request)
        .await
        .unwrap();

    assert_eq!(stub.hits(), 1);
    assert_eq!(
        body_of(&stub, 0),
        json!({
            "model": "claude-test",
            "max_tokens": 4096,
            "system": "You are terse.",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "first"},
                    {"type": "text", "text": "second"}
                ]}
            ],
            "stream": false
        })
    );
    assert_eq!(response.content, "hi");
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.usage.prompt_tokens, 11);
    assert_eq!(response.usage.completion_tokens, 7);
    assert_eq!(response.usage.total_tokens, 18);
    assert!(!response.usage_unknown);
    assert!(response.reasoning.is_none());
    assert!(response.structured.is_none());
}

#[tokio::test]
async fn sync_request_uses_the_given_max_tokens_and_sends_no_optional_keys() {
    let stub = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    let request = ChatRequest {
        messages: vec![msg("user", "hello")],
        max_tokens: Some(777),
        ..Default::default()
    };
    anthropic(&stub.url, 0, 1).call("m", request).await.unwrap();
    let body = body_of(&stub, 0);
    assert_eq!(body["max_tokens"], 777);
    for key in [
        "thinking",
        "temperature",
        "output_config",
        "system",
        "tools",
    ] {
        assert!(body.get(key).is_none(), "{key} must be absent: {body}");
    }
    assert_ne!(body["stream"], json!(true));
}

#[tokio::test]
async fn sync_multiple_system_messages_all_end_up_in_the_top_level_system() {
    let stub = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    let request = chat(vec![
        msg("system", "RULE-ONE"),
        msg("system", "RULE-TWO"),
        msg("user", "hello"),
    ]);
    anthropic(&stub.url, 0, 1).call("m", request).await.unwrap();
    let body = body_of(&stub, 0);
    let system = body["system"].as_str().expect("system is a string");
    assert!(
        system.contains("RULE-ONE") && system.contains("RULE-TWO"),
        "{system}"
    );
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn sync_alternating_turns_stay_separate_messages() {
    let stub = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    let request = chat(vec![
        msg("user", "q1"),
        msg("assistant", "a1"),
        msg("assistant", "a1b"),
        msg("user", "q2"),
    ]);
    anthropic(&stub.url, 0, 1).call("m", request).await.unwrap();
    assert_eq!(
        body_of(&stub, 0)["messages"],
        json!([
            {"role": "user", "content": [{"type": "text", "text": "q1"}]},
            {"role": "assistant", "content": [
                {"type": "text", "text": "a1"}, {"type": "text", "text": "a1b"}
            ]},
            {"role": "user", "content": [{"type": "text", "text": "q2"}]}
        ])
    );
}

#[tokio::test]
async fn sync_cache_tokens_are_part_of_the_prompt_tokens() {
    let stub = serve(vec![json_ok(&message_body(
        "end_turn",
        json!([{"type": "text", "text": "ok"}]),
        json!({"input_tokens": 10, "output_tokens": 5,
            "cache_creation_input_tokens": 3, "cache_read_input_tokens": 100}),
    ))])
    .await;
    let response = anthropic(&stub.url, 0, 1)
        .call("m", request())
        .await
        .unwrap();
    assert_eq!(response.usage.prompt_tokens, 113);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.total_tokens, 118);
    assert!(!response.usage_unknown);
}

#[tokio::test]
async fn sync_stop_reasons_map_to_host_finish_reasons() {
    for (stop, expected) in [
        ("end_turn", "stop"),
        ("stop_sequence", "stop"),
        ("max_tokens", "length"),
        ("refusal", "content_filter"),
    ] {
        let stub = serve(vec![json_ok(&message_body(
            stop,
            json!([{"type": "text", "text": "x"}]),
            json!({"input_tokens": 1, "output_tokens": 1}),
        ))])
        .await;
        let response = anthropic(&stub.url, 0, 1)
            .call("m", request())
            .await
            .unwrap();
        assert_eq!(response.finish_reason.as_deref(), Some(expected), "{stop}");
    }
}

#[tokio::test]
async fn sync_pause_turn_and_unknown_stop_reasons_are_errors_without_retry() {
    for stop in ["pause_turn", "brand_new_reason"] {
        let stub = serve(vec![json_ok(&message_body(
            stop,
            json!([{"type": "text", "text": MARKER}]),
            json!({"input_tokens": 1, "output_tokens": 1}),
        ))])
        .await;
        let error = anthropic(&stub.url, 3, 1)
            .call("m", request())
            .await
            .expect_err("a stop reason the host has no meaning for is an error");
        assert_eq!(error.error_code(), 1000000, "{stop}: {error}");
        assert_clean(&error);
        assert_eq!(stub.hits(), 1, "{stop}: not retried");
    }
}

#[tokio::test]
async fn sync_missing_or_empty_usage_is_zero_and_flagged_unknown() {
    for usage in [None, Some(json!({}))] {
        let mut body = json!({"id": "msg_1", "type": "message", "role": "assistant",
            "content": [{"type": "text", "text": "ok"}], "stop_reason": "end_turn"});
        if let Some(usage) = usage.clone() {
            body["usage"] = usage;
        }
        let stub = serve(vec![json_ok(&body)]).await;
        let response = anthropic(&stub.url, 0, 1)
            .call("m", request())
            .await
            .unwrap();
        assert_eq!(response.content, "ok", "usage {usage:?}");
        assert_eq!(response.usage.prompt_tokens, 0, "usage {usage:?}");
        assert_eq!(response.usage.completion_tokens, 0, "usage {usage:?}");
        assert_eq!(response.usage.total_tokens, 0, "usage {usage:?}");
        assert!(response.usage_unknown, "usage {usage:?}");
    }
}

#[tokio::test]
async fn sync_response_without_a_content_array_is_invalid_and_not_retried() {
    let bodies = [
        json!({"id": "msg_1", "type": "message", "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1}}),
        json!({"id": "msg_1", "type": "message", "content": MARKER, "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1}}),
    ];
    for body in bodies {
        let stub = serve(vec![json_ok(&body)]).await;
        let error = anthropic(&stub.url, 3, 1)
            .call("m", request())
            .await
            .expect_err("no content array is not a message");
        assert_eq!(error.error_code(), 1001025, "{error}");
        assert_clean(&error);
        assert_eq!(stub.hits(), 1, "an invalid answer is never retried");
    }
    let stub = serve(vec![response(200, &[], &format!("<html>{MARKER}</html>"))]).await;
    let error = anthropic(&stub.url, 3, 1)
        .call("m", request())
        .await
        .expect_err("a non-JSON body is not a message");
    assert_eq!(error.error_code(), 1001025, "{error}");
    assert_clean(&error);
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn sync_text_blocks_are_joined_and_unknown_blocks_are_skipped() {
    let stub = serve(vec![json_ok(&message_body(
        "end_turn",
        json!([
            {"type": "text", "text": "Hello, "},
            {"type": "server_tool_use", "id": "srv_1", "name": "web_search", "input": {}},
            {"type": "text", "text": "world"}
        ]),
        json!({"input_tokens": 1, "output_tokens": 1}),
    ))])
    .await;
    let response = anthropic(&stub.url, 0, 1)
        .call("m", request())
        .await
        .unwrap();
    assert_eq!(response.content, "Hello, world");
}

#[tokio::test]
async fn sync_thinking_blocks_fill_reasoning_and_never_the_content() {
    let stub = serve(vec![json_ok(&message_body(
        "end_turn",
        json!([
            {"type": "thinking", "thinking": "step one. ", "signature": "sig-1"},
            {"type": "redacted_thinking", "data": "opaque"},
            {"type": "thinking", "thinking": "step two.", "signature": "sig-2"},
            {"type": "text", "text": "the answer"}
        ]),
        json!({"input_tokens": 1, "output_tokens": 1}),
    ))])
    .await;
    let response = anthropic(&stub.url, 0, 1)
        .call("m", request())
        .await
        .unwrap();
    assert_eq!(response.content, "the answer");
    assert_eq!(response.reasoning.as_deref(), Some("step one. step two."));
}

// ---------------------------------------------------------------------------------------------
// AC22: tool use mapping
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn tools_tool_use_block_becomes_a_function_call_with_compact_json_arguments() {
    let stub = serve(vec![tool_use_stop(json!([
        {"type": "tool_use", "id": "toolu_1", "name": "read_file",
         "input": {"path": "a.txt", "depth": 2}}
    ]))])
    .await;
    let result = anthropic(&stub.url, 0, 1)
        .call_tools("m", tool_request(vec![user_tool_message("list files")]))
        .await
        .unwrap();

    assert_eq!(result.message.role, "assistant");
    assert!(result.message.content.is_none());
    assert!(result.message.reasoning.is_none());
    assert_eq!(result.message.tool_calls.len(), 1);
    let call = &result.message.tool_calls[0];
    assert_eq!(call.id, "toolu_1");
    assert_eq!(call.kind, "function");
    assert_eq!(call.function.name, "read_file");
    assert_eq!(
        call.function.arguments,
        json!({"path": "a.txt", "depth": 2}).to_string()
    );
    assert!(
        !call.function.arguments.contains(' '),
        "{}",
        call.function.arguments
    );
    assert_eq!(
        serde_json::from_str::<Value>(&call.function.arguments).unwrap(),
        json!({"path": "a.txt", "depth": 2})
    );
    assert_eq!(result.usage.prompt_tokens, 10);
    assert_eq!(result.usage.total_tokens, 15);
}

#[tokio::test]
async fn tools_text_and_tool_use_are_both_kept() {
    let stub = serve(vec![tool_use_stop(json!([
        {"type": "text", "text": "Reading it now."},
        {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {"path": "a.txt"}}
    ]))])
    .await;
    let result = anthropic(&stub.url, 0, 1)
        .call_tools("m", tool_request(vec![user_tool_message("go")]))
        .await
        .unwrap();
    assert_eq!(result.message.content.as_deref(), Some("Reading it now."));
    assert_eq!(result.message.tool_calls.len(), 1);
    assert_eq!(result.message.tool_calls[0].id, "toolu_1");
}

#[tokio::test]
async fn tools_final_answer_has_text_and_no_calls() {
    let stub = serve(vec![json_ok(&message_body(
        "end_turn",
        json!([{"type": "text", "text": "All done."}]),
        json!({"input_tokens": 4, "output_tokens": 2}),
    ))])
    .await;
    let result = anthropic(&stub.url, 0, 1)
        .call_tools("m", tool_request(vec![user_tool_message("go")]))
        .await
        .unwrap();
    assert_eq!(result.message.content.as_deref(), Some("All done."));
    assert!(result.message.tool_calls.is_empty());
}

#[tokio::test]
async fn tools_tool_use_without_input_has_empty_object_arguments() {
    let stub = serve(vec![tool_use_stop(json!([
        {"type": "tool_use", "id": "toolu_1", "name": "read_file"}
    ]))])
    .await;
    let result = anthropic(&stub.url, 0, 1)
        .call_tools("m", tool_request(vec![user_tool_message("go")]))
        .await
        .unwrap();
    assert_eq!(result.message.tool_calls[0].function.arguments, "{}");
}

#[tokio::test]
async fn tools_request_golden_maps_history_tools_and_tool_choice() {
    let history = || {
        vec![
            user_tool_message("list files"),
            tool_message(
                "assistant",
                None,
                None,
                vec![call_of("c1", "read_file", r#"{"path":"a.txt"}"#)],
                None,
            ),
            tool_message("tool", Some("content of a"), Some("c1"), vec![], None),
        ]
    };
    let expected = json!({
        "model": "m",
        "max_tokens": 256,
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "list files"}]},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "c1", "name": "read_file",
                 "input": {"path": "a.txt"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "c1", "content": "content of a"}
            ]}
        ],
        "stream": false,
        "tools": [{"name": "read_file", "description": "Selected tool",
            "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}}],
        "tool_choice": {"type": "auto", "disable_parallel_tool_use": true}
    });
    let done = || {
        json_ok(&message_body(
            "end_turn",
            json!([{"type": "text", "text": "ok"}]),
            json!({"input_tokens": 1, "output_tokens": 1}),
        ))
    };

    let generic = serve(vec![done()]).await;
    anthropic(&generic.url, 0, 1)
        .call_tools("m", tool_request(history()))
        .await
        .unwrap();
    assert_eq!(body_of(&generic, 0), expected);

    // The official profile maps the same conversation the same way.
    let official = serve(vec![done()]).await;
    anthropic_official(&official.url, AdapterOptions::default())
        .call_tools("m", tool_request(history()))
        .await
        .unwrap();
    assert_eq!(body_of(&official, 0), expected);
}

#[tokio::test]
async fn tools_empty_call_arguments_are_sent_as_an_empty_input_object() {
    let stub = serve(vec![json_ok(&message_body(
        "end_turn",
        json!([{"type": "text", "text": "ok"}]),
        json!({"input_tokens": 1, "output_tokens": 1}),
    ))])
    .await;
    let messages = vec![
        user_tool_message("go"),
        tool_message(
            "assistant",
            None,
            None,
            vec![
                call_of("c1", "read_file", ""),
                call_of("c2", "read_file", "  "),
            ],
            None,
        ),
        tool_message("tool", Some("r1"), Some("c1"), vec![], None),
        tool_message("tool", Some("r2"), Some("c2"), vec![], None),
    ];
    anthropic(&stub.url, 0, 1)
        .call_tools("m", tool_request(messages))
        .await
        .unwrap();
    let body = body_of(&stub, 0);
    let assistant = &body["messages"][1]["content"];
    assert_eq!(assistant[0]["input"], json!({}));
    assert_eq!(assistant[1]["input"], json!({}));
    // Both results answer the one assistant turn, in one user message.
    let results = body["messages"][2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["tool_use_id"], "c1");
    assert_eq!(results[1]["tool_use_id"], "c2");
}

#[tokio::test]
async fn tools_tool_results_come_before_other_blocks_of_the_user_turn() {
    let stub = serve(vec![json_ok(&message_body(
        "end_turn",
        json!([{"type": "text", "text": "ok"}]),
        json!({"input_tokens": 1, "output_tokens": 1}),
    ))])
    .await;
    let messages = vec![
        user_tool_message("go"),
        tool_message(
            "assistant",
            Some("working"),
            None,
            vec![
                call_of("c1", "read_file", r#"{"path":"a"}"#),
                call_of("c2", "read_file", r#"{"path":"b"}"#),
            ],
            None,
        ),
        // A plain user note arrives before the results; both are user turns and merge.
        user_tool_message("note from the user"),
        tool_message("tool", Some("result a"), Some("c1"), vec![], None),
        tool_message("tool", Some("result b"), Some("c2"), vec![], None),
    ];
    anthropic(&stub.url, 0, 1)
        .call_tools("m", tool_request(messages))
        .await
        .unwrap();
    let body = body_of(&stub, 0);
    let turns = body["messages"].as_array().unwrap();
    assert_eq!(turns.len(), 3, "{body}");
    assert_eq!(turns[2]["role"], "user");
    assert_eq!(
        turns[2]["content"],
        json!([
            {"type": "tool_result", "tool_use_id": "c1", "content": "result a"},
            {"type": "tool_result", "tool_use_id": "c2", "content": "result b"},
            {"type": "text", "text": "note from the user"}
        ])
    );
    assert_eq!(
        turns[1]["content"][0],
        json!({"type": "text", "text": "working"})
    );
    assert_eq!(turns[1]["content"][1]["type"], "tool_use");
    assert_eq!(turns[1]["content"][2]["type"], "tool_use");
}

#[tokio::test]
async fn tools_thinking_blocks_come_first_and_unchanged_in_the_assistant_turn() {
    let blocks = vec![
        json!({"type": "thinking", "thinking": "I should read the file.",
            "signature": "EuYB/+=\u{e9}\\n\"q\"=="}),
        json!({"type": "redacted_thinking", "data": "EmwKAhgB-opaque"}),
    ];
    let reasoning = MessageReasoning {
        text: "I should read the file.".into(),
        fields: vec![],
        blocks: blocks.clone(),
    };
    let messages = vec![
        user_tool_message("go"),
        tool_message(
            "assistant",
            Some("Let me look."),
            None,
            vec![call_of("c1", "read_file", r#"{"path":"a.txt"}"#)],
            Some(reasoning),
        ),
        tool_message("tool", Some("content"), Some("c1"), vec![], None),
    ];
    let stub = serve(vec![json_ok(&message_body(
        "end_turn",
        json!([{"type": "text", "text": "ok"}]),
        json!({"input_tokens": 1, "output_tokens": 1}),
    ))])
    .await;
    anthropic(&stub.url, 0, 1)
        .call_tools("m", tool_request(messages))
        .await
        .unwrap();

    let body = body_of(&stub, 0);
    let assistant = body["messages"][1]["content"].as_array().unwrap();
    assert_eq!(assistant.len(), 4, "{body}");
    assert_eq!(assistant[0], blocks[0]);
    assert_eq!(assistant[1], blocks[1]);
    assert_eq!(
        assistant[2],
        json!({"type": "text", "text": "Let me look."})
    );
    assert_eq!(assistant[3]["type"], "tool_use");
    // Byte for byte on the wire, signature included.
    let raw = &stub.requests()[0].body;
    for block in &blocks {
        let text = serde_json::to_string(block).unwrap();
        assert!(
            raw.contains(&text),
            "block not verbatim on the wire: {text}\n{raw}"
        );
    }
}

#[tokio::test]
async fn tools_stop_reason_and_content_mismatch_is_an_error_without_a_call() {
    let tool_use = || json!({"type": "tool_use", "id": "t1", "name": "read_file", "input": {}});
    let text = || json!({"type": "text", "text": MARKER});
    let cases: Vec<(&str, Value)> = vec![
        ("tool_use", json!([text()])),
        ("tool_use", json!([])),
        ("end_turn", json!([text(), tool_use()])),
        ("max_tokens", json!([tool_use()])),
        ("max_tokens", json!([text()])),
        ("pause_turn", json!([text()])),
        ("refusal", json!([text()])),
    ];
    for (stop, content) in cases {
        let label = format!("{stop} {content}");
        let stub = serve(vec![json_ok(&message_body(
            stop,
            content,
            json!({"input_tokens": 1, "output_tokens": 1}),
        ))])
        .await;
        let result = anthropic(&stub.url, 3, 1)
            .call_tools("m", tool_request(vec![user_tool_message("go")]))
            .await;
        let error = match result {
            Err(error) => error,
            Ok(response) => panic!("{label}: expected an error, got {:?}", response.message),
        };
        assert_clean(&error);
        assert_eq!(stub.hits(), 1, "{label}: tool calls are never retried");
    }
}

#[tokio::test]
async fn tools_response_reasoning_blocks_are_kept_verbatim() {
    let blocks = vec![
        json!({"type": "thinking", "thinking": "first, ", "signature": "sig-A"}),
        json!({"type": "redacted_thinking", "data": "hidden-bytes"}),
        json!({"type": "thinking", "thinking": "second.", "signature": "sig-B"}),
    ];
    let mut content = blocks.clone();
    content.push(json!({"type": "text", "text": "Calling."}));
    content.push(json!({"type": "tool_use", "id": "t1", "name": "read_file",
        "input": {"path": "a"}}));
    let stub = serve(vec![tool_use_stop(Value::Array(content))]).await;
    let result = anthropic(&stub.url, 0, 1)
        .call_tools("m", tool_request(vec![user_tool_message("go")]))
        .await
        .unwrap();

    let reasoning = result.message.reasoning.expect("reasoning is kept");
    assert_eq!(reasoning.blocks, blocks);
    assert_eq!(reasoning.text, "first, second.");
    assert!(reasoning.fields.is_empty());
    assert_eq!(result.message.content.as_deref(), Some("Calling."));
    assert_eq!(result.message.tool_calls.len(), 1);
}

#[tokio::test]
async fn tools_reasoning_round_trip_replays_the_blocks_in_the_next_request() {
    let blocks = vec![
        json!({"type": "thinking", "thinking": "plan", "signature": "sig-Z"}),
        json!({"type": "redacted_thinking", "data": "hidden"}),
    ];
    let mut content = blocks.clone();
    content.push(json!({"type": "tool_use", "id": "t1", "name": "read_file",
        "input": {"path": "a"}}));
    let stub = serve(vec![
        tool_use_stop(Value::Array(content)),
        json_ok(&message_body(
            "end_turn",
            json!([{"type": "text", "text": "done"}]),
            json!({"input_tokens": 1, "output_tokens": 1}),
        )),
    ])
    .await;
    let adapter = anthropic(&stub.url, 0, 1);
    let first = adapter
        .call_tools("m", tool_request(vec![user_tool_message("go")]))
        .await
        .unwrap();
    let call_id = first.message.tool_calls[0].id.clone();
    let second = adapter
        .call_tools(
            "m",
            tool_request(vec![
                user_tool_message("go"),
                first.message,
                tool_message("tool", Some("file body"), Some(&call_id), vec![], None),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(second.message.content.as_deref(), Some("done"));

    let body = body_of(&stub, 1);
    assert_eq!(
        body["messages"][1]["content"],
        json!([blocks[0], blocks[1],
            {"type": "tool_use", "id": "t1", "name": "read_file", "input": {"path": "a"}}])
    );
    assert_eq!(
        body["messages"][2]["content"],
        json!([{"type": "tool_result", "tool_use_id": "t1", "content": "file body"}])
    );
}

#[tokio::test]
async fn tools_invalid_history_arguments_are_request_not_supported_without_a_request() {
    let stub = silent_stub().await;
    let adapter = anthropic(&stub.url, 3, 1);
    for arguments in ["not json", "{", "[1]", "\"text\"", "3", "null"] {
        let messages = vec![
            user_tool_message("go"),
            tool_message(
                "assistant",
                None,
                None,
                vec![call_of("c1", "read_file", arguments)],
                None,
            ),
            tool_message("tool", Some("r"), Some("c1"), vec![], None),
        ];
        let error = adapter
            .call_tools("m", tool_request(messages))
            .await
            .expect_err("arguments must be a JSON object");
        assert_eq!(error.error_code(), 1001020, "{arguments:?}: {error}");
        assert_clean(&error);
    }
    assert_eq!(stub.hits(), 0);
}

// ---------------------------------------------------------------------------------------------
// AC26: local refusals (the upstream must see 0 requests)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn rejects_system_message_that_is_not_first() {
    let stub = silent_stub().await;
    let adapter = anthropic(&stub.url, 3, 1);
    for messages in [
        vec![
            msg("user", "hi"),
            msg("system", "late rule"),
            msg("user", "again"),
        ],
        vec![
            msg("user", "hi"),
            msg("assistant", "a"),
            msg("system", "late"),
            msg("user", "q"),
        ],
    ] {
        rejected_both(&adapter, &stub, chat(messages)).await;
    }
}

#[tokio::test]
async fn rejects_conversation_ending_with_assistant() {
    let stub = silent_stub().await;
    let adapter = anthropic(&stub.url, 3, 1);
    rejected_both(
        &adapter,
        &stub,
        chat(vec![msg("user", "hi"), msg("assistant", "prefill")]),
    )
    .await;
}

#[tokio::test]
async fn rejects_first_message_assistant() {
    let stub = silent_stub().await;
    let adapter = anthropic(&stub.url, 3, 1);
    rejected_both(
        &adapter,
        &stub,
        chat(vec![msg("assistant", "hello"), msg("user", "hi")]),
    )
    .await;
    rejected_both(
        &adapter,
        &stub,
        chat(vec![
            msg("system", "rule"),
            msg("assistant", "hello"),
            msg("user", "hi"),
        ]),
    )
    .await;
}

#[tokio::test]
async fn rejects_no_messages_and_system_only_conversations() {
    let stub = silent_stub().await;
    let adapter = anthropic(&stub.url, 3, 1);
    rejected_both(&adapter, &stub, chat(vec![])).await;
    rejected_both(&adapter, &stub, chat(vec![msg("system", "only a rule")])).await;
}

#[tokio::test]
async fn rejects_tool_conversations_the_protocol_cannot_express() {
    let stub = silent_stub().await;
    let adapter = anthropic(&stub.url, 3, 1);
    let ends_with_assistant = vec![
        user_tool_message("go"),
        tool_message("assistant", Some("thinking out loud"), None, vec![], None),
    ];
    let starts_with_assistant = vec![
        tool_message("assistant", Some("hello"), None, vec![], None),
        user_tool_message("go"),
    ];
    let result_without_id = vec![
        user_tool_message("go"),
        tool_message("tool", Some("orphan result"), None, vec![], None),
    ];
    for messages in [
        ends_with_assistant,
        starts_with_assistant,
        result_without_id,
    ] {
        let error = adapter
            .call_tools("m", tool_request(messages))
            .await
            .expect_err("must be refused");
        assert_eq!(error.error_code(), 1001020, "{error}");
    }
    assert_eq!(stub.hits(), 0);
}

#[tokio::test]
async fn rejects_temperature_on_the_official_profile() {
    let stub = silent_stub().await;
    let adapter = anthropic_official(&stub.url, AdapterOptions::default());
    let request = ChatRequest {
        temperature: Some(0.7),
        ..chat(vec![msg("user", "hi")])
    };
    rejected_both(&adapter, &stub, request).await;
}

#[tokio::test]
async fn rejects_temperature_on_official_but_generic_profile_sends_the_same_request() {
    let request = || ChatRequest {
        temperature: Some(0.7),
        ..chat(vec![msg("user", "hi")])
    };
    let official = silent_stub().await;
    rejected_call(
        &anthropic_official(&official.url, AdapterOptions::default()),
        &official,
        request(),
    )
    .await;

    let generic = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    anthropic(&generic.url, 0, 1)
        .call("m", request())
        .await
        .expect("the generic profile leaves the decision to the upstream");
    assert_eq!(generic.hits(), 1);
    assert_eq!(body_of(&generic, 0)["temperature"], json!(0.7));
}

#[tokio::test]
async fn rejects_streaming_with_response_format() {
    let schema = json!({"type": "object", "properties": {"n": {"type": "integer"}}});
    let request = ChatRequest {
        response_format: Some(ResponseFormat {
            kind: "json_schema".into(),
            name: "answer".into(),
            schema,
            strict: None,
        }),
        ..chat(vec![msg("user", "hi")])
    };
    let generic = silent_stub().await;
    rejected_stream(&anthropic(&generic.url, 3, 1), &generic, request.clone()).await;
    let official = silent_stub().await;
    rejected_stream(
        &anthropic_official(&official.url, AdapterOptions::default()),
        &official,
        request,
    )
    .await;
}

fn with_reasoning(
    effort: Option<&str>,
    budget_tokens: Option<u32>,
    max_tokens: Option<u32>,
) -> ChatRequest {
    ChatRequest {
        reasoning: Some(ReasoningConfig {
            effort: effort.map(str::to_owned),
            budget_tokens,
        }),
        max_tokens,
        ..chat(vec![msg("user", "hi")])
    }
}

#[tokio::test]
async fn rejects_reasoning_levels_without_a_mapping_on_the_generic_profile() {
    let stub = silent_stub().await;
    let adapter = anthropic(&stub.url, 3, 1);
    // `xhigh` and friends only exist as adaptive efforts of the official profile; `none` has
    // neither an effort mapping nor a budget on the generic profile.
    for effort in ["xhigh", "max", "none", "minimal", "turbo", ""] {
        rejected_both(
            &adapter,
            &stub,
            with_reasoning(Some(effort), None, Some(32_000)),
        )
        .await;
    }
    rejected_both(&adapter, &stub, with_reasoning(None, None, Some(32_000))).await;
}

#[tokio::test]
async fn rejects_reasoning_levels_without_a_mapping_on_the_official_profile() {
    let stub = silent_stub().await;
    let adapter = anthropic_official(&stub.url, AdapterOptions::default());
    for effort in ["none", "minimal", "turbo", ""] {
        rejected_both(
            &adapter,
            &stub,
            with_reasoning(Some(effort), None, Some(32_000)),
        )
        .await;
    }
    rejected_both(&adapter, &stub, with_reasoning(None, None, Some(32_000))).await;
}

#[tokio::test]
async fn rejects_thinking_budget_that_leaves_no_room_for_the_answer() {
    let stub = silent_stub().await;
    let adapter = anthropic(&stub.url, 3, 1);
    let cases = [
        // (effort, budget_tokens, max_tokens)
        (None, Some(4096), Some(4096)),
        (None, Some(5000), Some(4096)),
        (None, Some(2048), Some(1000)),
        // `high` is a 16384 budget; the default max_tokens is 4096.
        (Some("high"), None, None),
        (Some("low"), None, Some(2048)),
    ];
    for (effort, budget, max_tokens) in cases {
        rejected_both(&adapter, &stub, with_reasoning(effort, budget, max_tokens)).await;
    }
}

#[tokio::test]
async fn thinking_budget_below_max_tokens_is_sent_on_the_generic_profile() {
    let stub = serve(vec![json_ok(&serde_json::from_str(&ok_message()).unwrap())]).await;
    let adapter = anthropic(&stub.url, 0, 1);
    adapter
        .call("m", with_reasoning(None, Some(1024), Some(2048)))
        .await
        .unwrap();
    adapter
        .call("m", with_reasoning(Some("low"), None, Some(4096)))
        .await
        .unwrap();
    assert_eq!(stub.hits(), 2);
    assert_eq!(
        body_of(&stub, 0)["thinking"],
        json!({"type": "enabled", "budget_tokens": 1024})
    );
    assert_eq!(
        body_of(&stub, 1)["thinking"],
        json!({"type": "enabled", "budget_tokens": 2048})
    );
}

fn with_format(kind: &str, name: &str, schema: Value) -> ChatRequest {
    ChatRequest {
        response_format: Some(ResponseFormat {
            kind: kind.into(),
            name: name.into(),
            schema,
            strict: None,
        }),
        ..chat(vec![msg("user", "hi")])
    }
}

#[tokio::test]
async fn rejects_response_format_with_a_bad_name_schema_or_kind() {
    let good = || json!({"type": "object", "properties": {"n": {"type": "integer"}}});
    let long_name = "n".repeat(65);
    let cases: Vec<(&str, ChatRequest)> = vec![
        ("empty name", with_format("json_schema", "", good())),
        (
            "space in name",
            with_format("json_schema", "has space", good()),
        ),
        ("slash in name", with_format("json_schema", "a/b", good())),
        ("long name", with_format("json_schema", &long_name, good())),
        ("wrong kind", with_format("text", "answer", good())),
        (
            "string schema",
            with_format("json_schema", "answer", json!("object")),
        ),
        (
            "array schema",
            with_format("json_schema", "answer", json!([1, 2])),
        ),
        (
            "null schema",
            with_format("json_schema", "answer", Value::Null),
        ),
        (
            "unknown type keyword",
            with_format("json_schema", "answer", json!({"type": "nonsense"})),
        ),
        (
            "malformed properties",
            with_format(
                "json_schema",
                "answer",
                json!({"type": "object", "properties": 5}),
            ),
        ),
    ];
    for official in [false, true] {
        let stub = silent_stub().await;
        let adapter = if official {
            anthropic_official(&stub.url, AdapterOptions::default())
        } else {
            anthropic(&stub.url, 3, 1)
        };
        for (label, request) in &cases {
            let error = adapter
                .call("m", request.clone())
                .await
                .expect_err(&format!("{label} (official={official}) must be refused"));
            assert_eq!(
                error.error_code(),
                1001020,
                "{label} official={official}: {error}"
            );
            assert_clean(&error);
        }
        assert_eq!(stub.hits(), 0, "official={official}");
    }
}
