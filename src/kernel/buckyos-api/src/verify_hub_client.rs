use ::kRPC::*;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{AppDoc, AppType, SelectorType};

pub const VERIFY_HUB_UNIQUE_ID: &str = "verify-hub";
pub const VERIFY_HUB_SERVICE_NAME: &str = "verify-hub";
pub const VERIFY_HUB_TOKEN_EXPIRE_TIME: u64 = 60*10;//10 minutes
pub const VERIFY_HUB_SERVICE_PORT: u16 = 3210;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenPair {
    pub session_token: String,
    pub refresh_token: String,
}


#[derive(Serialize, Deserialize)]
struct VerifyHubSettings {
    trust_keys: Vec<String>,
}

pub struct VerifyHubClient {
    krpc_client: kRPC,
}

impl VerifyHubClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self { krpc_client }
    }

    pub async fn login_by_jwt(&self, jwt: String, login_params: Option<Value>) -> Result<TokenPair> {
        self.krpc_client.reset_session_token().await;
        let mut params = json!({
            "type": "jwt",
            "jwt": jwt
        });
        
        if let Some(additional_params) = login_params {
            if let Some(params_obj) = params.as_object_mut() {
                if let Some(additional_obj) = additional_params.as_object() {
                    for (key, value) in additional_obj {
                        params_obj.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        
        let result = self.krpc_client.call("login", params).await?;
        let token_pair: TokenPair = serde_json::from_value(result)
            .map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
        Ok(token_pair)
    }

    pub async fn verify_token(&self, session_token: &str, appid: Option<&str>) -> Result<Value> {
        let mut params = json!({
            "session_token": session_token,
        });
        if let Some(appid) = appid {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("appid".to_string(), Value::String(appid.to_string()));
            }
        }
        self.krpc_client.call("verify_token", params).await
    }

    // Backward-compatible convenience: exchange JWT for a single session token.
    pub async fn login_by_jwt_session_token(
        &self,
        jwt: String,
        login_params: Option<Value>,
    ) -> Result<RPCSessionToken> {
        let token_pair = self.login_by_jwt(jwt, login_params).await?;
        RPCSessionToken::from_string(token_pair.session_token.as_str())
    }
}

pub fn generate_verify_hub_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        VERIFY_HUB_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Verify Hub")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}

// #[async_trait]
// pub trait VerifyHubServer  {
//     async fn handle_login_by_jwt(&self, jwt: String, login_params: Option<Value>) -> Result<RPCSessionToken>;
// }

// pub fn handle_krpc(server: &dyn VerifyHubServer, req: RPCRequest, _ip_from: IpAddr) -> Result<Value> {
//     //根据req.method分发到对应的handler
// }

mod tests {

    #[test]
    fn test_generate_verify_hub_service_doc() {
        use super::generate_verify_hub_service_doc;
        let doc = generate_verify_hub_service_doc();
        let pkg_id = doc.get_package_id();
        let pkg_did = pkg_id.to_did();
        println!("pkg_id: {}", pkg_did.to_raw_host_name());
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }
}
