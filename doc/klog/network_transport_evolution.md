# klog network 方案设计

本文设计 `klog` 后续的 network 方案，目标是让 `klog` 同时兼容两类部署环境：

1. 直连环境
- 节点之间具备稳定的专用网络，可直接通过 `ip:port` 通信

2. gateway/proxy 环境
- 节点之间无法保证裸 `ip:port` 直连
- 需要借助 BuckyOS 的 gateway / rtcp / route map / proxy 进行中转

本文不直接修改当前实现，而是给出一版后续可落地的设计方案。

## 1. 设计目标

### 1.1 必须满足

1. 保留当前直连模式的低成本和低时延
2. 支持借助 gateway/proxy 转发内部 cluster 流量
3. 不影响现有 `klog-service` 的 `/kapi/klog-service` 访问语义
4. 不把 Raft 内部控制面直接暴露到公共业务入口
5. transport 能按流量类别区分：
   - raft control
   - inter-node data/meta
   - admin/membership

### 1.2 不追求一步到位

这版设计不试图直接把所有 cluster 流量塞进当前 `/kapi` 服务模型。  
原因是：

- `/kapi` 面向的是 service discovery 和业务请求
- Raft peer 流量要求明确指定目标节点
- gateway 现有的 `forward_to_service` 更偏向服务发现和 selector，而不是“精确 peer addressing”

因此更合理的方向是：

- 保持 `klog-service` 的 service 面不变
- 为 `klog` cluster traffic 设计独立的 transport abstraction 和内部路由面

## 2. 当前实现的限制

当前限制来自 `klog` 的 network client：

- `KNetworkClient` 和 `KDataClient` 都直接按 `target.addr + target.port` 构造 HTTP 地址
- `KNode` 中保存的是裸 `addr / port / inter_port / admin_port / rpc_port`
- `advertise_addr` / `advertise_*` 语义是“给 peer 一个直接可访问的地址”

这意味着：

- 直连环境可工作
- 只要节点不能直连，当前 Raft peer 网络就会失败

## 3. 方案总览

### 3.1 统一引入 Cluster Transport 层

建议在 `klog` 内部增加一个独立概念：

- `KClusterTransport`

它专门负责：

- peer endpoint 解析
- transport mode 选择
- 直连与 gateway/proxy 的切换

业务层和 openraft adapter 不再直接拼 `http://{addr}:{port}`。

### 3.2 三种模式

建议统一支持三种 transport mode：

1. `direct`
- 只走直连
- 对当前实现最接近
- 适合同内网 / VPC / overlay 完整互通环境

2. `gateway_proxy`
- 只走 gateway/proxy
- 适合节点之间不能直连，但本地 node gateway / RTCP 可用的环境

3. `hybrid`
- 优先直连
- 直连不可达或超时后回退到 gateway/proxy
- 适合作为默认演进模式

推荐默认值：

- 开发和压测：`direct`
- 正式生产首版：`direct`
- NAT / 多网络环境实验版：`hybrid`

`gateway_proxy` 作为受控环境下的显式模式，不建议默认启用。

## 4. 流量分类与 transport 策略

### 4.1 client rpc

流量：

- `/kapi/klog-service`

策略：

- 始终走现有 BuckyOS service/gateway 体系
- 不纳入本设计的 cluster transport

### 4.2 raft control plane

流量：

- `/klog/append-entries`
- `/klog/vote`
- `/klog/install-snapshot`

策略：

- `direct`：`peer.direct_addr + peer.raft_port`
- `gateway_proxy`：`local node gateway -> remote node gateway/proxy -> peer.raft_port`
- `hybrid`：优先直连，失败后切 proxy

### 4.3 inter-node data plane

流量：

- `/klog/data/append`
- `/klog/data/query`
- `/klog/data/meta-put`
- `/klog/data/meta-delete`
- `/klog/data/meta-query`

策略：

- 与 raft control plane 使用同一套 transport mode
- 但允许更独立的 timeout / body limit / retry policy

### 4.4 admin plane

流量：

- `/klog/admin/add-learner`
- `/klog/admin/remove-learner`
- `/klog/admin/change-membership`
- `/klog/admin/cluster-state`

策略：

- `direct`：走 peer admin address
- `gateway_proxy`：经内部 proxy 路由
- `hybrid`：优先直连，失败回退 proxy

特殊点：

