use sfo_result::{Error, Result};
pub(crate) use sfo_result::err as tun_err;
pub(crate) use sfo_result::into_err as into_tun_err;

#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum TunErrorCode {
    #[default]
    Failed,
    IoError,
    InvalidParam,
    RawCodecError,
    ConnectFailed,
    TunError,
    Timeout,
}

pub type TunResult<T> = Result<T, TunErrorCode>;
pub type TunError = Error<TunErrorCode>;
