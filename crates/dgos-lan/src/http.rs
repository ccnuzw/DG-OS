//! The remote HTTP whitelist (interface contract §14.2～§14.6, OpenAPI).
//!
//! Order per request: the `gate` layer checks peer, `Host` and `Origin`
//! (steps 1～3); routing is the whitelist (step 4: other `/api/` paths 404,
//! other methods 405, both empty); static SPA and pairing pass (step 5);
//! [`Admitted`] runs steps 6～8 before any body is read. No CORS header is
//! ever written and `OPTIONS` is not handled.
//!
//! Chat goes only through `LanCore` and its `ChatBackend`; nothing here
//! names an account, module, MCP, Provider or Skills management service.

use crate::code::token_hash;
use crate::core::{LanCore, PairError, RemoteError, SessionGuard};
use crate::net::is_private_lan;
use crate::spa::SpaFiles;
use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use dgos_workbench::ChatTargetInput;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

pub const COOKIE: &str = "dgos_lan";
const BODY_LIMIT: usize = 64 * 1024;
const PAIR_BODY_LIMIT: usize = 1024;
const BODY_TIMEOUT: Duration = Duration::from_secs(10);

/// The TCP peer, inserted by the accept loop. Never taken from a header.
#[derive(Clone, Copy, Debug)]
pub struct PeerAddr(pub SocketAddr);

/// The peer after step 1: a private IPv4 address.
#[derive(Clone, Copy, Debug)]
struct Peer(Ipv4Addr);

#[derive(Clone)]
struct AppState {
    core: Arc<LanCore>,
    host: Arc<str>,
    origin: Arc<str>,
    csp: Arc<str>,
    spa: Option<SpaFiles>,
}

fn empty(status: StatusCode) -> Response {
    status.into_response()
}

fn lan_error(status: StatusCode, code: u32, message: &str, detail: Option<Value>) -> Response {
    let mut body = json!({ "code": code, "message": message });
    if let Some(detail) = detail {
        body["detail"] = detail;
    }
    (status, Json(body)).into_response()
}

fn not_allowed() -> Response {
    lan_error(StatusCode::FORBIDDEN, 1010007, "ip_not_allowed", None)
}

fn cookie_header(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).expect("cookie header is ASCII")
}

/// `HttpOnly; SameSite=Strict; Path=/`, no `Secure` (no TLS), `Domain`,
/// `Max-Age` or `Expires` (D088).
fn session_cookie(token: &str) -> HeaderValue {
    cookie_header(&format!(
        "{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/"
    ))
}

fn clear_cookie() -> HeaderValue {
    cookie_header(&format!(
        "{COOKIE}=; Max-Age=0; HttpOnly; SameSite=Strict; Path=/"
    ))
}

fn session_invalid() -> Response {
    let mut response = lan_error(StatusCode::UNAUTHORIZED, 1010006, "session_not_found", None);
    response
        .headers_mut()
        .insert(header::SET_COOKIE, clear_cookie());
    response
}

fn cookie_hash(headers: &HeaderMap) -> Option<[u8; 32]> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().strip_prefix("dgos_lan="))
        .find_map(token_hash)
}

