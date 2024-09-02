
use std::sync::{Arc};
use std::{fs::File};
use log::*;
use name_lib::DeviceConfig;
use simplelog::*;
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use warp::{reply::{Reply, Response}, Filter};
use serde_json::{Value, json};
use name_lib::*;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use ::kRPC::*;


enum LoginType {
    ByPassword,
    BySignature,
    ByJWT,
}

lazy_static ! {
    static ref PRIVATE_KEY:EncodingKey = {
        let private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
"#;
        let private_key = EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
        private_key
    };
}

async fn get_trust_did_document(device_id:&str) -> Result<DeviceConfig> {
    unimplemented!("get_trust_device_did_document")
}

fn generate_session_token(appid:&str,userid:&str,login_nonce:u64) -> RPCSessionToken {
    unimplemented!()
}

fn verify_jws_signature(json_obj:&Value,signature:&str,decode_key:&DecodingKey) -> bool {
    unimplemented!("verify_signature")
}

async fn get_public_key_from_kid(kid:&Option<String>) -> Result<DecodingKey> {
    if kid.is_none() {
        //use default public key : owner's public key
        let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "vZ2kEJdazmmmmxTYIuVPCt0gGgMOnBP6mMrQmqminB0"
            }
        );
        let result_key : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        return Ok(DecodingKey::from_jwk(&result_key).unwrap());
        
    }    
    return Err(RPCErrors::ReasonError("kid not found".to_string()));
}

async fn verify_jwt(jwt:&str) -> Result<Value> {
    let header: jsonwebtoken::Header = jsonwebtoken::decode_header(jwt).map_err(|error| {
        error!("JWT decode header error: {}", error);
        RPCErrors::ReasonError("JWT decode header error".to_string())
    })?;
    let validation = Validation::new(header.alg);
    
    //try get public key from header.kid
    let public_key = get_public_key_from_kid(&header.kid).await?;
    
    //verify jwt
    let decoded_token = jsonwebtoken::decode::<Value>(&jwt, &public_key, &validation).map_err(|error| {
        error!("JWT verify error: {}", error);
        RPCErrors::ReasonError("JWT verify error".to_string())
    })?;

    return Ok(decoded_token.claims);
}

async fn handle_verify_session_token(params:Value) -> Result<Value> {
    let session_token = params.get("session_token")
        .ok_or(RPCErrors::ReasonError("Missing session_token".to_string()))?;
    let session_token = session_token.as_str().ok_or(RPCErrors::ReasonError("Invalid session_token".to_string()))?;
    let first_dot = session_token.find('.');
    if first_dot.is_none() {
        //this is not a jwt token, use token-store to verify
    } else {
        //this is a jwt token, verify it locally
        verify_jwt(session_token).await?;
    }
    
    Ok(Value::Bool(true))
}

async fn handle_login_by_jwt(params:Value,login_nonce:u64) -> Result<RPCSessionToken> {
    let jwt = params.get("jwt")
    .ok_or(RPCErrors::ReasonError("Missing jwt".to_string()))?;
    let jwt = jwt.as_str().ok_or(RPCErrors::ReasonError("Invalid jwt".to_string()))?;

    let jwt_payload = verify_jwt(jwt).await?;

    let userid = jwt_payload.get("userid")
        .ok_or(RPCErrors::ReasonError("Missing userid".to_string()))?;
    let userid = userid.as_str().ok_or(RPCErrors::ReasonError("Invalid userid".to_string()))?;
    let appid = jwt_payload.get("appid")
        .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
    let appid = appid.as_str().ok_or(RPCErrors::ReasonError("Invalid appid".to_string()))?;


    //generate session token
    let session_token = generate_session_token(appid,userid,login_nonce);

    return Ok(session_token);
}

