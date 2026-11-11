//! Multi-turn streaming chat (V1-FR-006, D072-D077).
//!
//! `ChatManager` owns the per-user registry of active generations and the chat repository. The
//! caller supplies the host-derived user and window label; nothing here accepts an identity from
//! the renderer. Provider and Skills managers are borrowed per call so the Tauri layer can keep
//! its existing managed state.
//!
//! Lifecycle of one generation: `send`/`regenerate` validate, assemble the context, register the
//! cancellation entry (check-then-insert, never overwrite), run the start transaction and return
//! a [`Sent`]; the caller then awaits [`ChatManager::run_generation`], which streams to the sink
//! and ends in exactly one terminal transaction.

use crate::tools::ToolProposal;
use crate::{skill_guidance, HOST_POLICY, OUTPUT_TOKENS};
use dgos_db::{
    sea_orm::DatabaseConnection, BeginGeneration, ChatMessage, ChatRepository, ChatSession,
    ChatStoreError, ChatTarget, FinalizeGeneration, Finalized, History, PinnedSkill, SearchHit,
};
use dgos_db::{ActivityPermit, MaintenanceGate};
use dgos_mcp::McpManager;
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use dgos_types::{
    BuiltinCaller, ChatMessage as WireMessage, ChatRequest, DgosError, ExecutionProvenance, Model,
    Provider,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

mod tool_turn;
mod contract;
mod continuous;
mod lifecycle;
mod foundation;
mod web_runtime;
mod stream;
pub use stream::{ChatLiveSnapshot, ChatRunSnapshot};
use stream::{OrderedChatSink, StreamState};
use foundation::effective_skill_mode;
pub use contract::*;
use lifecycle::{ChatStepGuard, CheckpointWriter, TerminalEvidence};
pub use tool_turn::{AdvanceChatToolTurnInput, ToolActionJob};
use tool_turn::{ToolPlan, ToolTurn};

const MAX_MESSAGE_BYTES: usize = 16 * 1024;
/// D018/D075: total request content ceiling.
const MAX_CONTEXT_BYTES: usize = 24 * 1024;
/// D074 (provisional): host limits per generation.
const MAX_REPLY_BYTES: usize = 64 * 1024;
const MAX_DURATION: Duration = Duration::from_secs(10 * 60);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

pub type ChatToolProposal = ToolProposal;

// ---------------------------------------------------------------------------------------------
// Errors

/// Chat failures with their registered numeric code (error codes §3.6). Provider failures keep
/// their own 1001xxx code; no upstream text is ever carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatError {
    SessionNotFound,
    MessageNotFound,
    SessionCreationFailed,
    SessionDeletionFailed,
    InvalidMessageContent,
    ContextTooLong,
    NoProviderConfigured,
    GenerationInProgress,
    RequestIdConflict,
    DestinationChanged { fingerprint: String, host: String },
    SkillUnavailable,
    MessageNotRegenerable,
    ProviderNotSelected,
    InvalidSearchQuery,
    ToolTurnInvalid,
    Web(String),
    AssetInvalid,
    AssetLimitExceeded,
    AssetNotFound,
    AssetReferenced,
    Provider(u32),
    Internal,
}

impl ChatError {
    pub fn code(&self) -> u32 {
        match self {
            Self::SessionNotFound => 1006001,
            Self::MessageNotFound => 1006002,
            Self::SessionCreationFailed => 1006005,
            Self::SessionDeletionFailed => 1006006,
            Self::InvalidMessageContent => 1006007,
            Self::ContextTooLong => 1006008,
            Self::NoProviderConfigured => 1006009,
            Self::GenerationInProgress => 1006010,
            Self::RequestIdConflict => 1006011,
            Self::DestinationChanged { .. } => 1006012,
            Self::SkillUnavailable => 1006013,
            Self::MessageNotRegenerable => 1006014,
            Self::ProviderNotSelected => 1006015,
            Self::InvalidSearchQuery => 1006016,
            Self::ToolTurnInvalid => 1006017,
            Self::Web(_) => 1006022,
            Self::AssetInvalid => 1006018,
            Self::AssetLimitExceeded => 1006019,
            Self::AssetNotFound => 1006020,
            Self::AssetReferenced => 1006021,
            Self::Provider(code) => *code,
            Self::Internal => 1000000,
        }
    }

    /// Fixed English summary; the renderer localizes by code.
    pub fn message(&self) -> &'static str {
        match self {
            Self::SessionNotFound => "session_not_found",
            Self::MessageNotFound => "message_not_found",
            Self::SessionCreationFailed => "session_creation_failed",
            Self::SessionDeletionFailed => "session_deletion_failed",
            Self::InvalidMessageContent => "invalid_message_content",
            Self::ContextTooLong => "context_too_long",
            Self::NoProviderConfigured => "no_provider_configured",
            Self::GenerationInProgress => "generation_in_progress",
            Self::RequestIdConflict => "request_id_conflict",
            Self::DestinationChanged { .. } => "destination_changed",
            Self::SkillUnavailable => "skill_unavailable",
            Self::MessageNotRegenerable => "message_not_regenerable",
            Self::ProviderNotSelected => "provider_not_selected",
            Self::InvalidSearchQuery => "invalid_search_query",
            Self::ToolTurnInvalid => "tool_turn_invalid",
            Self::Web(_) => "web_unavailable",
            Self::AssetInvalid => "asset_invalid",
            Self::AssetLimitExceeded => "asset_limit_exceeded",
            Self::AssetNotFound => "asset_not_found",
            Self::AssetReferenced => "asset_referenced",
            Self::Provider(_) => "provider_error",
            Self::Internal => "internal_error",
        }
    }

    /// Only safe host-owned destination metadata or fixed web failure categories are exposed.
    pub fn detail(&self) -> Option<Value> {
        match self {
            Self::DestinationChanged { fingerprint, host } => {
                Some(json!({"fingerprint": fingerprint, "host": host}))
            }
            Self::Web(reason) => Some(json!({"reason": reason})),
            _ => None,
        }
    }
}

impl From<DgosError> for ChatError {
    fn from(error: DgosError) -> Self {
        match error.error_code() {
            1000000 => Self::Internal,
            code => Self::Provider(code),
        }
    }
}

impl From<ChatStoreError> for ChatError {
    fn from(error: ChatStoreError) -> Self {
        match error {
            ChatStoreError::SessionNotFound => Self::SessionNotFound,
            ChatStoreError::MessageNotFound => Self::MessageNotFound,
            ChatStoreError::GenerationInProgress => Self::GenerationInProgress,
            ChatStoreError::RequestConflict => Self::RequestIdConflict,
            ChatStoreError::NotRegenerable => Self::MessageNotRegenerable,
            ChatStoreError::InvalidCursor => Self::InvalidSearchQuery,
            ChatStoreError::InvalidTitle => Self::InvalidMessageContent,
            ChatStoreError::AssetNotFound => Self::InvalidMessageContent,
            ChatStoreError::Db(_) => Self::Internal,
        }
    }
}

type Result<T, E = ChatError> = std::result::Result<T, E>;

