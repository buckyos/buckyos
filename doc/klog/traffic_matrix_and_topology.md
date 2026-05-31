# klog 流量矩阵与拓扑边界

本文梳理 `klog-service` 集成到 BuckyOS 后的访问路径、集群内部路径和推荐拓扑。

## 1. 结论

- BuckyOS 业务侧访问 `klog-service` 应走 `/kapi/klog-service`。
- `klog-service` client RPC 默认端口是 `4080`。
- Raft/inter/admin 三个集群内部面默认端口是 `21001`、`21002`、`21003`。
- 集群内部传输可以使用 `direct`、`gateway_proxy` 或 `hybrid`。
- gateway transport 解决节点寻址和转发路径，不改变 Raft 多数派要求。

## 2. 流量矩阵

| 流量类型 | 发起方 | 接收方 | 路径 | 默认端口 | 是否受 `cluster_network.mode` 影响 |
| --- | --- | --- | --- | --- | --- |
| 客户端 append/query/meta | app / service / SDK | `klog-service` | `/kapi/klog-service` | `4080` | 否 |
| Raft append/vote/snapshot | raft peer | raft peer | `/klog/*` | `21001` | 是 |
| follower/learner data 转发 | non-leader | leader | `/klog/data/*` | `21002` | 是 |
| cluster admin | 管理节点 / 新节点 | admin peer | `/klog/admin/*` | `21003` | 是 |
| 本机 cluster-state smoke | DV / 运维脚本 | node gateway | `/.cluster/klog/{node_name}/admin/cluster-state` | `3180` | 依赖 gateway route |

## 3. 推荐拓扑

### 单节点

适合开发、DV smoke 和最小部署验证。

```text
client / local service
        |
        v
node gateway :3180
        |
        v
klog-service :4080
        |
        v
single raft voter
```

单节点没有 peer replication，`cluster_network.mode` 对实际集群复制影响很小。

### 两节点

两节点只推荐作为非 HA 过渡拓扑：

- `1 voter + 1 learner`：有副本，但 voter 故障后不可写。
- `2 voter`：技术上可运行，但任一节点故障都会丢失 quorum，不应宣传为 HA。

`2 voter` 中如果一个 OOD 掉线，剩余单节点没有 quorum，不能继续强读或写入，也不能在线提交 membership shrink。恢复路径是掉线 OOD 回来后重新形成 quorum；如果要把单节点强制作为新集群继续运行，应按灾备流程处理，不能伪装成普通成员变更。

`1 voter -> 2 voters` 扩容时，新 OOD 应先以 learner 加入并完成日志或 snapshot 同步，再 promote 为 voter。对于长期不稳定的第二 OOD，`1 voter + 1 learner` 比 `2 voter` 更符合可用性预期。

### 三节点

正式 HA 的基础拓扑是 `3 voter`。

```text
ood1 voter  <---- cluster transport ---->  ood2 voter
     ^                                      ^
     |                                      |
     +---------- cluster transport ---------+
                    ood3 voter
```

如果节点之间有稳定内网或 overlay，优先使用 `direct`。如果节点不能稳定直连，但 BuckyOS node gateway 与 cluster route map 可用，可以使用 `gateway_proxy` 或 `hybrid`。

## 4. gateway 的职责边界

gateway 负责两件事：

- 对外服务访问：`/kapi/klog-service`。
- 集群内部 proxy：`/.cluster/klog/{node_name}/{raft|inter|admin}/...`。

gateway 不负责改变以下语义：

- Raft quorum 规则。
- 数据提交顺序。
- learner 是否能自动晋升为 voter。
- admin API 的授权和暴露边界。
- 当前 leader 不能被直接从 voters 中移除；admin API 会拒绝这种 change-membership，避免提交后剩余 voter 长时间无 leader。
