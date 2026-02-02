mod sled_msg_queue;

use buckyos_kit::init_logging;
use buckyos_api::*;
use buckyos_api::msg_queue::KMSG_SERVICE_MAIN_PORT;

use std::sync::Arc;

use log::error;
use sled_msg_queue::SledMsgQueueServer;
use server_runner::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging("kmsg",true);
    let mut runtime = init_buckyos_api_runtime("kmsg",None,BuckyOSRuntimeType::KernelService).await?;
    let login_result = runtime.login().await;
    if  login_result.is_err() {
        error!("kmsg service login to system failed! err:{:?}", login_result);
        return Err(anyhow::anyhow!("kmsg service login to system failed! err:{:?}", login_result));
    }
    runtime.set_main_service_port(KMSG_SERVICE_MAIN_PORT).await;
    set_buckyos_api_runtime(runtime);

    let server = SledMsgQueueServer::new();

    let runner = Runner::new(KMSG_SERVICE_MAIN_PORT);
    if let Err(err) = runner.add_http_server("/kapi/kmsg".to_string(), Arc::new(server)) {
        error!("failed to add kmsg http server: {:?}", err);
    }
    if let Err(err) = runner.run().await {
        error!("kmsg runner exited with error: {:?}", err);
    }

    Ok(())
}
