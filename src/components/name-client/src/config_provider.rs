use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use std::io::Result;
use std::time::SystemTime;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::{NSResult, NameInfo, NameProof, NsProvider, RecordType};
use name_lib::*;

/* config file example (toml):

[www.example.com]
TTL=1800
A=["192.168.1.102","192.168.1.103"]
DID="did:example:1234567890"
PKX="0:xxxxx"
CNAME="www.abc.com"

["*.example.com"]
A=["192.168.1.104","192.168.1.105"]
TXT="THIS_IS_TXT_RECORD"


[mail.example.com]
A=["192.168.1.106"]
MX=["mail.example.com"]


*/



#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct DomainConfig {
    #[serde(default)]
    pub ttl: u32,
    #[serde(default, rename = "A")]
    pub a: Vec<String>,
    #[serde(default, rename = "AAAA")]
    pub aaaa: Vec<String>, 
    #[serde(default, rename = "MX")]
    pub mx: Vec<String>,
    #[serde(default, rename = "TXT")]
    pub txt: Option<String>,
    #[serde(default, rename = "DID")]
    pub did: Option<String>,
    #[serde(default, rename = "PKX")]
    pub pkx: Vec<String>,   
    #[serde(default, rename = "CNAME")]
    pub cname: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct DnsLocalConfig {
    #[serde(flatten)]
    pub domains: HashMap<String, DomainConfig>,
}

pub struct ConfigProvider {
    inner: Arc<Mutex<ConfigProviderInner>>,
}

struct ConfigProviderInner {
    config: DnsLocalConfig,
    config_path: PathBuf,
    last_modified: SystemTime,
}

