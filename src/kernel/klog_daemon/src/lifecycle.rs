use klog::network::KNetworkServer;
use log::{error, info};
use tokio::task::JoinHandle;

pub async fn run_server_lifecycle(
    network_server: KNetworkServer,
    auto_join_task: Option<JoinHandle<()>>,
) -> Result<(), String> {
    let server_result = network_server.run_with_shutdown(shutdown_signal()).await;
    stop_auto_join_task(auto_join_task).await;
    server_result
}

pub async fn stop_auto_join_task(join_task: Option<JoinHandle<()>>) {
    if let Some(handle) = join_task {
        handle.abort();
        let _ = handle.await;
        info!("Auto-join task stopped because network server exited");
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            error!("Failed to listen for ctrl-c shutdown signal: {}", e);
        } else {
            info!("Received ctrl-c shutdown signal");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
                info!("Received SIGTERM shutdown signal");
            }
            Err(e) => {
                error!("Failed to listen for SIGTERM shutdown signal: {}", e);
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
