//! AC02 (invalid key) and AC37 (connection test diagnostics). Mock HTTP, no real provider.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::DgosError;

#[tokio::test]
async fn invalid_api_key_is_reported_for_a_401_model_list() {
    let mut server = mockito::Server::new_async().await;
    let models = server
        .mock("GET", "/models")
        .match_header("authorization", "Bearer synthetic-key-123")
        .with_status(401)
        .with_body(r#"{"error":{"message":"upstream-secret-body"}}"#)
        .expect(1)
        .create_async()
        .await;
    let error = adapter(&server.url(), 3, 1)
        .test_connection(None)
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::InvalidApiKey));
    assert_eq!(error.error_code(), 1001002);
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("upstream-secret-body") && !shown.contains(API_KEY));
    // A bad key is final: not retried.
    models.assert_async().await;
}

#[tokio::test]
async fn connection_diagnostics_map_each_failure_to_its_registered_code() {
    let cases: [(u16, &[(&str, &str)], &str, u32); 5] = [
        (403, &[], "", 1001019),
        (
            200,
            &[("content-type", "text/html")],
            "<html><body>Login</body></html>",
            1001025,
        ),
        (200, &[], r#"{"object":"list"}"#, 1001025),
        (
            302,
            &[("location", "http://127.0.0.1:1/elsewhere")],
            "",
            1001025,
        ),
        (500, &[], "", 1001014),
    ];
    for (status, headers, body, code) in cases {
        let mut server = mockito::Server::new_async().await;
        let mut mock = server.mock("GET", "/models").with_status(status.into());
        for &(name, value) in headers {
            mock = mock.with_header(name, value);
        }
        let mock = mock.with_body(body).expect(1).create_async().await;
        let error = adapter(&server.url(), 3, 1)
            .test_connection(None)
            .await
            .unwrap_err();
        assert_eq!(error.error_code(), code, "status {status} body {body:?}");
        mock.assert_async().await;
    }
}

#[tokio::test]
async fn connection_diagnostics_treat_404_and_405_as_a_missing_model_list() {
    for status in [404, 405] {
        let mut server = mockito::Server::new_async().await;
        let models = server
            .mock("GET", "/models")
            .with_status(status)
            .expect(1)
            .create_async()
            .await;
        let error = adapter(&server.url(), 3, 1)
            .test_connection(None)
            .await
            .unwrap_err();
        // Without a model name the form is asked for one (probe flow, AC17).
        assert!(matches!(error, DgosError::RequestNotSupported), "{status}");
        assert_eq!(error.error_code(), 1001020);
        models.assert_async().await;
    }
}

#[tokio::test]
async fn connection_diagnostics_report_a_refused_connection_as_a_network_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let error = adapter(&url, 0, 1).test_connection(None).await.unwrap_err();
    assert!(matches!(error, DgosError::NetworkError(_)), "{error:?}");
    assert_eq!(error.error_code(), 1000000);
}

