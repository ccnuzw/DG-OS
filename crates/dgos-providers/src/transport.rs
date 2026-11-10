//! HTTP plumbing shared by the protocol adapters: client without redirects, profile-driven
//! authentication headers, one logical request with a single parameter downgrade, and the
//! model-list GET. Everything protocol specific (bodies, parsing) stays in the adapters.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dgos_types::{DgosError, DgosResult, RetryConfig};
use reqwest::header::HeaderValue;
use reqwest::{redirect, Client, RequestBuilder, Response};
use tokio::sync::Mutex;
use tracing::debug;
use zeroize::Zeroizing;

use crate::endpoint::check_endpoint;
use crate::retry::{AttemptError, RetryPolicy};
use crate::upstream::{
    apply_downgrade, classify_status, named_downgrade, read_error_body, read_limited, retry_after,
    MAX_BODY_BYTES, MAX_EVENT_BYTES,
};
use crate::vendor::{AuthStyle, Profile};

/// Timeouts and limits (§2.4.6 #3). The model-list cap is the host's own choice: the spec asks
/// for a cap but gives no number.
#[derive(Debug, Clone)]
pub struct AdapterOptions {
    pub allow_insecure_http: bool,
    pub connect_timeout: Duration,
    pub models_timeout: Duration,
    /// Whole-exchange deadline of a non-streaming call. Never applied to a stream.
    pub request_timeout: Duration,
    /// Longest gap between two reads of a stream (and the wait for its response headers).
    pub stream_idle_timeout: Duration,
    pub max_body_bytes: usize,
    pub max_event_bytes: usize,
    pub max_models: usize,
    /// The user's `auth_style` override; the profile decides when `None`.
    pub auth_style: Option<AuthStyle>,
    /// `anthropic-workspace-id`, sent only under an official Anthropic profile.
    pub workspace_id: Option<String>,
}

impl Default for AdapterOptions {
    fn default() -> Self {
        Self {
            allow_insecure_http: false,
            connect_timeout: Duration::from_secs(10),
            models_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(120),
            stream_idle_timeout: Duration::from_secs(120),
            max_body_bytes: MAX_BODY_BYTES,
            max_event_bytes: MAX_EVENT_BYTES,
            max_models: 5000,
            auth_style: None,
            workspace_id: None,
        }
    }
}

/// Request body plus the "already downgraded" flag, shared by every attempt of one call so a
/// downgrade happens at most once per call and is never stored anywhere else (B01-71).
pub struct BodyState {
    body: serde_json::Value,
    downgraded: bool,
}

impl BodyState {
    pub fn new(body: serde_json::Value) -> Mutex<Self> {
        Mutex::new(Self {
            body,
            downgraded: false,
        })
    }
}

pub struct Transport {
    base_url: String,
    api_key: Zeroizing<String>,
    /// `None` only if the TLS backend failed to initialise; no fallback client is used because
    /// it would follow redirects.
    client: Option<Client>,
    pub retry: RetryPolicy,
    pub options: AdapterOptions,
    pub profile: &'static Profile,
    downgradable: &'static [&'static str],
}

/// Secure Transport lazily initializes CoreFoundation bundle preferences on its first context.
/// That synchronous work can scan the executable's directory for seconds and cannot be
/// interrupted by the connector's async timeout. Keep it in client construction instead.
#[cfg(target_os = "macos")]
fn initialize_native_tls() -> Option<()> {
    use security_framework::secure_transport::{SslConnectionType, SslContext, SslProtocolSide};
    use std::sync::LazyLock;

    static INITIALIZED: LazyLock<bool> = LazyLock::new(|| {
        SslContext::new(SslProtocolSide::CLIENT, SslConnectionType::STREAM).is_ok()
    });
    INITIALIZED.then_some(())
}

/// The one way an HTTP client is built: redirects are never followed, so a 3xx is final and the
/// credential only goes to the configured host (B01-68). `None` only if the TLS backend failed to
/// initialise. Building one can stall for seconds (one build took about 3.1 s on macOS), so the
/// manager builds it once and shares it; a `reqwest::Client` clone shares its connection pool.
pub(crate) fn http_client(connect_timeout: Duration) -> Option<Client> {
    #[cfg(target_os = "macos")]
    initialize_native_tls()?;
    Client::builder()
        .redirect(redirect::Policy::none())
        .connect_timeout(connect_timeout)
        .build()
        .ok()
}

