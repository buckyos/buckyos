# Agent Session

本文描述当前 OpenDAN Runtime 中 `AgentSession` 的职责、持久化模型和运行语义。

设计来源主要是 [NewOpenDANRuntime.md](../../notepads/NewOpenDANRuntime.md) 以及
[AgentSession状态管理补充.md](../../notepads/AgentSession状态管理补充.md);实现以
[agent_session.rs](../../src/frame/opendan/src/agent_session.rs)、
[session_model.rs](../../src/frame/opendan/src/session_model.rs)、
[agent_config.rs](../../src/frame/opendan/src/agent_config.rs)、
[agent.rs](../../src/frame/opendan/src/agent.rs) 为准。

事件订阅模式与等待语义有单独文档:[Agent Session 的事件订阅](./Agent%20Session的事件订阅.md)。
Behavior 模板能读到的环境变量契约见 [Agent Environment](./Agent%20Enviroment.md)。
agent.toml / behaviors 的最新 schema 见 [Agent 配置改进](./Agent配置改进.md)。

## 1. 定位

`AgentSession` 是 OpenDAN Runtime 的状态管理核心。新的 Runtime 不再在 opendan 里重复实现
LLM 推理循环、tool dispatch、step 记录和 resume 逻辑,而是:

- opendan 负责构造 `LLMContextRequest` 与 `LLMContextDeps`
- 调用 `LLMContext::run()` / `LLMContext::resume()`
- 消化 `LLMContextOutcome`
- 把 session 级状态、输入队列、workspace 绑定、行为指针、快照和订阅持久化

`LLMContext` 是推理 waist;`AgentSession` 是 waist 之上的 L3/L4 调度器和持久化层。

核心不变量:

1. 同一个 session 任意时刻只有一个 worker task 进入 LLM 推理。
2. 已从系统取走但还没被 LLM 消费的输入必须先落到 `SessionMeta.pending_inputs`。
3. msg-center 的 ack 只允许发生在 `pending_inputs` 落盘成功之后。
4. worker 只有在一次 turn 成功后才删除本轮消费的 pending input;失败时保留,供重启或人工重试。
5. **Session 状态唯一真相源 = 磁盘**。worker 是 ephemeral 的内存态机 + 缓存,
   按需 mount / unload(见 §7、§13)。

## 2. Session 类型

当前实现的四类 session(由 `SessionKind` enum 表示):

| 类型 (kind) | 默认 class 名 | 语义 | 创建方式 |
| --- | --- | --- | --- |
| `Ui` | `ui` / `group` | 与 UI tunnel / peer / group 对应的长期会话,负责收用户消息、回送 assistant 文本,并可创建或转发到 Work Session | dispatcher 按 `session_id_strategy = per_peer / per_group` 派生,首次到达即创建 |
| `Work` | `work` | 内部工作会话,绑定 workspace,承载一个具体 objective | UI session 通过 `try_create_worksession` / `create_worksession` 创建 |
| `SelfCheck` | `self_check` | 由 timer 事件驱动的周期性 / 精确触发自检 session(典型用途:提醒任务) | dispatcher 把 `timer.*` 事件路由到 `singleton` class |
| `SelfImprove` | `self_improve` | 周期性触发、依赖 agent global state / history 的自我改进 session,budget 用尽后等待下次触发 | 内部调度或 timer 触发 |

> SelfCheck / SelfImprove 本质都属于 Work Session 的特化形式,被同一套 worker 调度,
> 但它们的事件来源、生命周期、driver 配置不同于普通用户任务型 Work Session。
> agent.toml 通过 `[session.<class>].kind = "self_check" | "self_improve"` 显式声明,
> 默认 driver 由 [`default_self_check_driver` / `default_self_improve_driver`](../../src/frame/opendan/src/agent_config.rs)
> 提供。

Work Session 创建后会写入 `title`、`objective`、`workspace_id` 和可选的
`task_binding`，并在 worker 空闲且没有 pending input 时触发一次 bootstrap turn。这个首轮输入来自
`objective`,不是一条 `PendingInput::Msg`。

TaskMgr 绑定规则：

- 通过 `task_id` 创建 Work Session 时，`title` / `objective` / `workspace_id` 可以从 Task data 回填。
- 未指定 `task_id` 时，OpenDAN 会自动创建一个 `agent.delegate` Task 并写入 `task_binding`。
- WorkSession 状态变化会同步到 Task；TaskMgr 侧将 Task 置为 `Paused` / `Canceled` 时，会反向中断对应 WorkSession。

TODO:

- Work Session 设计中的 `report.md` 完成报告还没有形成稳定写入路径;当前结果主要依赖 snapshot、
  round history、worklog 和 session 状态。
- Session 的归档 / GC / `SLEEP` 生命周期还没有在 `SessionStatus` 中落地。

## 3. Session 状态管理视角

四类 session 的差异本质不在"谁创建",而在它如何等待事件、消费事件、转移状态、何时结束。
判别 session 行为可以从以下问题拆解:

- 它是否长期存在(`keep_alive`)?
- 它是否有明确的 objective?
- 它是否会结束到 `Ended`?
- 它等待什么事件?(用户输入 / timer / global state)
- 它是否消费 UI Session forward 过来的用户消息?

四类 session 在事件消费 / 生命周期上的对照:

