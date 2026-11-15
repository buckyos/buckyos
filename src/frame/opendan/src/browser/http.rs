//! The browser module's HTTP entries beside kRPC, below every mount of the
//! service: the screen (`GET …/browser/screen`, upgraded to a WebSocket) and
//! the upload of a local file for a page's file chooser
//! (`PUT …/browser/upload/<request_id>`).

use std::net::IpAddr;
use std::time::Duration;

use buckyos_http_server::{ServerError, ServerResult, StreamInfo};
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use serde_json::json;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::WebSocketStream;

use super::rpc::BrowserError;
use super::{screen, transfer};
use crate::service::StateService;

type Request = http::Request<BoxBody<Bytes, ServerError>>;
type Response = ServerResult<http::Response<BoxBody<Bytes, ServerError>>>;

const UPGRADE_WAIT: Duration = Duration::from_secs(5);
/// What a client sends over the screen is small.
const MAX_SCREEN_MESSAGE: usize = 1024 * 1024;

/// The method names the two entries are authorized as (`StateService::caller`):
/// both are the owner's.
pub const SCREEN_METHOD: &str = "browser.screen";
pub const UPLOAD_METHOD: &str = "browser.upload";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserRoute {
    Screen,
    /// `request_id` of the file chooser the upload answers.
    Upload(String),
}

/// The entry `method path` names, if it is one of the module's. `mounts`
/// are the paths the service is mounted at.
pub fn route(method: &http::Method, path: &str, mounts: &[&str]) -> Option<BrowserRoute> {
    let rest = mounts.iter().find_map(|mount| path.strip_prefix(mount)?.strip_prefix("/browser/"))?;
    match (method, rest) {
        (&http::Method::GET, "screen") => Some(BrowserRoute::Screen),
        (&http::Method::PUT, upload) => {
            let request_id = upload.strip_prefix("upload/")?;
            (!request_id.is_empty() && !request_id.contains('/')).then(|| BrowserRoute::Upload(request_id.to_string()))
        }
        _ => None,
    }
}

/// The caller's address, as `StateService::caller` takes it.
pub fn client_ip(info: &StreamInfo) -> Option<IpAddr> {
    info.src_addr.as_ref()?.parse::<std::net::SocketAddr>().ok().map(|a| a.ip())
}

pub fn json_response(status: http::StatusCode, body: serde_json::Value) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
    http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from(body.to_string())).map_err(|e| match e {}).boxed())
        .map_err(|e| buckyos_http_server::server_err!(buckyos_http_server::ServerErrorCode::InvalidData, "{e}"))
}

/// Serve one of the module's HTTP entries.
pub async fn serve(service: &StateService, route: BrowserRoute, req: Request, info: StreamInfo) -> Response {
    // A caller whose address is not known is not on the loopback interface.
    let ip = client_ip(&info).unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    match route {
        BrowserRoute::Screen => screen(service, req, ip),
        BrowserRoute::Upload(request_id) => upload(service, &request_id, req, ip).await,
    }
}

fn failed(e: BrowserError) -> Response {
    let status = match e.code {
        "upload_too_large" => http::StatusCode::PAYLOAD_TOO_LARGE,
        "invalid_argument" => http::StatusCode::BAD_REQUEST,
        "browser_unavailable" => http::StatusCode::SERVICE_UNAVAILABLE,
        code if code.starts_with("no_such_") => http::StatusCode::NOT_FOUND,
        _ => http::StatusCode::INTERNAL_SERVER_ERROR,
    };
    json_response(status, json!({ "code": e.code, "message": e.message }))
}

