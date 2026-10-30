//! Shared helpers for the provider integration tests.
//!
//! `serve` is a scripted local HTTP/1.1 server. One script is played per accepted connection, in
//! accept order (the last script repeats), so a test controls exactly what each attempt sees,
//! including the pacing of bytes, and can inspect every request the client sent. Responses always
//! close the connection, so every client attempt is a new connection and a new script.
#![allow(dead_code)]

use dgos_providers::vendor::{ANTHROPIC_OFFICIAL, OPENAI_OFFICIAL};
use dgos_providers::{AdapterOptions, AnthropicAdapter, OpenAIAdapter};
use dgos_types::{ChatChunk, ChatMessage, ChatRequest, DgosResult, RetryConfig};
use futures::{Stream, StreamExt};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const API_KEY: &str = "synthetic-key-123";
pub const DONE: &str = "data: [DONE]\n\n";

#[derive(Clone)]
pub enum Step {
    Send(Vec<u8>),
    Sleep(Duration),
}

#[derive(Clone, Debug)]
pub struct Captured {
    pub head: String,
    pub body: String,
}

pub struct Stub {
    pub url: String,
    captured: Arc<Mutex<Vec<Captured>>>,
}

impl Stub {
    pub fn hits(&self) -> usize {
        self.captured.lock().unwrap().len()
    }

    pub fn requests(&self) -> Vec<Captured> {
        self.captured.lock().unwrap().clone()
    }
}

pub fn text(value: &str) -> Step {
    Step::Send(value.as_bytes().to_vec())
}

pub fn sleep(millis: u64) -> Step {
    Step::Sleep(Duration::from_millis(millis))
}

/// A complete response with a body length.
pub fn response(status: u16, headers: &[(&str, &str)], body: &str) -> Vec<Step> {
    let mut head = format!(
        "HTTP/1.1 {status} Status\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    vec![text(&head), text(body)]
}

/// Headers of a streaming response; the body ends when the server closes.
pub fn sse_head() -> Step {
    text("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n")
}

pub fn delta(content: &str) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":content},"finish_reason":null}]})
    )
}

pub fn finish(reason: &str) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{},"finish_reason":reason}]})
    )
}

pub fn usage(prompt: u32, completion: u32) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices":[],"usage":{"prompt_tokens":prompt,"completion_tokens":completion,
            "total_tokens":prompt+completion}})
    )
}

/// A synchronous chat completion body.
pub fn ok_chat() -> String {
    json!({"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})
    .to_string()
}

/// A complete streaming exchange: headers, the given events, close.
pub fn sse_script(events: &[String]) -> Vec<Step> {
    let mut steps = vec![sse_head()];
    steps.extend(events.iter().map(|event| text(event)));
    steps
}

pub fn adapter(url: &str, max_retries: u32, delay_ms: u64) -> OpenAIAdapter {
    adapter_with(url, max_retries, delay_ms, AdapterOptions::default())
}

pub fn adapter_with(
    url: &str,
    max_retries: u32,
    delay_ms: u64,
    options: AdapterOptions,
) -> OpenAIAdapter {
    OpenAIAdapter::with_options(
        url.into(),
        API_KEY.into(),
        Some(RetryConfig {
            max_retries,
            initial_delay_ms: delay_ms,
            max_delay_ms: delay_ms.max(1) * 4,
        }),
        options,
    )
}

fn retry_config(max_retries: u32, delay_ms: u64) -> Option<RetryConfig> {
    Some(RetryConfig {
        max_retries,
        initial_delay_ms: delay_ms,
        max_delay_ms: delay_ms.max(1) * 4,
    })
}

/// `openai_compatible` adapter that uses the official OpenAI profile, pointed at a local stub.
/// Production picks the profile from the host; tests choose it directly.
pub fn openai_official(url: &str, options: AdapterOptions) -> OpenAIAdapter {
    OpenAIAdapter::with_profile(
        url.into(),
        API_KEY.into(),
        retry_config(0, 1),
        options,
        &OPENAI_OFFICIAL,
    )
}

/// `anthropic` adapter with the generic profile (Bearer + x-api-key, version 2023-06-01).
pub fn anthropic(url: &str, max_retries: u32, delay_ms: u64) -> AnthropicAdapter {
    anthropic_with(url, max_retries, delay_ms, AdapterOptions::default())
}

pub fn anthropic_with(
    url: &str,
    max_retries: u32,
    delay_ms: u64,
    options: AdapterOptions,
) -> AnthropicAdapter {
    AnthropicAdapter::with_options(
        url.into(),
        API_KEY.into(),
        retry_config(max_retries, delay_ms),
        options,
    )
}

/// `anthropic` adapter with the official profile (Bearer only, version 2026-10-21).
pub fn anthropic_official(url: &str, options: AdapterOptions) -> AnthropicAdapter {
    AnthropicAdapter::with_profile(
        url.into(),
        API_KEY.into(),
        retry_config(0, 1),
        options,
        &ANTHROPIC_OFFICIAL,
    )
}

/// A complete Messages response body with one text block.
pub fn ok_message() -> String {
    json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "m",
        "content": [{"type": "text", "text": "ok"}], "stop_reason": "end_turn",
        "usage": {"input_tokens": 3, "output_tokens": 2}})
    .to_string()
}

