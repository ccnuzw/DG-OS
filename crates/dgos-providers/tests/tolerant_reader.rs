//! AC28: lenient reading of synchronous responses and model lists. Mock HTTP.
mod common;

use common::*;
use dgos_providers::{AdapterOptions, ProviderAdapter};
use dgos_types::DgosError;
use serde_json::json;

async fn call_with_body(body: &str) -> (Result<dgos_types::ChatResponse, DgosError>, usize) {
    let stub = serve(vec![response(200, &[], body)]).await;
    let result = adapter(&stub.url, 3, 1).call("m", request()).await;
    (result, stub.hits())
}

#[tokio::test]
async fn missing_usage_empty_choices_unknown_fields_and_finish_reason_are_accepted() {
    let body = json!({
        "id": "x", "object": "chat.completion", "vendor_extension": {"a": 1},
        "choices": [],
    })
    .to_string();
    let (result, hits) = call_with_body(&body).await;
    let reply = result.unwrap();
    assert_eq!(reply.content, "");
    assert_eq!(reply.usage.total_tokens, 0);
    assert!(reply.usage_unknown);
    assert_eq!(hits, 1, "tolerated shapes are never retried");

    let body = json!({
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi", "extra": true},
            "finish_reason": "banana", "logprobs": null}],
        "system_fingerprint": "fp",
    })
    .to_string();
    let (result, hits) = call_with_body(&body).await;
    let reply = result.unwrap();
    assert_eq!(reply.content, "hi");
    assert_eq!(
        reply.finish_reason.as_deref(),
        Some("stop"),
        "unknown reason counts as stop"
    );
    assert!(reply.usage_unknown);
    assert_eq!(hits, 1);
}

#[tokio::test]
async fn finish_reasons_and_null_content_are_normalised() {
    for (raw, expected) in [
        ("stop", "stop"),
        ("length", "length"),
        ("tool_calls", "tool_calls"),
        ("content_filter", "content_filter"),
        ("function_call", "tool_calls"),
        ("something_new", "stop"),
    ] {
        let body =
            json!({"choices":[{"message":{"content":null},"finish_reason":raw}]}).to_string();
        let (result, _) = call_with_body(&body).await;
        let reply = result.unwrap();
        assert_eq!(reply.finish_reason.as_deref(), Some(expected), "{raw}");
        assert_eq!(reply.content, "");
    }
    let body = json!({"choices":[{"message":{"content":"x"},"finish_reason":null}]}).to_string();
    assert_eq!(call_with_body(&body).await.0.unwrap().finish_reason, None);
}

#[tokio::test]
async fn partial_usage_keeps_what_was_sent_and_computes_a_missing_total() {
    let body = json!({"choices":[{"message":{"content":"x"}}],
        "usage":{"prompt_tokens":4,"completion_tokens":6}})
    .to_string();
    let reply = call_with_body(&body).await.0.unwrap();
    assert_eq!(
        (
            reply.usage.prompt_tokens,
            reply.usage.completion_tokens,
            reply.usage.total_tokens
        ),
        (4, 6, 10)
    );
    assert!(!reply.usage_unknown);
}

#[tokio::test]
async fn malformed_bodies_are_provider_response_invalid_and_tried_exactly_once() {
    for body in [
        "<html>Not an API</html>",
        "",
        "{\"choices\": \"nope\"}",
        "{}",
        "[]",
        "{\"data\":[]}",
    ] {
        let (result, hits) = call_with_body(body).await;
        let error = result.unwrap_err();
        assert!(
            matches!(error, DgosError::ProviderResponseInvalid),
            "{body:?}"
        );
        assert_eq!(error.error_code(), 1001025);
        assert_eq!(hits, 1, "{body:?} must not be retried");
    }
}

