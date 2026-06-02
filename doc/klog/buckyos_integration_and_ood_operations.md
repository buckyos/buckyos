# klog 的 BuckyOS 集成与 OOD 运维模型

本文从 BuckyOS 视角整理 `klog-service` 的集成边界、组件依赖、system_config 替换路径，以及 OOD 集群扩容/缩容时的运维规则。后续讨论和实现调整应优先更新本文，再同步到更细的 transport、deployment 或测试文档。

## 1. 定位

`klog` 在 BuckyOS 中的目标是替换传统 etcd 角色，为多 OOD 模式提供 system_config 的一致性存储能力。核心原则：

1. klog 只运行在 OOD 节点上。
2. OOD 上的 klog 节点默认都是 Raft voter。
3. 普通节点不需要运行本地 klog follower；普通节点读写 system_config 时，应通过任意可用 OOD 上的 system_config 服务完成。
4. klog 提供一致性、复制、membership 和强读写语义；BuckyOS/gateway 提供身份、权限、路由和服务编排边界。

当前 system_config 切换到 klog backend 仍是显式 opt-in，不是默认路径：

```bash
BUCKYOS_SYSTEM_CONFIG_STORE=klog
```

已有 sled 数据迁移到 klog 时，只允许一个 OOD 临时设置：

```bash
BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED=true
```

## 2. 组件职责

### system_config

`system_config` 是 BuckyOS 的配置真相源，对外仍暴露 `/kapi/system_config`。klog backend 只是替换其底层 KV provider，不改变上层调用方式。

当前 klog provider 的关键语义：

1. `get/set/create/delete/list/exec_tx` 映射到 klog meta KV。
2. 强读默认通过 leader 或 follower 转发保证 linearizable 语义。
3. `exec_tx` 通过 klog meta transaction 承载多 key 原子写入和 optimistic guard。
4. prefix list 已支持 cursor/pagination，system_config klog provider 会自动翻页。
5. meta revision 已按全局 `mod_revision` + tombstone 方向实现，并在 API 层暴露 `create_revision`、`mod_revision`、`version`；`revision` 仅作为兼容别名。delete 后 recreate 不会复用旧 revision，stale CAS 会被 tombstone revision 拦截。`meta-query` 已支持按 `revision` 查询历史可见值集合，`meta-changes` 已支持 active polling/short long-poll，compaction 已支持显式 admin 操作和可选 revision-count auto compaction。

### scheduler

scheduler 从 system_config 中读取系统状态，推导 OOD 上应运行哪些服务，以及 node gateway 需要的 route 信息。

与 klog 相关的输入和输出：

1. `services/klog-service/spec` 定义 klog-service 服务规格。
2. `services/klog-service/settings.deployment.mode = "ood_voters"` 表示 klog voter 集合来自 `boot/config.oods`。
3. 当 mode 为 `ood_voters` 时，scheduler 将 `klog-service` 调度到所有 OOD 节点。
4. scheduler 生成 `node_gateway_info.json#cluster_route_map`，其中 key 是 `klog-service`，route path 前缀默认是 `/.cluster/klog`。

scheduler 只做确定性推导，不应直接调用 klog admin API 做有副作用的 membership 操作。

### node-daemon

node-daemon 负责把本机拉到目标状态。klog 相关职责：

1. 在 OOD 节点安装并启动 `klog-service`。
2. 当 `BUCKYOS_SYSTEM_CONFIG_STORE=klog` 时，确保 klog-service 先于 system_config 启动。
3. 为 klog-service 注入 BuckyOS managed 环境变量，包括 `KLOG_NODE_ID`、`KLOG_ADVERTISE_NODE_NAME`、`KLOG_CLUSTER_NETWORK_MODE=gateway_proxy`、`KLOG_CLUSTER_GATEWAY_ROUTE_PREFIX`、`KLOG_JOIN_TARGETS`。
4. `KLOG_NODE_ID` 从本机 `DeviceConfig.id` 派生，是 Raft membership 身份，不是 BuckyOS 节点名。
5. 非 seed OOD 会获得 `KLOG_JOIN_TARGETS` 和 `KLOG_JOIN_TARGET_ROLE=voter`，新 OOD 启动后由 klog_daemon 自动完成 join。
6. 当前 `KLOG_JOIN_TARGETS` 应包含除自己之外的其它 OOD admin route；这样 bootstrap seed 不可用时，新 OOD 仍可通过任意在线 OOD 发现 leader 并加入。

