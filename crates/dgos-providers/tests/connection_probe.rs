//! AC17: a service without a model list endpoint is verified with one small probe request.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::DgosError;
use mockito::Matcher;
use serde_json::json;

#[tokio::test]
async fn probe_sends_exactly_one_max_tokens_16_request_after_a_404_model_list() {
    for status in [404, 405] {
        let mut server = mockito::Server::new_async().await;
        let models = server
            .mock("GET", "/models")
            .with_status(status)
            .expect(1)
            .create_async()
            .await;
        let probe = server
            .mock("POST", "/chat/completions")
            .match_header("authorization", "Bearer synthetic-key-123")
            .match_body(Matcher::PartialJson(
                json!({"model": "local-model", "max_tokens": 16}),
            ))
            .with_status(200)
            // Any 2xx counts, whatever the body is.
            .with_body("not even json")
            .expect(1)
            .create_async()
            .await;
        assert!(adapter(&server.url(), 3, 1)
            .test_connection(Some("local-model"))
            .await
            .unwrap());
        models.assert_async().await;
        probe.assert_async().await;
    }
}

#[tokio::test]
async fn probe_is_not_sent_when_the_model_list_works_or_no_model_name_was_given() {
    let mut server = mockito::Server::new_async().await;
    let _models = server
        .mock("GET", "/models")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .create_async()
        .await;
    let probe = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .expect(0)
        .create_async()
        .await;
    assert!(adapter(&server.url(), 3, 1)
        .test_connection(Some("local-model"))
        .await
        .unwrap());
    probe.assert_async().await;

    let mut server = mockito::Server::new_async().await;
    let _missing = server
        .mock("GET", "/models")
        .with_status(404)
        .create_async()
        .await;
    let probe = server
        .mock("POST", "/chat/completions")
        .with_status(200)
        .expect(0)
        .create_async()
        .await;
    for model in [None, Some(""), Some("   ")] {
        let error = adapter(&server.url(), 3, 1)
            .test_connection(model)
            .await
            .unwrap_err();
        assert!(matches!(error, DgosError::RequestNotSupported), "{model:?}");
    }
    probe.assert_async().await;
}

