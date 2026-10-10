# Improve Behavior Report

2026-10-10 设计更新：XML 显式结束统一为 `<report end="true">`，该决策不得有 actions；缺省 / false 只更新报告，`next_behavior` 保留切换 / 等待语义，不再用 END 结束。此规则已定稿但尚未实现，详见 [Context 调度支持 TODO H4.1](../../notepads/llm-context-switch-support-todo.md) 与 [Agent Actions](<Agent Actions.md>)。下文“当前实现事实”保留实施前行为，不是新协议规范。

## 背景

本记录基于一次语音讨论和当前实现整理。其中 `opendan` Session 层的现状、处理流程和 MVP 描述的是旧 opendan Runtime（`src/frame/opendan`），待下一阶段 opendan 按 libopendan 的抽象重构接入；libopendan 已有的对应行为见“libopendan 当前实现”一节。Round / Step / Turn 的定义见 [readme.md](readme.md)。

核心结论：Behavior 模式里的 `<report>` 语义不是"发一条消息给用户"，而是"向上级 report"。谁是上级、如何投递、是否展示给最终 UI，不应该由 `llm_context` 判断，而应该由 Behavior / Session 层定义。

当前 `llm_context` 只需要维护一个事实：在 Behavior 模式下，LLM 输出 `<report>` 时，更新本 `LLMContext` 的 `last_report`。这个字段是 context 的 LastState，不带 target，不触发跨 session 投递。

## 当前实现事实

### `llm_context` 已经把 `<report>` 作为 LastState

相关代码：

- `src/frame/llm_context/src/xml_behavior.rs`
  - `<report>` 必须在 `<actions>` 外。
  - 解析后进入 `LLMBehaviorResult.self_report`，多次出现时 last one wins。
  - `<report>` 不会变成 `AiToolCall`。
- `src/frame/llm_context/src/context_loop.rs`
  - `StepRecord.self_report` 会在 action dispatch 前写入 `self.state.last_report`。
  - 同时发出 `WorkEvent::SelfReportSet { chars }`，当前主要用于 worklog / one-line status。
- `src/frame/llm_context/src/state.rs`
  - `LLMContextState.last_report: Option<String>` 随 snapshot 持久化。

这说明 waist 层已经满足 "`LLMContext` 只更新自己的 `last_report` 状态" 这个边界。

### `opendan` Session 层尚未完成 WorkSession report 上行

相关代码：

- `src/frame/opendan/src/agent_session.rs`
  - `handle_outcome(Done)` 会对所有 session 调 `post_outbound_message(&response.message)`，但 `post_outbound_message` 内部只允许 `SessionKind::Ui` 通过 msg-center 回送。
  - WorkSession 自然 Done 会进入 `NextAction::End`，但没有把 `final_snapshot.state.last_report` 投给创建它的 UI Session。
  - 注释里提到 WorkSession 通过 `report.md` 暴露结果，但当前源码里没有实际写 `report.md` 的实现。
  - `switch_behavior` / `handle_process_end` 已经会读取 `final_snapshot.state.last_report`，用于 behavior switch / fork child end 的内部 handoff。
  - `SessionMeta.process_stack` / `ProcessFrame` 已经表达了 behavior process 的父子调用栈；`handle_process_end` 在栈非空时把 child report 注入回 parent process，而不是结束整个 session。
- `src/frame/opendan/src/agent.rs`
  - `create_work_session` 创建 WorkSession 时，`SessionMeta.owner` 写的是 `created_by_session_id`。
  - 这已经能表示"哪个 UI Session 创建了这个 WorkSession"。
- `src/frame/opendan/src/worksession_tools.rs`
  - `try_create_worksession` / `create_worksession` 负责从 UI Session 派生 WorkSession。
  - `forward_msg` 已经支持 UI Session -> WorkSession 的进程内路由。

因此目前缺的是反向链路：WorkSession 触发 report 后，Session 层应把 report 作为有 envelope 的事件投给其上级 UI Session。

### libopendan 当前实现

相关代码：`src/frame/lib_opendan/src/runner/outcome.rs`（`handle_context_outcome`、`classify_done`、`finish_run`）。

