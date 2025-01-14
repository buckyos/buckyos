#![allow(unused)]

use std::collections::HashMap;

use std::net::SocketAddr;
use std::sync::Arc;
use std::fs;
use futures::stream::StreamExt;
use tokio::task;
use std::fs::File;
use std::io::BufReader;
use rustls_pemfile::{certs, pkcs8_private_keys};

use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_stream::wrappers::TcpListenerStream;
use tokio::time::{timeout, Duration};
use anyhow::Result;
use log::*;

use rustls::ServerConfig;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use hyper::server::accept::from_stream;

use buckyos_kit::get_buckyos_service_data_dir;
use cyfs_gateway_lib::*;
use crate::router::*;



struct ChallengeEntry {
    router: Router,
}


impl AcmeChallengeEntry for ChallengeEntry {
    type Responder = ChallengeResponder;
    fn create_challenge_responder(&self) -> Self::Responder {
        ChallengeResponder {
            router: self.router.clone(),
        }
    }
}

struct ChallengeResponder {
    router: Router,
}

#[async_trait::async_trait]
impl AcmeChallengeResponder for ChallengeResponder {
    async fn respond_http(&self, domain: &str, token: &str, key_auth: &str) -> Result<()> {
        let path = format!("/.well-known/acme-challenge/{}",token);
        let config = RouteConfig {
            enable_cors: false,
            response: Some(ResponseRouteConfig {
                status: Some(200),
                headers: Some(HashMap::from_iter(vec![("Content-Type".to_string(), "text/plain".to_string())])),
                body: Some(key_auth.to_string()),
            }),
            upstream: None,
            local_dir: None,
            inner_service: None,
            tunnel_selector: None,
            bucky_service: None,
            named_mgr: None,
        };
        self.router.insert_route_config(domain, path.as_str(), config);
        Ok(())
    }
    fn revert_http(&self, domain: &str, token: &str) {
        self.router.remove_route_config(domain, token);
    }

    async fn respond_dns(&self, domain: &str, digest: &str) -> Result<()> {
        Ok(())
    }
    fn revert_dns(&self, domain: &str, digest: &str) {
        
    }

    async fn respond_tls_alpn(&self, domain: &str, key_auth: &str) -> Result<()> {
        Ok(())
    }
    fn revert_tls_alpn(&self, domain: &str, key_auth: &str) {
        
    }
}

async fn handle_request(
    router: Router,
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


async fn listen_http(http_bind_addr: String, http_router: Router) -> Result<()> {
    let listener = TcpListener::bind(http_bind_addr.clone()).await
        .map_err(|e| {
            error!("bind http server {} failed,  {}",http_bind_addr, e);
            anyhow::anyhow!("bind http server {} failed, {}",http_bind_addr, e)
        })?;
    let listener_stream_http = TcpListenerStream::new(listener);
    let http_acceptor = from_stream(listener_stream_http);

    let make_svc = make_service_fn(move |conn: &tokio::net::TcpStream| {
        let client_ip = conn.peer_addr().unwrap();
        let http_router = http_router.clone();
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                handle_request(http_router.clone(), req, client_ip)
            }))
        }
    });
    let server_http = Server::builder(http_acceptor).serve(make_svc);
    info!("cyfs-warp HTTP Server running on http://{}", http_bind_addr);
    server_http.await.unwrap();
    Ok(())
}


async fn listen_https(https_bind_addr: String, https_router: Router, cert_mgr: Arc<CertManager<ChallengeEntry>>) -> Result<()> {
    let tls_cfg = Arc::new(ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_cert_resolver(cert_mgr));
    let tls_acceptor = TlsAcceptor::from(tls_cfg.clone());
    let listener = TcpListener::bind(https_bind_addr.clone()).await;
    if listener.is_err() {
        error!("bind https server {} failed, please check the port is used",https_bind_addr);
        return Err(anyhow::anyhow!("bind https server {} failed, please check the port is used",https_bind_addr));
    }
    let listener = listener.unwrap();
    let listener_stream = TcpListenerStream::new(listener); 
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
    let make_svc = make_service_fn(move |conn: &tokio_rustls::server::TlsStream<tokio::net::TcpStream>| {
        let client_ip = conn.get_ref().0.peer_addr().unwrap();
        let https_router = https_router.clone();
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                handle_request(https_router.clone(), req, client_ip)
            }))
        }
    });
    let server = Server::builder(acceptor).serve(make_svc);
    info!("cyfs-warp HTTPS Server running on https://{}", https_bind_addr);
    server.await.unwrap();
    Ok(())
}

pub async fn start_cyfs_warp_server(config: WarpServerConfig) -> Result<()> {
    let https_router = Router::new(HashMap::from_iter(
        config.hosts.iter().map(|(host, host_config)| {
            (host.clone(), HashMap::from_iter(host_config.routes.iter().map(|(route, route_config)| (route.clone(), Arc::new(route_config.clone())))))
        })
    ));
    

    
    let http_router = Router::new(HashMap::from_iter(
        config.hosts.iter().map(|(host, host_config)| {
            if host_config.redirect_to_https {
                (host.clone(), HashMap::from_iter(vec![("/".to_string(), Arc::new(RouteConfig {
                    enable_cors: host_config.enable_cors,
                    response: Some(ResponseRouteConfig {
                        status: Some(301),
                        headers: Some(HashMap::from_iter(vec![("Location".to_string(), format!("https://{}",host))])),
                        body: None,
                    }),
                    upstream: None,
                    local_dir: None,
                    inner_service: None,
                    tunnel_selector: None,
                    bucky_service: None,
                    named_mgr: None,
                }))]))
            } else {
                (host.clone(), HashMap::from_iter(host_config.routes.iter().map(|(route, route_config)| (route.clone(), Arc::new(route_config.clone())))))
            }
        })
    ));

    let root_path = get_buckyos_service_data_dir("cyfs-warp");
    let mut cert_mgr = CertManager::new(
        root_path.to_string_lossy().to_string(), 
        ChallengeEntry {
            router: http_router.clone(),
        }
    ).await?;

    for (host, host_config) in config.hosts.iter() {
        cert_mgr.insert_config(host.clone(), host_config.tls.clone())?;
    }
    let cert_mgr = Arc::new(cert_mgr);
    

    
    let bind = config.bind.unwrap_or("::;0.0.0.0".to_string());
    let bind_addrs: Vec<&str> = bind.split(';').collect();
    for bind_addr in bind_addrs {
        let http_router = http_router.clone();
        let https_router = https_router.clone();
        let cert_mgr = cert_mgr.clone();

        let formatted_bind_addr = if bind_addr.contains(":") && !bind_addr.starts_with("[") {
            format!("[{}]", bind_addr)
        } else {
            bind_addr.to_string()
        };
        let bind_addr_http = format!("{}:{}",formatted_bind_addr, config.http_port);
        let bind_addr_https = format!("{}:{}",formatted_bind_addr, config.tls_port);
        task::spawn(async move {
            listen_http(bind_addr_http, http_router).await;
        });
        task::spawn(async move {
            listen_https(bind_addr_https, https_router, cert_mgr.clone()).await;
        });
    }

    tokio::signal::ctrl_c().await?;
    info!("Received shutdown signal");
    
    Ok(())
}


    


