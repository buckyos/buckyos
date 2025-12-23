use buckyos_kit::buckyos_get_unix_timestamp;
use jsonwebtoken::{DecodingKey, EncodingKey};
use name_lib::{
    DID, DIDDocumentTrait, DeviceConfig, DeviceInfo, DeviceMiniConfig, EncodedDocument, NodeIdentityConfig, OODDescriptionString, OwnerConfig, ZoneBootConfig, ZoneConfig, generate_ed25519_key_pair, get_x_from_jwk
};
use package_lib::PackageId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::fs;
use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::{SigningKey, VerifyingKey};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use cyfs_sn::SnDB;

use crate::{AppDoc, LocalAppInstanceConfig, REPO_SERVICE_UNIQUE_ID, SCHEDULER_SERVICE_UNIQUE_ID, SMB_SERVICE_UNIQUE_ID, ServiceInstallConfig, ServiceInstanceState, SubPkgDesc, VERIFY_HUB_UNIQUE_ID};

// ============================================================================
// Constant Definitions
// ============================================================================

const BASE_TIME: u64 = 1743478939; // 2025-04-01
const DEFAULT_EXP_YEARS: u64 = 10;
const ADMIN_PASSWORD_HASH: &str = "o8XyToejrbCYou84h/VkF4Tht0BeQQbuX3XKG+8+GQ4="; // bucky2025

// ============================================================================
// Key Data Management
// ============================================================================

/// Test key pair collection
pub(crate) struct TestKeyPair {
    private_key_pem: String,
    public_key_x: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ZoneTxtRecord {
    pub boot_config_jwt: String,
    pub device_mini_doc_jwt: String,
    pub pkx: String,
}

struct TestKeys;

impl TestKeys {
    fn verify_key_pair(key_pair: &TestKeyPair) -> Result<(), String> {
        let signing_key = SigningKey::from_pkcs8_pem(key_pair.private_key_pem.as_str())
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
            "devtest_ood1",
            "devtest_node1",
            "devtests",
            "devtests_ood1",
            "sn_owner",
            "sn",
            "bob",
            "bob_ood1",
            "alice",
            "alice_ood1",
            "charlie",
            "charlie_ood1",
        ];
        for key_id in key_ids {
            println!("Testing key pair: {}", key_id);
            TestKeys::get_key_pair_by_id(key_id)?;
        }
        Ok(())
    }

    fn get_key_pair_by_id(id: &str) -> Result<TestKeyPair, String> {
        let key_pair = match id {
            //zone-id did:web:test.buckyos.io
            "devtest" => TestKeys::devtest_owner(),
            "devtest_ood1" => TestKeys::devtest_ood1(),
            "devtest.ood1" => TestKeys::devtest_ood1(),
            "devtest_node1" => TestKeys::devtest_node1(),
            "devtest.node1" => TestKeys::devtest_node1(),

            "sn_owner" => TestKeys::sn_owner(),
            //zone-id did:web:devtests.org
            "devtests" => TestKeys::sn_owner(),
            //zone-id None (sn is not a zone)
            "sn" => TestKeys::sn_server(),
            "sn_server" => TestKeys::sn_server(),
            "buckyos" => TestKeys::buckyos(),
            "sn.buckyos" => TestKeys::sn_buckyos(),
            "devtests_ood1" => TestKeys::devtests_ood1(),
            "devtests.ood1" => TestKeys::devtests_ood1(),
            "sn_web" => TestKeys::devtests_ood1(),

            //zone-id did:bns:bob
            "bob" => TestKeys::bob_owner(),
            "bob_ood1" => TestKeys::bob_ood1(),
            "bob.ood1" => TestKeys::bob_ood1(),

            //zone-id did:bns:alice
            "alice" => TestKeys::alice_owner(),
            "alice_ood1" => TestKeys::alice_ood1(),
            "alice.ood1" => TestKeys::alice_ood1(),

            //zone-id did:web:charlie.me
            "charlie" => TestKeys::charlie_owner(),
            "charlie_ood1" => TestKeys::charlie_ood1(),
            "charlie.ood1" => TestKeys::charlie_ood1(),
            _ => return Err(format!("unknown key pair id: {}", id)),
        };

        TestKeys::verify_key_pair(&key_pair)?;
        Ok(key_pair)
    }

