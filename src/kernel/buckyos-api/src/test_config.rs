use jsonwebtoken::{DecodingKey, EncodingKey};
use name_lib::{
    DID, DIDDocumentTrait, DeviceConfig, DeviceInfo, DeviceMiniConfig, EncodedDocument, NodeIdentityConfig, OODDescriptionString, OwnerConfig, ZoneBootConfig, ZoneConfig, get_x_from_jwk
};
use package_lib::PackageId;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::{SigningKey, VerifyingKey};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use cyfs_sn::SnDB;

use crate::{REPO_SERVICE_UNIQUE_ID, SCHEDULER_SERVICE_UNIQUE_ID, SMB_SERVICE_UNIQUE_ID, VERIFY_HUB_UNIQUE_ID};

// ============================================================================
// 常量定义
// ============================================================================

const BASE_TIME: u64 = 1743478939; // 2025-04-01
const DEFAULT_EXP_YEARS: u64 = 10;
const ADMIN_PASSWORD_HASH: &str = "o8XyToejrbCYou84h/VkF4Tht0BeQQbuX3XKG+8+GQ4="; // bucky2025

// ============================================================================
// 密钥数据管理
// ============================================================================

/// 测试密钥对集合
pub(crate) struct TestKeyPair {
    private_key_pem: &'static str,
    public_key_x: String,
}

struct TestKeys;

impl TestKeys {
    fn verify_key_pair(key_pair: &TestKeyPair) -> Result<(), String> {
        let signing_key = SigningKey::from_pkcs8_pem(key_pair.private_key_pem)
            .expect("Failed to parse private key PEM");
        
        let verifying_key: VerifyingKey = signing_key.verifying_key();

        let public_key_bytes = verifying_key.as_bytes();
        let public_key_x_from_private = URL_SAFE_NO_PAD.encode(public_key_bytes);
        
        if public_key_x_from_private != key_pair.public_key_x {
            return Err(format!("Public key extracted from private key does not match public_key_x. Expected: {}, Got: {}", key_pair.public_key_x, public_key_x_from_private));
        }
        //println!("✓ Key pair verification passed for public_key_x: {}", key_pair.public_key_x);
        Ok(())
    }

    #[allow(dead_code)]
    fn verify_all_key_pairs() -> Result<(), String> {
        let key_ids = vec![
            "devtest",
            "devtest.ood1",
            "devtest.node1",
            "sn",
            "sn_server",
            "bob",
            "bob.ood1",
            "alice",
            "alice.ood1",
        ];
        for key_id in key_ids {
            TestKeys::get_key_pair_by_id(key_id)?;
        }
        Ok(())
    }

    fn get_key_pair_by_id(id: &str) -> Result<TestKeyPair, String> {
        let key_pair = match id {
            "devtest" => TestKeys::devtest_owner(),
            "devtest.ood1" => TestKeys::devtest_ood1(),
            "devtest.node1" => TestKeys::devtest_node1(),
            "sn" => TestKeys::sn_owner(),
            "sn_server" => TestKeys::sn_device(),
            "bob" => TestKeys::bob_owner(),
            "bob.ood1" => TestKeys::bob_ood1(),
            "alice" => TestKeys::alice_owner(),
            "alice.ood1" => TestKeys::alice_ood1(),
            _ => return Err(format!("unknown key pair id: {}", id)),
        };
        TestKeys::verify_key_pair(&key_pair)?;
        Ok(key_pair)
    }

    fn devtest_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#,
            public_key_x: "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8".to_string(),
        }
    }

    fn devtest_ood1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----"#,
            public_key_x: "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc".to_string(),
        }
    }

    fn devtest_node1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEICwMZt1W7P/9v3Iw/rS2RdziVkF7L+o5mIt/WL6ef/0w
-----END PRIVATE KEY-----"#,
            public_key_x: "Bb325f2ed0XSxrPS5sKQaX7ylY9Jh9rfevXiidKA1zc".to_string(),
        }
    }

    fn bob_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEILQLoUZt2okCht0UVhsf4UlGAV9h3BoliwZQN5zBO1G+
-----END PRIVATE KEY-----"#,
            public_key_x: "y-kuJcQ0doFpdNXf4HI8E814lK8MB3-t4XjDRcR_QCU".to_string(),
        }
    }

    fn bob_ood1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIADmO0+u/gcmStDsHZOZCM5gxNYlQmP6jpMo279TQE75
-----END PRIVATE KEY-----"#,
            public_key_x: "iSMKakFEGzGAxLTlaB5TkqZ6d4wurObr-BpaQleoE2M".to_string(),
        }

    }

    fn sn_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIH3hgzhuE0wuR+OEz0Bx6I+YrJDtS0OIajH1rNkEfxnl
-----END PRIVATE KEY-----"#,
            public_key_x: "qJdNEtscIYwTo-I0K7iPEt_UZdBDRd4r16jdBfNR0tM".to_string(),
        }
    }

    fn sn_device() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIBvnIIa1Tx45SjRu9kBZuMgusP5q762SvojXZ4scFxVD
