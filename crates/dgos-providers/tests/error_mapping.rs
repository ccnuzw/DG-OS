//! AC29: twelve upstream outcomes map to the registered codes, for a synchronous call and for the
//! start of a stream, with retries off so the original code is visible. Mock HTTP.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;

const CONTEXT: &str = r#"{"error":{"message":"This model's maximum context length is 8192 tokens","code":"context_length_exceeded"}}"#;
const QUOTA: &str = r#"{"error":{"code":"insufficient_quota","message":"sk-leaked-secret"}}"#;
const LEAK: &str = r#"{"error":{"message":"sk-leaked-secret upstream detail","type":"x"}}"#;

fn cases() -> Vec<(u16, &'static str, u32, &'static str)> {
    vec![
        (401, LEAK, 1001002, "invalid_api_key"),
        (402, LEAK, 1001024, "provider_quota_exhausted"),
        (403, LEAK, 1001019, "provider_permission_denied"),
        (404, LEAK, 1001004, "model_not_found"),
        (400, LEAK, 1001017, "provider_request_rejected"),
        (400, CONTEXT, 1001018, "context_length_exceeded"),
        (413, LEAK, 1001017, "provider_request_rejected"),
        (429, LEAK, 1001005, "rate_limit_exceeded"),
        (429, QUOTA, 1001024, "provider_quota_exhausted"),
        (500, LEAK, 1001014, "upstream_server_error"),
        (504, LEAK, 1001006, "timeout"),
        (529, LEAK, 1001014, "upstream_server_error"),
    ]
}

fn assert_clean(error: &dgos_types::DgosError, label: &str) {
    let shown = format!("{error} | {error:?}");
    assert!(!shown.contains("sk-leaked-secret"), "{label}: {shown}");
    assert!(!shown.contains("upstream detail"), "{label}: {shown}");
    assert!(!shown.contains(API_KEY), "{label}: {shown}");
}

#[tokio::test]
async fn error_mapping_covers_twelve_outcomes_for_synchronous_calls() {
    for (status, body, code, name) in cases() {
        let stub = serve(vec![response(status, &[], body)]).await;
        let error = adapter(&stub.url, 0, 1)
            .call("the-model", request())
            .await
            .unwrap_err();
        let label = format!("{status} {name}");
        assert_eq!(error.error_code(), code, "{label}");
        assert_clean(&error, &label);
        assert_eq!(stub.hits(), 1, "{label}");
    }
}

#[tokio::test]
async fn error_mapping_covers_twelve_outcomes_for_stream_start() {
    for (status, body, code, name) in cases() {
        let stub = serve(vec![response(status, &[], body)]).await;
        let error = adapter(&stub.url, 0, 1)
            .stream("the-model", request())
            .await
            .err()
            .unwrap();
        let label = format!("{status} {name}");
        assert_eq!(error.error_code(), code, "{label}");
        assert_clean(&error, &label);
    }
}

#[tokio::test]
async fn error_mapping_names_only_the_callers_own_model_for_404() {
    let stub = serve(vec![response(404, &[], LEAK)]).await;
    let error = adapter(&stub.url, 0, 1)
        .call("the-model", request())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Model not found: the-model");
}

// ---------------------------------------------------------------------------------------------
// Anthropic Messages twin of the table above. The upstream marker sits in `message`.
// ---------------------------------------------------------------------------------------------

const MARKER: &str = "upstream-marker-anthropic-77";

fn anthropic_error(kind: &str) -> String {
    serde_json::json!({"type": "error", "error": {"type": kind, "message": MARKER}}).to_string()
}

fn anthropic_context() -> String {
    serde_json::json!({"type": "error", "error": {"type": "invalid_request_error",
        "message": format!("prompt is too long: 250000 tokens > 200000 maximum {MARKER}")}})
    .to_string()
}

fn anthropic_cases() -> Vec<(u16, String, u32, &'static str)> {
    vec![
        (
            401,
            anthropic_error("authentication_error"),
            1001002,
            "invalid_api_key",
        ),
        (
            402,
            anthropic_error("billing_error"),
            1001024,
            "provider_quota_exhausted",
        ),
        (
            403,
            anthropic_error("permission_error"),
            1001019,
            "provider_permission_denied",
        ),
        (
            404,
            anthropic_error("not_found_error"),
            1001004,
            "model_not_found",
        ),
        (
            400,
            anthropic_error("invalid_request_error"),
            1001017,
            "provider_request_rejected",
        ),
        (400, anthropic_context(), 1001018, "context_length_exceeded"),
        (
            413,
            anthropic_error("request_too_large"),
            1001017,
            "provider_request_rejected",
        ),
        (
            429,
            anthropic_error("rate_limit_error"),
            1001005,
            "rate_limit_exceeded",
        ),
        (
            429,
            anthropic_error("enforced_spend_limit_reached"),
            1001024,
            "provider_quota_exhausted",
        ),
        (
            500,
            anthropic_error("api_error"),
            1001014,
            "upstream_server_error",
        ),
        (504, anthropic_error("timeout_error"), 1001006, "timeout"),
        (
            529,
            anthropic_error("overloaded_error"),
            1001014,
            "upstream_server_error",
        ),
    ]
}

