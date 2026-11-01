//! FR-010 AC10 and B10-55～B10-59: `GET /api/v1/lan/stream` over a real
//! WebSocket client. Handshake admission, chat frames, one connection per
//! session (4409), revocation (`session_ended` + 4401), protocol violation
//! (1008), and that a dropped connection never cancels a generation.

mod common;

use common::*;
use dgos_lan::RevokeReason;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn connect(env: &Env, cookie: Option<&str>, origin: Option<&str>) -> Result<Ws, u16> {
    let mut request = format!("ws://{}/api/v1/lan/stream", env.addr)
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("x-test-peer", PEER.parse().unwrap());
    if let Some(origin) = origin {
        headers.insert("origin", origin.parse().unwrap());
    }
    if let Some(cookie) = cookie {
        headers.insert("cookie", cookie.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Err(response.status().as_u16())
        }
        Err(other) => panic!("unexpected handshake error: {other}"),
    }
}

async fn open(env: &Env, cookie: &str) -> Ws {
    connect(env, Some(cookie), Some(&env.origin()))
        .await
        .unwrap()
}

enum Frame {
    Text(Value),
    Close(u16),
    Ended,
}

async fn next(socket: &mut Ws) -> Frame {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("frame in time");
        match message {
            Some(Ok(Message::Text(text))) => {
                return Frame::Text(serde_json::from_str(&text).unwrap())
            }
            Some(Ok(Message::Close(frame))) => {
                return Frame::Close(frame.map(|f| u16::from(f.code)).unwrap_or(1005))
            }
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return Frame::Ended,
        }
    }
}

async fn send_chat(env: &Env, cookie: &str, chat: &str, text: &str) -> Reply {
    env.post(
        &format!("/api/v1/lan/chat/sessions/{chat}/messages"),
        json!({ "content": text, "client_request_id": uuid::Uuid::new_v4().to_string() }),
    )
    .cookie(cookie)
    .send()
    .await
}

#[tokio::test]
async fn the_handshake_needs_a_session_and_the_exact_origin() {
    let env = env().await;
    assert_eq!(
        connect(&env, None, Some(&env.origin())).await.err(),
        Some(401)
    );
    let cookie = env.paired(PEER).await;
    assert_eq!(connect(&env, Some(&cookie), None).await.err(), Some(403));
    assert_eq!(
        connect(&env, Some(&cookie), Some("http://evil.example"))
            .await
            .err(),
        Some(403)
    );
    assert!(connect(&env, Some(&cookie), Some(&env.origin()))
        .await
        .is_ok());
    // A plain GET (no upgrade) on the stream route is a client error, empty.
    let plain = env.get("/api/v1/lan/stream").cookie(&cookie).send().await;
    assert!((400..500).contains(&plain.status));
    assert!(plain.text.is_empty());
}

#[tokio::test]
async fn generation_events_stream_and_the_terminal_frame_carries_the_snapshot() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let chat = env.chat(&cookie).await;
    let mut socket = open(&env, &cookie).await;
    env.up.script(reply("streamed text"));
    let ack = send_chat(&env, &cookie, &chat, "hi").await;
    assert_eq!(ack.status, 202);
    let mut kinds = Vec::new();
    loop {
        match next(&mut socket).await {
            Frame::Text(frame) => {
                assert_eq!(frame["type"], "chat");
                let kind = frame["event"]["kind"].as_str().unwrap().to_owned();
                if kind == "done" {
                    assert_eq!(frame["event"]["message"]["content"], "streamed text");
                    assert_eq!(frame["event"]["message"]["status"], "completed");
                    kinds.push(kind);
                    break;
                }
                kinds.push(kind);
            }
            _ => panic!("stream ended early"),
        }
    }
    assert_eq!(kinds.first().map(String::as_str), Some("started"));
    assert!(kinds.iter().any(|k| k == "delta"));
    // Nothing more arrives after the terminal frame.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), socket.next())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_new_connection_replaces_the_old_one_with_4409() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let mut first = open(&env, &cookie).await;
    let mut second = open(&env, &cookie).await;
    assert!(matches!(next(&mut first).await, Frame::Close(4409)));
    // The new one receives events.
    let chat = env.chat(&cookie).await;
    send_chat(&env, &cookie, &chat, "x").await;
    assert!(matches!(next(&mut second).await, Frame::Text(f) if f["type"] == "chat"));
}

