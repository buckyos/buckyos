use crate::{RPCErrors, Result};
use buckyos_kit::buckyos_get_unix_timestamp;
use jsonwebtoken::{Algorithm, decode, DecodingKey, EncodingKey, Header, Validation, encode};
use log::{debug,warn};
use name_lib::decode_jwt_claim_without_verify;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::HashMap;

const DEFAULT_SESSION_TOKEN_EXPIRE_TIME: u64 = 60 * 15;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum RPCSessionTokenType {
    Normal,
    JWT,
}

impl Default for RPCSessionTokenType {
    fn default() -> Self {
        RPCSessionTokenType::Normal
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct RPCSessionToken {
    #[serde(skip_serializing, skip_deserializing)]
    pub token_type: RPCSessionTokenType,
    #[serde(skip_serializing, skip_deserializing)]
    pub token: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appid: Option<String>,

    #[serde(skip_serializing_if = "HashMap::is_empty")]
    #[serde(default)]
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl RPCSessionToken {
    pub fn generate_jwt_token(
        user_id: &str,
        app_id: &str,
        kid: Option<String>,
        private_key: &EncodingKey,
    ) -> Result<(String, RPCSessionToken)> {
        let timestamp = buckyos_get_unix_timestamp();
        let mut session_token = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            appid: Some(app_id.to_string()),
            exp: Some(timestamp + DEFAULT_SESSION_TOKEN_EXPIRE_TIME),
            iss: Some(user_id.to_string()),
            jti: None,
            session: None,
            sub: Some(user_id.to_string()),
            extra: HashMap::new(),
        };
        let result_str = session_token.generate_jwt(kid, private_key).map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to generate session token: {}", e))
        })?;

        session_token.token = Some(result_str.clone());
        Ok((result_str, session_token))
    }