node-daemon 适合做“本机服务启动/停止”和“新 OOD 自加入”的自动化，不适合单独决定全局 OOD 缩容。

### klog-daemon

klog-daemon 负责本机 klog 节点运行：

1. Raft control plane：默认 `21001`。
2. inter-node data/meta plane：默认 `21002`。
3. admin plane：默认 `21003`，正式 BuckyOS 场景应只监听 loopback。
4. client JSON-RPC plane：默认 `4080`，由 gateway 暴露为 `/kapi/klog-service`。
5. auto-join：当 `KLOG_AUTO_BOOTSTRAP=false` 且 `KLOG_JOIN_TARGETS` 非空时，启动后自动向任意可用 join target 查询 cluster-state，优先找到当前 leader 后执行 add-learner；若目标角色是 voter，再执行 change-membership promote。

klog-daemon 不做用户级 RBAC，也不应该根据 BuckyOS desired OOD 列表自行删除其他 Raft 成员。它只执行明确的 admin API 请求，并在一致性层面拒绝危险操作，例如直接移除当前 leader。

### node gateway / cyfs-gateway

gateway 有两类入口：

1. 业务服务入口：`/kapi/klog-service` 和 `/kapi/system_config`。
2. klog 集群内部入口：`/.cluster/klog/{node_name}/{raft|inter|admin}/...`。

注意两个名称层级：

1. `cluster_route_map` 的 key 是 `klog-service`。
2. 实际请求 path prefix 默认是 `/.cluster/klog`。

gateway 是 admin plane 的授权和暴露边界。ZoneGateway/公网业务入口不应暴露 `/klog/admin/*` 或 `/.cluster/klog/*`。

## 3. 启动和 rollout 顺序

### 新 Zone / 新环境

1. boot 配置中存在 OOD 列表。
2. system_config builder 写入 `services/klog-service/spec` 和默认 settings。
3. scheduler 根据 `deployment.mode = "ood_voters"` 生成 klog placement 和 cluster route map。
4. OOD node-daemon 启动 node gateway。
5. 如果启用 klog backend，node-daemon 先启动 klog-service，再启动 system_config。
6. 第一个 OOD 以 `KLOG_AUTO_BOOTSTRAP=true` 初始化单节点集群。
7. 后续 OOD 以 `KLOG_AUTO_BOOTSTRAP=false` 启动，并通过 auto-join 加入。

当前实现把 OOD 列表中的第一个 OOD 作为 bootstrap seed。它只用于全新集群初始化，不是长期固定 leader。长期更清晰的做法是在 Zone/部署配置中增加显式 `bootstrap_ood` 或等价字段；该字段只决定初始 bootstrap 节点，已有集群扩容仍应使用全部可用 OOD 作为 join candidates。

### 从 sled rollout 到 klog backend

推荐顺序：

1. 先确认 klog OOD voter 集群可用，gateway cluster route 可用。
2. 选择一个 OOD 作为 seed，同时设置 `BUCKYOS_SYSTEM_CONFIG_STORE=klog` 和 `BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED=true`。
3. 其它 OOD 只设置 `BUCKYOS_SYSTEM_CONFIG_STORE=klog`，不要设置 bootstrap flag。
4. seed OOD 首次迁移成功后移除 `BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED=true`。

该规则避免多个 OOD 把各自本地 sled 残留同时导入 klog。

## 4. OOD 拓扑建议

| 拓扑 | 可用性判断 | 建议 |
| --- | --- | --- |
| `1 voter` | 最小可用，无副本 | 适合单 OOD 家庭部署、开发、DV |
| `1 voter + 1 learner` | 有副本，voter 故障后不可写 | 适合第二 OOD 不稳定但希望保留副本的场景 |
| `2 voters` | 任一 voter 故障都会失去 quorum | 可以短期过渡，不应宣传为 HA |
| `3 voters` | 可容忍 1 个 voter 故障 | 推荐的最小稳定 HA 基线 |
| `3 voters + N learners` | HA + 扩容过渡/副本 | 适合后续扩展 |

家庭小集群里常见的“一台家中 OOD + 一台外部服务器 OOD”如果网络或供电不稳定，`2 voters` 的可用性可能反而差：任何一台掉线都会阻塞 system_config 强读写。此时更保守的选择是 `1 voter + 1 learner`，或者接受 `2 voters` 只在双节点都在线时可写的限制。

