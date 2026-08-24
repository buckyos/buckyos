mod kv_provider;
//mod etcd_provider;
//mod rocksdb_provider;
mod sled_provider;
mod zone_did_resolver;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use jsonwebtoken::{Algorithm, DecodingKey};
use log::*;

use async_trait::async_trait;
use lazy_static::lazy_static;
use serde_json::Value;
use tokio::sync::Mutex;

use ::kRPC::*;
use buckyos_api::{
    build_current_rbac_config, validate_verify_hub_token_claims, SystemServiceId, TokenUse,
    ZoneConfig, SYSTEM_CONFIG_BOOTSTRAP_AUDIENCE,
};
use buckyos_http_server::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use buckyos_kit::*;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use kv_provider::KVStoreProvider;
use name_lib::*;
use rbac::*;
use sled_provider::SledStore;

use crate::zone_did_resolver::ZoneDidResolver;

const SYS_CONFIG_URI_PERFIX: &str = "obj://config/";
const NODE_DAEMON_SERVICE_ID: &str = "node-daemon";

// A brand-new zone needs the boot scheduler to populate System Config before
// Verify Hub can start. The mode closes permanently when bootstrap explicitly
// refreshes trust keys or the first valid Verify Hub session arrives.
static BOOTSTRAP_CONFIG_MODE: AtomicBool = AtomicBool::new(false);

lazy_static! {
    static ref TRUST_KEYS: Arc<Mutex<HashMap<String, DecodingKey>>> = {
        let hashmap: HashMap<String, DecodingKey> = HashMap::new();
        Arc::new(Mutex::new(hashmap))
    };
}

lazy_static! {
    pub(crate) static ref SYS_STORE: Arc<Mutex<dyn KVStoreProvider>> =
        Arc::new(Mutex::new(SledStore::new().unwrap()));
}

const INTERNAL_META_PREFIX: &str = "__meta/";
const RESOLVER_CACHE_KEY_PREFIX: &str = "resolver/cache/";

// resolver/cache/* 是 zone 解析控制面（zone_did_resolver 消费的 override 登记，
// 见 zone_did_resolver.rs 文件头）。写权限由 RBAC allow-list 承担（kernel/system
// 角色与 root/su_admin），这里补需求要求的审计日志；强负状态（revoked/tombstoned/
// missing）会屏蔽该名字的后续解析，用 warn 级别突出。
fn value_carries_strong_negative_status(value: &str) -> bool {
    serde_json::from_str::<Value>(value)
        .ok()
        .and_then(|v| {
            v.get("document_status")
                .and_then(|s| s.as_str())
                .map(|s| matches!(s, "revoked" | "tombstoned" | "missing"))
        })
        .unwrap_or(false)
}

fn audit_resolver_cache_write(
    op: &str,
    real_key_path: &str,
    value: Option<&str>,
    userid: &str,
    appid: &str,
) {
    if !real_key_path.starts_with(RESOLVER_CACHE_KEY_PREFIX) {
        return;
    }
    if value
        .map(value_carries_strong_negative_status)
        .unwrap_or(false)
    {
        warn!(
            "AUDIT resolver-cache {} (strong negative document_status): key={} userid={} appid={}",
            op, real_key_path, userid, appid
        );
    } else {
        info!(
            "AUDIT resolver-cache {}: key={} userid={} appid={}",
            op, real_key_path, userid, appid
        );
    }
}

fn strip_config_key_prefix(key_path: &str) -> &str {
    let mut key = key_path.trim_start_matches(SYS_CONFIG_URI_PERFIX);
    if key.starts_with("/config/") {
        key = &key[8..];
    }
    key.trim_start_matches('/').trim_start_matches('\\')
}

fn is_internal_meta_key(key_path: &str) -> bool {
    strip_config_key_prefix(key_path).starts_with(INTERNAL_META_PREFIX)
}

fn get_full_res_path(key_path: &str) -> Result<(String, String)> {
    let key = strip_config_key_prefix(key_path);
    let normalized_path = normalize_path(key);

    return Ok((
        format!("{}{}", SYS_CONFIG_URI_PERFIX, normalized_path),
        normalized_path,
    ));
}

fn get_sudo_mode(session_token: &RPCSessionToken, userid: &str) -> Option<SudoMode> {
    if session_token.sudo {
        Some(SudoMode::Sudo(RPCSessionToken::get_default_sudo_userid(
            userid,
        )))
    } else {
        None
    }
}

fn require_session_identity(session_token: &RPCSessionToken) -> Result<(&str, String)> {
    let userid = session_token
        .sub
        .as_deref()
        .filter(|userid| !userid.trim().is_empty())
        .ok_or_else(|| RPCErrors::ParseRequestError("Missing userid".to_string()))?;
    match validate_verify_hub_token_claims(session_token, TokenUse::Session) {
        Ok(claims) => Ok((userid, claims.target.authorization_key())),
        Err(session_error) => {
            let device_name = current_bootstrap_device_name()?;
            validate_system_config_bootstrap_assertion(session_token, device_name.as_str())
                .map_err(|_| session_error)?;
            let service_id =
                SystemServiceId::parse(session_token.appid.as_deref().unwrap_or_default())
                    .map_err(RPCErrors::InvalidToken)?;
            Ok((userid, format!("system:{service_id}")))
        }
    }
}

