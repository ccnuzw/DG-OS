//! Accept loop (technical design "外部调用与补偿"): HTTP/1.1 with upgrades,
//! headers within 10 s, handlers within 30 s, at most 64 connections and 16
//! per source address. The TCP peer goes into the request as [`PeerAddr`],
//! the connection's slot as [`ConnectionSlot`]: an upgraded WebSocket keeps
//! the slot after hyper's connection future has finished.

use crate::http::PeerAddr;
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Router;
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const HANDLER_TIMEOUT: Duration = Duration::from_secs(30);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
pub const MAX_CONNECTIONS: usize = 64;
pub const MAX_CONNECTIONS_PER_IP: usize = 16;

#[derive(Default)]
struct Counts {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
}

struct Ticket {
    counts: Arc<Mutex<Counts>>,
    ip: IpAddr,
}

/// One counted connection. Released when the last clone is dropped: the
/// connection task's copy and, after an upgrade, the WebSocket task's copy.
#[derive(Clone)]
pub struct ConnectionSlot(#[allow(dead_code)] Arc<Ticket>);

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        counts.total -= 1;
        if let Some(n) = counts.per_ip.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                counts.per_ip.remove(&self.ip);
            }
        }
    }
}

fn ticket(counts: &Arc<Mutex<Counts>>, ip: IpAddr) -> Option<Ticket> {
    let mut guard = counts.lock().unwrap_or_else(|e| e.into_inner());
    let mine = guard.per_ip.get(&ip).copied().unwrap_or(0);
    if guard.total >= MAX_CONNECTIONS || mine >= MAX_CONNECTIONS_PER_IP {
        return None;
    }
    guard.total += 1;
    *guard.per_ip.entry(ip).or_default() += 1;
    Some(Ticket {
        counts: counts.clone(),
        ip,
    })
}

/// Errors that end one `accept` but not the listener.
fn transient(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
    ) || matches!(error.raw_os_error(), Some(23 | 24 | 10024))
}

/// Serve until `shutdown`. `Err` means the listener itself failed.
pub async fn serve(
    listener: TcpListener,
    router: Router,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let counts = Arc::new(Mutex::new(Counts::default()));
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT);
    loop {
        let accepted = tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            accepted = listener.accept() => accepted,
        };
        let (stream, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) if transient(&error) => {
                tracing::warn!("LAN accept failed transiently");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        let Some(ticket) = ticket(&counts, peer.ip()) else {
            drop(stream);
            continue;
        };
        let router = router.clone();
        let builder = builder.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            let slot = ConnectionSlot(Arc::new(ticket));
            let service =
                hyper::service::service_fn(move |mut request: hyper::Request<Incoming>| {
                    request.extensions_mut().insert(PeerAddr(peer));
                    request.extensions_mut().insert(slot.clone());
                    let router = router.clone();
                    async move {
                        match tokio::time::timeout(
                            HANDLER_TIMEOUT,
                            router.oneshot(request.map(Body::new)),
                        )
                        .await
                        {
                            Ok(response) => response,
                            Err(_) => Ok(StatusCode::SERVICE_UNAVAILABLE.into_response()),
                        }
                    }
                });
            let connection = builder
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades();
            let mut connection = std::pin::pin!(connection);
            tokio::select! {
                _ = connection.as_mut() => {}
                _ = shutdown.cancelled() => {
                    connection.as_mut().graceful_shutdown();
                    let _ = tokio::time::timeout(DRAIN_TIMEOUT, connection).await;
                }
            }
        });
    }
}
