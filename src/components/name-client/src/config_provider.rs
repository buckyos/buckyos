use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use std::io::Result;

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



#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DnsLocalConfig {
    #[serde(flatten)]
    pub domains: HashMap<String, DomainConfig>,
}

pub struct ConfigProvider {
    config: DnsLocalConfig,
}

impl ConfigProvider {
    pub fn new(config_path: &Path) -> Result<Self> {
        let mut file = File::open(config_path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        
        let config: DnsLocalConfig = toml::from_str(&contents)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        Ok(ConfigProvider { config })
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
        self.config.domains.keys().cloned().collect()
    }
}

#[async_trait::async_trait]
impl NsProvider for ConfigProvider {
    fn get_id(&self) -> String {
        "local dns-record-config provider".to_string()
    }

    async fn query(&self, domain: &str, record_type: Option<RecordType>, from_ip: Option<IpAddr>) -> NSResult<NameInfo> {
        let record_type = record_type.unwrap_or(RecordType::A);
    
        // First check for exact match
        let config = self.config.domains.get(domain);
        if config.is_none() {
            for (pattern, config) in &self.config.domains {
                if pattern.contains('*') {
                    if Self::matches_wildcard(pattern, domain) {
                        info!("{} found in matches_wildcard {}",domain,pattern);
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
        info!("{} found in dns-local-config!",domain);
        let config = config.unwrap();
        return Self::convert_domain_config_to_records(
            domain,
            config,
            record_type
        );
    }

    async fn query_did(&self, did: &DID, fragment: Option<&str>, from_ip: Option<IpAddr>) -> NSResult<EncodedDocument> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use hickory_resolver::proto::rr::domain;
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