pub use sfo_result::err as smb_err;
pub use sfo_result::into_err as into_smb_err;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum SmbErrorCode {
    Failed,
    CmdReturnFailed,
    LoadSmbConfFailed,
    ListUserFailed,
    SessionTokenNotFound,
}

pub type SmbResult<T> = sfo_result::Result<T, SmbErrorCode>;
pub type SmbError = sfo_result::Error<SmbErrorCode>;
