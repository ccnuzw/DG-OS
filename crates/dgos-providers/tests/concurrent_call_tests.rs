//! AC08: five calls at once are not queued. The local server holds every request until all five
//! have arrived and only then answers, so the calls can only succeed if the adapter had five
//! requests in flight at the same time; a queueing adapter would deadlock and hit the bounded
//! timeout instead of hanging the suite.
mod common;

use common::*;
use dgos_providers::{AnthropicAdapter, OpenAIAdapter, ProviderAdapter};
use futures::future::join_all;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Barrier;

const CALLS: usize = 5;

struct Rendezvous {
    url: String,
    hits: Arc<AtomicUsize>,
    in_flight: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

/// Reads one full HTTP request (head and `content-length` body) and discards it.
async fn drain_request(stream: &mut TcpStream) -> bool {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return false,
            Ok(read) => buf.extend_from_slice(&chunk[..read]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < head_end + length {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return false,
            Ok(read) => buf.extend_from_slice(&chunk[..read]),
        }
    }
    true
}

/// Answers `body` with 200 only after `CALLS` requests are waiting at the barrier together.
async fn rendezvous_server(body: String) -> Rendezvous {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let barrier = Arc::new(Barrier::new(CALLS));
    let hits = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (h, f, p) = (hits.clone(), in_flight.clone(), peak.clone());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (barrier, hits, in_flight, peak, body) = (
                barrier.clone(),
                h.clone(),
                f.clone(),
                p.clone(),
                body.clone(),
            );
            tokio::spawn(async move {
                if !drain_request(&mut socket).await {
                    return;
                }
                hits.fetch_add(1, Ordering::SeqCst);
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                // Released only when the fifth request is in.
                barrier.wait().await;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Rendezvous {
        url,
        hits,
        in_flight,
        peak,
    }
}

fn assert_all_concurrent(server: &Rendezvous) {
    assert_eq!(server.hits.load(Ordering::SeqCst), CALLS);
    assert_eq!(
        server.peak.load(Ordering::SeqCst),
        CALLS,
        "the server must have seen all {CALLS} requests in flight at once"
    );
    assert_eq!(server.in_flight.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn concurrent_calls_openai_compatible_are_not_queued() {
    let server = rendezvous_server(ok_chat()).await;
    let adapter: Arc<OpenAIAdapter> = Arc::new(adapter(&server.url, 0, 1));
    let calls = (0..CALLS).map(|_| {
        let adapter = adapter.clone();
        async move { adapter.call("m", request()).await }
    });
    let results = tokio::time::timeout(Duration::from_secs(10), join_all(calls))
        .await
        .expect("5 concurrent calls must not queue behind each other");
    assert_eq!(results.len(), CALLS);
    for result in results {
        assert_eq!(result.unwrap().content, "ok");
    }
    assert_all_concurrent(&server);
}

#[tokio::test]
async fn concurrent_calls_anthropic_are_not_queued() {
    let server = rendezvous_server(ok_message()).await;
    let adapter: Arc<AnthropicAdapter> = Arc::new(anthropic(&server.url, 0, 1));
    let calls = (0..CALLS).map(|_| {
        let adapter = adapter.clone();
        async move { adapter.call("m", request()).await }
    });
    let results = tokio::time::timeout(Duration::from_secs(10), join_all(calls))
        .await
        .expect("5 concurrent calls must not queue behind each other");
    assert_eq!(results.len(), CALLS);
    for result in results {
        assert_eq!(result.unwrap().content, "ok");
    }
    assert_all_concurrent(&server);
}
