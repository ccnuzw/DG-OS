//! In-memory pairing, sessions, admission and revocation (technical design
//! §2, §3, §4, §5; pseudocode B10-31～B10-64). Nothing here is persisted:
//! a restart invalidates every code and session (D088).
//!
//! Locks: `state` (a short `std` mutex, never held across `.await`) before a
//! session's `gate` (`tokio` `RwLock`). Requests hold the gate's read half
//! until their generation is registered; revocation removes the session under
//! `state`, releases it, then takes the write half and cancels the session's
//! window, so no generation can be registered after a revocation completes.

use crate::audit::AuditSink;
use crate::chat_port::{forwardable, ChatBackend, LanTargets};
use crate::code::{decode_code, encode_code, group, new_token, random_bytes, sha256};
use dgos_db::{ChatSession, LanAuditKind, LanAuditRecord};
use dgos_workbench::{
    ChatError, ChatEvent, ChatMessagePage, ChatSink, ChatTargetInput, ListChatMessagesInput,
    RegenerateChatInput, SendAck, SendChatInput,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, OwnedRwLockReadGuard, RwLock};

pub const CODE_TTL_SECS: i64 = 10 * 60;
pub const CODE_MAX_FAILURES: u32 = 10;
pub const SESSION_IDLE_SECS: i64 = 30 * 60;
pub const SESSION_MAX_SECS: i64 = 12 * 60 * 60;
pub const MAX_SESSIONS: usize = 8;
pub const MAX_CHAT_SESSIONS: usize = 100;
pub const PAIR_WINDOW_SECS: i64 = 60;
pub const PAIR_PER_IP: usize = 5;
pub const PAIR_GLOBAL: usize = 20;
const PAIR_IP_TABLE: usize = 256;
/// Per-connection send queue (D099 item 11).
pub const WS_QUEUE: usize = 256;

/// Unix seconds. Tests replace it.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        chrono::Utc::now().timestamp()
    }
}

/// The desktop user that may issue codes and that a remote session must still
/// match: the signed-in user, `default-user` while no account exists, `None`
/// while signed out, switching or deleting (technical design §5).
pub trait Issuer: Send + Sync + 'static {
    fn current_user(&self) -> Option<String>;
}

/// Fixed revocation reasons (technical design §2), recorded in the audit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevokeReason {
    HostDisconnect,
    HostDisconnectAll,
    SwitchOff,
    AppExit,
    ServiceFailed,
    NetworkChanged,
    PortChanged,
    IssuerEnded,
    IpChanged,
    IdleExpired,
    AbsoluteExpired,
    RePaired,
    Logout,
}

impl RevokeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HostDisconnect => "host_disconnect",
            Self::HostDisconnectAll => "host_disconnect_all",
            Self::SwitchOff => "switch_off",
            Self::AppExit => "app_exit",
            Self::ServiceFailed => "service_failed",
            Self::NetworkChanged => "network_changed",
            Self::PortChanged => "port_changed",
            Self::IssuerEnded => "issuer_ended",
            Self::IpChanged => "ip_changed",
            Self::IdleExpired => "idle_expired",
            Self::AbsoluteExpired => "absolute_expired",
            Self::RePaired => "re_paired",
            Self::Logout => "logout",
        }
    }

    /// WebSocket close code (D101): the service going away is 1001, every
    /// other revocation or expiry is 4401. A `session_ended` frame precedes both.
    pub fn close_code(self) -> u16 {
        match self {
            Self::SwitchOff
            | Self::AppExit
            | Self::ServiceFailed
            | Self::NetworkChanged
            | Self::PortChanged => 1001,
            _ => 4401,
        }
    }
}

pub const CLOSE_REPLACED: u16 = 4409;
pub const CLOSE_QUEUE_FULL: u16 = 4413;
pub const CLOSE_PROTOCOL: u16 = 1008;
pub const CLOSE_REVOKED: u16 = 4401;

/// Instruction to a WebSocket task, outside the bounded frame queue so it is
/// delivered even when the queue is full.
#[derive(Debug, Clone)]
pub struct WsClose {
    pub code: u16,
    /// The `session_ended` frame to send before closing.
    pub ended: Option<RevokeReason>,
}

struct WsSlot {
    id: u64,
    frames: mpsc::Sender<String>,
    control: mpsc::UnboundedSender<WsClose>,
}

/// What the WebSocket task drives.
pub struct WsAttachment {
    pub id: u64,
    pub frames: mpsc::Receiver<String>,
    pub control: mpsc::UnboundedReceiver<WsClose>,
}

