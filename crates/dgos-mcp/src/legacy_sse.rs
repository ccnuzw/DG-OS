use crate::{
    credentials::SecretToken,
    error::{McpError, Result},
    protocol::{server_request_reply, JsonRpcResponse, NegotiatedProtocol},
    transport::{
        check_status, event_boundary, request_id, validate_http_url_with_consent, Pending,
        PendingGuard, MAX_MESSAGE_BYTES,
    },
};
use reqwest::{Client, Response, Url};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{timeout, Duration},
};

#[derive(Clone)]
struct Sender {
    client: Client,
    endpoint: Url,
    bearer: Option<Arc<SecretToken>>,
    negotiated: Arc<OnceLock<NegotiatedProtocol>>,
}

impl Sender {
    async fn send<T: Serialize>(&self, payload: &T) -> Result<Response> {
        let mut request = self
            .client
            .post(self.endpoint.clone())
            .header("Accept", "application/json, text/event-stream")
            .json(payload);
        if let Some(token) = &self.bearer {
            request = request.bearer_auth(token.expose());
        }
        if let Some(protocol) = self.negotiated.get() {
            request = request.header("MCP-Protocol-Version", &protocol.version);
        }
        request
            .send()
            .await
            .map_err(|_| McpError::Transport("legacy SSE POST failed".into()))
    }
}

pub(crate) struct LegacySseSession {
    sender: Sender,
    pending: Pending,
    reader: JoinHandle<()>,
}

impl LegacySseSession {
    pub(crate) async fn connect(
        url: &str,
        client: Client,
        bearer: Option<Arc<SecretToken>>,
        negotiated: Arc<OnceLock<NegotiatedProtocol>>,
        allow_insecure_http: bool,
    ) -> Result<Self> {
        let base = validate_http_url_with_consent(url, allow_insecure_http)?;
        let mut request = client
            .get(base.clone())
            .header("Accept", "text/event-stream");
        if let Some(token) = &bearer {
            request = request.bearer_auth(token.expose());
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| McpError::Transport("legacy SSE GET failed".into()))?;
        check_status(response.status())?;
        if !response
            .headers()
            .get("Content-Type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
        {
            return Err(McpError::Protocol("legacy SSE stream expected".into()));
        }
        let mut events = Events::default();
        let endpoint = loop {
            if let Some((event, data)) = events.next()? {
                if data.is_empty() {
                    continue;
                }
                if event != "endpoint" {
                    return Err(McpError::Protocol("legacy SSE endpoint expected".into()));
                }
                break legacy_endpoint(&base, &data)?;
            }
            events.read(&mut response).await?;
        };
        let sender = Sender {
            client,
            endpoint,
            bearer,
            negotiated,
        };
        let reader_sender = sender.clone();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = pending.clone();
        let reader = tokio::spawn(async move {
            let _ = read_messages(response, events, reader_sender, reader_pending.clone()).await;
            reader_pending.lock().unwrap().clear();
        });
        Ok(Self {
            sender,
            pending,
            reader,
        })
    }

    pub(crate) async fn send<T: Serialize>(&self, payload: &T) -> Result<Response> {
        if self.reader.is_finished() {
            return Err(McpError::ConnectionFailed(
                "legacy SSE stream closed".into(),
            ));
        }
        self.sender.send(payload).await
    }

    pub(crate) async fn request<T: Serialize>(
        &self,
        payload: &T,
        timeout_ms: u64,
    ) -> Result<JsonRpcResponse> {
        let id = request_id(payload)?;
        let operation = async {
            let (send, receive) = oneshot::channel();
            self.pending.lock().unwrap().insert(id, send);
            let _guard = PendingGuard {
                id,
                pending: self.pending.clone(),
            };
            let response = self.send(payload).await?;
            check_status(response.status())?;
            receive
                .await
                .map_err(|_| McpError::ConnectionFailed("legacy SSE stream closed".into()))
        };
        timeout(Duration::from_millis(timeout_ms), operation)
            .await
            .map_err(|_| McpError::Timeout)?
    }

    pub(crate) fn close(&self) {
        self.reader.abort();
        self.pending.lock().unwrap().clear();
    }
}

impl Drop for LegacySseSession {
    fn drop(&mut self) {
        self.close();
    }
}

fn legacy_endpoint(base: &Url, data: &str) -> Result<Url> {
    let endpoint = base
        .join(data)
        .map_err(|_| McpError::Protocol("invalid legacy SSE endpoint".into()))?;
    // The validated base grants no permission to send credentials to another origin.
    validate_http_url_with_consent(endpoint.as_str(), base.scheme() == "http")?;
    // Endpoint discovery cannot move credentials or JSON-RPC messages to a
    // different origin, including a different port on the same machine.
    if endpoint.origin() != base.origin() {
        return Err(McpError::Protocol(
            "legacy SSE endpoint must have the configured origin".into(),
        ));
    }
    Ok(endpoint)
}

#[derive(Default)]
struct Events {
    buffer: Vec<u8>,
}

impl Events {
    async fn read(&mut self, response: &mut Response) -> Result<()> {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| McpError::Transport("legacy SSE read failed".into()))?
            .ok_or_else(|| McpError::ConnectionFailed("legacy SSE stream closed".into()))?;
        if self.buffer.len() + chunk.len() > MAX_MESSAGE_BYTES {
            return Err(McpError::Protocol(
                "legacy SSE event exceeds message limit".into(),
            ));
        }
        self.buffer.extend_from_slice(&chunk);
        Ok(())
    }

