# RTCP SOCKS 入口实现 review

日期：2026-11-18。范围：把 SOCKS5 作为向本机其他进程暴露 RTCP stack 的正式方式，评审现有实现。源码基线：`buckyos@317534347`、`cyfs-gateway@f0ef92f2`。本文只评审，不改代码。

## 0. 评审对象

| 部分 | 位置 |
|---|---|
| SOCKS server（type: socks） | cyfs-gateway `components/cyfs-socks`：`SocksServerFactory`（server/server.rs）、`Socks5Proxy`（socks/socks5.rs）、`SocksTunnelBuilder`（tunnel/tunnel_builder.rs） |
| Agent → Node Executor 入口 | buckyos `rootfs/etc/boot_gateway.yaml`：`node_executor_entry`（`0.0.0.0:3185`）→ `node_executor_socks` |
| 系统服务 → 跨 Zone 入口 | 同上：`cross_zone_entry`（`127.0.0.1:3186`）→ `cross_zone_socks`，凭据由 node_daemon 写入 |
| 凭据 | buckyos `kernel/node_daemon/src/boot.rs:599-650`（`write_cross_zone_socks_credentials`） |
| 客户端 | `buckyos-api/src/cross_zone_client.rs`（`CrossZoneClient`）、`agent_tool/src/runtime/node.rs:1933-2070`（`SocksNodeConnector`） |

## 1. 设计约定（已确认，不列为问题）

SOCKS 目标 host 原样进入 RTCP stack id，包括 `<bootstrap>@<host>` 中转形式和 `host:port` 显式端口：SOCKS 客户端可以表达完整的 RTCP 目标。这是设计意图。代价是网关在用户态，不能依赖内核/OS 的网络策略，**每个 SOCKS server 的 hook 就是策略边界**，需要按 RTCP 语义定制（允许哪些 stack id 形式、哪些中转、哪些端口）。正式接口文档应写明这一点。

## 2. 问题清单

| # | 级别 | 问题 |
|---|---|---|
| R1 | 高 | hook 没有给出明确决策时默认 `DIRECT`（fail-open） |
| R2 | 中 | SOCKS 回 `Succeeded` 不代表远端接受，策略拒绝与网络故障不可区分 |
| R3 | 中 | executor 入口按源 IP 前缀放行、无凭据，本节点设备身份借给所有本机进程和容器 |
| R4 | 低 | Windows 上凭据文件对所有本地用户可读可改 |
| R5 | 低 | 用户名/密码比较不是常量时间 |
| R6 | 低 | server 的 `connect_timeout` 只作用于 `DIRECT` |
| R7 | 低 | 配置面与实际行为不一致，有死代码 |
| R8 | 低 | 接口能力边界未成文 |

### R1 hook 没有给出明确决策时默认 DIRECT

`socks5.rs:186-226`。以下情况都落到 `ProxyAccessMethod::Direct`：

- 执行结果不是 control（块执行完没有 `return`），186-204；
- `return` 的值不是字符串，190-198；
- 返回值解析失败，例如 `REJECT` 拼成 `REJCT`，218-225；
- 返回值为空，212-214。

`DIRECT`（`process_socket_direct`，261-335）由网关进程直接连客户端给的任意 IP/域名和端口，包括本机回环上的 3180、3184、3186 等服务。也就是说，一个 hook 写错就会把 SOCKS 入口变成通往宿主机回环和局域网的开放代理。

现有两个 hook 每条路径都显式 return，目前不会触发，但正式接口不能依赖配置作者不犯错。

另外，`execute_lib` 出错时（180-184）直接返回 Err，不给 SOCKS 回复，客户端只看到 EOF。

建议：

- 无决策、非字符串、解析失败、空值一律按 `REJECT` 处理，`DIRECT` 只在显式返回时生效；
- 可以再加 server 级开关（如 `allow_direct`，默认 false），防止 hook 误放行；
- hook 执行出错时回复 `GeneralFailure`；
- 补测试：块执行完无 return、关键字拼错、非字符串返回值都应拒绝。

### R2 SOCKS 成功应答不代表远端接受

`SocksTunnelBuilder::build` 在 `open_stream_by_dest_with_timeout` 返回后，`socks5.rs:343-347` 回 `Succeeded`。但 RTCP 远端在 `on_open`（`rtcp.rs:5698-5752`）里只做资源准入（`admit_inbound_stream_build`）就回 `OPEN_RESULT_OK`。远端 stack 的 hook（`reject`、`forward` 失败、ACL 不通过）在 HelloStream 之后才执行。所以远端按策略拒绝时，客户端先收到 CONNECT 成功，随后才 EOF 或 reset。

