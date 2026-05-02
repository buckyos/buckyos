# implement-system-service skill

# Role

You are an expert Rust Backend Engineer specializing in BuckyOS system service development. Your task is to implement a complete, testable system service based on approved protocol documents and durable data schema documents, following BuckyOS platform conventions and infrastructure constraints.

# Context

BuckyOS system services follow a strict dev loop where **protocol design** and **durable data schema** must be approved before implementation begins. This skill corresponds to **Stage 3–5** of the Service Dev Loop：

- **Stage 3**: 实现阶段的基础设施约束（Infrastructure constraints）
- **Stage 4**: 长任务模式（Long task pattern, if applicable）
- **Stage 5**: cargo test 检查点（Unit test checkpoint）

Implementation的目标是：**把协议与数据设计映射到系统已有基础设施上，而不是重新发明一套存储与访问机制。**

# Applicable Scenarios

Use this skill when:

- Protocol document and durable data schema are approved, ready to implement the service.
- Adding major new functionality to an existing system service.
- Refactoring a service's core implementation while preserving its protocol interface.

Do NOT use this skill for:

- Protocol design（use `design-krpc-protocol`）.
- Durable data schema design（use `design-durable-data-schema`）.
- Build / scheduler integration（use `buckyos-intergate-service`）.
- DV Test / TypeScript SDK testing（use `service-dv-test`）.
- Pure bugfix that doesn't change service structure.

# Input

The user will provide:

1. **Service Name** — e.g., `repo-service`, `task-queue`.
2. **Approved Protocol Document** — kRPC method list, request/response types, error codes.
3. **Approved Durable Data Schema Document** — table definitions, storage strategy, version info.
4. **Implementation Requirements (Optional)** — additional business logic, long task needs, event subscriptions.

# Output

A working Rust service implementation that:

1. Compiles without errors.
2. Passes `cargo test`.
3. Follows the standard BuckyOS service structure.
4. Is ready for the next stage (build integration and scheduler hookup).

---

# BuckyOS API Map（快速参考）

系统服务实现时可能会用到的核心 crate 和 API，按用途分类：

## 核心运行时

| Crate / Module | 用途 | 关键 API |
|---|---|---|
| `buckyos-api::runtime` | 服务运行时初始化、login、heartbeat | `init_buckyos_api_runtime()`, `runtime.login()`, `set_main_service_port()`, `get_data_folder()` |
| `buckyos-kit` | 通用工具函数（日志初始化、时间戳等） | `init_logging()`, `get_unix_timestamp()` |
| `buckyos-http-server` | HTTP 服务器框架 | `Runner::new(port)`, `runner.add_http_server(path, server)`, `runner.run()` |

## RPC 框架

| Crate / Module | 用途 | 关键 API |
|---|---|---|
| `kRPC` | RPC 协议框架 | `RPCRequest`, `RPCResponse`, `RPCResult`, `RPCErrors`, `RPCHandler` trait, `RPCContext` |
| `cyfs-gateway-lib` | Gateway HTTP 工具 | `serve_http_by_rpc_handler()`, `HttpServer` trait |

## 数据存储

| Crate / Module | 用途 | 关键 API |
|---|---|---|
| `rusqlite` | SQLite 数据库（RDB instance 的当前实现） | `Connection::open()`, `conn.execute()`, `conn.query_row()` |
| `named_store` | 命名对象存储 | `NamedStoreMgr`, `NamedLocalStore` |
| `ndn-lib` / `ndn-toolkit` | NDN 对象模型与工具 | 对象计算、chunk 操作 |

## 事件与消息

| Crate / Module | 用途 | 关键 API |
|---|---|---|
| `buckyos-api::kevent_client` | 内核事件总线 | `Event { eventid, source_node, timestamp, data }` |
| `buckyos-api::msg_queue` | 内核消息队列 | `Message { index, payload, headers }` |
| `buckyos-api::task_mgr` | 任务管理器 | `TaskStatus`, `TaskPermissions`, create/query/update tasks |

## 配置与身份

| Crate / Module | 用途 | 关键 API |
|---|---|---|
| `buckyos-api::system_config` | 系统配置（带缓存） | `SystemConfigClient`, `get_optional_value_with_revision()` |
| `name-client` | 身份 / DID 服务 | DID 解析与验证 |
| `rbac` | 角色权限控制 | 权限检查 |

