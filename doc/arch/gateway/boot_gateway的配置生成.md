# Boot Gateway 配置生成逻辑

本文档定义 BuckyOS boot 阶段 gateway 配置、Boot 后网络观测上报，以及 scheduler 按节点生成 RTCP route 的规则。`doc/arch/gateway/Zone集群化.md` 解释整体思路，本文偏实际设计，用于指导下一步实现。

cyfs-gateway 上游已经完成 group forward 和 tunnel URL 状态查询后，BuckyOS 侧的重点是生成显式 route candidates，并把它们交给 cyfs-gateway 执行转发、失败降级和状态判断。

## 当前实现基线

当前 BuckyOS 的 gateway 配置分为三层：

1. `src/rootfs/etc/boot_gateway.yaml`
   - 静态 boot 配置。
   - 定义 `node_rtcp`、`zone_gateway_http`、`node_gateway_http` 和 `node_gateway` HTTP server。
   - 从 `node_gateway_info.json` 读取 `APP_INFO`、`SERVICE_INFO`、`ROUTES`（升级前为 `NODE_ROUTE_MAP`）、`TRUST_KEY`、`NODE_INFO`。
2. `nodes/<node>/gateway_info`
   - scheduler 按 source node 生成。
   - node-daemon 拉取后落地为 `$BUCKYOS_ROOT/etc/node_gateway_info.json`。
   - 当前包含 `node_info`、`app_info`、`service_info`、`node_route_map`（升级目标为 `routes`）、`trust_key`。
3. `nodes/<node>/gateway_config`
   - scheduler 按 source node 生成。
   - node-daemon 拉取后落地为 `$BUCKYOS_ROOT/etc/node_gateway.json` 并触发 cyfs-gateway reload。
   - 当前主要承载 zone TLS、ACME、静态 web dir server 等后置配置。

### cyfs-gateway 的运行时独立性

cyfs-gateway 进程的运行**只依赖 `boot_gateway.yaml` 这一份本地配置**。`node_gateway_info.json` 和 `node_gateway.json` 是 boot_gateway.yaml 引用的数据文件，scheduler 通过更新这两份文件来调整转发行为。

由此带来一个重要属性：

- **scheduler 失能不会让 cyfs-gateway 不可用**。当 system-config 不可写、scheduler 崩溃或 node-daemon 拉取失败时，唯一的后果是 `node_gateway_info.json` 停止更新；cyfs-gateway 仍按已落地的最后一份配置继续转发，已建立的 RTCP tunnel、已生效的 routes 和 selector 都保持工作。
- **boot 期与 scheduler 期的"切换"不是替换 cyfs-gateway 配置文件**。两者都是往同一份 `node_gateway_info.json` 写入；boot route builder 写第一版，scheduler 上线后覆盖。process-chain 永远从同一个数据源读取，不区分"boot 模式"和"正常模式"。
- node-daemon 在写入新版本 `node_gateway_info.json` 后通过 reload 通知 cyfs-gateway 重新加载，未受影响的 tunnel 和 selector 状态尽量保留（详见后文"scheduler 接管流程"中的 reload 行为）。

因此本文中说"scheduler 接管"或"配置切换"指的都是数据文件内容更新，不是 cyfs-gateway 重启或启用不同 yaml。

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

Boot 阶段必须区分"探测并发"和"路径优先级"：

- SN keep tunnel、Finder/LAN discovery、已知 IP 的 OOD direct 连接、relay 连接可以同时启动。
- route 选择不能按"谁先成功谁优先"固化。relay 往往比 direct 更早可用，但它只能作为 bootstrap/兜底路径。
- 对同一个目标 OOD，direct candidate 的长期优先级必须高于 via SN/ZoneGateway relay candidate。
- relay tunnel 成功后可以临时满足 boot 可达性，避免系统卡死；但 Finder 或 name-client 后续发现 direct endpoint 后，必须继续尝试 direct，并在 direct tunnel 成功后让业务 route 优先走 direct。
- 如果 direct tunnel 断开，可以降级回 relay；降级后仍应周期性或由 discovery 事件触发 direct 重试。

