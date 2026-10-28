use crate::credentials::SecretToken;
use crate::error::{McpError, Result};
use crate::legacy_sse::LegacySseSession;
use crate::protocol::{ensure_response, server_request_reply, JsonRpcResponse, NegotiatedProtocol};
use reqwest::{Client, Url};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex, OnceLock,
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{mpsc, oneshot, Mutex},
    task::JoinHandle,
    time::{timeout, Duration},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
pub(crate) const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) type Pending = Arc<StdMutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>;

pub enum Session {
    Stdio(StdioSession),
    Http(HttpSession),
}

#[cfg(test)]
pub(crate) fn validate_http_url(url: &str) -> Result<Url> {
    validate_http_url_with_consent(url, false)
}

pub(crate) fn validate_http_url_with_consent(url: &str, allow_insecure_http: bool) -> Result<Url> {
    let parsed = Url::parse(url).map_err(|_| McpError::InvalidConfig("invalid HTTP URL".into()))?;
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.host_str().is_none()
        || !(parsed.scheme() == "https"
            || (parsed.scheme() == "http" && (loopback || allow_insecure_http)))
    {
        return Err(McpError::InvalidConfig(
            "HTTP URL requires HTTPS, exact localhost, or explicit remote plaintext consent; credentials and fragments are forbidden".into(),
        ));
    }
    Ok(parsed)
}

