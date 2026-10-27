use dgos_db::{establish_connection, ModelRepository, ProviderRepository, UsageLogRepository};
use dgos_db::sea_orm::DatabaseConnection;
use dgos_types::{ChatRequest, ChatResponse, DgosError, DgosResult, Model, Provider, ProviderConfig, RetryConfig, UsageLog};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::info;

use crate::adapters::openai::OpenAIAdapter;
use crate::traits::ProviderAdapter;

/// Provider manager - main entry point for provider operations
pub struct ProviderManager {
    #[allow(dead_code)]
    db: DatabaseConnection,
    provider_repo: ProviderRepository,
    model_repo: ModelRepository,
    usage_log_repo: UsageLogRepository,
    // Cache of instantiated adapters
    adapter_cache: Arc<RwLock<std::collections::HashMap<String, Arc<dyn ProviderAdapter>>>>,
}

impl ProviderManager {
    /// Create a new provider manager
    pub async fn new(database_url: &str) -> DgosResult<Self> {
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
            adapter_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Create a new provider
    pub async fn create_provider(
        &self,
        config: ProviderConfig,
        user_id: String,
    ) -> DgosResult<Provider> {
        info!("Creating provider: {}", config.name);
        
        // Test connection first
        let adapter = self.create_adapter(&config)?;
        adapter.test_connection().await?;

        // Save to database
        let provider = self.provider_repo.create(config, user_id).await?;

        // Fetch and cache models
        self.refresh_models_internal(&provider.id, adapter).await?;

        Ok(provider)
    }

    /// Get a provider by ID
    pub async fn get_provider(&self, id: &str) -> DgosResult<Option<Provider>> {
        self.provider_repo.get(id).await
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
    ) -> DgosResult<Provider> {
        info!("Updating provider: {}", id);
        
        // Clear cached adapter
        self.adapter_cache.write().await.remove(id);

        self.provider_repo.update(id, config).await
    }

    /// Delete a provider
    pub async fn delete_provider(&self, id: &str) -> DgosResult<()> {
        info!("Deleting provider: {}", id);
        
        // Clear cached adapter
        self.adapter_cache.write().await.remove(id);

        self.provider_repo.delete(id).await
    }

    /// Update provider sort order
    pub async fn update_provider_order(&self, provider_ids: Vec<String>) -> DgosResult<()> {
        self.provider_repo.update_order(provider_ids).await
    }

    /// Toggle provider enabled status
    pub async fn toggle_provider(&self, id: &str, enabled: bool) -> DgosResult<Provider> {
        self.provider_repo.toggle_enabled(id, enabled).await
    }

    /// Test provider connection
    pub async fn test_connection(&self, config: &ProviderConfig) -> DgosResult<bool> {
        let adapter = self.create_adapter(config)?;
        adapter.test_connection().await
    }

    /// List models for a provider
    pub async fn list_models(&self, provider_id: &str) -> DgosResult<Vec<Model>> {
        self.model_repo.list(provider_id).await
    }

    /// Refresh models from provider API
    pub async fn refresh_models(&self, provider_id: &str) -> DgosResult<Vec<Model>> {
        info!("Refreshing models for provider: {}", provider_id);
        
        let provider = self.provider_repo.get(provider_id).await?
            .ok_or_else(|| DgosError::ProviderNotFound(provider_id.to_string()))?;

        let adapter = self.get_or_create_adapter(&provider).await?;
        self.refresh_models_internal(provider_id, adapter).await
    }

    async fn refresh_models_internal(
        &self,
        provider_id: &str,
        adapter: Arc<dyn ProviderAdapter>,
    ) -> DgosResult<Vec<Model>> {
        let models = adapter.list_models().await?;
        self.model_repo.sync_models(provider_id, models).await
    }

    /// Call a provider (synchronous)
    pub async fn call_provider(
        &self,
        provider_id: &str,
        model_id: &str,
        request: ChatRequest,
        user_id: &str,
    ) -> DgosResult<ChatResponse> {
        let start = Instant::now();
        
        let provider = self.provider_repo.get(provider_id).await?
            .ok_or_else(|| DgosError::ProviderNotFound(provider_id.to_string()))?;

        if !provider.enabled {
            return Err(DgosError::ProviderUnavailable(
                "Provider is disabled".to_string(),
            ));
        }

        let adapter = self.get_or_create_adapter(&provider).await?;

        let result = adapter.call(model_id, request).await;
        let latency_ms = start.elapsed().as_millis() as u32;

        // Log usage
        match &result {
            Ok(response) => {
                self.usage_log_repo
                    .create(
                        provider_id,
                        model_id,
                        user_id,
                        response.usage.prompt_tokens,
                        response.usage.completion_tokens,
                        latency_ms,
                        true,
                        None,
                    )
                    .await?;
            }
            Err(e) => {
                self.usage_log_repo
                    .create(
                        provider_id,
                        model_id,
                        user_id,
                        0,
                        0,
                        latency_ms,
                        false,
                        Some(e.error_code().to_string()),
                    )
                    .await?;
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
        _user_id: &str,
    ) -> DgosResult<crate::traits::BoxStream<crate::ChatChunk>> {
        let provider = self.provider_repo.get(provider_id).await?
            .ok_or_else(|| DgosError::ProviderNotFound(provider_id.to_string()))?;

        if !provider.enabled {
            return Err(DgosError::ProviderUnavailable(
                "Provider is disabled".to_string(),
            ));
        }

        let adapter = self.get_or_create_adapter(&provider).await?;
        adapter.stream(model_id, request).await
    }

    /// List usage logs
    pub async fn list_usage_logs(
        &self,
        user_id: &str,
        limit: Option<usize>,
    ) -> DgosResult<Vec<UsageLog>> {
        self.usage_log_repo.list(user_id, limit).await
    }

    /// Get or create adapter for a provider
    async fn get_or_create_adapter(
        &self,
        provider: &Provider,
    ) -> DgosResult<Arc<dyn ProviderAdapter>> {
        let cache = self.adapter_cache.read().await;
        
        if let Some(adapter) = cache.get(&provider.id) {
            return Ok(Arc::clone(adapter));
        }

        drop(cache);

        // Create new adapter
        let config = ProviderConfig {
            name: provider.name.clone(),
            provider_type: provider.provider_type.clone(),
            base_url: provider.base_url.clone(),
            api_key: provider.api_key.clone(),
            enabled: provider.enabled,
            config: provider.config.clone(),
        };

        let adapter = self.create_adapter(&config)?;
        
        // Cache it
        self.adapter_cache
            .write()
            .await
            .insert(provider.id.clone(), Arc::clone(&adapter));

        Ok(adapter)
    }

    /// Create an adapter from config
    fn create_adapter(&self, config: &ProviderConfig) -> DgosResult<Arc<dyn ProviderAdapter>> {
        let retry_config = config.config.as_ref()
            .and_then(|c| serde_json::from_value::<RetryConfig>(c.clone()).ok());

        match config.provider_type.as_str() {
            "openai_compatible" => {
                let adapter = OpenAIAdapter::new(
                    config.base_url.clone(),
                    config.api_key.clone(),
                    retry_config,
                );
                Ok(Arc::new(adapter))
            }
            _ => Err(DgosError::InternalError(format!(
                "Unsupported provider type: {}",
                config.provider_type
            ))),
        }
    }
}
