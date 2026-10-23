mod common;

use common::*;
use homestation::objects::{chunk_id_of, ChunkStore};
use ndn_lib::{ChunkId, ChunkList, DirObject, FileObject};
use serde_json::{json, Value};

async fn named(store: &dyn ChunkStore, id: &str, body: &str) {
    store.put_named_object(id, body).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn imported_directory_chunklist_and_empty_file_follow_audience_and_retention() {
    let net = Net::new(&["alice", "bob", "eve"], &[]).await;
    let alice = net.n("alice");
    let store = alice.station.chunks.as_ref();
    let pieces = [b"first part".to_vec(), b"second part".to_vec()];
    let mut chunks = Vec::new();
    for bytes in &pieces {
        let id = chunk_id_of(bytes).unwrap();
        store.put_chunk(&id, bytes.clone()).await.unwrap();
        chunks.push(ChunkId::new(&id).unwrap());
    }
    let (list_id, list_json) = ChunkList::from_chunk_list(chunks).unwrap().gen_obj_id();
    named(store, &list_id.to_string(), &list_json).await;
    let file = FileObject::new("同名.txt".into(), pieces.iter().map(|bytes| bytes.len() as u64).sum(), list_id.to_string());
    let body = serde_json::to_value(&file).unwrap();
    let (file_id, file_json) = homestation::protocol::obj_id_of(ndn_lib::OBJ_TYPE_FILE, &body).unwrap();
    named(store, &file_id, &file_json).await;
    let empty_chunk = chunk_id_of(b"").unwrap();
    store.put_chunk(&empty_chunk, vec![]).await.unwrap();
    let empty_file = FileObject::new("empty.txt".into(), 0, empty_chunk);
    let empty_body = serde_json::to_value(empty_file).unwrap();
    let (empty_file_id, empty_file_json) = homestation::protocol::obj_id_of(ndn_lib::OBJ_TYPE_FILE, &empty_body).unwrap();
    named(store, &empty_file_id, &empty_file_json).await;
    let (empty_id, empty_json) = DirObject::new(Some("空目录".into())).gen_obj_id().unwrap();
    named(store, &empty_id.to_string(), &empty_json).await;
    let mut directory = DirObject::new(Some("资料 folder".into()));
    directory.add_file("同名.txt".into(), body, file.size).unwrap();
    directory.add_file("empty.txt".into(), empty_body, 0).unwrap();
    directory.add_directory("空目录".into(), empty_id.clone(), 0).unwrap();
    let (dir_id, dir_json) = directory.gen_obj_id().unwrap();
    named(store, &dir_id.to_string(), &dir_json).await;
    let registered = alice.station.register_object(&dir_id.to_string(), json!({ "mime": "application/x-directory" })).await.unwrap();
    assert_eq!(registered["objId"], dir_id.to_string());
    assert_eq!(alice.station.register_object(&dir_id.to_string(), json!({ "mime": "application/x-directory" })).await.unwrap(), registered);
    let input = json!({ "text": "A directory", "attachments": [{ "id": "folder", "kind": "dir", "name": "资料 folder", "status": "uploaded", "object": dir_id.to_string() }], "audience": { "kind": "dids", "dids": [net.n("bob").did] } });
    let posted = publish(&net, "alice", "directory-once", input.clone()).await;
    assert_eq!(publish(&net, "alice", "directory-once", input).await["objId"], posted["objId"]);
    for id in [&dir_id.to_string(), &file_id, &empty_id.to_string(), &list_id.to_string()] {
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
    assert!(alice.station.reconcile_retentions().await.unwrap().issues.is_empty());
    net.rpc("alice", "entry.withdraw", json!({ "entry": posted["entry"] })).await;
    let retention = alice.station.retention_status().await.unwrap();
    assert_eq!(retention["held"], Value::from(0));
}
