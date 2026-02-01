use ::kRPC::*;
use async_trait::async_trait;
use log::*;
use name_lib::DeviceInfo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;
use std::result::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnBindZoneConfigReq {
    pub zone_config: String,
    pub user_name: String,
    pub user_domain: Option<String>,
}

impl SnBindZoneConfigReq {
    pub fn new(zone_config: String, user_name: String, user_domain: Option<String>) -> Self {
        Self {
            zone_config,
            user_name,
            user_domain,
        }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse SnBindZoneConfigReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnUpdateDeviceInfoReq {
    pub owner_id: String,
    pub device_id: String,
    pub device_info: DeviceInfo,
}

impl SnUpdateDeviceInfoReq {
    pub fn new(owner_id: String, device_id: String, device_info: DeviceInfo) -> Self {
        Self {
            owner_id,
            device_id,
            device_info,
        }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse SnUpdateDeviceInfoReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnGetDeviceInfoReq {
    pub owner_id: String,
    pub device_id: String,
}

impl SnGetDeviceInfoReq {
    pub fn new(owner_id: String, device_id: String) -> Self {
        Self {
            owner_id,
            device_id,
        }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse SnGetDeviceInfoReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnSetUserDidDocumentReq {
    pub owner_user: String,
    pub obj_name: String,
    pub did_document: Value,
    pub doc_type: String,
}

impl SnSetUserDidDocumentReq {
    pub fn new(
        owner_user: String,
        obj_name: String,
        did_document: Value,
        doc_type: String,
    ) -> Self {
        Self {
            owner_user,
            obj_name,
            did_document,
            doc_type,
        }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse SnSetUserDidDocumentReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnRegisterDeviceReq {
    pub user_name: String,
    pub device_name: String,
    pub device_did: String,
    pub device_ip: String,
    pub device_info: String,
    pub mini_config_jwt: String,
}

impl SnRegisterDeviceReq {
    pub fn new(
        user_name: String,
        device_name: String,
        device_did: String,
        device_ip: String,
        device_info: String,
        mini_config_jwt: String,
    ) -> Self {
        Self {
            user_name,
            device_name,
            device_did,
            device_ip,
            device_info,
            mini_config_jwt,
        }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse SnRegisterDeviceReq: {}", e))
        })
    }
}

pub enum SnClient {
    InProcess(Box<dyn SnHandler>),
    KRPC(Box<kRPC>),
}

impl SnClient {
    pub fn new_in_process(handler: Box<dyn SnHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(client: Box<kRPC>) -> Self {
        Self::KRPC(client)
    }

    pub async fn bind_zone_config(
        &self,
        zone_config_jwt: &str,
        user_name: &str,
        user_domain: Option<String>,
    ) -> Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_bind_zone_config(zone_config_jwt, user_name, user_domain)
                    .await
            }
            Self::KRPC(client) => {
                let req = SnBindZoneConfigReq::new(
                    zone_config_jwt.to_string(),
                    user_name.to_string(),
                    user_domain,
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize SnBindZoneConfigReq: {}",
                        e
                    ))
                })?;
                client.call("bind_zone_config", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn update_device_info(
        &self,
        owner_id: &str,
        device_id: &str,
        device_info: &DeviceInfo,
    ) -> Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_update_device_info(owner_id, device_id, device_info)
                    .await
            }
            Self::KRPC(client) => {
                let req = SnUpdateDeviceInfoReq::new(
                    owner_id.to_string(),
                    device_id.to_string(),
                    device_info.clone(),
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize SnUpdateDeviceInfoReq: {}",
                        e
                    ))
                })?;
                client.call("update", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn get_device_info(
        &self,
        owner_id: &str,
        device_id: &str,
    ) -> Result<DeviceInfo, RPCErrors> {
        match self {
            Self::InProcess(handler) => handler.handle_get_device_info(owner_id, device_id).await,
            Self::KRPC(client) => {
                let req = SnGetDeviceInfoReq::new(owner_id.to_string(), device_id.to_string());
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize SnGetDeviceInfoReq: {}", e))
                })?;
                let result = client.call("get", req_json).await?;
                serde_json::from_value(result).map_err(|e| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to deserialize DeviceInfo response: {}",
                        e
                    ))
                })
            }
        }
    }

    pub async fn register_device(
        &self,
        user_name: &str,
        device_name: &str,
        device_did: &str,
        device_ip: &str,
        device_info: &str,
        mini_config_jwt: &str,
    ) -> Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_register_device(
                        user_name,
                        device_name,
                        device_did,
                        device_ip,
                        device_info,
                        mini_config_jwt,
                    )
                    .await
            }
            Self::KRPC(client) => {
                let req = SnRegisterDeviceReq::new(
                    user_name.to_string(),
                    device_name.to_string(),
                    device_did.to_string(),
                    device_ip.to_string(),
                    device_info.to_string(),
                    mini_config_jwt.to_string(),
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize SnRegisterDeviceReq: {}",
                        e
                    ))
                })?;
                client.call("register", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn set_user_did_document(
        &self,
        owner_user: &str,
        obj_name: &str,
        did_document: &Value,
        doc_type: &str,
    ) -> Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                handler
                    .handle_set_user_did_document(owner_user, obj_name, did_document, doc_type)
                    .await
            }
            Self::KRPC(client) => {
                let req = SnSetUserDidDocumentReq::new(
                    owner_user.to_string(),
                    obj_name.to_string(),
                    did_document.clone(),
                    doc_type.to_string(),
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize SnSetUserDidDocumentReq: {}",
                        e
                    ))
                })?;
                client.call("set_user_did_document", req_json).await?;
                Ok(())
            }
        }
    }
}