    fn devtest_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8".to_string(),
        }
    }

    fn devtest_ood1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc".to_string(),
        }
    }

    fn devtest_node1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEICwMZt1W7P/9v3Iw/rS2RdziVkF7L+o5mIt/WL6ef/0w
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "Bb325f2ed0XSxrPS5sKQaX7ylY9Jh9rfevXiidKA1zc".to_string(),
        }
    }

    fn bob_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEILQLoUZt2okCht0UVhsf4UlGAV9h3BoliwZQN5zBO1G+
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "y-kuJcQ0doFpdNXf4HI8E814lK8MB3-t4XjDRcR_QCU".to_string(),
        }
    }

    fn bob_ood1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIADmO0+u/gcmStDsHZOZCM5gxNYlQmP6jpMo279TQE75
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "iSMKakFEGzGAxLTlaB5TkqZ6d4wurObr-BpaQleoE2M".to_string(),
        }

    }
    //did:bns:devtests
    fn sn_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMkwWZKUe7+z7NtfgbgxWwGjMddvxtrmeGJiJe8rq00M
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "blzinUlTNGYcvCPFT1OfPKPbmjvteuXWMwQG55cTo7M".to_string(),
        }
    }

    fn sn_server() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIBvnIIa1Tx45SjRu9kBZuMgusP5q762SvojXZ4scFxVD
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "FPvY3WXPxuWPYFuwOY0Qbh0O7-hhKr6ta1jTcX9ORPI".to_string(),
        }
    }

    fn devtests_ood1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEICBO4nQL1yMcu4uu51Grea+VTaaS+sswioMRZXoltzZh
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "waupPnLqJRwjr3hJ_2i2J4qGLx-8t5ihX6LET0ZY828".to_string(),
        }
    }

    //did:bns:alice
    fn alice_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIKH6oJdebg+xxICY7Z1vm84qMkSzm6Wk0ic88DGR90aq
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "uh7RD37tflN65CrcJSUQ3vGnyU4vmC7_M8IkEEOHnds".to_string(),
        }
    }

    fn alice_ood1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIGhyUJ3/YgIrLZxSGG7o1bgiWcyETZKjTBoGagNdpxVy
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "E1oQDYqzyX4ysrNgTJ5DAVaMgA3By8XpBa0e6r2gBqQ".to_string(),
        }
    }

    fn charlie_owner() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEICLjVTK81RKQ1aPtSLKFx/Fl33+WbxgqCpPCBFlqlBQX
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "cuFY7qeU1q96O1K5RRbXo7GXGR78szB-gmmkBXDMscE".to_string(),
        }
    }

    fn charlie_ood1() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMe0Q/tl7DWbu3SIQE8vnDxO8YQMIivAlCgKiNUfjcWU
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "PY9uu16H74QYVRjstVxdWdAsgkoy10-74fvQhx4ddek".to_string(),
        }
    }

    fn buckyos() -> TestKeyPair {
        TestKeyPair {
            private_key_pem: r#"-----BEGIN PRIVATE KEY-----
TODOTODO
-----END PRIVATE KEY-----"#.to_string(),
            public_key_x: "qmtOLLWpZeBMzt97lpfj2MxZGWn3QfuDB7Q4uaP3Eok".to_string(),
        }
    }

    fn sn_buckyos() -> TestKeyPair {
        let (private_key, public_key) = generate_ed25519_key_pair();
        let x = public_key
            .get("x").unwrap().as_str().unwrap().to_string();

        TestKeyPair {
            private_key_pem: private_key,
            public_key_x: x,
        }
    }

}

// ============================================================================
// Helper Functions
// ============================================================================

/// Write file and print log
fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
    println!("# Write file: {}", path.to_string_lossy());
}

/// Write JSON file
fn write_json<T: Serialize>(path: &Path, data: &T) {
    let content = serde_json::to_string_pretty(data).unwrap();
    write_file(path, &content);
}

/// Create EncodingKey from PEM string
fn get_encoding_key(pem: &str) -> EncodingKey {
    EncodingKey::from_ed_pem(pem.as_bytes()).unwrap()
}

/// Create JWK from Value
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
// Configuration Builder
// ============================================================================

/// Development environment configuration builder
pub struct DevEnvBuilder {
    root_dir: PathBuf,
    now: u64,
    exp: u64,
}

impl DevEnvBuilder {
    /// Create new builder
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

    /// Create builder from specified path
    pub fn from_path(root_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&root_dir).unwrap();
        println!("# Dev configs root: {:?}", root_dir);

        Self {
            root_dir,
            now: BASE_TIME,
            exp: BASE_TIME + 3600 * 24 * 365 * DEFAULT_EXP_YEARS,
        }
    }

    /// Get root directory
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// Get current timestamp
    #[allow(dead_code)]
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Get expiration timestamp
    pub fn exp(&self) -> u64 {
        self.exp
    }

    /// Create user scope
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

/// User environment scope
pub struct UserEnvScope<'a> {
    builder: &'a DevEnvBuilder,
    username: &'a str,
    did: DID,//owner did?
    key_pair: &'a TestKeyPair,
    user_dir: PathBuf,
    zone_did: DID,
}