- `handle_context_outcome` 解释 run 的 `Done`：终止 Step 的 `<report>`（`behavior_result.self_report`）优先作为本次结果（answer），没有时取最后的回答文本。`END` / `done`（xllm 解析器把只带 `<report>` 的 Step 判为 `done`）是终止，`WAIT_USER_MSG` 是等待输入，其它 `next_behavior` 是 behavior 交接，方式由目标 behavior 的进入模式决定（`extensions.opendan.behaviors.<name>.mode`：`switch_context` / `create_sub_context` / `fork`，没有默认模式；目标没有条目时 Turn 以 `behavior_config` 失败）。
- 子 context（`create_sub_context` / `fork`）：无论以什么结束都返回调用方，结果（终止 Step 的 `<report>`，没有时取最后的回答文本）连同 `status` 进 `state.process_result`。`END` / report → `ok`；`WAIT_USER_MSG` → `needs_user_input`（子 context 不消费调用方的输入队列）；不可重试错误、预算耗尽、未知交接目标 → `failed`；交接到 `switch_context` 目标也只是返回。由 `next_behavior` 触发时，调用方 run 恢复后在交接批次 `<session_input hook="on_behavior_switch">` 的 `<process_result behavior=… status=…>` 里读到；由工具 / action `call_behavior({behavior, task})` 触发时，结果作为该调用的工具结果回填（`ResumeFill::ToolResults`；failed 为工具错误，needs_user_input 为结构化 JSON）。这是 parent handoff，不上行、不完成 Turn。
- `switch_context` 切换不交接 report；这类 context 的 `END` 不弹回上一个 context，而是按 Session 的结束条件处理。
- `report` 工具与显式完成策略（H4）尚未实现，子 context 的完成仍由 `END` / 只带 `<report>` 的 Step 表达。
- Turn 完成由 Session 判定：交付了结果的 Done → Turn `completed`；`WAIT_USER_MSG` 只有本 run 有 `<report>`（`last_report`）或最后一个 Step 有 `<sendmsg>` 时才完成 Turn；交接和子 context 返回保持 Turn 打开。
- Session 结束（`end_condition` 满足）时，`finish_run` 把 answer 写进 `report.md`（`- turns: N` 为已完成的 Turn 数）和 `state.result.answer`。
- 没有向上级 Session 投递 report envelope 的实现。父 Session 可以通过 `origin.parent_session` 建立的半订阅观察子 Session 的状态，结果回传仍需调用方组合（见 [readme.md](readme.md) 的 AgentSession Tree）。

## 目标语义

### 1. `<report>` 的含义由当前 Behavior 定义

例如 Searching Behavior 可以在 prompt 中定义：

- 什么时候需要 report；
- report 是阶段性发现、阻塞原因、还是最终结论；
- report 的格式和信息密度；
- 阶段性报告的时机，以及什么条件满足后输出不带 actions 的 `<report end="true">`。

`llm_context` 不解释报告正文的业务含义；结束意图由 report 的 end 属性独立表达。宿主按当前 context 的调用关系与 Session 策略裁决完成，不能仅因为更新了 last_report 就认为任务结束。

### 2. Session 层定义"向上级 report"的实际行为

Session 层可以拦截每次 LLMContext run 返回的 Outcome，并根据自己的 session 类型和父子关系决定如何处理 report：

- 子 context（`create_sub_context` / `fork`）：上级是同一个 WorkSession 内的调用方 context，libopendan 把子 context 的结果交回调用方（见上文“libopendan 当前实现”）；旧 Runtime 的 fork / independent 子 process 通过 `last_report` 作为 handoff。
- WorkSession 顶层 context：默认上级是创建它的 UI Session，即 `SessionMeta.owner` 指向的 session。
- UI Session 顶层 context：已经是 Agent 内的最上层，是否把 report 展示给最终 UI 由 UI 实现决定。

### 3. "上级"按 context 调用栈精确定义

`<report>` 的上级不是固定等于 Session 的上级。Behavior 模式下应先看当前 context 在调用栈里的位置：

1. **调用栈深度 > 0：上级是 Parent Context**
   - 以 `create_sub_context` / `fork` 调用的 sub-context，其 report 首先属于父（调用方）context。
   - 典型例子：Plan 以子 context 调用 Searching。Searching 的 `<report>` 是向 Plan report；Session 站在旁路当然可以观察到这条 report，但默认不应把它当作 WorkSession 对 UI 的最终 report。
   - 父 context 原理上应该有机会读取 child `last_report` 后做处理：接受、压缩、忽略、重新表述，或继续调度别的子 context。
