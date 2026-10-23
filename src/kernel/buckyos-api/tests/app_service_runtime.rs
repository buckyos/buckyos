use buckyos_api::{
    init_buckyos_api_runtime, BuckyOSRuntimeType, BUCKYOS_APP_DID_ENV, BUCKYOS_APP_ID_ENV,
    BUCKYOS_APP_INSTANCE_ID_ENV, BUCKYOS_APP_TOKEN_ENV, BUCKYOS_DATA_DIR_ENV,
    BUCKYOS_OWNER_USER_ID_ENV,
};
use buckyos_kit::BuckyOSMachineConfig;
use name_lib::DID;
use std::{env, fs};

#[tokio::test]
async fn app_service_loads_machine_policy_and_keeps_env_identity() {
    let root = env::temp_dir().join(format!("buckyos-appservice-{}", uuid::Uuid::new_v4()));
    let app_id = "buckyos-jarvis.devtest.bns.did";
    env::set_var("BUCKYOS_ROOT", &root);
    env::set_var(BUCKYOS_APP_DID_ENV, "did:bns:buckyos-jarvis.devtest");
    env::set_var(BUCKYOS_APP_ID_ENV, app_id);
    env::set_var(BUCKYOS_APP_INSTANCE_ID_ENV, format!("{app_id}@devtest"));
    env::set_var(BUCKYOS_OWNER_USER_ID_ENV, "devtest");
    env::set_var(BUCKYOS_DATA_DIR_ENV, root.join("data"));
    env::set_var(BUCKYOS_APP_TOKEN_ENV, "dummy-session-token");
    env::remove_var("BUCKYOS_ZONE_CONFIG");
    env::remove_var("BUCKYOS_THIS_DEVICE");

    for has_etc in [false, true] {
        if has_etc {
            fs::create_dir_all(root.join("etc")).unwrap();
        }
        let runtime = init_buckyos_api_runtime("", None, BuckyOSRuntimeType::AppService)
            .await
            .unwrap();
        let defaults = BuckyOSMachineConfig::default();
        assert_eq!(runtime.web3_bridges, defaults.web3_bridge);
        assert_eq!(runtime.force_https, defaults.force_https);
    }
    fs::write(
        root.join("etc/machine.json"),
        serde_json::to_vec(&serde_json::json!({
            "web3_bridge": {"bns": "web3.devtests.org"},
            "force_https": false,
        }))
        .unwrap(),
    )
    .unwrap();
    let runtime = init_buckyos_api_runtime("", None, BuckyOSRuntimeType::AppService)
        .await
        .unwrap();
    assert_eq!(runtime.get_app_id(), app_id);
    assert_eq!(runtime.get_owner_user_id().as_deref(), Some("devtest"));
    assert_eq!(runtime.user_id.as_deref(), Some("devtest"));
    assert_eq!(runtime.get_authenticated_user_id().as_deref(), None);
    assert!(runtime.device_private_key.is_none());
    assert!(runtime.user_private_key.is_none());
    assert_eq!(
        runtime.session_token.read().await.as_str(),
        "dummy-session-token"
    );
    assert!(!runtime.force_https);
    assert_eq!(
        runtime.web3_bridges.get("bns").map(String::as_str),
        Some("web3.devtests.org")
    );
    name_client::init_name_lib_for_test(&runtime.web3_bridges)
        .await
        .unwrap();
    assert_eq!(
        DID::new("bns", "lurenpluto").to_host_name(),
        "lurenpluto.web3.devtests.org"
    );
    fs::write(root.join("etc/machine.json"), "{").unwrap();
    let error = init_buckyos_api_runtime("", None, BuckyOSRuntimeType::AppService)
        .await
        .err()
        .expect("malformed machine config must be rejected");
    assert!(error.to_string().contains("Failed to parse machine_config"));
    fs::remove_dir_all(root).unwrap();
}
