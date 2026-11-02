//! Service lifecycle (technical design §1, pseudocode B10-22～B10-30,
//! B10-69～B10-71): the switch and preferred port in `host_settings`, the
//! listener with its fallback ports, the network watch, failure and exit.
//!
//! One async mutex serializes start, stop, the switch, the port and the
//! failure paths. The fallback port is never written back (D107).

use crate::core::{LanCore, LanDevice, RevokeReason};
use crate::net::{bind_listener, pick_address, port_candidates, valid_port, Network, DEFAULT_PORT};
use crate::spa::SpaFiles;
use async_trait::async_trait;
use dgos_db::{HostSettingsRepository, LanAuditPage, LanAuditRepository};
use dgos_types::{DgosError, DgosResult};
use futures::future::BoxFuture;
use serde::Serialize;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const ENABLED_KEY: &str = "lan.enabled";
pub const PORT_KEY: &str = "lan.port";

/// `1010001` start failed, `1010002` no free port, `1010003` no private network.
pub const START_FAILED: u32 = 1010001;
pub const NO_PORT: u32 = 1010002;
pub const NO_NETWORK: u32 = 1010003;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LanServiceState {
    Stopped,
    Starting,
    Running,
    Failed,
}

/// Interface contract §14.8.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LanStatus {
    pub enabled: bool,
    pub state: LanServiceState,
    pub address: Option<String>,
    pub preferred_port: u16,
    pub port_fallback: bool,
    pub failure_code: Option<u32>,
    pub session_count: usize,
    pub code_expires_at: Option<i64>,
}

/// Plaintext code, returned only here (D099 item 6).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LanIssuedCode {
    pub code: String,
    pub expires_at: i64,
    pub pair_url: String,
}

/// The two persisted values. Production is `host_settings`.
#[async_trait]
pub trait LanSettings: Send + Sync + 'static {
    async fn get(&self, key: &str) -> DgosResult<Option<String>>;
    async fn set(&self, key: &str, value: &str) -> DgosResult<()>;
}

#[async_trait]
impl LanSettings for HostSettingsRepository {
    async fn get(&self, key: &str) -> DgosResult<Option<String>> {
        HostSettingsRepository::get(self, key).await
    }
    async fn set(&self, key: &str, value: &str) -> DgosResult<()> {
        HostSettingsRepository::set(self, key, value, chrono::Utc::now().timestamp()).await
    }
}

pub struct LanConfig {
    /// Bound-address check (D101 item 3).
    pub network_check: Duration,
    /// Session and code expiry sweep (B10-61).
    pub sweep: Duration,
    pub spa: Option<SpaFiles>,
    pub locale: Arc<dyn crate::locale::LocaleSource>,
}

impl Default for LanConfig {
    fn default() -> Self {
        Self {
            network_check: Duration::from_secs(10),
            sweep: Duration::from_secs(60),
            spa: None,
            locale: Arc::new(crate::locale::DefaultLocaleSource),
        }
    }
}

struct Running {
    addr: SocketAddrV4,
    shutdown: CancellationToken,
    server: JoinHandle<()>,
}

struct Life {
    enabled: bool,
    state: LanServiceState,
    failure: Option<u32>,
    preferred: u16,
    running: Option<Running>,
    generation: u64,
}

pub struct LanService {
    core: Arc<LanCore>,
    settings: Arc<dyn LanSettings>,
    network: Arc<dyn Network>,
    audit: Option<LanAuditRepository>,
    config: LanConfig,
    life: Mutex<Life>,
    me: Weak<LanService>,
}

fn void_reason(reason: RevokeReason) -> &'static str {
    reason.as_str()
}