2. **调用栈深度 = 0：上级才是 Session 的上级**
   - WorkSession 顶层 context 的 report 才默认向创建它的 UI Session 上行。
   - UI Session 顶层 context 的 report 是否继续展示给用户，是 UI Session / UI 层自己的策略。
3. **Session 层是当前实现的统一承接点**
   - 目前父 context 的"处理机会"还没有独立 runtime hook，所有 handoff 都在 `AgentSession`（libopendan 是 `SessionRunner`）里完成。
   - 因此文档里的"父 context 处理"是语义定义；实现上由 Session 层读取调用栈（`process_stack` 的 `Caller` 帧）和目标 behavior 的进入模式来完成这个 handoff。

### 4. 目标 behavior 的进入模式决定 report 来源和归属

系统根据目标 behavior 的进入模式决定对外 report 应取自 sub-context 还是 current context（libopendan 已按此实现 parent handoff）：

| 模式 | report 来源 | 默认上级 | 是否上行到 Session 上级 |
|---|---|---|---|
| `switch_context` | 当前 context 自己的 `last_report`；切换本身不交接 report | 当前 session 的上级，前提是它不在子 context 调用内（调用栈深度 0） | 仅栈深度 0 时可以 |
| `create_sub_context` | child context 的结果（`<report>` 或最后的回答）作为调用返回交给 parent context，带 `status` | parent context | 不直接上行，除非 parent 后续在栈深度 0 重新 report |
| `fork` | 同 `create_sub_context`；区别只在 child 的构造（保留 parent 的 system 与完整历史） | parent context | 同上 |

这个规则避免把内部子任务的中间结果泄漏给 UI，同时保留 Session 层观察和审计完整链路的能力（子 context 的全部记录仍在 worklog，重建 Session 历史时只渲染它的 `process_done` 结果）。

调用栈深度指 `process_stack` 里 `Caller` 帧的层数（最深 4 层）；`switch_context` 留下的 `Parked` 帧不构成调用关系，也没有返回语义。旧 opendan Runtime 的 `independent` 则是 child `END` 时把 `last_report` 作为 `process_return` history input 交给 parent，属于旧 Runtime 的行为。

### 5. WorkSession report 不等同于直接给用户发消息

WorkSession 的 report 应先投递到 UI Session。到达 UI Session 时需要带 envelope，让 UI Session 或前端 UI 有机会决定：

- 忽略阶段性 report；
- 只关注 WorkSession 结束时的最后一条 report；
- 将 report 渲染成进度卡片；
- 将 report 合并进下一次给用户的回复；
- 只存入历史，不打扰用户。

默认策略应偏保守：UI Session 通常只关心 WorkSession 完成时的最后一条 report。

## 建议的数据形态

WorkSession -> UI Session 的上行对象建议作为 `PendingInput::Event` 投递，而不是伪装成普通 user message。它不是来自用户，也不需要走 msg-center ack。

建议 envelope：

```json
{
  "type": "worksession_report",
  "report_id": "report:<work_session_id>:<seq>",
  "source_session_id": "ws-xxxx",
  "source_kind": "work",
  "target_session_id": "ui-xxxx",
  "title": "...",
  "objective": "...",
  "workspace_id": "...",
  "behavior": "...",
  "context_depth": 0,
  "process_entry": "...",
  "parent_process_entry": null,
  "phase": "checkpoint | final",
  "report": "...",
  "next_behavior": "END",
  "is_final": true,
  "trace_id": "...",
  "created_at_ms": 1710000000000
}
```

字段约定：

- `report`：来自 `final_snapshot.state.last_report`，这是唯一权威正文。
- `phase=checkpoint`：WorkSession 仍会继续工作，UI 默认可以忽略或折叠。
- `phase=final`：WorkSession 顶层结束时的最后报告，UI 默认应关注。
- `phase` 表示 WorkSession 是否结束，不等于 Turn 是否完成：libopendan 中 `WAIT_USER_MSG` 且已有 report 时当前 Turn 已 `completed`，但 Session 仍在等待输入，所以仍是 `checkpoint`；`final` 对应 Session 结束。
- `next_behavior`：来自 `behavior_result.next_behavior`，仅作为调试 / UI 判断辅助。
- `context_depth`：调用栈深度；只有 `0` 的 report 默认允许向 Session 上级传播。
- `process_entry` / `parent_process_entry`：用于解释 report 来自哪个 behavior process，以及它原本应交给谁。
- `source_session_id` / `target_session_id`：用于前端做关联展示和去重。

