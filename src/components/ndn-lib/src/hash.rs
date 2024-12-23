use crate::NdnError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
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

impl Serialize for HashMethod {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HashMethod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        HashMethod::from_str(&s).map_err(serde::de::Error::custom)
    }
}

pub struct HashHelper {}

impl HashHelper {
    pub fn calc_parent_hash(hash_method: HashMethod, left: &[u8], right: &[u8]) -> Vec<u8> {
        match hash_method {
            HashMethod::Sha256 => {
                let mut hasher = sha2::Sha256::new();
                hasher.update(left);
                hasher.update(right);
                hasher.finalize().to_vec()
            }
            HashMethod::Sha512 => {
                let mut hasher = sha2::Sha512::new();
                hasher.update(left);
                hasher.update(right);
                hasher.finalize().to_vec()
            }
        }
    }
}