impl Session {
    pub async fn stdio(executable: &str, args: &[String], cwd: Option<&str>) -> Result<Self> {
        validate_executable(executable)?;
        if cwd.is_some_and(|dir| !Path::new(dir).is_dir()) {
            return Err(McpError::InvalidConfig("stdio cwd does not exist".into()));
        }
        let mut command = Command::new(executable);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // Do not expose provider tokens or other host secrets inherited from the app.
        command.env_clear();
        for name in [
            "PATH",
            "HOME",
            "USERPROFILE",
            "SystemRoot",
            "TMPDIR",
            "TEMP",
            "LANG",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        let mut child = command
            .spawn()
            .map_err(|_| McpError::ConnectionFailed("could not start executable".into()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("missing stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("missing stdout".into()))?;
        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let (outbound, mut writes) = mpsc::channel::<Outbound>(32);
        let writer_pending = pending.clone();
        let writer = tokio::spawn(async move {
            while let Some(message) = writes.recv().await {
                // A caller timing out or cancelling must not truncate a JSON
                // frame. The owned writer finishes this frame or fails closed.
                let result = timeout(Duration::from_secs(15), async {
                    stdin.write_all(&message.bytes).await?;
                    stdin.flush().await
                })
                .await;
                let result = match result {
                    Ok(Ok(())) => Ok(()),
                    _ => Err(McpError::Transport("stdio write failed".into())),
                };
                let failed = result.is_err();
                let _ = message.done.send(result);
                if failed {
                    break;
                }
            }
            writer_pending.lock().unwrap().clear();
        });
        let reader_pending = pending.clone();
        let reader_outbound = outbound.clone();
        let reader = tokio::spawn(async move {
            let mut stdout = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                let read = (&mut stdout)
                    .take((MAX_MESSAGE_BYTES + 1) as u64)
                    .read_until(b'\n', &mut line)
                    .await;
                if !matches!(read, Ok(n) if n > 0) || line.len() > MAX_MESSAGE_BYTES {
                    break;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    break;
                };
                if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                    break;
                }
                if value.get("method").is_some() {
                    match server_request_reply(&value) {
                        Ok(Some(reply)) => {
                            let Ok(frame) = outbound_frame(&reply) else {
                                break;
                            };
                            if !matches!(
                                timeout(Duration::from_secs(1), reader_outbound.send(frame)).await,
                                Ok(Ok(()))
                            ) {
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                    continue;
                }
                let Ok(response) = serde_json::from_value::<JsonRpcResponse>(value) else {
                    break;
                };
                if let Some(id) = response.id {
                    if let Some(sender) = reader_pending.lock().unwrap().remove(&id) {
                        let _ = sender.send(response);
                    }
                }
            }
            reader_pending.lock().unwrap().clear();
        });
        Ok(Self::Stdio(StdioSession {
            child: Mutex::new(child),
            outbound,
            writer,
            pending,
            reader,
            negotiated: OnceLock::new(),
        }))
    }

    #[cfg(test)]
    pub fn http(url: String, client: Client, bearer: Option<SecretToken>) -> Result<Self> {
        Self::http_with_consent(url, client, bearer, false)
    }

    pub fn http_with_consent(
        url: String,
        client: Client,
        bearer: Option<SecretToken>,
        allow_insecure_http: bool,
    ) -> Result<Self> {
        validate_http_url_with_consent(&url, allow_insecure_http)?;
        Ok(Self::Http(HttpSession {
            allow_insecure_http,
            url,
            client,
            session_id: Mutex::new(None),
            bearer: bearer.map(|token| Arc::new(token)),
            negotiated: Arc::new(OnceLock::new()),
            legacy: Mutex::new(None),
        }))
    }

    pub fn complete_initialize(&self, result: Value) -> Result<()> {
        let protocol = NegotiatedProtocol::from_initialize(result)?;
        self.negotiated()
            .set(protocol)
            .map_err(|_| McpError::HandshakeFailed("session already initialized".into()))
    }

    fn negotiated(&self) -> &OnceLock<NegotiatedProtocol> {
        match self {
            Self::Stdio(session) => &session.negotiated,
            Self::Http(session) => &session.negotiated,
        }
    }

    pub fn has_tools(&self) -> Result<bool> {
        self.negotiated()
            .get()
            .map(|protocol| protocol.capabilities.contains_key("tools"))
            .ok_or_else(|| McpError::HandshakeFailed("session not initialized".into()))
    }

    pub async fn request<T: Serialize>(
        &self,
        payload: &T,
        timeout_ms: u64,
    ) -> Result<JsonRpcResponse> {
        match self {
            Self::Stdio(session) => session.request(payload, timeout_ms).await,
            Self::Http(session) => session.request(payload, timeout_ms).await,
        }
    }

    pub async fn notify<T: Serialize>(&self, payload: &T) -> Result<()> {
        match self {
            Self::Stdio(session) => session.send(payload).await,
            Self::Http(session) => {
                let response = timeout(Duration::from_secs(15), session.send(payload))
                    .await
                    .map_err(|_| McpError::Timeout)??;
                check_status(response.status())
            }
        }
    }

    pub async fn close(&self) {
        match self {
            Self::Stdio(session) => {
                session.reader.abort();
                session.writer.abort();
                session.pending.lock().unwrap().clear();
                let _ = session.child.lock().await.kill().await;
            }
            Self::Http(session) => {
                if let Some(legacy) = session.legacy.lock().await.take() {
                    legacy.close();
                } else if let Some(id) = session.session_id.lock().await.take() {
                    let mut request = session
                        .client
                        .delete(&session.url)
                        .header("Mcp-Session-Id", id);
                    if let Some(protocol) = session.negotiated.get() {
                        request = request.header("MCP-Protocol-Version", &protocol.version);
                    }
                    if let Some(token) = &session.bearer {
                        request = request.bearer_auth(token.expose());
                    }
                    // DELETE only releases protocol state; never replay a tool
                    // when the peer does not support session termination.
                    let _ = timeout(Duration::from_secs(1), request.send()).await;
                }
            }
        }
    }
}

pub(crate) fn validate_executable(executable: &str) -> Result<()> {
    let name = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if executable.trim().is_empty()
        || matches!(
            name.as_str(),
            "sh" | "bash"
                | "zsh"
                | "fish"
                | "cmd"
                | "cmd.exe"
                | "powershell"
                | "powershell.exe"
                | "pwsh"
                | "pwsh.exe"
        )
    {
        return Err(McpError::InvalidConfig(
            "stdio executable must not be empty or a shell interpreter".into(),
        ));
    }
    Ok(())
}

pub struct StdioSession {
    child: Mutex<Child>,
    outbound: mpsc::Sender<Outbound>,
    writer: JoinHandle<()>,
    pending: Pending,
    reader: JoinHandle<()>,
    negotiated: OnceLock<NegotiatedProtocol>,
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;

    #[tokio::test]
    async fn stdio_close_reaps_child_and_is_idempotent() {
        let session = Session::stdio(
            "node",
            &[format!(
                "{}/tests/fixtures/mcp-server.mjs",
                env!("CARGO_MANIFEST_DIR")
            )],
            None,
        )
        .await
        .unwrap();
        let Session::Stdio(child) = &session else {
            unreachable!()
        };
        assert!(child.child.lock().await.id().is_some());
        session.close().await;
        assert!(child.child.lock().await.try_wait().unwrap().is_some());
        session.close().await;
        assert!(session
            .request(
                &crate::protocol::request(next_id(), "ping", serde_json::json!({})),
                100
            )
            .await
            .is_err());
    }
}

struct Outbound {
    bytes: Vec<u8>,
    done: oneshot::Sender<Result<()>>,
}

fn outbound_frame<T: Serialize>(payload: &T) -> Result<Outbound> {
    let (message, _) = queued_frame(payload)?;
    Ok(message)
}

fn queued_frame<T: Serialize>(payload: &T) -> Result<(Outbound, oneshot::Receiver<Result<()>>)> {
    let mut bytes = serde_json::to_vec(payload)
        .map_err(|_| McpError::Serialization("invalid request".into()))?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(McpError::InvalidConfig(
            "request exceeds message limit".into(),
        ));
    }
    bytes.push(b'\n');
    let (done, written) = oneshot::channel();
    Ok((Outbound { bytes, done }, written))
}

// Removes abandoned requests on timeout, task cancellation, serialization or write failure.
pub(crate) struct PendingGuard {
    pub(crate) id: u64,
    pub(crate) pending: Pending,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
    }
}
impl Drop for StdioSession {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
        self.pending.lock().unwrap().clear();
    }
}

impl StdioSession {
    async fn send<T: Serialize>(&self, payload: &T) -> Result<()> {
        let (frame, written) = queued_frame(payload)?;
        self.outbound
            .send(frame)
            .await
            .map_err(|_| McpError::Transport("stdio writer closed".into()))?;
        written
            .await
            .map_err(|_| McpError::Transport("stdio writer closed".into()))?
    }

    async fn request<T: Serialize>(&self, payload: &T, timeout_ms: u64) -> Result<JsonRpcResponse> {
        let id = request_id(payload)?;
        let operation = async {
            if self.reader.is_finished() {
                return Err(McpError::ConnectionFailed(
                    "stdio process disconnected".into(),
                ));
            }
            let (sender, receiver) = oneshot::channel();
            self.pending.lock().unwrap().insert(id, sender);
            let _guard = PendingGuard {
                id,
                pending: self.pending.clone(),
            };
            self.send(payload).await?;
            receiver
                .await
                .map_err(|_| McpError::ConnectionFailed("stdio response stream closed".into()))
        };
        timeout(Duration::from_millis(timeout_ms), operation)
            .await
            .map_err(|_| McpError::Timeout)?
    }
}

pub struct HttpSession {
    allow_insecure_http: bool,
    url: String,
    client: Client,
    session_id: Mutex<Option<String>>,
    bearer: Option<Arc<SecretToken>>,
    negotiated: Arc<OnceLock<NegotiatedProtocol>>,
    legacy: Mutex<Option<Arc<LegacySseSession>>>,
}

impl HttpSession {
    async fn send<T: Serialize>(&self, payload: &T) -> Result<reqwest::Response> {
        let legacy = self.legacy.lock().await.clone();
        if let Some(legacy) = legacy {
            return legacy.send(payload).await;
        }
        self.send_streamable(payload).await
    }

    async fn send_streamable<T: Serialize>(&self, payload: &T) -> Result<reqwest::Response> {
        let mut request = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .json(payload);
        if let Some(token) = &self.bearer {
            request = request.bearer_auth(token.expose());
        }
        if let Some(session_id) = self.session_id.lock().await.clone() {
            request = request.header("Mcp-Session-Id", session_id);
        }
        if let Some(protocol) = self.negotiated.get() {
            request = request.header("MCP-Protocol-Version", &protocol.version);
        }
        let response = request
            .send()
            .await
            .map_err(|_| McpError::Transport("HTTP request failed".into()))?;
        if response.status().is_success() {
            if let Some(value) = response
                .headers()
                .get("Mcp-Session-Id")
                .and_then(|value| value.to_str().ok())
            {
                *self.session_id.lock().await = Some(value.to_owned());
            }
        }
        Ok(response)
    }

    async fn request<T: Serialize>(&self, payload: &T, timeout_ms: u64) -> Result<JsonRpcResponse> {
        let legacy = self.legacy.lock().await.clone();
        if let Some(legacy) = legacy {
            return legacy.request(payload, timeout_ms).await;
        }
        let id = request_id(payload)?;
        let operation = async {
            let mut response = self.send_streamable(payload).await?;
            // Only initialization can select legacy mode. Never replay tools or
            // downgrade after an authentication failure or ambiguous timeout.
            let initialize = serde_json::to_value(payload).ok().is_some_and(|value| {
                value.get("method").and_then(Value::as_str) == Some("initialize")
            });
            if initialize && matches!(response.status().as_u16(), 400 | 404 | 405) {
                drop(response);
                let legacy = Arc::new(
                    LegacySseSession::connect(
                        &self.url,
                        self.client.clone(),
                        self.bearer.clone(),
                        self.negotiated.clone(),
                        self.allow_insecure_http,
                    )
                    .await?,
                );
                *self.legacy.lock().await = Some(legacy.clone());
                return legacy.request(payload, timeout_ms).await;
            }
            check_status(response.status())?;
            let sse = response
                .headers()
                .get("Content-Type")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/event-stream"));
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| McpError::Transport("HTTP response stream failed".into()))?
            {
                body.extend_from_slice(&chunk);
                if body.len() > MAX_MESSAGE_BYTES {
                    return Err(McpError::Protocol("response exceeds message limit".into()));
                }
                if sse {
                    // Consume complete events immediately; a server may keep the stream open.
                    while let Some((end, delimiter)) = event_boundary(&body) {
                        let event: Vec<_> = body.drain(..end + delimiter).collect();
                        let text = std::str::from_utf8(&event)
                            .map_err(|_| McpError::Protocol("invalid SSE encoding".into()))?;
                        let data = text
                            .lines()
                            .filter_map(|line| line.strip_prefix("data:"))
                            .map(|line| line.strip_prefix(' ').unwrap_or(line))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if data.is_empty() {
                            continue;
                        }
                        if let Some(reply) =
                            server_request_reply(&serde_json::from_str::<Value>(&data).map_err(
                                |_| McpError::Protocol("invalid JSON-RPC response".into()),
                            )?)?
                        {
                            let response = self.send(&reply).await?;
                            check_status(response.status())?;
                            continue;
                        }
                        if let Some(response) = matching_response(data.as_bytes(), id)? {
                            return Ok(response);
                        }
                    }
                }
            }
            if !sse {
                if let Some(response) = matching_response(&body, id)? {
                    return Ok(response);
                }
            }
            Err(McpError::Protocol(
                "HTTP response did not contain a matching JSON-RPC response".into(),
            ))
        };
        timeout(Duration::from_millis(timeout_ms), operation)
            .await
            .map_err(|_| McpError::Timeout)?
    }
}

pub(crate) fn event_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    (0..bytes.len()).find_map(|index| {
        if bytes[index..].starts_with(b"\r\n\r\n") {
            Some((index, 4))
        } else if bytes[index..].starts_with(b"\n\n") {
            Some((index, 2))
        } else {
            None
        }
    })
}

fn matching_response(bytes: &[u8], id: u64) -> Result<Option<JsonRpcResponse>> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| McpError::Protocol("invalid JSON-RPC response".into()))?;
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpError::Protocol("invalid JSON-RPC version".into()));
    }
    if value.get("method").is_some() || value.get("id").and_then(Value::as_u64) != Some(id) {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|_| McpError::Protocol("invalid JSON-RPC response shape".into()))
}

pub(crate) fn check_status(status: reqwest::StatusCode) -> Result<()> {
    if status.is_success() {
        Ok(())
    } else if matches!(status.as_u16(), 401 | 403) {
        Err(McpError::CredentialUnavailable)
    } else {
        Err(McpError::ConnectionFailed(format!(
            "HTTP {}",
            status.as_u16()
        )))
    }
}
pub(crate) fn request_id<T: Serialize>(payload: &T) -> Result<u64> {
    serde_json::to_value(payload)
        .ok()
        .and_then(|value| value.get("id").and_then(Value::as_u64))
        .ok_or_else(|| McpError::Protocol("request requires numeric id".into()))
}
pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}
pub fn response_result(response: JsonRpcResponse) -> Result<Value> {
    ensure_response(response)
}
