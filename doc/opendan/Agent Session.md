# Agent Session

本文描述当前 OpenDAN Runtime 中 `AgentSession` 的职责、持久化模型和运行语义。

设计来源主要是 [NewOpenDANRuntime.md](../../notepads/NewOpenDANRuntime.md)，实现以
[agent_session.rs](../../src/frame/opendan/src/agent_session.rs)、
[session_model.rs](../../src/frame/opendan/src/session_model.rs)、
[agent.rs](../../src/frame/opendan/src/agent.rs) 为准。

事件订阅有单独文档：[Agent Session 的事件订阅](./Agent%20Session的事件订阅.md)。本文只说明
Session 与事件队列的交界，不展开事件订阅模式。

## 1. 定位

`AgentSession` 是 OpenDAN Runtime 的状态管理核心。新的 Runtime 不再在 opendan 里重复实现
LLM 推理循环、tool dispatch、step 记录和 resume 逻辑，而是：

- opendan 负责构造 `LLMContextRequest` 与 `LLMContextDeps`
- 调用 `LLMContext::run()` / `LLMContext::resume()`
- 消化 `LLMContextOutcome`
- 把 session 级状态、输入队列、workspace 绑定、行为指针、快照和订阅持久化

`LLMContext` 是推理 waist；`AgentSession` 是 waist 之上的 L3/L4 调度器和持久化层。

核心不变量：

1. 同一个 session 任意时刻只有一个 worker task 进入 LLM 推理。
2. 已从系统取走但还没被 LLM 消费的输入必须先落到 `SessionMeta.pending_inputs`。
3. msg-center 的 ack 只允许发生在 `pending_inputs` 落盘成功之后。
4. worker 只有在一次 turn 成功后才删除本轮消费的 pending input；失败时保留，供重启或人工重试。

## 2. Session 类型

当前实现有两类 session：

| 类型 | 语义 | 创建方式 |
| --- | --- | --- |
| `Ui` | 与 UI tunnel / peer 对应的长期会话，负责收用户消息、回送 assistant 文本，并可创建或转发到 Work Session | `AIAgent::resolve_ui_session(from)` 生成 `ui-<from>` |
| `Work` | 内部工作会话，绑定 workspace，承载一个具体 objective | UI session 通过 `try_create_worksession` / `create_worksession` 创建 |

Work Session 创建后会写入 `title`、`objective`、`workspace_id`，并在 worker 空闲且没有 pending input 时触发一次
bootstrap turn。这个首轮输入来自 `objective`，不是一条 `PendingInput::Msg`。

TODO:

- Work Session 设计中的 `report.md` 完成报告还没有形成稳定写入路径；当前结果主要依赖 snapshot、
  round history、worklog 和 session 状态。
- Session 的归档 / GC / `SLEEP` 生命周期还没有在 `SessionStatus` 中落地。

## 3. 持久化目录

Session 数据位于：

```text
<agent_root>/sessions/<session_id>/
  .meta/
    session.json
    state.snap
    behavior_<name>.snap
  readme.md
  tools/
  tool_plan.resolved.toml
  round_history/
```

关键文件：

- `.meta/session.json`：`SessionMeta`，是 session 级真相源。
- `.meta/state.snap`：当前栈顶 process 的 `LLMContextSnapshot`。
- `.meta/behavior_<name>.snap`：`switch_mode = "independent"` 时，挂起 process 的独立快照。
- `round_history/`：按 round 追加的审计历史，记录输入、step、outcome、压缩、interrupt 等事件。
- `tools/`：session 级工具声明和素材，不是进入 `PATH` 的执行视图。

禁止在 session 目录里放 `bin/`。进入 `PATH` 的 Session Exec Bin 由运行时渲染到
`<buckyos_root>/tools/<agent_id>/<session_id>/`，见 [Agent RootFS](./Agent%20RootFS.md)。

## 4. `SessionMeta`

当前 `SessionMeta` 字段以 [session_model.rs](../../src/frame/opendan/src/session_model.rs) 为准：

