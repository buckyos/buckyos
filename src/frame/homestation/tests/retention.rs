//! Posts hold their files in the zone named store independently of the share they came from
//! (share design D17): pins before publishing, release on withdraw, shared files, failures
//! that are retried, backfill of older posts, and real GC over an isolated store.

use async_trait::async_trait;
use homestation::error::{HsError, HsResult};
use homestation::objects::{ChunkStore, NdmChunkStore};
use homestation::protocol::{obj_id_of, OBJ_TYPE_FILE};
use homestation::standalone::{build_with_store, StandaloneConfig};
use homestation::Station;
use named_store::{NamedDataMgr, NamedLocalStore, PinScope, StoreLayout, StoreTarget};
use ndn_lib::{NdnError, ObjId};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The real store, with switches to make pins or releases fail.
struct FlakyStore {
    inner: NdmChunkStore,
    fail_retain: AtomicBool,
    fail_release: AtomicBool,
}

#[async_trait]
impl ChunkStore for FlakyStore {
    async fn put_chunk(&self, chunk_id: &str, data: Vec<u8>) -> HsResult<()> {
        self.inner.put_chunk(chunk_id, data).await
    }
    async fn get_chunk(&self, chunk_id: &str) -> HsResult<Option<Vec<u8>>> {
        self.inner.get_chunk(chunk_id).await
    }
    async fn has_chunk(&self, chunk_id: &str) -> bool {
        self.inner.has_chunk(chunk_id).await
    }
    async fn get_named_object(&self, obj_id: &str) -> HsResult<Option<String>> {
        self.inner.get_named_object(obj_id).await
    }
    async fn put_named_object(&self, obj_id: &str, body: &str) -> HsResult<()> {
        self.inner.put_named_object(obj_id, body).await
    }
    async fn retain(&self, obj_id: &str, owner: &str) -> HsResult<()> {
        if self.fail_retain.load(Ordering::SeqCst) {
            return Err(HsError::Internal("named store pin: injected failure".into()));
        }
        self.inner.retain(obj_id, owner).await
    }
    async fn release(&self, obj_id: &str, owner: &str) -> HsResult<()> {
        if self.fail_release.load(Ordering::SeqCst) {
            return Err(HsError::Internal("named store unpin: injected failure".into()));
        }
        self.inner.release(obj_id, owner).await
    }
}

async fn open_store(dir: &std::path::Path) -> NamedDataMgr {
    let store = NamedLocalStore::get_named_store_by_path(dir.to_path_buf()).await.unwrap();
    let store_id = store.store_id().to_string();
    let mgr = NamedDataMgr::new();
    mgr.register_store(Arc::new(tokio::sync::Mutex::new(store))).await;
    let target = StoreTarget { store_id, device_did: String::new(), capacity: None, used: None, readonly: false, enabled: true, weight: 1 };
    mgr.add_layout(StoreLayout::new(1, vec![target], 0, 0)).await;
    mgr
}

/// One GC pass as a deployment would run it: deliver pending pin cascades, then evict.
async fn run_gc(mgr: &NamedDataMgr) -> named_store::GcReport {
    let id = mgr.get_store_ids().await.pop().unwrap();
    let store = mgr.get_store(&id).await.unwrap();
    let store = store.lock().await;
    loop {
        let batch = store.fetch_outbox_ready(100).await.unwrap();
        if batch.is_empty() {
            break;
        }
        for entry in batch {
            store.apply_edge(entry.msg.clone()).await.unwrap();
            store.complete_outbox_entry(entry.seq).await.unwrap();
        }
    }
    store.gc_round(0).await.unwrap()
}

/// A file as nfs-server's share snapshot leaves it: chunk + FileObject, pinned by the share.
async fn snapshot(mgr: &NamedDataMgr, name: &str, data: &[u8]) -> (ObjId, ObjId) {
    let chunk = ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(data).unwrap();
    mgr.put_chunk(&chunk, data).await.unwrap();
    let mut file = ndn_lib::FileObject::new(name.to_string(), data.len() as u64, chunk.to_string());
    file.meta.insert("mime".into(), json!("image/png"));
    let (id, jcs) = obj_id_of(OBJ_TYPE_FILE, &serde_json::to_value(&file).unwrap()).unwrap();
    let id = ObjId::new(&id).unwrap();
    mgr.put_object(&id, &jcs).await.unwrap();
    mgr.pin(&id, "share:s1", PinScope::Recursive, None).await.unwrap();
    (id, chunk.to_obj_id())
}

fn post(object: &ObjId) -> homestation::publish::PublishInput {
    serde_json::from_value(json!({
        "text": "from my files",
        "attachments": [{ "id": "a1", "kind": "image", "name": "photo.png", "status": "uploaded", "object": object.to_string() }],
        "link": null,
        "audience": { "kind": "public" }
    }))
    .unwrap()
}

async fn publish(station: &Station, key: &str, object: &ObjId) -> String {
    station.register_object(&object.to_string(), json!({"mime":"image/png"})).await.unwrap();
    let row = station.publish(key, post(object)).await.unwrap();
    assert_eq!(row.stage, "published", "{row:?}");
    row.entry.unwrap()
}

fn owner(entry: &str) -> String {
    format!("homestation:{entry}")
}

async fn holds(mgr: &NamedDataMgr, obj: &ObjId, entry: &str) -> bool {
    match mgr.anchor_state(obj, &owner(entry)).await {
        Ok(_) => true,
        Err(NdnError::NotFound(_)) => false,
        Err(e) => panic!("anchor_state: {e}"),
    }
}