impl<'a> UserEnvScope<'a> {
    /// Create Owner configuration
    pub fn create_owner_config(&self) {
        write_file(&self.user_dir.join("user_private_key.pem"), self.key_pair.private_key_pem.as_str());

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

    /// Create Zone configuration 
    /// return zone_boot_config_jwt, TXT Records
    pub fn create_zone_boot_config_jwt(&self, sn_host: Option<String>, ood:OODDescriptionString, rtcp_port: u16) -> ZoneTxtRecord {
        let device_full_id = format!("{}.{}", self.username, ood.name.as_str());
        let device_key_pair = TestKeys::get_key_pair_by_id(&device_full_id).unwrap();
        let ood_net_id = ood.net_id.clone();
        let mut ddns_sn_url = None;
        let mut real_sn_host = sn_host.clone();
        if ood_net_id.is_some() {
            let ood_net_id = ood_net_id.unwrap();
            if ood_net_id.starts_with("wan") {
                real_sn_host = None;
            }

            if ood_net_id.starts_with("wan_dyn") {
                if sn_host.is_some() {
                    let sn_real_host = sn_host.clone().unwrap();
                    ddns_sn_url = Some(format!("https://{}/kapi/sn", sn_real_host));
                }
            }
        }


        let mut zone_boot = ZoneBootConfig {
            id: None,
            oods: vec![ood.clone()],
            sn: real_sn_host,
            exp: self.builder.exp,
            devices:HashMap::new(),
            owner: None,
            owner_key: None,
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

        let owner_key = get_encoding_key(self.key_pair.private_key_pem.as_str());
        let jwt = zone_boot.encode(Some(&owner_key)).unwrap();
        let pkx = get_x_from_jwk(&get_jwk(&self.key_pair.public_key_x)).unwrap();
        println!("=> {} TXT Record({}): BOOT={};", zone_host_name, jwt.to_string().len()+6, jwt.to_string());
        println!("=> {} TXT Record({}): PKX={};", zone_host_name, pkx.len()+5, pkx.as_str());

        let real_rtcp_port = if rtcp_port == 2980 { None } else { Some(rtcp_port as u32) };
        //ood1 mini config jwt
        let mini_config = DeviceMiniConfig {
            name: ood.name.clone(),
            x: device_key_pair.public_key_x.clone(),
            rtcp_port: real_rtcp_port,
            exp: self.builder.exp,
            extra_info: HashMap::new(),
        };   
        let mini_jwt = mini_config.to_jwt(&owner_key).unwrap();
        println!("=> {} TXT Record({}): DEV={};", zone_host_name, mini_jwt.len()+5, mini_jwt.to_string());

        // 2. Create device configuration and JWT
        let device_jwk = get_jwk(&device_key_pair.public_key_x);
        let mut device_config = DeviceConfig::new_by_jwk(ood.name.as_str(), device_jwk.clone());
        device_config.support_container = true;
        device_config.net_id = ood.net_id.clone();
        device_config.iss = self.did.to_string();
        device_config.ddns_sn_url = ddns_sn_url;
        let node_dir = self.user_dir.join(ood.name.as_str());
        write_json(&node_dir.join("node_device_config.json"), &device_config);
   

        println!(
            "{} device config: {}",
            ood.name.as_str(),
            serde_json::to_string_pretty(&device_config).unwrap()
        );

        let zone_txt_record = ZoneTxtRecord {
            boot_config_jwt: jwt.to_string(),
            device_mini_doc_jwt: mini_jwt.to_string(),
            pkx: pkx,
        };
        write_json(&self.user_dir.join("zone_txt_record.json"), &zone_txt_record);
        println!("zone txt record write to file: {}", self.user_dir.join("zone_txt_record.json").to_string_lossy());
        zone_txt_record
    }

    /// Create node configuration
    pub fn create_node_config(
        &self,
        device_name: &str,
        net_id: Option<String>,
    ) -> String {
        let full_device_name = format!("{}.{}", self.username, device_name);
        let device_key_pair = TestKeys::get_key_pair_by_id(&full_device_name).unwrap();
        let node_dir = self.user_dir.join(device_name);
        // 1. Write device private key
        write_file(&node_dir.join("node_private_key.pem"), device_key_pair.private_key_pem.as_str());

        // 2. Create device configuration and JWT
        let device_jwk = get_jwk(&device_key_pair.public_key_x);
        //load device_config from file
        let device_config_path = node_dir.join("node_device_config.json");
        let device_config: DeviceConfig = serde_json::from_str(
            &fs::read_to_string(&device_config_path).unwrap()
        ).unwrap();

        println!("input net_id: {:?},device_config.net_id: {:?}", net_id, device_config.net_id);
  
        let owner_key = get_encoding_key(self.key_pair.private_key_pem.as_str());
        let device_jwt = device_config.encode(Some(&owner_key)).unwrap();
        println!("{} device jwt: {}", device_name, device_jwt.to_string());

        // Create device_mini_config_jwt
        let device_mini_config = DeviceMiniConfig::new_by_device_config(&device_config);
        let device_mini_jwt = device_mini_config.to_jwt(&owner_key).unwrap();
        println!("{} device mini config jwt: {}", device_name, device_mini_jwt.to_string());
        write_file(&node_dir.join("device_mini_config.jwt"), device_mini_jwt.to_string().as_str());

        // 3. Create node identity configuration
        let identity_config = NodeIdentityConfig {
            zone_did: self.zone_did.clone(),
            owner_public_key: get_jwk(&self.key_pair.public_key_x),
            owner_did: self.did.clone(),
            device_doc_jwt: device_jwt.to_string(),
            zone_iat: self.builder.now as u32,
            device_mini_doc_jwt: device_mini_jwt.to_string(),
        };
        write_json(&node_dir.join("node_identity.json"), &identity_config);

        // 4. Create startup configuration (only for OOD nodes)
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

        // Return device self-signed JWT (for verification)
        let device_key = get_encoding_key(device_key_pair.private_key_pem.as_str());
        device_config.encode(Some(&device_key)).unwrap().to_string()
    }
}

// ============================================================================
// SN Configuration Generation
// ============================================================================
pub async fn create_formula_sn_config() {
    let sn_dir = PathBuf::from("/opt/sn.buckyos.ai/");
    let owner_keys = TestKeys::get_key_pair_by_id("buckyos").unwrap();
    let device_keys = TestKeys::get_key_pair_by_id("sn.buckyos").unwrap();
    let device_mini_config = DeviceMiniConfig {
        name: "sn".to_string(),
        x: device_keys.public_key_x.clone(),
        rtcp_port: None,
        exp: buckyos_get_unix_timestamp() + 3600 * 24 * 365 * DEFAULT_EXP_YEARS,
        extra_info: HashMap::new(),
    };
    let device_mini_jwt = device_mini_config.to_jwt(&get_encoding_key(owner_keys.private_key_pem.as_str())).unwrap();  
    let mut device_config = DeviceConfig::new_by_mini_config(&device_mini_config, DID::new("web", "sn.buckyos.ai"), DID::new("bns", "buckyos"));
    device_config.net_id = Some("wan".to_string());
    write_json(&sn_dir.join("sn_device_config.json"), &device_config);
    write_file(&sn_dir.join("sn_private_key.pem"), device_keys.private_key_pem.as_str());
    println!("- Created sn device config & private key.");


    // Create ZoneBootConfig
    let zone_boot = ZoneBootConfig {
        id: None,
        oods: vec!["sn".parse().unwrap()],
        sn: None,
        exp: buckyos_get_unix_timestamp() + 3600 * 24 * 365 * DEFAULT_EXP_YEARS,
        owner: None,
        owner_key: None,
        extra_info: HashMap::new(),
        devices: HashMap::new(),
    };

    let owner_key = get_encoding_key(owner_keys.private_key_pem.as_str());
    let zone_boot_jwt = zone_boot.encode(Some(&owner_key)).unwrap().to_string();
    let x_str = owner_keys.public_key_x;

    //create params.json
    let params = json!({"params":{
        "sn_boot_jwt": zone_boot_jwt,
        "sn_owner_pk": x_str,
        "sn_device_jwt": device_mini_jwt,
        "sn_host": "buckyos.ai",
        "sn_ip": "127.0.0.1".to_string(),
    }});
    println!("params: {:?}", params);
    write_json(&sn_dir.join("params.json"), &params);
    println!("- Created params.json.");

    // Create initial database
    let sn_db_path = sn_dir.join("sn_db.sqlite3");
    let db = SnDB::new_by_path(&sn_db_path.to_string_lossy()).unwrap();
    db.initialize_database().unwrap();
    println!("- Created SN database.");
}

/// Create SN server configuration
pub async fn create_sn_config(builder: &DevEnvBuilder,sn_ip:IpAddr,sn_base_host:String) {
    let sn_dir = builder.root_dir().join("sn_server");
    let owner_keys = TestKeys::get_key_pair_by_id("sn_owner").unwrap();
    let device_keys = TestKeys::get_key_pair_by_id("sn_server").unwrap();

    // Save owner_config to sn_dir directory
    write_file(&builder.root_dir().join("sn_server").join(".buckycli").join("user_private_key.pem"), owner_keys.private_key_pem.as_str());

    let owner_jwk = get_jwk(owner_keys.public_key_x.as_str());
    let owner_config = OwnerConfig::new(
        DID::new("bns", "sn"),
        "root".to_string(),
        "sn admin".to_string(),
        owner_jwk,
    );

    write_json(&builder.root_dir().join("sn_server").join(".buckycli").join("user_config.json"), &owner_config);
    println!("- Created owner config for sn admin.");

    // Create device JWT
    let device_mini_config = DeviceMiniConfig {
        name: "sn".to_string(),
        x: device_keys.public_key_x.clone(),
        rtcp_port: None,
        exp: builder.exp(),
        extra_info: HashMap::new(),
    };
    let device_mini_jwt = device_mini_config.to_jwt(&get_encoding_key(owner_keys.private_key_pem.as_str())).unwrap();

    let mut device_config = DeviceConfig::new_by_mini_config(&device_mini_config, DID::new("web", "sn.devtests.org"), DID::new("bns", "sn"));
    device_config.net_id = Some("wan".to_string());
    write_json(&sn_dir.join("sn_device_config.json"), &device_config);
    write_file(&sn_dir.join("sn_private_key.pem"), device_keys.private_key_pem.as_str());
    println!("- Created sn device config & private key.");

    // Create ZoneBootConfig
    let zone_boot = ZoneBootConfig {
        id: None,
        oods: vec!["sn".parse().unwrap()],
        sn: None,
        exp: builder.exp(),
        owner: None,
        owner_key: None,
        extra_info: HashMap::new(),
        devices: HashMap::new(),
    };

    let owner_key = get_encoding_key(owner_keys.private_key_pem.as_str());
    let zone_boot_jwt = zone_boot.encode(Some(&owner_key)).unwrap().to_string();
    let x_str = owner_keys.public_key_x;

    //create params.json
    let params = json!({"params":{
        "sn_boot_jwt": zone_boot_jwt,
        "sn_owner_pk": x_str,
        "sn_device_jwt": device_mini_jwt,
        "sn_host": sn_base_host,
        "sn_ip": sn_ip.to_string(),
        "sn_cer": "fullchain.cert",
        "sn_pem": "fullchain.pem",
        "web3_cer": "fullchain.cert",
        "web3_pem": "fullchain.pem"
    }});
    println!("params: {:?}", params);
    write_json(&sn_dir.join("params.json"), &params);
    println!("- Created params.json.");

    // Create initial database
    let sn_db_path = builder.root_dir().join("sn_server").join("sn_db.sqlite3");
    let db = SnDB::new_by_path(&sn_db_path.to_string_lossy()).unwrap();
    db.initialize_database().unwrap();
    for i in 0..9 {
        let code = format!("sndevtest{}", i);
        println!("insert activation code: {}", code);
        db.insert_activation_code(&code).unwrap();
    }

    println!("- Created SN database.");
   
}


pub async fn register_user_to_sn(
    builder: &DevEnvBuilder,
    user_zone_id: &str,
    sn_db_path: &str,
) -> Result<(), String> {
    // Parse username
    let user_config_path = builder.root_dir().join(user_zone_id).join("user_config.json");
    println!("user_config_path: {}", user_config_path.display());
    let user_config_content = fs::read_to_string(&user_config_path)
        .map_err(|e| format!("Failed to read user_config.json: {}", e))?;
    let user_config: Value = serde_json::from_str(&user_config_content)
        .map_err(|e| format!("Failed to parse user_config.json: {}", e))?;
    let username = user_config
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("user_config.json missing name field")?;

    let zone_config_file = builder.root_dir().join(user_zone_id).join("zone_config.json");
    let zone_config: ZoneConfig = serde_json::from_str(
            &fs::read_to_string(&zone_config_file)
                .map_err(|e| format!("Failed to read {:?}: {}", zone_config_file, e))?,
        )
        .map_err(|e| format!("Failed to parse {:?}: {}", zone_config_file, e))?;

    let zone_did = zone_config.id.clone();
    let mut user_domain = None;
    if zone_did.method != "bns" {
        user_domain = Some(zone_did.to_host_name());
    }

    let zone_record_path = builder.root_dir().join(user_zone_id).join("zone_txt_record.json");
    let zone_record: ZoneTxtRecord = serde_json::from_str(
        &fs::read_to_string(&zone_record_path)
            .map_err(|e| format!("Failed to read {:?}: {}", zone_record_path, e))?,
    )
    .map_err(|e| format!("Failed to parse {:?}: {}", zone_record_path, e))?;

    let db = SnDB::new_by_path(&sn_db_path).unwrap();
    let public_key_json = json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": zone_record.pkx,
    });
    println!("public_key_json: {:?}", public_key_json);
    db.register_user_directly(
        username,
        public_key_json.to_string().as_str(),
        zone_record.boot_config_jwt.as_str(),
        user_domain
    ).map_err(|e| format!("Failed to register user: {}", e))?;
    println!(
        "Successfully registered user {}@{} to SN database at {}",
        username, user_zone_id, sn_db_path
    );
    return Ok(());
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
    user_zone_id: &str,
    device_name: &str,
    sn_db_path: &str,
) -> Result<(), String> {
    // Find device config directory
    // Try to find device config in builder root: {username}/{device_name}/node_identity.json
    let device_dir = builder.root_dir().join(user_zone_id).join(device_name);
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

    let username = node_identity.owner_did.id;

    // Extract device DID from device_doc_jwt
    let device_doc_jwt = node_identity.device_doc_jwt.clone();
    let device_mini_doc_jwt = node_identity.device_mini_doc_jwt.clone();
    let encoded_doc = EncodedDocument::from_str(device_doc_jwt.clone())
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

    db.register_device(
        &username,
        device_name,
        &device_did.to_string(),
        &device_mini_doc_jwt,
        &device_ip,
        &device_info_json
    )
    .map_err(|e| format!("Failed to register device: {}", e))?;
    println!(
        "Successfully registered device {}.{} (DID: {:?}) to SN database at {}",
        username, device_name, device_did, sn_db_path
    );
    Ok(())
}

