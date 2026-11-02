//! FR-010 AC01/AC02/AC03/AC12/AC13/AC14 backend side and B10-22～B10-30,
//! B10-69～B10-71: switch and preferred port in settings, fallback without
//! write-back, all ports taken, no network, port change while running,
//! bound-address loss, exit, and the real accept loop's peer check.
//! Listeners bind 127.0.0.1 through a fake `Network`.

use async_trait::async_trait;
use dgos_db::ChatSession;
use dgos_lan::audit::MemoryAudit;
use dgos_lan::chat_port::{ChatBackend, LanTargets};
use dgos_lan::service::{LanSettings, ENABLED_KEY, PORT_KEY};
use dgos_lan::{
    Clock, Issuer, LanConfig, LanCore, LanService, LanServiceState, Network, SystemClock,
};
use dgos_types::DgosResult;
use dgos_workbench::{
    ChatError, ChatMessagePage, ChatSink, ChatTargetInput, GenerationJob, ListChatMessagesInput,
    RegenerateChatInput, SendChatInput, Sent,
};
use std::collections::HashMap;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct NoChat;

#[async_trait]
impl ChatBackend for NoChat {
    async fn targets(&self, _: &str) -> Result<LanTargets, ChatError> {
        Err(ChatError::Internal)
    }
    async fn create_session(&self, _: &str, _: ChatTargetInput) -> Result<ChatSession, ChatError> {
        Err(ChatError::Internal)
    }
    async fn get_session(&self, _: &str, _: &str) -> Result<ChatSession, ChatError> {
        Err(ChatError::Internal)
    }
    async fn list_messages(
        &self,
        _: &str,
        _: ListChatMessagesInput,
    ) -> Result<ChatMessagePage, ChatError> {
        Err(ChatError::Internal)
    }
    async fn session_of_message(&self, _: &str, _: &str) -> Result<Option<String>, ChatError> {
        Ok(None)
    }
    async fn session_of_task(&self, _: &str, _: &str) -> Result<Option<String>, ChatError> {
        Ok(None)
    }
    async fn send(&self, _: &str, _: &str, _: SendChatInput) -> Result<Sent, ChatError> {
        Err(ChatError::Internal)
    }
    async fn regenerate(
        &self,
        _: &str,
        _: &str,
        _: RegenerateChatInput,
    ) -> Result<Sent, ChatError> {
        Err(ChatError::Internal)
    }
    fn drive(&self, _: GenerationJob, _: Arc<dyn ChatSink>, _: Box<dyn FnOnce() + Send>) {}
    fn cancel(&self, _: &str, _: &str) -> bool {
        false
    }
    fn cancel_window(&self, _: &str) -> usize {
        0
    }
}

struct Alice;
impl Issuer for Alice {
    fn current_user(&self) -> Option<String> {
        Some("alice".into())
    }
}

#[derive(Default)]
struct MemorySettings {
    values: Mutex<HashMap<String, String>>,
    writes: Mutex<Vec<(String, String)>>,
}

impl MemorySettings {
    fn value(&self, key: &str) -> Option<String> {
        self.values.lock().unwrap().get(key).cloned()
    }
    fn writes(&self) -> Vec<(String, String)> {
        self.writes.lock().unwrap().clone()
    }
}

#[async_trait]
impl LanSettings for MemorySettings {
    async fn get(&self, key: &str) -> DgosResult<Option<String>> {
        Ok(self.value(key))
    }
    async fn set(&self, key: &str, value: &str) -> DgosResult<()> {
        self.values.lock().unwrap().insert(key.into(), value.into());
        self.writes.lock().unwrap().push((key.into(), value.into()));
        Ok(())
    }
}

struct FakeNetwork {
    candidates: Mutex<Vec<Ipv4Addr>>,
    route: Mutex<Option<Ipv4Addr>>,
}

impl FakeNetwork {
    fn loopback() -> Arc<Self> {
        Arc::new(Self {
            candidates: Mutex::new(vec![Ipv4Addr::LOCALHOST]),
            route: Mutex::new(Some(Ipv4Addr::LOCALHOST)),
        })
    }
}

