use crate::ControlPanelServer;
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{get_buckyos_api_runtime, SystemConfigClient};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

const SYS_CONFIG_TREE_MAX_DEPTH: u64 = 24;

impl ControlPanelServer {
    pub(crate) async fn handle_system_config_test(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let key = req
            .params
            .get("key")
            .and_then(|value| value.as_str())
            .unwrap_or("boot/config")
            .to_string();
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let value = client
            .get(&key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "value": value.value,
                "version": value.version,
                "isChanged": value.is_changed,
            })),
            req.seq,
        ))
    }
    
    pub(crate) async fn handle_sys_config_get(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let key = Self::require_param_str(&req, "key")?;
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let value = client
            .get(&key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "value": value.value,
                "version": value.version,
                "isChanged": value.is_changed,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_sys_config_set(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let key = Self::require_param_str(&req, "key")?;
        let value = Self::require_param_str(&req, "value")?;
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        client
            .set(&key, &value)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "key": key,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_sys_config_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let key = Self::param_str(&req, "key")
            .or_else(|| Self::param_str(&req, "prefix"))
            .unwrap_or_default();
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let items = client
            .list(&key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "items": items,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn build_sys_config_tree(
        &self,
        client: &SystemConfigClient,
        key: &str,
        depth: u64,
    ) -> Result<Value, RPCErrors> {
        if depth == 0 {
            return Ok(json!({}));
        }

        let mut queue: Vec<(String, u64)> = vec![(key.to_string(), depth)];
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();

        while let Some((current_key, current_depth)) = queue.pop() {
            if current_depth == 0 {
                continue;
            }
            let children = client
                .list(&current_key)
                .await
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
            children_map.insert(current_key.clone(), children.clone());
            if current_depth > 1 {
                for child in children {
                    let child_key = if current_key.is_empty() || child.starts_with(&current_key) {
                        child
                    } else {
                        format!("{}/{}", current_key, child)
                    };
                    queue.push((child_key, current_depth - 1));
                }
            }
        }

        fn build_tree_node(
            children_map: &HashMap<String, Vec<String>>,
            key: &str,
            depth: u64,
        ) -> Value {
            if depth == 0 {
                return json!({});
            }
            let mut map = Map::new();
            let children = children_map.get(key).cloned().unwrap_or_default();
            for child in children {
                let child_key = if key.is_empty() || child.starts_with(key) {
                    child.clone()
                } else {
                    format!("{}/{}", key, child)
                };
                let child_name = child
                    .split('/')
                    .next_back()
                    .unwrap_or(child.as_str())
                    .to_string();
                let subtree = if depth > 1 {
                    build_tree_node(children_map, &child_key, depth - 1)
                } else {
                    json!({})
                };
                map.insert(child_name, subtree);
            }
            Value::Object(map)
        }

        Ok(build_tree_node(&children_map, key, depth))
    }

    pub(crate) async fn handle_sys_config_tree(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let key = Self::param_str(&req, "key")
            .or_else(|| Self::param_str(&req, "prefix"))
            .unwrap_or_default();
        let depth = req
            .params
            .get("depth")
            .and_then(|value| value.as_u64())
            .unwrap_or(2);
        let depth = depth.min(SYS_CONFIG_TREE_MAX_DEPTH);
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let tree = self.build_sys_config_tree(&client, &key, depth).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "depth": depth,
                "tree": tree,
            })),
            req.seq,
        ))
    }
}
