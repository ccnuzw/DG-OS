use async_trait::async_trait;
use dgos_types::{
    ChatChunk, ChatRequest, ChatResponse, DgosError, DgosResult, Model, TokenUsage,
    ToolChatRequest, ToolChatResponse,
};
use futures::stream::Stream;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = DgosResult<T>> + Send>>;

/// A failed synchronous call, plus the tokens the upstream reported for it when it did answer.
/// A response the host rejects afterwards (`structured_output_invalid`) was still billed, so its
/// usage record must carry the upstream's numbers instead of 0.
#[derive(Debug)]
pub struct CallFailure {
    pub error: DgosError,
    pub usage: Option<TokenUsage>,
}

impl From<DgosError> for CallFailure {
    fn from(error: DgosError) -> Self {
        Self { error, usage: None }
    }
}

/// Core trait for provider adapters
/// All AI provider implementations must implement this trait
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Get the provider type identifier
    fn provider_type(&self) -> &str;

    /// The protocol rejects a request without `max_tokens`. The manager then fills it from the
    /// model's `max_output_tokens` when the caller gave none.
    fn requires_max_tokens(&self) -> bool {
        false
    }

    /// List available models from the provider
    async fn list_models(&self) -> DgosResult<Vec<Model>>;

    /// Make a synchronous chat completion request
    async fn call(&self, model_id: &str, request: ChatRequest) -> DgosResult<ChatResponse>;

    /// Same call, but a failure keeps the upstream's token usage when there was one. Adapters
    /// that cannot report it fall back to this default.
    async fn call_with_usage(
        &self,
        model_id: &str,
        request: ChatRequest,
    ) -> Result<ChatResponse, CallFailure> {
        self.call(model_id, request)
            .await
            .map_err(CallFailure::from)
    }

    async fn call_tools(
        &self,
        model_id: &str,
        request: ToolChatRequest,
    ) -> DgosResult<ToolChatResponse>;

    /// Genuine incremental tool-mode completion; never a buffered synchronous fallback.
    async fn stream_tools(
        &self,
        model_id: &str,
        request: ToolChatRequest,
    ) -> DgosResult<BoxStream<dgos_types::ToolStreamEvent>> {
        self.stream_tools_with_dispatch(model_id, request, None).await
    }

    /// Shared tool-stream implementation. Set the optional signal only after local request
    /// validation, immediately before a transport send attempt. Once set it must remain true,
    /// including on connection failure, cancellation, or an invalid upstream response: it
    /// records possible delivery, not a successful response or proof of remote receipt.
    async fn stream_tools_with_dispatch(
        &self,
        model_id: &str,
        request: ToolChatRequest,
        dispatched: Option<&AtomicBool>,
    ) -> DgosResult<BoxStream<dgos_types::ToolStreamEvent>>;

    /// Make a streaming chat completion request
    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<BoxStream<ChatChunk>> {
        self.stream_with_dispatch(model_id, request, None).await
    }

    /// Shared text-stream implementation, with the same send-attempt semantics as
    /// `stream_tools_with_dispatch`. Normal callers use `stream` without allocating a signal.
    async fn stream_with_dispatch(
        &self,
        model_id: &str,
        request: ChatRequest,
        dispatched: Option<&AtomicBool>,
    ) -> DgosResult<BoxStream<ChatChunk>>;

    /// Verify the endpoint and key.
    ///
    /// Requests the model list first. When the service has none (404 or 405) and `probe_model` is
    /// given, sends one minimal request to that model; any 2xx counts as success. Without a probe
    /// model that case is `request_not_supported`. Failures are returned as errors, never as
    /// `Ok(false)`, so callers can tell a bad key from a bad address.
    async fn test_connection(&self, probe_model: Option<&str>) -> DgosResult<bool>;
}