impl Network for FakeNetwork {
    fn candidate_ipv4(&self) -> Vec<Ipv4Addr> {
        self.candidates.lock().unwrap().clone()
    }
    fn default_route_ipv4(&self) -> Option<Ipv4Addr> {
        *self.route.lock().unwrap()
    }
}

struct Fx {
    service: Arc<LanService>,
    settings: Arc<MemorySettings>,
    network: Arc<FakeNetwork>,
    audit: Arc<MemoryAudit>,
}

fn fx(preferred: Option<u16>, enabled: Option<&str>) -> Fx {
    let settings = Arc::new(MemorySettings::default());
    if let Some(port) = preferred {
        settings
            .values
            .lock()
            .unwrap()
            .insert(PORT_KEY.into(), port.to_string());
    }
    if let Some(enabled) = enabled {
        settings
            .values
            .lock()
            .unwrap()
            .insert(ENABLED_KEY.into(), enabled.into());
    }
    let network = FakeNetwork::loopback();
    let audit = Arc::new(MemoryAudit::default());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let core = Arc::new(LanCore::new(
        clock,
        Arc::new(Alice),
        audit.clone(),
        Arc::new(NoChat),
    ));
    let service = LanService::new(
        core,
        settings.clone(),
        network.clone(),
        None,
        LanConfig {
            network_check: Duration::from_millis(30),
            sweep: Duration::from_millis(30),
            spa: None,
            locale: Arc::new(dgos_lan::locale::DefaultLocaleSource),
        },
    );
    Fx {
        service,
        settings,
        network,
        audit,
    }
}

/// A base port with the next 20 free on 127.0.0.1 and 0.0.0.0.
/// Every call takes a distinct 32-port block, so parallel tests never share ports.
fn free_range() -> u16 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    for _ in 0..200u32 {
        let block = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let base = 20000 + ((std::process::id() % 400 * 32 + block * 32) % 40000) as u16;
        let free = (0..20).all(|i| {
            TcpListener::bind((Ipv4Addr::UNSPECIFIED, base + i)).is_ok()
                && TcpListener::bind((Ipv4Addr::LOCALHOST, base + i)).is_ok()
        });
        if free {
            return base;
        }
    }
    panic!("no free port range");
}

async fn pair(service: &LanService) -> String {
    let core = service.core();
    let code = core.issue_code().unwrap().code;
    core.pair("192.168.1.10".parse().unwrap(), None, &code, None)
        .await
        .ok()
        .unwrap();
    core.list_devices()[0].session_id.clone()
}

fn revocations(audit: &MemoryAudit) -> Vec<&'static str> {
    audit
        .take()
        .into_iter()
        .filter(|r| r.event == dgos_db::LanAuditKind::SessionRevoked)
        .map(|r| r.reason.unwrap())
        .collect()
}

#[tokio::test]
async fn starts_on_by_default_on_the_preferred_port() {
    let base = free_range();
    let fx = fx(Some(base), None);
    let status = fx.service.on_app_start().await;
    assert_eq!(status.state, LanServiceState::Running);
    assert!(status.enabled);
    assert_eq!(
        status.address.as_deref(),
        Some(format!("127.0.0.1:{base}").as_str())
    );
    assert_eq!(status.preferred_port, base);
    assert!(!status.port_fallback);
    assert!(fx.settings.writes().is_empty(), "start writes nothing");
    // The port is really taken by us.
    assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, base)).is_err());
    // The real accept loop takes the peer from the socket: loopback is not a LAN peer.
    let response = reqwest::get(format!("http://127.0.0.1:{base}/"))
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["code"], 1010007);
    fx.service.shutdown().await;
}