// ============================================================================
// Command Line Interface Functions
// ============================================================================

/// Create user environment configuration (command line interface)
pub async fn cmd_create_user_env(
    username: &str,
    hostname: &str,
    ood_name: &str,
    sn_base_host: &str, 
    rtcp_port: u16,
    output_dir: Option<&str>,
) -> Result<(), String> {
    let root_dir = if let Some(dir) = output_dir {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    };

    let builder = DevEnvBuilder::from_path(root_dir);
    
    // Get or create user key pair
    let key_pair = TestKeys::get_key_pair_by_id(username)
        .or_else(|_| {
            // If predefined key not found, try username.owner format
            TestKeys::get_key_pair_by_id(&format!("{}.owner", username))
        })
        .or_else(|_| {
            // If still not found, use devtest as default (for testing only)
            println!("Warning: Key pair for user {} not found, using devtest key pair", username);
            TestKeys::get_key_pair_by_id("devtest")
        })?;

    // Create zone DID from hostname (using "web" scheme)
    let mut zone_did = DID::new("web", hostname);
    let mut sn_host = None;
    if sn_base_host.contains(".") {
        let web3_bns = format!("web3.{}", sn_base_host);
        zone_did = DID::from_host_name_by_bridge(hostname,"bns",&web3_bns).unwrap();
        sn_host = Some(format!("sn.{}", sn_base_host));
    }

    let scope = builder.user_scope(username, zone_did.clone(), &key_pair);
    
    // Create owner configuration
    scope.create_owner_config();
    
    // Create zone_boot_config (currently only generates a simple OOD description, SN is empty)
    let ood: OODDescriptionString = ood_name.to_string().parse().unwrap();
    let _zone_txt_record = scope.create_zone_boot_config_jwt(sn_host, ood, rtcp_port);

    println!("Successfully created user environment configuration: {}", username);
    println!("Zone hostname: {}", hostname);
    println!("Zone netid: {}", ood_name);
    Ok(())
}