#[tokio::test]
async fn a_body_over_the_size_limit_is_invalid_and_not_retried() {
    let options = AdapterOptions {
        max_body_bytes: 512,
        ..Default::default()
    };
    let padding = "x".repeat(2000);
    let body = json!({"choices":[{"message":{"content":padding}}]}).to_string();
    let stub = serve(vec![response(200, &[], &body)]).await;
    let error = adapter_with(&stub.url, 3, 1, options)
        .call("m", request())
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::ProviderResponseInvalid));
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn a_chunked_body_without_content_length_is_bounded_too() {
    let options = AdapterOptions {
        max_body_bytes: 512,
        ..Default::default()
    };
    let stub = serve(vec![vec![
        text("HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n"),
        text(&"y".repeat(400)),
        sleep(20),
        text(&"y".repeat(400)),
    ]])
    .await;
    let error = adapter_with(&stub.url, 3, 1, options)
        .call("m", request())
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::ProviderResponseInvalid));
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn model_lists_keep_only_valid_unique_ids_and_ignore_unknown_fields() {
    let long = "m".repeat(256);
    let ok_max = "n".repeat(255);
    let body = json!({"object":"list","data":[
        {"id":"gpt-4o","owned_by":"x","created":1},
        {"id":"gpt-4o","owned_by":"duplicate"},
        {"owned_by":"no id"},
        {"id":null},
        {"id":42},
        {"id":""},
        {"id":"   "},
        {"id": long},
        {"id": ok_max},
        "not an object",
        {"id":"text-embedding-3-small"},
    ]})
    .to_string();
    let stub = serve(vec![response(200, &[], &body)]).await;
    let models = adapter(&stub.url, 3, 1).list_models().await.unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.model_id.as_str()).collect();
    assert_eq!(ids, ["gpt-4o", ok_max.as_str(), "text-embedding-3-small"]);
    assert_eq!(stub.hits(), 1);
    // The first duplicate wins and every row is fully host-derived.
    assert!(models
        .iter()
        .all(|m| m.source == dgos_types::ModelSource::Remote));
    assert!(models
        .iter()
        .all(|m| !m.id.is_empty() && m.provider_id.is_empty()));
}

#[tokio::test]
async fn model_lists_are_truncated_at_the_configured_limit() {
    let data: Vec<_> = (0..50)
        .map(|i| json!({"id": format!("model-{i}")}))
        .collect();
    let stub = serve(vec![response(200, &[], &json!({"data": data}).to_string())]).await;
    let options = AdapterOptions {
        max_models: 10,
        ..Default::default()
    };
    let models = adapter_with(&stub.url, 3, 1, options)
        .list_models()
        .await
        .unwrap();
    assert_eq!(models.len(), 10);
    assert_eq!(models[0].model_id, "model-0");
    assert_eq!(models[9].model_id, "model-9");
}

#[tokio::test]
async fn model_lists_that_are_not_a_data_array_are_invalid() {
    for body in ["<html></html>", "{}", "{\"data\":\"x\"}", "[]"] {
        let stub = serve(vec![response(200, &[], body)]).await;
        let error = adapter(&stub.url, 3, 1).list_models().await.unwrap_err();
        assert!(
            matches!(error, DgosError::ProviderResponseInvalid),
            "{body}"
        );
        assert_eq!(stub.hits(), 1);
    }
}

#[tokio::test]
async fn list_models_saved_kinds_come_from_the_rules_and_streaming_follows_the_kind() {
    let body = json!({"data":[{"id":"gpt-4o"},{"id":"text-embedding-3-small"},{"id":"tts-1"}]})
        .to_string();
    let stub = serve(vec![response(200, &[], &body)]).await;
    let models = adapter(&stub.url, 3, 1).list_models().await.unwrap();
    assert_eq!(models[0].model_type, "text-generation");
    assert!(models[0].supports_streaming);
    assert_eq!(models[1].model_type, "embedding");
    assert!(!models[1].supports_streaming);
    assert_eq!(models[2].model_type, "speech-synthesis");
    assert!(models.iter().all(|m| m.max_output_tokens.is_none()));
}

// ---------------------------------------------------------------------------------------------
// Anthropic Messages (AC28)
// ---------------------------------------------------------------------------------------------

async fn anthropic_call(body: &str) -> (Result<dgos_types::ChatResponse, DgosError>, usize) {
    let stub = serve(vec![response(200, &[], body)]).await;
    let result = anthropic(&stub.url, 3, 1).call("m", request()).await;
    (result, stub.hits())
}

fn list_page(ids: &[&str], has_more: bool, last_id: Option<&str>) -> String {
    let data: Vec<_> = ids
        .iter()
        .map(|id| json!({"id": id, "type": "model"}))
        .collect();
    json!({"data": data, "has_more": has_more, "first_id": ids.first(), "last_id": last_id})
        .to_string()
}