#[tokio::test]
async fn connection_test_succeeds_on_a_model_list_and_never_returns_false() {
    let mut server = mockito::Server::new_async().await;
    let models = server
        .mock("GET", "/models")
        .match_header("authorization", "Bearer synthetic-key-123")
        .with_status(200)
        .with_body(r#"{"data":[]}"#)
        .expect(1)
        .create_async()
        .await;
    assert!(adapter(&server.url(), 3, 1)
        .test_connection(None)
        .await
        .unwrap());
    models.assert_async().await;
}

// ---------------------------------------------------------------------------------------------
// Anthropic Messages twin (AC37)
// ---------------------------------------------------------------------------------------------

const MARKER: &str = "upstream-marker-conn-52";

fn upstream_error(kind: &str) -> String {
    serde_json::json!({"type": "error", "error": {"type": kind, "message": MARKER}}).to_string()
}

fn assert_clean(error: &DgosError) {
    let shown = format!("{error} {error:?}");
    assert!(
        !shown.contains(MARKER) && !shown.contains(API_KEY),
        "{shown}"
    );
}

#[tokio::test]
async fn anthropic_invalid_api_key_is_reported_for_a_401_model_list_and_is_final() {
    let stub = serve(vec![
        response(401, &[], &upstream_error("authentication_error")),
        response(200, &[], r#"{"data":[]}"#),
    ])
    .await;
    let error = anthropic(&stub.url, 3, 1)
        .test_connection(Some("probe-model"))
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::InvalidApiKey), "{error:?}");
    assert_eq!(error.error_code(), 1001002);
    assert_clean(&error);
    assert_eq!(stub.hits(), 1, "a bad key is final: no retry, no probe");
    let requests = stub.requests();
    let head = requests[0].head.to_lowercase();
    assert!(head.starts_with("get /v1/models"));
    assert!(head.contains(&format!("authorization: bearer {}", API_KEY.to_lowercase())));
    assert!(head.contains(&format!("x-api-key: {}", API_KEY.to_lowercase())));
}

