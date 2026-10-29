//! Loader that really runs module code in a host WebView page (D042-D044).
//!
//! This type owns the protocol state machine and nothing Tauri-specific: it
//! sends `PageInstruction`s through a `ModulePageTransport` and consumes
//! `PageReceipt`s. The adapter in `src-tauri` supplies the transport (a Tauri
//! `Channel` per attachment), the commands that deliver receipts, and the
//! page-load / window-destroy signals.
//!
//! Bindings, all host-generated:
//! - `attachment_id`: one per page attachment, bound to a window label and
//!   the page generation at that time;
//! - `ticket`: one per instruction, single use, mapped to a `runtime_id`
//!   inside the host. Receipts carry only `(attachment_id, ticket, result)`.
//!
//! Every method is cancel-safe. Dropping a `start`/`stop` future abandons its
//! ticket, so a later receipt is classified `abandoned` and never applied.

use crate::loader::{
    verified_entry_source, ModuleLoadError, ModuleLoader, PageFailureStage, VerifiedModuleArtifact,
};
use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// Settled tickets remembered to classify duplicate and late receipts.
const MAX_SETTLED_TICKETS: usize = 256;

/// Host to page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageInstruction {
    Load {
        ticket: String,
        runtime_id: String,
        module_id: String,
        version: String,
        source: String,
        hook_timeout_ms: u64,
        /// Runtime id of the loaded instance of the same module this load
        /// replaces, so the page can hand state over (D055).
        replaces: Option<String>,
    },
    Unload {
        ticket: String,
        runtime_id: String,
        hook_timeout_ms: u64,
    },
}

/// Page to host. Coarse results only; never messages, stacks or module text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageReceipt {
    Loaded,
    LoadFailed(PageFailureStage),
    Unloaded { hook_failed: bool },
}

/// What the host did with a receipt. Only `Applied` changes state (D044).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptDisposition {
    Applied,
    /// The ticket was already settled by an earlier receipt.
    Duplicate,
    /// The attachment was replaced, reloaded or destroyed.
    Stale,
    /// The waiting call timed out or was cancelled before this receipt.
    Abandoned,
    /// The host never issued this ticket, or it does not fit the ticket.
    Unknown,
}

/// The instruction could not be delivered: the page channel is gone.
#[derive(Debug, Eq, PartialEq)]
pub struct PageSendError;

/// Delivery port. `send` must not block and must not call back into the
/// loader.
pub trait ModulePageTransport: Send + Sync {
    fn send(&self, attachment_id: &str, instruction: PageInstruction) -> Result<(), PageSendError>;
}

/// A loaded runtime whose page went away. The Application layer reports it
/// to the supervisor, which marks the runtime `failed` (D044).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeLost {
    pub module_id: String,
    pub runtime_id: String,
}

#[derive(Clone, Debug)]
pub struct PageLoaderConfig {
    /// The only window module code runs in.
    pub window_label: String,
    /// Page-side budget for `onLoad` / `onUnload`.
    pub hook_timeout: Duration,
}

impl Default for PageLoaderConfig {
    fn default() -> Self {
        Self {
            window_label: "main".into(),
            hook_timeout: Duration::from_secs(5),
        }
    }
}

/// Result of a new page attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    pub attachment_id: String,
    /// The attachment this one replaced, if any. The transport can drop its
    /// channel.
    pub superseded: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    Load,
    Unload,
}

enum Outcome {
    Receipt(PageReceipt),
    Lost,
}

struct Pending {
    attachment_id: String,
    runtime_id: String,
    kind: Kind,
    tx: oneshot::Sender<Outcome>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Status {
    Loading,
    Loaded,
}

struct Instance {
    module_id: String,
    attachment_id: String,
    status: Status,
}

#[derive(Clone, Copy)]
enum Settled {
    Done,
    Abandoned,
    Superseded,
}

#[derive(Default)]
struct WindowState {
    generation: u64,
    attachment: Option<String>,
}

struct AttachmentInfo {
    window: String,
    generation: u64,
}

#[derive(Default)]
struct State {
    windows: HashMap<String, WindowState>,
    attachments: HashMap<String, AttachmentInfo>,
    pending: HashMap<String, Pending>,
    instances: HashMap<String, Instance>,
    settled: HashMap<String, Settled>,
    settled_order: VecDeque<String>,
}

impl State {
    fn remember(&mut self, ticket: &str, how: Settled) {
        if self.settled.insert(ticket.to_owned(), how).is_none() {
            self.settled_order.push_back(ticket.to_owned());
        }
        while self.settled_order.len() > MAX_SETTLED_TICKETS {
            if let Some(oldest) = self.settled_order.pop_front() {
                self.settled.remove(&oldest);
            }
        }
    }

