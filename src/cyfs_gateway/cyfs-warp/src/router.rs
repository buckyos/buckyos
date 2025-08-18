// src/router.rs
#![allow(dead_code)]
#![allow(unused)]

use thiserror::Error;
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
    static ref INNER_SERVICES_BUILDERS: Arc<Mutex< HashMap<String, Arc<dyn Fn () -> Box<dyn InnerServiceHandler + Send + Sync>+ Send + Sync>>>> = Arc::new(Mutex::new(HashMap::new()));
}

#[derive(Error, Debug)]
pub enum RouterError {
    #[error("400 Bad Request: {0}")]
    BadRequest(String),
    #[error("401 Unauthorized: {0}")]
    Unauthorized(String),
    #[error("403 Forbidden: {0}")]
    Forbidden(String),
    #[error("404 Not found: {0}")]
    NotFound(String),
    #[error("429 Too Many Requests: {0}")]
    TooManyRequests(String),
    #[error("500 Internal Error: {0}")]
    Internal(String),
    #[error("502 Bad Gateway: {0}")]
    BadGateway(String),
    #[error("503 Service Unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("504 Gateway Timeout: {0}")]
    GatewayTimeout(String),
}

impl RouterError {
    pub fn build_response(&self)->Response<Body> {
        Response::builder()
            .status(self.status_code())
            .body(Body::empty())
            .unwrap()
    }

    pub fn status_code(&self)->StatusCode {
        match self {
            RouterError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            RouterError::BadGateway(_) => StatusCode::BAD_GATEWAY,
            RouterError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            RouterError::GatewayTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
            RouterError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            RouterError::NotFound(_) => StatusCode::NOT_FOUND,
            RouterError::BadRequest(_) => StatusCode::BAD_REQUEST,
            RouterError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            RouterError::Forbidden(_) => StatusCode::FORBIDDEN,
        }
    }
}

pub type RouterResult<T> = std::result::Result<T, RouterError>;