/// Create node configuration (command line interface)
///
/// env_dir should be a user environment directory generated by cmd_create_user_env.
pub async fn cmd_create_node_configs(
    device_name: &str,
    env_dir: &Path,
    output_dir: Option<&str>,
    net_id: Option<&str>,
) -> Result<(), String> {
    let _ = output_dir;
    // Use existing user environment directory
    let root_dir = env_dir.to_path_buf();
    let builder = DevEnvBuilder::from_path(root_dir.clone());

    // Parse username
    let user_config_path = root_dir.join("user_config.json");
    let user_config_content = fs::read_to_string(&user_config_path)
        .map_err(|e| format!("Failed to read user_config.json: {}", e))?;
    let user_config: Value = serde_json::from_str(&user_config_content)
        .map_err(|e| format!("Failed to parse user_config.json: {}", e))?;
    let username = user_config
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("user_config.json missing name field")?;

    // Parse zone configuration
    let zone_config_file = root_dir.join("zone_config.json");
    let zone_config: ZoneConfig = serde_json::from_str(
            &fs::read_to_string(&zone_config_file)
                .map_err(|e| format!("Failed to read {:?}: {}", zone_config_file, e))?,
        )
        .map_err(|e| format!("Failed to parse {:?}: {}", zone_config_file, e))?;
    let zone_did = zone_config.id.clone();

    // Get user key pair
    let key_pair = TestKeys::get_key_pair_by_id(username)
        .or_else(|_| {
            println!("Warning: Key pair for user {} not found, using devtest key pair", username);
            TestKeys::get_key_pair_by_id("devtest")
        })?;

    let scope = builder.user_scope(username, zone_did.clone(), &key_pair);

    // Ensure user configuration exists
    if !scope.user_dir.join("user_config.json").exists() {
        return Err(format!("User configuration does not exist: {}", scope.user_dir.join("user_config.json").display()));
    }

    scope.create_node_config(device_name, net_id.map(|s| s.to_string()));

    println!("Successfully created node configuration: {}.{}", username, device_name);
    Ok(())
}

