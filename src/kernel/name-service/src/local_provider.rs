use std::path::Path;
use crate::{NameInfo, NSCmdRegister, NSErrorCode, NSProvider, NSResult};
use crate::error::{into_ns_err, ns_err};

pub struct LocalProvider {
    local_path: String,
}

impl LocalProvider {
    pub fn new(local_path: String) -> LocalProvider {
        LocalProvider {
            local_path,
        }
    }
}

#[async_trait::async_trait]
impl NSProvider for LocalProvider {
    async fn load(&self, _cmd_register: &NSCmdRegister) -> NSResult<()> {
        Ok(())
    }

    async fn query(&self, name: &str) -> NSResult<NameInfo> {
        let path = format!("{}/{}", self.local_path, name);
        if !Path::new(path.as_str()).exists() {
            return Err(ns_err!(NSErrorCode::NotFound, "Name {} not found", name));
        }
        let content = tokio::fs::read_to_string(path.as_str()).await
            .map_err(into_ns_err!(NSErrorCode::READ_LOCAL_FILE_ERROR, "Failed to read local file {}", path))?;
        Ok(serde_json::from_str(content.as_str())
            .map_err(into_ns_err!(NSErrorCode::InvalidData, "Failed to parse json {}", content))?)
    }
}
