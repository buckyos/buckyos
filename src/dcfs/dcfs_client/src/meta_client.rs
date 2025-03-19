use thiserror::Error;

#[derive(Error, Debug)]
pub enum MetaClientError {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("File not found: {0}")]
    FileNotFoundError(String),
    // 其他错误类型
}

type Result<T> = std::result::Result<T, MetaClientError>;

pub struct MetaInfo {

}

pub struct MetaClient {
    device_id : String,
}

impl MetaClient {
    pub fn new() -> MetaClient {
        MetaClient {}
    }

    pub async fn read_meta(&self, path : String) -> Result<MetaInfo> {
        // 1) use disk_map locate the file:remote or local
        // 2) try read meta from disk or meta_server ,if all success, return the meta
        // 3) if read from disk or meta_server failed, return error
    }
}