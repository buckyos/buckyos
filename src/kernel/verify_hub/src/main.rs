
use std::env;
use std::sync::{Arc};
use std::collections::HashMap;
use log::*;
use tokio::sync::{Mutex, RwLock};
use lazy_static::lazy_static;
use warp::{Filter};
use serde_json::{Value};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, engine::general_purpose::STANDARD,Engine as _};
use sha2::{Sha256, Digest};

use jsonwebtoken::{Validation, EncodingKey, DecodingKey};
use name_lib::*;
use buckyos_kit::*; 
use sys_config::*;
use ::kRPC::*;

type Result<T> = std::result::Result<T, RPCErrors>;
enum LoginType {
    ByPassword,
    BySignature,
    ByJWT,
}

#[derive(Clone,Debug,PartialEq)]
struct VerifyServiceConfig {
    zone_config : ZoneConfig,
    token_from_device:String,
}

lazy_static ! {
    static ref PRIVATE_KEY:Arc<RwLock<EncodingKey>> = {
        let private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;
        let private_key = EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
        Arc::new(RwLock::new(private_key))
    };

    static ref TOKEN_CACHE:Arc<Mutex<HashMap<String,(u64,RPCSessionToken)>>> = {
        Arc::new(Mutex::new(HashMap::new()))
    };

    static ref TRUSTKEY_CACHE:Arc<Mutex<HashMap<String,DecodingKey>>> = {
        Arc::new(Mutex::new(HashMap::new()))
    };

    static ref VERIFY_SERVICE_CONFIG:Arc<Mutex<Option<VerifyServiceConfig>>> = {
        Arc::new(Mutex::new(None))
    };
}

async fn generate_session_token(appid:&str,userid:&str,login_nonce:u64,duration:u64) -> RPCSessionToken {
    let now = buckyos_get_unix_timestamp();
    let exp = now + duration;
    let login_nonce = login_nonce;
    
    let mut session_token = RPCSessionToken {
        token_type : RPCSessionTokenType::JWT,
        nonce:Some(login_nonce),
        appid: Some(appid.to_string()),
        userid: Some(userid.to_string()),
        token: None,
        iss: Some("{verify_hub}".to_string()),
        exp: Some(exp),
    };
    
    {
        let private_key = PRIVATE_KEY.read().await;
        session_token.token = Some(session_token.generate_jwt(Some("{verify_hub}".to_string()), &private_key).unwrap());
    }
    
    return session_token;
}

async fn load_token_from_cache(key:&str) -> Option<(u64,RPCSessionToken)> {
    let cache = TOKEN_CACHE.lock().await;
    let token = cache.get(key);
    if token.is_none() {
        return None;
    } else {
        let (create_nonce,token) = token.unwrap();
        return Some((create_nonce.clone(),token.clone()));
    }
}
async fn cache_token(key:&str,create_nonce:u64,token:RPCSessionToken) {
    TOKEN_CACHE.lock().await.insert(key.to_string(),(create_nonce,token));
}


async fn load_trustkey_from_cache(kid:&str) -> Option<DecodingKey> {
    let cache = TRUSTKEY_CACHE.lock().await;
    let decoding_key = cache.get(kid);
    if decoding_key.is_none() {
        return None;
    }
    return Some(decoding_key.unwrap().clone());
}

async fn cache_trustkey(kid:&str,key:DecodingKey) {
    TRUSTKEY_CACHE.lock().await.insert(kid.to_string(),key);
}

async fn get_trust_public_key_from_kid(kid:&Option<String>) -> Result<DecodingKey> {
    //turst keys include : zone's owner, admin users, server device
    //kid : {owner}
    //kid : #device_id

    if kid.is_none() {
        //return verify_hub's public key
        return load_trustkey_from_cache("{verify_hub}").await.ok_or(RPCErrors::ReasonError("Verify hub public key not found".to_string()));
    }

    let kid = kid.as_ref().unwrap();
    let cached_key = load_trustkey_from_cache(kid).await;
    if cached_key.is_some() {
        return Ok(cached_key.unwrap());
    }

    //not found in trustkey_cache, try load from system config service
    let result_key : DecodingKey;
    if kid == "{owner}" {
        //load zone config from system config service
        result_key = VERIFY_SERVICE_CONFIG.lock().await.as_ref().unwrap()
                        .zone_config.get_auth_key().ok_or(RPCErrors::ReasonError("Owner public key not found".to_string()))?;
        info!("load owner public key from zone config");
    } else {
        //load device config from system config service(not from name-lib)
        let _zone_config = VERIFY_SERVICE_CONFIG.lock().await.as_ref().unwrap().zone_config.clone();
        let token_from_device = VERIFY_SERVICE_CONFIG.lock().await.as_ref().unwrap().token_from_device.clone();
        let system_config_client = SystemConfigClient::new(None,Some(token_from_device.as_str()));
        let device_doc_path = format!("{}/doc",sys_config_get_device_path(kid));
        let get_result = system_config_client.get(device_doc_path.as_str()).await;
        if get_result.is_err() {
            return Err(RPCErrors::ReasonError("Trust key  not found".to_string()));
        }
        let (device_config,_version) = get_result.unwrap();
        let device_config:DeviceConfig= serde_json::from_str(&device_config).map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        result_key = device_config.get_auth_key().ok_or(RPCErrors::ReasonError("Device public key not found".to_string()))?;
    }

    //kid is device_id,try load device config from system config service
    cache_trustkey(&kid,result_key.clone()).await;

    return Ok(result_key);
}

