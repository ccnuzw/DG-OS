use dgos_db::sea_orm::DatabaseConnection;
use dgos_db::{establish_connection, ModelRepository, ProviderRepository, UsageLogRepository};
use dgos_types::{
    build_model, recompute_models, valid_model_id, ChatRequest, ChatResponse, DgosError,
    DgosResult, ExecutionProvenance, Model, ModelKind, ModelSource, Provider, ProviderConfig,
    RetryConfig, ToolChatRequest, ToolChatResponse, UsageLog, PROTOCOL_ANTHROPIC,
};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::adapters::anthropic::AnthropicAdapter;
use crate::adapters::openai::OpenAIAdapter;
use crate::credentials::{
    new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, SecretValue,
    UnavailableCredentialVault,
};
use crate::traits::ProviderAdapter;
use crate::transport::AdapterOptions;
use crate::vendor::validate_provider_config;
use dgos_db::repositories::provider_repository::CredentialCleanup;

/// Provider manager - main entry point for provider operations
pub struct ProviderManager {
    #[allow(dead_code)]
    db: DatabaseConnection,
    provider_repo: ProviderRepository,
    model_repo: ModelRepository,
    usage_log_repo: UsageLogRepository,
    vault: Arc<dyn ProviderCredentialVault>,
    // Serialize credential pointer changes and their outbox cleanup. Network
    // calls release this lock after obtaining their immutable adapter snapshot.
    credentials_lock: Mutex<()>,
    /// Built once and cloned into every adapter (clones share one connection pool). Building a
    /// client per call stalled for about 3 s on macOS, outside every timeout and while
    /// `credentials_lock` was held. Built with the default connect timeout, which is the only
    /// one `create_adapter` uses.
    http: Option<reqwest::Client>,
}

impl ProviderManager {
    /// Create a new provider manager
    pub async fn new(database_url: &str) -> DgosResult<Self> {
        Self::with_vault(database_url, Arc::new(UnavailableCredentialVault)).await
    }

    pub async fn with_vault(
        database_url: &str,
        vault: Arc<dyn ProviderCredentialVault>,
    ) -> DgosResult<Self> {
        let db = establish_connection(database_url)
            .await
            .map_err(|e| DgosError::DatabaseError(e.to_string()))?;

        let provider_repo = ProviderRepository::new(db.clone());
        let model_repo = ModelRepository::new(db.clone());
        let usage_log_repo = UsageLogRepository::new(db.clone());

        Ok(Self {
            db,
            provider_repo,
            model_repo,
            usage_log_repo,
            vault,
            credentials_lock: Mutex::new(()),
            http: crate::transport::http_client(AdapterOptions::default().connect_timeout),
        })
    }

    /// Create a new provider (without fetching models).
    ///
    /// A service without a model list endpoint (404 or 405) can only be verified with a probe
    /// request: without `probe_model` that case is `request_not_supported` and nothing is saved;
    /// with one, the provider is saved only if the probe succeeds (§2.4.5).
    pub async fn create_provider(
        &self,
        config: ProviderConfig,
        user_id: String,
        probe_model: Option<&str>,
    ) -> DgosResult<Provider> {
        info!("Creating provider");

        let mut config = config;
        prepare_config(&mut config)?;
        SecretValue::new(config.api_key.clone())?;
        self.verify_for_create(&config, probe_model).await?;

        self.persist_new(config, user_id, vec![]).await
    }

    /// Create a new provider and save models.
    ///
    /// Only `model_id` and `model_name` of the submitted models are used; identity, kind,
    /// capability hints and lengths are derived here (B01-67, AC35). An invalid id rejects the
    /// whole call before any credential or network step. `probe_model` works as in
    /// [`create_provider`](Self::create_provider).
    pub async fn create_provider_with_models(
        &self,
        config: ProviderConfig,
        user_id: String,
        models: Vec<Model>,
        probe_model: Option<&str>,
    ) -> DgosResult<Provider> {
        info!(model_count = models.len(), "Creating provider with models");

        let mut config = config;
        prepare_config(&mut config)?;
        let models = recompute_models(&config.provider_type, &models)?;
        SecretValue::new(config.api_key.clone())?;
        self.verify_for_create(&config, probe_model).await?;

        self.persist_new(config, user_id, models).await
    }

