use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_system_etc_dir};
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::{load_local_device_private_key, load_local_node_identity_config};

const SN_DEVICE_LOGIN_TOKEN_TTL_SECS: u64 = 15 * 60;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SnUserSession {
    pub session_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub refresh_expires_in: u64,
}

#[derive(Debug)]
pub struct SnUserLoginError {
    retryable: bool,
    message: String,
}

impl SnUserLoginError {
    fn fatal(message: impl Into<String>) -> Self {
        Self {
            retryable: false,
            message: message.into(),
        }
    }

    fn retryable(message: impl Into<String>) -> Self {
        Self {
            retryable: true,
            message: message.into(),
        }
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
    }
}

impl Display for SnUserLoginError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message.as_str())
    }
}

impl std::error::Error for SnUserLoginError {}

#[derive(Debug, Deserialize)]
struct SnUserLoginEnvelope {
    code: i64,
    #[serde(default)]
    msg: String,
    data: Option<SnUserSession>,
}

pub fn generate_sn_user_device_token(user_name: &str) -> Result<String, SnUserLoginError> {
    let user_name = user_name.trim();
    if user_name.is_empty() {
        return Err(SnUserLoginError::fatal("SN user name is empty"));
    }
    let node_identity_path = get_buckyos_system_etc_dir().join("node_identity.json");
    let node_identity = load_local_node_identity_config(node_identity_path.as_path())
        .map_err(|err| SnUserLoginError::fatal(format!("load node identity failed: {err}")))?;
    let private_key = load_local_device_private_key(&node_identity.device_did)
        .map_err(|err| SnUserLoginError::fatal(format!("load device private key failed: {err}")))?;
    generate_sn_user_device_token_with_identity(
        user_name,
        node_identity.device_name.as_str(),
        &private_key,
        buckyos_get_unix_timestamp(),
    )
}

fn generate_sn_user_device_token_with_identity(
    user_name: &str,
    device_name: &str,
    private_key: &jsonwebtoken::EncodingKey,
    now: u64,
) -> Result<String, SnUserLoginError> {
    let device_name = device_name.trim();
    if device_name.is_empty() {
        return Err(SnUserLoginError::fatal("SN device name is empty"));
    }
    RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(now.saturating_add(SN_DEVICE_LOGIN_TOKEN_TTL_SECS)),
        iss: Some(device_name.to_string()),
        jti: None,
        sub: Some(user_name.to_string()),
        appid: Some("aicc".to_string()),
        sudo: false,
        extra: HashMap::new(),
    }
    .generate_jwt(None, private_key)
    .map_err(|err| SnUserLoginError::fatal(format!("generate SN device token failed: {err}")))
}

pub async fn login_sn_user_by_device_token(
    client: &Client,
    login_url: &str,
    device_token: &str,
) -> Result<SnUserSession, SnUserLoginError> {
    let response = client
        .post(login_url)
        .json(&json!({ "device_token": device_token }))
        .send()
        .await
        .map_err(|err| {
            SnUserLoginError::retryable(format!("SN user login request failed: {err}"))
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|err| {
        SnUserLoginError::retryable(format!("read SN user login response failed: {err}"))
    })?;
    if !status.is_success() {
        let message = format!(
            "SN user login failed with status {}: {}",
            status.as_u16(),
            body.chars().take(320).collect::<String>()
        );
        return Err(if status.is_server_error() || status.as_u16() == 429 {
            SnUserLoginError::retryable(message)
        } else {
            SnUserLoginError::fatal(message)
        });
    }
    let envelope = serde_json::from_str::<SnUserLoginEnvelope>(body.as_str())
        .map_err(|err| SnUserLoginError::fatal(format!("invalid SN user login response: {err}")))?;
    if envelope.code != 0 {
        return Err(SnUserLoginError::fatal(format!(
            "SN user login rejected: code={} msg={}",
            envelope.code, envelope.msg
        )));
    }
    let session = envelope
        .data
        .ok_or_else(|| SnUserLoginError::fatal("SN user login response has no data"))?;
    if session.session_token.trim().is_empty() || session.expires_in == 0 {
        return Err(SnUserLoginError::fatal(
            "SN user login response contains an invalid session",
        ));
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
    use name_lib::generate_ed25519_key_pair;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct DeviceClaims {
        iss: String,
        sub: String,
        exp: u64,
        aud: Option<String>,
    }

    #[test]
    fn device_token_matches_sn_user_login_contract() {
        let (private_pem, public_jwk) = generate_ed25519_key_pair();
        let private_key = jsonwebtoken::EncodingKey::from_ed_pem(private_pem.as_bytes()).unwrap();
        let token =
            generate_sn_user_device_token_with_identity("alice", "ood1", &private_key, 1_000)
                .unwrap();
        assert_eq!(decode_header(token.as_str()).unwrap().alg, Algorithm::EdDSA);
        let public_x = public_jwk
            .get("x")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        let key = DecodingKey::from_ed_components(public_x).unwrap();
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = false;
        validation.validate_aud = false;
        let claims = decode::<DeviceClaims>(token.as_str(), &key, &validation)
            .unwrap()
            .claims;
        assert_eq!(claims.iss, "ood1");
        assert_eq!(claims.sub, "alice");
        assert_eq!(claims.exp, 1_900);
        assert_eq!(claims.aud, None);
    }
}
