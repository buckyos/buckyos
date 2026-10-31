//! Media by Range (第四期规划 §7.3.2): `asset.stream_url` gives a short-lived address a `<video>` plays from directly;
//! the route answers `Range` requests with the right slice, so a follower seeks into a long asset without fetching all
//! of it, and an address nobody was given answers nothing.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::json;
use wishkit::*;

#[test]
fn media_plays_by_range_from_a_short_lived_address() {
    run_test(|| async move {
        let mut srv = Srv::start_with(tempfile::tempdir().unwrap(), model(), json!({ "tok-alice": { "principal": "alice" } })).await;
        srv.ws = srv.rpc("ws.create", json!({ "title": "Media" })).await["workspace_id"].as_str().unwrap().to_string();
        let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let put = srv.nfs.put("alice", "assets", "upload/clip.bin", bytes.clone()).await;
        let staged = srv.rpc("asset.import_object", json!({ "bucket_id": put["bucket_id"], "key": put["key"] })).await;
        let object_id = staged["object_id"].as_str().unwrap().to_string();
        srv.commit(json!([{ "op": "entity.create", "entity_id": "clip", "type_id": "buckyos.asset-ref", "parent_id": "data", "order_key": "a", "payload": { "object_id": object_id, "file_name": "clip.bin" } }])).await;
        let r = srv.rpc("asset.stream_url", json!({ "object_id": object_id })).await;
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["size"], 300_000);
        let origin = srv.base.trim_end_matches("/kapi/aiworkspace").to_string();
        let url = format!("{origin}{}", r["url"].as_str().unwrap());
        let http = reqwest::Client::new();
        // a slice in the middle: 206 with the slice and its place
        let part = http.get(&url).header("range", "bytes=100000-100999").send().await.unwrap();
        assert_eq!(part.status().as_u16(), 206);
        assert_eq!(part.headers()["content-range"], "bytes 100000-100999/300000");
        assert_eq!(part.bytes().await.unwrap().to_vec(), bytes[100_000..101_000].to_vec());
        // the end, and the whole
        let tail = http.get(&url).header("range", "bytes=-10").send().await.unwrap();
        assert_eq!(tail.status().as_u16(), 206);
        assert_eq!(tail.bytes().await.unwrap().to_vec(), bytes[299_990..].to_vec());
        let whole = http.get(&url).send().await.unwrap();
        assert_eq!(whole.status().as_u16(), 200);
        assert_eq!(whole.headers()["accept-ranges"], "bytes");
        assert_eq!(whole.bytes().await.unwrap().len(), 300_000);
        // past the end, and an address nobody was given
        assert_eq!(http.get(&url).header("range", "bytes=400000-").send().await.unwrap().status().as_u16(), 416);
        assert_eq!(http.get(format!("{origin}/kapi/aiworkspace/stream/st_aaaaaaaaaaaaaaaaaaaaaaaaaa")).send().await.unwrap().status().as_u16(), 404);
    })
}
