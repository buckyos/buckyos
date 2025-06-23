use crate::{NdnError, NdnResult};
use blake2::{digest::Update as Blake2Update, Blake2s256, Digest as Blake2Digest};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256, Sha512};
use sha3::Keccak256;
use std::str::FromStr;
use hex;
use serde_json::{json, Value};
use crypto_common::hazmat::{SerializedState, SerializableState};

pub trait Hasher {
    fn support_state(&self) -> bool;
    fn get_pos(&self) -> u64;
    fn restore_from_state(&mut self, state_json: serde_json::Value) -> NdnResult<()>;
    fn save_state(&self) -> NdnResult<serde_json::Value>;
    fn update_from_bytes(&mut self, bytes: &[u8]) -> NdnResult<()>;
    fn finalize(self: Box<Self>) -> Vec<u8>;
}


pub const DEFAULT_HASH_METHOD: &str = "sha256";

#[derive(Debug, Clone,Copy, Eq, PartialEq)]
pub enum HashMethod {
    Sha256,
    Sha512,
    QCID,
    Blake2s256,
    Keccak256,
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
            Self::QCID => "qcid",   // QCID is a special case, not a hash method, use sha256 for hash
            Self::Blake2s256 => "blake2s256",
            Self::Keccak256 => "keccak256",
        }
    }

    pub fn as_mix_str(&self) -> &str {
        match self {
            Self::Sha256 => "mix256",
            Self::Sha512 => "mix512",
            Self::QCID => "mixqcid",
            Self::Blake2s256 => "mixblake2s256",
            Self::Keccak256 => "mixkeccak256",
        }
    }

    pub fn hash_bytes(&self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha512 => 64,
            Self::QCID => 32,
            Self::Blake2s256 => 32,
            Self::Keccak256 => 32,
        }
    }

    // Return the hash method from string, and a flag indicating if it is a mix hash
    pub fn parse(s: &str) -> NdnResult<(Self, bool)> {
        let is_mix = s.starts_with("mix");
        let hash_method = HashMethod::from_str(s)?;
        Ok((hash_method, is_mix))
    }
}

impl ToString for HashMethod {
    fn to_string(&self) -> String {
        self.as_str().to_string()
    }
}

impl FromStr for HashMethod {
    type Err = NdnError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sha256" | "mix256" => Ok(Self::Sha256),
            "sha512" | "mix512" => Ok(Self::Sha512),
            "qcid" | "mixqcid" => Ok(Self::QCID),
            "blake2s256" | "mixblake2s256" => Ok(Self::Blake2s256),
            "keccak256" | "mixkeccak256" => Ok(Self::Keccak256),
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

pub struct Sha256Hasher {
    hasher: Sha256,
    pos: u64,
}

impl Sha256Hasher {
    pub fn new() -> Self {
        Self { 
            hasher: Sha256::new(),
            pos: 0,
        }
    }
}

impl Hasher for Sha256Hasher {
    fn support_state(&self) -> bool { true }
    fn get_pos(&self) -> u64 { self.pos }

    fn restore_from_state(&mut self, state_json: serde_json::Value) -> NdnResult<()> {
        let pos = state_json["pos"].as_u64().unwrap_or(0);
        let serialized_state = hex::decode(
            state_json["state"].as_str().ok_or(NdnError::Internal("invalid hasher state json".to_string()))?)
            .map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?;

        self.hasher = Sha256::deserialize(
            &SerializedState::<Sha256>::try_from(&serialized_state[..]).map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?)
            .map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?;
        self.pos = pos;
        Ok(())
    }

    fn save_state(&self) -> NdnResult<serde_json::Value> {
        let state_json = json!({
            "hash_type": "sha256",
            "pos": self.pos,
            "state": hex::encode(self.hasher.serialize()),
        });
        Ok(state_json)
    }

    fn update_from_bytes(&mut self, bytes: &[u8]) -> NdnResult<()> {
        self.hasher.update(bytes);
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Vec<u8> {
        self.hasher.finalize().to_vec()
    }
}

pub struct Sha512Hasher {
    hasher: Sha512,
    pos: u64,
}

impl Sha512Hasher {
    pub fn new() -> Self {
        Self { 
            hasher: Sha512::new(),
            pos: 0,
        }
    }
}

impl Hasher for Sha512Hasher {
    fn support_state(&self) -> bool { true }
    fn get_pos(&self) -> u64 { self.pos }
    fn restore_from_state(&mut self, state_json: serde_json::Value) -> NdnResult<()> {
        let pos = state_json["pos"].as_u64().unwrap_or(0);
        let serialized_state = hex::decode(
            state_json["state"].as_str().ok_or(NdnError::Internal("invalid hasher state json".to_string()))?)
            .map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?;

        self.hasher = Sha512::deserialize(
            &SerializedState::<Sha512>::try_from(&serialized_state[..]).map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?)
            .map_err(|e| NdnError::Internal(format!("invalid hasher state json:{}",e.to_string())))?;
        self.pos = pos;
        Ok(())
    }

