use serde::{Deserialize, Serialize};

pub(crate) use sfo_result::err as ns_err;
pub type NSResult<T> = sfo_result::Result<T>;

#[repr(u16)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
pub enum NSErrorCode {
    #[default]
    Failed,
    InvalidData,
    NotFound,
    DnsTxtEncodeError,
}

pub type NSError = sfo_result::Error<NSErrorCode>;
