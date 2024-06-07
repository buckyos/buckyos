use crate::error::GatewayError;

use std::str::FromStr;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAddrType {
    WAN,
    LAN,
}

impl PeerAddrType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PeerAddrType::WAN => "wan",
            PeerAddrType::LAN => "lan",
        }
    }
}

impl FromStr for PeerAddrType {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "wan" => Ok(PeerAddrType::WAN),
            "lan" => Ok(PeerAddrType::LAN),
            _ => Err(GatewayError::InvalidParam("type".to_owned())),
        }
    }
}
