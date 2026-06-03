# klog_daemon Gateway Deployment Notes

本文说明在 BuckyOS 场景下，通过 gateway 转发访问 `klog_daemon` 时，监听地址与端口应如何规划。

## 1. 核心结论

如果你的部署模型是:

- 每台机器上都有本机 gateway；
- 所有外部流量都先到 gateway，再由 gateway 转发到本机 `klog_daemon`；

那么 `klog_daemon` 的监听地址可以全部使用 `127.0.0.1`（或 `localhost`）。

但要注意：

- 集群节点之间是否可互通，取决于 `advertise_*` 配置和 gateway 路由；
- `listen_*` 只是本机绑定地址，`advertise_*` 才是告诉其他节点“怎么找到我”。

## 2. 四类端口职责

`klog_daemon` 当前有四类服务入口：

1. `network.listen_addr`（Raft 控制面）
- 用于 `vote / append-entries / install-snapshot` 等 Raft 协议 RPC。
- 必须支持跨节点访问（通过 gateway 转发）。

2. `network.inter_node_listen_addr`（节点间业务转发）
- 用于 data/meta 相关的节点间转发请求（例如非 leader 写入转发）。
- 必须支持跨节点访问（通过 gateway 转发）。

3. `network.admin_listen_addr`（集群管理面）
- 用于 `add-learner / remove-learner / change-membership / cluster-state`。
- auto-join 流程依赖该端口访问其他节点。
- 通常应只在“集群内网/gateway 内部”可达，不应公网裸暴露。

4. `network.rpc_listen_addr`（本机客户端 RPC）
- 给本机业务服务（如 kmsg 等）调用。
- 默认建议仅本机使用，不需要跨节点开放。

## 3. 哪些端口需要 gateway 转发

在“多节点 Raft 集群”场景下，至少需要 gateway 做这三类跨节点转发：

1. `advertise_port` -> 本机 `network.listen_addr`
2. `advertise_inter_port` -> 本机 `network.inter_node_listen_addr`
3. `advertise_admin_port` -> 本机 `network.admin_listen_addr`

`rpc` 端口通常不需要跨节点转发，只给本机使用（`network.rpc_listen_addr`）。

## 4. 推荐配置模式（gateway 托管）

示例（单节点配置片段）：

```toml
[network]
listen_addr = "127.0.0.1:21001"
inter_node_listen_addr = "127.0.0.1:21002"
admin_listen_addr = "127.0.0.1:21003"
rpc_listen_addr = "127.0.0.1:4080"

advertise_addr = "node-a.example.internal"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 4080
```

解释：

- `listen_*`：本机 loopback 即可；
- `advertise_*`：写 gateway 对外可达的地址和端口；
- 其他节点会用 `advertise_addr + advertise_*` 访问你。

## 5. admin_local_only 与 gateway 的关系

`admin_local_only = true` 会在 server 侧检查来源地址是否 loopback。

在“本机 gateway -> 本机 daemon”模型下，daemon 看到的来源通常是 `127.0.0.1`，因此请求会被允许。

这意味着：

- `admin_local_only=true` 的真实效果更接近“只允许本机进程（包括本机 gateway）访问”；
- 外部是否能调用 admin，取决于 gateway 的鉴权和路由策略；
- 建议在 gateway 层对 admin 路径加严格 ACL/鉴权。

## 6. Authority 分层

klog 的 authority 设计原则是：BuckyOS/gateway 负责身份、权限和路由暴露策略，klog 只负责最小安全边界和一致性语义校验。

klog 层当前负责：

- admin/raft/inter/rpc 分端口，避免不同能力混在同一个入口；
- 默认监听 `127.0.0.1`，不主动暴露到公网或跨机网络；
- `admin_local_only=true` 时拒绝非 loopback 来源访问 admin API；
- 校验 cluster identity、membership 变更、leader-only 写入和配置变更冲突等一致性规则。

BuckyOS/gateway 层负责：

- 决定 `/.cluster/klog/...` 是否可从某个节点或某类入口访问；
- 对 admin plane 做 ACL、RBAC、session/node/service token 等策略控制；
- 基于 DID/RTCP/tunnel 建立节点身份边界；
- 避免把 klog admin plane 暴露到公网业务入口。

