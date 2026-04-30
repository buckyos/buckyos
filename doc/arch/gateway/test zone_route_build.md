

## 1. 整体思路（30 秒读完）

- scheduler 是 **source 视角** 的：同一个 target node 在不同 source 节点的 routes 里可以完全不一样。每个 source 独立计算自己的 `routes[target]`。
- `build_forward_plan(...)` 现在返回 `ForwardPlan { routes, did_ip_hints }` 两份输出：
  - **routes**：每个 target 最多 3 条 candidate（`direct`(10) / `via-sn`(30) / `via-zone-gateway`(40)），URL **一律是 DID hostname 形态**（`rtcp://<target>.<zone>[:port]/`）。
  - **did_ip_hints**：以 target 的 DID hostname 为 key 的 IP 事实清单，喂给 cyfs-gateway 的 `resolve_ips`，由后者按 family / scope / freshness / 历史 RTT 排序并 Happy Eyeballs 竞速。scheduler **不再做 IP 级排序**。
- direct candidate 至多 **一条**。决定它是否生成、附上哪种证据，由一组**互斥的判定**按优先序确定：SignedWanIp → DirectProbe → Ipv6GlobalEndpoint → WanTarget → SameNetId → SharedOodInference。任一成立即写一条 `direct`，priority 永远是 10，`evidence.evidence_type` 字段标明命中分支。
- direct / relay 的 priority 只剩 10 / 30 / 40 三档。direct 存在时，relay 自动打 `backup=true`；relay 只要 zone_config 配置就一定写入。
- **签名 IP 互斥**：target 在 DeviceConfig 中有 owner 签名的 wan IP 时，`did_ip_hints[target]` 仅保留 `SignedDeviceDoc` 来源；其它（LanEndpoint / GlobalIpv6）即使观测到也会被剔除。direct candidate 自身仍然只是 DID hostname URL，不再因签名 IP 改变 `keep_tunnel`。

## 2. 决策输入（哪些字段进了路由）

| 字段 | 来源 | 影响 |
|---|---|---|
| `device_doc.net_id` | DeviceConfig（激活时填）| 同 net_id / wan / wan_dyn / portmap / nat / lan* / unknown |
| `device_doc.ips` | 激活时签名 | target 有签名 wan IP → direct evidence = `signed_wan_ip`；IP 进 `did_ip_hints` 标 `SignedDeviceDoc`，并触发互斥 |
| `network_observation.direct_probe[]` | node-daemon 周期上报 | reachable + 新鲜 → direct evidence = `direct_probe`，evidence 上挂 rtt / last_success / ttl |
| `network_observation.endpoints[scope=lan]` | 系统接口枚举 | 进 `did_ip_hints` 标 `LanEndpoint`；同时参与子网一致性反证 |
| `network_observation.endpoints[scope=global, family=ipv6]` | 系统接口枚举 | 进 `did_ip_hints` 标 `GlobalIpv6`（target v6.state=unavailable 时不进） |
| `network_observation.ipv6.state` | source / target 自己探测 | source `egress_ok`/`rtcp_direct_ok` + target 非 `unavailable` + target 有 global v6 → direct evidence = `ipv6_global_endpoint`；source `rtcp_direct_ok` 时 v6 hint confidence=high，否则 medium |
| `network_observation.observed_at` + `probe.last_success` + `freshness_ttl_secs` | 同上 | `observed_at - last_success > ttl` → probe 视为 stale，丢弃 |
| `zone_config.sn` / `get_default_zone_gateway()` | ZoneConfig | 决定是否写 `via-sn` / `via-zone-gateway` relay |

## 3. node-node 物理链路场景矩阵

测试覆盖建议：每个场景至少跑一次 `build_forward_plan(source, ...)`，分别 dump `plan.routes[target]` 与 `plan.did_ip_hints[<target>.<zone>]`。

