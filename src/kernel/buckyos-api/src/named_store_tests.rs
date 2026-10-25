use super::*;
use ndn_lib::{ChunkHasher, FileObject, NamedObject};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

struct TestEnv {
    _lock: std::sync::MutexGuard<'static, ()>,
    gateway: Option<String>,
    root: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let lock = crate::tests::test_env_lock().lock().unwrap();
        let gateway = crate::tests::set_env_var(BUCKYOS_HOST_GATEWAY_ENV, "127.0.0.1");
        let root = std::env::temp_dir().join(format!(
            "buckyos-runtime-named-store-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self {
            _lock: lock,
            gateway,
            root,
        }
    }

    fn runtime(&self, kind: BuckyOSRuntimeType, port: u16) -> BuckyOSRuntime {
        let app = match kind {
            BuckyOSRuntimeType::AppClient | BuckyOSRuntimeType::AppService => "notes.example.com",
            _ => "nfs-server",
        };
        let mut runtime = BuckyOSRuntime::new(app, Some("alice".into()), kind);
        runtime.buckyos_root_dir = self.root.clone();
        runtime.node_gateway_port = port;
        runtime
    }

    fn configure_local_store(&self) {
        let storage = self.root.join("storage");
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(
            storage.join("named_store.json"),
            json!({
                "epoch": 1,
                "stores": [{"store_id": "test", "path": "data", "weight": 1}]
            })
            .to_string(),
        )
        .unwrap();
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        crate::tests::restore_env_var(BUCKYOS_HOST_GATEWAY_ENV, self.gateway.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn token_pair(runtime: &BuckyOSRuntime, generation: &str, expires: u64) -> TokenPair {
    let target = runtime.get_auth_target().unwrap();
    let (pem, _) = generate_ed25519_key_pair();
    let key = EncodingKey::from_ed_pem(pem.as_bytes()).unwrap();
    let make = |token_use| {
        let (_, mut token) =
            RPCSessionToken::generate_jwt_token("alice", target.appid_claim(), None, &key).unwrap();
        token.iss = Some(VERIFY_HUB_TOKEN_ISSUER.into());
        token.exp = Some(expires);
        token.jti = Some(generation.into());
        token.extra.insert("principal_kind".into(), json!("user"));
        token.extra.insert("session".into(), json!(42));
        crate::bind_token_target(&mut token, &target, token_use).unwrap();
        token
            .generate_jwt(Some(VERIFY_HUB_TOKEN_ISSUER.into()), &key)
            .unwrap()
    };
    TokenPair {
        session_token: make(TokenUse::Session),
        refresh_token: make(TokenUse::Refresh),
    }
}

fn file_object(bytes: &[u8]) -> (ChunkId, ObjId, String) {
    let chunk = ChunkHasher::new(None)
        .unwrap()
        .calc_mix_chunk_id_from_bytes(bytes)
        .unwrap();
    let mut file = FileObject::default();
    file.content_obj.name = "attachment.bin".into();
    file.size = bytes.len() as u64;
    file.content = chunk.to_string();
    let (id, data) = file.gen_obj_id();
    (chunk, id, data)
}

#[derive(Debug)]
struct Request {
    path: String,
    headers: HashMap<String, String>,
    body: Value,
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn json(body: Value) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.to_string().into_bytes(),
        }
    }

    fn content(bytes: &[u8]) -> Self {
        Self {
            status: 200,
            headers: vec![("ndm-total-size".into(), bytes.len().to_string())],
            body: bytes.to_vec(),
        }
    }

    fn denied(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: format!("denied-{status}").into_bytes(),
        }
    }
}