因此，在当前实现下，除未来可能增加的 BuckyOS 内部 token 防御性校验外，klog 层的 authority 边界已经完整：它不实现用户级 RBAC，也不替代 gateway 的访问控制。正式 BuckyOS 部署应保持 klog 监听 localhost，并通过本机 node_gateway 进入；`admin_local_only=false` 只适合 direct 调试或受控内网。

生产部署策略定稿：

- `admin_local_only` 默认保持 `true`，klog daemon 不直接监听公网或跨机地址。
- OOD voter 之间的 admin 调用只允许走 node gateway 的集群内部路由，用于 `add-learner`、`remove-learner`、`change-membership` 和 `cluster-state`。
- ZoneGateway/公网业务入口不暴露 `/klog/admin/*`。
- gateway 是 admin plane 的授权点；现阶段先依赖 gateway 的集群内部路由边界，后续如接入 token/RBAC，应在 gateway 或 klog admin handler 的前置层补充，不改变 klog 的一致性职责边界。

## 7. BuckyOS 下的 klog node id

`KLOG_NODE_ID` 是 OpenRaft 成员 ID，不是 BuckyOS 的用户可见 node name。BuckyOS 集成模式下由 node-daemon 自动生成：

1. 输入：本机 `DeviceConfig.id` 的 DID 字符串。
2. 算法：`FNV-1a 64`，输入前缀固定为 `buckyos:klog-node-id:v1:`。
3. 结果：非 0 的 `u64`，写入 `KLOG_NODE_ID`。

这个算法是 klog voter 身份协议的一部分，一旦集群初始化后不能随意修改。`KLOG_ADVERTISE_NODE_NAME` 仍使用 BuckyOS 的设备名，gateway cluster route 也继续按 node name 转发。

这样做的目的：

- OOD 列表顺序变化不会改变 raft 成员 ID；
- 设备名变化时，只要 Device DID 不变，klog node id 仍稳定；
- 替换 OOD 设备时 Device DID 变化，klog 会把它视为新的 raft 成员，符合 membership 语义。

## 8. System Config klog backend rollout

当前 `system_config` 切换到 klog backend 仍通过环境变量显式启用：

```bash
BUCKYOS_SYSTEM_CONFIG_STORE=klog
```

已有 sled 数据迁移到 klog 时，只允许一个 OOD 临时启用：

```bash
BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED=true
```

推荐 rollout 顺序：

1. 先启动 klog OOD voter 集群，并确认 gateway cluster route 可用。
2. 选择一个 OOD 作为 seed，启动 `system_config` 时设置 `BUCKYOS_SYSTEM_CONFIG_STORE=klog` 和 `BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED=true`。
3. 其它 OOD 只设置 `BUCKYOS_SYSTEM_CONFIG_STORE=klog`，不设置 bootstrap 开关，直接读取 klog 中的 system_config 数据。
4. seed OOD 首次迁移成功后，应移除 `BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED=true`，避免后续误触发。

本地 DV 覆盖入口：

```bash
uv run test/run.py -p klog_system_config_rollout_dv
uv run test/run.py -p klog_system_config_leader_failover_dv
uv run test/run.py -p klog_gateway_abnormal_dv
uv run test/run.py -p klog_system_config_stale_config_rejoin_dv
```

该用例会启动 3 节点 klog 集群、两个隔离的 `system_config` 实例，并验证只有 bootstrap OOD 的 sled 数据会迁移；非 bootstrap OOD 的本地 sled 残留不会进入 klog。

`klog_system_config_leader_failover_dv` 会启动真实 `/kapi/system_config` 服务并使用 klog backend。测试把 `system_config` 指向一个非 leader klog RPC endpoint，kill 当前 klog leader 后验证写入期间的 transient kRPC 错误语义；随后等待新 leader，使用同一 `system_config` endpoint 重试读写，最后重启旧 leader 并确认 klog-backed keys 追平。

