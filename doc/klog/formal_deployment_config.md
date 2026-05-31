# klog 正式部署配置

本文给出当前代码可落地的 `klog-service` / `klog_daemon` 部署配置。配置事实以 `src/kernel/klog_daemon/src/constants.rs` 和 `src/kernel/klog_daemon/src/config.rs` 为准。

BuckyOS 视角的完整集成、system_config rollout 和 OOD 运维模型见 `doc/klog/buckyos_integration_and_ood_operations.md`。

## 1. 默认端口

| 端口 | 配置项 | 作用 |
| --- | --- | --- |
| `4080` | `network.rpc_listen_addr` / `rpc_advertise_port` | BuckyOS service RPC，gateway 暴露为 `/kapi/klog-service` |
| `21001` | `network.listen_addr` / `advertise_port` | Raft control |
| `21002` | `network.inter_node_listen_addr` / `advertise_inter_port` | inter-node data/meta |
| `21003` | `network.admin_listen_addr` / `advertise_admin_port` | admin / membership |
| `3180` | `cluster_network.gateway_addr` | 本机 node gateway |

## 2. BuckyOS managed 部署入口

正式 BuckyOS 集成下，`klog-service` 不建议由人工维护完整 TOML。入口是：

1. `services/klog-service/settings.deployment.mode = "ood_voters"`。
2. scheduler 从 `boot/config.oods` 推导 OOD voter 集合，并把 `klog-service` 调度到这些 OOD。
3. scheduler/rootfs 在 `node_gateway_info.json#cluster_route_map` 下写入 key 为 `klog-service` 的 cluster route；该 route 的默认 `route_prefix` 是 `/.cluster/klog`。
4. node-daemon 启动 `klog-service` 时注入 `KLOG_NODE_ID`、`KLOG_ADVERTISE_NODE_NAME`、`KLOG_CLUSTER_NETWORK_MODE=gateway_proxy`、`KLOG_JOIN_TARGETS` 等环境变量。

正式 BuckyOS managed 模式下，`KLOG_NODE_ID` 按本机 `DeviceConfig.id` 派生，是 Raft 成员身份协议的一部分。不要在部署配置里手写或按 OOD 顺序重新分配 `node_id`。下面的 TOML 示例主要用于 standalone、direct 调试或 DV。

## 3. 单节点 standalone 配置

```toml
node_id = 1

[network]
listen_addr = "127.0.0.1:21001"
inter_node_listen_addr = "127.0.0.1:21002"
admin_listen_addr = "127.0.0.1:21003"
rpc_listen_addr = "127.0.0.1:4080"
advertise_addr = "127.0.0.1"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 4080

[storage]
data_dir = "/opt/buckyos/data/klog-service"

[cluster]
name = "klog"
id = "klog"
auto_bootstrap = true

[cluster_network]
mode = "direct"
gateway_addr = "127.0.0.1:3180"
gateway_route_prefix = "/.cluster/klog"

[join]
targets = []
blocking = false
target_role = "voter"
```

## 4. 三节点 direct 配置要点

三节点 HA 推荐 `3 voter`。每个节点必须有唯一 `node_id`，并使用同一个 `cluster.name` / `cluster.id`。

direct 模式适合 standalone 或受控内网调试。BuckyOS 正式部署优先使用 `gateway_proxy`，由 node gateway 提供集群内部 route。

node1 首次引导：

```toml
node_id = 1

[network]
listen_addr = "10.90.0.11:21001"
inter_node_listen_addr = "10.90.0.11:21002"
admin_listen_addr = "10.90.0.11:21003"
rpc_listen_addr = "127.0.0.1:4080"
advertise_addr = "10.90.0.11"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 4080

[cluster]
name = "prod-klog"
id = "prod-klog-v1"
auto_bootstrap = true

[cluster_network]
mode = "direct"

[join]
targets = []
target_role = "voter"
blocking = false
```

node2 / node3 加入：

```toml
[cluster]
name = "prod-klog"
id = "prod-klog-v1"
auto_bootstrap = false

[join]
targets = ["10.90.0.11:21003"]
target_role = "voter"
blocking = false
```

## 5. gateway_proxy / hybrid 配置要点

启用 gateway 传输时必须配置 BuckyOS 节点名：

```toml
[network]
advertise_node_name = "ood1"

[cluster_network]
mode = "gateway_proxy"
gateway_addr = "127.0.0.1:3180"
gateway_route_prefix = "/.cluster/klog"
```

`hybrid` 只需要把 mode 改成：

```toml
[cluster_network]
mode = "hybrid"
```

要求：

- scheduler/rootfs 必须生成匹配的 cluster route map。map key 是 `klog-service`，route path 前缀默认是 `/.cluster/klog`，二者不要混淆。
- gateway 必须能识别 `/.cluster/klog/{node_name}/{raft|inter|admin}/...`。
- BuckyOS runtime 中 `advertise_node_name` 必须等于当前 runtime node name。

## 6. admin 暴露策略

`admin.local_only` 默认应保持 `true`。BuckyOS 正式部署下，OOD voter 之间的 admin 调用通过 node gateway 的 cluster route 进入本机 loopback admin plane，不应让 klog daemon 直接监听公网或跨机地址。

多节点 direct 调试时，如果没有 gateway route，可能需要临时设置 `admin.local_only=false`，否则远端节点无法调用 add-learner / cluster-state。这个模式只适合受控网络和诊断，不是 BuckyOS 正式暴露策略。

gateway proxy 场景下，admin 请求经本机 gateway 转发进入 daemon，仍要把它视为集群内部能力，不应暴露到公网业务入口。