    /// Creation needs a reachable service and a key it accepts. Every failure is returned as is,
    /// including `request_not_supported` for a service that has no model list endpoint when no
    /// probe model was given: nothing is saved on an unverified endpoint.
    async fn verify_for_create(
        &self,
        config: &ProviderConfig,
        probe_model: Option<&str>,
    ) -> DgosResult<()> {
        self.create_adapter(config)?
            .test_connection(probe_model)
            .await
            .map(|_| ())
    }

    /// Adds a `source = manual` model. Idempotent per `(provider, model_id)`; `kind` defaults to
    /// the classification rules and is accepted only in its canonical spelling.
    pub async fn add_manual_model(
        &self,
        provider_id: &str,
        user_id: &str,
        model_id: &str,
        kind: Option<&str>,
    ) -> DgosResult<Model> {
        let model_id = model_id.trim();
        if !valid_model_id(model_id) {
            return Err(DgosError::RequestNotSupported);
        }
        let kind = match kind {
            Some(kind) => Some(ModelKind::parse(kind).ok_or(DgosError::RequestNotSupported)?),
            None => None,
        };
        let provider = self.owned_provider(provider_id, user_id).await?;
        let model = build_model(
            &provider.provider_type,
            model_id,
            None,
            ModelSource::Manual,
            kind,
        );
        self.provider_repo
            .add_manual_model(provider_id, user_id, model)
            .await
    }

    /// Removes a `source = manual` model; a missing or remote one is a successful no-op.
    pub async fn remove_manual_model(
        &self,
        provider_id: &str,
        user_id: &str,
        model_id: &str,
    ) -> DgosResult<()> {
        self.provider_repo
            .remove_manual_model(provider_id, user_id, model_id.trim())
            .await
    }

    /// Get a provider by ID
    pub async fn get_provider(&self, id: &str, user_id: &str) -> DgosResult<Option<Provider>> {
        self.provider_repo.get(id, user_id).await
    }

    /// List all providers for a user
    pub async fn list_providers(&self, user_id: &str) -> DgosResult<Vec<Provider>> {
        self.provider_repo.list(user_id).await
    }

    /// Update a provider
    pub async fn update_provider(
        &self,
        id: &str,
        config: ProviderConfig,
        user_id: &str,
    ) -> DgosResult<Provider> {
        info!("Updating provider: {}", id);
        let _guard = self.credentials_lock.lock().await;
        let current = self.owned_provider(id, user_id).await?;
        let mut config = config;
        prepare_config(&mut config)?;
        // A stored URL that differs from the submitted one only by a trailing slash is the same
        // endpoint and must not demand a new credential.
        if config.base_url == current.base_url.trim_end_matches('/') {
            config.base_url = current.base_url.clone();
        }
        if config.api_key.is_empty() {
            if config.base_url != current.base_url || config.provider_type != current.provider_type
            {
                return Err(DgosError::CredentialRequired);
            }
            let result = self.provider_repo.update(id, config, user_id).await;
            self.cleanup_unlocked().await;
            return result;
        }
        let scope = scope_for(user_id, id, &config.base_url, new_credential_ref());
        self.store_verified(&scope, std::mem::take(&mut config.api_key))
            .await?;
        let result = self
            .provider_repo
            .update_with_credential_ref(id, config, user_id, Some(scope.credential_ref))
            .await;
        self.cleanup_unlocked().await;
        result
    }

    /// Delete a provider
    pub async fn delete_provider(&self, id: &str, user_id: &str) -> DgosResult<()> {
        info!("Deleting provider: {}", id);

        let _guard = self.credentials_lock.lock().await;
        self.provider_repo.delete(id, user_id).await?;
        self.cleanup_unlocked().await;
        Ok(())
    }

    /// Update provider sort order
    pub async fn update_provider_order(
        &self,
        provider_ids: Vec<String>,
        user_id: &str,
    ) -> DgosResult<()> {
        self.provider_repo.update_order(provider_ids, user_id).await
    }

    /// Toggle provider enabled status
    pub async fn toggle_provider(
        &self,
        id: &str,
        enabled: bool,
        user_id: &str,
    ) -> DgosResult<Provider> {
        let _guard = self.credentials_lock.lock().await;
        self.provider_repo
            .toggle_enabled(id, enabled, user_id)
            .await
    }

