//! End-to-end protocol tests: a real server on an ephemeral port, exercised
//! over HTTP exactly as the WebUI would.

use nfs_server::config::{ExportRoot, ServerConfig};
use nfs_server::state::AppState;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct TestServer {
    base: String,
    addr: std::net::SocketAddr,
    client: reqwest::Client,
    session: String,
    seq: AtomicU64,
    /// export root "home" backing dir
    home: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn start_server() -> TestServer {
    start_server_with(|_| {}).await
}

async fn start_server_with(tune: impl FnOnce(&mut ServerConfig)) -> TestServer {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut config = ServerConfig::new(
        tmp.path().join("data"),
        vec![ExportRoot { id: "home".into(), path: home.clone() }],
    );
    config.debug_api = true;
    tune(&mut config);
    let state = AppState::new(config).unwrap();
    let app = nfs_server::server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();
    // hello
    let resp: Value = client
        .post(format!("{}/nfs/v1/hello", base))
        .json(&json!({"args": {"versions": ["nfsp/0"]}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["ok"], true, "hello failed: {}", resp);
    let session = resp["result"]["session"].as_str().unwrap().to_string();
    TestServer {
        base,
        addr,
        client,
        session,
        seq: AtomicU64::new(1),
        home,
        _tmp: tmp,
    }
}

impl TestServer {
    /// Read op: no seq.
    async fn call(&self, method: &str, body: Value) -> (u16, Value) {
        self.raw_call(method, body, None).await
    }

    /// Write op: auto-assigns the next seq.
    async fn write(&self, method: &str, body: Value) -> (u16, Value) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        self.raw_call(method, body, Some(seq)).await
    }

    async fn raw_call(&self, method: &str, mut body: Value, seq: Option<u64>) -> (u16, Value) {
        body["session"] = json!(self.session);
        if let Some(s) = seq {
            body["seq"] = json!(s);
        }
        let resp = self
            .client
            .post(format!("{}/nfs/v1/{}", self.base, method))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap())
    }

    async fn ok(&self, method: &str, body: Value) -> Value {
        let (status, v) = self.call(method, body).await;
        assert_eq!(v["ok"], true, "{} failed ({}): {}", method, status, v);
        v["result"].clone()
    }

    async fn ok_write(&self, method: &str, body: Value) -> Value {
        let (status, v) = self.write(method, body).await;
        assert_eq!(v["ok"], true, "{} failed ({}): {}", method, status, v);
        v["result"].clone()
    }

    async fn expect_err(&self, method: &str, body: Value, code: &str) -> (u16, Value) {
        let (status, v) = self.write(method, body).await;
        assert_eq!(v["ok"], false, "{} unexpectedly ok: {}", method, v);
        assert_eq!(v["error"]["code"], code, "{} wrong code: {}", method, v);
        (status, v)
    }

    /// Uploads content for parent/name via open_write + tus PATCH + commit_file.
    async fn upload(&self, parent_path: &str, name: &str, content: &[u8]) -> Value {
        let ow = self
            .ok_write(
                "open_write",
                json!({"args": {"parent_ref": self.path_ref(parent_path).await, "name": name, "size": content.len()}}),
            )
            .await;
        let fb = ow["fb_handle"].as_str().unwrap().to_string();
        let lease_id = ow["lease"]["lease_id"].as_str().unwrap().to_string();
        // Two PATCHes to exercise resumption.
        let mid = content.len() / 2;
        for (off, chunk) in [(0usize, &content[..mid]), (mid, &content[mid..])] {
            if chunk.is_empty() && off > 0 {
                continue;
            }
            let resp = self
                .client
                .patch(format!("{}/nfs/v1/uploads/{}", self.base, fb))
                .header("Upload-Offset", off)
                .header("Content-Type", "application/offset+octet-stream")
                .body(chunk.to_vec())
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status().as_u16(), 204, "PATCH failed");
        }
        self.ok_write(
            "commit_file",
            json!({
                "at": {"realm": "dfs", "path": parent_path},
                "args": {"name": name, "fb_handle": fb, "lease_id": lease_id}
            }),
        )
        .await
    }

    /// Resolves a path to its wire ref.
    async fn path_ref(&self, path: &str) -> Value {
        let info = self
            .ok("resolve", json!({"at": {"realm": "dfs", "path": path}}))
            .await;
        info["ref"].clone()
    }
}

// ---------- M1: browse ----------

