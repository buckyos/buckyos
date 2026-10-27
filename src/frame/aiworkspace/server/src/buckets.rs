//! Bindings through nfs-server's kRPC `bucket.*` (a Zone, or a standalone nfs-server with `--bucket-specs`).

use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::objects::Buckets;
use buckyos_api::bucket_client::{bucket_error_code, is_no_such_key, Action, BucketClient, BucketId, PutRequest, TokenRequest};
use kRPC::RPCErrors;

pub enum ZoneBuckets {
    /// The service's own session, taken from the runtime for each call (it renews itself).
    Runtime,
    /// A fixed endpoint (`<nfs-server>/kapi/nfs-server`).
    Fixed(BucketClient),
}

fn err(e: RPCErrors) -> WsError {
    let code = match bucket_error_code(&e) {
        Some("quota_exceeded" | "object_too_large") => Code::StorageFull,
        Some("content_not_available" | "content_evicted" | "not_held") => Code::DependencyUnavailable,
        Some("no_such_key" | "bucket_not_found") => Code::NotFound,
        _ => Code::StorageIoError,
    };
    WsError::new(code, format!("bucket: {e}"))
}

fn id(bucket: &str) -> WsResult<BucketId> {
    bucket.parse().map_err(|_| WsError::invalid_op(format!("bad bucket id {bucket}")))
}

impl ZoneBuckets {
    async fn session(&self) -> WsResult<Option<BucketClient>> {
        match self {
            ZoneBuckets::Fixed(_) => Ok(None),
            ZoneBuckets::Runtime => {
                let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| WsError::io(e.to_string()))?;
                Ok(Some(runtime.get_bucket_client().await.map_err(err)?))
            }
        }
    }

    fn client<'a>(&'a self, session: &'a Option<BucketClient>) -> &'a BucketClient {
        match (self, session) {
            (_, Some(c)) | (ZoneBuckets::Fixed(c), None) => c,
            (ZoneBuckets::Runtime, None) => unreachable!("the runtime session is taken first"),
        }
    }
}

#[async_trait::async_trait]
impl Buckets for ZoneBuckets {
    async fn put(&self, bucket: &str, key: &str, obj_id: &str) -> WsResult<u64> {
        let req = PutRequest { key: key.to_string(), obj_id: obj_id.to_string(), ..Default::default() };
        let session = self.session().await?;
        Ok(self.client(&session).put(&id(bucket)?, &req).await.map_err(err)?.version)
    }

    async fn get(&self, bucket: &str, key: &str) -> WsResult<Option<(String, u64)>> {
        let session = self.session().await?;
        match self.client(&session).get(&id(bucket)?, key, None).await {
            Ok(b) if b.is_delete_marker => Ok(None),
            Ok(b) => Ok(b.obj_id.map(|o| (o, b.version))),
            Err(e) if is_no_such_key(&e) => Ok(None),
            Err(e) => Err(err(e)),
        }
    }

    async fn delete(&self, bucket: &str, key: &str) -> WsResult<()> {
        let session = self.session().await?;
        match self.client(&session).delete(&id(bucket)?, key, None).await {
            Ok(_) => Ok(()),
            Err(e) if is_no_such_key(&e) => Ok(()),
            Err(e) => Err(err(e)),
        }
    }

    async fn read_token(&self, bucket: &str, key: &str, ttl_ms: i64) -> WsResult<String> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let req = TokenRequest {
            prefix: None,
            key: Some(key.to_string()),
            actions: [Action::Read].into_iter().collect(),
            expires_at: now + ttl_ms,
            max_uses: None,
            bind_user_id: None,
            bind_app_id: None,
        };
        let session = self.session().await?;
        Ok(self.client(&session).create_access_token(&id(bucket)?, &req).await.map_err(err)?.token)
    }
}
