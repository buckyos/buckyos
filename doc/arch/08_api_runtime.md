# 08. buckyos-api-runtime（服务发现 / 登录 / 调用路径选择）

buckyos-api-runtime 的定位是“每个进程自己的调用运行时”：
- 把身份（session-token）与信任根（trust keys）收敛到一个对象里。
- 把“我要访问哪个系统服务”收敛为 `get_zone_service_url()`，并按 runtime 类型选择兼容/性能路径。
- 把“如何拿到 system-config / verify-hub / control-panel 的客户端”标准化，避免每个 service 都重复写一套发现逻辑。

参考（概念与动机）：
- `new_doc/ref/notepads/buckyos-api-runtime.md`

参考（实现锚点）：
- `src/kernel/buckyos-api/src/lib.rs`
- `src/kernel/buckyos-api/src/runtime.rs`

## 读者的心智模型（runtime 到底在解决什么）

把 runtime 看成一个很小的“OS 调用层”，它主要解决两个现实问题：

1) 访问入口不稳定：家庭/边缘网络里，服务实例的端口与可达路径不是固定的。
- 因此 runtime 需要根据自身所处位置（AppClient / AppService / KernelService / Kernel）选择 URL。
- 同时要容忍“有网关就走网关，没有网关就走 hostname”，并尽可能把复杂性封装掉。

2) 权限与信任必须内建：系统内部调用同样要携带 token，并且 token 的验证需要一组可信公钥（trust keys）。
- runtime 在 `login()` 后会得到 `zone_config` 并刷新 `trust_keys`，供 `enforce()` 做标准化鉴权（`src/kernel/buckyos-api/src/runtime.rs`）。

## 三种访问路径（从兼容到性能）

notepads 给出的分层在代码里对应为：优先兼容，逐步提速（`new_doc/ref/notepads/buckyos-api-runtime.md`）。

1) 公网/通用：`https://$zone_host/kapi/<service_name>`
- 典型场景：AppClient（本机没有 NodeGateway）。
- 代码路径：`src/kernel/buckyos-api/src/runtime.rs` 的 `BuckyOSRuntime::get_zone_service_url()`，`BuckyOSRuntimeType::AppClient` 分支直接返回 `https?://{zone_id.to_host_name()}/kapi/<service>`。

2) 最大兼容：`http://127.0.0.1:3180/kapi/<service_name>`（NodeGateway）
- 典型场景：AppService 运行在节点上，本机一定有 cyfs-gateway / NodeGateway。
- 常量锚点：`DEFAULT_NODE_GATEWAY_PORT: u16 = 3180`（`src/kernel/buckyos-api/src/runtime.rs`）。
- 运行时字段：`BuckyOSRuntime.node_gateway_port`（默认为 3180，可被配置覆盖）。

3) 最佳性能：直连本机实例端口（`http://127.0.0.1:<port>/kapi/<service>`）或直连 rtcp
- 典型场景：KernelService / Kernel 在同机发现到目标 service instance 的 `www` 端口时直接本机 loopback。
- 代码路径：`src/kernel/buckyos-api/src/runtime.rs` 的 `BuckyOSRuntime::get_kernel_service_url()`。

重要区别：对 AppService 来说，即使 runtime 能算出“目标实例在哪”，实现上依旧倾向走 NodeGateway 以获得更强的兼容性与转发能力（见 `get_zone_service_url()` 的 AppService 分支）。

## 关键数据结构（从代码抽取的最小视图）

本节只摘“理解调用链路必须知道”的字段，忽略大量身份/目录/配置细节。

对应代码：`src/kernel/buckyos-api/src/runtime.rs`

```rust
pub struct BuckyOSRuntime {
    pub runtime_type: BuckyOSRuntimeType,
    pub zone_id: DID,
    pub node_gateway_port: u16,

    pub zone_config: Option<ZoneConfig>,

    pub session_token: Arc<RwLock<String>>,
    trust_keys: Arc<RwLock<HashMap<String, DecodingKey>>>,
}
```

