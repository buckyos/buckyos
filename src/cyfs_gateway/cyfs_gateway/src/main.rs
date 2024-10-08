#![allow(dead_code)]
#![allow(unused_imports)]
//mod config;
//mod gateway;
//mod interface;
mod dispatcher;
mod config_loader;
//mod peer;
//mod proxy;
//mod service;
//mod storage;
//mod tunnel;


use std::path::PathBuf;
use log::*;
use clap::{Arg, Command};
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use buckyos_kit::*;
use tokio::task;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn service_main(config: &str) -> Result<()> {
    let _json = serde_json::from_str(config).map_err(|e| {
        let msg = format!("Error parsing config: {} {}", e, config);
        error!("{}", msg);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    //load config
    let mut config_loader = config_loader::GatewayConfig::new();
    let load_result = config_loader.load_from_json_value(_json).await;
    if load_result.is_err() {
        let msg = format!("Error loading config: {}", load_result.err().unwrap());
        error!("{}", msg);
        std::process::exit(1);
    }

    //start servers
    for (server_id,server_config) in config_loader.servers.iter() {
        match server_config {
            ServerConfig::Warp(warp_config) => {
                let warp_config = warp_config.clone();
                task::spawn(async move {
                    let _ = start_cyfs_warp_server(warp_config).await;
                });
            },
            _ => {
                error!("Invalid server type: {}", server_id);
            },
        }
    }
    
    //start dispatcher
    let dispatcher = dispatcher::ServiceDispatcher::new(config_loader.dispatcher.clone());
    dispatcher.start().await;

    // sleep forever
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
        let msg = format!("Error reading config file {}: {}", real_config_file.display(), e);
        error!("{}", msg);
        Box::new(e) as Box<dyn std::error::Error>
    })

}

fn main() {
    let matches = Command::new("CYFS Gateway Service")
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
        .get_matches();

    // init log
    init_logging("cyfs_gateway");
    info!("cyfs_gateway start...");

    let config: String = load_config_from_args(&matches)
        .map_err(|e| {
            error!("Error loading config: {}", e);
            std::process::exit(1);
        })
        .unwrap();

    info!("Gateway config: {}", config);

    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        if let Err(e) = service_main(&config).await {
            error!("Gateway run error: {}", e);
        }
    });
}


#[cfg(test)]
mod tests {
    use super::*;
    use tokio::test;
    use tokio::net::UdpSocket;
    use tokio::task;

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
    async fn test_service_main() {
        task::spawn(async {
            start_test_udp_echo_server().await.unwrap();
        });
        let config = r#"
        {
            "dispatcher" : {
                "tcp://0.0.0.0:6001":{
                    "type":"forward",
                    "target":"tcp://192.168.1.188:8888"
                },
                "udp://0.0.0.0:6002":{
                    "type":"forward",
                    "target":"udp://192.168.1.188:8889"
                }
            }
        }
        "#;
        service_main(config).await.unwrap();
    }
}
    