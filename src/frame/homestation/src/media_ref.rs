//! The bucket binding of a held FileObject, for media readers that want the
//! nfs-server data plane (Range, `bkt_` tokens) instead of
//! `/home/<user>/objects/<id>/content` (preview iPhone media TODO D9): own
//! media are bound in `published` (or still in `scratch`), media of followed
//! entries in `following`. Bindings are found by listing those buckets; the
//! index is rebuilt at most every 10 s on a miss and each hit is re-checked.

use crate::error::{HsError, HsResult};
use crate::protocol::normalize_obj_id;
use crate::Station;
use buckyos_api::bucket_client::{Action, Availability, BucketId, ListRequest, TokenRequest};
use buckyos_api::{HOMESTATION_BUCKET_FOLLOWING, HOMESTATION_BUCKET_PUBLISHED, HOMESTATION_BUCKET_SCRATCH};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const TOKEN_TTL_MS: i64 = 6 * 3600 * 1000;
const REBUILD_EVERY: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Index {
    built: Option<Instant>,
    files: HashMap<String, (BucketId, String)>,
}

fn indexes() -> &'static Mutex<HashMap<String, Index>> {
    static INDEXES: OnceLock<Mutex<HashMap<String, Index>>> = OnceLock::new();
    INDEXES.get_or_init(Default::default)
}

/// Issued read tokens by `<bucket>/<key>`, reused while more than half their life remains.
fn tokens() -> &'static Mutex<HashMap<String, (String, i64)>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, (String, i64)>>> = OnceLock::new();
    TOKENS.get_or_init(Default::default)
}

fn bucket_err(e: impl std::fmt::Display) -> HsError {
    HsError::Unavailable(format!("bucket: {e}"))
}

impl Station {
    async fn media_bindings(&self) -> HsResult<HashMap<String, (BucketId, String)>> {
        let client = self.store.buckets().await?;
        let mut files = HashMap::new();
        for name in [HOMESTATION_BUCKET_PUBLISHED, HOMESTATION_BUCKET_FOLLOWING, HOMESTATION_BUCKET_SCRATCH] {
            let bucket = self.bucket(name)?;
            let mut start_after = None;
            loop {
                let req = ListRequest { start_after, ..Default::default() };
                let page = client.list_objects(&bucket, &req).await.map_err(bucket_err)?;
                for b in page.items {
                    if let Some(id) = b.obj_id.as_deref().map(normalize_obj_id) {
                        files.entry(id).or_insert_with(|| (bucket.clone(), b.key));
                    }
                }
                match page.next_start_after {
                    Some(next) => start_after = Some(next),
                    None => break,
                }
            }
        }
        Ok(files)
    }

    /// `{bucketId, key, version, token, mime, size}` of a held FileObject, or None when unbound.
    pub async fn content_ref(&self, file_id: &str) -> HsResult<Option<Value>> {
        let id = normalize_obj_id(file_id);
        let owner = self.cfg.owner.clone();
        let client = self.store.buckets().await?;
        for round in 0..2 {
            let found = indexes().lock().unwrap().get(&owner).and_then(|i| i.files.get(&id).cloned());
            if let Some((bucket, key)) = found {
                match client.head(&bucket, &key, None).await {
                    Ok((b, Availability::Local)) if !b.is_delete_marker && b.obj_id.as_deref().map(normalize_obj_id).as_deref() == Some(id.as_str()) => {
                        let now = buckyos_api::bucket_client::now_ms();
                        let slot = format!("{bucket}/{key}");
                        let cached = tokens().lock().unwrap().get(&slot).filter(|(_, exp)| exp - now > TOKEN_TTL_MS / 2).map(|(t, _)| t.clone());
                        let token = match cached {
                            Some(t) => t,
                            None => {
                                let req = TokenRequest {
                                    prefix: None,
                                    key: Some(key.clone()),
                                    actions: [Action::Read].into_iter().collect(),
                                    expires_at: now + TOKEN_TTL_MS,
                                    max_uses: None,
                                    bind_user_id: None,
                                    bind_app_id: None,
                                };
                                let issued = client.create_access_token(&bucket, &req).await.map_err(bucket_err)?;
                                let mut map = tokens().lock().unwrap();
                                map.retain(|_, (_, exp)| *exp > now);
                                map.insert(slot, (issued.token.clone(), now + TOKEN_TTL_MS));
                                issued.token
                            }
                        };
                        return Ok(Some(json!({
                            "bucketId": bucket.to_string(),
                            "key": key,
                            "version": b.version,
                            "token": token,
                            "mime": b.content_type,
                            "size": b.size,
                        })));
                    }
                    _ => {
                        if let Some(index) = indexes().lock().unwrap().get_mut(&owner) {
                            index.files.remove(&id);
                        }
                    }
                }
            }
            let stale = indexes().lock().unwrap().get(&owner).and_then(|i| i.built).is_none_or(|at| at.elapsed() >= REBUILD_EVERY);
            if round == 1 || !stale {
                break;
            }
            let files = self.media_bindings().await?;
            indexes().lock().unwrap().insert(owner.clone(), Index { built: Some(Instant::now()), files });
        }
        Ok(None)
    }
}
