//! Host-only provider snapshots for explicitly configured, trusted CLI processes.
//!
//! This module resolves configuration; it neither launches a process nor proxies a protocol.
use super::{scope_for, ProviderManager};
use crate::{vendor::validate_provider_config, AuthStyle, SecretValue};
use dgos_types::{DgosError, DgosResult, Model, ModelKind, PROTOCOL_ANTHROPIC, PROTOCOL_OPENAI_COMPATIBLE};

/// Wire protocol a CLI engine may probe. A candidate is never proof of compatibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliProviderProtocol {
    /// Codex requires Responses; the existing OpenAI chat adapter does not verify it.
    OpenAiResponses,
    /// Claude requires Anthropic Messages.
    AnthropicMessages,
}

impl CliProviderProtocol {
    /// Map the existing persisted provider kind (`Provider.provider_type`) to a candidate.
    ///
    /// `openai_compatible` describes the chat adapter, not Responses support. Even an
    /// official host or a successful chat call does not promote this candidate to verified.
    pub fn candidate_for_provider_type(provider_type: &str) -> Option<Self> {
        match provider_type {
            PROTOCOL_OPENAI_COMPATIBLE => Some(Self::OpenAiResponses),
            PROTOCOL_ANTHROPIC => Some(Self::AnthropicMessages),
            _ => None,
        }
    }
}

/// Non-secret, revision-bound configuration. It deliberately excludes arbitrary provider
/// options and vault references. `base_url` is a normalized base, never a chat/completions URL
/// synthesized for Responses. The CLI's own verified protocol determines the request path.
#[derive(Clone, Debug)]
pub struct CliProviderLaunchMetadata {
    pub user_id: String,
    pub provider_id: String,
    pub provider_type: String,
    pub config_revision: i64,
    pub model: Model,
    pub base_url: String,
    pub protocol_candidate: CliProviderProtocol,
    /// False means no engine-specific real-service proof, not an admission failure.
    pub protocol_verified: bool,
    pub auth_style: AuthStyle,
    pub allow_insecure_http: bool,
    pub anthropic_version: Option<&'static str>,
    pub workspace_id: Option<String>,
}

/// An immutable credential snapshot for the trusted desktop host and its explicitly authorized CLI.
///
/// This type intentionally implements neither `Serialize`, `Debug`, nor `Clone`. Never place it,
/// `credential().expose()`, or process configuration containing that value into a renderer DTO,
/// IPC response, log, command-line argument, repository file, or shared/global CLI configuration.
/// Pass the secret only through the host's controlled process environment or private temporary
/// configuration, and drop this snapshot after spawning. `SecretValue` zeroizes its own storage;
/// the host is responsible for any copies it makes and for the child process environment.
///
/// Resolution checks ownership, enabled state, model availability and the exact revision while
/// holding the manager's credential pointer lock. This snapshot alone is not a process lease.
/// A managed CLI host must register_cli_opening, resolve_cli_provider_for_lease, and use
/// with_cli_spawn for its final synchronous spawn boundary, retaining the lease until exit.
/// A protocol candidate is not compatibility evidence; the host must support the adapter and
/// confirm the effective route. No per-account paid compatibility probe is required.
///
/// ```compile_fail
/// fn cannot_serialize(launch: &dgos_providers::CliProviderLaunch) {
///     let _ = serde_json::to_string(launch);
/// }
/// ```
///
/// ```compile_fail
/// fn cannot_debug(launch: &dgos_providers::CliProviderLaunch) {
///     let _ = format!("{launch:?}");
/// }
/// ```
pub struct CliProviderLaunch {
    metadata: CliProviderLaunchMetadata,
    credential: SecretValue,
}

impl CliProviderLaunch {
    pub fn metadata(&self) -> &CliProviderLaunchMetadata {
        &self.metadata
    }

    /// Expose only to trusted host process configuration code; never serialize or log the value.
    pub fn credential(&self) -> &SecretValue {
        &self.credential
    }
}

