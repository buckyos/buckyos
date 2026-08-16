# Message 模块需求

> 状态：Draft  
> 对应 module：`message`

## 1. 目标与边界

通过 MessageHub 发送和查询消息、会话、投递状态与回执。外部平台 tunnel 是 transport
adapter，不能改变 MsgObject、DID 和 NamedObject 的身份语义。

## 2. 资源模型

- `from`：显式用户、Agent 或 Group DID；
- `to`：Zone 用户 DID、Group DID 或明确 external endpoint DID；
- `msg_id`：不可变消息对象 ID；
- `record_id` / delivery id：本地队列与投递状态；
- `session_id`：UI/会话投影，不替代消息对象。

发送给外部联系人时，canonical Contact DID 不足以确定目的地。调用方必须选择具体 endpoint，
CLI 不按最近活跃或 preferred tunnel 自动路由。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `message send` | write | sync/task | 显式 from/to，支持 idempotency key |
| `message get <msg-id>` | read | sync | 获取消息对象或引用 |
| `message box-list` | read | sync | 查询 inbox/outbox/request 等 box |
| `message session-list` | read | sync | 分页列出会话 |
| `message session-get <session-id>` | read | sync | 列出会话消息 |
| `message delivery-get <delivery-id>` | read | sync | 获取投递状态和失败原因 |
| `message retry <delivery-id>` | write | task/either | 对允许重试的投递重新排队 |
| `message receipt-list <msg-id>` | read | sync | 查询已读回执 |
| `message mark-read <msg-id>` | write | sync | 更新当前身份的阅读状态 |

附件和大对象只能使用 [Object 模块](object.md) 定义的 ResourceRef/ObjId，不把大文件直接塞入
跨 Zone 消息 body。

## 4. 权限与验收

- `from` 必须与当前 principal 或其明确委托关系一致。
- 默认输出不能暴露其它用户私有 box。
- `message send` 重试必须复用 idempotency key，避免重复投递。
- 投递失败、MessageHub 接受、对端送达和对端已读必须分别表达。