-----END PRIVATE KEY-----"#,
            public_key_x: "FPvY3WXPxuWPYFuwOY0Qbh0O7-hhKr6ta1jTcX9ORPI".to_string(),
        }
    }

    fn alice_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIKH6oJdebg+xxICY7Z1vm84qMkSzm6Wk0ic88DGR90aq
-----END PRIVATE KEY-----"#,
            public_key_x: "uh7RD37tflN65CrcJSUQ3vGnyU4vmC7_M8IkEEOHnds".to_string(),
        }
    }

    fn alice_ood1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIGhyUJ3/YgIrLZxSGG7o1bgiWcyETZKjTBoGagNdpxVy
-----END PRIVATE KEY-----"#,
            public_key_x: "E1oQDYqzyX4ysrNgTJ5DAVaMgA3By8XpBa0e6r2gBqQ".to_string(),
        }
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 写入文件并打印日志
fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
    println!("# Write file: {}", path.to_string_lossy());
}

/// 写入 JSON 文件
fn write_json<T: Serialize>(path: &Path, data: &T) {
    let content = serde_json::to_string_pretty(data).unwrap();
    write_file(path, &content);
}

/// 从 PEM 字符串创建 EncodingKey
fn get_encoding_key(pem: &str) -> EncodingKey {
    EncodingKey::from_ed_pem(pem.as_bytes()).unwrap()
}

/// 从 Value 创建 JWK
fn get_jwk(x: &str) -> jsonwebtoken::jwk::Jwk {
    serde_json::from_value(json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": x
    })).unwrap()
}

pub fn gen_kernel_service_docs() -> HashMap<DID, EncodedDocument> {
    let mut docs = HashMap::new();
    let verify_hub_doc = crate::generate_verify_hub_service_doc();
    let verify_hub_json = serde_json::to_string(&verify_hub_doc).unwrap();
    let verify_hub_did = PackageId::unique_name_to_did(VERIFY_HUB_UNIQUE_ID);

    let scheduler_doc = crate::generate_scheduler_service_doc();
    let scheduler_json = serde_json::to_string(&scheduler_doc).unwrap();
    let scheduler_did = PackageId::unique_name_to_did(SCHEDULER_SERVICE_UNIQUE_ID);

    let repo_doc = crate::generate_repo_service_doc();
    let repo_did = PackageId::unique_name_to_did(REPO_SERVICE_UNIQUE_ID);
    let repo_json = serde_json::to_string(&repo_doc).unwrap();

    let smb_doc = crate::generate_smb_service_doc();
    let smb_json = serde_json::to_string(&smb_doc).unwrap();
    let smb_did = PackageId::unique_name_to_did(SMB_SERVICE_UNIQUE_ID);
    docs.insert(verify_hub_did, EncodedDocument::from_str(verify_hub_json).unwrap());
    docs.insert(scheduler_did, EncodedDocument::from_str(scheduler_json).unwrap());
    docs.insert(repo_did, EncodedDocument::from_str(repo_json).unwrap());
    docs.insert(smb_did, EncodedDocument::from_str(smb_json).unwrap());
    docs
}

// ============================================================================
// 配置构建器
// ============================================================================

/// 开发环境配置构建器
pub struct DevEnvBuilder {
    root_dir: PathBuf,
    now: u64,
    exp: u64,
}

impl DevEnvBuilder {
    /// 创建新的构建器
    #[allow(dead_code)]
    pub fn new(root_dir_name: &str) -> Self {
        let root_dir = std::env::temp_dir().join(root_dir_name);
        std::fs::create_dir_all(&root_dir).unwrap();
        println!("# Dev configs root: {:?}", root_dir);

        Self {
            root_dir,
            now: BASE_TIME,
            exp: BASE_TIME + 3600 * 24 * 365 * DEFAULT_EXP_YEARS,
        }
    }

    /// 从指定路径创建构建器
    pub fn from_path(root_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&root_dir).unwrap();
        println!("# Dev configs root: {:?}", root_dir);

        Self {
            root_dir,
            now: BASE_TIME,
            exp: BASE_TIME + 3600 * 24 * 365 * DEFAULT_EXP_YEARS,
        }
    }

    /// 获取根目录
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// 获取当前时间戳
    #[allow(dead_code)]
    pub fn now(&self) -> u64 {
        self.now
    }

    /// 获取过期时间戳
    pub fn exp(&self) -> u64 {
        self.exp
    }

    /// 创建用户作用域
    fn user_scope<'a>(
        &'a self,
        username: &'a str,
        zone_did: DID,
        key_pair: &'a TestKeyPair,
    ) -> UserEnvScope<'a> {
        UserEnvScope {
            builder: self,
            username,
            did: DID::new("bns", username),
            zone_did,
            key_pair,
            user_dir: self.root_dir.clone(),
        }
    }
}

/// 用户环境作用域
pub struct UserEnvScope<'a> {
    builder: &'a DevEnvBuilder,
    username: &'a str,
    did: DID,//owner did?
    key_pair: &'a TestKeyPair,
    user_dir: PathBuf,
    zone_did: DID,
}

