use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::{NSCmdRegister, NSError, NSErrorCode, NSResult};

#[derive(Clone, Serialize, Deserialize)]
pub struct AddrInfo {
    protocol: String,
    addr: String,
    port: u16
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NameType {
    #[serde(rename="zone")]
    Zone,
    #[serde(rename="node")]
    Node,
    #[serde(rename="service")]
    Service,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NameInfo {
    pub name: String,
    #[serde(rename="type")]
    pub ty: NameType,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addrs: Option<Vec<AddrInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign: Option<String>,
}

impl NameInfo {
    pub fn set_extra<T: Serialize>(&mut self, extra: &T) -> NSResult<()>{
        self.extra = Some(serde_json::to_value(extra).map_err(|e| {
            NSError::new(NSErrorCode::Failed, format!("Failed to serialize extra: {}", e))
        })?);
        Ok(())
    }

    pub fn get_extra<T: for<'a> Deserialize<'a>>(&self) -> NSResult<Option<T>> {
        if let Some(extra) = &self.extra {
            Ok(Some(serde_json::from_value(extra.clone()).map_err(|e| {
                NSError::new(NSErrorCode::Failed, format!("Failed to deserialize extra: {}", e))
            })?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod test {
    use serde::{Deserialize, Serialize};
    use crate::NameInfo;
    use crate::provider::NameType;

    #[derive(Clone, Serialize, Deserialize)]
    struct Etcd {
        name: String,
        addr: String,
        port: u16,
        ad_port: u16,
    }

    #[derive(Clone, Serialize, Deserialize)]
    struct ZoneCofig {
        etcds: Vec<Etcd>,
    }

    #[test]
    fn test_name_info() {
        let mut name_info = NameInfo {
            name: "test.site".to_string(),
            ty: NameType::Zone,
            version: "1.0".to_string(),
            addrs: Some(vec![crate::AddrInfo {
                protocol: "tcp".to_string(),
                addr: "127.0.0.77".to_string(),
                port: 3456,
            }]),
            extra: None,
            sign: None,
        };

        name_info.set_extra(&ZoneCofig {
            etcds: vec![Etcd {
                name: "etcd1.test.site".to_string(),
                addr: "127.0.0.77".to_string(),
                port: 2379,
                ad_port: 2380,
            }, Etcd {
                name: "etcd2.test.site".to_string(),
                addr: "127.0.0.78".to_string(),
                port: 2379,
                ad_port: 2380,
            }, Etcd {
                name: "etcd3.test.site".to_string(),
                addr: "127.0.0.79".to_string(),
                port: 2379,
                ad_port: 2380,
            }]
            }).unwrap();
        let config = serde_json::to_string(&name_info).unwrap();
        println!("{}", config);
    }
}

#[async_trait::async_trait]
pub trait NSProvider: 'static + Send + Sync {
    async fn load(&self, cmd_register: &NSCmdRegister) -> NSResult<()>;
    async fn query(&self, name: &str) -> NSResult<NameInfo>;
}