含义：
- `runtime_type`：决定“走 hostname 还是走 127.0.0.1:3180”以及是否允许加载私钥（`BuckyOSRuntimeType` 在 `src/kernel/buckyos-api/src/runtime.rs`）。
- `zone_id`：所有 zone 级访问与 service url 选择的根（`login()` 会强校验必须有效，否则直接失败）。
- `node_gateway_port`：NodeGateway 的本地入口端口，默认 3180（`DEFAULT_NODE_GATEWAY_PORT`）。
- `zone_config`：`login()` 后从 control-panel / system-config 拉取到的 ZoneConfig（用于 trust keys、verify-hub 公钥等信息）。
- `session_token`：进程级调用 token（既用于调用 system services，也用于服务端 `enforce()` 验证客户端请求）。
- `trust_keys`：JWT 验签用的可信公钥集合（包含 device key、zone owner key、verify-hub key 等；刷新逻辑见 `refresh_trust_keys()`）。

## 初始化：runtime 如何“收集登陆所需信息”

初始化入口：`init_buckyos_api_runtime()`（`src/kernel/buckyos-api/src/lib.rs`）。它的目标不是“连上系统”，而是准备好 login 所需的数据来源。

代码关键点：
- `fill_policy_by_load_config()`：读取 machine.json 类策略（如 `force_https`、web3 bridge）。（`src/kernel/buckyos-api/src/runtime.rs`）
- `fill_by_load_config()`：按 runtime 类型读取 `node_identity.json` / `node_private_key.pem` / `user_config.json` / `user_private_key.pem` 等。（`src/kernel/buckyos-api/src/runtime.rs`）
- `fill_by_env_var()`：为服务类 runtime（KernelService / AppService 等）从环境变量读取启动 token；未配置会直接报错。（`src/kernel/buckyos-api/src/runtime.rs`）

token 环境变量名规则：`get_session_token_env_key()`（`src/kernel/buckyos-api/src/lib.rs`）：
- 先把 full_app_id upper-case 并把 `-` 替换为 `_`。
- service 默认读 `<APP>_SESSION_TOKEN`，AppService 读 `<APP>_TOKEN`。

## login：检查 token + 拉取 zone_config + 启动 keep-alive

`BuckyOSRuntime::login()`（`src/kernel/buckyos-api/src/runtime.rs`）做三件事：

1) 校验与补齐 `session_token`
- 如果 token 为空：
  - AppClient 优先尝试用 `user_private_key` 生成 token；否则尝试用 `device_private_key` 生成 token。
  - 仍为空则失败：`session_token is empty!`。
- 如果 token 不为空：解析 token 并检查 token 中 appid 与 `self.app_id` 一致，否则失败。

2) 连接 control-panel 并读取 `zone_config`
- 代码路径：`get_control_panel_client()` → `load_zone_config()`（`src/kernel/buckyos-api/src/runtime.rs`）。
- 结果写入 `self.zone_config`。

3) 启动 keep-alive 定时任务（5s 一次）
- 代码路径：`login()` 末尾 `tokio::task::spawn(...)`（`src/kernel/buckyos-api/src/runtime.rs`）。
- keep-alive 主要做：
  - `renew_token_from_verify_hub()`（必要时刷新 token）
  - `update_service_instance_info()`（服务实例上报）
  - 对服务类 runtime 还会：拉取 rbac 配置并 `refresh_trust_keys()`。

## 服务 URL 选择：get_zone_service_url() 的实际分支

核心 API：`BuckyOSRuntime::get_zone_service_url(service_name, https_only)`（`src/kernel/buckyos-api/src/runtime.rs`）。

```text
get_zone_service_url(service):
  if runtime_type == AppClient:
    return https?://<zone_host>/kapi/<service>

  if runtime_type in {AppService, FrameService}:
    (url,is_local) = get_kernel_service_url(service)
    if is_local: return url                     // http://127.0.0.1:<port>/kapi/<service>
    else: return http://127.0.0.1:<node_gateway_port>/kapi/<service>

  if runtime_type in {KernelService, Kernel}:
    return get_kernel_service_url(service)
```

其中 `get_kernel_service_url()` 会：
- 先从 system-config 读取 `service_info`（通过 `ControlPanelClient::get_services_info()`）。
- 如果本机就是一个 Started 的实例，并且能解析出 `www` 端口：直接返回 `http://127.0.0.1:<port>/kapi/<service>`。
- 否则：
  - 对非 Kernel 的调用方，直接退化到 NodeGateway（`http://127.0.0.1:3180/kapi/<service>`）。
  - 对 Kernel，进一步基于权重随机选节点，并构造 `rtcp://<node_did>/127.0.0.1:<port>`。（`src/kernel/buckyos-api/src/runtime.rs`）

## 调用范式：从 service_url 到具体 API 调用

