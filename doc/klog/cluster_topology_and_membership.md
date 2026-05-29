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

但 gateway transport 需要同时写入 `node_name`：

```text
/klog/admin/add-learner?node_id=2&node_name=ood2&addr=127.0.0.1&port=21001
```

`node_name` 用于 gateway 精确路由，不替代 OpenRaft 的 `node_id`。

## 3. 常见流程

### 新增 voter

1. 新节点以 `target_role = "voter"` 启动。
2. `join.targets` 指向现有节点 admin 入口。
3. 现有 leader 提交 membership 变更。
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

这些测试验证 transport 与 quorum 语义分离：gateway 可以提供可达路径，但不能让两节点集群获得三节点 quorum 能力。
