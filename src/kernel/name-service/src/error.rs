use serde::{Deserialize, Serialize};

pub(crate) use sfo_result::err as ns_err;
pub(crate) use sfo_result::into_err as into_ns_err;

#[repr(u16)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
pub enum NSErrorCode {
    #[default]
    Failed,
    InvalidData,
    NotFound,
    DnsTxtEncodeError,
    InvalidParam,
    Forbid,
}

pub type NSError = sfo_result::Error<NSErrorCode>;
pub type NSResult<T> = sfo_result::Result<T, NSErrorCode>;
