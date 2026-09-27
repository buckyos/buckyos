use crate::protocol::{
    register_sn_openai_adapter, CodecRegistry, ProtocolAdapterPlugin, ProtocolResultValue,
};

pub(super) struct Plugin;

impl ProtocolAdapterPlugin for Plugin {
    fn register(&self, registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
        register_sn_openai_adapter(registry)
    }
}
