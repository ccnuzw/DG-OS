//! `network.local` for module runtimes (D093 item 5).
//!
//! Only HTTP(S) to the loopback interface (`127.0.0.0/8`, `::1`, `localhost`),
//! methods GET and POST, no redirects, no proxy, no cookies and no host
//! credentials. The host resolves `localhost` itself, refuses the call when
//! any resolved address is not loopback and pins the connection to the
//! validated addresses, so a later DNS answer cannot redirect it. Request and
//! response bodies are capped at 1 MiB and the whole call at 30 seconds.
//!
//! Layers 1–3 (identity, manifest, persisted grant) and the settle-once
//! deadline are owned by `ModuleCapabilityDispatchService::run_bound`, which
//! the `module_network_request` command calls with `execute_network_request`
//! as the operation; this module only decides what a permitted request may
//! do. Neither the URL, the headers nor either body ever reaches a log line
//! or an error message.

use base64::Engine;
use dgos_types::{DgosError, DgosResult};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::net::{Ipv6Addr, SocketAddr};
use std::time::Duration;
use url::{Host, Url};

/// Total host-owned deadline of one `network.local` call (D093).
pub const NETWORK_LOCAL_DEADLINE: Duration = Duration::from_secs(30);
/// Largest accepted request body, in bytes.
pub const MAX_NETWORK_REQUEST_BODY_BYTES: usize = 1024 * 1024;
/// Largest accepted response body, in bytes.
pub const MAX_NETWORK_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

/// Fixed text of every transport failure; it never carries the URL or a body.
const NETWORK_FAILED: &str = "Local network request failed";

/// The business fields of `host.network.request`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleNetworkRequestInput {
    pub url: String,
    /// `"GET"` (default) or `"POST"`.
    pub method: Option<String>,
    /// Only `content-type` and `accept`.
    pub headers: Option<BTreeMap<String, String>>,
    pub body: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleNetworkBodyEncoding {
    Text,
    Base64,
}

#[derive(Serialize)]
pub struct ModuleNetworkBody {
    pub encoding: ModuleNetworkBodyEncoding,
    pub data: String,
}

#[derive(Serialize)]
pub struct ModuleNetworkResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: ModuleNetworkBody,
}

impl fmt::Debug for ModuleNetworkResponse {
    /// The body is the module's data, not diagnostic text: only its size is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleNetworkResponse")
            .field("status", &self.status)
            .field("encoding", &self.body.encoding)
            .field("data_len", &self.body.data.len())
            .finish()
    }
}

fn invalid() -> DgosError {
    DgosError::ModuleCapabilityInputInvalid
}

fn network_failed() -> DgosError {
    DgosError::NetworkError(NETWORK_FAILED.to_owned())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    /// A loopback IP literal; reqwest connects to it without any lookup.
    Literal,
    /// `localhost`, resolved and validated by the host.
    Localhost,
}

struct Prepared {
    url: Url,
    target: Target,
    port: u16,
    method: Method,
    headers: HeaderMap,
    body: Option<String>,
}

fn classify_host(url: &Url) -> DgosResult<Target> {
    match url.host() {
        None => Err(invalid()),
        Some(Host::Domain(domain)) if domain.eq_ignore_ascii_case("localhost") => {
            Ok(Target::Localhost)
        }
        Some(Host::Ipv4(ip)) if ip.is_loopback() => Ok(Target::Literal),
        // Only `::1`: IPv4-mapped forms such as `::ffff:127.0.0.1` are refused.
        Some(Host::Ipv6(ip)) if ip == Ipv6Addr::LOCALHOST => Ok(Target::Literal),
        Some(_) => Err(DgosError::ModuleNetworkTargetRefused),
    }
}

fn build_headers(input: Option<BTreeMap<String, String>>) -> DgosResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in input.unwrap_or_default() {
        let name = if name.eq_ignore_ascii_case("content-type") {
            CONTENT_TYPE
        } else if name.eq_ignore_ascii_case("accept") {
            ACCEPT
        } else {
            return Err(invalid());
        };
        if value.contains(['\r', '\n']) {
            return Err(invalid());
        }
        let value = HeaderValue::from_str(&value).map_err(|_| invalid())?;
        // The same header under two letter cases is ambiguous.
        if headers.insert(name, value).is_some() {
            return Err(invalid());
        }
    }
    Ok(headers)
}