| # | 物理拓扑 | source 标签 / 信号 | target 标签 / 信号 | direct evidence_type | did_ip_hints 来源 |
|---|---|---|---|---|---|
| 1 | 家用同 LAN，单子网 | lan1, lan ip 192.168.1.10 | lan1, lan endpoint 192.168.1.20 | `same_net_id` | `LanEndpoint`(192.168.1.20) |
| 2 | 同 net_id 标签但实测不同子网（VPN/桥接误标） | lan1, 192.168.1.x | lan1, 10.0.0.x | **无 direct**（子网反证） | `LanEndpoint`(10.0.0.x) |
| 3 | 同 LAN 但都未填 net_id | None | None | `same_net_id`（unknown_lan 视同同 LAN）| 视 endpoint |
| 4 | 同 LAN + 已 probe 成功 + 上报 lan endpoint | lan1 + probe→ood2 | lan1 + lan endpoint | `direct_probe`（优先级压过 same_net_id）| `LanEndpoint` |
| 5 | 同 LAN + probe 已 stale | lan1，probe last_success+ttl < observed_at | lan1 | `same_net_id`（stale 不再触发 direct_probe）| 视 endpoint |
| 6 | source LAN/NAT，target 公网静态签名 IP | nat / lan1 | wan + 签名 ips | `signed_wan_ip` | `SignedDeviceDoc`（其它来源被互斥剔除）|
| 7 | source LAN/NAT，target wan_dyn 无签名 IP（DDNS） | nat | wan_dyn 无 ips | `wan_target_net_id` | 视 endpoint（无签名 IP 不触发互斥）|
| 8 | source LAN，target portmap | lan1 | portmap | `wan_target_net_id` | 视 endpoint |
| 9 | 双 LAN 无 v6 无共同 OOD | lan1 | lan2 | **无 direct** | 视 endpoint |
| 10 | 双 LAN + 共同非公网 OOD（双方都 probe 到 ood1，ood1.net_id 非 wan）| lan1 + probe→ood1 | lan2 + probe→ood1 | `shared_ood_direct_probe`（evidence.witness_node="ood1"）| 视 endpoint |
| 11 | 双 LAN + witness OOD 在公网 | lan1 + probe→ood1(wan) | lan2 + probe→ood1(wan) | **无 direct**（witness 被过滤）| 视 endpoint |
| 12 | 双 LAN，双方 IPv6 egress_ok，target 有 global v6 endpoint | lan1, ipv6.state=egress_ok | lan2, global v6 endpoint | `ipv6_global_endpoint`，evidence.confidence=medium | `GlobalIpv6`(2001:..) confidence=medium |
| 13 | source v6 = address_only | lan1, ipv6.state=address_only | lan2 + global v6 | **无 v6 direct**（source 未验证出站）| `GlobalIpv6` 仍写入（事实层）|
| 14 | target v6 显式 unavailable | lan1, egress_ok | lan2, ipv6.state=unavailable + global v6 | **无 v6 direct** | **不写 `GlobalIpv6`** |
| 15 | source rtcp_direct_ok | lan1, ipv6.state=rtcp_direct_ok | lan2 + global v6 | `ipv6_global_endpoint`，evidence.confidence=**high** | `GlobalIpv6` confidence=**high**（target rtcp_direct_ok）|
| 16 | source 与 target 同节点 | this_node == target | - | **不进 routes / 不进 did_ip_hints** | selector 走 127.0.0.1 |
| 17 | 双方都 wan + 签名 IP | wan + ips | wan + ips | `signed_wan_ip` | `SignedDeviceDoc` |
| 18 | source 是 wan_dyn，target 是 nat / lan | wan_dyn | lan2 | **无 direct**（target 不公网）| 视 endpoint |
| 19 | source wan，target portmap | wan | portmap | `wan_target_net_id` | 视 endpoint |
| 20 | 双 LAN 且 zone_config 既无 sn 又无 zone_gateway，且无 direct evidence | lan1 | lan2 | **不写 routes**（candidate 列表空）| 视 endpoint，仍可能写 |
| 21 | target 的签名 IP 是 IPv6 | nat | wan + 签名 ipv6 | `signed_wan_ip` | `SignedDeviceDoc`(IPv6) |
| 22 | 旧格式 probe（直接挂 `device_doc.extra_info.direct_probe`，没有 network_observation 包裹）| 任意 | 任意 | `direct_probe` 仍生效，但**无 freshness 上下文**，永不 stale | 兼容路径 |
| 23 | source 无任何观测数据，target 有 lan endpoint | lan1 | lan1 + lan endpoint | `same_net_id`（source 缺数据不影响 net_id 判断）| `LanEndpoint`(target.endpoint) |
| 24 | 双方都标 `nat` | nat | nat | ⚠️ **当前实现：`same_net_id`**（同 net_id 字面相同视同同 LAN）| 视 endpoint。两个独立 NAT 后的设备并不真在同 LAN，这是实现已知的 over-trust |

