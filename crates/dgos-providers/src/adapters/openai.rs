use async_trait::async_trait;
use dgos_types::{
    Capability, ChatChunk, ChatMessage, ChatRequest, ChatResponse, DgosError, DgosResult, Model,
    RetryConfig, TokenUsage,
};
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};
use uuid::Uuid;
use chrono::Utc;

use crate::traits::{BoxStream, ProviderAdapter};
use crate::retry::RetryPolicy;

/// OpenAI compatible adapter
pub struct OpenAIAdapter {
    base_url: String,
    api_key: String,
    client: Client,
    retry_policy: RetryPolicy,
}

impl OpenAIAdapter {
    pub fn new(
        base_url: String,
        api_key: String,
        retry_config: Option<RetryConfig>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| Client::new());

        let retry_policy = RetryPolicy::new(retry_config.unwrap_or_default());

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
            client,
            retry_policy,
        }
    }

    /// Classify model capabilities based on model ID
    fn classify_model(&self, model_id: &str) -> Vec<Capability> {
        let id_lower = model_id.to_lowercase();
        
        if id_lower.contains("dall-e") || id_lower.contains("dalle") {
            vec![Capability::ImageGeneration]
        } else if id_lower.contains("whisper") {
            vec![Capability::AudioGeneration]
        } else if id_lower.contains("tts") {
            vec![Capability::AudioGeneration]
        } else if id_lower.contains("gpt") 
            || id_lower.contains("claude") 
            || id_lower.contains("chat") 
            || id_lower.contains("text") {
            vec![Capability::TextGeneration]
        } else {
            // Default to text generation
            vec![Capability::TextGeneration]
        }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAIAdapter {
    fn provider_type(&self) -> &str {
        "openai_compatible"
    }

    async fn list_models(&self) -> DgosResult<Vec<Model>> {
        let url = format!("{}/models", self.base_url);
        
        debug!("Fetching models from: {}", url);
        
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(DgosError::HttpError(response.status().as_u16()));
        }

        let models_response: OpenAIModelsResponse = response.json().await?;

        let models = models_response
            .data
            .into_iter()
            .map(|m| {
                let capabilities = self.classify_model(&m.id);
                Model {
                    id: Uuid::new_v4().to_string(),
                    provider_id: String::new(), // Will be set by caller
                    model_id: m.id.clone(),
                    model_name: m.id.clone(),
                    model_type: "chat".to_string(),
                    context_window: None,
                    max_output_tokens: None,
                    supports_streaming: capabilities.contains(&Capability::Streaming),
                    supports_vision: capabilities.contains(&Capability::Vision),
                    cached_at: Utc::now().to_rfc3339(),
                }
            })
            .collect();

        Ok(models)
    }

    async fn call(
        &self,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<ChatResponse> {
        let url = format!("{}/chat/completions", self.base_url);

        let openai_request = OpenAIChatRequest {
            model: model_id.to_string(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: Some(false),
        };

        // Execute with retry policy
        let response = self.retry_policy.execute(|| async {
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&openai_request)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                return Err(match status {
                    401 => DgosError::InvalidApiKey,
                    429 => DgosError::RateLimitExceeded,
                    _ => DgosError::HttpError(status),
                });
            }

            let chat_response: OpenAIChatResponse = resp.json().await?;
            
            Ok(ChatResponse {
                content: chat_response
                    .choices
                    .first()
                    .and_then(|c| c.message.content.clone())
                    .unwrap_or_default(),
                usage: TokenUsage {
                    prompt_tokens: chat_response.usage.prompt_tokens,
                    completion_tokens: chat_response.usage.completion_tokens,
                    total_tokens: chat_response.usage.total_tokens,
                },
            })
        }).await?;

        Ok(response)
    }

    async fn stream(
        &self,
        model_id: &str,
        request: ChatRequest,
    ) -> DgosResult<BoxStream<ChatChunk>> {
        let url = format!("{}/chat/completions", self.base_url);

        let openai_request = OpenAIChatRequest {
            model: model_id.to_string(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: Some(true),
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&openai_request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(match status {
                401 => DgosError::InvalidApiKey,
                429 => DgosError::RateLimitExceeded,
                _ => DgosError::HttpError(status),
            });
        }

        let stream = response.bytes_stream().map(|chunk_result| {
            match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    
                    // Parse SSE format: "data: {...}\n\n"
                    for line in text.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data.trim() == "[DONE]" {
                                return Ok(ChatChunk {
                                    delta: String::new(),
                                    finish_reason: Some("stop".to_string()),
                                });
                            }
                            
                            if let Ok(chunk_data) = serde_json::from_str::<OpenAIStreamChunk>(data) {
                                let delta = chunk_data
                                    .choices
                                    .first()
                                    .and_then(|c| c.delta.content.clone())
                                    .unwrap_or_default();
                                
                                let finish_reason = chunk_data
                                    .choices
                                    .first()
                                    .and_then(|c| c.finish_reason.clone());
                                
                                return Ok(ChatChunk {
                                    delta,
                                    finish_reason,
                                });
                            }
                        }
                    }
                    
                    // Empty chunk if parsing failed
                    Ok(ChatChunk {
                        delta: String::new(),
                        finish_reason: None,
                    })
                }
                Err(e) => {
                    warn!("Stream error: {}", e);
                    Err(DgosError::StreamInterrupted)
                }
            }
        });

        Ok(Box::pin(stream))
    }

    async fn test_connection(&self) -> DgosResult<bool> {
        let url = format!("{}/models", self.base_url);
        
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        Ok(response.status().is_success())
    }
}

// OpenAI API types

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModelData>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIModelData {
    id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIChatResponse {
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIMessage {
    content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIStreamChunk {
    choices: Vec<OpenAIStreamChoice>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIStreamChoice {
    delta: OpenAIDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIDelta {
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_model() {
        let adapter = OpenAIAdapter::new(
            "https://api.openai.com/v1".to_string(),
            "test-key".to_string(),
            None,
        );

        assert_eq!(
            adapter.classify_model("gpt-4"),
            vec![Capability::TextGeneration]
        );
        assert_eq!(
            adapter.classify_model("dall-e-3"),
            vec![Capability::ImageGeneration]
        );
        assert_eq!(
            adapter.classify_model("whisper-1"),
            vec![Capability::AudioGeneration]
        );
    }
}
