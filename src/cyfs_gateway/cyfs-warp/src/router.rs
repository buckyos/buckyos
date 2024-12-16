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
            RouteConfig {
                named_mgr: Some(named_mgr),
                ..
            } => self.handle_ndn(named_mgr, req, &host,  client_ip,route_path.as_str()).await,
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

    async fn handle_ndn(&self, mgr_config: &NamedDataMgrRouteConfig, req: Request<Body>, host: &str, client_ip:IpAddr,route_path: &str) -> Result<Response<Body>> {
        if req.method() != hyper::Method::GET {
            return Err(anyhow::anyhow!("Invalid method: {}", req.method()));
        }

        let named_mgr_id = mgr_config.named_data_mgr_id.clone();
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some(named_mgr_id.as_str())).await;
        if named_mgr.is_none() {
            warn!("Named manager not found: {}", named_mgr_id);
            return Err(anyhow::anyhow!("Named manager not found: {}", named_mgr_id));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
                
        let range_str = req.headers().get(hyper::header::RANGE);
        let mut start = 0;
        let mut chunk_size = 0;
        if range_str.is_some() {
            let range_str = range_str.unwrap().to_str().unwrap();
            (start,_) = parse_range(range_str,u64::MAX)
                .map_err(|e| {
                    warn!("parse range failed: {}", e);
                    anyhow::anyhow!("parse range failed: {}", e)
                })?;
        }

        let chunk_id_result;
        let chunk_id:ChunkId;
        let path = req.uri().path();
        let user_id = "guest";
        let app_id = "unknown";
        let mut chunk_reader;
    
        if mgr_config.is_chunk_id_in_path {
            //let sub_path = path.trim_start_matches(path);
            chunk_id_result = ChunkId::from_url_path(path);
        } else {
            //get chunkid by hostname
            chunk_id_result = ChunkId::from_hostname(host);
        }
        
        if chunk_id_result.is_err() {
            if mgr_config.enable_mgr_file_path {
                let sub_path = buckyos_kit::get_relative_path(route_path, path);
                let seek_from = SeekFrom::Start(start);
                (chunk_reader,chunk_size,chunk_id) = named_mgr.get_chunk_reader_by_path(sub_path, user_id, app_id, seek_from).await
                    .map_err(|e| {
                        warn!("get chunk reader by path failed: {}", e);
                        anyhow::anyhow!("get chunk reader by path failed: {}", e)
                    })?;
            } else {
                return Err(anyhow::anyhow!("failed to get chunk id from request!"));
            }
        } else {
            chunk_id = chunk_id_result.unwrap();
            let get_result = named_mgr.get_chunk_reader(&chunk_id, SeekFrom::Start(start), true).await;
            if get_result.is_err() {
                warn!("get chunk reader by chunkid:{} failed: {}",chunk_id.to_string(),get_result.err().unwrap());
                return Err(anyhow::anyhow!("get chunk reader by chunkid:{} failed.",chunk_id.to_string()));
            }
            (chunk_reader,chunk_size) = get_result.unwrap();
            info!("get chunk reader by chunkid:{} OK",chunk_id.to_string());
        }
        drop(named_mgr);
        //TODO:更合理的得到mime_type
        let mime_type = "application/octet-stream";
        let mut result = Response::builder()
            .header("Content-Type", mime_type)
            .header("Accept-Ranges", "bytes")
            .header("Cache-Control", "public,max-age=31536000")
            .header("cyfs-obj-id", chunk_id.to_string())
            .header("cyfs-data-size", chunk_size.to_string());

        if start > 0 {
            result = result.header("Content-Range", format!("bytes {}-{}/{}", start, chunk_size - 1, chunk_size))
            .header("Content-Length", chunk_size - start)
            .status(StatusCode::PARTIAL_CONTENT);
        } else {          
            result = result.header("Content-Length", chunk_size)
            .status(StatusCode::OK);
        }

        //let stream = tokio_util::io::ReaderStream::with_capacity(chunk_reader, chunk_size as usize);
        let stream = tokio_util::io::ReaderStream::new(chunk_reader);
        let body_result = result.body(Body::wrap_stream(stream))?;
        
        Ok(body_result)
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
        let sub_path = buckyos_kit::get_relative_path(route_path, path);
        let file_path = format!("{}/{}", local_dir, sub_path);
        info!("handle_local_dir will load file:{}", file_path);
        let path = Path::new(&file_path);

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

// 辅助函数：解析Range header
fn parse_range(range: &str, file_size: u64) -> Result<(u64, u64)> {
    // 解析 "bytes=start-end" 格式
    let range = range.trim_start_matches("bytes=");
    let mut parts = range.split('-');
    
    let start = parts.next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
        
    let end = parts.next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(file_size - 1);

    // 验证范围有效性
    if start >= file_size || end >= file_size || start > end {
        return Err(anyhow::anyhow!("Invalid range"));
    }

    Ok((start, end))
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
            info!("find tls config for host: {}",host);
            return host_config;
        }

        for (key,value) in self.configs.iter() {
            if key.starts_with("*.") {
                if host.ends_with(&key[2..]) {
                    info!("find tls config for host: {} ==> key:{}",host,key);
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
