# Task、日志与诊断模块需求

> 状态：Implemented
> 对应 modules：`task`、`log`、`diagnostic`
> 结果 schema version：1

## 1. 目标

提供所有业务模块共用的长任务观察、结构化日志查询和脱敏诊断导出。业务模块只
负责创建和执行领域 Task，不得分别实现私有 task 轮询、重试伪状态、日志格式或诊断压缩包。

## 2. 边界

### 2.1 负责

- 查询 TaskManager 中当前 principal 可见的 Task、结果和直接子任务；
- 通过 TaskManager control protocol 请求取消；
- 把可重试 Task 路由回声明该能力的领域服务，并返回新 Task；
- 查询受权限裁剪的系统日志；
- 使用 KEvent 加速等待，KEvent 不可用或丢事件时回退到 TaskManager snapshot；
- 生成带采集范围、脱敏版本、hash 和 expiry 的诊断 bundle；
- 将 log/diagnostic 导出下载到调用方明确指定的新文件。

### 2.2 不负责

- 不实现业务 Task 的执行、补偿、回滚或自动重试策略；
- 不把 Terminal Task 重新打开，不以复用旧 `task_id` 表示重试；
- 不提供任意 Host 路径读取或任意日志文件下载；
- 不在 CLI 内解析私有业务结果；部分成功由 Task `result` 的逐项结构表达；
- 不把 KEvent 当作状态真相源，TaskManager snapshot 和 durable event 才是真相源。
- 不在 Task/TaskEvent 之外维护另一套系统关键操作审计记录。

## 3. 资源模型

### 3.1 Task

Task ID 是 TaskManager 分配的 URL-safe opaque string。CLI 不解析其格式。Task 的 `input`、
`result`、`retry_of`、`parent_id`、`root_id`、`progress`、`phase` 和 `outcome` 均沿用 TaskMgr 2.0
协议。`task get` 的 `task` 字段是权限裁剪后的完整 snapshot，`children` 是一个独立分页结果。

### 3.2 LogEntry

日志条目为结构化对象：`timestamp`、`level`、`message`、`service`、`file`、可选 `line`。
服务端可以保留 `raw` 字段，但它必须和 `message` 使用相同脱敏器。`service` 必须来自服务端
枚举，`file` 只能在已选 service 的日志目录中按文件名匹配。

### 3.3 DiagnosticBundle

Bundle ID 是服务端分配的 opaque string。诊断 Task 的 terminal result 至少包含：

```json
{
  "schema_version": 1,
  "bundle_id": "diag-opaque",
  "scope": { "services": ["scheduler"], "since": "...", "until": "..." },
  "redaction_version": 1,
  "sha256": "lowercase-hex",
  "size": 1234,
  "created_at": 1787600000000,
  "expires_at": 1787603600000
}
```

Bundle ZIP 内必须包含与 Task result 相同的 manifest 字段。`sha256` 是按 ZIP entry name 和脱敏内容
计算的内容摘要，`size` 是脱敏内容总字节数；下载响应另给出 `artifact_sha256` 校验 ZIP 文件本身。
Bundle 文件是有 expiry 的 disposable artifact；Task result 是创建事实和内容校验元数据的 durable 记录。

## 4. 命令清单

| 命令 | 主要输入 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- | --- |
| `task list` | owner/type/state/since/until/cursor/limit | read | sync | 分页查询可见 Task |
| `task get <task-id>` | children-cursor/children-limit | read | sync | Task snapshot 和直接子任务 |
| `task wait <task-id>` | 全局 `--timeout` | read | stream | 变化 snapshot 的 JSONL 记录 + 最终 envelope |
| `task cancel <task-id>` | recursive/expected-revision | write | sync | 只请求取消，不承诺回滚副作用 |
| `task retry <task-id>` | 全局 idempotency-key、no-wait | write | either | 仅终态失败且领域服务声明可重试 |
| `log query` | service/file/level/keyword/since/until/direction/cursor/limit | read | sync | 服务端 filter + 分页 |
| `log tail` | service/file/level/keyword/from | read | stream | 轮询 cursor，持续 JSONL 输出 |
| `log export` | services/file/level/keyword/since/until/path | read | sync | 导出明确范围到新文件 |
| `diagnostic collect` | services/since/until/no-wait | privileged | task | 创建脱敏 bundle Task |
| `diagnostic export <bundle-id>` | path | privileged | sync | 校验 hash 后写入新文件 |

`owner` 对应 `creator_user_id`，`type` 对应 `schema_id`，`state` 对应 TaskMgr 2.0 `phase`。
Task 的 `since/until` 接受 RFC 3339 或 Unix milliseconds，发送给 TaskManager 时统一为 milliseconds；
log/diagnostic 使用 RFC 3339。
`limit` 的服务端上限是 500；CLI 不通过扩大 limit 绕过分页。

## 5. 输入与输出 schema

### 5.1 通用分页

分页结果统一保留服务字段名，不用总数伪装 snapshot：

```json
{
  "items": [],
  "next_cursor": null
}
```

Cursor 是服务端 opaque string。方向、filter 或 principal 改变后不得复用旧 cursor。

### 5.2 Task wait stream

`task wait` 在 stdout 输出 JSONL。每个 revision 最多输出一次 progress record：

