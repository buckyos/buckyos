use buckyos_api::*;
use name_lib::DID;
use ndn_lib::{NamedObject, ObjId};
use serde_json::json;

pub fn plan() -> InstallPlan {
    let owner = DID::new("bns", "alice");
    let app_doc = AppDoc::builder(AppType::Web, "notes", "0.1.2", "alice", &owner)
        .web_pkg(
            SubPkgDesc::new("all.web.notes.alice.bns.did#0.1.2")
                .package_meta_object_id(ObjId::new_by_raw("pkg".into(), vec![1; 32])),
        )
        .build()
        .unwrap();
    let object_id = app_doc.gen_obj_id().0;
    let app = AppDocumentRef {
        did: app_doc.app_did().clone(),
        object_id: object_id.clone(),
        show_name: "Notes".into(),
        version: "0.1.2".into(),
    };
    let resolution = serde_json::from_value(json!({
        "app_did": app.did, "doc_type": "app", "app_doc_object_id": object_id,
        "document_status": "Active",
    }))
    .unwrap();
    let mut plan = InstallPlan {
        schema_version: APP_INSTALL_SCHEMA_VERSION,
        plan_use: InstallPlanUse::FreshInstall,
        task_id: "install-1".into(),
        app_instance_id: AppInstanceId::from_app_did(&app.did, "alice").unwrap(),
        owner_user_id: "alice".into(),
        source_identity: InstallSourceIdentity::Catalog {
            app_doc_object_id: object_id,
        },
        app,
        app_doc,
        resolution,
        target: serde_json::from_value(json!({"os": "linux", "arch": "amd64"})).unwrap(),
        selected_packages: Vec::new(),
        required_contents: Vec::new(),
        install_params: InstallParams::default(),
        service_spec_config: ServiceSpecConfig::default(),
        plan_fingerprint: String::new(),
        created_at: 1,
    };
    plan.plan_fingerprint = plan.expected_fingerprint();
    plan
}

#[allow(dead_code)]
pub fn installation(plan: &InstallPlan, generation: u64) -> (AppServiceSpec, InstallRecord) {
    let deployment = DeploymentIdentity {
        app_instance_id: plan.app_instance_id.clone(),
        task_id: plan.task_id.clone(),
        app_doc_object_id: plan.app.object_id.clone(),
        spec_generation: generation,
        pikg_digest: None,
    };
    let spec = AppServiceSpec {
        app_instance_id: plan.app_instance_id.clone(),
        app_did: plan.app.did.clone(),
        deployment: deployment.clone(),
        app_doc: plan.app_doc.clone(),
        app_name: "notes".into(),
        app_host_name: "notes-alice".into(),
        app_index: 11,
        owner_user_id: plan.owner_user_id.clone(),
        permission: Vec::new(),
        selected_components: Vec::new(),
        packages: Vec::new(),
        enable: true,
        expected_instance_count: 1,
        state: ServiceState::New,
        spec_config: plan.service_spec_config.clone(),
    };
    let installed = InstallRecord {
        schema_version: APP_INSTALL_SCHEMA_VERSION,
        app: plan.app.clone(),
        owner_user_id: plan.owner_user_id.clone(),
        app_instance_id: plan.app_instance_id.clone(),
        resolution: plan.resolution.clone(),
        package_meta_ids: Vec::new(),
        pikg_digest: None,
        target: plan.target.clone(),
        install_params: plan.install_params.clone(),
        service_spec_config: plan.service_spec_config.clone(),
        target_deployment: Some(deployment),
        previous_deployment: None,
        state: InstallRecordState::Deploying,
        task_id: plan.task_id.clone(),
        proof_id: None,
        plan_fingerprint: plan.plan_fingerprint.clone(),
        created_at: 1,
        updated_at: 1,
        last_error: None,
    };
    (spec, installed)
}