#[tokio::test]
async fn anthropic_missing_or_empty_usage_is_zero_and_flagged_unknown_without_a_retry() {
    let bodies = [
        json!({"content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn"}),
        json!({"content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{}}),
        json!({"content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":null}),
        json!({"content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":"many"}),
    ];
    for body in bodies {
        let (result, hits) = anthropic_call(&body.to_string()).await;
        let reply = result.unwrap();
        assert_eq!(reply.content, "hi", "{body}");
        assert_eq!(reply.usage.prompt_tokens, 0);
        assert_eq!(reply.usage.completion_tokens, 0);
        assert_eq!(reply.usage.total_tokens, 0);
        assert!(reply.usage_unknown, "{body}");
        assert_eq!(reply.finish_reason.as_deref(), Some("stop"));
        assert_eq!(hits, 1, "a tolerated shape is never retried");
    }
}

#[tokio::test]
async fn anthropic_usage_sums_the_three_input_counters_and_keeps_a_lone_output_counter() {
    let body = json!({"content":[{"type":"text","text":"x"}],"stop_reason":"end_turn",
        "usage":{"input_tokens":10,"cache_creation_input_tokens":5,"cache_read_input_tokens":2,
                 "output_tokens":7}})
    .to_string();
    let reply = anthropic_call(&body).await.0.unwrap();
    assert_eq!(reply.usage.prompt_tokens, 17);
    assert_eq!(reply.usage.completion_tokens, 7);
    assert_eq!(reply.usage.total_tokens, 24);
    assert!(!reply.usage_unknown);

    let body =
        json!({"content":[{"type":"text","text":"x"}],"usage":{"output_tokens":4}}).to_string();
    let reply = anthropic_call(&body).await.0.unwrap();
    assert_eq!(reply.usage.prompt_tokens, 0);
    assert_eq!(reply.usage.completion_tokens, 4);
    assert_eq!(reply.usage.total_tokens, 4);
    assert!(!reply.usage_unknown, "a partial usage keeps what was sent");
}

#[tokio::test]
async fn anthropic_empty_content_array_is_an_empty_string_not_an_error() {
    let body = json!({"content":[],"stop_reason":"end_turn",
        "usage":{"input_tokens":3,"output_tokens":0}})
    .to_string();
    let (result, hits) = anthropic_call(&body).await;
    let reply = result.unwrap();
    assert_eq!(reply.content, "");
    assert_eq!(reply.usage.prompt_tokens, 3);
    assert!(reply.reasoning.is_none());
    assert_eq!(hits, 1);
}

#[tokio::test]
async fn anthropic_unknown_block_types_and_top_level_fields_are_ignored() {
    let body = json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "m",
        "container": {"id": "c1"}, "vendor_extension": [1, 2, 3], "stop_sequence": null,
        "content": [
            {"type": "server_tool_use", "id": "s1", "name": "web_search", "input": {}},
            {"type": "text", "text": "Hello ", "citations": [{"x": 1}]},
            {"type": "web_search_tool_result", "tool_use_id": "s1", "content": []},
            {"type": "text", "text": "world"},
            {"no_type_at_all": true},
            {"type": "text", "text": 7},
            "not even an object",
        ],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 1, "output_tokens": 2, "service_tier": "standard",
                  "server_tool_use": {"web_search_requests": 1}},
    })
    .to_string();
    let (result, hits) = anthropic_call(&body).await;
    let reply = result.unwrap();
    assert_eq!(reply.content, "Hello world");
    assert_eq!(reply.usage.total_tokens, 3);
    assert_eq!(hits, 1);
}

#[tokio::test]
async fn anthropic_known_stop_reasons_map_and_a_missing_one_counts_as_stop() {
    for (raw, expected) in [
        (Some("end_turn"), "stop"),
        (Some("stop_sequence"), "stop"),
        (Some("max_tokens"), "length"),
        (Some("model_context_window_exceeded"), "length"),
        (Some("tool_use"), "tool_calls"),
        (Some("refusal"), "content_filter"),
        (None, "stop"),
    ] {
        let mut body = json!({"content":[{"type":"text","text":"x"}],
            "usage":{"input_tokens":1,"output_tokens":1}});
        if let Some(raw) = raw {
            body["stop_reason"] = json!(raw);
        }
        let (result, hits) = anthropic_call(&body.to_string()).await;
        let reply = result.unwrap();
        assert_eq!(reply.finish_reason.as_deref(), Some(expected), "{raw:?}");
        assert_eq!(hits, 1, "{raw:?}");
    }
}

