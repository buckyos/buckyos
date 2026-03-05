//! # kRPC API Definition Example

use crate::{RPCContext, RPCErrors, RPCHandler, RPCRequest, RPCResponse, RPCResult, kRPC};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;


// Request/Response Data Structures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyApiAddReq {
    pub a: i32,
    pub b: i32,
}

impl MyApiAddReq {
    pub fn new(a: i32, b: i32) -> Self {
        Self { a, b }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        serde_json::from_value(value)
            .map_err(|e| RPCErrors::ParseRequestError(format!("Failed to parse MyApiAddReq: {}", e)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyApiDeleteAppDataReq {
    pub userid: String,
    pub appid: String,
}

impl MyApiDeleteAppDataReq {
    pub fn new(userid: &str, appid: &str) -> Self {
        Self { userid: userid.to_string(), appid: appid.to_string() }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        serde_json::from_value(value)
            .map_err(|e| RPCErrors::ParseRequestError(format!("Failed to parse MyApiDeleteAppDataReq: {}", e)))
    }
}
// Client Implementation
pub enum MyApiClient {
    InProcess(Box<dyn MyApiHandler>),
    KRPC(Box<kRPC>),
}

impl MyApiClient {

    pub fn new_in_process(handler: Box<dyn MyApiHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(client: Box<kRPC>) -> Self {
        Self::KRPC(client)
    }

    pub async fn set_context(&self, context: RPCContext)  {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await
            }
        }
    }

    pub async fn add(&self, a: i32, b: i32) -> Result<i32, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_add(a, b,ctx).await
            }
            Self::KRPC(client) => {

                let req = MyApiAddReq::new(a, b);
                let req_json = serde_json::to_value(&req)
                    .map_err(|e| RPCErrors::ReasonError(format!("Failed to serialize request: {}", e)))?;
                
                let result = client.call("add", req_json).await?;
                
                result.as_i64()
                    .map(|v| v as i32)
                    .ok_or_else(|| RPCErrors::ParserResponseError("Expected i32 result".to_string()))
            }
        }
    }

    pub async fn delete_app_data(&self, userid:&str, appid:&str) -> Result<(), RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_delete_app_data(userid, appid,ctx).await
            }
            Self::KRPC(client) => {
                let req = MyApiDeleteAppDataReq::new(userid, appid);
                let req_json = serde_json::to_value(&req)
                    .map_err(|e| RPCErrors::ReasonError(format!("Failed to serialize request: {}", e)))?;
                
                let result = client.call("delete_app_data", req_json).await?;
                
                let is_deleted = result.as_bool().ok_or_else(|| RPCErrors::ParserResponseError("Expected bool result".to_string()))?;
                if !is_deleted {
                    return Err(RPCErrors::ParserResponseError("Failed to delete app data".to_string()));
                }
                return Ok(());
            }
        }
    }
}


#[async_trait]
pub trait MyApiHandler: Send + Sync {
    async fn handle_add(&self, a: i32, b: i32,ctx:RPCContext) -> Result<i32, RPCErrors>;
    async fn handle_delete_app_data(&self, userid:&str, appid:&str,ctx:RPCContext) -> Result<(), RPCErrors>;
}


pub struct MyApiRpcHandler<T: MyApiHandler>(pub T);

impl<T: MyApiHandler> MyApiRpcHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}


/// Blanket implementation: 任何实现了 MyApiHandler 的类型自动实现 RPCHandler
#[async_trait]
impl<T: MyApiHandler> RPCHandler for MyApiRpcHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);
        
        let result = match req.method.as_str() {
            "add" => {
                let add_req = MyApiAddReq::from_json(req.params)?;
                let result = self.0.handle_add(add_req.a, add_req.b,ctx).await?;
                RPCResult::Success(json!(result))
            }
            "delete_app_data" => {
                let delete_app_data_req = MyApiDeleteAppDataReq::from_json(req.params)?;
                let result = self.0.handle_delete_app_data(&delete_app_data_req.userid, &delete_app_data_req.appid,ctx).await?;
                RPCResult::Success(json!(result))
            }
            _ => {
                return Err(RPCErrors::UnknownMethod(req.method.clone()));
            }
        };
    
        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};


    struct ExampleServer;

    #[async_trait]
    impl MyApiHandler for ExampleServer {
        async fn handle_add(&self, a: i32, b: i32,_ctx:RPCContext) -> Result<i32, RPCErrors> {
            // Business logic here
            Ok(a + b)
        }

        async fn handle_delete_app_data(&self, _userid:&str, _appid:&str,_ctx:RPCContext) -> Result<(), RPCErrors> {
            // Business logic here
            if _ctx.is_rpc {
                // let control_panel_client = get_runtime().get_control_panel_client();
                // control_panel_client.set_context(_ctx);
                // control_panel_client.delete_app_data(_userid, _appid).await?;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_handler_directly() {
        let server = ExampleServer;
        
        // Test through MyApiHandler directly
        let ctx = RPCContext::default();
        let result = server.handle_add(10, 20,ctx).await.unwrap();
        assert_eq!(result, 30);
    }

    #[tokio::test]
    async fn test_in_process_client() {

        let server = Box::new(ExampleServer);
        let client = MyApiClient::new_in_process(server);
        
        let result = client.add(10, 20).await.unwrap();
        assert_eq!(result, 30);
        
        let result = client.add(-5, 15).await.unwrap();
        assert_eq!(result, 10);
    }

    #[tokio::test]
    async fn test_rpc_handler() {
        let server = ExampleServer;
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        
        // Test through RPCHandler (auto-implemented via blanket impl)
        let rpc_handler: &dyn RPCHandler = &MyApiRpcHandler::new(server);
        
        let rpc_req = RPCRequest {
            method: "add".to_string(),
            params: json!({"a": 5, "b": 7}),
            seq: 1,
            token: None,
            trace_id: None,
        };
        
        let response = rpc_handler.handle_rpc_call(rpc_req, ip).await.unwrap();
        
        match response.result {
            RPCResult::Success(value) => {
                assert_eq!(value.as_i64().unwrap(), 12);
            }
            RPCResult::Failed(err) => {
                panic!("Unexpected error: {}", err);
            }
        }
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let server = ExampleServer;
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        
        let rpc_handler: &dyn RPCHandler = &MyApiRpcHandler::new(server);
        
        let rpc_req = RPCRequest {
            method: "unknown_method".to_string(),
            params: json!({}),
            seq: 1,
            token: None,
            trace_id: None,
        };
        
        let result = rpc_handler.handle_rpc_call(rpc_req, ip).await;
        assert!(result.is_err());
        
        match result {
            Err(RPCErrors::UnknownMethod(method)) => {
                assert_eq!(method, "unknown_method");
            }
            _ => panic!("Expected UnknownMethod error"),
        }
    }
}