impl LanService {
    pub fn new(
        core: Arc<LanCore>,
        settings: Arc<dyn LanSettings>,
        network: Arc<dyn Network>,
        audit: Option<LanAuditRepository>,
        config: LanConfig,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            core,
            settings,
            network,
            audit,
            config,
            life: Mutex::new(Life {
                enabled: true,
                state: LanServiceState::Stopped,
                failure: None,
                preferred: DEFAULT_PORT,
                running: None,
                generation: 0,
            }),
            me: me.clone(),
        })
    }

    pub fn core(&self) -> &Arc<LanCore> {
        &self.core
    }

    async fn read_enabled(&self) -> bool {
        match self.settings.get(ENABLED_KEY).await {
            Ok(value) => value.as_deref() != Some("false"),
            Err(_) => {
                tracing::warn!("lan.enabled could not be read; using the default (on)");
                true
            }
        }
    }

    async fn read_port(&self) -> u16 {
        match self.settings.get(PORT_KEY).await {
            Ok(Some(value)) => value
                .parse::<i64>()
                .ok()
                .and_then(valid_port)
                .unwrap_or(DEFAULT_PORT),
            Ok(None) => DEFAULT_PORT,
            Err(_) => {
                tracing::warn!("lan.port could not be read; using the default");
                DEFAULT_PORT
            }
        }
    }

    /// Application start (B10-22): read the switch and port, start when on.
    /// A start failure is only shown in the status (D039).
    pub async fn on_app_start(&self) -> LanStatus {
        let mut life = self.life.lock().await;
        life.enabled = self.read_enabled().await;
        life.preferred = self.read_port().await;
        if life.enabled {
            self.start_locked(&mut life);
        }
        self.status_locked(&life)
    }

    /// B10-23～B10-25. Never writes `lan.port`.
    fn start_locked(&self, life: &mut Life) {
        if life.running.is_some() {
            return;
        }
        life.failure = None;
        let Some(ip) = pick_address(self.network.as_ref()) else {
            life.state = LanServiceState::Failed;
            life.failure = Some(NO_NETWORK);
            return;
        };
        let mut bound = None;
        for port in port_candidates(life.preferred) {
            match bind_listener(ip, port) {
                Ok(listener) => {
                    bound = Some((listener, port));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
                Err(_) => {
                    life.state = LanServiceState::Failed;
                    life.failure = Some(START_FAILED);
                    return;
                }
            }
        }
        let Some((listener, port)) = bound else {
            life.state = LanServiceState::Failed;
            life.failure = Some(NO_PORT);
            return;
        };
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(_) => {
                life.state = LanServiceState::Failed;
                life.failure = Some(START_FAILED);
                return;
            }
        };
        life.generation += 1;
        let generation = life.generation;
        let addr = SocketAddrV4::new(ip, port);
        let shutdown = CancellationToken::new();
        let router = crate::http::router_with_locale(
            self.core.clone(),
            addr,
            self.config.spa.clone(),
            self.config.locale.clone(),
        );
        self.core.set_accepting(true);
        let me = self.me.clone();
        let token = shutdown.clone();
        let server = tokio::spawn(async move {
            if crate::server::serve(listener, router, token).await.is_err() {
                if let Some(service) = me.upgrade() {
                    service.listener_failed(generation).await;
                }
            }
        });
        tokio::spawn(monitor(
            self.me.clone(),
            generation,
            ip,
            shutdown.clone(),
            self.config.network_check,
            self.config.sweep,
        ));
        life.running = Some(Running {
            addr,
            shutdown,
            server,
        });
        life.state = LanServiceState::Running;
    }

    /// Revoke everything, void the code, stop listening and wait until the
    /// port is released. Does not touch `lan.enabled`.
    async fn stop_locked(&self, life: &mut Life, reason: RevokeReason) {
        self.core.set_accepting(false);
        self.core.revoke_all(reason).await;
        self.core.void_code(void_reason(reason));
        if let Some(running) = life.running.take() {
            running.shutdown.cancel();
            if tokio::time::timeout(Duration::from_secs(5), running.server)
                .await
                .is_err()
            {
                tracing::warn!("LAN listener did not stop in time");
            }
        }
        life.state = LanServiceState::Stopped;
        life.failure = None;
    }

    /// B10-28: the listener ended unexpectedly. The switch stays on; no restart (D088).
    fn listener_failed(self: Arc<Self>, generation: u64) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let mut life = self.life.lock().await;
            if life.generation != generation || life.running.is_none() {
                return;
            }
            if let Some(running) = life.running.take() {
                running.shutdown.cancel();
            }
            self.core.set_accepting(false);
            self.core.revoke_all(RevokeReason::ServiceFailed).await;
            self.core
                .void_code(void_reason(RevokeReason::ServiceFailed));
            life.state = LanServiceState::Failed;
            life.failure = Some(START_FAILED);
        })
    }

    /// B10-29: the bound address left the host's interfaces. Rebind.
    fn network_lost(self: Arc<Self>, generation: u64) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let mut life = self.life.lock().await;
            if life.generation != generation || life.running.is_none() {
                return;
            }
            self.stop_locked(&mut life, RevokeReason::NetworkChanged)
                .await;
            self.start_locked(&mut life);
        })
    }

    pub async fn status(&self) -> LanStatus {
        let life = self.life.lock().await;
        self.status_locked(&life)
    }

    fn status_locked(&self, life: &Life) -> LanStatus {
        let addr = life.running.as_ref().map(|r| r.addr);
        LanStatus {
            enabled: life.enabled,
            state: life.state,
            address: addr.map(|a| a.to_string()),
            preferred_port: life.preferred,
            port_fallback: addr.is_some_and(|a| a.port() != life.preferred),
            failure_code: if life.state == LanServiceState::Failed {
                life.failure
            } else {
                None
            },
            session_count: self.core.session_count(),
            code_expires_at: addr.and(self.core.code_expires_at()),
        }
    }

    /// B10-26, B10-27. Persists first; a start failure is only in the status.
    pub async fn set_enabled(&self, enabled: bool) -> DgosResult<LanStatus> {
        let mut life = self.life.lock().await;
        self.settings
            .set(ENABLED_KEY, if enabled { "true" } else { "false" })
            .await
            .map_err(|_| DgosError::InternalError("lan settings unavailable".into()))?;
        life.enabled = enabled;
        if enabled {
            if life.running.is_none() {
                self.start_locked(&mut life);
            }
        } else {
            self.stop_locked(&mut life, RevokeReason::SwitchOff).await;
        }
        Ok(self.status_locked(&life))
    }

    /// B10-69, B10-70. Invalid: `1010012`, nothing saved.
    pub async fn set_port(&self, port: i64) -> DgosResult<LanStatus> {
        let port = valid_port(port).ok_or(DgosError::LanPortInvalid)?;
        let mut life = self.life.lock().await;
        self.settings
            .set(PORT_KEY, &port.to_string())
            .await
            .map_err(|_| DgosError::InternalError("lan settings unavailable".into()))?;
        life.preferred = port;
        if life.running.as_ref().is_some_and(|r| r.addr.port() != port) {
            self.stop_locked(&mut life, RevokeReason::PortChanged).await;
            self.start_locked(&mut life);
        }
        Ok(self.status_locked(&life))
    }

    /// B10-45, B10-46. `None` when not running or no issuable user.
    pub async fn issue_code(&self) -> Option<LanIssuedCode> {
        let life = self.life.lock().await;
        let addr = life.running.as_ref()?.addr;
        let issued = self.core.issue_code()?;
        Some(LanIssuedCode {
            pair_url: format!("http://{addr}/#code={}", issued.code),
            code: issued.code,
            expires_at: issued.expires_at,
        })
    }

    pub fn list_devices(&self) -> Vec<LanDevice> {
        self.core.list_devices()
    }

    pub async fn disconnect_device(&self, session_id: &str) -> bool {
        self.core.disconnect(session_id).await
    }

    pub async fn disconnect_all(&self) -> usize {
        self.core.revoke_all(RevokeReason::HostDisconnectAll).await
    }

    /// FR-008 session end, step 4.
    pub async fn revoke_issued_by(&self, user: &str) {
        self.core.revoke_issued_by(user).await
    }

    pub async fn list_audit(
        &self,
        user: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> DgosResult<LanAuditPage> {
        let Some(audit) = &self.audit else {
            return Ok(LanAuditPage {
                items: Vec::new(),
                next_cursor: None,
            });
        };
        audit
            .list(user, cursor, limit)
            .await
            .map_err(|_| DgosError::InternalError("lan audit unavailable".into()))
    }

    /// B10-30: application exit. The switch keeps its value.
    pub async fn shutdown(&self) {
        let mut life = self.life.lock().await;
        self.stop_locked(&mut life, RevokeReason::AppExit).await;
    }
}

async fn monitor(
    service: Weak<LanService>,
    generation: u64,
    ip: Ipv4Addr,
    shutdown: CancellationToken,
    network_check: Duration,
    sweep: Duration,
) {
    let mut check = tokio::time::interval(network_check);
    let mut expire = tokio::time::interval(sweep);
    check.tick().await;
    expire.tick().await;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = check.tick() => {
                let Some(service) = service.upgrade() else { return };
                if !service.network.candidate_ipv4().contains(&ip) {
                    tokio::spawn(service.network_lost(generation));
                    return;
                }
            }
            _ = expire.tick() => {
                let Some(service) = service.upgrade() else { return };
                service.core.sweep().await;
            }
        }
    }
}
