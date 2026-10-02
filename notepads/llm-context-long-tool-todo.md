# llm_context 层修改 TODO：长命令、长工具与等待

日期：2026-10-02

状态：待 review。依据 2026-10-02 对长命令处理方式（同步硬等、同步执行中崩溃、串行等待、并行等待）的讨论与源码核对，并已结合 [lib_opendan 输入协议与 Turn Loop TODO](./lib-opendan-input-and-turn-loop-todo.md) 的 review 意见。实施前重新确认基线。

## 1. 范围与原则

- 范围：`llm_context` crate，以及 `agent_tool` 中属于 llm_context 层的 exec、exec_tracking、xllm。libopendan 的宿主接入写在 [lib_opendan TODO](./lib-opendan-input-and-turn-loop-todo.md)，本文只在需要时指向它。
- **llm_context 层的修改总是先进行**，libopendan / xagent 的宿主接入在其后。需要 review 确认的 llm_context 事项也记在本文。
- Runtime 配置、Sandbox 与统一派发沿用 [AgentRuntime 下移 TODO](./llm-context-agent-runtime-todo.md)，本文只补长命令相关的契约，交叉处已注明。
- 对 xAgent.md 的修改列在 §8，review 通过后再改该文档。
- 遵循 beta 2.2 规则：持久格式变化时显式升版并拒绝旧版本，不做旧格式兼容。

实现计划“实现落地”里 llm_context 层遗留的三项未完成，本文处理方式：

- deferred 工具回填：由 §4 接手。
- `ToolSpec.effect`：仍按 Runtime TODO §7 后移；§6 说明它对长命令作用有限。
- `ToolUse.args` 规范键序：与长命令无关，不在本文。

## 2. 现状（2026-10-02 源码核对）

| 场景 | 当前行为 | 问题 |
|---|---|---|
| 同步硬等 | waist 直接 `call_tool(call).await`（`context_loop.rs` 的 `run_tool_batch` 与 behavior action 派发）；interrupt 只与推理竞争，wallclock 只在迭代之间检查。xllm 的 `XllmToolManager` 自带 cancel watch 与总 deadline，触发时返回 `Observation::Error`，且只在 `XllmRun::execute` 接线（xAgent G5）。exec 默认 30 分钟、最长 60 分钟，超时杀进程组 | 宿主 run（libopendan）里 stop 和 wallclock 都打断不了工具，最长要等到 exec 超时；取消被记成业务失败，而不是 Cancelled |
| 同步执行中崩溃 | 派发前 fsync `InflightAction`；`TrackedBashRunner` 握手持久化 `ExecutionRecord`，子进程带 `OPENDAN_EXECUTION_ID`；恢复先 `stop_execution`（无法核验 → RecoveryBlocked），再 `materialize_unresolved` 注入“结果未知” | 方向正确：交给 Agent 判断，不重放。但 Agent 拿到的信息少：不知道命令跑了多久、恢复时是否仍在运行并被停止、已有哪些输出 |
| 异步串行（PendingTool） | waist 完整：挂起、快照、续派、`ResumeFill::ToolResults`。`PendingToolCall.eta_ms` 与 `Outcome::PendingTool.deadline_ms` 恒为 None；等待对象只在 `tool_result.task_id` 里，没有类型。三个宿主都是 `allow_deferred=false`：工具返回 Pending → 调用记 Unknown，run 以 `Error{Internal}` 结束；xllm 不接手 PendingTool 快照，libopendan 返回 RecoveryBlocked | 没有结构化等待记录、截止时间、查询接口和宿主等待循环。`llm_explore` / `llm_understand_media` 的子 run 暂停或中断时会返回 Pending（`task_id=run_id`），实际就会触发上述 Internal 错误 |
| 异步并行 | 没有任务化的后台执行。exec 工具说明建议 `nohup cmd > log 2>&1 &` 后轮询日志；命令正常退出时不杀进程组，后台进程能活过本次调用 | 后台进程继承执行标记，`ExecutionRecord` 留在 run 记录里（`TrackedBashRunner::sweep` 在生产代码中没有调用）。libopendan 的 `finish_run` 先 `stop_executions`，run 一结束后台进程就被杀；xllm 接手该 run 时由 `settle_previous_executor` 杀掉，而原调用早已返回，不会注入任何说明，Agent 只会看到截断的日志 |

## 3. P0：同步长命令——工具执行期间的取消与时限

不依赖输入协议，可以立即开工。

