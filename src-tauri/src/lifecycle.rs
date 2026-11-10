//! Desktop shutdown shared by the application event loop and isolated host smoke.
use crate::{capabilities, chat_media, commands, module_host, provider_streams, session};
use dgos_providers::ProviderManager;
use std::sync::Arc;
use tauri::Manager;

pub(crate) async fn shutdown(app: &tauri::AppHandle) {
    let module_session = app.try_state::<Arc<session::SessionResolver>>()
        .and_then(|resolver| resolver.current().ok());
    if let Some(resolver) = app.try_state::<Arc<session::SessionResolver>>() {
        resolver.close_agents();
    }
    if let Some(bridge) = app.try_state::<module_host::ModuleHostBridge>() {
        bridge.page_started(module_host::MODULE_WINDOW);
    }
    if let Some(streams) = app.try_state::<provider_streams::ProviderStreams>() {
        streams.cancel_owner(module_host::MODULE_WINDOW);
    }
    if let Some(transcriptions) = app.try_state::<chat_media::Transcriptions>() {
        transcriptions.cancel_owner(module_host::MODULE_WINDOW);
    }
    let ai = app.try_state::<dgos_application::AiService>().map(|v| v.inner().clone());
    let providers = app.try_state::<ProviderManager>().map(|v| v.inner().clone());
    let modules = app.try_state::<capabilities::ModuleOperationManager>().map(|v| v.inner().clone());
    // Accepted Provider mutation tails must finish before runtime/DB teardown.
    let service_cleanup = async move {
        let ai_cleanup = async { match ai { Some(ai) => ai.shutdown().await, None => true } };
        let provider_cleanup = async { match providers { Some(providers) => providers.shutdown_provider_mutations().await.is_ok(), None => true } };
        let module_cleanup = async {
            match (modules, module_session) {
                (Some(modules), Some(session)) => modules.stop_all(&session).await.is_empty(),
                _ => true,
            }
        };
        tokio::join!(ai_cleanup, provider_cleanup, module_cleanup)
    };
    let agents = app.try_state::<dgos_agents::AgentManager>().map(|m| m.inner().clone());
    if let Some(agents) = &agents { agents.close(); }
    let agent_cleanup = async move {
        match agents { Some(agents) => agents.shutdown().await, None => true }
    };
    // Missing managers must not skip available managers. Start independent drains together.
    let mcp = app.try_state::<commands::McpState>().map(|v| v.manager.clone());
    let workbench = app.try_state::<dgos_workbench::WorkbenchManager>().map(|v| v.inner().clone());
    let chat = app.try_state::<dgos_workbench::ChatManager>().map(|v| v.inner().clone());
    let lan = app.try_state::<Arc<dgos_lan::LanService>>().map(|v| v.inner().clone());
    let native = app.try_state::<crate::native_extensions::NativeHost>();
    let other_cleanup = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let lan_cleanup = async { if let Some(lan) = lan { lan.shutdown().await; } };
            let workbench_cleanup = async { match workbench { Some(workbench) => workbench.shutdown().await.is_ok(), None => true } };
            let mcp_cleanup = async { match mcp { Some(mcp) => mcp.shutdown().await.is_ok(), None => true } };
            let chat_cleanup = async { match chat { Some(chat) => chat.shutdown().await, None => true } };
            let native_cleanup = async { match native { Some(native) => native.shutdown().await, None => true } };
            tokio::join!(lan_cleanup, workbench_cleanup, mcp_cleanup, chat_cleanup, native_cleanup)
        }).await
    };
    match tokio::join!(other_cleanup, agent_cleanup, service_cleanup) {
        (Ok(((), true, true, true, true)), true, (true, true, true)) => {}
        _ => tracing::warn!("Desktop shutdown incomplete; startup recovery will finalize interrupted audits"),
    }
}