### 直连 evidence 的优先级（决定写哪一种）

> `signed_wan_ip` > `direct_probe` > `ipv6_global_endpoint` > `wan_target_net_id` > `same_net_id` > `shared_ood_direct_probe`

任一更高优先证据成立时，更低优先的判定不再触发 direct candidate。但**事实层**（did_ip_hints）独立于 evidence 选择：例如 evidence=`direct_probe` 时，target 上报的 LAN endpoint 仍会作为 `LanEndpoint` hint 写入。

## 4. did_ip_hints 单独断言

did_ip_hints 是给 cyfs-gateway resolve_ips 的 IP 事实清单，建议作为 routes 之外的**独立维度**断言，避免和 direct evidence 选择强耦合。

| 断言点 | 触发条件 | 期望 |
|---|---|---|
| 含 `SignedDeviceDoc` 来源 | target.net_id wan 系 + `device_doc.ips` 非空 | hints 包含每个签名 IP，confidence=high；**互斥**生效，hints 中只剩 SignedDeviceDoc |
| 含 `LanEndpoint` 来源 | target.network_observation 有 `scope=lan` endpoint | 每条 lan endpoint 都进 hints，confidence=medium；`last_observed_at` 优先取 endpoint.observed_at，否则 obs.observed_at |
| 含 `GlobalIpv6` 来源 | target.network_observation 有 `scope=global, family=ipv6` endpoint **且** ipv6.state ≠ "unavailable" | confidence=high（state=rtcp_direct_ok）/ medium（其它）|
| 互斥不变量 | 任意 hint 是 SignedDeviceDoc | 其它 source 全部被剔除，最终列表只剩 SignedDeviceDoc 项 |
| port 字段 | 任何 hint | 等于 `device_rtcp_port(target)`（target.device_doc.rtcp_port 或默认 2980）|
| key 形态 | 非空 hints | `did_ip_hints` 的 key = `format!("{}.{}", target_node_id, zone_host)`（DID hostname），不是 node_id |
| 空列表不写入 | target 既无签名 IP 也无 endpoints | `did_ip_hints` 不包含该 target 的 key |

## 5. 容易漏检的边界（建议重点扫）

1. **IPv6 link-local / ULA**：当前只消费 `scope=global` 的 v6 endpoint。fe80::/fc00:: 段不会进 hints，即使在同一段 v6 LAN 也是。
2. **IPv4 子网阈值**：子网一致性反证硬编码 /24（v4）和 /64（v6）。校园网 /23、运营商大段 /20 会被错判为不同 LAN（拒绝 same_net_id）；某些 /25、/26 切分则会被错判为同 LAN。
3. **target 多 NIC**：现在所有 lan endpoint 都进 `did_ip_hints`，不再像旧实现只取第一条；resolve_ips 端会自行排序竞速。测试时应预期 hints 长度 == endpoints 中 lan 条目数（dedup 按 ip+source）。
4. **scheduler 自身时钟**：probe 新鲜度只用 `observation.observed_at - last_success` 比较，不用 scheduler 当前时间。从 device 上报到 scheduler 调度之间的延迟（可能数分钟到数小时）不会导致 probe 进一步 stale。
5. **同 net_id 标签的"假同 LAN"**：场景 #24 提到的 `nat` / `portmap` 同 net_id 误判；以及任意 `lan*` 标签拼写一致但物理不同 LAN 的场景。`endpoints` 子网反证只在双方都有 LAN endpoint 时才生效。
6. **probe 单向性**：probe 只在 source → target 方向发挥作用。source 能 probe 到 target 不代表反向 routes（target → source）也有 `direct_probe` evidence。要分别测 A→B 和 B→A 两个 source 视角。
7. **签名 IP 互斥的副作用**：target 有签名 IP 时，**即使** source 与 target 同 LAN、有 lan endpoint、IPv6 都 ok：
   - `routes[target]` 的 direct evidence 一定是 `signed_wan_ip`（其它路径被优先级压住）。
   - `did_ip_hints[target.zone]` 一定只有 `SignedDeviceDoc` 项。
   这是有意的安全边界，不要把 LanEndpoint / GlobalIpv6 缺失误报为 bug。
