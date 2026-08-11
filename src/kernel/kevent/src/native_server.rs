//! Server side of the native TCP protocol (doc/arch/kevent/kevent.md §5.4.1).
//!
//! node-daemon hosts this; it lives here so both ends of the protocol —
//! and the tests that exercise reconnection — sit next to each other.

use crate::{decode_daemon_request, encode_daemon_response, KEventService, KEventSessionId};
use buckyos_api::{KEventDaemonResponse, KEventError};
use log::{error, info};
use std::io::{self, ErrorKind};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

pub const MAX_NATIVE_FRAME_SIZE: usize = 1024 * 1024;

/// Accept loop. Each connection is served independently and owns its own
/// reader namespace.
pub async fn run_native_tcp_server(
    service: Arc<KEventService>,
    listener: TcpListener,
) -> io::Result<()> {
    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let service = service.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_native_tcp_connection(service, stream).await {
                error!("kevent native tcp connection {} failed: {}", peer_addr, err);
            }
        });
    }
}

/// Serve one native TCP connection.
///
/// Every connection owns a reader namespace: reader ids only have to be
/// unique within the connection, and all readers registered on it are
/// reclaimed when it ends — including when the peer crashes without
/// unregistering. That is also how a client "closes" a reader: it drops the
/// connection instead of waiting for its in-flight long poll to return.
pub async fn handle_native_tcp_connection<S>(
    service: Arc<KEventService>,
    stream: S,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let session = service.open_session();
    let result = serve_native_tcp_session(&service, session, stream).await;
    let reclaimed = service.close_session(session).await;
    if reclaimed > 0 {
        info!(
            "kevent native connection {} closed, reclaimed {} reader(s)",
            session, reclaimed
        );
    }
    result
}

async fn serve_native_tcp_session<S>(
    service: &Arc<KEventService>,
    session: KEventSessionId,
    mut stream: S,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let frame_len = match stream.read_u32().await {
            Ok(len) => len as usize,
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(err) => return Err(err),
        };

        if frame_len == 0 || frame_len > MAX_NATIVE_FRAME_SIZE {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("invalid kevent native frame length: {}", frame_len),
            ));
        }

        let mut frame = vec![0_u8; frame_len];
        stream.read_exact(&mut frame).await?;

        let response = match decode_daemon_request(&frame) {
            Ok(request) => service.handle_protocol_request_in(session, request).await,
            Err(err) => error_response(err),
        };

        write_native_tcp_response(&mut stream, response).await?;
    }
}

async fn write_native_tcp_response<S>(
    stream: &mut S,
    response: KEventDaemonResponse,
) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let payload = encode_daemon_response(&response)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err.to_string()))?;
    stream.write_u32(payload.len() as u32).await?;
    stream.write_all(&payload).await?;
    stream.flush().await
}

