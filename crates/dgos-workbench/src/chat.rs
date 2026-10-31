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

use crate::tools::{ToolProposal, ToolSelection};
use crate::{skill_guidance, HOST_POLICY, OUTPUT_TOKENS};
use dgos_db::{
    sea_orm::DatabaseConnection, BeginGeneration, ChatMessage, ChatRepository, ChatSession,
    ChatStoreError, ChatTarget, FinalizeGeneration, Finalized, History, PinnedSkill, SearchHit,
};
use dgos_mcp::McpManager;
use dgos_providers::ProviderManager;
use dgos_skills::SkillsManager;
use dgos_types::{
    BuiltinCaller, ChatMessage as WireMessage, ChatRequest, DgosError, ExecutionProvenance, Model,
    Provider,
};
use futures::StreamExt;
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
            Self::Provider(_) => "provider_error",
            Self::Internal => "internal_error",
        }
    }

    /// Only `destination_changed` carries detail: the new fingerprint to accept and the host
    /// name (no path, no credentials).
    pub fn detail(&self) -> Option<Value> {
        match self {
            Self::DestinationChanged { fingerprint, host } => {
                Some(json!({"fingerprint": fingerprint, "host": host}))
            }
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
    pub skill_id: Option<String>,
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
    pub skill_id: Option<String>,
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
    /// One to eight MCP tools chosen for this send only (D022/D075). Non-empty makes the turn a
    /// non-streaming tool turn; nothing is remembered for the next send.
    #[serde(default)]
    pub tools: Vec<ToolSelection>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegenerateChatInput {
    pub message_id: String,
    pub client_request_id: String,
    #[serde(default)]
    pub accept_destination: Option<String>,
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
        task_id: String,
        assistant_message_id: String,
    },
    Delta {
        assistant_message_id: String,
        text: String,
    },
    /// Display only: never stored, logged, or sent back to the model (D072).
    ReasoningDelta {
        assistant_message_id: String,
        text: String,
    },
    ToolProposal {
        assistant_message_id: String,
        proposal: ChatToolProposal,
    },
    Done {
        message: ChatMessage,
    },
    Error {
        assistant_message_id: String,
        message: Option<ChatMessage>,
        error_code: String,
        provider_code: Option<u32>,
        result_unknown: bool,
        regeneration_discarded: bool,
    },
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
    request: ChatRequest,
    plan: Option<ToolPlan>,
    active: Arc<Active>,
    /// Frees the registry slot if the job is dropped without running.
    _guard: ReleaseGuard,
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
}

struct Prepared {
    provider: Provider,
    model: Model,
    fingerprint: String,
    skill: Option<PreparedSkill>,
}

struct PreparedSkill {
    pin: PinnedSkill,
    content: dgos_skills::PreparedContent,
}

impl ChatManager {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            repo: ChatRepository::new(db.clone()),
            db,
            active: Arc::new(Mutex::new(HashMap::new())),
            stopping: Arc::new(AtomicBool::new(false)),
            tool_turns: Arc::new(Mutex::new(HashMap::new())),
        }
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
            token: CancellationToken::new(),
            reason: Mutex::new(None),
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
            Some(target) => resolve_target(providers, skills, user, target).await?,
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
        self.repo.get_session(user, &input.session_id).await?;
        let target = resolve_target(
            providers,
            skills,
            user,
            &ChatTargetInput {
                provider_id: input.provider_id,
                model_id: input.model_id,
                skill_id: input.skill_id,
                no_skill: input.no_skill,
            },
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
        let task_id = canonical_uuid(&input.client_request_id)?;
        let content = input.content.trim();
        if content.is_empty() || content.len() > MAX_MESSAGE_BYTES {
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
            skill: None,
            tools: &[],
        };
        if let Some(begun) = self.repo.find_replay(&probe).await? {
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
            )
            .await?;
        let history = self
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
        let (policy, reserve) = if selected.is_empty() {
            (HOST_POLICY, 0)
        } else {
            (crate::tools::TOOL_HOST_POLICY, tool_reserve(&selected))
        };
        let (messages, dropped) = assemble_with(
            policy,
            reserve,
            prepared.skill.as_ref().map(|s| &s.content),
            &history,
            content,
            prepared.model.context_window,
        )?;
        let plan = (!selected.is_empty()).then(|| ToolPlan {
            messages: messages.clone(),
            tools: selected,
            window: prepared.model.context_window,
        });
        let active = self.register(user, &task_id, &input.session_id, window)?;
        let guard = ReleaseGuard {
            registry: self.active.clone(),
            task_id: task_id.clone(),
            entry: active.clone(),
        };
        // This process holds the user's only registry slot, so a `generating` row is an orphan.
        self.repo.recover_user_orphans(user).await?;
        let user_message_id = uuid::Uuid::new_v4().to_string();
        let assistant_message_id = uuid::Uuid::new_v4().to_string();
        let begun = self
            .repo
            .begin_generation(&BeginGeneration {
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
                skill: prepared.skill.as_ref().map(|s| &s.pin),
                tools: &tool_refs,
            })
            .await;
        let begun = match begun {
            Ok(begun) => begun,
            Err(error) => {
                self.release(&task_id);
                return Err(error.into());
            }
        };
        if begun.replayed {
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
                request: wire_request(messages),
                plan,
                active,
                _guard: guard,
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
            skill: None,
            tools: &[],
        };
        if let Some(begun) = self.repo.find_replay(&probe).await? {
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
        let prepared = self
            .prepare(
                providers,
                skills,
                user,
                &session_id,
                &task_id,
                input.accept_destination.as_deref(),
            )
            .await?;
        // History ends before the question; the replaced reply is not part of the context.
        let history = self
            .repo
            .history(user, &session_id, Some(question.seq), MAX_CONTEXT_BYTES)
            .await?;
        let (messages, dropped) = assemble(
            prepared.skill.as_ref().map(|s| &s.content),
            &history,
            &question.content,
            prepared.model.context_window,
        )?;
        let active = self.register(user, &task_id, &session_id, window)?;
        let guard = ReleaseGuard {
            registry: self.active.clone(),
            task_id: task_id.clone(),
            entry: active.clone(),
        };
        self.repo.recover_user_orphans(user).await?;
        let assistant_message_id = uuid::Uuid::new_v4().to_string();
        let begun = self
            .repo
            .begin_generation(&BeginGeneration {
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
                skill: prepared.skill.as_ref().map(|s| &s.pin),
                tools: &[],
            })
            .await;
        let begun = match begun {
            Ok(begun) => begun,
            Err(error) => {
                self.release(&task_id);
                return Err(error.into());
            }
        };
        if begun.replayed {
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
                request: wire_request(messages),
                plan: None,
                active,
                _guard: guard,
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
        let skill = resolve_skill(skills, user, &session, &provenance).await?;
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
            skill,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Generation

    /// Streams one generation to `sink` and ends it in exactly one terminal transaction. The
    /// registry entry is released after that transaction commits and before the terminal event
    /// is sent, so a client reacting to the event can send again immediately.
    pub async fn run_generation(
        &self,
        providers: &ProviderManager,
        job: GenerationJob,
        sink: &dyn ChatSink,
    ) {
        let GenerationJob {
            task_id,
            user,
            assistant_message_id,
            provider_id,
            model_id,
            revision,
            request,
            plan,
            active,
            _guard,
        } = job;
        if plan.is_some() {
            // A tool turn belongs to `run_tool_turn`; running it as plain text would drop its tools.
            self.finish(
                &user,
                &task_id,
                &assistant_message_id,
                String::new(),
                End::Tool("tool_turn_misrouted".into()),
                sink,
            )
            .await;
            return;
        }
        if !sink.send(ChatEvent::Started {
            task_id: task_id.clone(),
            assistant_message_id: assistant_message_id.clone(),
        }) {
            active.stop("task_cancelled");
        }
        let provenance = ExecutionProvenance {
            caller: BuiltinCaller::Workbench,
            request_id: task_id.clone(),
        };
        let deadline = tokio::time::sleep(MAX_DURATION);
        tokio::pin!(deadline);
        let mut buffer = String::new();
        let end = 'drain: {
            let opened = tokio::select! {
                biased;
                _ = active.token.cancelled() => break 'drain End::Stopped(active.reason()),
                _ = &mut deadline => break 'drain End::Stopped("generation_limit"),
                opened = providers.stream_provider_bound_cancellable_with_provenance(
                    &provider_id, &model_id, request, &user, revision, Some(&provenance),
                    active.token.clone(),
                ) => opened,
            };
            let mut stream = match opened {
                Ok(stream) => stream,
                Err(error) => break 'drain End::Failed(error.error_code()),
            };
            loop {
                let item = tokio::select! {
                    biased;
                    _ = active.token.cancelled() => break 'drain End::Stopped(active.reason()),
                    _ = &mut deadline => break 'drain End::Stopped("generation_limit"),
                    item = stream.next() => item,
                };
                match item {
                    None => break 'drain End::Failed(DgosError::StreamInterrupted.error_code()),
                    Some(Err(error)) => {
                        break 'drain if active.token.is_cancelled() {
                            End::Stopped(active.reason())
                        } else {
                            End::Failed(error.error_code())
                        };
                    }
                    Some(Ok(chunk)) => {
                        if let Some(text) = chunk.reasoning_delta.filter(|t| !t.is_empty()) {
                            if !sink.send(ChatEvent::ReasoningDelta {
                                assistant_message_id: assistant_message_id.clone(),
                                text,
                            }) {
                                active.stop("task_cancelled");
                            }
                        }
                        if !chunk.delta.is_empty() {
                            let room = MAX_REPLY_BYTES - buffer.len();
                            let (piece, over) = split_at_boundary(&chunk.delta, room);
                            buffer.push_str(piece);
                            if !piece.is_empty()
                                && !sink.send(ChatEvent::Delta {
                                    assistant_message_id: assistant_message_id.clone(),
                                    text: piece.to_owned(),
                                })
                            {
                                active.stop("task_cancelled");
                            }
                            if over {
                                break 'drain End::Stopped("generation_limit");
                            }
                        }
                        if let Some(finish_reason) = chunk.finish_reason {
                            break 'drain End::Completed {
                                finish_reason,
                                tokens: chunk
                                    .usage
                                    .map(|usage| i64::from(usage.total_tokens))
                                    .filter(|tokens| *tokens > 0),
                            };
                        }
                    }
                }
            }
        };
        self.finish(&user, &task_id, &assistant_message_id, buffer, end, sink)
            .await;
    }

    async fn finish(
        &self,
        user: &str,
        task_id: &str,
        assistant_message_id: &str,
        buffer: String,
        end: End,
        sink: &dyn ChatSink,
    ) {
        let outcome = outcome(&buffer, end);
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
        let event = match result {
            Ok(Finalized::Done { message, discarded }) => {
                if outcome.message_status == "completed" && !discarded {
                    ChatEvent::Done { message: *message }
                } else {
                    error_event(
                        assistant_message_id,
                        Some(*message),
                        error_code,
                        dgos_db::result_is_unknown(
                            outcome.message_status,
                            outcome.tokens,
                            outcome.task_error.as_deref(),
                        ),
                        discarded,
                    )
                }
            }
            Ok(Finalized::AlreadyFinalized) => {
                match self.repo.get_message(user, assistant_message_id).await {
                    Ok(message) if message.status == "completed" => ChatEvent::Done { message },
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

fn wire_request(messages: Vec<WireMessage>) -> ChatRequest {
    ChatRequest {
        messages,
        temperature: None,
        max_tokens: Some(OUTPUT_TOKENS),
        stream: Some(true),
        ..Default::default()
    }
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
    ChatEvent::Error {
        assistant_message_id: assistant_message_id.to_owned(),
        message,
        error_code,
        provider_code,
        result_unknown,
        regeneration_discarded,
    }
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
fn assemble(
    skill: Option<&dgos_skills::PreparedContent>,
    history: &History,
    current: &str,
    context_window: Option<i32>,
) -> Result<(Vec<WireMessage>, u32)> {
    assemble_with(HOST_POLICY, 0, skill, history, current, context_window)
}

/// Serialized size of the selected tool definitions plus a margin for the request's JSON
/// framing, counted against the budget so a tool turn does not trip the later whole-request
/// check (D075: tool round trips are not part of later context).
fn tool_reserve(tools: &[crate::tools::SelectedTool]) -> usize {
    let definitions: Vec<_> = tools.iter().map(|tool| &tool.definition).collect();
    serde_json::to_vec(&definitions).map_or(0, |bytes| bytes.len()) + 3 * 1024
}

fn assemble_with(
    policy: &str,
    reserve: usize,
    skill: Option<&dgos_skills::PreparedContent>,
    history: &History,
    current: &str,
    context_window: Option<i32>,
) -> Result<(Vec<WireMessage>, u32)> {
    if skill.is_some_and(|skill| skill.body.len() > MAX_MESSAGE_BYTES)
        || current.len() > MAX_MESSAGE_BYTES
    {
        return Err(ChatError::ContextTooLong);
    }
    let skill_message = skill.map(skill_guidance);
    let fixed =
        policy.len() + reserve + skill_message.as_ref().map_or(0, String::len) + current.len();
    // Same conservative rule as the single-task path: bytes plus a 1024 overhead and the output
    // reserve must fit a known window. Unknown windows fall back to the byte ceiling.
    let ceiling = match context_window {
        Some(window) if window <= 0 => return Err(ChatError::ContextTooLong),
        Some(window) => {
            let room = (window as usize).saturating_sub(1024 + OUTPUT_TOKENS as usize);
            MAX_CONTEXT_BYTES.min(room)
        }
        None => MAX_CONTEXT_BYTES,
    };
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
    for (question, reply) in turns.iter().rev() {
        let size = question.content.len() + reply.content.len();
        if used + size > budget {
            break;
        }
        used += size;
        kept += 1;
    }
    let included = &turns[turns.len() - kept..];
    let dropped = (history.total_completed_replies as usize).saturating_sub(included.len());

    let mut messages = Vec::with_capacity(2 + included.len() * 2 + 1);
    messages.push(WireMessage {
        role: "system".into(),
        content: policy.into(),
    });
    if let Some(guidance) = skill_message {
        messages.push(WireMessage {
            role: "user".into(),
            content: guidance,
        });
    }
    for (question, reply) in included {
        messages.push(WireMessage {
            role: "user".into(),
            content: question.content.clone(),
        });
        messages.push(WireMessage {
            role: "assistant".into(),
            content: reply.content.clone(),
        });
    }
    messages.push(WireMessage {
        role: "user".into(),
        content: current.to_owned(),
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
    let unsupported = || ChatError::Provider(DgosError::RequestNotSupported.error_code());
    if model.model_type != "text-generation" {
        return Err(unsupported());
    }
    if model
        .max_output_tokens
        .is_some_and(|max| max < OUTPUT_TOKENS as i32)
    {
        return Err(unsupported());
    }
    Ok((provider, model))
}

async fn resolve_skill(
    skills: &SkillsManager,
    user: &str,
    session: &ChatSession,
    provenance: &ExecutionProvenance,
) -> Result<Option<PreparedSkill>> {
    match (
        &session.skill_id,
        &session.skill_version_id,
        &session.skill_content_hash,
    ) {
        (Some(skill_id), Some(version_id), Some(hash)) => {
            let content = skills
                .prepare_content_skill_with_provenance(skill_id, user, Some(provenance))
                .await
                .map_err(|_| ChatError::SkillUnavailable)?;
            // Same version and bytes as pinned, or blocked: never a silent newer version.
            if format!("{}@{}", content.skill_id, content.version) != *version_id
                || content.content_hash != *hash
            {
                return Err(ChatError::SkillUnavailable);
            }
            if content.body.len() > MAX_MESSAGE_BYTES {
                return Err(ChatError::ContextTooLong);
            }
            Ok(Some(PreparedSkill {
                pin: PinnedSkill {
                    skill_id: skill_id.clone(),
                    version_id: version_id.clone(),
                    content_hash: hash.clone(),
                },
                content,
            }))
        }
        _ if session.no_skill_confirmed => Ok(None),
        // Neither a pinned version nor an explicit "no skill": the user has to choose.
        _ => Err(ChatError::SkillUnavailable),
    }
}

async fn resolve_target(
    providers: &ProviderManager,
    skills: &SkillsManager,
    user: &str,
    target: &ChatTargetInput,
) -> Result<ChatTarget> {
    if target.skill_id.is_some() && target.no_skill {
        return Err(ChatError::InvalidMessageContent);
    }
    match (&target.provider_id, &target.model_id) {
        (None, None) => {}
        (Some(provider), Some(model)) => {
            resolve_model(providers, user, Some(provider), Some(model)).await?;
        }
        _ => return Err(ChatError::ProviderNotSelected),
    }
    let skill = match &target.skill_id {
        Some(id) => {
            let content = skills
                .prepare_content_skill_with_provenance(id, user, None)
                .await
                .map_err(|_| ChatError::SkillUnavailable)?;
            Some(PinnedSkill {
                skill_id: content.skill_id.clone(),
                version_id: format!("{}@{}", content.skill_id, content.version),
                content_hash: content.content_hash.clone(),
            })
        }
        None => None,
    };
    Ok(ChatTarget {
        provider_id: target.provider_id.clone(),
        model_id: target.model_id.clone(),
        skill,
        no_skill: target.no_skill,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_db::HistoryRow;

    fn row(seq: i64, role: &str, content: &str) -> HistoryRow {
        HistoryRow {
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
        let (messages, dropped) = assemble(None, &h, "now", None).unwrap();
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
        let (messages, dropped) = assemble(None, &h, "now", None).unwrap();
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
        let (messages, dropped) = assemble(None, &h, "now", None).unwrap();
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
            assemble(None, &h, &too_long, None).unwrap_err(),
            ChatError::ContextTooLong
        );
        // A known small window leaves no room even for the host text.
        assert_eq!(
            assemble(None, &h, "hi", Some(2048)).unwrap_err(),
            ChatError::ContextTooLong
        );
        assert_eq!(
            assemble(None, &h, "hi", Some(0)).unwrap_err(),
            ChatError::ContextTooLong
        );
        assert!(assemble(None, &h, "hi", Some(32768)).is_ok());
    }

    #[test]
    fn assemble_with_the_whole_history_dropped_still_succeeds_when_the_current_message_fits() {
        let huge = "z".repeat(40 * 1024);
        let h = history(vec![row(1, "user", "q"), row(2, "assistant", &huge)], 1);
        let (messages, dropped) = assemble(None, &h, "now", None).unwrap();
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
        let manager = ChatManager::new(db);
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
        let manager = ChatManager::new(db);
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
        let manager = ChatManager::new(db);
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
    fn only_destination_changed_carries_detail() {
        let changed = ChatError::DestinationChanged {
            fingerprint: "fp".into(),
            host: "h".into(),
        };
        assert_eq!(changed.code(), 1006012);
        assert_eq!(changed.detail().unwrap()["host"], "h");
        assert!(ChatError::SessionNotFound.detail().is_none());
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