#[async_trait]
pub trait SnHandler: Send + Sync {
    async fn handle_bind_zone_config(
        &self,
        zone_config_jwt: &str,
        user_name: &str,
        user_domain: Option<String>,
    ) -> Result<(), RPCErrors>;

    async fn handle_update_device_info(
        &self,
        owner_id: &str,
        device_id: &str,
        device_info: &DeviceInfo,
    ) -> Result<(), RPCErrors>;

    async fn handle_get_device_info(
        &self,
        owner_id: &str,
        device_id: &str,
    ) -> Result<DeviceInfo, RPCErrors>;

    async fn handle_register_device(
        &self,
        user_name: &str,
        device_name: &str,
        device_did: &str,
        device_ip: &str,
        device_info: &str,
        mini_config_jwt: &str,
    ) -> Result<(), RPCErrors>;

    async fn handle_set_user_did_document(
        &self,
        owner_user: &str,
        obj_name: &str,
        did_document: &Value,
        doc_type: &str,
    ) -> Result<(), RPCErrors>;
}

pub struct SnServerHandler<T: SnHandler>(pub T);

impl<T: SnHandler> SnServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: SnHandler> RPCHandler for SnServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();

        let result = match req.method.as_str() {
            "bind_zone_config" => {
                let bind_req = SnBindZoneConfigReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_bind_zone_config(
                        &bind_req.zone_config,
                        &bind_req.user_name,
                        bind_req.user_domain,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "update" => {
                let update_req = SnUpdateDeviceInfoReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_device_info(
                        &update_req.owner_id,
                        &update_req.device_id,
                        &update_req.device_info,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "get" => {
                let get_req = SnGetDeviceInfoReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_device_info(&get_req.owner_id, &get_req.device_id)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "register" => {
                let register_req = SnRegisterDeviceReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_register_device(
                        &register_req.user_name,
                        &register_req.device_name,
                        &register_req.device_did,
                        &register_req.device_ip,
                        &register_req.device_info,
                        &register_req.mini_config_jwt,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "set_user_did_document" => {
                let set_req = SnSetUserDidDocumentReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_set_user_did_document(
                        &set_req.owner_user,
                        &set_req.obj_name,
                        &set_req.did_document,
                        &set_req.doc_type,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

pub async fn sn_bind_zone_config(
    sn_url: &str,
    session_token: Option<String>,
    username: &str,
    zone_config_jwt: &str,
    user_domain: Option<String>,
) -> Result<(), RPCErrors> {
    let client = SnClient::new_krpc(Box::new(kRPC::new(sn_url, session_token)));

    let real_username = username.to_lowercase();
    let req = SnBindZoneConfigReq::new(zone_config_jwt.to_string(), real_username, user_domain);

    info!(
        "bind zone config to sn for {} {}",
        username, zone_config_jwt
    );
    client
        .bind_zone_config(&req.zone_config, &req.user_name, req.user_domain)
        .await?;
    info!("bind zone config to sn for {} success", username);
    Ok(())
}

pub async fn sn_update_device_info(
    sn_url: &str,
    session_token: Option<String>,
    owner_id: &str,
    device_id: &str,
    device_info: &DeviceInfo,
) -> Result<(), RPCErrors> {
    let client = SnClient::new_krpc(Box::new(kRPC::new(sn_url, session_token)));

    info!(
        "update device info to sn {} for {}_{}",
        sn_url, owner_id, device_id
    );
    client
        .update_device_info(owner_id, device_id, device_info)
        .await?;
    Ok(())
}

pub async fn sn_get_device_info(
    sn_url: &str,
    session_token: Option<String>,
    owner_id: &str,
    device_id: &str,
) -> Result<DeviceInfo, RPCErrors> {
    let client = SnClient::new_krpc(Box::new(kRPC::new(sn_url, session_token)));

    //TODO: result must be DeviceConfig@JWT?
    let device_info = client.get_device_info(owner_id, device_id).await?;
    Ok(device_info)
}

pub async fn sn_register_device(
    sn_url: &str,
    session_token: Option<String>,
    username: &str,
    device_name: &str,
    device_did: &str,
    device_ip: &str,
    device_info: &str,
    mini_config_jwt: &str,
) -> Result<(), RPCErrors> {
    let client = SnClient::new_krpc(Box::new(kRPC::new(sn_url, session_token)));

    client
        .register_device(
            username,
            device_name,
            device_did,
            device_ip,
            device_info,
            mini_config_jwt,
        )
        .await?;
    Ok(())
}

pub async fn sn_set_user_did_document(
    sn_url: &str,
    session_token: Option<String>,
    owner_user: &str,
    obj_name: &str,
    did_document: &Value,
    doc_type: &str,
) -> Result<(), RPCErrors> {
    let client = SnClient::new_krpc(Box::new(kRPC::new(sn_url, session_token)));

    client
        .set_user_did_document(owner_user, obj_name, did_document, doc_type)
        .await
}

pub async fn get_real_sn_host_name(
    sn: &str,
    device_id: &str,
) -> std::result::Result<String, RPCErrors> {
    // 尝试通过 HTTP GET 请求获取 https://$sn/config?device_id=$device_id
    let url = format!("https://{}/config?device_id={}", sn, device_id);
    let response = match reqwest::get(&url).await {
        Ok(resp) => resp,
        Err(e) => {
            warn!(
                "get sn host name from {} failed! {},use sn as host name",
                url, e
            );
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

    let sn_config: Value = sn_config.unwrap();

    let host_name = sn_config["host"].as_str().unwrap();
    warn!("get sn real host from {} success! => {}", url, host_name);
    Ok(host_name.to_string())
}
