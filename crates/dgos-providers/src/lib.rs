// DG-OS Provider Adapter Framework
// Core traits and implementations for AI provider adapters

mod adapters;
pub mod classify;
mod credentials;
pub mod endpoint;
mod manager;
mod retry;
mod sse;
mod stream;
mod structured;
mod traits;
mod transport;
mod upstream;
pub mod vendor;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod credential_tests;

pub use adapters::anthropic::AnthropicAdapter;
pub use adapters::openai::OpenAIAdapter;
pub use credentials::{
    new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, SecretValue,
    UnavailableCredentialVault,
};
pub use manager::ProviderManager;
pub use retry::{is_retryable, AttemptError, RetryPolicy};
pub use traits::{BoxStream, CallFailure, ProviderAdapter};
pub use transport::AdapterOptions;
pub use vendor::{list_vendors, AuthStyle};

// Re-export types
pub use dgos_types::{
    Capability, ChatChunk, ChatMessage, ChatRequest, ChatResponse, DgosError, DgosResult, Model,
    Provider, ProviderConfig, RetryConfig, TokenUsage, UsageLog,
};
