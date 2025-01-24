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
use nvml_wrapper::*;
use nvml_wrapper::enum_wrappers::device::Clock;



// describe a device runtime info
#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct DeviceInfo {
    pub hostname:String,
    pub device_type:String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch:Option<String>,
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
    pub base_os_info:Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_info:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_num:Option<u32>,//cpu核心数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_mhz:Option<u32>,//cpu的最大性能,单位是MHZ
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_ratio:Option<f32>,//cpu的性能比率
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage:Option<f32>,//类似top里的load,0 -- core 
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_mem:Option<u64>,//单位是bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_usage:Option<u64>,//单位是bytes

    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_space:Option<u64>,//单位是bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_usage:Option<u64>,//单位是bytes

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_info:Option<String>,//gpu信息
    #[serde(skip_serializing_if = "Option::is_none")]     
    pub gpu_tflops:Option<f32>,//gpu的算力,单位是TFLOPS
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_total_mem:Option<u64>,//gpu总内存,单位是bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_used_mem:Option<u64>,//gpu已用内存,单位是bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_load:Option<f32>,//gpu负载
    
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
            state:Some("Ready".to_string()),
            arch:None,
            ip:ip,
            did:did,
            main_net_interface:None,
            net_id:net_id,
            base_os_info:None,
            cpu_info:None,
            cpu_num:None,
            cpu_mhz:None,
            cpu_ratio:None,
            cpu_usage:None,
            total_mem:None,
            mem_usage:None,
            total_space:None,
            disk_usage:None,
            sys_hostname:None,
            gpu_info:None,
            gpu_tflops:None,
            gpu_total_mem:None,
            gpu_used_mem:None,
            gpu_load:None,
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
        let mut cpu_usage = 0.0;
        let mut cpu_mhz:u32 = 0;
        let mut cpu_mhz_last:u32 = 0;
        let mut cpu_brand:String = "Unknown".to_string();
        self.cpu_ratio = Some(1.0);
        for cpu in sys.cpus() {
            cpu_brand = cpu.brand().to_string();
            cpu_usage += cpu.cpu_usage();
            cpu_mhz += cpu.frequency() as u32;
            cpu_mhz_last = cpu.frequency() as u32;
        }
        self.cpu_info = Some(format!("{} @ {} MHz,({} cores)", cpu_brand, cpu_mhz_last, sys.cpus().len()));
        self.cpu_num = Some(sys.cpus().len() as u32);
        self.cpu_mhz = Some(cpu_mhz);
        self.cpu_usage = Some(cpu_usage);
        // Get memory information
        self.total_mem = Some(sys.total_memory());
        self.mem_usage = Some(sys.used_memory());
        // Get hostname if not already set
        self.sys_hostname = Some(System::host_name().unwrap_or_default());

        // First try NVIDIA GPU
        let nvidia_info = match nvml_wrapper::Nvml::init() {
            Ok(nvml) => {
                if let Ok(device) = nvml.device_by_index(0) {
                    // Get GPU name
                    let name = device.name().ok();
                    let memory = device.memory_info().ok();
                    let utilization = device.utilization_rates().ok();
                    let clock = device.clock_info(Clock::Graphics).ok();
                    let cuda_cores = device.num_cores().ok();

                    Some((name, memory, utilization, clock, cuda_cores))
                } else {
                    None
                }
            }
            Err(_) => None,
        };

        if let Some((name, memory, utilization, clock, cuda_cores)) = nvidia_info {
            // NVIDIA GPU found
            self.gpu_info = name.map(|n| format!("NVIDIA {}", n));
            if let Some(mem) = memory {
                self.gpu_total_mem = Some(mem.total);
                self.gpu_used_mem = Some(mem.used);
            }
            if let Some(util) = utilization {
                self.gpu_load = Some(util.gpu as f32);
            }
            if let (Some(clock), Some(cores)) = (clock, cuda_cores) {
                let tflops = (clock as f32 * cores as f32 * 2.0) / 1_000_000.0;
                self.gpu_tflops = Some(tflops);
            }
        } else {
            // Try to get basic GPU info from system
            #[cfg(target_os = "linux")]
            {
                use std::fs;
                use std::path::Path;

                let gpu_dir = Path::new("/sys/class/drm");
                if gpu_dir.exists() {
                    if let Ok(entries) = fs::read_dir(gpu_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if let Some(name) = path.file_name() {
                                if name.to_string_lossy().starts_with("card") {
                                    // Try to read vendor name
                                    if let Ok(vendor) = fs::read_to_string(path.join("device/vendor")) {
                                        let vendor = vendor.trim();
                                        let gpu_type = match vendor {
                                            "0x1002" => "AMD",
                                            "0x8086" => "Intel",
                                            _ => "Unknown",
                                        };
                                        
                                        // Try to read device name
                                        if let Ok(device) = fs::read_to_string(path.join("device/device")) {
                                            self.gpu_info = Some(format!("{} GPU (Device ID: {})", 
                                                gpu_type, device.trim()));
                                        } else {
                                            self.gpu_info = Some(format!("{} GPU", gpu_type));
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // If no GPU info was found
        if self.gpu_info.is_none() {
            self.gpu_info = Some("No GPU detected or unable to get GPU information".to_string());
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_device_info() {
        let mut device_info = DeviceInfo::new("ood1@192.168.1.1#wan1", Some("did:bns:ood1".to_string()));
        device_info.auto_fill_by_system_info().await.unwrap();
        println!("{:?}", device_info);
    }
}