典型的系统服务调用写法在大量服务里复用（例如 repo-service / smb-service）：
- repo-service：`src/frame/repo_service/src/main.rs`
- smb-service：`src/frame/smb_service/src/main.rs`
- buckycli：`src/tools/buckycli/src/main.rs`

runtime 提供的“标准拼装”是：`get_zone_service_krpc_client()`（`src/kernel/buckyos-api/src/runtime.rs`）。

```text
client = runtime.get_zone_service_krpc_client("verify-hub")
verify_hub = VerifyHubClient::new(client)
verify_hub.some_call(...)
```

## 关键流程伪代码（runtime init → login → 访问服务）

伪代码目的：表达控制流和依赖项，而不是复刻实现。

```text
process_main():
  runtime = init_buckyos_api_runtime(app_id, owner_id, runtime_type)
    // src/kernel/buckyos-api/src/lib.rs
    //  - fill_policy_by_load_config()
    //  - fill_by_load_config() (some runtime types)
    //  - fill_by_env_var() (service types load token)

  runtime.login()
    // src/kernel/buckyos-api/src/runtime.rs
    //  - ensure session_token exists (env var or generated by private key)
    //  - control-panel.load_zone_config() => zone_config
    //  - spawn keep_alive timer

  set_buckyos_api_runtime(runtime)

  // later call
  url = runtime.get_zone_service_url(service, https_only)
  krpc = kRPC::new(url, runtime.session_token)
  ServiceClient::new(krpc).call(...)
```

## 常见坑（你会在生产环境里真的踩到）

### 1) NodeGateway 依赖：3180 挂了会发生什么

- AppService 在访问 system services 时，很多路径会退化到 `http://127.0.0.1:3180/kapi/<service>`（例如 `get_system_config_client()` 的 AppService 分支直接走 NodeGateway：`src/kernel/buckyos-api/src/runtime.rs`）。
- 这意味着：NodeGateway 不是“可选加速层”，在 AppService 的兼容模式下它是主链路。
- 对应 notepads 的提醒：NodeGateway down 会导致大量 app/service 暂时不可用（`new_doc/ref/notepads/buckyos-api-runtime.md`）。

### 2) token 刷新语义：有两套刷新路径，别混淆

代码里同时存在两种“续命”方式：

- `renew_token_from_verify_hub()`：
  - 只有当当前 token 非空且接近过期时才会触发（`exp` 小于当前时间 + 30s）。
  - 如果 token 的 `iss` 不是 `verify-hub` 也会触发刷新。
  - 刷新方式是调用 `verify_hub_client.login_by_jwt(old_token)` 并把返回的 token 写回 `self.session_token`。
  - 触发点是 keep-alive 定时任务（`login()` 启动的 5s timer）。
  - 代码路径：`src/kernel/buckyos-api/src/runtime.rs`。

- `get_session_token()`：
  - 在获取 token 时，如果发现 token 即将过期（`exp` 小于当前时间 + 10s），会尝试用 `device_private_key` 重新生成一个 JWT 并覆盖内存里的 token。
  - 这条路径不依赖 verify-hub 在线，但要求本进程有 device private key。
  - 代码路径：`src/kernel/buckyos-api/src/runtime.rs`。

实际影响：
- 你可能看到“verify-hub 暂时不可用但调用仍能继续一段时间”，因为 `get_session_token()` 还能本地续签（前提是有 device private key）。
- 也可能看到“token 看起来没过期但仍然会 refresh”，因为 `iss != verify-hub` 会触发刷新（例如某些自签 token 场景）。

### 3) 环境变量 token 缺失会导致服务启动直接失败

- 对 KernelService / FrameService / AppService，`fill_by_env_var()` 会按规则寻找 token env；缺失会直接返回 error（`load session_token from env var failed`）。
- 这通常发生在：node-daemon 拉起服务但没有把 token 注入到环境里，或者 env key 名称没按 `get_session_token_env_key()` 规则生成。

### 4) trust_keys 不刷新会导致 enforce 误判

- 服务端 `enforce()` 的 kid → decoding key 匹配依赖 `trust_keys`。
- keep-alive 中对服务类 runtime 会周期性 `refresh_trust_keys()`；如果 `zone_config.verify_hub_info` 缺失会 warn 并导致部分 token 验证失败。
- 代码路径：`src/kernel/buckyos-api/src/runtime.rs` 的 `refresh_trust_keys()` 与 `enforce()`。
