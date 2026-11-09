//! Shared fixture for the `dgos-lan` integration tests.
//!
//! - A loopback upstream speaking just enough OpenAI-compatible SSE for
//!   scripted replies. It proves host behaviour, not real Provider behaviour.
//! - Real FR-006 managers (`ChatManager`, `ProviderManager`, `SkillsManager`)
//!   on a migrated temp SQLite database.
//! - The production router on 127.0.0.1. The TCP peer of a request is
//!   injected by this harness's accept loop from the `x-test-peer` header
//!   (default 192.168.1.10), so private, public and changed source addresses
//!   can be exercised from one machine. Production takes the peer from the
//!   socket only (`server.rs`).
#![allow(dead_code)]

use dgos_db::{
    migrations::Migrator, sea_orm::DatabaseConnection, sea_orm_migration::MigratorTrait,
};
use dgos_lan::audit::MemoryAudit;
use dgos_lan::{ChatHost, Clock, Issuer, LanCore, ManagedChat, PeerAddr, SpaFiles};
use dgos_providers::{
    test_support::{seed_provider, MemoryCredentialVault},
    ProviderManager,
};
use dgos_skills::SkillsManager;
use dgos_types::ProviderConfig;
use dgos_workbench::ChatManager;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower::ServiceExt;

pub const ALICE: &str = "alice";
pub const BOB: &str = "bob";
pub const PEER: &str = "192.168.1.10";

// ---------------------------------------------------------------------------------------------
// Scripted upstream

#[derive(Clone)]
pub enum Step {
    Send(String),
    /// Hold the stream open until the client goes away.
    Hold,
}

pub fn delta(text: &str) -> Step {
    Step::Send(format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":text},"finish_reason":null}]})
    ))
}

pub fn reply(text: &str) -> Vec<Step> {
    vec![
        delta(text),
        Step::Send(format!(
            "data: {}\n\n",
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
        )),
        Step::Send(format!(
            "data: {}\n\n",
            json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}})
        )),
        Step::Send("data: [DONE]\n\n".into()),
    ]
}

pub struct Upstream {
    pub addr: SocketAddr,
    scripts: Arc<Mutex<VecDeque<Vec<Step>>>>,
    hits: Arc<AtomicUsize>,
    open: Arc<AtomicUsize>,
}

impl Upstream {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let scripts: Arc<Mutex<VecDeque<Vec<Step>>>> = Arc::default();
        let hits = Arc::new(AtomicUsize::new(0));
        let open = Arc::new(AtomicUsize::new(0));
        let (s, h, o) = (scripts.clone(), hits.clone(), open.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let (s, h, o) = (s.clone(), h.clone(), o.clone());
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let header_end = loop {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buffer[..header_end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    while buffer.len() < header_end + length {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                    }
                    h.fetch_add(1, Ordering::SeqCst);
                    let steps = s
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or_else(|| reply("default reply"));
                    o.fetch_add(1, Ordering::SeqCst);
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
                        )
                        .await;
                    for step in steps {
                        match step {
                            Step::Send(text) => {
                                if socket.write_all(text.as_bytes()).await.is_err() {
                                    break;
                                }
                                let _ = socket.flush().await;
                            }
                            Step::Hold => {
                                while matches!(socket.read(&mut chunk).await, Ok(n) if n > 0) {}
                                break;
                            }
                        }
                    }
                    o.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Self {
            addr,
            scripts,
            hits,
            open,
        }
    }

    pub fn script(&self, steps: Vec<Step>) {
        self.scripts.lock().unwrap().push_back(steps);
    }
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
    pub fn open(&self) -> usize {
        self.open.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------------------------
// Host fakes

#[derive(Default)]
pub struct FakeClock(pub AtomicI64);

impl FakeClock {
    pub fn at(now: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(now)))
    }
    pub fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct FakeIssuer(pub Mutex<Option<String>>);

impl FakeIssuer {
    pub fn signed_in(user: &str) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Some(user.into()))))
    }
    pub fn set(&self, user: Option<&str>) {
        *self.0.lock().unwrap() = user.map(str::to_owned);
    }
}