fn current_bootstrap_device_name() -> Result<String> {
    let raw = std::env::var("BUCKYOS_THIS_DEVICE").map_err(|_| {
        RPCErrors::InvalidToken("system config bootstrap device is unavailable".to_string())
    })?;
    let device: DeviceDocument = serde_json::from_str(raw.as_str()).map_err(|error| {
        RPCErrors::InvalidToken(format!(
            "invalid system config bootstrap device document: {error}"
        ))
    })?;
    Ok(device.name)
}

fn validate_system_config_bootstrap_assertion(
    token: &RPCSessionToken,
    expected_device_name: &str,
) -> Result<()> {
    let service_id_is_valid = token
        .appid
        .as_deref()
        .is_some_and(|appid| SystemServiceId::parse(appid).is_ok());
    let valid = token.iss.as_deref() == Some(expected_device_name)
        && token
            .sub
            .as_deref()
            .is_some_and(|subject| subject == "kernel" || subject == expected_device_name)
        && service_id_is_valid
        && token.aud.as_deref() == Some(SYSTEM_CONFIG_BOOTSTRAP_AUDIENCE)
        && token.exp.is_some()
        && token
            .jti
            .as_deref()
            .is_some_and(|jti| !jti.trim().is_empty())
        && !token.sudo
        && token.extra.is_empty();
    if !valid {
        return Err(RPCErrors::InvalidToken(
            "invalid system config bootstrap assertion".to_string(),
        ));
    }
    Ok(())
}

fn request_key(params: &Value) -> Option<&str> {
    params.get("key").and_then(Value::as_str)
}

fn bootstrap_request_allowed_after_boot(
    method: &str,
    params: &Value,
    device_name: &str,
    service_id: &str,
) -> bool {
    let key = request_key(params).map(strip_config_key_prefix);
    let own_device_doc = format!("devices/{device_name}/doc");
    let own_device_info = format!("devices/{device_name}/info");
    match method {
        "sys_config_get" | "sys_config_list" => true,
        "sys_config_set" if service_id == NODE_DAEMON_SERVICE_ID => {
            key.is_some_and(|key| key == own_device_doc || key == own_device_info)
        }
        _ => false,
    }
}

