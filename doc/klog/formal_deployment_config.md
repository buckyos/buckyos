# klog 正式部署配置方案

本文给出 `klog-service` / `klog_daemon` 在 BuckyOS 中的正式部署配置方案，目标是让部署者可以基于当前代码直接落地：

1. 单机开发/验证环境
2. 正式单 Zone 多节点集群
3. 含 learner 的扩展部署

本文只描述当前代码已经支持、或者只需常规运维配合即可落地的方案。  
对于“所有节点都在 NAT 后、Raft peer 之间无法直连”的场景，本文会明确标注为当前不支持，而不是给出伪方案。

## 1. 适用范围与前提

当前代码下，`klog` 网络面分成两层：

1. 服务访问层
- 对外服务名：`klog-service`
- 对外入口：`/kapi/klog-service`
- 由 BuckyOS gateway / scheduler / service discovery 体系负责暴露和访问

2. 集群内部层
- Raft 控制面：`/klog/append-entries`、`/klog/vote`、`/klog/install-snapshot`
- 节点间业务转发：`/klog/data/*`
- 集群管理：`/klog/admin/*`
- 当前由 `klog` 自己按 `target.addr:port` 直连 HTTP 访问

因此，正式部署的前提是：

- `klog-service` 的客户端流量可以依赖 BuckyOS gateway
- `klog` Raft peer 之间必须存在稳定可达的专用 cluster network

推荐的 cluster network 形态：

- 同一 VPC / 内网
- WireGuard / Tailscale / ZeroTier
- 任意可保证 peer 间稳定互通的 overlay 网络

不推荐作为当前正式方案的网络形态：

- 仅依赖 zone gateway / node gateway 暴露 HTTP 端口
- 节点彼此没有专用互通网络，只能靠公网入口互相访问

## 2. 推荐部署拓扑

### 2.1 推荐拓扑：客户端面与集群面分离

```text
client / app / sdk
        |
        v
zone gateway :80/:443
        |
        v
node gateway :3180
        |
        v
klog-service rpc :4070


raft peer A  <---- dedicated cluster network ---->  raft peer B
   :21001 raft                                   :21001 raft
   :21002 inter                                  :21002 inter
   :21003 admin                                  :21003 admin
```

这里建议把职责严格分开：

- `4070` 只承载客户端 RPC，也就是 `/kapi/klog-service`
- `21001` / `21002` / `21003` 只承载集群内部流量
- 集群内部流量不要复用公共 gateway 的业务入口

### 2.2 正式环境中的 gateway 职责

gateway 在正式环境中负责：

1. 对外暴露 `klog-service`
2. 根据 scheduler 的 service info，把 `/kapi/klog-service` 转发到本机 `klog-service` 实例
3. 为 BuckyOS 其他服务提供 `127.0.0.1:3180/kapi/klog-service` 风格的统一访问入口

gateway 不负责以下内容：

1. Raft peer 的选举和复制
2. follower 到 leader 的 `/klog/data/*` 内部转发
3. admin 成员管理接口的公网暴露

## 3. 默认端口与角色

当前默认端口如下：

| 端口 | 配置项 | 作用 | 推荐暴露范围 |
| --- | --- | --- | --- |
| `21001` | `network.listen_addr` / `advertise_port` | Raft 控制面 | cluster network only |
| `21002` | `network.inter_node_listen_addr` / `advertise_inter_port` | 节点间业务转发 | cluster network only |
| `21003` | `network.admin_listen_addr` / `advertise_admin_port` | admin / join / membership | cluster network only |
| `4070` | `network.rpc_listen_addr` / `rpc_advertise_port` | client RPC | 本机直连 + 通过 gateway 暴露 |
| `3180` | node gateway | BuckyOS 本机服务访问入口 | localhost only |
| `80/443` | zone gateway | 外部 zone 访问入口 | external |

推荐边界：

- `4070` 可以绑定 `127.0.0.1`，由本机 gateway 转发
- `21001` / `21002` / `21003` 应绑定 cluster network 地址或 `0.0.0.0`，但只在内网 / overlay 网络放通

## 4. 必备配置项

以下配置在正式环境下必须显式配置，不建议依赖默认值。

### 4.1 身份与集群标识

```toml
node_id = 1

[cluster]
name = "prod-klog"
id = "zone-prod-klog-v1"
auto_bootstrap = false
```

要求：

- `node_id` 必须全局唯一，且大于 `0`
- `cluster.name` 和 `cluster.id` 都必须显式设置
- 正式环境默认 `auto_bootstrap = false`

只有首个引导节点，才允许临时设为：

```toml
[cluster]
auto_bootstrap = true
```

并且仅第一台节点这样配置。

### 4.2 数据目录

```toml
data_dir = "/var/lib/buckyos/klog-service"
```

要求：

- 使用稳定持久盘
- 不与其他服务混用
- 要保证 node restart 后数据可恢复