/// Compare only settings consumed by the native launch adapter. Retry policy and
/// unrelated metadata do not change a running process's route or authorization.
pub(super) fn execution_settings_changed(before: &dgos_types::Provider, after: &dgos_types::ProviderConfig) -> bool {
    let Ok(before) = validate_provider_config(&before.provider_type, &before.base_url, before.config.as_ref()) else { return true; };
    let Ok(after) = validate_provider_config(&after.provider_type, &after.base_url, after.config.as_ref()) else { return true; };
    before.provider_type != after.provider_type || before.base_url != after.base_url
        || before.auth_override.unwrap_or(before.profile.auth) != after.auth_override.unwrap_or(after.profile.auth)
        || before.allow_insecure_http != after.allow_insecure_http
        || before.profile.version_header != after.profile.version_header
        || before.workspace_id.as_ref().filter(|_| before.profile.official) != after.workspace_id.as_ref().filter(|_| after.profile.official)
}

impl ProviderManager {
    /// Resolve a saved provider for an explicitly authorized CLI launch, without any network call.
    ///
    /// `user_id` must come from the host session. `expected_revision` must be the exact revision
    /// bound by the host, never an optional fallback to the current configuration. Missing,
    /// disabled, rotated or revoked credentials fail closed; legacy plaintext is never loaded.
    /// Vault errors are reduced to a fixed credential error and are never echoed or logged.
    ///
    /// The selected cached model must exist and be text-generation (including legacy `chat`).
    /// Successful resolution does not attest to Responses/Messages compatibility. Call
    /// `launch.metadata()` for the candidate, endpoint and binding, and use
    /// `launch.credential().expose()` only when configuring the trusted child. See
    /// [`CliProviderLaunch`] for the credential and revocation boundary.
    pub async fn resolve_cli_provider_launch(
        &self,
        provider_id: &str,
        model_id: &str,
        user_id: &str,
        expected_revision: i64,
    ) -> DgosResult<CliProviderLaunch> {
        let _permit = self.gate.admit().map_err(|_| DgosError::ProviderRequestRejected)?;
        let _guard = self.credentials_lock.lock().await;
        // Check the revision before looking up the model: a rotation may have removed that model.
        self.ensure_revision(provider_id, user_id, expected_revision).await?;
        let (provider, model) = self.provider_repo.model_snapshot(provider_id, user_id, model_id).await?;
        if !provider.enabled || provider.config_revision != expected_revision {
            return Err(DgosError::ProviderConfigurationChanged);
        }
        if model.model_type != ModelKind::TextGeneration.as_str() && model.model_type != "chat" {
            return Err(DgosError::RequestNotSupported);
        }
        let normalized = validate_provider_config(
            &provider.provider_type,
            &provider.base_url,
            provider.config.as_ref(),
        )?;
        let protocol_candidate = CliProviderProtocol::candidate_for_provider_type(normalized.provider_type)
            .ok_or(DgosError::UnsupportedProviderType)?;
        let reference = provider.credential_ref.as_ref().ok_or(DgosError::CredentialUnavailable)?;
        let credential = self.vault.load(&scope_for(
            user_id,
            provider_id,
            &provider.base_url,
            reference.clone(),
        )).await.map_err(|_| DgosError::CredentialUnavailable)?;
        // Also detect another manager/connection changing SQLite while the vault was awaited.
        // Do not retry with a new reference or return the stale secret after a revocation.
        self.ensure_revision(provider_id, user_id, expected_revision).await?;
        Ok(CliProviderLaunch {
            metadata: CliProviderLaunchMetadata {
                user_id: provider.user_id,
                provider_id: provider.id,
                provider_type: provider.provider_type,
                config_revision: provider.config_revision,
                model,
                base_url: normalized.base_url,
                protocol_candidate,
                protocol_verified: false,
                auth_style: normalized.auth_override.unwrap_or(normalized.profile.auth),
                allow_insecure_http: normalized.allow_insecure_http,
                anthropic_version: normalized.profile.version_header,
                workspace_id: normalized.workspace_id.filter(|_| normalized.profile.official),
            },
            credential,
        })
    }
}