/// Documented behaviour: a stop reason the host has no meaning for (`pause_turn`, anything new)
/// is an error, not `stop`; it is `InternalError` (code 1000000), never retried.
#[tokio::test]
async fn anthropic_pause_turn_and_unknown_stop_reasons_are_an_error_and_not_retried() {
    for raw in ["pause_turn", "banana"] {
        let body = json!({"content":[{"type":"text","text":"x"}],"stop_reason":raw,
            "usage":{"input_tokens":1,"output_tokens":1}})
        .to_string();
        let (result, hits) = anthropic_call(&body).await;
        let error = result.unwrap_err();
        assert!(
            matches!(error, DgosError::InternalError(_)),
            "{raw}: {error:?}"
        );
        assert_eq!(error.error_code(), 1000000, "{raw}");
        assert_eq!(hits, 1, "{raw}: not retried");
    }
}

#[tokio::test]
async fn anthropic_malformed_bodies_are_provider_response_invalid_and_tried_exactly_once() {
    let bodies = [
        "",
        "<html><body>Bad gateway page</body></html>",
        "not json at all",
        "null",
        "[]",
        "{}",
        r#"{"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#,
        r#"{"content":null}"#,
        r#"{"content":"hello"}"#,
        r#"{"content":{"type":"text","text":"hello"}}"#,
        r#"{"content":[{"type":"tool_use","name":"t","input":{}}]}"#,
        r#"{"content":[{"type":"text","text":"cut off"#,
    ];
    for body in bodies {
        let (result, hits) = anthropic_call(body).await;
        let error = result.unwrap_err();
        assert!(
            matches!(error, DgosError::ProviderResponseInvalid),
            "{body:?}: {error:?}"
        );
        assert_eq!(error.error_code(), 1001025, "{body:?}");
        assert_eq!(hits, 1, "{body:?}: invalid bodies are not retried");
    }
}