impl Issuer for FakeIssuer {
    fn current_user(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

pub struct TestChatHost {
    pub chat: ChatManager,
    pub providers: ProviderManager,
    pub skills: Arc<RwLock<SkillsManager>>,
}

impl ChatHost for TestChatHost {
    fn chat(&self) -> ChatManager {
        self.chat.clone()
    }
    fn providers(&self) -> &ProviderManager {
        &self.providers
    }
    fn skills(&self) -> Arc<RwLock<SkillsManager>> {
        self.skills.clone()
    }
}

// ---------------------------------------------------------------------------------------------
// Environment

pub struct Env {
    pub db: DatabaseConnection,
    pub host: Arc<TestChatHost>,
    pub up: Upstream,
    pub provider_id: String,
    pub clock: Arc<FakeClock>,
    pub issuer: Arc<FakeIssuer>,
    pub audit: Arc<MemoryAudit>,
    pub core: Arc<LanCore>,
    pub addr: SocketAddrV4,
    pub http: reqwest::Client,
    pub dir: tempfile::TempDir,
}

pub async fn env() -> Env {
    env_with_spa(None).await
}

pub async fn env_with_spa(spa: Option<SpaFiles>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("lan.db").display());
    let db = dgos_db::establish_connection(&url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let up = Upstream::start().await;
    let vault = Arc::new(MemoryCredentialVault::default());
    let provider = seed_provider(
        db.clone(),
        &vault,
        ProviderConfig {
            name: "Loopback".into(),
            provider_type: "openai_compatible".into(),
            base_url: format!("http://{}/v1", up.addr),
            api_key: "test-only-secret".into(),
            enabled: true,
            config: None,
        },
        ALICE,
    )
    .await
    .unwrap();
    let providers = ProviderManager::with_vault(&url, vault.clone())
        .await
        .unwrap();
    providers
        .add_manual_model(&provider.id, ALICE, "chat-model", Some("text-generation"))
        .await
        .unwrap();
    let host = Arc::new(TestChatHost {
        chat: ChatManager::new(db.clone(), dgos_db::MaintenanceGate::new()),
        providers,
        skills: Arc::new(RwLock::new(SkillsManager::new(
            db.clone(),
            dir.path().join("skills"),
        ))),
    });
    let clock = FakeClock::at(1_800_000_000);
    let issuer = FakeIssuer::signed_in(ALICE);
    let audit = Arc::new(MemoryAudit::default());
    let core = Arc::new(LanCore::new(
        clock.clone(),
        issuer.clone(),
        audit.clone(),
        Arc::new(ManagedChat(host.clone())),
    ));
    let addr = serve(core.clone(), spa).await;
    Env {
        db,
        host,
        up,
        provider_id: provider.id,
        clock,
        issuer,
        audit,
        core,
        addr,
        // Host overrides exercise admission on this socket, never an environment proxy.
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        dir,
    }
}

/// The production router on 127.0.0.1 with the harness peer injection.
pub async fn serve(core: Arc<LanCore>, spa: Option<SpaFiles>) -> SocketAddrV4 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = match listener.local_addr().unwrap() {
        SocketAddr::V4(v4) => v4,
        SocketAddr::V6(_) => unreachable!(),
    };
    let router = dgos_lan::router(core, addr, spa);
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(
                    move |mut request: hyper::Request<hyper::body::Incoming>| {
                        let ip: Ipv4Addr = request
                            .headers()
                            .get("x-test-peer")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse().ok())
                            .unwrap_or_else(|| PEER.parse().unwrap());
                        request
                            .extensions_mut()
                            .insert(PeerAddr(SocketAddr::from((ip, peer.port()))));
                        router.clone().oneshot(request.map(axum::body::Body::new))
                    },
                );
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    addr
}

pub struct Reply {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Value,
    pub text: String,
}

impl Reply {
    /// `dgos_lan=<token>` from `Set-Cookie`, ready for a `Cookie` header.
    pub fn cookie(&self) -> Option<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("dgos_lan="))
            .map(|v| v.split(';').next().unwrap().to_owned())
    }
    pub fn code(&self) -> Option<i64> {
        self.body.get("code").and_then(Value::as_i64)
    }
}