    fn save_state(&self) -> NdnResult<serde_json::Value> {
        let state_json = json!({
            "hash_type": "sha512",
            "pos": self.pos,
            "state": hex::encode(self.hasher.serialize()),
        });
        Ok(state_json)
    }

    fn update_from_bytes(&mut self, bytes: &[u8]) -> NdnResult<()> {
        self.hasher.update(bytes);
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Vec<u8> {
        self.hasher.finalize().to_vec()
    }
}

pub struct HashHelper {}

impl HashHelper {
    pub fn create_hasher(hash_method: HashMethod) -> NdnResult<Box<dyn Hasher + Send + Sync>> {
        match hash_method {
            HashMethod::Sha256 => Ok(Box::new(Sha256Hasher::new())),
            HashMethod::Sha512 => Ok(Box::new(Sha512Hasher::new())),
            _ => Err(NdnError::InvalidParam(format!("Unsupported hash method: {:?}", hash_method))),
        }
    }

    pub fn calc_hash(hash_method: HashMethod, data: &[u8]) -> Vec<u8> {
        match hash_method {
            HashMethod::Sha256 => {
                let mut hasher = Sha256::new();
                hasher.update(data);
                hasher.finalize().to_vec()
            }
            HashMethod::Sha512 => {
                let mut hasher = sha2::Sha512::new();
                hasher.update(data);
                hasher.finalize().to_vec()
            }
            HashMethod::Blake2s256 => {
                let mut hasher = Blake2s256::new();
                blake2::Digest::update(&mut hasher, data);
                hasher.finalize().to_vec()
            }
            HashMethod::Keccak256 => {
                let mut hasher = Keccak256::new();
                sha3::Digest::update(&mut hasher, data);
                hasher.finalize().to_vec()
            }
            HashMethod::QCID => {
                unimplemented!("QCID hash method not implemented yet");
            }
        }
    }

    pub fn calc_hash_list(hash_method: HashMethod, data: &[&[u8]]) -> Vec<u8> {
        match hash_method {
            HashMethod::Sha256 => {
                let mut hasher = sha2::Sha256::new();
                for item in data {
                    hasher.update(item);
                }
        
                hasher.finalize().to_vec()
            }
            HashMethod::Sha512 => {
                let mut hasher = sha2::Sha512::new();
                for item in data {
                    hasher.update(item);
                }
        
                hasher.finalize().to_vec()
            }
            HashMethod::Blake2s256 => {
                let mut hasher = Blake2s256::new();
                for item in data {
                    blake2::Digest::update(&mut hasher, item);
                }
        
                hasher.finalize().to_vec()
            }
            HashMethod::Keccak256 =>  {
                let mut hasher = Keccak256::new();
                for item in data {
                    sha3::Digest::update(&mut hasher, item);
                }
        
                hasher.finalize().to_vec()
            }
            HashMethod::QCID => {
                unimplemented!("QCID hash method not implemented yet");
            }
        }
    }

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
            HashMethod::Blake2s256 => {
                let mut hasher = Blake2s256::new();
                blake2::Digest::update(&mut hasher, left);
                blake2::Digest::update(&mut hasher, right);
                hasher.finalize().to_vec()
            }
            HashMethod::Keccak256 => {
                let mut hasher = Keccak256::new();
                sha3::Digest::update(&mut hasher, left);
                sha3::Digest::update(&mut hasher, right);
                hasher.finalize().to_vec()
            }
            HashMethod::QCID => {
                unimplemented!("QCID hash method not implemented yet");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    fn test_hasher_state_save_restore(hash_method: HashMethod) {
        let mut buffer = vec![0u8; 2048];
        let mut rng = rand::rng();
        rng.fill(&mut buffer[..]);

        let mut hasher = HashHelper::create_hasher(hash_method).unwrap();
        hasher.update_from_bytes(&buffer).unwrap();
        let hash_result = hasher.finalize();

        let hash_result_restored = {
            let mut hasher = HashHelper::create_hasher(hash_method).unwrap();
            hasher.update_from_bytes(&buffer[..1024]).unwrap();
            let state_json = hasher.save_state().unwrap();
            println!("state_json:{}", state_json.to_string());

            let mut hasher_restored = HashHelper::create_hasher(hash_method).unwrap();
            hasher_restored.restore_from_state(state_json).unwrap();
            hasher_restored.update_from_bytes(&buffer[1024..]).unwrap();
            hasher_restored.finalize()
        };

        assert_eq!(hash_result, hash_result_restored);
    }

    #[test]
    fn test_sha256_hasher_state_save_restore() {
        test_hasher_state_save_restore(HashMethod::Sha256);
    }

    #[test]
    fn test_sha512_hasher_state_save_restore() {
        test_hasher_state_save_restore(HashMethod::Sha512);
    }

}