    /// Test provider connection. `probe_model` is used only when the service has no model list
    /// endpoint; the probe sends one small request and may cost a few tokens (§2.4.5).
    pub async fn test_connection(
        &self,
        config: &ProviderConfig,
        probe_model: Option<&str>,
    ) -> DgosResult<bool> {
        let adapter = self.create_adapter(config)?;
        adapter.test_connection(probe_model).await
    }

    /// List models for an unsaved provider configuration.
    ///
    /// This is intentionally kept separate from `refresh_models`: the latter
    /// reads a persisted provider and updates its model cache, while this
    /// method is used by the create form to preview models before saving.
    pub async fn list_models_for_config(&self, config: &ProviderConfig) -> DgosResult<Vec<Model>> {
        let adapter = self.create_adapter(config)?;
        adapter.list_models().await
    }

    /// List models for a provider
    pub async fn list_models(&self, provider_id: &str, user_id: &str) -> DgosResult<Vec<Model>> {
        self.owned_provider(provider_id, user_id).await?;
        self.model_repo.list(provider_id).await
    }

    /// Refresh models from provider API
    pub async fn refresh_models(&self, provider_id: &str, user_id: &str) -> DgosResult<Vec<Model>> {
        info!("Refreshing models for provider: {}", provider_id);
        let (ticket, adapter) = {
            let _guard = self.credentials_lock.lock().await;
            let ticket = self
                .provider_repo
                .begin_model_refresh(provider_id, user_id)
                .await?;
            let adapter = self.adapter_for_snapshot(&ticket.provider, false).await?;
            (ticket, adapter)
        };
        let models = adapter.list_models().await?;
        self.provider_repo
            .publish_model_refresh(&ticket, models)
            .await
    }

    /// Call a provider (synchronous)
    pub async fn call_provider(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
    ) -> DgosResult<ChatResponse> {
        self.call_provider_bound(provider_id, model_id, request, user_id, None)
            .await
    }

    /// An expected revision is supplied by the trusted task binding, never a
    /// replacement identity or authorization token from a model/renderer.
    pub async fn call_provider_bound(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
        expected_revision: Option<i64>,
    ) -> DgosResult<ChatResponse> {
        self.call_provider_bound_with_provenance(
            provider_id,
            model_id,
            request,
            user_id,
            expected_revision,
            None,
        )
        .await
    }

    pub async fn call_provider_bound_with_provenance(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
        expected_revision: Option<i64>,
        provenance: Option<&ExecutionProvenance>,
    ) -> DgosResult<ChatResponse> {
        let start = Instant::now();

        let provider = self
            .provider_repo
            .get(provider_id, user_id)
            .await?
            .ok_or_else(|| DgosError::ProviderNotFound(provider_id.to_string()))?;

        if !provider.enabled {
            return Err(DgosError::ProviderUnavailable(
                "Provider is disabled".to_string(),
            ));
        }

        let adapter = self
            .adapter_for_saved(&provider, true, expected_revision)
            .await?;
        let mut request = request;
        self.fill_max_tokens(adapter.as_ref(), provider_id, model_id, &mut request)
            .await;

        let mut result = adapter
            .call_with_usage(model_id, request)
            .await
            .map_err(|failure| (failure.error, failure.usage));
        let latency_ms = start.elapsed().as_millis() as u32;

        // Log usage. A failure normally cost nothing (0 tokens); one the upstream answered and
        // the host then rejected carries the upstream's own numbers.
        match &result {
            Ok(response) => {
                let (prompt, completion) = (
                    response.usage.prompt_tokens,
                    response.usage.completion_tokens,
                );
                if let Err(audit_error) = self
                    .usage_log_repo
                    .create_with_provenance(
                        provider_id,
                        model_id,
                        user_id,
                        prompt,
                        completion,
                        prompt > 0 || completion > 0,
                        latency_ms,
                        true,
                        None,
                        provenance,
                    )
                    .await
                {
                    warn!(provider_id, model_id, error = %audit_error, "Provider succeeded but usage audit write failed");
                    if let Ok(response) = &mut result {
                        response.audit_warning = true;
                    }
                }
            }
            Err((e, usage)) => {
                let (prompt, completion) = usage.as_ref().map_or((0, 0), |usage| {
                    (usage.prompt_tokens, usage.completion_tokens)
                });
                if let Err(audit_error) = self
                    .usage_log_repo
                    .create_with_provenance(
                        provider_id,
                        model_id,
                        user_id,
                        prompt,
                        completion,
                        prompt > 0 || completion > 0,
                        latency_ms,
                        false,
                        Some(e.error_code().to_string()),
                        provenance,
                    )
                    .await
                {
                    warn!(provider_id, model_id, error = %audit_error, provider_error = %e, "Provider failed and usage audit write failed");
                }
            }
        }

        result.map_err(|(error, _)| error)
    }

