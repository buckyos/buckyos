use crate::NdnError;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum HashMethod {
    Sha256,
    Sha512,
}

impl Default for HashMethod {
    fn default() -> Self {
        Self::Sha256
    }
}

impl HashMethod {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    pub fn hash_bytes(&self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha512 => 64,
        }
    }
}

impl FromStr for HashMethod {
    type Err = NdnError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sha256" => Ok(Self::Sha256),
            "sha512" => Ok(Self::Sha512),
            _ => {
                let msg = format!("Invalid hash method: {}", s);
                error!("{}", msg);
                Err(NdnError::InvalidData(msg))
            }
        }
    }
}
