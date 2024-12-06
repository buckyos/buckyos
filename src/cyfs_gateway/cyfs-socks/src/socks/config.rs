use std::net::SocketAddr;
use crate::error::{SocksResult, SocksError};

#[derive(Debug, Clone)]
pub enum SocksProxyAuth {
    None,
    Password(String, String),
}

#[derive(Debug, Clone)]
pub struct SocksProxyConfig {
    pub id: String,
    
    pub bind: Option<String>,
    pub port: u16,
    pub addr: SocketAddr,

    pub auth: SocksProxyAuth,
}

impl SocksProxyConfig {
    pub fn load(config: &serde_json::Value) -> SocksResult<Self> {
        let id = config["id"].as_str().unwrap_or("socks5");

        let bind = config["bind"].as_str();
        let port = config["port"]
            .as_u64()
            .ok_or(SocksError::InvalidConfig("port".to_owned()))? as u16;

        let bind = bind.unwrap_or("0.0.0.0");
        let addr = format!("{}:{}", bind, port);
        let addr = addr.parse().map_err(|e| {
            let msg = format!("Error parsing addr: {}, {}", addr, e);
            error!("{}", msg);
            SocksError::InvalidConfig(msg)
        })?;

        let auth = if let Some(auth) = config.get("auth") {
            if !auth.is_object() {
                return Err(SocksError::InvalidConfig("auth".to_owned()));
            }

            let auth_type = auth["type"]
                .as_str()
                .ok_or(SocksError::InvalidConfig("auth.type".to_owned()))?;
            match auth_type {
                "password" => {
                    let username = auth["username"].as_str().unwrap();
                    let password = auth["password"].as_str().unwrap();
                    SocksProxyAuth::Password(username.to_owned(), password.to_owned())
                }
                _ => {
                    let msg = format!("Unknown auth type: {}", auth_type);
                    error!("{}", msg);
                    return Err(SocksError::InvalidConfig(msg));
                }
            }
        } else {
            SocksProxyAuth::None
        };

        Ok(Self {
            id: id.to_owned(),
            
            bind: Some(bind.to_owned()),
            port,
            addr,

            auth,
        })
    }

    pub fn dump(&self) -> serde_json::Value {
        let mut config = serde_json::Map::new();
        config.insert("block".to_owned(), "proxy".into());
        config.insert("type".to_owned(), "socks5".into());
        config.insert("id".to_owned(), self.id.clone().into());
        config.insert("addr".to_owned(), self.addr.ip().to_string().into());
        config.insert("port".to_owned(), self.addr.port().into());

        let auth = match &self.auth {
            SocksProxyAuth::None => serde_json::Value::Null,
            SocksProxyAuth::Password(username, password) => {
                let mut auth = serde_json::Map::new();
                auth.insert("type".to_owned(), "password".into());
                auth.insert("username".to_owned(), username.clone().into());
                auth.insert("password".to_owned(), password.clone().into());
                auth.into()
            }
        };

        if auth != serde_json::Value::Null {
            config.insert("auth".to_owned(), auth);
        }

        config.into()
    }
}
