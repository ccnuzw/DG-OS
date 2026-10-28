// DG-OS Provider Adapter Framework
// Core traits and implementations for AI provider adapters

mod adapters;
mod credentials;
mod manager;
mod retry;
mod traits;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod credential_tests;

pub use adapters::openai::OpenAIAdapter;
pub use credentials::{
    new_credential_ref, ProviderCredentialScope, ProviderCredentialVault, SecretValue,
    UnavailableCredentialVault,
};
pub use manager::ProviderManager;
pub use retry::RetryPolicy;
pub use traits::ProviderAdapter;

// Re-export types
pub use dgos_types::{
    Capability, ChatChunk, ChatMessage, ChatRequest, ChatResponse, DgosError, DgosResult, Model,
    Provider, ProviderConfig, RetryConfig, TokenUsage, UsageLog,
};