    /// Drop an attachment: fail its waiting calls, and report loaded
    /// instances on it as lost. Instances still `Loading` stay until their
    /// `stop`, which finds the attachment gone and releases them.
    fn invalidate(&mut self, attachment_id: &str) -> Vec<RuntimeLost> {
        self.attachments.remove(attachment_id);
        let tickets: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.attachment_id == attachment_id)
            .map(|(ticket, _)| ticket.clone())
            .collect();
        for ticket in tickets {
            if let Some(pending) = self.pending.remove(&ticket) {
                self.remember(&ticket, Settled::Superseded);
                let _ = pending.tx.send(Outcome::Lost);
            }
        }
        let lost: Vec<String> = self
            .instances
            .iter()
            .filter(|(_, i)| i.attachment_id == attachment_id && i.status == Status::Loaded)
            .map(|(runtime_id, _)| runtime_id.clone())
            .collect();
        lost.into_iter()
            .filter_map(|runtime_id| {
                self.instances
                    .remove(&runtime_id)
                    .map(|instance| RuntimeLost {
                        module_id: instance.module_id,
                        runtime_id,
                    })
            })
            .collect()
    }
}

struct Shared {
    transport: Arc<dyn ModulePageTransport>,
    config: PageLoaderConfig,
    lost: mpsc::UnboundedSender<RuntimeLost>,
    state: Mutex<State>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn publish(&self, lost: Vec<RuntimeLost>) {
        for event in lost {
            // The receiver only disappears at shutdown.
            let _ = self.lost.send(event);
        }
    }
}

/// Abandons a ticket when the waiting call is dropped before the receipt.
struct TicketGuard {
    shared: Arc<Shared>,
    ticket: String,
}

impl Drop for TicketGuard {
    fn drop(&mut self) {
        let mut state = self.shared.state();
        if state.pending.remove(&self.ticket).is_some() {
            state.remember(&self.ticket, Settled::Abandoned);
        }
    }
}

#[derive(Clone)]
pub struct PageModuleLoader {
    shared: Arc<Shared>,
}

impl PageModuleLoader {
    pub fn new(
        transport: Arc<dyn ModulePageTransport>,
        config: PageLoaderConfig,
    ) -> (Self, mpsc::UnboundedReceiver<RuntimeLost>) {
        let (lost, events) = mpsc::unbounded_channel();
        let loader = Self {
            shared: Arc::new(Shared {
                transport,
                config,
                lost,
                state: Mutex::new(State::default()),
            }),
        };
        (loader, events)
    }

    /// A page attached to `window`. Any previous attachment of the window is
    /// superseded: its waiting calls fail with `PageLost` and its loaded
    /// instances are reported lost. A page that attaches twice in one realm
    /// therefore loses its instances; the frontend attaches once.
    pub fn attach(&self, window: &str) -> Attachment {
        let (attachment, lost) = {
            let mut guard = self.shared.state();
            let state = &mut *guard;
            let entry = state.windows.entry(window.to_owned()).or_default();
            let superseded = entry.attachment.take();
            let attachment_id = Uuid::new_v4().to_string();
            entry.attachment = Some(attachment_id.clone());
            let generation = entry.generation;
            state.attachments.insert(
                attachment_id.clone(),
                AttachmentInfo {
                    window: window.to_owned(),
                    generation,
                },
            );
            let lost = superseded
                .as_deref()
                .map(|old| state.invalidate(old))
                .unwrap_or_default();
            (
                Attachment {
                    attachment_id,
                    superseded,
                },
                lost,
            )
        };
        self.shared.publish(lost);
        attachment
    }

