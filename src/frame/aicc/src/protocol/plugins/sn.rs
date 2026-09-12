use crate::protocol::{CodecRegistry, ProtocolAdapterPlugin, ProtocolResultValue};
use crate::provider::register_sn_openai_adapter;

pub(super) struct Plugin;

impl ProtocolAdapterPlugin for Plugin {
    fn register(&self, registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
        register_sn_openai_adapter(registry)
    }
}
