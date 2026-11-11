# Session Control Protocol（Session 控制）

版本 3 · 2026-10-03；2026-10-11 增加 `stop.scope`、`goal_skills` 与 Goal Session 的结束；2026-11-10 汇总已实现的 pause / resume、控制作用域与级联规则，校正取消确认及终态拒绝说明；同日增加 §9（owner 从用户角度给出的有效语义与控制的对应）和 §8 的宿主重启恢复现状，并把 owner 对 §9 五处待定的裁决写进 §9.2；同日 owner 确认了 §9.2 里的三条推定，按代码核对后修正了“正在工作”的判定，并补了 §9.3（核对出的缺口）。版本 4 · 2026-11-10：[Agent UI 升级 TODO](../../../notepads/agent-ui-upgrade-todo.md) 的 SC0–SC11 实施后回写 §3–§8：所有 Session 都有控制队列；`stop.scope` 增加 `archive`，Session 范围的 stop 按会话类型分派；新增 `retract`；暂停带来源；宿主重启后正在工作的 Session 停在“异常中断”，手工恢复。

本文是 Session 控制语义的统一入口，由 `libopendan` 的 `src/protocol/input.rs::ControlCommand`、`src/runner/{inputs,drive,outcome,goal}.rs` 反写。输入信封与消费规则见 [Session Input Protocol](<Session Input Protocol.md>)，持久字段、提交与恢复见 [Session Directory Protocol](<Session Directory Protocol.md>)，Goal 对象及授权见 [Agent State Protocol](<Agent State Protocol.md>) §9。架构约束见 [Session 架构设计 §12.12](<../OpenDAN Agent Session架构设计.md#1212-宿主装载运行状态与中断>)。

§3–§8 是现行合同。§9 保留 owner 给出语义和裁决时的记录，其中“现状”一栏写的是实施之前的情况；实施结果以 §3–§8 和 UI TODO §8 的实施记录为准。UI TODO §2.1 的推定 13 的两处修正、推定 15、16 实施后已由 owner 确认（UI TODO 的 D30）；§9.3 里说它们“等确认”的句子是确认之前写的。

## 1. 控制对象与边界

| 对象 | 控制边界 |
|---|---|
| Turn | 一次逻辑工作轮次，可跨多个 run 与上下文。停止当前 Turn 不结束 Session，也不等于成功完成任务。 |
| Run | 一个 LLMContext 的可恢复执行。暂停后继续同一个 Turn，不因恢复快照自动获得新 Turn。 |
| Session | 连续上下文与执行状态的容器。进入 `finished` 后不能由新输入或 resume 重新打开。 |
| Goal | 持久目标及其调度状态。Goal 休眠（状态 `paused`）后不再推理：不开新的 Activation，正在进行的在下一个推理边界收尾。它与暂停 Goal Session（`control(pause)`）是两件事。 |
| 子 Session / 外部 task | 有自己的执行状态。是否级联与是否支持取消分别判断，父级停止不证明它们已停止。 |

`stop` 结束所指定范围内的执行；`pause` 保留未完成的执行，等待 `resume`。协议没有独立的 Session `cancel` 命令，也没有用 stop / cancel 区分“优雅停止 / 强制杀死”的约定。界面的“取消”、聊天命令别名以及外部 Task 的取消接口必须明确映射到哪个对象与操作。

停止、暂停、Goal 休眠、归档、卸载执行者和撤销产物不是同一件事。停止不删除历史，也不撤销已经发生的文件修改或外部副作用；撤销能力由具体工具和授权决定。所以控制能给用户的只有“不再推理”：已经执行的工具和它造成的影响都留着，没有哪条控制能让一条输入像没发生过（§9 的推论）。

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
| `stop` | `reason?`、`scope?: session \| turn \| archive`（缺省 `session`） | `turn`：只关当前 Turn。`session`：按会话类型，Work / self_check 结束，Goal Session 让它的 Goal 休眠，UI Session 拒绝。`archive`：只对 UI Session，归档。见 §4、§6。 |
| `pause` | `reason?` | 暂停 Session 推理，保留当前 Turn / run，按 §7 级联。没有 `scope` 或 `cascade` 字段。暂停的来源由驱动者按 `reason` 和 Session 的出身认定（§5）。 |
| `resume` | `reason?` | 清除 Session 暂停，从检查点继续或等待输入；按 §7 恢复因它而暂停、或与它同一次重启中断的子 Session。 |
| `retract` | `key`（被撤回输入的 `key`，即消息的 ObjId）、`reason?` | 撤回一条输入，按它所处的阶段分三种（§4.1）。 |
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
| `stop(scope=session)`，Work / self_check | 置 `stop_requested`；打开的 Turn 以 `stopped` 关闭 | `run_state = finished`、`outcome = stopped` | 按 §7 停止未结束的子 Session |
| `stop(scope=session)`，Goal Session | 不直接动 Turn | 由驱动者以投递者的身份把它的 Goal 置 `paused`（休眠，§6）；Session 与 Goal State 保留。Goal 已经不是 `active` 时记录 `applied = false` 与原因 | 休眠的 Goal 派生的 Work 一并暂停（§7） |
| `stop(scope=session)`，UI Session | 不变 | 拒绝：记录 `applied = false`、`refused`。UI Session 不被停止 | 无 |
| `stop(scope=archive)`，UI Session | 打开的 Turn 以 `stopped` 关闭 | `finished`、`outcome = stopped`；同一 `route_key` 的下一条消息新建一代 UI Session | 同 Session 范围（宿主在归档前拒绝还有未结束 Work 的会话） |
| `stop(scope=archive)`，其它 Session | 不变 | 拒绝，`applied = false` | 无 |

停止的范围就是用户的两种意图：针对消息的 stop（`turn`）和针对 Session 的 stop（`session`）。结束一个 UI Session 只有归档这一条路；归档是 clean（“开始新对话”），由宿主的 `ui.rotate` 和聊天命令 `/clean`、`/new` 发出，旧会话的目录、worklog、Session id 都保留。没有打开 Turn 时，Turn 范围 stop 记录 `applied = false`，也不禁止未来的新 Turn；它不是持续的暂停开关。停止一个被重启中断的 Turn（暂停来源 `interrupted`）时一并清掉这个暂停，Session 回到等待输入；其它来源的暂停不受 stop 影响（§5）。

已经接受的最终报告先于后到的 stop 完成；先被宿主观察到的 stop 拒绝新的结束提交。stop 不把已经由报告提交确定的终局改写成 `stopped`；若目标 Turn 已由报告关闭，Turn stop 清除标志后返回。提交先后规则见 Session Directory Protocol §7.1。

运行中的推理在中断边界停止；工具执行期间由监视任务查看队列中的 stop 并发出中断，监视任务不消费输入、不写 Session state，最终由驱动者消费并提交。等待 task 的挂起调用按可获得的状态回填。该行为不是“所有在飞调用自然返回后再停止”，也不保证任意外部工具都能立即终止。

### 4.1 撤回输入

`retract {key}` 撤回一条 msg 输入，`key` 是它的输入 key（消息的 ObjId）。驱动者按这条输入所处的阶段处理，结果记在 `control_applied{command: retract}` 的 `detail.outcome`：

| 输入的阶段 | 处理 | `outcome` |
|---|---|---|
| 还在队列里、没有被消费 | 标记为已消费并记 `input_rejected{reason: retracted}`，不进任何 Turn | `dropped` |
| 已被消费、它的 Turn 还开着 | 按 `stop(scope=turn)` 关闭这个 Turn；历史由 worklog 重建，这条 `control_applied` 在下一个 Turn 的历史里渲染成一行“用户撤回了这个 Turn 的输入，已经做的事不会撤销” | `stopped` |
| 它的 Turn 已经关闭 | 不动 | `closed`（`applied = false`） |
| 找不到这个 key（例如被撤回的是一条表情回应） | 不动 | `unknown`（`applied = false`） |

worklog 不改写：被撤回的 Turn 已执行的步骤都留在历史里。Turn 合并了多条消息时，撤回其中一条同样停止整个 Turn。入口是消息一侧的撤回关系（`relates_to: redact`）：msg bridge 认出 owner 发来的 redact 后转成这条控制，目标是 redact 指向的消息；不是 owner 发的不生效。

## 5. 暂停与恢复

`pause` 写入 `state.paused = {source, at_ms, reason, by, in_run}`，暂停的是 Session 的推理推进。`source` 是暂停的来源，决定谁来恢复它：

| `source` | 来源 | 恢复 |
|---|---|---|
| `user` | 有人投了 `pause` | `resume` |
| `parent` | 父 Session 被暂停，级联下来（`reason = parent session <sid> was paused`） | 父 Session 的 resume 级联；也可以单独 resume |
| `interrupted` | 宿主重启时它正在工作（§8）。不经过队列，由驱动者在重启后的第一次驱动里写入 | 手工 `resume`，级联到同一次中断的子 Session；或停止 |
| `goal` | 它的 Goal 休眠了（`reason = goal <id> is dormant`，§7） | Goal 唤醒时由宿主 resume；也可以单独 resume |

来源由驱动者认定，不是 `pause` 的字段：`reason` 等于父级级联的那一句且这个 Session 确有这个父 Session 时是 `parent`；等于 Goal 休眠的那一句且它确由这个 Goal 派生时是 `goal`；其余是 `user`。暂停的效果对各种来源相同：

- 不打开新 Turn，不产生 tick；到达的 msg / event 留在队列，所跟 task 的结束留待恢复后处理。
- 运行中的 run 在下一个推理边界，即在飞调用返回并写入检查点后，以中断原因 `session paused` 挂起。run 保留检查点，Turn 保持打开；记录 `worklog outcome.kind = paused`，暂停标记 `in_run = true`。
- 已经交出的外部 task 按其执行器 / OS 进程语义继续；暂停 Session 不等于暂停这些进程。
- 登记表的 `status.paused` 是来源（`user \| parent \| interrupted \| goal`），没有暂停时缺省；暂停中的 Session 不报 `needs_input`。已暂停时再次 pause 不覆盖原暂停标记，记录 `applied = false`。

`resume` 清除 `state.paused`。Turn 内暂停的 run 通过 `state.resume_run` 从检查点继续，与崩溃恢复共用恢复路径；Turn 之间暂停的 Session 由积压输入或下一次有效运行机会打开新 Turn。未暂停的 Session 收到 resume 时记录 `applied = false`。pause / resume 都不重新打开已结束的 Session（§2）。

恢复保留调用身份，不清空累计用量，也不补充当前 Turn 的工具额度；连续执行墙钟仍按 Session Directory Protocol §7 重新计时，不能把恢复误记成新的任务或新的 Turn。等待工具、等待用户、等待调度和用户主动 pause 分别记录，不因 run 快照的 `status = paused` 就认定 Session 收到了 pause。

Session 已暂停时仍可停止当前 Turn 或结束 Session；停止处理先于暂停闸门。stop 不隐式清除 `state.paused`，所以暂停期间的 Turn stop 关闭 Turn 后，后续推理仍需 resume。例外是来源为 `interrupted` 的暂停：停止（或撤回）被中断的 Turn 时一并清掉它，否则一个对话会一直停在暂停；Goal 休眠时被中断的 Activation 也照此处理（§6）。

## 6. Goal 生命周期

Goal 对象的 `status = paused` 是休眠：不再打开新的 Activation，也不再发起下一次推理。正在进行的 Activation 里，在飞的这次推理和它的工具不打断；到下一个推理边界（以及驱动循环的每个提交点），驱动者读到 Goal 已休眠，就把当前 Activation 以 `stopped` 收尾（`control_applied{command: stop, input.src: "_runner", scope: turn}`），照常写回 Goal State。归档按同一条规则在推理边界生效。休眠不走 `control(pause)`，Goal Session 自己的 `state.paused` 不动；唤醒（恢复为 `active`）后等下一次运行机会，不续上被收尾的那次 Activation，也不等于 `control(resume)`。只结束当前 Activation 时用 `stop(scope=turn)`；对 Goal Session 发 `stop(scope=session)` 就是让它的 Goal 休眠（§4）。

Goal Session 的终止由 Goal 生命周期驱动，不是另一个 `ControlCommand`。驱动者读取 `state/goals/<id>/goal.json`，在 Goal 归档、达成或进入新 epoch 时写入 `state.goal.ending` 与 `control_applied{command: goal_end}`，再走停止提交路径；打开的 Activation 以 `stopped` 关闭并写回：

- Goal 归档：`ending.cascade = true`，向有队列的子 Session 投递 stop；Goal 的其它 Work 由托管进程按 Goal 已归档处理。
- Goal 达成：Goal Session 以 `succeeded` 结束。
- restart / reopen 切换 epoch：旧 Session 以 `stopped` 结束，不停止 Goal 派生的 Work；新 epoch 按 Agent State Protocol §9 接管。

界面和文档把 Goal 的手工 disable 叫“休眠”、恢复叫“唤醒”，以免和 Session 的暂停混在一起；状态值仍是 `paused`，CLI 仍是 `agent-session goal pause | resume`。Goal 状态操作及其权限以 Agent State Protocol §9 为准。

## 7. 子工作与外部 Task

| 操作 | 级联规则 |
|---|---|
| Turn 范围 stop | 不停止子 Session。外部 task 的取消请求与结果须单独核对。 |
| 非 Goal Session 以 stopped 结束 | 对未结束的子 Session 投递 stop。普通结束路径只处理需向父汇报的未结束子 Session。 |
| Session pause | 对未结束的子 Session 投递 pause，reason 为 `parent session <sid> was paused`（来源 `parent`）。 |
| Session resume | 只在这个 Session 自己是暂停的时候才往下级联。恢复两种子 Session：暂停 reason 匹配该父级级联原因的；这个 Session 的暂停来源是 `interrupted` 时，来源同为 `interrupted` 的。不唤醒原先独立暂停的子 Session，也不带动父级。 |
| Goal 派生的 Work（`origin.goal`） | Goal 归档或不存在时托管进程停止它，不因创建它的 epoch 结束而停止。Goal 休眠时托管进程对未结束、没有暂停的派生 Work 投递 pause（reason `goal <id> is dormant`，来源 `goal`）；Goal 回到 `active` 时只 resume 来源是 `goal` 的。Work 的暂停照常级联到它自己的子 Session。归属见 Agent State Protocol §9.3。 |

级联的起点必须是一个暂停着的 Session：对没有暂停的 Session 发 resume 记 `applied = false`，不往下走。父 Session 没有中断、子 Session 中断了的时候（空闲的对话名下有中断的 Work），由调用方找出中断的 Session 里父级没有中断的那些，逐个发 resume；宿主的 `/resume` 和界面的“全部恢复”都这么做。

没有队列客户端的宿主（不在 Zone 里、也没有 `--queue-dir` 的 `xagent`）建出的 Session 没有队列，上面的投递会跳过它们，由驱动它的进程自己处理停止。

级联投递与子工作实际停止分别观察。外部 task 的取消受执行器能力与停止收尾路径约束：`runner/drive.rs::end_open_work` 根据 resolver 和 run 记录向可取消的 task 请求取消，但取消调用的错误不会阻止 Turn 收尾。运行中断的其它路径不能据此视为已经逐项发出取消。调用方应核对外部 task 的状态，不能以父 Turn 的 `stopped` 推断全部 task 已收到或完成取消；无法确认的结果保留未知或仍在执行的事实。所有停止路径均不承诺撤销已发生的副作用。

## 8. 宿主和界面映射

- **控制队列**：宿主有队列客户端时，每个 Session 都有输入队列，控制命令一律走它。队列收不收 msg / Input event 是另一个属性：`session.input_policy = none` 的 Session（内置 `work` 类，以及模板里 `input = "none"` 的类）只收控制命令，它的首批材料来自 `prompt.initial_inputs`；向它投 msg 在投递时就被拒绝并说明原因。收消息的后台 Work 只有 plan（`input = "queue"`）、Todo 派发的 Work 和 `--interactive` 的 Work。
- **驱动者自己的停止请求**：驱动进程可以不经队列请求停止，例如 SIGINT。记录 `control_applied{command: stop, input.src: "_runner"}`，按 Session 范围停止；Goal Session 上只停当前 Activation。
- **无队列 Session**：只出现在没有队列客户端的宿主上（§7 末）。不接受队列型控制；`decide` 可由决定者在 `artifact:<aid>` 锁下直接操作产物列表（Agent State Protocol §6）。
- **聊天与按钮**：斜杠命令不由 Session 解析，msg bridge 在入队前按登记表与发送者身份处理。`/stop`、`/cancel` 转成 `stop(scope=turn)`，只投给对话当前的 UI Session，没有打开的 Turn 时不投、回一句提示；`/clean`、`/new` 由宿主归档当前 UI Session（有未结束的 Work 时拒绝并列出它们）；`/resume` 由宿主恢复这个对话里被中断的 Session（它自己，否则它名下最靠上的那些）；owner 的 redact 转成 `retract`。控制和撤回都不会为一个还没有 UI Session 的对话新建会话。界面的“停止”按钮在占位消息上发 `turn` 范围，在后台 Work 上发 `session` 范围。
- **任务服务**：TaskMgr 一侧的取消按 task 属于哪类会话转成 stop：UI Session 和 Goal Session 上是 `turn` 范围，Work 上是 `session` 范围（那个 task 就是这件工作）。暂停着的 Session，其打开的 Turn 的 task 是 TaskMgr 的 `Paused` 状态，message 写 `paused: …`，异常中断写 `interrupted: …`；恢复后回到 Running / Waiting。TaskMgr 一侧发起的 Pause / Resume 请求不转成控制：Session 暂停或恢复时，待处理的同类请求会被一并确认，除此之外不起作用。
- **宿主重启后的恢复**：宿主启动后第一次扫描登记表（`host/mod.rs` 的 `Supervisor::tick`）找出它驱动的未结束 Session，为每个起驱动循环。这一轮里（直到第一次扫描结束）起的驱动循环，在拿到 lease、读到 `state.json` 之后先判定这个 Session 重启前是不是正在工作；是的话写 `state.paused`（来源 `interrupted`，`reason = interrupted by a restart of the host`，`by` 是宿主）和 `control_applied{command: pause, input.src: "_runner", source: interrupted}`，然后停住，不推理。断电、崩溃、正常重启不区分。
  - 正在工作：Work Session 未结束、没有暂停、不是在等用户（`run_state = waiting` 且不在等自己发起的工作）就算，不管这一刻有没有打开的 Turn，包括刚建好还没开始的——等子任务的 Work 会自己按 tick 推理；UI、Goal、self_check Session 看“有打开的 Turn 且不是在等用户回答”。空闲的对话、两次 Activation 之间的 Goal Session、在等用户回答的 Session 不算，新消息和下一次运行机会照常处理。
  - 判定以 `state.json` 为准，登记表只用来找出要看的 Session。之后新建的 Session、驱动循环出错后重起的都不算。用户暂停过的 Session 重启后仍是原来的来源。
  - `resume` 清掉标记，从检查点继续（§5），级联到同一次中断的子 Session（§7）。中断期间到达的输入留在队列。停止被中断的 Turn 时一并清掉中断标记（§4）。
  - 每个宿主只处理自己驱动的 Session。`xagent` 命令行直接驱动的 Session 不经过 Supervisor，行为不变。自动恢复没有做。

## 9. 用户语义与控制的对应（owner，2026-11-10）

owner 从用户角度给出的有效语义。§9.1 把每一项对到前面各节的控制，并标出现状；§9.2 是 owner 同日对其中待定处的裁决，以及由此要补的协议；§9.3 是按“手动控制的语义要做对”对着代码核对出的缺口。标为“未支持”“目标”的都不是已有行为，实施时回写 §3–§8。任务拆分见 UI TODO 的 SC1、SC2、SC9、SC10、SC11。

### 9.1 语义与控制的对应

| # | 用户遇到的情况 | 用户想要 | 对应的控制 | 现状 |
|---|---|---|---|---|
| 1a | 发出消息后发现不对，例如关键信息输错了 | 立刻停止推理，马上能输入下一条；旧输入留在历史里，没有 Agent 的回复 | 对 UI Session 发 `stop(scope=turn)`（§4） | 控制已有，在飞的推理和工具会被打断。入口还没改：`/stop` 和 TaskMgr 取消今天发的是 session 范围，会结束整个 UI Session |
| 1b | 同上 | 立刻停止，并删掉这条输错的消息，相当于没输入过 | 1a 加“撤回输入”（§9.2 第 3 条） | 未支持。没有撤回输入的控制；worklog 不改写，下一个 Turn 的历史由它重建 |
| 2a | 看到后台 Work Session 做错了 | 发消息纠正它的方向 | 向 plan 会话的队列投 `msg`（Session Input Protocol §3） | 只对 plan（`plan_simple` 类）成立，今天已有：对话里的 Agent 把用户的话转给它（`agent-session post`）。纯 Work 不收用户消息，这是刻意的区分（§9.2 第 2 条） |
| 2b | 同上 | 先停下它的推理（省 token），发消息，再恢复 | `pause` → `msg` → `resume`（§5） | 同样只对 plan 成立。pause 在下一个推理边界生效；恢复后先跑完被打断的那一批，再接收消息（Session Input Protocol §3）。纯 Work 可以暂停和恢复，中间插不进消息；没有队列的今天连暂停也收不到（§8） |
| 2c | 同上 | 终止这个后台任务；之后可能再发消息触发新的任务 | 对这个 Work 发 `stop(scope=session)`（§4、§7） | 已有，连同它未结束的子 Session。没有队列的 Work 由宿主停止（§8） |
| 3 | Goal 没有效果 | 停用（休眠）或删除，不再推理 | Goal 状态置 `paused` 或 `archived`（§6） | 状态操作已有。生效点与目标不符：推理边界不看 Goal 状态，正在进行的 Activation 会一直推理到自然结束（§9.2 第 4 条）；休眠不影响 Goal 已经派出的 Work，它们照常跑到结束，只有归档停它们（§7，§9.3） |
| 4 | 断电后恢复 | 断电前正在工作的 Session 都变成“异常中断（可恢复）”，由用户手工恢复，按任务树恢复；也可以配置成自动恢复 | `resume`（§5），前提是有“异常中断”这个状态 | 未支持。今天宿主启动后全部自动接着跑（§8）；自动恢复的配置推到下一期（§9.2 第 5 条） |

推论（owner）：所有控制都做不到“消除一条错误输入带来的影响”。工具用了就是用了，能做到的只有停止推理、省下 token。§1 的边界与此一致。

按这份语义读前面各节：

- **停止要立刻生效。** 现有的 stop 就是打断，不等在飞的调用返回（§4），不用改。
- **停止后历史里留下的不只是旧输入。** 这个 Turn 已经做过的推理步骤和工具结果都在 worklog 里，下一条消息到来时模型看得到，知道自己做过什么。这与推论一致。

### 9.2 裁决与要补的协议（owner，2026-11-10）

“owner：”后面是原话，其后是整理者的展开。展开里没有 owner 原话支持的部分标了“推定”。owner 同日确认这三条推定都按建议，并说明自动恢复是下一期的，这一期把手动控制的语义做对；确认之后的修正和新的缺口在 §9.3。

1. **前台消息没有暂停这个操作。** owner：不要暂停。产品上这个操作没意义，（但断电恢复其实要求这个状态语义上是存在的）。
   - pause 保持 Session 级：不加 `scope`，不加不级联的选项。界面只在后台 Work 上提供暂停。
   - UI Session 仍然可以处在暂停状态，来源是异常中断（第 5 条），这时界面提供恢复和停止。
2. **后台 Work 里只有 plan 收用户消息。** owner：按设计，plan_simple 才可以收消息（纯 worksession 是不行的），这是一个刻意的区分。
   - 2a、2b 只对 plan 成立。纯 Work 能暂停、恢复、终止，不能用消息纠正；要改它的方向，终止后重新发起（2c）。
   - 今天的路径：用户在对话里说，UI Session 的 Agent 把话转给 plan，不转给 plan 派出的 Work（`behaviors/chat_route.toml`）。
   - 所有 Session 都带控制队列之后（UI TODO SC0），今天没有队列的 Work 只收控制命令，用 `session.input_policy = none` 表达。
3. **撤回输入（1b）。** owner：按你的建议吧。建议分两种情形：
   - 消息还在队列里、没有被消费：直接丢弃，真正等于没输入。
   - 已经开了 Turn：停止这个 Turn，记一条“用户撤回了这条输入”，并在时间线上撤回这条消息。Turn 里已经执行的步骤仍留在历史里，模型知道自己做过什么。
   - 推定（已确认）：Turn 已经关闭、Agent 已经回复的输入不能撤回。
   - 协议里要新增一条控制，以输入的 `key`（消息的 ObjId）为目标，命令名和字段没有定。消息一侧已有撤回关系（MsgObject 的 `relates_to: redact`），桥今天不处理它，可以作为入口。
4. **Goal 休眠后不再发起下一次推理。** owner：不发起下一次推理就好。
   - 推定的读法（已确认）：“下一次推理”指下一次模型调用。在飞的这次推理和它的工具不打断，到下一个推理边界，当前 Activation 以 `stopped` 收尾，之后不再打开新的 Activation。
   - 今天推理边界（`runner/hook.rs` 的 `boundary`）只看 stop、Turn stop 和 pause，不看 Goal 状态；Goal 的闸门只在驱动循环的提交点读（`runner/goal.rs` 的 `gate`）。归档今天也在这个点才生效，按同一条原则处理。
   - 实施后 §6 第一段“已打开的 Turn 跑到自然结束”要改。
5. **重启后一律手工恢复。** owner：现在都手工恢复吧，自动恢复留到下一期再做。
   - 断电、崩溃、正常重启不区分（宿主今天也分不出）。重启前正在工作的 Session 变成“异常中断（可恢复）”，用户手工恢复。
   - 这个状态沿用暂停（第 1 条 owner 的括号）：`state.paused` 带专门的原因，恢复用现有的 `resume`，走 §5 的检查点恢复路径。
   - 推定（已确认，随后有修正，见 §9.3 第 1 条）：“正在工作”指宿主启动后第一次扫描时有打开的 Turn 且没有暂停的未结束 Session。两次 Turn 之间的 Session 不算，照常等输入、等调度。
   - 按任务树恢复：resume 一个 Session 时，级联到它名下因同一次中断而停下的子 Session。今天 resume 只恢复原因是“父级被暂停”的子 Session（§7），要扩展。级联只从一个暂停着的 Session 开始，这一点不变。
   - 要改的两处现有合同。登记表的 `status.paused` 只是布尔值，要带上原因，界面才分得出用户暂停和异常中断。§5 的“stop 不清除暂停标记”对异常中断要例外：停止一个中断的 Turn 时一并清掉中断标记，否则 UI Session 会一直停在暂停。

### 9.3 核对出的缺口（2026-11-10）

owner 确认 §9.2 的三条推定时定了这一期的标准：自动恢复下一期再做，手动控制的语义要做对。检验办法是对 §9.1 的每一项问：用户做了这个操作之后，还有没有哪里在他不知道的情况下接着推理。下面是对着代码核对的结果，都是读代码得出的，没有跑用例。第 1 条是对已确认推定的修正，第 3、6 条是新的推定，在 UI TODO §2.1 等确认；其余是实现上的要求。

1. **“正在工作”不能只看有没有打开的 Turn（修正 §9.2 第 5 条的推定）。**
   - 多 Turn 的 Work Session 在两次 Turn 之间等自己发起的工作时，会按 `policy.tick_secs` 产生 tick，每次 tick 打开一个 Turn（`runner/ticks.rs`）。内置 `work` 类是 30 秒，`plan_simple` 是 600 秒。只按打开的 Turn 判定的话，重启后 plan 的子 Session 停住了，plan 自己还在按 tick 推理。所以 Work Session 的判定是：未结束、没有暂停、不是在等用户，不管这一刻有没有打开的 Turn，包括刚建好还没开始的。
   - 在等用户回答的 Session 不算，不管是哪一类。它不会自己推理，用户一回答就继续；标成中断的话，回答进了队列却没有反应。
   - UI、Goal、self_check Session 仍按“有打开的 Turn 且不是在等用户回答”判定。
   - 判定以 `state.json` 为准。提交时先写 state 再上报登记表，断电时登记表可能落后一步；登记表只用来找出要看的 Session。
2. **resume 的级联只从暂停着的 Session 开始（现有合同，不改）。** 对没有暂停的 Session 发 resume 记 `applied = false`，不往下级联。会出现父 Session 没有中断、子 Session 中断了的情形：空闲的 UI Session 名下有中断的 Work，两次 Activation 之间的 Goal Session 名下有中断的派生 Work。一次恢复全部由调用方做：找出中断的 Session 里父级没有中断的那些，逐个发 resume。
3. **Goal 休眠不停它已经派出的 Work（推定：要一并暂停）。** 今天宿主只在 Goal 归档或不存在时停这些 Work（§7）。用户对休眠的要求是不再推理，派生的 Work 接着推理就不符合。目标是休眠时对它们投 Session pause 并注明原因，唤醒时只恢复因此暂停的。这会成为暂停的又一个来源。
4. **停止一条消息不停它已经发起的 Work（§4、§7，已有裁决，不改）。** 这些 Work 还在按被停止的输入推理。协议不变，由界面在停止或撤回之后列出它们并提供停止。
5. **中断的 Goal Session 被置休眠时，要关掉中断的那次 Activation 并清掉中断标记。** 否则唤醒之后它仍是暂停的，等不到下一次运行机会。这与 §9.2 第 5 条里“停止一个中断的 Turn 时一并清掉中断标记”是同一条例外。
6. **没有界面的渠道要有恢复的入口（推定）。** 中断的 UI Session 收到普通消息时只是入队，用户看不到任何反应。目标是加聊天命令 `/resume`（转成对当前 UI Session 的 resume，并按第 2 条恢复它名下中断的 Work），并在普通消息到达中断的 UI Session 时回一句提示。两者都在 bridge 和宿主一侧，不新增控制命令。
7. **暂停后发消息再恢复（2b）还依赖提示词。** 对话里的 Agent 只把用户的话转给“还在运行”的 plan；plan 暂停着时它会不会转，提示词没有写明。要写明暂停中的 plan 照样转，消息在恢复后处理。
