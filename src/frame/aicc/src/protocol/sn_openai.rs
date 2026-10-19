use super::{
    openai_responses_adapter, AdapterCredentialContract, AdapterDescriptor, AdapterStatus,
    CodecRegistration, CodecRegistry, ProtocolResultValue, OPENAI_RESPONSES_ADAPTER_ID,
};

pub(crate) const SN_OPENAI_ADAPTER_ID: &str = "sn-openai";

pub(crate) fn sn_openai_adapter() -> ProtocolResultValue<AdapterDescriptor> {
    let (base, _) = openai_responses_adapter();
    Ok(AdapterDescriptor {
        protocol_family_id: base.protocol_family_id,
        protocol_adapter_id: SN_OPENAI_ADAPTER_ID.to_owned(),
        interface_generation: base.interface_generation,
        base_adapter_id: Some(OPENAI_RESPONSES_ADAPTER_ID.to_owned()),
        component_adapter_ids: Vec::new(),
        status: AdapterStatus::Stable,
        probe_priority: 200,
        probe_path: None,
        credential: AdapterCredentialContract::bearer(),
        operations: base.operations,
    })
}

pub(crate) fn register_sn_openai_adapter(registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
    registry.register_derived(sn_openai_adapter()?, CodecRegistration::default())
}
