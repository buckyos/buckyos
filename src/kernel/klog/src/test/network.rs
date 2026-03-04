use crate::network::{KNetworkClient, RaftRequest, RaftRequestType, RaftResponse};
use crate::{KNode, KNodeId, KTypeConfig};
use axum::Router;
use axum::body::Bytes;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use openraft::Vote;
use openraft::entry::EntryPayload;
use openraft::error::{Fatal, RPCError, RaftError};
use openraft::network::{RPCOption, RPCTypes, RaftNetwork};
use openraft::raft::{AppendEntriesRequest, AppendEntriesResponse, VoteRequest, VoteResponse};
use openraft::{CommittedLeaderId, Entry, LogId};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::task::JoinHandle;
use tower_http::limit::RequestBodyLimitLayer;

struct TestHttpServer {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Drop for TestHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestHttpServer {
    async fn try_start(app: Router) -> anyhow::Result<Option<Self>> {
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                warn!(
                    "skip network tests because socket bind is not permitted in this environment: {}",
                    err
                );
                return Ok(None);
            }
            Err(err) => return Err(err.into()),
        };

        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, app).await {
                error!("test http server stopped with error: {}", err);
            }
        });

        Ok(Some(Self { addr, task }))
    }
}

fn test_client(server: &TestHttpServer) -> KNetworkClient {
    let node = KNode {
        id: 2,
        addr: server.addr.ip().to_string(),
        port: server.addr.port(),
        inter_port: server.addr.port(),
        rpc_port: server.addr.port(),
    };
    KNetworkClient::new(1, 2, node)
}

fn vote_request() -> VoteRequest<KNodeId> {
    VoteRequest::new(Vote::new(3, 1), None)
}

fn append_entries_request() -> AppendEntriesRequest<KTypeConfig> {
    AppendEntriesRequest {
        vote: Vote::new(3, 1),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(3, 1), 1)),
        entries: vec![Entry {
            log_id: LogId::new(CommittedLeaderId::new(3, 1), 2),
            payload: EntryPayload::Blank,
        }],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(3, 1), 1)),
    }
}

fn octet_stream_response(status: StatusCode, body: Vec<u8>) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        body,
    )
        .into_response()
}

#[tokio::test]
async fn test_network_vote_success_roundtrip() -> anyhow::Result<()> {
    let vote_path = RaftRequestType::Vote.klog_path();
    let app = Router::new().route(
        &vote_path,
        post(|body: Bytes| async move {
            let req = match RaftRequest::deserialize(&body) {
                Ok(req) => req,
                Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
            };

            match req {
                RaftRequest::Vote(v) => {
                    if v.vote != Vote::new(3, 1) {
                        return (StatusCode::BAD_REQUEST, "unexpected vote request")
                            .into_response();
                    }
                }
                _ => return (StatusCode::BAD_REQUEST, "unexpected request type").into_response(),
            }

            let resp = RaftResponse::Vote(VoteResponse::new(Vote::new(3, 2), None, true));
            let bytes = resp.serialize().expect("serialize response");
            octet_stream_response(StatusCode::OK, bytes)
        }),
    );

    let Some(server) = TestHttpServer::try_start(app).await? else {
        return Ok(());
    };
    let mut client = test_client(&server);
    let resp = client
        .vote(vote_request(), RPCOption::new(Duration::from_secs(1)))
        .await?;

    assert_eq!(resp.vote, Vote::new(3, 2));
    assert!(resp.vote_granted);
    assert!(resp.last_log_id.is_none());
    Ok(())
}

#[tokio::test]
async fn test_network_timeout_maps_to_rpc_timeout() -> anyhow::Result<()> {
    let vote_path = RaftRequestType::Vote.klog_path();
    let app = Router::new().route(
        &vote_path,
        post(|| async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let resp = RaftResponse::Vote(VoteResponse::new(Vote::new(1, 1), None, true));
            let bytes = resp.serialize().expect("serialize response");
            octet_stream_response(StatusCode::OK, bytes)
        }),
    );

    let Some(server) = TestHttpServer::try_start(app).await? else {
        return Ok(());
    };
    let mut client = test_client(&server);
    let hard_ttl = Duration::from_millis(120);
    let err = client
        .vote(vote_request(), RPCOption::new(hard_ttl))
        .await
        .expect_err("vote should timeout");

    match err {
        RPCError::Timeout(timeout) => {
            assert_eq!(timeout.action, RPCTypes::Vote);
            assert_eq!(timeout.id, 1);
            assert_eq!(timeout.target, 2);
            assert_eq!(timeout.timeout, hard_ttl);
        }
        other => panic!("unexpected error: {}", other),
    }

    Ok(())
}