#[tokio::test]
async fn anthropic_connection_diagnostics_map_each_model_list_failure_to_its_code() {
    let cases: [(u16, &[(&str, &str)], String, u32); 7] = [
        (403, &[], upstream_error("permission_error"), 1001019),
        (
            200,
            &[("content-type", "text/html")],
            format!("<html><body>Login {MARKER}</body></html>"),
            1001025,
        ),
        (200, &[], r#"{"object":"list"}"#.into(), 1001025),
        (200, &[], r#"{"data":"nope"}"#.into(), 1001025),
        (500, &[], upstream_error("api_error"), 1001014),
        (503, &[], upstream_error("api_error"), 1001014),
        (529, &[], upstream_error("overloaded_error"), 1001014),
    ];
    for (status, headers, body, code) in cases {
        // With and without a probe model the answer is the same and no probe is sent.
        for probe in [None, Some("probe-model")] {
            let stub = serve(vec![
                response(status, headers, &body),
                response(200, &[], &ok_message()),
            ])
            .await;
            let error = anthropic(&stub.url, 3, 1)
                .test_connection(probe)
                .await
                .unwrap_err();
            assert_eq!(error.error_code(), code, "status {status} probe {probe:?}");
            assert_clean(&error);
            assert_eq!(
                stub.hits(),
                1,
                "status {status} probe {probe:?}: no retry, no probe"
            );
        }
    }
}

#[tokio::test]
async fn anthropic_a_redirect_is_invalid_and_the_second_host_is_never_contacted() {
    for status in [301, 302, 303, 307, 308] {
        let target = serve(vec![response(200, &[], r#"{"data":[]}"#)]).await;
        let location = format!("{}/v1/models", target.url);
        let origin = serve(vec![response(status, &[("location", &location)], "")]).await;
        let error = anthropic(&origin.url, 3, 1)
            .test_connection(Some("probe-model"))
            .await
            .unwrap_err();
        assert!(
            matches!(error, DgosError::ProviderResponseInvalid),
            "{status}: {error:?}"
        );
        assert_eq!(error.error_code(), 1001025, "{status}");
        assert_eq!(origin.hits(), 1, "{status}");
        assert_eq!(
            target.hits(),
            0,
            "{status}: the key must not follow a redirect"
        );
    }
}

#[tokio::test]
async fn anthropic_a_redirected_probe_is_invalid_and_never_followed_either() {
    let target = serve(vec![response(200, &[], &ok_message())]).await;
    let location = format!("{}/v1/messages", target.url);
    let origin = serve(vec![
        response(404, &[], ""),
        response(307, &[("location", &location)], ""),
    ])
    .await;
    let error = anthropic(&origin.url, 3, 1)
        .test_connection(Some("probe-model"))
        .await
        .unwrap_err();
    assert_eq!(error.error_code(), 1001025, "{error:?}");
    assert_eq!(origin.hits(), 2);
    assert_eq!(target.hits(), 0);
}

#[tokio::test]
async fn anthropic_a_missing_model_list_asks_for_a_model_or_runs_the_probe() {
    for status in [404, 405] {
        // Without a model name: the form is asked for one and nothing is posted.
        let stub = serve(vec![
            response(status, &[], &upstream_error("not_found_error")),
            response(200, &[], &ok_message()),
        ])
        .await;
        let error = anthropic(&stub.url, 3, 1)
            .test_connection(None)
            .await
            .unwrap_err();
        assert!(
            matches!(error, DgosError::RequestNotSupported),
            "{status}: {error:?}"
        );
        assert_eq!(error.error_code(), 1001020);
        assert_clean(&error);
        assert_eq!(stub.hits(), 1, "{status}");

        // With a model name: one probe POST decides.
        let stub = serve(vec![
            response(status, &[], &upstream_error("not_found_error")),
            response(200, &[], &ok_message()),
        ])
        .await;
        assert!(anthropic(&stub.url, 3, 1)
            .test_connection(Some("probe-model"))
            .await
            .unwrap());
        let requests = stub.requests();
        assert_eq!(requests.len(), 2, "{status}");
        assert!(requests[0].head.starts_with("GET /v1/models"));
        assert!(requests[1].head.starts_with("POST /v1/messages"));
    }
}

#[tokio::test]
async fn anthropic_a_refused_connection_is_a_network_error_not_ok_false() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    for probe in [None, Some("probe-model")] {
        let result = anthropic(&url, 0, 1).test_connection(probe).await;
        let error = result.unwrap_err();
        assert!(
            matches!(error, DgosError::NetworkError(_)),
            "{probe:?}: {error:?}"
        );
        assert_eq!(error.error_code(), 1000000);
        assert_clean(&error);
    }
}

#[tokio::test]
async fn anthropic_connection_test_succeeds_on_a_model_list_and_never_returns_false() {
    let stub = serve(vec![response(
        200,
        &[],
        r#"{"data":[{"id":"claude-x"}],"has_more":false}"#,
    )])
    .await;
    assert!(anthropic(&stub.url, 3, 1)
        .test_connection(None)
        .await
        .unwrap());
    assert_eq!(stub.hits(), 1);

    // Whatever the service answers, the result is Ok(true) or an error, never Ok(false).
    let outcomes: [(u16, &str); 14] = [
        (200, r#"{"data":[]}"#),
        (200, "{}"),
        (200, ""),
        (201, r#"{"data":[]}"#),
        (204, ""),
        (301, ""),
        (400, ""),
        (401, ""),
        (403, ""),
        (404, ""),
        (405, ""),
        (429, ""),
        (500, ""),
        (529, ""),
    ];
    for (status, body) in outcomes {
        for probe in [None, Some("probe-model")] {
            let stub = serve(vec![
                response(status, &[], body),
                response(status, &[], body),
            ])
            .await;
            let result = anthropic(&stub.url, 0, 1).test_connection(probe).await;
            assert_ne!(
                result.as_ref().ok().copied(),
                Some(false),
                "status {status} probe {probe:?}"
            );
        }
    }
}

#[tokio::test]
async fn anthropic_official_profile_authenticates_with_bearer_only() {
    let stub = serve(vec![response(200, &[], r#"{"data":[]}"#)]).await;
    assert!(anthropic_official(&stub.url, Default::default())
        .test_connection(None)
        .await
        .unwrap());
    let headers = header_lines(&stub.requests()[0]);
    assert!(headers.contains(&format!("authorization: bearer {}", API_KEY.to_lowercase())));
    assert!(!headers.iter().any(|line| line.starts_with("x-api-key")));
    assert!(headers.contains(&"anthropic-version: 2026-10-21".to_string()));
}