fn assert_anthropic_clean(error: &dgos_types::DgosError, label: &str) {
    let shown = format!("{error} | {error:?}");
    assert!(!shown.contains(MARKER), "{label}: {shown}");
    assert!(!shown.contains(API_KEY), "{label}: {shown}");
}

#[tokio::test]
async fn anthropic_error_mapping_covers_twelve_outcomes_for_synchronous_calls() {
    for (status, body, code, name) in anthropic_cases() {
        let stub = serve(vec![response(status, &[], &body)]).await;
        let error = anthropic(&stub.url, 0, 1)
            .call("the-model", request())
            .await
            .unwrap_err();
        let label = format!("{status} {name}");
        assert_eq!(error.error_code(), code, "{label}");
        assert_anthropic_clean(&error, &label);
        assert_eq!(stub.hits(), 1, "{label}");
    }
}

#[tokio::test]
async fn anthropic_error_mapping_covers_twelve_outcomes_for_stream_start() {
    for (status, body, code, name) in anthropic_cases() {
        let stub = serve(vec![response(status, &[], &body)]).await;
        let error = anthropic(&stub.url, 0, 1)
            .stream("the-model", request())
            .await
            .err()
            .unwrap();
        let label = format!("{status} {name}");
        assert_eq!(error.error_code(), code, "{label}");
        assert_anthropic_clean(&error, &label);
        assert_eq!(stub.hits(), 1, "{label}");
    }
}

#[tokio::test]
async fn anthropic_error_mapping_holds_under_the_official_profile_too() {
    for (status, body, code, name) in anthropic_cases() {
        let label = format!("official {status} {name}");
        let stub = serve(vec![response(status, &[], &body)]).await;
        let error = anthropic_official(&stub.url, Default::default())
            .call("the-model", request())
            .await
            .unwrap_err();
        assert_eq!(error.error_code(), code, "{label}");
        assert_anthropic_clean(&error, &label);

        let stub = serve(vec![response(status, &[], &body)]).await;
        let error = anthropic_official(&stub.url, Default::default())
            .stream("the-model", request())
            .await
            .err()
            .unwrap();
        assert_eq!(error.error_code(), code, "{label} stream");
        assert_anthropic_clean(&error, &label);
    }
}

#[tokio::test]
async fn anthropic_non_retryable_statuses_are_not_retried_even_with_retries_on() {
    let non_retryable: [u16; 7] = [401, 402, 403, 404, 400, 413, 422];
    for status in non_retryable {
        let stub = serve(vec![
            response(status, &[], &anthropic_error("invalid_request_error")),
            response(200, &[], &ok_message()),
        ])
        .await;
        let error = anthropic(&stub.url, 3, 1)
            .call("the-model", request())
            .await
            .unwrap_err();
        assert_eq!(stub.hits(), 1, "status {status} must not be retried");
        assert_anthropic_clean(&error, &status.to_string());
    }
}

#[tokio::test]
async fn anthropic_error_mapping_names_only_the_callers_own_model_for_404() {
    let stub = serve(vec![response(
        404,
        &[],
        &anthropic_error("not_found_error"),
    )])
    .await;
    let error = anthropic(&stub.url, 0, 1)
        .call("the-model", request())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Model not found: the-model");

    let stub = serve(vec![response(
        404,
        &[],
        &anthropic_error("not_found_error"),
    )])
    .await;
    let error = anthropic(&stub.url, 0, 1)
        .stream("the-model", request())
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "Model not found: the-model");
}

#[tokio::test]
async fn anthropic_error_bodies_that_are_not_json_still_map_by_status() {
    for (status, code) in [
        (401, 1001002),
        (429, 1001005),
        (500, 1001014),
        (400, 1001017),
    ] {
        let body = format!("<html>{MARKER}</html>");
        let stub = serve(vec![response(status, &[], &body)]).await;
        let error = anthropic(&stub.url, 0, 1)
            .call("the-model", request())
            .await
            .unwrap_err();
        assert_eq!(error.error_code(), code, "status {status}");
        assert_anthropic_clean(&error, &status.to_string());
    }
}