impl<'a> UserEnvScope<'a> {
    /// 创建 Owner 配置
    pub fn create_owner_config(&self) {
        write_file(&self.user_dir.join("user_private_key.pem"), self.key_pair.private_key_pem);

        let owner_jwk = get_jwk(&self.key_pair.public_key_x);
        let owner_config = OwnerConfig::new(
            self.did.clone(),
            self.username.to_string(),
            self.username.to_string(),
            owner_jwk,
        );

        write_json(&self.user_dir.join("user_config.json"), &owner_config);
        println!("Created owner config for {}", self.username);
    }

    /// 创建 Zone 配置 
    /// return zone_boot_config_jwt, TXT Records
    pub fn create_zone_boot_config_jwt(&self, sn_host: Option<String>, ood:OODDescriptionString) -> (String,Vec<String>) {
        let device_full_id = format!("{}.{}", self.username, ood.name.as_str());
        let device_key_pair = TestKeys::get_key_pair_by_id(&device_full_id).unwrap();

        let mut zone_boot = ZoneBootConfig {
            id: None,
            oods: vec![ood.clone()],
            sn: sn_host,
            exp: self.builder.exp,
            devices:HashMap::new(),
            owner: None,
            owner_key: None,
            gateway_devs: vec![],
            extra_info:HashMap::new(),
        };
        let zone_host_name = self.zone_did.to_raw_host_name();
        write_json(
            &self.user_dir.join(format!("{}.zone.json", zone_host_name)),
            &zone_boot,
        );
        zone_boot.id = Some(self.zone_did.clone());
        let mut zone_config = ZoneConfig::new(self.zone_did.clone(), self.did.clone(), get_jwk(&self.key_pair.public_key_x));
        zone_config.init_by_boot_config(&zone_boot);
        write_json(
            &self.user_dir.join("zone_config.json"),
            &zone_config,
        );

        let owner_key = get_encoding_key(self.key_pair.private_key_pem);
        let jwt = zone_boot.encode(Some(&owner_key)).unwrap();

        println!("=> {} TXT Record: DID={};", zone_host_name, jwt.to_string());
        println!("=> {} TXT Record: PKX=0:{};", zone_host_name, get_x_from_jwk(&get_jwk(&self.key_pair.public_key_x)).unwrap());
        //ood1 mini config jwt
        let mini_config = DeviceMiniConfig {
            name: ood.name.clone(),
            x: device_key_pair.public_key_x.clone(),
            rtcp_port: None,
            exp: self.builder.exp,
            extra_info: HashMap::new(),
        };   
        let mini_jwt = mini_config.to_jwt(&owner_key).unwrap();
        println!("=> {} TXT Record: DEV={};", zone_host_name, mini_jwt.to_string());

        let txt_records = vec![
            format!("DID={};", jwt.to_string()),
            format!("PKX=0:{};", get_x_from_jwk(&get_jwk(&self.key_pair.public_key_x)).unwrap()),
            format!("DEV={};", mini_jwt.to_string()),
        ];


        (jwt.to_string(), txt_records)
    }

    /// 创建节点配置
    pub fn create_node_config(
        &self,
        device_name: &str,
        net_id: Option<String>,
    ) -> String {
        let full_device_name = format!("{}.{}", self.username, device_name);
        let device_key_pair = TestKeys::get_key_pair_by_id(&full_device_name).unwrap();
        let node_dir = self.user_dir.join(device_name);
        // 1. 写入设备私钥
        write_file(&node_dir.join("node_private_key.pem"), device_key_pair.private_key_pem);

        // 2. 创建设备配置和 JWT
        let device_jwk = get_jwk(&device_key_pair.public_key_x);
        let mut device_config = DeviceConfig::new_by_jwk(device_name, device_jwk.clone());
        device_config.support_container = true;
        device_config.net_id = net_id;
        device_config.iss = self.did.to_string();

        println!(
            "{} device config: {}",
            device_name,
            serde_json::to_string_pretty(&device_config).unwrap()
        );

        let owner_key = get_encoding_key(self.key_pair.private_key_pem);
        let device_jwt = device_config.encode(Some(&owner_key)).unwrap();
        println!("{} device jwt: {}", device_name, device_jwt.to_string());

        // 3. 创建节点身份配置
        let identity_config = NodeIdentityConfig {
            zone_did: self.zone_did.clone(),
            owner_public_key: get_jwk(&self.key_pair.public_key_x),
            owner_did: self.did.clone(),
            device_doc_jwt: device_jwt.to_string(),
            zone_iat: self.builder.now as u32,
        };
        write_json(&node_dir.join("node_identity.json"), &identity_config);

        // 4. 创建启动配置（仅对 OOD 节点）
        if device_name.starts_with("ood") {
            let start_config = json!({
                "admin_password_hash": ADMIN_PASSWORD_HASH,
                "device_private_key": device_key_pair.private_key_pem,
                "device_public_key": device_jwk,
                "ood_jwt": device_jwt.to_string(),
                "friend_passcode": "sdfsdfsdf",
                "gateway_type": "PortForward",
                "guest_access": true,
                "private_key": self.key_pair.private_key_pem,
                "public_key": get_jwk(&self.key_pair.public_key_x),
                "user_name": self.username,
                "zone_name": self.zone_did.to_host_name(),
                "BUCKYOS_ROOT": "/opt/buckyos"
            });
            write_json(&node_dir.join("start_config.json"), &start_config);
        }

        // 返回设备自签名 JWT（用于验证）
        let device_key = get_encoding_key(device_key_pair.private_key_pem);
        device_config.encode(Some(&device_key)).unwrap().to_string()
    }
}

