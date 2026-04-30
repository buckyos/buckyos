# Boot Gateway 配置生成逻辑

本文档定义 BuckyOS boot 阶段 gateway 配置、Boot 后网络观测上报，以及 scheduler 按节点生成 RTCP route 的规则。`doc/arch/gateway/Zone集群化.md` 解释整体思路，本文偏实际设计，用于指导下一步实现。

cyfs-gateway 上游已经完成 group forward 和 tunnel URL 状态查询后，BuckyOS 侧的重点是生成显式 route candidates，并把它们交给 cyfs-gateway 执行转发、失败降级和状态判断。

## 当前实现基线

当前 BuckyOS 的 gateway 配置分为三层：

1. `src/rootfs/etc/boot_gateway.yaml`
   - 静态 boot 配置。
   - 定义 `node_rtcp`、`zone_gateway_http`、`node_gateway_http` 和 `node_gateway` HTTP server。
   - 从 `node_gateway_info.json` 读取 `APP_INFO`、`SERVICE_INFO`、`NODE_ROUTE_MAP`、`TRUST_KEY`、`NODE_INFO`。
2. `nodes/<node>/gateway_info`
   - scheduler 按 source node 生成。
   - node-daemon 拉取后落地为 `$BUCKYOS_ROOT/etc/node_gateway_info.json`。
   - 当前包含 `node_info`、`app_info`、`service_info`、`node_route_map`、`trust_key`。
3. `nodes/<node>/gateway_config`
   - scheduler 按 source node 生成。
   - node-daemon 拉取后落地为 `$BUCKYOS_ROOT/etc/node_gateway.json` 并触发 cyfs-gateway reload。
   - 当前主要承载 zone TLS、ACME、静态 web dir server 等后置配置。

当前 `node_route_map` 由 scheduler 根据 `devices/*/info` 生成，格式是：

```json
{
  "ood2": "rtcp://ood2.example.zone/",
  "node1": "rtcp://node1.example.zone:2981/"
}
```

这只表达了单一路径，无法表达从不同 source node 出发时不同的直连、SN relay、ZoneGateway relay、多端口、多优先级和失败降级。

当前 cyfs-gateway 已支持以下能力：

- RTCP `keep_tunnel` 配置和 `--keep_tunnel` 启动参数。
- RTCP `on_new_tunnel_hook_point` 准入控制。
- RTCP nested remote/bootstrap URL，使 `Node -> SN/ZoneGateway -> Target` 这类 relay route 可以被正式表达：

```text
rtcp://<percent-encoded bootstrap URL>@<target-did>[:port]/<target-stream>
```

- group forward：
  - 单 URL `forward <url>` 仍是基础 primitive。
  - 多 candidate 可表达 primary / backup / weight / `next_upstream`。
  - 执行阶段可在连接失败、tunnel open 失败、超时等安全边界内尝试下一个 candidate。
  - process-chain 可动态构造 group，而不是只能引用静态 upstream。
- tunnel_mgr 基于 URL 的状态查询：
  - 可查询单个或一组 Tunnel URL 的 `Reachable` / `Unreachable` / `Unknown` / `Probing` / `Unsupported` 状态。
  - 可返回 RTT、失败原因、状态来源、缓存新鲜度和排序结果。
  - keep_tunnel、业务建链和主动 probe 的结果会进入 URL history。

因此 BuckyOS 不再需要在 process-chain 外部手写“选一个 URL 后失败再重试”的逻辑，也不需要通过日志或协议私有状态判断 tunnel 是否就绪。

## Boot 前特殊阶段与 Boot 后常态阶段

Boot 前是系统的特殊阶段：system-config 可能还不可用，scheduler 产物可能不存在或不完整，节点只能依赖本机身份、`ZoneBootConfig`、Finder cache、SN/ZoneGateway 等少量可信输入。这个阶段的目标是最小可达性，不追求完整拓扑最优。

Boot 前生成的 route candidate 具有临时性：

- 只覆盖 OOD、ZoneGateway、SN 和 system-config 相关关键路径。
- direct 与 relay 可以并发探测，但 direct 的静态优先级仍高于 relay。
- relay 先成功只能用于 provisional 可达性，不能固化为长期主路径。
- readiness 只检查显式 URL 集合，不能让 gateway 自动发明未配置路径。

