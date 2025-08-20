mod kv_provider;
//mod etcd_provider;
//mod rocksdb_provider;
mod sled_provider;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::DecodingKey;
use log::*;

use lazy_static::lazy_static;
use serde_json::Value;
use tokio::sync::Mutex;
use warp::Filter;

use ::kRPC::*;
use buckyos_kit::*;
use kv_provider::KVStoreProvider;
use name_lib::*;
use rbac::*;
use sled_provider::SledStore;

lazy_static! {
    static ref TRUST_KEYS: Arc<Mutex<HashMap<String, DecodingKey>>> = {
        let hashmap: HashMap<String, DecodingKey> = HashMap::new();
        Arc::new(Mutex::new(hashmap))
    };
}

lazy_static! {
    static ref SYS_STORE: Arc<Mutex<dyn KVStoreProvider>> =
        Arc::new(Mutex::new(SledStore::new().unwrap()));
}


fn get_full_res_path(key_path:&str) -> Result<(String,String)> {
    let mut real_key_path = key_path;
    if key_path.starts_with("kv://") {
        real_key_path = &key_path[6..];
    }

    let key = real_key_path.trim_start_matches('/').trim_start_matches('\\');
    let normalized_path = normalize_path(key);
   
    return Ok((format!("kv://{}", normalized_path.as_str()),normalized_path));
}


async fn handle_get(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }

    let key = key.unwrap();
    let key = key.as_str();
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    let key = key.unwrap();

    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();

    let appid = session_token.appid.as_deref().unwrap_or("kernel");

    let (full_res_path,real_key_path) = get_full_res_path(key)?;
    info!("full_res_path:{},real_key_path:{:?},appid:{},userid:{},session_token:{:?}",full_res_path,real_key_path,appid,userid,session_token);
    let is_allowed = enforce(userid, Some(appid), full_res_path.as_str(), "read").await;
    if !is_allowed {
        warn!("No read permission");
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    };

    let store = SYS_STORE.lock().await;
    let result = store
        .get(real_key_path)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    if result.is_none() {
        return Ok(Value::Null);
    } else {
        return Ok(Value::String(result.unwrap()));
    }
}

