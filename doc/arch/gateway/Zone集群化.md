# Zone 的集群化

本文回答 `doc/arch/gateway/readme.md` 中提出的问题：在家庭和个人集群环境中，设备 IP、网络位置和公网可达性都会变化，Gateway 应如何让 Zone 先可用、再持续优化访问路径。

本文偏思路介绍，具体数据结构、生成规则和实现阶段见 `doc/arch/gateway/boot_gateway的配置生成.md`。

本文聚焦"Node 之间集群化"的拓扑问题。Client Device（笔记本、手机等不承载服务的设备）走的是另一条访问路径，详见本文"Client Device 的访问路径"一节。

核心思路是把 Node 的拓扑问题拆成三层：

1. **Boot 层**：只解决最小可达性。依赖 `ZoneBootConfig`、SN、ZoneGateway、OOD 描述和本地 Finder 缓存，先让 OOD、ZoneGateway、普通 Node 能找到 system-config 或至少找到一个可用中转。
2. **调度层**：以 system-config 为真相源。scheduler 根据 `devices/*/info`、服务实例上报、Zone 配置，按 source node 分别生成 `nodes/<node>/gateway_info` 和 `nodes/<node>/gateway_config`。
3. **Gateway/Tunnel 层**：只消费已发布的配置。NodeGateway 根据 selector 和 route 转发服务；RTCP/name-client 负责给定路径内部的地址解析、IP 竞速和连接级 failover。

因此，Gateway 拓扑算法不应该变成一个隐式全局搜索器。设备发现、候选 URL 生产、业务成本判断和刷新周期应由 Boot builder、scheduler 或 runtime（在 cyfs-gateway 视角下相当于应用层调度器）完成；Gateway 只在显式配置的路径集合内选择。

Boot 后的常态闭环应是：

```text
node-daemon 上报 DeviceInfo
  -> scheduler 读取 devices/*/info 并分析拓扑
  -> scheduler 按 source node 生成 nodes/<source>/gateway_info
  -> node-daemon 拉取并落地 node_gateway_info.json
  -> cyfs-gateway 按显式 routes 执行转发和 failover
```

其中 `DeviceInfo` 表达的是节点自己的网络事实和 probe 证据；`gateway_info.routes` 才是 scheduler 裁决后的转发计划。

## 当前实现基线

当前实现已经有一条稳定的服务访问链路：

```text
ServiceInstanceReportInfo + devices/*/info
  -> scheduler 计算 ServiceInfo 和每个 source node 的 route candidates
  -> scheduler 写 nodes/<source>/gateway_info
  -> source node 上的 node-daemon 落地 $BUCKYOS_ROOT/etc/node_gateway_info.json
  -> source node 上的 cyfs-gateway boot_gateway.yaml process-chain 转发
```

相关入口：

- `src/kernel/scheduler/src/scheduler.rs`
- `src/kernel/scheduler/src/service.rs`
- `src/kernel/scheduler/src/system_config_agent.rs`
- `src/kernel/node_daemon/src/node_daemon.rs`
- `src/rootfs/etc/boot_gateway.yaml`

当前 `ServiceInfo` 的语义是“哪些 Node 上有可用 Provider”。scheduler 只把 `Running` 且最近 `90s` 内有存活证明的实例放入 `ServiceInfo`，同一服务的刷新至少间隔 `30s`。写入 system-config 后，再转换成 `node_gateway_info.json` 中的：

```json
{
  "service_info": {
    "control-panel": {
      "selector": {
        "ood1": { "port": 3202, "weight": 100 }
      }
    }
  },
  "node_route_map": {
    "ood2": "rtcp://ood2.example.zone/"
  }
}
```

`boot_gateway.yaml` 的当前行为是：

1. 从 host 或 `/kapi/<service>` 找到目标 service。
2. 如果 selector 中包含本机 Node，直接转发到 `127.0.0.1:<port>`。
3. 否则从 `NODE_ROUTE_MAP[node_id]` 取一个 RTCP URL，拼接服务端口后 `round_robin` 转发。

这说明当前实现仍是“Provider Node 选择 + 每 Node 单 RTCP route”的模型，还没有实现“每个 source node 到每个 target node 多条候选路径”的完整集群化模型。

按节点生成配置是关键约束：同一个 target node，在不同 source node 的 `gateway_info.routes[target]` 中可以有不同候选路径。例如 `nodeA` 与 `ood1` 在同一 LAN 时应优先 direct，而 `nodeB` 在 Zone 外时可能只能优先经 ZoneGateway 或 SN relay。

## Zone 必须有公网入口或中转

一个 Zone 想要稳定支持公网访问，至少需要以下能力之一：

- `ZoneBootConfig` 配置 SN；
- `ZoneBootConfig` 或 ZoneConfig 能描述一个公网可达的 ZoneGatewayNode；
- 用户自有域名、DDNS、证书、端口映射和公网入口全部由用户自己维护。