#[tokio::test]
async fn hello_negotiation_and_browse() {
    let s = start_server().await;
    // Root resolve + list.
    let root = s.ok("resolve", json!({"at": {"realm": "dfs", "path": "/"}})).await;
    assert_eq!(root["kind"], "dir");
    assert_eq!(root["capabilities"]["list"], true);
    assert!(root["flags"].as_array().unwrap().iter().any(|f| f == "read_only"));

    std::fs::create_dir(s.home.join("docs")).unwrap();
    std::fs::write(s.home.join("a.txt"), b"hello").unwrap();
    std::fs::write(s.home.join("docs/b.md"), b"# b").unwrap();

    let listing = s
        .ok("list", json!({"at": {"realm": "dfs", "path": "/"}, "want": ["base", "ident"]}))
        .await;
    assert_eq!(listing["container"]["kind"], "dir");
    let entries = listing["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "home");
    assert_eq!(entries[0]["binding"], "native");

    let home = s
        .ok("list", json!({"at": {"realm": "dfs", "path": "/home"}, "want": ["base", "ident"]}))
        .await;
    let entries = home["entries"].as_array().unwrap();
    // Byte-order by name: "a.txt" < "docs".
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["name"], "a.txt");
    assert_eq!(entries[0]["target"]["kind"], "file");
    assert_eq!(entries[0]["target"]["attrs"]["size"], 5);
    assert_eq!(entries[1]["name"], "docs");
    assert_eq!(entries[1]["target"]["kind"], "dir");
    assert!(home["watch_token"].as_str().unwrap().starts_with("w_"));

    // stat by ref round-trips.
    let file_ref = entries[0]["target"]["ref"].clone();
    let info = s.ok("stat", json!({"at": {"ref": file_ref}, "want": ["base", "ident", "access"]})).await;
    assert_eq!(info["kind"], "file");
    assert_eq!(info["size"], 5);
    assert!(info["etag"].as_str().unwrap().contains('-'));
    let urls = info["access_urls"].as_array().unwrap();
    assert!(urls.iter().any(|u| u["kind"] == "fs" && u["url"] == "cyfs:///home/a.txt"));
    assert!(urls.iter().any(|u| u["kind"] == "read"));

    // Both documented current-Zone CYFS URI forms resolve to the same namespace.
    let by_canonical = s.ok("resolve", json!({"at": {"uri": "cyfs:///home/a.txt"}})).await;
    assert_eq!(by_canonical["ref"], info["ref"]);
    let by_alias = s.ok("resolve", json!({"at": {"uri": "cyfs://_/home/a.txt"}})).await;
    assert_eq!(by_alias["ref"], info["ref"]);

    // Unknown paths / realms.
    let (status, v) = s.call("resolve", json!({"at": {"realm": "dfs", "path": "/nope"}})).await;
    assert_eq!((status, v["error"]["code"].as_str().unwrap()), (404, "NOT_FOUND"));
    // list on a file → NOT_A_CONTAINER.
    let (status, v) = s.call("list", json!({"at": {"realm": "dfs", "path": "/home/a.txt"}})).await;
    assert_eq!((status, v["error"]["code"].as_str().unwrap()), (400, "NOT_A_CONTAINER"));
    // Escape attempt.
    let (_, v) = s.call("resolve", json!({"at": {"realm": "dfs", "path": "/home/../etc"}})).await;
    assert_eq!(v["error"]["code"], "INVALID_ARGUMENT");
}

#[tokio::test]
async fn list_pagination_and_cursor_stability() {
    let s = start_server().await;
    for i in 0..25 {
        std::fs::write(s.home.join(format!("f{:02}.txt", i)), b"x").unwrap();
    }
    let page1 = s
        .ok("list", json!({"at": {"realm": "dfs", "path": "/home"}, "args": {"limit": 10}}))
        .await;
    assert_eq!(page1["entries"].as_array().unwrap().len(), 10);
    assert_eq!(page1["truncated"], true);
    let cursor = page1["next_cursor"].as_str().unwrap().to_string();

    // Concurrent change between pages: cursor keeps advancing (D10, no reset).
    std::fs::write(s.home.join("f00-inserted.txt"), b"x").unwrap();
    let bump = s.ok_write("mkdir", json!({"at": {"ref": s.path_ref("/home").await}, "args": {"name": "zz-dir"}})).await;
    assert_eq!(bump["existed"], false);

    let page2 = s
        .ok(
            "list",
            json!({"at": {"realm": "dfs", "path": "/home"}, "args": {"limit": 10, "cursor": cursor}}),
        )
        .await;
    assert_eq!(page2["revision_changed"], true);
    let names: Vec<&str> = page2["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    // Continues after f09, never re-emits earlier names.
    assert_eq!(names[0], "f10.txt");

    // Filters.
    let filtered = s
        .ok(
            "list",
            json!({"at": {"realm": "dfs", "path": "/home"},
                   "args": {"filter": {"name_glob": "f1?.txt"}, "limit": 100}}),
        )
        .await;
    assert_eq!(filtered["entries"].as_array().unwrap().len(), 10);
}

#[tokio::test]
async fn batch_walk_stat_list() {
    let s = start_server().await;
    std::fs::create_dir_all(s.home.join("photos/2026")).unwrap();
    std::fs::write(s.home.join("photos/2026/cover.jpg"), b"jpg").unwrap();

    let r = s
        .ok(
            "batch",
            json!({"args": {
                "start": {"realm": "dfs", "path": "/home"},
                "ops": [
                    {"m": "walk", "args": {"names": ["photos", "2026"]}},
                    {"m": "list", "args": {"limit": 200}, "want": ["base", "ident"]},
                    {"m": "stat", "args": {"name": "cover.jpg"}, "want": ["base", "access"]}
                ]
            }}),
        )
        .await;
    assert_eq!(r["completed"], 3);
    let results = r["results"].as_array().unwrap();
    assert_eq!(results[0]["result"]["kind"], "dir");
    assert_eq!(results[1]["result"]["entries"][0]["name"], "cover.jpg");
    assert_eq!(results[2]["result"]["kind"], "file");
    assert_eq!(results[2]["result"]["name"], "cover.jpg");

    // abort on error (default).
    let r = s
        .ok(
            "batch",
            json!({"args": {
                "start": {"realm": "dfs", "path": "/home"},
                "ops": [
                    {"m": "walk", "args": {"name": "missing"}},
                    {"m": "list"}
                ]
            }}),
        )
        .await;
    assert_eq!(r["completed"], 0);
    let results = r["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["error"]["code"], "NOT_FOUND");

    // continue mode runs later ops.
    let r = s
        .ok(
            "batch",
            json!({"args": {
                "start": {"realm": "dfs", "path": "/home"},
                "on_error": "continue",
                "ops": [
                    {"m": "stat", "args": {"name": "missing"}},
                    {"m": "list"}
                ]
            }}),
        )
        .await;
    assert_eq!(r["completed"], 1);
    assert_eq!(r["results"].as_array().unwrap().len(), 2);
}

// ---------- M2: writes, CAS, leases, replay ----------

#[tokio::test]
async fn upload_commit_read_roundtrip() {
    let s = start_server().await;
    let content = b"The quick brown fox jumps over the lazy dog".to_vec();
    let committed = s.upload("/home", "fox.txt", &content).await;
    assert!(committed["obj"]["sha256"].as_str().unwrap().len() == 64);
    let node_id = committed["ref"]["node_id"].as_str().unwrap().to_string();

    // Full read.
    let resp = s
        .client
        .get(format!("{}/nfs/v1/read/{}", s.base, node_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.headers()["accept-ranges"], "bytes");
    let etag = resp.headers()["etag"].to_str().unwrap().to_string();
    assert_eq!(resp.bytes().await.unwrap().to_vec(), content);

    // Range read.
    let resp = s
        .client
        .get(format!("{}/nfs/v1/read/{}", s.base, node_id))
        .header("Range", "bytes=4-8")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 206);
    assert_eq!(resp.headers()["content-range"].to_str().unwrap(), format!("bytes 4-8/{}", content.len()));
    assert_eq!(resp.bytes().await.unwrap().to_vec(), b"quick".to_vec());

    // Conditional read.
    let resp = s
        .client
        .get(format!("{}/nfs/v1/read/{}", s.base, node_id))
        .header("If-None-Match", etag)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 304);

    // probe: content now known → not missing; unknown hash → missing.
    let hash = committed["obj"]["sha256"].as_str().unwrap();
    let probe = s
        .ok(
            "probe",
            json!({"args": {"digests": [
                {"hash": hash, "size": content.len()},
                {"hash": "0".repeat(64), "size": 1}
            ]}}),
        )
        .await;
    let missing = probe["missing"].as_array().unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0]["hash"], "0".repeat(64));

    // 秒传: commit the same content elsewhere by hash only.
    let dedup = s
        .ok_write(
            "commit_file",
            json!({"at": {"realm": "dfs", "path": "/home"},
                   "args": {"name": "fox-copy.txt", "hash": hash}}),
        )
        .await;
    assert_eq!(dedup["obj"]["sha256"], hash.to_string());
    assert_eq!(std::fs::read(s.home.join("fox-copy.txt")).unwrap(), content);

    // Unknown hash → NEED_PULL.
    let (status, v) = s
        .expect_err(
            "commit_file",
            json!({"at": {"realm": "dfs", "path": "/home"},
                   "args": {"name": "ghost.txt", "hash": "1".repeat(64)}}),
            "NEED_PULL",
        )
        .await;
    assert_eq!(status, 409);
    assert!(v["error"]["obj_id"].as_str().unwrap().starts_with("sha256:"));
}

