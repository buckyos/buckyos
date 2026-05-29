# klog 正式部署配置

本文给出当前代码可落地的 `klog-service` / `klog_daemon` 部署配置。配置事实以 `src/kernel/klog_daemon/src/constants.rs` 和 `src/kernel/klog_daemon/src/config.rs` 为准。

## 1. 默认端口

| 端口 | 配置项 | 作用 |
| --- | --- | --- |
| `4080` | `network.rpc_listen_addr` / `rpc_advertise_port` | BuckyOS service RPC，gateway 暴露为 `/kapi/klog-service` |
| `21001` | `network.listen_addr` / `advertise_port` | Raft control |
| `21002` | `network.inter_node_listen_addr` / `advertise_inter_port` | inter-node data/meta |
| `21003` | `network.admin_listen_addr` / `advertise_admin_port` | admin / membership |
| `3180` | `cluster_network.gateway_addr` | 本机 node gateway |

## 2. 单节点配置

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

## 3. 三节点 direct 配置要点

三节点 HA 推荐 `3 voter`。每个节点必须有唯一 `node_id`，并使用同一个 `cluster.name` / `cluster.id`。

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

## 4. gateway_proxy / hybrid 配置要点

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

- scheduler/rootfs 必须生成匹配的 cluster route map。
- gateway 必须能识别 `/.cluster/klog/{node_name}/{raft|inter|admin}/...`。
- BuckyOS runtime 中 `advertise_node_name` 必须等于当前 runtime node name。

## 5. admin 暴露策略

`admin.local_only` 在单节点或本机调试中可以为 `true`。多节点 direct 模式通常需要设为 `false`，否则远端节点无法调用 add-learner / cluster-state。

gateway proxy 场景下，admin 请求经本机 gateway 转发进入 daemon，仍要把它视为集群内部能力，不应暴露到公网业务入口。