Boot 后进入常态阶段：node-daemon 周期性上报 DeviceInfo，scheduler 读取 `devices/*/info` 后按 source node 生成 `nodes/<source>/gateway_info` 和 `nodes/<source>/gateway_config`，node-daemon 再把配置落地到本机 gateway。此时每个节点拿到的是“从自己出发”的 route 表，而不是全 Zone 共用的一张全局 route 表。

常态闭环如下：

```text
node-daemon
  -> 上报 DeviceInfo(network observation + tunnel probe)
  -> system-config: devices/<node>/info

scheduler
  -> 读取 devices/*/info、boot/config、service/app 状态
  -> 按 source node 生成 nodes/<source>/gateway_info
  -> 按 source node 生成 nodes/<source>/gateway_config

node-daemon
  -> 拉取本节点 gateway_info/gateway_config
  -> 写入 node_gateway_info.json / node_gateway.json
  -> reload cyfs-gateway

cyfs-gateway
  -> 只消费本节点配置中的显式 routes
  -> 执行 group forward、failover、tunnel status 查询
```

## 设计目标

BootGateway 的目标是：在 scheduler 产物不可用或不完整时，让 cyfs-gateway 提供最小可用网络能力，并在 scheduler 产物出现后平滑切换到正式配置。

必须满足：

1. OOD 能在 boot 阶段尽量与其它 OOD、SN、ZoneGateway 建立 RTCP tunnel。
2. 非 OOD ZoneGateway 能作为 OOD keep-tunnel 的目标，并在 OOD 连上后访问 system-config。
3. 普通 Node 能在没有完整 system-config 的情况下，尽量通过 ZoneGateway、LAN OOD、SN relay 连接到 system-config。
4. 访问 Zone 内服务时，优先使用短路径：
   - 本机服务：`127.0.0.1:<port>`
   - 远端服务直连：`rtcp://<target-device>/...`
   - 远端服务 relay：`rtcp://<encoded-bootstrap>@<target-device>/...`
5. `boot_gateway.yaml` 能从 `routes` 动态构造 group forward，让 direct candidate 作为 primary，relay candidate 作为 backup。
6. boot readiness 能通过 tunnel_mgr 按 URL 查询 keep_tunnel 和 OOD candidate 状态，而不是依赖日志或私有协议状态。
7. scheduler 后续生成的正式 `gateway_info/gateway_config` 可以接管 boot 期产物。

不在本阶段解决：

- RTCP 协议本身的加密、握手、anti-replay 逻辑。该部分以 cyfs-gateway `doc/rtcp.md` 为准。
- 大规模 OOD quorum 的一致性协议。本文只定义 gateway 层需要的连接与 route 产物。
- 完整 LAN discovery 协议。本文只定义它产生的 route 信息如何被消费。

## 关键约束

### 3180 绑定

`node_gateway_http` 当前绑定 `0.0.0.0:3180`，这是 Docker/容器访问约束导致的现实要求，不能简单改成 `127.0.0.1`。

因此安全边界不能依赖 bind address，必须由以下机制承担：

- 主机防火墙或部署环境限制外部访问 3180。
- `node_gateway` HTTP server 对 service/app 做鉴权与 RBAC。
- RTCP tunnel 建立前使用 `on_new_tunnel_hook_point` 做来源准入。
- 对敏感 service 保持服务自身鉴权，不把 3180 视为可信入口。

### route 选择职责

BuckyOS 不应在配置层手写 IP 级别的 `ipv4 > ipv6` 竞速逻辑。当前 cyfs-gateway/name-client 已负责 DID 地址解析、候选 IP 排序和 Happy Eyeballs 风格连接竞速。

BuckyOS 负责生成“路径候选”，并且这些候选必须按 source node 下发：

- direct RTCP candidate
- via SN candidate
- via ZoneGateway candidate
- local LAN discovery candidate

同一个 target node 对不同 source node 可以有不同候选路径。例如同 LAN source 可以把 direct 放在 primary，Zone 外 source 则可能只有 via ZoneGateway 或 via SN relay。

具体某条 RTCP direct candidate 内部使用哪个 IP，由 RTCP/name-client 决定。

cyfs-gateway 负责消费这些显式候选：

