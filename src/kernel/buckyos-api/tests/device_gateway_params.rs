use buckyos_api::save_node_gateway_params;
use name_lib::DID;
use serde_json::Value;

#[test]
fn gateway_params_follow_device_port_and_reject_invalid_ports() {
    let root = std::env::temp_dir().join(format!("gateway-params-{}", uuid::Uuid::new_v4()));
    let did = DID::new("web", "ood1.charlie.me");
    for (port, expected) in [(None, "2980"), (Some(2981), "2981")] {
        save_node_gateway_params(&root, &did, port).unwrap();
        let saved: Value =
            serde_json::from_slice(&std::fs::read(root.join("node_gateway_params.json")).unwrap())
                .unwrap();
        assert_eq!(saved["params"]["device_did"], did.to_string());
        assert_eq!(saved["params"]["rtcp_port"], expected);
    }
    for port in [0, 65536, u32::MAX] {
        assert!(save_node_gateway_params(&root, &did, Some(port)).is_err());
    }
    let saved: Value =
        serde_json::from_slice(&std::fs::read(root.join("node_gateway_params.json")).unwrap())
            .unwrap();
    assert_eq!(saved["params"]["rtcp_port"], "2981");
    std::fs::remove_dir_all(root).unwrap();
}