// ---------------------------------------------------------------------------------------------
// IPC inputs (renderer-facing; unknown fields, including `user_id`, are rejected)

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatTargetInput {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default)]
    pub generation: dgos_types::GenerationParameters,
    /// Compatibility metadata for older clients; empty skills need no confirmation.
    #[serde(default)]
    pub no_skill: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateChatSessionInput {
    #[serde(default)]
    pub target: Option<ChatTargetInput>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListChatSessionsInput {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameChatSessionInput {
    pub session_id: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateChatSessionTargetInput {
    pub session_id: String,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default)]
    pub generation: dgos_types::GenerationParameters,
    #[serde(default)]
    pub no_skill: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteChatSessionInput {
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchChatSessionsInput {
    pub query: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListChatMessagesInput {
    pub session_id: String,
    #[serde(default)]
    pub before_seq: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendChatInput {
    pub session_id: String,
    pub content: String,
    pub client_request_id: String,
    #[serde(default)]
    pub accept_destination: Option<String>,
    /// One to eight MCP tools chosen for this send only (D022/D075). The shared streaming
    /// executor parks for exact approvals; nothing is authorized for the next send.
    #[serde(default)]
    pub tools: Vec<dgos_types::AiToolSelection>,
    /// Controlled asset ids attached to this message (V2 D133 §2). Validated against the account's
    /// stored assets and linked in the same transaction as the message.
    #[serde(default)]
    pub asset_ids: Vec<String>,
    #[serde(default)]
    pub skill_mode: ChatSkillMode,
    #[serde(default)]
    pub authorized_skill_refs: Vec<dgos_types::AiSkillRef>,
    #[serde(default)]
    pub foundation: dgos_types::AiFoundationOptions,
    #[serde(default)]
    pub web_access: crate::web::WebAccess,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegenerateChatInput {
    pub message_id: String,
    pub client_request_id: String,
    #[serde(default)]
    pub accept_destination: Option<String>,
    #[serde(default)]
    pub foundation: dgos_types::AiFoundationOptions,
    #[serde(default)]
    pub skill_mode: ChatSkillMode,
    #[serde(default)]
    pub authorized_skill_refs: Vec<dgos_types::AiSkillRef>,
    #[serde(default)]
    pub web_access: crate::web::WebAccess,
}

// ---------------------------------------------------------------------------------------------
// IPC outputs

#[derive(Clone, Debug, Serialize)]
pub struct ChatSessionPage {
    pub items: Vec<ChatSession>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatMessagePage {
    pub items: Vec<ChatMessage>,
    pub next_before_seq: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatSearchPage {
    pub items: Vec<SearchHit>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SendAck {
    pub task_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub dropped_turns: u32,
    pub replayed: bool,
}

/// What a stream sends to the renderer's Channel. Exactly one of `Done`/`Error` ends a
/// generation; the terminal event carries the persisted message, and the renderer trusts that
/// snapshot rather than the accumulated deltas.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatEvent {
    Started {
        seq: u64,
        task_id: String,
        assistant_message_id: String,
    },
    Delta {
        seq: u64,
        assistant_message_id: String,
        text: String,
    },
    /// Display only: never stored, logged, or sent back to the model (D072).
    ReasoningDelta {
        seq: u64,
        assistant_message_id: String,
        text: String,
    },
    ToolProposal {
        seq: u64,
        assistant_message_id: String,
        proposal: ChatToolProposal,
    },
    Phase { seq: u64, task_id: String, assistant_message_id: String, phase: dgos_types::AiPhase },
    Activity { seq: u64, task_id: String, assistant_message_id: String, step: ChatRunStep },
    ModelSegment { seq: u64, task_id: String, assistant_message_id: String, round: u32 },
    SummaryDone { seq: u64, task_id: String, assistant_message_id: String, summary: ChatSummary,
        dispatched: bool, result_unknown: bool, audit_warning: Option<String> },
    Done {
        seq: u64,
        message: ChatMessage,
        dispatched: bool,
        result_unknown: bool,
        audit_warning: Option<String>,
    },
    Error {
        seq: u64,
        assistant_message_id: String,
        message: Option<ChatMessage>,
        error_code: String,
        provider_code: Option<u32>,
        result_unknown: bool,
        regeneration_discarded: bool,
        dispatched: bool,
        audit_warning: Option<String>,
    },
}

impl ChatEvent {
    fn sequence(&mut self) -> &mut u64 {
        match self {
            Self::Started { seq, .. } | Self::Delta { seq, .. } | Self::ReasoningDelta { seq, .. }
            | Self::ToolProposal { seq, .. } | Self::Phase { seq, .. } | Self::Activity { seq, .. }
            | Self::ModelSegment { seq, .. } | Self::SummaryDone { seq, .. }
            | Self::Done { seq, .. } | Self::Error { seq, .. } => seq,
        }
    }

    fn terminal(&self) -> bool { matches!(self, Self::Done { .. } | Self::Error { .. } | Self::SummaryDone { .. }) }
}


/// Where events go. `send` returns `false` once nobody can receive any more (the page is gone),
/// which stops the generation and saves what was received.
pub trait ChatSink: Send + Sync {
    fn send(&self, event: ChatEvent) -> bool;
}

impl<F> ChatSink for F
where
    F: Fn(ChatEvent) -> bool + Send + Sync,
{
    fn send(&self, event: ChatEvent) -> bool {
        self(event)
    }
}

// ---------------------------------------------------------------------------------------------
// Registry

#[derive(Debug)]
struct Active {
    user: String,
    session_id: String,
    window: String,
    token: CancellationToken,
    reason: Mutex<Option<&'static str>>,
    stream: Arc<Mutex<StreamState>>,
}

impl Active {
    /// The first reason wins; later calls only re-trigger the (idempotent) token.
    fn stop(&self, reason: &'static str) {
        {
            let mut slot = self.reason.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = Some(reason);
            }
        }
        self.token.cancel();
    }

    fn reason(&self) -> &'static str {
        self.reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unwrap_or("task_cancelled")
    }
}

/// Removes a registry entry when dropped, unless a newer entry reuses the id. `finish` removes
/// it explicitly first, so after a normal run this is a no-op.
struct ReleaseGuard {
    registry: Arc<Mutex<HashMap<String, Arc<Active>>>>,
    task_id: String,
    entry: Arc<Active>,
}

impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if registry
            .get(&self.task_id)
            .is_some_and(|e| Arc::ptr_eq(e, &self.entry))
        {
            registry.remove(&self.task_id);
        }
    }
}

/// A started generation waiting to be driven by [`ChatManager::run_generation`].
pub struct GenerationJob {
    task_id: String,
    user: String,
    assistant_message_id: String,
    provider_id: String,
    model_id: String,
    revision: Option<i64>,
    pins: Vec<PinnedSkill>,
    skill_runtime: Option<crate::skill_runtime::SkillRuntime>,
    web_runtime: Option<crate::web::WebRuntime>,
    blocked_effects: Vec<String>,
    summary: Option<continuous::SummaryPlan>,
    contexts: Vec<crate::foundation::ResolvedContext>,
    output: dgos_types::AiOutputContract,
    inline_receipts: Vec<dgos_types::AiContextReceipt>,
    source_runs: Vec<String>,
    request: ChatRequest,
    plan: Option<ToolPlan>,
    active: Arc<Active>,
    /// Frees the registry slot if the job is dropped without running.
    _guard: ReleaseGuard,
    /// Held from admission to the terminal write, so VACUUM cannot run during a generation.
    _permit: ActivityPermit,
}

impl GenerationJob {
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn assistant_message_id(&self) -> &str {
        &self.assistant_message_id
    }

    /// A tool turn is driven by `run_tool_turn`, not `run_generation`.
    pub fn is_tool_turn(&self) -> bool {
        self.plan.is_some()
    }
}

pub struct Sent {
    pub ack: SendAck,
    /// `None` for a replayed request: nothing new was started.
    pub job: Option<GenerationJob>,
}

#[derive(Clone)]
pub struct ChatManager {
    repo: ChatRepository,
    active: Arc<Mutex<HashMap<String, Arc<Active>>>>,
    stopping: Arc<AtomicBool>,
    db: DatabaseConnection,
    tool_turns: Arc<Mutex<HashMap<String, Arc<ToolTurn>>>>,
    gate: MaintenanceGate,
    account_lease: Option<CancellationToken>,
    web_vault: Option<Arc<dyn dgos_providers::ProviderCredentialVault>>,
    web_credential_lock: Option<Arc<tokio::sync::Mutex<()>>>,
}

struct Prepared {
    provider: Provider,
    model: Model,
    fingerprint: String,
    skills: Vec<PreparedSkill>,
    generation: dgos_types::GenerationParameters,
}

struct PreparedSkill {
    pin: PinnedSkill,
    content: dgos_skills::PreparedContent,
}

impl ChatManager {
    /// `gate` is the process-wide maintenance gate shared with every other writer (D133 item 5).
    /// Callers pass the same instance they manage as state; it is never created here.
    pub fn new(db: DatabaseConnection, gate: MaintenanceGate) -> Self {
        Self {
            repo: ChatRepository::new(db.clone()),
            db,
            active: Arc::new(Mutex::new(HashMap::new())),
            stopping: Arc::new(AtomicBool::new(false)),
            tool_turns: Arc::new(Mutex::new(HashMap::new())),
            gate,
            account_lease: None,
            web_vault: None,
            web_credential_lock: None,
        }
    }

    pub fn with_web_vault(mut self, vault: Arc<dyn dgos_providers::ProviderCredentialVault>) -> Self {
        self.web_vault = Some(vault);
        self
    }

    pub fn with_web_credential_lock(mut self, lock: Arc<tokio::sync::Mutex<()>>) -> Self {
        self.web_credential_lock = Some(lock);
        self
    }

    pub fn web_service(&self) -> std::result::Result<crate::web::WebService, String> {
        let vault = self.web_vault.clone().ok_or("web_unavailable")?;
        let mut service = crate::web::WebService::new(self.db.clone(), vault);
        if let Some(lease) = &self.account_lease { service = service.with_account_lease(lease.clone()); }
        if let Some(lock) = &self.web_credential_lock { service = service.with_credential_lock(lock.clone()); }
        Ok(service)
    }

    pub async fn configure_web(&self, user: &str, session_id: &str, window: &str, api_key: Option<String>, remove: bool) -> Result<crate::web::WebStatus> {
        self.check_lease()?;
        self.repo.get_session(user, session_id).await?;
        let _permit = self.gate.admit().map_err(|_| ChatError::GenerationInProgress)?;
        let task = uuid::Uuid::new_v4().to_string();
        let active = self.register(user, &task, session_id, window)?;
        let _guard = ReleaseGuard { registry: self.active.clone(), task_id: task, entry: active.clone() };
        let manager = self.clone();
        let (user, session_id) = (user.to_owned(), session_id.to_owned());
        // Native vault mutations are cancellation-safe owned work. Keep both reservations inside
        // that work even if the IPC command future or its window disappears while awaiting it.
        tokio::spawn(async move {
            let (_guard, _permit) = (_guard, _permit);
            let status = manager.web_service().map_err(ChatError::Web)?
                .configure(&user, api_key, remove).await.map_err(ChatError::Web)?;
            manager.check_lease()?;
            if active.token.is_cancelled() { return Err(ChatError::Provider(DgosError::SessionInvalid.error_code())); }
            manager.repo.get_session(&user, &session_id).await?;
            manager.check_lease()?;
            Ok(status)
        }).await.map_err(|_| ChatError::Internal)?
    }

    async fn authorize_web(&self, user: &str, task: &str, access: crate::web::WebAccess) -> Result<Option<crate::web::WebRuntime>> {
        self.check_lease()?;
        if !access.search && !access.fetch { return Ok(None); }
        let runtime = self.web_service().map_err(ChatError::Web)?
            .runtime(user, task, access).await.map_err(ChatError::Web)?;
        self.check_lease()?;
        Ok(Some(runtime))
    }

    /// Host captures owner and irreversible lease atomically before its first await. A bound
    /// clone shares registries and storage, never the account generation token with a later login.
    pub fn with_account_lease(&self, lease: CancellationToken) -> Self {
        let mut bound = self.clone();
        bound.account_lease = Some(lease);
        bound
    }

    fn check_lease(&self) -> Result<()> {
        if self.account_lease.as_ref().is_some_and(CancellationToken::is_cancelled) {
            Err(ChatError::Provider(DgosError::SessionInvalid.error_code()))
        } else { Ok(()) }
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Active>>> {
        self.active.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Check first, insert second, and never replace an entry: a duplicate task id must not
    /// orphan the original's cancellation token (D074).
    fn register(
        &self,
        user: &str,
        task_id: &str,
        session_id: &str,
        window: &str,
    ) -> Result<Arc<Active>> {
        self.check_lease()?;
        let mut registry = self.registry();
        if self.stopping.load(Ordering::SeqCst) {
            return Err(ChatError::Internal);
        }
        if registry.contains_key(task_id) {
            // Same answer for the owner and for a stranger reusing the id.
            return Err(ChatError::RequestIdConflict);
        }
        if registry.values().any(|entry| entry.user == user) {
            return Err(ChatError::GenerationInProgress);
        }
        let entry = Arc::new(Active {
            user: user.to_owned(),
            session_id: session_id.to_owned(),
            window: window.to_owned(),
            token: self.account_lease.as_ref().map(CancellationToken::child_token).unwrap_or_default(),
            reason: Mutex::new(None),
            stream: Arc::new(Mutex::new(StreamState::default())),
        });
        registry.insert(task_id.to_owned(), entry.clone());
        Ok(entry)
    }

    fn release(&self, task_id: &str) {
        self.registry().remove(task_id);
    }

    fn user_busy(&self, user: &str) -> bool {
        self.registry().values().any(|entry| entry.user == user)
    }

    pub fn has_active_session(&self, user: &str, session_id: &str) -> bool {
        self.registry()
            .values()
            .any(|entry| entry.user == user && entry.session_id == session_id)
    }

    /// Stop one of the user's own generations. Unknown, finished and foreign ids answer `false`;
    /// repeating the call changes nothing.
    pub fn cancel(&self, user: &str, task_id: &str) -> bool {
        let entry = self.registry().get(task_id).cloned();
        match entry {
            Some(entry) if entry.user == user => {
                entry.stop("task_cancelled");
                true
            }
            _ => false,
        }
    }

    /// The window was destroyed or its page reloaded: its generations end and keep what they
    /// received (D074 "leaving the page cancels").
    pub fn cancel_window(&self, window: &str) -> usize {
        let entries: Vec<_> = self
            .registry()
            .values()
            .filter(|entry| entry.window == window)
            .cloned()
            .collect();
        for entry in &entries {
            entry.stop("task_cancelled");
        }
        entries.len()
    }

    /// FR-008 session end (D087): stop every generation and tool turn of
    /// `user` and wait (bounded) for their terminal transactions. `false`
    /// means some were still running when the wait ended.
    pub async fn cancel_user(&self, user: &str) -> bool {
        let entries: Vec<_> = self
            .registry()
            .values()
            .filter(|entry| entry.user == user)
            .cloned()
            .collect();
        for entry in &entries {
            entry.stop("task_cancelled");
        }
        self.wait_drained(Some(user)).await
    }

    async fn wait_drained(&self, user: Option<&str>) -> bool {
        let wait = async {
            loop {
                let busy = self
                    .registry()
                    .values()
                    .any(|entry| user.is_none_or(|user| entry.user == user));
                if !busy {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(DRAIN_TIMEOUT, wait).await.is_ok()
    }

    /// Graceful exit: end every generation as `process_interrupted` and wait (bounded) for the
    /// terminal transactions to commit. `false` means some were still running; startup recovery
    /// finishes them.
    pub async fn shutdown(&self) -> bool {
        self.stopping.store(true, Ordering::SeqCst);
        let entries: Vec<_> = self.registry().values().cloned().collect();
        for entry in &entries {
            entry.stop("process_interrupted");
        }
        self.wait_drained(None).await
    }

    /// Startup, before any command is registered. The instance lock (D077) guarantees nothing
    /// else is generating.
    pub async fn recover_interrupted(&self) -> Result<u64> {
        self.repo.recover_runs(None).await?;
        Ok(self.repo.recover_generating().await?)
    }

    // -----------------------------------------------------------------------------------------
    // Sessions

    pub async fn create_session(
        &self,
        providers: &ProviderManager,
        skills: &SkillsManager,
        user: &str,
        input: CreateChatSessionInput,
    ) -> Result<ChatSession> {
        let target = match &input.target {
            Some(target) => resolve_target(providers, skills, user, target, &[]).await?,
            None => ChatTarget::default(),
        };
        self.repo
            .create_session(user, &target)
            .await
            .map_err(|_| ChatError::SessionCreationFailed)
    }

    pub async fn list_sessions(
        &self,
        user: &str,
        input: ListChatSessionsInput,
    ) -> Result<ChatSessionPage> {
        let limit = input.limit.unwrap_or(30).clamp(1, 50) as usize;
        let (items, next_cursor) = self
            .repo
            .list_sessions(user, input.cursor.as_deref(), limit)
            .await?;
        Ok(ChatSessionPage { items, next_cursor })
    }

    /// One of the user's sessions. Foreign and unknown ids are both `SessionNotFound`.
    pub async fn get_session(&self, user: &str, session_id: &str) -> Result<ChatSession> {
        Ok(self.repo.get_session(user, session_id).await?)
    }

    /// The session a message of the user belongs to; `None` for foreign or unknown ids.
    pub async fn session_of_message(&self, user: &str, message_id: &str) -> Result<Option<String>> {
        match self.repo.get_message(user, message_id).await {
            Ok(message) => Ok(Some(message.session_id)),
            Err(ChatStoreError::MessageNotFound) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// The session a task id of the user already belongs to (a replayed request).
    pub async fn session_of_task(&self, user: &str, task_id: &str) -> Result<Option<String>> {
        Ok(self.repo.session_of_task(user, task_id).await?)
    }

    pub async fn rename_session(
        &self,
        user: &str,
        input: RenameChatSessionInput,
    ) -> Result<ChatSession> {
        Ok(self
            .repo
            .rename_session(user, &input.session_id, &input.title)
            .await?)
    }

    pub async fn update_target(
        &self,
        providers: &ProviderManager,
        skills: &SkillsManager,
        user: &str,
        input: UpdateChatSessionTargetInput,
    ) -> Result<ChatSession> {
        // Existence first: a stranger's id must not reveal validation details.
        let session = self.repo.get_session(user, &input.session_id).await?;
        let target = resolve_target(
            providers,
            skills,
            user,
            &ChatTargetInput {
                provider_id: input.provider_id,
                model_id: input.model_id,
                skill_ids: input.skill_ids,
                generation: input.generation,
                no_skill: input.no_skill,
            },
            &session.skills,
        )
        .await?;
        if self.has_active_session(user, &input.session_id) {
            return Err(ChatError::GenerationInProgress);
        }
        Ok(self
            .repo
            .set_target(user, &input.session_id, &target)
            .await?)
    }

    pub async fn delete_session(&self, user: &str, input: DeleteChatSessionInput) -> Result<()> {
        if self.has_active_session(user, &input.session_id) {
            return Err(ChatError::GenerationInProgress);
        }
        self.repo
            .delete_session(user, &input.session_id)
            .await
            .map_err(|error| match error {
                ChatStoreError::Db(_) => ChatError::SessionDeletionFailed,
                other => other.into(),
            })
    }

    /// FR-008 dependency (D072): cancel the user's generation, wait for its terminal
    /// transaction, then remove all of the user's chat content.
    pub async fn purge_user(&self, user: &str) -> Result<u64> {
        let entries: Vec<_> = self
            .registry()
            .values()
            .filter(|entry| entry.user == user)
            .cloned()
            .collect();
        for entry in &entries {
            entry.stop("task_cancelled");
        }
        if !self.wait_drained(Some(user)).await {
            return Err(ChatError::GenerationInProgress);
        }
        Ok(self.repo.purge_user(user).await?)
    }

    pub async fn search_sessions(
        &self,
        user: &str,
        input: SearchChatSessionsInput,
    ) -> Result<ChatSearchPage> {
        let query = input.query.trim();
        let length = query.chars().count();
        if length == 0 || length > 128 {
            return Err(ChatError::InvalidSearchQuery);
        }
        let limit = input.limit.unwrap_or(30).clamp(1, 50) as usize;
        let (items, next_cursor) = self
            .repo
            .search_sessions(user, query, input.cursor.as_deref(), limit)
            .await?;
        Ok(ChatSearchPage { items, next_cursor })
    }

    pub async fn list_messages(
        &self,
        user: &str,
        input: ListChatMessagesInput,
    ) -> Result<ChatMessagePage> {
        let limit = input.limit.unwrap_or(50).clamp(1, 100) as usize;
        let (items, next_before_seq) = self
            .repo
            .list_messages(user, &input.session_id, input.before_seq, limit)
            .await?;
        Ok(ChatMessagePage {
            items,
            next_before_seq,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Send and regenerate

    pub async fn send(
        &self,
        providers: &ProviderManager,
        skills: &SkillsManager,
        user: &str,
        window: &str,
        input: SendChatInput,
    ) -> Result<Sent> {
        self.send_with_mcp(providers, skills, None, user, window, input)
            .await
    }

    /// `mcp` is needed only when the input selects tools; selecting tools without it is refused.
    pub async fn send_with_mcp(
        &self,
        providers: &ProviderManager,
        skills: &SkillsManager,
        mcp: Option<&McpManager>,
        user: &str,
        window: &str,
        input: SendChatInput,
    ) -> Result<Sent> {
        self.send_with_source(providers, skills, mcp, user, window, input, None).await
    }

    async fn send_with_source(
        &self,
        providers: &ProviderManager,
        skills: &SkillsManager,
        mcp: Option<&McpManager>,
        user: &str,
        window: &str,
        input: SendChatInput,
        source_run_id: Option<&str>,
    ) -> Result<Sent> {
        self.check_lease()?;
        let skill_mode = effective_skill_mode(input.skill_mode, &input.foundation);
        let task_id = canonical_uuid(&input.client_request_id)?;
        let content = input.content.trim();
        if (content.is_empty() && input.asset_ids.is_empty() && input.foundation.context_refs.is_empty()) || content.len() > MAX_MESSAGE_BYTES {
            return Err(ChatError::InvalidMessageContent);
        }
        if self.stopping.load(Ordering::SeqCst) {
            return Err(ChatError::Internal);
        }
        // Replay comes first so a repeated request never trips the busy check or re-validates.
        let probe = BeginGeneration {
            user,
            session_id: &input.session_id,
            task_id: &task_id,
            user_message_id: "",
            assistant_message_id: "",
            user_content: Some(content),
            regenerate_of: None,
            provider_id: "",
            model_id: "",
            provider_config_revision: None,
            endpoint_fingerprint: "",
            skills: &[],
            generation: &dgos_types::GenerationParameters::default(),
            tools: &[],
            assets: &input.asset_ids,
        };
        if let Some(begun) = self.repo.find_replay(&probe).await? {
            self.check_foundation_replay(user, &task_id, &input.foundation, &input.web_access).await?;
            return Ok(replayed(task_id, begun));
        }
        if self.user_busy(user) {
            return Err(ChatError::GenerationInProgress);
        }
        let prepared = self
            .prepare(
                providers,
                skills,
                user,
                &input.session_id,
                &task_id,
                input.accept_destination.as_deref(),
                skill_mode == ChatSkillMode::Manual,
            )
            .await?;
        let contexts = self.resolve_contexts(user, &input.session_id, &input.foundation).await?;
        let inline = foundation::inline_context(&contexts, &prepared.model, &input.foundation.output)?;
        let mut history = self
            .repo
            .history(user, &input.session_id, None, MAX_CONTEXT_BYTES)
            .await?;
        // Tools are re-selected on every send and checked against the current schema hashes.
        let selected = if input.tools.is_empty() {
            Vec::new()
        } else {
            let mcp = mcp.ok_or(ChatError::ToolTurnInvalid)?;
            if input.tools.len() > 8 {
                return Err(ChatError::ToolTurnInvalid);
            }
            crate::tools::resolve_selected_tools(mcp, user, input.tools.clone())
                .await
                .map_err(|_| ChatError::ToolTurnInvalid)?
        };
        let tool_refs: Vec<(String, String, String)> = selected
            .iter()
            .map(|t| {
                (
                    t.tool.server_id.clone(),
                    t.tool.tool_name.clone(),
                    t.tool.schema_hash.clone(),
                )
            })
            .collect();
        let runtime = self.authorize_runtime(skills, user, skill_mode, &input.authorized_skill_refs).await?;
        let web_runtime = self.authorize_web(user, &task_id, input.web_access.clone()).await?;
        let pins:Vec<PinnedSkill>=if let Some(runtime)=&runtime {runtime.pins().iter().map(|p|PinnedSkill{skill_id:p.skill_id.clone(),version_id:format!("{}@{}",p.skill_id,p.version),content_hash:p.content_hash.clone()}).collect()} else {prepared.skills.iter().map(|s|s.pin.clone()).collect()};
        let (runtime, local_reserve) = Self::local_schema_bytes(runtime, &contexts, web_runtime.as_ref())?;
        let summary = self.repo.summary(user, &input.session_id).await?;
        if let Some(summary) = &summary { history.rows.retain(|r| r.seq > summary.source_last_seq); }
        let has_tools = !selected.is_empty() || runtime.is_some() || web_runtime.is_some() || foundation::has_context_tools(&contexts);
        let (policy, reserve) = if !has_tools {
            (HOST_POLICY, 0)
        } else {
            (crate::tools::TOOL_HOST_POLICY, tool_reserve(&selected) + local_reserve)
        };
        let assets = crate::assets::ChatAssets::new(self.db.clone());
        let current_assets = assets.for_send(user, &input.asset_ids).await?;
        let history_assets = assets.for_messages(user, &history.rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>()).await?;
        let (messages, dropped) = assemble_assets(ContextAssembly {
            policy,
            reserve,
            skills: &prepared.skills.iter().map(|s| &s.content).collect::<Vec<_>>(),
            history: &history,
            current: content,
            context_window: prepared.model.context_window,
            generation: &prepared.generation,
            current_assets: &current_assets,
            history_assets: &history_assets,
            summary: summary.as_ref(),
            foundation: Some(&inline),
        })?;
        validate_image_support(&messages, &prepared.model)?;
        foundation::validate_budget(&messages, &prepared.generation, reserve, prepared.model.context_window, has_tools)?;
        let plan = has_tools.then(|| ToolPlan {
            messages: messages.clone(),
            tools: selected,
            window: prepared.model.context_window,
            generation: prepared.generation.clone(),
        });
        // Shared-database maintenance (D133 item 5) refuses new generations at once instead of waiting.
        let permit = self.gate.admit().map_err(|_| ChatError::GenerationInProgress)?;
        let active = self.register(user, &task_id, &input.session_id, window)?;
        let guard = ReleaseGuard {
            registry: self.active.clone(),
            task_id: task_id.clone(),
            entry: active.clone(),
        };
        // This process holds the user's only registry slot, so a `generating` row is an orphan.
        self.repo.recover_runs(Some(user)).await?;
        self.repo.recover_user_orphans(user).await?;
        let user_message_id = uuid::Uuid::new_v4().to_string();
        let assistant_message_id = uuid::Uuid::new_v4().to_string();
        let foundation_fingerprint = foundation::foundation_fingerprint(&input.foundation, &input.web_access)?;
        let begun = self
            .repo
            .begin_generation_with_metadata(&BeginGeneration {
                user,
                session_id: &input.session_id,
                task_id: &task_id,
                user_message_id: &user_message_id,
                assistant_message_id: &assistant_message_id,
                user_content: Some(content),
                regenerate_of: None,
                provider_id: &prepared.provider.id,
                model_id: &prepared.model.model_id,
                provider_config_revision: Some(prepared.provider.config_revision),
                endpoint_fingerprint: &prepared.fingerprint,
                skills: &pins,
                generation: &prepared.generation,
                tools: &tool_refs,
                assets: &input.asset_ids,
            }, Some(&foundation_fingerprint), source_run_id)
            .await;
        let begun = match begun {
            Ok(begun) => begun,
            Err(error) => {
                self.release(&task_id);
                return Err(error.into());
            }
        };
        if begun.replayed {
            self.check_foundation_replay(user, &task_id, &input.foundation, &input.web_access).await?;
            self.release(&task_id);
            return Ok(replayed(task_id, begun));
        }
        Ok(Sent {
            ack: SendAck {
                task_id: task_id.clone(),
                user_message_id: begun.user_message_id,
                assistant_message_id: begun.assistant_message_id.clone(),
                dropped_turns: dropped,
                replayed: false,
            },
            job: Some(GenerationJob {
                task_id,
                user: user.to_owned(),
                assistant_message_id: begun.assistant_message_id,
                provider_id: prepared.provider.id,
                model_id: prepared.model.model_id,
                revision: Some(prepared.provider.config_revision),
                pins,
                skill_runtime: runtime,
                web_runtime,
                blocked_effects: Vec::new(),
                summary: None,
                contexts,
                output: input.foundation.output,
                inline_receipts: inline.receipts,
                source_runs: Vec::new(),
                request: wire_request(messages, &prepared.generation),
                plan,
                active,
                _guard: guard,
                _permit: permit,
            }),
        })
    }

    pub async fn regenerate(
        &self,
        providers: &ProviderManager,
        skills: &SkillsManager,
        user: &str,
        window: &str,
        input: RegenerateChatInput,
    ) -> Result<Sent> {
        self.check_lease()?;
        let task_id = canonical_uuid(&input.client_request_id)?;
        if self.stopping.load(Ordering::SeqCst) {
            return Err(ChatError::Internal);
        }
        // A repeated request finds the session through its task id: the replaced reply is gone
        // once the first attempt succeeded.
        let known_session = self.repo.session_of_task(user, &task_id).await?;
        let message = match &known_session {
            Some(_) => None,
            None => Some(
                self.repo
                    .get_message(user, &input.message_id)
                    .await
                    .map_err(|error| match error {
                        ChatStoreError::MessageNotFound => ChatError::MessageNotRegenerable,
                        other => other.into(),
                    })?,
            ),
        };
        let session_id = match (&known_session, &message) {
            (Some(session), _) => session.clone(),
            (None, Some(message)) => message.session_id.clone(),
            (None, None) => return Err(ChatError::MessageNotRegenerable),
        };
        let probe = BeginGeneration {
            user,
            session_id: &session_id,
            task_id: &task_id,
            user_message_id: "",
            assistant_message_id: "",
            user_content: None,
            regenerate_of: Some(&input.message_id),
            provider_id: "",
            model_id: "",
            provider_config_revision: None,
            endpoint_fingerprint: "",
            skills: &[],
            generation: &dgos_types::GenerationParameters::default(),
            tools: &[],
            assets: &[],
        };
        if let Some(begun) = self.repo.find_replay(&probe).await? {
            self.check_foundation_replay(user, &task_id, &input.foundation, &input.web_access).await?;
            return Ok(replayed(task_id, begun));
        }
        let message = message.ok_or(ChatError::RequestIdConflict)?;
        if message.role != "assistant" || message.status == "generating" {
            return Err(ChatError::MessageNotRegenerable);
        }
        let question = self
            .repo
            .question_before(user, &session_id, message.seq)
            .await?;
        if self.user_busy(user) {
            return Err(ChatError::GenerationInProgress);
        }
        let skill_mode = effective_skill_mode(input.skill_mode, &input.foundation);
        let prepared = self
            .prepare(
                providers,
                skills,
                user,
                &session_id,
                &task_id,
                input.accept_destination.as_deref(),
                skill_mode == ChatSkillMode::Manual,
            )
            .await?;
        let contexts = self.resolve_contexts(user, &session_id, &input.foundation).await?;
        let inline = foundation::inline_context(&contexts, &prepared.model, &input.foundation.output)?;
        let runtime = self.authorize_runtime(skills, user, skill_mode, &input.authorized_skill_refs).await?;
        let web_runtime = self.authorize_web(user, &task_id, input.web_access.clone()).await?;
        let pins: Vec<PinnedSkill> = runtime.as_ref().map(|runtime| runtime.pins().iter().map(|pin| PinnedSkill {skill_id:pin.skill_id.clone(),version_id:format!("{}@{}",pin.skill_id,pin.version),content_hash:pin.content_hash.clone()}).collect()).unwrap_or_else(||prepared.skills.iter().map(|skill|skill.pin.clone()).collect());
        let has_tools = runtime.is_some() || web_runtime.is_some() || foundation::has_context_tools(&contexts);
        let (runtime, local_reserve) = Self::local_schema_bytes(runtime, &contexts, web_runtime.as_ref())?;
        // History ends before the question; the replaced reply is not part of the context.
        let history = self
            .repo
            .history(user, &session_id, Some(question.seq), MAX_CONTEXT_BYTES)
            .await?;
        let assets = crate::assets::ChatAssets::new(self.db.clone());
        let mut ids: Vec<_> = history.rows.iter().map(|row| row.id.clone()).collect();
        ids.push(question.id.clone());
        let mut history_assets = assets.for_messages(user, &ids).await?;
        let current_assets = history_assets.remove(&question.id).unwrap_or_default();
        let (messages, dropped) = assemble_assets(ContextAssembly {
            policy: if has_tools { crate::tools::TOOL_HOST_POLICY } else { HOST_POLICY },
            reserve: local_reserve,
            skills: &prepared.skills.iter().map(|s| &s.content).collect::<Vec<_>>(),
            history: &history,
            current: &question.content,
            context_window: prepared.model.context_window,
            generation: &prepared.generation,
            current_assets: &current_assets,
            history_assets: &history_assets,
            summary: None,
            foundation: Some(&inline),
        })?;
        validate_image_support(&messages, &prepared.model)?;
        foundation::validate_budget(&messages, &prepared.generation, local_reserve, prepared.model.context_window, has_tools)?;
        // Same admission as send: maintenance refuses new generations at once instead of waiting.
        let permit = self.gate.admit().map_err(|_| ChatError::GenerationInProgress)?;
        let active = self.register(user, &task_id, &session_id, window)?;
        let guard = ReleaseGuard {
            registry: self.active.clone(),
            task_id: task_id.clone(),
            entry: active.clone(),
        };
        self.repo.recover_runs(Some(user)).await?;
        self.repo.recover_user_orphans(user).await?;
        let assistant_message_id = uuid::Uuid::new_v4().to_string();
        let foundation_fingerprint = foundation::foundation_fingerprint(&input.foundation, &input.web_access)?;
        let begun = self
            .repo
            .begin_generation_with_metadata(&BeginGeneration {
                user,
                session_id: &session_id,
                task_id: &task_id,
                user_message_id: &question.id,
                assistant_message_id: &assistant_message_id,
                user_content: None,
                regenerate_of: Some(&input.message_id),
                provider_id: &prepared.provider.id,
                model_id: &prepared.model.model_id,
                provider_config_revision: Some(prepared.provider.config_revision),
                endpoint_fingerprint: &prepared.fingerprint,
                skills: &pins,
                generation: &prepared.generation,
                tools: &[],
                assets: &[],
            }, Some(&foundation_fingerprint), None)
            .await;
        let begun = match begun {
            Ok(begun) => begun,
            Err(error) => {
                self.release(&task_id);
                return Err(error.into());
            }
        };
        if begun.replayed {
            self.check_foundation_replay(user, &task_id, &input.foundation, &input.web_access).await?;
            self.release(&task_id);
            return Ok(replayed(task_id, begun));
        }
        Ok(Sent {
            ack: SendAck {
                task_id: task_id.clone(),
                user_message_id: begun.user_message_id,
                assistant_message_id: begun.assistant_message_id.clone(),
                dropped_turns: dropped,
                replayed: false,
            },
            job: Some(GenerationJob {
                task_id,
                user: user.to_owned(),
                assistant_message_id: begun.assistant_message_id,
                provider_id: prepared.provider.id,
                model_id: prepared.model.model_id,
                revision: Some(prepared.provider.config_revision),
                pins,
                skill_runtime: runtime,
                web_runtime,
                blocked_effects: Vec::new(),
                summary: None,
                contexts,
                output: input.foundation.output,
                inline_receipts: inline.receipts,
                source_runs: Vec::new(),
                request: wire_request(messages.clone(), &prepared.generation),
                plan: has_tools.then(|| ToolPlan { messages: messages.clone(), tools: Vec::new(), window: prepared.model.context_window, generation: prepared.generation.clone() }),
                active,
                _guard: guard,
                _permit: permit,
            }),
        })
    }

    /// Per-send snapshot (D075): the session's current target, a fresh provider/model read, the
    /// pinned skill revalidated, and the destination fingerprint compared to the last reply's.
    async fn prepare(
        &self,
        providers: &ProviderManager,
        skills: &SkillsManager,
        user: &str,
        session_id: &str,
        task_id: &str,
        accept_destination: Option<&str>,
        eager_skills: bool,
    ) -> Result<Prepared> {
        let session = self.repo.get_session(user, session_id).await?;
        let (provider, model) = resolve_model(
            providers,
            user,
            session.provider_id.as_deref(),
            session.model_id.as_deref(),
        )
        .await?;
        let provenance = ExecutionProvenance {
            caller: BuiltinCaller::Workbench,
            request_id: task_id.to_owned(),
        };
        session.generation.validate(Some(&model))?;
        validate_generation_profile(&provider, &session.generation)?;
        let skills = if eager_skills { resolve_skills(skills, user, &session, &provenance).await? } else { Vec::new() };
        let (fingerprint, host) = fingerprint(&provider.provider_type, &provider.base_url)?;
        let last = self
            .repo
            .last_endpoint_fingerprint(user, session_id)
            .await?;
        if last.as_deref().is_some_and(|last| last != fingerprint)
            && accept_destination != Some(fingerprint.as_str())
        {
            return Err(ChatError::DestinationChanged { fingerprint, host });
        }
        Ok(Prepared {
            provider,
            model,
            fingerprint,
            skills,
            generation: session.generation,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Generation

    /// Drive exactly one admitted generation and retain a bounded restart checkpoint.
    pub async fn run_generation(&self, providers: &ProviderManager, job: GenerationJob, sink: &dyn ChatSink) {
        use crate::execution::ExecutionGuard;
        let ordered = OrderedChatSink::new(sink, job.active.stream.clone());
        let sink: &dyn ChatSink = &ordered;
        let GenerationJob { task_id,user,assistant_message_id,provider_id,model_id,revision,pins,
            summary,request,plan,active,contexts,output,inline_receipts,source_runs,_guard,_permit,.. } = job;
        if plan.is_some() {
            self.finish(providers,&user,&task_id,&assistant_message_id,String::new(),End::Tool("tool_turn_misrouted".into()),sink).await;
            return;
        }
        if !sink.send(ChatEvent::Started { seq: 0, task_id:task_id.clone(),assistant_message_id:assistant_message_id.clone() }) { active.stop("task_cancelled"); }
        let checkpoint = CheckpointWriter::new(self.repo.clone(),user.clone(),task_id.clone());
        let guard = ChatStepGuard{manager:self,providers,user:&user,task:&task_id,assistant:&assistant_message_id,
            provider:&provider_id,model:&model_id,revision,active:&active,pins:&pins,blocked_effects:&[],contexts:&contexts,source_runs:&source_runs,sink};
        let forward = |event| {
            let sent = match event {
                crate::execution::CoreEvent::Delta(text) => { checkpoint.push(&text); sink.send(ChatEvent::Delta { seq: 0, assistant_message_id:assistant_message_id.clone(),text }) },
                crate::execution::CoreEvent::ReasoningDelta(text) => sink.send(ChatEvent::ReasoningDelta { seq: 0, assistant_message_id:assistant_message_id.clone(),text }),
                crate::execution::CoreEvent::Phase(phase) => sink.send(ChatEvent::Phase { seq: 0, task_id:task_id.clone(),assistant_message_id:assistant_message_id.clone(),phase }),
                crate::execution::CoreEvent::ModelSegment(round) => {
                    checkpoint.begin_segment();
                    sink.send(ChatEvent::ModelSegment { seq: 0, task_id: task_id.clone(), assistant_message_id: assistant_message_id.clone(), round })
                },
            };
            if !sent { active.stop("task_cancelled"); } sent
        };
        let provenance=ExecutionProvenance{caller:BuiltinCaller::Workbench,request_id:task_id.clone()};
        let binding=crate::execution::StepBinding{user:&user,provider_id:&provider_id,model_id:&model_id,revision,
            provenance:&provenance,cancellation:&active.token,deadline:tokio::time::Instant::now()+MAX_DURATION,guard:&guard,sink:&forward};
        let summary_valid=match summary.as_ref() {Some(plan)=>self.revalidate_summary(&user,&task_id,plan).await,None=>Ok(())};
        let admitted=match summary_valid {Ok(())=>guard.before_model().await,Err(error)=>Err(error)};
        let mut evidence=TerminalEvidence::default();
        let (content,mut end)=match admitted {
            Err(error)=>(String::new(),End::Tool(error)),
            Ok(())=>{
                sink.send(ChatEvent::Phase { seq: 0, task_id:task_id.clone(),assistant_message_id:assistant_message_id.clone(),phase:dgos_types::AiPhase::Model });
                let drained=crate::execution::drain_stream(providers,&binding,request).await;
                evidence.dispatched=drained.dispatched;
                evidence.audit_warning=drained.audit_warning.then(||"provider_audit_warning".into());
                let end=match drained.end {
                    crate::execution::StreamEnd::Completed{finish_reason,tokens}=>{evidence.tokens=tokens;End::Completed{finish_reason,tokens}},
                    crate::execution::StreamEnd::Stopped("task_cancelled")=>{evidence.result_unknown=drained.dispatched;End::Stopped(active.reason())},
                    crate::execution::StreamEnd::Stopped(reason)=>{evidence.result_unknown=drained.dispatched;End::Stopped(reason)},
                    crate::execution::StreamEnd::Failed(code)=>{evidence.result_unknown=drained.dispatched&&!matches!(code,1001001|1001002|1001003|1001004|1001005|1001012|1001015|1001016|1001017|1001018|1001019|1001020|1001022|1001023|1001024);End::Failed(code)},
                };(drained.content,end)
            }
        };
        if matches!(end,End::Completed{..}) {
            if let Err(error)=crate::foundation::FoundationGuard::revalidate(&guard,&contexts).await {end=End::Tool(error);}
        }
        if evidence.dispatched && self.record_inline_context(&user,&task_id,&inline_receipts).await.is_err() {
            end=End::Tool("persistence_failed".into());
            evidence.audit_warning=Some("context_receipt_persistence_failed".into());
        }
        if matches!(end, End::Completed { .. }) && crate::foundation::validate_output(&output, &content).is_err() {
            end = End::Tool("output_validation_failed".into());
        }
        if checkpoint.flush().await.is_err(){end=End::Tool("persistence_failed".into());evidence.audit_warning=Some("checkpoint_persistence_failed".into());}
        if evidence.audit_warning.is_some() && matches!(end,End::Completed{..}) { end=End::Tool("provider_audit_warning".into()); }
        if let Some(plan)=summary {
            self.finish_summary(providers,&user,&task_id,&assistant_message_id,&provider_id,&model_id,plan,content,end,evidence,&active,sink).await;
        } else {
            self.finish_evidenced(providers,&user,&task_id,&assistant_message_id,content,end,evidence,sink).await;
        }
    }

    async fn finish(
        &self,
        providers: &ProviderManager,
        user: &str,
        task_id: &str,
        assistant_message_id: &str,
        buffer: String,
        end: End,
        sink: &dyn ChatSink,
    ) {
        self.finish_evidenced(providers,user,task_id,assistant_message_id,buffer,end,TerminalEvidence::default(),sink).await;
    }

    async fn finish_evidenced(&self,providers:&ProviderManager,user:&str,task_id:&str,assistant_message_id:&str,buffer:String,mut end:End,mut evidence:TerminalEvidence,sink:&dyn ChatSink) {
        // Stream/future Drop may own a final observed-usage settlement. Its authoritative
        // write must finish before the run terminal can make those facts immutable.
        if !providers.settle_chat_usage(user,task_id).await {
            evidence.audit_warning.get_or_insert_with(||"provider_audit_warning".into());
            if matches!(end,End::Completed{..}) {end=End::Tool("provider_audit_warning".into());}
        }
        let mut outcome = outcome(&buffer, end);
        if evidence.tokens.is_some() { outcome.tokens=evidence.tokens; }
        let write = || async {
            self.repo
                .finalize_generation(&FinalizeGeneration {
                    user,
                    assistant_message_id,
                    task_id,
                    content: &buffer,
                    message_status: outcome.message_status,
                    finish_reason: outcome.finish_reason.as_deref(),
                    message_error: outcome.message_error.as_deref(),
                    task_status: outcome.task_status,
                    task_error: outcome.task_error.as_deref(),
                    total_tokens: outcome.tokens,
                    evidence: Some(dgos_db::repositories::chat_repository::ChatTerminalEvidence{dispatched:evidence.dispatched,result_unknown:evidence.result_unknown,audit_warning:evidence.audit_warning.clone()}),
                })
                .await
        };
        let mut result = write().await;
        if result.is_err() {
            tokio::time::sleep(Duration::from_millis(200)).await;
            result = write().await;
        }
        self.release(task_id);
        let error_code = outcome
            .task_error
            .clone()
            .or_else(|| outcome.message_error.clone())
            .unwrap_or_default();
        let mut event = match result {
            Ok(Finalized::Done { message, discarded }) => {
                if outcome.message_status == "completed" && !discarded {
                    ChatEvent::Done { seq: 0, dispatched:message.dispatched,result_unknown:message.result_unknown,audit_warning:message.audit_warning.clone(),message: *message }
                } else {
                    let unknown = evidence.result_unknown || (!discarded && message.result_unknown);
                    error_event(
                        assistant_message_id,
                        Some(*message),
                        error_code,
                        unknown,
                        discarded,
                    )
                }
            }
            Ok(Finalized::AlreadyFinalized) => {
                match self.repo.get_message(user, assistant_message_id).await {
                    Ok(message) if message.status == "completed" => ChatEvent::Done { seq: 0, dispatched:message.dispatched,result_unknown:message.result_unknown,audit_warning:message.audit_warning.clone(),message },
                    Ok(message) => {
                        let code = message.error_code.clone().unwrap_or_default();
                        let unknown = message.result_unknown;
                        error_event(assistant_message_id, Some(message), code, unknown, false)
                    }
                    Err(_) => error_event(
                        assistant_message_id,
                        None,
                        "persistence_failed".into(),
                        false,
                        false,
                    ),
                }
            }
            Err(_) => {
                // The row stays `generating` until restart recovery; nothing else can be done
                // about a database that refuses the terminal write.
                tracing_warn("chat terminal write failed");
                error_event(
                    assistant_message_id,
                    None,
                    "persistence_failed".into(),
                    false,
                    false,
                )
            }
        };
        if let ChatEvent::Error{dispatched,audit_warning,result_unknown,error_code,..}=&mut event {
            *dispatched=evidence.dispatched;
            *audit_warning=evidence.audit_warning.or_else(||(error_code.as_str()=="persistence_failed").then(||"terminal_persistence_failed".into()));
            *result_unknown|=evidence.result_unknown;
        }
        sink.send(event);
    }
}

fn tracing_warn(message: &str) {
    // `dgos-workbench` carries no logging dependency; the host logs through its own subscriber.
    eprintln!("dgos-workbench: {message}");
}

fn replayed(task_id: String, begun: dgos_db::Begun) -> Sent {
    Sent {
        ack: SendAck {
            task_id,
            user_message_id: begun.user_message_id,
            assistant_message_id: begun.assistant_message_id,
            dropped_turns: 0,
            replayed: true,
        },
        job: None,
    }
}

fn canonical_uuid(raw: &str) -> Result<String> {
    uuid::Uuid::parse_str(raw)
        .map(|id| id.to_string())
        .map_err(|_| ChatError::InvalidMessageContent)
}

fn wire_request(messages: Vec<WireMessage>, generation: &dgos_types::GenerationParameters) -> ChatRequest {
    let mut request = ChatRequest { messages, stream: Some(true), ..Default::default() };
    generation.apply(&mut request);
    request
}

/// Splits `text` so the first part fits `room` bytes on a character boundary; `true` when
/// something had to be cut.
fn split_at_boundary(text: &str, room: usize) -> (&str, bool) {
    if text.len() <= room {
        return (text, false);
    }
    let mut end = room;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn error_event(
    assistant_message_id: &str,
    message: Option<ChatMessage>,
    error_code: String,
    result_unknown: bool,
    regeneration_discarded: bool,
) -> ChatEvent {
    let provider_code = error_code
        .strip_prefix("provider_")
        .and_then(|code| code.parse::<u32>().ok());
    ChatEvent::Error { seq: 0, assistant_message_id: assistant_message_id.to_owned(),
    message,
    error_code,
    provider_code,
    result_unknown,
    regeneration_discarded,
    dispatched: false,
    audit_warning: None, }
}

// ---------------------------------------------------------------------------------------------
// Outcome mapping (D074)

#[derive(Debug, Clone, PartialEq, Eq)]
enum End {
    Completed {
        finish_reason: String,
        tokens: Option<i64>,
    },
    /// Cancelled or limited by the host; the reason is the task `error_code` category.
    Stopped(&'static str),
    /// Provider error code.
    Failed(u32),
    /// A tool-turn failure with its fixed error category (`tool_denied`, `confirmation_expired`,
    /// `provider_<code>`, ...). Tool turns never stream, so nothing was received.
    Tool(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Outcome {
    message_status: &'static str,
    finish_reason: Option<String>,
    message_error: Option<String>,
    task_status: &'static str,
    task_error: Option<String>,
    tokens: Option<i64>,
}

/// Message state follows whether anything was received; task state follows how the call ended.
fn outcome(buffer: &str, end: End) -> Outcome {
    let partial = if buffer.is_empty() {
        "failed"
    } else {
        "interrupted"
    };
    match end {
        End::Completed {
            finish_reason,
            tokens,
        } if !buffer.trim().is_empty() => Outcome {
            message_status: "completed",
            finish_reason: Some(finish_reason),
            message_error: None,
            task_status: "succeeded",
            task_error: None,
            tokens,
        },
        End::Completed { tokens, .. } => Outcome {
            message_status: "failed",
            finish_reason: None,
            message_error: Some("empty_response".into()),
            task_status: "failed",
            task_error: Some("empty_response".into()),
            tokens,
        },
        End::Stopped(reason) => Outcome {
            message_status: partial,
            finish_reason: None,
            message_error: Some(reason.into()),
            // An unanswered proposal or an idle task that times out is a failure; a stop the
            // user or the host asked for is an interruption.
            task_status: if matches!(reason, "confirmation_expired" | "task_expired") {
                "failed"
            } else {
                "interrupted"
            },
            task_error: Some(reason.into()),
            tokens: None,
        },
        End::Tool(category) => Outcome {
            message_status: partial,
            finish_reason: None,
            message_error: Some(category.clone()),
            task_status: "failed",
            task_error: Some(category),
            tokens: None,
        },
        End::Failed(code) => {
            let category = if code == DgosError::Timeout.error_code() {
                "provider_timeout".to_owned()
            } else {
                format!("provider_{code}")
            };
            Outcome {
                message_status: partial,
                finish_reason: None,
                message_error: Some(category.clone()),
                task_status: "failed",
                task_error: Some(category),
                tokens: None,
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Context assembly (D075)

/// system -> skill -> whole completed turns -> current message. Turns are dropped from the oldest
/// until the rest fits; a message is never truncated and the host system text is the only
/// system content. Returns the messages and the number of completed turns left out.
#[cfg(test)]
fn assemble(
    skills: &[&dgos_skills::PreparedContent],
    history: &History,
    current: &str,
    context_window: Option<i32>,
    generation: &dgos_types::GenerationParameters,
) -> Result<(Vec<WireMessage>, u32)> {
    assemble_assets(ContextAssembly {
        policy: HOST_POLICY,
        reserve: 0,
        skills,
        history,
        current,
        context_window,
        generation,
        current_assets: &[],
        history_assets: &HashMap::new(),
        summary: None,
        foundation: None,
    })
}

/// Serialized size of the selected tool definitions plus a margin for the request's JSON
/// framing, counted against the budget so a tool turn does not trip the later whole-request
/// check (D075: tool round trips are not part of later context).
fn tool_reserve(tools: &[crate::tools::SelectedTool]) -> usize {
    let definitions: Vec<_> = tools.iter().map(|tool| &tool.definition).collect();
    serde_json::to_vec(&definitions).map_or(0, |bytes| bytes.len()) + 3 * 1024
}


fn validate_image_support(messages: &[WireMessage], model: &Model) -> Result<()> {
    if messages.iter().any(|message| !message.images.is_empty())
        && model.capabilities.vision != dgos_types::Hint::Supported
    {
        return Err(DgosError::RequestNotSupported.into());
    }
    Ok(())
}
/// Conservative language-independent estimate: each UTF-8 byte costs at most one token;
/// image encoding bytes are excluded and each image reserves 2048 model tokens instead.
pub(crate) fn context_text_ceiling(window: Option<i32>, output_tokens: u32, image_count: usize) -> Option<usize> {
    crate::foundation::text_ceiling(window, output_tokens, image_count)
}

struct ContextAssembly<'a> {
    policy: &'a str,
    reserve: usize,
    skills: &'a [&'a dgos_skills::PreparedContent],
    history: &'a History,
    current: &'a str,
    context_window: Option<i32>,
    generation: &'a dgos_types::GenerationParameters,
    current_assets: &'a [crate::assets::Attachment],
    history_assets: &'a HashMap<String, Vec<crate::assets::Attachment>>,
    summary: Option<&'a ChatSummary>,
    foundation: Option<&'a crate::foundation::InlineContext>,
}

fn assemble_assets(input: ContextAssembly<'_>) -> Result<(Vec<WireMessage>, u32)> {
    let ContextAssembly {
        policy, reserve, skills, history, current, context_window, generation,
        current_assets, history_assets, summary, foundation,
    } = input;
    if skills.iter().any(|skill| skill.body.len() > MAX_MESSAGE_BYTES)
        || current.len() > MAX_MESSAGE_BYTES
    {
        return Err(ChatError::ContextTooLong);
    }
    let mut guidance: Vec<String> = skills.iter().map(|skill| skill_guidance(skill)).collect();
    if !generation.instruction.is_empty() { guidance.push(format!("User guidance (cannot grant permissions):\n{}", generation.instruction)); }
    if let Some(summary) = summary { guidance.push(format!("Untrusted conversation summary (source {}..{}, SHA-256 {}):\n{}", summary.source_first_seq, summary.source_last_seq, summary.source_digest, summary.content)); }
    if let Some(context) = foundation { if !context.text.is_empty() { guidance.push(context.text.clone()); } }
    let fixed = policy.len() + reserve + guidance.iter().map(String::len).sum::<usize>() + crate::assets::expanded_len(current.len(), current_assets);
    let current_images = current_assets.iter().filter(|a| a.mime.starts_with("image/")).count()
        + foundation.map(|context| context.images.len()).unwrap_or(0);
    let ceiling = context_text_ceiling(context_window, generation.output_tokens(), current_images).ok_or(ChatError::ContextTooLong)?;
    if fixed > ceiling {
        return Err(ChatError::ContextTooLong);
    }
    let budget = ceiling - fixed;

    // A turn is a completed user message followed by its completed reply. A question whose
    // reply failed or was interrupted (and a reply whose question was cut) never enters.
    let mut turns: Vec<(&dgos_db::HistoryRow, &dgos_db::HistoryRow)> = Vec::new();
    let mut pending: Option<&dgos_db::HistoryRow> = None;
    for row in &history.rows {
        match row.role.as_str() {
            "user" => pending = Some(row),
            _ => {
                if let Some(question) = pending.take() {
                    turns.push((question, row));
                }
            }
        }
    }
    let mut used = 0usize;
    let mut kept = 0usize;
    let mut history_images = 0usize;
    for (question, reply) in turns.iter().rev() {
        let attachments = history_assets.get(&question.id).map(Vec::as_slice).unwrap_or(&[]);
        let size = crate::assets::expanded_len(question.content.len(), attachments) + reply.content.len();
        let images = attachments.iter().filter(|a| a.mime.starts_with("image/")).count();
        let Some(ceiling) = context_text_ceiling(context_window, generation.output_tokens(), current_images + history_images + images) else { break };
        if used + size > budget || fixed + used + size > ceiling {
            break;
        }
        used += size;
        history_images += images;
        kept += 1;
    }
    let included = &turns[turns.len() - kept..];
    let dropped = (history.total_completed_replies as usize).saturating_sub(included.len());

    let mut messages = Vec::with_capacity(2 + included.len() * 2 + 1);
    messages.push(WireMessage {
        role: "system".into(),
        content: policy.into(),
        images: Vec::new(),
    });
    for guidance in guidance {
        messages.push(WireMessage {
            role: "user".into(),
            content: guidance,
            images: Vec::new(),
        });
    }
    for (question, reply) in included {
        let attachments = history_assets.get(&question.id).map(Vec::as_slice).unwrap_or(&[]);
        messages.push(WireMessage {
            role: "user".into(),
            content: crate::assets::expanded(&question.content, attachments),
            images: crate::assets::image_parts(attachments),
        });
        messages.push(WireMessage {
            role: "assistant".into(),
            content: reply.content.clone(),
            images: Vec::new(),
        });
    }
    let mut images = crate::assets::image_parts(current_assets);
    if let Some(context) = foundation { images.extend(context.images.iter().cloned()); }
    messages.push(WireMessage {
        role: "user".into(),
        content: crate::assets::expanded(current, current_assets),
        images,
    });
    Ok((messages, dropped as u32))
}

// ---------------------------------------------------------------------------------------------
// Target, skill and destination checks

/// Hash of provider type plus the endpoint's scheme, host and port (D075). Paths, query strings
/// and credentials are not part of it and the raw URL is never stored. Also returns the host
/// name for the confirmation prompt.
fn fingerprint(provider_type: &str, base_url: &str) -> Result<(String, String)> {
    let url = url::Url::parse(base_url)
        .map_err(|_| ChatError::Provider(DgosError::InsecureEndpoint.error_code()))?;
    let host = url
        .host_str()
        .ok_or(ChatError::Provider(
            DgosError::InsecureEndpoint.error_code(),
        ))?
        .to_ascii_lowercase();
    let port = url.port_or_known_default().unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{}\n{}\n{}\n{}",
        provider_type.to_ascii_lowercase(),
        url.scheme(),
        host,
        port
    ));
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok((hex, host))
}

async fn resolve_model(
    providers: &ProviderManager,
    user: &str,
    provider_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<(Provider, Model)> {
    let (Some(provider_id), Some(model_id)) = (provider_id, model_id) else {
        let any = providers.list_providers(user).await?;
        return Err(if any.is_empty() {
            ChatError::NoProviderConfigured
        } else {
            ChatError::ProviderNotSelected
        });
    };
    let (provider, model) = providers
        .model_snapshot(provider_id, model_id, user)
        .await
        .map_err(|error| match error {
            DgosError::ProviderNotFound(_) | DgosError::ModelNotFound(_) => {
                ChatError::ProviderNotSelected
            }
            other => other.into(),
        })?;
    if provider.user_id != user {
        return Err(ChatError::ProviderNotSelected);
    }
    validate_chat_model(&model)?;
    Ok((provider, model))
}

fn validate_chat_model(model:&Model)->Result<()> {
    let unsupported = || ChatError::Provider(DgosError::RequestNotSupported.error_code());
    if model.model_type != "text-generation" || model.capabilities.streaming == dgos_types::model_catalog::Hint::Unsupported {
        return Err(unsupported());
    }
    if model
        .max_output_tokens
        .is_some_and(|max| max < OUTPUT_TOKENS as i32)
    {
        return Err(unsupported());
    }
    Ok(())
}

pub(super) fn validate_generation_profile(provider: &Provider, generation: &dgos_types::GenerationParameters) -> Result<()> {
    crate::execution::validate_generation(provider, generation).map_err(Into::into)
}

async fn resolve_skills(
    skills: &SkillsManager,
    user: &str,
    session: &ChatSession,
    provenance: &ExecutionProvenance,
) -> Result<Vec<PreparedSkill>> {
    let mut prepared = Vec::with_capacity(session.skills.len());
    for pin in &session.skills {
        let content = skills.prepare_content_skill_with_provenance(&pin.skill_id, user, Some(provenance)).await.map_err(|_| ChatError::SkillUnavailable)?;
        if format!("{}@{}", content.skill_id, content.version) != pin.version_id || content.content_hash != pin.content_hash { return Err(ChatError::SkillUnavailable); }
        if content.body.len() > MAX_MESSAGE_BYTES { return Err(ChatError::ContextTooLong); }
        prepared.push(PreparedSkill { pin: pin.clone(), content });
    }
    Ok(prepared)
}

async fn resolve_target(
    providers: &ProviderManager,
    skills: &SkillsManager,
    user: &str,
    target: &ChatTargetInput,
    retained_pins: &[PinnedSkill],
) -> Result<ChatTarget> {
    if !target.skill_ids.is_empty() && target.no_skill {
        return Err(ChatError::InvalidMessageContent);
    }
    target.generation.validate(None)?;
    let mut seen = std::collections::HashSet::new();
    if target.skill_ids.iter().any(|id| !seen.insert(id)) { return Err(ChatError::InvalidMessageContent); }
    match (&target.provider_id, &target.model_id) {
        (None, None) => {}
        (Some(provider), Some(model)) => {
            let (provider, model) = resolve_model(providers, user, Some(provider), Some(model)).await?;
            target.generation.validate(Some(&model))?;
            validate_generation_profile(&provider, &target.generation)?;
        }
        _ => return Err(ChatError::ProviderNotSelected),
    }
    let mut pins = Vec::with_capacity(target.skill_ids.len());
    for id in &target.skill_ids {
        let content = skills.prepare_content_skill_with_provenance(id, user, None).await.map_err(|_| ChatError::SkillUnavailable)?;
        if let Some(pin) = retained_pins.iter().find(|pin| pin.skill_id == *id) {
            // Only removal followed by selection may resolve a replacement version.
            // Preferences, model changes and reordering must validate the original pin.
            if format!("{}@{}", content.skill_id, content.version) != pin.version_id
                || content.content_hash != pin.content_hash
            {
                return Err(ChatError::SkillUnavailable);
            }
            pins.push(pin.clone());
        } else {
            pins.push(PinnedSkill { skill_id: content.skill_id.clone(), version_id: format!("{}@{}", content.skill_id, content.version), content_hash: content.content_hash });
        }
    }
    Ok(ChatTarget {
        provider_id: target.provider_id.clone(),
        model_id: target.model_id.clone(),
        skills: pins,
        generation: target.generation.clone(),
        no_skill: target.no_skill,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::HistoryRow;

    fn row(seq: i64, role: &str, content: &str) -> HistoryRow {
        HistoryRow {
            id: format!("m{seq}"),
            seq,
            role: role.into(),
            content: content.into(),
        }
    }

    fn history(rows: Vec<HistoryRow>, total: i64) -> History {
        History {
            rows,
            total_completed_replies: total,
        }
    }

    fn roles(messages: &[WireMessage]) -> Vec<&str> {
        messages.iter().map(|m| m.role.as_str()).collect()
    }

    #[test]
    fn assemble_orders_system_history_current_and_never_takes_roles_from_input() {
        let h = history(
            vec![
                row(1, "user", "q1"),
                row(2, "assistant", "a1"),
                row(3, "user", "q2"),
                row(4, "assistant", "a2"),
            ],
            2,
        );
        let (messages, dropped) = assemble(&[], &h, "now", None, &dgos_types::GenerationParameters::default()).unwrap();
        assert_eq!(dropped, 0);
        assert_eq!(
            roles(&messages),
            ["system", "user", "assistant", "user", "assistant", "user"]
        );
        assert_eq!(messages[0].content, HOST_POLICY);
        assert_eq!(messages.last().unwrap().content, "now");
        assert_eq!(messages.iter().filter(|m| m.role == "system").count(), 1);
    }

    #[test]
    fn assemble_drops_whole_oldest_turns_and_reports_how_many() {
        let big = "x".repeat(8 * 1024);
        let h = history(
            vec![
                row(1, "user", "old question"),
                row(2, "assistant", &big),
                row(3, "user", "middle question"),
                row(4, "assistant", &big),
                row(5, "user", "recent question"),
                row(6, "assistant", &big),
            ],
            3,
        );
        // 24 KiB minus the host text leaves room for two 8 KiB turns, not three.
        let (messages, dropped) = assemble(&[], &h, "now", None, &dgos_types::GenerationParameters::default()).unwrap();
        assert_eq!(dropped, 1);
        let contents: Vec<_> = messages.iter().map(|m| m.content.as_str()).collect();
        assert!(!contents.contains(&"old question"));
        assert!(contents.contains(&"middle question") && contents.contains(&"recent question"));
        assert_eq!(
            messages.len(),
            1 + 4 + 1,
            "turns stay whole: no lone question or reply"
        );
    }

    #[test]
    fn assemble_skips_questions_without_a_completed_reply_and_replies_without_a_question() {
        let h = history(
            vec![
                row(1, "user", "failed question"),
                row(3, "user", "answered question"),
                row(4, "assistant", "answer"),
                row(5, "assistant", "reply whose question was cut"),
                row(7, "user", "trailing question without reply"),
            ],
            2,
        );
        let (messages, dropped) = assemble(&[], &h, "now", None, &dgos_types::GenerationParameters::default()).unwrap();
        let contents: Vec<_> = messages.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.contains(&"answered question") && contents.contains(&"answer"));
        assert!(!contents.contains(&"failed question"));
        assert!(!contents.contains(&"reply whose question was cut"));
        assert!(!contents.contains(&"trailing question without reply"));
        assert_eq!(dropped, 1, "the unpaired reply counts as an omitted turn");
    }

    #[test]
    fn assemble_refuses_instead_of_truncating() {
        let h = history(vec![], 0);
        let too_long = "y".repeat(MAX_MESSAGE_BYTES + 1);
        assert_eq!(
            assemble(&[], &h, &too_long, None, &Default::default()).unwrap_err(),
            ChatError::ContextTooLong
        );
        // A known small window leaves no room even for the host text.
        assert_eq!(
            assemble(&[], &h, "hi", Some(2048), &Default::default()).unwrap_err(),
            ChatError::ContextTooLong
        );
        assert_eq!(
            assemble(&[], &h, "hi", Some(0), &Default::default()).unwrap_err(),
            ChatError::ContextTooLong
        );
        assert!(assemble(&[], &h, "hi", Some(32768), &Default::default()).is_ok());
    }

    #[test]
    fn assemble_with_the_whole_history_dropped_still_succeeds_when_the_current_message_fits() {
        let huge = "z".repeat(40 * 1024);
        let h = history(vec![row(1, "user", "q"), row(2, "assistant", &huge)], 1);
        let (messages, dropped) = assemble(&[], &h, "now", None, &dgos_types::GenerationParameters::default()).unwrap();
        assert_eq!(dropped, 1);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn outcomes_follow_the_d074_table() {
        let done = outcome(
            "text",
            End::Completed {
                finish_reason: "length".into(),
                tokens: Some(9),
            },
        );
        assert_eq!(
            (done.message_status, done.task_status, done.tokens),
            ("completed", "succeeded", Some(9))
        );
        assert_eq!(done.finish_reason.as_deref(), Some("length"));

        let empty = outcome(
            "  ",
            End::Completed {
                finish_reason: "stop".into(),
                tokens: None,
            },
        );
        assert_eq!(
            (empty.message_status, empty.task_error.as_deref()),
            ("failed", Some("empty_response"))
        );

        let cancelled = outcome("partial", End::Stopped("task_cancelled"));
        assert_eq!(
            (cancelled.message_status, cancelled.task_status),
            ("interrupted", "interrupted")
        );
        let cancelled_empty = outcome("", End::Stopped("task_cancelled"));
        assert_eq!(cancelled_empty.message_status, "failed");
        assert_eq!(cancelled_empty.task_status, "interrupted");

        let limit = outcome("p", End::Stopped("generation_limit"));
        assert_eq!(
            (limit.task_status, limit.task_error.as_deref()),
            ("interrupted", Some("generation_limit"))
        );

        let network = outcome("p", End::Failed(1001007));
        assert_eq!(
            (network.message_status, network.task_status),
            ("interrupted", "failed")
        );
        assert_eq!(network.task_error.as_deref(), Some("provider_1001007"));
        let timeout = outcome("", End::Failed(1001006));
        assert_eq!(
            (timeout.message_status, timeout.task_error.as_deref()),
            ("failed", Some("provider_timeout"))
        );
        assert_eq!(network.tokens, None, "unknown usage is never 0");
    }

    #[test]
    fn fingerprint_ignores_path_and_credentials_but_not_host_port_scheme_or_type() {
        let base = fingerprint("openai_compatible", "https://api.example.test/v1").unwrap();
        assert_eq!(base.1, "api.example.test");
        assert_eq!(
            base.0,
            fingerprint(
                "openai_compatible",
                "https://API.example.test/other/path?x=1"
            )
            .unwrap()
            .0
        );
        assert_eq!(
            base.0,
            fingerprint("openai_compatible", "https://api.example.test:443/v2")
                .unwrap()
                .0
        );
        for changed in [
            fingerprint("openai_compatible", "https://api.other.test/v1")
                .unwrap()
                .0,
            fingerprint("openai_compatible", "https://api.example.test:8443/v1")
                .unwrap()
                .0,
            fingerprint("openai_compatible", "http://api.example.test/v1")
                .unwrap()
                .0,
            fingerprint("anthropic", "https://api.example.test/v1")
                .unwrap()
                .0,
        ] {
            assert_ne!(base.0, changed);
        }
        assert!(
            !base.0.contains("api.example.test"),
            "only a hash is stored"
        );
        assert_eq!(base.0.len(), 64);
    }

    #[tokio::test]
    async fn the_registry_never_overwrites_an_existing_cancellation_token() {
        let db = dgos_db::sea_orm::Database::connect("sqlite::memory:")
            .await
            .unwrap();
        let manager = ChatManager::new(db, dgos_db::MaintenanceGate::new());
        let first = manager.register("alice", "task-1", "s1", "main").unwrap();
        assert_eq!(
            manager
                .register("alice", "task-1", "s1", "main")
                .unwrap_err(),
            ChatError::RequestIdConflict,
            "a duplicate id is refused"
        );
        assert_eq!(
            manager.register("bob", "task-1", "s2", "main").unwrap_err(),
            ChatError::RequestIdConflict
        );
        assert!(
            manager.cancel("alice", "task-1"),
            "the original is still cancellable"
        );
        assert!(
            first.token.is_cancelled(),
            "the original token, not a replacement, fired"
        );
        assert_eq!(first.reason(), "task_cancelled");
    }

    #[tokio::test]
    async fn one_active_generation_per_user_and_cancel_is_owner_only_and_repeatable() {
        let db = dgos_db::sea_orm::Database::connect("sqlite::memory:")
            .await
            .unwrap();
        let manager = ChatManager::new(db, dgos_db::MaintenanceGate::new());
        let alice = manager.register("alice", "t1", "s1", "main").unwrap();
        assert_eq!(
            manager.register("alice", "t2", "s2", "main").unwrap_err(),
            ChatError::GenerationInProgress
        );
        let bob = manager.register("bob", "t3", "s3", "main").unwrap();
        assert!(
            !manager.cancel("bob", "t1"),
            "another user's task answers false"
        );
        assert!(!alice.token.is_cancelled());
        assert!(!manager.cancel("alice", "unknown"));
        assert!(manager.cancel("alice", "t1"));
        assert!(
            manager.cancel("alice", "t1"),
            "repeating has no further effect"
        );
        assert!(!bob.token.is_cancelled());
        assert!(
            manager.has_active_session("bob", "s3") && !manager.has_active_session("alice", "s3")
        );
        manager.release("t1");
        assert!(
            !manager.cancel("alice", "t1"),
            "a finished task answers false"
        );
        assert!(manager.register("alice", "t4", "s1", "main").is_ok());
    }

    #[tokio::test]
    async fn the_first_stop_reason_wins_and_windows_only_cancel_their_own() {
        let db = dgos_db::sea_orm::Database::connect("sqlite::memory:")
            .await
            .unwrap();
        let manager = ChatManager::new(db, dgos_db::MaintenanceGate::new());
        let a = manager.register("alice", "t1", "s1", "main").unwrap();
        let b = manager.register("bob", "t2", "s2", "other").unwrap();
        a.stop("process_interrupted");
        a.stop("task_cancelled");
        assert_eq!(a.reason(), "process_interrupted");
        assert_eq!(manager.cancel_window("main"), 1);
        assert!(!b.token.is_cancelled());
        assert_eq!(manager.cancel_window("other"), 1);
        assert!(b.token.is_cancelled());
    }

    #[test]
    fn split_at_boundary_keeps_characters_whole() {
        assert_eq!(split_at_boundary("abc", 10), ("abc", false));
        assert_eq!(split_at_boundary("abcdef", 3), ("abc", true));
        assert_eq!(split_at_boundary("字字字", 4), ("字", true));
    }

    #[test]
    fn inputs_reject_identity_and_unknown_fields() {
        let send = |value: Value| serde_json::from_value::<SendChatInput>(value);
        let good = json!({"session_id":"s","content":"c","client_request_id":"i"});
        assert!(send(good.clone()).is_ok());
        for key in ["user_id", "userId", "role", "system_prompt"] {
            let mut bad = good.clone();
            bad[key] = json!("x");
            assert!(send(bad).is_err(), "{key} must be rejected");
        }
        assert!(serde_json::from_value::<CreateChatSessionInput>(json!({"user_id":"u"})).is_err());
        assert!(serde_json::from_value::<ListChatMessagesInput>(
            json!({"session_id":"s","userId":"u"})
        )
        .is_err());
    }

    #[test]
    fn ordered_events_are_contiguous_and_terminal_is_immutable() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let output = received.clone();
        let sink: Arc<dyn ChatSink> = Arc::new(move |event| { output.lock().unwrap().push(event); true });
        let ordered = OrderedChatSink::new(sink, Arc::new(Mutex::new(StreamState::default())));
        assert!(ordered.send(ChatEvent::Started { seq: 0, task_id: "run".into(), assistant_message_id: "answer".into() }));
        assert!(ordered.send(ChatEvent::ModelSegment { seq: 0, task_id: "run".into(), assistant_message_id: "answer".into(), round: 1 }));
        assert!(ordered.send(error_event("answer", None, "task_cancelled".into(), false, false)));
        assert!(!ordered.send(ChatEvent::Delta { seq: 0, assistant_message_id: "answer".into(), text: "late".into() }));
        assert!(!ordered.send(error_event("answer", None, "different_terminal".into(), false, false)));
        let events = received.lock().unwrap();
        assert_eq!(events.len(), 3);
        for (index, event) in events.iter().enumerate() {
            assert_eq!(serde_json::to_value(event).unwrap()["seq"], index + 1);
        }
    }

    #[test]
    fn web_permission_defaults_off_and_is_strictly_request_scoped() {
        let send: SendChatInput = serde_json::from_value(json!({"session_id":"s","content":"c","client_request_id":"i"})).unwrap();
        let regenerate: RegenerateChatInput = serde_json::from_value(json!({"message_id":"m","client_request_id":"i"})).unwrap();
        let continuation: ContinueChatRunInput = serde_json::from_value(json!({"source_task_id":"t","intent":"next","client_request_id":"i"})).unwrap();
        assert!(!send.web_access.search && !send.web_access.fetch);
        assert!(!regenerate.web_access.search && !regenerate.web_access.fetch);
        assert!(!continuation.web_access.search && !continuation.web_access.fetch);
        assert!(serde_json::from_value::<SendChatInput>(json!({"session_id":"s","content":"c","client_request_id":"i","web_access":{"search":true,"admin":true}})).is_err());
        let options = dgos_types::AiFoundationOptions::default();
        assert_ne!(foundation::foundation_fingerprint(&options, &Default::default()).unwrap(),
            foundation::foundation_fingerprint(&options, &crate::web::WebAccess { search: true, fetch: false }).unwrap());
    }

    #[test]
    fn only_safe_destination_or_web_metadata_carries_detail() {
        let changed = ChatError::DestinationChanged {
            fingerprint: "fp".into(),
            host: "h".into(),
        };
        assert_eq!(changed.code(), 1006012);
        assert_eq!(changed.detail().unwrap()["host"], "h");
        assert!(ChatError::SessionNotFound.detail().is_none());
        assert_eq!(ChatError::Web("web_search_key_required".into()).detail().unwrap()["reason"], "web_search_key_required");
        assert_eq!(
            ChatError::from(DgosError::StreamInterrupted).code(),
            1001007
        );
        assert_eq!(
            ChatError::from(DgosError::InternalError("secret".into())).code(),
            1000000
        );
        assert!(!ChatError::Provider(1001007).message().contains("secret"));
    }
}
