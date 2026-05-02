use crate::KEventService;
use async_trait::async_trait;
use buckyos_api::{Event, KEventDaemonRequest, KEventDaemonResponse, KEventError};
use buckyos_http_server::{
    server_err, HttpServer, ServerError, ServerErrorCode, ServerResult, StreamInfo,
};
use bytes::Bytes;
use futures::{stream, TryStreamExt};
use http::header::{CACHE_CONTROL, CONTENT_TYPE};
use http::{Method, StatusCode, Version};
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::Frame;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

const DEFAULT_HTTP_STREAM_KEEPALIVE_MS: u64 = 15_000;

#[derive(Debug, Deserialize)]
pub struct KEventHttpStreamRequest {
    pub patterns: Vec<String>,
    #[serde(default)]
    pub keepalive_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct KEventHttpPublishRequest {
    pub eventid: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KEventStreamFrame {
    Ack {
        connection_id: String,
        keepalive_ms: u64,
    },
    Event {
        event: Event,
    },
    Keepalive {
        at_ms: u64,
    },
    Error {
        error: String,
    },
}

#[derive(Clone)]
pub struct KEventHttpServer {
    service: Arc<KEventService>,
    stream_seq: Arc<AtomicU64>,
}

impl KEventHttpServer {
    pub fn new(service: Arc<KEventService>) -> Self {
        Self {
            service,
            stream_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn service(&self) -> Arc<KEventService> {
        self.service.clone()
    }

    pub async fn handle_http_request(
        &self,
        path: &str,
        body: &[u8],
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        match normalize_http_path(path) {
            Some(KEventHttpRoute::Native) => self.handle_native_http(body).await,
            Some(KEventHttpRoute::Stream) => self.handle_stream_http(body).await,
            Some(KEventHttpRoute::Publish) => self.handle_publish_http(body).await,
            None => Self::build_http_json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": format!("Unsupported kevent path: {}", path) }),
            ),
        }
    }

    async fn handle_native_http(
        &self,
        body: &[u8],
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let req = serde_json::from_slice::<KEventDaemonRequest>(body).map_err(|error| {
            server_err!(
                ServerErrorCode::BadRequest,
                "Invalid kevent native request: {}",
                error
            )
        })?;
        let resp = self.service.handle_protocol_request(req).await;
        Self::build_daemon_response(StatusCode::OK, resp)
    }

    async fn handle_publish_http(
        &self,
        body: &[u8],
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let req = serde_json::from_slice::<KEventHttpPublishRequest>(body).map_err(|error| {
            server_err!(
                ServerErrorCode::BadRequest,
                "Invalid kevent publish request: {}",
                error
            )
        })?;

        match self
            .service
            .publish_http_global(req.eventid.as_str(), req.data)
            .await
        {
            Ok(_) => Self::build_daemon_response(
                StatusCode::OK,
                KEventDaemonResponse::Ok { event: None },
            ),
            Err(err) => Self::build_daemon_response(
                status_from_kevent_error(&err),
                KEventDaemonResponse::Err {
                    code: err.code().to_string(),
                    message: err.to_string(),
                },
            ),
        }
    }

    async fn handle_stream_http(
        &self,
        body: &[u8],
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let req = serde_json::from_slice::<KEventHttpStreamRequest>(body).map_err(|error| {
            server_err!(
                ServerErrorCode::BadRequest,
                "Invalid kevent stream request: {}",
                error
            )
        })?;

        let keepalive_ms = normalize_keepalive_ms(req.keepalive_ms);
        let reader_id = format!(
            "http_stream_{}",
            self.stream_seq.fetch_add(1, Ordering::Relaxed) + 1
        );

        if let Err(err) = self
            .service
            .register_reader(reader_id.as_str(), req.patterns)
            .await
        {
            return Self::build_http_json_response(
                status_from_kevent_error(&err),
                json!({ "error": err.to_string() }),
            );
        }

        let (sender, receiver) = mpsc::channel::<std::result::Result<Bytes, ServerError>>(32);
        if !Self::send_stream_frame(
            &sender,
            &KEventStreamFrame::Ack {
                connection_id: reader_id.clone(),
                keepalive_ms,
            },
        )
        .await
        {
            self.service.unregister_reader(reader_id.as_str()).await;
            return Self::build_http_json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Failed to initialize kevent stream" }),
            );
        }

        let service = self.service.clone();
        tokio::spawn(async move {
            loop {
                match service
                    .pull_event(reader_id.as_str(), Some(keepalive_ms))
                    .await
                {
                    Ok(Some(event)) => {
                        if !Self::send_stream_frame(&sender, &KEventStreamFrame::Event { event })
                            .await
                        {
                            break;
                        }
                    }
                    Ok(None) => {
                        if !Self::send_stream_frame(
                            &sender,
                            &KEventStreamFrame::Keepalive {
                                at_ms: current_time_ms(),
                            },
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = Self::send_stream_frame(
                            &sender,
                            &KEventStreamFrame::Error {
                                error: err.to_string(),
                            },
                        )
                        .await;
                        break;
                    }
                }
            }

            service.unregister_reader(reader_id.as_str()).await;
        });

        Self::build_stream_response(receiver)
    }

    fn boxed_http_body(bytes: Vec<u8>) -> BoxBody<Bytes, ServerError> {
        Full::new(Bytes::from(bytes))
            .map_err(|never: std::convert::Infallible| match never {})
            .boxed()
    }

    fn build_daemon_response(
        status: StatusCode,
        payload: KEventDaemonResponse,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let body = serde_json::to_vec(&payload).map_err(|error| {
            server_err!(
                ServerErrorCode::EncodeError,
                "Failed to serialize kevent response: {}",
                error
            )
        })?;
        http::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "application/json")
            .header(CACHE_CONTROL, "no-store")
            .body(Self::boxed_http_body(body))
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build kevent response: {}",
                    error
                )
            })
    }

    fn build_http_json_response(
        status: StatusCode,
        payload: Value,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let body = serde_json::to_vec(&payload).map_err(|error| {
            server_err!(
                ServerErrorCode::EncodeError,
                "Failed to serialize kevent JSON response: {}",
                error
            )
        })?;
        http::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "application/json")
            .header(CACHE_CONTROL, "no-store")
            .body(Self::boxed_http_body(body))
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build kevent JSON response: {}",
                    error
                )
            })
    }

    fn build_stream_response(
        receiver: mpsc::Receiver<std::result::Result<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let stream = stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        });
        let body = StreamBody::new(stream.map_ok(Frame::data));

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/x-ndjson")
            .header(CACHE_CONTROL, "no-store")
            .header("X-Accel-Buffering", "no")
            .body(BodyExt::map_err(body, |error| error).boxed())
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build kevent stream response: {}",
                    error
                )
            })
    }

    async fn send_stream_frame<T: Serialize>(
        sender: &mpsc::Sender<std::result::Result<Bytes, ServerError>>,
        payload: &T,
    ) -> bool {
        let mut bytes = match serde_json::to_vec(payload) {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = sender
                    .send(Err(server_err!(
                        ServerErrorCode::EncodeError,
                        "Failed to serialize kevent stream payload: {}",
                        error
                    )))
                    .await;
                return false;
            }
        };
        bytes.push(b'\n');
        sender.send(Ok(Bytes::from(bytes))).await.is_ok()
    }
}