`klog_gateway_abnormal_dv` 会启动 3 节点 target-gateway 模式 klog 集群，覆盖目标 gateway 停止、source gateway route map 指向陈旧地址，以及 admin route 返回错误的路径。测试会确认失败写入没有落入 klog，并检查错误里保留 route/status/connect 等诊断上下文。

`klog_system_config_stale_config_rejoin_dv` 会模拟 OOD 被 shrink 后，node-daemon 仍用旧配置重启该 OOD 上的 `klog-service`。测试会启动真实 `system_config` 指向这个 stale local klog endpoint，确认写入失败不会落入 active klog，也不会把被移除 OOD 自动加回 membership；active OOD 的 `system_config` 仍可继续读写。

## 9. MVCC auto compaction

`klog_daemon` 支持首版自动 MVCC metadata compaction，默认关闭。启用后，只有当前 Raft leader 会定期检查本地 `meta_revision` 和 `meta_compacted_revision`，并通过 Raft `CompactMeta` 写命令提交 compact，因此所有 voter/learner 仍通过状态机一致收敛。

当前支持的策略是 `revision_count`：保留最新 `retention_revisions` 个全局 meta revisions。示例：

```toml
[meta_compaction]
enabled = true
policy = "revision_count"
retention_revisions = 100000
check_interval_ms = 300000
min_compact_gap = 10000
```

环境变量等价入口：

```bash
KLOG_META_COMPACTION_ENABLED=true
KLOG_META_COMPACTION_POLICY=revision_count
KLOG_META_COMPACTION_RETENTION_REVISIONS=100000
KLOG_META_COMPACTION_CHECK_INTERVAL_MS=300000
KLOG_META_COMPACTION_MIN_COMPACT_GAP=10000
```

字段含义：

- `enabled`：是否启用自动 compaction，默认 `false`。
- `policy`：当前只支持 `revision_count`。
- `retention_revisions`：保留最新多少个全局 meta revisions。
- `check_interval_ms`：leader 定期检查间隔。
- `min_compact_gap`：目标 compact revision 相比当前 compacted revision 至少前进多少才提交 Raft 写入，避免频繁小步 compact。

启用后，落后于 compacted revision 的 historical query / change-feed resume 会返回 `COMPACTED`，调用方需要按当前状态重新建立 cursor。

MVCC compaction 与 snapshot 安装并发覆盖入口：

```bash
uv run test/run.py -p klog_mvcc_compact_during_snapshot_dv
```

该用例会先写入包含 update/delete/recreate 的 MVCC history，让 3 voter 集群生成 snapshot；随后新增 learner，在观察到 learner 本地 `snapshot.temp` 已收到数据后由 leader 执行显式 `meta-compact`。最终验证 learner 和已有 voter 对 compacted revision、保留的历史读、post-compaction change-feed 以及后续 gateway 写入保持一致。

## 10. OOD membership DV

BuckyOS 多 OOD 场景下，klog OOD voter 的增删本质上对应 OpenRaft membership 变更。当前本地 DV 覆盖入口：

```bash
uv run test/run.py -p klog_ood_membership_dv
```

覆盖场景：

1. `3 voters -> 4 voters -> 3 voters`：新增 OOD 先作为 learner 加入，再 promote 为 voter；删除时先 demote，再 remove learner 并停止节点。
2. `2 voters -> 3 voters -> 2 voters`：覆盖进入和退出推荐最小稳定集群规模的临界路径。
3. `1 voter -> 2 voters -> 1 voter`：覆盖家庭小集群的单 OOD 与双 OOD 切换。
4. 每次 topology 变化后都通过 gateway inter route 执行 log/meta roundtrip，确认读写仍可用。

该用例仍是本地多进程 DV；真实多 OOD/cascade DV 需要在实际 node-daemon/scheduler/ZoneConfig 变更链路中继续验证。

更重的 snapshot + membership 覆盖入口：

```bash
uv run test/run.py -p klog_ood_snapshot_membership_dv
```