async fn handle_login_by_signature(params:Value,login_nonce:u64) -> Result<RPCSessionToken> {
    let userid = params.get("userid")
    .ok_or(RPCErrors::ReasonError("Missing userid".to_string()))?;
    let userid = userid.as_str().ok_or(RPCErrors::ReasonError("Invalid userid".to_string()))?;
    let appid = params.get("appid")
        .ok_or(RPCErrors::ReasonError("Missing appid".to_string()))?;
    let appid = appid.as_str().ok_or(RPCErrors::ReasonError("Invalid appid".to_string()))?;

    let _from = params.get("from")
    .ok_or(RPCErrors::ReasonError("Missing from".to_string()))?;
    let from = _from.as_str().ok_or(RPCErrors::ReasonError("Invalid from".to_string()))?;
    let _signature = params.get("signature")
        .ok_or(RPCErrors::ReasonError("Missing signature".to_string()))?;
    let signature = _signature.as_str().ok_or(RPCErrors::ReasonError("Invalid signature".to_string()))?;

    //verify signature
    let trust_did_document = get_trust_did_document(from).await?;
    let device_public_key = trust_did_document.get_auth_key().ok_or(RPCErrors::ReasonError("Device public key not found".to_string()))?;

    //TODO:check login_nonce > last_login_nonce

    let mut will_hash = params.clone();
    let will_hash_obj = will_hash.as_object_mut().unwrap();
    will_hash_obj.remove("signature");
    will_hash_obj.remove("type");
    will_hash_obj.insert(String::from("login_nonce"),json!(login_nonce));

    if !verify_jws_signature(&will_hash,signature,&device_public_key) {
        return Err(RPCErrors::ReasonError("Invalid signature".to_string()));
    }

    //generate session token
    let session_token = generate_session_token(appid,userid,login_nonce);

    return Ok(session_token);
}

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
        LoginType::BySignature => {
            let session_token =  handle_login_by_signature(params,login_nonce).await?;
            return Ok(Value::String(session_token.to_string()));
        },
        _ => {
            return Err(RPCErrors::ReasonError("Invalid login type".to_string()));
        }   
    }
}


async fn process_request(method:String,param:Value,trace_id:u64) -> ::kRPC::Result<Value> {
    match method.as_str() {
        "login" => {
            return handle_login(param,trace_id).await;
        },
        "verify_token" => {
            return handle_verify_session_token(param).await;
        },
        // Add more methods here
        _ => Err(RPCErrors::UnknownMethod(String::from(method))),
    }
}


fn init_log_config() {
    let config = ConfigBuilder::new().build();
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
            File::create("verify_hub.log").unwrap(),
        ),
    ])
    .unwrap();
}



async fn service_main() {
    init_log_config();
    info!("Starting verify_hub service...");

    let rpc_route = warp::path("verify_hub")
    .and(warp::post())
    .and(warp::body::json())
    .and_then(|req: RPCRequest| async {
        info!("Received request: {}", serde_json::to_string(&req).unwrap());

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
        
        info!("Response: {}", serde_json::to_string(&rpc_response).unwrap());
        Ok::<_, warp::Rejection>(warp::reply::json(&rpc_response))
    });

    info!("verify_hub service initialized");
    warp::serve(rpc_route).run(([127, 0, 0, 1], 3032)).await;
}

#[tokio::main]
async fn main() {
    service_main().await;
}

mod test {
    use super::*;
    use tokio::time::{sleep,Duration};
    use tokio::task;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test]
    async fn test_login_and_verify() {
        let server = task::spawn(async {
            service_main().await;
        });

        sleep(Duration::from_millis(100)).await;
        let test_owner_public_key = "vZ2kEJdazmmmmxTYIuVPCt0gGgMOnBP6mMrQmqminB0";
        let test_owner_private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIK45kLWIAx3CHmbEmyCST4YB3InSCA4XAV6udqHtRV5P
-----END PRIVATE KEY-----
"#;
        //login test,use trust device JWT
        let private_key = EncodingKey::from_ed_pem(test_owner_private_key_pem.as_bytes()).unwrap();
        let mut client = kRPC::new("http://127.0.0.1:3032/verify_hub",&None);
        let mut header = Header::new(Algorithm::EdDSA);
        //完整的kid表达应该是 $zoneid#kid 这种形式，为了提高性能做了一点简化
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        header.kid = None;
        header.typ = None;
        let login_params = json!({
            "userid": "did:example:1234567890",
            "appid": "system", 
            "exp":(now + 3600) as usize
        });        
        let token = encode(&header, &login_params, &private_key).unwrap();

        let session_token = client.call("login", json!( {"type":"jwt","jwt":token})).await.unwrap();
        print!("session_token:{}",session_token);

        //verify token test,use JWT-session-token

        let verify_result = client.call("verify_token", json!( {"session_token":session_token})).await.unwrap();
        print!("verify result:{}",verify_result);

        drop(server);
    }
}
