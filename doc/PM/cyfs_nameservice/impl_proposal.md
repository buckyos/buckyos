#### 主要模块

NodeInfoStorage：本节点信息存储模块

HttpServer：用户调用http接口

NodeServer：节点ns通信服务端

NodeClient：节点ns通信客户端

NameQuerier：根据名字查询节点信息模块，包括provider管理

#### 模块定义

##### NodeInfoStorage

该模块主要存储节点的扩展数据

```rust
enum Scope {
    Zone_in,
    Zone_out,
}

struct ExtendItem {
    extend_key: string,
    extend_value: string,
    is_crypto: bool,
    scope: Scope
}

struct Extend {
   	data: HashMap<String, ExtendItem>
}

trait NodeInfoStorageEvent {
    async fn on_extend_change(&self) -> Result<()>;
}

struct Node {
    name: string,
    latest_update: u64,
    ...
}

trait NodeInfoStorage {
    fn add_event_listener(&self, listener: impl NodeInfoStorageEvent);
    async fn set_extend(&self, extend: &Extend) -> Result<()>;
    async fn del_extend(&self, extends: Vec<string>) -> Result<()>;
    async fn add_node_info(&self, node: Node) -> Result<()>;
    async fn get_node_info(&self, name: &str) -> Result<Node>;
}
```

##### NodeServer

提供一个命令处理框架，默认支持一些命令，provider也可以扩展命令

```rust
type CMD = u16;
struct Request {
    cmd: CMD,
    ...
}

struct Response {
    cmd: CMD,
    ...
}

trait CmdHandler {
    async fn on_handle(&self, req: Request) -> Result<Response>;
}

trait NodeServer {
	fn register_cmd(&self, cmd: impl CmdHandler) -> Result<()>;
    fn start();
}

// 默认支持命令
QueryCert = 1;
QueryExtend = 2;
QueryCertAndExtend = 3;
```

NodeClient

```rust
type CMD = u16;
struct Request {
    cmd: CMD,
    ...
}

struct Response {
    cmd: CMD,
    ...
}

trait NodeClient {
    async fn send(&self, req: Request) -> Result<Response>;
}
```

##### NameQuerier

```rust
enum Protocol {
    IPV4,
    IPV6,
    CYFS,
}

struct AddrInfo {
    protocol: Protocol,
    address: string,
}

struct Extend {
   	data: HashMap<String, String>
}

enum CertType {
    X509,
    CYFS
}

struct Cert {
    ty: CertType,
    cert: Vec<u8>,
}

struct Node {
    addr_info: Option<AddrInfo>,
    extend: Option<Extend>,
    cert: Option<Cert>,
}

type QueryType = u32;
AddrInfo = 1;
Cert = 2;
Extend = 4;

trait Provider {
	async fn query(&self, name: &str, ty: QueryType) -> Result<Node>;
}

trait NameQuerier {
    fn add_provider(provider: impl Provider);
    async fn query(&self, name: &str, ty: QueryType) -> Result<Node>;
}
```



