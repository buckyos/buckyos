mod common;
use common::*;
use homestation::audience::{AudienceSpec, Reader};
use homestation::protocol::{EntryNamespace, OBJ_TYPE_FEED};
use homestation::publish::{EntryKind, NewVersion};
use serde_json::{json, Value};

async fn legacy_post(node: &Node, key: &str, audience: AudienceSpec) -> homestation::publish::Published {
    let entry = node.station.own_entry(EntryNamespace::Feed, key).replacen("https://", "cyfs://", 1);
    node.station.publish_version(NewVersion {
        entry: entry.clone(), kind: EntryKind::Post, audience: Some(audience), target: None,
        category: None, obj_type: OBJ_TYPE_FEED,
        claims: json!({"kind":"post","publisher":node.did,"iat":homestation::now_s(),"entry":entry,
            "content":{"type":"text","text":"historical post"}}),
    }).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_signed_entries_keep_identity_and_access_through_current_http() {
    let net = Net::new(&["alice", "bob"], &[]).await;
    let alice = net.n("alice");
    let post = legacy_post(alice, "legacy", AudienceSpec::Public).await;
    let before = alice.station.read_entry_head(&Reader::Owner, &post.entry).await.unwrap().unwrap().0;
    alice.station.set_zone_listing(&post.entry, true).await.unwrap();
    let https = post.entry.replacen("cyfs://", "https://", 1);
    assert_eq!(alice.station.read_entry_head(&Reader::Anonymous, &https).await.unwrap().unwrap().0, before);
    let (status, http_wire, _) = net.get(&format!("{}/feed/@/legacy", alice.home), None).await;
    assert_eq!(status, 200);
    assert_eq!(http_wire, before);
    let (status, object_wire, _) = net.get(&format!("{}/objects/{}", alice.home, post.head_obj_id), None).await;
    assert_eq!(status, 200);
    assert_eq!(object_wire, before);
    let portal = net.rpc("alice", "portal.item", json!({"feed":"alice","key":"legacy"})).await;
    assert_eq!(portal["entry"]["entry"], post.entry, "{portal}");
    let listed = net.rpc("alice", "portal.list", json!({"feed":"~zone"})).await;
    assert!(listed["entries"].as_array().unwrap().iter().any(|v| v["entry"] == post.entry));
    let pulled = net.n("bob").station.resolve_remote_entry(&https).await.unwrap().unwrap();
    assert_eq!(pulled.entry, post.entry);
    assert_eq!(pulled.head_obj_id, post.head_obj_id);
    let changed = alice.station.edit(&post.entry, "edited historical post").await.unwrap();
    assert_eq!(changed.entry, post.entry);
    assert_eq!(changed.seq, post.seq + 1);
    let retained = alice.station.read_object(&Reader::Owner, &post.head_obj_id).await.unwrap().unwrap();
    assert_eq!(retained.jwt.as_deref(), Some(before.as_str()));
    let new_post = publish(&net,"alice","new",text_input("new post",public())).await;
    assert!(new_post["entry"].as_str().unwrap().starts_with("https://"));

    let private = legacy_post(alice,"restricted",AudienceSpec::only(&alice.did)).await;
    assert_eq!(net.get(&format!("{}/feed/@/restricted",alice.home),None).await.0,404);
    assert!(alice.station.read_entry_head(&Reader::Owner,&private.entry.replacen("cyfs://","https://",1)).await.unwrap().is_some());
    let other = net.try_rpc("bob","entry.withdraw",json!({"entry":private.entry})).await;
    assert!(other.is_err());

    let hidden_https = https.clone();
    alice.station.publish_version(NewVersion {
        entry: hidden_https.clone(),kind:EntryKind::Post,audience:Some(AudienceSpec::only(&alice.did)),target:None,category:None,obj_type:OBJ_TYPE_FEED,
        claims:json!({"kind":"post","publisher":alice.did,"iat":homestation::now_s(),"entry":hidden_https,"content":{"type":"text","text":"new private entry"}}),
    }).await.unwrap();
    assert_eq!(net.get(&format!("{}/feed/@/legacy",alice.home),None).await.0,404,"an exact restricted entry must not fall back to a public legacy alias");
}

#[test]
fn production_legacy_claims_remain_structurally_valid_without_rewriting() {
    let Ok(path) = std::env::var("HS_LEGACY_DB") else { return };
    let db = rusqlite::Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut stmt = db.prepare("SELECT obj_id,obj_type,body FROM objects").unwrap();
    let rows = stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).unwrap();
    let mut checked=0;
    for row in rows {
        let (id,kind,body)=row.unwrap();
        let v:Value=serde_json::from_str(&body).unwrap();
        if !v.get("entry").and_then(Value::as_str).is_some_and(|e|e.starts_with("cyfs://")){continue}
        match kind.as_str() {
            "cyfeed"=>homestation::protocol::validate_feed_object(&serde_json::from_value(v.clone()).unwrap()).unwrap(),
            "cyfhead"=>homestation::protocol::validate_head(&serde_json::from_value(v.clone()).unwrap()).unwrap(),
            "cyfollow"=>homestation::protocol::validate_follow(&serde_json::from_value(v.clone()).unwrap()).unwrap(),
            _=>continue,
        }
        assert_eq!(homestation::protocol::obj_id_of(&kind,&v).unwrap().0,id);
        checked+=1;
    }
    assert!(checked>2000,"expected the captured legacy dataset, got {checked}");
    eprintln!("validated {checked} unchanged legacy object IDs");
}