| 状态 / Session | UI Session | 普通 Work Session | SelfCheck Session | SelfImprove Session |
| --- | --- | --- | --- | --- |
| 接收用户输入 | 总是接收 | 通过 UI forward 或 dispatcher 路由接收 | 不接收 | 不接收 |
| WaitingInput 语义 | 常态开放等待 | 显式等待用户补充 | 不适用 | 不适用 |
| Running | 路由与协调 | 执行 objective | 检查 timer reason 对应事件 | 读取 history 并生成改进任务 |
| 主要事件来源 | 用户输入 | 用户输入、系统事件、tool 结果 | timer | timer 或内部触发 |
| 是否有明确 objective | 通常无单一短期 objective | 是 | 是(每次检查) | 是(每次改进) |
| Ended 触发 | 原则上不主动 Ended | objective 完成后 Ended | 单次检查结束,等待下次 timer | budget 用尽 / 任务完成,等待下次触发 |
| 允许 forwardMessage | 作为发送方 | 作为接收方 | 否 | 否 |
| 是否关心 global state | 间接关心 | 视任务而定 | 用于条件判断 | 核心依赖 |
| `keep_alive` 默认 | true | false | false | false |

### 3.1 UI Session

UI Session 是用户与 agent 系统交互的入口,关键特征:

- 永远处于可激活状态,**不在等待输入这件事上设置额外栅栏**;
- 用户消息首先进入 UI Session,再由 UI Session 决定是否 forward 给某个 Work Session;
- 路由策略(`session_id_strategy`)决定 session_id 形态:`per_peer`(每对话方一个) 或
  `per_group`(每个群一个)。

UI Session 承担消息路由职责:

- 当某个 Work Session 显式处于 `WaitingInput` 时,UI Session 的新用户消息可以 forward 给该 Work Session;
- 当 Work Session 尚未到达 `Ended` 但仍在工作时,新消息可以 `forwardMessage` 追加;
- 当 Work Session 已经 `Ended` 后,默认不再接收新的用户消息;**Agent main loop 在写 pending 前会先读
  目标 session.json,如果 `status == Ended` 则直接在 dispatcher 层拒绝写入,而不是依赖
  `AgentSession::enqueue_pending` 内部判断**(见 §7.3 END 后的 pending 写入拒绝)。

> 用户想追问已结束任务时,应由 UI / dispatcher 层创建新的 Work Session 并显式读取旧 session 历史作为
> 上下文,而不是把新消息写回旧 session。

### 3.2 普通 Work Session

面向明确 objective 的短期工作单元,围绕 objective 工作并最终进入 `Ended`。在需要用户信息或确认时进入
`WaitingInput`。`pending_inputs` 中合法的输入只在 UI / dispatcher 决定与本 Work Session 相关后才被写入。

### 3.3 SelfCheck Session

特殊 Work Session,**只消费 timer 事件**,不关心用户输入,不接收 UI Session 的 message forward。
核心用途:周期性或精确时间点的自检逻辑(提醒任务检查、scheduled task 检查)。

Timer 分为两层:

1. **硬栅栏 timer (`timer.hard_barrier`)**:固定频率(当前 60s,见
   `SELF_CHECK_HARD_BARRIER_INTERVAL_MS`)的兜底检查,启动时由
   `AIAgent::ensure_self_check_hard_barrier_timer` 注册,保证不会因为精确触发漏失而错过提醒。
2. **精确 trigger timer (`timer.reminder_check` / `timer.scheduled_task_check`)**:SelfCheck 在
   运行过程中根据具体提醒任务推断的精确触发时间,通过
   [`AIAgent::schedule_precise_timer`](../../src/frame/opendan/src/agent.rs) 创建。

精确 timer 必须带 `TimerReason`,schema 见 [`session_model::TimerReason`](../../src/frame/opendan/src/session_model.rs):

```json
{
  "trigger_type": "precise_trigger | hard_barrier",
  "target_type": "reminder | scheduled_task | other | <named>",
  "target_id": "string",
  "expected_trigger_time": "datetime",
  "reason": "string"
}
```

SelfCheck 被 timer 唤醒后**不应盲目扫描所有任务**,而应优先根据 `reason` 做定向检查;若需要发出提醒,
则调用 `send_message` action;若需要继续延后,可再 schedule 下一个 precise timer。Behavior 模板见
`src/rootfs/bin/buckyos_jarvis/behaviors/self_check.toml`。

事件分发上,SelfCheck 默认 driver 在每个 hook point 上把 `pull_event` 设置为 `timer.*` 过滤,
对应到 [Agent Environment §5](./Agent%20Enviroment.md) 的派生变量
`input.timer_events` / `input.reminder_events` / `input.hard_barrier_events` /
`input.scheduled_task_events`。

### 3.4 SelfImprove Session

特殊 Work Session,**不关心用户输入和外部事件本身**,只关心 agent 的 global state 与 history。
触发后流程为:

```text
trigger
    -> read history + global_state
    -> analyze possible improvements
    -> generate improvement tasks
    -> dispatch improvement tasks
```

SelfImprove 有 budget 限制([`ImprovementBudget`](../../src/frame/opendan/src/session_model.rs)),
当前 budget 单位为 `Token`。budget 不足时:

- 当前 SelfImprove 运行暂停或结束(`AgentSession::mark_improvement_budget_exhausted`);
- 未完成的改进任务持久化到 `SessionMeta.pending_improvement_tasks`
  ([`ImprovementTask { task_id, summary, source_report, created_at_ms, status }`](../../src/frame/opendan/src/session_model.rs));
- 下次触发时,global state 可能已经变化,需要重新读取与判断。

SelfImprove 的默认 driver 全程 `pull_msg = none`, `pull_event = none`——它不消费 pending queue 中的
外部输入。history 和 global state 通过环境变量自动注入(由
[`AIAgent::snapshot_global_state`](../../src/frame/opendan/src/agent.rs) 和
[`AgentSession::apply_hook`](../../src/frame/opendan/src/agent_session.rs) 协作)。
最小落地:改进任务 dispatch 写 `improvement_tasks.jsonl` 并同步更新
`SessionMeta.pending_improvement_tasks`;Behavior 模板见
`src/rootfs/bin/buckyos_jarvis/behaviors/self_improve.toml`。

