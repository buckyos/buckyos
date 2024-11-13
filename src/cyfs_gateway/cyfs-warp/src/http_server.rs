#![allow(unused)]

use std::collections::HashMap;

use std::net::SocketAddr;
use std::sync::Arc;
use std::fs;
use futures::stream::StreamExt;
use std::fs::File;
use std::io::BufReader;
use rustls_pemfile::{certs, pkcs8_private_keys};

use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_stream::wrappers::TcpListenerStream;
use tokio::time::{timeout, Duration};
use anyhow::Result;
use log::*;

use rustls::{Certificate, ServerConfig};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use hyper::server::accept::from_stream;

use cyfs_gateway_lib::*;
use crate::router::*;



async fn handle_request(
    router: Arc<Router>,
    tls_config: Option<Arc<ServerConfig>>,
    req: Request<Body>,
    client_ip:SocketAddr,
) -> Result<Response<Body>, hyper::Error> {
    
    match router.route(req,client_ip).await {
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

async fn _handle_request(
    router: Arc<Router>,
    tls_config: Option<Arc<ServerConfig>>,
    req: Request<Body>,
    client_ip:SocketAddr,
) -> Result<Response<Body>, hyper::Error> {
    let timeout_duration = Duration::from_secs(30);
    let result = timeout(timeout_duration, handle_request(router, tls_config, req, client_ip)).await;
    match result {
        Ok(res) => res,
        Err(_) => {
            Ok(Response::builder()
            .status(500)
            .body(Body::from("Internal Server Error:process timeout"))
            .unwrap())
        }
    }
}

pub async fn start_cyfs_warp_server(config:WarpServerConfig) -> Result<()> {
    let router = Arc::new(Router::new(config.clone()));
    let router2 = router.clone();
    let mut default_cert = None;
    let mut default_key = None;
    let tls_configs: HashMap<String, Arc<ServerConfig>> = config
        .hosts
        .iter()
        .filter_map(|(host, host_config)| {
            host_config.tls.as_ref().map(|tls_config| {
                let cert_file = &mut BufReader::new(File::open(&tls_config.cert_path).unwrap());
                let certs = rustls_pemfile::certs(cert_file).unwrap();
                if certs.is_empty() {
                    panic!("No certificates found in cert file");
                }
                let mut cert:Vec<Certificate> = certs.into_iter().map(Certificate).collect();
                let cert = cert.remove(0);
                info!("load tls cert: {:?} OK",cert);
                default_cert = Some(cert.clone());

                let key_file = &mut BufReader::new(File::open(&tls_config.key_path).unwrap());
                let mut keys = pkcs8_private_keys(key_file).unwrap();
                if keys.is_empty() {
                    panic!("No private keys found in key file");
                }
                let key = rustls::PrivateKey(keys.remove(0));
                default_key = Some(key.clone());

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

    let bind_addr = config.bind.unwrap_or("0.0.0.0".to_string());
    let bind_addr_http = bind_addr.clone();
    if default_cert.is_some() && default_key.is_some() && config.default_tls_host.is_some(){
        let default_tls_host = config.default_tls_host.unwrap();

        tokio::task::spawn(async move {
            let tls_cfg = Arc::new(ServerConfig::builder()
                .with_safe_defaults()
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(SNIResolver::new(tls_configs.clone(),default_tls_host.clone()))));

            let tls_acceptor = TlsAcceptor::from(tls_cfg.clone());

            let make_svc = make_service_fn(move |conn: &tokio_rustls::server::TlsStream<tokio::net::TcpStream>| {
                let router = router.clone();
                let sni_hostname = conn.get_ref().1.server_name().unwrap_or(default_tls_host.as_str()).to_owned();
                let client_ip = conn.get_ref().0.peer_addr().unwrap();
                let tls_cfg = tls_cfg.clone();
                
                async move {
                    Ok::<_, hyper::Error>(service_fn(move |req| {
                        handle_request(router.clone(), Some(tls_cfg.clone()), req,client_ip)
                    }))
                }
            });
        
            
            let https_bind_addr = format!("{}:{}",bind_addr,config.tls_port);
            //let addr = SocketAddr::from(([0, 0, 0, 0], tls_port));
            let listener = TcpListener::bind(https_bind_addr.clone()).await;
            if listener.is_err() {
                error!("bind https server failed, please check the port is used");
                return;
            }
            let listener = listener.unwrap();
            let listener_stream = TcpListenerStream::new(listener);
            let tls_acceptor = Arc::new(tls_acceptor);

            let incoming_tls_stream = listener_stream.filter_map(|conn| {
                info!("tls accept a new tcp stream ...");
                let tls_acceptor = tls_acceptor.clone();
                async move {
                    match conn {
                        Ok(stream) => {
                            match tls_acceptor.accept(stream).await {
                                Ok(tls_stream) => {
                                    info!("tls accept a new tls from tcp stream OK!");
                                    Some(Ok::<_, std::io::Error>(tls_stream))
                                },
                                Err(e) => {
                                    warn!("TLS handshake failed: {:?}", e);
                                    None // Ignore failed connections
                                }
                            }
                        }
                        Err(e) => {
                            warn!("TLS Connection acceptance failed: {:?}", e);
                            None
                        }
                    }    
                }
            });
        
            let acceptor = from_stream(incoming_tls_stream);
            let server = Server::builder(acceptor).serve(make_svc);
            info!("cyfs-warp HTTPs Server running on https://{}", https_bind_addr);
            server.await;
        });
    }

    let http_bind_addr = format!("{}:{}",bind_addr_http,config.http_port);
    let listener_http = TcpListener::bind(http_bind_addr.clone()).await;
    let listener_http = listener_http.unwrap();
    let listener_stream_http = TcpListenerStream::new(listener_http);
    let http_acceptor = from_stream(listener_stream_http);
    let make_svc = make_service_fn(move |conn: &tokio::net::TcpStream| {
        let router = router2.clone();
        let client_ip = conn.peer_addr().unwrap();
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                handle_request(router.clone(), None, req,client_ip)
            }))
        }
    });
    let server_http = Server::builder(http_acceptor).serve(make_svc);
    info!("cyfs-warp HTTP Server running on http://{}", http_bind_addr);
    let _ = server_http.await;

    Ok(())
}


    