// ============================================================================
// SN 配置生成
// ============================================================================

/// 创建 SN 服务器配置
pub async fn create_sn_config(builder: &DevEnvBuilder) {
    let sn_dir = builder.root_dir().join("sn_server");
    let owner_keys = TestKeys::get_key_pair_by_id("sn").unwrap();
    let device_keys = TestKeys::get_key_pair_by_id("sn_server").unwrap();

    // 写入设备密钥
    write_file(&sn_dir.join("sn_server_private_key.pem"), device_keys.private_key_pem);

    // 创建 ZoneBootConfig
    let zone_boot = ZoneBootConfig {
        id: None,
        oods: vec!["ood1".parse().unwrap()],
        sn: None,
        exp: builder.exp(),
        owner: None,
        owner_key: None,
        gateway_devs: vec![],
        extra_info: HashMap::new(),
        devices: HashMap::new(),
    };

    let owner_key = get_encoding_key(owner_keys.private_key_pem);
    let _zone_boot_jwt = zone_boot.encode(Some(&owner_key)).unwrap();
    let _x_str = owner_keys.public_key_x;

    // let sn_host = "buckyos.io";
    // let sn_ip = "192.168.1.188";

    // let config = json!({
    //     "device_name": "web3_gateway",
    //     "device_key_path": "/opt/web3_bridge/device_key.pem",
    //     "inner_services": {
    //         "main_sn": {
    //             "type": "cyfs-sn",
    //             "host": format!("web3.{}", sn_host),
    //             "aliases": vec![format!("sn.{}", sn_host)],
    //             "ip": sn_ip,
    //             "zone_config_jwt": zone_boot_jwt.to_string(),
    //             "zone_config_pkx": x_str
    //         },
    //         "zone_provider": {
    //             "type": "zone-provider"
    //         }
    //     },
    //     "servers": {
    //         "main_http_server": {
    //             "type": "cyfs-warp",
    //             "bind": "0.0.0.0",
    //             "http_port": 80,
    //             "tls_port": 443,
    //             "default_tls_host": format!("*.{}", sn_host),
    //             "hosts": {
    //                 format!("web3.{}", sn_host): {
    //                     "tls": { "disable_tls": true, "enable_acme": false },
    //                     "enable_cors": true,
    //                     "routes": { "/kapi/sn": { "inner_service": "main_sn" } }
    //                 },
    //                 format!("*.web3.{}", sn_host): {
    //                     "tls": { "disable_tls": true },
    //                     "routes": { "/": { "tunnel_selector": "main_sn" } }
    //                 },
    //                 "*": {
    //                     "routes": {
    //                         "/": { "tunnel_selector": "main_sn" },
    //                         "/resolve": { "inner_service": "zone_provider" }
    //                     }
    //                 }
    //             }
    //         },
    //         "main_dns_server": {
    //             "type": "cyfs-dns",
    //             "bind": "0.0.0.0",
    //             "port": 53,
    //             "this_name": format!("sn.{}", sn_host),
    //             "resolver_chain": [
    //                 { "type": "SN", "server_id": "main_sn" },
    //                 { "type": "dns", "cache": true }
    //             ],
    //             "fallback": ["8.8.8.8", "6.6.6.6"]
    //         }
    //     },
    //     "dispatcher": {
    //         "udp://0.0.0.0:53": { "type": "server", "id": "main_dns_server" },
    //         "tcp://0.0.0.0:80": { "type": "server", "id": "main_http_server" },
    //         "tcp://0.0.0.0:443": { "type": "server", "id": "main_http_server" }
    //     }
    // });

    // write_json(&sn_dir.join("web3_gateway.json"), &config);
}