/// Loopback is a local service boundary, never an upstream proxy destination.
/// Keep remote/system-proxy transport unchanged; private LAN addresses are not loopback.
pub(crate) fn loopback_endpoint(base_url: &str) -> bool {
    reqwest::Url::parse(base_url.trim()).is_ok_and(|url| {
        url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        })
    })
}

pub(crate) fn direct_http_client(connect_timeout: Duration) -> Option<Client> {
    #[cfg(target_os = "macos")]
    initialize_native_tls()?;
    Client::builder().no_proxy().redirect(redirect::Policy::none()).connect_timeout(connect_timeout).build().ok()
}

impl Transport {
    /// Builds its own client. Adapters made directly (tests, examples) use this.
    pub fn new(
        base_url: String,
        api_key: String,
        retry_config: Option<RetryConfig>,
        options: AdapterOptions,
        profile: &'static Profile,
        downgradable: &'static [&'static str],
    ) -> Self {
        let client = if loopback_endpoint(&base_url) { direct_http_client(options.connect_timeout) } else { http_client(options.connect_timeout) };
        Self::with_client(
            client,
            base_url,
            api_key,
            retry_config,
            options,
            profile,
            downgradable,
        )
    }

    /// Uses a client built by [`http_client`] elsewhere. Its connect timeout is fixed when it
    /// is built, so `options.connect_timeout` must be the value it was built with.
    pub(crate) fn with_client(
        client: Option<Client>,
        base_url: String,
        api_key: String,
        retry_config: Option<RetryConfig>,
        options: AdapterOptions,
        profile: &'static Profile,
        downgradable: &'static [&'static str],
    ) -> Self {
        let api_key = Zeroizing::new(api_key);
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: Zeroizing::new(api_key.trim().to_string()),
            client,
            retry: RetryPolicy::new(retry_config.unwrap_or_default()),
            options,
            profile,
            downgradable,
        }
    }

    /// The endpoint policy runs here, before every call, not only when a config is saved.
    pub fn url(&self, path: &str) -> DgosResult<String> {
        let base = check_endpoint(&self.base_url, self.options.allow_insecure_http)?;
        Ok(format!("{base}{path}"))
    }

    pub fn client(&self) -> DgosResult<&Client> {
        self.client
            .as_ref()
            .ok_or_else(|| DgosError::InternalError("HTTP client unavailable".into()))
    }

    /// Credential and protocol headers. The profile picks the shape; the secret is only read
    /// here and both authentication headers are marked sensitive.
    fn authorize(&self, mut request: RequestBuilder) -> DgosResult<RequestBuilder> {
        let style = self.options.auth_style.unwrap_or(self.profile.auth);
        let secret = self.api_key.as_str();
        if style.sends_bearer() {
            let mut value = HeaderValue::from_str(&format!("Bearer {secret}"))
                .map_err(|_| DgosError::InvalidApiKey)?;
            value.set_sensitive(true);
            request = request.header(reqwest::header::AUTHORIZATION, value);
        }
        if style.sends_x_api_key() {
            let mut value = HeaderValue::from_str(secret).map_err(|_| DgosError::InvalidApiKey)?;
            value.set_sensitive(true);
            request = request.header("x-api-key", value);
        }
        if let Some(version) = self.profile.version_header {
            request = request.header("anthropic-version", version);
        }
        if self.profile.official {
            if let Some(workspace) = &self.options.workspace_id {
                request = request.header("anthropic-workspace-id", workspace.as_str());
            }
        }
        Ok(request)
    }

    /// One logical request: sends the body, and on a 400 that names a downgradable parameter
    /// retries once without it. Other failures are classified, never echoed.
    ///
    /// `timeout` bounds the whole exchange (non-streaming). `idle` bounds the wait for response
    /// headers of a stream; hitting it is final, not retried.
    /// `dispatched` is sticky send-attempt evidence, not proof of receipt. Request/header/body
    /// construction failures leave it untouched; every outcome after `execute` starts retains it.
    pub async fn post_json(
        &self,
        url: &str,
        state: &Mutex<BodyState>,
        model_id: &str,
        timeout: Option<Duration>,
        idle: Option<Duration>,
        dispatched: Option<&AtomicBool>,
    ) -> Result<Response, AttemptError> {
        let client = self.client()?;
        loop {
            let body = state.lock().await.body.clone();
            let mut request = self.authorize(client.post(url).json(&body))?;
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            let request = request.build().map_err(DgosError::from)?;
            let send = async {
                if let Some(dispatched) = dispatched {
                    dispatched.store(true, Ordering::Release);
                }
                client.execute(request).await
            };
            let sent = match idle {
                Some(idle) => match tokio::time::timeout(idle, send).await {
                    Ok(sent) => sent,
                    Err(_) => return Err(AttemptError::terminal(DgosError::Timeout)),
                },
                None => send.await,
            };
            let response = sent.map_err(|error| AttemptError::new(DgosError::from(error)))?;
            if response.status().is_success() {
                return Ok(response);
            }
            let status = response.status().as_u16();
            let retry_after = retry_after(response.headers());
            let error_body = if (300..400).contains(&status) {
                Vec::new()
            } else {
                read_error_body(response).await
            };
            if status == 400 {
                let mut state = state.lock().await;
                if !state.downgraded {
                    if let Some(param) =
                        named_downgrade(&error_body, &state.body, self.downgradable)
                    {
                        debug!(
                            param,
                            "Retrying once without a parameter the upstream rejected"
                        );
                        apply_downgrade(&mut state.body, param);
                        state.downgraded = true;
                        continue;
                    }
                }
            }
            return Err(classify_status(status, &error_body, model_id, retry_after));
        }
    }

    /// One model-list page. `Ok(None)`: the service has no such endpoint (404 or 405).
    pub async fn get_models_page(&self, url: &str) -> DgosResult<Option<Vec<u8>>> {
        let response = self
            .authorize(self.client()?.get(url))?
            .timeout(self.options.models_timeout)
            .send()
            .await?;
        let status = response.status().as_u16();
        if matches!(status, 404 | 405) {
            return Ok(None);
        }
        if !response.status().is_success() {
            let retry_after = retry_after(response.headers());
            let body = if (300..400).contains(&status) {
                Vec::new()
            } else {
                read_error_body(response).await
            };
            return Err(classify_status(status, &body, "", retry_after).error);
        }
        read_limited(response, self.options.max_body_bytes)
            .await
            .map(Some)
    }
}