不建议把完整 `assistant_text` 放进 envelope。Behavior XML 响应可能包含 `<thinking>` / `<actions>` / `<next_behavior>`，不是 UI 需要消费的正文。

## Session 层处理流程

### WorkSession 的 Behavior Context run 以 Done 返回

在旧 Runtime 的 `AgentSession::handle_outcome(Done)` 中（libopendan 对应 `handle_context_outcome` / `finish_run`），拿到：

- `behavior_result`
- `response`
- `final_snapshot`
- 当前 `SessionMeta`

如果当前 session 是 `SessionKind::Work`：

1. 读取 `final_snapshot.state.last_report`。
2. 如果 report 为空，不产生上行 report event。
3. 先判断当前 context 调用栈深度：
   - `process_stack` 非空，或正在处理 fork / independent child end（旧 Runtime；libopendan 对应“当前 run 是子 context”，即栈顶是 `Caller` 帧）：上级是 parent context，只做 parent handoff，不投给 UI Session。
   - `process_stack` 为空：当前是顶层 context，可以继续判断是否向 Session 上级 report。
4. 判断当前 report 阶段：
   - 顶层 `END` 或 WorkSession 自然 Done 并进入 `NextAction::End`：`phase=final`
   - `WAIT_USER_MSG`、非终态 switch、继续等待工具/输入：`phase=checkpoint`
5. 将 report 写入 WorkSession 自身可观察位置：
   - 至少进入 worklog；
   - 可补齐当前设计文档里提到但尚未实现的 `report.md`。
6. 解析上级：
   - `target_session_id = meta.owner`
   - `AIAgent::get_session(target_session_id)` 必须存在且是 `SessionKind::Ui`
7. 向 UI Session `enqueue_pending(PendingInput::Event { ...envelope... })`。
8. 如果上级 session 不存在、不是 UI、或已经结束：
   - 不影响 WorkSession 自身结束；
   - 写 warn；
   - report 仍保留在 snapshot / worklog / report.md。

### UI Session 收到 report envelope

UI Session 收到 `worksession_report` event 后，不需要无条件回用户。

建议默认 prompt / runtime 约定：

- `phase=checkpoint`：只有用户明确关心该 WorkSession 或 report 要求人类输入时才展示。
- `phase=final`：把它作为 WorkSession 最终结果处理；可以发给前端，也可以等下一次 UI 回复时合并。
- 若最终 UI 协议支持结构化消息，应保留 envelope，而不是把它降级成纯文本。

### 子 context / 子 process

这类 report 的上级不是 UI Session，而是同一个 WorkSession 内的父 context（旧 Runtime：fork / independent 子 process 的父 process）。

旧 Runtime 已经在 `handle_process_end` 中通过 `final_snapshot.state.last_report` 构造父 process handoff；libopendan 由 `finish_run`（`process_done`）把子 context 的结果交回调用方。这里不应额外投给 UI，否则会把 WorkSession 内部子任务的中间 report 泄漏到最上层。

更精确地说：

- child context 的上级是 parent context，不是 Session 的上级。
- Session 层可以看见 child report，并负责把它转成 parent handoff。
- parent context 才有资格决定是否把 child report 原样、摘要后、或完全不作为自己的 report。

只有 WorkSession 顶层结束，才默认产生 `phase=final` 的 UI 上行 report。

## 去重与持久化

为了避免同一条 report 因 resume / retry 被重复投递，建议在 `SessionMeta` 增加轻量投递状态：

```rust
last_report_delivery: Option<ReportDeliveryState>
```

其中记录：

- `report_hash`
- `phase`
- `report_id`
- `delivered_at_ms`

去重规则：

- 同一 `report_hash + phase` 已成功投递过，不重复投递。
- 如果 checkpoint 已投递，WorkSession 结束时同一正文仍可以再投递一次 `phase=final`，因为 UI 对 final 有不同语义。
- 投递到 UI Session 前先更新本 WorkSession 自身持久化状态，避免崩溃后丢失最终 report。

如果暂时不改 `SessionMeta` 结构，MVP 可以只在 WorkSession 顶层结束时投递一次 final report，先避开 checkpoint 去重问题。

## 与现有概念的边界

### `<report>` vs `<sendmsg>`

