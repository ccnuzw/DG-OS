//! AC34: redirects are never followed. The second server must see zero requests, and the key must
//! only ever reach the configured host.
mod common;

use common::*;
use dgos_providers::ProviderAdapter;
use dgos_types::DgosError;

async fn origin_redirecting_to(target: &Stub) -> Stub {
    let location = format!("{}/stolen", target.url);
    serve(vec![response(307, &[("location", &location)], "")]).await
}

#[tokio::test]
async fn model_list_redirect_is_invalid_and_never_followed() {
    let target = serve(vec![response(200, &[], r#"{"data":[]}"#)]).await;
    let origin = origin_redirecting_to(&target).await;
    let error = adapter(&origin.url, 3, 1).list_models().await.unwrap_err();
    assert!(
        matches!(error, DgosError::ProviderResponseInvalid),
        "{error:?}"
    );
    assert_eq!(error.error_code(), 1001025);
    assert_eq!(origin.hits(), 1);
    assert_eq!(target.hits(), 0, "credentials must not follow a redirect");
}

#[tokio::test]
async fn synchronous_call_redirect_is_invalid_not_retried_and_not_followed() {
    let target = serve(vec![response(200, &[], &ok_chat())]).await;
    let origin = origin_redirecting_to(&target).await;
    let error = adapter(&origin.url, 3, 1)
        .call("m", request())
        .await
        .unwrap_err();
    assert!(
        matches!(error, DgosError::ProviderResponseInvalid),
        "{error:?}"
    );
    assert_eq!(origin.hits(), 1, "a 3xx is final, never retried");
    assert_eq!(target.hits(), 0);
}

#[tokio::test]
async fn stream_redirect_is_invalid_not_retried_and_not_followed() {
    let target = serve(vec![sse_script(&[delta("x"), DONE.into()])]).await;
    let origin = origin_redirecting_to(&target).await;
    let error = adapter(&origin.url, 3, 1)
        .stream("m", request())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, DgosError::ProviderResponseInvalid),
        "{error:?}"
    );
    assert_eq!(origin.hits(), 1);
    assert_eq!(target.hits(), 0);
}

#[tokio::test]
async fn every_redirect_status_is_final() {
    for status in [301, 302, 303, 307, 308] {
        let target = serve(vec![response(200, &[], &ok_chat())]).await;
        let location = format!("{}/x", target.url);
        let origin = serve(vec![response(status, &[("location", &location)], "")]).await;
        let error = adapter(&origin.url, 3, 1)
            .call("m", request())
            .await
            .unwrap_err();
        assert_eq!(error.error_code(), 1001025, "{status}");
        assert_eq!(target.hits(), 0, "{status}");
    }
}

#[tokio::test]
async fn the_key_goes_only_to_the_configured_host() {
    let origin = serve(vec![response(200, &[], &ok_chat())]).await;
    adapter(&origin.url, 0, 1)
        .call("m", request())
        .await
        .unwrap();
    let head = origin.requests()[0].head.to_lowercase();
    assert!(head.contains(&format!("authorization: bearer {API_KEY}")));
}
