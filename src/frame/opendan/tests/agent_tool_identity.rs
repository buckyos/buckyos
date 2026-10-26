use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use buckyos_api::{ChildControlPolicy, Task, TaskControlProfile, ZoneConfig};
use libopendan::session_context::{SessionContext, CONTEXT_DIR, CONTEXT_FILE, TOKEN_FILE};
use name_lib::{ZoneDocument, DID};
use serde_json::{json, Value};

const AGENT: &str = "jarvis.desktop-lite.invalid";
const OWNER: &str = "desktop-owner";
const SID: &str = "ui-desktop-lite";

#[derive(Clone, Copy)]
enum Identity {
    Valid,
    MissingToken,
    WrongApp,
}

struct Gateway {
    address: String,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    thread: Option<JoinHandle<()>>,
}

impl Gateway {
    fn start(identity: Identity, token: String, session_token: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = stop.clone();
        let recorded = requests.clone();
        let zone = ZoneDocument::new(
            DID::new("web", "desktop-lite.invalid"),
            DID::new("web", "desktop-owner.invalid"),
            serde_json::from_value(json!({
                "kty":"OKP", "crv":"Ed25519", "x":"qJdNEtscIYwTo-I0K7iPEt_UZdBDRd4r16jdBfNR0tM"
            }))
            .unwrap(),
        );
        let zone =
            serde_json::to_string(&ZoneConfig::from_zone_document(&zone, None).unwrap()).unwrap();
        let task = json!({
            "task_id":"task-desktop-lite", "name":"Agent task", "root_id":"task-desktop-lite",
            "child_control_policy":ChildControlPolicy::default(),
            "schema_id":"desktop-lite.test", "schema_version":1, "input":{}, "input_digest":"test",
            "creator":{"user_id":OWNER,"app_id":format!("app:{AGENT}@{OWNER}")},
            "storage_domain":"User", "idempotency_key":"desktop-lite-test",
            "executor":{"kind":"App","app_id":AGENT,"app_instance_id":format!("{AGENT}@{OWNER}")},
            "runner_epoch":1, "phase":"Running", "control_profile":TaskControlProfile::baseline(1),
            "policy_preset":"default", "permission_boundary":false, "revision":1,
            "created_at":1, "updated_at":1
        });
        serde_json::from_value::<Task>(task.clone()).unwrap();
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(25);
            while !stopped.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("accept mock gateway: {error}"),
                };
                let (mut stream, line, request) = read_request(stream);
                let method = request["method"].as_str().unwrap();
                recorded
                    .lock()
                    .unwrap()
                    .push((line.clone(), request.clone()));
                let path = match method {
                    "session.service_token" => format!("/kapi/{AGENT}"),
                    "sys_config_get" => "/kapi/system_config".into(),
                    "list_scheduled_tasks" => "/kapi/workflow".into(),
                    "get_task" | "request_control" => "/kapi/task-manager".into(),
                    _ => panic!("unexpected RPC method {method}"),
                };
                assert_eq!(
                    line.trim(),
                    format!("POST http://127.0.0.1:3180{path} HTTP/1.1")
                );
                let result = match method {
                    "session.service_token" => {
                        assert_eq!(request["sys"][1], session_token);
                        assert_eq!(request["params"], json!({"sid":SID}));
                        json!({
                            "token":if matches!(identity, Identity::MissingToken) { "" } else { &token },
                            "app_id":if matches!(identity, Identity::WrongApp) { "other.desktop-lite.invalid" } else { AGENT },
                            "owner_user_id":OWNER
                        })
                    }
                    _ => {
                        assert!(
                            matches!(identity, Identity::Valid),
                            "invalid identity reached {method}"
                        );
                        assert_eq!(
                            request["sys"][1], token,
                            "{method} did not use the Agent token"
                        );
                        match method {
                            "sys_config_get" => {
                                assert_eq!(request["params"], json!({"key":"boot/config"}));
                                json!({"value":zone,"version":1})
                            }
                            "list_scheduled_tasks" => {
                                assert_eq!(
                                    request["params"]["owner"],
                                    json!({"user_id":OWNER,"app_id":AGENT})
                                );
                                json!({"ok":true,"schedules":[]})
                            }
                            "get_task" => {
                                assert_eq!(
                                    request["params"],
                                    json!({"task_id":"task-desktop-lite"})
                                );
                                json!({"task":task})
                            }
                            "request_control" => {
                                assert_eq!(request["params"]["task_id"], "task-desktop-lite");
                                assert_eq!(request["params"]["action"], "Cancel");
                                assert_eq!(request["params"]["recursive"], true);
                                assert!(request["params"]["request_id"]
                                    .as_str()
                                    .unwrap()
                                    .starts_with("ctl-"));
                                json!({"kind":"Batch","result":{"requested":["task-desktop-lite"],"skipped_by_policy":[],"denied":[],"already_terminal":[]}})
                            }
                            _ => unreachable!(),
                        }
                    }
                };
                let response = json!({"sys":[request["sys"][0]],"result":result}).to_string();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
            }
        });
        Self {
            address,
            stop,
            requests,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> Vec<(String, Value)> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
        let requests = self.requests.lock().unwrap().clone();
        requests
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(stream: TcpStream) -> (TcpStream, String, Value) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut length = None;
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        assert!(line.len() < 65536);
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
    }
    let length = length.unwrap();
    assert!(length < 65536);
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).unwrap();
    (
        reader.into_inner(),
        first,
        serde_json::from_slice(&bytes).unwrap(),
    )
}