    pub fn from_string(token: &str) -> Result<Self> {
        if token.trim().starts_with('{') {
            let mut result_token: RPCSessionToken = serde_json::from_str(token).map_err(|e| {
                RPCErrors::ReasonError(format!("Failed to deserialize session token: {}", e))
            })?;
            result_token.token_type = RPCSessionTokenType::Normal;
            return Ok(result_token);
        } else {
            let payload = decode_jwt_claim_without_verify(token).map_err(|e| {
                RPCErrors::ReasonError(format!("Failed to decode session token: {}", e))
            })?;
            let mut session_token =
                serde_json::from_value::<RPCSessionToken>(payload).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to decode session token: {}", e))
                })?;
            session_token.token_type = RPCSessionTokenType::JWT;
            session_token.token = Some(token.to_string());
            return Ok(session_token);
        }
    }

    pub fn get_subs(&self) -> Result<(String, String)> {
        if self.sub.is_none() || self.appid.is_none() {
            return Err(RPCErrors::InvalidToken(
                "Invalid token: sub or aud is none".to_string(),
            ));
        }
        Ok((
            self.sub.as_ref().unwrap().to_string(),
            self.appid.as_ref().unwrap().to_string(),
        ))
    }

    pub fn to_string(&self) -> String {
        match self.token_type {
            RPCSessionTokenType::Normal => {
                return self.token.as_ref().unwrap().to_string();
            }
            RPCSessionTokenType::JWT => {
                if self.token.is_none() {
                    //let jwt_token
                    return "".to_string();
                } else {
                    return self.token.as_ref().unwrap().to_string();
                }
            }
        }
    }

    pub fn generate_jwt(&self, kid: Option<String>, private_key: &EncodingKey) -> Result<String> {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = kid;
        header.typ = None;
        let payload = serde_json::to_value(self)
            .map_err(|op| RPCErrors::ReasonError(format!("encode to JSON error:{}", op)))?;
        //info!("header: {:?}",header);
        //info!("payload: {:?}",payload);
        let token = encode(&header, &payload, private_key)
            .map_err(|op| RPCErrors::ReasonError(format!("JWT encode error:{}", op)))?;
        Ok(token)
    }

    pub fn is_self_verify(&self) -> bool {
        match self.token_type {
            RPCSessionTokenType::Normal => {
                return false;
            }
            RPCSessionTokenType::JWT => {
                return true;
            }
        }
    }

    pub fn verify_by_key(&mut self, public_key: &DecodingKey) -> Result<()> {
        if !self.is_self_verify() {
            return Err(RPCErrors::InvalidToken(
                "Not a self verify token".to_string(),
            ));
        }
        let token_str = self.token.as_ref().unwrap();
        let header: jsonwebtoken::Header =
            jsonwebtoken::decode_header(token_str).map_err(|error| {
                RPCErrors::InvalidToken(format!("JWT decode header error : {}", error))
            })?;

        if header.kid.is_some() {
            warn!("JWT kid could be none at specific key verify model");
            //return Err(RPCErrors::InvalidToken("JWT kid is not allowed at specific key verify_model".to_string()));
        }

        let mut validation = Validation::new(header.alg);
        validation.validate_aud = false;
        let decoded_token = decode::<serde_json::Value>(token_str, &public_key, &validation)
            .map_err(|error| RPCErrors::InvalidToken(format!("JWT decode error:{}", error)))?;

        let decoded_json = decoded_token
            .claims
            .as_object()
            .ok_or(RPCErrors::InvalidToken("Invalid token".to_string()))?;
        debug!("decoded token: {:?}", decoded_json);

        let sub_value = decoded_json
            .get("sub")
            .or_else(|| decoded_json.get("userid"))
            .ok_or(RPCErrors::InvalidToken("Missing sub".to_string()))?;
        let sub = sub_value
            .as_str()
            .ok_or(RPCErrors::InvalidToken("Invalid sub".to_string()))?;
        let appid = decoded_json.get("appid");
        if let Some(appid) = appid {
            if appid.is_null() {
                self.appid = None;
            } else {
                self.appid = Some(appid.as_str().unwrap().to_string());
            }
        }

        let aud = decoded_json.get("aud");
        if let Some(aud) = aud {
            if aud.is_null() {
                self.aud = None;
            } else if let Some(aud) = aud.as_str() {
                self.aud = Some(aud.to_string());
            } else if let Some(aud_list) = aud.as_array() {
                let first = aud_list
                    .iter()
                    .find_map(|item| item.as_str())
                    .ok_or(RPCErrors::InvalidToken("Invalid aud".to_string()))?;
                self.aud = Some(first.to_string());
            } else {
                return Err(RPCErrors::InvalidToken("Invalid aud".to_string()));
            }
        }

        let iss = decoded_json.get("iss");
        if let Some(iss) = iss {
            if iss.is_null() {
                self.iss = None;
            } else {
                let iss = iss
                    .as_str()
                    .ok_or(RPCErrors::InvalidToken("Invalid iss".to_string()))?;
                self.iss = Some(iss.to_string());
            }
        }

        let exp = decoded_json.get("exp");
        if let Some(exp) = exp {
            if exp.is_null() {
                self.exp = None;
            } else {
                let exp = exp
                    .as_u64()
                    .ok_or(RPCErrors::InvalidToken("Invalid expire time".to_string()))?;
                self.exp = Some(exp);
            }
        }

        let jti = decoded_json.get("jti").or_else(|| decoded_json.get("nonce"));
        if let Some(jti) = jti {
            if jti.is_null() {
                self.jti = None;
            } else if let Some(jti) = jti.as_str() {
                self.jti = Some(jti.to_string());
            } else if let Some(jti) = jti.as_u64() {
                self.jti = Some(jti.to_string());
            } else {
                return Err(RPCErrors::InvalidToken("Invalid jti".to_string()));
            }
        }

        self.sub = Some(sub.to_string());
        Ok(())
    }

    // //return kid
    // pub fn verify_by_key_map(
    //     &mut self,
    //     trust_keys: &HashMap<String, DecodingKey>,
    // ) -> Result<String> {
    //     if !self.is_self_verify() {
    //         return Err(RPCErrors::InvalidToken(
    //             "Not a self verify token".to_string(),
    //         ));
    //     }
    //     if self.token.is_none() {
    //         return Err(RPCErrors::InvalidToken("Token is empty".to_string()));
    //     }

    //     let token_str = self.token.as_ref().unwrap();
    //     let header: jsonwebtoken::Header =
    //         jsonwebtoken::decode_header(token_str).map_err(|error| {
    //             RPCErrors::InvalidToken(format!("JWT decode header error : {}", error))
    //         })?;

    //     if header.alg != Algorithm::EdDSA {
    //         return Err(RPCErrors::ReasonError("JWT algorithm not allowed".to_string()));
    //     }

    //     let kid: String;
    //     if header.kid.is_none() {
    //         kid = "$default".to_string();
    //     } else {
    //         kid = header.kid.unwrap();
    //     }
    //     let public_key = trust_keys.get(kid.as_str());
    //     if public_key.is_none() {
    //         return Err(RPCErrors::KeyNotExist(kid.clone()));
    //     }
    //     let public_key = public_key.unwrap();

    //     let validation = Validation::new(header.alg);
    //     let decoded_token = decode::<serde_json::Value>(token_str, &public_key, &validation)
    //         .map_err(|error| RPCErrors::InvalidToken(format!("JWT decode error:{}", error)))?;

    //     let decoded_json = decoded_token
    //         .claims
    //         .as_object()
    //         .ok_or(RPCErrors::InvalidToken("Invalid token".to_string()))?;
    //     debug!("decoded token: {:?}", decoded_json);

    //     let sub_value = decoded_json
    //         .get("sub")
    //         .or_else(|| decoded_json.get("userid"))
    //         .ok_or(RPCErrors::InvalidToken("Missing sub".to_string()))?;
    //     let sub = sub_value
    //         .as_str()
    //         .ok_or(RPCErrors::InvalidToken("Invalid sub".to_string()))?;

    //     let appid = decoded_json.get("appid");
    //     if let Some(appid) = appid {
    //         if appid.is_null() {
    //             self.appid = None;
    //         } else {
    //             self.appid = Some(appid.as_str().unwrap().to_string());
    //         }
    //     }

    //     let aud = decoded_json.get("aud");
    //     if let Some(aud) = aud {
    //         if aud.is_null() {
    //             self.aud = None;
    //         } else if let Some(aud) = aud.as_str() {
    //             self.aud = Some(aud.to_string());
    //         } else if let Some(aud_list) = aud.as_array() {
    //             let first = aud_list
    //                 .iter()
    //                 .find_map(|item| item.as_str())
    //                 .ok_or(RPCErrors::InvalidToken("Invalid aud".to_string()))?;
    //             self.aud = Some(first.to_string());
    //         } else {
    //             return Err(RPCErrors::InvalidToken("Invalid aud".to_string()));
    //         }
    //     }

    //     let iss = decoded_json.get("iss");
    //     if let Some(iss) = iss {
    //         if iss.is_null() {
    //             self.iss = None;
    //         } else {
    //             let iss = iss
    //                 .as_str()
    //                 .ok_or(RPCErrors::InvalidToken("Invalid iss".to_string()))?;
    //             self.iss = Some(iss.to_string());
    //         }
    //     }

    //     let exp = decoded_json.get("exp");
    //     if let Some(exp) = exp {
    //         if exp.is_null() {
    //             self.exp = None;
    //         } else {
    //             let exp = exp
    //                 .as_u64()
    //                 .ok_or(RPCErrors::InvalidToken("Invalid expire time".to_string()))?;
    //             self.exp = Some(exp);
    //         }
    //     }

    //     let jti = decoded_json.get("jti").or_else(|| decoded_json.get("nonce"));
    //     if let Some(jti) = jti {
    //         if jti.is_null() {
    //             self.jti = None;
    //         } else if let Some(jti) = jti.as_str() {
    //             self.jti = Some(jti.to_string());
    //         } else if let Some(jti) = jti.as_u64() {
    //             self.jti = Some(jti.to_string());
    //         } else {
    //             return Err(RPCErrors::InvalidToken("Invalid jti".to_string()));
    //         }
    //     }

    //     self.sub = Some(sub.to_string());
    //     Ok(kid)
    // }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, EncodingKey};
    use serde_json::json;

    const TEST_PRIVATE_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#;

    const TEST_PUBLIC_JWK: &str = r#"{
  "kty": "OKP",
  "crv": "Ed25519",
  "x": "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8"
}"#;
    #[test]
    fn test_session_token_serialize_deserialize() {
        let session_token = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            token: None,
            aud: Some("aud".to_string()),
            exp: Some(1000),
            iss: Some("iss".to_string()),
            jti: Some("jti".to_string()),
            session: Some(1000),
            sub: Some("sub".to_string()),
            appid: Some("appid".to_string()),
            extra: HashMap::new(),
        };
        let serialized = serde_json::to_string(&session_token).unwrap();
        println!("serialized = {}", serialized);
        let deserialized: RPCSessionToken = RPCSessionToken::from_string(&serialized).unwrap();
        assert_eq!(session_token, deserialized);

        let session_token_with_extra = json!({
            "aud": "aud",
            "exp": 1000,
            "iss": "iss",
            "jti": "jti",
            "session": 1000,
            "sub": "sub",
            "appid": "appid",
            "new_field": "new.value",
        });
        let new_session_token = serde_json::from_value::<RPCSessionToken>(session_token_with_extra).unwrap();
        assert_eq!(new_session_token.extra.get("new_field").unwrap().as_str().unwrap(), "new.value");

        let  serialized = serde_json::to_string(&new_session_token).unwrap();
        println!("serialized = {}", serialized);
        let new_serialized = format!("\n\n    {}   \n\n", serialized);
        let deserialized: RPCSessionToken = RPCSessionToken::from_string(&new_serialized).unwrap();
        assert_eq!(new_session_token, deserialized);

    }

    #[test]
    fn verify_by_key_accepts_valid_token_and_populates_claims() {
        let now = buckyos_get_unix_timestamp();

        // Build the claims we expect to read back after verification.
        let mut claims = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            token: None,
            aud: Some("test-aud".to_string()),
            exp: Some(now + 60),
            iss: Some("issuer-123".to_string()),
            jti: Some("nonce-1".to_string()),
            session: None,
            sub: Some("user-123".to_string()),
            appid: Some("app-42".to_string()),
            extra: HashMap::new(),
        };

        let private_key = EncodingKey::from_ed_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
        let jwt = claims.generate_jwt(None, &private_key).unwrap();

        // Simulate receiving a token that still needs verification/signature checking.
        let mut token_to_verify = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            token: Some(jwt),
            aud: None,
            exp: None,
            iss: None,
            jti: None,
            session: None,
            sub: None,
            appid: None,
            extra: HashMap::new(),
        };

        let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_str(TEST_PUBLIC_JWK).unwrap();
        let public_key = DecodingKey::from_jwk(&jwk).unwrap();

        token_to_verify.verify_by_key(&public_key).expect("verification should succeed");

        assert_eq!(token_to_verify.sub.as_deref(), Some("user-123"));
        assert_eq!(token_to_verify.appid.as_deref(), Some("app-42"));
        assert_eq!(token_to_verify.aud.as_deref(), Some("test-aud"));
        assert_eq!(token_to_verify.iss.as_deref(), Some("issuer-123"));
        assert_eq!(token_to_verify.jti.as_deref(), Some("nonce-1"));
        assert!(token_to_verify.exp.unwrap() >= now);
    }
}
