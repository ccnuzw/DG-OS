//! `mcp.call` for runtime-bound modules (D093 item 6, "MCP 确认").
//!
//! A module never supplies or sees a confirmation token. Every module-initiated
//! call needs the user's approval in a host dialog (outside the module
//! boundary), whatever risk the tool declares. The order is fixed:
//!
//! 1. layers 1–3 (`authorize_bound`, read from the database on every call);
//! 2. a pending confirmation bound to (user, module, attachment, server, tool,
//!    exact arguments) is registered and pushed to the page attachment that
//!    made the call;
//! 3. the user answers, or the shared deadline, a lost page or a rejection
//!    ends the call with `1003019` and no external call;
//! 4. on approval the host mints the existing one-time token itself
//!    (`McpManager::confirm_call`) and calls the tool at once.
//!
//! The 120 s deadline is measured from admission and includes the wait for
//! the user. Arguments and results never reach a log, an audit row or an
//! error message.

use crate::CapabilityGrant;
use dgos_db::{sea_orm::DatabaseConnection, sea_orm::EntityTrait, Modules};
use dgos_mcp::{CallMcpToolRequest, McpError, McpManager};
use dgos_modules::{BoundModuleCall, ModuleRequestOutcome};
use dgos_types::{DgosError, DgosResult};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::oneshot;

/// Total host-owned deadline of one module `mcp.call`, user confirmation
/// included (D093 item 8).
pub const MCP_CALL_DEADLINE: Duration = Duration::from_secs(120);

/// Error code of a call the user did not approve: `McpError::ConfirmationRequired`.
const CONFIRMATION_REFUSED_CODE: u32 = 1003019;

/// Fixed text of every refusal; never mentions arguments or the reason.
const CONFIRMATION_REFUSED_MESSAGE: &str = "MCP tool call was not confirmed";

/// What the page is asked to show. Borrowed from the pending request, so the
/// dialog always displays exactly what will be sent.
#[derive(Clone, Copy, Debug)]
pub struct McpConfirmationPrompt<'a> {
    pub confirmation_id: &'a str,
    pub module_name: &'a str,
    pub server_id: &'a str,
    pub tool_name: &'a str,
    pub arguments: &'a Value,
}

/// Delivery of confirmation prompts to the page attachment of the call.
/// Implemented by the Tauri channel transport.
pub trait McpConfirmationNotifier: Send + Sync {
    /// Push the prompt to exactly `attachment_id`. `Err` means it could not be
    /// delivered (no such attachment, dead channel); the call is then refused.
    fn open(&self, attachment_id: &str, prompt: McpConfirmationPrompt<'_>) -> Result<(), ()>;
    /// Tell the page the prompt is over (answered, expired or abandoned).
    /// Best effort.
    fn close(&self, attachment_id: &str, confirmation_id: &str);
}

/// How a pending confirmation ended without an external call being made, or
/// with the user's approval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpVerdict {
    Approved,
    Rejected,
    /// The page instance or its channel went away.
    Lost,
    TimedOut,
}

struct Pending {
    user_id: String,
    attachment_id: String,
    answer: oneshot::Sender<bool>,
}

/// Registry of confirmations awaiting the user. Single use: an id answers
/// once, then it is gone.
pub struct ModuleMcpConfirmations {
    notifier: Arc<dyn McpConfirmationNotifier>,
    pending: Mutex<HashMap<String, Pending>>,
}

/// One call's binding. Everything the dialog shows and everything sent after
/// approval is taken from these same references.
pub struct McpConfirmationRequest<'a> {
    pub user_id: &'a str,
    pub attachment_id: &'a str,
    pub module_name: &'a str,
    pub server_id: &'a str,
    pub tool_name: &'a str,
    pub arguments: &'a Value,
}

/// Removes the pending entry and closes the prompt on every exit path,
/// including the asking future being dropped.
struct PendingGuard<'a> {
    broker: &'a ModuleMcpConfirmations,
    confirmation_id: String,
    attachment_id: String,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.broker.pending().remove(&self.confirmation_id);
        self.broker
            .notifier
            .close(&self.attachment_id, &self.confirmation_id);
    }
}