    /// Structured tool proposals are data; this method never executes a tool.
    pub async fn call_provider_tools(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ToolChatRequest,
        user_id: &str,
    ) -> DgosResult<ToolChatResponse> {
        self.call_provider_tools_bound(provider_id, model_id, request, user_id, None)
            .await
    }

    pub async fn call_provider_tools_bound(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ToolChatRequest,
        user_id: &str,
        expected_revision: Option<i64>,
    ) -> DgosResult<ToolChatResponse> {
        self.call_provider_tools_bound_with_provenance(
            provider_id,
            model_id,
            request,
            user_id,
            expected_revision,
            None,
        )
        .await
    }

    pub async fn call_provider_tools_bound_with_provenance(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ToolChatRequest,
        user_id: &str,
        expected_revision: Option<i64>,
        provenance: Option<&ExecutionProvenance>,
    ) -> DgosResult<ToolChatResponse> {
        let provider = self
            .provider_repo
            .get(provider_id, user_id)
            .await?
            .filter(|p| p.enabled && p.user_id == user_id)
            .ok_or_else(|| {
                DgosError::ProviderUnavailable("Provider unavailable for this user".into())
            })?;
        let adapter = self
            .adapter_for_saved(&provider, true, expected_revision)
            .await?;
        let start = Instant::now();
        let mut result = adapter.call_tools(model_id, request).await;
        let (input, output, error) = match &result {
            Ok(response) => (
                response.usage.prompt_tokens,
                response.usage.completion_tokens,
                None,
            ),
            Err(error) => (0, 0, Some(error.error_code().to_string())),
        };
        if let Err(audit_error) = self
            .usage_log_repo
            .create_with_provenance(
                provider_id,
                model_id,
                user_id,
                input,
                output,
                input > 0 || output > 0,
                start.elapsed().as_millis() as u32,
                result.is_ok(),
                error,
                provenance,
            )
            .await
        {
            warn!(provider_id, model_id, error = %audit_error, "Provider tools response audit write failed");
            if let Ok(response) = &mut result {
                response.audit_warning = true;
            }
        }
        result
    }

    /// Stream from a provider
    pub async fn stream_provider(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
    ) -> DgosResult<crate::traits::BoxStream<crate::ChatChunk>> {
        self.stream_provider_bound(provider_id, model_id, request, user_id, None)
            .await
    }

    /// Start a stream from the saved provider snapshot. Callers that have
    /// already bound a task to a provider revision must pass that revision so
    /// a rotation cannot silently switch the stream to a new credential or
    /// endpoint.
    pub async fn stream_provider_bound(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
        expected_revision: Option<i64>,
    ) -> DgosResult<crate::traits::BoxStream<crate::ChatChunk>> {
        self.stream_provider_bound_with_provenance(
            provider_id,
            model_id,
            request,
            user_id,
            expected_revision,
            None,
        )
        .await
    }

    /// Same as [`stream_provider_bound`](Self::stream_provider_bound); the stream's one usage
    /// record carries the caller and request id like a synchronous call's does (AC33).
    pub async fn stream_provider_bound_with_provenance(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
        expected_revision: Option<i64>,
        provenance: Option<&ExecutionProvenance>,
    ) -> DgosResult<crate::traits::BoxStream<crate::ChatChunk>> {
        let provider = self
            .provider_repo
            .get(provider_id, user_id)
            .await?
            .ok_or_else(|| DgosError::ProviderNotFound(provider_id.to_string()))?;

        if !provider.enabled {
            return Err(DgosError::ProviderUnavailable(
                "Provider is disabled".to_string(),
            ));
        }

        let adapter = self
            .adapter_for_saved(&provider, true, expected_revision)
            .await?;
        let mut request = request;
        self.fill_max_tokens(adapter.as_ref(), provider_id, model_id, &mut request)
            .await;
        let started = Instant::now();
        match adapter.stream(model_id, request).await {
            Ok(stream) => {
                let audit = StreamAudit {
                    db: self.db.clone(),
                    provider_id: provider_id.to_owned(),
                    model_id: model_id.to_owned(),
                    user_id: user_id.to_owned(),
                    provenance: provenance.cloned(),
                    started,
                    finished: false,
                };
                Ok(audited_stream(stream, audit))
            }
            Err(error) => {
                write_stream_log(
                    &self.db,
                    provider_id,
                    model_id,
                    user_id,
                    provenance,
                    started,
                    StreamOutcome::Failed(error.error_code()),
                )
                .await;
                Err(error)
            }
        }
    }

