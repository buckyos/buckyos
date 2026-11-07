# llm_context：PLAN DO 所需的 behavior 决策钩子与继承报告

日期：2026-10-16

状态：已实施（2026-10-16，实施记录见 §4）。宿主接入见 [PLAN DO TODO](opendan-plan-do-todo-list-todo.md) §8。

来源：[OpenDAN Todo 驱动的 PLAN DO 实施 TODO](opendan-plan-do-todo-list-todo.md) 的 §2.2（无决策即等待、`next_behavior = do` 的可修正拒绝）与 §2.5（继承历史遗漏 `self_report`）。按约定，llm_context 层（`llm_context` crate 与 `agent_tool` 的 xllm）的修改单列在本文并先于宿主接入实施；宿主侧只写接入并引用本文。

## 1. 现状

| 入口 | 现状 |
| --- | --- |
| `llm_context/src/context_loop.rs` behavior loop | Step 只有阶段 `<report>`（无 actions、无 sendmsg、无 next_behavior）时 `complete_step` 不结束循环，`apply_step_result_hook` 因没有 action 结果不调用，Step 沉积后**再推理一次**。宿主没有机会把它当作“等待”。PLAN 写阶段报告等探索返回会变成空转。 |
| 同上，`next_behavior` 交接 | 解析与协议校验通过即 `Done`；宿主只能在 `Done` 之后拒绝。libopendan 的 `refuse_handover` 对顶层 run 是不可恢复失败并关闭 Turn，不能作为“可修正错误、run 继续”。 |
| `CheckpointHook::validate_report` | 已有先例：宿主在解析之后、Step 生效之前拒绝报告，拒绝折叠为纠错 Step，循环继续（可修正，计入连续错误）。 |
| `llm_context/src/step_record.rs` `render_inherited_step_record` | 只渲染 `thought`/`assistant_text`、observation、actions，遗漏 `self_report`。子 context 以 `inherit = steps` 继承调用方 Step 时看不到调用方的阶段报告；测试 `inherited_self_report_does_not_render_as_action` 断言的正是这一行为。 |
| `agent_tool/src/xllm.rs` behavior 协议文本 | Session 风格写“`<report>` 提交阶段报告并继续 context”，没有说明会话里只有阶段报告的回复会交还控制。 |

## 2. 改动

### L1 继承历史渲染 `self_report`

- `render_inherited_step_record` 在 `<intent>` 之后、`<actions>` 之前渲染 `<report end="true|false">…</report>`（无报告时不出现）。
- 报告仍不渲染为 action 结果（不出现 “Report acknowledged” / “- Report”）。
- 有意改动测试 `inherited_self_report_does_not_render_as_action`：断言报告正文出现、actions 仍为 `No action.`。

### L2 宿主的 behavior 决策钩子（交接与让出）

- 新增 `StepDecision`：
  - `Handover`：`next_behavior` 非空，且没有 actions / sendmsg（有动作时 next_behavior 本来就被忽略）。
  - `Yield`：只有阶段报告（`<report>` 非空且 `end` 不为 true），没有 actions、sendmsg、next_behavior。
- `LLMBehaviorResult::decision() -> Option<StepDecision>`。
- `CheckpointHook::review_decision(snapshot, result, step_index, decision) -> Option<DecisionVerdict>`，缺省 `None`：
  - `None`：宿主不表态，行为与现在完全相同（交接 `Done`；只有阶段报告时继续推理）。独立 xllm 与未实现该方法的宿主不受影响。
  - `Some(Accept)`：交接照常 `Done`；让出以 `Done` 结束循环（`behavior_result` 带阶段报告、无 next_behavior、`report_end = false`），由宿主解释。
  - `Some(Refuse(msg))`：与 `validate_report` 拒绝相同，折叠为纠错 Step（正文为宿主给出的 `msg`），计入连续错误，同一 run 继续推理。
- 调用位置：解析 + `validate` + `validate_report` 之后、Step 生效之前；只对外层 behavior loop（有 `checkpoint_hook` 的那一层）调用，不交给 Step 内层。
- llm_context 只做机械判断（有无动作、有无交接、有无报告），不理解 Todo、子 Session 或等待；“为什么拒绝”完全由宿主文本给出。

### L3 xllm 协议文本与接手

- `HostProtocolFlavor::Session` 的 behavior 规则补一句：只有阶段报告、没有 actions 与 next_behavior 的回复把控制交还 Session；Session 只在等待它启动的工作（子 Session）时接受，否则拒绝并说明，你继续工作。
- OneShot 文本不变。xllm 接手宿主 run 时不装宿主的决策钩子，行为不变（宿主 run 的 `Done` 由 Session 恢复时解释）。

## 3. 验证

- `cargo test -p llm_context`：新增 behavior loop 用例覆盖 `None`（行为不变）、让出被接受（`Done`、报告在 `behavior_result`、无额外推理）、让出被拒绝（纠错 Step、同 run 继续）、交接被拒绝（纠错 Step、无 `Done`）、有动作时不调用钩子。
- `cargo test -p llm_context step_record`：继承报告渲染。
- `cargo test -p agent_tool --lib xllm::behavior_protocol_tests`：Session 文本含让出说明，OneShot 不含。

## 4. 实施记录（2026-10-16）

| 项 | 落点 |
| --- | --- |
| L1 | `llm_context/src/step_record.rs::render_inherited_step_record`：`<intent>` 后渲染 `<report end="…">`（XML 转义）；测试改名 `inherited_self_report_renders_as_report_not_action` |
| L2 | `behavior_loop.rs`：`StepDecision {Handover, Yield}`、`DecisionVerdict {Accept, Refuse(String)}`、`LLMBehaviorResult::decision()`；`deps.rs`：`CheckpointHook::review_decision`（缺省 `None`）；`context_loop.rs`：解析 + `validate` + `validate_report` 之后调用，`Refuse` 走与报告拒绝相同的纠错 Step，`Accept` 的 Yield 在报告副作用之后直接 `finish_done_behavior`；`lib.rs` 导出 |
| L3 | `agent_tool/src/xllm.rs` Session 风格 behavior 规则：只有阶段报告的回复在等待本 Session 启动的工作时结束本步并由 Session 唤醒，没有可等的时只是阶段报告、继续；单测 `a_progress_report_alone_yields_only_in_a_session` |

与 §2 的差异（宿主侧的取舍，llm_context 契约不变）：libopendan 对 Yield 只在“顶层 context 且有未收齐的子 Session”时 `Accept`；没有可等的就返回 `None`（原有语义：报告照常记录、继续推理），只有连续第三个只有阶段报告的回复才 `Refuse`（防空转）。最初实现成“没有可等的就 Refuse”，与已有用例 `report.rs::behavior_stage_report_continues_until_an_explicit_final_report`（文档化的“阶段报告后继续”）冲突，改为现在的规则。

验证（在 `src/`）：`cargo test -p llm_context`（227，新增 `without_a_verdict_a_progress_report_continues`、`an_accepted_yield_ends_the_loop`、`refused_decisions_are_corrected_in_the_same_run`、`hand_overs_and_steps_with_actions`）；`cargo test -p agent_tool --lib xllm::behavior_protocol_tests`（4）。
