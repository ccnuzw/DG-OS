use super::*;

fn preview_error(error: String) -> String {
    match error.as_str() {
        "image_preview_expired" | "session_not_found" | "session_closed" | "session_invalid"
        | "stale_generation" | "native_extension_StaleGeneration" | "cancelled" => "image_preview_expired".into(),
        _ => "image_preview_unavailable".into(),
    }
}

impl Backend {
    async fn preview_output_image(&self, preview_id: &str) -> Result<crate::images::OutputImagePreview, String> {
        match self {
            Self::Codex(session) => session.preview_output_image(preview_id).await,
            Self::Claude(session) => session.preview_output_image(preview_id).await,
        }
    }
}

impl AgentManager {
    /// Reads only an image already retained by this live session generation.
    pub async fn preview_output_image(&self, owner: &str, id: &str, generation: &str, preview_id: &str) -> Result<crate::images::OutputImagePreview, String> {
        let (account, session, provider) = {
            let active = self.inner.active.lock().await;
            let entry = active.get(id).ok_or("image_preview_expired")?;
            (entry.account_lease.clone(), entry.lease.clone(), entry.provider_lease.cancellation_token())
        };
        if account.is_cancelled() || session.is_cancelled() || provider.is_cancelled() {
            return Err("image_preview_expired".into());
        }
        let _admission = self.admit_native_extensions(&account).await.map_err(preview_error)?;
        let (_, backend, _) = self.native_owned(owner, id, Some(generation), &account).await.map_err(preview_error)?;
        let preview = tokio::select! { biased;
            _ = account.cancelled() => return Err("image_preview_expired".into()),
            _ = session.cancelled() => return Err("image_preview_expired".into()),
            _ = provider.cancelled() => return Err("image_preview_expired".into()),
            result = backend.preview_output_image(preview_id) => result.map_err(preview_error)?,
        };
        // Account/provider revocation can race an in-memory read independently of admission.
        self.native_owned(owner, id, Some(generation), &account).await.map_err(preview_error)?;
        if account.is_cancelled() || session.is_cancelled() || provider.is_cancelled() {
            return Err("image_preview_expired".into());
        }
        Ok(preview)
    }
}

#[cfg(all(test, unix))]
#[path = "output_image_manager_tests.rs"]
mod tests;
