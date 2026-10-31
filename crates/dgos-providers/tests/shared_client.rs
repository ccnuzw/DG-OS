//! FR-001 known difference #8: `ProviderManager` builds one HTTP client and every adapter it
//! creates reuses it. Building a `reqwest::Client` per call was observed to stall for about 3 s
//! on macOS outside every timeout. Reuse is observable from outside as a kept-alive connection:
//! two calls through the same manager reach a keep-alive server over a single TCP connection,
//! while a fresh client per call would open a second one.

use dgos_providers::ProviderManager;
use dgos_types::ProviderConfig;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const MODELS: &str = r#"{"object":"list","data":[{"id":"m"}]}"#;

/// Answers every request on a connection with the same model list and keeps the connection
/// open. Returns the base URL and the number of accepted connections.
async fn keep_alive_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    // Requests here are GETs without a body: one blank line ends each.
                    while let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        buffer.drain(..end + 4);
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{MODELS}",
                            MODELS.len()
                        );
                        if socket.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    (url, accepted)
}

#[tokio::test]
async fn adapters_from_one_manager_share_a_client_and_its_connections() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("t.db").display());
    // `list_models_for_config` takes the key from the config and never reads the vault.
    let manager = ProviderManager::new(&url).await.unwrap();
    let (base_url, accepted) = keep_alive_server().await;
    let config = ProviderConfig {
        name: "Loopback".into(),
        provider_type: "openai_compatible".into(),
        base_url,
        api_key: "test-only-secret".into(),
        enabled: true,
        config: None,
    };
    for _ in 0..2 {
        let models = manager.list_models_for_config(&config).await.unwrap();
        assert_eq!(models.len(), 1);
    }
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "the second call reused the first call's connection, so the client was shared"
    );
}
