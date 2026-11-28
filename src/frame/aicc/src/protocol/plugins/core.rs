use crate::protocol::{
    claude_messages_adapter, doubao_media_adapter, doubao_speech_adapter, fal_queue_adapter,
    gemini_generate_content_adapter, gemini_interactions_adapter, glm_video_adapter, minimax_media_adapter,
    openai_audio_adapter, openai_chat_completions_adapter, openai_embeddings_adapter,
    openai_images_adapter, openai_responses_adapter, openai_videos_adapter,
    openrouter_native_adapter, bailian_media_adapter, rerank_adapter, CodecRegistry, ProtocolAdapterPlugin,
    ProtocolResultValue,
};

pub(super) struct Plugin;

impl ProtocolAdapterPlugin for Plugin {
    fn register(&self, registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
        for (descriptor, codecs) in [
            openai_responses_adapter(),
            openai_embeddings_adapter(),
            openai_images_adapter(),
            openai_audio_adapter(),
            openai_videos_adapter(),
            claude_messages_adapter(),
            gemini_interactions_adapter(),
            gemini_generate_content_adapter(),
            openai_chat_completions_adapter(),
            fal_queue_adapter(),
            glm_video_adapter(),
            minimax_media_adapter(),
            doubao_media_adapter(),
            doubao_speech_adapter(),
            bailian_media_adapter(),
            openrouter_native_adapter(),
            rerank_adapter(),
        ] {
            registry.register_codecs(descriptor, codecs)?;
        }
        Ok(())
    }
}