#[async_trait]
impl HttpServer for KEventHttpServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        _info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() != Method::POST {
            return Err(server_err!(
                ServerErrorCode::BadRequest,
                "Method not allowed"
            ));
        }

        let path = req.uri().path().to_string();
        let collected = req.into_body().collect().await.map_err(|error| {
            server_err!(
                ServerErrorCode::BadRequest,
                "Failed to read kevent request body: {}",
                error
            )
        })?;
        let body = collected.to_bytes();
        self.handle_http_request(path.as_str(), body.as_ref()).await
    }

    fn id(&self) -> String {
        "kevent-http-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

enum KEventHttpRoute {
    Native,
    Stream,
    Publish,
}

fn normalize_http_path(path: &str) -> Option<KEventHttpRoute> {
    let normalized = if path != "/" {
        path.trim_end_matches('/')
    } else {
        path
    };
    match normalized {
        "/" | "/kapi/kevent" => Some(KEventHttpRoute::Native),
        "/stream" | "/kapi/kevent/stream" => Some(KEventHttpRoute::Stream),
        "/publish" | "/kapi/kevent/publish" => Some(KEventHttpRoute::Publish),
        _ => None,
    }
}

fn normalize_keepalive_ms(keepalive_ms: Option<u64>) -> u64 {
    keepalive_ms
        .unwrap_or(DEFAULT_HTTP_STREAM_KEEPALIVE_MS)
        .max(1)
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn status_from_kevent_error(error: &KEventError) -> StatusCode {
    match error {
        KEventError::InvalidEventId(_)
        | KEventError::InvalidPattern(_)
        | KEventError::TimerInvalidTarget(_)
        | KEventError::TimerNotFound(_)
        | KEventError::NotSupported(_) => StatusCode::BAD_REQUEST,
        KEventError::DaemonUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        KEventError::ReaderClosed(_) => StatusCode::GONE,
        KEventError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::time::{timeout, Duration};

    async fn response_json(
        response: http::Response<BoxBody<Bytes, ServerError>>,
    ) -> serde_json::Value {
        let collected = response.into_body().collect().await.unwrap();
        serde_json::from_slice(&collected.to_bytes()).unwrap()
    }

    async fn read_stream_line(body: &mut BoxBody<Bytes, ServerError>) -> serde_json::Value {
        let frame = timeout(Duration::from_millis(200), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let data = frame.into_data().unwrap();
        serde_json::from_slice(data.as_ref()).unwrap()
    }

    #[tokio::test]
    async fn test_native_http_endpoint_roundtrip() {
        let service = Arc::new(KEventService::new("node_a"));
        let server = KEventHttpServer::new(service.clone());

        let response = server
            .handle_http_request(
                "/kapi/kevent",
                serde_json::to_vec(&KEventDaemonRequest::RegisterReader {
                    reader_id: "r1".to_string(),
                    patterns: vec!["/system/**".to_string()],
                })
                .unwrap()
                .as_slice(),
            )
            .await
            .unwrap();
        let value = response_json(response).await;
        assert_eq!(value, json!({ "status": "ok" }));

        service
            .publish_local_global("/system/node/online", json!({"ok": true}))
            .await
            .unwrap();

        let response = server
            .handle_http_request(
                "/kapi/kevent",
                serde_json::to_vec(&KEventDaemonRequest::PullEvent {
                    reader_id: "r1".to_string(),
                    timeout_ms: Some(50),
                })
                .unwrap()
                .as_slice(),
            )
            .await
            .unwrap();
        let value = response_json(response).await;
        assert_eq!(value["status"], "ok");
        assert_eq!(value["event"]["eventid"], "/system/node/online");
    }

    #[tokio::test]
    async fn test_publish_http_endpoint() {
        let service = Arc::new(KEventService::new("node_a"));
        let server = KEventHttpServer::new(service.clone());
        service
            .register_reader("r1", vec!["/taskmgr/**".to_string()])
            .await
            .unwrap();

        let response = server
            .handle_http_request(
                "/kapi/kevent/publish",
                serde_json::to_vec(&json!({
                    "eventid": "/taskmgr/new/task_001",
                    "data": { "ok": true }
                }))
                .unwrap()
                .as_slice(),
            )
            .await
            .unwrap();
        let value = response_json(response).await;
        assert_eq!(value, json!({ "status": "ok" }));

        let event = service.pull_event("r1", Some(50)).await.unwrap().unwrap();
        assert_eq!(event.source_node, "node_a");
        assert_eq!(event.ingress_node.as_deref(), Some("node_a"));
    }

    #[tokio::test]
    async fn test_stream_http_endpoint() {
        let service = Arc::new(KEventService::new("node_a"));
        let server = KEventHttpServer::new(service.clone());

        let response = server
            .handle_http_request(
                "/kapi/kevent/stream",
                serde_json::to_vec(&json!({
                    "patterns": ["/system/**"],
                    "keepalive_ms": 5
                }))
                .unwrap()
                .as_slice(),
            )
            .await
            .unwrap();

        let mut body = response.into_body();
        let ack = read_stream_line(&mut body).await;
        assert_eq!(ack["type"], "ack");

        service
            .publish_local_global("/system/node/online", json!({"ok": true}))
            .await
            .unwrap();

        let event = read_stream_line(&mut body).await;
        assert_eq!(event["type"], "event");
        assert_eq!(event["event"]["eventid"], "/system/node/online");
    }
}
