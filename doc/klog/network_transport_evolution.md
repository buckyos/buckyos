# klog cluster transport 方案与当前状态

本文说明 `klog-service` / `klog_daemon` 在 BuckyOS 中的集群内部传输模型。当前实现已经支持三种 `cluster_network.mode`：

BuckyOS 视角的完整集成、system_config rollout 和 OOD 运维模型见 `doc/klog/buckyos_integration_and_ood_operations.md`。

- `direct`：peer 之间直接按 `addr:port` 访问。
- `gateway_proxy`：集群内部流量经本机 node gateway 转发到目标节点。
- `hybrid`：先尝试 direct，连接或超时失败后回退到 gateway proxy。

## 1. 身份边界

`klog` 里必须区分两个身份：

- `node_id`：`u64`，OpenRaft membership 使用的内部一致性身份。
- `node_name`：BuckyOS 节点名，gateway 路由和业务日志来源使用的外部身份。

正式 BuckyOS managed 模式下，`KLOG_NODE_ID` 由 `node-daemon` 从本机 `DeviceConfig.id` 派生并注入，避免 OOD 列表顺序或设备名变化影响 Raft 成员身份。standalone / direct 调试仍可以手工配置 `node_id`，但必须保证它在集群生命周期内稳定且唯一。

启用 `gateway_proxy` 或 `hybrid` 时，`network.advertise_node_name` 是必填项。BuckyOS runtime 下它还必须等于当前节点的 runtime node name，否则启动会失败。

## 2. 流量分类

`klog` 有四类 HTTP 入口：

| 流量 | 路径 | 端口/服务 | 说明 |
| --- | --- | --- | --- |
| client RPC | `/kapi/klog-service` | `4080` behind gateway | BuckyOS 对外服务访问入口 |
| Raft control | `/klog/append-entries`, `/klog/vote`, `/klog/install-snapshot` | `21001` | OpenRaft 复制、投票、snapshot |
| inter-node data | `/klog/data/*` | `21002` | follower/learner 到 leader 的 data/meta 转发 |
| admin | `/klog/admin/*` | `21003` | add/remove learner、change-membership、cluster-state |

`/kapi/klog-service` 始终属于 BuckyOS service 访问面；`cluster_network.mode` 只影响 Raft、inter-node data 和 admin 这三类集群内部流量。

## 3. gateway proxy 路由

当前 gateway proxy URL 形态固定为：

```text
http://{gateway_addr}{gateway_route_prefix}/{node_name}/{plane}/{suffix}
```

其中：

- `gateway_addr` 默认是 `127.0.0.1:3180`。
- `gateway_route_prefix` 默认是 `/.cluster/klog`。
- `plane` 取值为 `raft`、`inter`、`admin`。
- `suffix` 是去掉原始 `/klog/...` 前缀后的路径，例如 `vote`、`append`、`cluster-state`。

gateway 加载的 `cluster_route_map` key 是服务名 `klog-service`；`/.cluster/klog` 是该 route entry 内部的 path prefix。二者是不同层级：前者用于从 gateway 配置中取出 route，后者用于匹配请求路径。

示例：

```text
http://127.0.0.1:3180/.cluster/klog/ood1/raft/vote
http://127.0.0.1:3180/.cluster/klog/ood1/inter/query
http://127.0.0.1:3180/.cluster/klog/ood1/admin/cluster-state
```

## 4. 当前限制

- `gateway_proxy` 依赖 scheduler/rootfs 生成可用的 cluster route map，否则 gateway 无法把 `/.cluster/klog/...` 转到目标节点。
- `hybrid` 只对连接失败或超时做回退，不会对所有 HTTP 业务错误做回退。
- `admin` 面即使经 gateway 转发，也仍应只作为集群内部能力使用，不应暴露成公网业务 API。
- `2 voter` 不是高可用拓扑；gateway transport 只解决路径可达，不改变 Raft quorum 语义。
- `2 voters -> 1 online` 时，剩余单节点没有 quorum；gateway route 正常也不能恢复强读或写入。