否则，Zone 内同局域网访问仍可能可用，但 Zone 外设备无法稳定进入 Zone。典型家庭 NAT 环境下，SN 或公网 ZoneGateway 不是优化项，而是“公网可达”的基础依赖。

公网入口的角色可以由 OOD 承担，也可以由普通 ServerNode 承担。二者的差异是：

- **OOD**：同时承担 system-config、scheduler 等核心职责。
- **非 OOD ZoneGatewayNode**：主要承担对外入口、TLS、RTCP relay/keep-tunnel target，不应被当作 system-config 真相源。

## 默认场景：Node 在同一局域网

最小家庭 Zone 可以假设 OOD 和常驻 Node 在同一 LAN 中。这个场景下的目标是：先通过局域网发现找到 OOD，再进入 system-config。

当前已有的 Finder 能力包括：

- OOD 运行 UDP Finder server；
- NodeFinderClient 通过 IPv4 broadcast 搜索 OOD；
- Finder 响应携带由 owner 签名的 OOD device doc，并完成校验；
- 发现结果会写入 `finder_cache.json`，包含 IP、RTCP port、net_id 等信息。

默认局域网流程应是：

```text
Node/Device
  -> Finder 或缓存找到 OOD endpoint
  -> 建立到 OOD NodeGateway 的 RTCP tunnel
  -> 访问 system-config
  -> 读取 devices/*/info 和 services/*/info
  -> 后续服务访问交给本机 NodeGateway
```

在这个阶段，局域网直连应优先于 relay。relay 可以先成功并临时保证可达，但不能因为“先成功”就长期覆盖 direct 的静态优先级。

## Client Device 的访问路径

笔记本、手机等 Client Device 经常在 Zone 外部网络中使用。它们不承载服务，也不接受 scheduler 的 `nodes/<source>/gateway_info` 下发；它们解决的是另一类问题——"如何从外部接入 Zone 并访问 Zone 内服务"。

Client Device 的访问路径流程：

1. **接通入口**：通过 Zone 的公网入口建立到 system-config 的连接。最常见的形式是 `https://<zone-host>/kapi/system_config`，由 ZoneGateway / SN 在公网侧承接。这一步成功后，Client Device 就拿到了访问 Zone 内服务的可信入口。
2. **拉取必要元数据**：从 system-config 读取目标服务的 `ServiceInfo` 和相关 Provider Node 的 `DeviceInfo`。这部分数据 scheduler 已经在维护，Client Device 只是读取。
3. **本地链路优化**：runtime 拿到 ServiceInfo + DeviceInfo 后，在客户端侧自己做"访问 Zone 内服务的最优路径"选择：
   - 如果 DeviceInfo 表明目标 Node 与本设备在同一 LAN，且新鲜度足够，尝试 direct RTCP；
   - 否则继续经 ZoneGateway / SN 中转；
   - 这里的探测、竞速、回退都是一次性的，由 runtime 自己驱动，不需要 scheduler 介入。

常见路径形态：

```text
Client Device -> ZoneGateway -> NodeGateway -> Service
Client Device -> SN -> ZoneGateway/OOD -> NodeGateway -> Service
Client Device -> direct RTCP -> NodeGateway -> Service     # 仅当本机网络观测支持
```

其中 SN 与 ZoneGateway 的定位不同：

- **SN**：公网协助节点，提供 DDNS、TXT/引导、证书挑战协助、HTTPS/RTCP relay、keep-tunnel target 等能力。
- **ZoneGateway**：Zone 自己的入口节点，持有 Zone hostname 的 TLS 能力，并能通过 NodeGateway 访问 Zone 内服务。

当 Zone 内只有某个 OOD 与 SN keep-tunnel 时，外部 Client Device 访问其它 Node 可能需要多跳：

```text
Client Device -> SN -> OOD1 -> Target Node
```

这条路径以显式 Tunnel URL 表达（参见 `服务的多链路选择.md`），由 Client Device runtime 在本地构造，而不是 Gateway 在 direct 失败后自动发明。

> 这一节解释 Client Device 的访问模型；本文剩余内容仍聚焦于 Node 之间的集群化拓扑（per source node 的 routes 下发、scheduler 接管、keep_tunnel 集合等）。两者不要混淆。

## 有公网 ServerNode 的 Zone

如果 Zone 有一个公网 ServerNode，它应被建模为 ZoneGatewayNode。这个节点通常具备固定 IP、动态公网 IP 或可端口映射的 RTCP 入口。

这种场景的启动与访问策略是：

