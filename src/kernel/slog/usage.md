# Slog 集群部署与 Gateway 使用说明

本文面向 BuckYOS 环境，重点说明：

- `slog_daemon` / `slog_server` 的配置方式
- 在 **所有跨节点通信都经 gateway 转发** 的前提下，哪些端口需要转发
- 哪些端口只需要本机使用
- 常见运行规则与排障要点

## 1. 组件职责

- `slog`（日志库）：业务服务本地写日志文件与 `log_meta.db`
- `slog_daemon`：扫描日志目录，读取并上传到 `slog_server`
- `slog_server`：接收日志（`POST /logs`）并落库（SQLite）

## 2. 端口与转发边界（最重要）

> 结论先行：在 gateway 模式下，**不建议直接暴露 `slog_server` 的 22001**。  
> 跨节点仅暴露 gateway 入口端口（例如 80/443 或 3180），由 gateway 转发到 `127.0.0.1:22001`。

| 组件 | 默认监听 | 是否跨节点直连 | 是否需要 gateway 转发 | 说明 |
|---|---:|---:|---:|---|
| `slog_server` ingest | `127.0.0.1:22001` | 否（推荐） | 是 | 仅本机监听，通过 gateway 暴露路径到别的 node |
| `slog_daemon` | 无入站监听 | 否 | 否 | daemon 仅主动发起 HTTP 上传 |
| NodeGateway | 常见 `:3180` | 视部署 | 是（节点间） | 作为统一入口进行服务转发 |
| ZoneGateway | 常见 `:80/:443` | 是 | 是（对外/跨节点） | 对外统一入口，建议走 HTTPS |

## 3. 推荐部署拓扑（Gateway-Only）

### 3.1 汇聚节点（Collector）

1. 启动 `slog_server`
2. 保持 `SLOG_SERVER_BIND=127.0.0.1:22001`
3. 在本机 gateway 增加路由：`/slog/logs -> http://127.0.0.1:22001/logs`
4. 通过 ZoneGateway/NodeGateway 对外暴露该路由（不是直接暴露 22001）

### 3.2 业务节点（Agent）

1. 启动 `slog_daemon`
2. `SLOG_SERVER_ENDPOINT` 配置为 gateway 地址（例如 `https://<collector-gateway>/slog/logs`）
3. 为每个节点设置唯一 `SLOG_NODE_ID`

## 4. 配置项

## 4.1 slog_daemon

| 环境变量 | 默认值 | 含义 |
|---|---|---|
| `SLOG_NODE_ID` | `node-001` | 上报时携带的节点标识，集群中必须唯一 |
| `SLOG_SERVER_ENDPOINT` | `http://127.0.0.1:22001/logs` | 上传目标 URL（gateway 模式应改为 gateway URL） |
| `SLOG_LOG_DIR` | `${BUCKYOS_ROOT}/logs` | 扫描日志根目录 |
| `SLOG_UPLOAD_TIMEOUT_SECS` | `10` | 上传请求超时时间（秒） |

## 4.2 slog_server

| 环境变量 | 默认值 | 含义 |
|---|---|---|
| `SLOG_SERVER_BIND` | `127.0.0.1:22001` | server 监听地址（推荐保持 loopback） |
| `SLOG_STORAGE_DIR` | `${BUCKYOS_ROOT}/slog_server` | SQLite 存储目录 |

## 5. Gateway 转发示例

> 以下是按 BuckYOS gateway 规则写的示意，实际字段名/DSL 以你当前网关版本配置为准。

### 5.1 Collector 节点 NodeGateway 转发到本机 slog_server

```yaml
servers:
  - id: slog_ingest_local
    type: http
    hook_point:
      - id: main
        prioity: 1
        blocks:
          - id: slog_logs
            block: |
              match REQ.path "^/slog/logs$" || pass
              return "forward 127.0.0.1:22001/logs"
```

### 5.2 ZoneGateway 对外暴露（可选）

```yaml
stacks:
  - id: zone_gateway_https
    protocol: tls
    bind: 0.0.0.0:443
    hook_point:
      - id: main
        prioity: 1
        blocks:
          - id: slog_logs
            block: |
              match REQ.path "^/slog/logs$" || pass
              return "server slog_ingest_local"
```

## 6. 运行规则（当前实现）

- daemon 扫描目录周期：`60s`（`UPDATE_DIR_INTERVAL_SECS`）
- 全局每轮读取上限：`100` 条（`READ_RECORD_BATCH_SIZE`）
- 单服务每轮配额：`10` 条（`READ_RECORD_PER_SERVICE_QUOTA`）
- 空闲轮询间隔：`1000ms`（`READ_RECORD_INTERVAL_MILLIS`）
- 上传失败退避：`2s -> 4s -> ... -> 120s`（指数退避）
- 默认排除上传服务：`slog_daemon`、`slog_server`（避免自日志回环）
- `Ctrl-C/SIGINT`：daemon 进入优雅退出（停止读新数据、尽量 drain 上传通道）

## 7. 生产建议

1. `slog_server` 只监听 loopback（`127.0.0.1:22001`），由 gateway 统一暴露。
2. 通过 gateway ACL 限制来源节点，避免任意节点写入日志。
3. `SLOG_NODE_ID` 纳入节点配置中心，禁止重复。
4. 对 gateway 路由加健康检查（collector 不可用时告警）。
5. 需要更低延迟时，再评估调小 daemon 轮询与扫描常量（当前多项仍为编译期常量）。

## 8. 快速检查清单

- [ ] 汇聚节点上 `POST /logs` 可经 gateway 访问
- [ ] 非汇聚节点 daemon 的 `SLOG_SERVER_ENDPOINT` 指向 gateway URL
- [ ] 所有节点 `SLOG_NODE_ID` 唯一
- [ ] 没有对外放通 `22001` 直连（仅 gateway 入口对外）
- [ ] daemon/server 重启后能继续上报（已由 e2e 用例覆盖）
