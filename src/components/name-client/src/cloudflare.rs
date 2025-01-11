use std::net::IpAddr;
use reqwest::{Client, Method, header::{HeaderMap, HeaderValue}};
use serde::{Deserialize, Serialize};
use crate::{NsProvider, NsUpdateProvider, NameInfo, RecordType, NSResult, NSError};
use name_lib::*;
use crate::utility::extract_root_domain;

#[derive(Debug, Clone)]
pub struct CloudflareConfig {
    pub api_token: String,
    pub email: String,
    pub known_domains: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CloudflareResponse<T> {
    success: bool,
    errors: Vec<CloudflareError>,
    messages: Vec<String>,
    result: Option<T>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CloudflareError {
    code: i32,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DnsRecord {
    id: String,
    #[serde(rename = "type")]
    record_type: String,
    name: String,
    content: String,
    proxied: bool,
    ttl: u32,
}

#[cfg(feature = "cloudflare")]
pub struct CloudflareProvider {
    config: CloudflareConfig,
    client: Client,
}

#[cfg(feature = "cloudflare")]
impl CloudflareProvider {
    pub fn new(config: CloudflareConfig) -> Self {
        Self { 
            config,
            client: Client::new(),
        }
    }

    async fn get_zone_id(&self, domain: &str) -> NSResult<String> {
        let root_domain = extract_root_domain(domain, &self.config.known_domains)?;
        
        let url = "https://api.cloudflare.com/client/v4/zones";
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", 
            HeaderValue::from_str(&format!("Bearer {}", self.config.api_token)).unwrap());
        headers.insert("X-Auth-Email", 
            HeaderValue::from_str(&self.config.email).unwrap());

        let response = self.client.get(url)
            .headers(headers)
            .query(&[("name", root_domain)])
            .send()
            .await
            .map_err(|e| NSError::Failed(format!("Failed to query zone: {}", e)))?;

        #[derive(Deserialize)]
        struct Zone {
            id: String,
            name: String,
        }

        let zones: CloudflareResponse<Vec<Zone>> = response.json()
            .await
            .map_err(|e| NSError::Failed(format!("Failed to parse zone response: {}", e)))?;

        if !zones.success {
            return Err(NSError::Failed(format!("Cloudflare API error: {:?}", zones.errors)));
        }

        let zone = zones.result
            .and_then(|zones| zones.into_iter().next())
            .ok_or_else(|| NSError::Failed(format!("No zone found for domain: {}", domain)))?;

        Ok(zone.id)
    }

    async fn make_request(&self, domain: &str, method: Method, path: &str, body: Option<Vec<u8>>) -> NSResult<reqwest::Response> {
        let zone_id = self.get_zone_id(domain).await?;
        let url = format!("https://api.cloudflare.com/client/v4/zones/{}/dns_records{}", 
            zone_id, path);
        
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", 
            HeaderValue::from_str(&format!("Bearer {}", self.config.api_token)).unwrap());
        headers.insert("X-Auth-Email", 
            HeaderValue::from_str(&self.config.email).unwrap());

        let mut req = self.client.request(method, &url)
            .headers(headers);

        if let Some(body) = body {
            req = req.header("Content-Type", "application/json")
                .body(body);
        }

        req.send().await.map_err(|e| NSError::Failed(format!("Request failed: {}", e)))
    }
}

#[cfg(feature = "cloudflare")]
#[async_trait::async_trait]
impl NsUpdateProvider for CloudflareProvider {
    async fn update(&self, record_type: RecordType, record: NameInfo) -> NSResult<NameInfo> {
        let dns_record = DnsRecord {
            id: String::new(),
            record_type: record_type.to_string(),
            name: record.name.clone(),
            content: match record_type {
                RecordType::A | RecordType::AAAA => {
                    if record.address.is_empty() {
                        return Err(NSError::Failed("No IP address provided".to_string()));
                    }
                    record.address[0].to_string()
                },
                RecordType::TXT => record.txt.clone()
                    .ok_or_else(|| NSError::Failed("No TXT content provided".to_string()))?,
                RecordType::CNAME => record.cname.clone()
                    .ok_or_else(|| NSError::Failed("No CNAME content provided".to_string()))?,
                _ => return Err(NSError::Failed(format!("Unsupported record type: {:?}", record_type)))
            },
            proxied: false,
            ttl: record.ttl.unwrap_or(1),
        };

        let body = serde_json::to_vec(&dns_record).map_err(|e| NSError::Failed(format!("Failed to serialize request: {}", e)))?;
        let response = self.make_request(&record.name, Method::POST, "", Some(body)).await?;

        let cf_response: CloudflareResponse<DnsRecord> = response.json()
            .await
            .map_err(|e| NSError::Failed(format!("Failed to parse response: {}", e)))?;

        if !cf_response.success {
            return Err(NSError::Failed(format!("Cloudflare API error: {:?}", cf_response.errors)));
        }

        Ok(record)
    }