fn app_token(expires: u64) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","typ":"JWT","kid":"verify-hub"}"#);
    let claims = URL_SAFE_NO_PAD.encode(json!({
        "iss":"verify-hub", "sub":OWNER, "appid":AGENT, "exp":expires, "jti":"desktop-lite-test",
        "principal_kind":"agent", "token_use":"session", "target_kind":"app",
        "app_instance_id":format!("{AGENT}@{OWNER}"), "app_owner_user_id":OWNER
    }).to_string());
    let token = format!("{header}.{claims}.{}", URL_SAFE_NO_PAD.encode([0; 64]));
    let parsed = kRPC::RPCSessionToken::from_string(&token).unwrap();
    buckyos_api::validate_verify_hub_token_claims(&parsed, buckyos_api::TokenUse::Session).unwrap();
    token
}

fn link_tool(path: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_opendan"), path).unwrap();
    #[cfg(not(unix))]
    std::fs::hard_link(env!("CARGO_BIN_EXE_opendan"), path)
        .or_else(|_| std::fs::copy(env!("CARGO_BIN_EXE_opendan"), path).map(|_| ()))
        .unwrap();
}

fn run_tool(tool: &str, alias: bool, identity: Identity) -> (Output, Vec<(String, Value)>) {
    let root = tempfile::tempdir().unwrap();
    let workdir = root.path().join("session");
    std::fs::create_dir_all(workdir.join(CONTEXT_DIR)).unwrap();
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 86400;
    let session_token = format!("odsess1.{SID}.{}.test-signature", expires * 1000);
    let gateway = Gateway::start(identity, app_token(expires), session_token.clone());
    let context = SessionContext {
        agent_did: format!("did:web:{AGENT}"),
        agent_id: AGENT.into(),
        session_id: SID.into(),
        trace_id: SID.into(),
        runtime_id: "desktop-lite".into(),
        workdir: workdir.display().to_string(),
        state_url: format!("http://127.0.0.1:3180/kapi/{AGENT}"),
        token_file: TOKEN_FILE.into(),
        expires_at_ms: expires * 1000,
        workspace: None,
    };
    std::fs::write(
        workdir.join(CONTEXT_DIR).join(CONTEXT_FILE),
        serde_json::to_vec(&context).unwrap(),
    )
    .unwrap();
    std::fs::write(workdir.join(CONTEXT_DIR).join(TOKEN_FILE), session_token).unwrap();
    let name = if alias { tool } else { "agent_tool" };
    let executable = root
        .path()
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    link_tool(&executable);
    let mut command = Command::new(executable);
    command
        .current_dir(&workdir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root.path())
        .env("BUCKYOS_ROOT", root.path())
        .env("HTTP_PROXY", &gateway.address)
        .env("HTTPS_PROXY", &gateway.address)
        .env("ALL_PROXY", &gateway.address)
        .env("NO_PROXY", "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !matches!(identity, Identity::MissingToken) {
        command
            .env("OPENDAN_WORKFLOW_URL", "http://127.0.0.1:1/kapi/workflow")
            .env(
                "OPENDAN_TASK_MANAGER_URL",
                "http://127.0.0.1:1/kapi/task-manager",
            )
            .env("OPENDAN_SESSION_TOKEN", "wrong-legacy-token");
    }
    if !alias {
        command.arg(tool);
    }
    match tool {
        "dcrontab" => {
            command.args(["list", "--agent", AGENT]);
        }
        "check_task" => {
            command.arg("task-desktop-lite");
        }
        "cancel_task" => {
            command.args(["task-desktop-lite", "--recursive"]);
        }
        _ => unreachable!(),
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "{tool} timed out: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    (child.wait_with_output().unwrap(), gateway.finish())
}

#[test]
fn desktop_lite_service_tools_use_agent_identity_without_container_environment() {
    for tool in ["dcrontab", "check_task", "cancel_task"] {
        for alias in [false, true] {
            let (output, requests) = run_tool(tool, alias, Identity::Valid);
            assert!(
                output.status.success(),
                "{tool} alias={alias}: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_ne!(result["status"], "error", "{result}");
            let methods: Vec<&str> = requests
                .iter()
                .map(|(_, rpc)| rpc["method"].as_str().unwrap())
                .collect();
            let business = match tool {
                "dcrontab" => vec!["list_scheduled_tasks"],
                "check_task" => vec!["get_task"],
                "cancel_task" => vec!["request_control", "get_task"],
                _ => unreachable!(),
            };
            let expected: Vec<&str> =
                [vec!["session.service_token", "sys_config_get"], business].concat();
            assert_eq!(methods, expected, "{tool} alias={alias}");
        }
    }
}

#[test]
fn desktop_lite_service_tools_reject_missing_or_conflicting_agent_identity() {
    for identity in [Identity::MissingToken, Identity::WrongApp] {
        for tool in ["dcrontab", "check_task", "cancel_task"] {
            let (output, requests) = run_tool(tool, false, identity);
            assert!(
                !output.status.success(),
                "{tool} accepted invalid Agent identity"
            );
            let methods: Vec<&str> = requests
                .iter()
                .map(|(_, rpc)| rpc["method"].as_str().unwrap())
                .collect();
            assert_eq!(methods, ["session.service_token"], "{tool}");
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["status"], "error", "{result}");
            let expected = match identity {
                Identity::MissingToken => "missing required $BUCKYOS_APPCLIENT_SESSION_TOKEN",
                Identity::WrongApp => "session token appid",
                Identity::Valid => unreachable!(),
            };
            assert!(
                result["summary"].as_str().unwrap().contains(expected),
                "{result}"
            );
        }
    }
}