//return (kid,payload)
async fn verify_jwt(jwt:&str) -> Result<(Option<String>,Value)> {
    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        error!("JWT decode header error: {}", error);
        RPCErrors::ReasonError("JWT decode header error".to_string())
    })?;
    let validation = Validation::new(header.alg);
    
    //try get public key from header.kid
    let public_key = get_trust_public_key_from_kid(&header.kid).await?;
    
    //verify jwt
    let decoded_token = jsonwebtoken::decode::<Value>(&jwt, &public_key, &validation).map_err(|error| {
        error!("JWT verify error: {}", error);
        RPCErrors::ReasonError("JWT verify error".to_string())
    })?;

    return Ok((header.kid,decoded_token.claims));
}

async fn handle_verify_session_token(params:Value) -> Result<Value> {
    let session_token = params.get("session_token")
        .ok_or(RPCErrors::ReasonError("Missing session_token".to_string()))?;
    let session_token = session_token.as_str().ok_or(RPCErrors::ReasonError("Invalid session_token".to_string()))?;
    let first_dot = session_token.find('.');
    if first_dot.is_none() {
        //this is not a jwt token, use token-store to verify
        return Err(RPCErrors::InvalidToken("not a jwt token".to_string()));
    } else {
        //this is a jwt token, verify it locally
        let (_iss,json_body) = verify_jwt(session_token).await?;
        let rpc_session_token : RPCSessionToken = serde_json::from_value(json_body.clone()).map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let now = buckyos_get_unix_timestamp();
        if rpc_session_token.exp.is_none() {
            return Err(RPCErrors::ReasonError("Token expired".to_string()));
        }
        let exp = rpc_session_token.exp.unwrap();
        if now > exp {
            return Err(RPCErrors::ReasonError("Token expired".to_string()));
        }
        Ok(json_body)
    }
}

async fn handle_login_by_jwt(params:Value,_login_nonce:u64) -> Result<RPCSessionToken> {
    let jwt = params.get("jwt")
    .ok_or(RPCErrors::ReasonError("Missing jwt".to_string()))?;
    let jwt = jwt.as_str().ok_or(RPCErrors::ReasonError("Invalid jwt".to_string()))?;


    let (iss,jwt_payload) = verify_jwt(jwt).await?;

    let userid = jwt_payload.get("userid")
        .ok_or(RPCErrors::ReasonError("Missing userid".to_string()))?;
    let userid = userid.as_str().ok_or(RPCErrors::ReasonError("Invalid userid".to_string()))?;
    let appid = jwt_payload.get("appid")
        .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
    let appid = appid.as_str().ok_or(RPCErrors::ReasonError("Invalid appid".to_string()))?;

    let exp = jwt_payload.get("exp")
        .ok_or(RPCErrors::ReasonError("Missing exp".to_string()))?;
    let exp = exp.as_u64().ok_or(RPCErrors::ReasonError("Invalid exp".to_string()))?;
    let login_nonce = jwt_payload.get("nonce");
    let mut create_nonce:u64 = 0;
    if login_nonce.is_some() {
        create_nonce = login_nonce.unwrap().as_u64().ok_or(RPCErrors::ReasonError("Invalid login_nonce".to_string()))?;
    }
    if buckyos_get_unix_timestamp() > exp {
        return Err(RPCErrors::ReasonError("Token expired".to_string()));
    }
    let iss = iss.ok_or(RPCErrors::ReasonError("Invalid iss".to_string()))?;
    //load last login token from cache;
    let key = format!("{}_{}_{}",userid,appid,iss);
    let cache_result = load_token_from_cache(key.as_str()).await;
    if cache_result.is_some() {
        
        let (last_nonce,old_token) = cache_result.unwrap();
        info!("old_token:{:?},{},req.token.nonce:{}",old_token,last_nonce,create_nonce);
        if last_nonce > create_nonce {
            return Err(RPCErrors::ReasonError("Invalid login_nonce".to_string()));
        }
        
        if last_nonce == create_nonce {
            return Ok(old_token);
        }
    }

    let session_token = generate_session_token(appid,userid,create_nonce,3600*24*30).await;
    //store session token to cache
    cache_token(key.as_str(),create_nonce,session_token.clone()).await;

    return Ok(session_token);
}

