# klog 集群拓扑与成员变更

本文说明当前 `klog` 推荐拓扑、成员变更方式和测试覆盖。

## 1. 推荐拓扑

| 拓扑 | 推荐程度 | 说明 |
| --- | --- | --- |
| 单节点 | 开发 / DV | 最小可用，不具备副本冗余 |
| `1 voter + 1 learner` | 过渡方案 | 有副本，但不是 HA |
| `2 voter` | 不推荐作为 HA | 任一节点故障都会失去 quorum |
| `3 voter` | 正式 HA 基线 | 满足多数派容错 |
| `3 voter + N learner` | 扩展方案 | learner 可作为副本或后续扩容过渡 |

## 2. 成员身份

成员变更仍以 `node_id` 操作 Raft membership：

- `add-learner?node_id=...`
- `remove-learner?node_id=...`
- `change-membership?voters=1,2,3`

`node_id` 是 OpenRaft 成员 ID，不是 BuckyOS 节点名。正式 BuckyOS managed 模式下，`klog-service` 由 `node-daemon` 启动，`KLOG_NODE_ID` 按本机 `DeviceConfig.id` 派生，避免 OOD 列表顺序变化导致 Raft 成员身份变化。文档里的 `1,2,3` 只用于 standalone / DV / direct 调试示例。

gateway transport 需要同时写入 `node_name`：

```text
/klog/admin/add-learner?node_id=2&node_name=ood2&addr=127.0.0.1&port=21001
```

`node_name` 用于 gateway 精确路由，不替代 OpenRaft 的 `node_id`。

## 3. 常见流程

### 新增 voter

1. 新节点先通过现有 leader 的 admin 入口执行 add-learner。
2. learner 复制日志或 snapshot，并能读取加入前的数据。
3. 如果新节点 `target_role = "voter"`，auto-join 会继续提交 change-membership，把 learner promote 为 voter。
4. 通过 `/klog/admin/cluster-state` 确认 voters 包含新节点。

### 新增 learner

1. 新节点以 `target_role = "learner"` 启动。
2. leader 执行 add-learner。
3. learner 复制日志，但不参与 quorum。
4. 后续如需晋升，使用 change-membership 把 learner 加入 voters。

### 移除 learner

1. 调用 remove-learner。
2. 等待所有 voter 的 cluster-state 中 learners 清空。
3. 停止被移除节点进程并清理部署。

### 缩容 voter

1. 优先移除非 leader voter：先用 change-membership 把它从 voters 中 demote，保留为 learner。
2. 确认剩余 voters 已形成新 membership 后，再 remove-learner 并停止节点。
3. 直接从 voters 中移除当前 leader 会被 admin API 拒绝并返回 `409 Conflict`，避免提交后剩余 voter 长时间没有 leader。
4. 如果当前 leader 被动掉线，且剩余节点仍有 quorum，例如 `3 voters -> 2 online`，应先等待剩余 voters 重新选主，再把 membership shrink 到存活的 2 voters。

### 两节点边界

`2 voter` 不是 HA：任一 voter 掉线后，剩余单节点没有 quorum，不能继续强读或写入，也不能通过正常 change-membership 把掉线节点移除。可恢复路径是让掉线 OOD 回来并重新形成 quorum；强行把单节点重建为新集群属于灾备流程，不属于在线 membership API。

`1 voter -> 2 voters` 是支持的过渡流程，但第二个 OOD 应先作为 learner 加入并完成数据同步，再 promote 为 voter。家庭小集群如果第二个 OOD 长期不稳定，更适合保持 `1 voter + 1 learner`，避免 `2 voter` 任一节点掉线即不可写。

## 4. gateway transport 下的成员变更

`gateway_proxy` / `hybrid` 下，membership 中的目标节点必须带 `node_name`，否则 client 无法构造：

```text
/.cluster/klog/{node_name}/{raft|inter|admin}/...
```

如果缺少 `node_name`，gateway transport 会在构造 endpoint 时直接报错，而不是退化成不明确的服务选择。

## 5. 测试覆盖

当前集成测试覆盖：

- `tests/two_node.rs`：`1 voter + 1 learner`、`2 voter`、两节点 gateway admin roundtrip。
- `tests/gateway_transport.rs`：三节点 `gateway_proxy` 复制与 follower 转发。
- `tests/gateway_transport.rs`：三节点 `hybrid` 在 direct 不可达时回退 gateway。
- `tests/failover.rs` 等既有测试：三节点 voter failover 与恢复语义。
- `test/klog_ood_membership_dv.sh`：`3 voters <-> 4 voters`、`2 voters <-> 3 voters`、`1 voter <-> 2 voters` 的本地 OOD membership roundtrip。
- `test/klog_ood_snapshot_membership_dv.sh`：新增 OOD learner 在大数据量和 snapshot 条件下的数据同步、promote、demote/remove。
- `test/klog_ood_leader_failover_shrink_dv.sh`：`3 voters -> leader 被动停止 -> 剩余 2 voters 重新选主 -> shrink 到 2 voters`。
- `test/klog_ood_single_to_two_dv.sh`：`1 voter -> add learner -> promote to 2 voters`。
- `test/klog_ood_two_voter_loss_dv.sh`：`2 voters -> leader 被动停止 -> 1 survivor` 的 quorum loss 边界。

这些测试验证 transport 与 quorum 语义分离：gateway 可以提供可达路径，但不能让两节点集群获得三节点 quorum 能力。
