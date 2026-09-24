use crate::protocol::{
    openai_responses_compatible_adapters, CodecRegistry, ProtocolAdapterPlugin,
    ProtocolResultValue,
};

pub(super) struct Plugin;

impl ProtocolAdapterPlugin for Plugin {
    fn register(&self, registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
        for (descriptor, codecs) in openai_responses_compatible_adapters()? {
            registry.register_derived(descriptor, codecs)?;
        }
        Ok(())
    }
}
