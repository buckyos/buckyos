use crate::cert::*;
use crate::router::*;
use anyhow::Result;
use buckyos_kit::get_buckyos_service_data_dir;
use cyfs_gateway_lib::*;
use futures::stream::StreamExt;
use hyper::server::accept::from_stream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use log::*;
use rustls::ServerConfig;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;
use tokio_stream::wrappers::TcpListenerStream;

pub struct CyfsWarpServer {
    config: WarpServerConfig,

    http_servers: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    https_servers: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl CyfsWarpServer {
    pub fn new(config: WarpServerConfig) -> Self {
        Self {
            config,
            http_servers: Arc::new(Mutex::new(Vec::new())),
            https_servers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn start(&self) -> Result<()> {
        // Router for HTTP and HTTPS
        let https_router = Router::new(HashMap::from_iter(self.config.hosts.iter().map(
            |(host, host_config)| {
                (
                    host.clone(),
                    HashMap::from_iter(host_config.routes.iter().map(|(route, route_config)| {
                        (route.clone(), Arc::new(route_config.clone()))
                    })),
                )
            },
        )));

        let http_router = Router::new(HashMap::from_iter(self.config.hosts.iter().map(
            |(host, host_config)| {
                if host_config.redirect_to_https {
                    (
                        host.clone(),
                        HashMap::from_iter(vec![(
                            "/".to_string(),
                            Arc::new(RouteConfig {
                                enable_cors: host_config.enable_cors,
                                response: Some(ResponseRouteConfig {
                                    status: Some(301),
                                    headers: Some(HashMap::from_iter(vec![(
                                        "Location".to_string(),
                                        format!("https://{}", host),
                                    )])),
                                    body: None,
                                }),
                                upstream: None,
                                local_dir: None,
                                inner_service: None,
                                tunnel_selector: None,
                                bucky_service: None,
                                named_mgr: None,
                            }),
                        )]),
                    )
                } else {
                    (
                        host.clone(),
                        HashMap::from_iter(host_config.routes.iter().map(
                            |(route, route_config)| (route.clone(), Arc::new(route_config.clone())),
                        )),
                    )
                }
            },
        )));

        // Cert manager for HTTPS
        let root_path = get_buckyos_service_data_dir("cyfs-warp");
        info!("Will use cyfs-warp data directory: {}", root_path.display());
        if !root_path.exists() {
            info!("Creating cyfs-warp data directory: {}", root_path.display());
            if let Err(e) = std::fs::create_dir_all(&root_path) {
                let msg = format!(
                    "Failed to create cyfs-warp data directory: {}, {}",
                    e,
                    root_path.display()
                );
                error!("{}", msg);
                return Err(anyhow::anyhow!(msg));
            }
        }

        let mut cert_mgr_config = CertManagerConfig::default();
        cert_mgr_config.keystore_path = root_path.to_string_lossy().to_string();

        let cert_mgr =
            CertManager::new(cert_mgr_config, ChallengeEntry::new(http_router.clone())).await?;

        for (host, host_config) in self.config.hosts.iter() {
            cert_mgr.insert_config(host.clone(), host_config.tls.clone())?;
        }

        // Start all servers
        let bind = self.config.bind.clone().unwrap_or("0.0.0.0".to_string());
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

            let bind_addr_http = format!("{}:{}", formatted_bind_addr, self.config.http_port);
            match Self::start_listen_http(bind_addr_http, http_router).await {
                Ok(server_task) => {
                    self.http_servers.lock().await.push(server_task);
                }
                Err(e) => {
                    // FIXME: should we return error here or just log it?
                    error!("Failed to start HTTP server: {}", e);
                }
            }
            if self.config.tls_port > 0 {
                let bind_addr_https = format!("{}:{}", formatted_bind_addr, self.config.tls_port);
                match Self::start_listen_https(
                    bind_addr_https,
                    https_router,
                    Arc::new(cert_mgr.clone()),
                )
                .await
                {
                    Ok(server_task) => {
                        self.https_servers.lock().await.push(server_task);
                    }
                    Err(e) => {
                        // FIXME: should we return error here or just log it?
                        error!("Failed to start HTTPS server: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let mut http_servers = self.http_servers.lock().await;
        for server in http_servers.iter_mut() {
            server.abort();
        }
        http_servers.clear();

        let mut https_servers = self.https_servers.lock().await;
        for server in https_servers.iter_mut() {
            server.abort();
        }
        https_servers.clear();

        Ok(())
    }

    async fn handle_request(
        router: Router,
        req: Request<Body>,
        client_ip: SocketAddr,
    ) -> Result<Response<Body>, hyper::Error> {
        match router.route(req, client_ip).await {
            Ok(response) => Ok(response),
            Err(_e) => {
                //error!("Error handling request: {}", e.to_string());
                Ok(Response::builder()
                    .status(500)
                    .body(Body::from("Internal Server Error"))
                    .unwrap())
            }
        }
    }

    async fn start_listen_http(
        http_bind_addr: String,
        http_router: Router,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let listener =
            TcpListener::bind(http_bind_addr.clone())
                .await
                .map_err(|e: std::io::Error| {
                    error!("bind http server {} failed,  {}", http_bind_addr, e);
                    anyhow::anyhow!("bind http server {} failed, {}", http_bind_addr, e)
                })?;
        let listener_stream_http = TcpListenerStream::new(listener);
        let http_acceptor = from_stream(listener_stream_http);

        let make_svc = make_service_fn(move |conn: &tokio::net::TcpStream| {
            let client_ip = conn.peer_addr().unwrap();
            let http_router = http_router.clone();
            async move {
                Ok::<_, hyper::Error>(service_fn(move |req| {
                    Self::handle_request(http_router.clone(), req, client_ip)
                }))
            }
        });

        let server_http = Server::builder(http_acceptor).serve(make_svc);
        info!("cyfs-warp HTTP Server running on http://{}", http_bind_addr);

        let server_task = tokio::task::spawn(async move {
            match server_http.await {
                Ok(_) => {
                    info!("cyfs-warp HTTP Server stopped: {}", http_bind_addr);
                }
                Err(e) => {
                    error!(
                        "cyfs-warp HTTP Server stopped with error: {:?}, {}",
                        e, http_bind_addr
                    );
                }
            }
        });

        Ok(server_task)
    }

    async fn start_listen_https(
        https_bind_addr: String,
        https_router: Router,
        cert_mgr: Arc<CertManager<ChallengeEntry>>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let tls_cfg = Arc::new(
            ServerConfig::builder()
                .with_safe_defaults()
                .with_no_client_auth()
                .with_cert_resolver(cert_mgr),
        );
        let tls_acceptor = TlsAcceptor::from(tls_cfg.clone());
        let listener = TcpListener::bind(https_bind_addr.clone()).await;
        if listener.is_err() {
            error!(
                "bind https server {} failed, please check the port is used",
                https_bind_addr
            );
            return Err(anyhow::anyhow!(
                "bind https server {} failed, please check the port is used",
                https_bind_addr
            ));
        }
        let listener = listener.unwrap();
        let listener_stream = TcpListenerStream::new(listener);
        let incoming_tls_stream = listener_stream.filter_map(move |conn| {
            info!("tls accept a new tcp stream ...");
            let tls_acceptor = tls_acceptor.clone();
            async move {
                match conn {
                    Ok(stream) => {
                        match tls_acceptor.accept(stream).await {
                            Ok(tls_stream) => {
                                info!("tls accept a new tls from tcp stream OK!");
                                Some(Ok::<_, std::io::Error>(tls_stream))
                            }
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
        let make_svc = make_service_fn(
            move |conn: &tokio_rustls::server::TlsStream<tokio::net::TcpStream>| {
                let client_ip = conn.get_ref().0.peer_addr().unwrap();
                let https_router = https_router.clone();
                async move {
                    Ok::<_, hyper::Error>(service_fn(move |req| {
                        Self::handle_request(https_router.clone(), req, client_ip)
                    }))
                }
            },
        );

        let server = Server::builder(acceptor).serve(make_svc);
        info!(
            "cyfs-warp HTTPS Server running on https://{}",
            https_bind_addr
        );

        let server_task = tokio::task::spawn(async move {
            match server.await {
                Ok(_) => {
                    info!("cyfs-warp HTTPS Server stopped: {}", https_bind_addr);
                }
                Err(e) => {
                    error!(
                        "cyfs-warp HTTPS Server stopped with error: {:?}, {}",
                        e, https_bind_addr
                    );
                }
            }
        });

        Ok(server_task)
    }
}

pub async fn start_cyfs_warp_server(config: WarpServerConfig) -> Result<CyfsWarpServer> {
    let server = CyfsWarpServer::new(config);
    server.start().await?;

    Ok(server)
}