async fn handle_login_by_password(params:Value,login_nonce:u64) -> Result<RPCSessionToken> {
    let password = params.get("password")
        .ok_or(RPCErrors::ParseRequestError("Missing password".to_string()))?;
    let password = password.as_str().ok_or(RPCErrors::ReasonError("Invalid password".to_string()))?;
    let username = params.get("username")
        .ok_or(RPCErrors::ParseRequestError("Missing username".to_string()))?;
    let username = username.as_str().ok_or(RPCErrors::ReasonError("Invalid username".to_string()))?;
    let appid = params.get("appid")
        .ok_or(RPCErrors::ParseRequestError("Missing appid".to_string()))?;
    let appid = appid.as_str().ok_or(RPCErrors::ReasonError("Invalid appid".to_string()))?;
    let nonce = params.get("nonce")
        .ok_or(RPCErrors::ParseRequestError("Missing nonce".to_string()))?;
    let nonce = nonce.as_u64().ok_or(RPCErrors::ReasonError("Invalid nonce".to_string()))?;
    let source_url = params.get("source_url")
        .ok_or(RPCErrors::ParseRequestError("Missing source_url".to_string()))?;
    let source_url = source_url.as_str().ok_or(RPCErrors::ParseRequestError("Invalid source_url".to_string()))?;
    
    let now = buckyos_get_unix_timestamp()*1000;
    let abs_diff = now.abs_diff(nonce);
    info!("login nonce and now abs_diff:{}",abs_diff);
    if now.abs_diff(nonce) > 3600*1000*8 {
        warn!("login nonce is too old,abs_diff:{},this is a possible ATTACK?",abs_diff);
        return Err(RPCErrors::ParseRequestError("Invalid nonce".to_string()));
    }

    //TODO:verify appid && source_url
    
    //read account info from system config service
    let user_info_path = format!("users/{}/info",username);
    let token_from_device = VERIFY_SERVICE_CONFIG.lock().await.as_ref().unwrap().token_from_device.clone();
    let system_config_client = SystemConfigClient::new(None,Some(token_from_device.as_str()));
    let user_info_result = system_config_client.get(user_info_path.as_str()).await;
    if user_info_result.is_err() {
        warn!("handle_login_by_password:user info not found {}",user_info_path);
        return Err(RPCErrors::UserNotFound(username.to_string()));
    }
    let (user_info,_version) = user_info_result.unwrap();
    let user_info:serde_json::Value = serde_json::from_str(&user_info)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let store_password = user_info.get("password")
        .ok_or(RPCErrors::ReasonError("password not set,cann't login by password".to_string()))?;
    let store_password = store_password.as_str()
        .ok_or(RPCErrors::ReasonError("Invalid password".to_string()))?;
    
    //encode password with nonce and check it is right
    let password_hash_input = STANDARD.decode(password)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    
    let salt = format!("{}{}",store_password,nonce);
    let hash = Sha256::digest(salt).to_vec();
    if hash != password_hash_input {
        return Err(RPCErrors::InvalidPassword);
    }

    //generate session token
    info!("login success, generate session token for user:{}",username);
    let session_token = generate_session_token(appid,username,nonce,3600*24*7).await;
    return Ok(session_token);
    
}

// async fn handle_login_by_signature(params:Value,login_nonce:u64) -> Result<RPCSessionToken> {
//     let userid = params.get("userid")
//     .ok_or(RPCErrors::ReasonError("Missing userid".to_string()))?;
//     let userid = userid.as_str().ok_or(RPCErrors::ReasonError("Invalid userid".to_string()))?;
//     let appid = params.get("appid")
//         .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
//     let appid = appid.as_str().ok_or(RPCErrors::ReasonError("Invalid appid".to_string()))?;

//     let _from = params.get("from")
//     .ok_or(RPCErrors::ReasonError("Missing from".to_string()))?;
//     let from = _from.as_str().ok_or(RPCErrors::ReasonError("Invalid from".to_string()))?;
//     let _signature = params.get("signature")
//         .ok_or(RPCErrors::ReasonError("Missing signature".to_string()))?;
//     let signature = _signature.as_str().ok_or(RPCErrors::ReasonError("Invalid signature".to_string()))?;