/// Create SN configuration (command line interface)
pub async fn cmd_create_sn_configs(output_dir: Option<&str>,sn_ip:IpAddr,sn_base_host:String) -> Result<(), String> {
    let root_dir = if let Some(dir) = output_dir {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    };

    let builder = DevEnvBuilder::from_path(root_dir);
    create_sn_config(&builder,sn_ip,sn_base_host).await;
    
    println!("Successfully created SN configuration");
    Ok(())
}

pub fn create_applist() -> Result<HashMap<String,LocalAppInstanceConfig>, String> {
    let mut app_list = HashMap::new();

    let cyfs_gateway_doc_json = json!({
        "pkg_name": "cyfs-gateway",
        "show_name": "CYFS Gateway",
        "app_icon_url": "https://cyfs-gateway.buckyos.ai/meta/icon.png",
        "selector_type": "single",
        "version": "0.5.1",
        "author": "buckyos.ai",
        "owner": "did:web:buckyos.ai",
        "description": {
            "detail": "CYFS Gateway Service"
        },
        "category": "sys_module,local_app",
        "pub_time": buckyos_kit::buckyos_get_unix_timestamp(),
        "exp": buckyos_kit::buckyos_get_unix_timestamp() + 3600 * 24 * 30,
        "pkg_list": {
            "amd64_win_app": {
                "pkg_id": "nightly-windows-amd64.cyfs-gateway#0.5.1",
            },
            "aarch64_apple_app": {
                "pkg_id": "nightly-apple-aarch64.cyfs-gateway#0.5.1"
            }
        }
    });

    let cyfs_gateway_doc : AppDoc = serde_json::from_value(cyfs_gateway_doc_json).unwrap();

    let cyfs_gateway_cfg = LocalAppInstanceConfig {
        target_state: ServiceInstanceState::Started,
        enable: true,
        app_doc: cyfs_gateway_doc,
        user_id: "devtest".to_string(),
        install_config: ServiceInstallConfig::default(),
    };
    app_list.insert("cyfs-gateway".to_string(), cyfs_gateway_cfg);  

    Ok(app_list)
}