pub struct RemoteSession {
    pub id: String,
    pub short_id: String,
    token_hash: [u8; 32],
    pub user: String,
    pub peer: Ipv4Addr,
    pub label: String,
    pub created_at: i64,
    last_active: AtomicU64,
    gate: Arc<RwLock<()>>,
    revoked: AtomicBool,
    chats: Mutex<HashSet<String>>,
    chat_reserved: AtomicUsize,
    tasks: Mutex<HashSet<String>>,
    ws: Mutex<Option<WsSlot>>,
    ws_next: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl RemoteSession {
    pub fn window(&self) -> String {
        format!("lan:{}", self.id)
    }

    fn last_active(&self) -> i64 {
        self.last_active.load(Ordering::SeqCst) as i64
    }

    fn expiry(&self, now: i64) -> Option<RevokeReason> {
        if now >= self.created_at + SESSION_MAX_SECS {
            Some(RevokeReason::AbsoluteExpired)
        } else if now >= self.last_active() + SESSION_IDLE_SECS {
            Some(RevokeReason::IdleExpired)
        } else {
            None
        }
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> LanSessionStatus {
        LanSessionStatus {
            short_id: self.short_id.clone(),
            device_label: self.label.clone(),
            created_at: self.created_at,
            expires_at: self.created_at + SESSION_MAX_SECS,
            idle_expires_at: self.last_active() + SESSION_IDLE_SECS,
        }
    }

    pub fn owns_chat(&self, chat_id: &str) -> bool {
        lock(&self.chats).contains(chat_id)
    }

    pub fn owns_task(&self, task_id: &str) -> bool {
        lock(&self.tasks).contains(task_id)
    }

    /// Install a new stream; the previous one closes with 4409 (D099 item 11).
    /// `None` once the session is revoked.
    pub fn attach_stream(&self) -> Option<WsAttachment> {
        let mut slot = lock(&self.ws);
        if self.is_revoked() {
            return None;
        }
        let (frames_tx, frames) = mpsc::channel(WS_QUEUE);
        let (control_tx, control) = mpsc::unbounded_channel();
        let id = self.ws_next.fetch_add(1, Ordering::SeqCst);
        if let Some(old) = slot.replace(WsSlot {
            id,
            frames: frames_tx,
            control: control_tx,
        }) {
            let _ = old.control.send(WsClose {
                code: CLOSE_REPLACED,
                ended: None,
            });
        }
        Some(WsAttachment {
            id,
            frames,
            control,
        })
    }

    /// The stream task ended; free the slot if it is still ours.
    pub fn detach_stream(&self, id: u64) {
        let mut slot = lock(&self.ws);
        if slot.as_ref().is_some_and(|s| s.id == id) {
            slot.take();
        }
    }

    /// Queue a frame. No connection: dropped (no buffering, no replay).
    /// Queue full: the connection closes with 4413; generation continues.
    fn push(&self, frame: String) {
        let mut slot = lock(&self.ws);
        let Some(current) = slot.as_ref() else {
            return;
        };
        match current.frames.try_send(frame) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                if let Some(full) = slot.take() {
                    let _ = full.control.send(WsClose {
                        code: CLOSE_QUEUE_FULL,
                        ended: None,
                    });
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                slot.take();
            }
        }
    }

    fn end_stream(&self, reason: RevokeReason) {
        if let Some(slot) = lock(&self.ws).take() {
            let _ = slot.control.send(WsClose {
                code: reason.close_code(),
                ended: Some(reason),
            });
        }
    }
}

/// Interface contract §14.5.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LanSessionStatus {
    pub short_id: String,
    pub device_label: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub idle_expires_at: i64,
}

/// Interface contract §14.8 `LanDevice`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LanDevice {
    pub session_id: String,
    pub short_id: String,
    pub label: String,
    pub peer_ip: String,
    pub created_at: i64,
    pub last_active_at: i64,
}

struct PendingCode {
    hash: [u8; 32],
    issuer: String,
    expires_at: i64,
    failures: u32,
}

#[derive(Default)]
struct State {
    accepting: bool,
    code: Option<PendingCode>,
    sessions: HashMap<[u8; 32], Arc<RemoteSession>>,
    pair_ip: HashMap<Ipv4Addr, VecDeque<i64>>,
    pair_global: VecDeque<i64>,
}

impl State {
    fn active_sessions(&self, now: i64) -> usize {
        self.sessions
            .values()
            .filter(|s| s.expiry(now).is_none())
            .count()
    }
}

/// Why admission failed (interface contract §14.2 steps 6～8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// 401 `1010006`, Cookie cleared.
    SessionInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    /// 429 `1010011`; `retry_after` only for `pair_ip`/`pair_global`.
    RateLimited {
        scope: &'static str,
        retry_after: Option<i64>,
    },
    /// 401 `1010004`.
    InvalidCode,
    /// 401 `1010005`.
    ExpiredCode,
}

pub struct Paired {
    pub status: LanSessionStatus,
    /// The Cookie value. Only ever placed in `Set-Cookie`.
    pub token: String,
}

