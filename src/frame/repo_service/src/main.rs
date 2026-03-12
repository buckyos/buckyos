#![allow(unused, dead_code)]

#[cfg(test)]
mod test;

use buckyos_api::*;
use std::time::Duration;

use log::*;
use buckyos_kit::*;

use anyhow::Result;

const REPO_SERVICE_MAIN_PORT: u16 = 4000;

async fn service_main() -> Result<()> {
    init_logging("repo_service", true);
    match init_buckyos_api_runtime("repo-service", None, BuckyOSRuntimeType::KernelService).await {
        Ok(mut runtime) => match runtime.login().await {
            Ok(_) => {
                runtime.set_main_service_port(REPO_SERVICE_MAIN_PORT).await;
                set_buckyos_api_runtime(runtime);
            }
            Err(err) => {
                warn!("repo service login skipped during refactor placeholder mode: {err}");
            }
        },
        Err(err) => {
            warn!("repo service runtime init skipped during refactor placeholder mode: {err}");
        }
    }

    info!("repo service implementation removed during refactor; running placeholder loop");
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    if let Err(err) = rt.block_on(service_main()) {
        error!("repo service exited with error: {err}");
        std::process::exit(1);
    }
}