## 5. 新增 OOD

新增 OOD 的协议步骤是 add-learner、数据同步、promote voter，但在 BuckyOS managed 模式下不应要求人工逐步调用 admin API。

推荐自动化流程：

1. BuckyOS 层完成新 OOD 加入 Zone，并更新 `boot/config.oods`。
2. scheduler 推导新的 `klog-service` placement 和 `cluster_route_map`。
3. 新 OOD node-daemon 启动 klog-service，并注入 `KLOG_JOIN_TARGETS`，其中包含除自己之外的其它 OOD gateway admin route。
4. 新 OOD 的 klog-daemon auto-join 依次尝试这些 join targets，向可用集群的 leader 执行 add-learner。
5. 新 learner 复制已有 log/snapshot，确认能读取加入前数据。
6. 因 `KLOG_JOIN_TARGET_ROLE=voter`，auto-join 继续执行 change-membership，把该节点 promote 为 voter。
7. 运维或 reconciler 通过 cluster-state 确认 voters 包含新 OOD。

需要注意：

1. 新 OOD 必须能访问本机 node gateway，并能经 gateway route 到达现有 OOD admin plane。
2. `KLOG_NODE_ID` 必须由该 OOD 的 `DeviceConfig.id` 派生，不能复用旧设备的 node id。
3. 如果是替换 OOD，旧设备 DID 变化意味着这是新的 Raft 成员，不能把它视为原成员原地恢复。

## 6. 删除 OOD

删除 OOD 是破坏性 membership 变更，不建议由任意 node-daemon 或 klog-daemon 自行发现并执行。它需要一个 BuckyOS 级的运维动作或 membership reconciler，统一读取 desired OOD set、当前 klog membership、leader 状态和 system_config 可用性后再操作。

### planned remove

目标 OOD 在线且可控时：

1. 确认删除后仍满足期望拓扑，例如 `4 -> 3`、`3 -> 2`、`2 -> 1`。
2. 确认目标不是当前 leader；如果目标是 leader，需要先完成 leadership transfer，或在 `3 voters` 及以上场景中通过受控停止触发剩余 quorum 重新选主后再 shrink。
3. 对非 leader 目标执行 change-membership，把它从 voters 中 demote，通常保留为 learner。
4. 等待剩余 voters 提交新 membership，并确认 system_config/klog 强读写正常。
5. 执行 remove-learner。
6. 停止目标 OOD 上的 klog-service 和 system_config。
7. 更新或确认 BuckyOS 的 OOD 配置、scheduler 结果和 gateway route map 已收敛。

当前还没有正式的 transfer-leader API，因此“删除当前 leader”的自动化应保持保守。`2 voters -> 1 voter` 尤其不能先停 leader，否则剩余单节点没有 quorum，无法在线 shrink。

### unplanned failure

目标 OOD 已掉线时：

1. `3 voters -> 2 online`：剩余两个 voter 仍有 quorum。应等待重新选主，确认读写恢复，再在明确决定移除故障 OOD 后 shrink 到 2 voters。
2. `2 voters -> 1 online`：剩余单节点没有 quorum，不能强读、写入或提交 remove/demote。正常恢复路径是让故障 OOD 回来。
3. `1 voter -> 0 online`：klog/system_config 不可用，只能恢复该 OOD 或走灾备。

对于 `2 voters -> 1 online`，如果业务必须继续运行，只能设计明确的灾备流程，例如人工确认旧节点永久失效、从幸存节点本地状态强制创建新单节点集群。这类流程会引入 split-brain 风险，不能通过普通 admin API 自动执行。

## 7. 自动化建议

### 可以自动化的部分

1. 新 OOD 启动后自动 add-learner/promote voter。
2. node-daemon 根据 BuckyOS 配置生成 klog-service 环境变量。
3. scheduler 生成 klog placement 和 gateway cluster route。
4. 本地/级联 DV 自动验证 gateway route、cluster-state、system_config roundtrip。

### 不应放在 klog-daemon 内部自动化的部分

1. 根据 BuckyOS OOD 列表自行删除 Raft member。
2. 在失去 quorum 时自动把本地节点重建成新集群。
3. 自动绕过 admin API 的 leader/quorum 安全限制。

