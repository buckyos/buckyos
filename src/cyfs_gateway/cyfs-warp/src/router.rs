// src/router.rs
#![allow(dead_code)]
#![allow(unused)]

use anyhow::Result;
use hyper::header::HeaderValue;
use hyper::{Body, Client, Request, Response, StatusCode};
use log::*;
use rustls::ServerConfig;
use url::Url;
use std::net::IpAddr;
use std::{net::SocketAddr, sync::Arc};
use std::path::Path;
use std::collections::HashMap;
use cyfs_gateway_lib::*;
use tokio::sync::{Mutex, OnceCell};
use serde_json::json;
use ::kRPC::*;
use lazy_static::lazy_static;
use std::fs::File;
use std::io::BufReader;
use rustls_pemfile::{certs, pkcs8_private_keys};
use cyfs_sn::*;

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

    fn get_config_by_host(&self,host:&str) -> Option<&HostConfig> {
        let host_config = self.config.hosts.get(host);
        if host_config.is_some() {
            return host_config;
        }

        for (key,value) in self.config.hosts.iter() {
            if key.starts_with("*.") {
                if host.ends_with(&key[2..]) {
                    return Some(value);
                }
            }

            if key.ends_with(".*") {
                if host.starts_with(&key[..key.len()-2]) {
                    return Some(value);
                }
            }
        }

        return self.config.hosts.get("*");
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
        let client_ip = client_ip.ip();
        info!("{}==>warp recv_req: {} {:?}",client_ip.to_string(),req_path,req.headers());

        let host_config = self.get_config_by_host(host.as_str());
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
            .filter(|(route, _)| {
                route_path = (*route).clone();
                return req_path.starts_with(*route);
            })
            .max_by_key(|(route, _)| route.len())
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
            } => {
                if host_config.enable_cors && req.method() == hyper::Method::OPTIONS {
                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::empty())?)
                } else {
                    self.handle_inner_service(inner_service.as_str(),req,client_ip).await
                }
            },
            RouteConfig {
                tunnel_selector: Some(tunnel_selector),
                ..
            } => self.handle_upstream_selector(tunnel_selector.as_str(), req, &host,  client_ip).await,
            _ => Err(anyhow::anyhow!("Invalid route configuration")),
        }.map(|mut resp| {
            if host_config.enable_cors {
                let header = resp.headers_mut();
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Content-Type, Authorization"));
            }
            resp
        })
    }

    async fn handle_inner_service(&self, inner_service_name: &str, req: Request<Body>, client_ip:IpAddr) -> Result<Response<Body>> {
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

        let body_str = String::from_utf8(body_bytes.to_vec()).map_err(|e| {
            anyhow::anyhow!("Failed to convert body to string: {}", e)
        })?;

        info!("|==>recv kRPC req: {}",body_str);

        //parse req to RPCRequest
        let rpc_request: RPCRequest = serde_json::from_str(body_str.as_str()).map_err(|e| {
            anyhow::anyhow!("Failed to parse request body to RPCRequest: {}", e)
        })?;

        let resp = true_service.handle_rpc_call(rpc_request,client_ip).await?;
        //parse resp to Response<Body>
        Ok(Response::new(Body::from(serde_json::to_string(&resp)?)))
    }

    async fn handle_upstream_selector(&self, selector_id:&str,req: Request<Body>,host:&str, client_ip:IpAddr) -> Result<Response<Body>> {
        //in early stage, only support sn server id
        let sn_server = get_sn_server_by_id(selector_id).await;
        if sn_server.is_some() {
            let sn_server = sn_server.unwrap();
            let req_path = req.uri().path();
            let tunnel_url = sn_server.select_tunnel_for_http_upstream(host,req_path).await;
            if tunnel_url.is_some() {
                let tunnel_url   = tunnel_url.unwrap();
                info!("select tunnel: {}",tunnel_url.as_str());
                return self.handle_upstream(req, tunnel_url.as_str()).await;
            }
        } else {
            warn!("No sn server found for selector: {}",selector_id);
        }

        return Err(anyhow::anyhow!("No tunnel selected"));
    }

    async fn handle_upstream(&self, req: Request<Body>, upstream: &str) -> Result<Response<Body>> {
        let url = format!("{}{}", upstream, req.uri().path_and_query().map_or("", |x| x.as_str()));
        let upstream_url = Url::parse(upstream);
        if upstream_url.is_err() {
            return Err(anyhow::anyhow!("Failed to parse upstream url: {}", upstream_url.err().unwrap()));
        }
        //TODO:support url rewrite
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
        let file_path = format!("{}/{}", local_dir, sub_path);
        info!("handle_local_dir will load file:{}",file_path);
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
    default_tls_host: String,
}

impl SNIResolver {
    pub fn new(configs: HashMap<String, Arc<ServerConfig>>,default_tls_host:String) -> Self {
        SNIResolver { configs,default_tls_host }
    }

    
    fn get_config_by_host(&self,host:&str) -> Option<&Arc<ServerConfig>> {
        let host_config = self.configs.get(host);
        if host_config.is_some() {
            return host_config;
        }

        for (key,value) in self.configs.iter() {
            if key.starts_with("*.") {
                if host.ends_with(&key[2..]) {
                    return Some(value);
                }
            }
        }

        return self.configs.get("*");
    }
}

impl rustls::server::ResolvesServerCert for SNIResolver {
    fn resolve(&self, client_hello: rustls::server::ClientHello) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let server_name = client_hello.server_name().unwrap_or(self.default_tls_host.as_str()).to_string();
        info!("try reslove tls certifiled key for : {}", server_name);

        let config = self.get_config_by_host(&server_name);
        if config.is_some() {
            return config.unwrap().cert_resolver.resolve(client_hello);
        } else {
            warn!("No tls config found for server_name: {}", server_name);
            return None;
        }
    }
}
