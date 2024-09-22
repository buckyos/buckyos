#![allow(dead_code)]

//mod config;
//mod gateway;
//mod interface;
mod log_util;
mod dispatcher;
mod config_loader;
//mod peer;
//mod proxy;
//mod service;
//mod storage;
//mod tunnel;

use log::*;
use clap::{Arg, ArgGroup, Command};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn service_main(config: &str) -> Result<()> {
    let _json = serde_json::from_str(config).map_err(|e| {
        let msg = format!("Error parsing config: {} {}", e, config);
        error!("{}", msg);
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    //load config
    let mut config_loader = config_loader::ConfigLoader::new();
    let load_result = config_loader.load_from_json_value(_json);
    if load_result.is_err() {
        let msg = format!("Error loading config: {}", load_result.err().unwrap());
        error!("{}", msg);
        std::process::exit(1);
    }

    //start servers
    
    //start dispatcher
    let dispatcher = dispatcher::ServiceDispatcher::new(config_loader.dispatcher.clone());
    dispatcher.start().await;

    // sleep forever
    let _ = tokio::signal::ctrl_c().await;

    Ok(())
}

// Parse config first, then config file if supplied by user
fn load_config_from_args(matches: &clap::ArgMatches) -> Result<String> {
    if let Some(config) = matches.get_one::<String>("config") {
        Ok(config.clone())
    } else {
        let config_file: &String = matches.get_one("config_file").unwrap();
        std::fs::read_to_string(config_file).map_err(|e| {
            let msg = format!("Error reading config file {}: {}", config_file, e);
            error!("{}", msg);
            Box::new(e) as Box<dyn std::error::Error>
        })
    }
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
    log_util::init_logging().unwrap();

    // Gets a value for config if supplied by user, or defaults to "default.json"
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
        log_util::init_logging().unwrap();
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
    