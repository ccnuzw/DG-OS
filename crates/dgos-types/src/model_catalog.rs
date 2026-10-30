//! Host-owned model classification (V1-FR-001 §2.4.3, technical design "模型分类规则").
//!
//! Everything here is a pure function of `(protocol, model_id)`. The renderer never
//! supplies a kind, a capability hint or a primary key (AGENTS.md §3).

use crate::{DgosError, DgosResult, Model};
use serde::{Deserialize, Serialize};

/// `models.model_id` column width and the §2.4.7 limit for remote ids.
pub const MAX_MODEL_ID_CHARS: usize = 255;

pub const PROTOCOL_OPENAI_COMPATIBLE: &str = "openai_compatible";
pub const PROTOCOL_ANTHROPIC: &str = "anthropic";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelKind {
    TextGeneration,
    Embedding,
    Rerank,
    ImageGeneration,
    VideoGeneration,
    SpeechSynthesis,
    SpeechTranscription,
    Moderation,
}

impl ModelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelKind::TextGeneration => "text-generation",
            ModelKind::Embedding => "embedding",
            ModelKind::Rerank => "rerank",
            ModelKind::ImageGeneration => "image-generation",
            ModelKind::VideoGeneration => "video-generation",
            ModelKind::SpeechSynthesis => "speech-synthesis",
            ModelKind::SpeechTranscription => "speech-transcription",
            ModelKind::Moderation => "moderation",
        }
    }

    /// Strict parse of the persisted/wire spelling. The legacy `chat` value is not a kind.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "text-generation" => ModelKind::TextGeneration,
            "embedding" => ModelKind::Embedding,
            "rerank" => ModelKind::Rerank,
            "image-generation" => ModelKind::ImageGeneration,
            "video-generation" => ModelKind::VideoGeneration,
            "speech-synthesis" => ModelKind::SpeechSynthesis,
            "speech-transcription" => ModelKind::SpeechTranscription,
            "moderation" => ModelKind::Moderation,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KindSource {
    Protocol,
    Rule,
    /// No rule matched; serialized as `default`.
    #[default]
    #[serde(rename = "default")]
    Fallback,
    Manual,
}

impl KindSource {
    pub fn as_str(self) -> &'static str {
        match self {
            KindSource::Protocol => "protocol",
            KindSource::Rule => "rule",
            KindSource::Fallback => "default",
            KindSource::Manual => "manual",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "protocol" => KindSource::Protocol,
            "rule" => KindSource::Rule,
            "default" => KindSource::Fallback,
            "manual" => KindSource::Manual,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelSource {
    #[default]
    Remote,
    Manual,
}

impl ModelSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelSource::Remote => "remote",
            ModelSource::Manual => "manual",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "remote" => Some(ModelSource::Remote),
            "manual" => Some(ModelSource::Manual),
            _ => None,
        }
    }
}

/// Display-only capability hint. The host never blocks a request because of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hint {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelCapabilities {
    pub streaming: Hint,
    pub vision: Hint,
    pub tools: Hint,
    pub reasoning: Hint,
    pub structured_output: Hint,
}

impl ModelCapabilities {
    /// Reads both persisted shapes: the new object, and the legacy array of capability
    /// names where only `streaming` and `vision` carry information (§4.3).
    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Array(items) => {
                let mut caps = ModelCapabilities::default();
                for item in items.iter().filter_map(|item| item.as_str()) {
                    match item {
                        "streaming" => caps.streaming = Hint::Supported,
                        "vision" => caps.vision = Hint::Supported,
                        _ => {}
                    }
                }
                caps
            }
            serde_json::Value::Object(_) => {
                serde_json::from_value(value.clone()).unwrap_or_default()
            }
            _ => ModelCapabilities::default(),
        }
    }

    pub fn to_json(self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

fn normalize(model_id: &str) -> String {
    let lower = model_id.to_lowercase();
    match lower.rsplit_once('/') {
        Some((_, tail)) => tail.to_owned(),
        None => lower,
    }
}

/// Left-boundary match: the pattern starts the id or follows `-`, `_`, `.`, `:` or a space.
fn has_pattern(id: &str, pattern: &str) -> bool {
    id.match_indices(pattern).any(|(start, _)| {
        start == 0
            || id[..start]
                .chars()
                .next_back()
                .is_some_and(|c| matches!(c, '-' | '_' | '.' | ':' | ' '))
    })
}

fn has_any(id: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| has_pattern(id, pattern))
}

/// Ordered rule table. The order is part of the rule (`bge-reranker-*` must stay a rerank model).
const KIND_RULES: &[(ModelKind, &[&str])] = &[
    (ModelKind::Rerank, &["rerank"]),
    (
        ModelKind::Embedding,
        &["embed", "bge-", "e5-", "gte-", "text-embedding"],
    ),
    (ModelKind::Moderation, &["moderation"]),
    (
        ModelKind::SpeechTranscription,
        &["whisper", "transcribe", "asr", "sensevoice", "paraformer"],
    ),
    (
        ModelKind::SpeechSynthesis,
        &["tts", "cosyvoice", "fish-speech", "text-to-speech"],
    ),
    (
        ModelKind::VideoGeneration,
        &[
            "sora", "veo", "kling", "runway", "hailuo", "cogvideo", "seedance", "wan2", "t2v",
            "i2v",
        ],
    ),
    (
        ModelKind::ImageGeneration,
        &[
            "dall-e",
            "dalle",
            "gpt-image",
            "flux",
            "stable-diffusion",
            "sdxl",
            "sd3",
            "midjourney",
            "imagen",
            "kolors",
            "cogview",
            "seedream",
            "qwen-image",
        ],
    ),
];

