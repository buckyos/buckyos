use std::path::PathBuf;

use crate::def::*;
use base64::{engine::general_purpose, Engine as _};
use ed25519_dalek::{pkcs8::DecodePrivateKey, Signature, Signer, SigningKey};
use ed25519_dalek::{Verifier as EdVerifier, VerifyingKey};
use log::info;
use name_client::*;

pub async fn verify(author: &str, chunk_id: &str, sign_base64: &str) -> RepoResult<()> {
    //TODO
    return Ok(());
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

    let sign_bytes = general_purpose::STANDARD.decode(sign_base64).map_err(|e| {
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

    info!(
        "verify success, author: {}, chunk_id: {}, sign: {}",
        author, chunk_id, sign_base64
    );

    Ok(())
}

pub fn sign_data(pem_file: &str, data: &str) -> RepoResult<String> {
    let signing_key = SigningKey::read_pkcs8_pem_file(pem_file).map_err(|e| {
        RepoError::LoadError(
            pem_file.to_string(),
            format!("read pkcs8 pem file failed: {:?}", e),
        )
    })?;

    let signature: Signature = signing_key.sign(data.as_bytes());

    // convert signature to base64
    let signature_base64 = general_purpose::STANDARD.encode(signature.to_bytes());

    Ok(signature_base64)
}