```rust
pub struct SessionMeta {
    pub session_id: String,
    pub kind: SessionKind,
    pub current_behavior: String,
    pub status: SessionStatus,
    pub owner: String,
    pub one_line_status: String,
    pub pending_inputs: Vec<PendingInput>,
    pub peer_did: Option<String>,
    pub peer_tunnel_did: Option<String>,
    pub event_subscriptions: Vec<EventSubscription>,
    pub workspace_id: Option<String>,
    pub pending_task_calls: Vec<PendingTaskCall>,
    pub title: String,
    pub objective: String,
    pub bootstrap_done: bool,
    pub process_entry: String,
    pub process_stack: Vec<ProcessFrame>,
}
```

`PendingInput` 当前有三类：

- `Msg { record_id, from, from_did, from_name, tunnel_did, text }`
- `Event { event_id, data }`
- `Interrupt { mode, id }`

`record_id` / `event_id` / `interrupt id` 组成稳定 dedup key。重复 Msg 与 Interrupt 会被折叠；
重复 Event 会按状态新旧进行 coalesce，终态事件优先保留。

TODO:

- 旧设计里的 `new_msg/history_msg`、`new_event/history_event` 双缓冲没有作为独立字段实现。
  当前实现是 `pending_inputs` 持久队列 + `round_history` / snapshot 累积历史。
- 旧设计里的 MsgTunnle link 模型尚未完全替代正文存储；当前 `PendingInput::Msg` 仍直接保存文本，
  以保证 crash 后可重放。

## 5. 状态机

当前实现的 `SessionStatus`：

| 状态 | 含义 |
| --- | --- |
| `Idle` | worker 空闲，可以消费 pending input |
| `Running` | 正在执行一次 LLMContext run/resume |
| `WaitingInput` | 等下一条用户消息或普通事件 |
| `WaitingTool` | 已产生 `PendingTool`，正在等 task_mgr 任务终态 |
| `Ended` | session 结束；重启时不会 restore |
| `Error` | turn 失败，pending input 保留，等待外部唤醒或人工处理 |

设计文档中的 `PAUSE`、`WAIT`、`WAIT_FOR_MSG`、`WAIT_FOR_EVENT`、`READY`、`SLEEP` 在当前实现里被收敛为
上面的状态集合：

- `WAIT_USER_MSG` sentinel 会映射为 `WaitingInput`。
- PendingTool / 异步 task 会映射为 `WaitingTool`。
- 普通空闲态是 `Idle`，不是显式 `READY`。

TODO:

- 用户手工 `PAUSE` / `RESUME` 以及 parent session 暂停时级联暂停 sub session 尚未落地。
- 精确的 `WAIT_FOR_MSG` / `WAIT_FOR_EVENT` 过滤状态尚未作为状态机字段落地；事件等待语义见事件订阅文档。
- `SLEEP`、归档、复活策略还停留在生命周期设计里。

## 6. 输入投递与 ack

消息进入 Session 的主路径：

```text
msg-center / local caller / event pump
  -> AIAgent::dispatch_inbound
  -> AgentSession::enqueue_pending(input)
  -> flush_meta()
  -> Wakeup worker
  -> msg-center update_record_state(Readed)
```

`enqueue_pending` 的语义：

1. 计算 dedup key。
2. 写入或合并 `meta.pending_inputs`。
3. `flush_meta()` 用 tmp + rename 写 `.meta/session.json`。
4. 落盘成功后发送 `SessionInput::Wakeup`。
5. 返回 `Ok(())` 后，上游才可以 ack。

这保证：

- 落盘前进程崩溃：msg-center 记录仍未 `Readed`，下次启动可重新拉取。
- 落盘后进程崩溃：session 已持久拥有输入，重启后 `restore_active_sessions` 会重放。
- ack 失败：msg-center 可能再次投递，但 session 会按 `record_id` 去重。

## 7. Worker 消费模型