8. **target 缺数据但 source 有**：source 有 IPv6 egress_ok 但 target 没有 `network_observation` → 没有 `ipv6_global_endpoint` direct，hints 也无 GlobalIpv6 项。反过来同理。两侧观测都需要齐备才生成。
9. **routes 完全为空的 target**：当 direct evidence 不成立、zone_config 既无 sn 又无 zone_gateway 时，`routes` 不会包含该 target 的 key。但 `did_ip_hints[target.zone]` 仍可能写入（事实层独立）。调用方不应假定每个 device_list 中的 target 都在 routes 里。
10. **`keep_tunnel` 归属**：`direct` candidate 的 `keep_tunnel` 恒为 `true`，**不再因签名 IP 改变**。`via-sn` candidate 的 `keep_tunnel` 在 target 是 wan + 有签名 IP 时为 `false`（稳定 wan target 不消耗 SN 资源），其它情况为 `true`。`via-zone-gateway` 恒为 `false`。
11. **URL 形态**：所有 candidate 的 URL **都是 DID hostname**，**不会**出现 `rtcp://203.0.113.10/` 这类 IP 字面 URL。如果发现 IP 形态 URL 进入了 candidate，是回归 bug。

## 6. 测试入口

```rust
let plan: ForwardPlan = zone_route_builder::build_forward_plan(
    this_node_id,            // source 节点 ID
    &zone_config,            // 含 sn / oods / zone_gateway
    &zone_host,              // 用于拼 DID hostname
    &device_list,            // HashMap<node_id, DeviceInfo>，每个 DeviceInfo
                             // 的 device_doc.extra_info["network_observation"]
                             // 决定 endpoints / probe / ipv6 信号
);

// 路由层断言
let routes: HashMap<String, Vec<NodeGatewayRouteCandidate>> = plan.routes;
// candidate 上稳定的断言锚：id ∈ {"direct","via-sn","via-zone-gateway-<n>"} /
// kind / priority(10/30/40) / backup / keep_tunnel / evidence.evidence_type /
// evidence.confidence / evidence.witness_node。

// IP 事实层断言（独立于路由层）
let hints: HashMap<String, Vec<DidIpHint>> = plan.did_ip_hints;
// key = "<target>.<zone_host>"
// hint 上稳定的断言锚：ip / port / source ∈ {SignedDeviceDoc, LanEndpoint, GlobalIpv6} /
// confidence(high|medium|low) / last_observed_at。
```

返回结构：

```rust
pub struct ForwardPlan {
    pub routes: HashMap<String, Vec<NodeGatewayRouteCandidate>>,
    pub did_ip_hints: HashMap<String, Vec<DidIpHint>>,
}

pub struct DidIpHint {
    pub ip: IpAddr,
    pub port: Option<u32>,
    pub source: DidIpHintSource,                  // SignedDeviceDoc / LanEndpoint / GlobalIpv6
    pub confidence: String,                        // high / medium / low
    pub last_observed_at: Option<u64>,
    pub freshness_ttl_secs: Option<u64>,
}
```
