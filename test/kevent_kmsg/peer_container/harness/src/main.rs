use async_trait::async_trait;
use buckyos_api::{Event, KEventDaemonRequest, KEventDaemonResponse, KEventError, KEventResult};
use kevent::{
    decode_daemon_request, encode_daemon_response, map_response_error, KEventPeerPublisher,
    KEventService,
};
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::sleep;

const MAX_FRAME_SIZE: usize = 1024 * 1024;

#[derive(Clone)]
struct TcpPeerPublisher {
    target: String,
}

#[async_trait]
impl KEventPeerPublisher for TcpPeerPublisher {
    async fn broadcast(&self, event: &Event) -> KEventResult<()> {
        let response = call(
            &self.target,
            KEventDaemonRequest::PublishGlobal {
                event: event.clone(),
            },
        )
        .await?;
        match response {
            KEventDaemonResponse::Ok { .. } => Ok(()),
            KEventDaemonResponse::Err { code, message } => Err(map_response_error(&code, &message)),
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

async fn run() -> KEventResult<()> {
    let args = env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("server") => run_server(&args).await,
        Some("client") => run_client(&args).await,
        _ => Err(KEventError::InvalidEventId(
            "usage: kevent_peer_harness server|client ...".to_string(),
        )),
    }
}

async fn run_server(args: &[String]) -> KEventResult<()> {
    let node = arg_value(args, "--node")?;
    let listen = arg_value(args, "--listen")?;
    let peer = optional_arg_value(args, "--peer");
    let service = Arc::new(KEventService::new(node.to_string()));
    if let Some(peer) = peer {
        service
            .add_peer_publisher(Arc::new(TcpPeerPublisher {
                target: peer.to_string(),
            }))
            .await;
    }

    let listener = TcpListener::bind(listen)
        .await
        .map_err(|err| KEventError::Internal(format!("bind {listen} failed: {err}")))?;
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|err| KEventError::Internal(format!("accept failed: {err}")))?;
        let service = service.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(service, stream).await {
                eprintln!("{err}");
            }
        });
    }
}

async fn run_client(args: &[String]) -> KEventResult<()> {
    let node_a = arg_value(args, "--node-a")?;
    let node_b = arg_value(args, "--node-b")?;
    let tag = format!("{}", now_millis());
    let eventid = format!("/peer/container/{tag}");
    let reader_id = format!("reader-{tag}");

    retry_call(
        node_b,
        KEventDaemonRequest::RegisterReader {
            reader_id: reader_id.clone(),
            patterns: vec!["/peer/**".to_string()],
        },
    )
    .await?;

    retry_call(
        node_a,
        KEventDaemonRequest::PublishGlobal {
            event: Event {
                eventid: eventid.clone(),
                source_node: "external-client".to_string(),
                source_pid: std::process::id(),
                ingress_node: None,
                timestamp: now_millis(),
                data: json!({ "tag": tag }),
            },
        },
    )
    .await?;

    let mut delivered = None;
    for _ in 0..20 {
        match call(
            node_b,
            KEventDaemonRequest::PullEvent {
                reader_id: reader_id.clone(),
                timeout_ms: Some(200),
            },
        )
        .await?
        {
            KEventDaemonResponse::Ok { event: Some(event) } => {
                delivered = Some(event);
                break;
            }
            KEventDaemonResponse::Ok { event: None } => sleep(Duration::from_millis(100)).await,
            KEventDaemonResponse::Err { code, message } => {
                return Err(map_response_error(&code, &message));
            }
        }
    }

    let event =
        delivered.ok_or_else(|| KEventError::Internal("node_b did not receive event".to_string()))?;
    if event.eventid != eventid {
        return Err(KEventError::Internal(format!(
            "unexpected eventid: {}",
            event.eventid
        )));
    }
    if event.source_node != "external-client" {
        return Err(KEventError::Internal(format!(
            "unexpected source_node: {}",
            event.source_node
        )));
    }

    println!(
        "{}",
        json!({
            "status": "passed",
            "eventid": event.eventid,
            "source_node": event.source_node,
            "ingress_node": event.ingress_node,
        })
    );
    Ok(())
}

async fn retry_call(target: &str, request: KEventDaemonRequest) -> KEventResult<KEventDaemonResponse> {
    let mut last_error = None;
    for _ in 0..30 {
        match call(target, request.clone()).await {
            Ok(response) => return Ok(response),
            Err(err) => {
                last_error = Some(err);
                sleep(Duration::from_millis(200)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| KEventError::Internal("retry failed".to_string())))
}

async fn call(target: &str, request: KEventDaemonRequest) -> KEventResult<KEventDaemonResponse> {
    let mut stream = TcpStream::connect(target)
        .await
        .map_err(|err| KEventError::DaemonUnavailable(format!("connect {target} failed: {err}")))?;
    let payload = kevent::encode_daemon_request(&request)?;
    stream
        .write_u32(payload.len() as u32)
        .await
        .map_err(|err| KEventError::Internal(format!("write frame length failed: {err}")))?;
    stream
        .write_all(&payload)
        .await
        .map_err(|err| KEventError::Internal(format!("write frame payload failed: {err}")))?;
    stream
        .flush()
        .await
        .map_err(|err| KEventError::Internal(format!("flush frame failed: {err}")))?;

    let frame_len = stream
        .read_u32()
        .await
        .map_err(|err| KEventError::Internal(format!("read response length failed: {err}")))?
        as usize;
    if frame_len == 0 || frame_len > MAX_FRAME_SIZE {
        return Err(KEventError::Internal(format!(
            "invalid response frame length: {frame_len}"
        )));
    }
    let mut frame = vec![0_u8; frame_len];
    stream
        .read_exact(&mut frame)
        .await
        .map_err(|err| KEventError::Internal(format!("read response payload failed: {err}")))?;
    kevent::decode_daemon_response(&frame)
}

async fn handle_connection(service: Arc<KEventService>, mut stream: TcpStream) -> KEventResult<()> {
    loop {
        let frame_len = match stream.read_u32().await {
            Ok(len) => len as usize,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(err) => {
                return Err(KEventError::Internal(format!("read request length failed: {err}")));
            }
        };
        if frame_len == 0 || frame_len > MAX_FRAME_SIZE {
            return Err(KEventError::Internal(format!(
                "invalid request frame length: {frame_len}"
            )));
        }
        let mut frame = vec![0_u8; frame_len];
        stream
            .read_exact(&mut frame)
            .await
            .map_err(|err| KEventError::Internal(format!("read request payload failed: {err}")))?;
        let response = match decode_daemon_request(&frame) {
            Ok(request) => service.handle_protocol_request(request).await,
            Err(err) => KEventDaemonResponse::Err {
                code: err.code().to_string(),
                message: err.to_string(),
            },
        };
        let payload = encode_daemon_response(&response)?;
        stream
            .write_u32(payload.len() as u32)
            .await
            .map_err(|err| KEventError::Internal(format!("write response length failed: {err}")))?;
        stream
            .write_all(&payload)
            .await
            .map_err(|err| KEventError::Internal(format!("write response payload failed: {err}")))?;
        stream
            .flush()
            .await
            .map_err(|err| KEventError::Internal(format!("flush response failed: {err}")))?;
    }
}

fn arg_value<'a>(args: &'a [String], name: &str) -> KEventResult<&'a str> {
    optional_arg_value(args, name)
        .ok_or_else(|| KEventError::InvalidEventId(format!("missing argument {name}")))
}

fn optional_arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