async fn authorize_bootstrap_request(
    method: &str,
    params: &Value,
    token: &RPCSessionToken,
) -> Result<()> {
    let device_name = current_bootstrap_device_name()?;
    validate_system_config_bootstrap_assertion(token, device_name.as_str())?;
    let service_id = token.appid.as_deref().unwrap_or_default();

    if BOOTSTRAP_CONFIG_MODE.load(Ordering::Acquire) && service_id == NODE_DAEMON_SERVICE_ID {
        if matches!(
            method,
            "sys_config_create"
                | "sys_config_get"
                | "sys_config_set"
                | "sys_config_set_by_json_path"
                | "sys_config_exec_tx"
                | "sys_config_delete"
                | "sys_config_append"
                | "sys_config_list"
                | "sys_refresh_trust_keys"
        ) {
            return Ok(());
        }
    } else {
        if bootstrap_request_allowed_after_boot(method, params, device_name.as_str(), service_id) {
            return Ok(());
        }
    }

    Err(RPCErrors::NoPermission(
        "system config bootstrap assertion is not authorized for this request".to_string(),
    ))
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

    if is_internal_meta_key(key) {
        return Err(RPCErrors::ReasonError(
            "internal metadata key is reserved".to_string(),
        ));
    }

    let (userid, appid) = require_session_identity(session_token)?;

    let (full_res_path, real_key_path) = get_full_res_path(key)?;
    info!(
        "GET: full_res_path:{},real_key_path:{:?},appid:{},userid:{}",
        full_res_path, real_key_path, appid, userid
    );
    let is_allowed = enforce(
        userid,
        appid.as_str(),
        full_res_path.as_str(),
        "read",
        get_sudo_mode(session_token, userid),
    )
    .await;
    if !is_allowed {
        warn!("No read permission");
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    };

    let store = SYS_STORE.lock().await;
    let result = store
        .get_with_revision(real_key_path)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    let Some((value, version)) = result else {
        return Ok(Value::Null);
    };
    Ok(serde_json::json!({
        "value": value,
        "version": version,
    }))
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
    if is_internal_meta_key(key) {
        return Err(RPCErrors::ReasonError(
            "internal metadata key is reserved".to_string(),
        ));
    }

    //check access control
    let (userid, appid) = require_session_identity(session_token)?;
    let (full_res_path, real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        appid.as_str(),
        full_res_path.as_str(),
        "write",
        get_sudo_mode(session_token, userid),
    )
    .await
    {
        warn!(
            "set denied: appid={} userid={} key={}",
            appid, userid, full_res_path
        );
        return Err(RPCErrors::NoPermission(format!(
            "No write permission for key: {}",
            &real_key_path
        )));
    }

    //do business logic
    audit_resolver_cache_write(
        "set",
        real_key_path.as_str(),
        Some(new_value),
        userid,
        appid.as_str(),
    );
    let store = SYS_STORE.lock().await;
    info!("Set key:[{}], value_len={}", key, new_value.len());
    store
        .set(real_key_path, String::from(new_value))
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    drop(store);
    if should_reload_security_state(&full_res_path) {
        info!(
            "security config changed via set, reloading trust keys and rbac: {}",
            full_res_path
        );
        handle_refresh_trust_keys().await?;
    }

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
    if is_internal_meta_key(key) {
        return Err(RPCErrors::ReasonError(
            "internal metadata key is reserved".to_string(),
        ));
    }

    //check access control
    let (userid, appid) = require_session_identity(session_token)?;
    let (full_res_path, real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        appid.as_str(),
        full_res_path.as_str(),
        "write",
        get_sudo_mode(session_token, userid),
    )
    .await
    {
        warn!(
            "create denied: appid={} userid={} key={}",
            appid, userid, full_res_path
        );
        return Err(RPCErrors::NoPermission(format!(
            "No write permission for key: {}",
            &real_key_path
        )));
    }

    //do business logic
    audit_resolver_cache_write(
        "create",
        real_key_path.as_str(),
        Some(new_value),
        userid,
        appid.as_str(),
    );
    let store = SYS_STORE.lock().await;
    info!("Create key:[{}], value_len={}", key, new_value.len());
    store
        .create(&real_key_path, new_value)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    drop(store);
    if should_reload_security_state(&full_res_path) {
        info!(
            "security config changed via create, reloading trust keys and rbac: {}",
            full_res_path
        );
        handle_refresh_trust_keys().await?;
    }

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
    if is_internal_meta_key(key) {
        return Err(RPCErrors::ReasonError(
            "internal metadata key is reserved".to_string(),
        ));
    }

    //check access control
    let (userid, appid) = require_session_identity(session_token)?;
    let (full_res_path, real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        appid.as_str(),
        full_res_path.as_str(),
        "write",
        get_sudo_mode(session_token, userid),
    )
    .await
    {
        warn!(
            "delete denied: appid={} userid={} key={}",
            appid, userid, full_res_path
        );
        return Err(RPCErrors::NoPermission(format!(
            "No write permission for key: {}",
            &real_key_path
        )));
    }

    //do business logic
    audit_resolver_cache_write(
        "delete",
        real_key_path.as_str(),
        None,
        userid,
        appid.as_str(),
    );
    let store = SYS_STORE.lock().await;
    info!("Delete key:[{}]", key);
    store
        .delete(&real_key_path)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    drop(store);
    if should_reload_security_state(&full_res_path) {
        info!(
            "security config changed via delete, reloading trust keys and rbac: {}",
            full_res_path
        );
        handle_refresh_trust_keys().await?;
    }

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
    if is_internal_meta_key(key) {
        return Err(RPCErrors::ReasonError(
            "internal metadata key is reserved".to_string(),
        ));
    }

    //check access control
    let (userid, appid) = require_session_identity(session_token)?;
    let (full_res_path, real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        appid.as_str(),
        full_res_path.as_str(),
        "write",
        get_sudo_mode(session_token, userid),
    )
    .await
    {
        warn!(
            "append denied: appid={} userid={} key={}",
            appid, userid, full_res_path
        );
        return Err(RPCErrors::NoPermission(format!(
            "No write permission for key: {}",
            &real_key_path
        )));
    }

    //read and append
    audit_resolver_cache_write(
        "append",
        real_key_path.as_str(),
        Some(append_value),
        userid,
        appid.as_str(),
    );
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
    if is_internal_meta_key(key) {
        return Err(RPCErrors::ReasonError(
            "internal metadata key is reserved".to_string(),
        ));
    }
    let new_value: Value =
        serde_json::from_str(new_value).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    let json_path = params.get("json_path");
    if json_path.is_none() {
        return Err(RPCErrors::ReasonError("Missing json_path".to_string()));
    }
    let json_path = json_path.unwrap();
    let json_path = json_path.as_str().unwrap();

    //check access control
    let (userid, appid) = require_session_identity(session_token)?;
    let (full_res_path, real_key_path) = get_full_res_path(key)?;
    if !enforce(
        userid,
        appid.as_str(),
        full_res_path.as_str(),
        "write",
        get_sudo_mode(session_token, userid),
    )
    .await
    {
        warn!(
            "set_by_json_path denied: appid={} userid={} key={}",
            appid, userid, full_res_path
        );
        return Err(RPCErrors::NoPermission(format!(
            "No write permission for key: {}",
            &real_key_path
        )));
    }

    //do business logic
    audit_resolver_cache_write(
        "set_by_json_path",
        real_key_path.as_str(),
        None,
        userid,
        appid.as_str(),
    );
    let store = SYS_STORE.lock().await;
    store
        .set_by_path(real_key_path, String::from(json_path), &new_value)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    //let result = store.get(String::from(key)).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    drop(store);
    if should_reload_security_state(&full_res_path) {
        info!(
            "security config changed via set_by_json_path, reloading trust keys and rbac: {}",
            full_res_path
        );
        handle_refresh_trust_keys().await?;
    }
    Ok(Value::Null)
}

