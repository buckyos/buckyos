

use tokio::fs;
use std::collections::HashMap;
use serde::Deserialize;
use url::Url;
use cyfs_socks::SocksProxyConfig;

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Clone)]
pub struct NamedDataMgrRouteConfig {
    pub named_data_mgr_id : String,
    #[serde(default = "default_true")]
    pub read_only:bool,
    #[serde(default = "default_true")]
    pub guest_access:bool,// 是否允许zone外访问
    #[serde(default = "default_true")]
    //是否将chunkid放在路径的第一级，
    //如果为true，则使用https://ndn.$zoneid/$chunkid/index.html?ref=www.buckyos.org 
    //如果为false，则将chunkid放在host的第一段https://$chunkid.ndn.$zoneid/index.html?ref=www.buckyos.org 
    pub is_object_id_in_path:bool,
    #[serde(default = "default_true")]
    pub enable_mgr_file_path:bool,// 是否使用mgr路径模式
    #[serde(default = "default_true")]
    pub enable_zone_put_chunk:bool
}

impl Default for NamedDataMgrRouteConfig {
    fn default()->Self {
        Self { 
            named_data_mgr_id:"default".to_string(), 
            read_only:true, 
            guest_access:false, 
            is_object_id_in_path:true,
            enable_mgr_file_path:true,
            enable_zone_put_chunk:true,
        }
    }
}


#[derive(Debug, Deserialize, Clone, Default)]
pub struct HostConfig {
    #[serde(default)]
    pub enable_cors: bool,
    #[serde(default)]
    pub redirect_to_https: bool, 
    #[serde(default)]
    pub tls: TlsConfig,
    pub routes: HashMap<String, RouteConfig>,
}


#[derive(Debug, Clone)]
pub enum RedirectType {
    None,
    Permanent,
    Temporary,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(from = "String")]

pub struct UpstreamRouteConfig {
    pub target: String,
    pub redirect: RedirectType,
}

impl From<String> for UpstreamRouteConfig {
    fn from(s: String) -> Self {
        Self::from_str(&s)
    }
}

impl UpstreamRouteConfig {
    pub fn from_str(s: &str) -> Self {
        let parts: Vec<&str> = s.split_whitespace().collect();
        let target = parts[0].to_string();
        let mut redirect = RedirectType::None;

        if parts.len() > 1 && parts[1] == "redirect" {
            if parts.len() > 2 {
                redirect = match parts[2] {
                    "permanent" => RedirectType::Permanent,
                    "temporary" => RedirectType::Temporary,
                    _ => RedirectType::None
                };
            } else {
                redirect = RedirectType::Temporary;
            }
        }

        Self {
            target,
            redirect
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ResponseRouteConfig {
    pub status: Option<u16>,
    pub headers: Option<HashMap<String, String>>,
    pub body: Option<String>,
}


fn default_enable_cors() -> bool {
    true
}
#[derive(Debug, Deserialize, Clone)]
pub struct RouteConfig {
    #[serde(default = "default_enable_cors")]
    pub enable_cors: bool, 
    pub response: Option<ResponseRouteConfig>,
    pub upstream: Option<UpstreamRouteConfig>,
    pub local_dir: Option<String>,
    pub inner_service: Option<String>,
    pub tunnel_selector: Option<String>,
    pub bucky_service: Option<String>,
    pub named_mgr: Option<NamedDataMgrRouteConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfig {
    pub disable_tls: bool,
    pub enable_acme: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}


impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            disable_tls: true,
            enable_acme: false,
            cert_path: None,
            key_path: None,
        }
    }
}


fn default_tls_port() -> u16 {
    0
}

fn default_http_port() -> u16 {
    80
}

#[derive(Debug, Deserialize, Clone)]
pub struct WarpServerConfig {
    #[serde(default = "default_tls_port")]
    pub tls_port:u16,
    #[serde(default = "default_http_port")]
    pub http_port:u16,
    pub bind:Option<String>,
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
    LocalConfig,
    
}

#[derive(Deserialize,Clone,Debug)]
pub struct DNSProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: DNSProviderType,
    #[serde(flatten)]
    pub config: serde_json::Value,
}

#[derive(Deserialize, Clone, Debug)]
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

#[derive(Debug)]
pub enum ServerConfig {
    Warp(WarpServerConfig),
    DNS(DNSServerConfig),
    Socks(SocksProxyConfig),
}

#[derive(Clone,Debug)]
pub enum DispatcherTarget {
    Forward(Url),
    Server(String),
    Selector(String),
    ProbeSelector(String,String), //probeid,selectorid
}

#[derive(Clone,Debug)]
pub struct DispatcherConfig {
    pub incoming: Url,
    pub target: DispatcherTarget
}


impl DispatcherConfig {
    pub fn new_forward(incoming: Url, target: Url) -> Self {
        DispatcherConfig {
            incoming,
            target : DispatcherTarget::Forward(target)
        }
    }

    pub fn new_server(incoming: Url, server_id: String) -> Self {
        DispatcherConfig {
            incoming,
            target : DispatcherTarget::Server(server_id),
        }
    }

    pub fn new_selector(incoming: Url, selector_id: String) -> Self {
        DispatcherConfig {
            incoming,
            target : DispatcherTarget::Selector(selector_id),
        }
    }

    pub fn new_probe_selector(incoming: Url, probe_id: String, selector_id: String) -> Self {
        DispatcherConfig {
            incoming,
            target : DispatcherTarget::ProbeSelector(probe_id, selector_id),
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