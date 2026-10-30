//! AC09: with `retry_count = 3` an upstream 500 is retried with growing waits.
//!
//! The policy test runs on tokio's paused clock, so the 1 s / 2 s / 4 s waits are measured exactly
//! without waiting. The adapter test uses real sockets, which do not mix with a paused clock (the
//! runtime would jump the clock while a response is still on its way), so it uses a short real
//! delay and checks the gaps the server observed.
mod common;

use common::*;
use dgos_providers::{DgosError, ProviderAdapter, RetryConfig, RetryPolicy};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Instant;

fn gaps(stamps: &[Instant]) -> Vec<Duration> {
    stamps.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

#[tokio::test(start_paused = true)]
async fn retry_delays_default_policy_waits_1s_2s_4s_across_three_retries() {
    let policy = RetryPolicy::new(RetryConfig::default());
    assert_eq!(RetryConfig::default().max_retries, 3);
    let stamps = Arc::new(Mutex::new(Vec::new()));
    let log = stamps.clone();
    let error = policy
        .execute(|| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(Instant::now());
                Err::<(), _>(DgosError::UpstreamServerError)
            }
        })
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::MaxRetriesExceeded), "{error:?}");
    let stamps = stamps.lock().unwrap();
    assert_eq!(stamps.len(), 4, "1 call + 3 retries");
    assert_eq!(
        gaps(&stamps),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn retry_delays_stop_growing_at_max_delay() {
    let policy = RetryPolicy::new(RetryConfig {
        max_retries: 5,
        initial_delay_ms: 1000,
        max_delay_ms: 3000,
    });
    let stamps = Arc::new(Mutex::new(Vec::new()));
    let log = stamps.clone();
    let _ = policy
        .execute(|| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(Instant::now());
                Err::<(), _>(DgosError::UpstreamServerError)
            }
        })
        .await;
    let secs: Vec<u64> = gaps(&stamps.lock().unwrap())
        .iter()
        .map(Duration::as_secs)
        .collect();
    assert_eq!(secs, vec![1, 2, 3, 3, 3]);
}

/// Answers every request with a 500 and records when each arrived.
async fn failing_server() -> (String, Arc<Mutex<Vec<std::time::Instant>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let arrivals = Arc::new(Mutex::new(Vec::new()));
    let log = arrivals.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => buf.extend_from_slice(&chunk[..read]),
                    }
                }
                log.lock().unwrap().push(std::time::Instant::now());
                let body = r#"{"error":{"message":"boom"}}"#;
                let reply = format!(
                    "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (url, arrivals)
}

fn assert_doubling_waits(arrivals: &Mutex<Vec<std::time::Instant>>, base_ms: u64) {
    let arrivals = arrivals.lock().unwrap();
    assert_eq!(arrivals.len(), 4, "1 call + 3 retries");
    let observed: Vec<Duration> = arrivals.windows(2).map(|p| p[1] - p[0]).collect();
    for (observed, factor) in observed.iter().zip([1u64, 2, 4]) {
        let wanted = Duration::from_millis(base_ms * factor);
        // A sleep never ends early; generous slack above for a loaded machine.
        assert!(
            *observed >= wanted,
            "waited {observed:?}, wanted {wanted:?}"
        );
        assert!(
            *observed < wanted + Duration::from_millis(500),
            "waited {observed:?}, wanted about {wanted:?}"
        );
    }
}

#[tokio::test]
async fn retry_delays_openai_adapter_retries_a_500_three_times_with_doubling_waits() {
    let (url, arrivals) = failing_server().await;
    let error = adapter(&url, 3, 100)
        .call("m", request())
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::MaxRetriesExceeded), "{error:?}");
    assert_doubling_waits(&arrivals, 100);
}

#[tokio::test]
async fn retry_delays_anthropic_adapter_retries_a_500_three_times_with_doubling_waits() {
    let (url, arrivals) = failing_server().await;
    let error = anthropic(&url, 3, 100)
        .call("m", request())
        .await
        .unwrap_err();
    assert!(matches!(error, DgosError::MaxRetriesExceeded), "{error:?}");
    assert_doubling_waits(&arrivals, 100);
}
