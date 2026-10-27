mod common;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use agent_tool::runtime::node::{NodeExecutorConnector, NodeIo, RemoteNodeRuntime};
use agent_tool::runtime::RuntimeConfig;
use async_trait::async_trait;
use base64::Engine;
use buckyos_api::{encode_executor_control, executor_content_hash, read_executor_frame, ExecutorFrame, ExecutorLimits};
use common::*;
use libopendan::bridge::OutboundRecord;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, ObjectStore, OutboundSink, SendResult, StopWhen, StoredObject};
use libopendan::{AgentStateClient, SessionTemplate};
use ndn_lib::{ObjId, RefTarget};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

const NODE: &str = "did:web:remote.test.buckyos.io";
const ROOT: &str = "/remote-only/session";
const FILE: &str = "/remote-only/session/cat_halloween.mp4";

struct Node {
    payload: Arc<Vec<u8>>,
    reads: Arc<Mutex<Vec<(u64, u64)>>>,
}

#[async_trait]
impl NodeExecutorConnector for Node {
    async fn connect(&self, node: &str) -> std::io::Result<Box<dyn NodeIo>> {
        assert_eq!(node, NODE);
        let payload = self.payload.clone();
        let reads = self.reads.clone();
        let (client, mut server) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            let mut operations: HashMap<String, (Value, Vec<u8>)> = HashMap::new();
            while let Some(ExecutorFrame::Control(bytes)) = read_executor_frame(&mut server).await.unwrap() {
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                let id = &request["request_id"];
                let response = match request["type"].as_str().unwrap() {
                    "login" => {
                        assert_eq!(request["token"], "test-session");
                        assert_eq!(request["profile"], "media");
                        json!({"type":"login_ok", "request_id":id, "posture":"normal",
                            "principal":AGENT, "app_instance_id":"app2@alice", "caps":["read","stat","exec"],
                            "limits":ExecutorLimits::default(), "revision":"one", "generation":1,
                            "auth_valid_until_ms":u64::MAX, "token_expires_at_ms":u64::MAX,
                            "store_id":"test-ledger", "server_epoch":"one"})
                    }
                    "describe" => {
                        assert!(request["workdir"].is_null() || request["workdir"] == ROOT);
                        json!({"type":"describe_ok", "request_id":id,
                            "target":{"node_did":NODE, "profile_id":"media", "uid":65534, "gid":65534,
                                "workdir":request["workdir"], "workdir_roots":[ROOT], "store_id":"test-ledger", "protocol_version":1},
                            "info":{"os":"linux", "arch":"x86_64", "hostname":"remote", "shell":"bash",
                                "time_ms":1, "identity_mode":"switch_user"}})
                    }
                    kind @ ("stat" | "read") => {
                        assert_eq!(request["store_id"], "test-ledger");
                        assert_eq!(request["path"], FILE);
                        let (file, data, content) = if kind == "stat" {
                            (json!({"path":FILE, "size":0, "hash":"", "path_type":"file"}), vec![], Value::Null)
                        } else {
                            let offset = request["offset"].as_u64().unwrap();
                            let length = request["length"].as_u64().unwrap();
                            assert!(length <= ExecutorLimits::default().max_file_bytes);
                            reads.lock().unwrap().push((offset, length));
                            let end = (offset + length).min(payload.len() as u64) as usize;
                            let data = payload[offset as usize..end].to_vec();
                            let content = json!({"total_bytes":data.len(), "retained_bytes":data.len(),
                                "truncated":false, "hash":executor_content_hash(&data)});
                            (json!({"path":FILE, "size":payload.len(), "hash":""}), data, content)
                        };
                        let op = json!({"op_id":request["op_id"], "kind":kind, "state":"succeeded", "accepted_at_ms":1,
                            "result":{"state":"succeeded", "file":file, "content":content, "finished_at_ms":2}});
                        operations.insert(request["op_id"].as_str().unwrap().into(), (op.clone(), data));
                        json!({"type":"accepted", "request_id":id, "op":op})
                    }
                    "query" => {
                        let (op, data) = operations.get(request["op_id"].as_str().unwrap()).unwrap();
                        let chunk = if request["channel"].is_null() {
                            Value::Null
                        } else {
                            assert_eq!(request["channel"], "content");
                            let offset = request["offset"].as_u64().unwrap() as usize;
                            let end = (offset + request["max_bytes"].as_u64().unwrap() as usize).min(data.len());
                            json!({"channel":"content", "offset":offset, "next_offset":end, "retained_from":0,
                                "truncated":false, "eof":end == data.len(),
                                "data_b64":base64::engine::general_purpose::STANDARD.encode(&data[offset..end])})
                        };
                        json!({"type":"query_ok", "request_id":id, "op":op, "chunk":chunk})
                    }
                    other => panic!("unexpected executor operation: {other}"),
                };
                server.write_all(&encode_executor_control(&response).unwrap()).await.unwrap();
            }
        });
        Ok(Box::new(client))
    }

    async fn session_token(&self) -> Result<String, String> {
        Ok("test-session".into())
    }
}

#[derive(Default)]
struct Objects(Mutex<Vec<(String, Vec<u8>)>>);

impl Objects {
    fn id() -> ObjId {
        ObjId::new_by_raw("cyfile".into(), vec![23; 32])
    }

    const URL: &'static str = "obj://bucket/user/jarvis/alice/artifacts/1717/cat_halloween.mp4";
}