该用例会临时调低 raft snapshot 阈值，写入较多 log/meta 数据后再新增 OOD learner，验证新增节点存在本地 `snapshots/snapshot_*` 文件并能通过 gateway 强读到加入前的数据；随后 promote 为 voter、demote/remove 该新增 OOD，并继续验证剩余 voter 的数据一致性。默认写入规模可通过环境变量调整：

```bash
KLOG_OOD_SNAPSHOT_DV_ITEMS=600 \
KLOG_OOD_SNAPSHOT_DV_VALUE_BYTES=1024 \
uv run test/run.py -p klog_ood_snapshot_membership_dv
```

当前 klog 会固定保留最近 3 个 snapshot 文件，避免 `install-snapshot` streaming 过程中旧 snapshot 被 cleanup 删除。这会比只保留最新 snapshot 增加少量磁盘占用；如果生产部署需要更严格的磁盘控制，后续应把 retain count 纳入 klog daemon/storage 配置。

leader 被动掉线后的 3 OOD 缩容覆盖入口：

```bash
uv run test/run.py -p klog_ood_leader_failover_shrink_dv
```

该用例覆盖 `3 voters -> leader 被动停止 -> 剩余 2 voters 重新选主 -> gateway log/meta 读写 -> change-membership 到 2 voters -> 继续读写`。每个阶段都会复查前序 log/meta witness，确认缩容前后的强读一致性和数据保留。

极端小集群覆盖入口：

```bash
uv run test/run.py -p klog_ood_single_to_two_dv
uv run test/run.py -p klog_ood_two_voter_loss_dv
uv run test/run.py -p klog_raft_quorum_loss_recovery_dv
uv run test/run.py -p klog_raft_membership_change_rejoin_dv
uv run test/run.py -p klog_raft_concurrent_membership_dv
uv run test/run.py -p klog_raft_join_retry_idempotency_dv
uv run test/run.py -p klog_raft_snapshot_install_crash_dv
```

`klog_ood_single_to_two_dv` 覆盖 `1 voter -> add learner -> promote to 2 voters`，验证加入前数据能同步到 learner，promote 后两个 voter 继续强读写。`klog_ood_two_voter_loss_dv` 覆盖 `2 voters -> 当前 leader 被动停止`，验证剩余单 voter 不能选主，也不能继续处理强读或写入；这是预期的 quorum 安全边界。

`klog_raft_quorum_loss_recovery_dv` 覆盖 `3 voters -> 停 2 个 follower -> 单 survivor 无 quorum`，验证单 survivor 的写入和强读都会失败，且无 quorum 期间发起的 meta 写不会在 quorum 恢复后 later apply；随后恢复 1 个节点验证 quorum 恢复后读写成功，再恢复第三个节点并确认追平。当前写服务在本地 leader 创建 Raft proposal 前会检查最近的 quorum ack，新鲜度不足时直接返回 unavailable，避免客户端侧失败的写请求在恢复 quorum 后被提交。

`klog_raft_membership_change_rejoin_dv` 覆盖 `3 voters -> 停止一个非 leader voter -> change-membership shrink 到剩余 2 voters -> 重启被移除节点`，验证旧 voter 重启后不能以旧 membership 影响活跃集群；随后通过 admin add learner 和 promote 重新加入，并确认加入前后的 log/meta witness 全部追平。

`klog_raft_concurrent_membership_dv` 覆盖同一 leader 上并发执行两个 add-learner admin 请求，验证 membership mutation in-flight 时第二个请求明确返回 `409 Conflict`，最终只有一个 learner 进入 membership，promote 后集群继续通过 gateway 完成 log/meta 强读写。

`klog_raft_join_retry_idempotency_dv` 覆盖 auto-join 的 add-learner 请求在服务端提交成功但客户端超时的场景；重试后应识别节点已经是 learner，不重复提交 add-learner，也不会在 `target_role=learner` 时错误 promote。用例随后手工 promote 并验证 gateway log/meta 一致性。

`klog_raft_snapshot_install_crash_dv` 覆盖 learner 通过 snapshot 追平过程中被动停止的场景：先让 3 voter 集群生成并保留 snapshot，再 add learner，观察到 learner 本地 `snapshot.temp` 已收到数据后 kill；重启后 learner 应重新接收/安装 snapshot，追平加入前 bulk 数据，并通过 gateway 继续完成 log/meta 写入。

