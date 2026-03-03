use klog::network::KNetworkServer;
use klog::rpc::KRpcServer;
use log::{error, info};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub async fn run_server_lifecycle(
    network_server: KNetworkServer,
    rpc_server: Option<KRpcServer>,
    auto_join_task: Option<JoinHandle<()>>,
) -> Result<(), String> {
    let (raft_shutdown_tx_raw, raft_shutdown_rx) = oneshot::channel::<()>();
    let mut raft_shutdown_tx = Some(raft_shutdown_tx_raw);

    let mut raft_task = tokio::spawn(async move {
        network_server
            .run_with_shutdown(async move {
                let _ = raft_shutdown_rx.await;
            })
            .await
    });

    enum Exit {
        Signal,
        Raft(Result<Result<(), String>, tokio::task::JoinError>),
        Rpc(Result<Result<(), String>, tokio::task::JoinError>),
    }

    let (mut rpc_shutdown_tx, mut rpc_task) = if let Some(rpc_server) = rpc_server {
        let (rpc_shutdown_tx_raw, rpc_shutdown_rx) = oneshot::channel::<()>();
        let rpc_task = tokio::spawn(async move {
            rpc_server
                .run_with_shutdown(async move {
                    let _ = rpc_shutdown_rx.await;
                })
                .await
        });
        (Some(rpc_shutdown_tx_raw), Some(rpc_task))
    } else {
        (None, None)
    };

    let server_result = if let Some(mut rpc_task_handle) = rpc_task.take() {
        match tokio::select! {
            _ = shutdown_signal() => Exit::Signal,
            raft_result = &mut raft_task => Exit::Raft(raft_result),
            rpc_result = &mut rpc_task_handle => Exit::Rpc(rpc_result),
        } {
            Exit::Signal => {
                info!("Shutdown signal received, stopping raft and rpc servers");
                if let Some(tx) = raft_shutdown_tx.take() {
                    let _ = tx.send(());
                }
                if let Some(tx) = rpc_shutdown_tx.take() {
                    let _ = tx.send(());
                }
                let raft_result = join_server_task("raft", raft_task).await;
                let rpc_result = join_server_task("rpc", rpc_task_handle).await;
                combine_server_results(raft_result, rpc_result)
            }
            Exit::Raft(raft_result) => {
                if let Some(tx) = rpc_shutdown_tx.take() {
                    let _ = tx.send(());
                }
                let raft_result = map_join_result("raft", raft_result);
                let rpc_result = join_server_task("rpc", rpc_task_handle).await;
                combine_server_results(raft_result, rpc_result)
            }
            Exit::Rpc(rpc_result) => {
                if let Some(tx) = raft_shutdown_tx.take() {
                    let _ = tx.send(());
                }
                let rpc_result = map_join_result("rpc", rpc_result);
                let raft_result = join_server_task("raft", raft_task).await;
                combine_server_results(raft_result, rpc_result)
            }
        }
    } else {
        tokio::select! {
            _ = shutdown_signal() => {
                info!("Shutdown signal received, stopping raft server");
                if let Some(tx) = raft_shutdown_tx.take() {
                    let _ = tx.send(());
                }
                join_server_task("raft", raft_task).await
            }
            raft_result = &mut raft_task => {
                map_join_result("raft", raft_result)
            }
        }
    };

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

fn map_join_result(
    server_name: &str,
    result: Result<Result<(), String>, tokio::task::JoinError>,
) -> Result<(), String> {
    match result {
        Ok(inner) => inner,
        Err(e) => Err(format!("{} server task join failed: {}", server_name, e)),
    }
}

async fn join_server_task(
    server_name: &str,
    handle: JoinHandle<Result<(), String>>,
) -> Result<(), String> {
    map_join_result(server_name, handle.await)
}

fn combine_server_results(a: Result<(), String>, b: Result<(), String>) -> Result<(), String> {
    match (a, b) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(e), Ok(())) | (Ok(()), Err(e)) => Err(e),
        (Err(e1), Err(e2)) => Err(format!(
            "both servers failed: raft_or_rpc_err='{}'; other_err='{}'",
            e1, e2
        )),
    }
}
