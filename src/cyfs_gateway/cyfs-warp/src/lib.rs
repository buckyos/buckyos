mod config;
mod router;


pub use config::*;
pub use router::*;

use anyhow::Result;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use log::{error, info};
use rustls::ServerConfig;
use tokio::net::TcpListener;
use core::task;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::fs;
use tokio_rustls::TlsAcceptor;
use tokio_stream::wrappers::TcpListenerStream;
use futures::stream::StreamExt;
use hyper::server::accept::from_stream;
use cyfs_gateway_lib::*;


async fn cyfs_warp_main() -> Result<()> {
    init_logging();
    let tls_port = 3000;
    let http_port = 3002;

    let config = Config::from_file("d:\\temp\\config.toml").await?;
    let router = Arc::new(Router::new(config.clone()));
    let router2 = router.clone();

    let tls_configs: HashMap<String, Arc<ServerConfig>> = config
        .hosts
        .iter()
        .filter_map(|(host, host_config)| {
            host_config.tls.as_ref().map(|tls_config| {
                let cert = fs::read(&tls_config.cert_path).unwrap();
                let key = fs::read(&tls_config.key_path).unwrap();
                let cert = rustls::Certificate(cert);
                let key = rustls::PrivateKey(key);
                let mut config = ServerConfig::builder()
                    .with_safe_defaults()
                    .with_no_client_auth()
                    .with_single_cert(vec![cert], key)
                    .unwrap();
                config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
                (host.clone(), Arc::new(config))
            })
        })
        .collect();


    
    let tls_cfg = Arc::new(ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(router::SNIResolver::new(tls_configs.clone()))));

    let tls_acceptor = TlsAcceptor::from(tls_cfg);
    
    let make_svc = make_service_fn(move |conn: &tokio_rustls::server::TlsStream<tokio::net::TcpStream>| {
        let router = router.clone();
        let tls_configs = tls_configs.clone();
        let sni_hostname = conn.get_ref().1.server_name().unwrap_or_default().to_owned();
        let tls_config = tls_configs.get(&sni_hostname).cloned();
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                handle_request(router.clone(), tls_config.clone(), req)
            }))
        }
    });
    let addr = SocketAddr::from(([0, 0, 0, 0], tls_port));
    let listener = TcpListener::bind(addr).await?;
    let listener_stream = TcpListenerStream::new(listener);
    let tls_acceptor = Arc::new(tls_acceptor);
    // 创建一个流来处理 TLS 握手
    let incoming_tls_stream = listener_stream.filter_map(|conn| {
        let tls_acceptor = tls_acceptor.clone();
        async move {
            match conn {
                Ok(stream) => {
                    match tls_acceptor.accept(stream).await {
                        Ok(tls_stream) => Some(Ok::<_, std::io::Error>(tls_stream)),
                        Err(e) => {
                            eprintln!("TLS handshake failed: {:?}", e);
                            None // Ignore failed connections
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Connection acceptance failed: {:?}", e);
                    None
                }
            }
                       
        }
    });

    let acceptor = from_stream(incoming_tls_stream);
    let server = Server::builder(acceptor).serve(make_svc);
    info!("HTTPS Server running on https://{}", addr);

    tokio::task::spawn(async move {
        let addr_http = SocketAddr::from(([0, 0, 0, 0], http_port));
        let listener_http = TcpListener::bind(addr_http).await;
        let listener_http = listener_http.unwrap();
        let listener_stream_http = TcpListenerStream::new(listener_http);
        let http_acceptor = from_stream(listener_stream_http);
        let make_svc = make_service_fn(move |conn| {
            let router = router2.clone();
            async move {
                Ok::<_, hyper::Error>(service_fn(move |req| {
                    handle_request(router.clone(), None, req)
                }))
            }
        });
        let server_http = Server::builder(http_acceptor).serve(make_svc);
        info!("HTTP Server running on http://{}", addr_http);
        server_http.await;
    });

    server.await?;

    Ok(())
}

async fn handle_request(
    router: Arc<Router>,
    tls_config: Option<Arc<ServerConfig>>,
    req: Request<Body>,
) -> Result<Response<Body>, hyper::Error> {
    match router.route(req, tls_config).await {
        Ok(response) => Ok(response),
        Err(e) => {
            error!("Error handling request: {:?}", e);
            Ok(Response::builder()
                .status(500)
                .body(Body::from("Internal Server Error"))
                .unwrap())
        }
    }
}

mod test {
    use super::*;

    #[tokio::test]
    async fn test_cyfs_warp_main() {
        let result = cyfs_warp_main().await;
        println!("result: {:?}", result);
        assert!(result.is_ok());
    }
}