fn error_response(err: KEventError) -> KEventDaemonResponse {
    KEventDaemonResponse::Err {
        code: err.code().to_string(),
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{map_response_error, KEventPeerPublisher};
    use async_trait::async_trait;
    use buckyos_api::{Event, KEventDaemonRequest, KEventResult};
    use serde_json::json;
    use tokio::io::duplex;

    struct FramedPeerPublisher {
        target: Arc<KEventService>,
    }

    impl FramedPeerPublisher {
        fn new(target: Arc<KEventService>) -> Self {
            Self { target }
        }
    }

    #[async_trait]
    impl KEventPeerPublisher for FramedPeerPublisher {
        async fn broadcast(&self, event: &Event) -> KEventResult<()> {
            let (mut client, server) = duplex(4096);
            let server_task =
                tokio::spawn(handle_native_tcp_connection(self.target.clone(), server));

            write_client_frame(
                &mut client,
                KEventDaemonRequest::PublishGlobal {
                    event: event.clone(),
                },
            )
            .await;
            let response = read_client_frame(&mut client).await;
            drop(client);
            server_task.await.unwrap().unwrap();

            match response {
                KEventDaemonResponse::Ok { .. } => Ok(()),
                KEventDaemonResponse::Err { code, message } => {
                    Err(map_response_error(&code, &message))
                }
            }
        }
    }

    #[tokio::test]
    async fn native_tcp_connection_roundtrip() {
        let service = Arc::new(KEventService::new("node_a"));
        let (mut client, server) = duplex(4096);

        let server_task = tokio::spawn(handle_native_tcp_connection(service.clone(), server));

        let register_req = KEventDaemonRequest::RegisterReader {
            reader_id: "r1".to_string(),
            patterns: vec!["/system/**".to_string()],
        };
        write_client_frame(&mut client, register_req).await;
        let register_resp = read_client_frame(&mut client).await;
        assert!(matches!(
            register_resp,
            KEventDaemonResponse::Ok { event: None }
        ));

        service
            .publish_local_global("/system/node/online", json!({ "ok": true }))
            .await
            .unwrap();

        let pull_req = KEventDaemonRequest::PullEvent {
            reader_id: "r1".to_string(),
            timeout_ms: Some(0),
        };
        write_client_frame(&mut client, pull_req).await;
        let pull_resp = read_client_frame(&mut client).await;
        match pull_resp {
            KEventDaemonResponse::Ok { event: Some(event) } => {
                assert_eq!(event.eventid, "/system/node/online");
            }
            other => panic!("unexpected response: {:?}", other),
        }

        drop(client);
        server_task.await.unwrap().unwrap();
    }

    /// A client that goes away without unregistering must not leave its
    /// reader (and its 1024-event queue) behind forever.
    #[tokio::test]
    async fn dropped_connection_reclaims_its_readers() {
        let service = Arc::new(KEventService::new("node_a"));
        let (mut client, server) = duplex(4096);
        let server_task = tokio::spawn(handle_native_tcp_connection(service.clone(), server));

        write_client_frame(
            &mut client,
            KEventDaemonRequest::RegisterReader {
                reader_id: "r1".to_string(),
                patterns: vec!["/system/**".to_string()],
            },
        )
        .await;
        let _ = read_client_frame(&mut client).await;
        assert_eq!(service.reader_count().await, 1);

        drop(client);
        server_task.await.unwrap().unwrap();
        assert_eq!(service.reader_count().await, 0);
    }

    /// Two independent clients both registering `r1` must not share a queue:
    /// each connection is its own namespace.
    #[tokio::test]
    async fn same_reader_id_on_two_connections_stays_isolated() {
        let service = Arc::new(KEventService::new("node_a"));
        let (mut client_a, server_a) = duplex(4096);
        let (mut client_b, server_b) = duplex(4096);
        let task_a = tokio::spawn(handle_native_tcp_connection(service.clone(), server_a));
        let task_b = tokio::spawn(handle_native_tcp_connection(service.clone(), server_b));

        for (client, pattern) in [
            (&mut client_a, "/tenant/a/**"),
            (&mut client_b, "/tenant/b/**"),
        ] {
            write_client_frame(
                client,
                KEventDaemonRequest::RegisterReader {
                    reader_id: "r1".to_string(),
                    patterns: vec![pattern.to_string()],
                },
            )
            .await;
            let _ = read_client_frame(client).await;
        }
        assert_eq!(service.reader_count().await, 2);

        service
            .publish_local_global("/tenant/a/hello", json!({ "seq": 1 }))
            .await
            .unwrap();

        write_client_frame(
            &mut client_a,
            KEventDaemonRequest::PullEvent {
                reader_id: "r1".to_string(),
                timeout_ms: Some(0),
            },
        )
        .await;
        match read_client_frame(&mut client_a).await {
            KEventDaemonResponse::Ok { event: Some(event) } => {
                assert_eq!(event.eventid, "/tenant/a/hello")
            }
            other => panic!("client a should have received its event: {:?}", other),
        }

        // B subscribed to a different subtree and must see nothing, even
        // though it used the very same reader id.
        write_client_frame(
            &mut client_b,
            KEventDaemonRequest::PullEvent {
                reader_id: "r1".to_string(),
                timeout_ms: Some(0),
            },
        )
        .await;
        assert!(matches!(
            read_client_frame(&mut client_b).await,
            KEventDaemonResponse::Ok { event: None }
        ));

        drop(client_a);
        drop(client_b);
        task_a.await.unwrap().unwrap();
        task_b.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn native_tcp_rejects_invalid_frame_length() {
        let service = Arc::new(KEventService::new("node_a"));

        for invalid_len in [0_u32, (MAX_NATIVE_FRAME_SIZE as u32) + 1] {
            let (mut client, server) = duplex(64);
            let server_task = tokio::spawn(handle_native_tcp_connection(service.clone(), server));

            client.write_u32(invalid_len).await.unwrap();
            drop(client);

            let err = server_task.await.unwrap().unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidData);
            assert!(err
                .to_string()
                .contains("invalid kevent native frame length"));
        }
    }

    #[tokio::test]
    async fn native_framed_peer_publish_delivers_one_way_current_behavior() {
        let service_a = Arc::new(KEventService::new("node_a"));
        let service_b = Arc::new(KEventService::new("node_b"));

        service_a
            .add_peer_publisher(Arc::new(FramedPeerPublisher::new(service_b.clone())))
            .await;
        service_b
            .register_reader("b_reader", vec!["/peer/**".to_string()])
            .await
            .unwrap();

        service_a
            .publish_local_global("/peer/native-framed", json!({"ok": true}))
            .await
            .unwrap();

        let event = service_b
            .pull_event("b_reader", Some(100))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.eventid, "/peer/native-framed");
        assert_eq!(event.source_node, "node_a");
        assert_eq!(event.ingress_node.as_deref(), Some("node_b"));
    }

    async fn write_client_frame(
        stream: &mut tokio::io::DuplexStream,
        request: KEventDaemonRequest,
    ) {
        let payload = serde_json::to_vec(&request).unwrap();
        stream.write_u32(payload.len() as u32).await.unwrap();
        stream.write_all(&payload).await.unwrap();
        stream.flush().await.unwrap();
    }

    async fn read_client_frame(stream: &mut tokio::io::DuplexStream) -> KEventDaemonResponse {
        let frame_len = stream.read_u32().await.unwrap() as usize;
        let mut frame = vec![0_u8; frame_len];
        stream.read_exact(&mut frame).await.unwrap();
        serde_json::from_slice(&frame).unwrap()
    }
}
