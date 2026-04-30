

## 1. 整体思路（30 秒读完）

- scheduler 是 **source 视角** 的：同一个 target node 在不同 source 节点的 routes 里可以完全不一样。每个 source 独立计算自己的 `routes[target]`。
- 一条 target 的 candidate 列表 = **0 到 N 个 direct + 0 到 2 个 relay**。direct 优先级永远高于 relay；relay（SN / ZoneGateway）只要在 zone_config 配置就一定写入，direct 存在时打 `backup=true`。
- direct candidate 由几路独立证据各自产出，可能并存：签名 IP / 直连 probe / IPv6 全局 / LAN endpoint / 同 net_id / target 公网可达 / 共同 OOD 推断。每路有自己的 priority 锚（1 / 10 / 14 / 19~24 / 18~25），用以稳定排序。
- **唯一互斥例外**：target 在 DeviceConfig 中有 owner 签名的 IP 时，所有其它 direct 路径全部不生成（防 IP 注入），仅签名 IP + relay backup。

## 2. 决策输入（哪些字段进了路由）

| 字段 | 来源 | 影响 |
|---|---|---|
| `device_doc.net_id` | DeviceConfig（激活时填）| 同 net_id / wan / wan_dyn / portmap / nat / lan* / unknown |
| `device_doc.ips` | 激活时签名 | target 有签名 IP → 唯一 direct 路径 |
| `network_observation.direct_probe[]` | node-daemon 周期上报 | reachable + 新鲜 → `direct-probed` |
| `network_observation.endpoints[]` | 系统接口枚举 | scope=lan → LAN IP candidate；scope=global+family=ipv6 → IPv6 candidate；用于子网一致性反证 |
| `network_observation.ipv6.state` | source 自己探测 | `egress_ok` / `rtcp_direct_ok` 才允许生成 v6 direct，target `unavailable` 则禁用 |
| `network_observation.observed_at` + `probe.last_success` + `freshness_ttl_secs` | 同上 | `observed_at - last_success > ttl` → probe 视为 stale，丢弃 |
| `zone_config.sn` / `get_default_zone_gateway()` | ZoneConfig | 决定是否写 SN / ZoneGateway relay |

## 3. node-node 物理链路场景矩阵

测试覆盖建议：每个场景至少跑一次 `build_forward_plan(source, ...)` 然后 dump `routes[target]` 看候选集合。

| # | 物理拓扑 | source 标签 / 信号 | target 标签 / 信号 | 期望出现的 direct candidate | 期望 relay |
|---|---|---|---|---|---|
| 1 | 家用同 LAN，单子网 | lan1, lan ip 192.168.1.10 | lan1, lan ip 192.168.1.20 | `direct-lan-endpoint-...` + `direct-net-id` | SN/ZG 若配置则 backup |
| 2 | 同 net_id 标签但实测不同子网（VPN/桥接误标） | lan1, 192.168.1.x | lan1, 10.0.0.x | **无 direct**（子网反证） | 仅 relay |
| 3 | 同 LAN 但都未填 net_id | None | None | `direct-net-id`（unknown_lan 视同同 LAN） | |
| 4 | 同 LAN + 已 probe 成功 + 上报 lan endpoint | lan1 + probe → ood2 | lan1 + lan endpoint | `direct-probed` + `direct-lan-endpoint-*` + `direct-net-id` | 三层叠加，URL 不同不会被去重 |
| 5 | 同 LAN + probe 已 stale | lan1，probe last_success+ttl < observed_at | lan1 | **无 `direct-probed`**，仍有 `direct-net-id` | 新鲜度过滤 |
| 6 | source LAN/NAT，target 公网静态签名 IP | nat / lan1 | wan + 签名 ips | **仅 `direct-signed-wan-ip`** | SN backup，但 keep_tunnel=false |
| 7 | source LAN/NAT，target wan_dyn 无签名 IP（DDNS） | nat | wan_dyn 无 ips | `direct-wan-target`（DID hostname） | |
| 8 | source LAN，target portmap | lan1 | portmap | `direct-wan-target` | |
| 9 | 双 LAN 无 v6 无共同 OOD | lan1 | lan2 | **无 direct** | 仅 relay |
| 10 | 双 LAN + 共同非公网 OOD（双方都 probe 到 ood1，ood1.net_id 非 wan） | lan1 + probe→ood1 | lan2 + probe→ood1 | `direct-shared-ood-ood1`（+ `direct-lan-endpoint-*` 如有 endpoint） | |
| 11 | 双 LAN + witness OOD 在公网 | lan1 + probe→ood1(wan) | lan2 + probe→ood1(wan) | **无 direct**（witness 被过滤） | 仅 relay |
| 12 | 双 LAN，双方 IPv6 egress_ok，target 有 global v6 endpoint | lan1, ipv6.state=egress_ok | lan2, global v6 endpoint | `direct-ipv6-global` confidence=medium | |
| 13 | source v6 = address_only | lan1, ipv6.state=address_only | lan2 + global v6 | **无 v6 direct**（source 未验证出站） | |
| 14 | target v6 显式 unavailable | lan1, egress_ok | lan2, ipv6.state=unavailable + global v6 | **无 v6 direct** | |
| 15 | source rtcp_direct_ok | lan1, ipv6.state=rtcp_direct_ok | lan2 + global v6 | `direct-ipv6-global` confidence=**high** | |
| 16 | source 与 target 同节点 | this_node == target | - | **不进 routes** | selector 走 127.0.0.1 |
| 17 | 双方都 wan + 签名 IP | wan + ips | wan + ips | `direct-signed-wan-ip` | source net_id 不影响 |
| 18 | source 是 wan_dyn，target 是 nat / lan | wan_dyn | lan2 | **无 direct**（target 不公网，source 公网无意义） | 仅 relay |
| 19 | source wan，target portmap | wan | portmap | `direct-wan-target` | |
| 20 | 双 LAN 且 zone_config 既无 sn 又无 zone_gateway | lan1 | lan2 | 整个 target **不写入 routes**（candidate 列表空） | 测试要确认 `routes.get(target)` 返回 None |
| 21 | target 的签名 IP 是 IPv6 | nat | wan + 签名 ipv6 | `direct-signed-wan-ip` URL 格式 `rtcp://[v6]/` | |
| 22 | 旧格式 probe（直接挂 `device_doc.extra_info.direct_probe`，没有 network_observation 包裹） | 任意 | 任意 | `direct-probed` 仍生效，但**无 freshness 上下文**，永不 stale | 兼容路径 |
| 23 | source 无任何观测数据，target 有 lan endpoint | lan1 | lan1 + lan endpoint | `direct-lan-endpoint-*` + `direct-net-id`（source 缺数据不影响 target 提供 endpoint） | |
| 24 | 双方都标 `nat` | nat | nat | ⚠️ **当前实现：`direct-net-id`**（同 net_id 字面相同视同同 LAN） | 测试团队应特别关注：两个独立 NAT 后的设备并不真在同 LAN，这是实现已知的 over-trust |