fn prepare(input: ModuleNetworkRequestInput) -> DgosResult<Prepared> {
    let url = Url::parse(&input.url).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid());
    }
    let method = match input.method.as_deref() {
        None | Some("GET") => Method::GET,
        Some("POST") => Method::POST,
        Some(_) => return Err(invalid()),
    };
    let headers = build_headers(input.headers)?;
    if let Some(body) = &input.body {
        if method == Method::GET || body.len() > MAX_NETWORK_REQUEST_BODY_BYTES {
            return Err(invalid());
        }
    }
    let target = classify_host(&url)?;
    let port = url.port_or_known_default().ok_or_else(invalid)?;
    Ok(Prepared {
        url,
        target,
        port,
        method,
        headers,
        body: input.body,
    })
}

/// The addresses a connection to `localhost` may use. Any non-loopback answer
/// refuses the whole call: the connection is never allowed to pick among
/// addresses the host did not validate.
fn validate_resolved(addresses: &[SocketAddr]) -> DgosResult<Vec<SocketAddr>> {
    if addresses.is_empty() {
        return Err(network_failed());
    }
    if addresses.iter().any(|address| !address.ip().is_loopback()) {
        return Err(DgosError::ModuleNetworkTargetRefused);
    }
    Ok(addresses.to_vec())
}