#[tokio::test]
async fn a_taken_preferred_port_falls_back_without_writing_it() {
    let base = free_range();
    // Another program on the wildcard address (D105).
    let occupant = TcpListener::bind((Ipv4Addr::UNSPECIFIED, base)).unwrap();
    let fx = fx(Some(base), None);
    let status = fx.service.on_app_start().await;
    assert_eq!(status.state, LanServiceState::Running);
    assert_eq!(
        status.address.as_deref(),
        Some(format!("127.0.0.1:{}", base + 1).as_str())
    );
    assert!(status.port_fallback);
    assert_eq!(status.preferred_port, base);
    assert_eq!(fx.settings.value(PORT_KEY), Some(base.to_string()));
    assert!(fx.settings.writes().is_empty());
    // The next start tries the preferred port again (B10-71).
    fx.service.shutdown().await;
    drop(occupant);
    let status = fx.service.on_app_start().await;
    assert_eq!(
        status.address.as_deref(),
        Some(format!("127.0.0.1:{base}").as_str())
    );
    assert!(!status.port_fallback);
    fx.service.shutdown().await;
}

#[tokio::test]
async fn twenty_taken_ports_fail_with_1010002_and_no_network_with_1010003() {
    let base = free_range();
    let occupants: Vec<_> = (0..20)
        .map(|i| TcpListener::bind((Ipv4Addr::UNSPECIFIED, base + i)).unwrap())
        .collect();
    let taken = fx(Some(base), None);
    let status = taken.service.on_app_start().await;
    assert_eq!(status.state, LanServiceState::Failed);
    assert_eq!(status.failure_code, Some(1010002));
    assert_eq!(status.address, None);
    assert!(status.enabled, "a failed start keeps the switch on");
    assert!(taken.service.issue_code().await.is_none());
    drop(occupants);

    let offline = fx(Some(base), None);
    offline.network.candidates.lock().unwrap().clear();
    let status = offline.service.on_app_start().await;
    assert_eq!(
        (status.state, status.failure_code),
        (LanServiceState::Failed, Some(1010003))
    );
}

#[tokio::test]
async fn the_switch_is_persisted_and_off_stops_and_disconnects() {
    let base = free_range();
    let fx = fx(Some(base), Some("false"));
    let status = fx.service.on_app_start().await;
    assert_eq!(
        (status.enabled, status.state),
        (false, LanServiceState::Stopped)
    );
    assert!(
        TcpListener::bind((Ipv4Addr::LOCALHOST, base)).is_ok(),
        "not listening"
    );

    let status = fx.service.set_enabled(true).await.unwrap();
    assert_eq!(status.state, LanServiceState::Running);
    assert_eq!(fx.settings.value(ENABLED_KEY).as_deref(), Some("true"));
    pair(&fx.service).await;
    let code = fx.service.issue_code().await.unwrap();
    assert_eq!(
        code.pair_url,
        format!("http://127.0.0.1:{base}/#code={}", code.code)
    );
    assert_eq!(code.code.len(), 26 + 6);
    assert_eq!(
        fx.service.status().await.code_expires_at,
        Some(code.expires_at)
    );
    assert_eq!(fx.service.status().await.session_count, 1);

    let status = fx.service.set_enabled(false).await.unwrap();
    assert_eq!(
        (status.enabled, status.state),
        (false, LanServiceState::Stopped)
    );
    assert_eq!(status.session_count, 0);
    assert_eq!(status.code_expires_at, None);
    assert_eq!(fx.settings.value(ENABLED_KEY).as_deref(), Some("false"));
    assert_eq!(revocations(&fx.audit), ["switch_off"]);
    assert!(
        TcpListener::bind((Ipv4Addr::LOCALHOST, base)).is_ok(),
        "port released"
    );
    assert!(fx.service.issue_code().await.is_none());
}

