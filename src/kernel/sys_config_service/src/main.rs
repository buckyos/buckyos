mod kv_provider;
//mod etcd_provider;
//mod rocksdb_provider;
mod sled_provider;


use std::collections::HashMap;
use std::fs::File;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::DecodingKey;
use log::*;

use simplelog::*;
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use warp::{Filter};
use serde_json::{Value};


use ::kRPC::*;
use rbac::*;
use name_lib::*;
use buckyos_kit::*;

use kv_provider::KVStoreProvider;
use sled_provider::SledStore;

lazy_static!{
    static ref TRUST_KEYS: Arc<Mutex<HashMap<String,DecodingKey> > > = {
        let hashmap : HashMap<String,DecodingKey> = HashMap::new();
        Arc::new(Mutex::new(hashmap))
    };
}

lazy_static!{
    static ref SYS_STORE: Arc<Mutex<dyn KVStoreProvider>> = {
        Arc::new(Mutex::new(SledStore::new().unwrap()))
    };
}

async fn handle_get(params:Value,session_token:&RPCSessionToken) -> Result<Value> {
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


    let full_res_path = format!("kv://{}",key);
    let is_allowed = enforce(userid, None, full_res_path.as_str(), "read").await;
    if !is_allowed {
        warn!("No read permission");
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    };

    let store = SYS_STORE.lock().await;
    let result = store.get(String::from(key)).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    if result.is_none() {
        return Ok(Value::Null);
    } else {
        return Ok(Value::String(result.unwrap()));
    }

}

