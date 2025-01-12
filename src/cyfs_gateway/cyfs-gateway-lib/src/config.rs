

use tokio::fs;
use std::collections::HashMap;
use serde::Deserialize;
use url::Url;
use cyfs_socks::SocksProxyConfig;

#[derive(Debug, Deserialize, Clone)]
pub struct NamedDataMgrRouteConfig {
    pub named_data_mgr_id : String,
    pub read_only:bool,
    pub guest_access:bool,// 是否允许zone外访问
    //是否将chunkid放在路径的第一级，
    //如果为true，则使用https://ndn.$zoneid/$chunkid/index.html?ref=www.buckyos.org 
    //如果为false，则将chunkid放在host的第一段https://$chunkid.ndn.$zoneid/index.html?ref=www.buckyos.org 
    pub is_chunk_id_in_path:bool,
    pub enable_mgr_file_path:bool,// 是否使用mgr路径模式
}

impl Default for NamedDataMgrRouteConfig {
    fn default()->Self {
        Self { 
            named_data_mgr_id:"default".to_string(), 
            read_only:true, 
            guest_access:true, 
            is_chunk_id_in_path:true,
            enable_mgr_file_path:true
        }
    }
}


#[derive(Debug, Deserialize, Clone)]
pub struct HostConfig {
    #[serde(default)]
    pub enable_cors: bool,
    pub routes: HashMap<String, RouteConfig>,
    pub tls: Option<TlsConfig>,
}

impl Default for HostConfig {
    fn default() -> Self {
        HostConfig { enable_cors: false, routes: HashMap::new(), tls: None}
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RouteConfig {
    pub upstream: Option<String>,
    pub local_dir: Option<String>,
    pub inner_service: Option<String>,
    pub tunnel_selector: Option<String>,
    pub bucky_service: Option<String>,
    pub named_mgr: Option<NamedDataMgrRouteConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WarpServerConfig {
    pub tls_port:u16,
    pub http_port:u16,
    pub bind:Option<String>,
    pub default_tls_host: Option<String>,
    pub hosts: HashMap<String, HostConfig>,
}

impl WarpServerConfig {
    pub async fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path).await?;
        let config: WarpServerConfig = serde_json::from_str(&content)?;
        Ok(config)
    }
}



#[derive(Deserialize, Debug,Clone)]
pub enum DNSProviderType {
    #[serde(rename = "dns")]
    DNS,//query name info by system
    SN,//query name info by sn server
}

#[derive(Deserialize,Clone)]
pub struct DNSProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: DNSProviderType,
    #[serde(flatten)]
    pub config: serde_json::Value,
}

#[derive(Deserialize, Clone)]
pub struct DNSServerConfig {
    pub bind : Option<String>,
    pub port : u16,
    //dot_port : u16,
    //doh_port : u16,
    //tls: Option<TlsConfig>, include cert.pem and key.pem
    //dnssec: bool,
    pub this_name:Option<String>,
    pub resolver_chain : Vec<DNSProviderConfig>,
    pub fallback : Vec<String>,//fallback dns servers
}


pub enum ServerConfig {
    Warp(WarpServerConfig),
    DNS(DNSServerConfig),
    Socks(SocksProxyConfig),
}

#[derive(Clone,Debug)]
pub enum DispatcherTarget {
    Forward(Url),
    Server(String),
}

#[derive(Clone,Debug)]
pub struct DispatcherConfig {
    pub incoming: Url,
    pub target: DispatcherTarget,
    pub enable_tunnels:Option<Vec<String>>,
}


impl DispatcherConfig {
    pub fn new_forward(incoming: Url, target: Url, enable_tunnels:Option<Vec<String>>) -> Self {
        DispatcherConfig {
            incoming,
            target : DispatcherTarget::Forward(target),
            enable_tunnels,
        }
    }

    pub fn new_server(incoming: Url, server_id: String, enable_tunnels:Option<Vec<String>>) -> Self {
        DispatcherConfig {
            incoming,
            target : DispatcherTarget::Server(server_id),
            enable_tunnels,
        }
    }
}


pub fn gen_demo_gateway_json_config() -> String {
    let result = r#"
{
    "tunnel_builder":{
        "tunnel_bdt" : {
            "enable-tunnel" : ["bdt","rtcp"],
            "sn" : "127.0.0.1"
        },
        "tunnel_ssr":{
            "enable-tunnel" : ["ssr","ss"],
            "proxy_config": {
                "host":"myssr.test.com",
                "port":8889,
                "auth":"aes:23323"
            }
        }
    },
    "servers":{
        "main_http_server":{
            "type":"cyfs-warp",
            "bind":"0.0.0.0",
            "http_port":80,
            "https_port":443,
            "hosts": {
                "another.com": {
                    "tls_only":1,
                    "tls": {
                        "cert_path": "/path/to/cert.pem",
                        "key_path": "/path/to/key.pem"
                    },
                    "routes": {
                        "/": {
                            "upstream": "http://localhost:9090"
                        }
                    }
                },
                "example.com": {
                    "routes": {
                        "/api": {
                            "upstream": "http://localhost:8080"
                        },
                        "/static": {
                            "local_dir": "D:\\temp"
                        }
                    }
                }
            }
        },
        "main_socks_server":{
            "type":"cyfs-socks",
            "bind":"localhost",
            "port":8000,

            "target":"ood02:6000",
            "enable-tunnel":["direct", "rtcp"],

            "rule_config":"http://www.buckyos.io/cyfs-socks-rule.toml"
        },
        "main_dns_server":{
            "type":"cyfs-dns",
            "bind":"localhost:53",
            "ddns":{
                "enable":true,
                "bind":"localhost:8080"
            },
            "rule_config":"http://www.buckyos.io/cyfs-socks-rule.toml",
            "providers":[
                {
                    "order":0,
                    "type":"zone_system_config"
                },
                {
                    "order":1,
                    "type":"d-dns"
                },
                {
                    "order":2,
                    "type":"ens-client",
                    "target":"http://ens.buckyos.org"
                },
                {
                    "order":3,
                    "type":"dns" 
                }

            ],
            "fallback":[
                "114.114.114.114:53",
                "8.8.8.8",
                "https://dns.google/dns-query"
            ]
        }
    },
    "dispatcher" : {
        "tcp://0.0.0.0:80":{
            "type":"server",
            "id":"main_http_server"
        },
        "tcp://0.0.0.0:443":{
            "type":"server",
            "id":"main_http_server"
        },
        "tcp://127.0.0.1:8000":{
            "type":"server",
            "id":"main_socks_server"
        },
        "udp://0.0.0.0:53":{
            "type":"server",
            "id":"main_dns_server"
        },

        "tcp://0.0.0.0:6000":{
            "type":"forward",
            "target":"ood02:6000",
            "enable-tunnel":["direct","rtcp"]
        },
        "tcp://0.0.0.0:6001":{
            "type":"forward",
            "target":"192.168.1.102:6001"
        }
    }
}    
    "#;

    return result.to_string();
}