## 4. 容易漏检的边界（建议重点扫）

1. **IPv6 link-local / ULA**：当前只消费 `scope=global` 的 v6 endpoint。fe80::/fc00:: 段不会进 candidate，即使在同一段 v6 LAN 也是。
2. **IPv4 子网阈值**：子网一致性硬编码 /24（v4）和 /64（v6）。校园网 /23、运营商大段 /20 会被错判为不同 LAN；某些 /25、/26 切分则会被错判为同 LAN。
3. **target 多 NIC**：`endpoints` 数组里多个 lan endpoint 时，`build_lan_endpoint_candidate` 只取第一条匹配的 ipv4。多网卡 target 不会写入多条 candidate。
4. **scheduler 自身时钟**：probe 新鲜度只用 `observation.observed_at - last_success` 比较，不用 scheduler 当前时间。从 device 上报到 scheduler 调度之间的延迟（可能数分钟到数小时）不会导致 probe 进一步 stale。
5. **同 net_id 标签的"假同 LAN"**：上面 #24 提到的 `nat` / `portmap` 同 net_id 误判；以及任意 `lan*` 标签拼写一致但物理不同 LAN 的场景。`endpoints` 子网反证只在双方都有 LAN endpoint 时才生效。
6. **probe 单向性**：probe 只在 source → target 方向发挥作用。source 能 probe 到 target 不代表反向 routes（target → source）也有 `direct-probed`。要分别测 A→B 和 B→A 两个 source 视角。
7. **签名 IP 互斥的副作用**：target 有签名 IP 时，**即使** source 与 target 同 LAN、有 lan endpoint、IPv6 都 ok，也只会写出签名 IP 一条 direct + relay backup。这是有意的安全边界，测试时不要把 candidate 缺失误报为 bug。
8. **target 缺数据但 source 有**：source 有 IPv6 egress_ok 但 target 没有 `network_observation` → 没有 `direct-ipv6-global`。反过来同理。两侧观测都需要齐备 v6 才生成。
9. **routes 完全为空的 target**：场景 #20 是允许的输出形态，调用方不应假定每个 device_list 中的 target 都在 routes 里。

## 5. 测试入口

```rust
zone_route_builder::build_forward_plan(
    this_node_id,            // source 节点 ID
    &zone_config,            // 含 sn / oods / zone_gateway
    &zone_host,              // 用于拼 DID hostname
    &device_list,            // HashMap<node_id, DeviceInfo>，每个 DeviceInfo
                             // 的 device_doc.extra_info["network_observation"]
                             // 决定 endpoints / probe / ipv6 信号
)
```

返回 `HashMap<target_node_id, Vec<NodeGatewayRouteCandidate>>`。candidate 中的 `id` / `kind` / `priority` / `backup` / `evidence.evidence_type` 是稳定的断言锚。