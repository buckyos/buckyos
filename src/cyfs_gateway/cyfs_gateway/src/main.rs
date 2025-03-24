#![allow(dead_code)]
#![allow(unused_imports)]
//mod config;
//mod gateway;
//mod interface;
mod config_loader;
mod dispatcher;
mod gateway;
mod socks;

//mod peer;
//mod proxy;
//mod service;
//mod storage;
//mod tunnel;

#[macro_use]
extern crate log;

use crate::gateway::{Gateway, GatewayParams};
use buckyos_kit::*;
use clap::{Arg, ArgAction, Command};
use console_subscriber::{self, Server};
use cyfs_dns::start_cyfs_dns_server;
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use log::*;
use name_client::*;
use name_lib::*;
use std::path::PathBuf;
use tokio::task;
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn service_main(config: &str, matches: &clap::ArgMatches) -> Result<()> {
    // Parse config in json format
    let json = serde_json::from_str(config).map_err(|e| {
        let msg = format!("Error parsing config: {} {}", e, config);
        error!("{}", msg);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    // Load config from json
    let load_result = config_loader::GatewayConfig::load_from_json_value(json).await;
    if load_result.is_err() {
        let msg = format!("Error loading config: {}", load_result.err().unwrap());
        error!("{}", msg);
        std::process::exit(1);
    }
    let config_loader = load_result.unwrap();

    // Extract necessary params from command line
    let params = GatewayParams {
        keep_tunnel: matches
            .get_many::<String>("keep_tunnel")
            .unwrap_or_default()
            .map(|s| s.to_string())
            .collect(),
    };

    let gateway = Gateway::new(config_loader);
    gateway.start(params).await;

    // Sleep forever
    let _ = tokio::signal::ctrl_c().await;

    Ok(())
}

// Parse config first, then config file if supplied by user
fn load_config_from_args(matches: &clap::ArgMatches) -> Result<String> {
    let default_config = get_buckyos_system_etc_dir().join("cyfs_gateway.json");
    let config_file = matches.get_one::<String>("config_file");
    let real_config_file;
    if config_file.is_none() {
        real_config_file = default_config;
    } else {
        real_config_file = PathBuf::from(config_file.unwrap());
    }
    std::fs::read_to_string(real_config_file.clone()).map_err(|e| {
        let msg = format!(
            "Error reading config file {}: {}",
            real_config_file.display(),
            e
        );
        error!("{}", msg);
        Box::new(e) as Box<dyn std::error::Error>
    })
}

fn generate_ed25519_key_pair_to_local() {
    // Get temp path
    let temp_dir = std::env::temp_dir();
    let key_dir = temp_dir.join("buckyos").join("keys");
    if !key_dir.is_dir() {
        std::fs::create_dir_all(&key_dir).unwrap();
    }
    println!("key_dir: {:?}", key_dir);

    let (private_key, public_key) = generate_ed25519_key_pair();

    let sk_file = key_dir.join("private_key.pem");
    std::fs::write(&sk_file, private_key).unwrap();
    println!("Private key saved to: {:?}", sk_file);

    let pk_file = key_dir.join("public_key.json");
    std::fs::write(&pk_file, serde_json::to_string(&public_key).unwrap()).unwrap();
    println!("Public key saved to: {:?}", pk_file);
}

fn main() {
    std::env::set_var("RUST_BACKTRACE", "1");

    let matches = Command::new("CYFS Gateway Service")
        .version(buckyos_kit::get_version())
        .arg(
            Arg::new("config")
                .long("config")
                .help("config in json format")
                .required(false),
        )
        .arg(
            Arg::new("config_file")
                .long("config_file")
                .help("config file path file with json format content")
                .required(false),
        )
        .arg(
            Arg::new("keep_tunnel")
                .long("keep_tunnel")
                .help("keep tunnel when start")
                .num_args(1..),
        )
        .arg(
            Arg::new("debug")
                .long("debug")
                .help("enable debug mode")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("new_key_pair")
                .long("new-key-pair")
                .help("Generate a new key pair for service")
                .required(false)
                .action(ArgAction::SetTrue),
        )
        .get_matches();

    // set buckyos root dir

    if matches.get_flag("new_key_pair") {
        generate_ed25519_key_pair_to_local();
        std::process::exit(0);
    }

    // init log
    init_logging("cyfs_gateway",true);
    info!("cyfs_gateway start...");

    let config: String = load_config_from_args(&matches)
        .map_err(|e| {
            error!("Error loading config: {}", e);
            std::process::exit(1);
        })
        .unwrap();

    info!("Gateway config: {}", config);

    let rt = tokio::runtime::Runtime::new().unwrap();

    if matches.get_flag("debug") {
        info!("Debug mode enabled");
        console_subscriber::init();
    }

    rt.block_on(async {
        if let Err(e) = service_main(&config, &matches).await {
            error!("Gateway run error: {}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_loader::GatewayConfig;
    use tokio::net::UdpSocket;
    use tokio::task;
    use tokio::test;

    async fn start_test_udp_echo_server() -> Result<()> {
        let socket = UdpSocket::bind("0.0.0.0:8889").await.unwrap();

        let mut buf = [0; 2048]; // 缓冲区，接收数据

        loop {
            let (len, addr) = socket.recv_from(&mut buf).await.unwrap();
            socket.send_to(&buf[..len], &addr).await?;
            info!("echo {} bytes back to {}", len, addr);
        }
    }

    #[test]
    async fn test_dispatcher() {
        std::env::set_var("BUCKY_LOG", "debug");
        buckyos_kit::init_logging("test_dispatcher",false);
        buckyos_kit::start_tcp_echo_server("127.0.0.1:8888").await;
        buckyos_kit::start_udp_echo_server("127.0.0.1:8889").await;

        let config = r#"
        {
            "tcp://0.0.0.0:6001":{
                "type":"forward",
                "target":"tcp:///:8888"
            },
            "udp://0.0.0.0:6002":{
                "type":"forward",
                "target":"udp:///:8889"
            },
            "tcp://0.0.0.0:6003":{
                "type":"forward",
                "target":"socks://192.168.1.188:7890/qq.com:80"
            },
            "tcp://0.0.0.0:6004":{
                "type":"probe_selector",
                "probe_id":"https-sni",
                "selector_id":"smart-selector",
            },
            "tcp://0.0.0.0:6005":{
                "type":"selector",
                "selector_id":"smart-selector",
            }

        }
        "#;
        let config: serde_json::Value = serde_json::from_str(config).unwrap();
        let dispatcher_cfg = GatewayConfig::load_dispatcher_config(&config)
            .await
            .unwrap();

        let dispatcher = dispatcher::ServiceDispatcher::new(dispatcher_cfg);
        dispatcher.start().await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        buckyos_kit::start_tcp_echo_client("127.0.0.1:6001").await;
        buckyos_kit::start_udp_echo_client("127.0.0.1:6002").await;
        tokio::time::sleep(std::time::Duration::from_secs(100)).await;
    }
}
