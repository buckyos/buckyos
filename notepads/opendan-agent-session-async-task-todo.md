# Agent Session 按"全异步 task 化"重构 TODO

日期：2026-11-01

状态：**待整理 / 待裁决。** 承接 [llm_context 冻结 TODO](./llm-context-freeze-todo.md)：冻结已落地（§8），libopendan 现在"能编译、测试全过"，但多处是为了跑通 B 方案做的临时取舍。用户 2026-11-01 裁决：agent-session 要基于"全异步 task 化"的假设重构一部分逻辑，和 agent-session 有关的修改与测试都放到这份 TODO；xllm 的 DV 已在冻结 TODO 里做完。

## 1. 前提（冻结后已定，不再讨论）

- llm_context 只跑 loop，没有任务和挂起原语；每个工具调用前一个 checkpoint；Yield 立即结束 run。
- 每个工具都是异步的：调用内只在 `RuntimeConfig.task_wait_ms` 预算内等，超出就返回"仍在运行，task X"；任务的结局以输入回来（`<<task_result>>`、子 session 结局事件、`<process_result>`）。
- 崩溃恢复：从调用前 checkpoint 重新派发，ToolManager 按 call_id 先找再做。
- ask-user 是 websdk `ask_user`：一条 `input` 通知 + TaskMgr 任务，答案是任务结果。

## 2. 现状里的临时取舍（重构时逐条重新设计）

- **R1 等待状态的形状。** `SessionState.waiting_for { kind: WaitingKind::{Input, Tool, Event, Children}, refs }` 由 outcome.rs / drive.rs 写，llm_context 不知道它。冻结后它只剩"run 保活时在等什么"和"空闲等用户下一条消息"两种含义，`watched_tasks` 与 run.json 的 `host.extra.{tasks, task_calls, sub_call, kept}` 又各记一份。想法：session 只记"在跟的任务 + 子 session"一张表，`waiting_for` 由它推导，或者干脆不存。
- **R2 子 session 的等待。** `SessionTaskResolver.wait("session:<sid>")` 立即答复（工作区委托给子 session 时，父 run 在调用内等会死锁到预算用完）；`agent-session wait <sid>` 去掉了 `--wait-ms`，等于"查状态 + 登记要等它"。要定：`wait` 命令还要不要（保活本来就会等本 run 登记的子 session），`create-worksession --wait` 的含义，子 session 是否就是 TaskMgr 任务（与 `t-` 任务同一套跟进）。
- **R3 跟进提示。** session 宿主用 `TaskFollowup::Input`："结果会作为输入回来，不用等"；但 `agent-session task wait/state/cancel` 和 `wait_task` 仍可用，模型仍可能在 loop 里等。要定：opendan 里是否还暴露"在调用内等"的入口。
- **R4 保活优先于完成策略。** 显式完成、没有输入队列的 work session（plan_simple）在本 run 的任务 / 子 session 未完时回复，本会判 `missing_end_report`，现在照样保活。要定 Turn / session 的结束规则在全异步下的统一说法。
- **R5 noted task 的分工。** `session:` 也记进 run 的 noted tasks（保活要用），报告校验只看非 `session:` 的任务，工作区交接检查忽略已结束的子 session，`children::mark_heard` 防止"调用结果就是子 session 结局"时重复投递。三处都是补丁，应随 R1 一起收敛。
- **R6 保活中的 UI Turn。** 中间那条回复（"已启动，完成后汇报"）不单独投递，Turn 关闭时只投最终回复。要定：中间回复怎么给用户（占位消息编辑 / 单独消息 / Turn 内多条）。
- **R7 提问的撤销。** 同一对话来了普通消息时，未答的 `ask_user` 通知要不要撤销（superseded）。
- **R8 `<<task_result>>` 的 status。** 现在只有 `finished` / `unknown`（`drive.rs::task_summary`），失败只写在正文；要不要给 `failed`，牵涉 task 事件的订阅匹配。
- **R9 `remote_node` 的 `XLLM_*` 环境变量**被过滤，`ask_user` 拿不到 `XLLM_RUN_ID / XLLM_CALL_ID`（幂等键）。
- **R10 宿主重启后的 `local:` 任务。** agent service 重启时，进程内的 shell 任务随进程结束（xllm 暂停时会把任务交给 OS，libopendan 没有这一步）；session 在跟的 `local:` 任务重启后只能从执行目录读结局。要定 opendan 的 shell 任务是否一律交给 Zone TaskMgr（`ZoneShellTasks` 已有），本进程任务只做兜底。

## 3. 测试与 DV（单 OOD，test.buckyos.io）

- **T1 ask_user**：Agent 在对话 / Work 里 `ask_user` → 通知到用户（MessageHub 呈现）→ 作答 → `<<task_result>>` 回到同一 run；过期与撤销；xllm 在注入了 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 的环境里同样经 Zone TaskMgr 拿到答案（`run_local_llm::with_zone_tasks`，冻结 DV 时没有可用会话 token，未测）。
- **T2 call_behavior**：段结束后切换到子 context，结果以带 `call_id` 的 `<process_result>` 回到调用方；崩溃在各阶段。
- **T3 shell 长命令**：超出预算转任务 → 回复保活 → `<<task_result>>` 注入同一 run；期间来新消息注入同一 run；stop 取消在跟的任务。
- **T4 子 session**：委托工作区、等待、结局回到父 run；plan_simple 用真实模型跑一遍。
- **T5 崩溃恢复**：保活中的 run 崩溃后由 `kept` 记录重做；工具调用中崩溃按 call_id 恢复。

## 4. 关联

- [llm_context 冻结 TODO](./llm-context-freeze-todo.md) §8（实施记录、偏差、xllm DV）。
- [Agent 通知人 TODO](./opendan-agent-notify-todo.md)、[plan_simple TODO](./opendan-plan-simple-todo.md)。