## 运行时类型

```rust
pub enum BuckyOSRuntimeType {
    AppClient,       // 用户侧应用
    AppService,      // 用户特定服务
    FrameService,    // 容器化 frame 服务
    KernelService,   // 系统内核服务
    Kernel,          // 核心守护进程
}
```

---

# Service Implementation Template

## 1. 项目结构

```
src/<layer>/<service_name>/
├── Cargo.toml
└── src/
    ├── main.rs          # 入口：创建 runtime，调用 run_service()
    ├── service.rs       # 核心服务逻辑 + Handler trait 实现 + HTTP Server
    └── <service>_db.rs  # 数据库抽象层（若有持久数据）
```

其中 `<layer>` 通常是 `kernel`（内核服务）或 `frame`（框架服务）。

## 2. Cargo.toml 依赖模式

```toml
[package]
name = "<service-name>"
version.workspace = true
edition = "2021"

[dependencies]
# 核心运行时
buckyos-kit = { workspace = true }
buckyos-api = { path = "../../kernel/buckyos-api" }  # 路径视目录层级调整
kRPC = { workspace = true }

# HTTP 服务
buckyos-http-server = { workspace = true }
cyfs-gateway-lib = { workspace = true }

# 异步运行时
tokio = { workspace = true }
async-trait = { workspace = true }

# 序列化
serde = { workspace = true }
serde_json = { workspace = true }

# 日志
log = { workspace = true }

# 数据库（若有结构化持久数据）
rusqlite = { workspace = true }

# 按需添加
# name-client = { workspace = true }
# ndn-lib = { workspace = true }
# named_store = { workspace = true }
```

## 3. main.rs 入口模式

```rust
mod service;
mod <service>_db;  // 若有

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        if let Err(e) = service::run_service().await {
            log::error!("Service exited with error: {}", e);
            std::process::exit(1);
        }
    });
}
```

## 4. service.rs 核心结构

### 4.1 服务常量

```rust
const SERVICE_NAME: &str = "<service-name>";
const SERVICE_PORT: u16 = <port>;  // 与 Service Spec 一致
```

### 4.2 服务初始化流程（run_service）

这是每个 BuckyOS 系统服务的标准启动流程，**顺序不可打乱**：

```rust
pub async fn run_service() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 初始化日志
    buckyos_kit::init_logging(SERVICE_NAME, true);

    // 2. 初始化 BuckyOS 运行时
    let runtime = buckyos_api::init_buckyos_api_runtime(
        SERVICE_NAME,
        None,                                    // app_owner_id, 内核服务填 None
        buckyos_api::BuckyOSRuntimeType::KernelService,  // 按服务类型选择
    ).await?;

    // 3. Login（建立服务身份、获取 token）
    runtime.login().await?;

    // 4. 注册服务端口
    runtime.set_main_service_port(SERVICE_PORT).await;

    // 5. 获取数据目录
    let data_dir = runtime.get_data_folder()?;

    // 6. 注册全局运行时单例
    buckyos_api::set_buckyos_api_runtime(runtime);

    // 7. 初始化服务逻辑（含数据库）
    let service = MyService::new(data_dir).await?;

    // 8. 构建 HTTP Server
    let http_server = MyHttpServer::new(service);

    // 9. 启动 Runner
    let mut runner = buckyos_http_server::Runner::new(SERVICE_PORT);
    runner.add_http_server("/kapi/<service-name>", Box::new(http_server));
    runner.run().await?;

    Ok(())
}
```

### 4.3 服务结构体

```rust
#[derive(Clone)]
pub struct MyService {
    db: Arc<dyn MyServiceDb>,
    // 其他状态...
}

impl MyService {
    pub async fn new(data_dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let db_path = data_dir.join("<service>.db");
        let db = SqliteMyServiceDb::open(db_path).await?;
        Ok(Self {
            db: Arc::new(db),
        })
    }
}
```

### 4.4 Handler Trait 实现

实现由 `design-krpc-protocol` 生成的 Handler trait：

```rust
#[async_trait]
impl MyServiceHandler for MyService {
    async fn handle_<method>(
        &self,
        /* 参数 */
        ctx: RPCContext,
    ) -> Result<ReturnType, RPCErrors> {
        // 业务逻辑
        // 使用 self.db 访问数据
        // 通过 ctx 获取调用者信息（DID, device 等）
    }
}
```

