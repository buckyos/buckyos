use bucky_raw_codec::{RawDecode, RawEncode, RawFixedBytes};
use crate::error::{TunError, TunErrorCode, TunResult};

#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd)]
pub enum CmdCode {
    HelloReq = 1,
    HelloResp = 2,
    Heart = 3,
    HeartResp = 4,
    Data = 0x10,
}

impl TryFrom<u8> for CmdCode {
    type Error = TunError;
    fn try_from(v: u8) -> std::result::Result<Self, Self::Error> {
        match v {
            1u8 => Ok(Self::HelloReq),
            2u8 => Ok(Self::HelloResp),
            3u8 => Ok(Self::Heart),
            4u8 => Ok(Self::HeartResp),
            0x10u8 => Ok(Self::Data),

            _ => Err(TunError::new(
                TunErrorCode::InvalidParam,
                format!("invalid command type value {}", v),
            )),
        }
    }
}

#[derive(RawDecode, RawEncode)]
pub struct HelloReq {
    pub client_key: String,
}

#[derive(RawDecode, RawEncode)]
pub struct HelloResp {
    pub client_ip: Option<String>,
}

#[derive(RawDecode, RawEncode)]
pub struct CmdHeader {
    pkg_len: u16,
    cmd_code: u8
}

impl CmdHeader {
    pub fn new(cmd_code: CmdCode, pkg_len: u16) -> Self {
        Self {
            pkg_len,
            cmd_code: cmd_code as u8
        }
    }

    pub fn cmd_code(&self) -> TunResult<CmdCode> {
        CmdCode::try_from(self.cmd_code)
    }

    pub fn pkg_len(&self) -> u16 {
        self.pkg_len
    }

    pub fn set_pkg_len(&mut self, pkg_len: u16) {
        self.pkg_len = pkg_len;
    }
}

impl RawFixedBytes for CmdHeader {
    fn raw_bytes() -> Option<usize> {
        Some(u16::raw_bytes().unwrap() + u8::raw_bytes().unwrap())
    }
}

#[derive(RawDecode, RawEncode)]
pub struct Cmd<T> {
    header: CmdHeader,
    body: T,
}

impl <T: RawEncode> Cmd<T> {
    pub fn new(cmd_code: CmdCode, body: T) -> Self {
        Self {
            header: CmdHeader::new(cmd_code, body.raw_measure(&None).unwrap() as u16),
            body
        }
    }

    pub fn cmd_code(&self) -> TunResult<CmdCode> {
        self.header.cmd_code()
    }

    pub fn body(&self) -> &T {
        &self.body
    }
}
