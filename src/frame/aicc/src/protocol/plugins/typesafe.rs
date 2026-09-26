use crate::protocol::{
    typesafe::typesafe_adapter, CodecRegistry, ProtocolAdapterPlugin, ProtocolResultValue,
};

pub(super) struct Plugin;

impl ProtocolAdapterPlugin for Plugin {
    fn register(&self, registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
        let (descriptor, codecs) = typesafe_adapter();
        registry.register_codecs(descriptor, codecs)
    }
}