/// One Messages stream event as it appears on the wire.
pub fn event(name: &str, data: serde_json::Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

pub fn message_start(input_tokens: u32) -> String {
    event(
        "message_start",
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
            "role": "assistant", "content": [],
            "usage": {"input_tokens": input_tokens, "output_tokens": 1}}}),
    )
}

pub fn text_delta(text: &str) -> String {
    event(
        "content_block_delta",
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": text}}),
    )
}

pub fn thinking_delta(text: &str) -> String {
    event(
        "content_block_delta",
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "thinking_delta", "thinking": text}}),
    )
}

pub fn message_delta(stop_reason: &str, output_tokens: u32) -> String {
    event(
        "message_delta",
        json!({"type": "message_delta", "delta": {"stop_reason": stop_reason},
            "usage": {"output_tokens": output_tokens}}),
    )
}

pub fn message_stop() -> String {
    event("message_stop", json!({"type": "message_stop"}))
}

pub fn ping() -> String {
    event("ping", json!({"type": "ping"}))
}

/// Lower-cased header lines of a captured request, for header assertions.
pub fn header_lines(captured: &Captured) -> Vec<String> {
    captured
        .head
        .lines()
        .map(|line| line.to_ascii_lowercase())
        .collect()
}

pub fn request() -> ChatRequest {
    ChatRequest {
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "hello".into(),
        }],
        temperature: None,
        max_tokens: Some(16),
        stream: None,
        ..Default::default()
    }
}

pub async fn drain<S>(mut stream: S) -> Vec<DgosResult<ChatChunk>>
where
    S: Stream<Item = DgosResult<ChatChunk>> + Unpin,
{
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item);
    }
    items
}

pub fn text_of(items: &[DgosResult<ChatChunk>]) -> String {
    items
        .iter()
        .filter_map(|item| item.as_ref().ok())
        .map(|chunk| chunk.delta.as_str())
        .collect()
}

async fn read_request(stream: &mut TcpStream) -> Option<Captured> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(position) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < head_end + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8_lossy(&buf[head_end..head_end + length]).to_string();
    Some(Captured { head, body })
}

pub async fn serve(scripts: Vec<Vec<Step>>) -> Stub {
    assert!(!scripts.is_empty());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Vec::new()));
    let log = captured.clone();
    tokio::spawn(async move {
        let mut index = 0usize;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let script = scripts[index.min(scripts.len() - 1)].clone();
            index += 1;
            let log = log.clone();
            tokio::spawn(async move {
                socket.set_nodelay(true).ok();
                if let Some(request) = read_request(&mut socket).await {
                    log.lock().unwrap().push(request);
                }
                for step in script {
                    match step {
                        Step::Send(bytes) => {
                            if socket.write_all(&bytes).await.is_err() {
                                return;
                            }
                            socket.flush().await.ok();
                        }
                        Step::Sleep(duration) => tokio::time::sleep(duration).await,
                    }
                }
                let _ = socket.shutdown().await;
            });
        }
    });
    Stub { url, captured }
}
