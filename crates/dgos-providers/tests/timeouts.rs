//! AC31: the non-streaming deadline does not apply to streams; streams are bounded by an idle
//! timeout instead. A scripted local server controls pacing, so nothing waits 60 seconds: the
//! deadlines are shortened through `AdapterOptions` and the stream lasts several times longer.
mod common;

use common::*;
use dgos_providers::{AdapterOptions, ProviderAdapter};
use dgos_types::DgosError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn options(request_ms: u64, idle_ms: u64) -> AdapterOptions {
    AdapterOptions {
        request_timeout: Duration::from_millis(request_ms),
        stream_idle_timeout: Duration::from_millis(idle_ms),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_stream_outlives_the_non_streaming_deadline_when_events_keep_coming() {
    // Ten events 100 ms apart: about a second in total, five times the 200 ms request deadline,
    // but never more than 100 ms of silence against a 500 ms idle timeout.
    let mut script = vec![sse_head()];
    for index in 0..10 {
        script.push(text(&delta(&format!("{index},"))));
        script.push(sleep(100));
    }
    script.push(text(&format!("{}{}{DONE}", finish("stop"), usage(1, 10))));
    let stub = serve(vec![script]).await;
    let started = Instant::now();
    let items = drain(
        adapter_with(&stub.url, 3, 1, options(200, 500))
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert!(
        started.elapsed() > Duration::from_millis(900),
        "{:?}",
        started.elapsed()
    );
    assert!(items.iter().all(|item| item.is_ok()), "{items:?}");
    assert_eq!(text_of(&items), "0,1,2,3,4,5,6,7,8,9,");
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn a_synchronous_call_is_bounded_by_the_request_deadline() {
    let stub = serve(vec![vec![
        text("HTTP/1.1 200 OK\r\ncontent-length: 10\r\nconnection: close\r\n\r\n"),
        text("{"),
        sleep(5_000),
    ]])
    .await;
    // Built first: creating the HTTP client can stall for seconds on macOS (one build took
    // 3.1 s in a 40-run probe) and is not part of the request deadline under test.
    let adapter = adapter_with(&stub.url, 0, 1, options(300, 5_000));
    let started = Instant::now();
    let error = adapter.call("m", request()).await.unwrap_err();
    assert!(matches!(error, DgosError::Timeout), "{error:?}");
    assert_eq!(error.error_code(), 1001006);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn an_idle_stream_times_out_once_keeping_what_arrived_and_is_not_retried() {
    let stub = serve(vec![vec![
        sse_head(),
        text(&delta("one ")),
        text(&delta("two")),
        sleep(10_000),
    ]])
    .await;
    let adapter = adapter_with(&stub.url, 3, 1, options(5_000, 300));
    let started = Instant::now();
    let items = drain(adapter.stream("m", request()).await.unwrap()).await;
    assert_eq!(text_of(&items), "one two");
    assert_eq!(items.iter().filter(|item| item.is_err()).count(), 1);
    let error = items.last().unwrap().as_ref().unwrap_err();
    assert!(matches!(error, DgosError::Timeout), "{error:?}");
    assert_eq!(error.error_code(), 1001006);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(stub.hits(), 1, "an idle stream is never replayed");
}

#[tokio::test]
async fn a_stream_that_never_sends_a_byte_times_out_without_retrying() {
    let stub = serve(vec![vec![sleep(10_000)]]).await;
    let adapter = adapter_with(&stub.url, 3, 1, options(5_000, 300));
    let started = Instant::now();
    let error = adapter.stream("m", request()).await.err().unwrap();
    assert!(matches!(error, DgosError::Timeout), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn a_stream_that_stops_after_the_headers_times_out_without_a_retry() {
    let stub = serve(vec![vec![sse_head(), sleep(10_000)]]).await;
    let items = drain(
        adapter_with(&stub.url, 3, 1, options(5_000, 300))
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], Err(DgosError::Timeout)));
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn keep_alive_comments_reset_the_idle_timer() {
    let mut script = vec![sse_head()];
    for _ in 0..6 {
        script.push(text(": ping\n\n"));
        script.push(sleep(120));
    }
    script.push(text(&format!("{}{}{DONE}", delta("late"), finish("stop"))));
    let stub = serve(vec![script]).await;
    // 720 ms of pings against a 300 ms idle timeout.
    let items = drain(
        adapter_with(&stub.url, 0, 1, options(5_000, 300))
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert!(items.iter().all(|item| item.is_ok()), "{items:?}");
    assert_eq!(text_of(&items), "late");
}

// ---------------------------------------------------------------------------------------------
// Anthropic twins of the AC31 tests.
// ---------------------------------------------------------------------------------------------

fn anthropic_tail() -> Vec<Step> {
    vec![text(&format!(
        "{}{}",
        message_delta("end_turn", 10),
        message_stop()
    ))]
}

#[tokio::test]
async fn anthropic_stream_outlives_the_non_streaming_deadline_when_events_keep_coming() {
    // Ten events 100 ms apart: about a second in total, five times the 200 ms request deadline,
    // but never more than 100 ms of silence against a 500 ms idle timeout.
    let mut script = vec![sse_head(), text(&message_start(3))];
    for index in 0..10 {
        script.push(text(&text_delta(&format!("{index},"))));
        script.push(sleep(100));
    }
    script.extend(anthropic_tail());
    let stub = serve(vec![script]).await;
    let started = Instant::now();
    let items = drain(
        anthropic_with(&stub.url, 3, 1, options(200, 500))
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert!(
        started.elapsed() > Duration::from_millis(900),
        "{:?}",
        started.elapsed()
    );
    assert!(items.iter().all(|item| item.is_ok()), "{items:?}");
    assert_eq!(text_of(&items), "0,1,2,3,4,5,6,7,8,9,");
    let terminal = items.last().unwrap().as_ref().unwrap();
    assert_eq!(terminal.finish_reason.as_deref(), Some("stop"));
    assert_eq!(terminal.usage.as_ref().unwrap().total_tokens, 13);
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn anthropic_synchronous_call_is_bounded_by_the_request_deadline() {
    let stub = serve(vec![vec![
        text("HTTP/1.1 200 OK\r\ncontent-length: 10\r\nconnection: close\r\n\r\n"),
        text("{"),
        sleep(5_000),
    ]])
    .await;
    let adapter = anthropic_with(&stub.url, 0, 1, options(300, 5_000));
    let started = Instant::now();
    let error = adapter.call("m", request()).await.unwrap_err();
    assert!(matches!(error, DgosError::Timeout), "{error:?}");
    assert_eq!(error.error_code(), 1001006);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn anthropic_idle_stream_times_out_once_keeping_what_arrived_and_is_not_retried() {
    let stub = serve(vec![vec![
        sse_head(),
        text(&message_start(3)),
        text(&text_delta("one ")),
        text(&text_delta("two")),
        sleep(10_000),
    ]])
    .await;
    let adapter = anthropic_with(&stub.url, 3, 1, options(5_000, 300));
    let started = Instant::now();
    let items = drain(adapter.stream("m", request()).await.unwrap()).await;
    assert_eq!(text_of(&items), "one two");
    assert_eq!(items.iter().filter(|item| item.is_err()).count(), 1);
    let error = items.last().unwrap().as_ref().unwrap_err();
    assert!(matches!(error, DgosError::Timeout), "{error:?}");
    assert_eq!(error.error_code(), 1001006);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(stub.hits(), 1, "an idle stream is never replayed");
}

#[tokio::test]
async fn anthropic_stream_that_never_sends_a_byte_times_out_without_retrying() {
    let stub = serve(vec![vec![sleep(10_000)]]).await;
    let adapter = anthropic_with(&stub.url, 3, 1, options(5_000, 300));
    let started = Instant::now();
    let error = adapter.stream("m", request()).await.err().unwrap();
    assert!(matches!(error, DgosError::Timeout), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn anthropic_stream_that_stops_after_the_headers_times_out_without_a_retry() {
    let stub = serve(vec![vec![sse_head(), sleep(10_000)]]).await;
    let items = drain(
        anthropic_with(&stub.url, 3, 1, options(5_000, 300))
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], Err(DgosError::Timeout)));
    assert_eq!(stub.hits(), 1);
}

#[tokio::test]
async fn anthropic_ping_events_reset_the_idle_timer() {
    let mut script = vec![sse_head(), text(&message_start(3))];
    for _ in 0..6 {
        script.push(text(&ping()));
        script.push(sleep(120));
    }
    script.push(text(&text_delta("late")));
    script.extend(anthropic_tail());
    let stub = serve(vec![script]).await;
    // 720 ms of pings against a 300 ms idle timeout.
    let started = Instant::now();
    let items = drain(
        anthropic_with(&stub.url, 0, 1, options(5_000, 300))
            .stream("m", request())
            .await
            .unwrap(),
    )
    .await;
    assert!(started.elapsed() > Duration::from_millis(700));
    assert!(items.iter().all(|item| item.is_ok()), "{items:?}");
    assert_eq!(text_of(&items), "late");
    assert_eq!(items.len(), 2, "pings produce no chunks");
}

// ---------------------------------------------------------------------------------------------
// Connection timeout (both adapters).
// ---------------------------------------------------------------------------------------------

/// A loopback listener that completes the TCP handshake and then never says a word. Against an
/// `https://` URL the client's TLS handshake waits for a ServerHello that never comes, so the
/// connect stalls on every OS (an unaccepted-backlog or unroutable-address trick is not portable).
/// Returns the URL and a counter of accepted connections.
async fn tls_stall() -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("https://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            held.push(socket);
        }
    });
    (url, accepted)
}

fn connect_options() -> AdapterOptions {
    AdapterOptions {
        connect_timeout: Duration::from_millis(300),
        // Long enough that only the connect timeout can end these calls early.
        request_timeout: Duration::from_secs(20),
        stream_idle_timeout: Duration::from_secs(20),
        models_timeout: Duration::from_secs(20),
        ..Default::default()
    }
}

/// A connect that never completes ends near `connect_timeout` as `timeout` (or a network error),
/// is not retried, and does not leak the key.
fn assert_connect_timeout(label: &str, error: &DgosError, started: Instant) {
    assert!(
        matches!(error, DgosError::Timeout | DgosError::NetworkError(_)),
        "{label}: {error:?}"
    );
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(API_KEY), "{label}: {shown}");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(250),
        "{label}: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(3), "{label}: {elapsed:?}");
}

#[tokio::test]
async fn openai_connect_that_never_completes_fails_within_the_connect_timeout() {
    let (url, accepted) = tls_stall().await;
    let adapter = adapter_with(&url, 0, 1, connect_options());
    let started = Instant::now();
    let error = adapter.call("m", request()).await.unwrap_err();
    assert_connect_timeout("call", &error, started);
    let started = Instant::now();
    let error = adapter.stream("m", request()).await.err().unwrap();
    assert_connect_timeout("stream", &error, started);
    let started = Instant::now();
    let error = adapter.list_models().await.unwrap_err();
    assert_connect_timeout("list_models", &error, started);
    // The TCP connection was made each time: it is the stalled handshake that timed out.
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn anthropic_connect_that_never_completes_fails_within_the_connect_timeout() {
    let (url, accepted) = tls_stall().await;
    let adapter = anthropic_with(&url, 0, 1, connect_options());
    let started = Instant::now();
    let error = adapter.call("m", request()).await.unwrap_err();
    assert_connect_timeout("call", &error, started);
    let started = Instant::now();
    let error = adapter.stream("m", request()).await.err().unwrap();
    assert_connect_timeout("stream", &error, started);
    let started = Instant::now();
    let error = adapter.list_models().await.unwrap_err();
    assert_connect_timeout("list_models", &error, started);
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
}

// ---------------------------------------------------------------------------------------------
// Model-list timeout (both adapters).
// ---------------------------------------------------------------------------------------------

fn models_options() -> AdapterOptions {
    AdapterOptions {
        models_timeout: Duration::from_millis(300),
        ..Default::default()
    }
}

const OPENAI_MODELS: &str = r#"{"object":"list","data":[{"id":"gpt-a"},{"id":"gpt-b"}]}"#;
const ANTHROPIC_MODELS: &str = r#"{"data":[{"id":"claude-a","type":"model"},{"id":"claude-b","type":"model"}],"has_more":false,"last_id":"claude-b"}"#;

fn model_ids(models: &[dgos_types::Model]) -> Vec<&str> {
    models.iter().map(|model| model.model_id.as_str()).collect()
}

#[tokio::test]
async fn openai_model_list_that_answers_too_slowly_times_out_without_waiting_for_it() {
    let slow_headers = vec![sleep(5_000)];
    let slow_body = vec![
        text("HTTP/1.1 200 OK\r\ncontent-length: 100\r\nconnection: close\r\n\r\n"),
        text("{"),
        sleep(5_000),
    ];
    for (label, script) in [("no answer", slow_headers), ("stalled body", slow_body)] {
        let stub = serve(vec![script]).await;
        // Built first: creating the HTTP client can be slow and is not part of the deadline.
        let adapter = adapter_with(&stub.url, 3, 1, models_options());
        let started = Instant::now();
        let error = adapter.list_models().await.unwrap_err();
        assert!(matches!(error, DgosError::Timeout), "{label}: {error:?}");
        assert_eq!(error.error_code(), 1001006, "{label}");
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(250),
            "{label}: {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(3), "{label}: {elapsed:?}");
        assert_eq!(stub.hits(), 1, "{label}");
        assert_eq!(
            stub.requests()[0].head.lines().next(),
            Some("GET /models HTTP/1.1")
        );
    }
}

#[tokio::test]
async fn openai_model_list_that_answers_within_the_limit_succeeds() {
    let stub = serve(vec![
        [vec![sleep(100)], response(200, &[], OPENAI_MODELS)].concat()
    ])
    .await;
    let models = adapter_with(&stub.url, 0, 1, models_options())
        .list_models()
        .await
        .unwrap();
    assert_eq!(model_ids(&models), ["gpt-a", "gpt-b"]);
}

#[tokio::test]
async fn anthropic_model_list_that_answers_too_slowly_times_out_without_waiting_for_it() {
    let slow_headers = vec![sleep(5_000)];
    let slow_body = vec![
        text("HTTP/1.1 200 OK\r\ncontent-length: 100\r\nconnection: close\r\n\r\n"),
        text("{"),
        sleep(5_000),
    ];
    for (label, script) in [("no answer", slow_headers), ("stalled body", slow_body)] {
        let stub = serve(vec![script]).await;
        let adapter = anthropic_with(&stub.url, 3, 1, models_options());
        let started = Instant::now();
        let error = adapter.list_models().await.unwrap_err();
        assert!(matches!(error, DgosError::Timeout), "{label}: {error:?}");
        assert_eq!(error.error_code(), 1001006, "{label}");
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(250),
            "{label}: {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(3), "{label}: {elapsed:?}");
        assert_eq!(stub.hits(), 1, "{label}");
        assert!(stub.requests()[0]
            .head
            .starts_with("GET /v1/models?limit=100 HTTP/1.1"));
    }
}

#[tokio::test]
async fn anthropic_model_list_that_answers_within_the_limit_succeeds() {
    let stub = serve(vec![[
        vec![sleep(100)],
        response(200, &[], ANTHROPIC_MODELS),
    ]
    .concat()])
    .await;
    let models = anthropic_with(&stub.url, 0, 1, models_options())
        .list_models()
        .await
        .unwrap();
    assert_eq!(model_ids(&models), ["claude-a", "claude-b"]);
}

// ---------------------------------------------------------------------------------------------
// Default limits (§2.4.6 #3): connect 10 s, models 30 s, request 120 s, stream idle 120 s,
// body 8 MiB, one SSE event 1 MiB.
// ---------------------------------------------------------------------------------------------

const EIGHT_MIB: usize = 8 * 1024 * 1024;

fn assert_spec_defaults() {
    let defaults = AdapterOptions::default();
    assert_eq!(defaults.connect_timeout, Duration::from_secs(10));
    assert_eq!(defaults.models_timeout, Duration::from_secs(30));
    assert_eq!(defaults.request_timeout, Duration::from_secs(120));
    assert_eq!(defaults.stream_idle_timeout, Duration::from_secs(120));
    assert_eq!(defaults.max_body_bytes, 8 * 1024 * 1024);
    assert_eq!(defaults.max_event_bytes, 1024 * 1024);
}

/// `body` padded with an unknown field to exactly `total` bytes.
fn padded(body: &str, total: usize) -> String {
    let mut value: serde_json::Value = serde_json::from_str(body).unwrap();
    value["pad"] = "".into();
    let base = value.to_string().len();
    value["pad"] = "x".repeat(total - base).into();
    let out = value.to_string();
    assert_eq!(out.len(), total);
    out
}

#[tokio::test]
async fn openai_family_defaults_match_the_spec_numbers() {
    assert_spec_defaults();
    // The constructor without options applies them: exactly 8 MiB is read, one byte more is not.
    let at_limit = padded(&ok_chat(), EIGHT_MIB);
    let over_limit = padded(&ok_chat(), EIGHT_MIB + 1);
    let stub = serve(vec![response(200, &[], &at_limit)]).await;
    let reply = dgos_providers::OpenAIAdapter::new(stub.url.clone(), API_KEY.into(), None)
        .call("m", request())
        .await
        .unwrap();
    assert_eq!(reply.content, "ok");
    let stub = serve(vec![response(200, &[], &over_limit)]).await;
    let error = dgos_providers::OpenAIAdapter::new(stub.url.clone(), API_KEY.into(), None)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001025, "{error:?}");

    // One SSE event: about 1.0 MB is delivered, about 1.1 MB interrupts the stream.
    for (size, ok) in [(1_000_000usize, true), (1_100_000, false)] {
        let stub = serve(vec![sse_script(&[
            delta(&"x".repeat(size)),
            finish("stop"),
            DONE.into(),
        ])])
        .await;
        let items = drain(
            dgos_providers::OpenAIAdapter::new(stub.url.clone(), API_KEY.into(), None)
                .stream("m", request())
                .await
                .unwrap(),
        )
        .await;
        if ok {
            assert!(items.iter().all(|item| item.is_ok()), "{size}");
            assert_eq!(text_of(&items).len(), size);
        } else {
            assert_eq!(text_of(&items), "", "{size}");
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].as_ref().unwrap_err().error_code(), 1001007);
        }
    }
}

#[tokio::test]
async fn anthropic_family_defaults_match_the_spec_numbers() {
    assert_spec_defaults();
    let at_limit = padded(&ok_message(), EIGHT_MIB);
    let over_limit = padded(&ok_message(), EIGHT_MIB + 1);
    let stub = serve(vec![response(200, &[], &at_limit)]).await;
    let reply = dgos_providers::AnthropicAdapter::new(stub.url.clone(), API_KEY.into(), None)
        .call("m", request())
        .await
        .unwrap();
    assert_eq!(reply.content, "ok");
    let stub = serve(vec![response(200, &[], &over_limit)]).await;
    let error = dgos_providers::AnthropicAdapter::new(stub.url.clone(), API_KEY.into(), None)
        .call("m", request())
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001025, "{error:?}");

    for (size, ok) in [(1_000_000usize, true), (1_100_000, false)] {
        let stub = serve(vec![sse_script(&[
            message_start(1),
            text_delta(&"x".repeat(size)),
            message_delta("end_turn", 1),
            message_stop(),
        ])])
        .await;
        let items = drain(
            dgos_providers::AnthropicAdapter::new(stub.url.clone(), API_KEY.into(), None)
                .stream("m", request())
                .await
                .unwrap(),
        )
        .await;
        if ok {
            assert!(items.iter().all(|item| item.is_ok()), "{size}");
            assert_eq!(text_of(&items).len(), size);
        } else {
            assert_eq!(text_of(&items), "", "{size}");
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].as_ref().unwrap_err().error_code(), 1001007);
        }
    }
}