#### 显式双 candidate，而不是隐式 relay 兜底

需要强调一个边界：Boot 阶段对每一个需要既"快可达"又"长期走 direct"的 target，**显式插入两个 tunnel URL** 到 ROUTES：

```text
ROUTES["ood2"] = [
  { id: "direct",  kind: rtcp_direct, priority: 10, backup: false, url: "rtcp://ood2.example.zone/" },
  { id: "via-sn",  kind: rtcp_relay,  priority: 30, backup: true,  url: "rtcp://<encoded-sn-bootstrap>@ood2.example.zone/" },
]
```

两条都是显式候选，区别只在 `backup` / `priority`。这意味着：

- "relay 先成功"是 forward executor 在 primary 全部 `fail_timeout` 内不可用时尝试 backup peer 的**正常 group forward 行为**，不是 Gateway "在 direct 失败后自动发明 relay"。
- 当 direct 后来探测可用，下一次 attempt 自然回到 primary peer，无需特殊"切回"逻辑。
- 如果 ROUTES 里没有显式写入 relay candidate，那就是真的没有 relay 路径——cyfs-gateway 不会自己去找一个。

因此 readme.md / Zone集群化.md 中的"禁止隐式中转扩散"在 Boot 阶段同样适用，区别只是 boot route builder 是显式构造 candidate 的合法主体。

#### Boot 推荐状态机

```text
unknown
  -> probing_direct + probing_relay
  -> relay_ready(provisional)        # backup peer 在 primary 不可用时被 group forward 选中
  -> direct_ready(preferred)         # primary peer 状态恢复后自然回到 direct
  -> relay_ready(degraded)           # direct 失效时再次进入 backup
```

这解决一个常见问题：直连依赖 Finder 或 DID/name 解析补齐真实 IP，relay 可能一步成功。如果实现只以首次成功路径作为唯一 route 或 keep tunnel 目标，就会退化成"中转优先"。正确行为是"连接探测并发、business forward 始终在显式 primary / backup 集合里按优先级选、direct 可用时自然占据 primary"。

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
5. 节点上报的 probe 结果在大多数情况下只对"从该节点出发"的 route 有直接意义；只有当其它 source 与本节点处于同 LAN 且本次 probe 的目标也在该 LAN 时，scheduler 才可在标注 evidence applicability 后复用（详见下文 evidence 模型）。

## 探测时机

direct probe、tunnel_mgr URL 状态查询、Finder 广播这些动作都有成本。本节明确"什么时候触发探测"，避免实现层把它们退化为高频轮询或反过来变成只在 boot 时跑一次的 dead code。

探测触发分为三类：

### 1. 事件触发（必须立即跑）

- **boot 阶段启动**：node-daemon / cyfs-gateway 起来后立即对 keep_tunnel 集合（OOD / SN / ZoneGateway）和 system-config 入口候选发起一次 probe。
- **网络环境变化**：node-daemon 检测到 `network_observation.generation` 递增（IP 列表变化、接口 up/down、默认网关切换）后，立即对本节点出发的所有 keep_tunnel candidate 重测，并刷新 DeviceInfo 上报。这是"笔记本换网络"场景的核心触发点。
- **scheduler 配置变化**：node-daemon 拿到新版 `nodes/<source>/gateway_info` 时，对其中新增或 URL 变化的 candidate 立即跑一次 probe，把结果回写 tunnel_mgr，让下一次业务请求能基于新鲜状态选路。
- **大流量业务启动前的强制刷新**：备份等成本敏感业务在启动前可显式触发一次 direct probe，没有 direct 时直接失败提示用户，不允许默认回落 relay。这条由业务侧调用，不是周期任务。

### 2. 周期触发（节流后台跑）

