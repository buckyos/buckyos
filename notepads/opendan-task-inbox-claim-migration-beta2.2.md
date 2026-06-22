# opendan Task Inbox Assigned Start Migration Plan for Beta2.2

更新时间：2026-06-12

## 1. 背景

opendan 的 task inbox 位于：

- `src/frame/opendan/src/agent_task_executor.rs`

当前 inbox 会周期 sweep 并监听 runner `task_ready` 事件。事件只作为唤醒信号，DB task 状态仍是唯一真相。

opendan 当前会处理这些 `agent.delegate` 状态：

- `Pending`
- `WaitingForApproval`
- `Running`
- `Paused`
- `Canceled`

这和 node_daemon 不一样。node_daemon 只处理简单的 `Pending -> Running -> terminal` 子进程执行模型；opendan 还要恢复会话、等待人类输入、反射暂停/取消控制。

## 2. 不能直接照搬 node_daemon 的原因

node_daemon 的 assigned start 模型：

- sweep `Pending`
- `start_assigned_task`
- 启动一个子进程
- 写终态

opendan 的模型：

- `Pending` 可能要创建新的 work session。
- `Running` 可能是已有 session 需要 recover / wake。
- `WaitingForApproval` 代表子 human input task 或父 delegate task 正在等待用户动作。
- `Paused` / `Canceled` 需要反射到 session control。
- `execution.session_id` 是避免重复创建 session 的核心幂等键。

因此 opendan 迁移必须是局部的，不能把所有状态都先进入 start guard。

## 3. 迁移目标

Beta2.2 目标：

- 只让“启动新 session 的 `Pending` 路径”进入 `start_assigned_task`。
- 保持 `WaitingForApproval`、`Running`、`Paused`、`Canceled` 的现有恢复/控制语义。
- 不改变 msg-center inbox pump 和 session inbox 模型。
- 不依赖 kevent 正确性；kevent 仍只做加速。

## 4. 推荐状态处理

### Pending

推荐迁移：

1. sweep 到 `Pending agent.delegate`。
2. 如果 task 已经带 `execution.session_id`，不要 start，走 existing session recover / wake。
3. 如果 task 没有绑定 session，调用 `start_assigned_task(id, runner)`。
4. start 成功后再创建 work session。
5. 创建 session 后立刻写回 `execution.session_id`，作为后续 sweep 的幂等键。
6. start 返回 `None` 时跳过，认为别的消费者赢得竞争。

### WaitingForApproval

不 start。

继续走：

- `resume_waiting_delegate_task`
- 检查 human input child task
- 人类输入完成后把父 task 恢复到 `Running`

原因：

- 这不是“领取新工作”，而是已有 session / blocker 的恢复。

### Running

不 start。

继续走：

- `execution.session_id` 存在时 `ensure_session(session_id)` 并 `wake`
- 尝试 `recover_existing_bound_session`

原因：

- `Running` 代表可能已有会话，不应被新 start 覆盖。

### Paused / Canceled

不 start。

继续走：

- `reflect_task_control_to_session`

原因：

- 这些是控制状态，不是待领取工作。

## 5. Timeout 策略

opendan 不应在第一步复用 node_daemon 的简单子进程终态模型。

推荐后续单独设计：

- start 成功到 session_id 写回之间使用明确 timeout。
- session 创建成功后，后续超时和恢复要以 session lifecycle 为准，而不是 task inbox sweep 为准。
- 如果 session 已绑定，timeout 不能简单把 `Running` 改回 `Pending`，否则可能产生重复会话。
- 如果需要重跑，应创建新的 task，而不是复用同一个 task instance。

Beta2.2 第一阶段建议：

- Pending 新 session 启动路径先接 `start_assigned_task`。
- 暂不对 opendan 启用自动 timeout sweep。
- timed-out `agent.delegate` 默认走 manual/recovery，不自动 retry。

## 6. 验证计划

单元测试优先：

- `Pending` 未绑定 session：start 成功后创建 session。
- `Pending` start 返回 `None`：不创建 session。
- `Pending` 已带 `execution.session_id`：不 start，直接 recover / wake。
- `WaitingForApproval`：不 start，继续检查 human input child。
- `Running`：不 start，按 session_id recover / wake。
- `Paused` / `Canceled`：不 start，反射控制到 session。

端到端验证：

- 两个 inbox sweep 并发时，同一 `Pending` delegate task 只创建一个 session。
- kevent 丢失时，poll sweep 仍能处理。
- 人类输入完成后，父 delegate task 能恢复运行。

## 7. 当前建议

本轮先不改 opendan 代码。

先完成 TaskManager runner 权限收敛和 TaskCenter `human_action` schema 对齐，再开单独变更迁移 opendan 的 `Pending` 新 session 路径。
