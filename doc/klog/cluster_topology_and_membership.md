# klog 集群拓扑与成员变更指南

本文从系统架构与运维视角说明 `klog-service` / `klog_daemon` 在不同节点规模下的推荐拓扑、能力边界和成员变更方式，重点覆盖：

1. 单节点、双节点、多节点的部署差异
2. 每种拓扑下 gateway 与 cluster network 的职责边界
3. 扩容、缩容、替换节点时的推荐流程
4. 当前代码已经支持什么、还不建议做什么

本文与以下文档互补：

- `doc/klog/formal_deployment_config.md`
- `doc/klog/traffic_matrix_and_topology.md`
- `doc/klog/network_transport_evolution.md`

## 1. 先看结论

### 1.1 从可用性角度的推荐顺序

- 开发与本地验证：单节点
- 两台机器但不要求高可用：`1 voter + 1 learner`
- 正式高可用：`3 voter`
- 扩容与读副本：`3 voter + N learner`

### 1.2 从 gateway 依赖角度的结论

- `klog-service` 作为 BuckyOS 服务对外访问时，仍然应通过 `/kapi/klog-service`，这一层天然依赖 gateway 体系
- `klog` 自身的 Raft 复制、投票、snapshot、admin、inter-node data/meta 转发，是否依赖 gateway 取决于 `cluster_network.mode`
- 单节点情况下几乎不存在真正的 cluster 内部通信，因此不会依赖 gateway 来完成集群复制

### 1.3 不推荐的拓扑

- `2 voter` 作为正式高可用集群
- 所有节点都在 NAT 后且彼此完全不能直连，但又没有启用并验证 `gateway_proxy/hybrid` 的集群
- 在公网直接暴露 `raft/inter/admin` 端口

## 2. 节点角色与身份语义

当前 `klog` 需要明确区分两种身份：

1. `raft_node_id`
- 配置字段仍然是 `node_id`
- 类型为 `u64`
- 仅用于 OpenRaft 内部一致性与 membership

2. `node_name`
- 对应 BuckyOS 节点名
- 用于 gateway 路由、业务来源身份、节点可读标识
- 在启用 `gateway_proxy/hybrid` 时是外部主身份

推荐理解方式：

- `node_id` 是内部一致性身份
- `node_name` 是系统与运维层的主身份

## 3. 单节点拓扑

### 3.1 适用场景

适用于：

- 本地开发
- 单机 DV 验证
- 只要求“可用”，不要求副本与容错
- 先把 `klog-service` 接入 BuckyOS，再逐步扩成集群

### 3.2 拓扑示意

```text
client / app
   |
   v
zone gateway / node gateway
   |
   v
klog-service rpc :4070
   |
   v
single raft voter
```

### 3.3 特点

- 唯一节点同时是 leader 和唯一 voter
- 没有 follower、learner，也没有真正的 peer replication
- cluster 内部流量几乎不存在

### 3.4 gateway 依赖分析

单节点下要分两层看：

1. 服务访问层
- 如果调用走 `/kapi/klog-service`
- 则仍然依赖 BuckyOS gateway/service discovery

2. 集群内部层
- 没有其他 peer
- 不依赖 gateway 完成 Raft 复制或投票

所以单节点下可以明确认为：

- `service access` 可能经过 gateway
- `cluster traffic` 不依赖 gateway

### 3.5 推荐配置

```toml
node_id = 1

data_dir = "/var/lib/buckyos/klog-service"

[cluster]
name = "dev-klog"
id = "dev-klog"
auto_bootstrap = true

[network]
listen_addr = "127.0.0.1:21001"
inter_node_listen_addr = "127.0.0.1:21002"
admin_listen_addr = "127.0.0.1:21003"
rpc_listen_addr = "127.0.0.1:4070"
advertise_addr = "127.0.0.1"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 4070
enable_rpc_server = true

[admin]
local_only = true
```

### 3.6 风险与限制

- 无任何副本冗余
- 进程或节点故障即不可用
- 不适合作为生产高可用拓扑

## 4. 双节点拓扑

双节点需要拆成两种完全不同的语义。

### 4.1 模式 A：`1 voter + 1 learner`

这是双节点里更合理的一种。

#### 4.1.1 适用场景

- 两台机器，希望有一份副本
- 但暂时做不到 `3 voter`
- 允许唯一 voter 故障时不可写

#### 4.1.2 特点