### 4.3 网络监听与 advertise

推荐用独立 overlay 网段，例如：

- node1: `10.90.0.11`
- node2: `10.90.0.12`
- node3: `10.90.0.13`

node1 配置示例：

```toml
[network]
listen_addr = "10.90.0.11:21001"
inter_node_listen_addr = "10.90.0.11:21002"
admin_listen_addr = "10.90.0.11:21003"
rpc_listen_addr = "127.0.0.1:4070"

advertise_addr = "10.90.0.11"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 4070
enable_rpc_server = true
```

设计原则：

- `listen_*` 决定本机绑定在哪个地址上
- `advertise_*` 决定其他 peer 如何找到当前节点
- 对 Raft 内部流量来说，`advertise_addr + advertise_*` 必须是 peer 真实可达地址

重要：

- `rpc_advertise_port` 目前不是集群内部关键依赖，可以维持本地服务语义
- `advertise_addr` 不应该填写公网 gateway 域名，除非后续传输层真的改成支持经 gateway 访问 Raft peer

### 4.4 admin 暴露控制

```toml
[admin]
local_only = false
```

推荐策略：

- 单机验证环境：`true`
- 正式多节点集群：`false`

原因：

- auto-join、change-membership、cluster-state 都依赖 admin peer 可达
- 如果 `local_only = true`，远端节点无法通过 admin 接口完成 join / membership

但即使 `local_only = false`，也只应在 cluster network 内可达，不能公网暴露。

### 4.5 join 配置

第二台及之后的节点需要配置：

```toml
[join]
targets = ["10.90.0.11:21003"]
target_role = "voter"
blocking = false
```

要求：

- `join.targets` 指向现有集群节点的 `admin` 地址
- 不能写 `127.0.0.1:*`
- 不能写 zone gateway 的对外业务域名，除非明确把 admin 路由纳入 cluster network 代理

推荐：

- 初始 3 节点集群先都作为 `voter`
- 大规模扩容时先 `learner`，追平后再晋升

### 4.6 durability 相关

```toml
state_store_sync_write = true
```

推荐保持默认：

- `true`

同时：

- Raft log backend 推荐使用 RocksDB
- 正式环境不建议继续只依赖内存 log/store

### 4.7 RPC 路由策略

```toml
[rpc.append]
timeout_ms = 3000
body_limit_bytes = 1048576
concurrency = 128

[rpc.query]
timeout_ms = 3000
body_limit_bytes = 1048576
concurrency = 128

[rpc.jsonrpc]
timeout_ms = 3000
body_limit_bytes = 1048576
concurrency = 128
```

正式环境建议：

- append/query/jsonrpc 三类策略分开配置
- gateway 的请求体限制要不小于 `jsonrpc.body_limit_bytes`
- 如果外部存在高并发写入，优先收敛到 `append/jsonrpc` 限流，而不是挤压 Raft 内部流量

### 4.8 OpenRaft 参数

推荐先使用当前默认值，除非已有明确压测结论：

```toml
[raft]
election_timeout_min_ms = 150
election_timeout_max_ms = 300
heartbeat_interval_ms = 50
install_snapshot_timeout_ms = 200
max_payload_entries = 300
replication_lag_threshold = 5000
snapshot_policy = "since_last:5000"
snapshot_max_chunk_size_bytes = 3145728
max_in_snapshot_log_to_keep = 1000
purge_batch_size = 1
```

调优原则：

- 跨机房、高 RTT 环境要适当放大 election timeout
- 大日志吞吐场景先做压测，再调大 `max_payload_entries`
- learner 大量追平场景重点观察 snapshot 配置

## 5. 推荐配置模板

### 5.1 三 voter 正式集群

#### node1

```toml
node_id = 1
data_dir = "/var/lib/buckyos/klog-service"

[cluster]
name = "prod-klog"
id = "zone-prod-klog-v1"
auto_bootstrap = true

[network]
listen_addr = "10.90.0.11:21001"
inter_node_listen_addr = "10.90.0.11:21002"
admin_listen_addr = "10.90.0.11:21003"
rpc_listen_addr = "127.0.0.1:4070"
advertise_addr = "10.90.0.11"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 4070
enable_rpc_server = true

[admin]
local_only = false

[join]
targets = []
target_role = "voter"
blocking = false
```

#### node2

```toml
node_id = 2
data_dir = "/var/lib/buckyos/klog-service"

[cluster]
name = "prod-klog"
id = "zone-prod-klog-v1"
auto_bootstrap = false

[network]
listen_addr = "10.90.0.12:21001"
inter_node_listen_addr = "10.90.0.12:21002"
admin_listen_addr = "10.90.0.12:21003"
rpc_listen_addr = "127.0.0.1:4070"
advertise_addr = "10.90.0.12"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 4070
enable_rpc_server = true

[admin]
local_only = false

[join]
targets = ["10.90.0.11:21003"]
target_role = "voter"
blocking = false
```

