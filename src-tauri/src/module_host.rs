//! Tauri adapter for the page module loader (D042-D046).
//!
//! All WebView specifics live here: the per-attachment `Channel` transport,
//! the wire DTOs, and the page-load / window-destroy hooks that tell the
//! loader when a JS realm goes away. The protocol state machine itself is
//! `dgos_modules::PageModuleLoader`; business orchestration stays in the
//! Application layer.
//!
//! Neither the channel nor the receipt command is an authority: a script
//! sharing the page's realm can call both. They only decide whether the host
//! marks a runtime ready. What may run is decided by admission (D041).

use dgos_modules::{
    ModuleCallRouter, ModulePageTransport, ModuleReadyObserver, ModuleRuntimeHandle,
    PageFailureStage, PageInstruction, PageModuleLoader, PageReceipt, PageSendError,
    ReceiptDisposition,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::ipc::Channel;
use tauri::Manager;

/// The only window module code runs in.
pub const MODULE_WINDOW: &str = "main";

/// Managed only by the production host: when present, a page attaching to the
/// module host starts every published, enabled module (D055).
pub struct StartupLoad;

/// Host to page. Mirrors `HostInstruction` in `src/modules/moduleRuntimeHost.ts`.
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InstructionDto {
    Load {
        ticket: String,
        runtime_id: String,
        module_id: String,
        version: String,
        source: String,
        hook_timeout_ms: u64,
        replaces: Option<String>,
    },
    Unload {
        ticket: String,
        runtime_id: String,
        hook_timeout_ms: u64,
    },
    /// Routing key for `host.providers.call` (D051). Not a ticket: it has no
    /// receipt and carries no identity, permission or capability.
    Route { runtime_id: String, route: String },
}

impl From<PageInstruction> for InstructionDto {
    fn from(instruction: PageInstruction) -> Self {
        match instruction {
            PageInstruction::Load {
                ticket,
                runtime_id,
                module_id,
                version,
                source,
                hook_timeout_ms,
                replaces,
            } => Self::Load {
                ticket,
                runtime_id,
                module_id,
                version,
                source,
                hook_timeout_ms,
                replaces,
            },
            PageInstruction::Unload {
                ticket,
                runtime_id,
                hook_timeout_ms,
            } => Self::Unload {
                ticket,
                runtime_id,
                hook_timeout_ms,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StageDto {
    Import,
    Contract,
    Render,
    Hook,
    Timeout,
    Cancelled,
}

impl From<StageDto> for PageFailureStage {
    fn from(stage: StageDto) -> Self {
        match stage {
            StageDto::Import => Self::Import,
            StageDto::Contract => Self::Contract,
            StageDto::Render => Self::Render,
            StageDto::Hook => Self::Hook,
            StageDto::Timeout => Self::Timeout,
            StageDto::Cancelled => Self::Cancelled,
        }
    }
}

/// Page to host. Coarse results only: no messages, stacks, ids or paths.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReceiptDto {
    Loaded,
    LoadFailed { stage: StageDto },
    Unloaded { hook_failed: bool },
}

impl From<ReceiptDto> for PageReceipt {
    fn from(receipt: ReceiptDto) -> Self {
        match receipt {
            ReceiptDto::Loaded => Self::Loaded,
            ReceiptDto::LoadFailed { stage } => Self::LoadFailed(stage.into()),
            ReceiptDto::Unloaded { hook_failed } => Self::Unloaded { hook_failed },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedDto {
    pub attachment_id: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DispositionDto {
    Applied,
    Duplicate,
    Stale,
    Abandoned,
    Unknown,
}

impl From<ReceiptDisposition> for DispositionDto {
    fn from(disposition: ReceiptDisposition) -> Self {
        match disposition {
            ReceiptDisposition::Applied => Self::Applied,
            ReceiptDisposition::Duplicate => Self::Duplicate,
            ReceiptDisposition::Stale => Self::Stale,
            ReceiptDisposition::Abandoned => Self::Abandoned,
            ReceiptDisposition::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportedDto {
    pub disposition: DispositionDto,
}

/// One Tauri `Channel` per live attachment. A channel is bound by Tauri to
/// the webview that created it, so a page that was replaced cannot receive
/// instructions meant for its successor.
#[derive(Default)]
pub struct ChannelTransport {
    channels: Mutex<HashMap<String, Channel<InstructionDto>>>,
}

impl ChannelTransport {
    fn channels(&self) -> MutexGuard<'_, HashMap<String, Channel<InstructionDto>>> {
        self.channels
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl ModulePageTransport for ChannelTransport {
    fn send(&self, attachment_id: &str, instruction: PageInstruction) -> Result<(), PageSendError> {
        let channels = self.channels();
        let channel = channels.get(attachment_id).ok_or(PageSendError)?;
        channel.send(instruction.into()).map_err(|error| {
            tracing::warn!(%error, "module host instruction could not be delivered");
            PageSendError
        })
    }
}

impl ChannelTransport {
    /// Deliver a route to the one page instance it was issued for.
    fn send_route(
        &self,
        attachment_id: &str,
        runtime_id: &str,
        route: &str,
    ) -> Result<(), PageSendError> {
        let channels = self.channels();
        let channel = channels.get(attachment_id).ok_or(PageSendError)?;
        channel
            .send(InstructionDto::Route {
                runtime_id: runtime_id.to_owned(),
                route: route.to_owned(),
            })
            .map_err(|_| {
                // The error text is not logged: it may echo the payload.
                tracing::warn!("module call route could not be delivered");
                PageSendError
            })
    }
}

/// Gives each runtime that becomes `ready` its call route (D051). Best
/// effort: a runtime without a route simply cannot call the host, and its
/// state is unaffected.
pub struct RoutePublisher {
    router: Arc<ModuleCallRouter>,
    transport: Arc<ChannelTransport>,
    /// `(runtime_id, attachment_id, route)` of every route handed to a page.
    /// Smoke-only: lets the harness replay a route host-side. Absent from
    /// production builds, where a route is never readable outside the page.
    #[cfg(any(test, feature = "webview-smoke"))]
    issued: Mutex<Vec<(String, String, String)>>,
}

impl RoutePublisher {
    pub fn new(router: Arc<ModuleCallRouter>, transport: Arc<ChannelTransport>) -> Self {
        Self {
            router,
            transport,
            #[cfg(any(test, feature = "webview-smoke"))]
            issued: Mutex::default(),
        }
    }

    #[cfg(any(test, feature = "webview-smoke"))]
    #[cfg_attr(not(feature = "webview-smoke"), allow(dead_code))]
    pub fn issued(&self) -> Vec<(String, String, String)> {
        self.issued
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ModuleReadyObserver for RoutePublisher {
    fn runtime_ready(&self, runtime: &ModuleRuntimeHandle) {
        match self.router.issue_with_attachment(runtime) {
            Ok((route, attachment_id)) => {
                #[cfg(any(test, feature = "webview-smoke"))]
                self.issued
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push((
                        runtime.runtime_id().to_owned(),
                        attachment_id.clone(),
                        route.as_str().to_owned(),
                    ));
                if self
                    .transport
                    .send_route(&attachment_id, runtime.runtime_id(), route.as_str())
                    .is_err()
                {
                    self.router.forget(runtime);
                }
            }
            Err(rejection) => {
                tracing::debug!(?rejection, "module call route was not issued");
            }
        }
    }
}

/// Managed Tauri state: ties the protocol state machine to its transport.
pub struct ModuleHostBridge {
    loader: PageModuleLoader,
    transport: Arc<ChannelTransport>,
}

impl ModuleHostBridge {
    pub fn new(loader: PageModuleLoader, transport: Arc<ChannelTransport>) -> Self {
        Self { loader, transport }
    }

    /// Register a new page attachment. The channel map stays locked while
    /// the loader learns about the attachment, so no instruction can race
    /// ahead of its own channel.
    pub fn attach(&self, channel: Channel<InstructionDto>) -> String {
        let mut channels = self.transport.channels();
        let attachment = self.loader.attach(MODULE_WINDOW);
        if let Some(superseded) = &attachment.superseded {
            channels.remove(superseded);
        }
        channels.insert(attachment.attachment_id.clone(), channel);
        attachment.attachment_id
    }

    pub fn report(
        &self,
        attachment_id: &str,
        ticket: &str,
        receipt: ReceiptDto,
    ) -> ReceiptDisposition {
        let disposition = self.loader.receive(attachment_id, ticket, receipt.into());
        if disposition != ReceiptDisposition::Applied {
            tracing::debug!(?disposition, "module host receipt was not applied");
        }
        disposition
    }

    /// A full navigation or reload of `window` started. SPA route changes
    /// never reach this hook.
    pub fn page_started(&self, window: &str) {
        if window == MODULE_WINDOW {
            self.loader.page_started(window);
            self.prune();
        }
    }

    pub fn window_destroyed(&self, window: &str) {
        if window == MODULE_WINDOW {
            self.loader.window_destroyed(window);
            self.prune();
        }
    }

    /// Drop channels of attachments the loader no longer considers current.
    fn prune(&self) {
        self.transport
            .channels()
            .retain(|attachment_id, _| self.loader.is_current(attachment_id));
    }

    #[cfg(any(test, feature = "webview-smoke"))]
    pub fn loader(&self) -> &PageModuleLoader {
        &self.loader
    }
}

/// Page-load hook body (D044): a navigation or reload of a window starts,
/// so its previous JS realm is going away. The bridge may not be managed
/// yet while the first page loads; there is nothing to invalidate then.
/// `on_page_load` replaces any earlier hook, so a builder that needs its own
/// page-load logic calls this from its closure.
pub fn handle_page_load<R: tauri::Runtime>(
    webview: &tauri::Webview<R>,
    payload: &tauri::webview::PageLoadPayload<'_>,
) {
    if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
        if let Some(bridge) = webview.try_state::<ModuleHostBridge>() {
            bridge.page_started(webview.label());
        }
        // A reload drops the page's Channel; its streams would otherwise run to the end unseen.
        if let Some(streams) = webview.try_state::<crate::provider_streams::ProviderStreams>() {
            streams.cancel_owner(webview.label());
        }
    }
}

pub fn handle_window_event<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    event: &tauri::WindowEvent,
) {
    if matches!(event, tauri::WindowEvent::Destroyed) {
        if let Some(bridge) = window.try_state::<ModuleHostBridge>() {
            bridge.window_destroyed(window.label());
        }
        if let Some(streams) = window.try_state::<crate::provider_streams::ProviderStreams>() {
            streams.cancel_owner(window.label());
        }
    }
}

/// Install the page-load and window-destroy hooks that keep page generations
/// and attachments correct (D044).
pub fn install<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .on_page_load(handle_page_load)
        .on_window_event(handle_window_event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dgos_modules::PageLoaderConfig;
    use serde_json::{json, Value};
    use tauri::ipc::InvokeResponseBody;

    type Sink = Arc<Mutex<Vec<Value>>>;

    fn capture() -> (Channel<InstructionDto>, Sink) {
        let sink: Sink = Arc::default();
        let writer = sink.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                writer
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&text).unwrap());
            }
            Ok(())
        });
        (channel, sink)
    }

    fn bridge() -> (Arc<ChannelTransport>, ModuleHostBridge) {
        let transport = Arc::new(ChannelTransport::default());
        let (loader, _lost) = PageModuleLoader::new(transport.clone(), PageLoaderConfig::default());
        (transport.clone(), ModuleHostBridge::new(loader, transport))
    }

    fn unload(ticket: &str) -> PageInstruction {
        PageInstruction::Unload {
            ticket: ticket.into(),
            runtime_id: "rt-1".into(),
            hook_timeout_ms: 5000,
        }
    }

    #[test]
    fn instructions_use_the_camel_case_wire_shape_the_page_expects() {
        let load = InstructionDto::from(PageInstruction::Load {
            ticket: "t1".into(),
            runtime_id: "rt-1".into(),
            module_id: "mod-a".into(),
            version: "1.0.0".into(),
            source: "export default 1;".into(),
            hook_timeout_ms: 5000,
            replaces: Some("rt-0".into()),
        });
        assert_eq!(
            serde_json::to_value(load).unwrap(),
            json!({
                "kind": "load", "ticket": "t1", "runtimeId": "rt-1", "moduleId": "mod-a",
                "version": "1.0.0", "source": "export default 1;", "hookTimeoutMs": 5000,
                "replaces": "rt-0"
            })
        );
        assert_eq!(
            serde_json::to_value(InstructionDto::from(unload("t2"))).unwrap(),
            json!({"kind": "unload", "ticket": "t2", "runtimeId": "rt-1", "hookTimeoutMs": 5000})
        );
    }

    #[test]
    fn receipts_parse_only_the_documented_shapes() {
        let parse = |value: Value| serde_json::from_value::<ReceiptDto>(value);
        assert_eq!(
            parse(json!({"kind": "loaded"})).unwrap(),
            ReceiptDto::Loaded
        );
        assert_eq!(
            parse(json!({"kind": "loadFailed", "stage": "render"})).unwrap(),
            ReceiptDto::LoadFailed {
                stage: StageDto::Render
            }
        );
        assert_eq!(
            parse(json!({"kind": "unloaded", "hookFailed": true})).unwrap(),
            ReceiptDto::Unloaded { hook_failed: true }
        );
        assert!(parse(json!({"kind": "loadFailed", "stage": "weird"})).is_err());
        assert!(parse(json!({"kind": "loadFailed"})).is_err());
        assert!(parse(json!({"kind": "unloaded"})).is_err());
        assert!(parse(json!({"kind": "exec"})).is_err());
    }

    #[test]
    fn instructions_reach_only_the_channel_of_the_current_attachment() {
        let (transport, bridge) = bridge();
        let (first, first_sink) = capture();
        let first_id = bridge.attach(first);
        transport.send(&first_id, unload("t1")).unwrap();
        assert_eq!(first_sink.lock().unwrap().len(), 1);

        // A second attachment supersedes the first and removes its channel.
        let (second, second_sink) = capture();
        let second_id = bridge.attach(second);
        assert_ne!(first_id, second_id);
        assert_eq!(transport.send(&first_id, unload("t2")), Err(PageSendError));
        transport.send(&second_id, unload("t3")).unwrap();
        assert_eq!(first_sink.lock().unwrap().len(), 1);
        assert_eq!(second_sink.lock().unwrap().len(), 1);
        assert_eq!(second_sink.lock().unwrap()[0]["ticket"], "t3");
    }

    #[test]
    fn page_start_and_window_destroy_drop_the_channel() {
        let (transport, bridge) = bridge();
        let (channel, _sink) = capture();
        let id = bridge.attach(channel);
        bridge.page_started("secondary");
        transport.send(&id, unload("t1")).unwrap();

        bridge.page_started(MODULE_WINDOW);
        assert_eq!(transport.send(&id, unload("t2")), Err(PageSendError));

        let (channel, _sink) = capture();
        let id = bridge.attach(channel);
        transport.send(&id, unload("t3")).unwrap();
        bridge.window_destroyed(MODULE_WINDOW);
        assert_eq!(transport.send(&id, unload("t4")), Err(PageSendError));
    }

    #[test]
    fn unknown_tickets_are_reported_back_without_effect() {
        let (_transport, bridge) = bridge();
        let (channel, _sink) = capture();
        let id = bridge.attach(channel);
        assert_eq!(
            bridge.report(&id, "never-issued", ReceiptDto::Loaded),
            ReceiptDisposition::Unknown
        );
    }

    #[test]
    fn only_the_main_window_lifecycle_touches_the_attachment() {
        let (transport, bridge) = bridge();
        let (channel, _sink) = capture();
        let id = bridge.attach(channel);
        for other in ["secondary", "Main", ""] {
            bridge.page_started(other);
            bridge.window_destroyed(other);
        }
        // Another window reloading or closing never invalidates main's page.
        transport.send(&id, unload("t1")).unwrap();
        assert!(bridge.loader().is_current(&id));
    }

    #[test]
    fn a_reloaded_page_loses_its_attachment_and_channel_and_needs_a_new_one() {
        let (transport, bridge) = bridge();
        let (channel, _sink) = capture();
        let old = bridge.attach(channel);
        bridge.page_started(MODULE_WINDOW);
        assert!(!bridge.loader().is_current(&old));
        assert_eq!(bridge.loader().generation(MODULE_WINDOW), 1);
        assert_eq!(transport.send(&old, unload("t2")), Err(PageSendError));
        // No ticket was issued to the dead realm, so nothing is applied.
        assert_ne!(
            bridge.report(&old, "t1", ReceiptDto::Loaded),
            ReceiptDisposition::Applied
        );

        let (channel, sink) = capture();
        let fresh = bridge.attach(channel);
        assert_ne!(old, fresh);
        assert_eq!(bridge.loader().attachment_generation(&fresh), Some(1));
        transport.send(&fresh, unload("t3")).unwrap();
        assert_eq!(sink.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_destroyed_window_loses_its_attachment() {
        let (transport, bridge) = bridge();
        let (channel, _sink) = capture();
        let id = bridge.attach(channel);
        bridge.window_destroyed(MODULE_WINDOW);
        assert!(!bridge.loader().has_attachment(MODULE_WINDOW));
        assert_eq!(transport.send(&id, unload("t1")), Err(PageSendError));
        assert_ne!(
            bridge.report(&id, "t1", ReceiptDto::Loaded),
            ReceiptDisposition::Applied
        );
    }
}
