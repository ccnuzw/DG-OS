//! Model classification (V1-FR-001 §2.4.3, B01-45).
//!
//! The rules live in `dgos-types` because `dgos-db` must derive the kind of rows written before
//! the kind migration. This module re-exports them for provider code and holds the case table of
//! the technical design ("模型分类规则"); every row is a unit test.

pub use dgos_types::{
    build_model, capability_hints, classify_model, recompute_models, valid_model_id, Hint,
    KindSource, ModelCapabilities, ModelKind, ModelSource, PROTOCOL_ANTHROPIC,
    PROTOCOL_OPENAI_COMPATIBLE,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn openai(id: &str) -> (ModelKind, KindSource) {
        classify_model(PROTOCOL_OPENAI_COMPATIBLE, id)
    }

    fn assert_all(ids: &[&str], kind: ModelKind, source: KindSource) {
        for id in ids {
            assert_eq!(openai(id), (kind, source), "{id}");
        }
    }

    #[test]
    fn classify_defaults_chat_models_to_text_generation() {
        assert_all(
            &[
                "gpt-4o",
                "deepseek-chat",
                "qwen2.5-72b-instruct",
                "llama-3.1-8b",
            ],
            ModelKind::TextGeneration,
            KindSource::Fallback,
        );
    }

    #[test]
    fn classify_keeps_audio_preview_chat_models_in_text_generation() {
        assert_all(
            &["gpt-4o-audio-preview"],
            ModelKind::TextGeneration,
            KindSource::Fallback,
        );
    }

    #[test]
    fn classify_treats_vision_models_as_text_generation_with_a_vision_hint() {
        for id in ["gpt-4-vision-preview", "qwen-vl-max"] {
            assert_eq!(
                openai(id),
                (ModelKind::TextGeneration, KindSource::Fallback)
            );
            let hints = capability_hints(PROTOCOL_OPENAI_COMPATIBLE, ModelKind::TextGeneration, id);
            assert_eq!(hints.vision, Hint::Supported, "{id}");
            assert_eq!(hints.streaming, Hint::Supported, "{id}");
        }
    }

    #[test]
    fn classify_embedding_models() {
        assert_all(
            &["text-embedding-3-small", "bge-m3", "nomic-embed-text"],
            ModelKind::Embedding,
            KindSource::Rule,
        );
    }

    #[test]
    fn classify_rerank_wins_over_embedding_by_rule_order() {
        // `bge-reranker-*` also starts with `bge-`.
        assert_all(
            &["bge-reranker-v2-m3", "jina-reranker-v2"],
            ModelKind::Rerank,
            KindSource::Rule,
        );
    }

    #[test]
    fn classify_moderation_is_not_turned_into_chat_by_omni() {
        assert_all(
            &["omni-moderation-latest"],
            ModelKind::Moderation,
            KindSource::Rule,
        );
        let hints = capability_hints(
            PROTOCOL_OPENAI_COMPATIBLE,
            ModelKind::Moderation,
            "omni-moderation-latest",
        );
        assert_eq!(hints, ModelCapabilities::default());
    }

    #[test]
    fn classify_transcription_models_are_not_audio_generation() {
        assert_all(
            &["whisper-1", "gpt-4o-transcribe", "sensevoice-small"],
            ModelKind::SpeechTranscription,
            KindSource::Rule,
        );
    }

    #[test]
    fn classify_speech_synthesis_models() {
        assert_all(
            &["tts-1", "gpt-4o-mini-tts", "cosyvoice-v1"],
            ModelKind::SpeechSynthesis,
            KindSource::Rule,
        );
    }

    #[test]
    fn classify_image_generation_models() {
        assert_all(
            &["dall-e-3", "gpt-image-1", "flux-dev", "stable-diffusion-xl"],
            ModelKind::ImageGeneration,
            KindSource::Rule,
        );
    }

    #[test]
    fn classify_video_generation_models() {
        assert_all(
            &["sora-2", "wan2.1-t2v"],
            ModelKind::VideoGeneration,
            KindSource::Rule,
        );
    }

    #[test]
    fn classify_strips_the_path_prefix_before_matching() {
        assert_all(
            &["openrouter/auto", "meta-llama/llama-3-70b"],
            ModelKind::TextGeneration,
            KindSource::Fallback,
        );
        // The prefix itself must not decide the kind, the last segment does.
        assert_all(
            &["vendor/text-embedding-3-small"],
            ModelKind::Embedding,
            KindSource::Rule,
        );
        assert_all(
            &["models/TTS-1"],
            ModelKind::SpeechSynthesis,
            KindSource::Rule,
        );
    }

    #[test]
    fn classify_requires_a_left_boundary_so_tts_inside_a_word_does_not_match() {
        assert_all(
            &["scotts-chat"],
            ModelKind::TextGeneration,
            KindSource::Fallback,
        );
    }

    #[test]
    fn classify_is_case_insensitive() {
        assert_eq!(openai("Text-Embedding-3-Small").0, ModelKind::Embedding);
        assert_eq!(openai("WHISPER-1").0, ModelKind::SpeechTranscription);
    }

    #[test]
    fn classify_anthropic_models_are_always_text_generation_by_protocol() {
        for id in ["claude-sonnet-5.5", "text-embedding-3-small", "whisper-1"] {
            assert_eq!(
                classify_model(PROTOCOL_ANTHROPIC, id),
                (ModelKind::TextGeneration, KindSource::Protocol),
                "{id}"
            );
        }
    }

    #[test]
    fn classify_reasoning_hint_follows_the_id_and_never_applies_to_other_kinds() {
        let hints = |id: &str, kind| capability_hints(PROTOCOL_OPENAI_COMPATIBLE, kind, id);
        assert_eq!(
            hints("o3-mini", ModelKind::TextGeneration).reasoning,
            Hint::Supported
        );
        assert_eq!(
            hints("deepseek-reasoner", ModelKind::TextGeneration).reasoning,
            Hint::Supported
        );
        assert_eq!(
            hints("gpt-4o", ModelKind::TextGeneration).reasoning,
            Hint::Unknown
        );
        assert_eq!(
            hints("o3-embedding", ModelKind::Embedding),
            ModelCapabilities::default()
        );
        // The rules can show support but never "unsupported".
        assert_eq!(
            hints("gpt-4o", ModelKind::TextGeneration).vision,
            Hint::Unknown
        );
    }

    #[test]
    fn classify_a_saved_remote_list_carries_kind_source_and_streaming() {
        let ids = [
            "gpt-4o",
            "text-embedding-3-small",
            "bge-reranker-v2-m3",
            "dall-e-3",
            "sora-2",
            "tts-1",
            "whisper-1",
            "omni-moderation-latest",
        ];
        let models: Vec<_> = ids
            .iter()
            .map(|id| {
                build_model(
                    PROTOCOL_OPENAI_COMPATIBLE,
                    id,
                    None,
                    ModelSource::Remote,
                    None,
                )
            })
            .collect();
        let kinds: Vec<_> = models.iter().map(|m| m.model_type.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "text-generation",
                "embedding",
                "rerank",
                "image-generation",
                "video-generation",
                "speech-synthesis",
                "speech-transcription",
                "moderation"
            ]
        );
        assert!(models[0].supports_streaming);
        assert!(models[1..].iter().all(|m| !m.supports_streaming));
        assert!(models.iter().all(|m| m.max_output_tokens.is_none()));
    }
}
