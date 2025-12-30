use name_lib::DeviceInfo;
use ::kRPC::{RPCErrors,kRPC};
use serde_json::{json,Value};
use log::*;


pub async fn sn_bind_zone_config(sn_url: &str, session_token: Option<String>, username:&str,zone_config_jwt: &str,user_domain:Option<String>)->Result<(),RPCErrors> {
    let client : kRPC = kRPC::new(sn_url,session_token);

    let real_username = username.to_lowercase();
    let mut params = json!({
        "zone_config": zone_config_jwt,
        "user_name": real_username
    });

    if user_domain.is_some() {
        params["user_domain"] = user_domain.unwrap().into();
    }

    info!("bind zone config to sn for {} {}",username,zone_config_jwt);

    let _result = client.call("bind_zone_config", params).await?;

    info!("bind zone config to sn for {} success",username);
    Ok(())
}

pub async fn sn_update_device_info(sn_url: &str, session_token: Option<String>, 
    owner_id: &str, device_id: &str, device_info: &DeviceInfo,) -> Result<(),RPCErrors> 
{
    let client : kRPC = kRPC::new(sn_url,session_token);
    let device_info_json = serde_json::to_value(device_info).map_err(|e|{
        error!("Failed to serialize device info to json_value,device_id:{},owner_id:{},error:{:?}",device_id,owner_id,e);
        RPCErrors::ParseRequestError(e.to_string())
    })?;

    info!("update device info to sn {} for {}_{}",sn_url,owner_id,device_id);

    let _result = client.call("update", json!({
        "device_id": device_id, 
        "owner_id": owner_id, 
        "device_info": device_info_json})).await?;
    
    Ok(())
}

pub async fn sn_get_device_info(sn_url: &str, session_token: Option<String>, 
    owner_id: &str, device_id: &str) -> Result<DeviceInfo,RPCErrors> 
{
    let client : kRPC = kRPC::new(sn_url,session_token);
    let result = client.call("get", json!({
        "device_id": device_id,
        "owner_id": owner_id
    })).await?;

    //TODO: result must be DeviceConfig@JWT?
    let device_info: DeviceInfo = serde_json::from_value(result).map_err(|e|{
        error!("Failed to deserialize device info from json_value,device_id:{},owner_id:{},error:{:?}",device_id,owner_id,e);
        RPCErrors::ParserResponseError(e.to_string())
    })?;

    Ok(device_info)
}


pub async fn sn_register_device(sn_url: &str, session_token: Option<String>, 
    username:&str,device_name:&str,device_did:&str,device_ip:&str,device_info:&str,mini_config_jwt:&str) -> Result<(),RPCErrors> {
        let client : kRPC = kRPC::new(sn_url,session_token);
        let _result = client.call("register", json!({
            "user_name": username,
            "device_name": device_name,
            "device_did": device_did,
            "device_ip": device_ip,
            "device_info": device_info,
            "mini_config_jwt": mini_config_jwt
        })).await?;
        
        Ok(())
}

pub async fn get_real_sn_host_name(sn: &str,device_id: &str) -> std::result::Result<String,RPCErrors> {
    // 尝试通过 HTTP GET 请求获取 https://$sn/config?device_id=$device_id
    let url = format!("https://{}/config?device_id={}", sn, device_id);
    let response = match reqwest::get(&url).await {
        Ok(resp) => resp,
        Err(e) => {
            warn!("get sn host name from {} failed! {},use sn as host name", url, e);
            return Ok(sn.to_string());
        }
    };


    let body = match response.text().await {
        Ok(text) => text,
        Err(e) => {
            warn!("get sn host name failed! {}", e);
            return Ok(sn.to_string());
        }
    };

    let sn_config = serde_json::from_str(&body);
    if sn_config.is_err() {
        warn!("get sn host name failed! {}", sn_config.err().unwrap());
        return Ok(sn.to_string());
    }

    let sn_config:Value = sn_config.unwrap();

    let host_name = sn_config["host"].as_str().unwrap();
    warn!("get sn real host from {} success! => {}", url,host_name);
    Ok(host_name.to_string())

}