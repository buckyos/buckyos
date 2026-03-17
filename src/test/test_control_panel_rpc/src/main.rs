use ::kRPC::*;
use buckyos_kit::*;
use name_lib::*;
use serde_json::{json, Value};

fn generate_session_token() -> std::result::Result<String, String> {
    let etc_dir = get_buckyos_system_etc_dir();
    let bucky_cli_dir = etc_dir.join(".buckycli");
    println!("buckycli dir {:?}", bucky_cli_dir);

    if !bucky_cli_dir.exists() {
        return Err("bucky_cli_dir not exists".to_string());
    }

    let user_config_file = bucky_cli_dir.join("user_config.json");
    let user_private_key_file = bucky_cli_dir.join("user_private_key.pem");

    if !user_config_file.exists() {
        return Err("user config file not exists".to_string());
    }
    if !user_private_key_file.exists() {
        return Err("user private key file not exists".to_string());
    }

    let owner_config =
        OwnerConfig::load_owner_config(&user_config_file).map_err(|err| err.to_string())?;
    let private_key = load_private_key(&user_private_key_file).map_err(|err| err.to_string())?;
    println!("owner user name: {:?}", owner_config.name);

    let (session_token, _real_session_token) =
        RPCSessionToken::generate_jwt_token(&owner_config.name, "control-panel", None, &private_key)
            .map_err(|err| err.to_string())?;

    Ok(session_token)
}

fn expect_string_field(payload: &Value, key: &str) -> std::result::Result<String, String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing or empty `{}` in payload: {}", key, payload))
}

async fn test() -> std::result::Result<(), String> {
    let session_token = generate_session_token()?;
    let client = kRPC::new(
        "http://127.0.0.1:4020/kapi/control-panel",
        Some(session_token),
    );

    println!("==> test ui.main");
    let main_payload = client
        .call("ui.main", json!({}))
        .await
        .map_err(|err| err.to_string())?;
    let marker = expect_string_field(&main_payload, "test")?;
    if marker != "test" {
        return Err(format!("unexpected ui.main payload: {}", main_payload));
    }
    println!("<== test ui.main, pass");

    println!("==> test system.overview");
    let overview_payload = client
        .call("system.overview", json!({}))
        .await
        .map_err(|err| err.to_string())?;
    let _name = expect_string_field(&overview_payload, "name")?;
    let _os = expect_string_field(&overview_payload, "os")?;
    overview_payload
        .get("uptime_seconds")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| format!("missing `uptime_seconds` in payload: {}", overview_payload))?;
    println!("<== test system.overview, pass");

    println!("==> test apps.list");
    let apps_payload = client
        .call("apps.list", json!({ "key": "services" }))
        .await
        .map_err(|err| err.to_string())?;
    let key = expect_string_field(&apps_payload, "key")?;
    if key != "services" {
        return Err(format!("unexpected apps.list key: {}", apps_payload));
    }
    apps_payload
        .get("items")
        .and_then(|value| value.as_array())
        .ok_or_else(|| format!("missing `items` array in payload: {}", apps_payload))?;
    println!("<== test apps.list, pass");

    Ok(())
}

#[tokio::main]
async fn main() {
    let result = test().await;
    if let Err(err) = result {
        println!("test failed: {}", err);
        std::process::exit(1);
    }
    println!("test success");
}