async fn handle_set(params:Value,session_token:&RPCSessionToken) -> Result<Value> {
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
    let full_res_path = format!("kv://{}",key);
    if !enforce(userid, session_token.appid.as_deref(), full_res_path.as_str(), "write").await {
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    info!("Set key:[{}] to value:[{}]",key,new_value);
    store.set(String::from(key),String::from(new_value)).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    return Ok(Value::Null);
}


async fn handle_create(params:Value,session_token:&RPCSessionToken) -> Result<Value> {
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
    let full_res_path = format!("kv://{}",key);
    if !enforce(userid, session_token.appid.as_deref(), full_res_path.as_str(), "write").await {
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    info!("Create key:[{}] to value:[{}]",key,new_value);
    store.create(key,new_value).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    return Ok(Value::Null);
}

async fn handle_delete(params:Value,session_token:&RPCSessionToken) -> Result<Value> {
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
    let full_res_path = format!("kv://{}",key);
    if !enforce(userid, session_token.appid.as_deref(), full_res_path.as_str(), "write").await {
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    info!("Delete key:[{}]",key);
    store.delete(key).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

    return Ok(Value::Null);
}

async fn handle_append(params:Value,session_token:&RPCSessionToken) -> Result<Value> {
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
    let full_res_path = format!("kv://{}",key);
    if !enforce(userid, session_token.appid.as_deref(), full_res_path.as_str(), "write").await {
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    }

    //read and append
    let store = SYS_STORE.lock().await;
    let result = store.get(String::from(key)).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    if result.is_none() {
        warn!("key:[{}] not exist,cann't append",key);
        return Err(RPCErrors::KeyNotExist);
    } else {
        let old_value = result.unwrap();
        let new_value = format!("{}{}",old_value,append_value);
        store.set(String::from(key),new_value).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        return Ok(Value::Null);
    }
}

async fn handle_set_by_json_path(params:Value,session_token:&RPCSessionToken) -> Result<Value> {
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
    let new_value:Value = serde_json::from_str(new_value).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

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
    let full_res_path = format!("kv://{}",key);
    if !enforce(userid, session_token.appid.as_deref(), full_res_path.as_str(), "write").await {
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    let result = store.get(String::from(key)).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    if result.is_none() {
        warn!("key:[{}] not exist,cann't append",key);
        return Err(RPCErrors::KeyNotExist);
    } else {
        let old_value = result.unwrap();
        let mut old_value:Value = serde_json::from_str(&old_value).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        set_json_by_path(&mut old_value,json_path,Some(&new_value));
        store.set(String::from(key),serde_json::to_string(&old_value).unwrap()).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        return Ok(Value::Null);
    }
}

async fn handle_list(params:Value,session_token:&RPCSessionToken) -> Result<Value> {
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
    let full_res_path = format!("kv://{}", key);
    if !enforce(userid, session_token.appid.as_deref(), full_res_path.as_str(), "read").await {
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    }

    //do business logic
    let store = SYS_STORE.lock().await;
    let result = store.list_direct_children(key.to_string()).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    Ok(Value::Array(result.iter().map(|v| Value::String(v.clone())).collect()))
}

async fn dump_configs_for_scheduler(_params:Value,session_token:&RPCSessionToken) -> Result<Value> {
    let appid = session_token.appid.as_deref().unwrap();
    if appid != "kernel" {
        return Err(RPCErrors::NoPermission("No permission".to_string()));
    }

    let store = SYS_STORE.lock().await;
    let mut config_map = HashMap::new();

    let boot_config = store.list_data("boot/").await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(boot_config);
    let devices_config = store.list_data("devices/").await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(devices_config);
    let users_config = store.list_data("users/").await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(users_config);
    let services_config = store.list_data("services/").await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(services_config);
    let system_config = store.list_data("system/").await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(system_config);
    let node_config = store.list_data("nodes/").await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    config_map.extend(node_config);

    let config_map = serde_json::to_value(&config_map).unwrap();
    return Ok(config_map);
}

async fn process_request(method:String,param:Value,session_token:Option<String>) -> ::kRPC::Result<Value> {
    //check session_token
    if session_token.is_some() {
        let session_token = session_token.unwrap();
        let mut rpc_session_token = RPCSessionToken::from_string(session_token.as_str())?;
        //veruft session token (need access trust did_list)
        verify_session_token(&mut rpc_session_token).await?;
        if rpc_session_token.exp.is_some() {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            if now > rpc_session_token.exp.unwrap()  {
                warn!("session token expired: {}",session_token);
                return Err(RPCErrors::TokenExpired(session_token));
            }
            info!("session token is valid: {}",session_token);
        }
        info!("ready to handle request : {}",method.as_str());
        match method.as_str() {
            "sys_config_create"=>{
                return handle_create(param,&rpc_session_token).await;
            },
            "sys_config_get" => {
                return handle_get(param,&rpc_session_token).await;
            },
            "sys_config_set" => {
                return handle_set(param,&rpc_session_token).await;
            },
            "sys_config_set_by_json_path" => {
                return handle_set_by_json_path(param,&rpc_session_token).await;
            },
            "sys_config_delete" => {
                return handle_delete(param,&rpc_session_token).await;
            },
            "sys_config_append" => {
                return handle_append(param,&rpc_session_token).await;
            },
            "sys_config_list" => {
                return handle_list(param,&rpc_session_token).await;
            },
            "dump_configs_for_scheduler" => {
                return dump_configs_for_scheduler(param,&rpc_session_token).await;
            },
            // Add more methods here
            _ => Err(RPCErrors::UnknownMethod(String::from(method))),
        }

    } else {
        return Err(RPCErrors::NoPermission("No session token".to_string()));
    }

}



fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new()
        .set_time_format_custom(format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"))
        .build();

    let log_path = get_buckyos_root_dir().join("logs").join("sys_config_service.log");
    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            config,
            File::create(log_path).unwrap(),
        ),
    ])
    .unwrap();
}


async fn verify_session_token(token: &mut RPCSessionToken) -> Result<()> {
    if token.is_self_verify() {
        let trust_keys = TRUST_KEYS.lock().await;
        token.verify_by_key_map(&trust_keys)?;
    }
    info!("verify_session_token: {:?}",token);
    Ok(())
}

async fn init_by_boot_config()->Result<()> {

    let store = SYS_STORE.lock().await;
    let rbac_model = store.get("system/rbac/model".to_string()).await;
    let rbac_policy = store.get("system/rbac/policy".to_string()).await;
    let mut set_rbac = false;
    if rbac_model.is_ok() && rbac_policy.is_ok() {
        let rbac_model = rbac_model.unwrap();
        let rbac_policy = rbac_policy.unwrap();
        if rbac_model.is_some() && rbac_policy.is_some() {
            info!("model config: {}",rbac_model.clone().unwrap());
            info!("policy config: {}",rbac_policy.clone().unwrap());
            rbac::create_enforcer(Some(rbac_model.unwrap().trim()),
             Some(rbac_policy.unwrap().trim())).await.unwrap();
            set_rbac = true;
            info!("load rbac model and policy from kv store successfully!");
        }
    }

    if !set_rbac {
        rbac::create_enforcer(None,None).await.unwrap();
        info!("load rbac model and policy defaut setting successfully!");
    }

    //let zone_config_str = std::env::var("BUCKY_ZONE_CONFIG");
    //if zone_config_str.is_ok() {
    //    let zone_config:ZoneConfig = serde_json::from_str(&zone_config_str.unwrap()).unwrap();
    //}

    let device_doc_str = std::env::var("BUCKY_THIS_DEVICE");
    if device_doc_str.is_ok() {
        let device_doc_str = device_doc_str.unwrap();
        let device_doc:DeviceConfig = serde_json::from_str(&device_doc_str).unwrap();
        let device_key_str = serde_json::to_string(&device_doc.auth_key).unwrap();
        let devcie_key = device_doc.get_auth_key();
        if devcie_key.is_some() {
            let devcie_key = devcie_key.unwrap();
            TRUST_KEYS.lock().await.insert(device_doc.name.clone(),devcie_key.clone());
            info!("Insert device name:[{}] - key:[{}] to trust keys",device_doc.name,device_key_str);
            TRUST_KEYS.lock().await.insert(device_doc.did.clone(),devcie_key);
            info!("Insert device did:[{}] - key:[{}] to trust keys",device_doc.did,device_key_str);
        }
    } else {
        error!("Missing BUCKY_THIS_DEVICE");
    }
    let zone_owner_str = std::env::var("BUCKY_ZONE_OWNER");
    if zone_owner_str.is_ok() {
        let zone_owner_key_str  = zone_owner_str.unwrap();
        let zone_owner_key : jsonwebtoken::jwk::Jwk = serde_json::from_str(&zone_owner_key_str).unwrap();
        let zone_owner_key = DecodingKey::from_jwk(&zone_owner_key).unwrap();
        TRUST_KEYS.lock().await.insert("{owner}".to_string(),zone_owner_key.clone());
        info!("Insert zone owner key:[{}] to trust keys",zone_owner_key_str);
        //TRUST_KEYS.lock().await.insert("{owner}".to_string(),zone_owner_key);
    } else {
        error!("Missing BUCKY_ZONE_OWNER");
    }


    Ok(())
}

async fn service_main() {
    init_log_config();
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
        info!("|==>Received request: {}", serde_json::to_string(&req).unwrap());

        let process_result =  process_request(req.method,req.params,req.token).await;

        let rpc_response : RPCResponse;
        match process_result {
            Ok(result) => {
                rpc_response = RPCResponse {
                    result: RPCResult::Success(result),
                    seq: req.seq,
                    token: None,
                    trace_id: req.trace_id.clone()
                };
                info!("<==|Response: OK {} {}", req.seq,req.trace_id.as_deref().unwrap_or(""));
            },
            Err(err) => {
                rpc_response = RPCResponse {
                    result: RPCResult::Failed(err.to_string()),
                    seq: req.seq,
                    token: None,
                    trace_id: req.trace_id
                };
                info!("<==|Response: {}", serde_json::to_string(&rpc_response).unwrap());
            }
        }

        Ok::<_, warp::Rejection>(warp::reply::json(&rpc_response))
    });

    info!("Starting system config service");
    warp::serve(cors_response.or(rpc_route)).run(([0, 0, 0, 0], 3200)).await;
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
    #[tokio::test]
    async fn test_server_interface() {
        {
            let jwk = json!(
                {
                    "kty": "OKP",
                    "crv": "Ed25519",
                    "x": "vZ2kEJdazmmmmxTYIuVPCt0gGgMOnBP6mMrQmqminB0"
                }
            );
            let result_key : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
            let mut hashmap = TRUST_KEYS.lock().await;

            hashmap.insert("{owner}".to_string(), DecodingKey::from_jwk(&result_key).unwrap());
        }

        let server = task::spawn(async {
            service_main().await;
        });

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let test_owner_private_key_pem = r#"
        -----BEGIN PRIVATE KEY-----
        MC4CAQAwBQYDK2VwBCIEIK45kLWIAx3CHmbEmyCST4YB3InSCA4XAV6udqHtRV5P
        -----END PRIVATE KEY-----
        "#;

        let private_key = EncodingKey::from_ed_pem(test_owner_private_key_pem.as_bytes()).unwrap();
        let token = RPCSessionToken{
            userid: Some("alice".to_string()),
            appid: None,
            exp: Some(now+5),//5 seconds
            token_type: RPCSessionTokenType::JWT,
            token: None,
            iss:None,
            nonce:None,
        };
        let jwt = token.generate_jwt(Some("{owner}".to_string()),&private_key).unwrap();

        sleep(Duration::from_millis(1000)).await;

        let client = kRPC::new("http://127.0.0.1:3200/kapi/system_config",Some(jwt));
        //test create
        println!("test create");
        client.call("sys_config_create", json!( {"key":"users/alice/test_key","value":"test_value_create"})).await.unwrap();
        //test set
        println!("test set");
        let _ = client.call("sys_config_set", json!( {"key":"users/alice/test_key","value":"test_value"})).await.unwrap();
        //test get
        println!("test get");
        let result = client.call("sys_config_get", json!( {"key":"boot/config"})).await.unwrap();
        assert_eq!(result.as_str().unwrap(), "test_value");
        //test no permission set
        println!("test no permission set");
        let result = client.call("sys_config_set", json!( {"key":"users/bob/test_key","value":"test_value"})).await;
        assert!(result.is_err());
        //test already exist create
        println!("test already exist create");
        let result = client.call("sys_config_create", json!( {"key":"users/alice/test_key","value":"test_value_create"})).await;
        assert!(result.is_err());
        //test delete
        println!("test delete");
        client.call("sys_config_delete", json!( {"key":"users/alice/test_key"})).await.unwrap();
        //test delete not exist
        println!("test delete not exist");
        let result = client.call("sys_config_delete", json!( {"key":"users/alice/test_key"})).await;
        assert!(result.is_err());

        //test token expired
        sleep(Duration::from_millis(8000)).await;
        println!("test token expired");
        let result = client.call("sys_config_set", json!( {"key":"users/alice/test_key","value":"test_value"})).await;
        assert!(result.is_err());

        drop(server);
    }
}


