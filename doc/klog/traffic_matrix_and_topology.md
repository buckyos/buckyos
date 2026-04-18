# klog 流量矩阵与推荐部署拓扑

本文梳理 `klog` 当前在 BuckyOS 中的流量类型、依赖路径和推荐部署拓扑，重点回答三个问题：

1. 本地单机验证时，BuckyOS 是如何启动并把 `klog-service` 跑起来的。
2. `gateway` 在这条链路里是否参与，以及参与了哪一层。
3. 正式环境里，如果部分节点位于内网、节点间不能直接互联，当前 `klog` 还缺什么。

## 1. 结论摘要

- `klog-service` 作为 BuckyOS 对外可见的系统服务，当前访问入口是 `/kapi/klog-service`。
- 对客户端访问和普通 BuckyOS 服务调用，`gateway` 是关键路径，不应绕开。
- 对 `klog` 自身的 Raft 复制、投票、snapshot、节点间 data/meta 转发，当前实现仍然是节点直连 HTTP，不走 BuckyOS gateway。
- 因此，当前版本已经完成了“BuckyOS 服务接入”，但还没有完成“Raft 内部传输层的 gateway/tunnel 化”。

## 2. 当前实现中的关键边界

### 2.1 BuckyOS 服务访问边界

`buckyos-api` 中的 `BuckyOSRuntime::get_klog_client()` 会调用 `get_zone_service_url("klog-service")`，最终生成 `/kapi/klog-service` 风格的访问地址。

- 代码入口：`src/kernel/buckyos-api/src/runtime.rs`
- `AppClient` 访问模式：`http://{zone-host}/kapi/{service}`
- `KernelService` / `FrameService` 访问模式：
  - 本机已启动实例时，直连 `127.0.0.1:{port}/kapi/{service}`
  - 否则走本机 `node gateway` 的 `127.0.0.1:3180/kapi/{service}`

也就是说，BuckyOS 语义里的“服务访问”已经默认绑定到了 `/kapi/...` 和 gateway 路径上。

### 2.2 klog 集群内部边界

`klog` 内部还有另一套独立网络面：

- Raft 控制面：`/klog/append-entries`、`/klog/vote`、`/klog/install-snapshot`
- 节点间业务转发：`/klog/data/append`、`/klog/data/query`、`/klog/data/meta-put`、`/klog/data/meta-delete`、`/klog/data/meta-query`
- 集群管理面：`/klog/admin/add-learner`、`/klog/admin/remove-learner`、`/klog/admin/change-membership`、`/klog/admin/cluster-state`

这些请求当前都不是通过 `BuckyOSRuntime` 或 `/kapi/...` 生成的，而是由 `klog` 自己按 `target.addr + target.port` 直接拼接 HTTP 地址发出去。

这意味着：

- `klog-service` 的对外服务入口已经接入 BuckyOS。
- `klog` 的 Raft 内部传输层还没有接入 BuckyOS gateway / rtcp / tunnel 体系。

## 3. 本地单机验证流程

当前本地单机验证已经整理成脚本：

- 启动和验证：`src/test/run_klog_remote_tests.sh`
- 停止和清理：`src/test/cleanup_klog_remote_tests.sh`

默认运行时根目录：

- `.dev_buckyos_klog`

启动流程如下：

1. 调用 `uv run src/start.py --all`，在 `.dev_buckyos_klog` 下生成本地单机 BuckyOS 运行时。
2. 等待核心服务启动：
   - `system_config` `3200`
   - `verify_hub` `3300`
   - `control_panel` `4020`
3. 为 `klog-service` 生成本地 bundle 和配置，并单独启动 `klog_daemon`。
4. 启动一个最小 gateway 容器，监听本机 `80` 和 `3180`，把 `/kapi/...` 代理到本机服务。
5. 运行 `src/kernel/buckyos-api/tests/klog_remote_tests.rs`，验证 `BuckyOSRuntime::get_klog_client()` 的真实链路。

这套验证流程的目标不是模拟完整正式环境，而是验证：

