#![allow(dead_code)]

mod config;
mod error;
mod gateway;
mod peer;
mod proxy;
mod service;
mod tunnel;
mod constants;

#[macro_use]
extern crate log;

use clap::{Command, Arg};
use error::*;
use gateway::Gateway;

//#[cfg(test)]
mod test;

async fn run(config: &str) -> GatewayResult<()> {
    let json = serde_json::from_str(config).map_err(|e| {
        let msg = format!("Error parsing config: {} {}", e, config);
        error!("{}", msg);
        GatewayError::InvalidConfig(msg)
    })?;

    let gateway = Gateway::load(&json)?;

    gateway.start().await?;

    // sleep forever
    let _ = tokio::signal::ctrl_c().await;

    Ok(())
}

fn main() {
    let matches = Command::new("Gateway service")
        .arg(
            Arg::new("config")
                .long("config")
                .help("config in json format")
                .required(true),
        )
        .get_matches();

    // Gets a value for config if supplied by user, or defaults to "default.json"
    let config: &String = matches.get_one("config").unwrap();
    info!("Gateway config: {}", config);

    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        if let Err(e) = run(&config).await {
            error!("Gateway run error: {}", e);
        }
    });
}
