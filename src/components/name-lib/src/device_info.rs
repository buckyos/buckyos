use std::net::{IpAddr, Ipv6Addr};
use std::str::FromStr;
use tokio::net::UdpSocket;
use std::net::ToSocketAddrs;
use serde::{Serialize,Deserialize};
use serde_json::json;
use thiserror::Error;


use crate::config::DeviceConfig;
use crate::{NSResult,NSError};
use sysinfo::{Components, Disks, Networks, System};




// describe a device runtime info
#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct DeviceInfo {
    pub hostname:String,
    pub device_type:String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub did:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip:Option<IpAddr>,//main_ip from device's self knowledge
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_net_interface:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_id:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sys_hostname : Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_daemon_ver:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_os_info:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_info:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage:Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_mem:Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_usage:Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_space:Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_usage:Option<u64>,
}

impl DeviceInfo {
    pub fn from_device_doc(device_doc:&DeviceConfig) -> Self {
        let mut device_info = DeviceInfo::new(device_doc.name.as_str(),Some(device_doc.did.clone()));
        //device_info.did = Some(device_doc.did.clone());
        device_info.device_type = device_doc.device_type.clone();
        device_info.ip = device_doc.ip.clone();
        device_info.net_id = device_doc.net_id.clone();

        return device_info;
    }

    pub fn new(ood_string:&str,did:Option<String>) -> Self {
        //device_string format: hostname@[ip]#[netid]
        let ip :Option<IpAddr>;
        let net_id :Option<String>;
        let parts: Vec<&str> = ood_string.split('@').collect();
        let hostname = parts[0];

        if parts.len() > 1 {
            let ip_str = parts[1];
            let ip_result = IpAddr::from_str(ip_str);
            if ip_result.is_ok() {
                ip = Some(ip_result.unwrap());
            } else {
                ip = None;
            }
        } else {
            ip = None;
        }

        let parts: Vec<&str> = ood_string.split('#').collect();
        if parts.len() == 2{
            net_id = Some(parts[1].to_string());
        } else {
            net_id = None;
        }   

        DeviceInfo {
            hostname:hostname.to_string(),
            device_type:"ood".to_string(),
            ip:ip,
            did:did,
            main_net_interface:None,
            net_id:net_id,
            node_daemon_ver:None,
            base_os_info:None,
            cpu_info:None,
            cpu_usage:None,
            total_mem:None,
            mem_usage:None,
            total_space:None,
            disk_usage:None,
            sys_hostname:None,
        }
    }

    pub async fn auto_fill_by_system_info(&mut self) -> NSResult<()> {
        let mut sys = System::new_all();
        sys.refresh_all();

        let test_socket = UdpSocket::bind("0.0.0.0:0").await;
        if test_socket.is_ok(){
            let test_socket = test_socket.unwrap();
            test_socket.connect("8.8.8.8:80").await;
            let local_addr = test_socket.local_addr().unwrap();
            self.ip = Some(local_addr.ip());
        }

        // Get OS information
        self.base_os_info = Some(format!("{} {} {}",System::name().unwrap_or_default(), System::os_version().unwrap_or_default(), System::kernel_version().unwrap_or_default()));

        // Get CPU information
        if let Some(cpu) = sys.cpus().first() {
            self.cpu_info = Some(format!("{} @ {} MHz", cpu.brand(), cpu.frequency()));
            self.cpu_usage = Some(cpu.cpu_usage() as f32);
        }

        // Get memory information
        self.total_mem = Some(sys.total_memory());
        self.mem_usage = Some((sys.used_memory() as f32 / sys.total_memory() as f32) * 100.0);
        // Get hostname if not already set
        self.sys_hostname = Some(System::host_name().unwrap_or_default());

        Ok(())
    }

    pub fn is_wan_device(&self) -> bool {
        if self.net_id.is_some() {
            let net_id = self.net_id.as_ref().unwrap();
            if net_id.starts_with("wan") {
                return true;
            }
        } 
        return false;
    }

}