- 如果请求最终是经“远端本机 gateway -> 本机 daemon”转发进入，那么服务端看到的来源是本地环回地址
- 这意味着 `admin_local_only = true` 在 proxy 模式下仍然可能成立

这点是值得保留的能力。

## 5. 核心数据结构设计

### 5.1 扩展 `KNode`

当前 `KNode` 只够表达直连 endpoint，不够表达 gateway/proxy 所需的目标身份信息。

建议扩展为：

```rust
pub struct KNode {
    pub id: KNodeId,

    pub addr: String,
    pub port: u16,
    pub inter_port: u16,
    pub admin_port: u16,
    pub rpc_port: u16,

    pub node_name: Option<String>,
    pub device_did: Option<String>,
    pub transport: Option<KNodeTransportAdvertise>,
}
```

其中：

- `node_name`：BuckyOS scheduler / gateway 语义下的节点名，例如 `ood1`
- `device_did`：未来如果要接 RTCP / route map，最好保留 DID 语义
- `transport`：声明本节点支持哪些集群传输方式

### 5.2 新增 transport advertise 结构

```rust
pub struct KNodeTransportAdvertise {
    pub preferred_mode: KClusterTransportMode,
    pub direct: Option<KDirectTransportAdvertise>,
    pub proxy: Option<KProxyTransportAdvertise>,
}

pub enum KClusterTransportMode {
    Direct,
    GatewayProxy,
    Hybrid,
}

pub struct KDirectTransportAdvertise {
    pub raft_addr: String,
    pub inter_addr: String,
    pub admin_addr: String,
}

pub struct KProxyTransportAdvertise {
    pub target_node_name: String,
    pub gateway_route_prefix: String,
}
```

设计要点：

- `direct` 和 `proxy` 都可以同时存在
- `Hybrid` 模式下不需要重新做配置切换，只是客户端选择策略不同
- `target_node_name` 用于和 `NODE_ROUTE_MAP` / scheduler 节点标识对齐

## 6. gateway/proxy 模式的路由设计

### 6.1 不复用 `/kapi/klog-service`

不建议把 cluster traffic 混入：

- `/kapi/klog-service`

原因：

1. `/kapi` 代表 service 面，而不是 peer 面
2. `/kapi` 的 selector 语义是“选一个可用实例”，不适合 Raft 精确指向某个 peer
3. Raft 请求需要严格路由到指定目标节点

### 6.2 建议新增独立的内部 route prefix

建议使用独立的内部路径前缀，例如：

```text
/.cluster/klog/<target-node>/<plane>/...
```

其中：

- `<target-node>`：目标节点名，例如 `ood1`
- `<plane>`：`raft` / `inter` / `admin`

示例：

```text
/.cluster/klog/ood2/raft/append-entries
/.cluster/klog/ood3/inter/meta-put
/.cluster/klog/ood1/admin/change-membership
```

这样有几个好处：

1. 目标节点是显式的，不依赖 service selector
2. 三类 plane 分离，便于不同 ACL 和限流
3. 可以在 gateway process-chain 中精确匹配和转发

### 6.3 本地 node gateway 的行为

本地 node gateway 需要一条内部 process-chain 规则：

1. 匹配 `/.cluster/klog/<target-node>/<plane>/...`
2. 如果 `<target-node>` 是本机：
   - 按 `<plane>` 转发到本机 `127.0.0.1:{raft/inter/admin port}`
3. 如果 `<target-node>` 不是本机：
   - 从 `NODE_ROUTE_MAP` 找到该节点对应 route
   - 转发到远端 node gateway 的 `3180`
   - 保留原始路径

这样 remote node gateway 收到同一路径后，会再次判断：

- 目标节点就是自己
- 再转发到本机 `klog_daemon` 的对应 plane 端口

### 6.4 为什么使用 node gateway 而不是 zone gateway

建议 cluster proxy 基于 node gateway，而不是 zone gateway。

原因：

1. node gateway 是内部设施，本来就适合节点间中转
2. zone gateway 更偏向外部流量入口
3. node gateway 已经有 `NODE_ROUTE_MAP` 和 RTCP 基础能力
4. 把 cluster traffic 从公共入口隔离开，更安全

## 7. transport 选择算法

### 7.1 direct 模式

算法：

1. 从 `peer.transport.direct` 读取目标地址
2. 直接构造 HTTP 请求
3. 不做 proxy fallback

适用：

- 单内网 / 单 VPC
- 延迟敏感环境

### 7.2 gateway_proxy 模式

算法：