### 4.5 HTTP Server 桥接

将 kRPC handler 桥接到 HTTP server：

```rust
pub struct MyHttpServer {
    rpc_handler: MyServiceRpcHandler<MyService>,
}

impl MyHttpServer {
    pub fn new(service: MyService) -> Self {
        Self {
            rpc_handler: MyServiceRpcHandler::new(service),
        }
    }
}

#[async_trait]
impl HttpServer for MyHttpServer {
    async fn serve_request(
        &self,
        req: hyper::Request<hyper::Body>,
        info: &RequestInfo,
    ) -> ServerResult<hyper::Response<hyper::Body>> {
        if *req.method() == hyper::Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        Err(ServerError::BadRequest("Only POST supported".to_string()))
    }
}

#[async_trait]
impl RPCHandler for MyHttpServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        self.rpc_handler.handle_rpc_call(req, ip_from).await
    }
}
```

## 5. 数据库抽象层（`<service>_db.rs`）

### 5.1 Trait 抽象

```rust
#[async_trait]
pub trait MyServiceDb: Send + Sync {
    async fn insert_record(&self, record: MyRecord) -> Result<(), RPCErrors>;
    async fn get_record(&self, id: &str) -> Result<Option<MyRecord>, RPCErrors>;
    async fn list_records(&self) -> Result<Vec<MyRecord>, RPCErrors>;
    async fn delete_record(&self, id: &str) -> Result<(), RPCErrors>;
    // ... 与 durable data schema 中的查询模式对应
}
```

### 5.2 SQLite 实现

```rust
pub struct SqliteMyServiceDb {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteMyServiceDb {
    pub async fn open(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let conn = tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(&path)?;
            // WAL 模式，提升并发性能
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "busy_timeout", 5000)?;
            // 建表（来自 durable data schema）
            conn.execute_batch("
                CREATE TABLE IF NOT EXISTS meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                -- 其他表...
            ")?;
            Ok::<_, Box<dyn std::error::Error>>(conn)
        }).await??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

#[async_trait]
impl MyServiceDb for SqliteMyServiceDb {
    async fn insert_record(&self, record: MyRecord) -> Result<(), RPCErrors> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO records (...) VALUES (...)",
                rusqlite::params![...],
            ).map_err(|e| RPCErrors::ReasonError(format!("DB error: {}", e)))?;
            Ok(())
        }).await
            .map_err(|e| RPCErrors::ReasonError(format!("Task join error: {}", e)))?
    }
    // ... 其他方法
}
```

**关键规则：**

- 所有 `rusqlite` 操作 **MUST** 通过 `tokio::task::spawn_blocking()` 执行，避免阻塞 async runtime。
- 建表语句 **MUST** 与 durable data schema 文档中的定义一致。
- **MUST** 启用 WAL 模式和合理的 PRAGMA 设置。

## 6. 长任务模式（若适用）

当业务逻辑涉及长时间执行时，**MUST** 使用以下组合：

```rust
// 使用 task_manager 创建任务
use buckyos_api::task_mgr::{TaskStatus, TaskPermissions};

// 使用 kevent 等待状态变化（而非轮询）
use buckyos_api::kevent_client::Event;

// 使用 msg_queue 进行持久消息传递
use buckyos_api::msg_queue::Message;
```

**推荐模式：**

```
发起长任务请求 → 获取 task_id → 订阅 keyevent 状态变化 → timeout 保底 → 完成后继续
```

**MUST NOT** 以高频 timer 轮询作为主流程。

## 7. 单元测试模式

单元测试由协议文档和 durable data schema **反推**，不是拍脑袋写。