- [ ] waist 为每次工具 / action 调用提供取消信号和截止时间，例如 `call_tool(call, ToolCallCtx { abort, deadline_ms })`，直接改 trait。`abort` 与推理共用 `LLMContextInterruptHandle` 的状态，stop 能打断正在执行的工具；`deadline_ms` 由 waist 按 `budget.max_wallclock_ms` 计算，各 ToolManager 不再自己维护。这同时解决 xAgent G5；Runtime TODO 的 `Sandbox::call_tool` 直接使用这个 ctx。
- [ ] 取消信号或截止时间触发后，允许 ToolManager 内联返回 `Observation::Cancelled`；其它情况下内联返回 Cancelled 仍属违反契约。waist 记 `ToolExecStatus::Cancelled`，同批余下调用记 `Unresolved{effect_unknown: false}`；中断给出 `Interrupted`，到期给出 `BudgetExhausted{Wallclock}`。
- [ ] 定义工具被中断时的快照语义。推理中断返回推理前快照 s0，恢复后重做推理；工具中断返回**已含配对 Cancelled 结果**的快照，恢复后不重跑该工具，LLM 看到的是“已取消”。behavior 模式按“第一个非成功结果停止其余 action”的规则处理。同步修改《LLM Context 设计》§8。
- [ ] 返回 Cancelled 的前提是执行体确认进程已停止（杀进程组并 probe）；无法确认时返回 `ToolDispatchError{effect_unknown: true}`。渲染文本写明“中途取消，可能已有部分副作用”，不暗示没有副作用。
- [ ] xllm：`XllmToolManager` 去掉自建的 cancel watch / deadline 分支，Ctrl-C 经 interrupt handle 进入 ctx。exec 自身超时仍是工具的 Error（`timed_out`），与 run 级取消区分开。
- [ ] 测试：function call 与 behavior 各覆盖执行中 interrupt、执行中 wallclock 到期、无法确认停止三种情况；快照恢复后不重跑已取消的工具。

宿主侧的配套（工具执行期间读取 stop 的监视任务）在 lib_opendan TODO §4。

## 4. P1：串行等待——PendingTool 的等待记录与查询接口

- [ ] **结构化等待记录**：`PendingToolCall` 增加 `wait`，由 ToolManager 从 `AgentToolResult` 的 `task_id / pending_reason / check_after / estimated_wait` 归一而来：

  ```text
  wait: {
    source: { kind, id },   // 与 AgentEvent.source 同一套词汇：job / subrun / task / approval …
    class,                  // wait_for_runtime_task | wait_for_task，对齐长任务 RFC §5.3
    check_after_ms, deadline_ms, detail
  }
  ```

  - `source` 回答“等的是谁”。它和事件来源用同一套词汇，lib_opendan TODO §4 的“挂起调用与事件匹配”只需比较 `(kind, id)` 是否相等。
  - `class` 回答“怎么等”：`wait_for_runtime_task` 可轮询、应有超时（本机 job、在后台推进的子 run）；`wait_for_task` 可以无限等待（TaskMgr、审批票据）。
  - 缺少可解析 `source` 的 Pending 一律拒绝（参照 `exec_bash` 不转发无 `task_id` 的 Pending）。
  - 快照升版。快照里的等待记录是唯一权威；Session 的 `waiting_for.refs` 在提交挂起时从它生成，`pending_task_calls` 删除（lib_opendan TODO §6.1）。非 Rust 实现可按 schema 解释。
- [ ] **截止时间**：`ToolPolicy` 增加 deferred 等待上限（全局默认，可按工具覆盖），填入 `Outcome::PendingTool.deadline_ms`。到期后宿主以 `Cancelled` 或 `Error{timeout}` 回填（见 §9 第 1 项），并经 resolver 停止后台任务。`wait_for_task` 类可以声明不过期。
- [ ] **查询接口**：`DeferredResolver` 定义在 llm_context 层，xllm 单独运行时也能使用；实现由宿主或 Runtime 提供，按 `wait.source.kind` 组合（示意）：

  ```rust
  #[async_trait]
  pub trait DeferredResolver: Send + Sync {
      /// 能解析的 `wait.source.kind`。
      fn kinds(&self) -> &[&str];
      /// Running { next_check_ms } | Ready(Observation) | Unknown { reason }
      async fn poll(&self, p: &PendingToolCall) -> ResolvePoll;
      async fn cancel(&self, p: &PendingToolCall, reason: &str) -> Result<Observation, String>;
  }
  ```

  - 通知只用来提前 poll，结果以 poll 为准（RFC §6–§7）。
  - Ready 的 Observation 与内联结果走同一条 `AgentToolResult → Observation` 映射。
  - Unknown 或查询失败时保留挂起现场并暴露诊断信息，不删除快照另起任务。