- voter 负责 quorum 与提交
- learner 负责复制，但不参与投票
- learner 可用于只读、副本同步、后续升级为 voter 的过渡节点

#### 4.1.3 拓扑示意

```text
node1: voter (leader)
node2: learner
```

#### 4.1.4 推荐配置策略

node1：

```toml
node_id = 1

[cluster]
name = "prod-klog"
id = "prod-klog-v1"
auto_bootstrap = true
```

node2：

```toml
node_id = 2

[cluster]
name = "prod-klog"
id = "prod-klog-v1"
auto_bootstrap = false

[join]
targets = ["10.90.0.11:21003"]
target_role = "learner"
blocking = false
```

#### 4.1.5 优缺点

优点：

- 有第二份副本
- 扩容到 3 节点时更自然
- 比 `2 voter` 更符合实际预期

缺点：

- 不是高可用
- voter 节点故障后无法继续写入
- learner 不会自动顶替为 leader

### 4.2 模式 B：`2 voter`

这个模式技术上可以组，但不应被当成高可用拓扑。

#### 4.2.1 原因

两节点都为 voter 时：

- quorum = 2
- 任一节点故障，剩余节点都无法形成多数派
- 结果是不能继续提交写入

#### 4.2.2 容错误区

很多人会误以为：

- 两台机器，两个副本
- 坏一台还能继续工作

但对 Raft 来说，这个判断是错的。复制副本数量不等于可形成多数派。

#### 4.2.3 结论

`2 voter` 可以作为：

- 协议研究
- 非生产实验环境
- 语义测试

不建议作为：

- 正式生产 HA 拓扑

## 5. 三节点及以上拓扑

### 5.1 最小生产推荐：`3 voter`

这是当前最推荐的正式高可用起点。

#### 5.1.1 特点

- quorum = 2
- 任意 1 个 voter 故障，仍可继续写入
- leader 故障后仍能完成重新选举

#### 5.1.2 典型拓扑

```text
node1: voter
node2: voter
node3: voter
```

#### 5.1.3 推荐用途

- 正式生产
- 强一致业务日志
- 需要真实 failover 的系统元数据存储

### 5.2 扩展拓扑：`3 voter + N learner`

适用于：

- 跨机房只读副本
- 扩容前先追日志
- 分阶段扩节点
- 观察节点、迁移节点、预热节点

#### 5.2.1 特点

- voter 数量保持在奇数
- learner 不影响 quorum
- 可以逐步升级 learner 为 voter

#### 5.2.2 推荐模式

- 小规模：`3 voter + 1 learner`
- 中规模：`3 voter + 2 learner`
- 更高可用要求：`5 voter + N learner`

### 5.3 什么时候从 3 voter 升级到 5 voter

仅当你明确需要以下能力时再考虑：

- 容忍 2 个 voter 同时故障
- 跨多个 fault domain / zone 分布
- 有足够网络与运维能力承受更高复制开销

否则 `3 voter` 通常是更平衡的选择。

## 6. gateway 与 cluster network 在不同拓扑中的边界

### 6.1 单节点

- `klog-service` 访问可以经过 gateway
- cluster 内部复制不存在，不依赖 gateway

### 6.2 双节点与多节点

要看 `cluster_network.mode`：

1. `direct`
- `raft/inter/admin` 只走节点直连
- 不依赖 gateway 承载 cluster traffic

2. `gateway_proxy`
- `raft/inter/admin` 经 node gateway 路由
- 依赖 gateway

3. `hybrid`
- 先直连，失败后回退 gateway
- 部分依赖 gateway

### 6.3 当前推荐

当前正式环境仍然建议：

- `service access` 走 gateway
- `cluster traffic` 优先走专用 cluster network
- `gateway_proxy/hybrid` 作为网络受限环境下的增强能力，而不是默认基础方案

## 7. 增加节点

### 7.1 增加 learner

这是最安全、最推荐的第一步。

#### 7.1.1 适合场景

- 扩容新节点
- 预热数据
- 新机房加副本
- 替换旧节点前的准备动作

#### 7.1.2 推荐流程

1. 在新节点部署 `klog_daemon`
2. 配置相同的 `cluster.name` / `cluster.id`
3. `auto_bootstrap = false`
4. 设置 `join.targets` 指向现有 admin peer
5. `target_role = "learner"`
6. 等待 learner 追平日志
7. 确认 cluster-state 中 learner 状态正常

### 7.2 learner 晋升为 voter