每个 active session 有一个 tokio worker。`SessionInput` 只是唤醒信号，真实载荷始终从
`meta.pending_inputs` 读取。

worker 每轮大致流程：

1. 优先处理 `Cancel`。
2. 克隆 `pending_inputs` 快照，不立即删除。
3. 处理 `Interrupt` barrier；必要时打断 in-flight LLMContext。
4. 把 `Msg` 和普通 `Event` 组成本轮输入。
5. 若有 `pending_task_calls`，优先等待 / 收集 task 完成事件。
6. 调 `run_one_round()`。
7. 成功后 `discard_consumed(keys)` 并 flush。
8. 失败时保留 pending input，状态置为 `Error`，等待下一次外部唤醒。

如果 snapshot 中仍有 pending tool calls，但 meta 中没有对应的 `pending_task_calls`，实现会认为这是
PendingTool persist 与 task dispatch 之间崩溃造成的孤儿挂起态，丢弃 snapshot 并记录 history 事件。

## 8. 构造与恢复 LLMContext

`AgentSession` 在 `build_or_resume` 中完成 `LLMContext` 的构造：

- 加载当前 `BehaviorCfg`
- 组装 `LLMContextDeps`
- 渲染 system messages
- 在有真实人类输入或普通事件输入时，追加一段 `[environment]` 前缀
- 优先加载 `.meta/state.snap`
- 根据 snapshot 状态选择 fresh run 或 resume

当前 resume 规则：

- snapshot 没有 `pending_tool_calls` 且有新用户输入：使用 snapshot 的 `state.accumulated` 作为历史，
  追加新 user message 后创建新的 `LLMContext`。
- snapshot 没有 `pending_tool_calls` 且没有新输入：`ResumeFill::ResumeFromMidRun`。
- snapshot 有 `pending_tool_calls`：正常路径由 `resume_with_tool_results` 使用
  `ResumeFill::ToolResults`；若没有 meta 侧 task 句柄则丢弃孤儿 snapshot。

环境消息当前只包含：

- behavior name
- session id / title
- workspace id
- recent activity
- `unix_ms` 时钟

TODO:

- 设计中的 auto-recall memory、event diff、弱订阅环境变量尚未接入环境消息。
- `HistoryCompressor` trait 作为 waist 可选项存在于设计中；当前主要实现是 opendan 侧对
  `ContextLimitReached` 的 message-level 压缩和 resume。

## 9. Behavior 切换

Behavior 配置来自 `<agent_root>/behaviors/<name>.toml`，由
[behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs) 翻译成 `LLMContextRequest` 和 deps：

- `mode = "behavior"`：装配 `XmlBehaviorParser` + `XmlStepRenderer`
- `mode = "agent"`：不装 parser/renderer，走普通 agent loop
- `tool_whitelist` 控制 `ToolPolicy`
- `tool_plan` 控制 Session Exec Bin 的 tombstone 策略
- `switch_mode` 控制切换语义

当前 `next_behavior` 处理：

- `END`：结束当前 independent process；如果没有 parent process，则结束 session。
- `WAIT_USER_MSG`：持久化最终 snapshot，session 进入 `WaitingInput`。
- 其他 behavior 名称：执行 `switch_behavior`。

`switch_mode` 当前状态：

| 模式 | 当前实现 |
| --- | --- |
| `normal` | 已实现。保留 accumulated history 和 steps，替换 system / policy / model / budget 等 request 字段。 |
| `independent` | 已实现。每个 behavior entry 有独立 snapshot，`process_stack` 负责父子 process 栈。 |
| `fork` | 不通过 `next_behavior` 触发；作为 session 内部原语由工具调用，例如 `try_create_worksession`。配置枚举仍存在，作为 switch mode 时降级到 normal 并 warn。 |

TODO:

- independent process 内发生 normal switch 后，再回到 entry system prompt 的语义仍有待真实用例确认。
- behavior 切换时 tool plan / SessionBinRenderer 目前不会重新计算；等 behavior 间 tool plan 差异成为真实需求后补。