## 4. 持久化目录

Session 数据位于:

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
  improvement_tasks.jsonl       # SelfImprove only
```

关键文件:

- `.meta/session.json`:`SessionMeta`,是 session 级真相源。
- `.meta/state.snap`:当前栈顶 process 的 `LLMContextSnapshot`。
- `.meta/behavior_<name>.snap`:`switch_mode = "independent"` 时,挂起 process 的独立快照。
- `round_history/`:按 round 追加的审计历史,记录输入、step、outcome、压缩、interrupt 等事件。
- `tools/`:session 级工具声明和素材,不是进入 `PATH` 的执行视图。
- `improvement_tasks.jsonl`:SelfImprove 生成的改进任务流水(append-only),与
  `SessionMeta.pending_improvement_tasks` 保持同步。

禁止在 session 目录里放 `bin/`。进入 `PATH` 的 Session Exec Bin 由运行时渲染到
`<buckyos_root>/tools/<agent_id>/<session_id>/`,见 [Agent RootFS](./Agent%20RootFS.md)。

## 5. `SessionMeta`

当前 `SessionMeta` 字段以 [session_model.rs](../../src/frame/opendan/src/session_model.rs) 为准:

```rust
pub struct SessionMeta {
    pub session_id: String,
    pub kind: SessionKind,                          // Ui / Work / SelfCheck / SelfImprove
    pub current_behavior: String,
    pub status: SessionStatus,
    pub status_changed_at_ms: u64,
    pub owner: String,
    pub keep_alive: bool,
    pub one_line_status: String,
    pub pending_inputs: Vec<PendingInput>,
    pub peer_did: Option<String>,
    pub peer_tunnel_did: Option<String>,
    pub event_subscriptions: Vec<EventSubscription>,
    pub background_events: Vec<BgEventSnapshot>,
    pub workspace_id: Option<String>,
    pub pending_task_calls: Vec<PendingTaskCall>,
    pub improvement_budget: Option<ImprovementBudget>,
    pub pending_improvement_tasks: Vec<ImprovementTask>,
    pub title: String,                              // 与目录名相同
    pub objective: String,
    pub bootstrap_done: bool,
    pub process_entry: String,
    pub process_stack: Vec<ProcessFrame>,
    pub last_report_delivery: Option<ReportDeliveryState>,
}
```

`PendingInput` 当前有三类:

- `Msg { record_id, from, from_did, from_name, tunnel_did, text }`
- `Event { event_id, data }`
- `Interrupt { mode, id }`

`record_id` / `event_id` / `interrupt id` 组成稳定 dedup key。重复 Msg 与 Interrupt 会被折叠;
重复 Event 会按状态新旧进行 coalesce,终态事件优先保留(对应 [事件订阅 §5.3](./Agent%20Session的事件订阅.md)
Coalesce / Override 语义)。

`background_events`:半订阅 (`bg_events`) 当前快照,不进入 pending queue,在 hook point 上作为
环境块的一部分注入(见 [Agent Environment §3.4](./Agent%20Enviroment.md))。

`improvement_budget` / `pending_improvement_tasks`:仅 SelfImprove 使用(见 §3.4)。

`last_report_delivery`:Work Session 上行到 UI Session 的最近一次 report 投递元数据,
用于按 `report_delivery` 策略(`final_only` / `top_level` / `all`)幂等去重。

TODO:

- 旧设计里的 `new_msg/history_msg`、`new_event/history_event` 双缓冲没有作为独立字段实现。
  当前实现是 `pending_inputs` 持久队列 + `round_history` / snapshot 累积历史。
- 旧设计里的 MsgTunnle link 模型尚未完全替代正文存储;当前 `PendingInput::Msg` 仍直接保存文本,
  以保证 crash 后可重放。

## 6. 状态机

当前实现的 `SessionStatus`:

| 状态 | 含义 |
| --- | --- |
| `Idle` | worker 空闲,可以消费 pending input |
| `Running` | 正在执行一次 LLMContext run/resume |
| `WaitingInput` | 等下一条用户消息或普通事件 |
| `WaitingTool` | 已产生 `PendingTool`,正在等 task_mgr 任务终态 |
| `Ended` | session 结束;重启时不会 restore |
| `Error` | turn 失败,pending input 保留,等待外部唤醒或人工处理 |

设计文档中的 `PAUSE`、`WAIT`、`WAIT_FOR_MSG`、`WAIT_FOR_EVENT`、`READY`、`SLEEP` 在当前实现里被收敛为
上面的状态集合:

- `WAIT_USER_MSG` sentinel 会映射为 `WaitingInput`。
- PendingTool / 异步 task 会映射为 `WaitingTool`。
- 普通空闲态是 `Idle`,不是显式 `READY`。

TODO:

- 用户手工 `PAUSE` / `RESUME` 以及 parent session 暂停时级联暂停 sub session 尚未落地。
- 精确的 `WAIT_FOR_MSG` / `WAIT_FOR_EVENT` 过滤状态尚未作为状态机字段落地;事件等待语义见事件订阅文档。
- `SLEEP`、归档、复活策略还停留在生命周期设计里。

## 7. Agent 主循环与 Worker 唤醒

新架构下,**Agent.main_loop 是事件 / 消息的唯一 pump**,session.worker 是 ephemeral 的、按需从磁盘
加载的状态机 + 缓存。这是相对旧实现(session.worker 常驻、消息分发逻辑嵌在 session loop 内部)的
根本变化。

### 7.1 主循环结构

```python
def AIAgent.main_loop():
    while not shutdown:
        # message pump
        msg = self.pull_msg()
        target_session_id = route_msg(msg)
        # 先读 session meta,END session 直接拒绝(见 §7.3)
        target_session = self.ensure_session(target_session_id)
        target_session.push_msg(msg)

        # event pump
        event = self.pull_events()
        target_session_ids = route_event(event.event_id)
        for sid in target_session_ids:
            target_session = self.ensure_session(sid)
            # event 是 notify 机制;main loop 是尽力通知,不关心后续
            target_session.notify_event(event)