#### 7.2.1 适合场景

- 从 `3 voter` 扩成 `5 voter`
- 替换旧 voter 时的先加后换
- 两节点过渡到三节点时，把 learner 晋升成第三个 voter

#### 7.2.2 注意点

- 必须等 learner 先追平
- 不要在 membership 正在变更时叠加下一次变更
- 优先一次只做一个成员变更动作

### 7.3 直接加入 voter 是否可行

可以，但不建议作为默认流程。

原因：

- 新节点未追平前就参与 quorum，更容易引发 membership 变更时序问题
- 对 leader 和复制链路冲击更大
- 在 `gateway_proxy/hybrid` 模式下，排障也更复杂

推荐做法仍然是：

- 先 learner
- 再 voter

## 8. 删除节点

### 8.1 删除 learner

删除 learner 是最简单的一类变更。

适用于：

- 缩容只读副本
- 迁移失败回滚
- 实验节点下线

推荐流程：

1. 先确认该节点不是 voter
2. 执行 remove learner
3. 确认 cluster-state 中 learners 已消失
4. 再停止并清理节点进程与数据目录

### 8.2 删除 voter

删除 voter 必须非常谨慎，因为会影响 quorum。

#### 8.2.1 风险判断

- `3 voter -> 2 voter`
  - 会把集群变成不推荐的非 HA 拓扑
- `5 voter -> 4 voter`
  - 仍然不推荐，因为 voter 数量变成偶数
- `5 voter -> 3 voter`
  - 可以，但最好分阶段控制风险

#### 8.2.2 推荐原则

- voter 数量优先保持奇数
- 删除 voter 前，优先先补新 learner 并追平
- 大改 membership 时，不要并发执行多个变更动作

### 8.3 替换节点

推荐使用“加新节点，再删旧节点”的方式。

标准流程：

1. 新节点以 learner 加入
2. learner 追平日志
3. 将新节点晋升为 voter
4. 再把旧 voter 从 membership 中移除
5. 最后停掉旧节点

这样可以避免：

- 先删后加导致 quorum 窗口变弱
- 替换期间写入不可用
- 新节点数据未追平就直接承担投票责任

## 9. 每种拓扑下的常见问题

### 9.1 单节点常见问题

- 误把单节点当成生产高可用部署
- `auto_bootstrap=true` 忘记在后续扩容时关闭
- 本地旧数据目录导致重启恢复异常

### 9.2 双节点常见问题

- 误以为 `2 voter` 等于高可用
- 用 `1 voter + 1 learner` 但又期待 learner 故障切换
- 迁移时直接删唯一 voter

### 9.3 多节点常见问题

- 多次 membership 变更并发触发
- learner 未追平就强行晋升
- `advertise_addr`、`advertise_node_name` 与真实可达路径不一致
- `gateway_proxy/hybrid` 下 route prefix、node_name、gateway 配置不一致

## 10. 当前实现已经覆盖到的阶段

已经有的覆盖：

- 单节点启动与业务写读
- 单节点重启恢复与 dedup
- 三节点 failover
- 三节点 gateway_proxy / hybrid 集成测试
- 单节点 node gateway smoke test

当前还缺：

- 专门的双节点测试
  - `1 voter + 1 learner`
  - `2 voter`
- 更完整的 gateway admin plane 集成测试
- `install-snapshot` 经 gateway 的专项测试
- 真实 `cyfs_gateway` DV 多节点 smoke

## 11. 推荐落地路径

### 11.1 开发阶段

- 用单节点验证功能
- 用三节点 direct 测 Raft 语义
- 用单节点 / 三节点 gateway 测服务访问和 transport 契约

### 11.2 首次生产上线

优先推荐：

- `3 voter`
- `cluster_network.mode = direct`
- gateway 只承载 `/kapi/klog-service`

### 11.3 后续演进

- 需要更多副本时，先加 learner
- 需要更复杂网络适配时，再引入 `hybrid`
- 只有在明确验证过 node gateway / route-prefix / membership 语义后，才考虑正式启用 `gateway_proxy`

## 12. 一句话建议

- 单节点：适合开发，不依赖 gateway 完成 cluster 复制
- 双节点：可以跑，但不等于高可用；优先 `1 voter + 1 learner`
- 多节点：正式推荐从 `3 voter` 起步
- 扩缩容：优先 `learner -> 追平 -> voter`，避免直接硬切 membership