    /// A full page navigation or reload of `window` started: the previous
    /// JS realm is going away. SPA route changes never call this.
    pub fn page_started(&self, window: &str) {
        let lost = {
            let mut guard = self.shared.state();
            let state = &mut *guard;
            let entry = state.windows.entry(window.to_owned()).or_default();
            entry.generation += 1;
            match entry.attachment.take() {
                Some(attachment) => state.invalidate(&attachment),
                None => Vec::new(),
            }
        };
        self.shared.publish(lost);
    }

    pub fn window_destroyed(&self, window: &str) {
        let lost = {
            let mut guard = self.shared.state();
            let state = &mut *guard;
            match state.windows.remove(window).and_then(|w| w.attachment) {
                Some(attachment) => state.invalidate(&attachment),
                None => Vec::new(),
            }
        };
        self.shared.publish(lost);
    }

    /// Classify and apply a receipt from a page (D044).
    pub fn receive(
        &self,
        attachment_id: &str,
        ticket: &str,
        receipt: PageReceipt,
    ) -> ReceiptDisposition {
        let mut guard = self.shared.state();
        let state = &mut *guard;
        if let Some(pending) = state.pending.get(ticket) {
            if pending.attachment_id != attachment_id {
                return if state.attachments.contains_key(attachment_id) {
                    ReceiptDisposition::Unknown
                } else {
                    ReceiptDisposition::Stale
                };
            }
            let fits = matches!(
                (pending.kind, receipt),
                (Kind::Load, PageReceipt::Loaded | PageReceipt::LoadFailed(_))
                    | (Kind::Unload, PageReceipt::Unloaded { .. })
            );
            if !fits {
                return ReceiptDisposition::Unknown;
            }
            let Some(pending) = state.pending.remove(ticket) else {
                return ReceiptDisposition::Unknown;
            };
            match receipt {
                PageReceipt::Loaded => {
                    if let Some(instance) = state.instances.get_mut(&pending.runtime_id) {
                        instance.status = Status::Loaded;
                    }
                }
                PageReceipt::LoadFailed(_) | PageReceipt::Unloaded { .. } => {
                    state.instances.remove(&pending.runtime_id);
                }
            }
            state.remember(ticket, Settled::Done);
            let _ = pending.tx.send(Outcome::Receipt(receipt));
            return ReceiptDisposition::Applied;
        }
        match state.settled.get(ticket) {
            Some(Settled::Done) => ReceiptDisposition::Duplicate,
            Some(Settled::Abandoned) => ReceiptDisposition::Abandoned,
            Some(Settled::Superseded) => ReceiptDisposition::Stale,
            None => ReceiptDisposition::Unknown,
        }
    }

    /// Whether `window` currently has a live page attachment.
    pub fn has_attachment(&self, window: &str) -> bool {
        self.shared
            .state()
            .windows
            .get(window)
            .is_some_and(|window| window.attachment.is_some())
    }

    /// Whether `attachment_id` is the live attachment of its window.
    pub fn is_current(&self, attachment_id: &str) -> bool {
        let state = self.shared.state();
        state.attachments.get(attachment_id).is_some_and(|info| {
            state
                .windows
                .get(&info.window)
                .is_some_and(|window| window.attachment.as_deref() == Some(attachment_id))
        })
    }

    /// Page generation of `window` (0 before any page load was observed).
    pub fn generation(&self, window: &str) -> u64 {
        self.shared
            .state()
            .windows
            .get(window)
            .map_or(0, |window| window.generation)
    }

    /// Generation the attachment was created in.
    pub fn attachment_generation(&self, attachment_id: &str) -> Option<u64> {
        self.shared
            .state()
            .attachments
            .get(attachment_id)
            .map(|info| info.generation)
    }

    /// `(attachment_id, generation)` of a runtime that finished loading, only
    /// while that attachment is still its window's live one. `None` for
    /// unknown, still-loading, lost or superseded instances.
    pub fn loaded_binding(&self, runtime_id: &str) -> Option<(String, u64)> {
        let state = self.shared.state();
        let instance = state.instances.get(runtime_id)?;
        if instance.status != Status::Loaded {
            return None;
        }
        let info = state.attachments.get(&instance.attachment_id)?;
        let live = state.windows.get(&info.window).is_some_and(|window| {
            window.attachment.as_deref() == Some(instance.attachment_id.as_str())
        });
        live.then(|| (instance.attachment_id.clone(), info.generation))
    }

