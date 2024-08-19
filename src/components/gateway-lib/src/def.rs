use crate::error::GatewayError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamServiceProtocol {
    Tcp,
    Udp,
    Http,
}

impl UpstreamServiceProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpstreamServiceProtocol::Tcp => "tcp",
            UpstreamServiceProtocol::Udp => "udp",
            UpstreamServiceProtocol::Http => "http",
        }
    }
}

impl ToString for UpstreamServiceProtocol {
    fn to_string(&self) -> String {
        self.as_str().to_owned()
    }
}

impl FromStr for UpstreamServiceProtocol {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tcp" => Ok(UpstreamServiceProtocol::Tcp),
            "udp" => Ok(UpstreamServiceProtocol::Udp),
            "http" => Ok(UpstreamServiceProtocol::Http),
            _ => Err(GatewayError::InvalidParam("type".to_owned())),
        }
    }
}

// Implementing Serialize for UpstreamServiceProtocol
impl Serialize for UpstreamServiceProtocol {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// Implementing Deserialize for UpstreamServiceProtocol
impl<'de> Deserialize<'de> for UpstreamServiceProtocol {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(|e| {
            serde::de::Error::custom(format!("Invalid UpstreamServiceProtocol: {}", e))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardProxyProtocol {
    Tcp,
    Udp,
}

impl ForwardProxyProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            ForwardProxyProtocol::Tcp => "tcp",
            ForwardProxyProtocol::Udp => "udp",
        }
    }
}

impl ToString for ForwardProxyProtocol {
    fn to_string(&self) -> String {
        self.as_str().to_owned()
    }
}
impl FromStr for ForwardProxyProtocol {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tcp" => Ok(ForwardProxyProtocol::Tcp),
            "udp" => Ok(ForwardProxyProtocol::Udp),
            _ => Err(GatewayError::InvalidConfig("proxy-type".to_owned())),
        }
    }
}

impl Serialize for ForwardProxyProtocol {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ForwardProxyProtocol {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s)
            .map_err(|e| serde::de::Error::custom(format!("Invalid ForwardProxyProtocol: {}", e)))
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Config,
    Dynamic,
}