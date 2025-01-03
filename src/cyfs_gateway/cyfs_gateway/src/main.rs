#![allow(dead_code)]
#![allow(unused_imports)]
//mod config;
//mod gateway;
//mod interface;
mod dispatcher;
mod config_loader;
mod socks;

//mod peer;
//mod proxy;
//mod service;
//mod storage;
//mod tunnel;

#[macro_use]
extern crate log;

use std::path::PathBuf;
use cyfs_dns::start_cyfs_dns_server;
use log::*;
use clap::{Arg, ArgAction, Command};
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use buckyos_kit::*;
use tokio::task;
use url::Url;
use name_client::*;
use name_lib::*;
use console_subscriber::{self, Server};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn service_main(config: &str,matches: &clap::ArgMatches) -> Result<()> {
    let _json = serde_json::from_str(config).map_err(|e| {
        let msg = format!("Error parsing config: {} {}", e, config);
        error!("{}", msg);
        Box::new(e) as Box<dyn std::error::Error>
    })?;


    //load config
    let load_result = config_loader::GatewayConfig::load_from_json_value(_json).await;
    if load_result.is_err() {
        let msg = format!("Error loading config: {}", load_result.err().unwrap());
        error!("{}", msg);
        std::process::exit(1);
    }
    let config_loader = load_result.unwrap();
    
    let disable_buckyos = matches.get_flag("disable-buckyos");
    if !disable_buckyos {
        init_global_buckyos_value_by_env("GATEWAY");
        let this_device = CURRENT_DEVICE_CONFIG.get();
        let this_device = this_device.unwrap();
        let this_device_info = DeviceInfo::from_device_doc(this_device);
        let session_token = CURRENT_APP_SESSION_TOKEN.get();
        let _ = enable_zone_provider (Some(&this_device_info),session_token,true).await;
        //keep tunnel
        let keep_tunnel = matches.get_many::<String>("keep_tunnel");
        if keep_tunnel.is_some() {
            let keep_tunnel = keep_tunnel.unwrap();
            let keep_tunnel: Vec<String> = keep_tunnel.map(|s| s.to_owned()).collect();
            
            for tunnel in keep_tunnel.iter() {
                let tunnel_url = format!("rtcp://{}",tunnel);
                info!("keep tunnel: {}", tunnel_url);
                let tunnel_url = Url::parse(tunnel_url.as_str());
                if tunnel_url.is_err() {
                    warn!("Invalid tunnel url: {}", tunnel_url.err().unwrap());
                    continue;
                }
                
                task::spawn(async move {
                    let tunnel_url = tunnel_url.unwrap();
                    loop {
                        let last_ok;
                        let tunnle = get_tunnel(&tunnel_url,None).await;
                        if tunnle.is_err() {
                            warn!("Error getting tunnel: {}", tunnle.err().unwrap());
                            last_ok = false;
                        } else {
                            let tunnel = tunnle.unwrap();
                            let ping_result = tunnel.ping().await;
                            if ping_result.is_err() {
                                warn!("Error pinging tunnel: {}", ping_result.err().unwrap());
                                last_ok = false;
                            } else {
                                last_ok = true;
                            }
                        }

                        if last_ok {
                            tokio::time::sleep(std::time::Duration::from_secs(60*2)).await;
                        } else {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                        }
                    }
                });
            }
        }
    } else {
        info!("TODO:disable buckyos,set device config for test");
        init_default_name_client().await.unwrap();

        let pk = if let Some(sk)  = CURRENT_DEVICE_RRIVATE_KEY.get() {
            let pk_value = encode_ed25519_pkcs8_sk_to_pk(sk);
            info!("Will use device pk: {}",pk_value);
            pk_value
        } else {
            // TODO use default pk or set it to none? 
            let pk_value = "8vlobDX73HQj-w5TUjC_ynr_ljsWcDAgVOzsqXCw7no".to_string();
            info!("Will use default device pk: {}",pk_value);
            pk_value
        };

        let this_device_config = DeviceConfig::new("web3.buckyos.io", Some(pk));
        // load device config from config files
        let set_result = CURRENT_DEVICE_CONFIG.set(this_device_config);
        if set_result.is_err() {
            error!("Failed to set CURRENT_DEVICE_CONFIG");
        }
    }

    //start servers
    for (_server_id,server_config) in config_loader.servers.into_iter() {
        match server_config {
            ServerConfig::Warp(warp_config) => {
                let warp_config = warp_config.clone();
                task::spawn(async move {
                    let _ = start_cyfs_warp_server(warp_config).await;
                });
            },
            ServerConfig::DNS(dns_config) => {
                let dns_config = dns_config.clone();
                task::spawn(async move {
                    let _ = start_cyfs_dns_server(dns_config).await;
                });
            },
            ServerConfig::Socks(socks_config) => {
                let tunnel_provider = crate::socks::SocksTunnelBuilder::new_ref();

                // let socks_config_param = socks_config.clone();
                if let Err(e) = cyfs_socks::start_cyfs_socks_server(socks_config, tunnel_provider).await {
                    error!("Error starting socks server: {}", e);
                }
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

fn generate_ed25519_key_pair_to_local() {
    // Get temp path
    let temp_dir = std::env::temp_dir();
    let key_dir = temp_dir.join("buckyos").join("keys");
    if !key_dir.is_dir() {
        std::fs::create_dir_all(&key_dir).unwrap();
    }
    println!("key_dir: {:?}",key_dir);

    let (private_key, public_key) = generate_ed25519_key_pair();

    let sk_file = key_dir.join("private_key.pem");
    std::fs::write(&sk_file, private_key).unwrap();
    println!("Private key saved to: {:?}",sk_file);

    let pk_file = key_dir.join("public_key.json");
    std::fs::write(&pk_file, serde_json::to_string(&public_key).unwrap()).unwrap();
    println!("Public key saved to: {:?}",pk_file);
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
        .arg(Arg::new("keep_tunnel")
            .long("keep_tunnel")
            .help("keep tunnel when start")
            .num_args(1..))
        .arg(Arg::new("disable-buckyos")
            .long("disable-buckyos")
            .help("disable init buckyos system services")
            .action(ArgAction::SetTrue))
        .arg(Arg::new("debug")
            .long("debug")
            .help("enable debug mode")
            .action(ArgAction::SetTrue))
        .arg(Arg::new("buckyos_root")
            .long("buckyos-root")
            .help("Change buckyos root dir, same as BUCKYOS_ROOT env var")
            .required(false))
        .arg(Arg::new("new_key_pair")
            .long("new-key-pair")
            .help("Generate a new key pair for service")
            .required(false)
            .action(ArgAction::SetTrue))
        .get_matches();

    // set buckyos root dir
    if let Some(buckyos_root) = matches.get_one::<String>("buckyos_root") {
        std::env::set_var("BUCKYOS_ROOT", buckyos_root);
    }

    if matches.get_flag("new_key_pair") {
        generate_ed25519_key_pair_to_local();
        std::process::exit(0);
    }

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

    if matches.get_flag("debug") {
        info!("Debug mode enabled");
        console_subscriber::init();
    }

    rt.block_on(async {
        if let Err(e) = service_main(&config,&matches).await {
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
    