#[tokio::test]
async fn revocation_sends_session_ended_then_4401_and_service_stop_1001() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let mut socket = open(&env, &cookie).await;
    let id = env.core.list_devices()[0].session_id.clone();
    env.core.disconnect(&id).await;
    match next(&mut socket).await {
        Frame::Text(frame) => {
            assert_eq!(
                frame,
                json!({"type": "session_ended", "reason": "host_disconnect"})
            )
        }
        _ => panic!("expected session_ended"),
    }
    assert!(matches!(next(&mut socket).await, Frame::Close(4401)));
    // Reconnecting with the revoked Cookie is refused at the handshake.
    assert_eq!(
        connect(&env, Some(&cookie), Some(&env.origin()))
            .await
            .err(),
        Some(401)
    );

    let cookie = env.paired(PEER).await;
    let mut socket = open(&env, &cookie).await;
    env.core.revoke_all(RevokeReason::SwitchOff).await;
    match next(&mut socket).await {
        Frame::Text(frame) => assert_eq!(frame["reason"], "switch_off"),
        _ => panic!("expected session_ended"),
    }
    assert!(matches!(next(&mut socket).await, Frame::Close(1001)));
}

#[tokio::test]
async fn a_client_data_frame_closes_with_1008() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let mut socket = open(&env, &cookie).await;
    socket.send(Message::Text("hello".into())).await.unwrap();
    match next(&mut socket).await {
        Frame::Close(code) => assert_eq!(code, u16::from(CloseCode::Policy)),
        _ => panic!("expected a close"),
    }
    // The session itself is fine.
    assert_eq!(
        env.get("/api/v1/lan/session")
            .cookie(&cookie)
            .send()
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn a_dropped_connection_does_not_cancel_and_the_list_recovers_the_reply() {
    let env = env().await;
    let cookie = env.paired(PEER).await;
    let chat = env.chat(&cookie).await;
    let mut socket = open(&env, &cookie).await;
    env.up.script(vec![delta("one "), Step::Hold]);
    send_chat(&env, &cookie, &chat, "long").await;
    assert!(matches!(next(&mut socket).await, Frame::Text(_)));
    drop(socket);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(env.up.open(), 1, "still generating after the stream closed");
    // Reconnect; the message list is the source of truth.
    let _again = open(&env, &cookie).await;
    let items = env.messages(&cookie, &chat).await;
    assert_eq!(items[1]["status"], "generating");
    // Ends normally through an explicit cancel, not through the disconnect.
    let task = items[1]["task_id"].as_str().unwrap().to_owned();
    let cancelled = env
        .post(&format!("/api/v1/lan/chat/tasks/{task}/cancel"), json!({}))
        .cookie(&cookie)
        .send()
        .await;
    assert_eq!(cancelled.body["cancelled"], true);
    let items = env.settled(&cookie, &chat).await;
    assert_eq!(items[1]["content"], "one ");
}

/// Through the production accept loop (`server::serve`, 16 connections per
/// source address): an upgraded stream keeps its connection slot until the
/// WebSocket task ends, then gives it back. Every connection here comes from
/// 127.0.0.1; the layer only presents a private peer to the router's gate.
#[tokio::test]
async fn an_open_stream_holds_its_connection_slot_until_it_ends() {
    use dgos_lan::server::{serve, MAX_CONNECTIONS_PER_IP};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let env = env().await;
    let cookie = env.paired(PEER).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = match listener.local_addr().unwrap() {
        std::net::SocketAddr::V4(v4) => v4,
        std::net::SocketAddr::V6(_) => unreachable!(),
    };
    let router = dgos_lan::router(env.core.clone(), addr, None).layer(axum::middleware::from_fn(
        |mut request: axum::extract::Request, next: axum::middleware::Next| async move {
            let peer =
                std::net::SocketAddr::from((PEER.parse::<std::net::Ipv4Addr>().unwrap(), 50000));
            request.extensions_mut().insert(dgos_lan::PeerAddr(peer));
            next.run(request).await
        },
    ));
    let shutdown = tokio_util::sync::CancellationToken::new();
    tokio::spawn(serve(listener, router, shutdown.clone()));

    // `true` when the server answered; a refused connection is closed unanswered.
    let probe = || async move {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "GET /api/v1/lan/session HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n\r\n"
        );
        if stream.write_all(request.as_bytes()).await.is_err() {
            return false;
        }
        let mut reply = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut reply))
            .await
            .expect("answered or closed in time");
        reply.starts_with(b"HTTP/1.1 ")
    };

    let mut request = format!("ws://{addr}/api/v1/lan/stream")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", format!("http://{addr}").parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    // Fill the remaining per-address slots with idle connections (accepted in order).
    let mut idle = Vec::new();
    for _ in 1..MAX_CONNECTIONS_PER_IP {
        idle.push(TcpStream::connect(addr).await.unwrap());
    }
    assert!(!probe().await, "the upgraded stream still counts");

    drop(socket);
    let mut released = false;
    for _ in 0..50 {
        if probe().await {
            released = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(released, "the slot comes back once the stream ends");
    drop(idle);
    shutdown.cancel();
}