### 7.1 测试工具函数

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn test_service() -> (TempDir, MyService, MyServiceClient) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let service = MyService::new(dir.path().to_path_buf()).await.unwrap();
        let client = MyServiceClient::new_in_process(Box::new(service.clone()));
        (dir, service, client)
    }
}
```

### 7.2 必须覆盖的测试类型

来自协议文档：

- [ ] 每个 kRPC 方法的正常路径
- [ ] 输入参数边界条件
- [ ] 错误码行为（如 NotFound、PermissionDenied）
- [ ] 幂等性验证（若协议声明幂等）

来自 durable data schema：

- [ ] 数据 CRUD round-trip
- [ ] 索引查询正确性
- [ ] Schema version 读写
- [ ] JSON 元数据字段序列化/反序列化

### 7.3 测试示例

```rust
#[tokio::test]
async fn create_and_get_record() {
    let (_dir, _service, client) = test_service().await;

    // 创建
    let id = client.create("test-name", "payload").await.unwrap();
    assert!(!id.is_empty());

    // 读取
    let record = client.get(&id).await.unwrap();
    assert_eq!(record.name, "test-name");
}

#[tokio::test]
async fn get_nonexistent_returns_not_found() {
    let (_dir, _service, client) = test_service().await;
    let result = client.get("nonexistent-id").await;
    assert!(matches!(result, Err(RPCErrors::ReasonError(_))));
}

#[tokio::test]
async fn db_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let db = SqliteMyServiceDb::open(dir.path().join("test.db")).await.unwrap();

    let record = MyRecord { id: "1".into(), name: "test".into(), /* ... */ };
    db.insert_record(record.clone()).await.unwrap();

    let fetched = db.get_record("1").await.unwrap();
    assert_eq!(fetched.unwrap().name, "test");
}
```

---

# Implementation Checklist

实现过程中按顺序验证：

## 基础设施约束

- [ ] 结构化数据使用 RDB instance（当前为 rusqlite），未直接绑定特定后端
- [ ] 非结构化数据使用 object 管理（若有）
- [ ] 未围绕文件系统路径设计核心数据（若有例外，已文档说明）
- [ ] 长任务使用 task_manager / kevent / msg_queue 组合（若有长任务）

## 代码结构

- [ ] 项目结构符合模板（main.rs / service.rs / db.rs）
- [ ] 服务初始化顺序正确（logging → runtime → login → port → data_dir → set_runtime → service → server）
- [ ] Handler trait 实现覆盖协议文档中所有方法
- [ ] DB 抽象层与 durable data schema 一致
- [ ] 建表语句包含 schema_version

## 测试

- [ ] `cargo test` 全部通过
- [ ] 协议解析测试覆盖
- [ ] 数据 round-trip 测试覆盖
- [ ] 错误码行为测试覆盖
- [ ] 边界条件测试覆盖

---

# Common Failure Modes

1. **初始化顺序错误** — `login()` 必须在 `init_buckyos_api_runtime()` 之后、`set_buckyos_api_runtime()` 之前调用。顺序错误会导致 panic 或 token 失效。
2. **阻塞 async runtime** — `rusqlite` 操作未用 `spawn_blocking()` 包裹，导致 tokio runtime 被阻塞，heartbeat 超时。
3. **DB schema 与文档不一致** — 建表语句与 durable data schema 文档不匹配，导致后续升级兼容性破裂。
4. **缺少 WAL 模式** — SQLite 未启用 WAL，并发读写时性能极差或死锁。
5. **绕过 kRPC 直接暴露 HTTP** — 服务应通过 kRPC handler 处理请求，不应自行解析 HTTP body。
6. **未注册 kAPI 路径** — `runner.add_http_server()` 的路径必须是 `/kapi/<service-name>`，否则 gateway 无法路由。
7. **测试依赖真实运行时** — 单元测试应使用 `InProcess` client 和 `tempdir`，不依赖 login / scheduler / gateway。
8. **直接绑定 sqlite** — 虽然当前 RDB 实现是 sqlite，但代码应通过 trait 抽象，方便未来替换。
9. **忽略 RPCContext** — Handler 方法的 `ctx` 参数包含调用者身份信息（DID、device），权限检查和审计依赖此信息。

---

# Pass Criteria

满足以下全部条件，视为本阶段通过：

- [ ] `cargo build` 无错误
- [ ] `cargo test` 全部通过
- [ ] 项目结构符合标准模板
- [ ] 服务初始化流程正确
- [ ] Handler 实现覆盖协议文档所有方法
- [ ] DB 层与 durable data schema 文档一致
- [ ] 单元测试覆盖协议解析、数据 round-trip、错误码、边界条件
- [ ] 无 clippy 严重警告
- [ ] 可进入下一阶段（build integration + scheduler hookup）
