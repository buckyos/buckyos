use async_trait::async_trait;
use named_store::{ChunkLocalInfo, ChunkStoreState, ChunkWriteOutcome, NamedDataMgr, ObjectState};
use ndn_lib::{ChunkId, ChunkReader, NdnError, NdnResult, ObjId};
use ndn_toolkit::{NamedObjectStore, NdmClient, NdmClientConfig};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

use crate::BuckyOSRuntime;

#[derive(Clone)]
pub struct RuntimeNamedStore<'a> {
    backend: Backend<'a>,
}

#[derive(Clone)]
enum Backend<'a> {
    Local(NamedDataMgr),
    Remote(&'a BuckyOSRuntime),
}

async fn remote_client(runtime: &BuckyOSRuntime) -> NdnResult<NdmClient> {
    runtime
        .renew_token_from_verify_hub()
        .await
        .map_err(|e| NdnError::RemoteError(format!("named store authentication: {e}")))?;
    let token = runtime.get_session_token().await;
    if token.is_empty() {
        return Err(NdnError::RemoteError(
            "named store session token is missing".into(),
        ));
    }
    let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| NdnError::InvalidParam("invalid named store session token".into()))?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| NdnError::RemoteError(format!("named store HTTP client: {e}")))?;
    Ok(NdmClient::with_client(
        NdmClientConfig {
            base_url: runtime.named_store_base_url(),
        },
        client,
    ))
}

macro_rules! store_call {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            Backend::Local(store) => store.$method($($arg),*).await,
            Backend::Remote(runtime) => remote_client(runtime).await?.$method($($arg),*).await,
        }
    };
}

impl<'a> RuntimeNamedStore<'a> {
    pub(crate) fn local(store: NamedDataMgr) -> Self {
        Self {
            backend: Backend::Local(store),
        }
    }

    pub(crate) fn remote(runtime: &'a BuckyOSRuntime) -> Self {
        Self {
            backend: Backend::Remote(runtime),
        }
    }

    pub async fn get_object(&self, obj_id: &ObjId) -> NdnResult<String> {
        store_call!(self, get_object(obj_id))
    }

    pub async fn open_object(
        &self,
        obj_id: &ObjId,
        inner_path: Option<String>,
    ) -> NdnResult<String> {
        store_call!(self, open_object(obj_id, inner_path))
    }

    pub async fn get_dir_child(&self, obj_id: &ObjId, name: &str) -> NdnResult<ObjId> {
        store_call!(self, get_dir_child(obj_id, name))
    }

    pub async fn is_object_stored(
        &self,
        obj_id: &ObjId,
        inner_path: Option<String>,
    ) -> NdnResult<bool> {
        store_call!(self, is_object_stored(obj_id, inner_path))
    }

    pub async fn is_object_exist(&self, obj_id: &ObjId) -> NdnResult<bool> {
        store_call!(self, is_object_exist(obj_id))
    }

    pub async fn query_object_by_id(&self, obj_id: &ObjId) -> NdnResult<ObjectState> {
        store_call!(self, query_object_by_id(obj_id))
    }

    pub async fn put_object(&self, obj_id: &ObjId, data: &str) -> NdnResult<()> {
        store_call!(self, put_object(obj_id, data))
    }

    pub async fn open_reader(
        &self,
        obj_id: &ObjId,
        inner_path: Option<String>,
    ) -> NdnResult<(ChunkReader, u64)> {
        store_call!(self, open_reader(obj_id, inner_path))
    }

    pub async fn open_chunk_reader(
        &self,
        chunk_id: &ChunkId,
        offset: u64,
    ) -> NdnResult<(ChunkReader, u64)> {
        store_call!(self, open_chunk_reader(chunk_id, offset))
    }

    pub async fn open_chunklist_reader(
        &self,
        obj_id: &ObjId,
        offset: u64,
    ) -> NdnResult<(ChunkReader, u64)> {
        store_call!(self, open_chunklist_reader(obj_id, offset))
    }

    pub async fn query_chunk_state(&self, chunk_id: &ChunkId) -> NdnResult<(ChunkStoreState, u64)> {
        store_call!(self, query_chunk_state(chunk_id))
    }

    pub async fn have_chunk(&self, chunk_id: &ChunkId) -> bool {
        self.is_object_stored(&chunk_id.to_obj_id(), None)
            .await
            .unwrap_or(false)
    }

    pub async fn get_chunk_data(&self, chunk_id: &ChunkId) -> NdnResult<Vec<u8>> {
        store_call!(self, get_chunk_data(chunk_id))
    }

    pub async fn put_chunk(&self, chunk_id: &ChunkId, data: &[u8]) -> NdnResult<()> {
        store_call!(self, put_chunk(chunk_id, data))
    }

    pub async fn put_chunk_by_reader(
        &self,
        chunk_id: &ChunkId,
        size: u64,
        reader: ChunkReader,
    ) -> NdnResult<ChunkWriteOutcome> {
        store_call!(self, put_chunk_by_reader(chunk_id, size, reader))
    }

    pub async fn add_chunk_by_link_to_local_file(
        &self,
        chunk_id: &ChunkId,
        size: u64,
        info: &ChunkLocalInfo,
    ) -> NdnResult<()> {
        store_call!(self, add_chunk_by_link_to_local_file(chunk_id, size, info))
    }
}

#[async_trait]
impl NamedObjectStore for RuntimeNamedStore<'_> {
    async fn get_object(&self, obj_id: &ObjId) -> NdnResult<String> {
        self.get_object(obj_id).await
    }

    async fn have_chunk(&self, chunk_id: &ChunkId) -> bool {
        self.have_chunk(chunk_id).await
    }

    async fn put_object(&self, obj_id: &ObjId, data: &str) -> NdnResult<()> {
        self.put_object(obj_id, data).await
    }

    async fn put_chunk(&self, chunk_id: &ChunkId, data: &[u8]) -> NdnResult<()> {
        self.put_chunk(chunk_id, data).await
    }

    async fn add_chunk_by_link_to_local_file(
        &self,
        chunk_id: &ChunkId,
        size: u64,
        info: &ChunkLocalInfo,
    ) -> NdnResult<()> {
        self.add_chunk_by_link_to_local_file(chunk_id, size, info)
            .await
    }
}
