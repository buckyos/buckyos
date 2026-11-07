//! Extension packages: `system/ext_tools/desired/<id>` × each node's platform becomes `nodes/<node>/config.ext_tools`
//! (ExtTools TODO §3.3, decision 15). node_daemon converges on that and never reads the Zone-level keys.

use std::collections::{BTreeMap, HashMap};

use buckyos_api::{ext_tools_for_node, ExtToolsDesired, NodeExtTools, EXT_TOOLS_DESIRED_PREFIX};
use buckyos_kit::KVAction;
use log::*;
use name_lib::DeviceInfo;
use serde_json::Value;

use crate::executor_auth::node_ids;

fn desired_state(input: &HashMap<String, String>) -> BTreeMap<String, ExtToolsDesired> {
    input
        .iter()
        .filter_map(|(key, raw)| {
            let id = key.strip_prefix(EXT_TOOLS_DESIRED_PREFIX)?;
            match serde_json::from_str(raw) {
                Ok(desired) => Some((id.to_string(), desired)),
                Err(err) => {
                    warn!("ext_tools: {key} is invalid: {err}");
                    None
                }
            }
        })
        .collect()
}

fn current(input: &HashMap<String, String>, node_id: &str) -> BTreeMap<String, NodeExtTools> {
    input
        .get(&buckyos_api::node_config_key(node_id))
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|config| config.get("ext_tools").cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(crate) fn update_ext_tools_projection(input: &HashMap<String, String>) -> HashMap<String, KVAction> {
    let desired = desired_state(input);
    let mut actions = HashMap::new();
    for node_id in node_ids(input) {
        let Some(info) = input.get(&format!("devices/{node_id}/info")).and_then(|raw| serde_json::from_str::<DeviceInfo>(raw).ok()) else {
            continue;
        };
        let next = ext_tools_for_node(&desired, &info.os, &info.arch, info.device_doc.support_container);
        if next != current(input, &node_id) {
            let value = serde_json::to_value(&next).unwrap_or(Value::Null);
            actions.insert(buckyos_api::node_config_key(&node_id), KVAction::SetByJsonPath(HashMap::from([("/ext_tools".to_string(), Some(value))])));
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::ext_tools_desired_key;
    use name_lib::DeviceDocument;
    use serde_json::json;

    fn device(os: &str, arch: &str, containers: bool) -> String {
        let mut config = DeviceDocument::new("node1", "2bh0FYj-placeholder-x25519-public-key-000000".to_string());
        config.support_container = containers;
        let mut info = DeviceInfo::from_device_doc(&config);
        info.os = os.to_string();
        info.arch = arch.to_string();
        serde_json::to_string(&info).unwrap()
    }

    fn desired(version: &str, enabled: bool) -> String {
        serde_json::to_string(&ExtToolsDesired {
            version: version.to_string(),
            pikg_digest: format!("sha256:{version}"),
            content_id: format!("file:{version}"),
            enabled,
            platforms: vec!["linux-x86_64".to_string(), "darwin-arm64".to_string()],
            by: "root".to_string(),
            at_ms: 1,
        })
        .unwrap()
    }

    fn input() -> HashMap<String, String> {
        HashMap::from([
            ("nodes/node1/config".to_string(), json!({"node_id":"node1"}).to_string()),
            ("devices/node1/info".to_string(), device("linux", "amd64", true)),
        ])
    }

    fn projected(actions: &HashMap<String, KVAction>) -> Value {
        let KVAction::SetByJsonPath(paths) = &actions["nodes/node1/config"] else { panic!("{actions:?}") };
        paths["/ext_tools"].clone().unwrap()
    }

    #[test]
    fn the_desired_state_reaches_each_node_for_its_platform_and_only_when_it_changes() {
        let mut input = input();
        assert!(update_ext_tools_projection(&input).is_empty(), "nothing desired, nothing written");

        input.insert(ext_tools_desired_key("ext-core"), desired("1.0.0", true));
        let actions = update_ext_tools_projection(&input);
        let value = projected(&actions);
        assert_eq!(value["ext-core"]["version"], "1.0.0");
        assert_eq!(value["ext-core"]["payload"], "linux-x86_64");
        assert_eq!(value["ext-core"]["container_payload"], "linux-x86_64");

        input.insert("nodes/node1/config".to_string(), json!({"node_id":"node1", "ext_tools": value}).to_string());
        assert!(update_ext_tools_projection(&input).is_empty(), "already there: no write, no RBAC churn");

        input.insert(ext_tools_desired_key("ext-core"), desired("1.1.0", false));
        let value = projected(&update_ext_tools_projection(&input));
        assert_eq!(value["ext-core"]["version"], "1.1.0");
        assert_eq!(value["ext-core"]["enabled"], false);

        input.remove(&ext_tools_desired_key("ext-core"));
        assert_eq!(projected(&update_ext_tools_projection(&input)), json!({}));
    }

    #[test]
    fn a_platform_with_no_payload_is_unsupported_and_a_silent_node_waits() {
        let mut input = input();
        input.insert(ext_tools_desired_key("ext-core"), desired("1.0.0", true));
        input.insert("devices/node1/info".to_string(), device("windows", "amd64", false));
        let value = projected(&update_ext_tools_projection(&input));
        assert!(value["ext-core"].get("payload").is_none());
        assert!(value["ext-core"].get("container_payload").is_none());
        input.remove("devices/node1/info");
        assert!(update_ext_tools_projection(&input).is_empty());
    }

    #[test]
    fn executor_and_extension_writes_to_one_node_config_merge() {
        let mut merged = HashMap::from([("nodes/node1/config".to_string(), KVAction::SetByJsonPath(HashMap::from([("/executor".to_string(), Some(json!({})))])))]);
        let mut input = input();
        input.insert(ext_tools_desired_key("ext-core"), desired("1.0.0", true));
        buckyos_kit::extend_kv_action_map(&mut merged, &update_ext_tools_projection(&input));
        let KVAction::SetByJsonPath(paths) = &merged["nodes/node1/config"] else { panic!() };
        assert!(paths.contains_key("/executor") && paths.contains_key("/ext_tools"));
    }
}
