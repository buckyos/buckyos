use thiserror::Error;

#[derive(Error, Debug)]
pub enum NSError {
    #[error("Failed: {0}")]
    Failed(String),
    #[error("Invalid response")]
    InvalidData,
    #[error("{0} not found")]
    NotFound(String),
    #[error("decode txt record error")]
    DnsTxtEncodeError,
    #[error("forbidden")]
    Forbid,
    #[error("DNS protocl error: {0}")]
    DNSProtoError(String),
    #[error("Failed to serialize extra: {0}")]
    ReadLocalFileError(String),
}

pub type NSResult<T> = Result<T, NSError>;

pub fn is_did(identifier: &str) -> bool {
    if identifier.starts_with("did:") {
        let parts: Vec<&str> = identifier.split(':').collect();
        return parts.len() == 3 && !parts[1].is_empty() && !parts[2].is_empty();
    }
    false
}