pub async fn register_inner_service_builder<F>(inner_service_name: &str, constructor : F)
    where F: Fn () -> Box<dyn InnerServiceHandler + Send + Sync> + 'static + Send + Sync,
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
    inner_service: OnceCell<Box<dyn InnerServiceHandler + Send + Sync> >,
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
                        if host.ends_with(&key[1..]) {
                            host_config = Some(value);
                            break;
                        }
                    }
                    
                    //appid.* 这种格式，通过SN转发的时候 appid.username.web3.buckyos.io 可以转发到特定appid的upstream
                    if key.ends_with(".*") {
                        if host.starts_with(&key[..key.len()-1]) {
                            host_config = Some(value);
                            break;
                        }
                    }
                    //appid-* 这种格式， 通过SN转发的时候  appid-username.web3.buckyos.io 可以转移到特定appid的upstream
                    if key.ends_with("*") {
                        if host.starts_with(&key[..key.len()-1]) {
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
    ) -> RouterResult<Response<Body>> { 
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
        let req_method = req.method();
        let client_ip = client_ip.ip();
        info!("{}==> {} {},{:?}",client_ip.to_string(),req_method,req_path,req.headers());

        let route_config = self.get_route_config(host.as_str(), req_path);
        if route_config.is_none() {
            warn!("Route Config not found: {}", host);
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Route not found")).unwrap());
        }

        let (route_path, route_config) = route_config.unwrap();
        debug!("route_config: {:?}", route_config);

        let real_resp = match &*route_config {
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
                let resp = builder.body(Body::from(body)).unwrap();
                Ok(resp)
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
                        .body(Body::empty()).unwrap())
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
            _ => Err(RouterError::BadGateway("Invalid route configuration".to_string())),
        }.map(|mut resp| {
            if route_config.enable_cors {
                //info!("enable cors for route: {}",route_path);
                let header = resp.headers_mut();
                
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
                header.insert(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Content-Type, Authorization"));
            }
            resp
        });

        if real_resp.is_err() {
            let err_msg = real_resp.as_ref().unwrap_err().to_string();
            error!("{} <==| {}",client_ip.to_string(),err_msg);
            
        } else {
            info!("{} <==| {}",client_ip.to_string(),real_resp.as_ref().unwrap().status());
        }

        return real_resp;
    }



    async fn handle_inner_service(&self, inner_service_name: &str, req: Request<Body>, client_ip:IpAddr) -> RouterResult<Response<Body>> {
        let inner_service = self.inner.inner_service.get();
        let true_service;
        if inner_service.is_none() {
            let inner_service_builder_map = INNER_SERVICES_BUILDERS.lock().await;
            let inner_service_builder = inner_service_builder_map.get(inner_service_name);
            if inner_service_builder.is_none() {
                return Err(RouterError::BadGateway(format!("Inner service not found: {}", inner_service_name)));
            }

            let inner_service_builder = inner_service_builder.unwrap();
            let inner_service = inner_service_builder();
            let _ =self.inner.inner_service.set(inner_service);
            true_service = self.inner.inner_service.get().unwrap();
        } else {
            true_service = inner_service.unwrap();
        }

        //先判断请求的类型，有2种，1种是标准的krpc请求，另一种是标准的HTTP RESETful API请求
        let method = req.method();
        match *method {
            hyper::Method::POST => {
                let body_bytes = hyper::body::to_bytes(req.into_body()).await.map_err(|e| {
                    RouterError::BadRequest(format!("Failed to read body: {}", e))
                })?;

                let body_str = String::from_utf8(body_bytes.to_vec()).map_err(|e| {
                    RouterError::BadRequest(format!("Failed to convert body to string: {}", e))
                })?;

                info!("|==>recv kRPC req: {}",body_str);

                //parse req to RPCRequest
                let rpc_request: RPCRequest = serde_json::from_str(body_str.as_str()).map_err(|e| {
                    RouterError::BadRequest(format!("Failed to parse request body to RPCRequest: {}", e))
                })?;

                let resp = true_service.handle_rpc_call(rpc_request,client_ip).await.map_err(|e| {
                    RouterError::Internal(format!("Failed to handle rpc call: {}", e))
                })?;

                //parse resp to Response<Body>
                Ok(Response::new(Body::from(serde_json::to_string(&resp).map_err(|e| {
                    RouterError::Internal(format!("Failed to convert response to string: {}", e))
                })?)))
            }
            hyper::Method::GET => {
                let uri = req.uri().to_string();
                let resp = true_service.handle_http_get(uri.as_str(),client_ip).await;
                //TODO: RPCError to RouterError
                if resp.is_err() {
                    return Err(RouterError::Internal(format!("Failed to handle http get: {}", resp.as_ref().unwrap_err())));
                }
                let resp = resp.unwrap();
                Ok(Response::new(Body::from(resp)))
            }
            _ => {
                return Err(RouterError::BadRequest(format!("Not supported request method: {}", req.method())));
            }
        }
        
    }

    async fn handle_upstream_selector(&self, selector_id:&str,req: Request<Body>,host:&str, client_ip:IpAddr) -> RouterResult<Response<Body>> {
        //in early stage, only support sn server id
        let sn_server = get_sn_server_by_id(selector_id).await;
        if sn_server.is_some() {
            let sn_server = sn_server.unwrap();
            let req_path = req.uri().path();
            let tunnel_url = sn_server.select_tunnel_for_http_upstream(host,req_path).await;
            if tunnel_url.is_some() {
                let tunnel_url = tunnel_url.unwrap();
                info!("select tunnel: {}",tunnel_url.as_str());
                return self.handle_upstream(req, &UpstreamRouteConfig{target:tunnel_url, redirect:RedirectType::None}).await;
            }
        } else {
            warn!("No sn server found for selector: {}",selector_id);
        }

        return Err(RouterError::BadGateway("No tunnel selected".to_string()));
    }

    async fn handle_upstream(&self, req: Request<Body>, upstream: &UpstreamRouteConfig) -> RouterResult<Response<Body>> {
        let org_url = req.uri().to_string();
        let url = format!("{}{}", upstream.target, org_url);
        info!("handle_upstream url: {}", url);
        let upstream_url = Url::parse(upstream.target.as_str());
        if upstream_url.is_err() {
            return Err(RouterError::BadGateway(format!("Failed to parse upstream url: {}", upstream_url.err().unwrap())));
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
                        .body(req.into_body()).map_err(|e| {
                            RouterError::Internal(format!("Failed to build request: {}", e))
                        })?;

                        *upstream_req.headers_mut() = header;
                    
                        let resp = client.request(upstream_req).await.map_err(|e| {
                            RouterError::Internal(format!("Failed to request upstream: {}", e))
                        })?;
                        return Ok(resp)
                    }, 
                    RedirectType::Permanent => {
                        let resp = Response::builder()
                            .status(StatusCode::PERMANENT_REDIRECT)
                            .header(hyper::header::LOCATION, url)
                            .body(Body::empty()).unwrap();
                        return Ok(resp);
                    },
                    RedirectType::Temporary => {
                        let resp = Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header(hyper::header::LOCATION, url)
                        .body(Body::empty()).unwrap();
                        return Ok(resp);
                    }
                }
            },
            _ => {
                let tunnel_connector = TunnelConnector {
                    target_stream_url: upstream.target.clone(),
                };

                let client: Client<TunnelConnector, Body> = Client::builder()
                    .build::<_, hyper::Body>(tunnel_connector);

                let header = req.headers().clone();
                let mut host_name = "127.0.0.1".to_string();
                let hname =  req.headers().get("host");
                if hname.is_some() {
                    host_name = hname.unwrap().to_str().unwrap().to_string();
                }
                let fake_url = format!("http://{}{}", host_name, org_url);
                let mut upstream_req = Request::builder()
                    .method(req.method())
                    .uri(fake_url)
                    .body(req.into_body()).map_err(|e| {
                        RouterError::BadGateway(format!("Failed to build upstream_req: {}", e))
                    })?;

                *upstream_req.headers_mut() = header;
                let resp = client.request(upstream_req).await.map_err(|e| {
                    RouterError::Internal(format!("Failed to request upstream: {}", e))
                })?;
                return Ok(resp)
            }
        }

    }

    async fn handle_local_dir(&self, req: Request<Body>, local_dir: &str, route_path: &str) -> RouterResult<Response<Body>> {
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
            let file = tokio::fs::File::open(&path).await.map_err(|e| {
                warn!("Failed to open file: {}", e);
                RouterError::Internal(format!("Failed to open file: {}", e))
            })?;

            let file_meta = file.metadata().await.map_err(|e| {
                warn!("Failed to get file metadata: {}", e);
                RouterError::Internal(format!("Failed to get file metadata: {}", e))
            })?;
            let file_size = file_meta.len();
            let mime_type = mime_guess::from_path(&file_path).first_or_octet_stream();

            // 处理Range请求
            if let Some(range_header) = req.headers().get(hyper::header::RANGE) {
                if let Ok(range_str) = range_header.to_str() {
                    if let Ok((start, end)) = parse_range(range_str, file_size) {
                        let mut file = tokio::io::BufReader::new(file);
                        // 设置读取位置
                        tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(start)).await.map_err(|e| {
                            RouterError::Internal(format!("Failed to seek file: {}", e))
                        })?;

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
                            .body(Body::wrap_stream(stream)).map_err(|e| {
                                RouterError::Internal(format!("Failed to build response: {}", e))
                            })?);
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
                .body(Body::wrap_stream(stream)).map_err(|e| {
                    RouterError::Internal(format!("Failed to build response: {}", e))
                })?)
        } else {
            return Err(RouterError::NotFound(format!("File not found: {}", file_path.to_string_lossy().to_string())));
        }
    }
}