fn should_reload_security_state(full_res_path: &str) -> bool {
    if let Some(key) = full_res_path.strip_prefix(SYS_CONFIG_URI_PERFIX) {
        return key == "boot/config" || key.starts_with("system/rbac/");
    }
    false
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
    let (userid, appid) = require_session_identity(session_token)?;

    let mut tx_actions = HashMap::new();
    let mut need_reload_security_state = false;

    // Process each action into KVAction
    for (key, action) in actions.as_object().unwrap() {
        if is_internal_meta_key(key) {
            return Err(RPCErrors::ReasonError(format!(
                "internal metadata key is reserved: {}",
                key
            )));
        }
        let (full_res_path, real_key_path) = get_full_res_path(key)?;
        if !enforce(
            userid,
            appid.as_str(),
            full_res_path.as_str(),
            "write",
            get_sudo_mode(session_token, userid),
        )
        .await
        {
            warn!(
                "exec_tx denied: appid={} userid={} key={}",
                appid, userid, full_res_path
            );
            return Err(RPCErrors::NoPermission(format!(
                "No write permission for key: {}",
                &real_key_path
            )));
        }
        if should_reload_security_state(&full_res_path) {
            need_reload_security_state = true;
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
        audit_resolver_cache_write(
            action_type,
            real_key_path.as_str(),
            action.get("value").and_then(|v| v.as_str()),
            userid,
            appid.as_str(),
        );
        tx_actions.insert(real_key_path.clone(), kv_action);
    }
    let real_main_key = parse_tx_main_key(&params)?;

    let store = SYS_STORE.lock().await;
    store
        .exec_tx(tx_actions, real_main_key)
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    drop(store);

    if need_reload_security_state {
        info!("exec_tx touched security config, reloading trust keys and rbac");
        handle_refresh_trust_keys().await?;
    }

    Ok(Value::Null)
}

fn parse_tx_main_key(params: &Value) -> Result<Option<(String, u64)>> {
    let Some(main_key) = params.get("main_key") else {
        return Ok(None);
    };
    let main_key = main_key
        .as_object()
        .ok_or_else(|| RPCErrors::ParseRequestError("main_key must be an object".to_string()))?;
    let key = main_key
        .get("key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| RPCErrors::ParseRequestError("main_key.key is required".to_string()))?;
    let revision = main_key
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            RPCErrors::ParseRequestError("main_key.revision must be a u64".to_string())
        })?;
    let (_, real_key_path) = get_full_res_path(key)?;
    Ok(Some((real_key_path, revision)))
}

