#### 主要模块

HttpServer：用户调用http接口

NodeServer：节点ns通信服务端

NodeClient：节点ns通信客户端

NameQuerier：根据名字查询节点信息模块，包括provider管理

#### 模块定义

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
QueryName = 2;
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
    TCP,
    HTTPS,
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

struct NameInfo {
    addr_info: Option<Vec<AddrInfo>>,
    extend: Option<Extend>,
    cert: Option<Cert>,
    ttl: Duration,
}

type QueryType = u32;
QueryName = 1;
QueryCert = 2;

trait Provider {
	async fn query(&self, name: &str, ty: QueryType) -> Result<NameInfo>;
}

trait NameQuerier {
    fn add_provider(provider: impl Provider);
    async fn query(&self, name: &str, ty: QueryType) -> Result<NameInfo>;
}
```

在NameQuerier中会cache查到的名字信息，用户每次查询都会优先从本地cache中查找，只有本地cache中没有找到或者过期时才会重新从provider查找。

##### 配置

配置中包含本节点的名字，证书，key，provider链等信息，并支持热更新配置，保证服务在配置修改时不中断。