//     //verify signature
//     let trust_did_document = get_trust_did_document(from).await?;
//     let device_public_key = trust_did_document.get_auth_key().ok_or(RPCErrors::ReasonError("Device public key not found".to_string()))?;

//     //TODO:check login_nonce > last_login_nonce

//     let mut will_hash = params.clone();
//     let will_hash_obj = will_hash.as_object_mut().unwrap();
//     will_hash_obj.remove("signature");
//     will_hash_obj.remove("type");
//     will_hash_obj.insert(String::from("login_nonce"),json!(login_nonce));

//     if !verify_jws_signature(&will_hash,signature,&device_public_key) {
//         return Err(RPCErrors::ReasonError("Invalid signature".to_string()));
//     }

//     //generate session token
//     let session_token = generate_session_token(appid,userid,login_nonce);

//     return Ok(session_token);
// }

async fn handle_login(params:Value,login_nonce:u64) -> Result<Value> {
    //default logint type is JWT
    let mut real_login_type = LoginType::ByJWT;
    let login_type = params.get("type");

    if login_type.is_some() {
        let login_type = login_type.unwrap().as_str().ok_or(RPCErrors::ReasonError("Invalid login type".to_string()))?;
        match login_type {
            "password" => {
               real_login_type = LoginType::ByPassword;
            },
            "jwt" => {
                real_login_type = LoginType::ByJWT;
            },
            "signature" => {
                real_login_type = LoginType::BySignature;
            },
            _ => {
                return Err(RPCErrors::ReasonError("Invalid login type".to_string()));
            }
        }
    }

    match real_login_type {
        LoginType::ByJWT => {
            let session_token = handle_login_by_jwt(params,login_nonce).await?;
            return Ok(Value::String(session_token.to_string()));
        },
        LoginType::ByPassword => {
            let session_token = handle_login_by_password(params,login_nonce).await?;
            return Ok(Value::String(session_token.to_string()));
        }
        // LoginType::BySignature => {
        //     let session_token =  handle_login_by_signature(params,login_nonce).await?;
        //     return Ok(Value::String(session_token.to_string()));
        // },
        _ => {
            return Err(RPCErrors::ReasonError("Invalid login type".to_string()));
        }   
    }
}


async fn process_request(method:String,param:Value,req_seq:u64) -> ::kRPC::Result<Value> {
    match method.as_str() {
        "login" => {
            return handle_login(param,req_seq).await;
        },
        "verify_token" => {
            return handle_verify_session_token(param).await;
        },
        // Add more methods here
        _ => Err(RPCErrors::UnknownMethod(String::from(method))),
    }
}

async fn init_service_config() -> Result<()> {
    //load zone config form env
    let zone_config_str = env::var("BUCKY_ZONE_CONFIG").map_err(|error| RPCErrors::ReasonError(error.to_string()));
    if zone_config_str.is_err() {
        warn!("BUCKY_ZONE_CONFIG not set,use default zone config for test!");
        return Ok(());
    }
    let zone_config_str = zone_config_str.unwrap();
    let session_token = env::var("VERIFY_HUB_SESSION_TOKEN").map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    info!("zone_config_str:{}",zone_config_str);
    
    let new_service_config = VerifyServiceConfig {
        zone_config: serde_json::from_str(zone_config_str.as_str()).map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
        token_from_device: session_token.clone(),
    };

    {
        let mut service_config = VERIFY_SERVICE_CONFIG.lock().await;
        if service_config.is_some() {
            return Ok(());
        }
        service_config.replace(new_service_config);
    }
    
    info!("start load config from system config service.");
    let system_config_client = SystemConfigClient::new(None,Some(session_token.as_str()));
    let private_key_str = system_config_client.get("system/verify_hub/key").await;
    if private_key_str.is_ok() {
        let (private_key,_) = private_key_str.unwrap();
        let private_key = EncodingKey::from_ed_pem(private_key.as_bytes());
        if private_key.is_ok() {
            let private_key = private_key.unwrap();
            let mut verify_hub_private_key = PRIVATE_KEY.write().await;
            *verify_hub_private_key = private_key;
        } else {
            warn!("verify_hub private key format error,use default private key for test!");
        }
    } else {
        warn!("verify_hub private key cann't load from system config service,use default private key for test!");
    }

    info!("verify_hub init success!");
    Ok(())
}