/// The screen: the request becomes a WebSocket, whose first message says
/// who looks (`screen::serve`).
fn screen(service: &StateService, mut req: Request, ip: IpAddr) -> Response {
    let invalid = || buckyos_http_server::server_err!(buckyos_http_server::ServerErrorCode::BadRequest, "invalid websocket request");
    let mut head = http::Request::builder().method(req.method()).uri(req.uri()).version(req.version()).body(()).map_err(|_| invalid())?;
    *head.headers_mut() = req.headers().clone();
    let Ok(response) = tokio_tungstenite::tungstenite::handshake::server::create_response(&head) else {
        return failed(BrowserError::invalid("the screen is a WebSocket; this request does not ask for one"));
    };
    let upgrade = hyper::upgrade::on(&mut req);
    let (browser, access) = (service.browser.clone(), service.access.clone());
    tokio::spawn(async move {
        let Ok(Ok(upgraded)) = tokio::time::timeout(UPGRADE_WAIT, upgrade).await else { return };
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(MAX_SCREEN_MESSAGE);
        config.max_frame_size = Some(MAX_SCREEN_MESSAGE);
        let socket = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(config)).await;
        let authorize: screen::Authorize =
            Box::new(move |token| Box::pin(async move { access.caller(token.as_deref(), SCREEN_METHOD, ip).await.map_err(|e| e.to_string()) }));
        screen::serve(socket, browser, authorize).await;
    });
    let (parts, _) = response.into_parts();
    Ok(http::Response::from_parts(parts, Full::new(Bytes::new()).map_err(|e| match e {}).boxed()))
}

/// A file of the App's device for a file chooser request (protocol §4).
async fn upload(service: &StateService, request_id: &str, req: Request, ip: IpAddr) -> Response {
    let token = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string);
    if let Err(e) = service.caller(token.as_deref(), UPLOAD_METHOD, ip).await {
        let status = if token.is_some() { http::StatusCode::FORBIDDEN } else { http::StatusCode::UNAUTHORIZED };
        return json_response(status, json!({ "code": "unauthorized", "message": e.to_string() }));
    }
    if let Err(e) = service.browser.usable() {
        return failed(e);
    }
    let (mut name, mut done) = (None, false);
    for (key, value) in url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "name" => name = Some(value.into_owned()),
            "done" => done = matches!(value.as_ref(), "1" | "true"),
            _ => {}
        }
    }
    let Some(name) = name.filter(|name| !name.trim().is_empty()) else {
        return failed(BrowserError::invalid("the upload names no file (`?name=`)"));
    };
    let limit = service.browser.tuning.upload_limit;
    let declared = req.headers().get(http::header::CONTENT_LENGTH).and_then(|value| value.to_str().ok()?.parse::<u64>().ok());
    if declared.is_some_and(|length| length > limit) {
        return failed(transfer::too_large(limit));
    }
    let request_id = percent_encoding::percent_decode_str(request_id).decode_utf8_lossy();
    match transfer::receive(&service.browser, &request_id, &name, done, req.into_body()).await {
        Ok(()) => json_response(http::StatusCode::OK, json!({})),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_and_the_upload_are_found_below_every_mount() {
        let mounts = ["/kapi/opendan", "/api/opendan", "/kapi/jarvis.alice"];
        let (get, put, post) = (http::Method::GET, http::Method::PUT, http::Method::POST);
        for mount in mounts {
            assert_eq!(route(&get, &format!("{mount}/browser/screen"), &mounts), Some(BrowserRoute::Screen));
            assert_eq!(route(&put, &format!("{mount}/browser/upload/r12"), &mounts), Some(BrowserRoute::Upload("r12".into())));
            assert_eq!(route(&get, mount, &mounts), None, "the page's metadata");
            assert_eq!(route(&post, &format!("{mount}/browser/screen"), &mounts), None);
            assert_eq!(route(&get, &format!("{mount}/browser/upload/r12"), &mounts), None);
            assert_eq!(route(&put, &format!("{mount}/browser/upload/"), &mounts), None);
            assert_eq!(route(&put, &format!("{mount}/browser/upload/a/b"), &mounts), None);
            assert_eq!(route(&get, &format!("{mount}/obj/browser/screen"), &mounts), None);
        }
        assert_eq!(route(&get, "/kapi/other/browser/screen", &mounts), None);
        let info = StreamInfo::new("127.0.0.1:5000".to_string());
        assert!(client_ip(&info).unwrap().is_loopback());
    }
}