- **keep_tunnel 长连接**：tunnel 自身的 ping/pong 已提供持续的可达性信号，tunnel_mgr 直接消费。**不再额外跑周期 probe**。
- **节点级 direct probe**：对"在 keep_tunnel 集合外、但 scheduler 在 routes 里写过的 target node"，按 source × target 矩阵周期性跑。家用规模 n ≤ 10 时矩阵规模可控；建议 boot 阶段 30s 一轮，平稳阶段 5–10 分钟一轮。
- **Finder 广播**：仍按 `FINDER_BROADCAST_INTERVAL_SECS = 2s` 进行，作为 LAN 内 OOD 发现的低成本背景流量。
- **DeviceInfo 上报**：node-daemon 周期上报 DeviceInfo 到 system-config，间隔与 `network_observation` 内的最快字段对齐（建议 30–60s）；变化触发与周期触发取较短者。

### 3. 按需触发（selector 没得选时）

- forward executor 在某 group 全部 candidate 都被 `fail_timeout` 标记不可用时，可调用 tunnel_mgr 的 URL 状态查询接口做一次同步 probe（cyfs-gateway `forward机制升级需求.md` 阶段 4 的 `apply_least_time_via_tunnel_mgr` 路径），刷新候选状态。这种 probe 必须有预算限制（默认 50ms），超时退化为现有 candidate 顺序。
- runtime / Client Device 在拿到 ServiceInfo + DeviceInfo 后做一次性 URL probe，决定是否走 direct。这类 probe 的结果不回写 system-config，只用于本次会话。

### 反模式

下面这些实现选择是被明确反对的：

- 服务级周期 probe（每个 service 都跑一遍 source × target × url 矩阵）。**节点级探测优先于服务级探测**，service 级别只在 selector 找不到可用 URL 时按需触发。
- 用 probe 失败直接删除 direct candidate。失败只能降低当前排序，candidate 集合由 scheduler / boot route builder 决定。
- 把 probe 结果写回静态 priority。priority 是稳定排序锚（direct < relay），不被运行时探测覆盖。

## scheduler 的确定性选路输入

不是所有路径都需要靠 probe 决定。下面三类信息允许 scheduler 在 **不发起任何 probe** 的情况下生成正确的 candidate，省掉一次往返开销，也让 routes 在节点刚加入或网络刚变化时就能立刻可用。

### 1. net_id 推断

DeviceInfo / DeviceConfig 中的 `net_id` 是设备主人在激活时给出的网络位置语义，scheduler 直接消费，不再二次验证。规则：

- **填了 net_id 就信任它**。`wan` / `wan_dyn` / `portmap` / `nat` / `lan*` 等取值是"事实级"输入，scheduler 据此生成 candidate；不需要 probe 来验证 device 是否真的处于该网络位置。
- **不填 = unknown，默认推断为 LAN**。如果设备真在公网，激活流程会显式打 `wan` 系标签；不填可以肯定它**不是公网**，因此兜底视为某个 LAN。具体在哪个 LAN 由 `network_observation.endpoints.scope == lan` 与子网一致性判定。
- 两个 device 的 net_id 字面相同（比如都标 `lan1`）时，scheduler 视为同 LAN。

scheduler 可以从 net_id 直接推出的确定性决策（**不需要 probe**）：

| source net_id | target net_id | scheduler 行为 |
| --- | --- | --- |
| 同值（含都为 unknown） | 同值 | 生成 direct primary candidate，applicability 标 `same_lan` |
| 任意 | `wan` / `wan_dyn` / `portmap` | 任意 source 都生成 direct candidate（target 公网可达） |
| `nat` / `lan*` / unknown | 不同的 `lan*` 或 `nat` | direct 不可行，只生成 relay candidate（source 无公网出口、target 也不在同 LAN） |
| `wan` 系 | `nat` / `lan*` | 一般情况下 direct 不可行（target 在 NAT 后）；只有 target 显式标 `portmap` 才生成 direct candidate |

这意味着家用最常见的"同 LAN 全直连"和"Zone 外节点 → NAT 后节点必走 relay"两类拓扑可以由 scheduler 在第一次生成 routes 时就给出正确答案，不需要等 probe 收敛。