1. `ZoneBootConfig` 或 ZoneConfig 中能描述该 GatewayNode。
2. Zone 内 OOD 与 GatewayNode 建立或保持 RTCP tunnel。
3. Zone 外设备优先连接 GatewayNode，而不是盲目搜索内网 OOD。
4. 服务访问仍通过 `ServiceInfo + NodeGateway` 完成；GatewayNode 只是入口和中转，不直接决定服务实例。

如果 GatewayNode 是非 OOD 节点，它启动时必须先以 relay/target 身份可用，允许 OOD 连接进来；连上 OOD 后再通过 OOD/system-config 接管正式配置。

## 多 OOD 与集群初始化

当产品形态允许时，应尽量在初次建 Zone 时就按目标拓扑初始化，而不是先创建一个单 OOD 系统再反复迁移。

推荐原则：

- 单 OOD + 无公网入口：适合纯局域网或开发环境，但公网访问能力受限。
- 单 OOD + SN：最常见家庭形态，公网访问依赖 SN keep-tunnel 或 relay。
- 单 OOD + 公网 ZoneGatewayNode：适合有廉价 VPS 的用户，能降低对 SN relay 的依赖。
- `2n+1` OOD：适合高可用 Zone，system-config/klog 一致性要求应和 Gateway boot 路径一起设计。
- 多 LAN：每个 LAN 至少应有一个稳定节点与公网入口或 SN 维持可达路径，避免孤岛。

从单 OOD 演进到集群时，以下变化通常需要修改 `ZoneBootConfig` 或 ZoneConfig：

- 新增公网 GatewayNode；
- 新增 SN；
- 单 OOD 演进为 `2n+1` OOD；
- OOD 的 net_id、RTCP port、hostname 或入口职责发生变化。

## 路径选择的目标模型

`readme.md` 提出的“持续探测拓扑，并优先识别少量关键节点”可以落到 route candidate 模型上。

目标数据形态应从当前：

```text
node_route_map: node_id -> rtcp URL
```

演进为：

```text
nodes/<source>/gateway_info.routes: target_node_id -> route candidate list
```

