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
rpc_listen_addr = "127.0.0.1:21101"

advertise_addr = "node-a.example.internal"
advertise_port = 21001
advertise_inter_port = 21002
advertise_admin_port = 21003
rpc_advertise_port = 21101
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

## 6. 常见误配

1. 只改了 `listen_*`，没改 `advertise_*`
- 结果：本机能起，集群互联失败，选举/复制报连接错误。

2. `join.targets` 配成 `127.0.0.1:*`
- 多机环境下会指向“自己机器”，join 失败。
- 应配置为目标节点的 `advertise_addr:advertise_admin_port`（或等价 gateway 地址）。

3. 只开放 Raft 端口，未开放 inter/admin
- 结果：协议层可能通，但转发写入/成员变更失败。

## 7. 最小运维检查清单

1. 每个节点 `advertise_addr` 是否为其他节点可达地址。
2. `advertise_port/inter/admin` 是否都配置了 gateway 转发。
3. `join.targets` 是否使用了远端节点 admin 可达地址，而不是 localhost。
4. `network.rpc_listen_addr` 是否只本机暴露（默认建议）。
5. gateway 是否对 admin 接口实施了鉴权/访问控制。

## 8. 压测工具（klog_bench）

为了验证吞吐和延迟，`klog_daemon` 新增了本地压测二进制：

- 路径：`kernel/klog_daemon/src/bin/klog_bench.rs`
- 能力：自动拉起本地集群（默认 3 节点），并发发起 append，输出 TPS 和延迟分位数（P50/P95/P99）。

### 8.1 快速开始

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

### 8.2 常用参数

1. `--nodes`：本地拉起节点数（默认 `3`）。
2. `--concurrency`：并发 worker 数（默认 `32`）。
3. `--duration-sec`：正式压测时长秒数（默认 `30`）。
4. `--warmup-sec`：预热时长秒数（默认 `3`）。
5. `--payload-bytes`：日志消息体大小（默认 `256`）。
6. `--write-target`：写入目标策略（`leader` / `round-robin` / `random`）。
7. `--sync-write`：state-store 是否启用同步写（默认 `true`）。
8. `--report-json`：输出 JSON 报告路径（可选）。
9. `--keep-data`：保留临时数据目录（用于问题排查）。

### 8.3 指标含义

- `throughput`：成功请求的平均吞吐（req/s）。
- `success_rate`：成功请求占比。
- `latency(avg/p50/p95/p99/max)`：单请求端到端延迟（ms）。
- `error_code_counts`：失败请求按业务错误码聚合统计。
