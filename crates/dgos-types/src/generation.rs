use serde::{Deserialize, Serialize};
use crate::{ChatRequest, Model, ReasoningConfig, DgosError, DgosResult};
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GenerationParameters {
    pub temperature: Option<f32>,
    pub reasoning: Option<ReasoningConfig>,
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub instruction: String,
}
impl GenerationParameters {
    /// Audit retains controls only, never user instruction text.
    pub fn audit_json(&self) -> serde_json::Value { serde_json::json!({"temperature":self.temperature,"reasoning":self.reasoning,"max_tokens":self.output_tokens()}) }
    pub fn output_tokens(&self) -> u32 { self.max_tokens.unwrap_or(1024) }
    pub fn validate(&self, model: Option<&Model>) -> DgosResult<()> {
        if self.reasoning.is_some() && model.is_some_and(|m| m.capabilities.reasoning == crate::model_catalog::Hint::Unsupported) { return Err(DgosError::RequestNotSupported); }
        if self.instruction.len() > 16384 || self.temperature.is_some_and(|v| !v.is_finite() || !(0.0..=2.0).contains(&v)) || self.output_tokens() == 0 || self.output_tokens() > 65536 || model.and_then(|m| m.max_output_tokens).is_some_and(|limit| limit < self.output_tokens() as i32) {
            return Err(DgosError::RequestNotSupported);
        }
        Ok(())
    }
    pub fn apply(&self, request: &mut ChatRequest) {
        request.temperature = self.temperature;
        request.reasoning = self.reasoning.clone();
        request.max_tokens = Some(self.output_tokens());
    }
}