async fn handle_list(params: Value, session_token: &RPCSessionToken) -> Result<Value> {
    //check params
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    let key = key.unwrap();
    let key = key.as_str().unwrap();
    if is_internal_meta_key(key) {
        return Err(RPCErrors::ReasonError(
            "internal metadata key is reserved".to_string(),
        ));
    }

    //check access control
    let (userid, appid) = require_session_identity(session_token)?;
    let (full_res_path, real_key_path) = get_full_res_path(key)?;
    info!(
        "full_res_path: {},userid: {},appid: {}",
        full_res_path, userid, appid
    );
    if !enforce(
        userid,
        appid.as_str(),
        full_res_path.as_str(),
        "read",
        get_sudo_mode(session_token, userid),
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
            let zone_document = zone_config.zone_document().map_err(|err| {
                error!("Failed to parse zone document from boot/config: {}", err);
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
            if zone_document.owner.is_valid() {
                let owner_did = zone_document.owner.clone();
                if let Some(owner_key) = zone_document.get_default_key() {
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
        let device_doc: DeviceDocument = serde_json::from_str(&device_doc_str).unwrap();
        //device_doc.iss
        let devcie_key = device_doc.get_default_key();

        if devcie_key.is_some() {
            let devcie_key = devcie_key.unwrap();
            let real_key = DecodingKey::from_jwk(&devcie_key).unwrap();
            TRUST_KEYS
                .lock()
                .await
                .insert(device_doc.name.clone(), real_key.clone());
            info!("Insert device name:[{}] to trust keys", device_doc.name);

            TRUST_KEYS
                .lock()
                .await
                .insert(device_doc.id.to_string(), real_key);
            info!(
                "Insert device did:[{}] to trust keys",
                device_doc.id.to_string()
            );
        }
    } else {
        error!("Missing BUCKYOS_THIS_DEVICE");
    }

    let rbac_policy = store.get("system/rbac/policy".to_string()).await;
    let rbac_policy = rbac_policy.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    let rbac_config = build_current_rbac_config(rbac_policy.as_deref());
    info!(
        "rbac config loaded, model_bytes={}, policy_tail_bytes={}, policy_bytes={}",
        rbac_config.model.len(),
        rbac_config.policy_tail.len(),
        rbac_config.policy.len()
    );
    rbac::create_enforcer(rbac_config.model.as_str(), rbac_config.policy.as_str())
        .await
        .unwrap();

    Ok(Value::Null)
}

async fn dump_configs_for_scheduler(
    _params: Value,
    session_token: &RPCSessionToken,
) -> Result<Value> {
    let (_userid, appid) = require_session_identity(session_token)?;
    if appid != "system:scheduler" && appid != "system:node-daemon" {
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

#[derive(Clone)]
struct SystemConfigServer {}

impl SystemConfigServer {
    fn new() -> Self {
        SystemConfigServer {}
    }

    async fn process_request(
        &self,
        method: String,
        param: Value,
        session_token: Option<String>,
    ) -> ::kRPC::Result<Value> {
        //check session_token
        if session_token.is_some() {
            let session_token = session_token.unwrap();
            let rpc_session_token = verify_trusted_jwt(session_token.as_str()).await?;
            if validate_verify_hub_token_claims(&rpc_session_token, TokenUse::Session).is_ok() {
                BOOTSTRAP_CONFIG_MODE.store(false, Ordering::Release);
            } else {
                authorize_bootstrap_request(method.as_str(), &param, &rpc_session_token).await?;
            }
            //let mut rpc_session_token = RPCSessionToken::from_string(session_token.as_str())?;
            //veruft session token (need access trust did_list)
            //verify_session_token(&mut rpc_session_token).await?;
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
                    let _ = require_session_identity(&rpc_session_token)?;
                    let result = handle_refresh_trust_keys().await;
                    if result.is_ok()
                        && rpc_session_token.aud.as_deref()
                            == Some(SYSTEM_CONFIG_BOOTSTRAP_AUDIENCE)
                    {
                        BOOTSTRAP_CONFIG_MODE.store(false, Ordering::Release);
                    }
                    return result;
                }
                // Add more methods here
                _ => Err(RPCErrors::UnknownMethod(String::from(method))),
            }
        } else {
            return Err(RPCErrors::NoPermission("No session token".to_string()));
        }
    }
}

#[async_trait]
impl RPCHandler for SystemConfigServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let result = self
            .process_request(req.method, req.params, req.token)
            .await;

        match result {
            Ok(value) => Ok(RPCResponse::new(RPCResult::Success(value), req.seq)),
            Err(err) => Ok(RPCResponse::new(
                RPCResult::Failed(err.to_string()),
                req.seq,
            )),
        }
    }
}

#[async_trait]
impl HttpServer for SystemConfigServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        return Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ));
    }

    fn id(&self) -> String {
        "system-config-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

async fn load_device_doc(device_name: &str) -> Result<DeviceDocument> {
    let store = SYS_STORE.lock().await;
    let device_doc = store
        .get(format!("devices/{}/doc", device_name))
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    if device_doc.is_none() {
        return Err(RPCErrors::KeyNotExist(format!(
            "devices/{}/doc",
            device_name
        )));
    }
    let device_doc_str = device_doc.unwrap();
    let device_doc: EncodedDocument = EncodedDocument::from_str(device_doc_str.clone())
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    let device_doc: DeviceDocument = DeviceDocument::decode(&device_doc, None)
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    return Ok(device_doc);
}

fn is_privileged_user_type(user_type: &str) -> bool {
    matches!(
        user_type.trim().to_ascii_lowercase().as_str(),
        "admin" | "root"
    )
}

fn user_has_privileged_role_in_policy(policy: &str, user_name: &str) -> bool {
    policy.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }

        let parts: Vec<&str> = line.split(',').map(|part| part.trim()).collect();
        if parts.len() < 3 || parts[0] != "g" || parts[1] != user_name {
            return false;
        }

        is_privileged_user_type(parts[2])
    })
}

async fn load_privileged_user_doc(user_name: &str) -> Result<OwnerDocument> {
    let store = SYS_STORE.lock().await;
    let mut is_privileged = false;

    let user_settings = store
        .get(format!("users/{}/settings", user_name))
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    if let Some(user_settings) = user_settings.as_ref() {
        let user_settings_json: Value = serde_json::from_str(user_settings).map_err(|err| {
            RPCErrors::ReasonError(format!("parse user settings failed: {}", err))
        })?;
        let user_type = user_settings_json
            .get("type")
            .or_else(|| user_settings_json.get("user_type"))
            .and_then(|value| value.as_str());
        if let Some(user_type) = user_type {
            is_privileged = is_privileged_user_type(user_type);
        }
    }

    if !is_privileged {
        if let Some(policy) = store
            .get("system/rbac/policy".to_string())
            .await
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
        {
            is_privileged = user_has_privileged_role_in_policy(&policy, user_name);
        }
    }

    if !is_privileged {
        return Err(RPCErrors::NoPermission(format!(
            "user {} is not granted admin/root privilege",
            user_name
        )));
    }

    let owner_doc = store
        .get(format!("users/{}/doc", user_name))
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    drop(store);

    let owner_doc =
        owner_doc.ok_or_else(|| RPCErrors::KeyNotExist(format!("users/{}/doc", user_name)))?;
    let encoded_doc = EncodedDocument::from_str(owner_doc)
        .map_err(|err| RPCErrors::ReasonError(format!("parse owner doc failed: {}", err)))?;
    let owner_config = OwnerDocument::decode(&encoded_doc, None)
        .map_err(|err| RPCErrors::ReasonError(format!("decode owner doc failed: {}", err)))?;
    Ok(owner_config)
}