#### node3

```toml
node_id = 3
data_dir = "/var/lib/buckyos/klog-service"

[cluster]
name = "prod-klog"
id = "zone-prod-klog-v1"
auto_bootstrap = false

[network]
listen_addr = "10.90.0.13:21001"
inter_node_listen_addr = "10.90.0.13:21002"
admin_listen_addr = "10.90.0.13:21003"
rpc_listen_addr = "127.0.0.1:4070"
advertise_addr = "10.90.0.13"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 4070
enable_rpc_server = true

[admin]
local_only = false

[join]
targets = ["10.90.0.11:21003"]
target_role = "voter"
blocking = false
```

### 5.2 三 voter + 两 learner

在上面 3 voter 的基础上，新增：

- node4 `10.90.0.14`
- node5 `10.90.0.15`

推荐：

```toml
[join]
targets = ["10.90.0.11:21003"]
target_role = "learner"
blocking = false
```

等 learner 追平后，再通过 admin 变更成 voter。

## 6. BuckyOS gateway 配置要求

### 6.1 必须经过 gateway 的只有 `klog-service`

当前 scheduler 会为 kernel service 自动生成：

- `/kapi/klog-service`

因此 gateway 的正式职责是把 `/kapi/klog-service` 转发到本机 `rpc_port`。

推荐：

- `rpc_listen_addr = 127.0.0.1:4070`
- 不把 `4070` 直接对公网暴露
- 对外只暴露 zone gateway 的 `80/443`

### 6.2 不要把 admin/raft/inter 直接挂到公共 gateway

当前不推荐把下面这些流量直接塞进公共 service 路由：

- `/klog/append-entries`
- `/klog/vote`
- `/klog/install-snapshot`
- `/klog/data/*`
- `/klog/admin/*`

理由：

1. 这些不是标准 `/kapi` 服务流量
2. 它们对时延、稳定性更敏感
3. 会扩大攻击面
4. 当前 `klog` 代码本身也没有按 gateway 模型组织 peer target

## 7. 防火墙与 ACL 建议

### 7.1 zone gateway

公网开放：

- `80`
- `443`

### 7.2 node gateway

仅本机开放：

- `3180`

### 7.3 klog-service rpc

仅本机开放：

- `4070`

### 7.4 klog cluster network

仅 cluster network 开放：

- `21001`
- `21002`
- `21003`

不应对公网开放：

- `21001`
- `21002`
- `21003`
- `4070`

## 8. 启动顺序建议

### 8.1 首次部署

1. 部署 node1，设置：
   - `auto_bootstrap = true`
   - `join.targets = []`
2. 确认 node1 成为 leader
3. 部署 node2、node3，设置：
   - `auto_bootstrap = false`
   - `join.targets = ["node1-admin"]`
4. 等 3 voter 稳定
5. 如需 learner，再追加 node4、node5

### 8.2 重启要求

重启后不要改动：

- `node_id`
- `cluster.id`
- `data_dir`

如果这些发生变化，会破坏持久化状态和集群身份一致性。

## 9. 当前不支持的正式部署方式

以下场景不应作为当前版本的正式交付承诺：

1. 所有节点都在 NAT 后，peer 之间无专用互通网络
2. 仅通过 zone gateway / node gateway 暴露的公共地址互相做 Raft 复制
3. 用 `/kapi/klog-service` 替代所有 Raft peer 内部流量

如果要支持这些场景，需要单独改造：

- `KNetworkClient`
- `KDataClient`
- `advertise_*` 语义
- 以及 peer transport 抽象

## 10. 运维检查清单

正式部署前至少确认：

1. 每个节点 `cluster.name` / `cluster.id` 一致
2. 每个节点 `node_id` 唯一
3. `advertise_addr + advertise_*` 在 cluster network 上真实可达
4. `rpc_listen_addr` 只绑定到本机
5. `admin_local_only = false`，但 `admin_port` 仅在 cluster network 开放
6. `join.targets` 使用的是远端 `admin` 地址
7. `state_store_sync_write = true`
8. zone gateway 能正确转发 `/kapi/klog-service`
9. `3180` 只本机开放
10. peer 之间不依赖公网业务入口做 Raft 复制

## 11. 与现有文档的关系

当前仓库里已有一份：

- `src/kernel/klog_daemon/readme.md`

其中关于“通过 gateway 转发访问 `klog_daemon` 时，raft/inter/admin 也可通过 gateway 转发”的表述，与当前代码实现和本文推荐拓扑并不完全一致。

按当前代码和验证结果，应该以本文为准：

- `rpc` / `/kapi/klog-service` 走 gateway
- `raft/inter/admin` 走专用 cluster network

后续应再统一更新旧文档，避免产生错误部署预期。