### 2. DeviceConfig 中的签名 IP

当 `DeviceConfig.ips` 非空、且 device_doc 由 owner 私钥签名时，这些 IP 是"**带签名的事实**"——设备主人在配置 device 时已经百分之百确定它拥有固定 IP（典型场景：公网 ZoneGatewayNode、固定 IP 的家用 OOD）。

scheduler 对带签名 IP 的处理：

- **优先级最高**。相关 direct candidate 的 evidence `confidence = high`、`applicability = zone_wide`、`freshness_ttl_secs` 视同"长期有效"，所有 source 都可参考。
- **不再启动 IP 级探索**。name-client 在已有签名 IP 时不应额外加 DNS 解析候选；Finder / LAN discovery 的发现结果不应覆盖签名 IP，只能作为 evidence 旁路记录。
- **不需要 keep_tunnel via SN**。带签名 IP 通常意味着 device 不是动态网络，没必要为它单独维持 SN relay keep_tunnel。
- **是一道安全边界**。如果 scheduler 总是探测一切候选 IP，攻击者可以通过 DNS 投毒、广播伪造等方式注入虚假 IP；信任签名 IP 而不再尝试其它候选意味着这条攻击路径被堵死。

实现要点：

- 写入 candidate 时优先用签名 IP 直接构造 `rtcp://<ip>:<port>/`，而不是依赖 DID hostname 解析。
- 同一 device 同时有签名 IP 和 DID hostname 时，签名 IP 形态进 primary，hostname 形态可作为 evidence/诊断保留，不进 forward 候选。
- 签名 IP 不可达时不应 fallback 到任意"探测得到的 IP"——可能就是攻击者注入的；fallback 只能走显式 relay candidate。

### 3. 通过共同 OOD 推断同 LAN

direct probe 的结果可以横向使用，避免 device 之间互测：

```text
若 device A 能 direct 连上 OOD1
   且 device B 能 direct 连上 OOD1
   且 OOD1 不在公网（net_id 不以 wan 开头）
   则 device A 与 device B 大概率位于同一 LAN
```

依据是：非公网 OOD 通常是 LAN 内的固定节点；能 direct 连上它的两个 device 最自然的解释就是与 OOD 同处一个 LAN，因此彼此之间也可以 direct。如果 OOD 本身是 `wan` 系，该推断不成立——A、B 只是都有公网出口、未必能互通。

scheduler 对该推断的使用：

- 仅在 OOD `net_id` **不是** `wan` / `wan_dyn` / `portmap` 时启用。
- 同一聚类内的 device 互相生成 direct candidate，evidence 标记：
  - `type = "co_ood_inference"`
  - `applicability = "same_lan"`
  - `confidence = "medium"`（未经直接验证，需要后续业务建链回写升级到 high）
- 推断 candidate 仍是显式写入的，priority 与"自己 probe 得到的 direct"持平或低半档，写入后由 forward executor 在业务建链时通过 tunnel_mgr URL history 自然升级或降级。
- 聚类内若有 device 持有签名 IP，优先按签名 IP 形态构造 candidate；其它 device 用 `network_observation.endpoints` 拼 URL。

实现位置：scheduler 在每轮生成 routes 前先扫一遍 `devices/*/info` 中的 `direct_probe`，按 target = 非 wan OOD 做共同节点聚类，再把聚类信息喂给 routes 构造流程。这样省掉了 device 数量级的 n² probe，把成本压在 OOD 数量级上。

### 与 probe 的优先级关系

三类确定性输入和 probe 不是互相替代，而是分层叠加：

1. 签名 IP（最高）→ 直接生成 candidate，scheduler 不发起探测；
2. net_id 推断 → 决定是否生成 direct/relay candidate，evidence confidence = medium；
3. 共同 OOD 推断 → 在前两者没结论时填补 device 之间的 direct candidate，evidence confidence = medium；
4. 主动 direct probe / 业务建链回写 → 升级 evidence confidence 到 high，影响当前排序但不改变静态 priority。