/// The plaintext code, returned once to the host page.
pub struct IssuedCode {
    pub code: String,
    pub expires_at: i64,
}

/// A remote request admitted through §14.2: the session and its gate's read
/// half, held until the handler returns (generation registered).
pub struct SessionGuard {
    pub session: Arc<RemoteSession>,
    _gate: OwnedRwLockReadGuard<()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    Chat(ChatError),
    /// 429 `1010011`, `scope = chat_sessions`.
    ChatLimit,
}

impl From<ChatError> for RemoteError {
    fn from(error: ChatError) -> Self {
        Self::Chat(error)
    }
}

pub struct LanCore {
    state: Mutex<State>,
    clock: Arc<dyn Clock>,
    issuer: Arc<dyn Issuer>,
    audit: Arc<dyn AuditSink>,
    chat: Arc<dyn ChatBackend>,
}

fn canonical_uuid(raw: &str) -> Option<String> {
    uuid::Uuid::parse_str(raw).ok().map(|id| id.to_string())
}

/// Platform and browser family from the User-Agent; the raw header is not kept.
pub fn device_label(user_agent: Option<&str>) -> String {
    let ua = user_agent.unwrap_or_default();
    let platform = if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac OS X") || ua.contains("Macintosh") {
        "Mac"
    } else if ua.contains("CrOS") {
        "ChromeOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "未知设备"
    };
    let browser = if ua.contains("MicroMessenger") {
        "微信"
    } else if ua.contains("Edg/") || ua.contains("EdgA/") || ua.contains("EdgiOS/") {
        "Edge"
    } else if ua.contains("Firefox/") || ua.contains("FxiOS/") {
        "Firefox"
    } else if ua.contains("Chrome/") || ua.contains("CriOS/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "浏览器"
    };
    format!("{platform} · {browser}")
}