async fn reconcile(station: &Station) -> Value {
    serde_json::to_value(station.reconcile_retentions().await.unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn posts_hold_their_files_until_withdrawn() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = open_store(&tmp.path().join("store")).await;
    let store = Arc::new(FlakyStore { inner: NdmChunkStore { ndm: mgr.clone() }, fail_retain: AtomicBool::new(false), fail_release: AtomicBool::new(false) });
    let (pem, _) = homestation::sign::generate_key();
    let cfg: StandaloneConfig = serde_json::from_value(json!({
        "owner": "did:test:alice", "owner_name": "alice", "user": "alice", "zone": "alice.test",
        "private_key_pem": pem, "workers": false, "spider": false,
        "tokens": { "tok-alice": { "principal": "alice", "did": "did:test:alice", "app_id": "control-panel" } }
    }))
    .unwrap();
    let hs = build_with_store(cfg, &tmp.path().join("hs"), None, None, store.clone()).unwrap();
    let station = hs.station.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(homestation::http::serve_listener(hs.state.clone(), listener));
    let http = reqwest::Client::new();
    let content = |obj: &ObjId| http.get(format!("{base}/home/alice/objects/{obj}/content")).send();

    let photo = vec![9u8; 5000];
    let (obj, chunk) = snapshot(&mgr, "photo.png", &photo).await;

    // Published: held under the post's own owner, independent of the share.
    let first = publish(&station, "p1", &obj).await;
    assert!(holds(&mgr, &obj, &first).await);
    assert_eq!(station.retention_status().await.unwrap()["held"], 1);
    mgr.unpin(&obj, "share:s1").await.unwrap();
    let report = run_gc(&mgr).await;
    assert_eq!(report.evicted_objects, 0, "{report:?}");
    assert!(mgr.is_object_exist(&obj).await.unwrap());
    assert_eq!(mgr.debug_dump_expand_state(&chunk).await.unwrap().eviction_class, 1, "the content chunk is protected");
    assert_eq!(content(&obj).await.unwrap().bytes().await.unwrap().to_vec(), photo);

    // Two posts share the file: withdrawing one keeps it for the other.
    let second = publish(&station, "p2", &obj).await;
    station.withdraw(&first).await.unwrap();
    assert!(!holds(&mgr, &obj, &first).await);
    assert!(holds(&mgr, &obj, &second).await);
    assert_eq!(run_gc(&mgr).await.evicted_objects, 0);
    assert_eq!(content(&obj).await.unwrap().bytes().await.unwrap().to_vec(), photo);

    // The last reference goes: the store may collect the file.
    station.withdraw(&second).await.unwrap();
    assert_eq!(station.retention_status().await.unwrap()["held"], 0);
    assert_eq!(run_gc(&mgr).await.evicted_objects, 1);
    assert!(!mgr.is_object_exist(&obj).await.unwrap());
    assert_eq!(mgr.debug_dump_expand_state(&chunk).await.unwrap().eviction_class, 0);

    // A failed pin fails the publish; the reconcile drops the half-made hold; a retry works.
    let (other, _) = snapshot(&mgr, "other.png", &[3u8; 700]).await;
    station.register_object(&other.to_string(), json!({"mime":"image/png"})).await.unwrap();
    store.fail_retain.store(true, Ordering::SeqCst);
    let failed = station.publish("p3", post(&other)).await.unwrap();
    assert_eq!(failed.stage, "failed");
    assert_eq!(station.retention_status().await.unwrap()["pending"], 1);
    reconcile(&station).await;
    assert_eq!(station.retention_status().await.unwrap()["pending"], 0);
    store.fail_retain.store(false, Ordering::SeqCst);
    let third = station.retry_publish("p3").await.unwrap();
    assert_eq!(third.stage, "published");
    let third = third.entry.unwrap();
    assert!(holds(&mgr, &other, &third).await);

    // A failed release keeps the hold recorded until a later reconcile finishes it.
    store.fail_release.store(true, Ordering::SeqCst);
    station.withdraw(&third).await.unwrap();
    assert!(holds(&mgr, &other, &third).await);
    assert_eq!(station.retention_status().await.unwrap()["releasing"], 1);
    assert_eq!(reconcile(&station).await["failed"], 1);
    store.fail_release.store(false, Ordering::SeqCst);
    assert_eq!(reconcile(&station).await["released"], 1);
    assert!(!holds(&mgr, &other, &third).await);

    // Backfill: a post published before holds were recorded gets them; missing content is reported.
    let (old, old_chunk) = snapshot(&mgr, "old.png", &[5u8; 900]).await;
    let fourth = publish(&station, "p4", &old).await;
    mgr.unpin(&old, &owner(&fourth)).await.unwrap();
    station.db.call(|c| Ok(c.execute("DELETE FROM retentions", [])?)).await.unwrap();
    mgr.remove_chunk(&ndn_lib::ChunkId::from_obj_id(&old_chunk)).await.unwrap();
    let report = reconcile(&station).await;
    assert_eq!(report["pinned"], 1, "{report}");
    assert!(holds(&mgr, &old, &fourth).await);
    let status = station.retention_status().await.unwrap();
    assert_eq!(status["issues"][0]["issue"], "content_missing", "{status}");
    assert_eq!(status["issues"][0]["objId"], old.to_string());
}