#[tokio::test]
async fn probe_failures_are_reported_with_their_own_codes_and_not_retried() {
    let cases = [
        (401, 1001002),
        (403, 1001019),
        (404, 1001004),
        (400, 1001017),
        (500, 1001014),
    ];
    for (status, code) in cases {
        let mut server = mockito::Server::new_async().await;
        let _models = server
            .mock("GET", "/models")
            .with_status(404)
            .create_async()
            .await;
        let probe = server
            .mock("POST", "/chat/completions")
            .with_status(status)
            .expect(1)
            .create_async()
            .await;
        let error = adapter(&server.url(), 3, 1)
            .test_connection(Some("local-model"))
            .await
            .unwrap_err();
        assert_eq!(error.error_code(), code, "probe status {status}");
        probe.assert_async().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Anthropic Messages (AC17): the model list is `GET /v1/models`, the probe is `POST /v1/messages`.
// ---------------------------------------------------------------------------------------------

const MARKER: &str = "upstream-marker-probe-31";

fn posts(stub: &Stub) -> Vec<Captured> {
    stub.requests()
        .into_iter()
        .filter(|captured| captured.head.starts_with("POST "))
        .collect()
}

fn request_line(captured: &Captured) -> &str {
    captured.head.lines().next().unwrap_or_default()
}

fn body_json(captured: &Captured) -> serde_json::Value {
    serde_json::from_str(&captured.body).unwrap()
}

#[tokio::test]
async fn anthropic_probe_sends_exactly_one_minimal_messages_request_after_a_404_or_405_model_list()
{
    for status in [404, 405] {
        let stub = serve(vec![
            response(status, &[], ""),
            response(200, &[], &ok_message()),
        ])
        .await;
        assert!(anthropic(&stub.url, 3, 1)
            .test_connection(Some("probe-model"))
            .await
            .unwrap());
        let requests = stub.requests();
        assert_eq!(
            requests.len(),
            2,
            "status {status}: one list GET, one probe POST"
        );
        assert!(request_line(&requests[0]).starts_with("GET /v1/models"));
        assert!(request_line(&requests[1]).starts_with("POST /v1/messages "));
        let body = body_json(&requests[1]);
        assert_eq!(body["model"], "probe-model");
        assert_eq!(body["max_tokens"], 16);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let blocks = messages[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        for absent in [
            "thinking",
            "output_config",
            "tools",
            "tool_choice",
            "temperature",
        ] {
            assert!(body.get(absent).is_none(), "probe must not send {absent}");
        }
        let headers = header_lines(&requests[1]);
        assert!(headers.contains(&format!("authorization: bearer {}", API_KEY.to_lowercase())));
        assert!(headers.contains(&format!("x-api-key: {}", API_KEY.to_lowercase())));
        assert!(headers.contains(&"anthropic-version: 2023-06-01".to_string()));
    }
}

#[tokio::test]
async fn anthropic_probe_counts_any_2xx_as_success_whatever_the_body_is() {
    let cases: [(u16, &str); 6] = [
        (200, "{}"),
        (200, "not even json"),
        (200, ""),
        (200, r#"{"content":[]}"#),
        (201, r#"{"unexpected":true}"#),
        (204, ""),
    ];
    for (status, body) in cases {
        let stub = serve(vec![response(404, &[], ""), response(status, &[], body)]).await;
        let ok = anthropic(&stub.url, 3, 1)
            .test_connection(Some("probe-model"))
            .await
            .unwrap();
        assert!(ok, "status {status} body {body:?}");
        assert_eq!(stub.hits(), 2, "status {status} body {body:?}");
        assert_eq!(posts(&stub).len(), 1);
    }
}

#[tokio::test]
async fn anthropic_model_list_success_is_true_without_any_probe_request() {
    for body in [
        r#"{"data":[]}"#,
        r#"{"data":[{"id":"claude-x"}],"has_more":false}"#,
    ] {
        let stub = serve(vec![
            response(200, &[], body),
            response(200, &[], &ok_message()),
        ])
        .await;
        assert!(anthropic(&stub.url, 3, 1)
            .test_connection(Some("probe-model"))
            .await
            .unwrap());
        assert_eq!(stub.hits(), 1, "{body}");
        assert!(request_line(&stub.requests()[0]).starts_with("GET /v1/models"));
        assert!(posts(&stub).is_empty());
    }
}

#[tokio::test]
async fn anthropic_without_a_usable_probe_model_is_request_not_supported_and_sends_no_post() {
    let too_long = "m".repeat(256);
    let models: [Option<&str>; 5] = [
        None,
        Some(""),
        Some("   "),
        Some(too_long.as_str()),
        Some("\t\n"),
    ];
    for model in models {
        let stub = serve(vec![
            response(404, &[], ""),
            response(200, &[], &ok_message()),
        ])
        .await;
        let error = anthropic(&stub.url, 3, 1)
            .test_connection(model)
            .await
            .unwrap_err();
        assert!(
            matches!(error, DgosError::RequestNotSupported),
            "{:?}: {error:?}",
            model.map(str::len)
        );
        assert_eq!(error.error_code(), 1001020);
        assert_eq!(stub.hits(), 1, "only the model list GET is sent");
        assert!(posts(&stub).is_empty());
    }
}

#[tokio::test]
async fn anthropic_probe_model_id_of_exactly_255_chars_is_still_probed() {
    let model = "m".repeat(255);
    let stub = serve(vec![
        response(404, &[], ""),
        response(200, &[], &ok_message()),
    ])
    .await;
    assert!(anthropic(&stub.url, 3, 1)
        .test_connection(Some(&model))
        .await
        .unwrap());
    let sent = posts(&stub);
    assert_eq!(sent.len(), 1);
    assert_eq!(body_json(&sent[0])["model"], model.as_str());
}

#[tokio::test]
async fn anthropic_probe_failures_are_classified_not_true_and_not_retried() {
    let cases = [
        (401, 1001002),
        (403, 1001019),
        (404, 1001004),
        (400, 1001017),
        (429, 1001005),
        (500, 1001014),
        (503, 1001014),
        (529, 1001014),
    ];
    for (status, code) in cases {
        let upstream =
            json!({"type":"error","error":{"type":"api_error","message":MARKER}}).to_string();
        let stub = serve(vec![
            response(404, &[], ""),
            response(status, &[], &upstream),
            response(200, &[], &ok_message()),
        ])
        .await;
        let result = anthropic(&stub.url, 3, 1)
            .test_connection(Some("probe-model"))
            .await;
        let error = result.unwrap_err();
        assert_eq!(error.error_code(), code, "probe status {status}");
        assert_eq!(
            stub.hits(),
            2,
            "probe status {status}: one GET, one POST, no retry"
        );
        let shown = format!("{error} {error:?}");
        assert!(
            !shown.contains(MARKER) && !shown.contains(API_KEY),
            "{shown}"
        );
        if status == 404 {
            assert_eq!(error.to_string(), "Model not found: probe-model");
        }
    }
}

#[tokio::test]
async fn anthropic_probe_under_the_official_profile_sends_bearer_only_and_the_official_version() {
    let stub = serve(vec![
        response(404, &[], ""),
        response(200, &[], &ok_message()),
    ])
    .await;
    let adapter = anthropic_official(&stub.url, Default::default());
    assert!(adapter.test_connection(Some("probe-model")).await.unwrap());
    let sent = posts(&stub);
    assert_eq!(sent.len(), 1);
    let headers = header_lines(&sent[0]);
    assert!(headers.contains(&format!("authorization: bearer {}", API_KEY.to_lowercase())));
    assert!(!headers.iter().any(|line| line.starts_with("x-api-key")));
    assert!(headers.contains(&"anthropic-version: 2026-10-21".to_string()));
    let body = body_json(&sent[0]);
    assert_eq!(body["max_tokens"], 16);
    assert!(body.get("thinking").is_none());
}