1. 构造本地 node gateway URL：

```text
http://127.0.0.1:3180/.cluster/klog/<target-node>/<plane>/...
```

2. 本地 gateway 负责把请求转给远端 node gateway
3. 远端 node gateway 再转给本机 `klog_daemon`

适用：

- peer 之间不能稳定直连
- 已具备 node gateway / rtcp route map

### 7.3 hybrid 模式

算法：

1. 优先尝试 direct
2. 遇到以下错误时回退到 proxy：
   - connect refused
   - timeout
   - unreachable
3. 业务返回类错误不回退：
   - invalid argument
   - membership conflict
   - not leader

建议：

- 对 `vote` 类请求，fallback 超时时间要更保守
- 对 `install_snapshot`，尽量固定使用 direct 或固定使用 proxy，不建议在大 payload 上频繁切换

## 8. 配置模型建议

建议在 `klog_daemon` 配置中新增一段：

```toml
[cluster_network]
mode = "direct"                # direct | gateway_proxy | hybrid
gateway_addr = "127.0.0.1:3180"
gateway_route_prefix = "/.cluster/klog"
direct_connect_timeout_ms = 300
proxy_connect_timeout_ms = 3000
prefer_direct_for_snapshot = true
```

设计说明：

- `gateway_addr` 指本机 node gateway
- `gateway_route_prefix` 是内部 cluster proxy 路由前缀
- `direct_connect_timeout_ms` 用于 hybrid 模式快速探测直连可达性
- `prefer_direct_for_snapshot` 用于避免大块 snapshot 经 proxy 带来额外放大

## 9. 与现有实现的兼容性

### 9.1 向后兼容

首版改造建议：

- 默认 `mode = "direct"`
- 如果没有 `transport` 字段，仍按旧的 `addr + port` 工作

这样：

- 现有测试和部署不需要立刻调整
- 新模式只在明确配置时生效

### 9.2 渐进演进

建议分三步：

#### Phase 1

- 引入 `KClusterTransportMode`
- 封装 endpoint builder
- 不改 gateway，只保留 direct

#### Phase 2

- 给 `KNode` 增加 node identity / proxy advertise 信息
- 引入 `gateway_proxy` 模式
- 增加内部 route prefix 设计文档和 gateway config 生成逻辑

#### Phase 3

- 增加 `hybrid`
- 增加 direct -> proxy fallback
- 增加可观测性指标：
  - direct 成功率
  - proxy 成功率
  - fallback 次数
  - plane 维度延迟

## 10. 安全与运维要求

### 10.1 ACL 边界

cluster proxy 路径不应进入公共 service 面。

建议：

- `/.cluster/klog/*` 只允许 node gateway 内部流量或 zone 内可信设备访问
- 禁止从公网直接访问该路径

### 10.2 限流与隔离

即使后续采用 gateway_proxy，也应分开对待：

- raft control
- inter-node data
- admin

至少要区分：

1. timeout
2. body limit
3. concurrency

### 10.3 可观测性

建议后续增加：

- 每种 transport mode 的请求计数
- fallback 触发计数
- direct/proxy 分 plane 延迟
- 目标节点维度错误码分布

## 11. 推荐结论

### 11.1 短期推荐

当前最稳妥路线：

- `klog-service` 继续通过 `/kapi/klog-service` 接入 gateway
- cluster network 保持 `direct`
- 正式环境通过 overlay 网络解决 peer 直连

### 11.2 中期推荐

如果明确存在“节点无法直连”的部署需求：

- 引入本文的 `KClusterTransport` 抽象
- 先实现 `gateway_proxy`
- 最后实现 `hybrid`

不要直接把当前 `KNetworkClient` 里的 URL 拼接改成 gateway 地址硬编码。  
那样只会把“一个直连假设”替换成“另一个硬编码假设”，后面还会重构一次。

## 12. 后续落地入口

建议后续实现从这几个文件切入：

- `src/kernel/klog/src/network/client.rs`
  - endpoint builder 和 transport mode 选择

- `src/kernel/klog/src/lib.rs`
  - `KNode` 扩展

- `src/kernel/klog_daemon/src/config.rs`
  - 新增 `cluster_network` 配置段

- `src/rootfs/etc/boot_gateway.yaml`
  - 增加 `/.cluster/klog/*` 的内部 route 规则

- `src/kernel/scheduler`
  - 如果最终要自动生成 gateway cluster route，需要接入 scheduler 的 gateway_config 生成链路
