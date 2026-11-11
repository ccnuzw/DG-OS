//! Request-scoped web configuration for the built-in chat composer. Neither command
//! accepts an owner, endpoint or backend: those are host-derived/fixed respectively.
use super::provider_commands::{CommandError, CommandResponse};
use crate::host::{self, Capability};
use dgos_types::{BuiltinCaller, DgosError};
use dgos_workbench::{web::WebStatus, ChatError, ChatManager};
use serde::Deserialize;
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatWebStatusInput {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureChatWebInput {
    session_id: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    remove: bool,
}

fn chat_failure(error: ChatError) -> CommandResponse<WebStatus> {
    CommandResponse { success: false, data: None, error: Some(CommandError {
        code: error.code(), message: error.message().into(), detail: error.detail(),
    }) }
}

fn web_failure(reason: String) -> CommandResponse<WebStatus> {
    // All service failures are fixed host strings, never a vault/network error or secret.
    chat_failure(ChatError::Web(reason))
}

#[tauri::command]
pub async fn get_chat_web_status(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: ChatWebStatusInput,
) -> CommandResponse<WebStatus> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let resolver = app.state::<Arc<crate::session::SessionResolver>>();
    let (session, lease) = match resolver.agent_session() {
        Ok(value) => value,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    if session.user_id() != context.user_id() || lease.is_cancelled() {
        return CommandResponse::from(Err(DgosError::SessionInvalid));
    }
    let chat = app.state::<ChatManager>().with_account_lease(lease.clone());
    if let Err(error) = chat.get_session(context.user_id(), &input.session_id).await { return chat_failure(error); }
    if lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    let service = match chat.web_service() { Ok(service) => service, Err(reason) => return web_failure(reason) };
    let result = tokio::select! {
        biased;
        _ = lease.cancelled() => return CommandResponse::from(Err(DgosError::SessionInvalid)),
        result = service.status(context.user_id()) => result,
    };
    if lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    if let Err(error) = chat.get_session(context.user_id(), &input.session_id).await { return chat_failure(error); }
    if lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    match result {
        Ok(data) => CommandResponse { success: true, data: Some(data), error: None },
        Err(reason) => web_failure(reason),
    }
}

#[tauri::command]
pub async fn configure_chat_web(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: ConfigureChatWebInput,
) -> CommandResponse<WebStatus> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let resolver = app.state::<Arc<crate::session::SessionResolver>>();
    let (session, lease) = match resolver.agent_session() {
        Ok(value) => value,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    if session.user_id() != context.user_id() || lease.is_cancelled() {
        return CommandResponse::from(Err(DgosError::SessionInvalid));
    }
    let chat = app.state::<ChatManager>().with_account_lease(lease.clone());
    // ChatManager reserves the same per-account admission slot as generation. A
    // check-then-write in this command would permit a new run to race key rotation.
    let result = chat.configure_web(context.user_id(), &input.session_id, window.label(), input.api_key, input.remove).await;
    if lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    if let Err(error) = chat.get_session(context.user_id(), &input.session_id).await { return chat_failure(error); }
    if lease.is_cancelled() { return CommandResponse::from(Err(DgosError::SessionInvalid)); }
    match result {
        Ok(data) => CommandResponse { success: true, data: Some(data), error: None },
        Err(error) => chat_failure(error),
    }
}
