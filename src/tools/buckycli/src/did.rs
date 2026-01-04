use buckyos_kit::get_buckyos_system_etc_dir;
use clap::ArgMatches;
use jsonwebtoken::EncodingKey;
use name_lib::{
    DID, DIDDocumentTrait, DeviceConfig, DeviceMiniConfig, NodeIdentityConfig, OwnerConfig, ZoneBootConfig, generate_ed25519_key_pair
};
use ndn_lib::named_obj_to_jwt;
use std::fs::File;
use std::io::Write;

pub(crate) async fn sign_json_data(
    matches: &ArgMatches,
    private_key: Option<(String, EncodingKey)>,
) {
    let json = matches.get_one::<String>("json").unwrap();
    println!("data: {} ", json);
    let _ = serde_json::from_str::<serde_json::Value>(json);
    let json = serde_json::to_value(&json)
        .map_err(|e| {
            println!("serde_json::to_value error {}", e);
            e
        })
        .unwrap();

    // private_key comes from user_private_key.pem file, which may be empty
    if let Some((kid, private_key)) = private_key {
        // check json data valid
        let result = named_obj_to_jwt(&json, &private_key, Some(kid.to_string()))
            .map_err(|e| {
                println!("named_obj_to_jwt error {}", e);
                e
            })
            .unwrap();
        println!("named_obj_to_jwt {}", result);
    } else {
        // No user_private_key.pem file, read from start config
        println!("empty user_private_key.pem file!");
        let start_params_file_path = get_buckyos_system_etc_dir().join("start_config.json");
        let start_params_str = tokio::fs::read_to_string(start_params_file_path)
            .await
            .unwrap();
        let start_params: serde_json::Value = serde_json::from_str(&start_params_str).unwrap();
        let user_private_key = start_params["private_key"].as_str().unwrap();
        let user_private_key = user_private_key.trim();
        println!("user_private_key: {}", user_private_key);

        let private_key = EncodingKey::from_ed_pem(user_private_key.as_bytes())
            .map_err(|e| {
                println!("EncodingKey::from_ed_pem error {}", e);
                e
            })
            .unwrap();
        let result = named_obj_to_jwt(&json, &private_key, Some("ood".to_string()))
            .map_err(|e| {
                println!("named_obj_to_jwt error {}", e);
                e
            })
            .unwrap();
        println!("named_obj_to_jwt: {}", result);
    }
}

pub(crate) fn did_matches(matches: &ArgMatches) {
    if matches.subcommand().is_some() {
        let did_command = matches.subcommand().unwrap();
        match did_command {
            ("genkey", _) => {
                println!("genkey");
                return did_genkey();
            }
            _ => {}
        }
    }
    if let Some(file) = matches.get_one::<String>("open") {
        return did_open_file(file);
    }

    // Create a userconfig
    if let Some(_value) = matches.get_one::<String>("create_user") {
        let values: Vec<&String> = matches
            .get_many::<String>("create_user")
            .expect("missing name and jwt ")
            .collect();
        let name = values[0];
        let owner_jwk = values[1];
        return did_create_user_config(name, owner_jwk);
    }

    // Create a deviceconfig
    if let Some(_value) = matches.get_one::<String>("create_device") {
        let values: Vec<&String> = matches
            .get_many::<String>("create_device")
            .expect("missing name and jwt ")
            .collect();
        let user_name = values[0];
        let zone_name = values[1];
        let owner_jwk = values[2];
        let user_private_key = values[3];
        return did_create_device_config(user_name, zone_name, owner_jwk, user_private_key);
    }

    if let Some(_value) = matches.get_one::<String>("create_zoneboot") {
        let values: Vec<&String> = matches
            .get_many::<String>("create_zoneboot")
            .expect("missing oods and sn_host ")
            .collect();
        let oods = values[0];
        let sn_host = Some(values[1].to_string());
        let oods: Vec<&str> = oods.split(",").collect();
        let mut oods_vec = Vec::new();
        for ood in oods {
            oods_vec.push(ood.to_string());
        }
        return did_create_zoneboot(oods_vec, sn_host);
    }


    println!("no mathch arg")
}

fn did_genkey() {
    let (private_key, public_key) = generate_ed25519_key_pair();
    // println!("{} \n\n\n {}", private_key, public_key);
    let mut private_key_file = File::create("user_private_key.pem").unwrap();
    private_key_file.write_all(private_key.as_bytes()).unwrap();
    let mut public_key_file = File::create("user_public_key.pem").unwrap();

    public_key_file
        .write_all(public_key.to_string().as_bytes())
        .unwrap();
    println!("genkey OK! user_private_key.pem and user_public_key.pem in current dir");
}

fn did_open_file(file: &str) {
    let file_content = std::fs::read_to_string(file).unwrap();
    let json = serde_json::from_str::<serde_json::Value>(&file_content);
    let toml = toml::from_str::<serde_json::Value>(&file_content);
    if toml.is_ok() || json.is_ok() {
        println!("{}", file_content);
    } else {
        println!("not json or toml");
    }
}