原因是 klog-daemon 只知道当前 Raft 状态，不应该成为 BuckyOS desired topology 的真相源。尤其当 system_config 已经依赖 klog 时，错误的自动 shrink 可能同时破坏配置源和一致性集群。

### 推荐新增的控制器

如果需要产品级自动收敛，建议新增一个 BuckyOS OOD membership reconciler。它可以是 node-daemon 中的受控任务，也可以是独立 kernel service，但应满足：

1. 只在 OOD 上运行。
2. 有明确的 leader/lock 机制，避免多个控制器并发提交 membership。
3. 读取 BuckyOS desired OOD set 和当前 klog cluster-state。
4. 对新增 OOD 主要做观察和补偿；正常路径仍由新 OOD auto-join。
5. 对删除 OOD 执行 planned remove runbook，并拒绝无 quorum 或高 split-brain 风险的操作。
6. 所有 destructive 操作必须可审计，并保留人工确认或策略开关。

## 8. 运维检查

### cluster-state

通过本机 node gateway 检查目标 OOD 的 klog admin plane：

```bash
curl "http://127.0.0.1:3180/.cluster/klog/<ood_name>/admin/cluster-state"
```

应重点检查：

1. `current_leader` 是否存在。
2. `voters` 是否符合预期。
3. `learners` 是否存在长时间未 promote 的节点。
4. `nodes` 中每个 node 是否有 `node_name`，否则 gateway transport 无法稳定构造 endpoint。

### system_config

启用 klog backend 后，应通过 `/kapi/system_config` 验证：

1. `get/create/set/delete/list/exec_tx` 能完成 roundtrip。
2. 多 OOD 上的 system_config 读到同一份数据。
3. 只有 seed OOD 执行过 sled bootstrap。
4. scheduler 能继续读取 system_config 并生成 node_config、service_info、gateway_config。

### gateway route

检查 `node_gateway_info.json#cluster_route_map`：

1. 存在 `klog-service` entry。
2. `route_prefix` 默认是 `/.cluster/klog` 或与 klog env/settings 一致。
3. 每个 OOD node 都包含 `raft`、`inter`、`admin` 端口。
4. node gateway 进程已加载最新配置。

## 9. 当前测试覆盖

本地 DV 覆盖：

```bash
uv run test/run.py -p klog_gateway_smoke
uv run test/run.py -p klog_cluster_dv_smoke
uv run test/run.py -p klog_membership_dv
uv run test/run.py -p klog_restart_recovery_dv
uv run test/run.py -p klog_system_config_kv_dv
uv run test/run.py -p klog_system_config_service_dv
uv run test/run.py -p klog_system_config_pagination_dv
uv run test/run.py -p klog_system_config_rollout_dv
uv run test/run.py -p klog_ood_membership_dv
uv run test/run.py -p klog_ood_snapshot_membership_dv
uv run test/run.py -p klog_ood_leader_failover_shrink_dv
uv run test/run.py -p klog_ood_seed_unavailable_join_dv
uv run test/run.py -p klog_ood_single_to_two_dv
uv run test/run.py -p klog_ood_two_voter_loss_dv
```

仍需补齐：

1. 真实多 OOD/cascade DV，而不是本地多进程 harness。
2. BuckyOS 级新增 OOD 全链路：Zone 配置更新、scheduler、node-daemon、新 OOD auto-join、system_config roundtrip。
3. BuckyOS 级 planned remove 全链路。
4. transfer-leader 或等价安全删除 leader 的流程。
5. `2 voters -> 1 online` 的明确灾备 runbook。

## 10. 待讨论问题

1. klog backend 何时从环境变量 opt-in 切到产品级配置入口。
2. 是否在 Zone/部署配置里增加显式 `bootstrap_ood`，替代“第一个 OOD”这个隐式规则。
3. OOD membership reconciler 放在 node-daemon、scheduler 旁路任务，还是独立 kernel service。
4. 删除 OOD 是否必须人工确认，哪些拓扑允许自动 shrink。
5. 是否需要 transfer-leader admin API。
6. admin plane 的 BuckyOS token/RBAC 校验放在 gateway 还是 klog admin handler 前置层。
7. 生产级 MVCC watch 生命周期、client resume 规则、backpressure 和 auto compaction 默认策略。
8. `2 voters` 家庭小集群是否默认推荐 `1 voter + 1 learner`，以及 UI/产品上如何表达可用性边界。
