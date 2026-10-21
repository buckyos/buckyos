//! DV probe for a running zone (run as root on the OOD).
//!
//! dv_probe token <owner> <app_id>
//!     mint an App session token the way node_daemon does for app containers
//! dv_probe exec <owner> <app_id> <profile> <target> <cwd> <cmd>
//!     log in and run one command; <target> is `127.0.0.1:3184` (direct) or
//!     `socks:<gateway>:3185@<node-host>` (through the node gateway entry)
//! dv_probe login <token> <profile> <target>
//!     log in with a given token and print the reply

use std::collections::BTreeMap;
use std::time::Duration;

use buckyos_api::*;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn app_token(owner: &str, app_id: &str) -> String {
    let identity: serde_json::Value =
        serde_json::from_slice(&std::fs::read("/opt/buckyos/etc/node_identity.json").unwrap()).unwrap();
    let device_did = name_lib::DID::from_str(identity["device_did"].as_str().unwrap()).unwrap();
    let device_name = identity["device_name"].as_str().unwrap();
    let key = load_local_device_private_key(&device_did).unwrap();
    let (assertion, _) = generate_service_login_assertion(owner, app_id, device_name, &key).unwrap();
    let client = VerifyHubClient::new(kRPC::kRPC::new("http://127.0.0.1:3180/kapi/verify-hub", None));
    let target = AuthTarget::App {
        app_instance_id: format!("{app_id}@{owner}").parse().unwrap(),
    };
    client.login_by_jwt(&assertion, target).await.unwrap().session_token
}

async fn connect(target: &str) -> Box<dyn Io> {
    if let Some(rest) = target.strip_prefix("socks:") {
        let (gateway, host) = rest.split_once('@').unwrap();
        let mut s = tokio::net::TcpStream::connect(gateway).await.unwrap();
        s.write_all(&[5, 1, 0]).await.unwrap();
        let mut b = [0u8; 2];
        s.read_exact(&mut b).await.unwrap();
        assert_eq!(b, [5, 0], "socks greeting");
        let mut req = vec![5, 1, 0];
        match host.parse::<std::net::Ipv4Addr>() {
            Ok(ip) => {
                req.push(1);
                req.extend_from_slice(&ip.octets());
            }
            Err(_) => {
                req.push(3);
                req.push(host.len() as u8);
                req.extend_from_slice(host.as_bytes());
            }
        }
        req.extend_from_slice(&NODE_EXECUTOR_SERVICE_PORT.to_be_bytes());
        s.write_all(&req).await.unwrap();
        let mut head = [0u8; 4];
        s.read_exact(&mut head).await.unwrap();
        assert_eq!(head[1], 0, "socks connect reply {}", head[1]);
        let skip = match head[3] {
            1 => 4,
            4 => 16,
            _ => {
                let mut n = [0u8; 1];
                s.read_exact(&mut n).await.unwrap();
                n[0] as usize
            }
        };
        let mut rest = vec![0u8; skip + 2];
        s.read_exact(&mut rest).await.unwrap();
        Box::new(s)
    } else {
        Box::new(tokio::net::TcpStream::connect(target).await.unwrap())
    }
}

async fn recv(io: &mut Box<dyn Io>) -> Option<ExecutorServerMessage> {
    loop {
        match tokio::time::timeout(Duration::from_secs(40), read_executor_frame(io)).await {
            Ok(Ok(Some(ExecutorFrame::Control(p)))) => return serde_json::from_slice(&p).ok(),
            Ok(Ok(Some(ExecutorFrame::Data(_)))) => continue,
            other => {
                eprintln!("stream ended: {other:?}");
                return None;
            }
        }
    }
}

async fn login(io: &mut Box<dyn Io>, token: String, profile: &str) -> Option<ExecutorLoginOk> {
    write_executor_control(
        io,
        &ExecutorClientMessage::Login(ExecutorLogin {
            v: NODE_EXECUTOR_PROTOCOL_VERSION,
            request_id: Some("login".into()),
            token,
            profile: profile.into(),
            client: ExecutorClientInfo {
                runtime_id: "dv-probe".into(),
                run_id: format!("dv-{}", std::process::id()),
            },
        }),
    )
    .await
    .unwrap();
    let reply = recv(io).await;
    println!("login: {}", serde_json::to_string(&reply).unwrap());
    match reply {
        Some(ExecutorServerMessage::LoginOk(ok)) => Some(ok),
        _ => None,
    }
}