/// Register device to SN database
/// 
/// Device registration types supported:
/// - NAT behind OOD + subdomain
/// - NAT behind OOD + custom domain
/// - NAT behind OOD with only port 2980 mapping + subdomain
/// - NAT behind OOD with only port 2980 mapping + custom domain
/// - NAT behind OOD with full port mapping + subdomain
/// - NAT behind OOD with full port mapping + custom domain
/// - WAN OOD + subdomain
/// - WAN OOD + custom domain
/// 
/// Not supported:
/// - WAN OOD with fixed IP, using own domain, using own NS server (no SN needed)
pub async fn register_device_to_sn(
    builder: &DevEnvBuilder,
    username: &str,
    device_name: &str,
    sn_db_path: &str,
) -> Result<(), String> {
    // Find device config directory
    // Try to find device config in builder root: {username}/{device_name}/node_identity.json
    let device_dir = builder.root_dir().join(username).join(device_name);
    let node_identity_path = device_dir.join("node_identity.json");
    
    if !node_identity_path.exists() {
        return Err(format!(
            "Device config not found: {}",
            node_identity_path.display()
        ));
    }

    // Read node_identity.json to get device DID
    let node_identity: NodeIdentityConfig = serde_json::from_str(
        &std::fs::read_to_string(&node_identity_path)
            .map_err(|e| format!("Failed to read node_identity.json: {}", e))?
    )
    .map_err(|e| format!("Failed to parse node_identity.json: {}", e))?;

    // Extract device DID from device_doc_jwt
    let device_doc_jwt = node_identity.device_doc_jwt.clone();
    let encoded_doc = EncodedDocument::from_str(device_doc_jwt)
        .map_err(|e| format!("Failed to create EncodedDocument: {}", e))?;
    let device_doc = DeviceConfig::decode(
        &encoded_doc,
        Some(&DecodingKey::from_jwk(&node_identity.owner_public_key)
            .map_err(|e| format!("Failed to decode owner public key: {}", e))?)
    )
    .map_err(|e| format!("Failed to decode device_doc_jwt: {}", e))?;

    // Get device DID from device config id field
    let device_did = device_doc.id.clone();

    // Create DeviceInfo and fill system info
    let mut device_info = DeviceInfo::new(device_name, device_did.clone());
    device_info.auto_fill_by_system_info().await
        .map_err(|e| format!("Failed to fill device system info: {}", e))?;

    // Get device IP from DeviceInfo (ip is Option<IpAddr>)
    let device_ip = device_info.ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    // Serialize device info to JSON
    let device_info_json = serde_json::to_string_pretty(&device_info)
        .map_err(|e| format!("Failed to serialize device info: {}", e))?;

    // Open SN database
    let db = SnDB::new_by_path(sn_db_path)
        .map_err(|e| format!("Failed to open SN database: {}", e))?;

    // Initialize database if needed
    db.initialize_database()
        .map_err(|e| format!("Failed to initialize SN database: {}", e))?;

    // Register device
    db.register_device(
        username,
        device_name,
        &device_did.to_string(),
        &device_ip,
        &device_info_json,
    )
    .map_err(|e| format!("Failed to register device: {}", e))?;

    println!(
        "Successfully registered device {}.{} (DID: {:?}) to SN database at {}",
        username, device_name, device_did, sn_db_path
    );

    Ok(())
}

// ============================================================================
// 命令行接口函数
// ============================================================================

/// 创建用户环境配置（命令行接口）
pub async fn cmd_create_user_env(
    username: &str,
    hostname: &str,
    ood_name: &str,
    sn_base_host: &str,
    output_dir: Option<&str>,
) -> Result<(), String> {
    let root_dir = if let Some(dir) = output_dir {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| format!("获取当前目录失败: {}", e))?
    };

    let builder = DevEnvBuilder::from_path(root_dir);
    
    // 获取或创建用户密钥对
    let key_pair = TestKeys::get_key_pair_by_id(username)
        .or_else(|_| {
            // 如果找不到预定义的密钥，尝试使用 username.owner 格式
            TestKeys::get_key_pair_by_id(&format!("{}.owner", username))
        })
        .or_else(|_| {
            // 如果还是找不到，使用 devtest 作为默认（仅用于测试）
            println!("警告: 未找到用户 {} 的密钥对，使用 devtest 密钥对", username);
            TestKeys::get_key_pair_by_id("devtest")
        })?;

    // 从 hostname 创建 zone DID（使用 "web" scheme）
    let mut zone_did = DID::new("web", hostname);
    let mut sn_host = None;
    if sn_base_host.contains(".") {
        let web3_bns = format!("web3.{}", sn_base_host);
        zone_did = DID::from_host_name_by_bridge(hostname,"bns",&web3_bns).unwrap();
        sn_host = Some(format!("sn.{}", sn_base_host));
    }

    let scope = builder.user_scope(username, zone_did.clone(), &key_pair);
    
    // 创建 owner 配置
    scope.create_owner_config();
    
    // 创建 zone_boot_config（目前仅生成一个简单的 OOD 描述，SN 为空）
    let ood: OODDescriptionString = ood_name.to_string().parse().unwrap();
    let (_zone_boot_jwt, _txt_records) = scope.create_zone_boot_config_jwt(sn_host, ood);

    println!("成功创建用户环境配置: {}", username);
    println!("Zone hostname: {}", hostname);
    println!("Zone netid: {}", ood_name);
    Ok(())
}