#[tokio::test]
async fn anthropic_a_body_over_the_size_limit_is_invalid_and_not_retried() {
    let options = AdapterOptions {
        max_body_bytes: 512,
        ..Default::default()
    };
    let body = json!({"content":[{"type":"text","text":"x".repeat(2000)}],
        "stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}})
    .to_string();
    let stub = serve(vec![response(200, &[], &body)]).await;
    let error = anthropic_with(&stub.url, 3, 1, options.clone())
        .call("m", request())
        .await
        .unwrap_err();
    assert!(
        matches!(error, DgosError::ProviderResponseInvalid),
        "{error:?}"
    );
    assert_eq!(stub.hits(), 1);

    // The limit is enforced while reading, also without a content-length.
    let stub = serve(vec![vec![
        text("HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n"),
        text(&"y".repeat(400)),
        sleep(20),
        text(&"y".repeat(400)),
    ]])
    .await;
    let error = anthropic_with(&stub.url, 3, 1, options.clone())
        .call("m", request())
        .await
        .unwrap_err();
    assert!(
        matches!(error, DgosError::ProviderResponseInvalid),
        "{error:?}"
    );
    assert_eq!(stub.hits(), 1);

    // A model list over the limit is invalid too.
    let page = list_page(&["a".repeat(200).as_str(); 10], false, None);
    let stub = serve(vec![response(200, &[], &page)]).await;
    let error = anthropic_with(&stub.url, 3, 1, options)
        .list_models()
        .await
        .unwrap_err();
    assert!(
        matches!(error, DgosError::ProviderResponseInvalid),
        "{error:?}"
    );
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn anthropic_model_lists_keep_only_valid_unique_ids_and_ignore_unknown_fields() {
    let long = "m".repeat(256);
    let ok_max = "n".repeat(255);
    let body = json!({"data":[
        {"id":"claude-a","type":"model","display_name":"A","created_at":"2026-01-01T00:00:00Z",
         "vendor_field":{"x":1}},
        {"id":"claude-a","display_name":"duplicate"},
        {"display_name":"no id"},
        {"id":null},
        {"id":42},
        {"id":["claude-array"]},
        {"id":""},
        {"id":"   "},
        {"id": long},
        {"id": ok_max},
        "not an object",
        null,
        {"id":"claude-b"},
    ],"has_more":false,"first_id":"claude-a","last_id":"claude-b","extra":true})
    .to_string();
    let stub = serve(vec![response(200, &[], &body)]).await;
    let models = anthropic(&stub.url, 3, 1).list_models().await.unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.model_id.as_str()).collect();
    assert_eq!(ids, ["claude-a", ok_max.as_str(), "claude-b"]);
    assert_eq!(stub.hits(), 1);
    // The first duplicate wins.
    assert_eq!(models[0].model_name, "A");
    assert!(models
        .iter()
        .all(|m| m.source == dgos_types::ModelSource::Remote));
}

#[tokio::test]
async fn anthropic_model_lists_are_truncated_at_the_configured_limit() {
    let ids: Vec<String> = (0..50).map(|i| format!("model-{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let stub = serve(vec![response(200, &[], &list_page(&refs, false, None))]).await;
    let options = AdapterOptions {
        max_models: 10,
        ..Default::default()
    };
    let models = anthropic_with(&stub.url, 3, 1, options)
        .list_models()
        .await
        .unwrap();
    assert_eq!(models.len(), 10);
    assert_eq!(models[0].model_id, "model-0");
    assert_eq!(models[9].model_id, "model-9");
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn anthropic_model_list_limit_applies_across_pages_and_stops_fetching() {
    let stub = serve(vec![
        response(200, &[], &list_page(&["m1", "m2", "m3"], true, Some("m3"))),
        response(200, &[], &list_page(&["m4", "m5", "m6"], true, Some("m6"))),
        response(200, &[], &list_page(&["m7", "m8", "m9"], false, Some("m9"))),
    ])
    .await;
    let options = AdapterOptions {
        max_models: 5,
        ..Default::default()
    };
    let models = anthropic_with(&stub.url, 3, 1, options)
        .list_models()
        .await
        .unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.model_id.as_str()).collect();
    assert_eq!(ids, ["m1", "m2", "m3", "m4", "m5"]);
    assert_eq!(stub.hits(), 2, "the third page is never requested");
    let requests = stub.requests();
    assert!(!requests[0].head.contains("after_id"));
    assert!(requests[1]
        .head
        .lines()
        .next()
        .unwrap()
        .contains("after_id=m3"));
}

#[tokio::test]
async fn anthropic_pages_are_followed_with_after_id_and_duplicates_across_pages_dropped() {
    let stub = serve(vec![
        response(200, &[], &list_page(&["m1", "m2"], true, Some("m2"))),
        response(200, &[], &list_page(&["m2", "m3"], false, Some("m3"))),
    ])
    .await;
    let models = anthropic(&stub.url, 3, 1).list_models().await.unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.model_id.as_str()).collect();
    assert_eq!(ids, ["m1", "m2", "m3"]);
    assert_eq!(stub.hits(), 2);
}

#[tokio::test]
async fn anthropic_a_repeated_cursor_ends_the_listing_instead_of_looping() {
    let stub = serve(vec![
        response(200, &[], &list_page(&["m1"], true, Some("m1"))),
        response(200, &[], &list_page(&["m2"], true, Some("m1"))),
        response(200, &[], &list_page(&["m3"], true, Some("m3"))),
    ])
    .await;
    let models = anthropic(&stub.url, 3, 1).list_models().await.unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.model_id.as_str()).collect();
    assert_eq!(ids, ["m1", "m2"]);
    assert_eq!(stub.hits(), 2);
}

#[tokio::test]
async fn anthropic_model_lists_that_are_not_a_data_array_are_invalid_after_one_request() {
    for body in [
        "<html></html>",
        "",
        "{}",
        r#"{"data":"x"}"#,
        r#"{"data":null}"#,
        "[]",
        "null",
    ] {
        let stub = serve(vec![response(200, &[], body)]).await;
        let error = anthropic(&stub.url, 3, 1).list_models().await.unwrap_err();
        assert!(
            matches!(error, DgosError::ProviderResponseInvalid),
            "{body:?}: {error:?}"
        );
        assert_eq!(error.error_code(), 1001025, "{body:?}");
        assert_eq!(stub.hits(), 1, "{body:?}");
    }
}

#[tokio::test]
async fn anthropic_an_invalid_or_missing_later_page_makes_the_whole_list_invalid() {
    let first = list_page(&["m1"], true, Some("m1"));
    for second in [
        response(200, &[], "<html></html>"),
        response(200, &[], "{}"),
        response(404, &[], ""),
    ] {
        let stub = serve(vec![response(200, &[], &first), second]).await;
        let error = anthropic(&stub.url, 3, 1).list_models().await.unwrap_err();
        assert!(
            matches!(error, DgosError::ProviderResponseInvalid),
            "{error:?}"
        );
        assert_eq!(stub.hits(), 2);
    }
}
