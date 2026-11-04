//! D133 explicit audio transcription: one bounded multipart request, no automatic retry.
#[cfg(test)]
#[path = "transcription_tests.rs"]
mod tests;
use super::*;
use reqwest::{header::HeaderValue, multipart::{Form, Part}};
use std::time::Duration;

impl ProviderManager {
    pub async fn transcribe_audio(&self, user: &str, provider_id: &str, model_id: &str, mime: &str, audio: Vec<u8>, cancel: CancellationToken) -> DgosResult<String> {
        let extension = match mime.split(';').next().unwrap_or("").trim() {
            "audio/webm" => "webm", "audio/mp4" => "mp4", "audio/ogg" => "ogg",
            _ => return Err(DgosError::RequestNotSupported),
        };
        if audio.is_empty() || audio.len() > 25 * 1024 * 1024 { return Err(DgosError::ProviderRequestRejected); }
        let operation = async {
            let (provider, normalized, secret) = {
                let _guard = self.credentials_lock.lock().await;
                let provider = self.owned_provider(provider_id, user).await?;
                if !provider.enabled { return Err(DgosError::ProviderUnavailable("Provider is disabled".into())); }
                let normalized = validate_provider_config(&provider.provider_type, &provider.base_url, provider.config.as_ref())?;
                if normalized.provider_type != dgos_types::PROTOCOL_OPENAI_COMPATIBLE { return Err(DgosError::RequestNotSupported); }
                if !self.model_repo.list(provider_id).await?.iter().any(|model| model.model_id == model_id && model.model_type == "speech-transcription") { return Err(DgosError::RequestNotSupported); }
                let reference = provider.credential_ref.clone().ok_or(DgosError::CredentialUnavailable)?;
                let secret = self.vault.load(&scope_for(user, provider_id, &provider.base_url, reference)).await?;
                (provider, normalized, secret)
            };
            let client = self.http.as_ref().ok_or_else(|| DgosError::InternalError("HTTP client unavailable".into()))?;
            let part = Part::bytes(audio).file_name(format!("recording.{extension}")).mime_str(mime).map_err(|_| DgosError::ProviderRequestRejected)?;
            let form = Form::new().text("model", model_id.to_owned()).text("response_format", "json").part("file", part);
            let mut request = client.post(format!("{}/audio/transcriptions", normalized.base_url)).multipart(form);
            let auth = normalized.auth_override.unwrap_or(normalized.profile.auth);
            if auth.sends_bearer() {
                let mut value = HeaderValue::from_str(&format!("Bearer {}", secret.expose())).map_err(|_| DgosError::InvalidApiKey)?;
                value.set_sensitive(true); request = request.header(reqwest::header::AUTHORIZATION, value);
            }
            if auth.sends_x_api_key() {
                let mut value = HeaderValue::from_str(secret.expose()).map_err(|_| DgosError::InvalidApiKey)?;
                value.set_sensitive(true); request = request.header("x-api-key", value);
            }
            let response = request.send().await.map_err(|_| DgosError::NetworkError("transcription transport failed; result unknown".into()))?;
            if !response.status().is_success() {
                return Err(match response.status().as_u16() {
                    401 => DgosError::InvalidApiKey, 403 => DgosError::ProviderPermissionDenied,
                    404 | 405 => DgosError::RequestNotSupported, 429 => DgosError::RateLimitExceeded,
                    500..=599 => DgosError::UpstreamServerError, _ => DgosError::ProviderRequestRejected,
                });
            }
            let bytes = crate::upstream::read_limited(response, 1024 * 1024).await.map_err(|_| DgosError::ProviderResponseInvalid)?;
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| DgosError::ProviderResponseInvalid)?;
            let text = value.get("text").and_then(serde_json::Value::as_str).filter(|text| !text.trim().is_empty() && text.len() <= 1024 * 1024).ok_or(DgosError::ProviderResponseInvalid)?;
            let latest = self.owned_provider(provider_id, user).await?;
            if !latest.enabled || latest.config_revision != provider.config_revision { return Err(DgosError::ProviderConfigurationChanged); }
            Ok(text.to_owned())
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(DgosError::StreamInterrupted),
            result = tokio::time::timeout(Duration::from_secs(120), operation) => result.unwrap_or(Err(DgosError::Timeout)),
        }
    }
}
