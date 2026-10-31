//! Chat commands (V1-FR-006, interface contract §10.1).
//!
//! Every command passes the same host gate as the single-task commands (trusted main window,
//! `Workbench` caller, `TasksRun`), takes one `deny_unknown_fields` input and never an identity.
//! Streaming events go only to the `Channel` the caller passed; there are no global events. A
//! generation is owned by the calling window: destroying or reloading it cancels the generation
//! and the received part is saved (see the `module_host` hooks).
//!
//! State is read from the `AppHandle` inside the command (like the provider commands), so the
//! commands can return a plain `CommandResponse` instead of a `Result`.

use super::provider_commands::{CommandError, CommandResponse};
use super::{McpState, SkillsState};
use crate::host::{self, Capability};
use dgos_providers::ProviderManager;
use dgos_types::{BuiltinCaller, DgosError};
use dgos_workbench::{
    AdvanceChatToolTurnInput, ChatError, ChatEvent, ChatManager, ChatMessagePage, ChatSearchPage,
    ChatSessionPage, ChatSink, CreateChatSessionInput, DeleteChatSessionInput,
    ListChatMessagesInput, ListChatSessionsInput, RegenerateChatInput, RenameChatSessionInput,
    SearchChatSessionsInput, SendAck, SendChatInput, Sent, UpdateChatSessionTargetInput,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::{Manager, WebviewWindow};

fn respond<T: Serialize>(result: Result<T, ChatError>) -> CommandResponse<T> {
    match result {
        Ok(data) => CommandResponse {
            success: true,
            data: Some(data),
            error: None,
        },
        Err(error) => CommandResponse {
            success: false,
            data: None,
            error: Some(CommandError {
                code: error.code(),
                message: error.message().to_owned(),
                detail: error.detail(),
            }),
        },
    }
}

/// Forwards events to the renderer's Channel. `false` once the page is gone, which stops the
/// generation and saves what was received.
struct ChannelSink(Channel<ChatEvent>);

impl ChatSink for ChannelSink {
    fn send(&self, event: ChatEvent) -> bool {
        self.0.send(event).is_ok()
    }
}

/// Runs a started generation in the background; the ack has already been decided.
fn drive(app: &tauri::AppHandle, sent: Sent, on_event: Channel<ChatEvent>) -> SendAck {
    let Sent { ack, job } = sent;
    if let Some(job) = job {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let chat = handle.state::<ChatManager>();
            let providers = handle.state::<ProviderManager>();
            if job.is_tool_turn() {
                // Tool turns do not stream: they park at a proposal and continue through
                // `advance_chat_tool_turn`, reporting to this same Channel.
                let mcp = handle.state::<McpState>().manager.clone();
                chat.run_tool_turn(&providers, &mcp, job, Arc::new(ChannelSink(on_event)))
                    .await;
            } else {
                chat.run_generation(&providers, job, &ChannelSink(on_event))
                    .await;
            }
        });
    }
    ack
}

#[tauri::command]
pub async fn create_chat_session(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: CreateChatSessionInput,
) -> CommandResponse<dgos_db::ChatSession> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    let providers = app.state::<ProviderManager>();
    let skills = app.state::<SkillsState>();
    let skills = skills.manager.read().await;
    respond(
        chat.create_session(&providers, &skills, context.user_id(), input)
            .await,
    )
}

#[tauri::command]
pub async fn list_chat_sessions(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: ListChatSessionsInput,
) -> CommandResponse<ChatSessionPage> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    respond(chat.list_sessions(context.user_id(), input).await)
}

#[tauri::command]
pub async fn rename_chat_session(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: RenameChatSessionInput,
) -> CommandResponse<dgos_db::ChatSession> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    respond(chat.rename_session(context.user_id(), input).await)
}

#[tauri::command]
pub async fn update_chat_session_target(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: UpdateChatSessionTargetInput,
) -> CommandResponse<dgos_db::ChatSession> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    let providers = app.state::<ProviderManager>();
    let skills = app.state::<SkillsState>();
    let skills = skills.manager.read().await;
    respond(
        chat.update_target(&providers, &skills, context.user_id(), input)
            .await,
    )
}

#[tauri::command]
pub async fn delete_chat_session(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: DeleteChatSessionInput,
) -> CommandResponse<()> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    respond(chat.delete_session(context.user_id(), input).await)
}

#[tauri::command]
pub async fn search_chat_sessions(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: SearchChatSessionsInput,
) -> CommandResponse<ChatSearchPage> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    respond(chat.search_sessions(context.user_id(), input).await)
}

#[tauri::command]
pub async fn list_chat_messages(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: ListChatMessagesInput,
) -> CommandResponse<ChatMessagePage> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    respond(chat.list_messages(context.user_id(), input).await)
}

#[tauri::command]
pub async fn send_chat_message(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: SendChatInput,
    on_event: Channel<ChatEvent>,
) -> CommandResponse<SendAck> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    let providers = app.state::<ProviderManager>();
    let sent = {
        let skills = app.state::<SkillsState>();
        let skills = skills.manager.read().await;
        let mcp = app.state::<McpState>();
        chat.send_with_mcp(
            &providers,
            &skills,
            Some(&mcp.manager),
            context.user_id(),
            window.label(),
            input,
        )
        .await
    };
    respond(sent.map(|sent| drive(&app, sent, on_event)))
}

#[tauri::command]
pub async fn regenerate_chat_message(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: RegenerateChatInput,
    on_event: Channel<ChatEvent>,
) -> CommandResponse<SendAck> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    let providers = app.state::<ProviderManager>();
    let sent = {
        let skills = app.state::<SkillsState>();
        let skills = skills.manager.read().await;
        chat.regenerate(
            &providers,
            &skills,
            context.user_id(),
            window.label(),
            input,
        )
        .await
    };
    respond(sent.map(|sent| drive(&app, sent, on_event)))
}

/// Approves or denies the one outstanding proposal of a tool turn (D022). The decision is
/// validated and the parked context taken before this returns; the model/MCP work then continues
/// in the background and reports through the turn's original Channel.
#[tauri::command]
pub async fn advance_chat_tool_turn(
    app: tauri::AppHandle,
    window: WebviewWindow,
    input: AdvanceChatToolTurnInput,
) -> CommandResponse<()> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    let job = match chat.begin_tool_action(context.user_id(), input).await {
        Ok(job) => job,
        Err(error) => return respond::<()>(Err(error)),
    };
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let chat = handle.state::<ChatManager>();
        let providers = handle.state::<ProviderManager>();
        let mcp = handle.state::<McpState>().manager.clone();
        chat.run_tool_action(&providers, &mcp, job).await;
    });
    respond(Ok(()))
}

/// Stops one of the caller's generations. Unknown, finished and foreign ids answer `false`.
#[tauri::command]
pub async fn cancel_chat_generation(
    app: tauri::AppHandle,
    window: WebviewWindow,
    task_id: String,
) -> CommandResponse<bool> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) {
        Ok(context) => context,
        Err(error) => return CommandResponse::from(Err(error)),
    };
    let chat = app.state::<ChatManager>();
    CommandResponse::from(Ok::<bool, DgosError>(
        chat.cancel(context.user_id(), &task_id),
    ))
}
