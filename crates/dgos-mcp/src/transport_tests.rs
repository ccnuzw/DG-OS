use crate::{
    protocol::request,
    transport::{response_result, Session},
    McpError,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    time::{timeout, Duration},
};

async fn http_fixture(mode: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        for index in 0..2 {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                socket.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            let lower = headers.to_lowercase();
            assert!(lower.contains("accept: application/json, text/event-stream"));
            if index == 1 {
                assert!(lower.contains("mcp-session-id: fixture-session"));
                assert!(lower.contains("mcp-protocol-version: 2024-11-05"));
            } else {
                assert!(!lower.contains("mcp-protocol-version:"));
            }
            let length: usize = lower
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            let payload: Value = serde_json::from_slice(&body).unwrap();
            let id = payload["id"].as_u64().unwrap();
            if mode == "failure" {
                socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 12\r\nConnection: close\r\n\r\nsecret-token").await.unwrap();
                break;
            }
            if mode == "timeout" {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n").await.unwrap();
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
            if mode == "sse" {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nMcp-Session-Id: fixture-session\r\nConnection: close\r\n\r\n: heartbeat\r\n\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}\r\n\r\n").await.unwrap();
                let event = format!("event: message\r\ndata: {{\"jsonrpc\":\"2.0\",\"id\":{id},\r\ndata: \"result\":{{\"ok\":true}}}}\r\n\r\n");
                for chunk in event.as_bytes().chunks(7) {
                    socket.write_all(chunk).await.unwrap();
                }
                // A streaming server does not close after delivering the result.
                tokio::time::sleep(Duration::from_secs(2)).await;
                break;
            }
            let response = json!({"jsonrpc":"2.0", "id":if mode == "mismatch" { id + 1 } else { id }, "result":{"ok":true}}).to_string();
            let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: fixture-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            if mode == "mismatch" {
                break;
            }
        }
    });
    (url, handle)
}

#[tokio::test]
async fn http_json_matches_request_and_reuses_session_header() {
    let (url, server) = http_fixture("json").await;
    let session = Session::http(url, reqwest::Client::new(), None).unwrap();
    for id in 1..=2 {
        let response = session
            .request(&request(id, "ping", json!({})), 1_000)
            .await
            .unwrap();
        assert_eq!(response_result(response).unwrap()["ok"], true);
        if id == 1 {
            session
                .complete_initialize(json!({
                    "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                    "serverInfo": {"name": "fixture", "version": "1"}
                }))
                .unwrap();
        }
    }
    server.await.unwrap();
}

#[tokio::test]
async fn http_close_releases_session_once_with_protocol_and_scoped_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for method in ["POST", "DELETE"] {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                socket.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            assert!(headers.starts_with(&format!("{method} /mcp HTTP/1.1")));
            let lower = headers.to_lowercase();
            assert!(lower.contains("authorization: bearer synthetic-close-token"));
            if method == "POST" {
                let length: usize = lower
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut body = vec![0; length];
                socket.read_exact(&mut body).await.unwrap();
                let id = serde_json::from_slice::<Value>(&body).unwrap()["id"].clone();
                let response = json!({"jsonrpc":"2.0", "id":id, "result":{}}).to_string();
                let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: closing-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response);
                socket.write_all(headers.as_bytes()).await.unwrap();
            } else {
                assert!(lower.contains("mcp-session-id: closing-session"));
                assert!(lower.contains("mcp-protocol-version: 2025-06-18"));
                socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
            socket.shutdown().await.unwrap();
        }
        assert!(timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err());
    });
    let session = Session::http(
        url,
        reqwest::Client::new(),
        Some(crate::SecretToken::new("synthetic-close-token".into()).unwrap()),
    )
    .unwrap();
    session
        .request(&request(1, "initialize", json!({})), 1_000)
        .await
        .unwrap();
    session.complete_initialize(json!({"protocolVersion":"2025-06-18", "capabilities":{}, "serverInfo":{"name":"fixture", "version":"1"}})).unwrap();
    session.close().await;
    session.close().await;
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn http_sse_answers_server_requests_on_separate_posts() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut original = BufReader::new(socket);
        let read_payload = |mut socket: BufReader<tokio::net::TcpStream>| async move {
            let mut length = 0;
            loop {
                let mut line = String::new();
                socket.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            (socket, serde_json::from_slice::<Value>(&body).unwrap())
        };
        let (socket, request) = read_payload(original).await;
        original = socket;
        original
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        for (id, method) in [
            ("ping-id", "ping"),
            ("unsupported-id", "fixture/notSupported"),
        ] {
            let event = format!(
                "data: {}\n\n",
                json!({"jsonrpc": "2.0", "id": id, "method": method})
            );
            original.write_all(event.as_bytes()).await.unwrap();
            let (socket, _) = listener.accept().await.unwrap();
            let (mut socket, reply) = read_payload(BufReader::new(socket)).await;
            assert_eq!(reply["id"], id);
            if method == "ping" {
                assert_eq!(reply["result"], json!({}));
            } else {
                assert_eq!(reply["error"]["code"], -32601);
            }
            socket
                .write_all(
                    b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        }
        let event = format!(
            "data: {}\n\n",
            json!({"jsonrpc": "2.0", "id": request["id"], "result": {"ok": true}})
        );
        original.write_all(event.as_bytes()).await.unwrap();
    });
    let session = Session::http(url, reqwest::Client::new(), None).unwrap();
    let response = session
        .request(&request(8, "ping", json!({})), 2000)
        .await
        .unwrap();
    assert_eq!(response_result(response).unwrap()["ok"], true);
    server.await.unwrap();
}

