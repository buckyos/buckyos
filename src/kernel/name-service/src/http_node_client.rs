use serde::{Deserialize, Serialize};
use sfo_http::http_util::{HttpClient};
use crate::{NSNodeClient, NSResult};

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
    async fn call<'a, P: Serialize + Send + Sync, R: Deserialize<'a>>(&self, cmd_name: &str, p: P) -> NSResult<R> {
        todo!()
    }
}
