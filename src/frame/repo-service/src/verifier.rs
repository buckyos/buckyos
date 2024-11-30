use crate::error::{RepoError, RepoResult};
use ed25519_dalek::{Signature, Verifier as EdVerifier, VerifyingKey};
use name_client::*;

pub struct Verifier {}

impl Verifier {
    pub async fn verify(author: &str, chunk_id: &str, sign_base64: &str) -> RepoResult<()> {
        let (auth_key, remote_did_id) = resolve_ed25519_auth_key(author).await.map_err(|e| {
            RepoError::VerifyError(format!(
                "resolve_ed25519_auth_key failed, author: {}, {:?}",
                author, e
            ))
        })?;
        //verify sign
        let public_key = VerifyingKey::from_bytes(&auth_key).map_err(|e| {
            RepoError::VerifyError(format!(
                "invalid public key, author: {}, error: {:?}",
                author, e
            ))
        })?;

        let sign_bytes = base64::decode(sign_base64).map_err(|e| {
            RepoError::VerifyError(format!(
                "base64 decode sign failed, sign: {}, error: {:?}",
                sign_base64, e
            ))
        })?;

        // 检查字节数组的长度是否为 64
        if sign_bytes.len() != 64 {
            return Err(RepoError::VerifyError(format!(
                "invalid signature length, expected 64 bytes, got {} bytes",
                sign_bytes.len()
            )));
        }

        let signature = Signature::from_bytes(&sign_bytes.try_into().map_err(|e| {
            RepoError::VerifyError(format!("conversion to fixed-size array failed: {:?}", e))
        })?);

        public_key
            .verify(chunk_id.as_bytes(), &signature)
            .map_err(|e| RepoError::VerifyError(format!("verify failed, error: {:?}", e)))?;

        Ok(())
    }
}
