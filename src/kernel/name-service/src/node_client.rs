use serde::{Deserialize, Serialize};
use crate::NSResult;

#[async_trait::async_trait]
pub trait NSNodeClient {
    async fn call<'a, P: Serialize + Send + Sync, R: Deserialize<'a>>(&self, cmd_name: &str, p: P) -> NSResult<R>;
}