pub struct Call<'a> {
    env: &'a Env,
    method: reqwest::Method,
    path: String,
    peer: String,
    cookie: Option<String>,
    origin: Option<String>,
    host: Option<String>,
    body: Option<Value>,
    raw: Option<(String, Vec<u8>)>,
    user_agent: Option<String>,
}

impl Env {
    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn call(&self, method: &str, path: &str) -> Call<'_> {
        let method: reqwest::Method = method.parse().unwrap();
        let origin = (method == reqwest::Method::POST).then(|| self.origin());
        Call {
            env: self,
            method,
            path: path.into(),
            peer: PEER.into(),
            cookie: None,
            origin,
            host: None,
            body: None,
            raw: None,
            user_agent: None,
        }
    }

    pub fn get(&self, path: &str) -> Call<'_> {
        self.call("GET", path)
    }

    pub fn post(&self, path: &str, body: Value) -> Call<'_> {
        self.call("POST", path).json(body)
    }

    /// A fresh code from the core (the host command path, minus the service).
    pub fn code(&self) -> String {
        self.core.issue_code().expect("issuable").code
    }

    /// Pair `peer` with a fresh code and return its Cookie.
    pub async fn paired(&self, peer: &str) -> String {
        let code = self.code();
        let reply = self
            .post("/api/v1/lan/pair", json!({ "code": code }))
            .peer(peer)
            .send()
            .await;
        assert_eq!(reply.status, 200, "{}", reply.text);
        reply.cookie().unwrap()
    }

    pub fn target(&self) -> Value {
        json!({"target": {"provider_id": self.provider_id, "model_id": "chat-model"}})
    }

    pub async fn chat(&self, cookie: &str) -> String {
        let reply = self
            .post("/api/v1/lan/chat/sessions", self.target())
            .cookie(cookie)
            .send()
            .await;
        assert_eq!(reply.status, 201, "{}", reply.text);
        reply.body["id"].as_str().unwrap().to_owned()
    }

    pub async fn messages(&self, cookie: &str, chat: &str) -> Vec<Value> {
        let reply = self
            .get(&format!("/api/v1/lan/chat/sessions/{chat}/messages"))
            .cookie(cookie)
            .send()
            .await;
        assert_eq!(reply.status, 200, "{}", reply.text);
        reply.body["items"].as_array().unwrap().clone()
    }

    /// Wait until the chat's last assistant message is no longer generating.
    pub async fn settled(&self, cookie: &str, chat: &str) -> Vec<Value> {
        for _ in 0..300 {
            let items = self.messages(cookie, chat).await;
            if items
                .iter()
                .all(|m| m["status"].as_str() != Some("generating"))
            {
                return items;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("generation did not settle");
    }
}

impl<'a> Call<'a> {
    pub fn peer(mut self, peer: &str) -> Self {
        self.peer = peer.into();
        self
    }
    pub fn cookie(mut self, cookie: &str) -> Self {
        self.cookie = Some(cookie.into());
        self
    }
    pub fn origin(mut self, origin: Option<&str>) -> Self {
        self.origin = origin.map(str::to_owned);
        self
    }
    pub fn host(mut self, host: &str) -> Self {
        self.host = Some(host.into());
        self
    }
    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
    pub fn raw(mut self, content_type: &str, body: Vec<u8>) -> Self {
        self.raw = Some((content_type.into(), body));
        self
    }
    pub fn user_agent(mut self, ua: &str) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    pub async fn send(self) -> Reply {
        let url = format!("http://{}{}", self.env.addr, self.path);
        let mut request = self
            .env
            .http
            .request(self.method, url)
            .header("x-test-peer", &self.peer);
        if let Some(cookie) = &self.cookie {
            request = request.header("cookie", cookie);
        }
        if let Some(origin) = &self.origin {
            request = request.header("origin", origin);
        }
        if let Some(host) = &self.host {
            request = request.header("host", host);
        }
        if let Some(ua) = &self.user_agent {
            request = request.header("user-agent", ua);
        }
        if let Some(body) = &self.body {
            request = request.json(body);
        }
        if let Some((content_type, body)) = self.raw {
            request = request.header("content-type", content_type).body(body);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let text = response.text().await.unwrap();
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        Reply {
            status,
            headers,
            body,
            text,
        }
    }
}

pub async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..300 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached in time");
}