```

参考实现:
[`AIAgent::main_loop`](../../src/frame/opendan/src/agent.rs)、
[`AIAgent::dispatch_inbound`](../../src/frame/opendan/src/agent.rs)、
[`AIAgent::ensure_session`](../../src/frame/opendan/src/agent.rs)。

关键变化:

1. **dispatch 包含两步**:`route → ensure_session`(按需从磁盘加载并启动 worker)。
2. **session.worker 是 transient**:处理完一批工作可休眠/退出,释放内存。
3. **dispatch 只 push 不假设 worker 存在**:push msg = 写 pending queue + 唤醒 worker;
   不是 forward 给某个 running coroutine。
4. **Session 状态唯一真相源 = 磁盘**,worker 启停不丢任何状态。

### 7.2 `ensure_session` 的并发与生命周期

- **并发**:多个 event 同时分发给同一 `session_id` → per-session 锁,序列化处理。
- **退出时机**(候选策略):
  - 处理完当前 batch 即退出(最省内存,但每次唤醒有冷启动成本);
  - 空闲超时退出(兼顾热路径 + 长尾节省);
  - 显式 shutdown(如 session 走到 `Ended`)。
- **`keep_alive` 字段在新架构下的语义**:
  - 当前稳定阶段:所有 session worker 都允许在 idle 后自动卸载;下次有 event/msg 到达再
    `ensure_session`。
  - `keep_alive` 只作为未来热缓存 hint,不改变 session 的逻辑生命周期,也不能要求 Agent 启动时
    预加载 worker。
- **启动恢复策略**:
  - Agent 启动时只恢复**轻量路由索引**(UI tunnel 绑定、event subscription pattern),不启动
    所有非 `Ended` session worker(见 §13)。
  - `ensure_session` 只能在 main loop 已经决定要向某个 session 写入 pending input 后调用。
  - 空闲 worker 退出时只卸载内存态 worker,不删除磁盘 meta,也不删除仍然有效的 event subscription
    路由。

### 7.3 END 后的 pending 写入拒绝

`Ended` 是 session 逻辑生命周期的终态。Agent main loop 在写入 pending input 之前必须先读取目标
session 的持久化状态:

```text
route msg/event -> target_session_id
    -> read session meta status
    -> if status == Ended:
        reject pending write at dispatch layer
    -> else:
        ensure_session(target_session_id)
        enqueue_pending(...)
```

这个拒绝发生在 **Agent main loop**,而不是 `AgentSession::enqueue_pending` 内部。这样 main loop
可以用统一方式处理拒绝(记录、丢弃 stale event、回复用户或创建新 Work Session),同时 `AgentSession`
不需要理解"这条输入为什么被路由给我"的上层策略。

拒绝规则:

- 显式 target 指向 `Ended` session:拒绝写入,不自动 reopen。
- 隐式路由不应选择 `Ended` session 作为候选。
- 如果用户想追问已结束任务,应由 UI / dispatcher 路由层创建新的 Work Session,并显式读取旧 session
  历史作为上下文,而不是把新消息写回旧 session。

### 7.4 输入投递与 ack

消息进入 Session 的主路径:

```text
msg-center / local caller / event pump
  -> AIAgent::dispatch_inbound
  -> route + END-check
  -> ensure_session(target_session_id)
  -> AgentSession::enqueue_pending(input)
  -> flush_meta()
  -> Wakeup worker
  -> msg-center update_record_state(Readed)
