use async_trait::async_trait;
use buckyos_api::msg_queue::*;
use kRPC::{RPCContext, RPCErrors, RPCHandler, RPCRequest, RPCResult};
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
struct CreateObservation {
    name: Option<String>,
    appid: String,
    app_owner: String,
    ctx: RPCContext,
}

struct FakeMsgQueue {
    last_create: Mutex<Option<CreateObservation>>,
    fail_get_stats: bool,
}

impl FakeMsgQueue {
    fn new() -> Self {
        Self {
            last_create: Mutex::new(None),
            fail_get_stats: false,
        }
    }

    fn failing_get_stats() -> Self {
        Self {
            last_create: Mutex::new(None),
            fail_get_stats: true,
        }
    }
}

#[async_trait]
impl MsgQueueHandler for FakeMsgQueue {
    async fn handle_create_queue(
        &self,
        name: Option<&str>,
        appid: &str,
        app_owner: &str,
        _config: QueueConfig,
        ctx: RPCContext,
    ) -> Result<QueueUrn, RPCErrors> {
        *self.last_create.lock().await = Some(CreateObservation {
            name: name.map(ToOwned::to_owned),
            appid: appid.to_string(),
            app_owner: app_owner.to_string(),
            ctx,
        });
        Ok(calc_queue_urn(
            appid,
            app_owner,
            name.unwrap_or("generated"),
        ))
    }

    async fn handle_delete_queue(
        &self,
        _queue_urn: &str,
        _ctx: RPCContext,
    ) -> Result<(), RPCErrors> {
        Ok(())
    }

    async fn handle_get_queue_stats(
        &self,
        _queue_urn: &str,
        _ctx: RPCContext,
    ) -> Result<QueueStats, RPCErrors> {
        if self.fail_get_stats {
            return Err(RPCErrors::ReasonError("stats failed".to_string()));
        }
        Ok(QueueStats::default())
    }

    async fn handle_update_queue_config(
        &self,
        _queue_urn: &str,
        _config: QueueConfig,
        _ctx: RPCContext,
    ) -> Result<(), RPCErrors> {
        Ok(())
    }

    async fn handle_post_message(
        &self,
        _queue_urn: &str,
        _message: Message,
        _ctx: RPCContext,
    ) -> Result<MsgIndex, RPCErrors> {
        Ok(7)
    }

    async fn handle_subscribe(
        &self,
        _queue_urn: &str,
        _user_id: &str,
        _app_id: &str,
        sub_id: Option<String>,
        _position: SubPosition,
        _ctx: RPCContext,
    ) -> Result<SubscriptionId, RPCErrors> {
        Ok(sub_id.unwrap_or_else(|| "sub-generated".to_string()))
    }

    async fn handle_unsubscribe(&self, _sub_id: &str, _ctx: RPCContext) -> Result<(), RPCErrors> {
        Ok(())
    }

    async fn handle_fetch_messages(
        &self,
        _sub_id: &str,
        _length: usize,
        _auto_commit: bool,
        _ctx: RPCContext,
    ) -> Result<Vec<Message>, RPCErrors> {
        Ok(vec![Message::new(b"fake".to_vec())])
    }

    async fn handle_read_message(
        &self,
        _queue_urn: &str,
        _cursor: MsgIndex,
        _length: usize,
        _ctx: RPCContext,
    ) -> Result<Vec<Message>, RPCErrors> {
        Ok(vec![Message::new(b"fake".to_vec())])
    }

    async fn handle_commit_ack(
        &self,
        _sub_id: &str,
        _index: MsgIndex,
        _ctx: RPCContext,
    ) -> Result<(), RPCErrors> {
        Ok(())
    }

    async fn handle_seek(
        &self,
        _sub_id: &str,
        _index: SubPosition,
        _ctx: RPCContext,
    ) -> Result<(), RPCErrors> {
        Ok(())
    }

    async fn handle_delete_message_before(
        &self,
        _queue_urn: &str,
        _index: MsgIndex,
        _ctx: RPCContext,
    ) -> Result<u64, RPCErrors> {
        Ok(3)
    }
}

fn rpc_req(method: &str, params: serde_json::Value) -> RPCRequest {
    let mut req = RPCRequest::new(method, params);
    req.seq = 42;
    req.token = Some("session-token".to_string());
    req.trace_id = Some("trace-1".to_string());
    req
}

fn localhost() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
}

#[tokio::test]
async fn create_queue_parses_params_and_preserves_rpc_metadata() {
    let fake = FakeMsgQueue::new();
    let server = MsgQueueServerHandler::new(fake);
    let params = serde_json::to_value(MsgQueueCreateQueueReq::new(
        Some("alpha".to_string()),
        "app".to_string(),
        "owner".to_string(),
        QueueConfig::default(),
    ))
    .unwrap();

    let response = server
        .handle_rpc_call(rpc_req("create_queue", params), localhost())
        .await
        .unwrap();

    assert_eq!(response.seq, 42);
    assert_eq!(response.trace_id.as_deref(), Some("trace-1"));
    match response.result {
        RPCResult::Success(value) => assert_eq!(value, json!("app::owner::alpha")),
        RPCResult::Failed(err) => panic!("unexpected rpc failure: {}", err),
    }

    let observed = server.0.last_create.lock().await.clone().unwrap();
    assert_eq!(observed.name.as_deref(), Some("alpha"));
    assert_eq!(observed.appid, "app");
    assert_eq!(observed.app_owner, "owner");
    assert_eq!(observed.ctx.seq, 42);
    assert_eq!(observed.ctx.token.as_deref(), Some("session-token"));
    assert_eq!(observed.ctx.trace_id.as_deref(), Some("trace-1"));
    assert_eq!(observed.ctx.from_ip, Some(localhost()));
    assert!(observed.ctx.is_rpc);
}

#[tokio::test]
async fn unknown_method_returns_rpc_error() {
    let server = MsgQueueServerHandler::new(FakeMsgQueue::new());
    let err = server
        .handle_rpc_call(rpc_req("missing_method", json!({})), localhost())
        .await
        .unwrap_err();
    assert!(matches!(err, RPCErrors::UnknownMethod(method) if method == "missing_method"));
}

#[tokio::test]
async fn malformed_params_return_parse_error() {
    let server = MsgQueueServerHandler::new(FakeMsgQueue::new());
    let err = server
        .handle_rpc_call(
            rpc_req(
                "create_queue",
                json!({
                    "name": "alpha",
                    "appid": "app"
                }),
            ),
            localhost(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RPCErrors::ParseRequestError(_)));
}

#[tokio::test]
async fn wrong_param_type_returns_parse_error() {
    let server = MsgQueueServerHandler::new(FakeMsgQueue::new());
    let err = server
        .handle_rpc_call(
            rpc_req(
                "fetch_messages",
                json!({
                    "sub_id": "sub",
                    "length": "not-a-number",
                    "auto_commit": true
                }),
            ),
            localhost(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RPCErrors::ParseRequestError(_)));
}

#[tokio::test]
async fn handler_error_is_propagated_as_rpc_error() {
    let server = MsgQueueServerHandler::new(FakeMsgQueue::failing_get_stats());
    let err = server
        .handle_rpc_call(
            rpc_req("get_queue_stats", json!({ "queue_urn": "missing" })),
            localhost(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RPCErrors::ReasonError(message) if message == "stats failed"));
}