## 10. PendingTool、Interrupt 与长任务

`LLMContextOutcome::PendingTool` 表示 waist 让出控制权，等待外部 task 结束。Session 的处理方式：

1. 持久化包含 `pending_tool_calls` 的 snapshot。
2. 通过 `TaskDispatch` 创建 task_mgr 任务。
3. 写入 `SessionMeta.pending_task_calls`。
4. 订阅 `/task_mgr/<task_id>`。
5. 状态进入 `WaitingTool`。
6. task 终态事件回来后转成 `Observation`。
7. 收齐所有 pending call 后使用 `ResumeFill::ToolResults` 恢复 LLMContext。
8. 成功后清理 `pending_task_calls` 并取消订阅。

`Interrupt` 是 pending 队列里的 barrier：

- `Graceful`：给未完成 tool calls 注入 `Observation::Cancelled`，让 LLMContext 走到终态。
- `Discard`：尝试通过 `LLMContextInterruptHandle` 立即中断推理，并截断持有未完成 `tool_use` 的 assistant turn。

## 11. Workspace 绑定

Session 与 workspace 的绑定以 `SessionMeta.workspace_id` 为真相源。

`WorkspaceRecord.current_session` 只是冲突检测 hint。重启时 `restore_active_sessions` 通过
`AgentSessionBuild::existing_meta` 恢复 `workspace_id`，然后重新建立运行期 session 和 workspace 的关联。

TODO:

- 同一个 local workspace 同时只能有一个 session `Running` 的强约束尚未作为统一锁实现。
  当前有 workspace 记录与 `current_session` hint，但还不是完整调度锁。

## 12. 输出回送

UI Session 在 `Outcome::Done` 或可返回 partial 的 budget outcome 中，会把 assistant text：

1. 发送到本地 `SessionReply::AssistantText`，用于 CLI / 日志。
2. 如果 runtime 有 `msg_center`、session 有 `peer_did`，则用 agent DID 作为 sender 调
   `msg_center.post_send` 回送给 peer。

Work Session 当前不主动通过 msg-center 回送结果，预期由 worklog、round history、snapshot 和未来的
`report.md` 提供结果查看。

## 13. 恢复语义

`AIAgent::run()` 启动时会调用 `restore_active_sessions()`：

- 扫描 `<agent_root>/sessions/*/.meta/session.json`
- 跳过 `status == Ended`
- 使用 `AgentSessionBuild { existing_meta }` 恢复 session
- 重建 worker
- 把 `event_subscriptions` 推回 `SessionEventPump`
- 让 worker 自动消费遗留 `pending_inputs`

恢复范围包括：

- `pending_inputs`
- peer 路由信息
- workspace 绑定
- event subscriptions
- `pending_task_calls`
- `process_entry` / `process_stack`
- independent process snapshot 文件

## 14. 与旧设计的差异

旧版 `Agent Session.md` 以 `generate_input()` / `update_input_used()` 为中心。新 runtime 后，这两个概念已经分散到：

- `pending_inputs`：持久输入队列
- worker drain：选择本轮真实输入
- `build_or_resume`：把输入合成为 `LLMContextRequest.input` 或 resume fill
- `LLMContext`：负责 step loop、tool dispatch、step record 与 snapshot
- `discard_consumed`：turn 成功后删除已消费 pending input
- `round_history` / snapshot：保存可回放历史

因此当前文档不再建议新增独立的 `generate_input()` API。若后续要恢复模板驱动的输入判空，应先确认它属于
Behavior prompt 编译层，还是 Session worker 的队列消费层。

TODO:

- “零 LLM 空转”目前主要靠 `pending_inputs.is_empty()` 时 worker 阻塞、Work Session bootstrap 只执行一次、
  以及 `WAIT_USER_MSG` 停车实现；设计中的“模板替换后全 Null 则跳过推理”尚未作为统一函数实现。
- Todo/PDCA 状态补丁、Session Summary 深度更新、Session GC、MsgTunnle link 投影等仍属于后续模块化工作。