#[async_trait]
impl ObjectStore for Objects {
    async fn put_file(&self, path: &Path, name: &str) -> Result<StoredObject, String> {
        assert!(path.components().any(|part| part.as_os_str() == "reports"));
        self.0.lock().unwrap().push((name.into(), std::fs::read(path).unwrap()));
        Ok(StoredObject { obj_id: Self::id(), url: Some(Self::URL.into()) })
    }

    async fn name_of(&self, id: &ObjId) -> Option<String> {
        assert_eq!(*id, Self::id());
        Some("cat_halloween.mp4".into())
    }
}

#[derive(Default)]
struct Sink(Mutex<Vec<OutboundRecord>>);

#[async_trait]
impl OutboundSink for Sink {
    async fn send(&self, _: &str, record: &OutboundRecord) -> SendResult {
        self.0.lock().unwrap().push(record.clone());
        SendResult::Sent { msg_id: None, deliveries: vec![] }
    }
}

#[tokio::test]
async fn a_remote_report_uploads_the_file_and_the_ui_forwards_its_object_id() {
    let env = Env::new();
    let agent = env.agent();
    let payload = Arc::new((0..2 * 1024 * 1024 + 137).map(|i| (i % 251) as u8).collect::<Vec<_>>());
    let node = Arc::new(Node { payload: payload.clone(), reads: Arc::new(Mutex::new(vec![])) });
    let config: RuntimeConfig = serde_json::from_value(json!({"kind":"remote_node", "id":"remote-media",
        "workdir":ROOT, "remote_node":{"node_did":NODE, "profile_id":"media"}})).unwrap();
    agent.runtimes().register(&RuntimeRegister {
        runtime_id: "remote-media".into(), kind: RuntimeKind::RemoteNode, name: "Remote media".into(),
        description: String::new(), exec: Some(serde_json::to_value(&config).unwrap()), roots: vec![],
        capabilities: json!({}), default: false, computer: Default::default(),
        baseline: buckyos_api::RuntimeBaseline::Standard,
    }, APP).await.unwrap();
    let mut spec = SessionTemplate::load("work", None).unwrap().spec("deliver the remote video");
    spec.prompt.llm_context = json!({"tools":{"enabled":true,"tools":[{"groupname":"bash"},{"name":"report"}]}});
    spec.runtime.requirement.runtime_id = Some("remote-media".into());
    let work = env.create_work(spec).await;
    assert!(work.config().unwrap().runtime.is_node());
    assert!(!work.path().join("cat_halloween.mp4").exists());
    let objects = Arc::new(Objects::default());
    let mut deps = env.deps(ScriptedLlm::new(|_, n| {
        assert_eq!(n, 0);
        tool_call("video", "report", json!({"report":"Video ready", "artifacts":["cat_halloween.mp4"], "is_end":true}))
    }));
    deps.runtime = Arc::new(RemoteNodeRuntime::new(config, node.clone()));
    deps.xllm.node_executor = Some(node.clone());
    deps.objects = Some(objects.clone());
    let result = drive(&work, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    let binding = work.binding_opt().unwrap().unwrap();
    assert_eq!(binding.kind, "remote_node");
    assert_eq!(binding.target["store_id"], "test-ledger");
    assert_eq!(binding.workdir, ROOT);
    let report = work.state().unwrap().final_report.unwrap();
    let artifact = &report.artifacts[0];
    assert_eq!(artifact.obj_id.as_deref(), Some(Objects::id().to_string().as_str()));
    assert!(!artifact.reference.starts_with("remote-media:"));
    assert_eq!(artifact.url.as_deref(), Some(Objects::URL));
    // The bucket retains it: the downloaded copy is gone.
    assert!(!work.path().join(&artifact.reference).exists());
    assert_eq!(*objects.0.lock().unwrap(), vec![("cat_halloween.mp4".into(), payload.as_ref().clone())]);
    assert_eq!(node.reads.lock().unwrap().len(), 3);

    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("relay the completed video");
    spec.prompt.llm_context = json!({"tools":{"enabled":true,"tools":[{"groupname":"bash"},{"name":"report"}]}});
    spec.outbound = Some(OutboundBinding { to: "did:bns:bob".into(), to_session: None, kind: "chat".into() });
    let ui = env.create_work(spec).await;
    let object_id = artifact.obj_id.clone().unwrap();
    let mut deps = env.deps(ScriptedLlm::new(move |_, n| {
        assert_eq!(n, 0);
        tool_call("relay", "report", json!({"report":"Here is the video", "artifacts":[object_id], "is_end":true}))
    }));
    let sink = Arc::new(Sink::default());
    deps.objects = Some(objects.clone());
    deps.outbound = Some(sink.clone());
    let input = PostedInput::msg(APP,
        text_msg(&parse_did("did:bns:bob").unwrap(), &parse_did(AGENT).unwrap(), &report.delivery_text()),
        MsgDelivery::default()).unwrap();
    libopendan::post_input(agent.as_ref(), ui.sid(), &input).await.unwrap();
    let result = drive(&ui, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    let sent = sink.0.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].msg.content.content, "Here is the video");
    assert_eq!(sent[0].msg.content.refs.len(), 1);
    assert!(matches!(&sent[0].msg.content.refs[0].target, RefTarget::DataObj { obj_id, .. } if *obj_id == Objects::id()));
    assert_eq!(sent[0].msg.content.refs[0].label.as_deref(), Some("cat_halloween.mp4"));
    assert_eq!(objects.0.lock().unwrap().len(), 1);
    assert_eq!(node.reads.lock().unwrap().len(), 3);
}