#[tokio::test]
async fn set_port_validates_saves_and_rebinds_only_when_it_changes() {
    let base = free_range();
    let fx = fx(Some(base), None);
    fx.service.on_app_start().await;
    for bad in [80, 1023, 65536, 6000, 10080, -5] {
        let error = fx.service.set_port(bad).await.unwrap_err();
        assert_eq!(error.error_code(), 1010012, "{bad}");
    }
    assert!(
        fx.settings.writes().is_empty(),
        "invalid ports are not saved"
    );

    pair(&fx.service).await;
    // Same port: saved, nothing revoked.
    let status = fx.service.set_port(base as i64).await.unwrap();
    assert_eq!(status.session_count, 1);
    assert!(revocations(&fx.audit).is_empty());
    // Another port while running: all sessions revoked (port_changed), new listener.
    let status = fx.service.set_port(base as i64 + 10).await.unwrap();
    assert_eq!(
        status.address.as_deref(),
        Some(format!("127.0.0.1:{}", base + 10).as_str())
    );
    assert_eq!(status.session_count, 0);
    assert_eq!(fx.settings.value(PORT_KEY), Some((base + 10).to_string()));
    assert_eq!(revocations(&fx.audit), ["port_changed"]);
    assert!(
        TcpListener::bind((Ipv4Addr::LOCALHOST, base)).is_ok(),
        "old port released"
    );

    // Stopped: only saved.
    fx.service.set_enabled(false).await.unwrap();
    let status = fx.service.set_port(base as i64 + 5).await.unwrap();
    assert_eq!(
        (status.state, status.preferred_port),
        (LanServiceState::Stopped, base + 5)
    );
}

#[tokio::test]
async fn losing_the_bound_address_revokes_and_rebinds_but_a_new_preference_does_not() {
    let base = free_range();
    let fx = fx(Some(base), None);
    fx.service.on_app_start().await;
    pair(&fx.service).await;
    // Another preferred address appears; the bound one is still here (D101 item 3).
    let other: Ipv4Addr = "10.9.9.9".parse().unwrap();
    fx.network.candidates.lock().unwrap().push(other);
    *fx.network.route.lock().unwrap() = Some(other);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let status = fx.service.status().await;
    assert_eq!(
        status.address.as_deref(),
        Some(format!("127.0.0.1:{base}").as_str())
    );
    assert_eq!(status.session_count, 1);

    // The bound address disappears: revoke (network_changed), void the code, rebind.
    let _code = fx.service.issue_code().await.unwrap();
    fx.network.candidates.lock().unwrap().clear();
    let mut status = fx.service.status().await;
    for _ in 0..100 {
        status = fx.service.status().await;
        if status.state != LanServiceState::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Rebinding found no private address this time.
    assert_eq!(
        (status.state, status.failure_code),
        (LanServiceState::Failed, Some(1010003))
    );
    assert_eq!(status.session_count, 0);
    assert_eq!(revocations(&fx.audit), ["network_changed"]);
    // Back on the network: switching on again binds.
    fx.network
        .candidates
        .lock()
        .unwrap()
        .push(Ipv4Addr::LOCALHOST);
    let status = fx.service.set_enabled(true).await.unwrap();
    assert_eq!(status.state, LanServiceState::Running);
    fx.service.shutdown().await;
}

#[tokio::test]
async fn exit_revokes_everything_and_keeps_the_switch() {
    let base = free_range();
    let fx = fx(Some(base), None);
    fx.service.on_app_start().await;
    pair(&fx.service).await;
    fx.service.shutdown().await;
    let status = fx.service.status().await;
    assert_eq!(
        (status.enabled, status.state),
        (true, LanServiceState::Stopped)
    );
    assert_eq!(revocations(&fx.audit), ["app_exit"]);
    assert!(fx.settings.writes().is_empty());
    assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, base)).is_ok());
}

#[tokio::test]
async fn host_disconnect_commands_report_what_they_did() {
    let base = free_range();
    let fx = fx(Some(base), None);
    fx.service.on_app_start().await;
    let id = pair(&fx.service).await;
    assert_eq!(fx.service.list_devices().len(), 1);
    assert!(fx.service.disconnect_device(&id).await);
    assert!(!fx.service.disconnect_device(&id).await);
    pair(&fx.service).await;
    assert_eq!(fx.service.disconnect_all().await, 1);
    assert_eq!(fx.service.disconnect_all().await, 0);
    assert_eq!(
        revocations(&fx.audit),
        ["host_disconnect", "host_disconnect_all"]
    );
    fx.service.shutdown().await;
}