- `runtime -> /kapi/klog-service -> klog-service`

这条服务访问链路已经成立。

## 4. gateway 在本地验证里扮演什么角色

本地单机验证中，gateway 是参与的，但参与的是“服务访问层”，不是“Raft 集群层”。

### 4.1 参与的部分

最小 gateway 容器负责代理：

- `/kapi/system_config`
- `/kapi/verify-hub`
- `/kapi/control-panel`
- `/kapi/klog-service`

因此本地 `klog_remote_tests` 验证的是：

- `AppClient` 通过 zone host 访问 `/kapi/klog-service`
- gateway 将其转发到本机 `klog_daemon` 的 JSON-RPC 入口

### 4.2 未参与的部分

本地验证没有覆盖：

- follower 到 leader 的 `/klog/data/*` 转发
- Raft 的 `/klog/append-entries`、`/klog/vote`、`/klog/install-snapshot`
- auto-join / membership 相关 `/klog/admin/*`

这些都还是 `klog` 自己的点对点 HTTP。

## 5. 完整流量矩阵

| 流量类型 | 发起方 | 接收方 | 当前路径/端口 | 当前是否走 gateway | 正式环境建议 |
| --- | --- | --- | --- | --- | --- |
| 客户端日志写入 / 查询 / meta 读写 | `AppClient`、外部 SDK | `klog-service` | `/kapi/klog-service` | 是 | 必须走 zone gateway / node gateway |
| 本机服务访问本机 `klog-service` | `KernelService` / `FrameService` | 本机 `klog-service` | `127.0.0.1:{rpc_port}/kapi/klog-service` | 可绕过 | 本机短路直连是合理的 |
| 非本机服务访问 `klog-service` | `KernelService` / `FrameService` | 远端 `klog-service` | `127.0.0.1:3180/kapi/klog-service` | 是 | 必须走 node gateway |
| 非 leader 写入后的节点间转发 | follower / learner | leader | `/klog/data/*` + `inter_port` | 否 | 当前要求节点直连；正式环境应走专用可达网络，或后续改成 tunnel 化 |
| Raft 复制 / 选举 / snapshot | Raft peer | Raft peer | `/klog/append-entries`、`/klog/vote`、`/klog/install-snapshot` + `port` | 否 | 当前要求节点直连；不应依赖公共服务 gateway |
| 集群管理 / auto-join | 管理节点 / 新节点 | admin peer | `/klog/admin/*` + `admin_port` | 否 | 应限制在集群内部网络，不能公网裸露 |
| 管理员或本机工具访问 cluster state | 本机运维脚本 / 管理客户端 | 本机 `klog-service` 或 admin 入口 | `/kapi/klog-service` 或 `/klog/admin/*` | `/kapi` 走 gateway；admin 当前不走 | 建议优先经本机 service 能力访问，admin 仅保留内部用途 |

## 6. 推荐部署拓扑

### 6.1 推荐拓扑：客户端面与集群面分离

推荐把 `klog` 拆成两层网络：

1. 客户端面
- 对外暴露 `klog-service`
- 统一通过 `/kapi/klog-service`
- 走 BuckyOS 的 zone gateway / node gateway

2. 集群面
- 用于 Raft peer 间复制、投票、snapshot、成员管理
- 不走公共 `/kapi` 路由
- 运行在节点之间可达的专用网络上

推荐拓扑如下：

```text
client / app service
        |
        v
zone gateway :80/:443
        |
        v
node gateway :3180
        |
        v
klog-service rpc :4070   (client-facing)


raft peer A  <---- dedicated cluster network ---->  raft peer B
   :raft_port                                      :raft_port
   :inter_port                                     :inter_port
   :admin_port                                     :admin_port
```

这里的专用 cluster network 可以是：

- 同一内网 / 同一 VPC
- WireGuard / Tailscale / ZeroTier
- 任何对等节点之间稳定可达的 overlay 网络

### 6.2 为什么不推荐把 Raft 全部塞进公共 gateway