#[tokio::test]
async fn legacy_sse_get_discovers_post_endpoint_and_routes_response_over_get_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sse", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut initial = BufReader::new(socket);
        let mut length = 0;
        loop {
            let mut line = String::new();
            initial.read_line(&mut line).await.unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0; length];
        initial.read_exact(&mut body).await.unwrap();
        initial.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let mut get = BufReader::new(socket);
        let mut first_line = String::new();
        get.read_line(&mut first_line).await.unwrap();
        assert!(first_line.starts_with("GET /sse "));
        loop {
            let mut line = String::new();
            get.read_line(&mut line).await.unwrap();
            if line == "\r\n" {
                break;
            }
        }
        get.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n",
        )
        .await
        .unwrap();
        get.write_all(b"event: endpoint\ndata: /messages?sessionId=fixture\n\n")
            .await
            .unwrap();
        let (post, _) = listener.accept().await.unwrap();
        let mut post = BufReader::new(post);
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            post.read_line(&mut line).await.unwrap();
            if line == "\r\n" {
                break;
            }
            headers.push_str(&line);
        }
        assert!(headers
            .to_lowercase()
            .contains("/messages?sessionid=fixture"));
        let length = headers
            .to_lowercase()
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        let mut body = vec![0; length];
        post.read_exact(&mut body).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        post.get_mut()
            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let event = format!(
            "data: {}\n\n",
            json!({"jsonrpc": "2.0", "id": payload["id"], "result": {
                "protocolVersion": "2025-03-26", "capabilities": {"tools": {}}, "serverInfo": {"name": "legacy", "version": "1"}
            }})
        );
        get.write_all(event.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let session = Session::http(url, reqwest::Client::new(), None).unwrap();
    let response = session
        .request(&request(99, "initialize", json!({})), 2000)
        .await
        .unwrap();
    assert_eq!(
        response_result(response).unwrap()["serverInfo"]["name"],
        "legacy"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn http_sse_reads_multiline_fragmented_event_without_waiting_for_eof() {
    let (url, server) = http_fixture("sse").await;
    let session = Session::http(url, reqwest::Client::new(), None).unwrap();
    let response = timeout(
        Duration::from_millis(800),
        session.request(&request(7, "ping", json!({})), 1_000),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response_result(response).unwrap()["ok"], true);
    server.abort();
}

#[tokio::test]
async fn http_timeout_includes_body_and_errors_do_not_disclose_response() {
    for mode in ["timeout", "failure", "mismatch"] {
        let (url, server) = http_fixture(mode).await;
        let session = Session::http(url, reqwest::Client::new(), None).unwrap();
        let error = session
            .request(&request(3, "ping", json!({})), 100)
            .await
            .unwrap_err();
        match mode {
            "timeout" => assert!(matches!(error, McpError::Timeout)),
            "failure" => assert!(matches!(error, McpError::CredentialUnavailable)),
            _ => assert!(matches!(error, McpError::Protocol(_))),
        }
        assert!(!error.to_string().contains("secret-token"));
        server.abort();
    }
}

#[tokio::test]
async fn stdio_request_timeout_is_bounded() {
    let session = Session::stdio(
        "node",
        &[
            format!(
                "{}/tests/fixtures/mcp-server.mjs",
                env!("CARGO_MANIFEST_DIR")
            ),
            "silent".into(),
        ],
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        session
            .request(&request(1, "initialize", json!({})), 100)
            .await,
        Err(McpError::Timeout)
    ));
    session.close().await;
}

#[tokio::test]
async fn abandoned_stdio_write_does_not_truncate_frame_or_poison_next_request() {
    let session = Session::stdio(
        "node",
        &[
            format!(
                "{}/tests/fixtures/mcp-server.mjs",
                env!("CARGO_MANIFEST_DIR")
            ),
            "delayed-stdin".into(),
        ],
        None,
    )
    .await
    .unwrap();
    let large = request(
        101,
        "initialize",
        json!({"padding": "x".repeat(512 * 1024)}),
    );
    assert!(matches!(
        session.request(&large, 25).await,
        Err(McpError::Timeout)
    ));
    let next = session
        .request(&request(102, "ping", json!({})), 2000)
        .await
        .unwrap();
    assert_eq!(response_result(next).unwrap(), json!({}));
    session.close().await;
}