async fn handle_set(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    //check params
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    let key = key.unwrap();
    let key = key.as_str().unwrap();

    let new_value = params.get("value");
    if new_value.is_none() {
        return Err(RPCErrors::ReasonError("Missing value".to_string()));
    }
    let new_value = new_value.unwrap();
    let new_value = new_value.as_str().unwrap();

    //check access control
    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();
    let (full_res_path,real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        session_token.appid.as_deref(),
        full_res_path.as_str(),
        "write",
    )
    .await
    {
        return Err(RPCErrors::NoPermission("No write permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    info!("Set key:[{}] to value:[{}]", key, new_value);
    store
        .set(real_key_path, String::from(new_value))
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    return Ok(Value::Null);
}

async fn handle_create(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    //check params
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    let key = key.unwrap();
    let key = key.as_str().unwrap();

    let new_value = params.get("value");
    if new_value.is_none() {
        return Err(RPCErrors::ReasonError("Missing value".to_string()));
    }
    let new_value = new_value.unwrap();
    let new_value = new_value.as_str().unwrap();

    //check access control
    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();
    let (full_res_path,real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        session_token.appid.as_deref(),
        full_res_path.as_str(),
        "write",
    )
    .await
    {
        return Err(RPCErrors::NoPermission("No write permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    info!("Create key:[{}] to value:[{}]", key, new_value);
    store
        .create(&real_key_path, new_value)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    //if key is boot/config,will update trust_keys

    return Ok(Value::Null);
}

async fn handle_delete(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    //check params
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    let key = key.unwrap();
    let key = key.as_str().unwrap();

    //check access control
    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();
    let (full_res_path,real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        session_token.appid.as_deref(),
        full_res_path.as_str(),
        "write",
    )
    .await
    {
        return Err(RPCErrors::NoPermission("No write permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    info!("Delete key:[{}]", key);
    store
        .delete(&real_key_path)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    return Ok(Value::Null);
}

async fn handle_append(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    let key = key.unwrap();
    let key = key.as_str().unwrap();

    let append_value = params.get("append_value");
    if append_value.is_none() {
        return Err(RPCErrors::ReasonError("Missing append_value".to_string()));
    }
    let append_value = append_value.unwrap();
    let append_value = append_value.as_str().unwrap();

    //check access control
    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();
    let (full_res_path,real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        session_token.appid.as_deref(),
        full_res_path.as_str(),
        "write",
    )
    .await
    {
        return Err(RPCErrors::NoPermission("No write permission".to_string()));
    }

    //read and append
    let store = SYS_STORE.lock().await;
    let result = store
        .get(real_key_path.clone())
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    if result.is_none() {
        warn!("key:[{}] not exist,cann't append", key);
        return Err(RPCErrors::KeyNotExist(key.to_string()));
    } else {
        let old_value = result.unwrap();
        let new_value = format!("{}{}", old_value, append_value);
        store
            .set(real_key_path, new_value)
            .await
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        return Ok(Value::Null);
    }
}

async fn handle_set_by_json_path(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    //check params
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    let key = key.unwrap();
    let key = key.as_str().unwrap();

    let new_value = params.get("value");
    if new_value.is_none() {
        return Err(RPCErrors::ReasonError("Missing value".to_string()));
    }
    let new_value = new_value.unwrap();
    let new_value = new_value.as_str().unwrap();
    let new_value: Value =
        serde_json::from_str(new_value).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    let json_path = params.get("json_path");
    if json_path.is_none() {
        return Err(RPCErrors::ReasonError("Missing json_path".to_string()));
    }
    let json_path = json_path.unwrap();
    let json_path = json_path.as_str().unwrap();

    //check access control
    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();
    let (full_res_path,real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        session_token.appid.as_deref(),
        full_res_path.as_str(),
        "write",
    )
    .await
    {
        return Err(RPCErrors::NoPermission("No write permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    store
        .set_by_path(real_key_path, String::from(json_path), &new_value)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    //let result = store.get(String::from(key)).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    Ok(Value::Null)
}

async fn handle_exec_tx(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    // Check params
    let actions = params.get("actions");
    if actions.is_none() {
        return Err(RPCErrors::ReasonError("Missing actions".to_string()));
    }
    let actions = actions.unwrap();
    if !actions.is_object() {
        return Err(RPCErrors::ReasonError(
            "Actions must be an object".to_string(),
        ));
    }

    // Check access control for all keys
    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();

    let mut tx_actions = HashMap::new();

    // Process each action into KVAction
    for (key, action) in actions.as_object().unwrap() {
        let (full_res_path,real_key_path) = get_full_res_path(key)?;
        if !enforce(
            userid,
            session_token.appid.as_deref(),
            full_res_path.as_str(),
            "write",
        )
        .await
        {
            return Err(RPCErrors::NoPermission(format!(
                "No write permission for key: {}",
                &real_key_path
            )));
        }

        let action_type = action
            .get("action")
            .ok_or(RPCErrors::ReasonError(format!(
                "Missing action type for key: {}",
                &real_key_path
            )))?
            .as_str()
            .ok_or(RPCErrors::ReasonError(
                "Action type must be string".to_string(),
            ))?;

        let kv_action = match action_type {
            "create" => {
                let value = action
                    .get("value")
                    .ok_or(RPCErrors::ReasonError(
                        "Missing value for create".to_string(),
                    ))?
                    .as_str()
                    .ok_or(RPCErrors::ReasonError("Value must be string".to_string()))?;
                KVAction::Create(value.to_string())
            }
            "update" => {
                let value = action
                    .get("value")
                    .ok_or(RPCErrors::ReasonError(
                        "Missing value for update".to_string(),
                    ))?
                    .as_str()
                    .ok_or(RPCErrors::ReasonError("Value must be string".to_string()))?;
                KVAction::Update(value.to_string())
            }
            "append" => {
                let value = action
                    .get("value")
                    .ok_or(RPCErrors::ReasonError(
                        "Missing value for append".to_string(),
                    ))?
                    .as_str()
                    .ok_or(RPCErrors::ReasonError("Value must be string".to_string()))?;
                KVAction::Append(value.to_string())
            }
            "set_by_path" => {
                let all_set = action.get("all_set").ok_or(RPCErrors::ReasonError(
                    "Missing all_set for set_by_path".to_string(),
                ))?;
                //all_set is a json map
                let all_set: HashMap<String, Option<Value>> =
                    serde_json::from_value(all_set.clone())
                        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
                KVAction::SetByJsonPath(all_set)
            }
            "remove" => KVAction::Remove,
            _ => {
                return Err(RPCErrors::ReasonError(format!(
                    "Unknown action type: {}",
                    action_type
                )))
            }
        };
        tx_actions.insert(real_key_path.clone(), kv_action);
    }
    let mut real_main_key = None;
    let main_key = params.get("main_key");
    if main_key.is_some() {
        let main_key = main_key.unwrap();
        let main_key = main_key.as_str().unwrap();
        let main_key = main_key.split(":").collect::<Vec<&str>>();
        let main_key = (main_key[0].to_string(), main_key[1].parse::<u64>().unwrap());
        real_main_key = Some(main_key);
    }

    let store = SYS_STORE.lock().await;
    store
        .exec_tx(tx_actions, real_main_key)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    Ok(Value::Null)
}

async fn handle_list(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    //check params
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    let key = key.unwrap();
    let key = key.as_str().unwrap();

    //check access control
    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();
    let (full_res_path,real_key_path) = get_full_res_path(key)?;
    info!("full_res_path: {},userid: {},appid: {}", full_res_path,userid,session_token.appid.as_deref().unwrap());
    if !enforce(
        userid,
        session_token.appid.as_deref(),
        full_res_path.as_str(),
        "read",
    )
    .await
    {
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    let result = store
        .list_direct_children(real_key_path)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    Ok(Value::Array(
        result.iter().map(|v| Value::String(v.clone())).collect(),
    ))
}

async fn handle_refresh_trust_keys() -> Result<Value> {
    TRUST_KEYS.lock().await.clear();
    info!("TRUST_KEYS cleared,refresh_trust_keys");
    let store = SYS_STORE.lock().await;
    let zone_config = store.get("boot/config".to_string()).await;
    if zone_config.is_ok() {
        let zone_config = zone_config.unwrap();
        if zone_config.is_some() {
            let zone_config_str = zone_config.unwrap();
            //info!("boot_info: {}",boot_info_str);
            let zone_config: ZoneConfig =
                serde_json::from_str(&zone_config_str).map_err(|err| {
                    error!("Failed to parse zone config from boot/config: {}", err);
                    RPCErrors::ReasonError(err.to_string())
                })?;

            if zone_config.verify_hub_info.is_some() {
                let verify_hub_info = zone_config.verify_hub_info.as_ref().unwrap();
                let verify_hub_public_key = DecodingKey::from_jwk(&verify_hub_info.public_key)
                    .map_err(|err| {
                        error!(
                            "Failed to parse verify_hub_public_key from zone_config: {}",
                            err
                        );
                        RPCErrors::ReasonError(err.to_string())
                    })?;

                TRUST_KEYS
                    .lock()
                    .await
                    .insert("verify-hub".to_string(), verify_hub_public_key.clone());
                info!("update verify_hub_public_key to trust keys");
            } else {
                error!("Missing verify_hub_info from zone_config");
            }
            if zone_config.owner.is_some() {
                let owner_key = zone_config.get_default_key();
                let owner_did = zone_config.owner.as_ref().unwrap().clone();
                if owner_key.is_some() {
                    let owner_key = owner_key.unwrap();
                    let owner_public_key = DecodingKey::from_jwk(&owner_key).map_err(|err| {
                        error!("Failed to parse owner_public_key from zone_config: {}", err);
                        RPCErrors::ReasonError(err.to_string())
                    })?;
                    let mut trust_keys = TRUST_KEYS.lock().await;
                    trust_keys.insert(owner_did.to_string(), owner_public_key.clone());
                    trust_keys.insert("root".to_string(), owner_public_key.clone());
                    trust_keys.insert("$default".to_string(), owner_public_key.clone());
                    info!(
                        "update owner_public_key [{}],[{}] to trust keys",
                        owner_did.to_string(),
                        owner_did.id
                    );
                }
            }
        }
        
    }

    let device_doc_str = std::env::var("BUCKYOS_THIS_DEVICE");
    if device_doc_str.is_ok() {
        let device_doc_str = device_doc_str.unwrap();
        let device_doc: DeviceConfig = serde_json::from_str(&device_doc_str).unwrap();
        //device_doc.iss
        let devcie_key = device_doc.get_default_key();

        if devcie_key.is_some() {
            let devcie_key = devcie_key.unwrap();
            let device_key_str = serde_json::to_string(&devcie_key).unwrap();
            let real_key = DecodingKey::from_jwk(&devcie_key).unwrap();
            TRUST_KEYS
                .lock()
                .await
                .insert(device_doc.name.clone(), real_key.clone());
            info!(
                "Insert device name:[{}] - key:[{}] to trust keys",
                device_doc.name, device_key_str
            );

            TRUST_KEYS
                .lock()
                .await
                .insert(device_doc.id.to_string(), real_key);
            info!(
                "Insert device did:[{}] - key:[{}] to trust keys",
                device_doc.id.to_string(),
                device_key_str
            );
        }
    } else {
        error!("Missing BUCKYOS_THIS_DEVICE");
    }
    
    let rbac_model = store.get("system/rbac/model".to_string()).await;
    let rbac_policy = store.get("system/rbac/policy".to_string()).await;
    let mut set_rbac = false;
    if rbac_model.is_ok() && rbac_policy.is_ok() {
        let rbac_model = rbac_model.unwrap();
        let rbac_policy = rbac_policy.unwrap();
        if rbac_model.is_some() && rbac_policy.is_some() {
            info!("model config: {}", rbac_model.clone().unwrap());
            info!("policy config: {}", rbac_policy.clone().unwrap());
            rbac::create_enforcer(
                Some(rbac_model.unwrap().trim()),
                Some(rbac_policy.unwrap().trim()),
            )
            .await
            .unwrap();
            set_rbac = true;
            info!("load rbac model and policy from kv store successfully!");
        }
    }

    if !set_rbac {
        rbac::create_enforcer(None, None).await.unwrap();
        info!("load rbac model and policy default setting successfully!");
    }

    Ok(Value::Null)
}

async fn dump_configs_for_scheduler(
    _params: Value,
    session_token: &RPCSessionToken,
) -> Result<Value> {
    let appid = session_token.appid.as_deref().unwrap();
    if appid != "scheduler" && appid != "node-daemon" {
        return Err(RPCErrors::NoPermission("No permission".to_string()));
    }

    let store = SYS_STORE.lock().await;
    let mut config_map = HashMap::new();

    let boot_config = store
        .list_data("boot/")
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(boot_config);
    let devices_config = store
        .list_data("devices/")
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(devices_config);
    let users_config = store
        .list_data("users/")
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(users_config);
    let services_config = store
        .list_data("services/")
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(services_config);
    let system_config = store
        .list_data("system/")
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(system_config);
    let node_config = store
        .list_data("nodes/")
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(node_config);


    let config_map = serde_json::to_value(&config_map).unwrap();
    return Ok(config_map);
}

async fn process_request(
    method: String,
    param: Value,
    session_token: Option<String>,
) -> ::kRPC::Result<Value> {
    //check session_token
    if session_token.is_some() {
        let session_token = session_token.unwrap();
        let mut rpc_session_token = RPCSessionToken::from_string(session_token.as_str())?;
        //veruft session token (need access trust did_list)
        verify_session_token(&mut rpc_session_token).await?;
        if rpc_session_token.exp.is_some() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now > rpc_session_token.exp.unwrap() {
                warn!("session token expired: {}", session_token);
                return Err(RPCErrors::TokenExpired(session_token));
            }
            debug!("session token is valid: {}", session_token);
        }
        debug!("ready to handle request : {}", method.as_str());
        match method.as_str() {
            "sys_config_create" => {
                return handle_create(param, &rpc_session_token).await;
            }
            "sys_config_get" => {
                return handle_get(param, &rpc_session_token).await;
            }
            "sys_config_set" => {
                return handle_set(param, &rpc_session_token).await;
            }
            "sys_config_set_by_json_path" => {
                return handle_set_by_json_path(param, &rpc_session_token).await;
            }
            "sys_config_exec_tx" => {
                return handle_exec_tx(param, &rpc_session_token).await;
            }
            "sys_config_delete" => {
                return handle_delete(param, &rpc_session_token).await;
            }
            "sys_config_append" => {
                return handle_append(param, &rpc_session_token).await;
            }
            "sys_config_list" => {
                return handle_list(param, &rpc_session_token).await;
            }
            "dump_configs_for_scheduler" => {
                return dump_configs_for_scheduler(param, &rpc_session_token).await;
            }
            "sys_refresh_trust_keys" => {
                return handle_refresh_trust_keys().await;
            }
            // Add more methods here
            _ => Err(RPCErrors::UnknownMethod(String::from(method))),
        }
    } else {
        return Err(RPCErrors::NoPermission("No session token".to_string()));
    }
}

async fn load_device_doc(device_name:&str) -> Result<DeviceConfig> {
    let store = SYS_STORE.lock().await;
    let device_doc = store.get(format!("devices/{}/doc", device_name))
        .await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    if device_doc.is_none() {
        return Err(RPCErrors::KeyNotExist(format!("devices/{}/doc", device_name)));
    }
    let device_doc_str = device_doc.unwrap();
    let device_doc: EncodedDocument = EncodedDocument::from_str(device_doc_str.clone())
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    let device_doc: DeviceConfig = DeviceConfig::decode(&device_doc, None)
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    return Ok(device_doc);
}

async fn verify_session_token(token: &mut RPCSessionToken) -> Result<()> {
    if token.is_self_verify() {
        let mut trust_keys = TRUST_KEYS.lock().await;
        let kid = token.verify_by_key_map(&trust_keys);
        if kid.is_err() {
            let kid_err = kid.err().unwrap();
            match kid_err {
                RPCErrors::KeyNotExist(kid) => {
                    info!("kid not exist: {},try to load device doc", kid);
                    //kid is device name, try load device doc
                    let device_doc = load_device_doc(kid.as_str()).await?;
                    if device_doc.device_type != "ood" && device_doc.device_type != "node" {
                        return Err(RPCErrors::ReasonError(format!("device type is not ood or node: {}", kid)));
                    }
                    let device_key = device_doc.get_default_key();
                    if device_key.is_some() {
                        let device_key = device_key.unwrap();
                        info!("load device {} doc successfully, insert public key to trust keys", kid);
                        trust_keys.insert(kid.clone(), DecodingKey::from_jwk(&device_key).unwrap());
                    }
                    token.verify_by_key_map(&trust_keys)?;
                    return Ok(());
                },
                _ => {
                    return Err(kid_err);
                }
            }
        }
        debug!("verify_session_token: {:?}", token);
        return Ok(())
    } else {
        unimplemented!();
    }

}

async fn init_by_boot_config() -> Result<()> {
    let r = handle_refresh_trust_keys().await;
    if r.is_err() {
        error!("Failed to refresh trust keys: {}", r.err().unwrap());
    }

    let device_doc_str = std::env::var("BUCKYOS_THIS_DEVICE");
    if device_doc_str.is_ok() {
        let device_doc_str = device_doc_str.unwrap();
        let device_doc: DeviceConfig = serde_json::from_str(&device_doc_str).unwrap();
        //device_doc.iss
        let devcie_key = device_doc.get_default_key();

        if devcie_key.is_some() {
            let devcie_key = devcie_key.unwrap();
            let device_key_str = serde_json::to_string(&devcie_key).unwrap();
            let real_key = DecodingKey::from_jwk(&devcie_key).unwrap();
            TRUST_KEYS
                .lock()
                .await
                .insert(device_doc.name.clone(), real_key.clone());
            info!(
                "Insert device name:[{}] - key:[{}] to trust keys",
                device_doc.name, device_key_str
            );

            TRUST_KEYS
                .lock()
                .await
                .insert(device_doc.id.to_string(), real_key);
            info!(
                "Insert device did:[{}] - key:[{}] to trust keys",
                device_doc.id.to_string(),
                device_key_str
            );
        }
    } else {
        error!("Missing BUCKYOS_THIS_DEVICE");
    }

    Ok(())
}

async fn service_main() {
    //std::env::set_var("BUCKY_LOG","debug");
    init_logging("system_config_service", true);
    info!("Starting system config service............................");
    init_by_boot_config().await.unwrap();
    // Select the rear end storage, here you can switch different implementation

    let cors_response = warp::path!("kapi" / "system_config")
        .and(warp::options())
        .map(|| {
            info!("Handling OPTIONS request");
            warp::http::Response::builder()
                .header("Access-Control-Allow-Origin", "*")
                .header("Access-Control-Allow-Methods", "POST, OPTIONS")
                .header("Access-Control-Allow-Headers", "Content-Type")
                .body("")
        });

    let rpc_route = warp::path!("kapi" / "system_config")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(|req: RPCRequest| async {
            info!(
                "|==>Received request: {}",
                serde_json::to_string(&req).unwrap()
            );

            let process_result = process_request(req.method, req.params, req.token).await;

            let rpc_response: RPCResponse;
            match process_result {
                Ok(result) => {
                    rpc_response = RPCResponse {
                        result: RPCResult::Success(result),
                        seq: req.id,
                        token: None,
                        trace_id: req.trace_id.clone(),
                    };
                    info!(
                        "<==|Response: OK {} {}",
                        req.id,
                        req.trace_id.as_deref().unwrap_or("")
                    );
                }
                Err(err) => {
                    rpc_response = RPCResponse {
                        result: RPCResult::Failed(err.to_string()),
                        seq: req.id,
                        token: None,
                        trace_id: req.trace_id,
                    };
                    info!(
                        "<==|Response: {}",
                        serde_json::to_string(&rpc_response).unwrap()
                    );
                }
            }

            Ok::<_, warp::Rejection>(warp::reply::json(&rpc_response))
        });

    info!("Starting system config service");
    warp::serve(cors_response.or(rpc_route))
        .run(([0, 0, 0, 0], 3200))
        .await;
}

#[tokio::main]
async fn main() {
    service_main().await;
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use jsonwebtoken::EncodingKey;
    use serde_json::json;
    use tokio::{task, time::sleep};

    use super::*;
    //#[tokio::test(flavor = "current_thread")]
    async fn test_server_interface() {
        {
            let jwk = json!(
                {
                    "kty": "OKP",
                    "crv": "Ed25519",
                    "x": "vZ2kEJdazmmmmxTYIuVPCt0gGgMOnBP6mMrQmqminB0"
                }
            );
            let result_key: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
            let mut hashmap = TRUST_KEYS.lock().await;

            hashmap.insert(
                "{owner}".to_string(),
                DecodingKey::from_jwk(&result_key).unwrap(),
            );
        }

        let server = task::spawn(async {
            service_main().await;
        });

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let test_owner_private_key_pem = r#"
        -----BEGIN PRIVATE KEY-----
        MC4CAQAwBQYDK2VwBCIEIK45kLWIAx3CHmbEmyCST4YB3InSCA4XAV6udqHtRV5P
        -----END PRIVATE KEY-----
        "#;

        let private_key = EncodingKey::from_ed_pem(test_owner_private_key_pem.as_bytes()).unwrap();
        let token = RPCSessionToken {
            userid: Some("alice".to_string()),
            appid: Some("test".to_string()),
            exp: Some(now + 5), //5 seconds
            token_type: RPCSessionTokenType::JWT,
            token: None,
            iss: None,
            nonce: None,
            session: None,
        };
        let jwt = token
            .generate_jwt(Some("{owner}".to_string()), &private_key)
            .unwrap();

        sleep(Duration::from_millis(1000)).await;

        let client = kRPC::new("http://127.0.0.1:3200/kapi/system_config", Some(jwt));
        //test create
        println!("test create");
        client
            .call(
                "sys_config_create",
                json!( {"key":"users/alice/test_key","value":"test_value_create"}),
            )
            .await
            .unwrap();
        //test set
        println!("test set");
        let _ = client
            .call(
                "sys_config_set",
                json!( {"key":"users/alice/test_key","value":"test_value"}),
            )
            .await
            .unwrap();

        //test no permission set
        println!("test no permission set");
        let result = client
            .call(
                "sys_config_set",
                json!( {"key":"users/bob/test_key","value":"test_value"}),
            )
            .await;
        assert!(result.is_err());
        //test already exist create
        println!("test already exist create");
        let result = client
            .call(
                "sys_config_create",
                json!( {"key":"users/alice/test_key","value":"test_value_create"}),
            )
            .await;
        assert!(result.is_err());
        //test delete
        println!("test delete");
        client
            .call("sys_config_delete", json!( {"key":"users/alice/test_key"}))
            .await
            .unwrap();
        //test delete not exist
        println!("test delete not exist");
        let result = client
            .call("sys_config_delete", json!( {"key":"users/alice/test_key"}))
            .await;
        assert!(result.is_err());

        //test set by json pathf
        println!("test set by json path");
        client
            .call(
                "sys_config_create",
                json!( {"key":"users/alice/test_json_key","value":"{\"field\":\"old_value\"}"}),
            )
            .await
            .unwrap();
        client.call("sys_config_set_by_json_path", json!( {"key":"users/alice/test_json_key","json_path":"/field","value":"\"new_value\""})).await.unwrap();
        let result = client
            .call(
                "sys_config_get",
                json!( {"key":"users/alice/test_json_key"}),
            )
            .await
            .unwrap();
        assert_eq!(result.as_str().unwrap(), "{\"field\":\"new_value\"}");
        //test token expired
        sleep(Duration::from_millis(8000)).await;
        println!("test token expired");
        let result = client
            .call(
                "sys_config_set",
                json!( {"key":"users/alice/test_key","value":"test_value"}),
            )
            .await;
        assert!(result.is_err());

        drop(server);
    }

    //#[tokio::test(flavor = "current_thread")]
    async fn test_transaction_processing() {
        // Setup trust keys like in the existing test
        {
            let jwk = json!(
                {
                    "kty": "OKP",
                    "crv": "Ed25519",
                    "x": "vZ2kEJdazmmmmxTYIuVPCt0gGgMOnBP6mMrQmqminB0"
                }
            );
            let result_key: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
            let mut hashmap = TRUST_KEYS.lock().await;
            hashmap.insert(
                "{owner}".to_string(),
                DecodingKey::from_jwk(&result_key).unwrap(),
            );
        }

        let server = task::spawn(async {
            service_main().await;
        });

        // Create JWT token for authentication
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let test_owner_private_key_pem = r#"
        -----BEGIN PRIVATE KEY-----
        MC4CAQAwBQYDK2VwBCIEIK45kLWIAx3CHmbEmyCST4YB3InSCA4XAV6udqHtRV5P
        -----END PRIVATE KEY-----
        "#;

        let private_key = EncodingKey::from_ed_pem(test_owner_private_key_pem.as_bytes()).unwrap();
        let token = RPCSessionToken {
            userid: Some("alice".to_string()),
            appid: None,
            exp: Some(now + 30),
            token_type: RPCSessionTokenType::JWT,
            token: None,
            iss: None,
            nonce: None,
            session: None,
        };
        let jwt = token
            .generate_jwt(Some("alice".to_string()), &private_key)
            .unwrap();

        sleep(Duration::from_millis(1000)).await;
        let client = kRPC::new("http://127.0.0.1:3200/kapi/system_config", Some(jwt));

        // Test transaction with multiple operations
        println!("Testing transaction processing");
        let tx_request = json!({
            "actions": {
                "users/alice/key1": {
                    "action": "create",
                    "value": "value1"
                },
                "users/alice/key2": {
                    "action": "create",
                    "value": "value2"
                }
            }
        });

        // Execute transaction
        let result = client.call("sys_config_exec_tx", tx_request).await;
        assert!(result.is_ok(), "Transaction should succeed");

        // Verify the results
        let get_key1 = client
            .call("sys_config_get", json!({"key": "users/alice/key1"}))
            .await;
        assert_eq!(
            get_key1.unwrap().as_str().unwrap(),
            "value1",
            "Key1 should have correct value"
        );

        let get_key2 = client
            .call("sys_config_get", json!({"key": "users/alice/key2"}))
            .await;
        assert_eq!(
            get_key2.unwrap().as_str().unwrap(),
            "value2",
            "Key2 should have correct value"
        );

        // Test transaction rollback
        println!("Testing transaction rollback");
        let invalid_tx = json!({
            "actions": {
                "users/alice/key3": {
                    "action": "create",
                    "value": "value3"
                },
                "users/alice/key1": { // This should fail due to permissions
                    "action": "create",
                    "value": "value4"
                }
            }
        });

        let result = client.call("sys_config_exec_tx", invalid_tx).await;
        assert!(
            result.is_err(),
            "Transaction should fail due to permissions"
        );

        // Verify that no changes were made
        let get_key3 = client
            .call("sys_config_get", json!({"key": "users/alice/key3"}))
            .await;
        assert!(
            get_key3.unwrap().is_null(),
            "Key3 should not exist after failed transaction"
        );

        // Cleanup
        client
            .call("sys_config_delete", json!({"key": "users/alice/key1"}))
            .await
            .unwrap();
        client
            .call("sys_config_delete", json!({"key": "users/alice/key2"}))
            .await
            .unwrap();
        //client.call("buckyos_api_delete", json!({"key": "users/alice/json_key"})).await.unwrap();

        drop(server);
    }
}