/// 创建节点配置（命令行接口）
///
/// env_dir 应为已通过 cmd_create_user_env 生成的用户环境目录。
pub async fn cmd_create_node_configs(
    device_name: &str,
    env_dir: &Path,
    output_dir: Option<&str>,
    net_id: Option<&str>,
) -> Result<(), String> {
    let _ = output_dir;
    // 使用已有用户环境目录
    let root_dir = env_dir.to_path_buf();
    let builder = DevEnvBuilder::from_path(root_dir.clone());

    // 解析 username
    let user_config_path = root_dir.join("user_config.json");
    let user_config_content = fs::read_to_string(&user_config_path)
        .map_err(|e| format!("读取 user_config.json 失败: {}", e))?;
    let user_config: Value = serde_json::from_str(&user_config_content)
        .map_err(|e| format!("解析 user_config.json 失败: {}", e))?;
    let username = user_config
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("user_config.json 缺少 name 字段")?;

    // 解析 zone 配置
    let zone_config_file = root_dir.join("zone_config.json");
    let zone_config: ZoneConfig = serde_json::from_str(
            &fs::read_to_string(&zone_config_file)
                .map_err(|e| format!("读取 {:?} 失败: {}", zone_config_file, e))?,
        )
        .map_err(|e| format!("解析 {:?} 失败: {}", zone_config_file, e))?;
    let zone_did = zone_config.id.clone();

    // 获取用户密钥对z
    let key_pair = TestKeys::get_key_pair_by_id(username)
        .or_else(|_| {
            println!("警告: 未找到用户 {} 的密钥对，使用 devtest 密钥对", username);
            TestKeys::get_key_pair_by_id("devtest")
        })?;

    let scope = builder.user_scope(username, zone_did.clone(), &key_pair);

    // 确保用户配置已存在
    if !scope.user_dir.join("user_config.json").exists() {
        return Err(format!("用户配置不存在: {}", scope.user_dir.join("user_config.json").display()));
    }

    scope.create_node_config(device_name, net_id.map(|s| s.to_string()));

    println!("成功创建节点配置: {}.{}", username, device_name);
    Ok(())
}

/// 创建 SN 配置（命令行接口）
pub async fn cmd_create_sn_configs(output_dir: Option<&str>) -> Result<(), String> {
    let root_dir = if let Some(dir) = output_dir {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| format!("获取当前目录失败: {}", e))?
    };

    let builder = DevEnvBuilder::from_path(root_dir);
    create_sn_config(&builder).await;
    
    println!("成功创建 SN 配置");
    Ok(())
}

/// Register device to SN database (command line interface)
pub async fn cmd_register_device(
    username: &str,
    device_name: &str,
    sn_db_path: &str,
    output_dir: Option<&str>,
) -> Result<(), String> {
    let root_dir = if let Some(dir) = output_dir {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    };

    let builder = DevEnvBuilder::from_path(root_dir);
    register_device_to_sn(&builder, username, device_name, sn_db_path).await?;
    
    println!("Successfully registered device {}.{} to SN", username, device_name);
    Ok(())
}

// ============================================================================
// 主入口函数
// ============================================================================