- [ ] **接手时的能力检查**：run.json 记录挂起调用所需的 resolver 种类（即 `wait.source.kind`）。任何接手方（xllm、xagent）缺少对应 resolver 时拒绝接手并说明原因，做法同现在的 `app_tools`。这回答 lib_opendan TODO §7 中“交接时挂起结果的提供方与能力不足时的行为”。
- [ ] **xllm 等待循环**：
  - 允许开启 `allow_deferred`：`.llm_context` 显式开关，或者所有可能返回 Pending 的工具都有 resolver 时自动开启。
  - 遇到 PendingTool 时：持久化快照 → run 状态 `waiting`（带等待记录）→ 按 `check_after` 有界退避 poll → `ResumeFill::ToolResults` 续跑同一 run。
  - Ctrl-C 或崩溃后，`xllm --resume` 重新进入等待循环，取代现在“不接手 PendingTool 快照”的行为。
  - `xllm status` 显示等待对象和截止时间。

  这就是 xllm 上的串行等待：function call 不返回，定期检查，崩溃后继续等。
- [ ] **区分“后台在推进”与“需要外部动作”**：`llm_explore` / `llm_understand_media` 的子 run 处于 Paused / Interrupted 时不会自己推进，poll 只会一直等到超时。需要二选一（见 §9 第 2 项）：
  - 改为 Error，并附 `xllm --resume --run <id>` 提示；
  - 作为 `wait_for_task` 类等待，由宿主或人工接手子 run 后再回填。

  只有真正在后台推进的子 run 才用 `wait_for_runtime_task`。
- [ ] **派发时宿主未开 deferred 的降级**：工具返回 Pending 而宿主 `allow_deferred=false` 时，不再以 Internal 错误结束整个 run。ToolManager 知道是否开启；关闭时按工具声明降级为以下两种之一（waist 的严格契约不变，规则见 §9 第 3 项）：
  - 在工具内阻塞，直到终态（同步语义，受 §3 的取消和时限约束）；
  - 转成携带任务引用和查询方法的普通 Success（并行语义）。

  这与“已处于挂起态的快照遇到缺能力的接手方 → 拒绝 / RecoveryBlocked、保留现场”（lib_opendan TODO §6.1）是两个不同时机，不矛盾。
- [ ] 一次挂起只等一个调用，同批后续调用等回填后再派发——保持现状（串行语义），写进设计文档，本期不做并发等待。
- [ ] 挂起期间收到 stop 的处理按 §9 第 7 项定稿后，在 waist 文档中写明回填 Cancelled 后 run 的结束方式。
- [ ] 测试：function call 与 behavior 各覆盖以下路径：
  - Pending → 持久化 → kill -9 → resume 后继续等待 → 终态回填 → 同一 run 续跑；
  - 截止时间到期；
  - resolver 返回 Unknown；
  - 接手方缺少 resolver 时拒绝；
  - 回填后再次 Pending 的链式等待。

## 5. P1：并行等待——后台 job 与 run 的所有权分离

执行跟踪的前提是“run 结束或被接手之前，它启动的进程全部确认停止”；并行等待要求后台任务比 run 活得久。两者只有在所有权被显式区分后才能共存。

- [ ] **两类执行**：
  - run-owned：前台 exec 及其派生进程，保持现有纪律，run 结束或被接手前必须停止。
  - detached job：显式以后台方式启动，不进 `run.executions`；run 结束、Runner 崩溃、run 被接手都不停止它。

  job 属于执行体（Runtime TODO 的 Sandbox），归属记录在 Session（libopendan）或 workdir 下的 job 存储（xllm）。
- [ ] **启动与记录**：
  - exec 增加 `background: true`（或独立的 `job_start`，见 §9 第 5 项），立即返回 Success，内容包括 `job_id`、日志位置和查询方法。
  - 沿用启动握手：先持久化 job 记录，再放行命令。
  - job 用独立的环境标记（如 `OPENDAN_JOB_ID`），run 恢复时的扫描匹配不到它。
  - 包装脚本在命令结束时写入退出码和结束时间，job 状态不依赖 Runner 存活。
- [ ] **稳定身份与崩溃恢复**（同 lib_opendan TODO §6.3 的原则）：`job_id` 由 `(session, run, call_id)` 推导（xllm 无 Session 时用 `(run, call_id)`）。启动 job 的调用如果在 Runner 崩溃时仍在 inflight：
  - job 记录已存在 → 回填为带 `job_id` 的已知结果，不注入“结果未知”，避免 Agent 重复启动；
  - job 记录不存在 → 启动握手保证命令没有执行，记为未执行。