async fn verify_trusted_jwt(jwt: &str) -> Result<RPCSessionToken> {
    // Parse header first to check algorithm and kid
    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        error!("JWT decode header error: {}", error);
        RPCErrors::ReasonError("JWT decode header error".to_string())
    })?;

    if header.alg != Algorithm::EdDSA {
        return Err(RPCErrors::ReasonError(
            "JWT algorithm not allowed".to_string(),
        ));
    }

    // Decode claims without verification so we can pick a key fallback (iss) when kid is missing
    let claims = decode_jwt_claim_without_verify(jwt).map_err(|error| {
        error!("decode_jwt_claim_without_verify error: {}", error);
        RPCErrors::ReasonError("decode_jwt_claim_without_verify error".to_string())
    })?;

    let key_id = claims
        .get("iss")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .or_else(|| header.kid.clone())
        .unwrap_or_else(|| "$default".to_string());

    // Fast path: try current trust keys
    let mut trust_keys = TRUST_KEYS.lock().await;
    let mut public_key = trust_keys.get(&key_id);

    // If missing, attempt to load a privileged user doc first, then a device doc, and cache its public key.
    if public_key.is_none() {
        drop(trust_keys); // release lock before awaiting
        let real_key = match load_privileged_user_doc(&key_id).await {
            Ok(owner_config) => {
                let owner_key = owner_config.get_default_key().ok_or_else(|| {
                    RPCErrors::ReasonError(format!("privileged user {} has no default key", key_id))
                })?;
                info!("loaded privileged user doc for JWT issuer {}", key_id);
                DecodingKey::from_jwk(&owner_key).map_err(|err| {
                    RPCErrors::ReasonError(format!("parse privileged user key failed: {}", err))
                })?
            }
            Err(user_err) => {
                info!(
                    "issuer {} is not a privileged user doc source: {}",
                    key_id, user_err
                );
                let device_doc = load_device_doc(&key_id).await?;
                if device_doc.device_type != "ood" && device_doc.device_type != "node" {
                    return Err(RPCErrors::ReasonError(format!(
                        "device type is not ood or node: {}",
                        key_id
                    )));
                }
                let device_key = device_doc.get_default_key();
                if device_key.is_none() {
                    return Err(RPCErrors::ReasonError(format!(
                        "device {} has no default key",
                        key_id
                    )));
                }
                let device_key = device_key.unwrap();
                info!("loaded device doc for JWT issuer {}", key_id);
                DecodingKey::from_jwk(&device_key).map_err(|err| {
                    RPCErrors::ReasonError(format!("parse device key failed: {}", err))
                })?
            }
        };

        // cache and reuse
        trust_keys = TRUST_KEYS.lock().await;
        trust_keys.insert(key_id.clone(), real_key);
        public_key = trust_keys.get(&key_id);
    }

    let public_key = public_key.ok_or(RPCErrors::KeyNotExist(key_id.clone()))?;

    // Build token from string and verify signature/claims using the selected key
    let mut rpc_token = RPCSessionToken::from_string(jwt)?;
    rpc_token.verify_by_key(public_key)?;

    Ok(rpc_token)
}

// async fn verify_session_token(token: &mut RPCSessionToken) -> Result<()> {
//     if token.is_self_verify() {
//         let mut trust_keys = TRUST_KEYS.lock().await;
//         let kid = token.verify_by_key_map(&trust_keys);

//         if kid.is_err() {
//             let kid_err = kid.err().unwrap();
//             match kid_err {
//                 RPCErrors::KeyNotExist(kid) => {
//                     info!("kid not exist: {},try to load device doc", kid);
//                     //kid is device name, try load device doc
//                     let device_doc = load_device_doc(kid.as_str()).await?;
//                     if device_doc.device_type != "ood" && device_doc.device_type != "node" {
//                         return Err(RPCErrors::ReasonError(format!("device type is not ood or node: {}", kid)));
//                     }
//                     let device_key = device_doc.get_default_key();
//                     if device_key.is_some() {
//                         let device_key = device_key.unwrap();
//                         info!("load device {} doc successfully, insert public key to trust keys", kid);
//                         trust_keys.insert(kid.clone(), DecodingKey::from_jwk(&device_key).unwrap());
//                     }
//                     token.verify_by_key_map(&trust_keys)?;
//                     return Ok(());
//                 },
//                 _ => {
//                     return Err(kid_err);
//                 }
//             }
//         }
//         debug!("verify_session_token: {:?}", token);
//         return Ok(())
//     } else {
//         return Err(RPCErrors::ReasonError("Not a self verify token".to_string()));
//     }

// }