```

`enqueue_pending` 的语义:

1. 计算 dedup key。
2. 写入或合并 `meta.pending_inputs`。
3. `flush_meta()` 用 tmp + rename 写 `.meta/session.json`。
4. 落盘成功后发送 `SessionInput::Wakeup`。
5. 返回 `Ok(())` 后,上游才可以 ack。

这保证:

- 落盘前进程崩溃:msg-center 记录仍未 `Readed`,下次启动可重新拉取。
- 落盘后进程崩溃:session 已持久拥有输入,重启后由 `restore_session_routes` + on-demand `ensure_session`
  重放。
- ack 失败:msg-center 可能再次投递,但 session 会按 `record_id` 去重。

### 7.5 Worker 消费模型

每个 active session 有一个 tokio worker。`SessionInput` 只是唤醒信号,真实载荷始终从
`meta.pending_inputs` 读取。

worker 每轮大致流程:

1. 优先处理 `Cancel`。
2. 克隆 `pending_inputs` 快照,不立即删除。
3. 处理 `Interrupt` barrier;必要时打断 in-flight LLMContext。
4. 按当前 hook point driver 配置 pull msg / event(见 §8)。
5. 若有 `pending_task_calls`,优先等待 / 收集 task 完成事件。
6. 调 `run_one_round()`。
7. **进入推理 = commit pop**:user message 一旦成功追加进即将运行的 LLMContext,本轮 pull 到的
   input 立即从 `pending_inputs` 删除(见 §8.5)。
8. 失败时保留 pending input,状态置为 `Error`,等待下一次外部唤醒。

如果 snapshot 中仍有 pending tool calls,但 meta 中没有对应的 `pending_task_calls`,实现会认为这是
PendingTool persist 与 task dispatch 之间崩溃造成的孤儿挂起态,丢弃 snapshot 并记录 history 事件。

## 8. Driver 配置:把 Session 类型差异显式化

§2-§3 描述的 4 类 Session 本质是同一个 Session 抽象的不同 specialization——它们的差异
("等什么、消费什么、何时触发推理、何时 Ended")被统一抽象成一组 driver 配置,作为
`[session.<class>].driver` 的一等配置项。

> Worker 启停是 §7 的事(决定"worker 何时在跑");Driver 配置是本节的事(决定"worker 跑起来后
> 如何消费 pending queue")。两层正交。

### 8.1 Hook Point × Filter × Pull Policy

Session 在物理状态机上有 4 个 hook point,每个 hook 是一次"渲染 user_message 并启动新一轮推理"的
窗口。Hook point 数量有物理上限,不会膨胀。

| Hook Point | 触发时机 |
| --- | --- |
| `on_init` | session / context 启动时一次性触发,渲染初始 system prompt |
| `on_behavior_switch` | 每次 behavior switch / fork / independent 切换时触发,渲染新 behavior 的入口 user_message |
| `on_behavior_step_ob` | Behavior Loop 内每个 step 边界(观察阶段)触发 |
| `on_wakeup` | session 处于 idle / waiting 状态、新 pending input 到达时触发 |

每个 hook point 上挂三个配置:

| 配置 | 含义 | 取值集合 |
| --- | --- | --- |
| `filter` | 哪些 behavior 启用本 hook | `top` / `default_only` / `all` / `none` / `<behavior_name>` |
| `pull_msg` | 从 pending message queue 拉取的策略 | `none` / `one` / `all` |
| `pull_event` | 从 pending event queue 拉取的策略 | `none` / `<filter_name>` / `all` |

`filter` 前 4 个是固定 enum,`<behavior_name>` 是闭集合标识符(startup 时对照 session 的 behavior
列表 validate);`<filter_name>` 当前命名空间约束在 `timer.*`(由
[`TimerEventKind`](../../src/frame/opendan/src/session_model.rs) 列举,
[`validate_driver_filters`](../../src/frame/opendan/src/agent_config.rs) 在 startup 时校验)。

参考实现:
[`SessionDriverCfg`](../../src/frame/opendan/src/agent_config.rs)、
[`HookPointCfg`](../../src/frame/opendan/src/agent_config.rs)、
[`BehaviorFilter`](../../src/frame/opendan/src/agent_config.rs)、
[`PullMsgPolicy`](../../src/frame/opendan/src/agent_config.rs)、
[`PullEventPolicy`](../../src/frame/opendan/src/agent_config.rs)、
[`AgentSession::apply_hook`](../../src/frame/opendan/src/agent_session.rs)。

### 8.2 为什么驱动力在 Session 层而不是 Behavior 层

1. **同一 behavior 在不同 session 下驱动力不同**:`chat_route` 在 UI session 是 `per_peer`,在 group
   session 是 `per_group`。绑死在 behavior 上无法跨 session 复用。
2. **同一 session 多个 behavior 通常共享驱动策略**:每个 behavior 重复声明是 DRY 违反。
3. **Behavior 配置只剩纯渲染**:心智模型干净,可独立测试。

这也是 [Agent 配置改进 §4](./Agent配置改进.md) 把 `switch_mode` 上提到 session class 的同源理由。

### 8.3 Driver / Behavior 契约:固定 env schema + 模板幂等

Driver 和 Behavior 在新架构下的契约非常薄:

1. **Driver 负责**:按 hook point 配置 pull + 构造 `llm_context_env`;
2. **Behavior 模板负责**:从 `llm_context_env` 读取所需字段,渲染 user_message;
3. **`llm_context_env` schema 固定**(由框架定义,见 [Agent Environment §3](./Agent%20Enviroment.md)),
   不是 per-behavior declared。

没有 binding name 校验、没有 binding usage tracking、没有"必须引用某 binding"的硬契约——模板自由
读取固定 schema 即可,未读取的字段直接忽略。

> **模板引擎必须是幂等的**:同一个 `llm_context_env` 渲染 N 次结果必须完全一致。**模板不幂等就是 bug**,
> 没有例外。直接代价:模板不能产生副作用、不能依赖随机/时间/外部状态;时间 / 半订阅事件快照 /
> pending tool result 等所有外部状态由 Driver 在构造 env 时 freeze 进去。

### 8.4 四类 Session 在 Hook Point 视角下的默认配置

`agent.toml` 中可显式重写;以下是当前 `default_*_driver` 给出的默认值:

| Session | on_init | on_behavior_switch | on_behavior_step_ob | on_wakeup |
| --- | --- | --- | --- | --- |
| UI | `filter=all, pull_msg=none, pull_event=none` | `filter=top, pull_msg=all` | — | `filter=top, pull_msg=all` |
| 普通 Work | 同上 | `filter=top, pull_msg=all, pull_event=all` | `filter=top, pull_msg=all, pull_event=all` | `filter=top, pull_msg=one` |
| SelfCheck | `filter=all, pull_event=timer.*` | `filter=top, pull_event=timer.*` | — | — |
| SelfImprove | `filter=all, pull_msg=none, pull_event=none` | `filter=top, pull_msg=none, pull_event=none` | — | — |

观察:

- **SelfImprove 全程 `pull_msg=none / pull_event=none`**——它"不消费外部输入"的具体表达。
  history 和 global_state 通过 env 自动注入(不在 pull 范畴内)。
- **SelfCheck 的事件路由**(按 `timer.reason` 区分提醒类型)通过 `pull_event` 的 filter 名称命名空间
  参数化。
- **UI Session 的 `on_wakeup` 是核心**——它常态在 idle + 监听 pending input;`on_behavior_switch` 在
  UI 内基本对应路由判断。
- **普通 Work Session 的 `on_behavior_step_ob` 是 Behavior Loop 独有**——step 边界拉新 message 是允许
  在 step 中段感知用户追加输入的关键。

`inject_background_environment`(driver 字段)控制本 hook 是否在 user message 前渲染
`<background_environment>` 块(模板见 [Agent Environment §6](./Agent%20Enviroment.md))。UI session
默认开,Work / SelfCheck / SelfImprove 默认关。

### 8.5 关键 invariant:进入推理 = commit pop

> **一旦 render 成功且 user_msg 喂给 `llm_context.drive_to_end()`,本次 pull 拿到的 input 立即从 pending
> queue 消失**。Commit pop 不依赖推理成功 / 失败,不依赖 tool 调用结果,不依赖 `drive_to_end` 返回值。

实现上,commit-pop 的边界是:

1. Driver 在当前 hook point 按 `pull_msg` / `pull_event` 从 `pending_inputs` 选择本轮输入。
2. Runtime 使用这些输入构造固定 `llm_context_env`,并渲染本轮 user message。
3. 只要 user message 已经成功追加进即将运行的 `LLMContext`,本轮 pull 到的 input 就从
   `.meta/session.json.pending_inputs` 删除。

未被当前 hook point pull 到的 input 必须继续留在 pending queue;它们不属于本轮推理,不能因为同一批
pending 中有其它 input 被消费而被误删。

当前阶段只实现 **pull** 语义;未来如果引入 peek,peek 到的 input 只能进入 env / background context,
不能加入本轮 commit-pop 集合。

直接推论:

- **推理失败 → input 丢失**,by design,框架**不**自动重放;
- **Crash recovery 不允许自动重新触发未完成的推理**——上次崩在哪一步无法精确还原,重放等于把 tool
  调用风险翻倍;
- **重试只能由上层显式做**——用户重发消息 / scheduler 重新 push event;**永远不能是框架隐式重放**。

为什么不选 peek + 后置 commit:

| 风险 | 代价 | 可恢复性 |
| --- | --- | --- |
| Input 丢失(推理挂了 msg 没了) | 中 | ✅ 用户 / scheduler 可重新 push |
| Tool 调用副作用重复(rollback 后 msg 回 queue → 下次推理重新触发同一 tool) | **灾难** | ❌ 账户扣两次 / 邮件发两次 / 文件改两次——不可逆 |

推理过程中的 tool 调用本身就不幂等,重放比丢消息危险得多。早 commit pop 是用"可恢复的小风险"换
"不可恢复的大风险"——这是有意识的设计抉择。

## 9. 构造与恢复 LLMContext

`AgentSession` 在 `build_or_resume` 中完成 `LLMContext` 的构造:

- 加载当前 `BehaviorCfg`
- 组装 `LLMContextDeps`
- 按当前 hook point 配置渲染 system / user message(见 §8)
- 优先加载 `.meta/state.snap`
- 根据 snapshot 状态选择 fresh run 或 resume

当前 resume 规则:

- snapshot 没有 `pending_tool_calls` 且有新用户输入:使用 snapshot 的 `state.accumulated` 作为历史,
  追加新 user message 后创建新的 `LLMContext`。
- snapshot 没有 `pending_tool_calls` 且没有新输入:`ResumeFill::ResumeFromMidRun`。
- snapshot 有 `pending_tool_calls`:正常路径由 `resume_with_tool_results` 使用
  `ResumeFill::ToolResults`;若没有 meta 侧 task 句柄则丢弃孤儿 snapshot。

环境块默认包含:behavior name、session id / title、workspace id、recent activity、`unix_ms` 时钟,
以及 `input.bg_events` 半订阅事件快照——具体模板可在 `[session.<class>].driver.<hook>` 中通过
`inject_background_environment` 关闭。

TODO:

- 设计中的 auto-recall memory、event diff 等弱订阅环境变量尚未接入环境消息。
- `HistoryCompressor` trait 作为 waist 可选项存在于设计中;当前主要实现是 opendan 侧对
  `ContextLimitReached` 的 message-level 压缩和 resume。

## 10. Behavior 切换

Behavior 配置来自 `<agent_root>/behaviors/<name>.toml`,由
[behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs) 翻译成 `LLMContextRequest` 和 deps:

- `loop_mode = "behavior"`:装配 `XmlBehaviorParser` + `XmlStepRenderer`
- `loop_mode = "agent"`:不装 parser/renderer,走普通 agent loop
- `[capabilities].tool_whitelist` 控制 `ToolPolicy`
- `[capabilities].tool_plan` 控制 Session Exec Bin 的 tombstone 策略
- `[session.<class>].driver.switch_mode` 控制切换语义(由 session class 决定,不由 behavior 决定)

当前 `next_behavior` 处理:

- `END`:结束当前 independent process;如果没有 parent process,则结束 session。
- `WAIT_USER_MSG`:持久化最终 snapshot,session 进入 `WaitingInput`。
- 其他 behavior 名称:执行 `switch_behavior`。

`switch_mode` 当前状态:

| 模式 | 当前实现 |
| --- | --- |
| `normal` | 已实现。保留 accumulated history 和 steps,替换 system / policy / model / budget 等 request 字段。 |
| `independent` | 已实现。每个 behavior entry 有独立 snapshot,`process_stack` 负责父子 process 栈。 |
| `fork` | 已实现并作为 jarvis Work Session 的默认 switch mode。`try_create_worksession` 等工具仍然走 fork 原语来 spawn 子 session。 |

TODO:

- independent process 内发生 normal switch 后,再回到 entry system prompt 的语义仍有待真实用例确认。
- behavior 切换时 tool plan / SessionBinRenderer 目前不会重新计算;等 behavior 间 tool plan 差异成为
  真实需求后补。

## 11. PendingTool、Interrupt 与长任务

`LLMContextOutcome::PendingTool` 表示 waist 让出控制权,等待外部 task 结束。Session 的处理方式:

1. 持久化包含 `pending_tool_calls` 的 snapshot。
2. 通过 `TaskDispatch` 创建 task_mgr 任务。
3. 写入 `SessionMeta.pending_task_calls`。
4. 订阅 `/task_mgr/<task_id>`。
5. 状态进入 `WaitingTool`。
6. task 终态事件回来后转成 `Observation`。
7. 收齐所有 pending call 后使用 `ResumeFill::ToolResults` 恢复 LLMContext。
8. 成功后清理 `pending_task_calls` 并取消订阅。

`Interrupt` 是 pending 队列里的 barrier:

- `Graceful`:给未完成 tool calls 注入 `Observation::Cancelled`,让 LLMContext 走到终态。
- `Discard`:尝试通过 `LLMContextInterruptHandle` 立即中断推理,并截断持有未完成 `tool_use` 的
  assistant turn。

## 12. Workspace 绑定

Session 与 workspace 的绑定以 `SessionMeta.workspace_id` 为真相源。

`WorkspaceRecord.current_session` 只是冲突检测 hint。重启时 `restore_session_routes` 只恢复
路由索引;真正写入 pending input 时由 `ensure_session` 通过 `AgentSessionBuild::existing_meta` 恢复
`workspace_id`,然后重新建立运行期 session 和 workspace 的关联。

TODO:

- 同一个 local workspace 同时只能有一个 session `Running` 的强约束尚未作为统一锁实现。
  当前有 workspace 记录与 `current_session` hint,但还不是完整调度锁。

## 13. 输出回送与 Report Delivery

UI Session 在 `Outcome::Done` 或可返回 partial 的 budget outcome 中,会把 assistant text:

1. 发送到本地 `SessionReply::AssistantText`,用于 CLI / 日志。
2. 如果 runtime 有 `msg_center`、session 有 `peer_did`,则用 agent DID 作为 sender 调
   `msg_center.post_send` 回送给 peer。

Work Session 不主动通过 msg-center 回送结果,而是按 `[session.<class>].driver.report_delivery`
策略投递 `worksession_report` 事件给上行 UI Session(对应 [Agent Environment §5](./Agent%20Enviroment.md)
`input.worksession_reports`):

| 模式 | 含义 |
| --- | --- |
| `final_only` | 只在 final report 时投递(默认) |
| `top_level` | 顶层 Work Session 的 checkpoint + final 都投递 |
| `all` | 所有层级的 checkpoint + final 都投递 |

`SessionMeta.last_report_delivery` 记录最近一次投递的 `report_id` / `report_hash` / `phase` /
`delivered_at_ms`,用于幂等去重。

## 14. 恢复语义

`AIAgent::run()` 启动时调用
[`restore_session_routes()`](../../src/frame/opendan/src/agent.rs)——**只恢复轻量路由索引,不预启动
worker**:

- 扫描 `<agent_root>/sessions/*/.meta/session.json`
- 跳过 `status == Ended`
- 对 `kind == Ui` 且有 `owner` 的 session,把 `owner → session_id` 灌入 `tunnel_to_ui_session`
- 把 `event_subscriptions` 推回 `SessionEventPump`

Worker 真正被创建是在 main loop 第一次需要向某个 session 写 pending input 时,由
[`ensure_session`](../../src/frame/opendan/src/agent.rs) 完成:从磁盘读 `.meta/session.json`,
通过 `AgentSessionBuild { existing_meta }` 重建 session、worker、tool plan,并立即消费遗留
`pending_inputs`(`crash recovery` 不重放未完成推理,只把 pending 重新喂回 driver,见 §8.5)。

恢复范围(运行期落地)包括:

- `pending_inputs`
- peer 路由信息
- workspace 绑定
- event subscriptions
- `pending_task_calls`
- `process_entry` / `process_stack`
- independent process snapshot 文件
- `improvement_budget` / `pending_improvement_tasks`(SelfImprove)

## 15. 与旧设计的差异

旧版 `Agent Session.md` 以 `generate_input()` / `update_input_used()` 为中心。新 runtime 后,这两个
概念已经分散到:

- `pending_inputs`:持久输入队列
- Driver hook point 的 pull policy:选择本轮真实输入
- `build_or_resume`:把输入合成为 `LLMContextRequest.input` 或 resume fill
- `LLMContext`:负责 step loop、tool dispatch、step record 与 snapshot
- 进入推理 = commit pop:turn 提交时删除已消费 pending input(§8.5)
- `round_history` / snapshot:保存可回放历史

因此当前文档不再建议新增独立的 `generate_input()` API。若后续要恢复模板驱动的输入判空,应先确认它属于
Behavior prompt 编译层,还是 Session worker 的队列消费层。

session.worker 也从"常驻 + 内嵌分发"变成"按需 mount + main loop 分发":

- Agent.main_loop 是唯一的事件 / 消息 pump(§7.1);
- `ensure_session` 按需从磁盘恢复 session(§7.2);
- `restore_session_routes` 启动期只恢复路由索引,不启动所有非 Ended worker(§14);
- session 类型的差异 = (worker 生命周期策略) × (driver 配置)。

TODO:

- "零 LLM 空转"目前主要靠 `pending_inputs.is_empty()` 时 worker 阻塞、Work Session bootstrap 只执行一次、
  以及 `WAIT_USER_MSG` 停车实现;设计中的"模板替换后全 Null 则跳过推理"尚未作为统一函数实现。
- Todo/PDCA 状态补丁、Session Summary 深度更新、Session GC、MsgTunnle link 投影等仍属于后续模块化工作。

## 16. 设计约束与待明确点

为避免实现期出现状态管理歧义,以下约束需要在 schema / 实现层显式说明:

### 16.1 forwardMessage 路由策略

属于 dispatcher 层 driver(§7.1),不属于单个 session 的 driver:

- 一个用户消息最多 forward 给几个 Work Session?(当前实现:1)
- 多个 Work Session 都处于 `WaitingInput` 时如何选择?(当前实现:由 UI session 路由 behavior 决定,
  无系统兜底)
- 用户打断当前任务时,是 forward 到原 Work Session,还是创建新 Work Session?(当前实现:LLM 决定)

### 16.2 Work Session 的 END 语义

- `Ended` 后**不允许 reopen**(§7.3);
- 用户在 `Ended` 后追问同一任务 → 由 UI / dispatcher 创建新 Work Session,显式引用旧 session 历史;
- `Ended` 后仍保留 routing metadata 与 session 目录,只是从 active 集合移除。

### 16.3 SelfCheck 的 TimerReason schema

见 §3.3,标准化字段:`trigger_type` / `target_type` / `target_id` / `expected_trigger_time` /
`reason`。实现在
[`session_model::TimerReason`](../../src/frame/opendan/src/session_model.rs),测试在
`session_model::tests::timer_reason_round_trips_fixed_schema`。

### 16.4 SelfImprove 的 budget 与任务续跑

见 §3.4。当前最小落地:

- budget 单位:`Token`(`ImprovementBudgetUnit::Token`);
- budget 用尽 → `AgentSession::mark_improvement_budget_exhausted` 标记并暂停;
- 未完成任务持久化在 `SessionMeta.pending_improvement_tasks` + `improvement_tasks.jsonl`;
- 下次触发时,framework 不自动续跑;由 Behavior 模板基于最新 history / global state 决策是否复用旧
  task。

---

## 附录:实现映射(Phase 3 - Phase 5)

### A.1 SelfCheck Session

- §3.3 hard barrier timer / precise trigger timer:
  - `src/frame/opendan/src/agent.rs::AIAgent::ensure_self_check_hard_barrier_timer`
  - `src/frame/opendan/src/agent.rs::AIAgent::schedule_precise_timer`
  - `src/frame/opendan/src/session_model.rs::{TimerReason, TimerTriggerType, TimerTargetType, TimerEventKind}`
- §3.3 TimerReason schema:
  - `src/frame/opendan/src/session_model.rs::TimerReason`
  - 测试:`session_model::tests::timer_reason_round_trips_fixed_schema`
- §3.3 reminder trigger path:
  - `src/frame/opendan/src/agent_session.rs::AgentSession::dispatch_behavior_send_messages`
  - `src/frame/opendan/src/agent_session.rs::AgentSession::post_send_message_record`
  - 行为模板:`src/rootfs/bin/buckyos_jarvis/behaviors/self_check.toml`
- §8.1 timer pull_event filter 命名空间:
  - `src/frame/opendan/src/session_model.rs::TimerEventKind`
  - `src/frame/opendan/src/agent_config.rs::validate_driver_filters`
- §8.4 SelfCheck driver 默认配置:
  - `src/frame/opendan/src/agent_config.rs::default_self_check_driver`
  - 模板:`src/rootfs/bin/buckyos_jarvis/agent.toml [session.self_check]`

### A.2 SelfImprove Session

- §3.4 budget 状态机:
  - `src/frame/opendan/src/session_model.rs::{ImprovementBudget, ImprovementBudgetUnit, ImprovementTask, ImprovementTaskStatus}`
  - `SessionMeta.improvement_budget` / `SessionMeta.pending_improvement_tasks`
  - `src/frame/opendan/src/agent_session.rs::AgentSession::mark_improvement_budget_exhausted`
- §8.3 / §8.4 history + global_state 注入:
  - `src/frame/opendan/src/prompt_env.rs::LlmContextEnv`
  - `src/frame/opendan/src/agent.rs::AIAgent::snapshot_global_state`
  - `src/frame/opendan/src/agent_session.rs::AgentSession::apply_hook`
- §3.4 改进任务 dispatch:
  - `src/frame/opendan/src/agent_session.rs::dispatch_self_improvement_tasks`
  - 最小落地:写 `improvement_tasks.jsonl`,并同步更新 `SessionMeta.pending_improvement_tasks`
  - 行为模板:`src/rootfs/bin/buckyos_jarvis/behaviors/self_improve.toml`
- §8.4 SelfImprove driver 默认配置:
  - `src/frame/opendan/src/agent_config.rs::default_self_improve_driver`
  - 模板:`src/rootfs/bin/buckyos_jarvis/agent.toml [session.self_improve]`

### A.3 Driver 配置与回归

- Driver 配置与 baseline 组合测试:
  - `src/frame/opendan/src/agent_config.rs::tests::{defaults_when_no_toml, driver_hook_point_round_trips, rejects_unknown_timer_driver_filter, jarvis_work_session_uses_fork_switch_mode}`
- `/timer/wake` 测试已迁移到新 TimerReason schema:
  - `src/frame/opendan/src/agent_session_test.rs::{format_event_for_turn_includes_id_and_data, session_meta_round_trips_pending_inputs}`
- 启动恢复轻量化:
  - `src/frame/opendan/src/agent.rs::tests::restore_session_routes_does_not_mount_workers`
- 运行验证:
  - `cargo test -p opendan`
