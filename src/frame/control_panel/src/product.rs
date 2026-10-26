//! The product edition (`system/product`) as control-panel enforces and
//! shows it: which apps an installation runs and who holds the one Agent run
//! slot of BuckyOS Desktop Lite.

use crate::ControlPanelServer;
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    get_buckyos_api_runtime, AdmissionDenied, AdmissionOrigin, AgentSlot, AppDoc,
    InstallInspection, InstallReadiness, ProductProfile, ReadinessState, SystemConfigError,
    AGENT_SLOT_KEY, PRODUCT_PROFILE_KEY,
};
use serde_json::json;

pub(crate) async fn current_product() -> Result<ProductProfile, RPCErrors> {
    let client = get_buckyos_api_runtime()?.get_system_config_client().await?;
    match client.get(PRODUCT_PROFILE_KEY).await {
        Ok(value) => ProductProfile::parse(&value.value).map_err(RPCErrors::ReasonError),
        Err(SystemConfigError::KeyNotFound(_)) => Ok(ProductProfile::full()),
        Err(error) => Err(RPCErrors::ReasonError(format!(
            "read {PRODUCT_PROFILE_KEY} failed: {error}"
        ))),
    }
}

pub(crate) async fn current_agent_slot() -> Result<Option<AgentSlot>, RPCErrors> {
    let client = get_buckyos_api_runtime()?.get_system_config_client().await?;
    client.invalidate_cache(AGENT_SLOT_KEY).await;
    match client.get(AGENT_SLOT_KEY).await {
        Ok(value) => serde_json::from_str(&value.value)
            .map(Some)
            .map_err(|error| RPCErrors::ReasonError(format!("invalid {AGENT_SLOT_KEY}: {error}"))),
        Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
        Err(error) => Err(RPCErrors::ReasonError(format!(
            "read {AGENT_SLOT_KEY} failed: {error}"
        ))),
    }
}

/// Marks an inspection the product cannot carry out: the plan's target is
/// not ready and the reason is the first target issue, so `apps.inspect`
/// shows it and an install task stops at Inspect.
pub(crate) fn block_inspection(inspection: &mut InstallInspection, denied: &AdmissionDenied) {
    let status = &mut inspection.status;
    status
        .target_issues
        .insert(0, format!("{} ({})", denied, denied.code()));
    status.readiness.target = ReadinessState::NotReady;
    if status.readiness.install != InstallReadiness::IdentityRevoked {
        status.readiness.install = InstallReadiness::UnsupportedTarget;
    }
}

/// `admit_app` as an inspection constraint.
pub(crate) fn apply_admission(
    product: &ProductProfile,
    app_doc: &AppDoc,
    origin: AdmissionOrigin,
    inspection: &mut InstallInspection,
) {
    if let Err(denied) = product.admit_app(app_doc, origin) {
        block_inspection(inspection, &denied);
    }
}

impl ControlPanelServer {
    /// system.product.get: the edition, what it runs and the Agent run slot.
    pub(crate) async fn handle_system_product_get(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let product = current_product().await?;
        let agent_slot = if product.capabilities.max_running_agents.is_some() {
            current_agent_slot().await?
        } else {
            None
        };
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "edition": product.edition,
                "capabilities": product.capabilities,
                "preinstalled_script_apps": product.preinstalled_script_apps,
                "agent_slot": agent_slot,
            })),
            req.seq,
        ))
    }
}
