// DG-OS Provider Adapter Framework
// Core traits and implementations for AI provider adapters

mod traits;
mod adapters;
mod manager;
mod retry;

pub use traits::ProviderAdapter;
pub use adapters::openai::OpenAIAdapter;
pub use manager::ProviderManager;
pub use retry::RetryPolicy;

// Re-export types
pub use dgos_types::{
    Capability, Model, ChatRequest, ChatResponse, ChatMessage, ChatChunk, 
    Provider, ProviderConfig, RetryConfig, TokenUsage, UsageLog, DgosError, DgosResult
};
