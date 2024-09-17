use serde::{Deserialize, Serialize};
use sfo_http::http_util::{HttpClient};
use sfo_serde_result::SerdeResult;
use crate::{NSError, NSErrorCode, NSNodeClient, NSResult};
use crate::error::ns_err;

pub struct HttpNSNodeClient {
    http_client: HttpClient,
}

impl HttpNSNodeClient {
    // pub fn new(url: &str, ca: ) -> Self {
    //     let builder = HttpClientBuilder::default()
    //         .https_only(true).add_root_certificate().identity().build();
    // }
}

#[async_trait::async_trait]
impl NSNodeClient for HttpNSNodeClient {
    async fn call<P: Serialize + Send + Sync, R: for<'a> Deserialize<'a> + Serialize>(&self, cmd_name: &str, p: P) -> NSResult<R> {
        let resp: SerdeResult<R, NSError> = self.http_client.post_json(cmd_name, &p).await.map_err(|e| {
            ns_err!(NSErrorCode::InvalidData, "Failed to call http node server: {:?}", e)
        })?;

        resp.into()
    }
}
