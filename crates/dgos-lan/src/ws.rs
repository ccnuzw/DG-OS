//! `GET /api/v1/lan/stream` after the upgrade (interface contract §14.7).
//!
//! Server-to-client text frames only. One connection per remote session (a
//! newer one closes this with 4409); a full 256-frame queue closes with
//! 4413; revocation sends `session_ended` then 4401 (1001 when the service
//! stops); a client data frame closes with 1008. Ping every 30 s, closed after
//! 60 s without a pong. Frames and pings never refresh the session's idle time,
//! and closing never cancels a generation.
//!
//! A write that cannot complete (the client stopped reading) is bounded by the
//! pong timeout and gives way to revocation: the loop must keep seeing control
//! messages and the ping deadline while a frame is in flight.

use crate::core::{session_ended_frame, RemoteSession, WsClose, CLOSE_PROTOCOL, CLOSE_REVOKED};
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{timeout, Instant};

const PING_EVERY: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(60);
/// A data write slower than this means the client is not reading.
const WRITE_TIMEOUT: Duration = PONG_TIMEOUT;
/// Best effort for the final `session_ended` / close frame.
const CLOSE_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// `false` when the write failed or timed out.
async fn send(socket: &mut WebSocket, message: Message, limit: Duration) -> bool {
    matches!(timeout(limit, socket.send(message)).await, Ok(Ok(())))
}

async fn close(socket: &mut WebSocket, code: u16) {
    let frame = Message::Close(Some(CloseFrame {
        code,
        reason: Utf8Bytes::from_static(""),
    }));
    send(socket, frame, CLOSE_WRITE_TIMEOUT).await;
}

async fn end(socket: &mut WebSocket, control: Option<WsClose>) {
    if let Some(control) = control {
        if let Some(reason) = control.ended {
            let text = Message::Text(session_ended_frame(reason).into());
            send(socket, text, CLOSE_WRITE_TIMEOUT).await;
        }
        close(socket, control.code).await;
    }
}

pub async fn run(mut socket: WebSocket, session: Arc<RemoteSession>) {
    let Some(mut attachment) = session.attach_stream() else {
        close(&mut socket, CLOSE_REVOKED).await;
        return;
    };
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_pong = Instant::now();
    loop {
        tokio::select! {
            biased;
            control = attachment.control.recv() => {
                end(&mut socket, control).await;
                break;
            }
            frame = attachment.frames.recv() => match frame {
                Some(frame) => {
                    // Revocation wins over a frame that is still being written.
                    let written = tokio::select! {
                        biased;
                        control = attachment.control.recv() => Err(control),
                        written = send(&mut socket, Message::Text(frame.into()), WRITE_TIMEOUT) => Ok(written),
                    };
                    match written {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(control) => {
                            end(&mut socket, control).await;
                            break;
                        }
                    }
                }
                None => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Pong(_))) => last_pong = Instant::now(),
                Some(Ok(Message::Ping(_))) => {}
                Some(Ok(Message::Text(_) | Message::Binary(_))) => {
                    close(&mut socket, CLOSE_PROTOCOL).await;
                    break;
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
            },
            _ = ping.tick() => {
                if last_pong.elapsed() > PONG_TIMEOUT {
                    break;
                }
                if !send(&mut socket, Message::Ping(Default::default()), WRITE_TIMEOUT).await {
                    break;
                }
            }
        }
    }
    session.detach_stream(attachment.id);
}