示例：

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
        "url": "rtcp://ood2.example.zone/",
        "source": "system_config"
      },
      {
        "id": "via-sn",
        "kind": "rtcp_relay",
        "priority": 30,
        "url": "rtcp://rtcp%3A%2F%2Fsn.example.org%2F@ood2.example.zone/",
        "source": "zone_boot_config"
      }
    ]
  }
}
```

`routes` 是下发到当前 source node 的配置，不是全局拓扑表。`boot_gateway.yaml` 从当前节点的 candidate list 中生成转发 map。

排序原则：

- local 优先于 remote；
- direct 优先于 relay；
- 同等路径下再看 RTT、历史成功率、业务成本；
- relay 可兜底，但不能隐式扩散；
- 连接成功时间只能影响当前可用性，不能反写静态优先级。

## DeviceInfo 与拓扑证据

Boot 后，node-daemon 应周期性上报更完整的网络观测信息，帮助 scheduler 构造正确的 per-node route candidates。

DeviceInfo 中适合保存：

- 当前网络环境是否变化：用 `network_observation_id`、`network_generation`、`network_changed_at` 表达，而不是只依赖一个 bool。
- 必要网络信息：IP 地址列表、RTCP 端口、net_id、网络来源和观测时间。
- IPv6 能力：区分“有 IPv6 地址”和“IPv6 真的可用”。光有地址不代表能访问公网 IPv6，也不代表 RTCP 可被其它节点经 IPv6 连上。
- direct probe 结果：本节点能用 direct RTCP 连上的 OOD 或关键 Node 列表，包含 URL、状态、RTT、失败原因和时间戳。

DeviceInfo 不应直接声明最终 forward-plan。节点可以上报“我 direct probe 到 ood1 成功”，但不应上报“业务流量必须走 direct”。是否把这条证据转成 primary route，由 scheduler 结合全局 Zone 状态决定。

## 分层职责

### Boot builder

Boot builder 输入 `ZoneBootConfig`、本机角色、本机 device doc、Finder cache 和可选 SN 信息，输出 boot 期 route candidates 与 keep_tunnel 目标。

它负责：

- OOD 之间的 direct 和 relay candidate；
- OOD 到 SN、ZoneGateway 的 keep_tunnel；
- 普通 Node 到 system-config 的最小访问路径；
- direct 与 relay 并发探测，但保持 direct 长期优先。

### scheduler

scheduler 在 system-config 可用后接管：

- 从 `devices/*/info` 构造 Node 列表、网络观测和 probe 证据；
- 从实例上报构造 `ServiceInfo`；
- 按 source node 生成 `nodes/<node>/gateway_info`，其中 `routes[target]` 是从该 source node 出发的候选路径；
- 按 source node 生成 `nodes/<node>/gateway_config`，包括 TLS、ACME、静态 Web dir server、RTCP 准入等后置配置。

scheduler 是服务 Provider 集合和 per-node route candidate 的唯一决策者。Gateway 不应该绕过 scheduler 自行访问 system-config 做二次调度。

### NodeGateway

NodeGateway 负责把请求限制在已配置的 service selector 和 route 集合内：

- 本机 Provider 直接转发到 `127.0.0.1:<port>`；
- 远端 Provider 通过配置中的 RTCP route 转发；
- 多 Provider 时按 selector 策略做稳定选择；
- 多 route 时按显式 candidate 和 failover 策略选择。

### RTCP/name-client

RTCP/name-client 负责给定 URL 内部的连接问题：

- DID/hostname 解析；
- 候选 IP 排序；
- Happy Eyeballs 风格的连接竞速；
- 给定 IP 列表内的慢路径切换；
- tunnel keepalive 和连接级 failover。

BuckyOS 配置层不应手写 IP 级别的 `ipv4 > ipv6` 或 LAN 地址竞速逻辑。

## 运行期优化

拓扑优化应是持续过程，但需要控制成本。由于家庭 Zone 的规模通常很小，可部署 Node 一般不超过 10 台，所有 Device 通常不超过 100 台，因此节点级探测可以接受有限的 `n^2` 成本，但仍应避免服务级重复探测。

推荐策略：

- Boot 阶段高频探测关键节点：OOD、SN、ZoneGateway。
- 平稳阶段降低刷新频率，例如 5 到 10 分钟一次。
- DeviceInfo 在 30 秒内可认为新鲜，优先用其网络观测和 probe 证据构造 direct candidate。
- 拿到新鲜 DeviceInfo 后停止更广泛搜索，避免搜索结果和自上报结果互相干扰。
- 大流量业务启动前可以强制刷新发现和 Probe；没有 direct 时应失败或提示，而不是自动走 relay。

## 故障模型

### 单 OOD + 单 ZoneGateway

- OOD 掉线：system-config 不可写，ZoneGateway 只能提供有限只读入口或缓存内容。
- ZoneGateway 掉线：Zone 外访问不可用；同 LAN 设备仍可通过 Finder/direct RTCP 访问 OOD 和本地服务。
- SN 掉线：如果没有公网 ZoneGateway，Zone 外访问不可用；Zone 内访问不应受影响。

### 3 OOD + 单 ZoneGateway

- 单个 OOD 掉线：system-config/klog 应继续可用，前提是 OOD 间仍能形成 quorum。
- ZoneGateway 掉线：公网入口受影响；如果 OOD 分布在多个 LAN 且只能通过该 Gateway 互联，可能连带影响 quorum。
- SN 掉线：有公网 Gateway 时可降级为直连 Gateway；无公网 Gateway 时 Zone 外访问受限。

### 多 LAN

多 LAN 的关键风险是“局部网络孤岛”。每个 LAN 中应至少有一个稳定 Node 能：

- 与本 LAN Node direct；
- 与 OOD quorum 或公网入口保持可达；
- 必要时通过 SN/ZoneGateway relay 兜底。

## 演进步骤

1. 扩展 DeviceInfo 的网络观测和 direct probe 信息。
2. scheduler 基于 `devices/*/info`、`boot/config`、SN/ZoneGateway 配置，按 source node 生成 `routes` candidate list。
3. 增加 boot route builder，解决 scheduler 产物不可用时的最小可达性。
4. 修改 `boot_gateway.yaml`，远端服务从 candidate list 生成 forward map。
5. 接入 tunnel_mgr URL 状态查询，让 boot readiness、诊断和调度刷新获得 URL 可达性、RTT、排序和失败原因。
6. 根据业务需要扩展 selector：`single`、`round_robin`、`rtt_first`、`cost_first_then_rtt`、`failover`。
7. 补齐 RTCP tunnel 准入策略，默认只允许同 Zone 或显式 trust relationship 的来源使用 relay 能力。

## 与其它文档的关系

- `doc/arch/gateway/zone-boot-config与zone-gateway.md`：解释 ZoneBootConfig、ZoneGateway、SN、域名和访问路径组合。
- `doc/arch/gateway/boot_gateway的配置生成.md`：定义 boot 阶段 gateway 配置、DeviceInfo 网络观测和 per-node route candidate 的具体生成逻辑。
- `doc/arch/gateway/service selector.md`：定义 Service Selector 的边界，即只在已配置 Provider 和 URL 集合内做选择。
- `doc/arch/gateway/服务的多链路选择.md`：展开 DeviceInfo、直连、中转、Gateway Probe 和业务调度的多链路模型。

一句话总结：Zone 集群化不是让 Gateway 自动猜测全网拓扑，而是让 Boot、scheduler、Gateway、Tunnel 各自只解决一层问题；先用少量关键路径保证可达，再用显式 route candidate 持续优化访问路径。