pub fn cmd_create_applist(output_dir: Option<&str>) -> Result<(), String> {
    let root_dir = if let Some(dir) = output_dir {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    };
    
    let app_list = create_applist()?;
    write_json(&root_dir.join("applist.json"), &app_list);
    Ok(())
}

pub async fn cmd_register_user_to_sn(
    username: &str,
    sn_db_path: &str,
    output_dir: Option<&str>,
) -> Result<(), String> {
    // Use existing user environment directory
    let root_dir = if let Some(dir) = output_dir {
        PathBuf::from(dir)
    } else {
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?
    };

    let builder = DevEnvBuilder::from_path(root_dir.clone());
    register_user_to_sn(&builder, username, sn_db_path).await?;

    Ok(())
}

/// Register device to SN database (command line interface)
pub async fn cmd_register_device_to_sn(
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
    
    Ok(())
}

// ============================================================================
// Main Entry Functions
// ============================================================================

/// Create complete test environment configuration
#[allow(dead_code)]
pub async fn create_test_env_configs() {
    let builder = DevEnvBuilder::new("buckyos_dev_configs");

    // Set known web3 bridge configuration (if global configuration exists)
    // let mut test_web3_bridge = HashMap::new();
    // test_web3_bridge.insert("bns".to_string(), "web3.buckyos.io".to_string());
    // KNOWN_WEB3_BRIDGE_CONFIG.set(test_web3_bridge.clone());

    // 1. Create devtest user environment
    let devtest_keys = TestKeys::devtest_owner();
    let devtest_scope = builder.user_scope("devtest", DID::new("bns", "devtest"), &devtest_keys);
    devtest_scope.create_owner_config();

    // devtest node1
    let _node1_keys = TestKeys::devtest_node1();
    devtest_scope.create_node_config(
        "node1",
        Some("lan1".to_string()),
    );

    // 2. Create bob user environment
    let bob_keys = TestKeys::bob_owner();
    let bob_scope = builder.user_scope("bobdev", DID::new("bns", "bobdev"), &bob_keys);
    bob_scope.create_owner_config();

    // Bob Zone
    let _bob_zone_jwt = bob_scope.create_zone_boot_config_jwt(
        Some("sn.buckyos.io".to_string()),
        "ood1".to_string().parse().unwrap(),
        2980,
    );

    // Bob OOD1
    bob_scope.create_node_config("ood1", Some("lan2".to_string()));

    // 3. Create SN configuration
    create_sn_config(&builder, "127.0.0.1".parse().unwrap(), "buckyos.io".to_string()).await;

    // 4. Initialize SN database (if SnDB is available)
    // let sn_db_path = builder.root_dir().join("sn_db.sqlite3");
    // if sn_db_path.exists() {
    //     std::fs::remove_file(&sn_db_path).unwrap();
    // }
    // 
    // let db = SnDB::new_by_path(sn_db_path.to_str().unwrap()).unwrap();
    // db.initialize_database();
    // 
    // // Insert activation codes
    // for code in &["test-active-sn-code-bob", "11111", "22222", "33333", "44444", "55555"] {
    //     db.insert_activation_code(code).unwrap();
    // }
    // 
    // // Register user
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
    // // Register device
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

    /// Create DevEnvBuilder uniformly for tests, root directory distinguished by test name
    fn new_test_builder(test_name: &str) -> DevEnvBuilder {
        let root_dir = format!(".buckycli_{}", test_name);
        DevEnvBuilder::new(&root_dir)
    }
    
    #[tokio::test]
    pub async fn test_verify_all_key_pairs() {
        TestKeys::verify_all_key_pairs().unwrap();
    }

    #[tokio::test]
    /// Create simple test configuration (for test_all_dev_env_configs)
    pub async fn test_all_dev_env_configs() {
        let builder = new_test_builder("all_dev_env_configs");
        let owner_keys = TestKeys::get_key_pair_by_id("devtest").unwrap();
        let device_keys = TestKeys::get_key_pair_by_id("devtest.ood1").unwrap();

        let scope = builder.user_scope("devtest", DID::new("bns", "devtest"), &owner_keys);
        scope.create_owner_config();

        // Create device configuration
        let device_jwk = get_jwk(&device_keys.public_key_x);
        let mut device_config = DeviceConfig::new_by_jwk("ood1", device_jwk.clone());
        device_config.support_container = true;
        device_config.iss = "did:bns:devtest".to_string();

        let owner_key = get_encoding_key(owner_keys.private_key_pem.as_str());
        let device_jwt = device_config.encode(Some(&owner_key)).unwrap();
        let device_mini_doc = DeviceMiniConfig::new_by_device_config(&device_config);
        let device_mini_doc_jwt = device_mini_doc.to_jwt(&owner_key).unwrap();
        // Create Zone configuration
        let zone_did = DID::new("web", "test.buckyos.io");
        let ood: OODDescriptionString = "ood1".to_string().parse().unwrap();
        let zone_txt_record = scope.create_zone_boot_config_jwt(None, ood, 2980);

        // Create node identity configuration
        let node_identity_config = NodeIdentityConfig {
            zone_did: zone_did.clone(),
            owner_public_key: get_jwk(&owner_keys.public_key_x),
            owner_did: DID::new("bns", "devtest"),
            device_doc_jwt: device_jwt.to_string(),
            device_mini_doc_jwt: device_mini_doc_jwt.to_string(),
            zone_iat: builder.now() as u32,
        };
        write_json(
            &builder.root_dir().join("node_identity.json"),
            &node_identity_config,
        );

        // Create startup configuration
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

        // Output DNS records
        println!("# test.buckyos.io TXT Record: BOOT={};", zone_txt_record.boot_config_jwt);
        if let Ok(owner_x) = get_x_from_jwk(&get_jwk(&owner_keys.public_key_x)) {
            println!("# test.buckyos.io TXT Record: PKX={};", owner_x);
        }
    }

    // ============================================================================
    // Unit Tests
    // ============================================================================

    #[test]
    fn test_zone_boot_config() {
        // Use DevEnvBuilder + create_zone_boot_config_jwt to construct ZoneBootConfig JWT
        let builder = new_test_builder("zone_boot_config");
        let owner_keys = TestKeys::devtest_owner();
        let zone_did = DID::new("bns", "devtest");
        let scope = builder.user_scope("devtest", zone_did.clone(), &owner_keys);

        // Construct a simple OOD description and SN hostname
        let ood: OODDescriptionString = "ood1".to_string().parse().unwrap();
        let sn_host = Some("sn.buckyos.io".to_string());

        // Generate JWT through create_zone_boot_config_jwt
        let zone_txt_record = scope.create_zone_boot_config_jwt(sn_host.clone(), ood.clone(), 2980);

        // Decode JWT using owner public key
        let public_key_jwk = get_jwk(&owner_keys.public_key_x);
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        // Wrap JWT string as EncodedDocument, then decode
        let encoded_doc = EncodedDocument::from_str(zone_txt_record.boot_config_jwt.clone()).unwrap();
        let zone_boot_config_decoded =
            ZoneBootConfig::decode(&encoded_doc, Some(&public_key)).unwrap();
        println!("zone_boot_config_decoded: {:?}", zone_boot_config_decoded);

        // Construct expected ZoneBootConfig, consistent with logic in create_zone_boot_config_jwt
        let expected_zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec![ood],
            sn: sn_host,
            exp: builder.exp(),
            devices: HashMap::new(),
            owner: None,
            owner_key: None,
            extra_info: HashMap::new(),
        };

        assert_eq!(expected_zone_boot_config, zone_boot_config_decoded);
    }


    #[tokio::test]
    async fn test_create_formula_sn_config() {
        create_formula_sn_config().await;
    }

    #[tokio::test]
    async fn test_device_config() {
        // Use TestKeys + helper functions to construct owner key
        let owner_keys = TestKeys::devtest_owner();
        let public_key_jwk = get_jwk(&owner_keys.public_key_x);
        let owner_private_key = get_encoding_key(owner_keys.private_key_pem.as_str());
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        // OOD1 device
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

        // Note: DeviceInfo needs to be imported from buckyos_api
        // let mut device_info_ood = DeviceInfo::from_device_doc(&decoded);
        // device_info_ood.auto_fill_by_system_info().await.unwrap();
        // let device_info_str = serde_json::to_string(&device_info_ood).unwrap();
        // println!("ood device_info: {}", device_info_str);

        assert_eq!(device_config, decoded);
        assert_eq!(encoded, token2);

        // Gateway device
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

        // Server device
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
        // Use TestKeys + helper functions to construct keys required for owner configuration
        let owner_keys = TestKeys::devtest_owner();
        let public_key_jwk = get_jwk(&owner_keys.public_key_x);
        let private_key = get_encoding_key(owner_keys.private_key_pem.as_str());
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

    #[test]
    fn test_create_applist() {
        let app_list = create_applist().unwrap();
        let app_list_json = serde_json::to_string_pretty(&app_list).unwrap();
        println!("app_list:\n{}", app_list_json);
    }
}