impl LanCore {
    pub fn new(
        clock: Arc<dyn Clock>,
        issuer: Arc<dyn Issuer>,
        audit: Arc<dyn AuditSink>,
        chat: Arc<dyn ChatBackend>,
    ) -> Self {
        Self {
            state: Mutex::new(State {
                accepting: true,
                ..State::default()
            }),
            clock,
            issuer,
            audit,
            chat,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    pub fn chat(&self) -> &dyn ChatBackend {
        self.chat.as_ref()
    }

    /// The service opens pairing when it listens and closes it before it stops.
    pub fn set_accepting(&self, accepting: bool) {
        self.state().accepting = accepting;
    }

    #[allow(clippy::too_many_arguments)]
    fn audit(
        &self,
        event: LanAuditKind,
        reason: Option<&'static str>,
        peer: Option<Ipv4Addr>,
        session: Option<&RemoteSession>,
        user: Option<String>,
        error_code: Option<u32>,
    ) {
        self.audit.record(LanAuditRecord {
            at: self.now(),
            event,
            reason,
            peer_ip: peer,
            session_short_id: session.map(|s| s.short_id.clone()),
            user_id: user.or_else(|| session.map(|s| s.user.clone())),
            error_code,
        });
    }

    /// `request_rejected` for steps 1～3 (peer, Host, Origin).
    pub fn reject_request(&self, reason: &'static str, peer: Option<Ipv4Addr>) {
        self.audit(
            LanAuditKind::RequestRejected,
            Some(reason),
            peer,
            None,
            None,
            Some(1010007),
        );
    }

    // ---------------------------------------------------------------------------------------
    // Access code (B10-45, B10-46)

    /// A new code replacing any unused one. `None` without an issuable user
    /// or while the service is not accepting.
    pub fn issue_code(&self) -> Option<IssuedCode> {
        let issuer = self.issuer.current_user()?;
        let raw: [u8; 16] = random_bytes();
        let expires_at = self.now() + CODE_TTL_SECS;
        let replaced = {
            let mut state = self.state();
            if !state.accepting {
                return None;
            }
            state.code.replace(PendingCode {
                hash: sha256(&raw),
                issuer: issuer.clone(),
                expires_at,
                failures: 0,
            })
        };
        if let Some(old) = replaced {
            self.code_voided("replaced", old.issuer);
        }
        Some(IssuedCode {
            code: group(&encode_code(&raw)),
            expires_at,
        })
    }

    fn code_voided(&self, reason: &'static str, issuer: String) {
        self.audit(
            LanAuditKind::CodeVoided,
            Some(reason),
            None,
            None,
            Some(issuer),
            None,
        );
    }

    /// Void the unused code, if any.
    pub fn void_code(&self, reason: &'static str) {
        let old = self.state().code.take();
        if let Some(old) = old {
            self.code_voided(reason, old.issuer);
        }
    }

    pub fn code_expires_at(&self) -> Option<i64> {
        let now = self.now();
        self.state()
            .code
            .as_ref()
            .map(|code| code.expires_at)
            .filter(|at| *at > now)
    }

    // ---------------------------------------------------------------------------------------
    // Pairing (B10-40～B10-44)

    pub async fn pair(
        &self,
        peer: Ipv4Addr,
        cookie: Option<[u8; 32]>,
        code: &str,
        user_agent: Option<&str>,
    ) -> Result<Paired, PairError> {
        let now = self.now();
        let mut stale = None;
        let result = {
            let mut state = self.state();
            if !state.accepting {
                return Err(PairError::InvalidCode);
            }
            // Sliding 60-second windows; rejected requests are not counted.
            let horizon = now - PAIR_WINDOW_SECS;
            while state.pair_global.front().is_some_and(|t| *t <= horizon) {
                state.pair_global.pop_front();
            }
            let ip_window = state.pair_ip.entry(peer).or_default();
            while ip_window.front().is_some_and(|t| *t <= horizon) {
                ip_window.pop_front();
            }
            let limited = if ip_window.len() >= PAIR_PER_IP {
                Some(("pair_ip", ip_window[0]))
            } else if state.pair_global.len() >= PAIR_GLOBAL {
                Some(("pair_global", state.pair_global[0]))
            } else {
                None
            };
            if let Some((scope, oldest)) = limited {
                drop(state);
                self.audit(
                    LanAuditKind::PairRateLimited,
                    Some(scope),
                    Some(peer),
                    None,
                    None,
                    Some(1010011),
                );
                return Err(PairError::RateLimited {
                    scope,
                    retry_after: Some((oldest + PAIR_WINDOW_SECS - now).max(1)),
                });
            }
            state.pair_ip.entry(peer).or_default().push_back(now);
            state.pair_global.push_back(now);
            if state.pair_ip.len() > PAIR_IP_TABLE {
                let quiet = state
                    .pair_ip
                    .iter()
                    .filter(|(ip, _)| **ip != peer)
                    .min_by_key(|(_, times)| times.back().copied().unwrap_or(i64::MIN))
                    .map(|(ip, _)| *ip);
                if let Some(quiet) = quiet {
                    state.pair_ip.remove(&quiet);
                }
            }

            // The device's previous session, if its Cookie is still valid.
            let mut old = cookie
                .and_then(|hash| state.sessions.get(&hash).cloned())
                .filter(|s| s.expiry(now).is_none() && !s.is_revoked());
            if let Some(previous) = old.take() {
                if previous.peer == peer {
                    old = Some(previous);
                } else {
                    // D105: another IP. Revoke first, then pair normally.
                    state.sessions.remove(&previous.token_hash);
                    stale = Some(previous);
                }
            }
            self.pair_locked(&mut state, now, peer, old, code, user_agent)
        };
        if let Some(stale) = stale {
            self.finish_revoke(&stale, RevokeReason::IpChanged).await;
        }
        let (paired, replaced) = result?;
        if let Some(replaced) = replaced {
            self.finish_revoke(&replaced, RevokeReason::RePaired).await;
        }
        Ok(paired)
    }

    #[allow(clippy::type_complexity)]
    fn pair_locked(
        &self,
        state: &mut State,
        now: i64,
        peer: Ipv4Addr,
        old: Option<Arc<RemoteSession>>,
        code: &str,
        user_agent: Option<&str>,
    ) -> Result<(Paired, Option<Arc<RemoteSession>>), PairError> {
        // The same device re-pairing does not count against the limit (D101).
        if old.is_none() && state.active_sessions(now) >= MAX_SESSIONS {
            self.audit(
                LanAuditKind::PairRejected,
                Some("session_limit"),
                Some(peer),
                None,
                None,
                Some(1010011),
            );
            return Err(PairError::RateLimited {
                scope: "session_limit",
                retry_after: None,
            });
        }
        let presented = decode_code(code).map(|raw| sha256(&raw));
        let matches = match (&state.code, presented) {
            (Some(pending), Some(hash)) => bool::from(pending.hash.ct_eq(&hash)),
            _ => false,
        };
        if !matches {
            let voided = match state.code.as_mut() {
                Some(pending) => {
                    pending.failures += 1;
                    pending.failures >= CODE_MAX_FAILURES
                }
                None => false,
            };
            if voided {
                if let Some(pending) = state.code.take() {
                    self.code_voided("too_many_failures", pending.issuer);
                }
            }
            self.audit(
                LanAuditKind::PairRejected,
                Some("invalid_code"),
                Some(peer),
                None,
                None,
                Some(1010004),
            );
            return Err(PairError::InvalidCode);
        }
        let pending = state.code.take().expect("matched a pending code");
        if pending.expires_at <= now {
            self.code_voided("expired", pending.issuer);
            self.audit(
                LanAuditKind::PairRejected,
                Some("expired_code"),
                Some(peer),
                None,
                None,
                Some(1010005),
            );
            return Err(PairError::ExpiredCode);
        }
        // Replace the device's own session inside the lock: the count stays the same.
        if let Some(old) = &old {
            state.sessions.remove(&old.token_hash);
        }
        let (token, token_hash) = new_token();
        let id = uuid::Uuid::new_v4().to_string();
        let session = Arc::new(RemoteSession {
            short_id: id[..8].to_owned(),
            id,
            token_hash,
            user: pending.issuer,
            peer,
            label: device_label(user_agent),
            created_at: now,
            last_active: AtomicU64::new(now.max(0) as u64),
            gate: Arc::new(RwLock::new(())),
            revoked: AtomicBool::new(false),
            chats: Mutex::new(HashSet::new()),
            chat_reserved: AtomicUsize::new(0),
            tasks: Mutex::new(HashSet::new()),
            ws: Mutex::new(None),
            ws_next: AtomicU64::new(1),
        });
        state.sessions.insert(token_hash, session.clone());
        self.audit(
            LanAuditKind::PairSucceeded,
            None,
            Some(peer),
            Some(&session),
            None,
            None,
        );
        Ok((
            Paired {
                status: session.status(),
                token,
            },
            old,
        ))
    }

    // ---------------------------------------------------------------------------------------
    // Admission (§14.2 steps 6～8, B10-36～B10-39)

    pub async fn admit(
        &self,
        peer: Ipv4Addr,
        cookie: Option<[u8; 32]>,
    ) -> Result<SessionGuard, Reject> {
        let now = self.now();
        let invalid = || {
            if cookie.is_some() {
                self.audit(
                    LanAuditKind::RequestRejected,
                    Some("session_invalid"),
                    Some(peer),
                    None,
                    None,
                    Some(1010006),
                );
            }
            Reject::SessionInvalid
        };
        let session = cookie.and_then(|hash| self.state().sessions.get(&hash).cloned());
        let Some(session) = session else {
            return Err(invalid());
        };
        if let Some(reason) = session.expiry(now) {
            self.revoke_session(&session, reason).await;
            return Err(invalid());
        }
        let gate = session.gate.clone().read_owned().await;
        if session.is_revoked() {
            return Err(invalid());
        }
        if session.peer != peer {
            drop(gate);
            self.revoke_session(&session, RevokeReason::IpChanged).await;
            return Err(Reject::SessionInvalid);
        }
        if self.issuer.current_user().as_deref() != Some(session.user.as_str()) {
            drop(gate);
            self.revoke_session(&session, RevokeReason::IssuerEnded)
                .await;
            return Err(Reject::SessionInvalid);
        }
        session
            .last_active
            .fetch_max(now.max(0) as u64, Ordering::SeqCst);
        Ok(SessionGuard {
            session,
            _gate: gate,
        })
    }

    // ---------------------------------------------------------------------------------------
    // Revocation (B10-60～B10-63)

    /// Remove `session` if it is still registered, then finish revoking it.
    async fn revoke_session(&self, session: &Arc<RemoteSession>, reason: RevokeReason) -> bool {
        let removed = {
            let mut state = self.state();
            match state.sessions.get(&session.token_hash) {
                Some(current) if Arc::ptr_eq(current, session) => {
                    state.sessions.remove(&session.token_hash);
                    true
                }
                _ => false,
            }
        };
        if removed {
            self.finish_revoke(session, reason).await;
        }
        removed
    }

    /// Wait for in-flight requests (write half of the gate), cancel the
    /// session's generations (kept as interrupted, FR-006), end its stream, audit.
    async fn finish_revoke(&self, session: &Arc<RemoteSession>, reason: RevokeReason) {
        let _gate = session.gate.write().await;
        if session.revoked.swap(true, Ordering::SeqCst) {
            return;
        }
        self.chat.cancel_window(&session.window());
        session.end_stream(reason);
        self.audit(
            LanAuditKind::SessionRevoked,
            Some(reason.as_str()),
            Some(session.peer),
            Some(session),
            None,
            None,
        );
    }

    /// Every session; expired ones keep their own reason. Returns how many
    /// unexpired sessions `reason` revoked.
    pub async fn revoke_all(&self, reason: RevokeReason) -> usize {
        let now = self.now();
        let drained: Vec<_> = self.state().sessions.drain().map(|(_, s)| s).collect();
        let mut count = 0;
        for session in drained {
            match session.expiry(now) {
                Some(expired) => self.finish_revoke(&session, expired).await,
                None => {
                    count += 1;
                    self.finish_revoke(&session, reason).await;
                }
            }
        }
        count
    }

    /// FR-008 session end, step 4 (`RemoteSessionRevoker::revoke_issued_by`).
    pub async fn revoke_issued_by(&self, user: &str) {
        let (code, sessions) = {
            let mut state = self.state();
            let code = match &state.code {
                Some(pending) if pending.issuer == user => state.code.take(),
                _ => None,
            };
            let ids: Vec<[u8; 32]> = state
                .sessions
                .iter()
                .filter(|(_, s)| s.user == user)
                .map(|(hash, _)| *hash)
                .collect();
            let sessions: Vec<_> = ids
                .iter()
                .filter_map(|hash| state.sessions.remove(hash))
                .collect();
            (code, sessions)
        };
        if let Some(code) = code {
            self.code_voided("issuer_ended", code.issuer);
        }
        for session in sessions {
            self.finish_revoke(&session, RevokeReason::IssuerEnded)
                .await;
        }
    }

    /// Host "断开" (B10-63). `false` for an unknown id.
    pub async fn disconnect(&self, session_id: &str) -> bool {
        let session = self
            .state()
            .sessions
            .values()
            .find(|s| s.id == session_id)
            .cloned();
        match session {
            Some(session) => {
                self.revoke_session(&session, RevokeReason::HostDisconnect)
                    .await
            }
            None => false,
        }
    }

    /// Periodic expiry (B10-61): sessions past idle or absolute limits, and
    /// the expired code.
    pub async fn sweep(&self) {
        let now = self.now();
        let (expired, code) = {
            let mut state = self.state();
            let expired: Vec<_> = state
                .sessions
                .values()
                .filter_map(|s| s.expiry(now).map(|reason| (s.clone(), reason)))
                .collect();
            let code = match &state.code {
                Some(pending) if pending.expires_at <= now => state.code.take(),
                _ => None,
            };
            (expired, code)
        };
        if let Some(code) = code {
            self.code_voided("expired", code.issuer);
        }
        for (session, reason) in expired {
            self.revoke_session(&session, reason).await;
        }
    }

    pub fn list_devices(&self) -> Vec<LanDevice> {
        let now = self.now();
        let mut devices: Vec<_> = self
            .state()
            .sessions
            .values()
            .filter(|s| s.expiry(now).is_none())
            .map(|s| LanDevice {
                session_id: s.id.clone(),
                short_id: s.short_id.clone(),
                label: s.label.clone(),
                peer_ip: s.peer.to_string(),
                created_at: s.created_at,
                last_active_at: s.last_active(),
            })
            .collect();
        devices.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.short_id.cmp(&b.short_id))
        });
        devices
    }

    pub fn session_count(&self) -> usize {
        let now = self.now();
        self.state().active_sessions(now)
    }

    /// `POST /logout`: revoke the Cookie's session if it exists. An expired
    /// session ends with its expiry reason, a different source address as
    /// `ip_changed`; otherwise `logout`.
    pub async fn logout(&self, peer: Ipv4Addr, cookie: Option<[u8; 32]>) {
        let session = cookie.and_then(|hash| self.state().sessions.get(&hash).cloned());
        if let Some(session) = session {
            let reason = session
                .expiry(self.now())
                .unwrap_or(if session.peer == peer {
                    RevokeReason::Logout
                } else {
                    RevokeReason::IpChanged
                });
            self.revoke_session(&session, reason).await;
        }
    }

    // ---------------------------------------------------------------------------------------
    // Remote chat (B10-47～B10-54); the guard is held by the caller.

    pub async fn targets(&self, guard: &SessionGuard) -> Result<LanTargets, RemoteError> {
        Ok(self.chat.targets(&guard.session.user).await?)
    }

    pub async fn create_chat(
        &self,
        guard: &SessionGuard,
        target: ChatTargetInput,
    ) -> Result<ChatSession, RemoteError> {
        // The target is fixed at creation (D099 item 10); skills are optional.
        if target.provider_id.is_none() || target.model_id.is_none() {
            return Err(ChatError::ProviderNotSelected.into());
        }
        if !target.skill_ids.is_empty() && target.no_skill {
            return Err(ChatError::InvalidMessageContent.into());
        }
        let session = &guard.session;
        // Reserve a slot before the await so concurrent creates cannot pass 100.
        let mut reserved = session.chat_reserved.load(Ordering::SeqCst);
        loop {
            if reserved >= MAX_CHAT_SESSIONS {
                return Err(RemoteError::ChatLimit);
            }
            match session.chat_reserved.compare_exchange(
                reserved,
                reserved + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(current) => reserved = current,
            }
        }
        match self.chat.create_session(&session.user, target).await {
            Ok(chat) => {
                lock(&session.chats).insert(chat.id.clone());
                Ok(chat)
            }
            Err(error) => {
                session.chat_reserved.fetch_sub(1, Ordering::SeqCst);
                Err(error.into())
            }
        }
    }

    /// Only this pairing's chats, `updated_at` descending, not paged.
    pub async fn list_chats(&self, guard: &SessionGuard) -> Result<Vec<ChatSession>, RemoteError> {
        let ids: Vec<String> = lock(&guard.session.chats).iter().cloned().collect();
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            match self.chat.get_session(&guard.session.user, &id).await {
                Ok(chat) => items.push(chat),
                // Deleted on the desktop meanwhile.
                Err(ChatError::SessionNotFound) => {}
                Err(error) => return Err(error.into()),
            }
        }
        items.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then(b.created_at.cmp(&a.created_at))
        });
        Ok(items)
    }

    pub async fn list_messages(
        &self,
        guard: &SessionGuard,
        chat_id: &str,
        before_seq: Option<i64>,
        limit: Option<u32>,
    ) -> Result<ChatMessagePage, RemoteError> {
        let chat_id = canonical_uuid(chat_id).ok_or(ChatError::SessionNotFound)?;
        if !guard.session.owns_chat(&chat_id) {
            return Err(ChatError::SessionNotFound.into());
        }
        Ok(self
            .chat
            .list_messages(
                &guard.session.user,
                ListChatMessagesInput {
                    session_id: chat_id,
                    before_seq,
                    limit,
                },
            )
            .await?)
    }

    pub async fn send(
        &self,
        guard: &SessionGuard,
        chat_id: &str,
        content: String,
        client_request_id: String,
    ) -> Result<SendAck, RemoteError> {
        let chat_id = canonical_uuid(chat_id).ok_or(ChatError::SessionNotFound)?;
        if !guard.session.owns_chat(&chat_id) {
            return Err(ChatError::SessionNotFound.into());
        }
        let sent = self
            .chat
            .send(
                &guard.session.user,
                &guard.session.window(),
                SendChatInput {
                    session_id: chat_id,
                    content,
                    client_request_id,
                    accept_destination: None,
                    tools: Vec::new(),
                    asset_ids: Vec::new(),
                },
            )
            .await?;
        Ok(self.start(guard, sent))
    }

    pub async fn regenerate(
        &self,
        guard: &SessionGuard,
        message_id: &str,
        client_request_id: String,
    ) -> Result<SendAck, RemoteError> {
        let user = &guard.session.user;
        // A repeated request finds its session through the task id: the reply
        // it replaced is gone once the first attempt succeeded.
        let chat = match canonical_uuid(&client_request_id) {
            Some(task) => self.chat.session_of_task(user, &task).await?,
            None => None,
        };
        let chat = match chat {
            Some(chat) => Some(chat),
            None => match canonical_uuid(message_id) {
                Some(message) => self.chat.session_of_message(user, &message).await?,
                None => None,
            },
        };
        if !chat.is_some_and(|chat| guard.session.owns_chat(&chat)) {
            return Err(ChatError::MessageNotRegenerable.into());
        }
        let sent = self
            .chat
            .regenerate(
                user,
                &guard.session.window(),
                RegenerateChatInput {
                    message_id: message_id.to_owned(),
                    client_request_id,
                    accept_destination: None,
                },
            )
            .await?;
        Ok(self.start(guard, sent))
    }

    /// Register a new generation with this session and drive it. A replay
    /// starts nothing and adds no task.
    fn start(&self, guard: &SessionGuard, sent: dgos_workbench::Sent) -> SendAck {
        let dgos_workbench::Sent { ack, job } = sent;
        if let Some(job) = job {
            let task_id = job.task_id().to_owned();
            lock(&guard.session.tasks).insert(task_id.clone());
            let weak = Arc::downgrade(&guard.session);
            let done_weak = weak.clone();
            self.chat.drive(
                job,
                Arc::new(SessionSink(weak)),
                Box::new(move || {
                    if let Some(session) = done_weak.upgrade() {
                        lock(&session.tasks).remove(&task_id);
                    }
                }),
            );
        }
        ack
    }

    /// Only tasks this session started and that are still running.
    pub fn cancel(&self, guard: &SessionGuard, task_id: &str) -> bool {
        match canonical_uuid(task_id) {
            Some(task) if guard.session.owns_task(&task) => {
                self.chat.cancel(&guard.session.user, &task)
            }
            _ => false,
        }
    }
}

