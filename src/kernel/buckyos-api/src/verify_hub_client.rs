use ::kRPC::*;
use name_lib::DID;
use package_lib::PackageMeta;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{KernelServiceDoc, SelectorType};

pub const VERIFY_HUB_UNIQUE_ID: &str = "verify-hub";
pub const VERIFY_HUB_SERVICE_NAME: &str = "verify-hub";
pub const VERIFY_HUB_TOKEN_EXPIRE_TIME: u64 = 60*10;//10 minutes
pub const VERIFY_HUB_SERVICE_PORT: u16 = 3210;


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

    pub async fn login_by_jwt(&self, jwt: String, login_params: Option<Value>) -> Result<RPCSessionToken> {
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
        let session_token_str = result.as_str()
            .ok_or(RPCErrors::ParserResponseError("Response is not a string".to_string()))?;
        let session_token = RPCSessionToken::from_string(session_token_str)?;
        Ok(session_token)
    }
}

pub fn generate_verify_hub_service_doc() -> KernelServiceDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    let mut pkg_meta = PackageMeta::new(VERIFY_HUB_UNIQUE_ID, VERSION, "did:bns:buckyos",&owner_did, None);

    let doc = KernelServiceDoc {
        meta: pkg_meta,
        show_name: "Verify Hub".to_string(),
        selector_type: SelectorType::Random,
    };
    return doc;
}

mod tests {

    #[test]
    fn test_generate_verify_hub_service_doc() {
        use super::generate_verify_hub_service_doc;
        let doc = generate_verify_hub_service_doc();
        let pkg_id = doc.meta.get_package_id();
        let pkg_did = pkg_id.to_did();
        println!("pkg_id: {}", pkg_did.to_raw_host_name());
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }
}
