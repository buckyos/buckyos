use buckyos_api::{
    Event, KEventDaemonResponse, KEventError, SharedKEventRingBuffer, KEVENT_SERVICE_MAIN_PORT,
    KEVENT_SERVICE_NATIVE_PORT,
};
use kevent::{decode_daemon_request, encode_daemon_response, KEventHttpServer, KEventService};
use log::{error, info};
use server_runner::Runner;
use std::io::{self, ErrorKind};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

const MAX_NATIVE_FRAME_SIZE: usize = 1024 * 1024;
const SHARED_RING_DRAIN_BATCH: usize = 128;
#[cfg(target_os = "linux")]
const SHARED_RING_WAIT_TIMEOUT_MS: u64 = 500;
#[cfg(not(target_os = "linux"))]
const SHARED_RING_WAIT_TIMEOUT_MS: u64 = 1;

pub async fn start_node_kevent_service(service: Arc<KEventService>) {
    info!(
        "start kevent service on http port {} and native tcp port {} for source_node={}",
        KEVENT_SERVICE_MAIN_PORT,
        KEVENT_SERVICE_NATIVE_PORT,
        service.source_node()
    );

    match SharedKEventRingBuffer::open() {
        Ok(shared_ring) => {
            let shared_ring = Arc::new(shared_ring);
            service.set_shared_ring(shared_ring.clone()).await;
            start_shared_ring_importer(service.clone(), shared_ring);
        }
        Err(err) => {
            error!("kevent shared ring disabled: {}", err);
        }
    }

    let http_server = Arc::new(KEventHttpServer::new(service.clone()));
    let runner = Runner::new(KEVENT_SERVICE_MAIN_PORT);

    let add_result = runner.add_http_server("/kapi/kevent".to_string(), http_server);
    if let Err(err) = add_result {
        error!("Failed to add kevent http server: {}", err);
        return;
    }

    let native_service = service.clone();
    tokio::spawn(async move {
        if let Err(err) = run_native_tcp_server(native_service).await {
            error!("kevent native tcp server stopped: {}", err);
        }
    });

    runner.run().await;
}

fn start_shared_ring_importer(
    service: Arc<KEventService>,
    shared_ring: Arc<SharedKEventRingBuffer>,
) {
    shared_ring.prime_cursors();

    let runtime_handle = tokio::runtime::Handle::current();
    if let Err(err) = std::thread::Builder::new()
        .name("kevent-shared-ring-import".to_string())
        .spawn(move || loop {
            let seq_before = shared_ring.load_notify_seq();
            let events = shared_ring.drain_events::<Event>(SHARED_RING_DRAIN_BATCH);

            if !events.is_empty() {
                let service = service.clone();
                runtime_handle.spawn(async move {
                    for event in events {
                        if let Err(err) = service.publish_external_global(event).await {
                            error!("kevent shared ring import failed: {}", err);
                        }
                    }
                });
            }

            shared_ring.wait_for_events(
                seq_before,
                Duration::from_millis(SHARED_RING_WAIT_TIMEOUT_MS),
            );
        })
    {
        error!(
            "failed to start kevent shared ring importer thread: {}",
            err
        );
    }
}

async fn run_native_tcp_server(service: Arc<KEventService>) -> io::Result<()> {
    let addr = format!("0.0.0.0:{}", KEVENT_SERVICE_NATIVE_PORT);
    let listener = TcpListener::bind(&addr).await?;
    info!("kevent native tcp listener bound at {}", addr);

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

async fn handle_native_tcp_connection<S>(
    service: Arc<KEventService>,
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
            Ok(request) => service.handle_protocol_request(request).await,
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
    use buckyos_api::{KEventDaemonRequest, KEventDaemonResponse};
    use serde_json::json;
    use tokio::io::duplex;

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
