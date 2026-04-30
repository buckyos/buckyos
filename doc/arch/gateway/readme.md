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

## 关键基础设施实例

下面几个例子用于把 gateway 相关的基础设施配置串起来。例子里的 Zone 域名假设为 `test.buckyos.io`，节点为 `ood1`、`ood2`、`node1`，SN 为 `sn.devtests.org`。

### keep_tunnel 与 RTCP tunnel URL

`keep_tunnel` 表示 RTCP stack 启动后要长期保持的后台 tunnel 目标。典型场景是：

- 非公网可达的 OOD / Node 与 SN 保持 tunnel，让外部请求有机会经 SN 反向进入 Zone。
- 多 OOD 之间互相保持 tunnel，boot 阶段可以尽快满足 OOD quorum。
- OOD 与承担公网入口的 ZoneGatewayNode 保持 tunnel，让 ZoneGateway 可以稳定访问 Zone 内服务。

cyfs-gateway 上游原生 stack 配置里，`keep_tunnel` 条目是 RTCP 目标字符串；当前实现会在内部把条目拼成 `rtcp://<entry>` 后交给 tunnel manager。因此原生配置通常写 authority / path 部分，而不是再套一层 scheme：

```yaml
stacks:
  node_rtcp:
    protocol: rtcp
    bind: 0.0.0.0:2980
    key_path: ./node_private_key.pem
    device_config_path: ./node_device_config.json
    keep_tunnel:
      - sn.devtests.org/
      - ood2.test.buckyos.io/
```

BuckyOS 的 `routes` / `node_gateway_info.json` 里则保存完整 RTCP URL，因为它们会被 `forward` 直接消费：

```json
{
  "routes": {
    "ood2": [
      {
        "id": "direct",
        "kind": "rtcp_direct",
        "backup": false,
        "keep_tunnel": true,
        "url": "rtcp://ood2.test.buckyos.io/"
      },
      {
        "id": "via-sn",
        "kind": "rtcp_relay",
        "backup": true,
        "keep_tunnel": true,
        "url": "rtcp://rtcp%3A%2F%2Fsn.devtests.org%2F@ood2.test.buckyos.io/"
      }
    ]
  }
}
```

RTCP tunnel URL 的关键特点：

- `rtcp://ood2.test.buckyos.io/` 表示“到 `ood2` 的 RTCP tunnel”，目标设备身份由 authority 决定。
- `rtcp://ood2.test.buckyos.io/:3200` 表示“先到 `ood2` 的 RTCP tunnel，再让远端访问默认主机的 `3200` 端口”。BuckyOS 的 service forward 会用 `route.url + service port` 拼出这种 URL。
- `rtcp://<percent-encoded bootstrap URL>@ood2.test.buckyos.io/` 表示“先用 bootstrap URL 建底层 stream，再在这条 stream 上建立到 `ood2` 的外层 RTCP tunnel”。`@` 后面的 remote 始终是外层 RTCP 的真实身份，不会被 bootstrap 改写。

### forward group 配置实例

Gateway 入口不是直接写死 upstream，而是在 process-chain 中根据 `SERVICE_INFO.selector` 和本节点的 `ROUTES` 动态构造 group forward。文档里常把这个模型叫 `forward-plan`；cyfs-gateway 当前落地形态是 `forward --map/--backup-map/...` 在命令内部生成 `ForwardPlan`，再返回 `forward-group "<encoded-plan>"` 给 HTTP / stream / datagram 执行层消费。也就是说，现在不是一个独立的字面参数 `forward --plan`，也不是旧文档中出现过的 `--group-map`；primary peer map 使用的真实参数名是 `--map`。

当前 BuckyOS 的 `node_gateway_info.json` 同时写入新格式 `routes` 和兼容旧配置的 `node_route_map`。`boot_gateway.yaml` 会优先使用 `routes` 构造 group forward；如果某个目标 node 没有 `ROUTES[node_id]`，服务转发和 app 转发仍会回退到 `NODE_ROUTE_MAP[node_id]` 生成单条 primary peer。这是兼容路径，不是新的调度语义；新增多链路能力应以 `routes` 为准。

例如 `node1` 要访问 `system_config`，service selector、routes 与兼容字段可以是：

```json
{
  "service_info": {
    "system_config": {
      "selector": {
        "ood1": { "port": 3200, "weight": 100 },
        "ood2": { "port": 3200, "weight": 100 }
      }
    }
  },
  "node_route_map": {
    "ood1": "rtcp://ood1.test.buckyos.io/",
    "ood2": "rtcp://ood2.test.buckyos.io/"
  },
  "routes": {
    "ood1": [
      { "id": "direct", "url": "rtcp://ood1.test.buckyos.io/", "backup": false }
    ],
    "ood2": [
      { "id": "direct", "url": "rtcp://ood2.test.buckyos.io/", "backup": false },
      { "id": "via-sn", "url": "rtcp://rtcp%3A%2F%2Fsn.devtests.org%2F@ood2.test.buckyos.io/", "backup": true }
    ]
  }
}
```

对应的 process-chain 逻辑是：

