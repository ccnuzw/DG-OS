//! AC30 at adapter level: which upstream outcomes are retried and what the caller finally sees.
//! The policy-level cases are in `src/retry.rs`. Scripted local server.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::DgosError;
use std::time::Instant;

fn error_body() -> &'static str {
    r#"{"error":{"message":"nope"}}"#
}

#[tokio::test]
async fn retry_classification_429_waits_for_the_smaller_of_retry_after_and_max_delay() {
    let stub = serve(vec![
        response(429, &[("retry-after", "30")], error_body()),
        response(200, &[], &ok_chat()),
    ])
    .await;
    // max_delay is 40 ms here, so the 30 s the upstream asked for must be capped.
    let adapter = adapter(&stub.url, 3, 10);
    let started = Instant::now();
    let reply = adapter.call("m", request()).await.unwrap();
    assert_eq!(reply.content, "ok");
    assert_eq!(stub.hits(), 2);
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}

#[tokio::test]
async fn retry_classification_503_is_retried_until_it_succeeds() {
    let stub = serve(vec![
        response(503, &[], error_body()),
        response(503, &[], error_body()),
        response(200, &[], &ok_chat()),
    ])
    .await;
    assert!(adapter(&stub.url, 3, 1).call("m", request()).await.is_ok());
    assert_eq!(stub.hits(), 3);
}