- `<report>`：更新当前 LLMContext 的 `last_report`，由 Session 层决定是否向上级投递。
- `<sendmsg>`：过程通信动作，有明确收件方；当前实现仍是 worklog stub，不应写入 `last_report`。

二者不能合并，否则会把 LastState 和消息副作用混在一起。

### `<report>`、behavior Done、Turn 完成与 WorkSession 结束

- `<report>`：只更新 `last_report`。中间 report 不结束 behavior，也不完成 Turn。
- behavior Done：`next_behavior`（`END` / `done` / `WAIT_USER_MSG` / 跳转目标）或什么都没做的收敛 Step 让 `LLMContext::run()` 以 `Done` 返回。它只是一次 `run()` 调用的返回，不等于 Turn 完成：交接（`switch_context`）和子 context 返回之后 Turn 仍然打开。
- Turn 完成：只由 Session 判定（规则见上文“libopendan 当前实现”）。单独的 `<sendmsg>` 或中间 report 不完成 Turn。
- WorkSession 结束（`phase=final`）：Session 按 `end_condition` 结束。一个 Session 可以先完成多个 Turn（`end_condition.type = "max_turns"`）。

### `last_report` vs `one_line_status`

- `last_report`：面向上级的结构化/半结构化产出正文，可以较长。
- `one_line_status`：面向 UI 列表和可观察性的短状态，由 WorkEvent / status sink 更新。

WorkSession 列表应继续读 `one_line_status`，不要用 report 替代。

### report envelope vs msg-center outbound

WorkSession -> UI Session 是 Agent 内部 session 路由，不应直接 `msg_center.post_send`。

只有 UI Session 决定要对最终用户说话时，才走现有 `post_outbound_message` / msg-center 链路。

## MVP 修改范围

以下针对旧 opendan Runtime；下一阶段 opendan 基于 libopendan 重构时，需按 `handle_context_outcome` / `finish_run` 和 Turn 规则重新落位。建议先做最小闭环：

1. 在 `AgentSession::handle_outcome(Done)` 的 WorkSession 顶层结束路径中，读取 `final_snapshot.state.last_report`。
2. 确认当前 context 位于调用栈 0 号位置；若是 child context / child process，只做 parent handoff。
3. 若非空，构造 `worksession_report` envelope。
4. 通过 `meta.owner` 找到创建它的 UI Session。
5. 投递为 `PendingInput::Event`。
6. UI Session 默认只处理 `phase=final`。
7. 补一个 opendan 单测：
   - UI Session 创建 WorkSession；
   - WorkSession final snapshot 带 `last_report`；
   - WorkSession End 后 UI Session 的 pending inputs 中出现 `worksession_report` event；
   - envelope 的 `source_session_id`、`target_session_id`、`phase`、`report` 正确。
   - fork / independent child END 时，不产生 UI 上行 report，只产生 parent handoff。

暂不做：

- checkpoint 实时投递；
- report.md 完整历史格式；
- 前端 UI 卡片样式；
- 跨 Agent / 跨 msg-center 的 report route。

## 风险与注意事项

- `SessionMeta.owner` 当前在 UI Session 中表示 tunnel / owner，在 WorkSession 中表示创建者 session id，语义有重载。MVP 可以沿用，但后续建议显式拆成 `created_by_session_id`。
- `handle_process_end` 会在顶层 END 时 discard snapshot。最终 report 投递必须在丢弃 snapshot 前读取。
- 判断"顶层"不能只看 `SessionKind::Work`，还必须看 behavior process 调用栈；否则会把 Plan 下面的 Searching report 误投给 UI。
- Behavior 模式的 `ContextOutput::Text` 当前是原始 assistant text，可能包含 XML，不应作为用户可见 report 使用。
- 如果 UI Session 已结束或不存在，WorkSession 不能因此失败；report 应仍可从 WorkSession 本地状态追溯。

## 验收标准

- `llm_context` 不新增 target / session 语义。
- WorkSession 最终 `<report>` 能通过创建它的 UI Session 上行。
- 子 context / 子 process 的 `<report>` 先交给 parent context，不直接上行给 UI Session。
- 只有调用栈 0 号位置的顶层 context 才能默认向 Session 的上级 report。
- UI Session 能区分普通用户输入和 `worksession_report` envelope。
- 默认 UI 行为只关注 WorkSession 的最后一条 final report。
- `cargo test -p llm_context --lib` 与 `cargo test -p opendan --lib` 应保持通过。