impl ConfigProvider {
    pub fn new(config_path: &Path) -> Result<Self> {
        let mut file = File::open(config_path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        
        let config: DnsLocalConfig = toml::from_str(&contents)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        let metadata = file.metadata()?;
        let last_modified = metadata.modified()?;
        
        let inner = ConfigProviderInner {
            config,
            config_path: config_path.to_path_buf(),
            last_modified,
        };
        
        Ok(ConfigProvider { 
            inner: Arc::new(Mutex::new(inner))
        })
    }

    fn check_and_reload_config(inner: &mut ConfigProviderInner) -> Result<bool> {
        let metadata = std::fs::metadata(&inner.config_path)?;
        let current_modified = metadata.modified()?;
        
        if current_modified > inner.last_modified {
            let mut file = File::open(&inner.config_path)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            
            let new_config: DnsLocalConfig = toml::from_str(&contents)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            
            inner.config = new_config;
            inner.last_modified = current_modified;
            info!("Config file reloaded successfully");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn matches_wildcard(pattern: &str, name: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        let pattern_parts: Vec<&str> = pattern.split('.').collect();
        let name_parts: Vec<&str> = name.split('.').collect();

        if pattern_parts.len() != name_parts.len() {
            return false;
        }

        pattern_parts.iter().zip(name_parts.iter()).all(|(p, n)| {
            *p == "*" || *p == *n
        })
    }

    fn convert_domain_config_to_records(domain: &str, config: &DomainConfig,record_type: RecordType) -> NSResult<NameInfo> {
        let default_ttl = config.ttl;
        let mut name_info = NameInfo {
            name: domain.to_string(),
            address: Vec::new(),
            cname: None,
            txt: None,
            did_document: None,
            pk_x_list: None,
            proof_type: NameProof::None,
            create_time: 0,
            ttl: Some(default_ttl),
        };

        match record_type {
            RecordType::A => {
                name_info.address = config.a.iter()
                    .filter_map(|addr| IpAddr::from_str(addr).ok())
                    .filter(|addr| addr.is_ipv4())
                    .collect();
                if name_info.address.len() < 1 {
                    return Err(NSError::InvalidData);
                }
            },
            RecordType::AAAA => {
                name_info.address = config.aaaa.iter()
                    .filter_map(|addr| IpAddr::from_str(addr).ok())
                    .filter(|addr| addr.is_ipv6())
                    .collect();
                if name_info.address.len() < 1 {
                    return Err(NSError::InvalidData);
                }
            },
            RecordType::CNAME => {
                if config.cname.is_some() {
                    name_info.cname = config.cname.clone();
                } else {
                    return Err(NSError::InvalidData);
                } 
                
            },
            RecordType::TXT => {
                name_info.txt = config.txt.clone();
                if config.did.is_some() {
                    let doc_doc_str = config.did.clone().unwrap();
                    let did_doc = EncodedDocument::from_str(doc_doc_str)?;
                    name_info.did_document = Some(did_doc);
                }
                if !config.pkx.is_empty() {
                    name_info.pk_x_list = Some(config.pkx.clone());
                }
            },
            _ => {
                warn!("record type {} not support in dns local file config provider", record_type.to_string());
                return Err(NSError::InvalidData);
            }
        }

        Ok(name_info)
    }

    pub fn get_all_domains(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.config.domains.keys().cloned().collect()
    }
}

#[async_trait::async_trait]
impl NsProvider for ConfigProvider {
    fn get_id(&self) -> String {
        "local dns-record-config provider".to_string()
    }

    async fn query(&self, domain: &str, record_type: Option<RecordType>, _from_ip: Option<IpAddr>) -> NSResult<NameInfo> {
        let record_type = record_type.unwrap_or(RecordType::A);
        
        // 获取内部配置的锁
        let mut inner = self.inner.lock().unwrap();
        
        // 检查并重新加载配置
        if let Err(e) = Self::check_and_reload_config(&mut inner) {
            warn!("Failed to reload config: {}", e);
        }
        
        // First check for exact match
        let config = inner.config.domains.get(domain);
        if config.is_none() {
            for (pattern, config) in &inner.config.domains {
                if pattern.contains('*') {
                    if Self::matches_wildcard(pattern, domain) {
                        debug!("{} found in matches_wildcard {}",domain,pattern);
                        return Self::convert_domain_config_to_records(
                            domain,
                            config,
                            record_type
                        );
                    }
                }
            }
            return Err(NSError::NotFound(domain.to_string()));
        }
        debug!("{} found in dns-local-config!",domain);
        let config = config.unwrap();
        return Self::convert_domain_config_to_records(
            domain,
            config,
            record_type
        );
    }

    async fn query_did(&self, did: &DID, _fragment: Option<&str>, _from_ip: Option<IpAddr>) -> NSResult<EncodedDocument> {
        let domain = did.to_host_name();
        // 获取内部配置的锁
        let mut inner = self.inner.lock().unwrap();
        
        // 检查并重新加载配置
        if let Err(e) = Self::check_and_reload_config(&mut inner) {
            warn!("Failed to reload config: {}", e);
        }
        
        // First check for exact match
        let config = inner.config.domains.get(&domain);
        if config.is_none() {
            for (pattern, config) in &inner.config.domains {
                if pattern.contains('*') {
                    if Self::matches_wildcard(pattern, &domain) {
                        if config.did.is_some() {
                            debug!("query_did: {} found in matches_wildcard {}",domain,pattern);
                            let doc_doc_str = config.did.clone().unwrap();
                            let did_doc = EncodedDocument::from_str(doc_doc_str)?;
                            return Ok(did_doc);
                        }
                    }
                }
            }
            return Err(NSError::NotFound(domain.to_string()));
        }

        debug!("query_did: {} found in dns-local-config!",domain);
        let config = config.unwrap();
        if config.did.is_some() {
            let doc_doc_str = config.did.clone().unwrap();
            let did_doc = EncodedDocument::from_str(doc_doc_str)?;
            return Ok(did_doc);
        }
        return Err(NSError::NotFound(domain.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;
    use buckyos_kit::init_logging;

    fn create_test_config() -> NamedTempFile {
        init_logging("config-provider-test", false);
        let config_content = r#"
["www.example.com"]
ttl = 300
A = ["192.168.1.1"]
TXT="THISISATEST"
DID="eyJhbGciOiJFZERTQSJ9.eyJvb2RzIjpbIm9vZDEiXSwiZXhwIjoyMDU4ODM4OTM5LCJpYXQiOjE3NDM0Nzg5Mzl9.6p01rckQkSoZ4kMOQfqZ_JIfHisYI27xNMtmHdYiu_0J_FZC-9j6JzN8PO3PO2A9Eugwo2877LJ5cyHGYEIbCw"

["*.example.com"]
ttl = 300
A = ["192.168.1.2"]

["*.sub.example.com"]
ttl = 300
A = ["192.168.1.3"]

["mail.example.com"]
ttl = 300
A = ["192.168.1.106"]
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(config_content.as_bytes()).unwrap();
        temp_file
    }

    #[test]
    fn test_wildcard_matching() {
        assert!(ConfigProvider::matches_wildcard("*", "www"));
        assert!(ConfigProvider::matches_wildcard("*.example.com", "www.example.com"));
        assert!(ConfigProvider::matches_wildcard("*.example.com", "mail.example.com"));
        assert!(!ConfigProvider::matches_wildcard("*.example.com", "example.com"));
        assert!(!ConfigProvider::matches_wildcard("*.example.com", "sub.www.example.com"));
    }

    #[tokio::test]
    async fn test_config_provider() {
        let temp_file = create_test_config();
        let provider = ConfigProvider::new(temp_file.path()).unwrap();
        // Test exact domain match
        let result = provider.query("www.example.com", Some(RecordType::A), None).await.unwrap();
        assert_eq!(result.name, "www.example.com");
        assert_eq!(result.ttl.unwrap(), 300);
        assert_eq!(result.address.len(), 1);
        assert_eq!(result.address[0].to_string(), "192.168.1.1");

        let result = provider.query_did(&DID::new("web","www.example.com"), None, None).await.unwrap();
        assert_eq!(result.to_string(),"eyJhbGciOiJFZERTQSJ9.eyJvb2RzIjpbIm9vZDEiXSwiZXhwIjoyMDU4ODM4OTM5LCJpYXQiOjE3NDM0Nzg5Mzl9.6p01rckQkSoZ4kMOQfqZ_JIfHisYI27xNMtmHdYiu_0J_FZC-9j6JzN8PO3PO2A9Eugwo2877LJ5cyHGYEIbCw");

        let result = provider.query("www.example.com", Some(RecordType::TXT), None).await.unwrap();
        assert_eq!(result.name, "www.example.com");
        assert_eq!(result.ttl.unwrap(), 300);
        assert_eq!(result.txt,Some("THISISATEST".to_string()));
        

        // Test wildcard domain match
        let result = provider.query("test.example.com", Some(RecordType::A), None).await.unwrap();
        assert_eq!(result.name, "test.example.com"); 
        assert_eq!(result.ttl.unwrap(), 300);
        

        // Test nested wildcard domain match
        let result = provider.query("foo.sub.example.com", Some(RecordType::A), None).await.unwrap();
        assert_eq!(result.name, "foo.sub.example.com");
        assert_eq!(result.ttl.unwrap(), 300);

        // Test non-existent domain
        let result = provider.query("nonexistent.com", Some(RecordType::A), None).await;
        assert!(result.is_err());
    }
}