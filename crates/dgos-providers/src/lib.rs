// DG-OS Provider Adapter Framework
// Core traits and implementations for AI provider adapters

mod adapters;
mod manager;
mod retry;
mod traits;

pub use adapters::openai::OpenAIAdapter;
pub use manager::ProviderManager;
pub use retry::RetryPolicy;
pub use traits::ProviderAdapter;

// Re-export types
pub use dgos_types::{
    Capability, ChatChunk, ChatMessage, ChatRequest, ChatResponse, DgosError, DgosResult, Model,
    Provider, ProviderConfig, RetryConfig, TokenUsage, UsageLog,
};