async fn reply_for(io: &mut Box<dyn Io>, rid: &str) -> Option<ExecutorServerMessage> {
    loop {
        let msg = recv(io).await?;
        let id = match &msg {
            ExecutorServerMessage::Accepted(m) | ExecutorServerMessage::Status(m) => Some(m.request_id.clone()),
            ExecutorServerMessage::QueryOk(m) => Some(m.request_id.clone()),
            ExecutorServerMessage::Error(m) => m.request_id.clone(),
            ExecutorServerMessage::DescribeOk(m) => Some(m.request_id.clone()),
            _ => None,
        };
        if id.as_deref() == Some(rid) {
            return Some(msg);
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("token") => println!("{}", app_token(&args[2], &args[3]).await),
        Some("login") => {
            let mut io = connect(&args[4]).await;
            login(&mut io, args[2].clone(), &args[3]).await;
        }
        Some("exec") => {
            let (owner, app_id, profile, target, cwd, cmd) =
                (&args[2], &args[3], &args[4], &args[5], &args[6], &args[7]);
            let token = app_token(owner, app_id).await;
            let mut io = connect(target).await;
            let Some(ok) = login(&mut io, token, profile).await else { std::process::exit(2) };
            write_executor_control(
                &mut io,
                &ExecutorClientMessage::Describe(ExecutorDescribeRequest {
                    request_id: "d".into(),
                    workdir: Some(cwd.clone()),
                }),
            )
            .await
            .unwrap();
            println!("describe: {}", serde_json::to_string(&reply_for(&mut io, "d").await).unwrap());
            let op_id = format!("op-{}", std::process::id());
            write_executor_control(
                &mut io,
                &ExecutorClientMessage::Exec(ExecutorExecRequest {
                    request_id: "e".into(),
                    store_id: ok.store_id.clone(),
                    op_id: op_id.clone(),
                    run_id: format!("dv-{}", std::process::id()),
                    call_id: "c1".into(),
                    cmd: cmd.clone(),
                    cwd: cwd.clone(),
                    env_overrides: BTreeMap::new(),
                    execution_timeout_ms: Some(60_000),
                }),
            )
            .await
            .unwrap();
            println!("exec: {}", serde_json::to_string(&reply_for(&mut io, "e").await).unwrap());
            for i in 0..10 {
                let rid = format!("w{i}");
                write_executor_control(
                    &mut io,
                    &ExecutorClientMessage::Query(ExecutorQueryRequest {
                        request_id: rid.clone(),
                        store_id: ok.store_id.clone(),
                        op_id: op_id.clone(),
                        channel: None,
                        offset: 0,
                        max_bytes: None,
                        wait_ms: Some(20_000),
                    }),
                )
                .await
                .unwrap();
                if let Some(ExecutorServerMessage::QueryOk(q)) = reply_for(&mut io, &rid).await {
                    if q.op.state.is_terminal() {
                        break;
                    }
                }
            }
            write_executor_control(
                &mut io,
                &ExecutorClientMessage::Query(ExecutorQueryRequest {
                    request_id: "q".into(),
                    store_id: ok.store_id,
                    op_id,
                    channel: Some(ExecutorChannel::Stdout),
                    offset: 0,
                    max_bytes: None,
                    wait_ms: None,
                }),
            )
            .await
            .unwrap();
            if let Some(ExecutorServerMessage::QueryOk(q)) = reply_for(&mut io, "q").await {
                use base64::Engine;
                let out = q.chunk.map(|c| base64::engine::general_purpose::STANDARD.decode(c.data_b64).unwrap()).unwrap_or_default();
                println!("state: {:?} exit: {:?}", q.op.state, q.op.result.as_ref().and_then(|r| r.exit_code));
                println!("stdout:\n{}", String::from_utf8_lossy(&out));
            }
        }
        _ => eprintln!("usage: dv_probe token|login|exec ..."),
    }
}