    /// Stream with host-owned cancellation. Dropping the adapter stream after
    /// cancellation closes the underlying HTTP response and emits one stable
    /// interruption error; callers must persist any already-received content.
    pub async fn stream_provider_bound_cancellable(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
        expected_revision: Option<i64>,
        cancellation: CancellationToken,
    ) -> DgosResult<crate::traits::BoxStream<crate::ChatChunk>> {
        self.stream_provider_bound_cancellable_with_provenance(
            provider_id,
            model_id,
            request,
            user_id,
            expected_revision,
            None,
            cancellation,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn stream_provider_bound_cancellable_with_provenance(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
        expected_revision: Option<i64>,
        provenance: Option<&ExecutionProvenance>,
        cancellation: CancellationToken,
    ) -> DgosResult<crate::traits::BoxStream<crate::ChatChunk>> {
        let stream = self
            .stream_provider_bound_with_provenance(
                provider_id,
                model_id,
                request,
                user_id,
                expected_revision,
                provenance,
            )
            .await?;
        let stream = cancellable_stream(stream, cancellation);
        Ok(Box::pin(stream))
    }

    /// List usage logs
    pub async fn list_usage_logs(
        &self,
        user_id: &str,
        limit: Option<usize>,
    ) -> DgosResult<Vec<UsageLog>> {
        self.usage_log_repo.list(user_id, limit).await
    }

    async fn owned_provider(&self, provider_id: &str, user_id: &str) -> DgosResult<Provider> {
        self.provider_repo
            .get(provider_id, user_id)
            .await?
            .ok_or_else(|| DgosError::ProviderNotFound(provider_id.into()))
    }

    /// Load the current credential into an in-flight adapter, never a long-lived cache.
    async fn adapter_for_saved(
        &self,
        provider: &Provider,
        require_enabled: bool,
        expected_revision: Option<i64>,
    ) -> DgosResult<Arc<dyn ProviderAdapter>> {
        let _guard = self.credentials_lock.lock().await;
        // Re-read after acquiring the pointer lock; an earlier query may have
        // preceded an endpoint change or a credential rotation.
        let provider = self.owned_provider(&provider.id, &provider.user_id).await?;
        if expected_revision.is_some_and(|revision| provider.config_revision != revision) {
            return Err(DgosError::ProviderConfigurationChanged);
        }
        self.adapter_for_snapshot(&provider, require_enabled).await
    }

    async fn adapter_for_snapshot(
        &self,
        provider: &Provider,
        require_enabled: bool,
    ) -> DgosResult<Arc<dyn ProviderAdapter>> {
        if require_enabled && !provider.enabled {
            return Err(DgosError::ProviderUnavailable(
                "Provider is disabled".into(),
            ));
        }
        let reference = provider
            .credential_ref
            .as_ref()
            .ok_or(DgosError::CredentialUnavailable)?;
        let secret = self
            .vault
            .load(&scope_for(
                &provider.user_id,
                &provider.id,
                &provider.base_url,
                reference.clone(),
            ))
            .await?;
        let config = ProviderConfig {
            name: provider.name.clone(),
            provider_type: provider.provider_type.clone(),
            base_url: provider.base_url.clone(),
            api_key: secret.expose().to_owned(),
            enabled: provider.enabled,
            config: provider.config.clone(),
        };

        self.create_adapter(&config)
    }

    /// `probe_model` is used only when the service has no model list endpoint (§2.4.5).
    pub async fn test_provider_by_id(
        &self,
        id: &str,
        user_id: &str,
        probe_model: Option<&str>,
    ) -> DgosResult<bool> {
        let provider = self.owned_provider(id, user_id).await?;
        self.adapter_for_saved(&provider, false, None)
            .await?
            .test_connection(probe_model)
            .await
    }

    async fn persist_new(
        &self,
        mut config: ProviderConfig,
        user_id: String,
        models: Vec<Model>,
    ) -> DgosResult<Provider> {
        let _guard = self.credentials_lock.lock().await;
        let id = uuid::Uuid::new_v4().to_string();
        let scope = scope_for(&user_id, &id, &config.base_url, new_credential_ref());
        self.store_verified(&scope, std::mem::take(&mut config.api_key))
            .await?;
        let result = self
            .provider_repo
            .create_with_models(id, config, user_id, Some(scope.credential_ref), models)
            .await;
        self.cleanup_unlocked().await;
        result
    }

    pub async fn model_snapshot(
        &self,
        provider_id: &str,
        model_id: &str,
        user_id: &str,
    ) -> DgosResult<(Provider, Model)> {
        let snapshot = self
            .provider_repo
            .model_snapshot(provider_id, user_id, model_id)
            .await?;
        if !snapshot.0.enabled {
            return Err(DgosError::ProviderUnavailable(
                "Provider is disabled".into(),
            ));
        }
        Ok(snapshot)
    }

    pub async fn ensure_revision(
        &self,
        provider_id: &str,
        user_id: &str,
        expected_revision: i64,
    ) -> DgosResult<()> {
        let provider = self.owned_provider(provider_id, user_id).await?;
        if !provider.enabled || provider.config_revision != expected_revision {
            return Err(DgosError::ProviderConfigurationChanged);
        }
        Ok(())
    }

    async fn store_verified(
        &self,
        scope: &ProviderCredentialScope,
        value: String,
    ) -> DgosResult<()> {
        let secret = SecretValue::new(value)?;
        let expected = SecretValue::new(secret.expose().to_owned())?;
        self.provider_repo
            .stage_credential_cleanup(&cleanup_for(scope))
            .await?;
        let result = async {
            self.vault.store(scope, secret).await?;
            let loaded = self.vault.load(scope).await?;
            if loaded.expose() != expected.expose() {
                return Err(DgosError::CredentialUnavailable);
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            self.cleanup_unlocked().await;
        }
        result
    }

    /// Only explicit migration uses the legacy value as a credential; normal
    /// consumption authenticates from the vault. Failure leaves the row intact.
    pub async fn migrate_provider_credential(
        &self,
        id: &str,
        user_id: &str,
    ) -> DgosResult<Provider> {
        let _guard = self.credentials_lock.lock().await;
        let provider = self.owned_provider(id, user_id).await?;
        if provider.credential_ref.is_some() {
            return Ok(provider);
        }
        let legacy = self
            .provider_repo
            .legacy_api_key(id, user_id)
            .await?
            .ok_or(DgosError::CredentialRequired)?;
        let scope = scope_for(user_id, id, &provider.base_url, new_credential_ref());
        self.store_verified(&scope, legacy).await?;
        let result = self
            .provider_repo
            .commit_credential_migration(&provider, &scope.credential_ref)
            .await;
        self.cleanup_unlocked().await;
        result
    }

    pub async fn recover_credentials(&self, user_id: &str) -> DgosResult<()> {
        {
            let _guard = self.credentials_lock.lock().await;
            self.cleanup_unlocked().await;
        }
        for provider in self.provider_repo.list(user_id).await? {
            if provider.credential_status == dgos_types::CredentialStatus::MigrationRequired
                && self
                    .migrate_provider_credential(&provider.id, user_id)
                    .await
                    .is_err()
            {
                tracing::warn!(provider_id = %provider.id, "Provider credential migration pending; retry from Settings");
            }
        }
        Ok(())
    }

    async fn cleanup_unlocked(&self) {
        let entries = match self.provider_repo.pending_credential_cleanup().await {
            Ok(entries) => entries,
            Err(_) => {
                tracing::warn!("Provider credential cleanup queue unavailable");
                return;
            }
        };
        for entry in entries {
            match self
                .provider_repo
                .credential_is_active(&entry.credential_ref)
                .await
            {
                Ok(false) => {}
                _ => continue,
            }
            let scope = scope_for(
                &entry.user_id,
                &entry.provider_id,
                &entry.endpoint,
                entry.credential_ref.clone(),
            );
            if self.vault.remove(&scope).await.is_ok() {
                if self
                    .provider_repo
                    .acknowledge_credential_cleanup(&entry.credential_ref)
                    .await
                    .is_err()
                {
                    tracing::warn!("Provider credential cleanup acknowledgement pending");
                }
            } else {
                tracing::warn!("Provider credential cleanup pending");
            }
        }
    }

    /// `max_tokens` for protocols that require it: the caller's value, else the model's
    /// `max_output_tokens`; the adapter's profile default applies when neither exists (AC18).
    async fn fill_max_tokens(
        &self,
        adapter: &dyn ProviderAdapter,
        provider_id: &str,
        model_id: &str,
        request: &mut ChatRequest,
    ) {
        if !adapter.requires_max_tokens() || request.max_tokens.is_some() {
            return;
        }
        if let Ok(Some(model)) = self.model_repo.get(provider_id, model_id).await {
            request.max_tokens = model
                .max_output_tokens
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0);
        }
    }

    /// Create an adapter from config. Vendor, options and endpoint policy are checked here too,
    /// so a configuration saved earlier is refused before any request is sent (AC32). The
    /// profile is resolved from the address, then the adapter is chosen by `provider_type`.
    fn create_adapter(&self, config: &ProviderConfig) -> DgosResult<Arc<dyn ProviderAdapter>> {
        let normalized = validate_provider_config(
            &config.provider_type,
            &config.base_url,
            config.config.as_ref(),
        )?;
        let retry_config = config
            .config
            .as_ref()
            .and_then(|c| serde_json::from_value::<RetryConfig>(c.clone()).ok());
        let options = AdapterOptions {
            allow_insecure_http: normalized.allow_insecure_http,
            auth_style: normalized.auth_override,
            workspace_id: normalized.workspace_id.clone(),
            ..Default::default()
        };
        let adapter: Arc<dyn ProviderAdapter> = if normalized.provider_type == PROTOCOL_ANTHROPIC {
            Arc::new(AnthropicAdapter::with_client(
                self.http.clone(),
                normalized.base_url,
                config.api_key.clone(),
                retry_config,
                options,
                normalized.profile,
            ))
        } else {
            Arc::new(OpenAIAdapter::with_client(
                self.http.clone(),
                normalized.base_url,
                config.api_key.clone(),
                retry_config,
                options,
                normalized.profile,
            ))
        };
        Ok(adapter)
    }
}

/// Save-time checks shared by create and update: adapter, vendor, option values, endpoint
/// policy. Normalises `base_url` (trailing slash removed; a trailing `/v1` removed for
/// `anthropic`).
fn prepare_config(config: &mut ProviderConfig) -> DgosResult<()> {
    let normalized = validate_provider_config(
        &config.provider_type,
        &config.base_url,
        config.config.as_ref(),
    )?;
    config.base_url = normalized.base_url;
    Ok(())
}

enum StreamOutcome {
    /// `usage` is `None` when the upstream ended the stream without reporting usage.
    Done {
        usage: Option<(u32, u32)>,
    },
    Failed(u32),
}

async fn write_stream_log(
    db: &DatabaseConnection,
    provider_id: &str,
    model_id: &str,
    user_id: &str,
    provenance: Option<&ExecutionProvenance>,
    started: Instant,
    outcome: StreamOutcome,
) {
    // A stream that fails or is cancelled never has usage (D081): the upstream may already
    // have produced and billed output, so the zeros are recorded as unknown, not as free.
    let (prompt, completion, reported, error_code) = match outcome {
        StreamOutcome::Done {
            usage: Some((prompt, completion)),
        } => (prompt, completion, prompt > 0 || completion > 0, None),
        StreamOutcome::Done { usage: None } => (0, 0, false, None),
        StreamOutcome::Failed(code) => (0, 0, false, Some(code.to_string())),
    };
    if let Err(error) = UsageLogRepository::new(db.clone())
        .create_with_provenance(
            provider_id,
            model_id,
            user_id,
            prompt,
            completion,
            reported,
            started.elapsed().as_millis() as u32,
            error_code.is_none(),
            error_code,
            provenance,
        )
        .await
    {
        warn!(provider_id, model_id, error = %error, "Stream usage audit write failed");
    }
}

/// Writes the one usage record of a stream (B01-63). The record is written when the stream ends
/// or fails. A stream dropped before that (consumer cancelled or went away) is recorded as
/// interrupted from `Drop`, on the runtime.
struct StreamAudit {
    db: DatabaseConnection,
    provider_id: String,
    model_id: String,
    user_id: String,
    provenance: Option<ExecutionProvenance>,
    started: Instant,
    finished: bool,
}

impl StreamAudit {
    async fn finish(&mut self, outcome: StreamOutcome) {
        if std::mem::replace(&mut self.finished, true) {
            return;
        }
        write_stream_log(
            &self.db,
            &self.provider_id,
            &self.model_id,
            &self.user_id,
            self.provenance.as_ref(),
            self.started,
            outcome,
        )
        .await;
    }
}

impl Drop for StreamAudit {
    fn drop(&mut self) {
        if std::mem::replace(&mut self.finished, true) {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            warn!("Stream dropped outside a runtime; usage record skipped");
            return;
        };
        let (db, provider_id, model_id, user_id, provenance, started) = (
            self.db.clone(),
            self.provider_id.clone(),
            self.model_id.clone(),
            self.user_id.clone(),
            self.provenance.clone(),
            self.started,
        );
        runtime.spawn(async move {
            write_stream_log(
                &db,
                &provider_id,
                &model_id,
                &user_id,
                provenance.as_ref(),
                started,
                StreamOutcome::Failed(DgosError::StreamInterrupted.error_code()),
            )
            .await;
        });
    }
}

fn audited_stream(
    stream: crate::traits::BoxStream<crate::ChatChunk>,
    audit: StreamAudit,
) -> crate::traits::BoxStream<crate::ChatChunk> {
    Box::pin(futures::stream::unfold(
        (stream, audit),
        |(mut stream, mut audit)| async move {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    // Only the terminal chunk carries a finish reason.
                    if chunk.finish_reason.is_some() {
                        let usage = chunk
                            .usage
                            .as_ref()
                            .map(|usage| (usage.prompt_tokens, usage.completion_tokens));
                        audit.finish(StreamOutcome::Done { usage }).await;
                    }
                    Some((Ok(chunk), (stream, audit)))
                }
                Some(Err(error)) => {
                    audit
                        .finish(StreamOutcome::Failed(error.error_code()))
                        .await;
                    Some((Err(error), (stream, audit)))
                }
                None => {
                    // Ended without a terminal item: nothing completed it.
                    audit
                        .finish(StreamOutcome::Failed(
                            DgosError::StreamInterrupted.error_code(),
                        ))
                        .await;
                    None
                }
            }
        },
    ))
}

