# Session Control Protocol（Session 控制）

版本 3 · 2026-10-03（同日补充驱动者停止请求与无队列 Session；2026-10-11 增加 `stop.scope`、`goal_skills` 与 Goal Session 的结束）· 由 `libopendan` 反写（`src/protocol/input.rs::ControlCommand`、`src/runner/inputs.rs`、`src/runner/goal.rs`）。

控制命令与 Agent 输入走同一条总线（`type = "control"`，信封见 [Session Input Protocol](<Session Input Protocol.md>) §2），但不是 Agent 的输入：由 Runner（lease 持有者）执行，不进入 LLM 上下文，不产生输入 receipt。搭同一条队列只是为了多方投递、持久化和与事件保持同一个顺序。

```json
{ "schema": "opendan.session_input/3", "type": "control", "key": "ctl:stop:1790899260000",
  "from": "did:bns:alice", "at_ms": 1790899260000,
  "payload": { "command": "stop", "reason": "用户取消" } }
```

| `command` | 字段 | 应用规则 |
|---|---|---|
| `stop` | `reason?`、`scope?: session \| turn`（缺省 `session`） | `session`：置 `stop_requested`。运行中：在下一个检查点打断推理；工具执行期间由监视任务发现队列中的 stop 并打断（监视任务只查看，由驱动者消费并提交）。run 结束，打开的 Turn 以 `stopped` 关闭，`finished + stopped`。等待 task 的挂起调用按当时状态回填，可取消的 task 被取消。`turn`：只在有打开的 Turn 时置 `turn_stop_requested`，同样打断，Turn 以 `stopped` 关闭，Session 继续等待下一个输入（不停子 Session），之后清除标志；没有打开的 Turn 时只记录（`detail.applied = false`）。**Goal Session 收到的任何 stop 都按 `turn` 处理**：它只随它的 Goal 结束 |
| `goal_skills` | `add[]`（`{name, ref?, version?, note?}` 或名字）、`remove[]`（名字） | 仅 Goal Session：改它拥有的 Goal State 的 `current_skills`（`state.goal.state.rev + 1`），随后发布；其它 Session → `invalid_payload` |
| `decide` | `decision: accept \| discard`、`by`、`note?` | finished 之后唯一仍可接受的输入，仅 work session。运行中或未 finished 时留在队列并写 `pending_decision`；accept 需 `acceptance = pending`，discard 需 `pending \| accepted`，否则拒绝（`input_rejected`） |
| `subscribe` | `subscription: {id, mode: active \| semi, source, watch[]}` | 加入 `session_config.subscriptions`（`config_rev + 1`）。`source`：`{type: session, ref}`、`{type: object_event, object, event?}`、`{type: task, task_id}`、`{type: timer, name}`、`{type: system, id}` |
| `unsubscribe` | `id` | 移除订阅、其拉模式游标和 `pending_events` 中尚未注入的状态；之后到达的该订阅事件丢弃 |
| `activity` | `summary?`、`touch[]`、`clear` | 合并进 `state.activity`（touching 上限 16） |
| `perceive` | `kind`（默认 `observation`）、`summary`、`tags[]`、`objects[]` | 驱动者以 lease 追加到 Agent 感知流（按 seq 幂等），再消费输入 |

- 控制命令与事件按投递 index 依次生效（同一源内）。
- 应用时写 `control_applied`（decide 写 `decide`），与消费标记同一次提交；之后确认输入源。
- 未知 `command` → `unknown_command`；字段不合法 → `invalid_payload`。
- 控制命令与 msg / event 共用每个 Session 64 条 pending 记录的上限。
- **驱动者自己的停止请求**：驱动进程可以不经队列请求停止（收到 SIGINT、宿主停止一个没有队列的子 Session）。效果与已消费的 `stop` 相同：置 `stop_requested`，写 `control_applied{command: stop, input.src: "_runner"}`，运行中的推理 / 工具被打断，Turn 以 `stopped` 关闭。没有输入队列的 Session 只有这一种 stop。Goal Session 上它只停当前 Activation（每次 drive 最多一次）。
- 托管进程对子 Session 的停止：父以 `stopped` 结束时停止它的子 Session；Goal 派生的 Work（`origin.goal`）只在其 Goal 被归档（或不存在）时停止，与创建它的 epoch 是否结束无关。
- **Goal Session 的结束**不是控制命令：驱动者在每个提交点读取它的 Goal（`state/goals/<id>/goal.json`），Goal 被归档（`ending.cascade = true`：同时向本 Session 的有队列子 Session 投递 stop；Goal 的其它 Work 由托管进程按“Goal 已归档”停止）、达成（`succeeded`）、或进入了新 epoch（restart / reopen：`stopped`，不停止 Work）时写入 `state.goal.ending` 与 `control_applied{command: goal_end}`，再按 `stop_requested` 的路径结束：打开的 Activation 以 `stopped` 关闭并写回。暂停的 Goal 不结束 Session，只不再打开新的 Activation。
- 没有输入队列的 Session 不接受队列型控制；`decide` 由决定者在 `artifact:<aid>` 锁下直接走产物列表（Agent State Protocol §6），不经 Session。
- 斜杠命令不由 Session 解析：msg bridge 在入队前按登记表与发送者身份把 `/stop` 等转成 `control`。
