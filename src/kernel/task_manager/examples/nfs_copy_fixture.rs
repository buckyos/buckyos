#![allow(dead_code)]
#[path = "../src/acl.rs"]
mod acl;
#[path = "../src/dispatcher/mod.rs"]
mod dispatcher;
#[path = "../src/json_schema.rs"]
mod json_schema;
#[path = "../src/server.rs"]
mod server;
#[path = "../src/task_store.rs"]
mod task_store;

use async_trait::async_trait;
use buckyos_api::*;
use buckyos_http_server::Runner;
use jsonwebtoken::{DecodingKey, EncodingKey};
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use serde_json::json;
use std::sync::Arc;

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
struct Verifier;
#[async_trait]
impl server::SessionTokenVerifier for Verifier {
    async fn verify(&self, token: &str) -> kRPC::Result<RPCSessionToken> {
        let mut parsed = RPCSessionToken::from_string(token)?;
        parsed.verify_by_key(&DecodingKey::from_ed_components(PUBLIC).unwrap())?;
        Ok(parsed)
    }
}
fn token(user: &str, service: &str, kind: TokenPrincipalKind) -> String {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 86400),
        iss: Some(VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(user.into()),
        appid: None,
        sudo: false,
        extra: Default::default(),
    };
    bind_token_principal_kind(&mut token, kind);
    bind_token_target(
        &mut token,
        &AuthTarget::system(SystemServiceId::parse(service).unwrap()),
        TokenUse::Session,
    )
    .unwrap();
    token
        .generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap())
        .unwrap()
}
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::path::PathBuf::from(args.get(1).expect("temporary fixture directory required"));
    let port: u16 = args.get(2).map(|p| p.parse().unwrap()).unwrap_or(3381);
    std::fs::create_dir_all(&dir).unwrap();
    let store = task_store::TaskStore::open_partitioned(
        &format!("sqlite://{}?mode=rwc", dir.join("user.db").display()),
        RdbBackend::Sqlite,
        None,
        &format!("sqlite://{}?mode=rwc", dir.join("system.db").display()),
        RdbBackend::Sqlite,
        None,
    )
    .await
    .unwrap();
    let service = server::TaskManagerService::new(
        Arc::new(store),
        KEventClient::new_local(TASK_MANAGER_SERVICE_NAME),
        Arc::new(Verifier),
    );
    service.ensure_builtin_schemas().await.unwrap();
    std::fs::write(dir.join("nfs-copy-test.json"),json!({"task_mgr_url":format!("http://127.0.0.1:{port}/kapi/task-manager"),"service_token":token("copy-test-node",NFS_SERVER_SERVICE_NAME,TokenPrincipalKind::System),"public_key":PUBLIC}).to_string()).unwrap();
    std::fs::write(
        dir.join("user-token"),
        token(
            "copy-test-user",
            CONTROL_PANEL_SERVICE_NAME,
            TokenPrincipalKind::User,
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("other-token"),
        token(
            "copy-other-user",
            CONTROL_PANEL_SERVICE_NAME,
            TokenPrincipalKind::User,
        ),
    )
    .unwrap();
    let runner = Runner::new(port);
    runner
        .add_http_server(
            "/kapi/task-manager".into(),
            Arc::new(server::TaskManagerHttpServer::new(service)),
        )
        .unwrap();
    runner.run().await.unwrap();
}