- process-chain 根据 `routes[target_node]` 构造 group forward。
- forward executor 按 primary / backup / weight / `next_upstream` 执行连接阶段 failover。
- tunnel_mgr 只对 BuckyOS 传入或配置允许的 URL 集合做状态查询和排序，不自动扩散到未配置的 relay 或 direct path。

### Boot 阶段连接调度

Boot 阶段必须区分“探测并发”和“路径优先级”：

- SN keep tunnel、Finder/LAN discovery、已知 IP 的 OOD direct 连接、relay 连接可以同时启动。
- route 选择不能按“谁先成功谁优先”固化。relay 往往比 direct 更早可用，但它只能作为 bootstrap/兜底路径。
- 对同一个目标 OOD，direct candidate 的长期优先级必须高于 via SN/ZoneGateway relay candidate。
- relay tunnel 成功后可以临时满足 boot 可达性，避免系统卡死；但 Finder 或 name-client 后续发现 direct endpoint 后，必须继续尝试 direct，并在 direct tunnel 成功后让业务 route 优先走 direct。
- 如果 direct tunnel 断开，可以降级回 relay；降级后仍应周期性或由 discovery 事件触发 direct 重试。

因此 Boot 阶段的推荐状态机是：

```text
unknown
  -> probing_direct + probing_relay
  -> relay_ready(provisional)
  -> direct_ready(preferred)
  -> relay_ready(degraded)    # direct 失效时降级
```

这解决一个常见问题：直连依赖 Finder 或 DID/name 解析补齐真实 IP，relay 可能一步成功。如果实现只以首次成功路径作为唯一 route 或 keep tunnel 目标，就会退化成“中转优先”。正确行为是“连接探测并发、业务转发直连优先、relay 保持可用兜底”。

## DeviceInfo 网络观测模型

Boot 后 scheduler 构造 per-node route candidate 的主要输入是 `devices/*/info`。node-daemon 上报的 DeviceInfo 应携带更多网络观测信息，但这些信息只能表达事实和 probe 证据，不能直接表达最终 forward-plan。

推荐新增 `network_observation` 结构：

```json
{
  "network_observation": {
    "generation": 12,
    "observation_id": "sha256:...",
    "changed_at": 1710000000,
    "observed_at": 1710000030,
    "rtcp_port": 2980,
    "ipv6": {
      "state": "egress_ok",
      "probe_target": "ipv6.test.example",
      "last_probe": 1710000030,
      "failure_reason": null
    },
    "endpoints": [
      {
        "ip": "192.168.1.23",
        "family": "ipv4",
        "scope": "lan",
        "source": "system_interface",
        "observed_at": 1710000030
      },
      {
        "ip": "2001:db8::23",
        "family": "ipv6",
        "scope": "global",
        "source": "system_interface",
        "observed_at": 1710000030
      }
    ],
    "direct_probe": [
      {
        "target_node": "ood1",
        "kind": "rtcp_direct",
        "url": "rtcp://ood1.example.zone/",
        "status": "reachable",
        "rtt_ms": 12,
        "last_probe": 1710000030,
        "last_success": 1710000030,
        "failure_reason": null,
        "source": "tunnel_mgr"
      }
    ]
  }
}
```

字段语义：

- `generation`：本节点网络观测递增版本。只要 route-relevant 信息发生变化就递增。
- `observation_id`：对 endpoint、IPv6 能力、关键 probe 结果等稳定字段计算的 hash。scheduler 可用它判断是否需要重算。
- `changed_at`：本节点认为网络环境最近一次变化的时间。
- `observed_at`：本次上报时间。scheduler 仍应自行比较旧值，不能只依赖节点判断。
- `ipv6.state`
  - `unknown`：未探测。
  - `unavailable`：没有可用 IPv6。
  - `address_only`：有全局 IPv6 地址，但未验证可访问典型 IPv6 目标。
  - `egress_ok`：能访问典型 IPv6 目标。
  - `rtcp_direct_ok`：已验证 RTCP direct 可经 IPv6 到达关键节点。
- `endpoints`：当前可用于构造 direct candidate 的本机地址事实。现有 `ips/all_ip` 可以继续作为兼容输入，但不应把运行时观测长期混入 DID Document 语义。
- `direct_probe`：本节点到 OOD 或关键 Node 的 direct RTCP probe 结果。它是 scheduler 构造 route 的证据，不是最终转发决策。