struct TestServer {
    port: u16,
    requests: mpsc::UnboundedReceiver<Request>,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for mut reply in replies {
                let (stream, _) = listener.accept().await.unwrap();
                let mut stream = BufReader::new(stream);
                let mut first = String::new();
                stream.read_line(&mut first).await.unwrap();
                let mut parts = first.split_whitespace();
                assert_eq!(parts.next(), Some("POST"));
                let path = parts.next().unwrap().to_string();
                let mut headers = HashMap::new();
                loop {
                    let mut line = String::new();
                    assert!(stream.read_line(&mut line).await.unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    let (key, value) = line.split_once(':').unwrap();
                    headers.insert(key.to_ascii_lowercase(), value.trim().to_string());
                }
                let length: usize = headers.get("content-length").unwrap().parse().unwrap();
                let mut bytes = vec![0; length];
                stream.read_exact(&mut bytes).await.unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                if path.starts_with("/kapi/verify-hub") {
                    let mut response: Value = serde_json::from_slice(&reply.body).unwrap();
                    response["sys"] = json!([body["sys"][0]]);
                    reply.body = response.to_string().into_bytes();
                }
                sender
                    .send(Request {
                        path,
                        headers,
                        body,
                    })
                    .unwrap();
                let mut head = format!(
                    "HTTP/1.1 {} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.body.len()
                );
                for (key, value) in reply.headers {
                    head.push_str(&format!("{key}: {value}\r\n"));
                }
                head.push_str("\r\n");
                stream.write_all(head.as_bytes()).await.unwrap();
                stream.write_all(&reply.body).await.unwrap();
                stream.flush().await.unwrap();
            }
        });
        Self {
            port,
            requests,
            task,
        }
    }

    async fn next(&mut self) -> Request {
        tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn named_store_application_runtimes_do_not_open_local_configuration() {
    let env = TestEnv::new();
    for kind in [
        BuckyOSRuntimeType::AppClient,
        BuckyOSRuntimeType::AppService,
        BuckyOSRuntimeType::FrameService,
    ] {
        let runtime = env.runtime(kind, 3180);
        runtime.get_named_store().await.unwrap();
        assert!(matches!(
            runtime.get_local_named_store().await,
            Err(RPCErrors::NoPermission(_))
        ));
        assert!(runtime.named_store_mgr.get().is_none());
        assert!(!env.root.join("storage").exists());
    }
}

#[tokio::test]
async fn named_store_kernel_runtimes_share_the_local_manager() {
    let env = TestEnv::new();
    env.configure_local_store();
    let bytes = b"named object content\0\xff";
    let (chunk, id, object) = file_object(bytes);
    for kind in [
        BuckyOSRuntimeType::Kernel,
        BuckyOSRuntimeType::KernelService,
    ] {
        let runtime = env.runtime(kind, 1);
        let store = runtime.get_named_store().await.unwrap();
        store.put_chunk(&chunk, bytes).await.unwrap();
        store.put_object(&id, &object).await.unwrap();
        assert_eq!(store.get_object(&id).await.unwrap(), object);
        let (mut reader, size) = store.open_reader(&id, None).await.unwrap();
        let mut actual = Vec::new();
        reader.read_to_end(&mut actual).await.unwrap();
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(actual, bytes);
        assert_eq!(
            runtime
                .get_local_named_store()
                .await
                .unwrap()
                .get_object(&id)
                .await
                .unwrap(),
            object
        );
        assert!(runtime.named_store_mgr.get().is_some());
    }
}

#[tokio::test]
async fn named_store_remote_runtimes_read_bytes_with_their_session_token() {
    let env = TestEnv::new();
    let bytes = b"remote image bytes\0\xfe";
    let (_, id, object) = file_object(bytes);
    for kind in [
        BuckyOSRuntimeType::AppClient,
        BuckyOSRuntimeType::AppService,
        BuckyOSRuntimeType::FrameService,
    ] {
        let mut server = TestServer::start(vec![
            Reply::json(json!({"obj_data": object})),
            Reply::content(bytes),
        ])
        .await;
        let mut runtime = env.runtime(kind, server.port);
        let pair = token_pair(&runtime, "first", buckyos_get_unix_timestamp() + 600);
        runtime.replace_token_pair(pair.clone()).await.unwrap();
        let store = runtime.get_named_store().await.unwrap();
        assert_eq!(store.get_object(&id).await.unwrap(), object);
        let (mut reader, size) = store.open_reader(&id, None).await.unwrap();
        let mut actual = Vec::new();
        reader.read_to_end(&mut actual).await.unwrap();
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(actual, bytes);
        for path in [
            "/ndm/proxy/v1/rpc/get_object",
            "/ndm/proxy/v1/read/object/open",
        ] {
            let request = server.next().await;
            assert_eq!(request.path, path);
            assert_eq!(
                request.headers.get("authorization"),
                Some(&format!("Bearer {}", pair.session_token))
            );
            assert_eq!(request.body["obj_id"], id.to_string());
        }
        assert!(runtime.named_store_mgr.get().is_none());
        assert!(!env.root.join("storage").exists());
    }
}

#[tokio::test]
async fn named_store_retained_handle_uses_the_latest_token() {
    let env = TestEnv::new();
    let (_, id, object) = file_object(b"rotating credentials");
    let mut server = TestServer::start(vec![
        Reply::json(json!({"obj_data": object})),
        Reply::json(json!({"obj_data": object})),
    ])
    .await;
    let mut runtime = env.runtime(BuckyOSRuntimeType::AppClient, server.port);
    let first = token_pair(&runtime, "first", buckyos_get_unix_timestamp() + 600);
    runtime.replace_token_pair(first.clone()).await.unwrap();
    let store = runtime.get_named_store().await.unwrap();
    assert_eq!(store.get_object(&id).await.unwrap(), object);
    let second = token_pair(&runtime, "second", buckyos_get_unix_timestamp() + 600);
    runtime.commit_token_pair(second.clone()).await.unwrap();
    assert_eq!(store.get_object(&id).await.unwrap(), object);
    for pair in [first, second] {
        let request = server.next().await;
        assert_eq!(
            request.headers.get("authorization"),
            Some(&format!("Bearer {}", pair.session_token))
        );
    }
}

#[tokio::test]
async fn named_store_expired_session_refreshes_before_reading() {
    let env = TestEnv::new();
    let bytes = b"read after refresh\0\xff";
    let (_, id, _) = file_object(bytes);
    let mut runtime = env.runtime(BuckyOSRuntimeType::AppService, 1);
    let first = token_pair(&runtime, "first", buckyos_get_unix_timestamp() + 600);
    let renewed = token_pair(&runtime, "renewed", buckyos_get_unix_timestamp() + 600);
    let mut server = TestServer::start(vec![
        Reply::json(json!({"result": renewed})),
        Reply::content(bytes),
    ])
    .await;
    runtime.node_gateway_port = server.port;
    runtime.replace_token_pair(first.clone()).await.unwrap();
    let store = runtime.get_named_store().await.unwrap();
    *runtime.session_token.write().await =
        token_pair(&runtime, "expired", buckyos_get_unix_timestamp() - 1).session_token;
    let (mut reader, size) = store.open_reader(&id, None).await.unwrap();
    let mut actual = Vec::new();
    reader.read_to_end(&mut actual).await.unwrap();
    assert_eq!(size, bytes.len() as u64);
    assert_eq!(actual, bytes);
    let refresh = server.next().await;
    assert_eq!(refresh.path.trim_end_matches('/'), "/kapi/verify-hub");
    assert_eq!(refresh.body["method"], "refresh_token");
    assert_eq!(refresh.body["params"]["refresh_token"], first.refresh_token);
    let read = server.next().await;
    assert_eq!(read.path, "/ndm/proxy/v1/read/object/open");
    assert_eq!(
        read.headers.get("authorization"),
        Some(&format!("Bearer {}", renewed.session_token))
    );
    assert_eq!(runtime.get_session_token().await, renewed.session_token);
}

#[tokio::test]
async fn named_store_remote_denials_never_read_a_local_copy() {
    let env = TestEnv::new();
    env.configure_local_store();
    let bytes = b"local copy must not bypass authorization";
    let (chunk, id, object) = file_object(bytes);
    let kernel = env.runtime(BuckyOSRuntimeType::Kernel, 1);
    let local = kernel.get_local_named_store().await.unwrap();
    local.put_chunk(&chunk, bytes).await.unwrap();
    local.put_object(&id, &object).await.unwrap();
    for status in [401, 403] {
        let mut server =
            TestServer::start(vec![Reply::denied(status), Reply::denied(status)]).await;
        let mut runtime = env.runtime(BuckyOSRuntimeType::AppClient, server.port);
        let pair = token_pair(&runtime, "denied", buckyos_get_unix_timestamp() + 600);
        runtime.replace_token_pair(pair).await.unwrap();
        let store = runtime.get_named_store().await.unwrap();
        let error = store.get_object(&id).await.unwrap_err();
        assert!(
            error.to_string().contains(&format!("denied-{status}")),
            "{error}"
        );
        let error = match store.open_reader(&id, None).await {
            Ok(_) => panic!("read bypassed remote denial"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(&format!("denied-{status}")),
            "{error}"
        );
        assert_eq!(server.next().await.path, "/ndm/proxy/v1/rpc/get_object");
        assert_eq!(server.next().await.path, "/ndm/proxy/v1/read/object/open");
        assert!(runtime.named_store_mgr.get().is_none());
    }
    assert_eq!(local.get_object(&id).await.unwrap(), object);
}