#[cfg(test)]
mod routing_tests {
    use super::loopback_endpoint;
    #[test]
    fn loopback_routing_does_not_bypass_proxy_for_private_or_similar_named_hosts() {
        for url in ["http://127.0.0.1:1", "http://127.13.2.3", "https://[::1]", "http://LOCALHOST:1"] { assert!(loopback_endpoint(url), "{url}"); }
        for url in ["http://10.0.0.1", "http://192.168.1.1", "https://localhost.example", "http://127.0.0.1.example", "https://api.example", "invalid"] { assert!(!loopback_endpoint(url), "{url}"); }
    }

    #[tokio::test]
    async fn dispatch_signal_stays_false_for_auth_and_request_builder_rejection() {
        use super::{AdapterOptions, BodyState, Transport};
        use std::sync::atomic::{AtomicBool, Ordering};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        for invalid_auth in [true, false] {
            let transport = Transport::new(
                endpoint.clone(),
                if invalid_auth { "synthetic\ninvalid" } else { "synthetic-valid" }.into(),
                None,
                AdapterOptions {
                    workspace_id: (!invalid_auth).then(|| "invalid\nworkspace".into()),
                    ..Default::default()
                },
                &crate::vendor::ANTHROPIC_OFFICIAL,
                crate::upstream::ANTHROPIC_DOWNGRADABLE,
            );
            let url = transport.url("/v1/messages").unwrap();
            let state = BodyState::new(serde_json::json!({"messages": []}));
            let dispatched = AtomicBool::new(false);
            let result = transport.post_json(&url, &state, "fixture-chat", None, None, Some(&dispatched)).await;
            assert!(result.is_err(), "invalid local headers unexpectedly passed validation");
            assert!(!dispatched.load(Ordering::Acquire), "invalid headers must not claim a transport send attempt");
        }
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept()).await.is_err());
    }
}
