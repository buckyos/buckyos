use super::*;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Duration, Instant};

#[derive(Clone)]
enum Reply {
    Success(Value),
    Error(&'static str),
    HttpForbidden,
    Disconnect,
    Hang,
}

struct RpcFixture {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl RpcFixture {
    async fn start(delay: Duration, replies: Vec<Reply>, fallback: Reply) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = socket.local_addr().unwrap();
        let mut socket = Some(socket);
        let listener = if delay.is_zero() {
            Some(socket.take().unwrap().listen(16).unwrap())
        } else {
            None
        };
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded_requests = requests.clone();
        let task = tokio::spawn(async move {
            let listener = match listener {
                Some(listener) => listener,
                None => {
                    tokio::time::sleep(delay).await;
                    socket.take().unwrap().listen(16).unwrap()
                }
            };
            let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
            let mut handlers = JoinSet::new();
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let requests = recorded_requests.clone();
                let replies = replies.clone();
                let fallback = fallback.clone();
                handlers.spawn(async move {
                    let request = read_request(&mut stream).await;
                    requests.lock().unwrap().push(request.clone());
                    let reply = replies.lock().unwrap().pop_front().unwrap_or(fallback);
                    let body = match reply {
                        Reply::Success(value) => json!({"result": value, "sys": [request["sys"][0]]}),
                        Reply::Error(error) => json!({"error": error, "sys": [request["sys"][0]]}),
                        Reply::HttpForbidden => {
                            let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                            return;
                        }
                        Reply::Disconnect => return,
                        Reply::Hang => {
                            std::future::pending::<()>().await;
                            return;
                        }
                    }
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        Self {
            url: format!("http://{address}/kapi/system_config"),
            requests,
            task,
        }
    }

    fn client(&self) -> SystemConfigClient {
        SystemConfigClient::new(Some(&self.url), Some("test-boot-token"))
    }

    fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for RpcFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(stream: &mut TcpStream) -> Value {
    let mut received = Vec::new();
    loop {
        let mut buffer = [0u8; 4096];
        let count = stream.read(&mut buffer).await.unwrap();
        assert_ne!(count, 0, "connection closed before request completed");
        received.extend_from_slice(&buffer[..count]);
        let Some(header_end) = received.windows(4).position(|value| value == b"\r\n\r\n") else {
            continue;
        };
        let headers = std::str::from_utf8(&received[..header_end]).unwrap();
        let content_length = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .unwrap()
            .1
            .trim()
            .parse::<usize>()
            .unwrap();
        let body_start = header_end + 4;
        if received.len() >= body_start + content_length {
            return serde_json::from_slice(&received[body_start..body_start + content_length])
                .unwrap();
        }
    }
}

#[tokio::test]
async fn system_config_boot_waits_for_delayed_listener() {
    let mut cases = JoinSet::new();
    for seconds in [0, 3, 10, 30] {
        cases.spawn(async move {
            let delay = Duration::from_secs(seconds);
            let fixture = RpcFixture::start(
                delay,
                Vec::new(),
                Reply::Success(json!({"value": "{}", "version": 1})),
            )
            .await;
            let started = Instant::now();
            wait_system_config_ready(
                &fixture.client(),
                Duration::from_secs(45),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert!(started.elapsed() >= delay);
            let requests = fixture.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0]["method"], "sys_config_get");
            assert_eq!(requests[0]["params"]["key"], "boot/config");
            assert_eq!(requests[0]["sys"][1], "test-boot-token");
        });
    }
    while let Some(result) = cases.join_next().await {
        result.unwrap();
    }
}

#[tokio::test]
async fn system_config_boot_accepts_missing_boot_config_on_first_activation() {
    let fixture = RpcFixture::start(Duration::ZERO, Vec::new(), Reply::Success(Value::Null)).await;
    let client = fixture.client();
    wait_system_config_ready(&client, Duration::from_secs(1), Duration::from_millis(200))
        .await
        .unwrap();
    retry_system_config_boot(
        "register device doc",
        Duration::from_secs(1),
        Duration::from_millis(200),
        || client.set("devices/ood1/doc", "test-device-doc"),
    )
    .await
    .unwrap();
    let requests = fixture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["method"], "sys_config_get");
    assert_eq!(requests[1]["method"], "sys_config_set");
    assert_eq!(requests[1]["params"]["key"], "devices/ood1/doc");
}

#[tokio::test]
async fn system_config_boot_times_out_when_listener_never_starts() {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = socket.local_addr().unwrap();
    let client = SystemConfigClient::new(
        Some(&format!("http://{address}/kapi/system_config")),
        Some("test-boot-token"),
    );
    let budget = Duration::from_millis(350);
    let started = Instant::now();
    let error = wait_system_config_ready(&client, budget, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(started.elapsed() >= budget);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(error.contains("system-config"), "{error}");
    assert!(
        error.contains("timed out") || error.contains("timeout"),
        "{error}"
    );
}

#[tokio::test]
async fn system_config_boot_bounds_a_hanging_rpc_by_total_deadline() {
    let fixture = RpcFixture::start(Duration::ZERO, Vec::new(), Reply::Hang).await;
    let budget = Duration::from_millis(350);
    let started = Instant::now();
    let error = wait_system_config_ready(&fixture.client(), budget, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(started.elapsed() >= budget);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        error.contains("timed out") || error.contains("timeout"),
        "{error}"
    );
    assert_eq!(fixture.requests().len(), 1);
}

#[tokio::test]
async fn system_config_boot_retries_a_request_timeout_before_deadline() {
    let fixture = RpcFixture::start(
        Duration::ZERO,
        vec![Reply::Hang],
        Reply::Success(Value::Null),
    )
    .await;
    wait_system_config_ready(
        &fixture.client(),
        Duration::from_secs(2),
        Duration::from_millis(100),
    )
    .await
    .unwrap();
    assert_eq!(fixture.requests().len(), 2);
}

#[tokio::test]
async fn system_config_boot_fails_fast_on_authentication_and_permission_errors() {
    for reason in ["Invalid token: bad signature", "No permission: forbidden"] {
        let fixture = RpcFixture::start(Duration::ZERO, Vec::new(), Reply::Error(reason)).await;
        let error = wait_system_config_ready(
            &fixture.client(),
            Duration::from_secs(2),
            Duration::from_millis(500),
        )
        .await
        .unwrap_err();
        assert!(error.contains(reason), "{error}");
        assert_eq!(fixture.requests().len(), 1);
    }
    let fixture = RpcFixture::start(Duration::ZERO, Vec::new(), Reply::HttpForbidden).await;
    let error = wait_system_config_ready(
        &fixture.client(),
        Duration::from_secs(2),
        Duration::from_millis(500),
    )
    .await
    .unwrap_err();
    assert!(error.contains("403"), "{error}");
    assert_eq!(fixture.requests().len(), 1);
}

#[tokio::test]
async fn system_config_boot_retries_registration_after_ready_connection_drops() {
    let fixture = RpcFixture::start(
        Duration::ZERO,
        vec![Reply::Success(Value::Null), Reply::Disconnect],
        Reply::Success(Value::Null),
    )
    .await;
    let client = fixture.client();
    wait_system_config_ready(&client, Duration::from_secs(1), Duration::from_millis(200))
        .await
        .unwrap();
    retry_system_config_boot(
        "register device doc",
        Duration::from_secs(2),
        Duration::from_millis(200),
        || client.set("devices/ood1/doc", "test-device-doc"),
    )
    .await
    .unwrap();
    let requests = fixture.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["method"], "sys_config_get");
    assert_eq!(requests[1]["method"], "sys_config_set");
    assert_eq!(requests[1]["params"], requests[2]["params"]);
}

#[tokio::test]
async fn system_config_boot_does_not_retry_registration_permission_denied() {
    let fixture = RpcFixture::start(
        Duration::ZERO,
        vec![Reply::Success(Value::Null)],
        Reply::Error("No permission: write denied"),
    )
    .await;
    let client = fixture.client();
    wait_system_config_ready(&client, Duration::from_secs(1), Duration::from_millis(200))
        .await
        .unwrap();
    let error = retry_system_config_boot(
        "register device doc",
        Duration::from_secs(2),
        Duration::from_millis(200),
        || client.set("devices/ood1/doc", "test-device-doc"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("write denied"), "{error}");
    assert_eq!(fixture.requests().len(), 2);
}
