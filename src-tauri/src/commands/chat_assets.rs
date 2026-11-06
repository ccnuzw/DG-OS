//! Built-in workbench controlled assets and explicit transcription (D133).
use super::provider_commands::{CommandError, CommandResponse};
use crate::{host::{self, Capability}, chat_media::Transcriptions};
use base64::Engine as _;
use dgos_types::{BuiltinCaller, DgosError};
use dgos_workbench::{ChatAssets, ChatAssetImportInput, ChatAssetPreview, ChatAssetRemoved, ChatError};
use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewWindow};

fn respond<T: Serialize>(result: Result<T, ChatError>) -> CommandResponse<T> {
    match result {
        Ok(data) => CommandResponse { success: true, data: Some(data), error: None },
        Err(error) => CommandResponse { success: false, data: None, error: Some(CommandError { code: error.code(), message: error.message().into(), detail: error.detail() }) },
    }
}
#[tauri::command]
pub async fn import_chat_asset(app: tauri::AppHandle, window: WebviewWindow, input: ChatAssetImportInput) -> CommandResponse<dgos_db::AssetRef> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) { Ok(value) => value, Err(error) => return CommandResponse::from(Err(error)) };
    let _permit = match app.state::<dgos_db::MaintenanceGate>().admit() { Ok(value) => value, Err(_) => return CommandResponse::from(Err(DgosError::ProviderRequestRejected)) };
    respond(app.state::<ChatAssets>().import(context.user_id(), input).await)
}
#[tauri::command]
pub async fn preview_chat_asset(app: tauri::AppHandle, window: WebviewWindow, asset_id: String) -> CommandResponse<ChatAssetPreview> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) { Ok(value) => value, Err(error) => return CommandResponse::from(Err(error)) };
    respond(app.state::<ChatAssets>().preview(context.user_id(), &asset_id).await)
}
#[tauri::command]
pub async fn remove_chat_asset(app: tauri::AppHandle, window: WebviewWindow, asset_id: String) -> CommandResponse<ChatAssetRemoved> {
    let context = match host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun) { Ok(value) => value, Err(error) => return CommandResponse::from(Err(error)) };
    let _permit = match app.state::<dgos_db::MaintenanceGate>().admit() { Ok(value) => value, Err(_) => return CommandResponse::from(Err(DgosError::ProviderRequestRejected)) };
    respond(app.state::<ChatAssets>().remove(context.user_id(), &asset_id).await)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptionInput { request_id: String, provider_id: String, model_id: String, mime: String, base64: String }
#[derive(Serialize)]
pub struct TranscriptionResult { text: String }
#[tauri::command]
pub async fn transcribe_chat_audio(app: tauri::AppHandle, window: WebviewWindow, input: TranscriptionInput) -> CommandResponse<TranscriptionResult> {
    let result = async {
        let context = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun)?;
        if input.request_id.is_empty() || input.request_id.len() > 128 || input.base64.len() > (25_usize * 1024 * 1024).div_ceil(3) * 4 { return Err(DgosError::ProviderRequestRejected); }
        let _permit = app.state::<dgos_db::MaintenanceGate>().admit().map_err(|_| DgosError::ProviderRequestRejected)?;
        let (session, lease) = app.state::<std::sync::Arc<crate::session::SessionResolver>>().agent_session()?;
        if session.user_id() != context.user_id() { return Err(DgosError::SessionInvalid); }
        let audio = base64::engine::general_purpose::STANDARD.decode(input.base64).map_err(|_| DgosError::ProviderRequestRejected)?;
        let mime = input.mime.split(';').next().unwrap_or("").trim();
        let signed = match mime {
            "audio/webm" => audio.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
            "audio/ogg" => audio.starts_with(b"OggS"),
            "audio/mp4" => audio.len() >= 12 && &audio[4..8] == b"ftyp",
            _ => false,
        };
        if !signed { return Err(DgosError::ProviderRequestRejected); }
        let registry = app.state::<Transcriptions>();
        let cancel = registry.begin(context.user_id(), &input.request_id, window.label())?;
        let providers = app.state::<dgos_providers::ProviderManager>();
        let result = tokio::select! {
            biased;
            _ = lease.cancelled() => Err(DgosError::SessionInvalid),
            result = providers.transcribe_audio(context.user_id(), &input.provider_id, &input.model_id, mime, audio, cancel) => result,
        };
        let cancelled = registry.finish(context.user_id(), &input.request_id);
        if lease.is_cancelled() { return Err(DgosError::SessionInvalid); }
        if cancelled { return Err(DgosError::StreamInterrupted); }
        result.map(|text| TranscriptionResult { text })
    }.await;
    CommandResponse::from(result)
}
#[tauri::command]
pub async fn cancel_chat_transcription(app: tauri::AppHandle, window: WebviewWindow, request_id: String) -> CommandResponse<bool> {
    let result = host::context(&window, BuiltinCaller::Workbench, Capability::TasksRun).map(|context| app.state::<Transcriptions>().cancel(context.user_id(), &request_id));
    CommandResponse::from(result)
}