```json
{"schema_version":1,"type":"task-progress","task_id":"t-...","revision":3,"phase":"Running","progress":{"completed":1,"total":2},"message":"..."}
```

Task 进入 Terminal 后，最后一行是标准 success envelope，其 `data` 是最终 Task snapshot，包含逐项
`result`。Terminal/Failed 和 Terminal/Canceled 仍然是成功完成“等待”命令；调用方根据
`data.outcome` 判断业务结果。只有 RPC、权限、本地超时或用户中断产生 error envelope。

`--timeout` 只结束本地 reader/poll loop，不调用 `task cancel`。建立 KEvent reader 后必须立即读
一次 Task snapshot，收到事件后再次读取 snapshot；读事件 payload 不能替代 snapshot。

### 5.3 Retry

CLI 先读取原 Task，再调用 Control Panel `task.retry`。服务端必须同时验证：

- 原 Task 为 `Terminal/Failed`；
- schema 已注册 retry handler；
- 领域状态仍允许重试；
- idempotency key 未绑定到不同请求。

成功返回 `task_id`、`retry_of` 和可选 `parent_id/root_id`。新 `task_id` 必须不同于原 ID。
当前第一批 retry handler 是 Control Panel 拥有的 `app.install/v1`、`app.uninstall/v1`、
`app.start/v1`、`app.update/v1`、`app.update_batch/v1`；具体 handler 仍可按领域状态拒绝。

### 5.4 Log filter

`log query/tail/export` 共享字段：

```json
{
  "services": ["scheduler"],
  "file": "scheduler.log",
  "level": "error",
  "keyword": "timeout",
  "since": "2026-08-25T00:00:00Z",
  "until": "2026-08-25T01:00:00Z"
}
```

`query` 另有 `direction`、`cursor`、`limit`；`tail` 只允许一个 service，另有 `from=start|end`
和内部 cursor；`export` 要求 service 且至少提供 `since`/`until` 之一，禁止无时间范围导出全部 Zone 日志。

## 6. 权限与安全

- Task read/control 由 TaskManager ACL 计算；不可见 Task 一律表现为 not found；
- diagnostic collect/export 只允许 Admin/Root；下载 token 短期有效且只绑定一个 bundle；
- log service/file 必须先通过服务端 allowlist，不接受 `/`、`..` 或任意 Host path；
- query、tail、log export 和 diagnostic bundle 使用同一个 versioned redactor；
- 至少脱敏 JWT/session/refresh token、password/secret/api key/private key 字段、PEM private key、
  带凭证 URL 和完整数据库 URI；仅保留 `[REDACTED:<kind>]`；
- filter 在原始内容上匹配，返回或写入 artifact 前脱敏，避免 secret 通过 keyword 反射；
- 导出目标必须是显式 `--path`，CLI 使用 create-new，不覆盖现有文件；
- stdout、stderr、error details 和 manifest 不得包含下载 token 或凭证。

## 7. 服务映射

| Facade | Service / protocol |
| --- | --- |
| task list/get/wait/cancel | `/kapi/task-manager`: `list_tasks`、`get_task`、`get_subtasks`、`request_control`; KEvent `/task_mgr/<id>` |
| task retry | `/kapi/control-panel`: `task.retry`; handler 再调用领域 service |
| log query/tail/export | `/kapi/control-panel`: `system.logs.query/tail/download` |
| diagnostic collect/export | `/kapi/control-panel`: `diagnostic.collect/export`；Task 状态仍由 TaskManager 提供 |

TS facade 不直接读写 `system-config`、TaskManager RDB 或 `/opt/buckyos/logs`。

## 8. 持久化和 artifact 生命周期

- Task 和 TaskEvent 使用 TaskManager 平台 RDB instance，作为系统关键操作的唯一事实记录；
- diagnostic ZIP、log export ZIP 和短期下载 token 是 disposable artifact；过期后服务端删除；
- bundle 的创建范围、redaction version、hash、size、created/expiry 写入 Task terminal result；

## 9. 验收标准

- `task list/get/cancel` 各有 parser、schema 和 mock client 单测；
- `task wait` 覆盖 KEvent 唤醒、无 KEvent 轮询、超时不 cancel、Terminal 结果和部分成功结果；
- retry 覆盖非终态、不可重试、idempotent replay、新旧 ID 不同；
- log query/tail/export 共享 filter schema，tail 输出 JSONL；
- redactor 覆盖 token、password、private key、database URI 和第三方 secret；
- diagnostic manifest 与 Task result 的 scope/redaction/content hash/expiry 一致，export 校验 `artifact_sha256`；
- `deno task check`、`deno task test`、`cargo test -p task_manager`、
  `cargo test -p control_panel` 通过；
- `command describe` 能发现全部 10 条命令和固定 schema。

## 10. 当前实现基础与限制

TaskManager 已有 TaskMgr 2.0 snapshot、ACL、控制、durable event 与 KEvent 通知；Control Panel
已有 system logs list/query/tail/download 和 App Installer retry。本阶段组合这些能力并补齐统一
facade、脱敏和 diagnostic bundle。

第一版通用 retry registry 只登记 Control Panel 当前真实支持的 App Task。Workflow、OpenDAN、
AICC 等服务在提供“由旧 Task 构造新 Task”的正式 RPC 前，`task retry` 对其返回
`TASK_NOT_RETRYABLE`，CLI 不复制它们的 Input 猜测执行方式。
