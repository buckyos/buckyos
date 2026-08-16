# Task、审计与诊断模块需求

> 状态：Draft  
> 对应 modules：`task`、`audit`、`log`、`diagnostic`

## 1. 目标与边界

提供所有业务模块共用的长任务观察、操作审计、日志查询和诊断导出。业务模块不得分别实现
私有 task 轮询和日志格式。

## 2. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `task list` | read | sync | 按 owner/type/state/time 分页查询 |
| `task get <task-id>` | read | sync | 获取状态、进度、结果和子任务 |
| `task wait <task-id>` | read | stream | jsonl 进度 + 最终 envelope |
| `task cancel <task-id>` | write | sync | 请求取消，不保证已发生副作用回滚 |
| `task retry <task-id>` | write | sync/task | 仅对服务声明可重试的 Task |
| `audit query` | privileged/read | sync | 按 actor/action/resource/trace 查询 |
| `log query` | read | sync | 服务端 filter + 分页 |
| `log tail` | read | stream | jsonl 持续输出 |
| `log export` | read | task/either | 导出明确范围 |
| `diagnostic collect` | privileged | task | 生成脱敏诊断 bundle |
| `diagnostic export <bundle-id>` | privileged | sync | 下载到显式路径 |

## 3. Task 语义

- 业务命令返回的 `task_id` 必须能由本模块查询。
- `task wait --timeout` 超时只停止等待，不自动 cancel。
- `task retry` 不能复用旧 task id 伪造状态；应返回新 task 和 parent/retry 关系。
- 部分成功必须保留逐项结果，不能只返回 succeeded/failed bool。

## 4. 日志和隐私

- 默认按当前用户权限裁剪日志，不提供任意 Host 文件读取。
- query/tail 使用结构化字段，不要求 Agent 正则解析整行文本。
- token、password、private key、完整数据库 URI 和外部平台 secret 必须脱敏。
- diagnostic bundle 在创建时记录采集范围、脱敏版本、hash 和 expiry。

## 5. 实现基础

TaskManager 已有持久任务、状态、取消和多类业务 Task；Control Panel 已有 system logs list/query/
tail/download。需要统一 TS facade、filter schema、KEvent 加速等待和脱敏诊断 bundle 协议。