## 10. 常见误配

1. 只改了 `listen_*`，没改 `advertise_*`
- 结果：本机能起，集群互联失败，选举/复制报连接错误。

2. `join.targets` 配成 `127.0.0.1:*`
- 多机环境下会指向“自己机器”，join 失败。
- 应配置为目标节点的 `advertise_addr:advertise_admin_port`（或等价 gateway 地址）。

3. 只开放 Raft 端口，未开放 inter/admin
- 结果：协议层可能通，但转发写入/成员变更失败。

## 11. 最小运维检查清单

1. 每个节点 `advertise_addr` 是否为其他节点可达地址。
2. `advertise_port/inter/admin` 是否都配置了 gateway 转发。
3. `join.targets` 是否使用了远端节点 admin 可达地址，而不是 localhost。
4. `network.rpc_listen_addr` 是否只本机暴露（默认建议）。
5. gateway 是否对 admin 接口实施了鉴权/访问控制。

## 12. 压测工具（klog_bench）

为了验证吞吐和延迟，`klog_daemon` 新增了本地压测二进制：

- 路径：`kernel/klog_daemon/src/bin/klog_bench.rs`
- 能力：自动拉起本地集群（默认 3 节点），并发发起 append，输出 TPS 和延迟分位数（P50/P95/P99）。

### 12.1 快速开始

先构建 daemon 可执行文件：

```bash
cd src
cargo build -p klog_daemon --bin klog_daemon
```

执行 3 节点 30 秒压测：

```bash
cd src
cargo run -p klog_daemon --bin klog_bench -- \
  --nodes 3 \
  --concurrency 64 \
  --duration-sec 30 \
  --warmup-sec 5 \
  --payload-bytes 256 \
  --write-target round-robin \
  --report-json /tmp/klog_bench_report.json
```

### 12.2 常用参数

1. `--nodes`：本地拉起节点数（默认 `3`）。
2. `--concurrency`：并发 worker 数（默认 `32`）。
3. `--duration-sec`：正式压测时长秒数（默认 `30`）。
4. `--warmup-sec`：预热时长秒数（默认 `3`）。
5. `--payload-bytes`：日志消息体大小（默认 `256`）。
6. `--write-target`：写入目标策略（`leader` / `round-robin` / `random`）。
7. `--config`：从 TOML 文件加载压测配置（支持 CLI 覆盖）。
8. `--append-weight/--query-weight/--meta-put-weight/--meta-query-weight`：混合负载权重。
9. `--query-limit/--query-strong-read/--meta-query-strong-read`：读请求参数。
10. `--meta-key-space`：meta 压测随机 key 空间。
11. `--fault-kill-leader-at-sec`：在测量阶段第 N 秒 kill 当前 leader（故障注入）。
12. `--fault-wait-new-leader-timeout-sec`：故障后等待新 leader 超时（秒）。
13. `--sync-write`：state-store 是否启用同步写（默认 `true`）。
14. `--report-json`：输出 JSON 报告路径（可选）。
15. `--keep-data`：保留临时数据目录（用于问题排查）。

### 12.3 配置文件模式

仓库提供示例：

- `src/kernel/klog_daemon/bench.example.toml`

使用方式：

```bash
cd src
cargo run -p klog_daemon --bin klog_bench -- \
  --config kernel/klog_daemon/bench.example.toml \
  --report-json /tmp/klog_bench_report.json
```

### 12.4 指标含义

- `throughput`：成功请求的平均吞吐（req/s）。
- `success_rate`：成功请求占比。
- `latency(avg/p50/p95/p99/max)`：单请求端到端延迟（ms）。
- `error_code_counts`：失败请求按业务错误码聚合统计。
- `operation_stats`：按 `append/query/meta-put/meta-query` 维度拆分统计。
- `correctness`：append 返回 ID 去重统计、各节点最大日志 ID 一致性。
- `fault`：故障注入是否触发、切主耗时、故障后首个成功请求恢复耗时。
