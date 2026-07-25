use buckyos_api::{
    Event, SharedKEventRingBuffer, KEVENT_SERVICE_MAIN_PORT, KEVENT_SERVICE_NATIVE_PORT,
};
use buckyos_http_server::Runner;
use kevent::{run_native_tcp_server, KEventHttpServer, KEventService};
use log::{error, info};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

const SHARED_RING_DRAIN_BATCH: usize = 128;
#[cfg(target_os = "linux")]
const SHARED_RING_WAIT_TIMEOUT_MS: u64 = 500;
#[cfg(not(target_os = "linux"))]
const SHARED_RING_WAIT_TIMEOUT_MS: u64 = 1;

pub async fn start_node_kevent_service(service: Arc<KEventService>) {
    info!(
        "start kevent service on http port {} and native tcp port {} for source_node={}",
        KEVENT_SERVICE_MAIN_PORT,
        KEVENT_SERVICE_NATIVE_PORT,
        service.source_node()
    );

    match SharedKEventRingBuffer::open() {
        Ok(shared_ring) => {
            let shared_ring = Arc::new(shared_ring);
            service.set_shared_ring(shared_ring.clone()).await;
            start_shared_ring_importer(service.clone(), shared_ring);
        }
        Err(err) => {
            error!("kevent shared ring disabled: {}", err);
        }
    }

    let http_server = Arc::new(KEventHttpServer::new(service.clone()));
    let runner = Runner::new(KEVENT_SERVICE_MAIN_PORT);

    let add_result = runner.add_http_server("/kapi/kevent".to_string(), http_server);
    if let Err(err) = add_result {
        error!("Failed to add kevent http server: {}", err);
        return;
    }

    let native_service = service.clone();
    tokio::spawn(async move {
        if let Err(err) = start_native_tcp_server(native_service).await {
            error!("kevent native tcp server stopped: {}", err);
        }
    });

    runner.run().await;
}

fn start_shared_ring_importer(
    service: Arc<KEventService>,
    shared_ring: Arc<SharedKEventRingBuffer>,
) {
    shared_ring.prime_cursors();

    let runtime_handle = tokio::runtime::Handle::current();
    if let Err(err) = std::thread::Builder::new()
        .name("kevent-shared-ring-import".to_string())
        .spawn(move || loop {
            let seq_before = shared_ring.load_notify_seq();
            let events = shared_ring.drain_events::<Event>(SHARED_RING_DRAIN_BATCH);

            if !events.is_empty() {
                let service = service.clone();
                runtime_handle.spawn(async move {
                    for event in events {
                        if let Err(err) = service.publish_external_global(event).await {
                            error!("kevent shared ring import failed: {}", err);
                        }
                    }
                });
            }

            shared_ring.wait_for_events(
                seq_before,
                Duration::from_millis(SHARED_RING_WAIT_TIMEOUT_MS),
            );
        })
    {
        error!(
            "failed to start kevent shared ring importer thread: {}",
            err
        );
    }
}

async fn start_native_tcp_server(service: Arc<KEventService>) -> io::Result<()> {
    let addr = format!("0.0.0.0:{}", KEVENT_SERVICE_NATIVE_PORT);
    let listener = TcpListener::bind(&addr).await?;
    info!("kevent native tcp listener bound at {}", addr);
    run_native_tcp_server(service, listener).await
}
