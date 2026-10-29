//! Attachments and buckets (doc/homestation/HomeStation Durable Data Schema.md): an upload bound
//! in the user's `scratch` bucket is registered, a publication binds its version in `entries`
//! and its files in `published`, the audience reads them, a withdrawal unbinds both; a reader's
//! prepared copy of another publisher's post is kept in `following` / `following-entries` with
//! its origin until it leaves the reading list.

mod common;

use buckyos_api::bucket_client::{Binding, ListRequest, PutRequest};
use buckyos_api::{HOMESTATION_BUCKET_ENTRIES, HOMESTATION_BUCKET_FOLLOWING, HOMESTATION_BUCKET_FOLLOWING_ENTRIES, HOMESTATION_BUCKET_PUBLISHED, HOMESTATION_BUCKET_SCRATCH};
use common::*;
use homestation::objects::chunk_id_of;
use homestation::Station;
use ndn_lib::{ChunkId, ChunkList, DirObject, FileObject, ObjId};
use serde_json::json;

async fn named(station: &Station, id: &str, body: &str) {
    station.store.named().put_object(&ObjId::new(id).unwrap(), body).await.unwrap();
}

async fn chunk(station: &Station, bytes: &[u8]) -> String {
    let id = chunk_id_of(bytes).unwrap();
    station.store.named().put_chunk(&ChunkId::new(&id).unwrap(), bytes).await.unwrap();
    id
}

/// What the browser's `finalize` with `auto_bind` does.
async fn upload_binding(station: &Station, key: &str, obj_id: &str) {
    let req = PutRequest { key: key.into(), obj_id: obj_id.into(), ..Default::default() };
    station.store.buckets().await.unwrap().put(&station.bucket(HOMESTATION_BUCKET_SCRATCH).unwrap(), &req).await.unwrap();
}