scheduler 使用 DeviceInfo 时应遵循：

1. DeviceInfo 新鲜时优先使用其 endpoint 和 direct probe 证据。
2. 只有地址、没有成功 probe 时，可以生成低置信 direct candidate，但不应压过已验证的 relay 可达性用于 readiness。
3. direct probe 成功可以提升 direct candidate 的当前排序或 readiness 权重，但不能让 relay 从配置中消失。
4. probe 失败不能直接删除 direct candidate；应结合失败原因、时间新鲜度和 Zone 拓扑决定是否降级。
5. 节点上报的 probe 结果只对“从该节点出发”的 route 有直接意义，不应无条件复用给其它 source node。

## Boot Route 数据模型

`node_route_map` 应被 `target_node_id -> route candidates` 替换。`routes` 是唯一正式数据源，本轮 RTCP 改造是 breaking change，不保留旧 `NODE_ROUTE_MAP` 配置路径。

`routes` 存在于 `nodes/<source>/gateway_info` 中，语义是“从 `<source>` 出发访问 target node 的候选路径”。它不是全局共享拓扑表。

目标结构：

```json
{
  "node_info": {
    "this_node_id": "node1",
    "this_zone_host": "example.zone"
  },
  "routes": {
    "ood2": [
      {
        "id": "direct",
        "kind": "rtcp_direct",
        "priority": 10,
        "weight": 100,
        "backup": false,
        "keep_tunnel": true,
        "url": "rtcp://ood2.example.zone/",
        "source": "system_config",
        "evidence": {
          "type": "direct_probe",
          "source_node": "node1",
          "last_success": 1710000030,
          "rtt_ms": 12
        }
      },
      {
        "id": "via-sn",
        "kind": "rtcp_relay",
        "priority": 30,
        "weight": 100,
        "backup": true,
        "keep_tunnel": true,
        "url": "rtcp://rtcp%3A%2F%2Fsn.example.org%2F@ood2.example.zone/",
        "relay_node": "sn.example.org",
        "source": "zone_config"
      }
    ]
  }
}
```

字段说明：

- `node_info.this_node_id`：当前配置所属 source node。
- `routes`：当前 source node 的正式 route candidate 列表。
- 同一 target 的候选列表必须稳定排序：`rtcp_direct` 优先于 `rtcp_relay`；连接成功时间不能改变静态优先级，只能影响当前可用性。
- `kind`
  - `rtcp_direct`：直接连接目标 device RTCP stack。
  - `rtcp_relay`：通过 SN 或 ZoneGateway 建立 bootstrap-backed RTCP tunnel。
  - `local`：本机服务，不进入 `routes`，由 selector 命中本机时直接走 `127.0.0.1:<port>`。
- `priority`：数值越小优先级越高。
- `weight`：同一优先级、同一 primary/backup 分组内的转发权重，默认继承 service selector 中该 provider 的 weight。
- `backup`：写入 group forward 时是否进入 backup peers。relay 默认是 backup；只有明确声明 relay 可以作为主路径时才设置为 `false`。
- `keep_tunnel`：boot 阶段是否需要对该 route 建立或维持后台 tunnel。
- `source`
  - `zone_boot_config`
  - `zone_config`
  - `system_config`
  - `lan_discovery`
  - `manual`
- `evidence`：可选字段，记录该 candidate 的生成依据，例如 DeviceInfo direct probe、Finder cache、ZoneBootConfig OOD 描述、SN/ZoneGateway 配置。它用于诊断和后续调度，不参与 gateway 鉴权。

`node_gateway_info.json` 中只写入 `ROUTES`。process-chain 必须使用 `ROUTES` 构造 group forward；如果目标 node 没有 route candidate，应返回明确错误，不再降级到 `NODE_ROUTE_MAP`。

## RTCP URL 生成规则

### Direct route

默认 RTCP 端口：

```text
rtcp://<device-did-host>/
```

非默认 RTCP 端口：

```text
rtcp://<device-did-host>:<rtcp-port>/
```

其中 `<device-did-host>` 优先使用可被 name-client 解析的 device DID hostname，例如：

```text
ood2.test.buckyos.io
```

### Relay route

通过 relay 节点建立到 target 的 RTCP tunnel 时，使用 cyfs-gateway 当前 nested remote/bootstrap URL：