- [ ] **查询与停止**：
  - `job_status`：running / exited{code} / lost（进程不在也没有退出记录，即被杀或重启过）/ unknown；以 probe 结果和退出记录为准。
  - `job_output`：日志尾部，有长度上限。
  - `job_stop`。
- [ ] **生命周期**：并发数和最长存活时间有上限；提供按归属停止并清理 job 的接口（libopendan 在 Session finished / discard 时调用，见 lib_opendan TODO §6.2）；xllm 的 job 存储要有 GC 规则；无法核验的 job 不静默删除。
- [ ] **完成通知钩子**：提供 job 完成的观察接口（watch 或 poll 退出记录），宿主据此产出 `AgentEvent{source: {kind: job, id}}`。llm_context 层不负责事件路由；libopendan 以 Runner 内置 bridge 接入（lib_opendan TODO §6.2）。
- [ ] **串行等待也用 job**：同一个 job 也可以按串行语义使用。比如 `background: "wait"`（名称待定）启动 job 后返回 `Pending{wait: {source: {kind: job, id}, class: wait_for_runtime_task}}`，由 job resolver（§4）poll 并回填。这样 exec 的串行、并行两种等待共用一套 job 底座，崩溃恢复行为一致。串行模式的 job 不登记事件订阅，避免同一完成既回填结果又作为事件注入。
- [ ] **前台 exec 留下后台进程**（命令里自己写了 `&` / nohup）：
  - 结果里明确告知 LLM：这些进程属于本 run，run 结束时会被停止；长时间任务请用 background 模式。
  - xllm 和 libopendan 统一在 run 结束时停止这些进程。xllm 现在不停，是否统一见 §9 第 4 项。
  - 恢复时被停止的进程，要在 transcript / worklog 里留说明，不再静默。
  - 在 checkpoint 边界调用 `sweep()`，及时释放已退出的执行记录。
- [ ] 改写 exec 工具说明：去掉 nohup 建议，改为介绍 background 模式和 job 工具的用法。
- [ ] 实施位置：首版只做 native。tmux / remote_ssh 按 Runtime TODO 的执行体接口实现同一契约；未实现时，background 明确报告能力不足。
- [ ] 测试：job 在 run 结束、Runner kill -9、run 被接手后都继续运行；启动后、结果持久化前崩溃不产生重复 job；退出记录在 Runner 不在时仍能写入；lost 状态可识别。

## 6. P2：同步崩溃恢复的信息补全

- [ ] 恢复时区分“命令已退出”和“仍在运行、被停止”，写进 unresolved 的 reason，并附上命令、开始时间和已运行时长。
- [ ] `TrackedBashRunner` 把输出同时写入执行目录下的有界日志，崩溃后 unresolved 结果可以附上输出尾部，帮助 Agent 判断进度和是否重试。
- [ ] 保持不重放。`ToolSpec.effect` 的迁移仍按 Runtime TODO §7 后移。`exec` 的副作用取决于具体命令，effect 只能是 unknown，所以长命令崩溃后始终交给 Agent 判断。需要自动续跑的长任务走 §5 的 job，依据 job 状态判断，不重放命令。

## 7. 与 lib_opendan TODO 的对应

| 本文 | lib_opendan TODO | 关系 |
|---|---|---|
| §3 工具取消与时限 | §4 工具执行期间的 stop 监视；§5 单写者纪律 | 监视任务依赖 `ToolCallCtx`；只查看控制输入并触发中断，不确认 / 消费输入、不写 state |
| §4 `wait.source` / `wait.class` | §4 挂起调用与普通订阅分别登记和匹配 | 匹配按 `(kind, id)` 相等；`summary` 不参与 |
| §4 等待记录 | §6.1 `waiting_for.refs`、删除 `pending_task_calls` | refs 从快照的等待记录生成 |
| §4 `DeferredResolver`、run.json 记录所需种类 | §6.1 宿主查询能力；§7 xagent / xllm 交接 | 接口在本层，实现由宿主 / Runtime 提供 |
| §4 派发时降级 | §6.1 缺能力时 RecoveryBlocked | 两个不同时机 |
| §5 job 与完成钩子 | §6.2 自动订阅、内置 bridge、Session 结束停止 job | 本层提供 job 与钩子，路由与订阅在 Session |
| §5 job 稳定身份 | §6.3 dispatch intent 与幂等身份 | 同一原则 |
| §9 第 7 项 | §4 停止、审批和结果完成的先后 | 规则定稿后两边同步 |

## 8. 对 xAgent.md 的修改（review 通过后再改）

