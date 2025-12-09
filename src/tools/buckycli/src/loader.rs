//HOW to use buckycli.loader to test app/kernel service
//1. update boot.template.toml to add g,app_id,kernel.
//2. start buckyos,and wait it ready.
//3. use buckycli.loader to load app/kernel service for debug，app/kernel service会像被node_daemon启动一样正常启动
//P.S buckycli.loader 只适合“不被其它服务依赖的服务”，如果你的服务要被其它服务依赖，还是需要用标准的node_daemon启动方式

use buckyos_api::{get_buckyos_api_runtime, get_session_token_env_key, VERIFY_HUB_TOKEN_EXPIRE_TIME};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use tokio::process::Command;

pub async fn load_app_service(app_id: &str, app_service_path: &str) -> Result<(), String> {
    let timestamp = buckyos_get_unix_timestamp();
    let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
    let device_doc = runtime
        .device_config
        .as_ref()
        .ok_or_else(|| "device_config not found".to_string())?;
    let device_private_key = runtime
        .device_private_key
        .as_ref()
        .ok_or_else(|| "device_private_key not found".to_string())?;

    let device_session_token = kRPC::RPCSessionToken {
        token_type: kRPC::RPCSessionTokenType::JWT,
        nonce: Some(timestamp),
        session: None,
        userid: Some(device_doc.name.clone()),
        appid: Some(app_id.to_string()),
        exp: Some(timestamp + VERIFY_HUB_TOKEN_EXPIRE_TIME * 2),
        iss: Some(device_doc.name.clone()),
        token: None,
    };

    let device_session_token_jwt = device_session_token
        .generate_jwt(Some(device_doc.name.clone()), device_private_key)
        .map_err(|err| {
            error!("generate session token for {} failed! {}", app_id, err);
            err.to_string()
        })?;

    let env_key = get_session_token_env_key(app_id, false);


    println!(
        "MAKE SURE {} already in system/rbac/base_policy like g,{},kernel. (You can add it in boot.template.toml)",
        app_id, app_id
    );

    // 启动 app/kernel service 进程
    let mut command = Command::new(app_service_path);
    command.env(env_key.as_str(), device_session_token_jwt);
    let mut child = command.spawn().map_err(|err| {
        error!("start app/kernel service failed! {}", err);
        err.to_string()
    })?;

    child.wait().await.map_err(|err| {
        error!("wait app/kernel service failed! {}", err);
        err.to_string()
    })?;

    Ok(())
}