fn cancellable_stream(
    stream: crate::traits::BoxStream<crate::ChatChunk>,
    cancellation: CancellationToken,
) -> impl futures::Stream<Item = DgosResult<crate::ChatChunk>> + Send {
    futures::stream::unfold(
        (stream, cancellation, false),
        |(mut stream, cancellation, emitted_interrupt)| async move {
            if emitted_interrupt {
                return None;
            }
            tokio::select! {
                item = stream.next() => item.map(|item| (item, (stream, cancellation, false))),
                _ = cancellation.cancelled() => Some((Err(DgosError::StreamInterrupted), (stream, cancellation, true))),
            }
        },
    )
}

fn scope_for(
    user_id: &str,
    provider_id: &str,
    endpoint: &str,
    credential_ref: String,
) -> ProviderCredentialScope {
    ProviderCredentialScope {
        user_id: user_id.into(),
        provider_id: provider_id.into(),
        endpoint: endpoint.into(),
        credential_ref,
    }
}

fn cleanup_for(scope: &ProviderCredentialScope) -> CredentialCleanup {
    CredentialCleanup {
        user_id: scope.user_id.clone(),
        provider_id: scope.provider_id.clone(),
        endpoint: scope.endpoint.clone(),
        credential_ref: scope.credential_ref.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::cancellable_stream;
    use crate::traits::BoxStream;
    use dgos_types::{ChatChunk, DgosError};
    use futures::{stream, StreamExt};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn cancellable_stream_emits_one_interruption_without_replay() {
        let cancellation = CancellationToken::new();
        let source: BoxStream<ChatChunk> = Box::pin(stream::pending());
        let output = cancellable_stream(source, cancellation.clone());
        futures::pin_mut!(output);
        cancellation.cancel();
        assert!(matches!(
            output.next().await,
            Some(Err(DgosError::StreamInterrupted))
        ));
        assert!(output.next().await.is_none());
    }
}