scheduler 在每轮构造 routes 时按这个顺序消费输入：先吃确定性信号，再用 probe 结果做加成；得不到确定性信号的位置才依赖周期 probe 收敛。

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
- `evidence`：可选字段，记录该 candidate 的生成依据，例如 DeviceInfo direct probe、Finder cache、ZoneBootConfig OOD 描述、SN/ZoneGateway 配置。它用于诊断和后续调度，不参与 gateway 鉴权。结构如下：

  ```json
  {
    "type": "direct_probe",
    "source_node": "node1",
    "last_success": 1710000030,
    "rtt_ms": 12,
    "confidence": "high",
    "freshness_ttl_secs": 600,
    "applicability": "source_node"
  }
  ```

  - `confidence`
    - `high`：tunnel_mgr 主动 probe 或 forward executor 业务建链回写的成功记录。
    - `medium`：DeviceInfo 中的 endpoint 事实、Finder cache 命中。
    - `low`：仅来自 ZoneBootConfig / ZoneConfig 的静态描述，未经探测验证。
  - `freshness_ttl_secs`：该 evidence 的有效期。常用值：
    - direct_probe / 业务建链：300–600s。
    - LAN 发现 / Finder cache：≥ TTL（默认 7 天）。
    - ZoneBootConfig / ZoneConfig：长期有效，scheduler 不应据此让 candidate 过期。
    - 超过 TTL 的 evidence 仍可保留供诊断，但不再用于提升 candidate 排序。
  - `applicability`
    - `source_node`：仅对 `evidence.source_node` 出发的 routes 有意义（默认）。
    - `same_lan`：可被同 LAN 的其它 source 复用（按 `network_observation.endpoints.scope == lan` 且子网一致判定）。
    - `zone_wide`：来自 ZoneConfig 等全局事实，所有 source 都可参考。
    - applicability 描述"这条证据可以被哪些 source 借用"，scheduler 在为别的 source 生成 routes 时查这个字段决定是否复用。

  evidence TTL 过期不直接删除 candidate（candidate 由 scheduler/boot route builder 决定是否存在），只会让它退回到"无业务级 evidence 加成"的默认排序。

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

1. 如果 service/app selector 命中 `THIS_NODE_ID`，直接 `forward "tcp:///127.0.0.1:<port>"`。
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
    forward "tcp:///127.0.0.1:${node_info.port}"
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
Boot成功后，立刻停止Finder流程

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

1）大部分情况下，只有ZoneGateway可用立刻就能成功
2）Node的正常流程，需要和至少1个，至多2个OOD Keep-tunnel. 可以复用前面的“在局域网找OOD”的流程，以实现“即使ZoneGateway失效，Zone内服务也可正常"

候选路径：

1. ZoneGateway：

```text
https://<zone-host>/kapi/system_config
```

2. 本机 gateway 到 OOD （

```text
http://127.0.0.1:3180/kapi/system_config
```

其底层 route candidate可能是两个（需要在boot的时候写入tunnel url才有可能连接成功)

```text
rtcp://<ood-device>/:3200
```

```text
rtcp://<encoded-sn-bootstrap>@<ood-device>/:3200
```


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

#### 准入与 system-config 的解耦

device_doc 的可信性不依赖 system-config：

- 每个 device 的 `device_doc` 在**激活流程加入 Zone 时**就已经创建并落地到本机（owner 私钥签名，本地存储）。
- 因此当任意 device A 向 device B 发起 RTCP tunnel 时，A 携带的 `device_doc_jwt` 可以被 B 用 owner 公钥独立验证，**不需要 B 已经连上 system-config**。
- 这避免了"先有 tunnel 才能拿 system-config，又要先有 system-config 才能允许 tunnel"的循环依赖。

因此 boot 阶段的 `on_new_tunnel_hook_point` 不需要"bootstrap 模式 vs 正常模式"两套规则；只要 owner 公钥在本机可信即可执行完整准入策略。

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