pub struct SNIResolver {
    configs: HashMap<String, ServerConfig>,
}

impl SNIResolver {
    pub fn new(configs: HashMap<String, ServerConfig>) -> Self {
        SNIResolver { configs }
    }

    fn get_config_by_host(&self,host:&str) -> Option<&ServerConfig> {
        let host_config = self.configs.get(host);
        if host_config.is_some() {
            debug!("find tls config for host: {}",host);
            return host_config;
        }

        for (key,value) in self.configs.iter() {
            if key.starts_with("*.") {
                if host.ends_with(&key[2..]) {
                    debug!("find tls config for host: {} ==> key:{}",host,key);
                    return Some(value);
                }
            }
        }

        return self.configs.get("*");
    }
}

impl rustls::server::ResolvesServerCert for SNIResolver {
    fn resolve(&self, client_hello: rustls::server::ClientHello) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let server_name = client_hello.server_name();
        if server_name.is_none() {
            warn!("No server name found in sni-client hello");
            return None;
        }
        let server_name = server_name.unwrap();
        debug!("try reslove tls certifiled key for : {}", server_name);

        let config = self.get_config_by_host(&server_name);
        if config.is_some() {
            return config.unwrap().cert_resolver.resolve(client_hello);
        } else {
            warn!("No tls config found for server_name: {}", server_name);
            return None;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_stream_url() {
        let stream_url = "rtcp://sdSKcMuxU_BtGAvqs729BIBwe5H9Jo3T_wj4GdRgCfE.dev.did/:80/static/index.html";
        let url = Url::parse(stream_url).unwrap();
        println!("url.path: {}", url.path());
        assert_eq!(url.scheme(), "rtcp");
        assert_eq!(url.host_str(), Some("sdSKcMuxU_BtGAvqs729BIBwe5H9Jo3T_wj4GdRgCfE.dev.did"));
        assert_eq!(url.port(),None);
    }
}