#[tokio::test]
async fn retry_classification_does_not_retry_400_or_exhausted_quota() {
    let stub = serve(vec![response(400, &[], error_body())]).await;
    let error = adapter(&stub.url, 3, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);

    let quota = r#"{"error":{"code":"insufficient_quota"}}"#;
    let stub = serve(vec![response(429, &[("retry-after", "1")], quota)]).await;
    let error = adapter(&stub.url, 3, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001024);
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn retry_classification_reports_max_retries_only_after_a_retry_also_failed() {
    let stub = serve(vec![response(503, &[], error_body())]).await;
    let error = adapter(&stub.url, 2, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::MaxRetriesExceeded));
    assert_eq!(stub.hits(), 3, "one call and two retries");

    let stub = serve(vec![response(503, &[], error_body())]).await;
    let error = adapter(&stub.url, 0, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(
        error.error_code(),
        1001014,
        "no retry configured: original code"
    );
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn retry_classification_retries_a_stream_that_fails_before_any_event() {
    let stub = serve(vec![
        response(503, &[], error_body()),
        sse_script(&[delta("hi"), finish("stop"), DONE.into()]),
    ])
    .await;
    let items = drain(
        adapter(&stub.url, 3, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "hi");
    assert_eq!(stub.hits(), 2);
}

#[tokio::test]
async fn retry_classification_never_retries_or_replays_a_stream_that_already_produced_output() {
    let stub = serve(vec![
        sse_script(&[delta("partial")]),
        sse_script(&[delta("replayed"), finish("stop"), DONE.into()]),
    ])
    .await;
    let items = drain(
        adapter(&stub.url, 3, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "partial");
    assert!(matches!(
        items.last(),
        Some(Err(DgosError::StreamInterrupted))
    ));
    assert_eq!(stub.hits(), 1);
}

// ---------------------------------------------------------------------------------------------
// Anthropic twins (generic profile). Same policy, Anthropic error bodies.
// ---------------------------------------------------------------------------------------------

/// Distinctive text in upstream error bodies; it must never reach an error message.
const MARKER: &str = "UPSTREAM-MARKER-77c0";

fn anthropic_error(kind: &str) -> String {
    format!(r#"{{"type":"error","error":{{"type":"{kind}","message":"{MARKER}"}}}}"#)
}

fn assert_no_leak(error: &DgosError) {
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(API_KEY), "{shown}");
    assert!(!shown.contains(MARKER), "{shown}");
}

#[tokio::test]
async fn anthropic_retry_classification_429_waits_for_the_smaller_of_retry_after_and_max_delay() {
    let stub = serve(vec![
        response(
            429,
            &[("retry-after", "30")],
            &anthropic_error("rate_limit_error"),
        ),
        response(200, &[], &ok_message()),
    ])
    .await;
    // max_delay is 40 ms here, so the 30 s the upstream asked for must be capped.
    let adapter = anthropic(&stub.url, 3, 10);
    let started = Instant::now();
    let reply = adapter.call("m", request()).await.unwrap();
    let elapsed = started.elapsed();
    assert_eq!(reply.content, "ok");
    assert_eq!(stub.hits(), 2);
    assert!(
        elapsed >= std::time::Duration::from_millis(40),
        "waited the capped delay: {elapsed:?}"
    );
    assert!(elapsed.as_secs() < 5, "{elapsed:?}");
    let requests = stub.requests();
    assert_eq!(
        requests[0].body, requests[1].body,
        "the retry sends the same body"
    );
}

#[tokio::test]
async fn anthropic_retry_classification_503_and_529_are_retried_until_it_succeeds() {
    for (status, kind) in [
        (503, "api_error"),
        (529, "overloaded_error"),
        (500, "api_error"),
    ] {
        let stub = serve(vec![
            response(status, &[], &anthropic_error(kind)),
            response(status, &[], &anthropic_error(kind)),
            response(200, &[], &ok_message()),
        ])
        .await;
        let reply = anthropic(&stub.url, 3, 1)
            .call("m", request())
            .await
            .unwrap_or_else(|error| panic!("{status}: {error:?}"));
        assert_eq!(reply.content, "ok", "{status}");
        assert_eq!(stub.hits(), 3, "{status}");
    }
}

#[tokio::test]
async fn anthropic_retry_classification_does_not_retry_400_or_exhausted_quota() {
    let stub = serve(vec![response(
        400,
        &[],
        &anthropic_error("invalid_request_error"),
    )])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001017);
    assert_eq!(stub.hits(), 1);
    assert_no_leak(&error);

    let stub = serve(vec![response(
        429,
        &[("retry-after", "1")],
        &anthropic_error("enforced_spend_limit_reached"),
    )])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001024);
    assert_eq!(stub.hits(), 1);
    assert_no_leak(&error);
}

#[tokio::test]
async fn anthropic_retry_classification_reports_max_retries_only_after_a_retry_also_failed() {
    let stub = serve(vec![response(
        529,
        &[],
        &anthropic_error("overloaded_error"),
    )])
    .await;
    let error = anthropic(&stub.url, 2, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::MaxRetriesExceeded), "{error:?}");
    assert_eq!(error.error_code(), 1001008);
    assert_eq!(stub.hits(), 3, "one call and two retries");
    assert_no_leak(&error);

    let stub = serve(vec![response(
        529,
        &[],
        &anthropic_error("overloaded_error"),
    )])
    .await;
    let error = anthropic(&stub.url, 0, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(
        error.error_code(),
        1001014,
        "no retry configured: original code"
    );
    assert_eq!(stub.hits(), 1);
    assert_no_leak(&error);
}

#[tokio::test]
async fn anthropic_retry_classification_retries_a_stream_that_fails_before_any_event() {
    let stub = serve(vec![
        response(529, &[], &anthropic_error("overloaded_error")),
        sse_script(&[
            message_start(2),
            text_delta("hi"),
            message_delta("end_turn", 1),
            message_stop(),
        ]),
    ])
    .await;
    let items = drain(
        anthropic(&stub.url, 3, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert!(items.iter().all(|item| item.is_ok()), "{items:?}");
    assert_eq!(text_of(&items), "hi");
    assert_eq!(stub.hits(), 2);
}

#[tokio::test]
async fn anthropic_retry_classification_never_retries_or_replays_a_stream_that_already_produced_output(
) {
    let stub = serve(vec![
        sse_script(&[message_start(2), text_delta("partial")]),
        sse_script(&[
            message_start(2),
            text_delta("replayed"),
            message_delta("end_turn", 1),
            message_stop(),
        ]),
    ])
    .await;
    let items = drain(
        anthropic(&stub.url, 3, 1)
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text_of(&items), "partial");
    assert_eq!(items.iter().filter(|item| item.is_err()).count(), 1);
    assert!(matches!(
        items.last(),
        Some(Err(DgosError::StreamInterrupted))
    ));
    assert_eq!(stub.hits(), 1);
}