async fn init_by_boot_document() -> Result<()> {
    let r = handle_refresh_trust_keys().await;
    if r.is_err() {
        error!("Failed to refresh trust keys: {}", r.err().unwrap());
    }

    let device_doc_str = std::env::var("BUCKYOS_THIS_DEVICE");
    if device_doc_str.is_ok() {
        let device_doc_str = device_doc_str.unwrap();
        let device_doc: DeviceDocument = serde_json::from_str(&device_doc_str).unwrap();
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
    let boot_config_exists = {
        let store = SYS_STORE.lock().await;
        store
            .get("boot/config".to_string())
            .await
            .ok()
            .flatten()
            .is_some()
    };
    BOOTSTRAP_CONFIG_MODE.store(!boot_config_exists, Ordering::Release);
    info!("system config bootstrap mode: {}", !boot_config_exists);
    init_by_boot_document().await.unwrap();

    let server = SystemConfigServer::new();
    const SYSTEM_CONFIG_SERVICE_MAIN_PORT: u16 = 3200;
    info!(
        "Starting system config service on port {}",
        SYSTEM_CONFIG_SERVICE_MAIN_PORT
    );
    let runner = Runner::new(SYSTEM_CONFIG_SERVICE_MAIN_PORT);
    let _ = runner.add_http_server("/kapi/system_config".to_string(), Arc::new(server));

    let resolver = ZoneDidResolver {};
    let _ = runner.add_http_server("/".to_string(), Arc::new(resolver));
    let _ = runner.run().await;
}

#[tokio::main]
async fn main() {
    service_main().await;
}

#[cfg(test)]
mod test {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use jsonwebtoken::EncodingKey;
    use serde_json::json;
    use tokio::{task, time::sleep};

    use super::*;
    use buckyos_api::{
        bind_token_principal_kind, bind_token_target, AuthTarget, SystemServiceId,
        TokenPrincipalKind, VERIFY_HUB_TOKEN_ISSUER,
    };

    fn test_session_token(sub: Option<&str>, appid: Option<&str>) -> RPCSessionToken {
        let mut token = RPCSessionToken {
            sub: sub.map(|value| value.to_string()),
            appid: appid.map(|value| value.to_string()),
            exp: None,
            token_type: RPCSessionTokenType::Normal,
            token: None,
            iss: Some(VERIFY_HUB_TOKEN_ISSUER.to_string()),
            jti: None,
            aud: None,
            sudo: false,
            extra: HashMap::new(),
        };
        if let Some(appid) = appid.filter(|value| !value.is_empty()) {
            bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
            bind_token_target(
                &mut token,
                &AuthTarget::system(SystemServiceId::parse(appid).unwrap()),
                TokenUse::Session,
            )
            .unwrap();
        }
        token
    }

    #[test]
    fn require_session_identity_rejects_missing_or_blank_fields() {
        let missing_userid = test_session_token(None, Some("test"));
        assert!(matches!(
            require_session_identity(&missing_userid),
            Err(RPCErrors::ParseRequestError(message)) if message == "Missing userid"
        ));

        let blank_userid = test_session_token(Some("  "), Some("test"));
        assert!(matches!(
            require_session_identity(&blank_userid),
            Err(RPCErrors::ParseRequestError(message)) if message == "Missing userid"
        ));

        let missing_appid = test_session_token(Some("alice"), None);
        assert!(require_session_identity(&missing_appid).is_err());

        let blank_appid = test_session_token(Some("alice"), Some(""));
        assert!(require_session_identity(&blank_appid).is_err());

        let valid = test_session_token(Some("alice"), Some("control-panel"));
        assert_eq!(
            require_session_identity(&valid).unwrap(),
            ("alice", "system:control-panel".to_string())
        );
    }

    #[test]
    fn bootstrap_assertion_is_explicit_and_read_only_except_for_own_device() {
        let mut assertion = RPCSessionToken {
            sub: Some("kernel".to_string()),
            appid: Some(NODE_DAEMON_SERVICE_ID.to_string()),
            exp: Some(buckyos_get_unix_timestamp() + 900),
            token_type: RPCSessionTokenType::Normal,
            token: None,
            iss: Some("ood1".to_string()),
            jti: Some("boot-1".to_string()),
            aud: Some(SYSTEM_CONFIG_BOOTSTRAP_AUDIENCE.to_string()),
            sudo: false,
            extra: HashMap::new(),
        };
        assert!(validate_system_config_bootstrap_assertion(&assertion, "ood1").is_ok());

        for params in [
            json!({"key": "boot/config"}),
            json!({"key": "devices"}),
            json!({"key": "devices/ood2/info"}),
        ] {
            assert!(bootstrap_request_allowed_after_boot(
                "sys_config_get",
                &params,
                "ood1",
                NODE_DAEMON_SERVICE_ID,
            ));
        }
        assert!(bootstrap_request_allowed_after_boot(
            "sys_config_set",
            &json!({"key": "devices/ood1/doc"}),
            "ood1",
            NODE_DAEMON_SERVICE_ID,
        ));
        assert!(!bootstrap_request_allowed_after_boot(
            "sys_config_set",
            &json!({"key": "devices/ood2/doc"}),
            "ood1",
            NODE_DAEMON_SERVICE_ID,
        ));
        assert!(bootstrap_request_allowed_after_boot(
            "sys_config_get",
            &json!({"key": "users/alice/settings"}),
            "ood1",
            "verify-hub",
        ));
        assert!(!bootstrap_request_allowed_after_boot(
            "sys_config_exec_tx",
            &json!({"actions": {}}),
            "ood1",
            NODE_DAEMON_SERVICE_ID,
        ));
        assert!(!bootstrap_request_allowed_after_boot(
            "sys_config_set",
            &json!({"key": "devices/ood1/info"}),
            "ood1",
            "verify-hub",
        ));

        assertion.aud = None;
        assert!(validate_system_config_bootstrap_assertion(&assertion, "ood1").is_err());
        assertion.aud = Some(SYSTEM_CONFIG_BOOTSTRAP_AUDIENCE.to_string());
        assertion.sub = Some("ood1".to_string());
        assert!(validate_system_config_bootstrap_assertion(&assertion, "ood1").is_ok());
        assertion.appid = Some("verify-hub".to_string());
        assert!(validate_system_config_bootstrap_assertion(&assertion, "ood1").is_ok());
        assertion.extra.insert(
            "target_kind".to_string(),
            Value::String("system".to_string()),
        );
        assert!(validate_system_config_bootstrap_assertion(&assertion, "ood1").is_err());
        assertion.extra.clear();
        assert!(validate_system_config_bootstrap_assertion(&assertion, "ood2").is_err());
    }

    #[test]
    fn get_full_res_path_builds_config_uri_and_storage_key() {
        assert_eq!(
            get_full_res_path("users/alice/settings").unwrap(),
            (
                "obj://config/users/alice/settings".to_string(),
                "users/alice/settings".to_string()
            )
        );
        assert_eq!(
            get_full_res_path("/config/system/rbac/policy").unwrap(),
            (
                "obj://config/system/rbac/policy".to_string(),
                "system/rbac/policy".to_string()
            )
        );
        assert_eq!(
            get_full_res_path("obj://config/boot/config").unwrap(),
            (
                "obj://config/boot/config".to_string(),
                "boot/config".to_string()
            )
        );
    }

    #[test]
    fn transaction_main_key_preserves_colons_in_key() {
        let params = json!({
            "main_key": {
                "key": "users/devtest/apps/did:bns:demo.root/mutation",
                "revision": 7,
            }
        });
        assert_eq!(
            parse_tx_main_key(&params).unwrap(),
            Some((
                "users/devtest/apps/did:bns:demo.root/mutation".to_string(),
                7,
            ))
        );
        assert!(parse_tx_main_key(&json!({ "main_key": "legacy:7" })).is_err());
    }

    #[test]
    fn security_state_reload_uses_config_uri() {
        assert!(should_reload_security_state("obj://config/boot/config"));
        assert!(should_reload_security_state(
            "obj://config/system/rbac/policy"
        ));
        assert!(!should_reload_security_state("/config/system/rbac/policy"));
        assert!(!should_reload_security_state(
            "obj://config/users/alice/settings"
        ));
    }

    #[test]
    fn strong_negative_resolver_state_detection() {
        assert!(value_carries_strong_negative_status(
            r#"{"document_status":"revoked","updated_by":"admin"}"#
        ));
        assert!(value_carries_strong_negative_status(
            r#"{"document_status":"missing"}"#
        ));
        assert!(!value_carries_strong_negative_status(
            r#"{"document_status":"active"}"#
        ));
        assert!(!value_carries_strong_negative_status("not json"));
        assert!(!value_carries_strong_negative_status(r#"{"other":1}"#));
    }

    #[test]
    fn internal_meta_key_accepts_config_uri() {
        assert!(is_internal_meta_key("obj://config/__meta/revision"));
        assert!(is_internal_meta_key("/config/__meta/revision"));
        assert!(is_internal_meta_key("/__meta/revision"));
        assert!(!is_internal_meta_key("obj://config/users/__meta/revision"));
    }

    #[allow(dead_code)]
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
            sub: Some("alice".to_string()),
            appid: Some("test".to_string()),
            exp: Some(now + 5), //5 seconds
            token_type: RPCSessionTokenType::Normal,
            token: None,
            iss: Some("{owner}".to_string()),
            jti: None,
            aud: None,
            sudo: false,
            extra: HashMap::new(),
        };
        let jwt = token.generate_jwt(None, &private_key).unwrap();

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
        assert_eq!(
            result
                .get("value")
                .and_then(|value| value.as_str())
                .unwrap(),
            "{\"field\":\"new_value\"}"
        );
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

    #[allow(dead_code)]
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
            sub: Some("alice".to_string()),
            appid: Some("test".to_string()),
            aud: None,
            exp: Some(now + 30),
            token_type: RPCSessionTokenType::Normal,
            token: None,
            iss: Some("alice".to_string()),
            jti: None,
            sudo: false,
            extra: HashMap::new(),
        };
        let jwt = token.generate_jwt(None, &private_key).unwrap();

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
            get_key1
                .unwrap()
                .get("value")
                .and_then(|value| value.as_str())
                .unwrap(),
            "value1",
            "Key1 should have correct value"
        );

        let get_key2 = client
            .call("sys_config_get", json!({"key": "users/alice/key2"}))
            .await;
        assert_eq!(
            get_key2
                .unwrap()
                .get("value")
                .and_then(|value| value.as_str())
                .unwrap(),
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
