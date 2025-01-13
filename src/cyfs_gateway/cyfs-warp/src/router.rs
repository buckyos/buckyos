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
use std::io::SeekFrom;
use std::sync::RwLock;
use tokio::{
    fs::{self, File,OpenOptions},
    io::{self, AsyncRead,AsyncWrite, AsyncReadExt, AsyncWriteExt, AsyncSeek, AsyncSeekExt},
};
use std::{net::SocketAddr, sync::Arc};
use std::path::Path;
use std::collections::HashMap;
use cyfs_gateway_lib::*;
use tokio::sync::{Mutex, OnceCell};
use serde_json::json;
use ::kRPC::*;
use lazy_static::lazy_static;
use std::io::BufReader;
use rustls_pemfile::{certs, pkcs8_private_keys};
use cyfs_sn::*;
use ndn_lib::*;

use crate::ndn_router::*;
use crate::*;

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
    inner: Arc<RouterInner>,
}

impl Clone for Router {
    fn clone(&self) -> Self {
        Router {
            inner: self.inner.clone(),
        }
    }
}

struct RouterInner {
    hosts: RwLock<HashMap<String, HashMap<String, Arc<RouteConfig>> >>,
    inner_service: OnceCell<Box<dyn kRPCHandler + Send + Sync> >,
}

impl Router {
    pub fn new(hosts: HashMap<String, HashMap<String, Arc<RouteConfig>>>) -> Self {
        Router {
            inner: Arc::new(RouterInner {
                hosts: RwLock::new(hosts),
                inner_service: OnceCell::new(),
            })
        }
    }


    fn get_route_config(&self, host:&str, path:&str) -> Option<(String, Arc<RouteConfig>)> {
        let hosts = self.inner.hosts.read().unwrap();
        let host_config = {
            let host_config = hosts.get(host);
            if host_config.is_some() {
                host_config
            } else {
                let mut host_config =  hosts.get("*");
                for (key,value) in hosts.iter() {
                    if key.starts_with("*.") {
                        if host.ends_with(&key[2..]) {
                            host_config = Some(value);
                            break;
                        }
                    }
        
                    if key.ends_with(".*") {
                        if host.starts_with(&key[..key.len()-2]) {
                            host_config = Some(value);
                            break;
                        }
                    }
                }
                host_config
            }
        };


        if host_config.is_none() {
            return None;
        }

        let host_config = host_config.unwrap();
        debug!("host_config: {:?}", host_config);

        host_config
            .iter()
            .filter(|(route, _)| {
                path.starts_with(*route)
            })
            .max_by_key(|(route, _)| route.len())
            .map(|(route, config)| (route.clone(), config.clone()))
    }

    pub fn insert_route_config(&self, host:&str, path:&str, config: RouteConfig) -> Option<Arc<RouteConfig>> {
        let mut hosts = self.inner.hosts.write().unwrap();
        let host_config = hosts.entry(host.to_string()).or_insert(HashMap::new());
        host_config.insert(path.to_string(), Arc::new(config))
    }

