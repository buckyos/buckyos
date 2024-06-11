/*
Add/Remove services via http protocol

/service/upstream
/service/proxy/socks5
/service/proxy/forward

POST /service/upstream
{
    "id": "id",
    "protocol": "tcp",
    "addr": "127.0.0.1",
    "port": 2000,
    "type": "tcp"
}

Delete /service/upstream
{
    "id": "id"
}

POST /service/proxy/socks5
{
    "id": "id",
    "addr": "127.0.0.1",
    "port": 2000,
    "type": "socks5"
}

Delete /service/proxy/socks5
{
    "id": "id"
}

POST /service/proxy/forward
{
    "id": "id",
    "addr": "127.0.0.1",
    "port": 2000,
    "protocol": "tcp",
    "target_device": "device_id",
    "target_port": 2000
    "type": "forward"
}

Delete /service/proxy/forward
{
    "id": "id"
}
*/

use reqwest::Client;
use serde::Serialize;
use std::net::IpAddr;

use crate::constants::HTTP_INTERFACE_DEFAULT_PORT;
use crate::def::*;
use crate::error::*;

#[derive(Debug, Serialize)]
pub struct AddUpstreamRequest {
    pub id: String,
    pub protocol: UpstreamServiceProtocol,
    pub addr: IpAddr,
    pub port: u16,
}

#[derive(Debug, Serialize)]
pub struct DeleteUpstreamRequest {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct AddSocks5ProxyRequest {
    pub id: String,
    pub addr: IpAddr,
    pub port: u16,
    pub r#type: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteSocks5ProxyRequest {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct AddForwardProxyRequest {
    pub id: String,
    pub addr: IpAddr,
    pub port: u16,
    pub protocol: ForwardProxyProtocol,
    pub target_device: String,
    pub target_port: u16,
    pub r#type: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteForwardProxyRequest {
    pub id: String,
}

pub struct GatewayStub {
    client: Client,
    base_url: String,
}

impl Default for GatewayStub {
    fn default() -> Self {
        let url = format!("http://127.0.0.1:{}", HTTP_INTERFACE_DEFAULT_PORT);
        Self::new(url)
    }
}

impl GatewayStub {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
        }
    }

    async fn on_response(resp: reqwest::Response) -> GatewayResult<()> {
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = resp.json::<serde_json::Value>().await.map_err(|e| {
                let msg = format!("Error parsing response body: {}", e);
                error!("{}", msg);
                GatewayError::HttpError(msg)
            })?;

            let msg = body.get("msg").map(|v| v.to_string()).unwrap_or_default();
            Err(GatewayError::HttpError(msg))
        }
    }

    pub async fn add_upstream(&self, req: AddUpstreamRequest) -> GatewayResult<()> {
        let url = format!("{}/service/upstream", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("Request to gateway failed: {}, {}", url, e);
                error!("{}", msg);
                GatewayError::HttpError(msg)
            })?;

        Self::on_response(resp).await
    }

    pub async fn delete_upstream(&self, req: DeleteUpstreamRequest) -> GatewayResult<()> {
        let url = format!("{}/service/upstream", self.base_url);
        let resp = self
            .client
            .delete(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("Request to gateway failed: {}, {}", url, e);
                error!("{}", msg);
                GatewayError::HttpError(msg)
            })?;

        Self::on_response(resp).await
    }

    pub async fn add_socks5_proxy(&self, req: AddSocks5ProxyRequest) -> GatewayResult<()> {
        let url = format!("{}/service/proxy/socks5", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("Request to gateway failed: {}, {}", url, e);
                error!("{}", msg);
                GatewayError::HttpError(msg)
            })?;

        Self::on_response(resp).await
    }

    pub async fn delete_socks5_proxy(&self, req: DeleteSocks5ProxyRequest) -> GatewayResult<()> {
        let url = format!("{}/service/proxy/socks5", self.base_url);
        let resp = self
            .client
            .delete(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("Request to gateway failed: {}, {}", url, e);
                error!("{}", msg);
                GatewayError::HttpError(msg)
            })?;

        Self::on_response(resp).await
    }

    pub async fn add_forward_proxy(&self, req: AddForwardProxyRequest) -> GatewayResult<()> {
        let url = format!("{}/service/proxy/forward", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("Request to gateway failed: {}, {}", url, e);
                error!("{}", msg);
                GatewayError::HttpError(msg)
            })?;

        Self::on_response(resp).await
    }

    pub async fn delete_forward_proxy(&self, req: DeleteForwardProxyRequest) -> GatewayResult<()> {
        let url = format!("{}/service/proxy/forward", self.base_url);
        let resp = self
            .client
            .delete(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("Request to gateway failed: {}, {}", url, e);
                error!("{}", msg);
                GatewayError::HttpError(msg)
            })?;

        Self::on_response(resp).await
    }
}