//  buckycli did --create_user aa '{"crv":"Ed25519","kty":"OKP","x":"14pk3c3XO9_xro5S6vSr_Tvq5eTXbFY8Mop-Vj1D0z8"}'
// Create user configuration file
fn did_create_user_config(name: &str, owner_jwk: &str) {
    // Create DID identifier based on username
    let user_did = DID::new("bns", name);
    let user_name = name.to_string();

    // Parse json string into serde_json::Value
    let owner_jwk = serde_json::from_str::<serde_json::Value>(owner_jwk).unwrap();
    let owner_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(owner_jwk.clone()).unwrap();

    // Create user configuration object
    let owner_config = OwnerConfig::new(
        user_did.clone(),
        user_name.clone(),
        user_name.clone(),
        owner_jwk.clone(),
    );
    // Serialize configuration object to JSON string
    let owner_config_json_str = serde_json::to_string_pretty(&owner_config).unwrap();
    // Write to configuration file
    let mut owner_config_file = std::fs::File::create("user_config.json").unwrap();
    owner_config_file
        .write_all(owner_config_json_str.as_bytes())
        .unwrap();
    println!("create user OK! user_config.json in current dir");
}

// Create device configuration file
fn did_create_device_config(
    user_name: &str,
    zone_name: &str,
    owner_jwk: &str,
    user_private_key: &str,
) {
    // Create DID identifiers for zone and user
    let zone_did = DID::new("bns", zone_name);
    let user_did = DID::new("bns", user_name);
    // Parse json string into serde_json::Value
    let owner_jwk = serde_json::from_str::<serde_json::Value>(owner_jwk).unwrap();
    let owner_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(owner_jwk.clone()).unwrap();

    // Get current timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let device_name = "ood1";

    // Create device public/private key pair
    let (privete_key, public_key) = generate_ed25519_key_pair();
    // Save device private key
    let private_file_name = format!("device_{}_private_key.pem", device_name);
    let mut device_private_key_file = File::create(private_file_name.clone()).unwrap();
    device_private_key_file
        .write_all(privete_key.as_bytes())
        .unwrap();
    // Save device public key
    let public_file_name = format!("device_{}_public_key.pem", device_name);
    let mut device_public_key_file = File::create(public_file_name.clone()).unwrap();
    device_public_key_file
        .write_all(public_key.to_string().as_bytes())
        .unwrap();

    // Create device configuration
    let device_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(public_key.clone()).unwrap();
    let device_config = DeviceConfig::new_by_jwk(device_name, device_jwk);

    // Read user private key and create encoding key
    let user_private_key = std::fs::read_to_string(user_private_key).unwrap();
    let user_private_key = user_private_key.trim();
    println!("user_private_key: {}", user_private_key);
    let encode_key = EncodingKey::from_ed_pem(user_private_key.as_bytes()).unwrap();
    let device_jwt = device_config.encode(Some(&encode_key)).unwrap();
    let device_mini_config = DeviceMiniConfig::new_by_device_config(&device_config);
    let device_mini_doc_jwt = device_mini_config.to_jwt(&encode_key).unwrap();
    // Create node identity configuration
    let node_identity_config = NodeIdentityConfig {
        zone_did: zone_did,
        owner_public_key: owner_jwk.clone(),
        owner_did: user_did,
        device_doc_jwt: device_jwt.to_string(),
        device_mini_doc_jwt: device_mini_doc_jwt.to_string(),
        zone_iat: now as u32,
    };
    // Serialize and save node identity configuration
    let node_identity_config_json_str =
        serde_json::to_string_pretty(&node_identity_config).unwrap();
    let mut node_identity_file = std::fs::File::create("node_identity.json").unwrap();
    node_identity_file
        .write_all(node_identity_config_json_str.as_bytes())
        .unwrap();
    println!(
        "create OK! Generate {}, {},  node_identity.json files in current dir",
        private_file_name, public_file_name
    );
}

fn did_create_zoneboot(oods: Vec<String>, sn_host: Option<String>) {
    // Get current timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let exp = now + 3600 * 24 * 365 * 10;

    let zone_boot_config = ZoneBootConfig {
        id: None,
        oods: oods.into_iter().map(|ood| ood.parse().unwrap()).collect(),
        sn: sn_host,
        exp,
        owner: None,
        owner_key: None,
        extra_info: std::collections::HashMap::new(),

    };
    let zone_boot_config_json_str = serde_json::to_string_pretty(&zone_boot_config).unwrap();

    let mut zone_boot_config_file = std::fs::File::create("zone.json").unwrap();
    zone_boot_config_file
        .write_all(zone_boot_config_json_str.as_bytes())
        .unwrap();
    println!("zone boot config created!");
}