impl ModuleMcpConfirmations {
    pub fn new(notifier: Arc<dyn McpConfirmationNotifier>) -> Self {
        Self {
            notifier,
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn pending(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Register, push to the page and wait for the answer for at most `wait`.
    pub async fn ask(&self, request: McpConfirmationRequest<'_>, wait: Duration) -> McpVerdict {
        let confirmation_id = uuid::Uuid::new_v4().to_string();
        let (answer, answered) = oneshot::channel();
        // Registered before the prompt is pushed, so a fast answer or a page
        // loss cannot slip past it.
        self.pending().insert(
            confirmation_id.clone(),
            Pending {
                user_id: request.user_id.to_owned(),
                attachment_id: request.attachment_id.to_owned(),
                answer,
            },
        );
        let _guard = PendingGuard {
            broker: self,
            confirmation_id: confirmation_id.clone(),
            attachment_id: request.attachment_id.to_owned(),
        };
        let prompt = McpConfirmationPrompt {
            confirmation_id: &confirmation_id,
            module_name: request.module_name,
            server_id: request.server_id,
            tool_name: request.tool_name,
            arguments: request.arguments,
        };
        if self.notifier.open(request.attachment_id, prompt).is_err() {
            return McpVerdict::Lost;
        }
        match tokio::time::timeout(wait, answered).await {
            Ok(Ok(true)) => McpVerdict::Approved,
            Ok(Ok(false)) => McpVerdict::Rejected,
            // The sender was dropped: `attachment_lost` removed the entry.
            Ok(Err(_)) => McpVerdict::Lost,
            Err(_) => McpVerdict::TimedOut,
        }
    }

    /// The user's answer for `user_id`. An unknown, expired, already answered
    /// or foreign id is `PermissionDenied`. The id is consumed either way.
    ///
    /// The command that calls this can also be called by scripts sharing the
    /// page's realm (D050 residual risk): the dialog guards against a module
    /// calling without the user knowing, it is not isolation.
    pub fn resolve(&self, user_id: &str, confirmation_id: &str, approved: bool) -> DgosResult<()> {
        let pending = {
            let mut pending = self.pending();
            match pending.get(confirmation_id) {
                Some(entry) if entry.user_id == user_id => pending.remove(confirmation_id),
                _ => None,
            }
        };
        pending
            .ok_or(DgosError::PermissionDenied)?
            .answer
            .send(approved)
            .map_err(|_| DgosError::PermissionDenied)
    }

    /// The page instance behind `attachment_id` is gone: every confirmation
    /// asked of it can no longer be answered and ends as `Lost`.
    pub fn attachment_lost(&self, attachment_id: &str) {
        self.pending()
            .retain(|_, entry| entry.attachment_id != attachment_id);
    }

    /// FR-008 session end: every confirmation asked for `user_id` can no
    /// longer be answered by that user and ends as `Lost`.
    pub fn cancel_user(&self, user_id: &str) {
        self.pending().retain(|_, entry| entry.user_id != user_id);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pending().len()
    }
}

/// What a module receives from `mcp.callTool`: the tool's own result only.
#[derive(Clone, Debug, Serialize)]
pub struct ModuleMcpResult {
    pub is_error: bool,
    pub content: Value,
    pub structured_content: Option<Value>,
}

#[derive(Clone)]
pub struct ModuleMcpService {
    confirmations: Arc<ModuleMcpConfirmations>,
    mcp: Arc<McpManager>,
    db: DatabaseConnection,
    deadline: Duration,
}

impl ModuleMcpService {
    pub fn new(
        confirmations: Arc<ModuleMcpConfirmations>,
        mcp: Arc<McpManager>,
        db: DatabaseConnection,
    ) -> Self {
        Self {
            confirmations,
            mcp,
            db,
            deadline: MCP_CALL_DEADLINE,
        }
    }

    /// Override the total deadline (tests only; production keeps 120 s).
    #[cfg(test)]
    fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    pub fn confirmations(&self) -> &Arc<ModuleMcpConfirmations> {
        &self.confirmations
    }

    /// `host.mcp.callTool` for a call the channel admitted at `admitted_at`
    /// and `ModuleCapabilityDispatchService::authorize_bound` authorized for
    /// `ModulePermission::McpCall` (layers 1–3). The caller settles nothing:
    /// this method settles `call` exactly once. The total deadline runs from
    /// `admitted_at` and includes the wait for the user.
    pub async fn call_tool(
        &self,
        call: BoundModuleCall,
        grant: CapabilityGrant,
        admitted_at: Instant,
        server_id: String,
        tool_name: String,
        arguments: Value,
    ) -> DgosResult<ModuleMcpResult> {
        let started = admitted_at;
        let control = call.control();
        let module_name = self.module_name(&grant.module_id).await;
        // A cancel during the user's confirmation ends the call `Cancelled`:
        // dropping `ask` removes the pending entry and closes the prompt, and
        // no tool call was made (D113).
        let verdict = tokio::select! {
            biased;
            _ = control.cancelled() => None,
            verdict = self.confirmations.ask(
                McpConfirmationRequest {
                    user_id: &grant.user_id,
                    attachment_id: call.attachment_id(),
                    module_name: &module_name,
                    server_id: &server_id,
                    tool_name: &tool_name,
                    arguments: &arguments,
                },
                self.deadline.saturating_sub(started.elapsed()),
            ) => Some(verdict),
        };
        let Some(verdict) = verdict else {
            call.finish_cancelled();
            return Err(DgosError::ModuleCallCancelled);
        };
        if verdict != McpVerdict::Approved {
            // Refused by the user, expired or abandoned: not a permission
            // failure, so no permission audit row; the external call never ran.
            if !call.finish(ModuleRequestOutcome::Denied) {
                return Err(DgosError::ModuleCallCancelled);
            }
            return Err(DgosError::CapabilityFailed {
                code: CONFIRMATION_REFUSED_CODE,
                message: CONFIRMATION_REFUSED_MESSAGE.to_owned(),
            });
        }
        let remaining = self.deadline.saturating_sub(started.elapsed());
        let marker = control.clone();
        let invocation = async {
            // The tool call starts here; a cancel from now on only stops the
            // wait and the call settles `Interrupted` (D113).
            marker.mark_started();
            self.invoke(&grant, server_id, tool_name, arguments, remaining)
                .await
        };
        tokio::select! {
            biased;
            _ = control.cancelled() => {
                call.finish_cancelled();
                Err(DgosError::ModuleCallCancelled)
            }
            outcome = tokio::time::timeout(remaining, invocation) => match outcome {
                // A tool that answers `is_error` still completed the call.
                Ok(result) => crate::settle_bound(call, result),
                Err(_) => {
                    if call.finish(ModuleRequestOutcome::Interrupted) {
                        Err(DgosError::Timeout)
                    } else {
                        Err(DgosError::ModuleCallCancelled)
                    }
                }
            },
        }
    }

    /// Mint the one-time token and use it at once. The request is identical
    /// to the one the user approved; the token never leaves this function.
    async fn invoke(
        &self,
        grant: &CapabilityGrant,
        server_id: String,
        tool_name: String,
        arguments: Value,
        remaining: Duration,
    ) -> DgosResult<ModuleMcpResult> {
        let mut request = CallMcpToolRequest {
            server_id,
            tool_name,
            arguments,
            call_id: Some(uuid::Uuid::new_v4().to_string()),
            task_ref: None,
            // The host deadline decides; the manager's own timeout only
            // backs it up, a second later, so the outer one always wins.
            timeout_ms: Some(
                remaining
                    .saturating_add(Duration::from_secs(1))
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            ),
            confirmation_token: None,
        };
        request.confirmation_token = Some(
            self.mcp
                .confirm_call(&grant.user_id, &request)
                .await
                .map_err(mcp_failure)?,
        );
        let provenance = grant.provenance();
        let response = self
            .mcp
            .call_tool_with_provenance(&grant.user_id, request, &provenance)
            .await
            .map_err(mcp_failure)?;
        // A host-level failure inside the call (timeout, cancellation,
        // transport) is reported by the manager as an error object, not as the
        // tool's own content array. Keep its category, drop its text.
        if response.is_error && !response.content.is_array() {
            return Err(DgosError::CapabilityFailed {
                code: match response.outcome.as_str() {
                    "timeout" => 1003006,
                    "cancelled" => 1003016,
                    _ => 1003005,
                },
                message: "MCP tool call failed".to_owned(),
            });
        }
        Ok(ModuleMcpResult {
            is_error: response.is_error,
            content: response.content,
            structured_content: response.structured_content,
        })
    }

    async fn module_name(&self, module_id: &str) -> String {
        match Modules::find_by_id(module_id).one(&self.db).await {
            Ok(Some(module)) => module.display_name,
            _ => module_id.to_owned(),
        }
    }
}

/// Keeps the MCP error's own `1003xxx` code; the text is fixed because MCP
/// error texts can echo server data.
fn mcp_failure(error: McpError) -> DgosError {
    DgosError::CapabilityFailed {
        code: error.error_code(),
        message: "MCP tool call failed".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;

    #[derive(Debug, Eq, PartialEq)]
    struct Opened {
        attachment_id: String,
        confirmation_id: String,
        module_name: String,
        server_id: String,
        tool_name: String,
        arguments: Value,
    }

    struct Recorder {
        opened: mpsc::UnboundedSender<Opened>,
        closed: Mutex<Vec<(String, String)>>,
        deliverable: bool,
        opens: AtomicUsize,
    }

    impl Recorder {
        fn new(deliverable: bool) -> (Arc<Self>, mpsc::UnboundedReceiver<Opened>) {
            let (opened, receiver) = mpsc::unbounded_channel();
            (
                Arc::new(Self {
                    opened,
                    closed: Mutex::new(Vec::new()),
                    deliverable,
                    opens: AtomicUsize::new(0),
                }),
                receiver,
            )
        }

        fn closed(&self) -> Vec<(String, String)> {
            self.closed.lock().unwrap().clone()
        }
    }

    impl McpConfirmationNotifier for Recorder {
        fn open(&self, attachment_id: &str, prompt: McpConfirmationPrompt<'_>) -> Result<(), ()> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            if !self.deliverable {
                return Err(());
            }
            self.opened
                .send(Opened {
                    attachment_id: attachment_id.to_owned(),
                    confirmation_id: prompt.confirmation_id.to_owned(),
                    module_name: prompt.module_name.to_owned(),
                    server_id: prompt.server_id.to_owned(),
                    tool_name: prompt.tool_name.to_owned(),
                    arguments: prompt.arguments.clone(),
                })
                .map_err(|_| ())
        }

        fn close(&self, attachment_id: &str, confirmation_id: &str) {
            self.closed
                .lock()
                .unwrap()
                .push((attachment_id.to_owned(), confirmation_id.to_owned()));
        }
    }

    fn request<'a>(arguments: &'a Value) -> McpConfirmationRequest<'a> {
        McpConfirmationRequest {
            user_id: "alice",
            attachment_id: "attachment-1",
            module_name: "Example",
            server_id: "server-1",
            tool_name: "echo",
            arguments,
        }
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn approve_shows_exact_binding_and_closes_the_prompt() {
        let (recorder, mut opened) = Recorder::new(true);
        let broker = ModuleMcpConfirmations::new(recorder.clone());
        let arguments = json!({"text": "exact-value", "n": [1, 2]});
        let asking = broker.ask(request(&arguments), WAIT);
        let answering = async {
            let prompt = opened.recv().await.unwrap();
            assert_eq!(
                (
                    prompt.attachment_id.as_str(),
                    prompt.module_name.as_str(),
                    prompt.server_id.as_str(),
                    prompt.tool_name.as_str(),
                ),
                ("attachment-1", "Example", "server-1", "echo")
            );
            assert_eq!(prompt.arguments, arguments);
            broker
                .resolve("alice", &prompt.confirmation_id, true)
                .unwrap();
            prompt.confirmation_id
        };
        let (verdict, confirmation_id) = tokio::join!(asking, answering);
        assert_eq!(verdict, McpVerdict::Approved);
        assert_eq!(
            recorder.closed(),
            vec![("attachment-1".to_owned(), confirmation_id)]
        );
        assert_eq!(broker.len(), 0);
    }

    #[tokio::test]
    async fn reject_is_a_verdict_not_an_error() {
        let (recorder, mut opened) = Recorder::new(true);
        let broker = ModuleMcpConfirmations::new(recorder);
        let arguments = json!({});
        let (verdict, _) = tokio::join!(broker.ask(request(&arguments), WAIT), async {
            let prompt = opened.recv().await.unwrap();
            broker
                .resolve("alice", &prompt.confirmation_id, false)
                .unwrap();
        });
        assert_eq!(verdict, McpVerdict::Rejected);
    }

    #[tokio::test]
    async fn no_answer_within_the_wait_times_out_and_the_id_dies() {
        let (recorder, mut opened) = Recorder::new(true);
        let broker = ModuleMcpConfirmations::new(recorder.clone());
        let arguments = json!({});
        let verdict = broker
            .ask(request(&arguments), Duration::from_millis(50))
            .await;
        assert_eq!(verdict, McpVerdict::TimedOut);
        let prompt = opened.recv().await.unwrap();
        assert!(matches!(
            broker.resolve("alice", &prompt.confirmation_id, true),
            Err(DgosError::PermissionDenied)
        ));
        assert_eq!(recorder.closed().len(), 1);
        assert_eq!(broker.len(), 0);
    }

    #[tokio::test]
    async fn double_resolve_unknown_and_foreign_ids_are_permission_denied() {
        let (recorder, mut opened) = Recorder::new(true);
        let broker = ModuleMcpConfirmations::new(recorder);
        let arguments = json!({});
        let (verdict, _) = tokio::join!(broker.ask(request(&arguments), WAIT), async {
            let prompt = opened.recv().await.unwrap();
            // Another user cannot answer, and a failed attempt keeps it open.
            assert!(matches!(
                broker.resolve("mallory", &prompt.confirmation_id, true),
                Err(DgosError::PermissionDenied)
            ));
            assert!(matches!(
                broker.resolve("alice", "unknown", true),
                Err(DgosError::PermissionDenied)
            ));
            broker
                .resolve("alice", &prompt.confirmation_id, true)
                .unwrap();
            assert!(matches!(
                broker.resolve("alice", &prompt.confirmation_id, false),
                Err(DgosError::PermissionDenied)
            ));
        });
        assert_eq!(verdict, McpVerdict::Approved);
    }

    #[tokio::test]
    async fn a_lost_attachment_ends_only_its_own_confirmations() {
        let (recorder, mut opened) = Recorder::new(true);
        let broker = ModuleMcpConfirmations::new(recorder);
        let arguments = json!({});
        let other = McpConfirmationRequest {
            attachment_id: "attachment-2",
            ..request(&arguments)
        };
        let (lost, kept, _) = tokio::join!(
            broker.ask(request(&arguments), WAIT),
            broker.ask(other, WAIT),
            async {
                let first = opened.recv().await.unwrap();
                let second = opened.recv().await.unwrap();
                let (lost_id, kept_id) = if first.attachment_id == "attachment-1" {
                    (first.confirmation_id, second.confirmation_id)
                } else {
                    (second.confirmation_id, first.confirmation_id)
                };
                broker.attachment_lost("attachment-1");
                assert!(matches!(
                    broker.resolve("alice", &lost_id, true),
                    Err(DgosError::PermissionDenied)
                ));
                broker.resolve("alice", &kept_id, true).unwrap();
            }
        );
        assert_eq!(lost, McpVerdict::Lost);
        assert_eq!(kept, McpVerdict::Approved);
    }

    #[tokio::test]
    async fn an_undeliverable_prompt_is_lost_without_waiting() {
        let (recorder, _opened) = Recorder::new(false);
        let broker = ModuleMcpConfirmations::new(recorder.clone());
        let arguments = json!({});
        assert_eq!(
            broker.ask(request(&arguments), WAIT).await,
            McpVerdict::Lost
        );
        assert_eq!(recorder.opens.load(Ordering::SeqCst), 1);
        assert_eq!(broker.len(), 0);
    }

    #[tokio::test]
    async fn dropping_the_asking_future_removes_the_entry_and_closes_the_prompt() {
        let (recorder, mut opened) = Recorder::new(true);
        let broker = ModuleMcpConfirmations::new(recorder.clone());
        let arguments = json!({});
        {
            let asking = broker.ask(request(&arguments), WAIT);
            tokio::pin!(asking);
            tokio::select! {
                _ = &mut asking => panic!("nobody answered"),
                prompt = opened.recv() => assert!(prompt.is_some()),
            }
        }
        assert_eq!(broker.len(), 0);
        assert_eq!(recorder.closed().len(), 1);
    }
    /// Adapter tests against the real `McpManager` and its stdio fixture
    /// server (`crates/dgos-mcp/tests/fixtures/mcp-server.mjs`, needs
    /// `node`), real SQLite, a published module and a runtime loaded through
    /// `PageModuleLoader`. Only the WebView page is scripted.
    mod adapter {
        use super::*;
        use crate::{HostCapabilityService, ModuleCapabilityDispatchService, ModuleReleaseService};
        use dgos_db::sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
        use dgos_db::ModulePermission;
        use dgos_db::ModulePermissionRepository;
        use dgos_db::{migrations::Migrator, sea_orm_migration::MigratorTrait};
        use dgos_mcp::{CreateMcpServerRequest, McpCallPolicy, StdioConfig, TransportKind};
        use dgos_modules::{
            ModuleCallRouter, ModulePageTransport, ModulePublisher, ModuleRuntimeHandle,
            ModuleRuntimeSupervisor, PageInstruction, PageLoaderConfig, PageModuleLoader,
            PageReceipt, PageSendError, PinnedModuleAdmission, RuntimeLost,
        };
        use dgos_types::HostSession;
        use tempfile::{tempdir, TempDir};

        const MODULE: &str = "module-1";
        const USER: &str = "default-user";
        const SECRET: &str = "private-value-4e1c";

        struct Allow;

        #[async_trait::async_trait]
        impl McpCallPolicy for Allow {
            async fn allows_call(&self, user: &str, _: &str, _: &str) -> bool {
                user == USER
            }
        }

        struct ScriptedPage(mpsc::UnboundedSender<(String, PageInstruction)>);

        impl ModulePageTransport for ScriptedPage {
            fn send(
                &self,
                attachment_id: &str,
                instruction: PageInstruction,
            ) -> Result<(), PageSendError> {
                self.0
                    .send((attachment_id.to_owned(), instruction))
                    .map_err(|_| PageSendError)
            }
        }

        fn package() -> TempDir {
            let dir = tempdir().unwrap();
            std::fs::write(dir.path().join("entry.js"), "export default '1.0.0';").unwrap();
            let manifest = |sha256: &str| {
                json!({
                    "id": MODULE, "name": "Mcp Example", "version": "1.0.0", "entry": "entry.js",
                    "artifact": {"version": "1.0.0", "sha256": sha256},
                    "permissions": ["mcp.call"]
                })
                .to_string()
            };
            std::fs::write(dir.path().join("manifest.json"), manifest(&"0".repeat(64))).unwrap();
            let integrity = dgos_modules::package_integrity(dir.path()).unwrap();
            let hex = integrity.strip_prefix("v2:").unwrap();
            std::fs::write(dir.path().join("manifest.json"), manifest(hex)).unwrap();
            dir
        }

        enum Answer {
            Resolve(bool),
            /// The prompt appears and nobody answers.
            Unanswered,
            /// The page goes away while the prompt is open.
            PageLost,
            /// The grant is revoked while the prompt is open, then approved.
            RevokeThenApprove,
            /// The page cancels the call while the prompt is open.
            CancelWhileAsking,
            /// The user approves, the tool starts, then the page cancels.
            ApproveThenCancel,
            /// No prompt may appear.
            NoPrompt,
        }

        struct World {
            _db_dir: TempDir,
            _store: TempDir,
            db: DatabaseConnection,
            permissions: Arc<ModulePermissionRepository>,
            mcp: Arc<McpManager>,
            service: ModuleMcpService,
            dispatch: ModuleCapabilityDispatchService,
            handle: ModuleRuntimeHandle,
            router: ModuleCallRouter,
            route: String,
            attachment: String,
            server_id: String,
            opened: mpsc::UnboundedReceiver<Opened>,
            _supervisor: ModuleRuntimeSupervisor,
            _lost: mpsc::UnboundedReceiver<RuntimeLost>,
            driver: tokio::task::JoinHandle<()>,
        }

        impl Drop for World {
            fn drop(&mut self) {
                self.driver.abort();
            }
        }

        impl World {
            async fn new() -> Self {
                let db_dir = tempdir().unwrap();
                let url = format!(
                    "sqlite://{}?mode=rwc",
                    db_dir.path().join("fixture.db").display()
                );
                let db = dgos_db::establish_connection(&url).await.unwrap();
                Migrator::up(&db, None).await.unwrap();
                let store = tempdir().unwrap();
                let publisher = Arc::new(ModulePublisher::new(store.path(), db.clone()));
                let release = ModuleReleaseService::new(publisher.clone())
                    .publish(package().path())
                    .await
                    .unwrap();

                let (sender, mut instructions) = mpsc::unbounded_channel();
                let (loader, lost) = PageModuleLoader::new(
                    Arc::new(ScriptedPage(sender)),
                    PageLoaderConfig::default(),
                );
                let attachment = loader.attach("main").attachment_id;
                let router = ModuleCallRouter::new(loader.clone());
                let answering = loader.clone();
                let driver = tokio::spawn(async move {
                    while let Some((attachment, instruction)) = instructions.recv().await {
                        let (ticket, receipt) = match instruction {
                            PageInstruction::Load { ticket, .. } => (ticket, PageReceipt::Loaded),
                            PageInstruction::Unload { ticket, .. } => {
                                (ticket, PageReceipt::Unloaded { hook_failed: false })
                            }
                        };
                        answering.receive(&attachment, &ticket, receipt);
                    }
                });
                let supervisor = ModuleRuntimeSupervisor::new(publisher.clone(), Arc::new(loader))
                    .with_admission(Arc::new(PinnedModuleAdmission::new([(
                        MODULE,
                        "1.0.0",
                        release.integrity.as_str(),
                    )])));
                let handle = supervisor
                    .start(&HostSession::local_default(), MODULE)
                    .await
                    .unwrap();
                let (route, issued_for) = router.issue_with_attachment(&handle).unwrap();
                assert_eq!(issued_for, attachment);

                let permissions = Arc::new(ModulePermissionRepository::new(db.clone()));
                let dispatch = ModuleCapabilityDispatchService::new(
                    HostCapabilityService::with_publisher(permissions.clone(), publisher),
                );
                let mcp = Arc::new(McpManager::with_policy(db.clone(), Arc::new(Allow)));
                let server = mcp
                    .create_server(
                        USER,
                        CreateMcpServerRequest {
                            name: "fixture".into(),
                            transport: TransportKind::Stdio,
                            stdio: Some(StdioConfig {
                                executable: "node".into(),
                                args: vec![
                                    concat!(
                                        env!("CARGO_MANIFEST_DIR"),
                                        "/../dgos-mcp/tests/fixtures/mcp-server.mjs"
                                    )
                                    .into(),
                                    "normal".into(),
                                ],
                                cwd: None,
                            }),
                            http: None,
                            enabled: Some(true),
                            auto_connect: None,
                        },
                    )
                    .await
                    .unwrap();
                mcp.connect(USER, &server.id).await.unwrap();

                let (recorder, opened) = Recorder::new(true);
                let service = ModuleMcpService::new(
                    Arc::new(ModuleMcpConfirmations::new(recorder)),
                    mcp.clone(),
                    db.clone(),
                );
                Self {
                    _db_dir: db_dir,
                    _store: store,
                    db,
                    permissions,
                    mcp,
                    service,
                    dispatch,
                    handle,
                    router,
                    route: route.as_str().to_owned(),
                    attachment,
                    server_id: server.id,
                    opened,
                    _supervisor: supervisor,
                    _lost: lost,
                    driver,
                }
            }

            async fn grant(&self) {
                self.permissions
                    .grant(USER, MODULE, ModulePermission::McpCall)
                    .await
                    .unwrap();
            }

            async fn call(
                &mut self,
                tool: &str,
                arguments: Value,
                answer: Answer,
            ) -> (DgosResult<ModuleMcpResult>, Option<Opened>) {
                let call_id = uuid::Uuid::new_v4().to_string();
                let call = self
                    .router
                    .admit_once(&self.route, &self.attachment, &call_id)
                    .unwrap();
                let router = &self.router;
                let route = self.route.clone();
                let service = self.service.clone();
                let confirmations = service.confirmations().clone();
                let permissions = self.permissions.clone();
                let attachment = self.attachment.clone();
                let opened = &mut self.opened;
                let dispatch = self.dispatch.clone();
                let admitted_at = Instant::now();
                let server_id = self.server_id.clone();
                let tool = tool.to_owned();
                let asking = async move {
                    let (call, grant) = dispatch
                        .authorize_bound(call, ModulePermission::McpCall)
                        .await?;
                    service
                        .call_tool(call, grant, admitted_at, server_id, tool, arguments)
                        .await
                };
                let answering = async move {
                    if matches!(answer, Answer::NoPrompt) {
                        return None;
                    }
                    let prompt = opened.recv().await.unwrap();
                    match answer {
                        Answer::Resolve(approved) => {
                            confirmations
                                .resolve(USER, &prompt.confirmation_id, approved)
                                .unwrap();
                        }
                        Answer::Unanswered | Answer::NoPrompt => {}
                        Answer::PageLost => confirmations.attachment_lost(&attachment),
                        Answer::CancelWhileAsking => {
                            assert_eq!(router.cancel(&route, &attachment, &call_id), Ok(true));
                        }
                        Answer::ApproveThenCancel => {
                            confirmations
                                .resolve(USER, &prompt.confirmation_id, true)
                                .unwrap();
                            // The tool sleeps for seconds; give it time to start.
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            assert_eq!(router.cancel(&route, &attachment, &call_id), Ok(true));
                        }
                        Answer::RevokeThenApprove => {
                            permissions
                                .revoke(USER, MODULE, ModulePermission::McpCall)
                                .await
                                .unwrap();
                            confirmations
                                .resolve(USER, &prompt.confirmation_id, true)
                                .unwrap();
                        }
                    }
                    Some(prompt)
                };
                tokio::join!(asking, answering)
            }

            fn no_prompt_pending(&mut self) {
                assert!(self.opened.try_recv().is_err(), "a prompt was pushed");
            }

            async fn tool_calls(&self) -> usize {
                self.mcp
                    .list_usage_logs(USER, Some(&self.server_id), None)
                    .await
                    .unwrap()
                    .len()
            }

            async fn audit_rows(&self, action: &str) -> Vec<String> {
                self.db
                    .query_all(Statement::from_string(
                        self.db.get_database_backend(),
                        format!(
                            "SELECT request_id FROM module_permission_audits WHERE action = '{action}' AND module_id = '{MODULE}' AND permission = 'mcp.call'"
                        ),
                    ))
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| row.try_get::<Option<String>>("", "request_id").unwrap().unwrap())
                    .collect()
            }

            async fn audit_dump(&self) -> String {
                self.db
                    .query_all(Statement::from_string(
                        self.db.get_database_backend(),
                        "SELECT * FROM module_permission_audits",
                    ))
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| format!("{row:?}"))
                    .collect()
            }

            async fn finish(self) {
                self.mcp.shutdown().await.unwrap();
            }
        }

        fn echo() -> Value {
            json!({"text": SECRET})
        }

        #[tokio::test]
        async fn without_a_grant_the_call_is_denied_before_any_prompt() {
            let mut world = World::new().await;
            let (result, prompt) = world.call("echo", echo(), Answer::NoPrompt).await;
            assert!(prompt.is_none());
            assert!(matches!(result, Err(DgosError::PermissionDenied)));
            world.no_prompt_pending();
            assert_eq!(world.tool_calls().await, 0);
            let stats = world.handle.request_stats();
            assert_eq!((stats.admitted, stats.denied, stats.in_flight), (1, 1, 0));
            // One `denied` audit row with the host request id (AC04/AC05).
            assert_eq!(world.audit_rows("denied").await.len(), 1);
            assert!(!world.audit_dump().await.contains(SECRET));
            world.finish().await;
        }

        #[tokio::test]
        async fn approved_call_runs_the_tool_once_and_returns_only_the_result() {
            let mut world = World::new().await;
            world.grant().await;
            let (result, prompt) = world.call("echo", echo(), Answer::Resolve(true)).await;
            let prompt = prompt.unwrap();
            assert_eq!(
                (prompt.module_name.as_str(), prompt.tool_name.as_str()),
                ("Mcp Example", "echo")
            );
            assert_eq!(prompt.server_id, world.server_id);
            assert_eq!(prompt.arguments, echo());

            let result = result.unwrap();
            assert!(!result.is_error);
            assert_eq!(result.content[0]["text"], SECRET);
            let wire = serde_json::to_value(&result).unwrap();
            let mut keys: Vec<_> = wire.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            assert_eq!(keys, ["content", "is_error", "structured_content"]);

            let logs = world
                .mcp
                .list_usage_logs(USER, Some(&world.server_id), None)
                .await
                .unwrap();
            assert_eq!(logs.len(), 1);
            assert_eq!(logs[0].caller.as_deref(), Some("module"));
            let stats = world.handle.request_stats();
            assert_eq!((stats.succeeded, stats.in_flight), (1, 0));
            assert!(world.service.confirmations().len() == 0);
            world.finish().await;
        }

        #[tokio::test]
        async fn a_tool_reporting_an_error_is_still_a_result() {
            let mut world = World::new().await;
            world.grant().await;
            let (result, _) = world
                .call(
                    "echo",
                    json!({"text": "boom", "fail": true}),
                    Answer::Resolve(true),
                )
                .await;
            assert!(result.unwrap().is_error);
            assert_eq!(world.handle.request_stats().succeeded, 1);
            world.finish().await;
        }

        #[tokio::test]
        async fn rejection_never_reaches_the_tool() {
            let mut world = World::new().await;
            world.grant().await;
            let (result, _) = world.call("echo", echo(), Answer::Resolve(false)).await;
            assert!(matches!(
                result,
                Err(DgosError::CapabilityFailed { code: 1003019, .. })
            ));
            assert_eq!(world.tool_calls().await, 0);
            let stats = world.handle.request_stats();
            assert_eq!((stats.denied, stats.in_flight), (1, 0));
            world.finish().await;
        }

        #[tokio::test]
        async fn a_lost_page_ends_the_call_without_the_tool() {
            let mut world = World::new().await;
            world.grant().await;
            let (result, _) = world.call("echo", echo(), Answer::PageLost).await;
            assert!(matches!(
                result,
                Err(DgosError::CapabilityFailed { code: 1003019, .. })
            ));
            assert_eq!(world.tool_calls().await, 0);
            assert_eq!(world.handle.request_stats().denied, 1);
            world.finish().await;
        }

        #[tokio::test]
        async fn no_answer_by_the_deadline_is_refused_without_the_tool() {
            let mut world = World::new().await;
            world.grant().await;
            world.service = world
                .service
                .clone()
                .with_deadline(Duration::from_millis(200));
            let (result, prompt) = world.call("echo", echo(), Answer::Unanswered).await;
            assert!(matches!(
                result,
                Err(DgosError::CapabilityFailed { code: 1003019, .. })
            ));
            // The id died with the call.
            assert!(matches!(
                world
                    .service
                    .confirmations()
                    .resolve(USER, &prompt.unwrap().confirmation_id, true),
                Err(DgosError::PermissionDenied)
            ));
            assert_eq!(world.tool_calls().await, 0);
            assert_eq!(world.handle.request_stats().denied, 1);
            world.finish().await;
        }

        #[tokio::test]
        async fn deadline_expiring_after_approval_interrupts_the_call() {
            let mut world = World::new().await;
            world.grant().await;
            world.service = world.service.clone().with_deadline(Duration::from_secs(1));
            let (result, _) = world
                .call(
                    "echo",
                    json!({"text": "slow", "delay": 5000}),
                    Answer::Resolve(true),
                )
                .await;
            assert!(matches!(result, Err(DgosError::Timeout)));
            let stats = world.handle.request_stats();
            assert_eq!((stats.interrupted, stats.in_flight), (1, 0));
            world.finish().await;
        }

        #[tokio::test]
        async fn cancelling_during_the_confirmation_never_runs_the_tool_and_settles_cancelled() {
            let mut world = World::new().await;
            world.grant().await;
            let (result, prompt) = world.call("echo", echo(), Answer::CancelWhileAsking).await;
            assert!(matches!(result, Err(DgosError::ModuleCallCancelled)));
            assert_eq!(DgosError::ModuleCallCancelled.error_code(), 1004028);
            // The prompt was closed with the call: its id can no longer be answered.
            assert!(matches!(
                world
                    .service
                    .confirmations()
                    .resolve(USER, &prompt.unwrap().confirmation_id, true),
                Err(DgosError::PermissionDenied)
            ));
            assert_eq!(world.tool_calls().await, 0);
            let stats = world.handle.request_stats();
            assert_eq!(
                (
                    stats.admitted,
                    stats.cancelled,
                    stats.denied,
                    stats.interrupted
                ),
                (1, 1, 0, 0)
            );
            assert_eq!(stats.in_flight, 0);
            assert_eq!(
                stats.admitted,
                stats.in_flight as u64
                    + stats.succeeded
                    + stats.failed
                    + stats.denied
                    + stats.interrupted
                    + stats.cancelled
            );
            world.finish().await;
        }

        #[tokio::test]
        async fn cancelling_after_the_tool_started_stops_waiting_and_settles_interrupted() {
            let mut world = World::new().await;
            world.grant().await;
            let (result, _) = world
                .call(
                    "echo",
                    json!({"text": "slow", "delay": 5000}),
                    Answer::ApproveThenCancel,
                )
                .await;
            assert!(matches!(result, Err(DgosError::ModuleCallCancelled)));
            let stats = world.handle.request_stats();
            assert_eq!(
                (
                    stats.admitted,
                    stats.interrupted,
                    stats.cancelled,
                    stats.in_flight
                ),
                (1, 1, 0, 0)
            );
            world.finish().await;
        }

        #[tokio::test]
        async fn mcp_errors_keep_their_own_code_with_fixed_text() {
            let mut world = World::new().await;
            world.grant().await;
            let (result, _) = world
                .call("no_such_tool", echo(), Answer::Resolve(true))
                .await;
            let error = result.unwrap_err();
            assert!(matches!(
                &error,
                DgosError::CapabilityFailed { code: 1003004, message }
                    if message == "MCP tool call failed"
            ));
            assert!(!format!("{error:?}").contains(SECRET));
            assert_eq!(world.handle.request_stats().failed, 1);
            world.finish().await;
        }

        #[tokio::test]
        async fn revocation_affects_the_next_call_only() {
            let mut world = World::new().await;
            world.grant().await;
            // Revoked while the prompt is open: this call keeps the outcome
            // it already had (layers 1-3 were checked at its start).
            let (result, _) = world.call("echo", echo(), Answer::RevokeThenApprove).await;
            assert!(result.is_ok());
            assert_eq!(world.tool_calls().await, 1);

            let (next, prompt) = world.call("echo", echo(), Answer::NoPrompt).await;
            assert!(prompt.is_none());
            assert!(matches!(next, Err(DgosError::PermissionDenied)));
            world.no_prompt_pending();
            assert_eq!(world.tool_calls().await, 1);
            assert_eq!(world.audit_rows("denied").await.len(), 1);
            let stats = world.handle.request_stats();
            assert_eq!((stats.succeeded, stats.denied), (1, 1));
            world.finish().await;
        }
    }
}
