// src/config.rs
use serde::Deserialize;
use std::collections::HashMap;
use tokio::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub hosts: HashMap<String, HostConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HostConfig {
    pub routes: HashMap<String, RouteConfig>,
    pub tls: Option<TlsConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RouteConfig {
    pub upstream: Option<String>,
    pub local_dir: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

impl Config {
    pub async fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path).await?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