    /// Instances the loader currently tracks (loading or loaded).
    pub fn tracked_instances(&self) -> usize {
        self.shared.state().instances.len()
    }

    fn hook_timeout_ms(&self) -> u64 {
        u64::try_from(self.shared.config.hook_timeout.as_millis()).unwrap_or(u64::MAX)
    }
}

// The entry source comes from the bytes the publisher verified (D057); the
// disk is not consulted, so a file changed after verification cannot be sent.
fn read_entry(artifact: &VerifiedModuleArtifact) -> Result<String, ModuleLoadError> {
    verified_entry_source(artifact)
}

#[async_trait]
impl ModuleLoader for PageModuleLoader {
    fn executes_module_code(&self) -> bool {
        true
    }

    /// Host-side only: nothing is sent to a page.
    async fn smoke(&self, artifact: &VerifiedModuleArtifact) -> Result<(), ModuleLoadError> {
        read_entry(artifact).map(drop)
    }

    async fn start(
        &self,
        runtime_id: &str,
        artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError> {
        let source = read_entry(artifact)?;
        let ticket = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        let (attachment_id, replaces) = {
            let mut guard = self.shared.state();
            let state = &mut *guard;
            let attachment_id = state
                .windows
                .get(&self.shared.config.window_label)
                .and_then(|window| window.attachment.clone())
                .ok_or(ModuleLoadError::HostNotAttached)?;
            if state.instances.contains_key(runtime_id) {
                return Err(ModuleLoadError::Rejected(
                    "runtime is already loaded".into(),
                ));
            }
            // The one live instance of this module in the same page, if any: a
            // hot update replaces it (D055). More than one is not a hand-over.
            let mut live = state.instances.iter().filter(|(_, other)| {
                other.module_id == artifact.module_id()
                    && other.attachment_id == attachment_id
                    && other.status == Status::Loaded
            });
            let replaces = match (live.next(), live.next()) {
                (Some((id, _)), None) => Some(id.clone()),
                _ => None,
            };
            state.instances.insert(
                runtime_id.to_owned(),
                Instance {
                    module_id: artifact.module_id().to_owned(),
                    attachment_id: attachment_id.clone(),
                    status: Status::Loading,
                },
            );
            state.pending.insert(
                ticket.clone(),
                Pending {
                    attachment_id: attachment_id.clone(),
                    runtime_id: runtime_id.to_owned(),
                    kind: Kind::Load,
                    tx,
                },
            );
            (attachment_id, replaces)
        };
        let _guard = TicketGuard {
            shared: self.shared.clone(),
            ticket: ticket.clone(),
        };
        let sent = self.shared.transport.send(
            &attachment_id,
            PageInstruction::Load {
                ticket: ticket.clone(),
                runtime_id: runtime_id.to_owned(),
                module_id: artifact.module_id().to_owned(),
                version: artifact.version().to_owned(),
                source,
                hook_timeout_ms: self.hook_timeout_ms(),
                replaces,
            },
        );
        if sent.is_err() {
            let mut state = self.shared.state();
            state.pending.remove(&ticket);
            state.instances.remove(runtime_id);
            return Err(ModuleLoadError::PageLost);
        }
        match rx.await {
            Ok(Outcome::Receipt(PageReceipt::Loaded)) => {}
            Ok(Outcome::Receipt(PageReceipt::LoadFailed(stage))) => {
                return Err(ModuleLoadError::PageRejected(stage));
            }
            Ok(Outcome::Receipt(PageReceipt::Unloaded { .. })) => {
                return Err(ModuleLoadError::Rejected("unexpected receipt".into()));
            }
            Ok(Outcome::Lost) | Err(_) => return Err(ModuleLoadError::PageLost),
        }
        // The page may have been replaced between the receipt and now.
        let state = self.shared.state();
        match state.instances.get(runtime_id) {
            Some(instance)
                if instance.status == Status::Loaded
                    && state.attachments.contains_key(&instance.attachment_id) =>
            {
                Ok(())
            }
            _ => Err(ModuleLoadError::PageLost),
        }
    }

    async fn stop(
        &self,
        runtime_id: &str,
        _artifact: &VerifiedModuleArtifact,
    ) -> Result<(), ModuleLoadError> {
        let ticket = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        let attachment_id = {
            let mut guard = self.shared.state();
            let state = &mut *guard;
            let Some(instance) = state.instances.get(runtime_id) else {
                // Never loaded here, already unloaded, or reported lost.
                return Ok(());
            };
            let attachment_id = instance.attachment_id.clone();
            if !state.attachments.contains_key(&attachment_id) {
                // The page realm is gone, so nothing is left to unload.
                state.instances.remove(runtime_id);
                return Ok(());
            }
            state.pending.insert(
                ticket.clone(),
                Pending {
                    attachment_id: attachment_id.clone(),
                    runtime_id: runtime_id.to_owned(),
                    kind: Kind::Unload,
                    tx,
                },
            );
            attachment_id
        };
        let _guard = TicketGuard {
            shared: self.shared.clone(),
            ticket: ticket.clone(),
        };
        let sent = self.shared.transport.send(
            &attachment_id,
            PageInstruction::Unload {
                ticket: ticket.clone(),
                runtime_id: runtime_id.to_owned(),
                hook_timeout_ms: self.hook_timeout_ms(),
            },
        );
        if sent.is_err() {
            self.shared.state().pending.remove(&ticket);
            // Release is unconfirmed: keep the instance so `stop` retries.
            return Err(ModuleLoadError::PageLost);
        }
        match rx.await {
            Ok(Outcome::Receipt(PageReceipt::Unloaded { hook_failed })) => {
                if hook_failed {
                    tracing::warn!(runtime_id, "module onUnload hook failed; instance disposed");
                }
                Ok(())
            }
            Ok(Outcome::Receipt(_)) => Err(ModuleLoadError::Rejected("unexpected receipt".into())),
            Ok(Outcome::Lost) | Err(_) => {
                self.shared.state().instances.remove(runtime_id);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    #[derive(Default)]
    struct FakeTransport {
        sent: Mutex<Vec<(String, PageInstruction)>>,
        fail: Mutex<bool>,
    }

    impl FakeTransport {
        fn fail_sends(&self, fail: bool) {
            *self.fail.lock().unwrap() = fail;
        }

        fn count(&self) -> usize {
            self.sent.lock().unwrap().len()
        }

        async fn nth(&self, index: usize) -> (String, PageInstruction) {
            for _ in 0..400 {
                if let Some(item) = self.sent.lock().unwrap().get(index).cloned() {
                    return item;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("instruction {index} was never sent");
        }
    }

    impl ModulePageTransport for FakeTransport {
        fn send(
            &self,
            attachment: &str,
            instruction: PageInstruction,
        ) -> Result<(), PageSendError> {
            if *self.fail.lock().unwrap() {
                return Err(PageSendError);
            }
            self.sent
                .lock()
                .unwrap()
                .push((attachment.to_owned(), instruction));
            Ok(())
        }
    }

    struct Fixture {
        _dir: TempDir,
        artifact: VerifiedModuleArtifact,
        transport: Arc<FakeTransport>,
        loader: PageModuleLoader,
        lost: mpsc::UnboundedReceiver<RuntimeLost>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("entry.js"), "export default () => ({});").unwrap();
        let artifact = VerifiedModuleArtifact::new(
            "mod-a".into(),
            "1.0.0".into(),
            format!("v2:{}", "0".repeat(64)),
            std::sync::Arc::from(&b"export default () => ({});"[..]),
        );
        let transport = Arc::new(FakeTransport::default());
        let (loader, lost) = PageModuleLoader::new(transport.clone(), PageLoaderConfig::default());
        Fixture {
            _dir: dir,
            artifact,
            transport,
            loader,
            lost,
        }
    }

    fn ticket(instruction: &PageInstruction) -> String {
        match instruction {
            PageInstruction::Load { ticket, .. } | PageInstruction::Unload { ticket, .. } => {
                ticket.clone()
            }
        }
    }

    /// Start a load and answer it with `receipt`; returns the start result.
    async fn load_with(
        f: &Fixture,
        runtime: &str,
        index: usize,
        attachment: &str,
        receipt: PageReceipt,
    ) -> Result<(), ModuleLoadError> {
        let loader = f.loader.clone();
        let artifact = f.artifact.clone();
        let runtime_id = runtime.to_owned();
        let task = tokio::spawn(async move { loader.start(&runtime_id, &artifact).await });
        let (sent_to, instruction) = f.transport.nth(index).await;
        assert_eq!(sent_to, attachment);
        assert_eq!(
            f.loader.receive(attachment, &ticket(&instruction), receipt),
            ReceiptDisposition::Applied
        );
        task.await.unwrap()
    }

    #[tokio::test]
    async fn load_and_stop_use_real_instructions_and_confirm_by_receipt() {
        let f = fixture();
        assert!(f.loader.executes_module_code());
        let attachment = f.loader.attach("main").attachment_id;

        load_with(&f, "rt-1", 0, &attachment, PageReceipt::Loaded)
            .await
            .unwrap();
        match &f.transport.nth(0).await.1 {
            PageInstruction::Load {
                runtime_id,
                module_id,
                version,
                source,
                hook_timeout_ms,
                ..
            } => {
                assert_eq!(
                    (runtime_id.as_str(), module_id.as_str(), version.as_str()),
                    ("rt-1", "mod-a", "1.0.0")
                );
                assert_eq!(source, "export default () => ({});");
                assert_eq!(*hook_timeout_ms, 5000);
            }
            other => panic!("expected load, got {other:?}"),
        }
        assert_eq!(f.loader.tracked_instances(), 1);

        let loader = f.loader.clone();
        let artifact = f.artifact.clone();
        let stop = tokio::spawn(async move { loader.stop("rt-1", &artifact).await });
        let (_, unload) = f.transport.nth(1).await;
        assert!(
            matches!(unload, PageInstruction::Unload { ref runtime_id, .. } if runtime_id == "rt-1")
        );
        assert_eq!(
            f.loader.receive(
                &attachment,
                &ticket(&unload),
                PageReceipt::Unloaded { hook_failed: true }
            ),
            ReceiptDisposition::Applied
        );
        // A failing onUnload hook still counts as released (D044).
        stop.await.unwrap().unwrap();
        assert_eq!(f.loader.tracked_instances(), 0);

        // Released: another stop reaches no page.
        f.loader.stop("rt-1", &f.artifact).await.unwrap();
        assert_eq!(f.transport.count(), 2);
    }

    #[tokio::test]
    async fn start_without_an_attached_page_fails_closed() {
        let f = fixture();
        let error = f.loader.start("rt-1", &f.artifact).await.unwrap_err();
        assert!(matches!(error, ModuleLoadError::HostNotAttached));
        assert_eq!(f.transport.count(), 0);
        assert_eq!(f.loader.tracked_instances(), 0);
    }

    #[tokio::test]
    async fn smoke_reads_the_host_side_only() {
        let f = fixture();
        f.loader.smoke(&f.artifact).await.unwrap();
        assert_eq!(f.transport.count(), 0);
    }

    #[tokio::test]
    async fn page_rejection_leaves_nothing_to_release() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let error = load_with(
            &f,
            "rt-1",
            0,
            &attachment,
            PageReceipt::LoadFailed(PageFailureStage::Render),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            ModuleLoadError::PageRejected(PageFailureStage::Render)
        ));
        assert_eq!(f.loader.tracked_instances(), 0);
        f.loader.stop("rt-1", &f.artifact).await.unwrap();
        assert_eq!(f.transport.count(), 1);
    }

    #[tokio::test]
    async fn late_receipt_after_cancellation_is_abandoned_and_stop_still_unloads() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;

        // The supervisor drops the loader future at its deadline.
        let timed_out = tokio::time::timeout(
            Duration::from_millis(30),
            f.loader.start("rt-1", &f.artifact),
        )
        .await;
        assert!(timed_out.is_err());
        let (_, load) = f.transport.nth(0).await;

        // The page finishes late: never applied, never ready.
        assert_eq!(
            f.loader
                .receive(&attachment, &ticket(&load), PageReceipt::Loaded),
            ReceiptDisposition::Abandoned
        );
        assert_eq!(
            f.loader.receive(
                &attachment,
                &ticket(&load),
                PageReceipt::LoadFailed(PageFailureStage::Hook)
            ),
            ReceiptDisposition::Abandoned
        );

        // Cleanup after the failed start still reaches the page, because the
        // host cannot know whether the page finished the load.
        let loader = f.loader.clone();
        let artifact = f.artifact.clone();
        let stop = tokio::spawn(async move { loader.stop("rt-1", &artifact).await });
        let (_, unload) = f.transport.nth(1).await;
        assert!(matches!(unload, PageInstruction::Unload { .. }));
        f.loader.receive(
            &attachment,
            &ticket(&unload),
            PageReceipt::Unloaded { hook_failed: false },
        );
        stop.await.unwrap().unwrap();
        assert_eq!(f.loader.tracked_instances(), 0);
    }

    #[tokio::test]
    async fn duplicate_unknown_and_mismatched_receipts_change_nothing() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        let loader = f.loader.clone();
        let artifact = f.artifact.clone();
        let start = tokio::spawn(async move { loader.start("rt-1", &artifact).await });
        let (_, load) = f.transport.nth(0).await;
        let ticket = ticket(&load);

        assert_eq!(
            f.loader
                .receive(&attachment, "no-such-ticket", PageReceipt::Loaded),
            ReceiptDisposition::Unknown
        );
        // An unload receipt cannot answer a load ticket, which stays pending.
        assert_eq!(
            f.loader.receive(
                &attachment,
                &ticket,
                PageReceipt::Unloaded { hook_failed: false }
            ),
            ReceiptDisposition::Unknown
        );
        // Another live attachment cannot answer it either.
        let other = f.loader.attach("secondary").attachment_id;
        assert_eq!(
            f.loader.receive(&other, &ticket, PageReceipt::Loaded),
            ReceiptDisposition::Unknown
        );
        assert!(!start.is_finished());

        assert_eq!(
            f.loader.receive(&attachment, &ticket, PageReceipt::Loaded),
            ReceiptDisposition::Applied
        );
        start.await.unwrap().unwrap();
        assert_eq!(
            f.loader.receive(&attachment, &ticket, PageReceipt::Loaded),
            ReceiptDisposition::Duplicate
        );
        // A duplicate failure receipt cannot undo a loaded instance.
        assert_eq!(
            f.loader.receive(
                &attachment,
                &ticket,
                PageReceipt::LoadFailed(PageFailureStage::Hook)
            ),
            ReceiptDisposition::Duplicate
        );
        assert_eq!(f.loader.tracked_instances(), 1);
    }

    #[tokio::test]
    async fn reload_during_load_fails_it_and_a_replayed_receipt_is_stale() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        assert_eq!(f.loader.attachment_generation(&attachment), Some(0));
        let loader = f.loader.clone();
        let artifact = f.artifact.clone();
        let start = tokio::spawn(async move { loader.start("rt-1", &artifact).await });
        let (_, load) = f.transport.nth(0).await;

        f.loader.page_started("main");
        assert_eq!(f.loader.generation("main"), 1);
        assert!(!f.loader.is_current(&attachment));
        assert!(matches!(
            start.await.unwrap(),
            Err(ModuleLoadError::PageLost)
        ));
        assert_eq!(
            f.loader
                .receive(&attachment, &ticket(&load), PageReceipt::Loaded),
            ReceiptDisposition::Stale
        );

        // Cleanup of the failed start: the old realm is gone, nothing to send.
        f.loader.stop("rt-1", &f.artifact).await.unwrap();
        assert_eq!(f.transport.count(), 1);

        // The new page attaches in the new generation and loads normally.
        let next = f.loader.attach("main").attachment_id;
        assert_eq!(f.loader.attachment_generation(&next), Some(1));
        load_with(&f, "rt-2", 1, &next, PageReceipt::Loaded)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn loaded_instance_is_reported_lost_when_its_page_goes_away() {
        let mut f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        load_with(&f, "rt-1", 0, &attachment, PageReceipt::Loaded)
            .await
            .unwrap();

        f.loader.page_started("main");
        let event = f.lost.try_recv().unwrap();
        assert_eq!(
            event,
            RuntimeLost {
                module_id: "mod-a".into(),
                runtime_id: "rt-1".into()
            }
        );
        assert_eq!(f.loader.tracked_instances(), 0);
        // The failed runtime's release has nothing left to unload.
        f.loader.stop("rt-1", &f.artifact).await.unwrap();
        assert_eq!(f.transport.count(), 1);
    }

    #[tokio::test]
    async fn destroyed_window_and_superseded_attachment_both_invalidate() {
        let mut f = fixture();
        let first = f.loader.attach("main").attachment_id;
        load_with(&f, "rt-1", 0, &first, PageReceipt::Loaded)
            .await
            .unwrap();

        let second = f.loader.attach("main");
        assert_eq!(second.superseded.as_deref(), Some(first.as_str()));
        assert_eq!(f.lost.try_recv().unwrap().runtime_id, "rt-1");

        load_with(&f, "rt-2", 1, &second.attachment_id, PageReceipt::Loaded)
            .await
            .unwrap();
        f.loader.window_destroyed("main");
        assert_eq!(f.lost.try_recv().unwrap().runtime_id, "rt-2");
        assert!(!f.loader.is_current(&second.attachment_id));
        assert!(matches!(
            f.loader.start("rt-3", &f.artifact).await,
            Err(ModuleLoadError::HostNotAttached)
        ));
    }

    #[tokio::test]
    async fn undeliverable_instructions_fail_and_unload_stays_retryable() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;

        f.transport.fail_sends(true);
        assert!(matches!(
            f.loader.start("rt-1", &f.artifact).await,
            Err(ModuleLoadError::PageLost)
        ));
        assert_eq!(f.loader.tracked_instances(), 0);
        f.transport.fail_sends(false);

        load_with(&f, "rt-2", 0, &attachment, PageReceipt::Loaded)
            .await
            .unwrap();
        f.transport.fail_sends(true);
        assert!(matches!(
            f.loader.stop("rt-2", &f.artifact).await,
            Err(ModuleLoadError::PageLost)
        ));
        // Not released: still tracked, and a later stop can finish it.
        assert_eq!(f.loader.tracked_instances(), 1);
        f.transport.fail_sends(false);
        let loader = f.loader.clone();
        let artifact = f.artifact.clone();
        let stop = tokio::spawn(async move { loader.stop("rt-2", &artifact).await });
        let (_, unload) = f.transport.nth(1).await;
        f.loader.receive(
            &attachment,
            &ticket(&unload),
            PageReceipt::Unloaded { hook_failed: false },
        );
        stop.await.unwrap().unwrap();
        assert_eq!(f.loader.tracked_instances(), 0);
    }

    #[tokio::test]
    async fn the_page_receives_the_verified_bytes_even_if_the_file_changed_afterwards() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        // The file next to the artifact is irrelevant: the loader has no path.
        fs::write(
            f._dir.path().join("entry.js"),
            "export default () => ({evil:1});",
        )
        .unwrap();
        load_with(&f, "rt-1", 0, &attachment, PageReceipt::Loaded)
            .await
            .unwrap();
        let (_, instruction) = f.transport.nth(0).await;
        match instruction {
            PageInstruction::Load { source, .. } => {
                assert_eq!(source, "export default () => ({});");
            }
            other => panic!("expected load, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_runtime_id_cannot_be_loaded_twice() {
        let f = fixture();
        let attachment = f.loader.attach("main").attachment_id;
        load_with(&f, "rt-1", 0, &attachment, PageReceipt::Loaded)
            .await
            .unwrap();
        assert!(matches!(
            f.loader.start("rt-1", &f.artifact).await,
            Err(ModuleLoadError::Rejected(_))
        ));
        assert_eq!(f.transport.count(), 1);
    }
}