/// Forwards the session's generation events to its stream. Always `true`:
/// a lost connection never cancels a generation (D099 item 11).
struct SessionSink(Weak<RemoteSession>);

impl ChatSink for SessionSink {
    fn send(&self, event: ChatEvent) -> bool {
        if !forwardable(&event) {
            return true;
        }
        if let Some(session) = self.0.upgrade() {
            if !session.is_revoked() {
                if let Ok(frame) =
                    serde_json::to_string(&serde_json::json!({"type": "chat", "event": event}))
                {
                    session.push(frame);
                }
            }
        }
        true
    }
}

/// The `session_ended` frame text.
pub fn session_ended_frame(reason: RevokeReason) -> String {
    serde_json::json!({"type": "session_ended", "reason": reason.as_str()}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::MemoryAudit;
    use async_trait::async_trait;
    use dgos_workbench::{GenerationJob, Sent};

    struct NoChat;

    #[async_trait]
    impl ChatBackend for NoChat {
        async fn targets(&self, _: &str) -> Result<LanTargets, ChatError> {
            Err(ChatError::Internal)
        }
        async fn create_session(
            &self,
            _: &str,
            _: ChatTargetInput,
        ) -> Result<ChatSession, ChatError> {
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

    async fn paired() -> (LanCore, SessionGuard) {
        let core = LanCore::new(
            Arc::new(SystemClock),
            Arc::new(Alice),
            Arc::new(MemoryAudit::default()),
            Arc::new(NoChat),
        );
        let peer: Ipv4Addr = "192.168.1.10".parse().unwrap();
        let code = core.issue_code().unwrap().code;
        let paired = core.pair(peer, None, &code, None).await.ok().unwrap();
        let hash = crate::code::token_hash(&paired.token);
        let guard = core.admit(peer, hash).await.ok().unwrap();
        (core, guard)
    }

    /// D099 item 11: a full 256-frame queue closes the stream with 4413; the
    /// next frames are dropped (no connection) and nothing else is affected.
    #[tokio::test]
    async fn a_full_send_queue_closes_the_stream_with_4413() {
        let (_core, guard) = paired().await;
        let session = guard.session.clone();
        let mut stream = session.attach_stream().unwrap();
        for n in 0..WS_QUEUE {
            session.push(format!("frame {n}"));
        }
        assert!(stream.control.try_recv().is_err(), "256 frames fit");
        session.push("one too many".into());
        let close = stream.control.try_recv().unwrap();
        assert_eq!((close.code, close.ended), (CLOSE_QUEUE_FULL, None));
        // The slot is free: later frames are dropped, not queued.
        session.push("dropped".into());
        let mut received = 0;
        while stream.frames.try_recv().is_ok() {
            received += 1;
        }
        assert_eq!(received, WS_QUEUE);
        assert!(!session.is_revoked());
    }

    #[tokio::test]
    async fn a_second_stream_replaces_the_first_and_revocation_ends_the_current_one() {
        let (core, guard) = paired().await;
        let session = guard.session.clone();
        let mut first = session.attach_stream().unwrap();
        let mut second = session.attach_stream().unwrap();
        let close = first.control.try_recv().unwrap();
        assert_eq!(close.code, CLOSE_REPLACED);
        // The old task detaching must not free the new connection's slot.
        session.detach_stream(first.id);
        session.push("to second".into());
        assert_eq!(second.frames.try_recv().unwrap(), "to second");
        drop(guard);
        assert!(core.disconnect(&session.id).await);
        let close = second.control.try_recv().unwrap();
        assert_eq!(
            (close.code, close.ended),
            (CLOSE_REVOKED, Some(RevokeReason::HostDisconnect))
        );
        assert!(
            session.attach_stream().is_none(),
            "no stream after revocation"
        );
    }

    /// Revocation waits for an in-flight request: while a guard is held the
    /// write half cannot be taken, so nothing registers after revocation.
    #[tokio::test]
    async fn revocation_waits_for_the_admitted_request() {
        let (core, guard) = paired().await;
        let core = Arc::new(core);
        let id = guard.session.id.clone();
        let revoking = tokio::spawn({
            let core = core.clone();
            async move { core.disconnect(&id).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!revoking.is_finished());
        assert!(!guard.session.is_revoked());
        drop(guard);
        assert!(revoking.await.unwrap());
    }

    #[test]
    fn close_codes_follow_d101() {
        for reason in [
            RevokeReason::SwitchOff,
            RevokeReason::AppExit,
            RevokeReason::ServiceFailed,
            RevokeReason::NetworkChanged,
            RevokeReason::PortChanged,
        ] {
            assert_eq!(reason.close_code(), 1001, "{reason:?}");
        }
        for reason in [
            RevokeReason::HostDisconnect,
            RevokeReason::HostDisconnectAll,
            RevokeReason::IssuerEnded,
            RevokeReason::IpChanged,
            RevokeReason::IdleExpired,
            RevokeReason::AbsoluteExpired,
            RevokeReason::RePaired,
            RevokeReason::Logout,
        ] {
            assert_eq!(reason.close_code(), 4401, "{reason:?}");
        }
    }

    #[test]
    fn device_labels_never_keep_the_raw_user_agent() {
        let ua = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/130.0 Mobile Safari/537.36";
        assert_eq!(device_label(Some(ua)), "Android · Chrome");
        assert_eq!(device_label(None), "未知设备 · 浏览器");
        assert_eq!(
            device_label(Some(
                "Mozilla/5.0 (Windows NT 10.0) Chrome/130 Safari/537 Edg/130"
            )),
            "Windows · Edge"
        );
    }
}
