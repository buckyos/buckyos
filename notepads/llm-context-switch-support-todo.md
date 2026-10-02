# llm_context 层修改 TODO：Context 调度所需的原语

日期：2026-10-02

状态：待 review。Context 调度的语义（转移表 T0–T6、普通切换换配置、fork / independent、工具子上下文）属于 Agent Session，定义在 [xAgent.md §3.2–§3.7](../doc/opendan/xAgent.md)；本文只列 llm_context 层要先补的原语。

## 1. 范围与原则

- 范围：`llm_context` crate 的 `snapshot_overrides.rs`，以及 `agent_tool` 中的 xllm（run 记录与 hosted run 的结束状态）。
- 按“llm_context 修改先行”的规则，本文各项先于 xAgent.md §11 C14 实施；宿主侧（libopendan / xagent）只调用这里的原语，不再自己改快照内部字段。
- llm_context 不感知 Turn、process_stack、behavior 配置；这些都在 Session。
- 持久格式变化按 beta 2.2 规则升版并拒绝旧版本。

## 2. 现状（2026-10-02 源码核对）

- `rebuild_with_inherit(base, overrides, deps)` 要求 base 快照既没挂起也不在批次中途（`suspended.is_none() && !has_continuation()`），否则返回 `SnapshotCorrupted`。工具子上下文（T4）的父快照必然处于 PendingTool 挂起，不能用它派生子快照。
- `RequestOverrides.user_messages` 会清空 steps、summaries、history_inputs，只留新的 user 段；opendan `try_create_worksession` 靠它把“父最近对话”作为一条消息交给子 context。
- libopendan fork 子 run（`runner/live.rs::new_run_context`）先建普通 run，再手工复制父快照的 `steps` + `last_step`、`history_summaries`、`next_step_index`、`next_action_id`，并在 host meta 里记 `inherited_below`；只支持 behavior 模式。
- xllm `rebuild_toolset(record)` 按 run.json `config` 重建工具；没有给宿主替换 run 中途 `config` 的接口。
- xllm 接手的 hosted run 以 `next_behavior = B` 结束时记 `Completed`；libopendan `reconcile.rs` 把它当 Turn 结束，切换丢失（xAgent.md G8）。

## 3. 原语

- [ ] **S1 `derive_child`**（P0，T2 / T4 共用）：

  ```rust
  pub enum Inherit { Steps, Transcript { recent: usize }, None }

  /// 从父快照的“已定历史”派生子快照；父快照可以处于 PendingTool 挂起或批次中途。
  /// child_request 由宿主给出（子 behavior 的 system、可选 <session_history>、工具策略、预算）。
  pub fn derive_child(parent: &LLMContextSnapshot, inherit: Inherit, child_request: LLMContextRequest) -> Result<LLMContextSnapshot, LLMComputeError>;
  ```

  - 已定历史：behavior 模式取 `steps` + `last_step` 与 `history_summaries`，不取进行中的 `action_step`；function_call 模式取 `accumulated` 中最后一个未完成工具批次之前的前缀（不含该批次的 assistant tool_calls 消息与已到的部分结果），保证子请求的消息序列合法。
  - `Steps`：要求父子 `loop_model` 相同，否则报错；step / action 编号接续父快照；返回继承边界（即 libopendan 的 `inherited_below`），宿主据此不重复写 worklog。
  - `Transcript{recent}`：取父快照最近 n 条 user / assistant 文本（不含工具消息与 step 内部记录），以结构化列表返回给宿主渲染成一条 user 消息；渲染格式由宿主定（xAgent.md §12 第 14 项）。
  - `None`：只用 child_request。
  - 子快照不挂起、没有续跑状态，计数按 child_request 重新开始。
  - 取代 libopendan 的手工复制；`rebuild_with_inherit` 保持原有前置条件（T1 普通切换仍用它）。
  - 测试：两种 loop 模式 × 三档 inherit；父快照分别处于正常、PendingTool 挂起、批次中途；编号接续；继承的 steps 在子 context 里按继承记录渲染。
- [ ] **S2 run 中途替换 `config`**（P0，T1）：xllm 的 run 记录提供宿主替换有效配置的接口（如 `RunHandle::replace_config`），写 run.json 时与快照的 `request.tool_policy` / `model_policy` 一致；resume 与 `rebuild_toolset` 只认最新值。测试：替换后 `xllm --resume` 用新工具集与新模型。
- [ ] **S3 hosted run 停在跳转处**（P0，G8）：xllm 推进 hosted run（`host` 非空）时，`Done` 的 `next_behavior` 不是 `END` / `done` / `WAIT_USER_MSG` 就记非终态（`Paused`，并在 run.json 记下 `next_behavior`），由宿主完成转移；非 hosted run 行为不变。测试：hosted run 跳转后状态非终态；独立 xllm run 不受影响。

`subrun` 作为 `wait.source.kind` 已在 [long-tool TODO](./llm-context-long-tool-todo.md) §4 的词汇表里；resolver 由 Session 实现，本层没有新增。T4 依赖该文 §4 的等待记录与接手能力检查先落地。

## 4. 待 review

1. S1 的 `Transcript` 是否需要放在 llm_context：也可以完全由宿主读快照自行抽取，本层只提供 `Steps` / `None`。
2. S3 的非终态用 `Paused` 还是新增状态（如 `Handover`）；后者语义更清楚，但 run.json 状态枚举要升版。

## 5. 实施顺序与验证

S3 → S2 → S1（S3、S2 是 E19 的前置，S1 是 E20 与 fork 重构的前置）。验证：`cargo test -p llm_context`、`cargo test -p agent_tool xllm`，宿主侧以 xAgent.md E19 / E20 验收。
