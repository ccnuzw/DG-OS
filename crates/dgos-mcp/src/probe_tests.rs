use super::*;
use std::sync::{Arc, Mutex};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};

async fn fixture(mode: &'static str) -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let methods = Arc::new(Mutex::new(Vec::new())); let seen = methods.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new(); let mut buffer = [0_u8; 4096];
            let (header_end, length) = loop {
                let read = socket.read(&mut buffer).await.unwrap(); if read == 0 { return; } bytes.extend_from_slice(&buffer[..read]);
                if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers.lines().find_map(|line| line.split_once(':').filter(|(name,_)| name.eq_ignore_ascii_case("content-length")).map(|(_,value)| value.trim().parse::<usize>().unwrap())).unwrap_or(0);
                    break (end + 4, length);
                }
                assert!(bytes.len() < 64 * 1024);
            };
            while bytes.len() < header_end + length { let read = socket.read(&mut buffer).await.unwrap(); assert!(read > 0); bytes.extend_from_slice(&buffer[..read]); }
            let delete = bytes.starts_with(b"DELETE ");
            let request: Value = if length == 0 { json!({}) } else { serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap() };
            let method = if delete { "DELETE" } else { request["method"].as_str().unwrap() };
            seen.lock().unwrap().push(method.to_owned());
            let result = match method {
                "initialize" if mode == "http-only" => json!({"healthy":true}),
                "initialize" => json!({"protocolVersion":"2025-06-18", "capabilities":{"tools":{}}, "serverInfo":{"name":"fixture","version":"1"}}),
                "tools/list" if mode == "missing-schema" => json!({"tools":[{"name":"unsafe"}]}),
                "tools/list" if request.pointer("/params/cursor").is_none() => json!({"tools":[{"name":"first","description":"Complete first definition","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]},"annotations":{"readOnlyHint":true}}],"nextCursor":"opaque:second"}),
                "tools/list" => { assert_eq!(request["params"]["cursor"], "opaque:second"); json!({"tools":[{"name":if mode == "duplicate" { "first" } else { "second" },"inputSchema":{"type":"object"},"outputSchema":{"type":"object","properties":{"ok":{"type":"boolean"}}}}]}) },
                "notifications/initialized" | "DELETE" => Value::Null,
                _ => panic!("probe must not invoke {method}"),
            };
            let body = if request.get("id").is_some() { json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string() } else { String::new() };
            let status = if method == "DELETE" && mode == "cleanup-unknown" { "405 Method Not Allowed" } else { "200 OK" };
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nMcp-Session-Id: owned-fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap(); socket.shutdown().await.unwrap();
        }
    });
    (url, methods, task)
}

#[tokio::test]
async fn initializes_reads_all_pages_and_preserves_complete_schemas_without_calls() {
    let (url, methods, task) = fixture("complete").await;
    let connection = ProbeConnection::connect(ProbeEndpoint::Http { url, headers: vec![], legacy_sse: false }).unwrap();
    let tools = tokio::time::timeout(Duration::from_secs(2), connection.read_tools(|_| {})).await.unwrap().unwrap();
    assert_eq!(tools.tools.len(), 2); assert_eq!(tools.tools[0]["annotations"]["readOnlyHint"], true);
    assert_eq!(tools.tools[0]["inputSchema"]["required"], json!(["path"]));
    assert_eq!(tools.tools[1]["outputSchema"]["properties"]["ok"]["type"], "boolean");
    assert!(connection.cleanup().await);
    assert_eq!(*methods.lock().unwrap(), ["initialize", "notifications/initialized", "tools/list", "tools/list", "DELETE"]);
    task.abort();
}
#[tokio::test]
async fn rejects_http_success_missing_schemas_and_duplicate_pages() {
    for mode in ["http-only", "missing-schema", "duplicate"] {
        let (url, methods, task) = fixture(mode).await;
        let connection = ProbeConnection::connect(ProbeEndpoint::Http { url, headers: vec![], legacy_sse: false }).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), connection.read_tools(|_| {})).await.unwrap().is_err());
        assert!(!methods.lock().unwrap().iter().any(|method| method == "tools/call"));
        assert!(connection.cleanup().await); task.abort();
    }
}
#[tokio::test]
async fn failed_cleanup_keeps_the_original_session_for_inspection() {
    let (url, methods, task) = fixture("cleanup-unknown").await;
    let connection = ProbeConnection::connect(ProbeEndpoint::Http { url, headers: vec![], legacy_sse: false }).unwrap();
    connection.read_tools(|_| {}).await.unwrap();
    assert!(!connection.cleanup().await); assert!(!connection.cleanup().await);
    assert_eq!(methods.lock().unwrap().iter().filter(|method| method.as_str() == "DELETE").count(), 2);
    assert_eq!(methods.lock().unwrap().iter().filter(|method| method.as_str() == "initialize").count(), 1);
    task.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_of_a_silent_owned_process_has_bounded_confirmed_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let connection = ProbeConnection::connect(ProbeEndpoint::Stdio { executable: "/bin/sleep".into(), args: vec!["30".into()], cwd: directory.path().to_string_lossy().into_owned(), environment: vec![], container: None }).unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(20), connection.read_tools(|_| {})).await.is_err());
    assert!(tokio::time::timeout(Duration::from_secs(2), connection.cleanup()).await.unwrap());
}