```text
rtcp://<percent-encoded bootstrap URL>@<target-device-did-host>[:target-rtcp-port]/
```

示例：

```text
rtcp://rtcp%3A%2F%2Fsn.devtests.org%2F@ood2.test.buckyos.io/
```

语义：

1. 先通过 `rtcp://sn.devtests.org/` 建立 bootstrap stream。
2. 再在该 stream 上与 `ood2.test.buckyos.io` 建立外层 RTCP tunnel。
3. 外层 RTCP 身份认证仍以 target device DID 为准，relay 不参与 target 身份认证。

不再使用旧式模糊表达：

```text
rtcp://relay/rtcp://target/
```

## process-chain 消费规则

`boot_gateway.yaml` 的 `forward_to_service` 和 `forward_to_app` 应把本节点 `gateway_info.routes` 转换为 cyfs-gateway group forward：

1. 如果 service/app selector 命中 `THIS_NODE_ID`，直接 `forward "http://127.0.0.1:<port>"`。
2. 如果目标在远端 node，遍历 `TARGET_SERVICE_INFO.selector` 或 `TARGET_APP_INFO.selector`。
3. 对每个 provider node：
   - 优先读取本 source node 配置中的 `ROUTES[node_id]`。
   - 将 `backup = false` 的 route 放入 primary peers。
   - 将 `backup = true` 的 route 放入 backup peers。
   - 拼接 service/app port 后得到最终 URL，例如 `rtcp://ood2.example.zone/:3202`。
   - peer weight 默认使用 provider weight；route 上显式 weight 可作为乘数或覆盖值，具体实现保持一致即可。
4. 调用 group forward，启用 `next_upstream error,timeout` 和有限 tries。
5. 如果 `ROUTES[node_id]` 缺失或候选列表为空，返回 route missing 错误，不做旧字段降级。

逻辑形态如下：

```text
map-create primary_peers
map-create backup_peers

for node_id, node_info in TARGET_SERVICE_INFO.selector:
  if node_id == THIS_NODE_ID:
    forward "http://127.0.0.1:${node_info.port}"
  else if ROUTES contains node_id:
    for route in ROUTES[node_id]:
      target_url = append_port(route.url, node_info.port)
      if route.backup:
        map-add backup_peers target_url node_info.weight
      else:
        map-add primary_peers target_url node_info.weight
  else:
    return route_missing(node_id)

forward --group-map primary_peers --backup-map backup_peers --next-upstream error,timeout --tries 3
```

状态查询不应替代 group forward 的执行阶段失败处理。推荐分工是：

- boot readiness、诊断页面和调度刷新使用 tunnel_mgr 批量查询 URL 状态。
- 单次业务请求由 group forward 在连接阶段执行 failover。
- tunnel_mgr 查询结果只能影响显式候选的排序或 readiness 判断，不能生成新的隐式候选。

process-chain 不应读取 system-config 或其它节点的 `gateway_info` 做二次调度。它只能使用 node-daemon 已经落地到本机的 `node_gateway_info.json`。

## 角色启动流程

### OOD

输入：

- 本机 device doc/private key。
- `ZoneBootConfig`。
- 本地缓存的 LAN discovery 结果。
- 可选 SN 信息。

Boot 阶段行为：

1. 启动 `cyfs_gateway`。
2. 启动 Finder，持续发现其它 OOD 的 LAN endpoint，并读取本地 Finder cache 作为初始 direct endpoint。
3. 根据 `ZoneBootConfig.oods` 为其它 OOD 生成 direct route candidates（如果有）。
4. 如果存在 SN，生成 via SN relay candidates，并按本机网络形态决定是否 keep tunnel to SN。
5. 如果 `ZoneBootConfig` 标记了 ZoneGateway 节点，生成 via ZoneGateway relay candidates。
6. 为其它 OOD 生成带 relay 的 tunnel URL，并把 direct route 标记为 primary、relay route 标记为 backup。
7. 把需要长期保持的目标写入 RTCP `keep_tunnel`：
   - 其它 OOD direct candidate 或 direct 目标 DID。
   - SN candidate，前提是本机不是稳定 WAN 可达。
   - ZoneGateway candidate，前提是该节点承担 relay/公网入口职责。