/// 创建完整的测试环境配置
#[allow(dead_code)]
pub async fn create_test_env_configs() {
    let builder = DevEnvBuilder::new("buckyos_dev_configs");

    // 设置已知的 web3 bridge 配置（如果存在全局配置）
    // let mut test_web3_bridge = HashMap::new();
    // test_web3_bridge.insert("bns".to_string(), "web3.buckyos.io".to_string());
    // KNOWN_WEB3_BRIDGE_CONFIG.set(test_web3_bridge.clone());

    // 1. 创建 devtest 用户环境
    let devtest_keys = TestKeys::devtest_owner();
    let devtest_scope = builder.user_scope("devtest", DID::new("bns", "devtest"), &devtest_keys);
    devtest_scope.create_owner_config();

    // devtest node1
    let _node1_keys = TestKeys::devtest_node1();
    devtest_scope.create_node_config(
        "node1",
        Some("lan1".to_string()),
    );

    // 2. 创建 bob 用户环境
    let bob_keys = TestKeys::bob_owner();
    let bob_scope = builder.user_scope("bobdev", DID::new("bns", "bobdev"), &bob_keys);
    bob_scope.create_owner_config();

    // Bob Zone
    let _bob_zone_jwt = bob_scope.create_zone_boot_config_jwt(
        Some("sn.buckyos.io".to_string()),
        "ood1".to_string().parse().unwrap(),
    );

    // Bob OOD1
    bob_scope.create_node_config("ood1", Some("lan2".to_string()));

    // 3. 创建 SN 配置
    create_sn_config(&builder).await;

    // 4. 初始化 SN 数据库（如果 SnDB 可用）
    // let sn_db_path = builder.root_dir().join("sn_db.sqlite3");
    // if sn_db_path.exists() {
    //     std::fs::remove_file(&sn_db_path).unwrap();
    // }
    // 
    // let db = SnDB::new_by_path(sn_db_path.to_str().unwrap()).unwrap();
    // db.initialize_database();
    // 
    // // 插入激活码
    // for code in &["test-active-sn-code-bob", "11111", "22222", "33333", "44444", "55555"] {
    //     db.insert_activation_code(code).unwrap();
    // }
    // 
    // // 注册用户
    // let bob_public_key_str = serde_json::to_string(&bob_keys.public_key_jwk).unwrap();
    // db.register_user(
    //     "test-active-sn-code-bob",
    //     "bob",
    //     bob_public_key_str.as_str(),
    //     bob_zone_jwt.as_str(),
    //     None,
    // )
    // .unwrap();
    // 
    // // 注册设备
    // let mut device_info = DeviceInfo::new("ood1", bob_ood1_did.clone());
    // device_info.auto_fill_by_system_info().await.unwrap();
    // let device_info_json = serde_json::to_string_pretty(&device_info).unwrap();
    // 
    // db.register_device(
    //     "bob",
    //     "ood1",
    //     bob_ood1_did.to_string().as_str(),
    //     "192.168.100.100",
    //     device_info_json.as_str(),
    // )
    // .unwrap();
    // 
    // println!("# sn_db created at {}", sn_db_path.to_string_lossy());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 为测试统一创建 DevEnvBuilder，根目录按测试名区分
    fn new_test_builder(test_name: &str) -> DevEnvBuilder {
        let root_dir = format!(".buckycli_{}", test_name);
        DevEnvBuilder::new(&root_dir)
    }

    #[tokio::test]
    /// 创建简单的测试配置（用于 test_all_dev_env_configs）
    pub async fn test_all_dev_env_configs() {
        let builder = new_test_builder("all_dev_env_configs");
        let owner_keys = TestKeys::get_key_pair_by_id("devtest").unwrap();
        let device_keys = TestKeys::get_key_pair_by_id("devtest.ood1").unwrap();

        let scope = builder.user_scope("devtest", DID::new("bns", "devtest"), &owner_keys);
        scope.create_owner_config();

        // 创建设备配置
        let device_jwk = get_jwk(&device_keys.public_key_x);
        let mut device_config = DeviceConfig::new_by_jwk("ood1", device_jwk.clone());
        device_config.support_container = true;
        device_config.iss = "did:bns:devtest".to_string();

        let owner_key = get_encoding_key(owner_keys.private_key_pem);
        let device_jwt = device_config.encode(Some(&owner_key)).unwrap();

        // 创建 Zone 配置
        let zone_did = DID::new("web", "test.buckyos.io");
        let ood: OODDescriptionString = "ood1".to_string().parse().unwrap();
        let (zone_boot_jwt, _txt_records) = scope.create_zone_boot_config_jwt(None, ood);

        // 创建节点身份配置
        let node_identity_config = NodeIdentityConfig {
            zone_did: zone_did.clone(),
            owner_public_key: get_jwk(&owner_keys.public_key_x),
            owner_did: DID::new("bns", "devtest"),
            device_doc_jwt: device_jwt.to_string(),
            zone_iat: builder.now() as u32,
        };
        write_json(
            &builder.root_dir().join("node_identity.json"),
            &node_identity_config,
        );

        // 创建启动配置
        let start_config = json!({
            "admin_password_hash": ADMIN_PASSWORD_HASH,
            "device_private_key": device_keys.private_key_pem,
            "device_public_key": device_jwk,
            "friend_passcode": "sdfsdfsdf",
            "gateway_type": "PortForward",
            "guest_access": true,
            "private_key": owner_keys.private_key_pem,
            "public_key": get_jwk(&owner_keys.public_key_x),
            "user_name": "devtest",
            "zone_name": zone_did.to_host_name(),
            "BUCKYOS_ROOT": "/opt/buckyos"
        });
        write_json(&builder.root_dir().join("start_config.json"), &start_config);

        // 输出 DNS 记录
        println!("# test.buckyos.io TXT Record: DID={};", zone_boot_jwt);
        if let Ok(owner_x) = get_x_from_jwk(&get_jwk(&owner_keys.public_key_x)) {
            println!("# test.buckyos.io TXT Record: PKX=0:{};", owner_x);
        }
        if let Ok(ood_x) = get_x_from_jwk(&get_jwk(&device_keys.public_key_x)) {
            println!("# test.buckyos.io TXT Record: PKX=1:{};", ood_x);
        }
    }

    // ============================================================================
    // 单元测试
    // ============================================================================

    #[test]
    fn test_zone_boot_config() {
        // 使用 DevEnvBuilder + create_zone_boot_config_jwt 来构造 ZoneBootConfig 的 JWT
        let builder = new_test_builder("zone_boot_config");
        let owner_keys = TestKeys::devtest_owner();
        let zone_did = DID::new("bns", "devtest");
        let scope = builder.user_scope("devtest", zone_did.clone(), &owner_keys);

        // 构造一个简单的 OOD 描述和 SN 主机名
        let ood: OODDescriptionString = "ood1".to_string().parse().unwrap();
        let sn_host = Some("sn.buckyos.io".to_string());

        // 通过 create_zone_boot_config_jwt 生成 JWT
        let (zone_boot_config_jwt, _txt_records) =
            scope.create_zone_boot_config_jwt(sn_host.clone(), ood.clone());

        // 使用 owner 公钥对 JWT 进行解码
        let public_key_jwk = get_jwk(&owner_keys.public_key_x);
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        // 将 JWT 字符串封装为 EncodedDocument，再进行解码
        let encoded_doc = EncodedDocument::from_str(zone_boot_config_jwt.clone()).unwrap();
        let zone_boot_config_decoded =
            ZoneBootConfig::decode(&encoded_doc, Some(&public_key)).unwrap();
        println!("zone_boot_config_decoded: {:?}", zone_boot_config_decoded);

        // 构造期望的 ZoneBootConfig，与 create_zone_boot_config_jwt 中的逻辑保持一致
        let expected_zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec![ood],
            sn: sn_host,
            exp: builder.exp(),
            devices: HashMap::new(),
            owner: None,
            owner_key: None,
            gateway_devs: vec![],
            extra_info: HashMap::new(),
        };

        assert_eq!(expected_zone_boot_config, zone_boot_config_decoded);
    }



    #[tokio::test]
    async fn test_device_config() {
        // 使用 TestKeys + 辅助函数来构造 owner 密钥
        let owner_keys = TestKeys::devtest_owner();
        let public_key_jwk = get_jwk(&owner_keys.public_key_x);
        let owner_private_key = get_encoding_key(owner_keys.private_key_pem);
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        // OOD1 设备
        let ood_public_key = json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": "5bUuyWLOKyCre9az_IhJVIuOw8bA0gyKjstcYGHbaPE"
        });
        let _ood_key_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(ood_public_key).unwrap();
        let mut device_config = DeviceConfig::new(
            "ood1",
            "5bUuyWLOKyCre9az_IhJVIuOw8bA0gyKjstcYGHbaPE".to_string(),
        );
        device_config.iss = "did:bns:lzc".to_string();

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("ood json_str: {}", json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("ood encoded: {:?}", encoded);

        let decoded = DeviceConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!(
            "ood decoded: {:?}",
            serde_json::to_string(&decoded).unwrap()
        );
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        // 注意：DeviceInfo 需要从 buckyos_api 导入
        // let mut device_info_ood = DeviceInfo::from_device_doc(&decoded);
        // device_info_ood.auto_fill_by_system_info().await.unwrap();
        // let device_info_str = serde_json::to_string(&device_info_ood).unwrap();
        // println!("ood device_info: {}", device_info_str);

        assert_eq!(device_config, decoded);
        assert_eq!(encoded, token2);

        // Gateway 设备
        let gateway_public_key = json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": "M3-pAdhs0uFkWmmjdHLBfs494R91QmQeXzCEhEHP-tI"
        });
        let _gateway_key_jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(gateway_public_key).unwrap();
        let device_config = DeviceConfig::new(
            "gateway",
            "M3-pAdhs0uFkWmmjdHLBfs494R91QmQeXzCEhEHP-tI".to_string(),
        );

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("gateway json_str: {:?}", json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("gateway encoded: {:?}", encoded);

        let decoded = DeviceConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!(
            "gateway decoded: {:?}",
            serde_json::to_string(&decoded).unwrap()
        );
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        assert_eq!(device_config, decoded);
        assert_eq!(encoded, token2);

        // Server 设备
        let server_public_key = json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": "LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0"
        });
        let _server_key_jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(server_public_key).unwrap();
        let mut device_config = DeviceConfig::new(
            "server1",
            "LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0".to_string(),
        );
        device_config.iss = "did:bns:waterflier".to_string();
        device_config.ip = None;
        device_config.net_id = None;

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("server json_str: {:?}", json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("server encoded: {:?}", encoded);

        let decoded = DeviceConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!(
            "server decoded: {:?}",
            serde_json::to_string(&decoded).unwrap()
        );
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        assert_eq!(device_config, decoded);
        assert_eq!(encoded, token2);
    }

    #[test]
    fn test_owner_config() {
        // 使用 TestKeys + 辅助函数来构造 owner 配置所需的密钥
        let owner_keys = TestKeys::devtest_owner();
        let public_key_jwk = get_jwk(&owner_keys.public_key_x);
        let private_key = get_encoding_key(owner_keys.private_key_pem);
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        let mut owner_config = OwnerConfig::new(
            DID::new("bns", "lzc"),
            "lzc".to_string(),
            "zhicong liu".to_string(),
            public_key_jwk,
        );

        owner_config.set_default_zone_did(DID::new("bns", "waterflier"));

        let json_str = serde_json::to_string_pretty(&owner_config).unwrap();
        println!("json_str: {}", json_str.as_str());

        let encoded = owner_config.encode(Some(&private_key)).unwrap();
        println!("encoded: {:?}", encoded);

        let decoded = OwnerConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!(
            "decoded: {}",
            serde_json::to_string_pretty(&decoded).unwrap()
        );
        let token2 = decoded.encode(Some(&private_key)).unwrap();

        assert_eq!(owner_config, decoded);
        assert_eq!(encoded, token2);
    }
}