    pub fn remove_route_config(&self, host:&str, path:&str) -> Option<Arc<RouteConfig>> {
        let mut hosts = self.inner.hosts.write().unwrap();
        let host_config = hosts.entry(host.to_string()).or_insert(HashMap::new());
        host_config.remove(path)
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

        let route_config = self.get_route_config(host.as_str(), req_path);
        if route_config.is_none() {
            warn!("Route Config not found: {}", host);
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Route not found"))?);
        }

        let (route_path, route_config) = route_config.unwrap();
        debug!("route_config: {:?}", route_config);

        match &*route_config {
            RouteConfig {
                response: Some(response),
                ..
            } => {
                let mut builder = Response::builder()
                    .status(response.status.unwrap_or(200));
                if let Some(headers) = &response.headers {
                    for (key, value) in headers.iter() {
                        builder = builder.header(key, value);
                    }
                }
                let body = response.body.clone().unwrap_or_default();
                let resp = builder.body(Body::from(body))?;
                return Ok(resp);
            }
            RouteConfig {
                upstream: Some(upstream),
                ..
            } => self.handle_upstream(req, upstream).await,
            RouteConfig {
                local_dir: Some(local_dir),
                ..
            } => self.handle_local_dir(req, local_dir.as_str(),route_path.as_str()).await,
            RouteConfig {
                inner_service: Some(inner_service),
                ..
            } => {
                if route_config.enable_cors && req.method() == hyper::Method::OPTIONS {
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
            RouteConfig {
                named_mgr: Some(named_mgr),
                ..
            } => handle_ndn(named_mgr, req, &host,  client_ip,route_path.as_str()).await,
            _ => Err(anyhow::anyhow!("Invalid route configuration")),
        }.map(|mut resp| {
            if route_config.enable_cors {
                let header = resp.headers_mut();
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Content-Type, Authorization"));
            }
            resp
        })
    }



    async fn handle_inner_service(&self, inner_service_name: &str, req: Request<Body>, client_ip:IpAddr) -> Result<Response<Body>> {
        let inner_service = self.inner.inner_service.get();
        let true_service;
        if inner_service.is_none() {
            let inner_service_builder_map = INNER_SERVICES_BUILDERS.lock().await;
            let inner_service_builder = inner_service_builder_map.get(inner_service_name);
            let inner_service_builder = inner_service_builder.unwrap();
            let inner_service = inner_service_builder();
            let _ =self.inner.inner_service.set(inner_service);
            true_service = self.inner.inner_service.get().unwrap();
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
                return self.handle_upstream(req, &UpstreamRouteConfig{target:tunnel_url, redirect:RedirectType::None}).await;
            }
        } else {
            warn!("No sn server found for selector: {}",selector_id);
        }

        return Err(anyhow::anyhow!("No tunnel selected"));
    }

    async fn handle_upstream(&self, req: Request<Body>, upstream: &str) -> Result<Response<Body>> {
        let org_url = req.uri().to_string();
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
                match &upstream.redirect {
                    RedirectType::None => {
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
                    RedirectType::Permanent => {
                        let resp = Response::builder()
                            .status(StatusCode::PERMANENT_REDIRECT)
                            .header(hyper::header::LOCATION, url)
                            .body(Body::empty())?;
                        return Ok(resp);
                    },
                    RedirectType::Temporary => {
                        let resp = Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header(hyper::header::LOCATION, url)
                        .body(Body::empty())?;
                        return Ok(resp);
                    }
                }
            },
            _ => {
                let tunnel_connector = TunnelConnector;
                let client: Client<TunnelConnector, Body> = Client::builder()
                    .build::<_, hyper::Body>(tunnel_connector);

                let header = req.headers().clone();
                let mut upstream_req = Request::builder()
                .method(req.method())
                .uri(&org_url)
                .body(req.into_body())?;

                *upstream_req.headers_mut() = header;
                let resp = client.request(upstream_req).await?;
                return Ok(resp)
            }
        }

    }

    async fn handle_local_dir(&self, req: Request<Body>, local_dir: &str, route_path: &str) -> Result<Response<Body>> {
        let path = req.uri().path();
        let sub_path = buckyos_kit::get_relative_path(route_path, path);
        let file_path = if sub_path.starts_with("/") {
            Path::new(local_dir).join(&sub_path[1..])
        } else {
            Path::new(local_dir).join(&sub_path)
        };
        info!("handle_local_dir will load file:{}", file_path.to_string_lossy().to_string());
        let path = file_path.as_path();

        if path.is_file() {
            let file = match tokio::fs::File::open(&path).await {
                Ok(file) => file,
                Err(_) => return Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("File not found"))?),
            };

            let file_meta = file.metadata().await.map_err(|e| {
                warn!("Failed to get file metadata: {}", e);
                anyhow::anyhow!("Failed to get file metadata: {}", e)
            })?;
            let file_size = file_meta.len();
            let mime_type = mime_guess::from_path(&file_path).first_or_octet_stream();

            // 处理Range请求
            if let Some(range_header) = req.headers().get(hyper::header::RANGE) {
                if let Ok(range_str) = range_header.to_str() {
                    if let Ok((start, end)) = parse_range(range_str, file_size) {
                        let mut file = tokio::io::BufReader::new(file);
                        // 设置读取位置
                        tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(start)).await?;

                        let content_length = end - start + 1;
                        let stream = tokio_util::io::ReaderStream::with_capacity(
                            file.take(content_length),
                            content_length as usize
                        );

                        return Ok(Response::builder()
                            .status(StatusCode::PARTIAL_CONTENT)
                            .header("Content-Type", mime_type.as_ref())
                            .header("Content-Length", content_length)
                            .header("Content-Range", format!("bytes {}-{}/{}", start, end, file_size))
                            .header("Accept-Ranges", "bytes")
                            .body(Body::wrap_stream(stream))?);
                    }
                }
            }

            // 非Range请求返回完整文件
            let stream = tokio_util::io::ReaderStream::with_capacity(file, file_size as usize);

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime_type.as_ref())
                .header("Content-Length", file_size)
                .header("Accept-Ranges", "bytes")
                .body(Body::wrap_stream(stream))?)
        } else {
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("File not found"))?)
        }
    }
}

