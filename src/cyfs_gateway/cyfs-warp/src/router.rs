// src/router.rs
#![allow(dead_code)]
#![allow(unused)]

use anyhow::Result;
use hyper::{Body, Client, Request, Response, StatusCode};
use log::*;
use rustls::ServerConfig;
use url::Url;
use std::{net::SocketAddr, sync::Arc};
use std::path::Path;
use std::collections::HashMap;
use cyfs_gateway_lib::*;
use tokio::sync::{Mutex, OnceCell};
use serde_json::json;
use ::kRPC::*;
use lazy_static::lazy_static;



lazy_static!{
    static ref INNER_SERVICES_BUILDERS: Arc<Mutex< HashMap<String, Arc<dyn Fn () -> Box<dyn kRPCHandler + Send + Sync>+ Send + Sync>>>> = Arc::new(Mutex::new(HashMap::new()));
}

pub async fn register_inner_service_builder<F>(inner_service_name: &str, constructor : F) 
    where F: Fn () -> Box<dyn kRPCHandler + Send + Sync> + 'static + Send + Sync,
{
    let mut inner_service_builder = INNER_SERVICES_BUILDERS.lock().await;
    inner_service_builder.insert(inner_service_name.to_string(), Arc::new(constructor));

}

pub struct Router {
    config: WarpServerConfig,
    inner_service: OnceCell<Box<dyn kRPCHandler + Send + Sync> >,

}

impl Router {
    pub fn new(config: WarpServerConfig) -> Self {
        Router { 
            config,
            inner_service: OnceCell::new(),
        }
    }



    pub async fn route(
        &self,
        req: Request<Body>,
        client_ip:SocketAddr,
    ) -> Result<Response<Body>> {
        let mut host = req
            .headers()
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default()
            .to_string();

        if host.len() > 1 {
            let result = host.split_once(':');
            if result.is_some() {
                host = result.unwrap().0.to_string();
            } 
        } 
        let req_path = req.uri().path();
        info!("{}==>warp recv_req: {} {:?}",client_ip,req_path,req.headers());

        let host_config = self.config.hosts.get(&host).or_else(|| self.config.hosts.get("*"));
        if host_config.is_none() {
            warn!("Route Config not found: {}", host);
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Route not found"))?);
        }

        let host_config = host_config.unwrap();
        debug!("host_config: {:?}", host_config);

        let mut route_path = String::new();
        let route_config = host_config
            .routes
            .iter()
            .find(|(route, _)| {
                route_path = (*route).clone();
                return req_path.starts_with(*route);
            })
            .map(|(_, config)| config);

        if route_config.is_none() {
            warn!("Route Config not found: {}", req_path);
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Route not found"))?);
        }

        let route_config = route_config.unwrap();   
        info!("route_config: {:?}",route_config);

        match route_config {
            RouteConfig {
                upstream: Some(upstream),
                ..
            } => self.handle_upstream(req, upstream.as_str()).await,
            RouteConfig {
                local_dir: Some(local_dir),
                ..
            } => self.handle_local_dir(req, local_dir.as_str(),route_path.as_str()).await,
            RouteConfig {
                inner_service: Some(inner_service),
                ..
            } => self.handle_inner_service(req, inner_service.as_str()).await,
            _ => Err(anyhow::anyhow!("Invalid route configuration")),
        }
    }

    async fn handle_inner_service(&self, req: Request<Body>, inner_service_name: &str) -> Result<Response<Body>> {
        let inner_service = self.inner_service.get();
        let true_service;
        if inner_service.is_none() {
            let inner_service_builder_map = INNER_SERVICES_BUILDERS.lock().await;
            let inner_service_builder = inner_service_builder_map.get(inner_service_name);
            let inner_service_builder = inner_service_builder.unwrap();
            let inner_service = inner_service_builder();
            let _ =self.inner_service.set(inner_service);
            true_service = self.inner_service.get().unwrap();   
        } else {
            true_service = inner_service.unwrap();
        }
        
        let body_bytes = hyper::body::to_bytes(req.into_body()).await.map_err(|e| {
            anyhow::anyhow!("Failed to read body: {}", e)
        })?;

        //parse req to RPCRequest
        let rpc_request: RPCRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
            anyhow::anyhow!("Failed to parse request body to RPCRequest: {}", e)
        })?;

        let resp = true_service.handle_rpc_call(rpc_request).await?;
        //parse resp to Response<Body>
        Ok(Response::new(Body::from(serde_json::to_string(&resp)?)))
    }

    async fn handle_upstream(&self, req: Request<Body>, upstream: &str) -> Result<Response<Body>> {
        let url = format!("{}{}", upstream, req.uri().path_and_query().map_or("", |x| x.as_str()));
        let upstream_url = Url::parse(upstream);
        if upstream_url.is_err() {
            return Err(anyhow::anyhow!("Failed to parse upstream url: {}", upstream_url.err().unwrap()));
        }
        let upstream_url = upstream_url.unwrap();
        let scheme = upstream_url.scheme();
        match scheme {
            "tcp"|"http"|"https" => {
                let client = Client::new();
                let header = req.headers().clone();
                let mut upstream_req = Request::builder()
                    .method(req.method())
                    .uri(&url)
                    .body(req.into_body())?;
        
                *upstream_req.headers_mut() = header;
        
                let resp = client.request(upstream_req).await?;
                return Ok(resp)
            },
            _ => {
                let tunnel_connector = TunnelConnector;
                let client: Client<TunnelConnector, Body> = Client::builder()
                    .build::<_, hyper::Body>(tunnel_connector);

                let header = req.headers().clone();
                let mut upstream_req = Request::builder()
                .method(req.method())
                .uri(&url)
                .body(req.into_body())?;

                *upstream_req.headers_mut() = header;
                let resp = client.request(upstream_req).await?;
                return Ok(resp)
            }
        }
 
    }

    async fn handle_local_dir(&self, req: Request<Body>, local_dir: &str, route_path: &str) -> Result<Response<Body>> {
        let path = req.uri().path();
        let sub_path = path.trim_start_matches(route_path);
        let file_path = format!("{}{}", local_dir, sub_path);
        let path = Path::new(&file_path);

        if path.is_file() {
            let file = match tokio::fs::File::open(&path).await {
                Ok(file) => file,
                Err(_) => return Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("File not found"))?),
            };
            let mime_type = mime_guess::from_path(&file_path).first_or_octet_stream();
            let stream = tokio_util::io::ReaderStream::new(file);
            let body = Body::wrap_stream(stream);

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime_type.as_ref())
                .body(body)?)
        } else {
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("File not found"))?)
        }
    }
}

pub struct SNIResolver {
    configs: HashMap<String, Arc<ServerConfig>>,
}

impl SNIResolver {
    pub fn new(configs: HashMap<String, Arc<ServerConfig>>) -> Self {
        SNIResolver { configs }
    }
}

impl rustls::server::ResolvesServerCert for SNIResolver {
    fn resolve(&self, client_hello: rustls::server::ClientHello) -> Option<Arc<rustls::sign::CertifiedKey>> {
        unimplemented!()
    }
}
