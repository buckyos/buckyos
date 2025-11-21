use ::kRPC::*;
use anyhow::Result;
use async_trait::async_trait;
use buckyos_kit::*;
use cyfs_gateway_lib::WarpServerConfig;
use cyfs_warp::*;
use log::*;
// use name_client::*;
use serde_json::*;
use std::net::IpAddr;

#[derive(Clone)]
struct ControlPanelServer {}

impl ControlPanelServer {
    pub fn new() -> Self {
        ControlPanelServer {}
    }

    async fn handle_main(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        return Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "test":"test",
            })),
            req.id,
        ));
    }
}

#[async_trait]
impl InnerServiceHandler for ControlPanelServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            "main" => self.handle_main(req).await,
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }

    async fn handle_http_get(&self, req_path: &str, _ip_from: IpAddr) -> Result<String, RPCErrors> {
        return Err(RPCErrors::UnknownMethod(req_path.to_string()));
    }
}

async fn service_main() {
    init_logging("control_server", true);

    let control_server = ControlPanelServer::new();
    // .map_err(|e| {
    //     error!("control_server init error! err:{}", e);
    //     anyhow::anyhow!("control_server init error! err:{}", e)
    // })?;

    // control_server.init().await?;
    info!("control_server init check OK.");

    register_inner_service_builder("control_server", move || Box::new(control_server.clone()))
        .await;

    let service_config = json!({
      "http_port":3180,
      "tls_port":0,
      "hosts": {
        "*": {
          "enable_cors":true,
          "routes": {
            "/kapi/control-panel" : {
                "inner_service":"control_server"
            }
          }
        }
      }
    });

    let service_config: WarpServerConfig = serde_json::from_value(service_config).unwrap();
    let _ = start_cyfs_warp_server(service_config).await;
    let _ = tokio::signal::ctrl_c().await;
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}
