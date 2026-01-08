

use name_lib::DID;
use serde_json::json;
use std::collections::HashMap;
use ::kRPC::*;
use serde::{Deserialize, Serialize};

use crate::{AppDoc, AppType, SelectorType};

pub const REPO_SERVICE_UNIQUE_ID: &str = "repo-service";
pub const REPO_SERVICE_SERVICE_NAME: &str = "repo-service";
pub const REPO_SERVICE_SERVICE_PORT: u16 = 4000;


#[derive(Serialize, Deserialize)]
struct RepoServiceSettings {
    remote_source: HashMap<String, String>,
    enable_dev_mode: bool,
}

pub struct RepoClient {
    krpc_client: kRPC,
}

impl RepoClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self { krpc_client }
    }

    pub async fn pub_index(&self) -> Result<()> {
        let params = json!({});
        let _result = self.krpc_client.call("pub_index", params).await?;
        Ok(())
    }

    pub async fn pub_pkg(&self,pkg_meta_jwt_map: HashMap<String,String>) -> Result<()> {
        let params = json!({
            "pkg_list": pkg_meta_jwt_map
        });
        let _result = self.krpc_client.call("pub_pkg", params).await?;
        Ok(())
    }

    // install pkg at current zone
    // pkg_id -> will_install_chunk_id (can be empty)
    pub async fn install_pkg(&self,pkg_list: &HashMap<String,String>,install_task_name: &str) -> Result<()> {
        let params = json!({
            "pkg_list": pkg_list,
            "task_name": install_task_name
        });
        let _result = self.krpc_client.call("install_pkg", params).await?;
        Ok(())
    }

    pub async fn sync_from_remote_source(&self) -> Result<()> {
        let params = json!({});
        let _result = self.krpc_client.call("sync_from_remote_source", params).await?;
        Ok(())
    }
}


pub fn generate_repo_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        REPO_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Repo Service")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}

mod tests {

    #[test]
    fn test_generate_repo_service_doc() {
        use super::generate_repo_service_doc;
        let doc = generate_repo_service_doc();
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }
}