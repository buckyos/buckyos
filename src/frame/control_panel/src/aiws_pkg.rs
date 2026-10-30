//! `apps.submit` of an `aiws` PIKG (AI Workspace controls, src/frame/aiworkspace/README.md “Extension controls”):
//! the staged package is verified like every PIKG, then recorded in the submitter's AI Workspace control library by
//! the aiworkspace service (`ext.install`, mode `installer`). No AppSpec, no install record, nothing is deployed.

use crate::agent_mgr::reason;
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest};
use buckyos_api::{
    get_buckyos_api_runtime, AdmissionOrigin, AiWorkspaceClient, AppType, InstallError, InstallErrorCode, InstallInspection, InstallSource,
    InstallSourceIdentity, InstallStage, PikgStagingPurpose, AIWORKSPACE_SERVICE_NAME,
};
use log::info;
use serde_json::{json, Value};

fn aiws_error(error: impl std::fmt::Display) -> RPCErrors {
    reason("aiws_package_invalid", error)
}

impl ControlPanelServer {
    /// Verify the staged `aiws` PIKG and hand it to the aiworkspace service for the owner's library.
    pub(crate) async fn register_aiws_package(
        &self,
        req: &RPCRequest,
        principal: &RpcAuthPrincipal,
        inspection: &InstallInspection,
    ) -> Result<Value, RPCErrors> {
        let InstallSource::LocalPikg { staging_handle } = Self::parse_install_source(req)? else {
            return Err(reason("aiws_source_unsupported", "AI Workspace control packages are installed from a PIKG"));
        };
        let InstallSourceIdentity::Pikg { pikg_digest, .. } = &inspection.plan.source_identity else {
            return Err(reason("aiws_source_unsupported", "AI Workspace control packages are installed from a PIKG"));
        };
        let readiness = &inspection.status.readiness;
        if !readiness.trust.is_ready() || !readiness.package_integrity.is_ready() {
            return Err(Self::install_error_to_rpc(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::TrustResolutionRequired,
                false,
                "the control package is not trusted or not intact",
            )));
        }
        let runtime = get_buckyos_api_runtime()?;
        let (metadata, staged) = self
            .staging_store
            .resolve(&staging_handle, &principal.username, &principal.authenticated_app_id, &runtime.zone_id, PikgStagingPurpose::Inspect, None)
            .await?;
        let reader = self.staging_store.reader(&staged, &metadata.pikg_digest).await.map_err(aiws_error)?;
        let package = reader.inspection();
        if &metadata.pikg_digest != pikg_digest || package.app_doc_object_id != inspection.plan.app.object_id {
            return Err(aiws_error("the staged PIKG is not the inspected App Document"));
        }
        if package.app_doc.get_app_type() != AppType::Aiws {
            return Err(aiws_error("not an aiws package"));
        }
        if let Err(denied) = crate::product::current_product().await?.admit_app(&package.app_doc, AdmissionOrigin::User) {
            return Err(Self::install_error_to_rpc(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::UnsupportedTarget,
                false,
                format!("{denied} ({})", denied.code()),
            )));
        }
        let owner = inspection.plan.owner_user_id.clone();
        // the aiworkspace service reads the staged object from the named store and checks the bundle itself
        let client = AiWorkspaceClient::new(runtime.get_zone_service_krpc_client(AIWORKSPACE_SERVICE_NAME).await?);
        let result = client.call("ext.install", json!({ "pikg": { "obj_id": staged.to_string() }, "mode": "installer", "owner": owner })).await?;
        if result.get("ok") == Some(&json!(false)) {
            let error = &result["error"];
            return Err(reason(
                &format!("aiws_{}", error["code"].as_str().unwrap_or("INSTALL_FAILED").to_ascii_lowercase()),
                error["detail"].as_str().unwrap_or("the AI Workspace library refused the package"),
            ));
        }
        info!("AI Workspace controls {} {} for {owner}: {}", result["pkg_id"], result["version"], result["action"]);
        Ok(json!({
            "action": result["action"],
            "task_id": null,
            "pkg_id": result["pkg_id"],
            "version": result["version"],
            "controls": result["controls"],
            "app_doc_object_id": package.app_doc_object_id,
        }))
    }
}