async fn service_main() -> i32 {
    init_logging("verify_hub");
    info!("Starting verify_hub service...");
    //init service config from system config service and env
    let _ = init_service_config().await.map_err(
        |error| {
            error!("init service config failed:{}",error);
            return -1;
        }
    );
    //load cache from service_cache@dfs:// and service_local_cache@fs://

    let rpc_route = warp::path!("kapi" / "verify_hub")
    .and(warp::post())
    .and(warp::body::json())
    .and_then(|req: RPCRequest| async {
        info!("|==>Received request: {}", serde_json::to_string(&req).unwrap());

        let process_result =  process_request(req.method,req.params,req.seq).await;
        
        let rpc_response : RPCResponse;
        match process_result {
            Ok(result) => {
                rpc_response = RPCResponse {
                    result: RPCResult::Success(result),
                    seq:req.seq,
                    token:None,
                    trace_id:req.trace_id,
                };
            },
            Err(err) => {
                rpc_response = RPCResponse {
                    result: RPCResult::Failed(err.to_string()),
                    seq:req.seq,
                    token:None,
                    trace_id:req.trace_id,
                };
            }
        }
        
        info!("<==|Response: {}", serde_json::to_string(&rpc_response).unwrap());
        Ok::<_, warp::Rejection>(warp::reply::json(&rpc_response))
    });

    info!("verify_hub service initialized");
    warp::serve(rpc_route).run(([127, 0, 0, 1], 3300)).await;
    return 0;
}

#[tokio::main]
async fn main() {
    service_main().await;
}

#[cfg(test)]
mod test {
    use super::*;
    use jsonwebtoken::{encode, Algorithm, Header};
    use serde_json::json;
    use tokio::time::{sleep};
    use tokio::task;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn test_login_and_verify() {
        let zone_config = ZoneConfig::get_test_config();
        env::set_var("ZONE_CONFIG", serde_json::to_string(&zone_config).unwrap());
        env::set_var("SESSION_TOKEN", "abcdefg");//for test only
        

        let server = task::spawn(async {
            service_main().await;
        });

        sleep(Duration::from_millis(100)).await;
        let test_jwk = json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc",
        });
        let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(test_jwk).unwrap();
        let test_pk = DecodingKey::from_jwk(&public_key_jwk).unwrap();
       
        cache_trustkey("{verify_hub}",test_pk.clone()).await;
        cache_trustkey("{owner}",test_pk).await;
        let test_owner_private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;
        //login test,use trust device JWT
        let private_key = EncodingKey::from_ed_pem(test_owner_private_key_pem.as_bytes()).unwrap();
        let mut client = kRPC::new("http://127.0.0.1:3300/kapi/verify_hub",None);
        let mut header = Header::new(Algorithm::EdDSA);
        //完整的kid表达应该是 $zoneid#kid 这种形式，为了提高性能做了一点简化
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        header.kid = Some("{owner}".to_string());
        header.typ = None;
        let login_params = json!({
            "userid": "did:example:1234567890",
            "appid": "system", 
            "exp":(now + 3600) as usize
        });        
        let token = encode(&header, &login_params, &private_key).unwrap();
        
        let test_login_token = RPCSessionToken {
            token_type : RPCSessionTokenType::JWT,
            nonce:Some(buckyos_get_unix_timestamp()*1_000_000),
            appid: Some("kernel".to_string()),
            userid: Some("alice".to_string()),
            token: None,
            iss: Some("{owner}".to_string()),
            exp: Some(now + 3600),
        };

        let test_jwt = test_login_token.generate_jwt(Some("{owner}".to_string()), &private_key).unwrap();

        let session_token = client.call("login", json!( {"type":"jwt","jwt":test_jwt})).await.unwrap();
        print!("session_token:{}",session_token);

        //verify token test,use JWT-session-token

        let verify_result = client.call("verify_token", json!( {"session_token":session_token})).await.unwrap();
        print!("verify result:{}",verify_result);

        //test expired token
        let test_login_token = RPCSessionToken {
            token_type : RPCSessionTokenType::JWT,
            nonce:Some((buckyos_get_unix_timestamp()-10000)*1_000_000),
            appid: Some("kernel".to_string()),
            userid: Some("alice".to_string()),
            token: None,
            iss: Some("{owner}".to_string()),
            exp: Some(now + 3600),
        };

        let test_jwt = test_login_token.generate_jwt(Some("{owner}".to_string()), &private_key).unwrap();

        let session_token = client.call("login", json!( {"type":"jwt","jwt":test_jwt})).await;
        assert!(session_token.is_err());

        drop(server);
    }
}