8. 通过 cyfs-gateway 的 tunnel_mgr 批量查询接口，判断是否有足够多到其它 OOD 的 tunnel 已经建立。`2n+1` OOD 系统至少需要与 `n` 个其它 OOD 的 keep_tunnel 达到 `Reachable`，或在策略允许时达到 `Unknown/Probing` 并继续后台探测。
9. 访问 `127.0.0.1:3180/kapi/system_config` 并获得 boot/config，这是进入后续 boot 流程的 gateway 闸门。
10. 只有 OOD 列表中的第一个 OOD 有资格执行 boot/config 构造，其它 OOD 等待该配置出现。

多 OOD quorum 的一致性判断仍由 system-config/klog 设计定义。gateway 层只负责提供 route、keep_tunnel 和 URL 状态查询能力，不把 relay 先成功反写为更高静态优先级。

### 非 OOD ZoneGateway

输入：

- 本机 device doc/private key。
- `ZoneBootConfig` 或 ZoneGateway 注册信息。
- 可选 SN 信息。

Boot 阶段行为：

1. 启动 `cyfs_gateway`，允许 OOD 建立 tunnel。
2. 使用 `on_new_tunnel_hook_point` 限制只有同 Zone OOD 或受信设备可建立 tunnel。
3. 一旦有 OOD tunnel 建立成功，优先通过该 tunnel 访问 system-config。
4. 如果长时间没有 OOD 连入，可主动尝试：
   - direct OOD route
   - via SN route
   - via other ZoneGateway route
5. 通过 tunnel_mgr 查询 OOD candidate 的状态，用可解释的 URL 状态判断当前是否应继续等待、尝试主动连接或走 relay 降级路径。
6. 访问 `127.0.0.1:3180/kapi/system_config` 并获得 boot/config。

非 OOD ZoneGateway 和普通 Node 的区别是：它必须先作为 OOD 的 relay/target 存在，不能只作为 system-config client。

### 普通 Node

输入：

- 本机 device doc/private key。
- zone hostname 或 ZoneBootConfig/ZoneConfig 缓存。
- LAN discovery 缓存。

Boot 阶段目标只有一个：连接上 system-config。

候选路径：

1. ZoneGateway：

```text
https://<zone-host>/kapi/system_config
```

2. 本机 gateway 到 OOD direct：

```text
http://127.0.0.1:3180/kapi/system_config
```

底层 route candidate：

```text
rtcp://<ood-device>/:3200
```

3. 本机 gateway 经 SN 到 OOD：

```text
rtcp://<encoded-sn-bootstrap>@<ood-device>/:3200
```

4. LAN discovery 得到的 OOD direct candidate。

普通 Node 当前实现缺口较大：node-daemon 的非 OOD `get_system_config_client()` 仍需要实现，且需要能使用 boot route candidates。

## scheduler 接管流程

scheduler 启动后，根据 system-config 为每个 source node 生成正式产物：

- `nodes/<source>/gateway_info`
  - `node_info`
  - `app_info`
  - `service_info`
  - `routes`
  - `trust_key`
- `nodes/<source>/gateway_config`
  - zone TLS stack
  - ACME 配置
  - 静态 web dir server
  - 后续需要时可包含 RTCP on_new_tunnel 策略和 keep_tunnel 配置

scheduler 的 route 构造流程：

1. 读取 `boot/config`，获得 OOD、ZoneGateway、SN 和 Zone 基础身份信息。
2. 读取所有 `devices/*/info`，获得各节点 DeviceInfo、网络观测、IPv6 能力、endpoint 和 direct probe 证据。
3. 读取服务实例状态，生成 `ServiceInfo` / `AppInfo` selector，确定哪些 target node 承载 provider。
4. 对每个 source node 单独构造 `routes`：
   - source 与 target 相同：不进入 `routes`，由 selector 命中本机时直接走 `127.0.0.1:<port>`。
   - source 对 target 有新鲜 direct probe 成功：生成 direct primary candidate。
   - target 有可解析 DID hostname 或可信 endpoint：生成 direct candidate，置信度由 probe 和 endpoint 新鲜度决定。
   - 存在 SN：生成 via SN relay backup candidate。
   - 存在 ZoneGateway 且策略允许 relay：生成 via ZoneGateway relay backup candidate。
   - LAN discovery/Finder cache 只作为 direct candidate 的 evidence，不应绕过显式 route 写入。