/// `(protocol, model_id) -> (kind, kind_source)`; pure and total.
pub fn classify_model(protocol: &str, model_id: &str) -> (ModelKind, KindSource) {
    if protocol == PROTOCOL_ANTHROPIC {
        return (ModelKind::TextGeneration, KindSource::Protocol);
    }
    let id = normalize(model_id);
    for (kind, patterns) in KIND_RULES {
        if has_any(&id, patterns) {
            return (*kind, KindSource::Rule);
        }
    }
    (ModelKind::TextGeneration, KindSource::Fallback)
}

/// Capability hints derived from the kind and the id only (no upstream metadata).
pub fn capability_hints(protocol: &str, kind: ModelKind, model_id: &str) -> ModelCapabilities {
    if kind != ModelKind::TextGeneration {
        return ModelCapabilities::default();
    }
    let mut caps = ModelCapabilities {
        streaming: Hint::Supported,
        ..Default::default()
    };
    if protocol != PROTOCOL_ANTHROPIC {
        let id = normalize(model_id);
        if has_any(&id, &["vision", "vl", "omni"]) {
            caps.vision = Hint::Supported;
        }
        if has_any(
            &id,
            &["o1", "o3", "o4", "r1", "reasoner", "thinking", "qwq"],
        ) {
            caps.reasoning = Hint::Supported;
        }
    }
    caps
}

pub fn valid_model_id(model_id: &str) -> bool {
    !model_id.trim().is_empty() && model_id.chars().count() <= MAX_MODEL_ID_CHARS
}

/// Builds a fully host-derived model row. `manual_kind` is honoured only for manual models.
pub fn build_model(
    protocol: &str,
    model_id: &str,
    model_name: Option<&str>,
    source: ModelSource,
    manual_kind: Option<ModelKind>,
) -> Model {
    let (kind, kind_source) = match (source, manual_kind) {
        (ModelSource::Manual, Some(kind)) => (kind, KindSource::Manual),
        _ => classify_model(protocol, model_id),
    };
    let capabilities = capability_hints(protocol, kind, model_id);
    let name = model_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(model_id);
    Model {
        id: uuid::Uuid::new_v4().to_string(),
        provider_id: String::new(),
        model_id: model_id.to_owned(),
        model_name: name.chars().take(MAX_MODEL_ID_CHARS).collect(),
        model_type: kind.as_str().to_owned(),
        context_window: None,
        max_output_tokens: None,
        supports_streaming: capabilities.streaming == Hint::Supported,
        supports_vision: capabilities.vision == Hint::Supported,
        cached_at: chrono::Utc::now().to_rfc3339(),
        kind_source,
        source,
        capabilities,
    }
}

/// Save-time recomputation (B01-67): only `model_id` and `model_name` of the submitted
/// models are used. Identity, kind, hints and lengths are generated by the host.
/// One invalid id rejects the whole submission.
pub fn recompute_models(protocol: &str, submitted: &[Model]) -> DgosResult<Vec<Model>> {
    if submitted.iter().any(|m| !valid_model_id(&m.model_id)) {
        return Err(DgosError::RequestNotSupported);
    }
    let mut seen = std::collections::HashSet::new();
    Ok(submitted
        .iter()
        .filter(|m| seen.insert(m.model_id.as_str()))
        .map(|m| {
            build_model(
                protocol,
                &m.model_id,
                Some(&m.model_name),
                ModelSource::Remote,
                None,
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_capability_arrays_read_streaming_and_vision_only() {
        let caps =
            ModelCapabilities::from_json(&serde_json::json!(["streaming", "text-generation"]));
        assert_eq!(caps.streaming, Hint::Supported);
        assert_eq!(caps.vision, Hint::Unknown);
        assert_eq!(caps.tools, Hint::Unknown);
        let caps = ModelCapabilities::from_json(&serde_json::json!(["vision"]));
        assert_eq!(caps.vision, Hint::Supported);
        assert_eq!(caps.streaming, Hint::Unknown);
        assert_eq!(
            ModelCapabilities::from_json(&serde_json::json!([])),
            ModelCapabilities::default()
        );
    }

    #[test]
    fn object_capabilities_round_trip_and_tolerate_missing_keys() {
        let caps = ModelCapabilities {
            streaming: Hint::Supported,
            reasoning: Hint::Unsupported,
            ..Default::default()
        };
        assert_eq!(ModelCapabilities::from_json(&caps.to_json()), caps);
        let partial = ModelCapabilities::from_json(&serde_json::json!({"vision": "supported"}));
        assert_eq!(partial.vision, Hint::Supported);
        assert_eq!(partial.streaming, Hint::Unknown);
    }

    #[test]
    fn model_id_validity_counts_characters_not_bytes() {
        assert!(valid_model_id(&"é".repeat(255)));
        assert!(!valid_model_id(&"é".repeat(256)));
        assert!(!valid_model_id(""));
        assert!(!valid_model_id("   "));
    }

    #[test]
    fn kind_and_source_spellings_round_trip() {
        for kind in [
            ModelKind::TextGeneration,
            ModelKind::Embedding,
            ModelKind::Rerank,
            ModelKind::ImageGeneration,
            ModelKind::VideoGeneration,
            ModelKind::SpeechSynthesis,
            ModelKind::SpeechTranscription,
            ModelKind::Moderation,
        ] {
            assert_eq!(ModelKind::parse(kind.as_str()), Some(kind));
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
        }
        assert_eq!(ModelKind::parse("chat"), None);
        for source in [
            KindSource::Protocol,
            KindSource::Rule,
            KindSource::Fallback,
            KindSource::Manual,
        ] {
            assert_eq!(KindSource::parse(source.as_str()), Some(source));
            assert_eq!(serde_json::to_value(source).unwrap(), source.as_str());
        }
    }
}
