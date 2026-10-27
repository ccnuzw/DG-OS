use async_trait::async_trait;
use dgos_types::{DgosResult, Model, ChatRequest, ChatResponse, ChatChunk};
use futures::stream::Stream;
use std::pin::Pin;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = DgosResult<T>> + Send>>;

/// Core trait for provider adapters
/// All AI provider implementations must implement this trait
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Get the provider type identifier
    fn provider_type(&self) -> &str;
    
    /// List available models from the provider
    async fn list_models(&self) -> DgosResult<Vec<Model>>;
    
    /// Make a synchronous chat completion request
    async fn call(
        &self,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<ChatResponse>;
    
    /// Make a streaming chat completion request
    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<BoxStream<ChatChunk>>;
    
    /// Test the connection to the provider
    async fn test_connection(&self) -> DgosResult<bool>;
}