#[tokio::test]
async fn revision_cas_and_seq_replay() {
    let s = start_server().await;
    let home_list = s.ok("list", json!({"at": {"realm": "dfs", "path": "/home"}})).await;
    let rev = home_list["container"]["revision"].as_str().unwrap().to_string();

    // CAS success.
    let mk = s
        .ok_write(
            "mkdir",
            json!({"at": {"ref": s.path_ref("/home").await},
                   "args": {"name": "d1", "expected_revision": rev}}),
        )
        .await;
    assert_eq!(mk["existed"], false);

    // Stale revision now fails.
    let (status, _) = s
        .expect_err(
            "mkdir",
            json!({"at": {"ref": s.path_ref("/home").await},
                   "args": {"name": "d2", "expected_revision": rev}}),
            "REV_MISMATCH",
        )
        .await;
    assert_eq!(status, 409);

    // Exactly-once: replaying the same seq returns the cached result verbatim,
    // without re-executing (existed stays false).
    let seq = s.seq.fetch_add(1, Ordering::SeqCst);
    let (_, first) = s
        .raw_call(
            "mkdir",
            json!({"at": {"realm": "dfs", "path": "/home"}, "args": {"name": "d3"}}),
            Some(seq),
        )
        .await;
    assert_eq!(first["result"]["existed"], false);
    let (_, replay) = s
        .raw_call(
            "mkdir",
            json!({"at": {"realm": "dfs", "path": "/home"}, "args": {"name": "d3"}}),
            Some(seq),
        )
        .await;
    assert_eq!(replay["result"]["existed"], false, "must replay cached result");

    // Write without seq is rejected.
    let (_, v) = s
        .raw_call("mkdir", json!({"at": {"realm": "dfs", "path": "/home"}, "args": {"name": "d4"}}), None)
        .await;
    assert_eq!(v["error"]["code"], "INVALID_ARGUMENT");

    // mkdir -p path form.
    let deep = s
        .ok_write("mkdir", json!({"at": {"uri": "cyfs:///home/x/y/z"}}))
        .await;
    assert_eq!(deep["existed"], false);
    assert!(s.home.join("x/y/z").is_dir());
    let again = s
        .ok_write("mkdir", json!({"at": {"uri": "cyfs://_/home/x/y/z"}}))
        .await;
    assert_eq!(again["existed"], true);
}

