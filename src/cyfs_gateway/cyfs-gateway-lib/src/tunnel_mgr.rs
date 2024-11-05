
#![allow(unused)]

use crate::{DatagramServer, DatagramServerBox, RTcpStack, StreamListener, Tunnel, TunnelBox, TunnelBuilder, TunnelError, TunnelResult};
use serde_json::Value;
use url::Url;
use std::collections::HashMap;
use std::sync::{Arc};
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use log::*;
use name_lib::*;
use once_cell::sync::OnceCell;
lazy_static!{
    static ref RTCP_STACK_MAP:Arc<Mutex<HashMap<String, RTcpStack >>> = Arc::new(Mutex::new(HashMap::new()));
}

pub static CURRENT_DEVICE_RRIVATE_KEY: OnceCell<[u8;48]> = OnceCell::new();

#[derive(Debug,PartialEq, Eq)]
pub enum ProtocolCategory {
    Stream,
    Datagram,
    //Named Object
}

pub fn get_protocol_category(str_protocol:&str) -> TunnelResult<ProtocolCategory> {
    //lowercase
    let str_protocol = str_protocol.to_lowercase();
    match str_protocol.as_str() {
        "tcp" => Ok(ProtocolCategory::Stream),
        "rtcp" => Ok(ProtocolCategory::Stream),
        "udp" => Ok(ProtocolCategory::Datagram),
        _ => Err(TunnelError::UnknowProtocol(str_protocol)),
    }
}



pub async fn get_tunnel_builder_by_protocol(protocol:&str) -> TunnelResult<Box<dyn TunnelBuilder>> {
    match protocol {
        "tcp" => {
            return Ok(Box::new(crate::IPTunnelBuilder::new()))
        },
        "udp" => {
            return Ok(Box::new(crate::IPTunnelBuilder::new()))
        },
        "rtcp" => {
            let this_device_config = CURRENT_DEVICE_CONFIG.get();
            let this_device_private_key = CURRENT_DEVICE_RRIVATE_KEY.get();
            if this_device_config.is_none() || this_device_private_key.is_none() {
                return Err(TunnelError::BindError("CURRENT_DEVICE_CONFIG or CURRENT_DEVICE_PRIVATE_KEY not set".to_string()));
            }
            let this_device_config = this_device_config.unwrap();
            info!("RTCP stack will init by this_device_config: {:?}",this_device_config);
            let this_device_private_key = this_device_private_key.unwrap().clone();
            //info!("this_device_private_key: {:?}",this_device_private_key);
            let this_device_hostname:String;
            let this_device_did = DID::from_str(this_device_config.did.as_str());
            if this_device_did.is_none() {
                this_device_hostname = this_device_config.did.clone();
            } else {
                this_device_hostname = this_device_did.unwrap().to_host_name();
            }
            info!("this_device_hostname: {}",this_device_hostname);

            let mut rtcp_stack_map = RTCP_STACK_MAP.lock().await;
            let rtcp_stack = rtcp_stack_map.get(this_device_hostname.as_str());
            if rtcp_stack.is_some() {
                let result_builder = rtcp_stack.unwrap().to_owned();
                return Ok(Box::new(result_builder));
            }
            //let device_did = device_did.replace(":", ".");
            info!("create rtcp stack for {}",this_device_hostname.as_str());
            let mut result_rtcp_stack = crate::RTcpStack::new(this_device_hostname.clone(),2980,Some(this_device_private_key));
            result_rtcp_stack.start().await;
            rtcp_stack_map.insert(this_device_hostname.clone(),result_rtcp_stack.clone());
            return Ok(Box::new(result_rtcp_stack));
        }
        _ => return Err(TunnelError::UnknowProtocol(protocol.to_string()))
    }
}

lazy_static!{
    static ref TUNNEL_MAP:Arc<Mutex<HashMap<String,Box<dyn TunnelBox>>>> = {
        Arc::new(Mutex::new(HashMap::new()))
    };
}

pub async fn get_tunnel(target_url:&Url,enable_tunnel:Option<Vec<String>>) 
    -> TunnelResult<Box<dyn TunnelBox>> 
{

    info!("try create tunnel for {}", target_url);
    //url like tcp://deviceid 
    let builder = get_tunnel_builder_by_protocol(target_url.scheme()).await?;
    let tunnel = builder.create_tunnel(target_url).await?;

    info!("create tunnel for {} success,add to tunnel cache", target_url);
    return Ok(tunnel);
}


pub async fn create_listner_by_url(bind_url:&Url) -> TunnelResult<Box<dyn StreamListener>> {
    let builder = get_tunnel_builder_by_protocol(bind_url.scheme()).await?;
    let listener = builder.create_listener(bind_url).await?;
    return Ok(listener);
}

pub async fn create_datagram_server_by_url(bind_url:&Url) -> TunnelResult<Box<dyn DatagramServerBox>> {
    let builder = get_tunnel_builder_by_protocol(bind_url.scheme()).await?;
    let server = builder.create_datagram_server(bind_url).await?;
    return Ok(server);
}