以上是直连 Open 路径的核实结果；ROpen（经中转）路径没有逐行核对。

对 `CrossZoneClient` 的影响：

- `send` 只在 `error.is_connect() || replayable` 时换 HTTPS 重试（`cross_zone_client.rs:521`）。CONNECT 成功之后的 EOF 不算连接错误，而 `replayable` 只认 GET/HEAD（184），所以 krpc（包括 login）这类 POST 不会回退，当次请求直接失败。
- 同时 `record` 把该 Zone 标为 `HttpsFallback`（545-563），之后 300 秒（`RTCP_REPROBE_INTERVAL`）只走 HTTPS。

作为正式接口，调用方无法区分"被远端策略拒绝"和"网络不通"，也就没法决定该重试、换传输还是报权限错误。

建议（需改 RTCP 协议）：

- 让远端在回 OpenResp 前跑完 stack hook 的准入部分，或在 HelloStream 后补一个结果帧；
- 映射到 SOCKS 回复码：`0x02` 策略拒绝，`0x05` 拒绝连接，`0x04` 主机不可达，`0x06` 超时。

改协议之前，至少在客户端文档里写明 CONNECT 成功之后仍可能被拒。

### R3 executor 入口的访问控制与身份外借

`boot_gateway.yaml:107-123`：`node_executor_entry` 绑定 `0.0.0.0:3185`，用 `match ${REQ.conn_source_ip}` 放行 `127.*` 和 `172.*`。`node_executor_socks`（141-157）不要求凭据。

- **`172.*` 范围过宽**：它覆盖整个 `172.0.0.0/8`，而私有地址只有 `172.16.0.0/12`。节点有公网 IPv4 或做了端口映射时，来自 `172.x` 公网段的连接也能通过。
- **对 Docker 网段有假设**：daemon 的 `default-address-pools` 或自定义网络可以用 `10.x`、`192.168.x`，那样容器会被拒，功能失效。process chain 也没有 CIDR 匹配命令，目前只能写 glob。
- **身份外借**：远端只按设备核对 `EXECUTOR_ACL`（`source_device_id` = 本节点），所以本机任何进程、任何容器都以本节点身份访问其他节点的 executor。
- **绕过设备 ACL 检查**：目标 `127.0.0.1:3184` 时返回 `DIRECT`（153），容器因此能连到宿主机回环上的本地 executor。gateway 的 `EXECUTOR_ACL` 检查只挂在 `node_rtcp` 入站上，这条路径完全不经过它。

现有缓解：executor 登录要求 verify-hub 签发的 App session token，并且要求该 App instance 绑定到已授权的 Agent（`node_executor/src/auth.rs:189-213`）。所以这不等于拿到执行权，但设备级 ACL 这一层对本机调用方等于不存在，实际授权只剩 token 一层。

`SocksNodeConnector` 用 `get_local_gateway_host()` 找入口（`agent_tool/.../node.rs:1960-1965`），容器内通常是网桥网关地址，这正是放行 `172.*` 的原因。

建议：

- 只绑定 `127.0.0.1` 和 docker 网桥的网关地址，不绑 `0.0.0.0`；
- 和 `cross_zone_entry` 一样要求凭据，挂载进需要访问 executor 的容器。更理想的是按 App 区分凭据，让网关知道调用方是谁，也就能在 hook 里按 App 限制可达节点；
- 给 process chain 增加 CIDR 匹配，替代前缀 glob。

### R4 Windows 上凭据文件对所有本地用户可读可改

`write_private_json`（`boot.rs:633-650`）只在 `#[cfg(unix)]` 下设置 0600。Windows 上文件继承目录 ACL。本机实测 `C:\buckyos\etc\cross_zone_socks.json`：

```
BUILTIN\Users:(I)(RX)
NT AUTHORITY\Authenticated Users:(I)(M)
```

任何本地用户都能读出密码，经 `cross_zone_entry` 以本节点身份发起跨 Zone 请求，也能改写这个文件。`cross_zone_socks_params.json` 情况相同。

建议：Windows 下创建文件时设置只给 SYSTEM、Administrators 和服务账户的 DACL，并去掉继承。或者把凭据放进只有服务账户可访问的目录。

### R5 用户名/密码比较不是常量时间

fast-socks5 1.0.0 `server.rs:177`：`username == self.username && password == self.password`。入口只监听回环、密码 256 位随机，实际风险很低。如果以后入口对容器开放（见 R3 的建议），可以换成自定义的 `Authentication` 实现，用常量时间比较。