#[tokio::test]
async fn lease_conflict_and_bypass_detection() {
    let s = start_server().await;
    s.upload("/home", "doc.txt", b"v1 content").await;

    // Session A holds the lease.
    let ow = s
        .ok_write(
            "open_write",
            json!({"args": {"parent_ref": s.path_ref("/home").await, "name": "doc.txt"}}),
        )
        .await;
    let lease_id = ow["lease"]["lease_id"].as_str().unwrap().to_string();
    let fb = ow["fb_handle"].as_str().unwrap().to_string();

    // Session B conflicts.
    let resp: Value = s
        .client
        .post(format!("{}/nfs/v1/hello", s.base))
        .json(&json!({"args": {}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_b = resp["result"]["session"].as_str().unwrap();
    let resp: Value = s
        .client
        .post(format!("{}/nfs/v1/open_write", s.base))
        .json(&json!({"session": session_b, "seq": 1,
                      "args": {"parent_ref": s.path_ref("/home").await, "name": "doc.txt"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], "LEASE_CONFLICT");
    assert!(resp["error"]["holder_session"].as_str().is_some());

    // Bypass edit between open_write and commit → explicit conflict.
    std::fs::write(s.home.join("doc.txt"), b"bypass writer changed me!").unwrap();
    let r = s
        .client
        .patch(format!("{}/nfs/v1/uploads/{}", s.base, fb))
        .header("Upload-Offset", 0)
        .body("v2 content".as_bytes().to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 204);
    let (status, v) = s
        .expect_err(
            "commit_file",
            json!({"at": {"realm": "dfs", "path": "/home"},
                   "args": {"name": "doc.txt", "fb_handle": fb, "lease_id": lease_id}}),
            "TARGET_MISMATCH",
        )
        .await;
    assert_eq!(status, 409);
    assert_eq!(v["error"]["reason"], "bypass_modified");
    // The bypass content was NOT clobbered.
    assert_eq!(std::fs::read(s.home.join("doc.txt")).unwrap(), b"bypass writer changed me!");
}

// ---------- M2/M6: move/delete/bind/unlink ----------

#[tokio::test]
async fn move_delete_bindings_and_stale() {
    let s = start_server().await;
    std::fs::create_dir(s.home.join("src")).unwrap();
    std::fs::create_dir(s.home.join("dst")).unwrap();
    s.upload("/home/src", "file.txt", b"content!").await;

    // Anchor it via set_meta so we can watch identity survive the move.
    s.ok_write(
        "set_meta",
        json!({"args": {"ref": s.path_ref("/home/src/file.txt").await,
                        "records": [{"ns": "user", "key": "rating", "value": 5}]}}),
    )
    .await;
    let before = s
        .ok("resolve", json!({"at": {"realm": "dfs", "path": "/home/src/file.txt"}, "want": ["ident"]}))
        .await;
    let stable_id = before["node_id"].as_str().unwrap().to_string();
    assert!(stable_id.starts_with("n_"), "anchored node expected, got {}", stable_id);

    // Protocol move: O(1), anchors follow.
    s.ok_write(
        "move",
        json!({"args": {
            "from": {"parent_ref": s.path_ref("/home/src").await, "name": "file.txt"},
            "to":   {"parent_ref": s.path_ref("/home/dst").await, "name": "renamed.txt"}
        }}),
    )
    .await;
    assert!(s.home.join("dst/renamed.txt").is_file());
    // The stable ref still resolves, at the new path.
    let after = s
        .ok("stat", json!({"at": {"ref": {"type": "live", "node_id": stable_id, "gen": 1}}, "want": ["base"]}))
        .await;
    assert_eq!(after["name"], "renamed.txt");
    // Meta survived (anchored on node identity, not path).
    let meta = s
        .ok("get_meta", json!({"args": {"ref": {"type": "live", "node_id": stable_id, "gen": 1}}}))
        .await;
    assert_eq!(meta["records"][0]["value"], 5);

    // bind_ref into src, unlink removes only the entry.
    let bind = s
        .ok_write(
            "bind_ref",
            json!({"args": {"parent_ref": s.path_ref("/home/src").await, "name": "link-to-file",
                            "target_ref": {"type": "live", "node_id": stable_id, "gen": 1}}}),
        )
        .await;
    let entry_ref = bind["entry_ref"].as_str().unwrap().to_string();
    let listing = s
        .ok("list", json!({"at": {"realm": "dfs", "path": "/home/src"}, "want": ["base"]}))
        .await;
    let entries = listing["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["binding"], "reference");
    assert_eq!(entries[0]["target"]["kind"], "file");
    assert_eq!(entries[0]["canonical_path"], "cyfs:///home/dst/renamed.txt");

    // delete may not touch reference entries.
    let (_, v) = s
        .expect_err(
            "delete",
            json!({"at": {"realm": "dfs", "path": "/home/src"}, "args": {"name": "link-to-file"}}),
            "INVALID_ARGUMENT",
        )
        .await;
    assert!(v["error"]["message"].as_str().unwrap().contains("unlink"));

    // unlink removes the entry; the target file is untouched.
    s.ok_write("unlink", json!({"args": {"entry_ref": entry_ref}})).await;
    assert!(s.home.join("dst/renamed.txt").is_file());
    let listing = s.ok("list", json!({"at": {"realm": "dfs", "path": "/home/src"}})).await;
    assert_eq!(listing["entries"].as_array().unwrap().len(), 0);

    // delete the target → the stable ref goes STALE (410), honestly.
    s.ok_write(
        "delete",
        json!({"at": {"realm": "dfs", "path": "/home/dst"}, "args": {"name": "renamed.txt"}}),
    )
    .await;
    let (status, v) = s
        .call("stat", json!({"at": {"ref": {"type": "live", "node_id": stable_id, "gen": 1}}}))
        .await;
    assert_eq!((status, v["error"]["code"].as_str().unwrap()), (410, "STALE"));

    // Non-empty delete guard.
    std::fs::create_dir(s.home.join("full")).unwrap();
    std::fs::write(s.home.join("full/x"), b"x").unwrap();
    s.expect_err(
        "delete",
        json!({"at": {"realm": "dfs", "path": "/home"}, "args": {"name": "full"}}),
        "NOT_EMPTY",
    )
    .await;
    s.ok_write(
        "delete",
        json!({"at": {"realm": "dfs", "path": "/home"}, "args": {"name": "full", "recursive": true}}),
    )
    .await;
    assert!(!s.home.join("full").exists());

    // Moving a dir into itself is rejected.
    std::fs::create_dir_all(s.home.join("cyc/inner")).unwrap();
    let (_, v) = s
        .expect_err(
            "move",
            json!({"args": {
                "from": {"parent_ref": s.path_ref("/home").await, "name": "cyc"},
                "to": {"parent_ref": s.path_ref("/home/cyc/inner").await, "name": "cyc2"}
            }}),
            "INVALID_ARGUMENT",
        )
        .await;
    assert!(v["error"]["message"].as_str().unwrap().contains("itself"));
}

#[tokio::test]
async fn binding_native_conflict_surfaces() {
    let s = start_server().await;
    std::fs::create_dir(s.home.join("d")).unwrap();
    std::fs::write(s.home.join("target.txt"), b"t").unwrap();
    s.ok_write(
        "bind_ref",
        json!({"args": {"parent_ref": s.path_ref("/home/d").await, "name": "item",
                        "target_ref": s.path_ref("/home/target.txt").await}}),
    )
    .await;
    // Bypass writer creates a same-name native file.
    std::fs::write(s.home.join("d/item"), b"native").unwrap();
    let listing = s.ok("list", json!({"at": {"realm": "dfs", "path": "/home/d"}})).await;
    let entries = listing["entries"].as_array().unwrap();
    // Native item visible unchanged; binding surfaced as conflict, not silently rebound.
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["binding"], "native");
    let conflicts = listing["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["name"], "item");
    assert_eq!(conflicts[0]["reason"], "native_shadow");

    // Creating a binding over an existing native name is refused up front.
    s.expect_err(
        "bind_ref",
        json!({"args": {"parent_ref": s.path_ref("/home/d").await, "name": "item",
                        "target_ref": s.path_ref("/home/target.txt").await}}),
        "NAMESPACE_CONFLICT",
    )
    .await;
}

// ---------- M5: collections & views ----------

#[tokio::test]
async fn collection_lifecycle() {
    let s = start_server().await;
    std::fs::write(s.home.join("one.pdf"), b"1").unwrap();
    std::fs::write(s.home.join("two.pdf"), b"2").unwrap();

    let c = s
        .ok_write("create_collection", json!({"args": {"title": "Reading List", "collection_id": "reading"}}))
        .await;
    assert_eq!(c["kind"], "collection");
    assert_eq!(c["capabilities"]["accepts_content"], false);
    assert_eq!(c["capabilities"]["remove_semantics"], "unlink");
    let cref = c["ref"].clone();
    let rev = c["revision"].as_str().unwrap().to_string();

    let patched = s
        .ok_write(
            "collection_patch",
            json!({"args": {"ref": cref, "expected_revision": rev, "ops": [
                {"add_ref": {"target_ref": s.path_ref("/home/one.pdf").await}},
                {"add_ref": {"target_ref": s.path_ref("/home/two.pdf").await, "position": 0, "name": "second-first"}},
                {"create_group": {"name": "papers"}}
            ]}}),
        )
        .await;
    let rev2 = patched["revision"].as_str().unwrap().to_string();
    assert_ne!(rev, rev2);

    // Same target twice is legal.
    s.ok_write(
        "collection_patch",
        json!({"args": {"ref": cref, "ops": [
            {"add_ref": {"target_ref": s.path_ref("/home/one.pdf").await, "name": "one-again"}}
        ]}}),
    )
    .await;

    let listing = s
        .ok("list", json!({"at": {"uri": "collection://reading"}, "want": ["base"]}))
        .await;
    assert_eq!(listing["container"]["kind"], "collection");
    let entries = listing["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0]["name"], "second-first");
    assert_eq!(entries[0]["binding"], "reference");
    assert_eq!(entries[0]["canonical_path"], "cyfs:///home/two.pdf");
    assert_eq!(entries[1]["name"], "one.pdf");
    assert_eq!(entries[2]["binding"], "member");
    assert_eq!(entries[2]["target"]["kind"], "group");
    let group_entry_ref = entries[2]["entry_ref"].as_str().unwrap().to_string();
    let group_ref = entries[2]["target"]["ref"].clone();

    // Add into the group, list the group as its own container.
    s.ok_write(
        "collection_patch",
        json!({"args": {"ref": cref, "ops": [
            {"add_ref": {"target_ref": s.path_ref("/home/two.pdf").await, "parent_entry_ref": group_entry_ref}}
        ]}}),
    )
    .await;
    let group_listing = s.ok("list", json!({"at": {"ref": group_ref}})).await;
    assert_eq!(group_listing["container"]["kind"], "group");
    assert_eq!(group_listing["entries"].as_array().unwrap().len(), 1);

    // walk by ambiguous name → AMBIGUOUS_ENTRY (one.pdf appears twice by target
    // but names differ; make names collide first).
    s.ok_write(
        "collection_patch",
        json!({"args": {"ref": cref, "ops": [
            {"add_ref": {"target_ref": s.path_ref("/home/two.pdf").await, "name": "one.pdf"}}
        ]}}),
    )
    .await;
    let r = s
        .ok(
            "batch",
            json!({"args": {"start": {"uri": "collection://reading"},
                            "on_error": "continue",
                            "ops": [{"m": "walk", "args": {"name": "one.pdf"}}]}}),
        )
        .await;
    assert_eq!(r["results"][0]["error"]["code"], "AMBIGUOUS_ENTRY");

    // Deleting the native target leaves a stale member entry (not dropped).
    s.ok_write(
        "delete",
        json!({"at": {"realm": "dfs", "path": "/home"}, "args": {"name": "two.pdf"}}),
    )
    .await;
    let listing = s.ok("list", json!({"at": {"uri": "collection://reading"}})).await;
    let stale_entries: Vec<&Value> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["target"]["target_state"] == "stale")
        .collect();
    assert!(!stale_entries.is_empty(), "dead targets must surface as stale: {}", listing);

    // rename_group + remove_entry.
    s.ok_write(
        "collection_patch",
        json!({"args": {"ref": cref, "ops": [
            {"rename_group": {"entry_ref": group_entry_ref, "name": "archive"}},
            {"remove_entry": {"entry_ref": group_entry_ref}}
        ]}}),
    )
    .await;
    let listing = s.ok("list", json!({"at": {"uri": "collection://reading"}})).await;
    assert!(listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["binding"] != "member"));
}

#[tokio::test]
async fn view_readonly_container() {
    let s = start_server().await;
    std::fs::create_dir_all(s.home.join("photos")).unwrap();
    std::fs::write(s.home.join("photos/a.jpg"), b"a").unwrap();
    std::fs::write(s.home.join("photos/b.jpg"), b"b").unwrap();

    // Seed a Topic view through the debug door (v1 has no AI generator).
    let resp = s
        .client
        .post(format!("{}/nfs/v1/debug/create_view", s.base))
        .json(&json!({
            "view_id": "topic/hokkaido",
            "title": "北海道之行",
            "groups": [{"label": "Day 1", "by": "time"}],
            "members": [
                {"path": "home/photos/a.jpg", "group": "Day 1",
                 "provenance": {"why": "同一行程", "matched_by": "story.im", "score": 0.9}},
                {"path": "home/photos/b.jpg"}
            ]
        }))
        .send()
        .await
        .unwrap();
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["ok"], true, "create_view: {}", v);

    let view = s.ok("open_view", json!({"args": {"view_id": "topic/hokkaido"}})).await;
    assert_eq!(view["kind"], "view");
    assert_eq!(view["title"], "北海道之行");
    assert_eq!(view["capabilities"]["accepts_content"], false);
    assert!(view["flags"].as_array().unwrap().iter().any(|f| f == "read_only"));

    // Unified list: groups first, then ungrouped members with canonical_path.
    let listing = s.ok("list", json!({"at": {"uri": "view://topic/hokkaido"}, "want": ["base"]})).await;
    let entries = listing["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["target"]["kind"], "group");
    assert_eq!(entries[0]["context"]["count"], 1);
    assert_eq!(entries[1]["binding"], "derived");
    assert_eq!(entries[1]["canonical_path"], "cyfs:///home/photos/b.jpg");

    // Group is itself listable; members carry provenance.
    let group_ref = entries[0]["target"]["ref"].clone();
    let group_listing = s.ok("list", json!({"at": {"ref": group_ref}, "want": ["base"]})).await;
    let members = group_listing["entries"].as_array().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["canonical_path"], "cyfs:///home/photos/a.jpg");
    assert_eq!(members[0]["context"]["provenance"]["why"], "同一行程");

    // Resolving by uri and by ref lands on the same LiveRef.
    let by_uri = s.ok("resolve", json!({"at": {"uri": "view://topic/hokkaido"}})).await;
    assert_eq!(by_uri["ref"], view["ref"]);
}

// ---------- meta / search / grants ----------

#[tokio::test]
async fn meta_permissions_and_lazy_anchor() {
    let s = start_server().await;
    std::fs::write(s.home.join("m.txt"), b"m").unwrap();
    // Pure browse leaves the node unanchored (handle ref).
    let before = s.ok("resolve", json!({"at": {"realm": "dfs", "path": "/home/m.txt"}, "want": ["ident"]})).await;
    assert!(before["node_id"].as_str().unwrap().starts_with("nh_"));
    // get_meta on an unanchored node: empty, and still unanchored after.
    let meta = s.ok("get_meta", json!({"at": {"realm": "dfs", "path": "/home/m.txt"}})).await;
    assert_eq!(meta["records"].as_array().unwrap().len(), 0);

    // set_meta anchors lazily.
    s.ok_write(
        "set_meta",
        json!({"at": {"realm": "dfs", "path": "/home/m.txt"},
               "args": {"records": [{"ns": "user", "key": "note", "value": "important"}]}}),
    )
    .await;
    let after = s.ok("resolve", json!({"at": {"realm": "dfs", "path": "/home/m.txt"}, "want": ["ident", "meta"]})).await;
    assert!(after["node_id"].as_str().unwrap().starts_with("n_"));
    assert_eq!(after["meta_summary"]["user"], 1);

    // Non-user ns is refused with the policy-style explanation.
    let (status, v) = s
        .expect_err(
            "set_meta",
            json!({"at": {"realm": "dfs", "path": "/home/m.txt"},
                   "args": {"records": [{"ns": "ai.vision.v1", "key": "caption", "value": "x"}]}}),
            "PERMISSION_DENIED",
        )
        .await;
    assert_eq!(status, 403);
    assert_eq!(v["error"]["required_op"], "meta.write.ai.vision.v1");
}

#[tokio::test]
async fn search_name_mode() {
    let s = start_server().await;
    std::fs::create_dir_all(s.home.join("deep/nest")).unwrap();
    std::fs::write(s.home.join("hokkaido-1.jpg"), b"x").unwrap();
    std::fs::write(s.home.join("deep/nest/hokkaido-2.jpg"), b"x").unwrap();
    std::fs::write(s.home.join("unrelated.txt"), b"x").unwrap();

    let r = s
        .ok("search", json!({"args": {"q": "hokkaido", "limit": 1}, "want": ["base"]}))
        .await;
    let hits = r["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["match_source"], "name");
    assert!(hits[0]["explain"]["matcher"].as_str().is_some());
    assert_eq!(r["sources"][0]["mode"], "name");
    assert_eq!(r["sources"][0]["state"], "ok");
    let cursor = r["next_cursor"].as_str().unwrap().to_string();

    let r2 = s
        .ok("search", json!({"args": {"q": "hokkaido", "limit": 10, "cursor": cursor}}))
        .await;
    let hits2 = r2["hits"].as_array().unwrap();
    assert_eq!(hits2.len(), 1);
    assert_ne!(hits[0]["canonical_path"], hits2[0]["canonical_path"]);
    assert!(r2.get("next_cursor").is_none());

    // Scoped search.
    let r3 = s
        .ok(
            "search",
            json!({"args": {"q": "hokkaido", "scope": {"realm": "dfs", "path": "/home/deep"}}}),
        )
        .await;
    assert_eq!(r3["hits"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn grants_record_and_revoke() {
    let s = start_server().await;
    std::fs::create_dir(s.home.join("public")).unwrap();
    let g = s
        .ok_write(
            "grant",
            json!({"args": {"subtree": {"realm": "dfs", "path": "/home/public"},
                            "ops": ["read", "list"], "ttl": 3600}}),
        )
        .await;
    let cap_id = g["cap_id"].as_str().unwrap().to_string();
    assert!(g["token"].as_str().unwrap().len() > 30);
    assert_eq!(g["subtree"], "cyfs:///home/public");
    assert!(g["expires_at"].as_i64().unwrap() > 0);
    s.ok_write("revoke", json!({"args": {"cap_id": cap_id}})).await;
    let (_, v) = s.expect_err("revoke", json!({"args": {"cap_id": "cap_missing"}}), "NOT_FOUND").await;
    assert_eq!(v["error"]["code"], "NOT_FOUND");
}

// ---------- M3: reconciler ----------

#[tokio::test]
async fn reconciler_follows_bypass_changes() {
    let s = start_server().await;
    std::fs::create_dir(s.home.join("watched")).unwrap();
    s.upload("/home/watched", "tracked.txt", b"original").await;
    s.ok_write(
        "set_meta",
        json!({"at": {"realm": "dfs", "path": "/home/watched/tracked.txt"},
               "args": {"records": [{"ns": "user", "key": "k", "value": 1}]}}),
    )
    .await;
    let info = s
        .ok("resolve", json!({"at": {"realm": "dfs", "path": "/home/watched/tracked.txt"}, "want": ["ident"]}))
        .await;
    let stable_id = info["node_id"].as_str().unwrap().to_string();
    assert!(stable_id.starts_with("n_"));

    // Baseline scan.
    let reconcile = |c: &reqwest::Client, base: String| {
        let c = c.clone();
        async move {
            let v: Value = c
                .post(format!("{}/nfs/v1/debug/reconcile", base))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(v["ok"], true, "reconcile: {}", v);
            v["result"].clone()
        }
    };
    reconcile(&s.client, s.base.clone()).await;

    // 1) Bypass rename: anchor path follows, LiveRef keeps resolving.
    std::fs::rename(s.home.join("watched/tracked.txt"), s.home.join("watched/moved.txt")).unwrap();
    let rep = reconcile(&s.client, s.base.clone()).await;
    assert_eq!(rep["renamed"], 1, "report: {}", rep);
    let after = s
        .ok("stat", json!({"at": {"ref": {"type": "live", "node_id": stable_id, "gen": 1}}, "want": ["base"]}))
        .await;
    assert_eq!(after["name"], "moved.txt");
    // Meta still reachable through the stable ref.
    let meta = s
        .ok("get_meta", json!({"args": {"ref": {"type": "live", "node_id": stable_id, "gen": 1}}}))
        .await;
    assert_eq!(meta["records"].as_array().unwrap().len(), 1);

    // 2) Bypass edit (delete+recreate = new ino at the same path): rebind.
    std::fs::remove_file(s.home.join("watched/moved.txt")).unwrap();
    std::fs::write(s.home.join("watched/moved.txt"), b"overwritten by editor").unwrap();
    let rep = reconcile(&s.client, s.base.clone()).await;
    assert!(
        rep["rebound"].as_u64().unwrap() >= 1 || rep["touched"].as_u64().unwrap() >= 1,
        "report: {}",
        rep
    );
    let after = s
        .ok("stat", json!({"at": {"ref": {"type": "live", "node_id": stable_id, "gen": 1}}, "want": ["base"]}))
        .await;
    assert_eq!(after["size"], 21);

    // 3) Bypass delete: honest stale.
    std::fs::remove_file(s.home.join("watched/moved.txt")).unwrap();
    let rep = reconcile(&s.client, s.base.clone()).await;
    assert_eq!(rep["staled"], 1, "report: {}", rep);
    let (status, v) = s
        .call("stat", json!({"at": {"ref": {"type": "live", "node_id": stable_id, "gen": 1}}}))
        .await;
    assert_eq!((status, v["error"]["code"].as_str().unwrap()), (410, "STALE"));
}

#[tokio::test]
async fn stale_handle_after_bypass_replace() {
    let s = start_server().await;
    std::fs::write(s.home.join("h.txt"), b"one").unwrap();
    let info = s.ok("resolve", json!({"at": {"realm": "dfs", "path": "/home/h.txt"}, "want": ["ident"]})).await;
    let handle = info["node_id"].as_str().unwrap().to_string();
    assert!(handle.starts_with("nh_"));
    // Replace the file behind the handle. Rename the original away first so
    // its inode stays alive and cannot be recycled for the new file.
    std::fs::rename(s.home.join("h.txt"), s.home.join("h-old.txt")).unwrap();
    std::fs::write(s.home.join("h.txt"), b"two").unwrap();
    let (status, v) = s
        .call("stat", json!({"at": {"ref": {"type": "live", "node_id": handle, "gen": 0}}}))
        .await;
    assert_eq!((status, v["error"]["code"].as_str().unwrap()), (410, "STALE"));
    // Re-resolve from the trusted locator recovers.
    let again = s.ok("resolve", json!({"at": {"realm": "dfs", "path": "/home/h.txt"}})).await;
    assert_eq!(again["kind"], "file");
}

// ---------- watch (SSE) ----------

#[tokio::test]
async fn watch_resync_and_container_changed() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let s = start_server().await;
    std::fs::create_dir(s.home.join("observed")).unwrap();
    let listing = s.ok("list", json!({"at": {"realm": "dfs", "path": "/home/observed"}})).await;
    let token = listing["watch_token"].as_str().unwrap().to_string();

    // Raw SSE client (avoids extra dev-deps).
    let mut conn = tokio::net::TcpStream::connect(s.addr).await.unwrap();
    let req = format!(
        "GET /nfs/v1/watch?session={}&tokens={} HTTP/1.1\r\nHost: t\r\nAccept: text/event-stream\r\n\r\n",
        s.session, token
    );
    conn.write_all(req.as_bytes()).await.unwrap();

    async fn read_until(conn: &mut tokio::net::TcpStream, needle: &str) -> String {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                conn.read(&mut chunk),
            )
            .await
            .expect("SSE read timed out")
            .unwrap();
            assert!(n > 0, "SSE closed early");
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf).into_owned();
            if text.contains(needle) {
                return text;
            }
        }
    }

    // First event is always resync (D11 reconnect semantics).
    let text = read_until(&mut conn, "event: resync").await;
    assert!(text.contains("text/event-stream"));

    // A write into the watched dir produces container_changed with revision.
    s.ok_write(
        "mkdir",
        json!({"at": {"ref": s.path_ref("/home/observed").await}, "args": {"name": "newdir"}}),
    )
    .await;
    let text = read_until(&mut conn, "event: container_changed").await;
    assert!(text.contains("entries_changed"));
    assert!(text.contains("revision"));

    // A write to an UNWATCHED dir must not arrive: emit one, then a watched
    // marker event, and assert ordering shows no unwatched payload.
    s.ok_write("mkdir", json!({"at": {"ref": s.path_ref("/home").await}, "args": {"name": "elsewhere"}}))
        .await;
    s.ok_write(
        "mkdir",
        json!({"at": {"ref": s.path_ref("/home/observed").await}, "args": {"name": "marker"}}),
    )
    .await;
    let text = read_until(&mut conn, "marker").await;
    assert!(
        !text.contains("elsewhere"),
        "unwatched container leaked into filtered stream: {}",
        text
    );
}

// ---------- session lifecycle ----------

#[tokio::test]
async fn session_required_and_bye() {
    let s = start_server().await;
    // No session.
    let v: Value = s
        .client
        .post(format!("{}/nfs/v1/list", s.base))
        .json(&json!({"at": {"realm": "dfs", "path": "/"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED");
    // Unknown method.
    let (_, v) = s.call("frobnicate", json!({})).await;
    assert_eq!(v["error"]["code"], "UNSUPPORTED");
    // Critical unknown ext.
    let v: Value = s
        .client
        .post(format!("{}/nfs/v1/list", s.base))
        .json(&json!({"session": s.session, "at": {"realm": "dfs", "path": "/"},
                      "ext": [{"id": "x.future", "critical": true}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["error"]["code"], "UNSUPPORTED_EXT");
    // Non-critical unknown ext is ignored.
    let v: Value = s
        .client
        .post(format!("{}/nfs/v1/list", s.base))
        .json(&json!({"session": s.session, "at": {"realm": "dfs", "path": "/"},
                      "ext": [{"id": "x.future", "critical": false}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["ok"], true);
    // bye invalidates the session.
    s.ok("bye", json!({})).await;
    let (_, v) = s.call("list", json!({"at": {"realm": "dfs", "path": "/"}})).await;
    assert_eq!(v["error"]["code"], "PERMISSION_DENIED");
}

/// BuckyOS-mode wiring: cyfs:/// backed by a data root — visible first-level
/// dirs exported, .fsdb hidden. The zone gateway forwards the zone-level
/// protocol path `/nfs/v1/*` verbatim (boot_gateway.yaml), so the plain
/// router is the whole story: no /kapi prefix exists on this service.
#[tokio::test]
async fn buckyos_mode_data_root_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().join("data");
    std::fs::create_dir_all(data_root.join("home")).unwrap();
    std::fs::create_dir_all(data_root.join("srv")).unwrap();
    std::fs::write(data_root.join("home").join("a.txt"), b"hi").unwrap();

    let config = nfs_server::config::config_from_data_root(&data_root).unwrap();
    let state = AppState::new(config).unwrap();
    let app = nfs_server::server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    let resp: Value = client
        .post(format!("{}/nfs/v1/hello", base))
        .json(&json!({"args": {"versions": ["nfsp/0"]}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["ok"], true, "hello failed: {}", resp);
    let session = resp["result"]["session"].as_str().unwrap();

    let v: Value = client
        .post(format!("{}/nfs/v1/list", base))
        .json(&json!({"session": session, "at": {"realm": "dfs", "path": "/"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["ok"], true, "list failed: {}", v);
    let names: Vec<&str> = v["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["home", "srv"], ".fsdb hidden, dirs exported");

    // data plane: read the file back through the same root-level paths.
    let v: Value = client
        .post(format!("{}/nfs/v1/resolve", base))
        .json(&json!({"session": session, "at": {"realm": "dfs", "path": "/home/a.txt"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["ok"], true, "resolve failed: {}", v);
    let node_id = v["result"]["ref"]["node_id"].as_str().unwrap();
    let body = client
        .get(format!("{}/nfs/v1/read/{}", base, node_id))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&body[..], b"hi");
}
