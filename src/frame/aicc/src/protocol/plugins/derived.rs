use crate::protocol::{
    legacy_claude_messages_adapter, legacy_openai_chat_completions_adapter,
    openrouter_responses_adapter, CodecRegistry, ProtocolAdapterPlugin, ProtocolResultValue,
};

pub(super) struct Plugin;

impl ProtocolAdapterPlugin for Plugin {
    fn register(&self, registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
        for (descriptor, codecs) in [
            legacy_openai_chat_completions_adapter(),
            legacy_claude_messages_adapter(),
            openrouter_responses_adapter(),
        ] {
            registry.register_derived(descriptor, codecs)?;
        }
        Ok(())
    }
}