#[tokio::test]
async fn test_network_payload_too_large_maps_to_rpc_payload_too_large() -> anyhow::Result<()> {
    let append_entries_path = RaftRequestType::AppendEntries.klog_path();
    let app = Router::new()
        .route(
            &append_entries_path,
            // Force body consumption path so RequestBodyLimit is enforced deterministically.
            post(|_body: Bytes| async move {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unexpected handler invocation",
                )
                    .into_response()
            }),
        )
        .route_layer(RequestBodyLimitLayer::new(1));

    let Some(server) = TestHttpServer::try_start(app).await? else {
        return Ok(());
    };
    let mut client = test_client(&server);
    let err = client
        .append_entries(
            append_entries_request(),
            RPCOption::new(Duration::from_secs(1)),
        )
        .await
        .expect_err("append_entries should be rejected by body size limit");

    match err {
        RPCError::PayloadTooLarge(payload_too_large) => {
            assert_eq!(payload_too_large.action(), RPCTypes::AppendEntries);
            assert!(payload_too_large.entries_hint() > 0);
        }
        other => panic!("unexpected error: {}", other),
    }
    Ok(())
}

#[tokio::test]
async fn test_network_vote_payload_too_large_maps_to_network_error() -> anyhow::Result<()> {
    let vote_path = RaftRequestType::Vote.klog_path();
    let app = Router::new()
        .route(
            &vote_path,
            post(|_body: Bytes| async move {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unexpected handler invocation",
                )
                    .into_response()
            }),
        )
        .route_layer(RequestBodyLimitLayer::new(1));

    let Some(server) = TestHttpServer::try_start(app).await? else {
        return Ok(());
    };
    let mut client = test_client(&server);
    let err = client
        .vote(vote_request(), RPCOption::new(Duration::from_secs(1)))
        .await
        .expect_err("vote should be rejected by body size limit");

    assert!(matches!(err, RPCError::Network(_)));
    Ok(())
}

#[tokio::test]
async fn test_network_vote_remote_error_passthrough() -> anyhow::Result<()> {
    let vote_path = RaftRequestType::Vote.klog_path();
    let app = Router::new().route(
        &vote_path,
        post(|| async move {
            let remote_error = RaftError::<KNodeId>::Fatal(Fatal::Stopped);
            let resp = RaftResponse::VoteError(remote_error);
            let bytes = resp.serialize().expect("serialize response");
            octet_stream_response(StatusCode::OK, bytes)
        }),
    );

    let Some(server) = TestHttpServer::try_start(app).await? else {
        return Ok(());
    };
    let mut client = test_client(&server);
    let err = client
        .vote(vote_request(), RPCOption::new(Duration::from_secs(1)))
        .await
        .expect_err("vote should return remote error");

    match err {
        RPCError::RemoteError(remote) => {
            assert_eq!(remote.target, 2);
            assert_eq!(
                remote.target_node.expect("target node").port,
                server.addr.port()
            );
            assert_eq!(remote.source, RaftError::<KNodeId>::Fatal(Fatal::Stopped));
        }
        other => panic!("unexpected error variant: {}", other),
    }

    Ok(())
}

#[tokio::test]
async fn test_network_out_of_order_response_type_maps_to_network_error() -> anyhow::Result<()> {
    let vote_path = RaftRequestType::Vote.klog_path();
    let app = Router::new().route(
        &vote_path,
        post(|| async move {
            // Return append-entries response on vote endpoint to simulate response disorder.
            let resp = RaftResponse::AppendEntries(AppendEntriesResponse::Success);
            let bytes = resp.serialize().expect("serialize response");
            octet_stream_response(StatusCode::OK, bytes)
        }),
    );

    let Some(server) = TestHttpServer::try_start(app).await? else {
        return Ok(());
    };
    let mut client = test_client(&server);
    let err = client
        .vote(vote_request(), RPCOption::new(Duration::from_secs(1)))
        .await
        .expect_err("vote should fail on mismatched response type");

    assert!(matches!(err, RPCError::Network(_)));
    Ok(())
}

#[tokio::test]
async fn test_network_corrupted_packet_maps_to_unreachable() -> anyhow::Result<()> {
    let vote_path = RaftRequestType::Vote.klog_path();
    let app = Router::new().route(
        &vote_path,
        post(|| async move {
            octet_stream_response(StatusCode::OK, b"corrupted-response-payload".to_vec())
        }),
    );

    let Some(server) = TestHttpServer::try_start(app).await? else {
        return Ok(());
    };
    let mut client = test_client(&server);
    let err = client
        .vote(vote_request(), RPCOption::new(Duration::from_secs(1)))
        .await
        .expect_err("vote should fail on corrupted packet");

    assert!(matches!(err, RPCError::Unreachable(_)));
    Ok(())
}
