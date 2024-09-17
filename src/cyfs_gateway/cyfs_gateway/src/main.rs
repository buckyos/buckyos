#![allow(dead_code)]

//mod config;
//mod gateway;
//mod interface;
mod log_util;
mod dispatcher;
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

    //let gateway = Gateway::load(&json).await?;

    //gateway.start().await?;

    // Start http interface
    //let interface = interface::GatewayInterface::new(
    //    gateway.upstream_manager(),
    //    gateway.proxy_manager(),
    //    gateway.config_storage(),
    //);
    //interface.start().await?;

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
