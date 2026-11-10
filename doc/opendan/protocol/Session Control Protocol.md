# Session Control Protocol（Session 控制）

版本 3 · 2026-10-03；2026-10-11 增加 `stop.scope`、`goal_skills` 与 Goal Session 的结束；2026-11-10 汇总已实现的 pause / resume、控制作用域与级联规则，校正取消确认及终态拒绝说明。本次整理不改变总线 schema 或命令字段。

本文是 Session 控制语义的统一入口，由 `libopendan` 的 `src/protocol/input.rs::ControlCommand`、`src/runner/{inputs,drive,outcome,goal}.rs` 反写。输入信封与消费规则见 [Session Input Protocol](<Session Input Protocol.md>)，持久字段、提交与恢复见 [Session Directory Protocol](<Session Directory Protocol.md>)，Goal 对象及授权见 [Agent State Protocol](<Agent State Protocol.md>) §9。架构约束见 [Session 架构设计 §12.12](<../OpenDAN Agent Session架构设计.md#1212-宿主装载运行状态与中断>)。

[Agent UI 升级 TODO](../../../notepads/agent-ui-upgrade-todo.md) 中的控制变更尚未实施，不能直接当作本文已有行为；实施时必须同步修改本协议、共享类型、调用方和验证用例。

## 1. 控制对象与边界

| 对象 | 控制边界 |
|---|---|
| Turn | 一次逻辑工作轮次，可跨多个 run 与上下文。停止当前 Turn 不结束 Session，也不等于成功完成任务。 |
| Run | 一个 LLMContext 的可恢复执行。暂停后继续同一个 Turn，不因恢复快照自动获得新 Turn。 |
| Session | 连续上下文与执行状态的容器。进入 `finished` 后不能由新输入或 resume 重新打开。 |
| Goal | 持久目标及其调度状态。Goal 暂停禁止后续 Activation，与暂停 Goal Session 当前的执行不同。 |
| 子 Session / 外部 task | 有自己的执行状态。是否级联与是否支持取消分别判断，父级停止不证明它们已停止。 |

`stop` 结束所指定范围内的执行；`pause` 保留未完成的执行，等待 `resume`。协议没有独立的 Session `cancel` 命令，也没有用 stop / cancel 区分“优雅停止 / 强制杀死”的约定。界面的“取消”、聊天命令别名以及外部 Task 的取消接口必须明确映射到哪个对象与操作。

停止、暂停、Goal 休眠、归档、卸载执行者和撤销产物不是同一件事。停止不删除历史，也不撤销已经发生的文件修改或外部副作用；撤销能力由具体工具和授权决定。

## 2. 投递与提交

控制命令与 Agent 输入走同一条总线（`type = "control"`，信封见 Session Input Protocol §2），但不是 Agent 的输入：由 Runner（lease 持有者）执行，不进入 LLM 上下文，不产生输入 receipt。搭同一条队列是为了多方投递、持久化和与事件保持同一个顺序；控制处理与普通输入触发推理的路径分开。

```json
{ "schema": "opendan.session_input/3", "type": "control", "key": "ctl:stop:1790899260000",
  "from": "did:bns:alice", "at_ms": 1790899260000,
  "payload": { "command": "stop", "reason": "用户取消" } }
```

- 控制命令与事件按投递 index 依次生效（同一源内）。暂停期间仍处理控制命令，普通 msg / event 不触发推理。
- 应用时写 `control_applied`（decide 写 `decide`），与消费标记同一次提交；之后确认输入源。
- 投递成功只表示控制已入队；`control_applied` 表示驱动者已应用控制，不表示外部 task 或全部子工作均已停止。调用方依据 Turn / Session 的提交状态及相关工作的实际结果判断完成。
- 未知 `command` → `unknown_command`；字段不合法 → `invalid_payload`。已结束的 Session 只接受 `decide`，新的 stop / pause / resume 等输入被拒绝（`SessionFinished` / `session_finished`），不恢复执行。
- 控制命令与 msg / event 共用每个 Session 64 条 pending 记录的上限。队列满会拒绝投递，不能把失败的投递显示成已暂停或已停止。

## 3. 控制命令

| `command` | 字段 | 应用规则 |
|---|---|---|
| `stop` | `reason?`、`scope?: session \| turn`（缺省 `session`） | 结束 Session 或当前 Turn；Goal Session 特例与中断规则见 §4、§6。 |
| `pause` | `reason?` | 暂停 Session 推理，保留当前 Turn / run，按 §7 级联。当前没有 `scope` 或 `cascade` 字段。 |
| `resume` | `reason?` | 清除 Session 暂停，从检查点继续或等待输入；按 §7 恢复因父级而暂停的子 Session。 |
| `goal_skills` | `add[]`（`{name, ref?, version?, note?}` 或名字）、`remove[]`（名字） | 仅 Goal Session：改它拥有的 Goal State 的 `current_skills`（`state.goal.state.rev + 1`），随后发布；其它 Session → `invalid_payload` |
| `decide` | `decision: accept \| discard`、`by`、`note?` | finished 之后唯一仍可接受的输入，仅 work session。运行中或未 finished 时留在队列并写 `pending_decision`；accept 需 `acceptance = pending`，discard 需 `pending \| accepted`，否则拒绝（`input_rejected`） |
| `subscribe` | `subscription: {id, mode: active \| semi, source, watch[]}` | 加入 `session_config.subscriptions`（`config_rev + 1`）。`source`：`{type: session, ref}`、`{type: object_event, object, event?}`、`{type: task, task_id}`、`{type: timer, name}`、`{type: system, id}` |
| `unsubscribe` | `id` | 移除订阅、其拉模式游标和 `pending_events` 中尚未注入的状态；之后到达的该订阅事件丢弃 |
| `activity` | `summary?`、`touch[]`、`clear` | 合并进 `state.activity`（touching 上限 16） |
| `perceive` | `kind`（默认 `observation`）、`summary`、`tags[]`、`objects[]` | 驱动者以 lease 追加到 Agent 感知流（按 seq 幂等），再消费输入 |

## 4. 停止

| 请求 | Turn 的结局 | Session 的结局 | 子 Session |
|---|---|---|---|
| `stop(scope=turn)` | 有打开的 Turn 时置 `turn_stop_requested`，以 `stopped` 关闭，之后清除标志；没有打开的 Turn 时记录 `detail.applied = false` | 保留上下文；未暂停时等待下一个有效输入，已暂停时仍保持暂停 | 不级联 |
| `stop(scope=session)`，非 Goal Session | 置 `stop_requested`；打开的 Turn 以 `stopped` 关闭 | `run_state = finished`、`outcome = stopped` | 按 §7 停止未结束的子 Session |
| Goal Session 的任意 `stop` | 按 `scope=turn` 处理，只结束当前 Activation | 保留 Session 与 Goal 状态，等待后续运行机会 | 不因当前 Activation 停止而停止 Goal 派生的 Work |

UI Session 在现有协议中可以收到 Session 范围的 stop 并结束；“UI 不提供结束按钮”与“协议拒绝这种请求”是不同要求。没有打开 Turn 时，Turn 范围 stop 也不禁止未来的新 Turn；它不是持续的暂停开关。

已经接受的最终报告先于后到的 stop 完成；先被宿主观察到的 stop 拒绝新的结束提交。stop 不把已经由报告提交确定的终局改写成 `stopped`；若目标 Turn 已由报告关闭，Turn stop 清除标志后返回。提交先后规则见 Session Directory Protocol §7.1。

运行中的推理在中断边界停止；工具执行期间由监视任务查看队列中的 stop 并发出中断，监视任务不消费输入、不写 Session state，最终由驱动者消费并提交。等待 task 的挂起调用按可获得的状态回填。该行为不是“所有在飞调用自然返回后再停止”，也不保证任意外部工具都能立即终止。

## 5. 暂停与恢复

`pause` 写入 `state.paused = {at_ms, reason, by, in_run}`，暂停的是 Session 的推理推进：

- 不打开新 Turn，不产生 tick；到达的 msg / event 留在队列，所跟 task 的结束留待恢复后处理。
- 运行中的 run 在下一个推理边界，即在飞调用返回并写入检查点后，以中断原因 `session paused` 挂起。run 保留检查点，Turn 保持打开；记录 `worklog outcome.kind = paused`，暂停标记 `in_run = true`。
- 已经交出的外部 task 按其执行器 / OS 进程语义继续；暂停 Session 不等于暂停这些进程。
- 登记表投影为 `status.paused`；暂停中的 Session 不报 `needs_input`。已暂停时再次 pause 不覆盖原暂停标记，记录 `applied = false`。

`resume` 清除 `state.paused`。Turn 内暂停的 run 通过 `state.resume_run` 从检查点继续，与崩溃恢复共用恢复路径；Turn 之间暂停的 Session 由积压输入或下一次有效运行机会打开新 Turn。未暂停的 Session 收到 resume 时记录 `applied = false`。pause / resume 都不重新打开已结束的 Session（§2）。

恢复保留调用身份，不清空累计用量，也不补充当前 Turn 的工具额度；连续执行墙钟仍按 Session Directory Protocol §7 重新计时，不能把恢复误记成新的任务或新的 Turn。等待工具、等待用户、等待调度和用户主动 pause 分别记录，不因 run 快照的 `status = paused` 就认定 Session 收到了 pause。

Session 已暂停时仍可停止当前 Turn 或结束 Session；停止处理先于暂停闸门。stop 不隐式清除 `state.paused`，所以暂停期间的 Turn stop 关闭 Turn 后，后续推理仍需 resume。

## 6. Goal 生命周期

Goal 对象的 `status = paused` 是调度开关：不再打开新的 Activation，已打开的 Turn 跑到自然结束。恢复 Goal 为 `active` 允许后续运行机会，不等于 `control(resume)`，也不清除 Goal Session 自己的 `state.paused`。需要暂停当前推理时使用 Session pause；只结束当前 Activation 时使用 stop。

Goal Session 的终止由 Goal 生命周期驱动，不是另一个 `ControlCommand`。驱动者读取 `state/goals/<id>/goal.json`，在 Goal 归档、达成或进入新 epoch 时写入 `state.goal.ending` 与 `control_applied{command: goal_end}`，再走停止提交路径；打开的 Activation 以 `stopped` 关闭并写回：

- Goal 归档：`ending.cascade = true`，向有队列的子 Session 投递 stop；Goal 的其它 Work 由托管进程按 Goal 已归档处理。
- Goal 达成：Goal Session 以 `succeeded` 结束。
- restart / reopen 切换 epoch：旧 Session 以 `stopped` 结束，不停止 Goal 派生的 Work；新 epoch 按 Agent State Protocol §9 接管。

界面可以把 Goal 暂停称为“休眠”、恢复称为“唤醒”，但这不会改变 stop 的含义。Goal 状态操作及其权限以 Agent State Protocol §9 为准。

## 7. 子工作与外部 Task

| 操作 | 级联规则 |
|---|---|
| Turn 范围 stop | 不停止子 Session。外部 task 的取消请求与结果须单独核对。 |
| 非 Goal Session 以 stopped 结束 | 对未结束且有队列的子 Session 投递 stop；无队列的子由宿主发出驱动者停止请求。普通结束路径只处理需向父汇报的未结束子 Session。 |
| Session pause | 对未结束且有队列的子 Session 投递 pause，reason 为 `parent session <sid> was paused`；当前会跳过无队列子 Session。 |
| Session resume | 只对暂停 reason 匹配该父级级联原因的子 Session 投递 resume，不唤醒原先独立暂停的子 Session。 |
| Goal 派生的 Work（`origin.goal`） | 托管进程只因其 Goal 被归档或不存在而停止它，不因创建它的 epoch 结束而停止；归属见 Agent State Protocol §9.3。 |

级联投递与子工作实际停止分别观察。外部 task 的取消受执行器能力与停止收尾路径约束：`runner/drive.rs::end_open_work` 根据 resolver 和 run 记录向可取消的 task 请求取消，但取消调用的错误不会阻止 Turn 收尾。运行中断的其它路径不能据此视为已经逐项发出取消。调用方应核对外部 task 的状态，不能以父 Turn 的 `stopped` 推断全部 task 已收到或完成取消；无法确认的结果保留未知或仍在执行的事实。所有停止路径均不承诺撤销已发生的副作用。

## 8. 宿主和界面映射

- **驱动者自己的停止请求**：驱动进程可以不经队列请求停止，例如 SIGINT 或宿主停止无队列子 Session。记录 `control_applied{command: stop, input.src: "_runner"}`，按 Session 范围停止；Goal Session 上仍只停当前 Activation。无队列 Session 当前仅有这条停止路径，没有 pause / resume 的旁路。
- **无队列 Session**：不接受队列型控制。`decide` 可由决定者在 `artifact:<aid>` 锁下直接操作产物列表（Agent State Protocol §6）。“所有 Session 都有控制队列”是 UI TODO 中待实施的变更。
- **聊天与按钮**：斜杠命令不由 Session 解析，msg bridge 在入队前按登记表与发送者身份转成 control。UI 的“停止回答”应明确选择 Turn 作用域；命令名、按钮名和 TaskMgr 的“取消”字样都不能替代作用域。各入口当前映射与改动计划见 UI TODO §1.7、§2.4。
- **任务服务**：外部取消 / 暂停要转为经过授权的控制请求，再由 Session 报告实际状态；不能只修改 TaskMgr 的显示状态。当前 opendan 对暂停 Turn 仍上报 Running / Waiting 并在 message 写 `paused: …`，TaskMgr Paused 状态桥接属于 UI TODO SC3。
