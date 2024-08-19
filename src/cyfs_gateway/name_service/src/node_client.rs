use serde::{Deserialize, Serialize};
use crate::NSResult;

#[async_trait::async_trait]
pub trait NSNodeClient {
    async fn call<P: Serialize + Send + Sync, R: for<'a> Deserialize<'a>+ Serialize>(&self, cmd_name: &str, p: P) -> NSResult<R>;
}