5. 对每个 candidate 写入稳定 priority、backup、keep_tunnel、source/evidence。
6. 写入 `nodes/<source>/gateway_info`，由该 source node 的 node-daemon 拉取。

同一个 target node 的 route 在不同 source node 下可能不同。scheduler 不应把某个节点上报的 direct probe 结果直接当作其它节点也可直连。

node-daemon 负责：

1. 拉取 `gateway_info`，落地为 `node_gateway_info.json`。
2. 拉取 `gateway_config`，落地为 `node_gateway.json`。
3. 检测内容变化后 reload cyfs-gateway。
4. 保留现有 tunnel，由 cyfs-gateway 自行复用或重建；除非配置显式删除某类准入或 route。
5. 配置变化后让 tunnel_mgr 失效受影响 URL 的旧状态；未受影响 URL 的 history 可继续作为排序和诊断输入。

如果 scheduler 产物缺失，cyfs-gateway 应继续使用 boot 阶段 route，不应导致系统网络能力完全消失。

scheduler 生成的正式 `routes` 必须继续保持“显式候选”原则：如果没有写入 relay URL，forward 失败后不能由 gateway 隐式扩散到 relay；如果写入了 relay URL，应明确标记 primary/backup 和 `keep_tunnel` 意图。

## 安全边界

### RTCP tunnel 准入

Boot 阶段必须补充 `node_rtcp.on_new_tunnel_hook_point`。

准入策略：

1. 同 Zone device 默认允许。
2. OOD、ZoneGateway、SN 可按角色加入 allow list。
3. 跨 Zone device 只有在明确存在 trust relationship 时允许。
4. 未携带可验证 `device_doc_jwt` 的来源只能使用较弱字段，如 `source_device_id`，默认不应允许敏感 relay 能力。

可用字段以 cyfs-gateway 当前实现为准：

- `REQ.source_device_id`
- `REQ.source_device_name`
- `REQ.source_device_owner`
- `REQ.source_zone_did`
- `REQ.source_device_doc_jwt`
- `REQ.source_addr`

### relay 权限

RTCP relay 不应默认开放给任意来源。

最低要求：

- SN relay：只允许已认证 OOD/ZoneGateway/同 Zone device 使用。
- ZoneGateway relay：只允许同 Zone device 使用，或显式受信 device 使用。
- 普通 Node 默认不作为 relay，除非配置明确标记。

### 3180 HTTP 入口

因为 Docker 约束，3180 绑定 `0.0.0.0`。实现时必须假定 3180 可能被非本机访问，因此：

- `/kapi/*` 仍必须依赖 service 自身认证和 RBAC。
- gateway process chain 不能因为请求来自 3180 就跳过鉴权。
- 部署脚本或文档应建议通过防火墙限制 3180 外部访问。

## 下一阶段实现任务

cyfs-gateway 的 group forward 和 tunnel_mgr URL 状态查询已经可用，BuckyOS 侧不再需要等待上游能力，后续任务应集中在配置产物、process-chain 接入和 boot readiness。

### 阶段 1：DeviceInfo 网络观测

1. 在 DeviceInfo 中增加 `network_observation`。
2. node-daemon 定期采集 endpoint、IPv6 能力和 RTCP direct probe 结果。
3. `network_observation` 必须能表达网络环境变化：
   - `generation`
   - `observation_id`
   - `changed_at`
   - `observed_at`
4. IPv6 判断必须区分有地址、出站可用和 RTCP direct 可用。
5. direct probe 先覆盖 OOD 和 ZoneGateway 等关键节点，不做服务级 probe。
6. 单元测试覆盖：
   - 网络观测 hash/generation 变化。
   - 有 IPv6 地址但 probe 失败。
   - direct probe reachable/unreachable/stale。

### 阶段 2：per-node route candidate 产物

1. 在 scheduler 的 `gateway_info` 中用 `routes` 替换 `node_route_map`。
2. scheduler 必须按 source node 生成 `nodes/<source>/gateway_info`。
3. route candidate 支持：
   - direct RTCP。
   - via SN RTCP relay。
   - via ZoneGateway RTCP relay。
   - LAN discovery 产生的 direct candidate。
