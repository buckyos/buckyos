use ::kRPC::*;
use serde_json::{Value, json};


pub const VERIFY_HUB_SERVICE_NAME: &str = "verify-hub";
pub const VERIFY_HUB_TOKEN_EXPIRE_TIME: u64 = 60*10;

pub struct VerifyHubClient {
    krpc_client: kRPC,
}

impl VerifyHubClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self { krpc_client }
    }

    pub async fn login_by_jwt(&self, jwt: String, login_params: Option<Value>) -> Result<RPCSessionToken> {
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
        let session_token_str = result.as_str()
            .ok_or(RPCErrors::ParserResponseError("Response is not a string".to_string()))?;
        let session_token = RPCSessionToken::from_string(session_token_str)?;
        Ok(session_token)
    }
}
