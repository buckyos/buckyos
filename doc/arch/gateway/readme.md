# Gateway 架构问题

本目录主要讨论 BuckyOS 在家用环境中部署 Personal Cluster 时，如何处理集群网络拓扑，以及如何在缺少人工运维的情况下完成自组织网络拓扑管理。

这类问题与传统集群环境不同。传统集群通常具备以下前提：

1. 设备通常位于同一个局域网内。
2. 所有设备都有稳定的 IP 地址。
3. 当需要跨机房访问时，通常由运维人员手工配置网络。

因此，大规模集群的网络拓扑通常是由运维人员维护的静态拓扑。家用环境并不具备这些条件。

首先，设备的 IP 地址可能随时变化。设备重启、路由器重启，或者家庭断电后恢复，都可能导致局域网内设备重新分配 IP。

其次，设备所在的网络位置也可能变化。当前可部署服务的 Node 通常仍然是固定设备，但家用环境中可能新增路由器或交换机层级，也可能将设备移动到不同房间，从而改变局域网拓扑。

此外，作为客户端使用的 Client Device 更容易发生位置变化。典型场景是笔记本电脑：平时在家使用，偶尔带到办公室或其他网络环境中使用。Client Device 的访问路径不依赖 scheduler 下发，由本地 runtime 在拿到 system-config 入口和目标 DeviceInfo 后即时优化（详见下文"集群内设备的分类"一节）。

最后，Zone 的物理分布可能更加复杂。部分用户为了提升物理可靠性，可能会把设备分别部署在家中、公司，甚至父母家的局域网中。

这些环境特点要求 Gateway 具备分层的拓扑处理策略。核心逻辑包括：

1. 持续探测当前拓扑（Topology），并优先识别少量关键节点，先保证系统可用性。
2. 在系统运行过程中持续优化访问路径，逐步寻找更合适的拓扑结构。

由于拓扑本身会持续变化，拓扑发现与优化也需要持续运行。同时，这套机制不能对日常使用造成明显影响，额外网络开销和计算开销都需要控制在可接受范围内。

## 集群内设备的分类

讨论拓扑前必须先区分 Zone 内不同类型的设备，因为它们在访问路径、配置下发和运维要求上各不相同。

| 类别 | 说明 | 是否承载服务 | gateway 配置来源 | 路径选择方式 |
| --- | --- | --- | --- | --- |
| **OOD** | 特殊的 Node。同时承担 system-config / scheduler / klog 等核心职责，是 Zone 的真相源 | 是 | scheduler 写入的 `nodes/<source>/gateway_info` + 本机 `boot_gateway.yaml` | 完整 routes，参与 keep_tunnel 集合 |
| **Node**（含 ZoneGatewayNode） | 可以运行服务的设备，是 service 的载体；ZoneGatewayNode 是承担公网入口职责的特殊 Node | 是 | scheduler 写入的 `nodes/<source>/gateway_info` + 本机 `boot_gateway.yaml` | 完整 routes |
| **Client Device** | 笔记本、手机等只访问 Zone 服务、不承载 service 的设备 | 否 | 不接受 scheduler 下发；本机不运行 cyfs-gateway 或只跑客户端形态 | 通过 `https://<zone-host>/kapi/system_config` 一次性获取 ServiceInfo + DeviceInfo，由 runtime 在客户端侧完成链路优化 |

关键约定：

- **scheduler 下发的 `nodes/<source>/gateway_info` 只面向 Node**（包括 OOD 与 ZoneGatewayNode）。Client Device 不在该机制覆盖范围内。
- **Client Device 不需要 per-source 的 routes 配置**。它在拿到 ServiceInfo + 目标 Node 的 DeviceInfo 后，由本地 runtime 直接做"访问 Zone 内服务的最优路径"选择：先经 ZoneGateway / SN 接通 system-config 入口（`https://<zone-host>/kapi/system_config`），再基于返回的 DeviceInfo 评估直连可行性、按需走中转。
- 这意味着设计中"per source node 的 routes 爆炸"问题被天然限制在 Node 数量内（家用场景 ≤ 10），而不是所有 Device 数量内。

所以 readme 后续提到的 "Node" 默认指 OOD 与 Node 的合集；提到 "Device" 指 Client Device。当二者需要明确区分时会用全称。

## 规模约束

BuckyOS 的一个重要约束是系统规模相对较小。在一般家用环境下，可部署服务的 Node 数量通常不超过 10 台；即使计算所有 Client Device，常规规模也应在 100 台以内。

Node 规模决定了 scheduler 需要为多少 source 生成 routes（≤ 10），也决定了 keep_tunnel 长连接对（OOD ↔ OOD / OOD ↔ SN / OOD ↔ ZoneGateway）的总数。Client Device 数量主要影响并发访问压力，不影响 scheduler 的 route 生产成本。

这一规模是 Gateway 拓扑算法设计的重要前提。部分探测过程可能具有 `n^2` 级别的复杂度，相关流量开销也可能随节点规模平方增长。在家用场景的规模范围内（n ≤ 10），这类策略仍然可以保持可控，不会导致系统不可用。

文档地图（按顺序阅读)
- `Zone集群化.md`:目前解决方案的整体思路
- `服务的多链路选择.md` + `service selector.md`: 站在访问集群内的服务的角度，说明上述成型的拓扑是如何被使用的
- `boot_gateway的配置生成.md` ： 基于思路的细节落地，说明ZeroOP的集群网络配置是构造和自动更新的
- `zone-boot-config与zone-gateway.md` : 说明的Zone如何通过外部配置，解决Boot阶段的一致性问题
- `SN.md` : 统一整理了Zone外基础设施SN的功能