### R6 connect_timeout 只作用于 DIRECT

`SocksServerConfig.connect_timeout` 只在 `process_socket_direct` 里用到（`socks5.rs:269,292`）。PROXY 路径的 `open_stream_by_dest_with_timeout` 用的是 `tunnel_manager.connect_timeout()`（`tunnel_builder.rs:60`），前面的 `get_tunnel`（32-35）也不受这个配置约束。配置项看起来对整个 server 生效，实际不是。

建议：PROXY 路径改用 server 配置的超时，并把 `get_tunnel` 和 `open_stream` 放进同一个连接预算；或者在配置文档里写明这个字段只管 DIRECT。

### R7 配置面与行为不一致，有死代码

- **`rule_config` 和规则引擎**：`SocksServerConfig.rule_config` 和 `SocksProxyConfig.rule_engine` 从未生效。`SocksServerFactory` 固定写 `rule_engine: None`（`cyfs-socks/src/server/server.rs:155`），`SocksProxyConfig::load_rules` 没有调用方，`rule/` 下的规则引擎和 PAC 实现整块未使用。
- **`enable_tunnel`**：`enable_tunnel` 一路传到 `TunnelManager::get_tunnel`，那里参数名是 `_enable_tunnel`（`tunnel_mgr.rs:349`），被忽略了。
- **gateway-lib 的 `Socks5Server`**：它是空壳。`serve_connection` 直接返回 Ok，`id()` 是 `todo!()`，却通过 `server/mod.rs:35` 公开导出（`cyfs-gateway-lib/src/server/socks5_server.rs`）。真正的实现是 cyfs-socks 的 `SocksServer`。

建议：正式化之前，要么把这些接上，要么删掉，让配置面和行为一致。

### R8 接口能力边界未成文

以下行为是实现事实，正式接口文档应写明：

- 只支持 CONNECT，UDP ASSOCIATE 和 BIND 回 `CommandNotSupported`（`socks5.rs:111-124`）；
- 认证只有单组用户名/密码，或无认证；
- 回复里的 BND.ADDR 固定是 `0.0.0.0:0`（`socks/util.rs:45`）；
- hook 里的 `REQ` 只有 `inbound`、`target{type, ip|host, port, addr}`、`source_ip`、`source_port`，来源取自 `StreamInfo.src_addr`。按 TCP 对端过滤只能在入口 stack 用 `conn_source_ip` 做，SOCKS hook 里看不到；
- 除 R3 所述的入口过滤外，网关不知道调用方是谁：远端看到的永远是本节点设备身份，调用方身份要靠应用层（App session token、跨 Zone 登录断言）自行携带；
- R2 所述的成功应答语义。

## 3. 已核实没有问题的部分

- **连接之间互相隔离**：每个连接都由 `prepare_exec_lib` 新建全局 `Env`，`add_external("socks")` 挂在各自的 env 上，并发连接不会串 `REQ`（`cyfs-process-chain/src/hook_point/hook_point.rs:107-124`、`chain/exec.rs:151`）。
- **不会污染已有隧道**：RTCP tunnel 缓存 key 包含 bootstrap URL（`rtcp.rs:1557-1572`），客户端指定的中转不会影响别人到同一设备的已有隧道。
- **应答时机**：本地这一段是 tunnel 建好、stream 打开之后才回 `Succeeded`，失败时回 `GeneralFailure`（`socks5.rs:343-357`）。
- **跨 Zone 入口的凭据**：256 位随机数，unix 下 0600，先写临时文件再 rename（原子写入）；凭据保持稳定，不打断正在运行的客户端；入口只监听回环。
- **远端兜底**：远端 `node_rtcp` 的 `external` 块把非本 Zone 来源固定到 80 → `server node_gateway`、443 → 本地 TLS，其他端口拒绝，跨 Zone SOCKS 流量到不了远端节点的其他服务。

## 4. 建议处理顺序

1. **R1**：默认拒绝。改动小，cyfs-socks 内部即可完成，补测试。
2. **R3**：收紧绑定地址并加凭据。需要同时改 boot_gateway.yaml、node_daemon 的凭据生成，以及 agent 容器的凭据挂载。
3. **R4**：Windows ACL，和 R3 的凭据生成一起做。
4. **R2**：涉及 RTCP 协议，单独设计。之前先写进客户端文档，并评估 `CrossZoneClient` 对 POST 的回退策略。
5. **R6、R7、R8**：正式化接口文档时一并处理。
