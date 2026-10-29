//! Page bridge commands for real module loading (D046).
//!
//! The only renderer-to-host path of the module host. They serve the page's
//! own lifecycle: `attach_module_host` registers the instruction channel and
//! `report_module_host` returns the result of an instruction. Neither runs a
//! lifecycle operation, returns module data, or takes a module id, user,
//! version, digest, path, URL or permission. A receipt names only a
//! host-issued `attachment_id` and `ticket`; the host maps the ticket to its
//! runtime. The gate is the trusted `main` window, not the Settings route:
//! the page attaches at application start and keeps its modules across SPA
//! navigation. Neither command is an authority; a script sharing the page's
//! realm can call both, so admission (D041) is the real boundary.

use super::CommandResponse;
use crate::host;
use crate::module_host::{
    AttachedDto, InstructionDto, ModuleHostBridge, ReceiptDto, ReportedDto, StartupLoad,
};
use dgos_application::{
    ModuleCapabilityDispatchService, ModuleCrashService, ModuleOperationManager,
};
use dgos_modules::{CallRejection, ModuleCallRouter};
use dgos_providers::ProviderManager;
use dgos_types::{ChatRequest, ChatResponse, DgosError, HostSession};
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub async fn attach_module_host(
    app: tauri::AppHandle,
    window: WebviewWindow,
    on_instruction: Channel<InstructionDto>,
) -> CommandResponse<AttachedDto> {
    if let Err(error) = host::module_host_window(&window) {
        return CommandResponse::from(Err(error));
    }
    let bridge = app.state::<ModuleHostBridge>();
    let attachment_id = bridge.attach(on_instruction);
    // Startup load (D055): once the page can receive instructions, start every
    // published, enabled module. Only hosts that manage `StartupLoad` do this,
    // so the lifecycle smoke targets keep their explicit starts.
    if app.try_state::<StartupLoad>().is_some() {
        let manager = app.state::<ModuleOperationManager>().inner().clone();
        tauri::async_runtime::spawn(async move {
            for (module_id, accepted) in manager.start_enabled(&HostSession::local_default()).await
            {
                if let Err(kind) = accepted {
                    tracing::warn!(
                        module_id,
                        code = kind.code(),
                        "startup load was not accepted"
                    );
                }
            }
        });
    }
    CommandResponse::from(Ok(AttachedDto { attachment_id }))
}

#[tauri::command]
pub async fn report_module_host(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    ticket: String,
    receipt: ReceiptDto,
) -> CommandResponse<ReportedDto> {
    if let Err(error) = host::module_host_window(&window) {
        return CommandResponse::from(Err(error));
    }
    let bridge = app.state::<ModuleHostBridge>();
    let disposition = bridge.report(&attachment_id, &ticket, receipt);
    CommandResponse::from(Ok(ReportedDto {
        disposition: disposition.into(),
    }))
}

/// `providers.call` for module code (D051). The only identity input is the
/// page's `attachment_id` plus the host-issued `route`; the module, runtime,
/// version, digest and user all come from the host-side binding. Channel
/// admission failures answer `1004004` without saying why; a runtime at its
/// in-flight limit answers the retryable `1001005`. Authorization is checked
/// again on every call by the dispatch service.
#[tauri::command]
pub async fn module_call_provider(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
    provider_id: String,
    model_id: String,
    request: ChatRequest,
) -> CommandResponse<ChatResponse> {
    if let Err(error) = host::module_host_window(&window) {
        return CommandResponse::from(Err(error));
    }
    let router = app.state::<Arc<ModuleCallRouter>>();
    let call = match router.admit_once(&route, &attachment_id, &call_id) {
        Ok(call) => call,
        Err(CallRejection::Busy) => {
            return CommandResponse::from(Err(DgosError::RateLimitExceeded));
        }
        Err(rejection) => {
            tracing::debug!(?rejection, "module call was not admitted");
            return CommandResponse::from(Err(DgosError::PermissionDenied));
        }
    };
    let dispatch = app.state::<ModuleCapabilityDispatchService>();
    let providers = app.state::<ProviderManager>();
    let result = dispatch
        .call_provider_bound(call, providers.inner(), &provider_id, &model_id, request)
        .await;
    CommandResponse::from(result)
}

#[derive(serde::Serialize)]
pub struct CrashRecorded {
    pub recorded: bool,
}

/// A post-mount render crash reported by the page (D055). The runtime comes
/// from the host-issued route, never from the page; the host sanitizes
/// `error_name` and `stack` and keeps no message text. Every refusal is
/// `1004004` without a reason.
#[tauri::command]
pub async fn report_module_crash(
    app: tauri::AppHandle,
    window: WebviewWindow,
    attachment_id: String,
    route: String,
    call_id: String,
    source: String,
    error_name: String,
    stack: String,
) -> CommandResponse<CrashRecorded> {
    if let Err(error) = host::module_host_window(&window) {
        return CommandResponse::from(Err(error));
    }
    let router = app.state::<Arc<ModuleCallRouter>>();
    let runtime = match router.resolve_once(&route, &attachment_id, &call_id) {
        Ok(runtime) => runtime,
        Err(rejection) => {
            tracing::debug!(?rejection, "module crash report was not accepted");
            return CommandResponse::from(Err(DgosError::PermissionDenied));
        }
    };
    let crashes = app.state::<ModuleCrashService>();
    let result = crashes
        .report(&runtime, &source, &error_name, &stack)
        .await
        .map(|()| CrashRecorded { recorded: true });
    CommandResponse::from(result)
}