async fn bound(station: &Station, bucket: &str, prefix: &str) -> Vec<Binding> {
    let req = ListRequest { prefix: prefix.into(), ..Default::default() };
    station.store.buckets().await.unwrap().list_objects(&station.bucket(bucket).unwrap(), &req).await.unwrap().items
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_are_published_into_buckets_and_withdrawn_from_them() {
    let net = Net::new(&["alice", "bob", "eve"], &[]).await;
    let alice = net.n("alice");
    let station = alice.station.as_ref();
    let pieces = [b"first part".to_vec(), b"second part".to_vec()];
    let mut chunks = Vec::new();
    for bytes in &pieces {
        chunks.push(ChunkId::new(&chunk(station, bytes).await).unwrap());
    }
    let (list_id, list_json) = ChunkList::from_chunk_list(chunks).unwrap().gen_obj_id();
    named(station, &list_id.to_string(), &list_json).await;
    let file = FileObject::new("同名.txt".into(), pieces.iter().map(|bytes| bytes.len() as u64).sum(), list_id.to_string());
    let body = serde_json::to_value(&file).unwrap();
    let (file_id, file_json) = homestation::protocol::obj_id_of(ndn_lib::OBJ_TYPE_FILE, &body).unwrap();
    named(station, &file_id, &file_json).await;
    let empty_chunk = chunk(station, b"").await;
    let empty_file = FileObject::new("empty.txt".into(), 0, empty_chunk);
    let empty_body = serde_json::to_value(empty_file).unwrap();
    let (empty_file_id, empty_file_json) = homestation::protocol::obj_id_of(ndn_lib::OBJ_TYPE_FILE, &empty_body).unwrap();
    named(station, &empty_file_id, &empty_file_json).await;
    let (empty_id, empty_json) = DirObject::new(Some("空目录".into())).gen_obj_id().unwrap();
    named(station, &empty_id.to_string(), &empty_json).await;
    let mut directory = DirObject::new(Some("资料 folder".into()));
    directory.add_file("同名.txt".into(), body, file.size).unwrap();
    directory.add_file("empty.txt".into(), empty_body, 0).unwrap();
    directory.add_directory("空目录".into(), empty_id.clone(), 0).unwrap();
    let (dir_id, dir_json) = directory.gen_obj_id().unwrap();
    let dir_id = dir_id.to_string();
    named(station, &dir_id, &dir_json).await;

    // The upload's binding in the user's own scratch bucket proves the caller holds it.
    let meta = json!({ "mime": "application/x-directory" });
    let missing = station.register_object(&dir_id, meta.clone(), Some("uploads/u0/folder")).await.unwrap_err();
    assert!(missing.to_string().contains("upload_missing"), "{missing}");
    upload_binding(station, "uploads/u1/file", &file_id).await;
    let other = station.register_object(&dir_id, meta.clone(), Some("uploads/u1/file")).await.unwrap_err();
    assert!(other.to_string().contains("another object"), "{other}");
    let target = net.rpc("alice", "object.upload_target", json!({ "name": "资料/folder" })).await;
    assert_eq!(target["bucketId"], "user/homestation/alice/scratch");
    let key = target["key"].as_str().unwrap();
    assert!(key.starts_with("uploads/u-") && key.ends_with("/资料_folder"), "{target}");
    upload_binding(station, key, &dir_id).await;
    let registered = net.rpc("alice", "object.register", json!({ "objId": dir_id, "metadata": meta, "key": key })).await;
    assert_eq!(registered["objId"], dir_id);
    assert_eq!(station.register_object(&dir_id, meta, Some(key)).await.unwrap(), registered);

    let input = json!({ "text": "A directory", "attachments": [{ "id": "folder", "kind": "dir", "name": "资料 folder", "status": "uploaded", "object": dir_id }], "audience": { "kind": "dids", "dids": [net.n("bob").did] } });
    let posted = publish(&net, "alice", "directory-once", input.clone()).await;
    assert_eq!(publish(&net, "alice", "directory-once", input).await["objId"], posted["objId"]);
    let version = posted["objId"].as_str().unwrap();
    let prefix = format!("feed/{}/", posted["entry"].as_str().unwrap().rsplit('/').next().unwrap());
    let entries = bound(station, HOMESTATION_BUCKET_ENTRIES, &prefix).await;
    assert_eq!(entries.iter().map(|b| (b.key.clone(), b.obj_id.clone())).collect::<Vec<_>>(), vec![(format!("{prefix}{version}"), Some(version.to_string()))]);
    let published = bound(station, HOMESTATION_BUCKET_PUBLISHED, &prefix).await;
    assert_eq!(published.iter().map(|b| (b.key.clone(), b.obj_id.clone())).collect::<Vec<_>>(), vec![(format!("{prefix}{version}/0"), Some(dir_id.clone()))]);

    for id in [&dir_id, &file_id, &empty_id.to_string(), &list_id.to_string()] {
        let url = format!("{}/objects/{id}", alice.home);
        assert_eq!(net.get(&url, Some(net.proof("bob", "alice"))).await.0, 200);
        assert_eq!(net.get(&url, Some(net.proof("eve", "alice"))).await.0, 404);
        assert_eq!(net.get(&url, None).await.0, 404);
    }
    let (status, data, _) = net.get(&format!("{}/objects/{file_id}/content", alice.home), Some(net.proof("bob", "alice"))).await;
    assert_eq!(status, 200);
    assert_eq!(data.as_bytes(), pieces.concat());
    let (status, data, _) = net.get(&format!("{}/objects/{empty_file_id}/content", alice.home), Some(net.proof("bob", "alice"))).await;
    assert_eq!((status, data), (200, String::new()));

    // Content streams with Range across the chunk list; the reader may also read it on the
    // nfs-server data plane through its binding and a short read token.
    let whole = pieces.concat();
    let ranged = net.http.get(format!("{}/objects/{file_id}/content", alice.home)).header("authorization", net.proof("bob", "alice")).header("range", "bytes=6-14").send().await.unwrap();
    assert_eq!(ranged.status().as_u16(), 206);
    assert_eq!(ranged.headers()["content-range"], format!("bytes 6-14/{}", whole.len()).as_str());
    assert_eq!(ranged.headers()["accept-ranges"], "bytes");
    assert_eq!(ranged.bytes().await.unwrap().as_ref(), &whole[6..=14]);
    let past = net.http.get(format!("{}/objects/{file_id}/content", alice.home)).header("authorization", net.proof("bob", "alice")).header("range", "bytes=999-").send().await.unwrap();
    assert_eq!(past.status().as_u16(), 416);
    let (status, found, _) = net.get(&format!("{}/objects/{file_id}/ref", alice.home), Some(net.proof("bob", "alice"))).await;
    assert_eq!(status, 200, "{found}");
    let found: serde_json::Value = serde_json::from_str(&found).unwrap();
    assert_eq!((found["bucketId"].as_str(), found["key"].as_str()), (Some("user/homestation/alice/scratch"), Some("uploads/u1/file")));
    let token = found["token"].as_str().unwrap();
    let data_plane = net.http.get(format!("{}/bucket/v1/user/homestation/alice/scratch/uploads/u1/file?token={token}", alice.nfs)).header("range", "bytes=0-4").send().await.unwrap();
    assert_eq!(data_plane.status().as_u16(), 206);
    assert_eq!(data_plane.bytes().await.unwrap().as_ref(), &whole[0..=4]);
    assert_eq!(net.get(&format!("{}/objects/{file_id}/ref", alice.home), Some(net.proof("eve", "alice"))).await.0, 404, "the same audience rules as /content");

    // An edit is a new version: both versions are bound while the entry is active.
    net.rpc("alice", "entry.edit", json!({ "entry": posted["entry"], "text": "A directory, renamed" })).await;
    assert_eq!(bound(station, HOMESTATION_BUCKET_ENTRIES, &prefix).await.len(), 2);
    assert_eq!(bound(station, HOMESTATION_BUCKET_PUBLISHED, &prefix).await.len(), 2);

    // 撤回 = 解绑; withdrawing again settles anything left behind.
    net.rpc("alice", "entry.withdraw", json!({ "entry": posted["entry"] })).await;
    assert!(bound(station, HOMESTATION_BUCKET_ENTRIES, &prefix).await.is_empty());
    assert!(bound(station, HOMESTATION_BUCKET_PUBLISHED, &prefix).await.is_empty());
    net.rpc("alice", "entry.withdraw", json!({ "entry": posted["entry"] })).await;
    assert_eq!(bound(station, HOMESTATION_BUCKET_SCRATCH, "uploads/").await.len(), 2, "uploads expire on their own");
}

#[tokio::test(flavor = "multi_thread")]
async fn prepared_posts_of_others_are_ambient() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let (alice, bob) = (net.n("alice"), net.n("bob"));
    alice.add_contact(bob, true, &[]);
    bob.add_contact(alice, true, &[]);
    let (photo, _) = alice.station.store_file("uploads/p1/photo.png", "photo.png", "image/png", vec![9u8; 2048], json!({ "width": 4, "height": 3 })).await.unwrap();
    let post = publish(&net, "alice", "img", json!({
        "text": "a photo", "attachments": [{ "id": "a1", "kind": "image", "name": "photo.png", "status": "uploaded", "object": photo }],
        "link": null, "audience": { "kind": "public" }
    }))
    .await;
    let feed_id = post["objId"].as_str().unwrap().to_string();
    net.deliver_all().await;
    net.rpc("bob", "admin.run", json!({ "task": "friends" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "pull" })).await;
    net.rpc("bob", "admin.run", json!({ "task": "selection" })).await;

    let entries = bound(&bob.station, HOMESTATION_BUCKET_FOLLOWING_ENTRIES, &feed_id).await;
    assert_eq!(entries.iter().map(|b| b.key.clone()).collect::<Vec<_>>(), vec![feed_id.clone()]);
    let files = bound(&bob.station, HOMESTATION_BUCKET_FOLLOWING, &format!("{feed_id}/")).await;
    assert_eq!(files.iter().map(|b| (b.key.clone(), b.obj_id.clone())).collect::<Vec<_>>(), vec![(format!("{feed_id}/{photo}"), Some(photo.clone()))]);
    let origin = files[0].origin.clone().expect("ambient bindings carry their origin");
    assert_eq!((origin.author.as_str(), origin.source.as_str()), (alice.did.as_str(), post["entry"].as_str().unwrap()));
    assert!(bound(&alice.station, HOMESTATION_BUCKET_FOLLOWING, "").await.is_empty(), "own posts stay in published");
    let (status, body, _) = net.get(&format!("{}/objects/{photo}/content?access={}", bob.home, bob.token), None).await;
    assert_eq!((status, body.len()), (200, 2048));

    // Saved items keep it; once the item left the reading list and is unsaved, it is released.
    net.rpc("bob", "interact.read_later", json!({ "objId": feed_id, "on": true })).await;
    bob.station.db.call(|c| Ok(c.execute("DELETE FROM reading WHERE owner=?1", [c.owner])?)).await.unwrap();
    net.rpc("bob", "interact.read_later", json!({ "objId": feed_id, "on": false })).await;
    assert!(bound(&bob.station, HOMESTATION_BUCKET_FOLLOWING_ENTRIES, &feed_id).await.is_empty());
    assert!(bound(&bob.station, HOMESTATION_BUCKET_FOLLOWING, &feed_id).await.is_empty());
}
