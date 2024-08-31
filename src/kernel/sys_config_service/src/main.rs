mod kv_provider;
//mod etcd_provider;
//mod rocksdb_provider;
mod sled_provider;

use std::sync::{Arc};
use std::{fs::File};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use log::*;
use simplelog::*;
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use warp::{reply::{Reply, Response}, Filter};
use serde_json::{Value, json};
use jsonwebtoken::{DecodingKey};

use ::kRPC::*;
use rbac::*;

use kv_provider::KVStoreProvider;
use sled_provider::SledStore; 

lazy_static!{
    static ref TRUST_KEYS: Arc<Mutex<HashMap<String,DecodingKey> > > = {
        let hashmap : HashMap<String,DecodingKey> = HashMap::new();  

        Arc::new(Mutex::new(hashmap))
    };
}
async fn handle_get(params:Value,session_token:&RPCSessionToken) -> Result<Value> {
    //TODO:ACL control here
    let key = params.get("key");
    if key.is_none() {
        return Err(RPCErrors::ReasonError("Missing key".to_string()));
    }
    
    let key = key.unwrap().as_str().unwrap();

    if session_token.userid.is_none() {
        return Err(RPCErrors::NoPermission("No userid".to_string()));
    }
    let userid = session_token.userid.as_ref().unwrap();
    let full_res_path = format!("kv://{}",key);
    if !enforce(userid, session_token.appid.as_deref(), full_res_path.as_str(), "read").await {
        return Err(RPCErrors::NoPermission("No read permission".to_string()));
    }

    
    let store = SYS_STORE.lock().await;
    let result = store.get(String::from(key)).await.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
    return Ok(Value::String(result));
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
                return Err(RPCErrors::TokenExpired(session_token));
            }
        }
        
        if rpc_session_token.is_self_verify() {
            //generate a non-store quick verify token for next-call
        }
        //do access control here
        
        match method.as_str() {
            "sys_config_get" => {
                return handle_get(param,&rpc_session_token).await;
            },
            "sys_config_set" => {
                return handle_set(param,&rpc_session_token).await;
            },
            // Add more methods here
            _ => Err(RPCErrors::UnknownMethod(String::from(method))),
        }
        
    } else {
        return Err(RPCErrors::NoPermission("No session token".to_string()));
    }

}

lazy_static!{
    static ref SYS_STORE: Arc<Mutex<dyn KVStoreProvider>> = {
        Arc::new(Mutex::new(SledStore::new("system_config").unwrap()))
    };
}

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new().build();

    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        // 同时将日志输出到文件
        WriteLogger::new(
            LevelFilter::Info,
            config,
            File::create("sys_config_service.log").unwrap(),
        ),
    ])
    .unwrap();
}


async fn verify_session_token(token: &mut RPCSessionToken) -> Result<()> {
    if token.is_self_verify() {
        let trust_keys = TRUST_KEYS.lock().await;
        token.do_self_verify(&trust_keys)?;
    }

    Ok(())
}

async fn service_main() {
    init_log_config();
    info!("Starting system config service...");
    // Select the rear end storage, here you can switch different implementation
    let store: Arc<dyn KVStoreProvider> = Arc::new(
        //EtcdStore::new(&["http://127.0.0.1:2379"]).await.unwrap()
        // RocksDBStore::new("./system_config.rsdb").unwrap()
        SledStore::new("sled_db").unwrap()
    );

    let rpc_route = warp::path("system_config")
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
                    trace_id: req.trace_id
                };
            },
            Err(err) => {
                rpc_response = RPCResponse {
                    result: RPCResult::Failed(err.to_string()),
                    seq: req.seq,
                    token: None,
                    trace_id: req.trace_id
                };
            }
        }
        
        info!("<==|Response: {}", serde_json::to_string(&rpc_response).unwrap());
        Ok::<_, warp::Rejection>(warp::reply::json(&rpc_response))
    });

    info!("Starting system config service");
    warp::serve(rpc_route).run(([127, 0, 0, 1], 3030)).await;
}

#[tokio::main]
async fn main() {
    service_main().await;
}

mod test {
    use super::*;
    use jsonwebtoken::EncodingKey;
    use tokio::time::{sleep,Duration};
    use tokio::task;
    
    #[tokio::test]
    async fn test_server_get_set() {
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
            exp: Some(now+5),
            token_type: RPCSessionTokenType::JWT,
            token: None,
        };
        let jwt = token.generate_jwt(Some("{owner}".to_string()),&private_key).unwrap();
    
        sleep(Duration::from_millis(1000)).await;

        let client = kRPC::new("http://127.0.0.1:3030/system_config",&Some(jwt));
        let _ = client.call("sys_config_set", json!( {"key":"users/alice/test_key","value":"test_value"})).await.unwrap();
        let result = client.call("sys_config_get", json!( {"key":"users/alice/test_key"})).await.unwrap();
        assert_eq!(result.as_str().unwrap(), "test_value");
        let result = client.call("sys_config_set", json!( {"key":"users/bob/test_key","value":"test_value"})).await;
        assert!(result.is_err());

        //test timeout
        
        drop(server);

    }
}
