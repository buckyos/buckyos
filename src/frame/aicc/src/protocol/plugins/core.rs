use crate::protocol::{
    fal_queue_adapter, gemini_interactions_adapter, openai_chat_completions_adapter,
    openai_responses_adapter, CodecRegistry, ProtocolAdapterPlugin, ProtocolResultValue,
};
use crate::provider::claude_messages_adapter;

pub(super) struct Plugin;

impl ProtocolAdapterPlugin for Plugin {
    fn register(&self, registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
        for (descriptor, codecs) in [
            openai_responses_adapter(),
            claude_messages_adapter(),
            gemini_interactions_adapter(),
            openai_chat_completions_adapter(),
            fal_queue_adapter(),
        ] {
            registry.register_codecs(descriptor, codecs)?;
        }
        Ok(())
    }
}
