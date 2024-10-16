use name_lib::DeviceInfo;
use ::kRPC::{RPCErrors,kRPC};
use serde_json::{Value, json};
use log::*;

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

    let device_info: DeviceInfo = serde_json::from_value(result).map_err(|e|{
        error!("Failed to deserialize device info from json_value,device_id:{},owner_id:{},error:{:?}",device_id,owner_id,e);
        RPCErrors::ParserResponseError(e.to_string())
    })?;

    Ok(device_info)
}
