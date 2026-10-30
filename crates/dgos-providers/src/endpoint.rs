//! Endpoint policy (V1-FR-001 §2.4.6 #10, B01-37, AC32).
//!
//! Applied when a configuration is saved and again before every call, so a configuration
//! that was stored earlier is still rejected before any request leaves the host.

use dgos_types::{DgosError, DgosResult};
use reqwest::Url;
use std::net::IpAddr;

/// Returns the normalised base URL (trailing `/` removed) or `InsecureEndpoint`.
///
/// * only `http` and `https` are accepted (`file://`, `ftp://`, scheme-less input are not);
/// * `http` needs a loopback or private literal address, or `allow_insecure_http`;
/// * host names other than `localhost` are never resolved, so `ollama.lan` counts as public.
pub fn check_endpoint(base_url: &str, allow_insecure_http: bool) -> DgosResult<String> {
    let trimmed = base_url.trim();
    let url = Url::parse(trimmed).map_err(|_| DgosError::InsecureEndpoint)?;
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or(DgosError::InsecureEndpoint)?;
    match url.scheme() {
        "https" => {}
        "http" if allow_insecure_http || is_local_host(host) => {}
        _ => return Err(DgosError::InsecureEndpoint),
    }
    Ok(trimmed.trim_end_matches('/').to_owned())
}

/// Loopback, link-local and private literal addresses, plus the name `localhost`.
pub fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenAIAdapter, ProviderAdapter};

    #[test]
    fn endpoint_policy_accepts_https_and_normalises_the_trailing_slash() {
        assert_eq!(
            check_endpoint("https://api.example.com/v1/", false).unwrap(),
            "https://api.example.com/v1"
        );
        assert_eq!(
            check_endpoint("  https://api.example.com/v1  ", false).unwrap(),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn endpoint_policy_rejects_every_scheme_other_than_http_and_https() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/v1",
            "javascript:alert(1)",
            "localhost:8080/v1",
            "example.com/v1",
            "",
            "https://",
        ] {
            assert!(
                matches!(check_endpoint(url, true), Err(DgosError::InsecureEndpoint)),
                "{url} must be rejected even with allow_insecure_http"
            );
        }
    }

    #[test]
    fn endpoint_policy_allows_plain_http_for_loopback_and_private_literals() {
        for url in [
            "http://localhost:11434/v1",
            "http://LOCALHOST/v1",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:8080/v1",
            "http://10.1.2.3/v1",
            "http://172.16.0.1/v1",
            "http://172.31.255.255/v1",
            "http://192.168.1.20/v1",
            "http://169.254.1.1/v1",
            "http://[fd12:3456::1]/v1",
            "http://[fe80::1]/v1",
        ] {
            assert!(check_endpoint(url, false).is_ok(), "{url}");
        }
    }

    #[test]
    fn endpoint_policy_rejects_plain_http_for_public_hosts_unless_opted_in() {
        for url in [
            "http://example.com/v1",
            "http://ollama.lan/v1",
            "http://172.32.0.1/v1",
            "http://172.15.0.1/v1",
            "http://8.8.8.8/v1",
            "http://127.0.0.1.evil.example/v1",
            "http://localhost.evil.example/v1",
            "http://[2001:db8::1]/v1",
        ] {
            assert!(
                matches!(check_endpoint(url, false), Err(DgosError::InsecureEndpoint)),
                "{url}"
            );
            assert!(check_endpoint(url, true).is_ok(), "{url} with opt-in");
        }
    }

    #[tokio::test]
    async fn endpoint_policy_blocks_a_saved_insecure_config_before_any_request() {
        for base in ["http://example.com/v1", "file:///etc/passwd"] {
            let adapter = OpenAIAdapter::new(base.into(), "synthetic-key".into(), None);
            assert!(matches!(
                adapter.list_models().await,
                Err(DgosError::InsecureEndpoint)
            ));
            assert!(matches!(
                adapter.test_connection(None).await,
                Err(DgosError::InsecureEndpoint)
            ));
        }
    }
}