| 位置 | 修改 |
|---|---|
| §0 第 5 点、§5.2 | `DoContext.deadline` 与取消改由 waist 的 `ToolCallCtx` 提供（§3）；`RequireApproval` 的 Pending 带 `wait{source: {kind: approval, id: ticket}, class: wait_for_task}`，审批经 resolver / 控制协议回填 |
| §1.1 Agent Runtime 行 | 职责补上“后台 job 的所有权与查询”；“后台进程的识别与停止”限定为 run-owned 执行 |
| §3.1 G5 | 改为由 waist ctx 解决，`SessionToolManager` 不再自带 deadline |
| §4.3 / §4.6 | `EventSource` 与 `wait.source` 共用词汇，增加 `job`（以及 `subrun` / `approval`）；本机 job watcher 作为 Runner 内置 bridge，先于 task_mgr 桥落地，用来验证并行等待 |
| §4.7 Session 模板 | 无队列模板的并行 job 处理（§9 第 6 项）；Session 结束时停止其 job |
| §9.3 / §9.5 | PendingTool 分支：`waiting_for.refs` 由等待记录生成；等待出口与 inbox 是否为空无关，统一调用 resolver poll，Ready 后提交 ToolResults，续跑同一 run / Turn；挂起期间收到 stop 的规则（§9 第 7 项）；工具执行期间的 stop 监视任务 |
| §9.4 | 删除 PendingTool → `RecoveryBlocked`；有 resolver 时恢复等待，缺少能力时才返回 RecoveryBlocked |
| §10 验证矩阵 | 新增四个实验：① 串行等待：job Pending，等待中 kill -9，恢复后继续等，终态回填到同一 Turn。② 并行等待：background job，Turn 关闭后 job 完成，job 事件唤醒 active session；期间 Runner 被杀，job 不受影响。③ 长 exec 中途 stop：Cancelled、快照配对、不重跑。④ 同步 exec 中途崩溃：unresolved 说明“已被停止”，不重放 |
| §11 C8 / C12 | C12 的“`allow_deferred` 可开”改为依赖本文 §4 的 resolver 与等待循环；C8 的 Sandbox 包含 job 所有权；job 子系统新增一项或并入 C8 |

## 9. 待 review 决定

1. deferred 截止时间到期时回填 `Cancelled` 还是 `Error{timeout}`。
2. 子 run 暂停 / 中断时返回 Pending 的工具（`llm_explore`、`llm_understand_media`），改为 Error 加接手提示，还是作为 `wait_for_task` 类等待。
3. 派发时宿主未开 deferred 的降级，是由每个工具声明（阻塞或返回 Success），还是统一一种规则。
4. xllm 是否也在 run 结束时停止 run-owned 的后台进程，与 libopendan 保持一致。
5. 后台执行的接口形态：`exec` 加参数，还是独立的 `job_*` 工具；`wait.source.kind` / `EventSource` 的词汇表（`job` 是否独立于 `task`）。
6. 无输入队列的模板（work session）里的并行 job：由 Runner 内置 bridge 把完成事件写入 `pending_events`，还是这类模板只允许串行等待。
7. 挂起期间收到 stop。建议规则：调用 `resolver.cancel`，回填 Cancelled，run 以 Stopped 结束；如果结果已经回填并提交，先完成回填再处理 stop。另需确定审批票据在 stop 时是否同时作废。

## 10. 实施顺序与验证

命令在 `src/` 下执行。

1. §3（P0）：立即开工，不依赖 lib_opendan 的输入协议，可与 lib_opendan TODO §8 第 1、2 步并行。验证 `cargo test -p llm_context`、`cargo test -p agent_tool --lib`。
2. §4（P1）：等待记录、快照升版、resolver、接手检查、xllm 等待循环与降级规则；先定 §9 第 1–3 项。
3. §5（P1）：job 子系统（native），与 Runtime TODO 的 Sandbox 协调先后；同步改 exec 工具说明。先定 §9 第 4–6 项。
4. §6（P2）：恢复信息补全。
5. 之后才进入 lib_opendan TODO §6 的宿主接入（`cargo test -p libopendan -- --test-threads=1`）；review 通过后修改 xAgent.md，同步《LLM Context 设计》、xllm Rust SDK、Session Directory Protocol 和 fixtures。

参考入口：`llm_context/src/{context_loop,suspension,observation,request,interrupt}.rs`，`agent_tool/src/{llm_bash,exec_tracking,xllm}.rs`；[LLM Context 设计](<../doc/llm_context/LLM Context 设计.md>) §8–§9；[长任务 RFC](<../doc/opendan/OpenDAN Long Task & Sub-Agent.md>) §5–§7。