```text
map-create primary_peers;
map-create backup_peers;

for node_id, node_info in $TARGET_SERVICE_INFO.selector then
  if match-include $ROUTES $node_id then
    for route_id, route in $ROUTES[$node_id] then
      local target_url="${route.url}:${node_info.port}";
      if eq $route.backup true then
        map-add backup_peers $target_url $node_info.weight;
      else
        map-add primary_peers $target_url $node_info.weight;
      end
    end
  else
    local target_rtcp_url=${NODE_ROUTE_MAP[$node_id]};
    if !eq $target_rtcp_url "" then
      local target_url="${target_rtcp_url}:${node_info.port}";
      map-add primary_peers $target_url $node_info.weight;
    end
  end
end

forward round_robin --map $primary_peers \
                    --backup-map $backup_peers \
                    --group "service:${SERVICE_ID}" \
                    --next-upstream error,timeout \
                    --tries 3;
```

基于上面的例子，gateway 会得到如下语义：

1. `ood1` direct 和 `ood2` direct 是 primary peers，参与 `round_robin`，权重来自 service selector。
2. `ood2 via-sn` 是 backup peer，只有 primary 连接阶段失败、且 `next_upstream` 允许重试时才进入尝试集合。
3. `--group "service:${SERVICE_ID}"` 让失败历史按逻辑服务隔离；同一个 URL 在不同 group 里不会互相污染 `max_fails / fail_timeout` 状态。
4. `--tries 3` 限制单次请求最多尝试 3 个 candidate，避免因为 route 多而无限放大一次访问成本。
5. `--next-upstream error,timeout` 只启用连接阶段的 error / timeout failover；请求已经绑定到某个 stream 后，不做透明迁移。
6. 当前 BuckyOS process-chain 生成的是扁平 primary / backup peer map，没有使用 cyfs-gateway 已支持的 `--server-map` provider-first 形态。因此 group forward 可以在一次请求内跨 provider 重试；如果某个有状态服务需要“先固定 provider，再只在该 provider 的多条 route 内切换”，需要后续显式改成 provider-first plan 或用 hash / consistent_hash 约束。
7. 所有多链路候选都来自本机已落地配置：优先来自 `ROUTES`，兼容场景下可来自 `NODE_ROUTE_MAP` 的单条 direct URL。gateway 不会在 forward 失败后自己发明新的 SN / relay 路径。

### Device 多 IP 来源与 RTCP 选址顺序

一个 Device 出现多个 IP 是正常情况，常见来源有：

- `DeviceConfig.ips`：设备 DID Document / DeviceConfig 中的 IP。若该文档是 owner 签名的 Device Document，name-client 会把它视为最高可信来源。
- `DeviceInfo.all_ip`：节点运行时自观测得到的本机地址集合。`DeviceInfo.auto_fill_by_system_info()` 会从系统网络接口收集可用地址，并过滤 loopback、multicast、unspecified、文档地址；IPv6 还会过滤 link-local、ULA、文档地址等不适合跨节点 RTCP 连接的地址。
- name provider / SN 查询结果：`name_client.resolve(name)` 返回的 `NameInfo.address`，通常来自 DNS、SN 或其它 name provider。
- 连接历史：addr-rtt 数据库不提供新 IP，但会记录 `(local_ip, remote_ip:port)` 的成功 RTT、失败、超时等结果，用来影响下一次排序。

cyfs-gateway 的 RTCP 内部实现把“候选 IP 展开和排序”交给 `name-client`，RTCP 自己只负责并发竞速建链。顺序如下：

1. 解析 `rtcp://<remote>[:port]/...` 中的 `<remote>`。
2. 调用 `name_client::resolve_ips(remote)` 获取候选 IP。
3. 如果能解析到 owner 签名的 Device Document，且其中有 `ips`，直接使用这组签名 IP；此时不会再合并未签名的 `NameInfo.address` 或 `DeviceInfo.all_ip`。
4. 如果没有签名 IP，则先合并 `NameInfo.address`，再合并 DeviceInfo 中的 `device_doc.ips` 和 `all_ip`，按出现顺序去重。
5. 如果本机已有 addr-rtt 历史，name-client 会按历史 RTT、成功率、失败惩罚、地址族偏好等重新排序；默认策略偏好 IPv6。没有本地历史时保持上一步的顺序。
6. RTCP 按候选顺序启动 Happy Eyeballs 风格的并发竞速：第一个 attempt 立即启动，之后每 250ms 启动下一个，单个 TCP connect 10s 超时；最先完成 RTCP 协议层握手的 attempt 获胜。
7. 获胜 attempt 的 Hello RTT 会回写 addr-rtt 数据库，失败 attempt 也会记录 timeout / unreachable，作为后续 `resolve_ips` 排序依据。
8. 已有 tunnel 上的 direct stream reconnect 会优先尝试上一次成功的 peer IP，然后再合并 `resolve_ips(remote)` 的其它候选并去重。

所以，Device 多 IP 不等于 gateway 顺序串行尝试所有地址。上层只需要给出可解释的候选来源；实际建链由 name-client 排序、RTCP 并发竞速，并用历史结果持续修正。

文档地图（按顺序阅读)
- `Zone集群化.md`:目前解决方案的整体思路
- `服务的多链路选择.md` + `service selector.md`: 站在访问集群内的服务的角度，说明上述成型的拓扑是如何被使用的
- `boot_gateway的配置生成.md` ： 基于思路的细节落地，说明ZeroOP的集群网络配置是构造和自动更新的
- `zone-boot-config与zone-gateway.md` : 说明的Zone如何通过外部配置，解决Boot阶段的一致性问题
- `SN.md` : 统一整理了Zone外基础设施SN的功能
