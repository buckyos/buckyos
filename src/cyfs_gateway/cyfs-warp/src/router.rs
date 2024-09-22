// src/router.rs
use crate::config::{Config, RouteConfig};
use anyhow::Result;
use hyper::{header, Body, Request, Response, StatusCode};
use log::*;
use rustls::ServerConfig;
use std::sync::Arc;
use tokio::fs;
use url::Url;
use std::collections::HashMap;


pub struct Router {
    config: Config,
}

impl Router {
    pub fn new(config: Config) -> Self {
        Router { config }
    }

    pub async fn route(
        &self,
        req: Request<Body>,
        tls_config: Option<Arc<ServerConfig>>,
    ) -> Result<Response<Body>> {
        let host = req
            .headers()
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default()
            .to_string();

        let host_config = self.config.hosts.get(&host).ok_or_else(|| {
            anyhow::anyhow!("Host not found in configuration: {}", host)
        })?;

        let path = req.uri().path();
        let route_config = host_config
            .routes
            .iter()
            .find(|(route, _)| path.starts_with(*route))
            .map(|(_, config)| config);

        if route_config.is_none() {
            warn!("Route Config not found: {}", path);
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Route not found"))?);
        }

        let route_config = route_config.unwrap();   

        match route_config {
            RouteConfig {
                upstream: Some(upstream),
                ..
            } => self.handle_upstream(req, upstream).await,
            RouteConfig {
                local_dir: Some(local_dir),
                ..
            } => self.handle_local_dir(req, local_dir).await,
            _ => Err(anyhow::anyhow!("Invalid route configuration")),
        }
    }

    async fn handle_upstream(&self, req: Request<Body>, upstream: &str) -> Result<Response<Body>> {
        let url = format!("{}{}", upstream, req.uri().path_and_query().map_or("", |x| x.as_str()));
        let client = hyper::Client::new();
        let header = req.headers().clone();
        let mut upstream_req = Request::builder()
            .method(req.method())
            .uri(&url)
            .body(req.into_body())?;

        *upstream_req.headers_mut() = header;

        let resp = client.request(upstream_req).await?;
        Ok(resp)
    }

    async fn handle_local_dir(&self, req: Request<Body>, local_dir: &str) -> Result<Response<Body>> {
        let path = req.uri().path();
        let file_path = format!("{}{}", local_dir, path);

        if let Ok(contents) = fs::read(&file_path).await {
            let mime_type = mime_guess::from_path(&file_path).first_or_octet_stream();
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime_type.as_ref())
                .body(Body::from(contents))?)
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