/// Validates and performs one request. Called as the operation of
/// `run_bound`, i.e. after authorization and under the host deadline, so an
/// unauthorized module learns nothing about what would have been accepted.
pub async fn execute_network_request(
    input: ModuleNetworkRequestInput,
) -> DgosResult<ModuleNetworkResponse> {
    let prepared = prepare(input)?;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    if prepared.target == Target::Localhost {
        let resolved: Vec<SocketAddr> = tokio::net::lookup_host(("localhost", prepared.port))
            .await
            .map_err(|_| network_failed())?
            .collect();
        let pinned = validate_resolved(&resolved)?;
        builder = builder.resolve_to_addrs("localhost", &pinned);
    }
    let client = builder.build().map_err(|_| network_failed())?;

    let mut request = client
        .request(prepared.method, prepared.url)
        .headers(prepared.headers);
    if let Some(body) = prepared.body {
        request = request.body(body);
    }
    let mut response = request.send().await.map_err(|_| network_failed())?;

    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if response
        .content_length()
        .is_some_and(|length| length > MAX_NETWORK_RESPONSE_BODY_BYTES as u64)
    {
        return Err(DgosError::ModuleNetworkResponseTooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| network_failed())? {
        if bytes.len() + chunk.len() > MAX_NETWORK_RESPONSE_BODY_BYTES {
            return Err(DgosError::ModuleNetworkResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }

    let body = match String::from_utf8(bytes) {
        Ok(text) => ModuleNetworkBody {
            encoding: ModuleNetworkBodyEncoding::Text,
            data: text,
        },
        Err(error) => ModuleNetworkBody {
            encoding: ModuleNetworkBodyEncoding::Base64,
            data: base64::engine::general_purpose::STANDARD.encode(error.into_bytes()),
        },
    };
    Ok(ModuleNetworkResponse {
        status,
        content_type,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::execute_network_request as execute;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    const SECRET_PATH: &str = "secret-path-5d1c";
    const SECRET_BODY: &str = "SECRET-REQUEST-BODY-83ab";

    enum Reply {
        Bytes(Vec<u8>),
        Hang,
    }

    /// `Connection: close` response with an exact `Content-Length`.
    fn http(status: &str, headers: &[&str], body: &[u8]) -> Reply {
        let mut out = format!("HTTP/1.1 {status}\r\n");
        for header in headers {
            out.push_str(header);
            out.push_str("\r\n");
        }
        out.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(body);
        Reply::Bytes(bytes)
    }

    /// Local HTTP fixture on 127.0.0.1 that records every raw request.
    struct Server {
        port: u16,
        connections: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<String>>>,
        task: JoinHandle<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl Server {
        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.port)
        }

        fn localhost_url(&self, path: &str) -> String {
            format!("http://localhost:{}{path}", self.port)
        }

        fn connections(&self) -> usize {
            self.connections.load(Ordering::SeqCst)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let head_end = buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|at| at + 4);
            if let Some(head_end) = head_end {
                let head = String::from_utf8_lossy(&buffer[..head_end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buffer.len() >= head_end + length {
                    return buffer;
                }
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return buffer,
                Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            }
        }
    }

    async fn serve<F>(reply: F) -> Server
    where
        F: Fn(&str) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let reply = Arc::new(reply);
        let task = {
            let connections = connections.clone();
            let requests = requests.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    connections.fetch_add(1, Ordering::SeqCst);
                    let reply = reply.clone();
                    let requests = requests.clone();
                    tokio::spawn(async move {
                        let raw = read_request(&mut stream).await;
                        let text = String::from_utf8_lossy(&raw).into_owned();
                        requests.lock().unwrap().push(text.clone());
                        match reply(&text) {
                            Reply::Bytes(bytes) => {
                                let _ = stream.write_all(&bytes).await;
                                let _ = stream.shutdown().await;
                            }
                            Reply::Hang => tokio::time::sleep(Duration::from_secs(60)).await,
                        }
                    });
                }
            })
        };
        Server {
            port,
            connections,
            requests,
            task,
        }
    }

    fn get(url: impl Into<String>) -> ModuleNetworkRequestInput {
        ModuleNetworkRequestInput {
            url: url.into(),
            method: None,
            headers: None,
            body: None,
        }
    }

    fn post(url: impl Into<String>, body: &str) -> ModuleNetworkRequestInput {
        ModuleNetworkRequestInput {
            url: url.into(),
            method: Some("POST".into()),
            headers: None,
            body: Some(body.into()),
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> Option<BTreeMap<String, String>> {
        Some(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }

    fn error_of(result: DgosResult<ModuleNetworkResponse>) -> DgosError {
        match result {
            Ok(response) => panic!("expected an error, got {response:?}"),
            Err(error) => error,
        }
    }

    #[tokio::test]
    async fn get_and_post_reach_loopback_by_literal_and_by_localhost() {
        let server = serve(|request| {
            if request.starts_with("POST ") {
                http("201 Created", &["Content-Type: text/plain"], b"posted")
            } else {
                http(
                    "200 OK",
                    &["Content-Type: application/json; charset=utf-8"],
                    br#"{"ok":true}"#,
                )
            }
        })
        .await;

        let response = execute(get(server.url("/status"))).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(
            response.content_type.as_deref(),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(response.body.encoding, ModuleNetworkBodyEncoding::Text);
        assert_eq!(response.body.data, r#"{"ok":true}"#);

        let response = execute(post(server.localhost_url("/submit"), "hello"))
            .await
            .unwrap();
        assert_eq!(response.status, 201);
        assert_eq!(response.body.data, "posted");

        let response = execute(get(format!("http://[::1]:{}/", server.port))).await;
        // The fixture listens on IPv4 only: the literal is allowed, the
        // connection itself fails with the fixed transport message.
        assert!(matches!(
            &response,
            Err(DgosError::NetworkError(message)) if message == NETWORK_FAILED
        ));

        let seen = server.requests();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("GET /status HTTP/1.1\r\n"));
        assert!(seen[1].starts_with("POST /submit HTTP/1.1\r\n"));
        assert!(seen[1].ends_with("hello"));
    }

    #[tokio::test]
    async fn requests_carry_no_ambient_credentials_and_only_allowed_headers() {
        let server = serve(|_| http("200 OK", &[], b"ok")).await;
        let mut request = post(server.url("/"), "{}");
        request.headers = headers(&[
            ("Content-Type", "application/json"),
            ("ACCEPT", "application/json"),
        ]);
        execute(request).await.unwrap();
        let seen = server.requests()[0].to_ascii_lowercase();
        assert!(seen.contains("content-type: application/json\r\n"));
        assert!(seen.contains("accept: application/json\r\n"));
        for forbidden in [
            "cookie:",
            "authorization:",
            "proxy-authorization:",
            "referer:",
        ] {
            assert!(!seen.contains(forbidden), "sent {forbidden}");
        }
    }

    #[tokio::test]
    async fn a_request_body_of_exactly_one_mebibyte_is_accepted() {
        let server = serve(|_| http("200 OK", &[], b"ok")).await;
        let body = "a".repeat(MAX_NETWORK_REQUEST_BODY_BYTES);
        let response = execute(post(server.url("/"), &body)).await.unwrap();
        assert_eq!(response.status, 200);
        assert!(server.requests()[0].ends_with(&body));
    }

    #[tokio::test]
    async fn non_loopback_targets_are_refused_before_any_connection() {
        let server = serve(|_| http("200 OK", &[], b"ok")).await;
        let port = server.port;
        for url in [
            "http://10.0.0.1/".to_owned(),
            "http://192.168.1.1:8080/".to_owned(),
            "http://172.16.0.1/".to_owned(),
            "http://8.8.8.8/".to_owned(),
            "http://0.0.0.0/".to_owned(),
            "https://example.com/".to_owned(),
            "http://localhost.example.com/".to_owned(),
            format!("http://127.0.0.1.nip.io:{port}/"),
            format!("http://foo.localhost:{port}/"),
            "http://[::ffff:8.8.8.8]/".to_owned(),
            format!("http://[::ffff:127.0.0.1]:{port}/"),
            "http://[::2]/".to_owned(),
            "http://[fe80::1]/".to_owned(),
        ] {
            let result = tokio::time::timeout(Duration::from_secs(2), execute(get(url.clone())))
                .await
                .unwrap_or_else(|_| panic!("{url} was not refused immediately"));
            assert!(
                matches!(result, Err(DgosError::ModuleNetworkTargetRefused)),
                "{url} was not refused"
            );
        }
        assert_eq!(server.connections(), 0);
    }

    #[tokio::test]
    async fn malformed_urls_schemes_userinfo_and_hostless_urls_are_invalid() {
        let server = serve(|_| http("200 OK", &[], b"ok")).await;
        let port = server.port;
        for url in [
            String::new(),
            "not a url".to_owned(),
            "file:///etc/passwd".to_owned(),
            "ftp://127.0.0.1/".to_owned(),
            "ws://127.0.0.1/".to_owned(),
            "data:text/plain,hi".to_owned(),
            "http:///".to_owned(),
            format!("http://user@127.0.0.1:{port}/"),
            format!("http://user:pw@127.0.0.1:{port}/"),
            format!("http://127.0.0.1:{port}@example.com/"),
            format!("http://example.com@127.0.0.1:{port}/"),
        ] {
            let result = execute(get(url.clone())).await;
            assert!(
                matches!(result, Err(DgosError::ModuleCapabilityInputInvalid)),
                "{url} was not invalid"
            );
        }
        assert_eq!(server.connections(), 0);
    }

    #[tokio::test]
    async fn methods_headers_and_bodies_outside_the_contract_are_invalid() {
        let server = serve(|_| http("200 OK", &[], b"ok")).await;
        let url = server.url("/");

        for method in ["PUT", "DELETE", "HEAD", "PATCH", "get", "", "CONNECT"] {
            let mut request = get(url.clone());
            request.method = Some(method.to_owned());
            assert!(
                matches!(
                    execute(request).await,
                    Err(DgosError::ModuleCapabilityInputInvalid)
                ),
                "method {method:?}"
            );
        }

        for pairs in [
            &[("authorization", "Bearer x")][..],
            &[("Cookie", "a=b")],
            &[("host", "example.com")],
            &[("x-custom", "1")],
            &[("content-length", "3")],
            &[("accept", "a\r\nX-Injected: 1")],
            &[("accept", "a\nb")],
            &[("content-type", "a\rb")],
            &[("accept", "a\u{0}b")],
            &[("Accept", "a"), ("accept", "b")],
        ] {
            let mut request = get(url.clone());
            request.headers = headers(pairs);
            assert!(
                matches!(
                    execute(request).await,
                    Err(DgosError::ModuleCapabilityInputInvalid)
                ),
                "headers {pairs:?}"
            );
        }

        // A body on GET is invalid, even an empty one.
        for body in ["x", ""] {
            let mut request = get(url.clone());
            request.body = Some(body.to_owned());
            assert!(matches!(
                execute(request).await,
                Err(DgosError::ModuleCapabilityInputInvalid)
            ));
        }
        let oversized = "a".repeat(MAX_NETWORK_REQUEST_BODY_BYTES + 1);
        assert!(matches!(
            execute(post(url, &oversized)).await,
            Err(DgosError::ModuleCapabilityInputInvalid)
        ));
        assert_eq!(server.connections(), 0);
    }

    #[test]
    fn unknown_request_fields_are_rejected() {
        let parsed = serde_json::from_value::<ModuleNetworkRequestInput>(serde_json::json!({
            "url": "http://127.0.0.1/",
            "proxy": "http://example.com"
        }));
        assert!(parsed.is_err());
        let parsed = serde_json::from_value::<ModuleNetworkRequestInput>(serde_json::json!({
            "url": "http://127.0.0.1/",
            "method": "POST",
            "headers": {"accept": "text/plain"},
            "body": "x"
        }));
        assert!(parsed.is_ok());
    }

    #[test]
    fn every_resolved_address_must_be_loopback() {
        let addr = |text: &str| text.parse::<SocketAddr>().unwrap();
        let both = [addr("[::1]:80"), addr("127.0.0.1:80")];
        assert_eq!(validate_resolved(&both).unwrap(), both.to_vec());
        for mixed in [
            vec![addr("127.0.0.1:80"), addr("10.0.0.1:80")],
            vec![addr("10.0.0.1:80")],
            vec![addr("[::1]:80"), addr("8.8.8.8:80")],
            vec![addr("[::ffff:127.0.0.1]:80")],
        ] {
            assert!(matches!(
                validate_resolved(&mixed),
                Err(DgosError::ModuleNetworkTargetRefused)
            ));
        }
        assert!(matches!(
            validate_resolved(&[]),
            Err(DgosError::NetworkError(_))
        ));
    }

    #[tokio::test]
    async fn redirects_are_returned_as_is_and_not_followed() {
        let server = serve(|request| {
            if request.starts_with("GET /start ") {
                http("302 Found", &["Location: /elsewhere"], b"")
            } else {
                http("200 OK", &[], b"followed")
            }
        })
        .await;
        let response = execute(get(server.url("/start"))).await.unwrap();
        assert_eq!(response.status, 302);
        assert_eq!(response.body.data, "");
        assert_eq!(server.requests().len(), 1);

        // Not even to a non-loopback Location.
        let server = serve(|_| http("301 Moved", &["Location: http://10.0.0.1/"], b"")).await;
        let response = execute(get(server.url("/"))).await.unwrap();
        assert_eq!(response.status, 301);
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn responses_over_one_mebibyte_are_refused() {
        // Declared length over the cap.
        let server = serve(|_| {
            http(
                "200 OK",
                &[],
                &vec![b'a'; MAX_NETWORK_RESPONSE_BODY_BYTES + 1],
            )
        })
        .await;
        assert!(matches!(
            execute(get(server.url("/"))).await,
            Err(DgosError::ModuleNetworkResponseTooLarge)
        ));

        // No declared length: the cap applies while streaming.
        let server = serve(|_| {
            let mut bytes = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
            bytes.extend(vec![b'a'; MAX_NETWORK_RESPONSE_BODY_BYTES + 1]);
            Reply::Bytes(bytes)
        })
        .await;
        assert!(matches!(
            execute(get(server.url("/"))).await,
            Err(DgosError::ModuleNetworkResponseTooLarge)
        ));

        // Exactly the cap is fine.
        let server =
            serve(|_| http("200 OK", &[], &vec![b'a'; MAX_NETWORK_RESPONSE_BODY_BYTES])).await;
        let response = execute(get(server.url("/"))).await.unwrap();
        assert_eq!(response.body.data.len(), MAX_NETWORK_RESPONSE_BODY_BYTES);
    }

    #[tokio::test]
    async fn non_utf8_bodies_are_returned_as_base64() {
        let raw = [0xffu8, 0xfe, 0x00, 0x80, b'a'];
        let server =
            serve(move |_| http("200 OK", &["Content-Type: application/octet-stream"], &raw)).await;
        let response = execute(get(server.url("/"))).await.unwrap();
        assert_eq!(response.body.encoding, ModuleNetworkBodyEncoding::Base64);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&response.body.data)
                .unwrap(),
            raw
        );
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["body"]["encoding"], "base64");
        assert_eq!(json["content_type"], "application/octet-stream");

        let server = serve(|_| http("204 No Content", &[], b"")).await;
        let response = execute(get(server.url("/"))).await.unwrap();
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["status"], 204);
        assert!(json["content_type"].is_null());
        assert_eq!(json["body"]["encoding"], "text");
    }

    #[tokio::test]
    async fn transport_failures_use_a_fixed_message_without_url_or_body() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let mut request = post(
            format!("http://127.0.0.1:{port}/{SECRET_PATH}"),
            SECRET_BODY,
        );
        request.headers = headers(&[("accept", "text/plain")]);
        let error = error_of(execute(request).await);
        assert!(matches!(
            &error,
            DgosError::NetworkError(message) if message == NETWORK_FAILED
        ));
        let text = format!("{error} {error:?}");
        for secret in [SECRET_PATH, SECRET_BODY, &port.to_string(), "127.0.0.1"] {
            assert!(!text.contains(secret), "error leaked {secret}");
        }

        // The server closes without answering.
        let server = serve(|_| Reply::Bytes(Vec::new())).await;
        let error =
            error_of(execute(post(server.url(&format!("/{SECRET_PATH}")), SECRET_BODY)).await);
        assert!(matches!(
            &error,
            DgosError::NetworkError(message) if message == NETWORK_FAILED
        ));
    }

    #[tokio::test]
    async fn validation_errors_do_not_echo_the_input() {
        let mut request = post(format!("http://10.0.0.1/{SECRET_PATH}"), SECRET_BODY);
        request.headers = headers(&[("x-secret", SECRET_BODY)]);
        let error = error_of(execute(request).await);
        let text = format!("{error} {error:?}");
        assert!(!text.contains(SECRET_PATH) && !text.contains(SECRET_BODY));
        let error = error_of(execute(get(format!("file:///{SECRET_PATH}"))).await);
        assert!(!format!("{error} {error:?}").contains(SECRET_PATH));
    }

    /// The admitted-call path: a real supervisor-issued runtime, a real
    /// `ModuleCallRouter` and SQLite, against the HTTP fixture. Only the
    /// WebView page is scripted.
    mod admitted_call {
        use super::*;
        use crate::ModuleCapabilityDispatchService;
        use crate::{HostCapabilityService, ModuleReleaseService};
        use dgos_db::migrations::Migrator;
        use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
        use dgos_db::sea_orm_migration::MigratorTrait;
        use dgos_db::ModulePermission;
        use dgos_db::ModulePermissionRepository;
        use dgos_modules::BoundModuleCall;
        use dgos_modules::{
            ModuleCallRouter, ModulePageTransport, ModulePublisher, ModuleRuntimeHandle,
            ModuleRuntimeSupervisor, PageInstruction, PageLoaderConfig, PageModuleLoader,
            PageReceipt, PageSendError, PinnedModuleAdmission, RuntimeLost,
        };
        use dgos_types::HostSession;
        use tempfile::{tempdir, TempDir};
        use tokio::sync::mpsc;

        const MODULE: &str = "module-1";

        struct ScriptedPage(mpsc::UnboundedSender<(String, PageInstruction)>);

        impl ModulePageTransport for ScriptedPage {
            fn send(
                &self,
                attachment_id: &str,
                instruction: PageInstruction,
            ) -> Result<(), PageSendError> {
                self.0
                    .send((attachment_id.to_owned(), instruction))
                    .map_err(|_| PageSendError)
            }
        }

        fn package() -> TempDir {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join("entry.js"), "export default '1.0.0';").unwrap();
            // The digest excludes `artifact.sha256`, so write a placeholder first.
            let manifest = |sha256: &str| {
                serde_json::json!({
                    "id": MODULE, "name": "Example", "version": "1.0.0", "entry": "entry.js",
                    "artifact": {"version": "1.0.0", "sha256": sha256},
                    "permissions": ["network.local"]
                })
                .to_string()
            };
            std::fs::write(dir.path().join("manifest.json"), manifest(&"0".repeat(64))).unwrap();
            let integrity = dgos_modules::package_integrity(dir.path()).unwrap();
            let hex = integrity.strip_prefix("v2:").unwrap();
            std::fs::write(dir.path().join("manifest.json"), manifest(hex)).unwrap();
            dir
        }

        struct World {
            _db_dir: TempDir,
            _store: TempDir,
            db: DatabaseConnection,
            permissions: Arc<ModulePermissionRepository>,
            dispatch: ModuleCapabilityDispatchService,
            deadline: Duration,
            supervisor: ModuleRuntimeSupervisor,
            router: ModuleCallRouter,
            attachment_id: String,
            _lost: mpsc::UnboundedReceiver<RuntimeLost>,
            driver: JoinHandle<()>,
        }

        impl Drop for World {
            fn drop(&mut self) {
                self.driver.abort();
            }
        }

        impl World {
            async fn new(deadline: Option<Duration>) -> Self {
                let db_dir = tempdir().unwrap();
                let url = format!(
                    "sqlite://{}?mode=rwc",
                    db_dir.path().join("fixture.db").display()
                );
                let db = dgos_db::establish_connection(&url).await.unwrap();
                Migrator::up(&db, None).await.unwrap();
                let store = tempdir().unwrap();
                let publisher = Arc::new(ModulePublisher::new(store.path(), db.clone()));
                let release = ModuleReleaseService::new(publisher.clone())
                    .publish(package().path())
                    .await
                    .unwrap();

                let (sender, receiver) = mpsc::unbounded_channel();
                let (loader, lost) = PageModuleLoader::new(
                    Arc::new(ScriptedPage(sender)),
                    PageLoaderConfig::default(),
                );
                let attachment_id = loader.attach("main").attachment_id;
                let router = ModuleCallRouter::new(loader.clone());
                let driver = answer_page(loader.clone(), receiver);

                let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
                let dispatch = ModuleCapabilityDispatchService::new(
                    HostCapabilityService::with_publisher(permissions.clone(), publisher.clone()),
                );
                let deadline = deadline.unwrap_or(NETWORK_LOCAL_DEADLINE);
                let admission =
                    PinnedModuleAdmission::new([(MODULE, "1.0.0", release.integrity.as_str())]);
                let supervisor = ModuleRuntimeSupervisor::new(publisher, Arc::new(loader))
                    .with_admission(Arc::new(admission));
                Self {
                    _db_dir: db_dir,
                    _store: store,
                    db,
                    permissions,
                    dispatch,
                    deadline,
                    supervisor,
                    router,
                    attachment_id,
                    _lost: lost,
                    driver,
                }
            }

            async fn start(&self) -> ModuleRuntimeHandle {
                self.supervisor
                    .start(&HostSession::local_default(), MODULE)
                    .await
                    .unwrap()
            }

            async fn grant(&self) {
                self.permissions
                    .grant("default-user", MODULE, ModulePermission::NetworkLocal)
                    .await
                    .unwrap();
            }

            async fn revoke(&self) {
                self.permissions
                    .revoke("default-user", MODULE, ModulePermission::NetworkLocal)
                    .await
                    .unwrap();
            }

            fn admit(&self, handle: &ModuleRuntimeHandle) -> BoundModuleCall {
                let route = self.router.issue(handle).unwrap();
                self.router
                    .admit(route.as_str(), &self.attachment_id)
                    .unwrap()
            }

            async fn call(
                &self,
                handle: &ModuleRuntimeHandle,
                input: ModuleNetworkRequestInput,
            ) -> DgosResult<ModuleNetworkResponse> {
                self.dispatch
                    .run_bound(
                        self.admit(handle),
                        ModulePermission::NetworkLocal,
                        self.deadline,
                        |_grant| execute_network_request(input),
                    )
                    .await
            }

            /// `permission` of every audit row with the given action.
            async fn audited(&self, action: &str) -> Vec<String> {
                self.db
                    .query_all(Statement::from_string(
                        self.db.get_database_backend(),
                        format!(
                            "SELECT permission FROM module_permission_audits WHERE action = '{action}' AND module_id = '{MODULE}'"
                        ),
                    ))
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| row.try_get::<String>("", "permission").unwrap())
                    .collect()
            }

            async fn audit_dump(&self) -> String {
                self.db
                    .query_all(Statement::from_string(
                        self.db.get_database_backend(),
                        "SELECT user_id, module_id, permission, action, resource_id, request_id FROM module_permission_audits",
                    ))
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| format!("{row:?}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }

        fn answer_page(
            loader: PageModuleLoader,
            mut instructions: mpsc::UnboundedReceiver<(String, PageInstruction)>,
        ) -> JoinHandle<()> {
            tokio::spawn(async move {
                while let Some((attachment, instruction)) = instructions.recv().await {
                    let (ticket, receipt) = match instruction {
                        PageInstruction::Load { ticket, .. } => (ticket, PageReceipt::Loaded),
                        PageInstruction::Unload { ticket, .. } => {
                            (ticket, PageReceipt::Unloaded { hook_failed: false })
                        }
                    };
                    loader.receive(&attachment, &ticket, receipt);
                }
            })
        }

        #[tokio::test]
        async fn a_grant_is_required_for_every_call_and_revocation_hits_the_next_one() {
            let world = World::new(None).await;
            let handle = world.start().await;
            let server = serve(|request| {
                if request.starts_with("POST ") {
                    http("200 OK", &[], b"posted")
                } else {
                    http("200 OK", &[], b"fetched")
                }
            })
            .await;

            // Admitted but not granted (AC04): refused before any connection.
            let denied = world
                .call(&handle, get(server.url(&format!("/{SECRET_PATH}"))))
                .await;
            assert!(matches!(denied, Err(DgosError::PermissionDenied)));
            assert_eq!(server.connections(), 0);
            assert_eq!(world.audited("denied").await, ["network.local"]);

            // Granted (AC03): GET by literal and POST by `localhost`.
            world.grant().await;
            let fetched = world
                .call(&handle, get(server.url(&format!("/{SECRET_PATH}"))))
                .await
                .unwrap();
            assert_eq!(fetched.body.data, "fetched");
            let posted = world
                .call(&handle, post(server.localhost_url("/"), SECRET_BODY))
                .await
                .unwrap();
            assert_eq!(posted.body.data, "posted");
            assert_eq!(server.requests().len(), 2);

            // Revoked (AC07): the next call is denied and never connects.
            world.revoke().await;
            let revoked = world.call(&handle, get(server.url("/"))).await;
            assert!(matches!(revoked, Err(DgosError::PermissionDenied)));
            assert_eq!(server.requests().len(), 2);
            assert_eq!(
                world.audited("denied").await,
                ["network.local", "network.local"]
            );

            let stats = handle.request_stats();
            assert_eq!(
                (
                    stats.admitted,
                    stats.succeeded,
                    stats.failed,
                    stats.denied,
                    stats.in_flight
                ),
                (4, 2, 0, 2, 0)
            );
            // AC05: the audit never records the URL or a body.
            let dump = world.audit_dump().await;
            assert!(!dump.contains(SECRET_PATH) && !dump.contains(SECRET_BODY));
            assert!(!dump.contains("127.0.0.1"));
        }

        #[tokio::test]
        async fn refused_and_invalid_requests_settle_as_failed_and_never_connect() {
            let world = World::new(None).await;
            world.grant().await;
            let handle = world.start().await;
            let server = serve(|_| http("200 OK", &[], b"ok")).await;

            let refused = world.call(&handle, get("http://10.0.0.1/")).await;
            assert!(matches!(
                refused,
                Err(DgosError::ModuleNetworkTargetRefused)
            ));
            let mut bad_header = get(server.url("/"));
            bad_header.headers = headers(&[("authorization", "Bearer x")]);
            assert!(matches!(
                world.call(&handle, bad_header).await,
                Err(DgosError::ModuleCapabilityInputInvalid)
            ));
            assert!(matches!(
                world.call(&handle, get("file:///etc/passwd")).await,
                Err(DgosError::ModuleCapabilityInputInvalid)
            ));
            assert_eq!(server.connections(), 0);

            // A permission refusal is never reported with these codes.
            let stats = handle.request_stats();
            assert_eq!(
                (
                    stats.failed,
                    stats.denied,
                    stats.interrupted,
                    stats.in_flight
                ),
                (3, 0, 0, 0)
            );
        }

        #[tokio::test]
        async fn the_host_deadline_settles_a_slow_request_as_interrupted() {
            let world = World::new(Some(Duration::from_millis(150))).await;
            world.grant().await;
            let handle = world.start().await;
            let server = serve(|_| Reply::Hang).await;

            let result = world.call(&handle, get(server.url("/"))).await;
            assert!(matches!(result, Err(DgosError::Timeout)));
            assert_eq!(server.connections(), 1);
            let stats = handle.request_stats();
            assert_eq!(
                (
                    stats.interrupted,
                    stats.failed,
                    stats.succeeded,
                    stats.in_flight
                ),
                (1, 0, 0, 0)
            );
        }

        #[tokio::test]
        async fn the_production_deadline_is_thirty_seconds() {
            assert_eq!(NETWORK_LOCAL_DEADLINE, Duration::from_secs(30));
            let world = World::new(None).await;
            assert_eq!(world.deadline, Duration::from_secs(30));
        }
    }
}
