use buckyos_api::KEVENT_SERVICE_MAIN_PORT;
use kevent::{KEventHttpServer, KEventService};
use log::{error, info};
use server_runner::Runner;
use std::sync::Arc;

pub async fn start_node_kevent_service(source_node: String) {
    info!(
        "start kevent service on port {} for source_node={}",
        KEVENT_SERVICE_MAIN_PORT, source_node
    );

    let service = Arc::new(KEventService::new(source_node));
    let http_server = Arc::new(KEventHttpServer::new(service));
    let runner = Runner::new(KEVENT_SERVICE_MAIN_PORT);

    let add_result = runner.add_http_server("/kapi/kevent".to_string(), http_server);
    if let Err(err) = add_result {
        error!("Failed to add kevent http server: {}", err);
        return;
    }

    runner.run().await;
}