fn is_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// HTTP status of a chat or Provider code (error-code registry "HTTP 状态码").
fn chat_status(code: u32) -> StatusCode {
    match code {
        1006001 | 1006002 | 1001001 | 1001004 => StatusCode::NOT_FOUND,
        1006007 | 1006008 | 1006009 | 1006015 | 1006016 | 1001016 | 1001017 | 1001018 | 1001020
        | 1001022 | 1001023 => StatusCode::BAD_REQUEST,
        1006010..=1006014 | 1006017 | 1001012 => StatusCode::CONFLICT,
        1001002 | 1001015 => StatusCode::UNAUTHORIZED,
        1001019 => StatusCode::FORBIDDEN,
        1001024 => StatusCode::PAYMENT_REQUIRED,
        1001005 => StatusCode::TOO_MANY_REQUESTS,
        1001003 => StatusCode::SERVICE_UNAVAILABLE,
        1001006 => StatusCode::GATEWAY_TIMEOUT,
        1001007 | 1001008 | 1001014 | 1001021 | 1001025 => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Chat errors carry no `detail` on the LAN: in particular `1006012` has no
/// fingerprint or host, so the remote cannot confirm a new destination (D099 item 8).
fn remote_error(error: RemoteError) -> Response {
    match error {
        RemoteError::ChatLimit => lan_error(
            StatusCode::TOO_MANY_REQUESTS,
            1010011,
            "lan_rate_limited",
            Some(json!({ "scope": "chat_sessions" })),
        ),
        RemoteError::Chat(error) => {
            let code = error.code();
            lan_error(chat_status(code), code, error.message(), None)
        }
    }
}

// -------------------------------------------------------------------------------------------
// Steps 1～3 and response headers

async fn gate(State(app): State<AppState>, mut request: Request, next: Next) -> Response {
    let peer = request.extensions().get::<PeerAddr>().map(|p| p.0);
    let peer_v4 = match peer {
        Some(SocketAddr::V4(v4)) => Some(*v4.ip()),
        _ => None,
    };
    let Some(ip) = peer_v4.filter(|ip| is_private_lan(*ip)) else {
        app.core.reject_request("peer_not_private", peer_v4);
        return not_allowed();
    };
    let headers = request.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if host != Some(&*app.host) {
        app.core.reject_request("host_mismatch", Some(ip));
        return not_allowed();
    }
    let origin = headers.get(header::ORIGIN);
    let method = request.method();
    let needs_origin = method == Method::POST
        || method == Method::DELETE
        || is_upgrade(headers)
        || origin.is_some();
    if needs_origin && origin.and_then(|v| v.to_str().ok()) != Some(&*app.origin) {
        app.core.reject_request("origin_mismatch", Some(ip));
        return not_allowed();
    }
    let api = request.uri().path().starts_with("/api/");
    request.extensions_mut().insert(Peer(ip));
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if api {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    } else if let Ok(csp) = HeaderValue::from_str(&app.csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    response
}

// -------------------------------------------------------------------------------------------
// Extractors

/// Steps 6～8 of §14.2. Holds the session gate until the handler returns.
pub struct Admitted(pub SessionGuard);

impl FromRequestParts<AppState> for Admitted {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, app: &AppState) -> Result<Self, Response> {
        let Peer(peer) = *parts
            .extensions
            .get::<Peer>()
            .ok_or_else(|| empty(StatusCode::FORBIDDEN))?;
        app.core
            .admit(peer, cookie_hash(&parts.headers))
            .await
            .map(Admitted)
            .map_err(|_| session_invalid())
    }
}

/// `application/json` (else 415), at most `LIMIT` bytes (else 413), read
/// within 10 s and decoded with `deny_unknown_fields` (else 400).
/// All rejections have an empty body (D099 item 9).
struct LanJson<T, const LIMIT: usize>(T);

impl<T, S, const LIMIT: usize> FromRequest<S> for LanJson<T, LIMIT>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, _: &S) -> Result<Self, Response> {
        let headers = request.headers();
        let json = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"));
        if !json {
            return Err(empty(StatusCode::UNSUPPORTED_MEDIA_TYPE));
        }
        let declared = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok());
        if declared.is_some_and(|length| length > LIMIT) {
            return Err(empty(StatusCode::PAYLOAD_TOO_LARGE));
        }
        let body = Limited::new(request.into_body(), LIMIT);
        let bytes = match tokio::time::timeout(BODY_TIMEOUT, body.collect()).await {
            Err(_) => return Err(empty(StatusCode::BAD_REQUEST)),
            Ok(Err(error)) if error.downcast_ref::<LengthLimitError>().is_some() => {
                return Err(empty(StatusCode::PAYLOAD_TOO_LARGE))
            }
            Ok(Err(_)) => return Err(empty(StatusCode::BAD_REQUEST)),
            Ok(Ok(collected)) => collected.to_bytes(),
        };
        serde_json::from_slice(&bytes)
            .map(LanJson)
            .map_err(|_| empty(StatusCode::BAD_REQUEST))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairInput {
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetInput {
    provider_id: String,
    model_id: String,
    skill_id: Option<String>,
    no_skill: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateChatInput {
    target: TargetInput,
}

/// No `tools`, no `accept_destination` (D099 item 8).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendInput {
    content: String,
    client_request_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegenerateInput {
    client_request_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessagesQuery {
    before_seq: Option<i64>,
    limit: Option<u32>,
}

// -------------------------------------------------------------------------------------------
// Handlers

async fn pair(
    State(app): State<AppState>,
    Extension(Peer(peer)): Extension<Peer>,
    headers: HeaderMap,
    LanJson(input): LanJson<PairInput, PAIR_BODY_LIMIT>,
) -> Response {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    match app
        .core
        .pair(peer, cookie_hash(&headers), &input.code, user_agent)
        .await
    {
        Ok(paired) => {
            let mut response = Json(paired.status).into_response();
            response
                .headers_mut()
                .insert(header::SET_COOKIE, session_cookie(&paired.token));
            response
        }
        Err(PairError::RateLimited { scope, retry_after }) => {
            let mut detail = json!({ "scope": scope });
            if let Some(seconds) = retry_after {
                detail["retry_after_seconds"] = json!(seconds);
            }
            let mut response = lan_error(
                StatusCode::TOO_MANY_REQUESTS,
                1010011,
                "lan_rate_limited",
                Some(detail),
            );
            if let Some(seconds) = retry_after {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
            }
            response
        }
        Err(PairError::InvalidCode) => lan_error(
            StatusCode::UNAUTHORIZED,
            1010004,
            "invalid_access_token",
            None,
        ),
        Err(PairError::ExpiredCode) => {
            lan_error(StatusCode::UNAUTHORIZED, 1010005, "token_expired", None)
        }
    }
}

async fn session(Admitted(guard): Admitted) -> Response {
    Json(guard.session.status()).into_response()
}

async fn logout(
    State(app): State<AppState>,
    Extension(Peer(peer)): Extension<Peer>,
    headers: HeaderMap,
) -> Response {
    app.core.logout(peer, cookie_hash(&headers)).await;
    let mut response = empty(StatusCode::NO_CONTENT);
    response
        .headers_mut()
        .insert(header::SET_COOKIE, clear_cookie());
    response
}

async fn targets(State(app): State<AppState>, Admitted(guard): Admitted) -> Response {
    match app.core.targets(&guard).await {
        Ok(targets) => Json(targets).into_response(),
        Err(error) => remote_error(error),
    }
}

async fn create_chat(
    State(app): State<AppState>,
    Admitted(guard): Admitted,
    LanJson(input): LanJson<CreateChatInput, BODY_LIMIT>,
) -> Response {
    let target = ChatTargetInput {
        provider_id: Some(input.target.provider_id),
        model_id: Some(input.target.model_id),
        skill_id: input.target.skill_id,
        no_skill: input.target.no_skill,
    };
    match app.core.create_chat(&guard, target).await {
        Ok(chat) => (StatusCode::CREATED, Json(chat)).into_response(),
        Err(error) => remote_error(error),
    }
}

async fn list_chats(State(app): State<AppState>, Admitted(guard): Admitted) -> Response {
    match app.core.list_chats(&guard).await {
        Ok(items) => Json(json!({ "items": items })).into_response(),
        Err(error) => remote_error(error),
    }
}

async fn list_messages(
    State(app): State<AppState>,
    Admitted(guard): Admitted,
    Path(chat_id): Path<String>,
    query: Result<Query<MessagesQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return empty(StatusCode::BAD_REQUEST);
    };
    match app
        .core
        .list_messages(&guard, &chat_id, query.before_seq, query.limit)
        .await
    {
        Ok(page) => Json(page).into_response(),
        Err(error) => remote_error(error),
    }
}

async fn send(
    State(app): State<AppState>,
    Admitted(guard): Admitted,
    Path(chat_id): Path<String>,
    LanJson(input): LanJson<SendInput, BODY_LIMIT>,
) -> Response {
    match app
        .core
        .send(&guard, &chat_id, input.content, input.client_request_id)
        .await
    {
        Ok(ack) => (StatusCode::ACCEPTED, Json(ack)).into_response(),
        Err(error) => remote_error(error),
    }
}

async fn regenerate(
    State(app): State<AppState>,
    Admitted(guard): Admitted,
    Path(message_id): Path<String>,
    LanJson(input): LanJson<RegenerateInput, BODY_LIMIT>,
) -> Response {
    match app
        .core
        .regenerate(&guard, &message_id, input.client_request_id)
        .await
    {
        Ok(ack) => (StatusCode::ACCEPTED, Json(ack)).into_response(),
        Err(error) => remote_error(error),
    }
}

async fn cancel(
    State(app): State<AppState>,
    Admitted(guard): Admitted,
    Path(task_id): Path<String>,
) -> Response {
    Json(json!({ "cancelled": app.core.cancel(&guard, &task_id) })).into_response()
}

async fn stream(
    Admitted(guard): Admitted,
    slot: Option<axum::Extension<crate::server::ConnectionSlot>>,
    upgrade: Result<
        axum::extract::ws::WebSocketUpgrade,
        axum::extract::ws::rejection::WebSocketUpgradeRejection,
    >,
) -> Response {
    let upgrade = match upgrade {
        Ok(upgrade) => upgrade,
        Err(rejection) => return empty(rejection.into_response().status()),
    };
    // The gate is not held across the upgrade; the stream re-checks revocation.
    let session = guard.session.clone();
    drop(guard);
    // The upgraded socket stays counted against the connection caps until it ends.
    upgrade
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            crate::ws::run(socket, session).await
        })
        .into_response()
}

/// Static SPA for non-`/api/` `GET`; every other unmatched request is an empty 404/405.
async fn fallback(State(app): State<AppState>, request: Request) -> Response {
    let path = request.uri().path();
    if path.starts_with("/api/") {
        return empty(StatusCode::NOT_FOUND);
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let Some(spa) = &app.spa else {
        return empty(StatusCode::NOT_FOUND);
    };
    if let Some(name) = path.strip_prefix("/assets/") {
        return match spa.asset(name).await {
            Some((body, kind)) => {
                ([(header::CONTENT_TYPE, kind)], Body::from(body)).into_response()
            }
            None => empty(StatusCode::NOT_FOUND),
        };
    }
    match spa.index().await {
        Some(body) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            Body::from(body),
        )
            .into_response(),
        None => empty(StatusCode::NOT_FOUND),
    }
}

/// The whitelist router for one bound address.
pub fn router(core: Arc<LanCore>, bound: SocketAddrV4, spa: Option<SpaFiles>) -> Router {
    let host = bound.to_string();
    let app = AppState {
        core,
        origin: format!("http://{host}").into(),
        csp: format!(
            "default-src 'self'; connect-src 'self' ws://{host}; script-src 'self'; \
             style-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; \
             form-action 'self'"
        )
        .into(),
        host: host.into(),
        spa,
    };
    Router::new()
        .route("/api/v1/lan/pair", post(pair))
        .route("/api/v1/lan/session", get(session))
        .route("/api/v1/lan/logout", post(logout))
        .route("/api/v1/lan/targets", get(targets))
        .route(
            "/api/v1/lan/chat/sessions",
            post(create_chat).get(list_chats),
        )
        .route(
            "/api/v1/lan/chat/sessions/{session_id}/messages",
            get(list_messages).post(send),
        )
        .route(
            "/api/v1/lan/chat/messages/{message_id}/regenerate",
            post(regenerate),
        )
        .route("/api/v1/lan/chat/tasks/{task_id}/cancel", post(cancel))
        .route("/api/v1/lan/stream", get(stream))
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(app.clone(), gate))
        .with_state(app)
}