    fn next(&mut self) -> Result<Option<(String, String)>> {
        let Some((end, delimiter)) = event_boundary(&self.buffer) else {
            return Ok(None);
        };
        let bytes: Vec<_> = self.buffer.drain(..end + delimiter).collect();
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| McpError::Protocol("invalid legacy SSE encoding".into()))?;
        let mut event = "message";
        let mut data = Vec::new();
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                event = value.strip_prefix(' ').unwrap_or(value);
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push(value.strip_prefix(' ').unwrap_or(value));
            }
        }
        Ok(Some((event.to_owned(), data.join("\n"))))
    }
}

async fn read_messages(
    mut response: Response,
    mut events: Events,
    sender: Sender,
    pending: Pending,
) -> Result<()> {
    loop {
        while let Some((event, data)) = events.next()? {
            if data.is_empty() || event != "message" {
                continue;
            }
            let value: Value = serde_json::from_str(&data)
                .map_err(|_| McpError::Protocol("invalid legacy SSE message".into()))?;
            if let Some(reply) = server_request_reply(&value)? {
                let posted = timeout(Duration::from_secs(5), sender.send(&reply))
                    .await
                    .map_err(|_| McpError::Timeout)??;
                check_status(posted.status())?;
            } else if value.get("method").is_none() {
                let response: JsonRpcResponse = serde_json::from_value(value)
                    .map_err(|_| McpError::Protocol("invalid legacy SSE response".into()))?;
                if let Some(id) = response.id {
                    if let Some(sender) = pending.lock().unwrap().remove(&id) {
                        let _ = sender.send(response);
                    }
                }
            }
        }
        events.read(&mut response).await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_supports_relative_and_same_origin_urls() {
        let base = Url::parse("https://mcp.example.test/sse").unwrap();
        for value in [
            "/messages?sessionId=test",
            "messages?sessionId=test",
            "https://mcp.example.test/messages?sessionId=test",
        ] {
            let endpoint = legacy_endpoint(&base, value).unwrap();
            assert_eq!(endpoint.origin(), base.origin());
            assert_eq!(endpoint.query(), Some("sessionId=test"));
        }
        let plaintext = Url::parse("http://mcp.example.test/sse").unwrap();
        assert!(legacy_endpoint(&plaintext, "/messages").is_ok());
        for value in [
            "http://other.example.test/messages",
            "https://mcp.example.test/messages",
            "//mcp.example.test:8080/messages",
            "/messages#secret",
            "http://name:secret@mcp.example.test/messages",
        ] {
            assert!(legacy_endpoint(&plaintext, value).is_err());
        }
    }
}
