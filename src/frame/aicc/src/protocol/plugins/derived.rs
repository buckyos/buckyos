use crate::protocol::{
    glm_chat_adapter, kimi_chat_adapter, minimax_messages_adapter, openrouter_chat_adapter,
    CodecRegistry, ProtocolAdapterPlugin, ProtocolResultValue,
};

pub(super) struct Plugin;

impl ProtocolAdapterPlugin for Plugin {
    fn register(&self, registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
        for (descriptor, codecs) in [
            minimax_messages_adapter(),
            openrouter_chat_adapter(),
            kimi_chat_adapter(),
            glm_chat_adapter(),
        ] {
            registry.register_derived(descriptor, codecs)?;
        }
        Ok(())
    }
}