Raft 内部流量和普通业务流量不同：

- 长期存在心跳和复制流量
- 对时延和稳定性比普通业务请求更敏感
- 包含成员变更和 snapshot 等内部控制面

如果把这些流量全部压进公共 gateway：

- 容易与业务访问流量互相挤压
- 让 gateway 成为选举和复制的关键瓶颈
- 会把原本不该暴露的内部接口暴露到更大的攻击面

所以当前更合理的边界是：

- `/kapi/klog-service` 通过 gateway
- `/klog/*` 内部集群接口走专用 cluster network

## 7. 正式环境下 gateway 的必要性

### 7.1 对 `klog-service` 服务访问来说是必须的

如果调用方是：

- 用户态客户端
- 其他 BuckyOS 服务
- 通过 `BuckyOSRuntime::get_klog_client()` 获取 client 的代码

那么 gateway 是必要组成部分，因为这些调用默认都落到 `/kapi/klog-service`。

### 7.2 对 Raft 内部来说当前还不成立

如果你的目标场景是：

- 节点不一定有公网 IP
- 节点可能在 NAT / 内网
- 节点之间没有直接 IP:port 互通

那么当前 `klog` 实现还不满足这个场景。

原因不是 gateway 配几条规则就能解决，而是传输层模型还没改：

- `KNetworkClient` 和 `KDataClient` 仍然按 `target.addr:port` 直接请求
- `advertise_addr` / `advertise_port` 的语义仍然是“给 peer 一个可直接访问的地址”

这意味着只要 peer 之间不能直连，当前 Raft 层就会失败。

## 8. 当前缺口

如果后续要支持“节点无法直连，只能依赖 gateway / tunnel / relay”，必须补以下能力：

1. 为 Raft 内部流量抽象独立的 transport 层
- 不能继续把 peer 目标仅建模成 `addr + port`
- 需要能表达 `rtcp route`、`relay target`、`node gateway route`

2. 改造 `KNetworkClient` 和 `KDataClient`
- 不能继续直接拼 `http://{target.addr}:{port}`
- 需要根据 transport 类型选择直连、gateway、tunnel 或 relay

3. 重新定义 `advertise_*`
- 现在它表达的是“别人如何直接访问我”
- 后续可能要表达“别人通过哪条 tunnel / route 找到我”

4. 明确 admin 面是否 tunnel 化
- `change-membership`、`add-learner`、`cluster-state` 是否也走内部 relay
- 还是保持在专用管理网络中

## 9. 推荐后续路线

### 9.1 短期可落地方案

短期先采用下面的生产拓扑：

- `klog-service rpc` 接入 BuckyOS gateway
- `raft/inter/admin` 只在专用 cluster network 中开放
- 每个 `klog` 节点都拥有可被其他 peer 访问的稳定 overlay 地址

这条路线对现有实现改动最小，能尽快让 `klog` 在 BuckyOS 中进入可用状态。

### 9.2 中期演进方案

如果明确要求：

- 任意节点可在 NAT 后
- 节点之间不能保证直连
- 仍然要组成稳定 Raft 集群

那么应该单独立项改造 `klog` 传输层，把 Raft 内部网络接进 BuckyOS 的 node gateway / rtcp / relay 能力，而不是继续靠裸 HTTP peer 直连。

## 10. 相关代码入口

- `src/kernel/buckyos-api/src/runtime.rs`
  - `get_klog_client`
  - `get_kernel_service_url`
  - `get_zone_service_url`

- `src/kernel/scheduler/src/system_config_builder.rs`
  - kernel service 的 `/kapi/{service}` 暴露规则

- `src/kernel/scheduler/src/scheduler.rs`
  - gateway 路由生成原则

- `src/kernel/klog/src/network/client.rs`
  - Raft peer / inter-node data 的直连 HTTP 实现

- `src/kernel/klog/src/network/request.rs`
  - `/klog/*` 请求路径定义

- `src/rootfs/etc/boot_gateway.yaml`
  - zone gateway / node gateway / RTCP 的基本转发模型
