//! Outbound credentials of Self-host Group Hooks (self-host-group-extension
//! TODO D-3): every `HookBinding` carries a secret the group host generated;
//! each request is signed with HMAC-SHA256 over `"<timestamp>.<body>"` and the
//! App verifies it with the same secret, without any BuckyOS runtime.

use sha2::{Digest, Sha256};

pub const GROUP_HOOK_TIMESTAMP_HEADER: &str = "x-buckyos-hook-timestamp";
pub const GROUP_HOOK_SIGNATURE_HEADER: &str = "x-buckyos-hook-signature";
/// A request whose timestamp is further than this from the receiver's clock is stale.
pub const GROUP_HOOK_MAX_SKEW_SECS: u64 = 300;
pub const GROUP_HOOK_SECRET_BYTES: usize = 32;

/// base64url of `bytes` random bytes.
pub fn random_secret(bytes: usize) -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut buffer = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buffer);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buffer)
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block_key = [0u8; BLOCK];
    if key.len() > BLOCK {
        block_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block_key.iter().map(|b| b ^ 0x36).collect::<Vec<_>>());
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(block_key.iter().map(|b| b ^ 0x5c).collect::<Vec<_>>());
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// Lowercase hex of HMAC-SHA256(secret, `"<timestamp>." + body`).
pub fn group_hook_signature(secret: &str, timestamp: u64, body: &[u8]) -> String {
    let mut message = timestamp.to_string().into_bytes();
    message.push(b'.');
    message.extend_from_slice(body);
    hmac_sha256(secret.as_bytes(), &message)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The receiver's check: a well-formed timestamp within the skew window and a
/// matching signature.
pub fn verify_group_hook_signature(
    secret: &str,
    timestamp: &str,
    body: &[u8],
    signature: &str,
    now: u64,
) -> bool {
    let Ok(timestamp) = timestamp.trim().parse::<u64>() else {
        return false;
    };
    if now.abs_diff(timestamp) > GROUP_HOOK_MAX_SKEW_SECS {
        return false;
    }
    crate::constant_time_eq(
        group_hook_signature(secret, timestamp, body).as_bytes(),
        signature.trim().to_ascii_lowercase().as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc4231_case_2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            mac.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn signatures_bind_timestamp_body_and_secret_within_the_skew_window() {
        let body = br#"{"operation":"post"}"#;
        let signature = group_hook_signature("s3cret", 1_000, body);
        assert!(verify_group_hook_signature("s3cret", "1000", body, &signature, 1_100));
        assert!(verify_group_hook_signature(
            "s3cret",
            "1000",
            body,
            &signature.to_ascii_uppercase(),
            1_000 + GROUP_HOOK_MAX_SKEW_SECS
        ));
        assert!(!verify_group_hook_signature("s3cret", "1000", body, &signature, 1_000 + GROUP_HOOK_MAX_SKEW_SECS + 1));
        assert!(!verify_group_hook_signature("s3cret", "1001", body, &signature, 1_000));
        assert!(!verify_group_hook_signature("other", "1000", body, &signature, 1_000));
        assert!(!verify_group_hook_signature("s3cret", "1000", b"{}", &signature, 1_000));
        assert!(!verify_group_hook_signature("s3cret", "abc", body, &signature, 1_000));
        assert_eq!(random_secret(GROUP_HOOK_SECRET_BYTES).len(), 43);
        assert_ne!(random_secret(8), random_secret(8));
    }
}
