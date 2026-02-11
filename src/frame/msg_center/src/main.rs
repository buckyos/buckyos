mod contact_mgr;
mod msg_center;
mod msg_tunnel;
#[cfg(test)]
mod test_msg_center;
mod tg_tunnel;

use ::kRPC::*;
use anyhow::{Context, Result};
use buckyos_api::{
    init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType, MsgCenterServerHandler,
    MSG_CENTER_SERVICE_NAME, MSG_CENTER_SERVICE_PORT,
};
use buckyos_kit::init_logging;
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::{error, info, warn};
use name_lib::DID;
use server_runner::Runner;
use std::net::IpAddr;
use std::sync::Arc;

use crate::msg_center::MessageCenter;
use crate::msg_tunnel::{MsgTunnel, MsgTunnelInstanceMgr};
use crate::tg_tunnel::{TgTunnel, TgTunnelConfig};

const MSG_CENTER_HTTP_PATH: &str = "/kapi/msg-center";
const MSG_CENTER_DEFAULT_TG_TUNNEL_DID: &str = "did:bns:msg-center-default-tunnel";
const MSG_CENTER_TG_TUNNEL_DID_ENV_KEY: &str = "BUCKYOS_MSG_CENTER_TG_TUNNEL_DID";
const TG_API_ID_ENV_KEY: &str = "BUCKYOS_TG_API_ID";
const TG_API_HASH_ENV_KEY: &str = "BUCKYOS_TG_API_HASH";

struct MsgCenterHttpServer {
    rpc_handler: MsgCenterServerHandler<MessageCenter>,
    _tunnel_mgr: MsgTunnelInstanceMgr,
}

impl MsgCenterHttpServer {
    fn new(center: MessageCenter, tunnel_mgr: MsgTunnelInstanceMgr) -> Self {
        Self {
            rpc_handler: MsgCenterServerHandler::new(center),
            _tunnel_mgr: tunnel_mgr,
        }
    }
}

#[async_trait::async_trait]
impl RPCHandler for MsgCenterHttpServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        self.rpc_handler.handle_rpc_call(req, ip_from).await
    }
}

#[async_trait::async_trait]
impl HttpServer for MsgCenterHttpServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        MSG_CENTER_SERVICE_NAME.to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

fn resolve_tg_tunnel_did() -> Result<DID> {
    if let Ok(raw_did) = std::env::var(MSG_CENTER_TG_TUNNEL_DID_ENV_KEY) {
        let raw_did = raw_did.trim();
        if !raw_did.is_empty() {
            return DID::from_str(raw_did).map_err(|e| {
                anyhow::anyhow!(
                    "invalid {}={}, err={}",
                    MSG_CENTER_TG_TUNNEL_DID_ENV_KEY,
                    raw_did,
                    e
                )
            });
        }
    }

    DID::from_str(MSG_CENTER_DEFAULT_TG_TUNNEL_DID).map_err(|e| {
        anyhow::anyhow!(
            "invalid default tg tunnel did {}, err={}",
            MSG_CENTER_DEFAULT_TG_TUNNEL_DID,
            e
        )
    })
}

fn should_use_grammers_gateway() -> bool {
    std::env::var(TG_API_ID_ENV_KEY)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        && std::env::var(TG_API_HASH_ENV_KEY)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
}

fn build_tg_tunnel(cfg: TgTunnelConfig) -> Result<TgTunnel> {
    if should_use_grammers_gateway() {
        let tunnel = TgTunnel::with_grammers_gateway_from_env(cfg)
            .context("init telegram tunnel with grammers gateway failed")?;
        info!("telegram tunnel initialized with grammers gateway");
        return Ok(tunnel);
    }

    warn!(
        "{} or {} not set, fallback to dry-run telegram gateway",
        TG_API_ID_ENV_KEY, TG_API_HASH_ENV_KEY
    );
    Ok(TgTunnel::new(cfg))
}

async fn init_tg_tunnel(center: &MessageCenter) -> Result<MsgTunnelInstanceMgr> {
    let tunnel_did = resolve_tg_tunnel_did()?;
    let cfg = TgTunnelConfig::new(tunnel_did.clone());
    let tg_tunnel = Arc::new(build_tg_tunnel(cfg)?);

    tg_tunnel
        .bind_msg_center_handler(Arc::new(center.clone()))
        .context("bind msg_center handler to telegram tunnel failed")?;

    let tunnel_mgr = MsgTunnelInstanceMgr::new();
    tunnel_mgr
        .register(tg_tunnel.clone())
        .map_err(|e| anyhow::anyhow!("register telegram tunnel failed: {}", e))?;

    match tunnel_mgr.start_instance(&tunnel_did).await {
        Ok(_) => {
            info!(
                "telegram tunnel {} started (ingress={}, egress={})",
                tunnel_did.to_string(),
                tg_tunnel.supports_ingress(),
                tg_tunnel.supports_egress()
            );
        }
        Err(err) => {
            warn!(
                "telegram tunnel {} start failed, continue without tg tunnel: {}",
                tunnel_did.to_string(),
                err
            );
        }
    }

    Ok(tunnel_mgr)
}

pub async fn start_msg_center_service() -> Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        MSG_CENTER_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    let login_result = runtime.login().await;
    if login_result.is_err() {
        error!(
            "msg-center service login to system failed! err:{:?}",
            login_result
        );
        return Err(anyhow::anyhow!(
            "msg-center service login to system failed! err:{:?}",
            login_result
        ));
    }
    runtime.set_main_service_port(MSG_CENTER_SERVICE_PORT).await;
    set_buckyos_api_runtime(runtime);

    let center = MessageCenter::try_new()
        .map_err(|err| anyhow::anyhow!("create message center failed: {:?}", err))?;
    let tunnel_mgr = init_tg_tunnel(&center).await?;
    let server = MsgCenterHttpServer::new(center, tunnel_mgr);

    let runner = Runner::new(MSG_CENTER_SERVICE_PORT);
    if let Err(err) = runner.add_http_server(MSG_CENTER_HTTP_PATH.to_string(), Arc::new(server)) {
        error!("failed to add msg-center http server: {:?}", err);
        return Err(anyhow::anyhow!(
            "failed to add msg-center http server: {:?}",
            err
        ));
    }
    if let Err(err) = runner.run().await {
        error!("msg-center runner exited with error: {:?}", err);
        return Err(anyhow::anyhow!(
            "msg-center runner exited with error: {:?}",
            err
        ));
    }

    info!(
        "msg-center service started at port {}",
        MSG_CENTER_SERVICE_PORT
    );
    Ok(())
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    if let Err(err) = rt.block_on(async {
        init_logging("msg_center", true);
        start_msg_center_service().await
    }) {
        error!("msg-center service start failed: {:?}", err);
    }
}