4. 每个 candidate 写入 `priority`、`weight`、`backup`、`keep_tunnel`、`source`。
5. 每个 candidate 可写入 `evidence`，用于解释来源和诊断。
6. 用当前 `devices/*/info`、`boot/config` 和 ZoneGateway/SN 信息生成 route candidates。
7. 不同 source node 到同一 target node 的 routes 可以不同，测试必须覆盖。
8. 单元测试覆盖：
   - 默认 2980 端口。
   - 非默认 RTCP 端口。
   - 有 SN 时生成 relay backup candidate。
   - 有 ZoneGateway 时生成 relay backup candidate。
   - source A direct reachable、source B 只能 relay。
   - route 缺失时返回明确错误。

### 阶段 3：boot route builder

1. 增加 boot route builder，输入 `ZoneBootConfig`、本机角色、本机 device doc、缓存 discovery 结果。
2. 生成 boot 期 `node_gateway_info.json` 或等价临时配置，字段包含 `ROUTES`。
3. 生成 RTCP keep_tunnel 列表，并保证 direct 目标与 relay bootstrap 目标分开表达。
4. node-daemon 启动 cyfs-gateway 时不再只临时传 SN，而是传入或落地完整 keep_tunnel。
5. boot route builder 必须把 direct candidate 排在 relay candidate 前面；relay 的先成功状态只能进入 tunnel_mgr URL history，不能反写成更高静态优先级。

### 阶段 4：process-chain 接入 group forward

1. 修改 `boot_gateway.yaml` 的 `forward_to_service` 和 `forward_to_app`。
2. 当目标不在本机时，从本节点配置中的 `ROUTES[target_node]` 生成 primary / backup forward map。
3. 使用 cyfs-gateway group forward，并启用连接阶段 `next_upstream error,timeout` 和有限 tries。
4. debug tests 覆盖 direct 和 relay URL。
5. debug tests 必须覆盖 relay 先可用、direct 后发现的场景，验证最终业务 route 优先使用 direct。

### 阶段 5：boot readiness 接入 tunnel_mgr

1. OOD 启动时批量查询需要 keep_tunnel 的 OOD / SN / ZoneGateway URL。
2. 非 OOD ZoneGateway 查询 OOD candidate 状态，决定等待 OOD 连入还是主动尝试 direct / relay。
3. 普通 Node 查询 system-config 相关候选 URL，优先走 ZoneGateway，再按配置降级到本机 3180 group forward、LAN discovery 或 SN relay。
4. readiness 逻辑只消费显式 URL 集合；状态查询不能自动生成新的候选路径。
5. 日志必须输出候选 URL、状态、失败原因和最终采用的路径，便于解释“为什么走 relay”。

### 阶段 6：非 OOD Node 启动

1. 实现 node-daemon 非 OOD `get_system_config_client()`。
2. 支持通过本机 `3180` 和 boot route candidate 访问 OOD system-config。
3. 支持 ZoneGateway 失败后降级到 LAN discovery 或 SN relay。

### 阶段 7：RTCP 准入策略

1. 在 boot gateway 中加入 `on_new_tunnel_hook_point`。
2. scheduler 根据 zone/device/trust 信息生成正式准入配置。
3. 增加同 Zone allow、跨 Zone deny、relay deny/allow 的测试。

## 验证要求

每个阶段完成后至少验证：

```bash
cargo test
uv run buckyos-build.py --skip-web
uv run src/test/test_boot_gatweay/run_debug_tests.py
```

涉及 cyfs-gateway RTCP nested URL、keep_tunnel、group forward 或 tunnel_mgr URL 状态查询行为时，还需要在 cyfs-gateway 仓库运行对应测试。

## 需要同步更新的文档

修改实现时必须同步检查：

- `doc/arch/gateway/Zone集群化.md`
- `doc/arch/gateway/zone-boot-config与zone-gateway.md`
- `doc/arch/02_boot_and_activation.md`
- `doc/arch/06_network_and_gateways.md`
- `doc/arch/09_pitfalls.md`
- cyfs-gateway `doc/rtcp.md`
- cyfs-gateway `doc/forward机制升级需求.md`
- cyfs-gateway `doc/tunnel_mgr基于url状态查询需求.md`

route 数据模型从 `node_route_map` 单字符串升级后，所有引用 `NODE_ROUTE_MAP` 的实现、文档和测试都必须删除或改为 `ROUTES`。
