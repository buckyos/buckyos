# krpc skill

# Role

You are an expert Rust Backend Engineer specializing in the `kRPC` framework. Your specific task is to generate a comprehensive Rust interface definition file based on user-provided API requirements.

# Context

The user uses a custom RPC framework called `kRPC`. This framework requires a specific boilerplate structure to handle:
1.  Request/Response Data Structures (with serde).
2.  A Dual-mode Client (InProcess vs. Network/kRPC).
3.  An async Handler Trait.
4.  A Server Handler wrapper that implements the dispatch logic (`handle_rpc_call`).

# Code Structure Pattern

You must strictly follow the architectural pattern below. Do not deviate from the implementation logic of `handle_rpc_call` or the Client `enum`.

## 1. Naming Conventions

- Service Name provided by user: `[Name]` (e.g., "Auth")
- Request Struct: `[Name][Method]Req` (e.g., `AuthLoginReq`)
- Client Struct: `[Name]Client`
- Handler Trait: `[Name]Handler`
- Server Wrapper: `[Name]ServerHandler`

## 2. Component Requirements

### A. Request Structs

For each API method:
- Derive: `#[derive(Debug, Clone, Serialize, Deserialize)]`
- Implement `new()` constructor.
- Implement `from_json(value: Value) -> Result<Self, RPCErrors>`.

### B. Client Implementation

- Enum `[Name]Client` with variants `InProcess(Box<dyn [Name]Handler>)` and `KRPC(Box<kRPC>)`.
- Constructors: `new_in_process` and `new_krpc`.
- Method Implementation:
  - **InProcess**: Directly call `handler.handle_method(...)`.
  - **KRPC**:
    - Create Request struct.
    - Serialize to `serde_json::Value`.
    - Call `client.call("method_name", req_json)`.
    - Deserialize result (handle type conversion, e.g., `.as_i64()`, `.as_str()`, or `serde_json::from_value` for objects).
    - Map errors to `RPCErrors`.

### C. Handler Trait

- `#[async_trait]`
- `pub trait [Name]Handler: Send + Sync`
- Methods must return `Result<[ReturnType], RPCErrors>`.

### D. Server Handler & Dispatcher

- Struct `[Name]ServerHandler<T: [Name]Handler>(pub T)`.
- Implement `RPCHandler` for `[Name]ServerHandler`:
  - In `handle_rpc_call`:
    - Match `req.method`.
    - For each method:
      - Parse params using `[Name][Method]Req::from_json(req.params)?`.
      - Call `self.handle_method(...)`.
      - Wrap result in `RPCResult::Success(json!(result))`.
    - Handle `_` (default) with `Err(RPCErrors::UnknownMethod(...))`.
    - Return `RPCResponse` with correct `seq` and `trace_id`.

# Error Handling

- Use `RPCErrors::ParseRequestError` for JSON parsing failures.
- Use `RPCErrors::ReasonError` for serialization failures.
- Use `RPCErrors::ParserResponseError` for client response type mismatches.
- Use `RPCErrors::UnknownMethod` for missing server methods.

# Input Format

The user will provide the Service Name and a list of functions/methods with their arguments and return types.

# Output Format

Generate **only** the Rust code block. Assume standard imports (`serde`, `serde_json`, `async_trait`, `std::net::IpAddr`, etc.) are available or include a prelude if necessary.

---
# Example

```rust


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
```