    async fn delete(&self, name: &str, record_type: RecordType) -> NSResult<Option<NameInfo>> {
        let list_path = format!("?type={}&name={}", record_type.to_string(), name);
        let response = self.make_request(name, Method::GET, &list_path, None).await?;
        
        let list_response: CloudflareResponse<Vec<DnsRecord>> = response.json().await.map_err(|e| NSError::Failed(format!("Failed to parse response: {}", e)))?;
        if !list_response.success {
            return Err(NSError::Failed(format!("Cloudflare API error: {:?}", list_response.errors)));
        }

        if let Some(records) = list_response.result {
            for record in records {
                let delete_path = format!("/{}", record.id);
                let response = self.make_request(name, Method::DELETE, &delete_path, None).await?;
                
                let delete_response: CloudflareResponse<DnsRecord> = response.json().await.map_err(|e| NSError::Failed(format!("Failed to parse response: {}", e)))?;
                if !delete_response.success {
                    return Err(NSError::Failed(format!("Failed to delete record: {:?}", delete_response.errors)));
                }
            }
        }

        Ok(None)
    }
}

#[cfg(feature = "cloudflare")]
#[async_trait::async_trait]
impl NsProvider for CloudflareProvider {
    fn get_id(&self) -> String {
        "cloudflare".to_string()
    }

    async fn query(&self, name: &str, record_type: Option<RecordType>, _from_ip: Option<IpAddr>) -> NSResult<NameInfo> {
        let record_type = record_type.unwrap_or(RecordType::A);
        let path = format!("?type={}&name={}", record_type.to_string(), name);
        
        let response = self.make_request(name, Method::GET, &path, None).await.map_err(|e| NSError::Failed(format!("Request failed: {}", e)))?;
        let cf_response: CloudflareResponse<Vec<DnsRecord>> = response.json().await.map_err(|e| NSError::Failed(format!("Failed to parse response: {}", e)))?;

        if !cf_response.success {
            return Err(NSError::Failed(format!("Cloudflare API error: {:?}", cf_response.errors)));
        }

        let records = cf_response.result.unwrap_or_default();
        if records.is_empty() {
            return Err(NSError::NotFound(format!("No {} record found for {}", record_type.to_string(), name)));
        }

        let mut name_info = NameInfo {
            name: name.to_string(),
            address: vec![],
            cname: None,
            txt: None,
            did_document: None,
            proof_type: crate::NameProof::None,
            create_time: 0,
            ttl: Some(records[0].ttl),
        };

        match record_type {
            RecordType::A | RecordType::AAAA => {
                for record in records {
                    if let Ok(ip) = record.content.parse() {
                        name_info.address.push(ip);
                    }
                }
            }
            RecordType::TXT => {
                name_info.txt = Some(records[0].content.clone());
            }
            RecordType::CNAME => {
                name_info.cname = Some(records[0].content.clone());
            }
            _ => {}
        }

        Ok(name_info)
    }

    async fn query_did(&self, _did: &str, _fragment: Option<&str>, _from_ip: Option<IpAddr>) -> NSResult<EncodedDocument> {
        Err(NSError::Failed("DID query not supported by Cloudflare provider".to_string()))
    }
}
