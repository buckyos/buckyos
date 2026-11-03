# OpenDAN：可重试错误的退避与上限、窗口上限裁决、遗留 DV（宿主 TODO）

日期：2026-11-02

状态：**H1 / H2 / 文档已实施（2026-11-02，未提交），单测过；H3 DV 未做**（要重建容器、停 codex-proxy，等用户安排），记录见 §7。原状态：已 review、已裁决（Q1–Q3 均按建议，2026-11-02），待实施。review 补充的 R1–R7 已写回 §3。来源：[Work Session 上下文压缩与工具预算修正](./opendan-work-context-budget-fix-todo.md) 实施后的 DV（2026-11-02，`jarvis.test.buckyos.io` 容器形态，plan `work-20261103-3` 与它派发的 9 个 Todo session）。同一次 DV 拆出的另外两单：AICC 请求体 1 MiB 上限 [aicc-request-body-limit-todo.md](./aicc-request-body-limit-todo.md)（另行处理中）；xllm 的 AICC 错误归类 [xllm-aicc-error-mapping-todo.md](./xllm-aicc-error-mapping-todo.md)（先行，本单 H1 依赖它的 P2）。

范围：libopendan 宿主（`host/mod.rs` 的驱动循环与 `ChildDriver`、`runner/outcome.rs` 的可重试错误、`runner/live.rs` 的窗口解析）、Jarvis 包配置、上一单 §6 未做的 DV。

路径均相对 `buckyos/src/frame/lib_opendan/`，另注明的除外。

## 1. DV 结论摘要

| 项 | 结果 |
| --- | --- |
| 失败 | 3 个 `explore` 失败：2 个 413（AICC 1 MiB，见 AICC 那一单）、1 个 503 过载被当作不可恢复（xllm 单 P2）。plan 自己没有失败，靠另开 Todo session 重做（7、8、9、11 成功），现在仍在运行 |
| 窗口 | 所有 run 的窗口都按 AICC 解析成 1,050,000（`llm.chat` / `llm.plan` / `llm.code` → codex-proxy `gpt-6.1-sol` / `gpt-6-astra`），没有一个 run 到达 80% 触发线，`compactions = 0` |
| 工具预算 | 新预算生效（`work_simple` 100、`explore` 60）；所有 explicit report 的 function call run 都是 `wrap_up_tools = ["report"]`；`work-20261103-11` 用到剩 3 / 60，收到一次 `<<budget>>` 提示后正常结束。收尾模式（用完 / 墙钟 / token）没有被触发 |
| 成本 | 12 个 run 共 $9.21；长 run 每个约 $0.5–2.4，缓存命中 87–97% |

## 2. 现状（源码核对）

### 2.1 可重试错误的重试节奏

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| 何为可重试 | `is_retryable_error`：`Timeout` / `Cancelled`、`ProviderFailure::Transient`、Runtime 来源的错误 → `recoverable: true`，run 保留为 `Paused`、Turn 不关、`state.last_error` 记下；drive 返回 `DriveResult::Error` | `runner/outcome.rs:273-279,749-775` |
| 顶层会话 | 常驻循环 `serve_one`：`Error` 且 `recoverable` → 退避从 `poll_interval`（5 s）起翻倍、封顶 60 s，**没有次数或时长上限**，也不看 provider 给的重试间隔 | `host/mod.rs:563-608` |
| 子会话（Todo 的 work session） | `ChildDriver::tick` 每 ≤ 1 s 跑一次（`poll_interval.min(1s)`）；`advanceable` 对 `DriveResult::Error` 不做区分（只排除 NotDriver / Unregistered / RecoveryBlocked / 不可恢复的 BindFailed），所以**可重试错误的子会话大约每秒被重新驱动一次，没有退避，也没有上限** | `host/mod.rs:193-238,248-320,645-660` |
| 计数 | libopendan 没有任何“连续可重试失败”的计数或时间记录 | — |

今天这条路径主要被超时类错误走到。xllm 单 P2 让 AICC 标 `retriable` 的 503 / 429 也进来以后，provider 过载时 Todo session 会每秒重打一次，一直打下去。

### 2.2 窗口

上一单 §8.3 的待裁决项：窗口按 AICC `max_context_tokens` 取，codex-proxy 的模型是 1.05M，80% 触发线是 84 万 token。AICC 那一单把上限放到 32 MiB 后，这条线在技术上可以达到，问题变成每轮重发几十万 token 的成本、延迟和 Codex 额度（与本人 Codex CLI 共用 5 小时 / 周窗口）。本次 DV 最大的单轮输入约 20 万 token，缓存命中 87–97%。

## 3. 方案

### H1 可重试错误的退避与上限（依赖 xllm 单 P2；**P2 也不得先于 H1 部署**，见 R7）

- 会话状态加 `retry`（`SessionState`，缺省不序列化）：`{ attempts, since_ms, not_before_ms }`，在可重试错误的提交里写入。**清空条件（R2）：任何不是可重试错误的 outcome 都清空**——Done / PendingTool / ContextLimitReached / Interrupted / Turn 结束 / 不可恢复失败——只有连续的可重试错误才累计；一个长 run 每隔十分钟碰一次 503、中间推理和工具都正常，不能累计到用尽。随 state 持久化，宿主重启不会把计数清零，停机时间也计入 `since_ms`。
- **注册表投影（R1）**：`SessionStatus`（`protocol/agent_state.rs`）加 `retry_not_before_ms: Option<u64>`，由 `session/live.rs` `status()` 投影。`ChildDriver::advanceable` 只看 `RegistryEntry.status`、不开 SessionDir，没有这个投影就得每个 tick 读每个孩子的 state 文件。注册表是对外契约，`doc/opendan/protocol/Session Directory Protocol.md` 的注册表一段同步。
- 退避：`not_before_ms = now + min(base × 2^(attempts−1), cap)`（Q1 定 base 30 s、cap 10 min）。
- **两层分工（R4）**：
  - host 侧只管子会话：`ChildDriver::advanceable` 在 `now < retry_not_before_ms` 时返回 false，不再每秒重驱；退避期内的子会话不被 spawn，不占 `max_child_concurrency` 名额。`ChildDriver.results` 只在内存里，宿主重启后靠持久化的 `not_before_ms` 兜住。
  - 顶层会话 `serve_one` 照旧按 wake_event / poll 唤醒并进入 `drive`（输入批次在 `drive` 里才读到）；退避期的判断在 runner：run 处在退避期时不推理，只处理控制（stop / resume）与输入。**Q3 = A 的越过规则也在 runner**：本批输入里有用户消息（input_view 已有的 `is_user_msg`）就无视 `not_before_ms` 立即重试；定时器、任务结束、子会话回报等自动输入仍按退避。
- 上限：`attempts` 达到 `max_retry_attempts` 或 `now − since_ms` 超过 `max_retry_duration`（Q1 定 6 次 / 30 分钟，先到为准；6 次累计约 25.5 分钟）→ 按不可恢复处理：Turn 失败；无人应答的会话照旧 `fail_unattended`。**`last_error` 的形状（R5）**：`kind = "provider_unavailable"`、`retries_exhausted: true`、`attempts`、`since_ms`、`message` 为最后一次错误；`todos.rs` `observe()` 把这个 kind 带进 Todo 观察（状态 Failed，原因“上游不可用、重试 N 次 M 分钟后放弃”），plan 据此决定等一会儿还是换方式，而不是只看到一句错误文本。
- **墙钟（R3）**：退避期不计入 `timeout` 预算——`runner/live.rs` resume 时已按“连续运行段”重置 `started_at_ms`，H1 不改它，但 H1-4 加测试锁住：暂停 10 分钟后重试的 run 不会因 Wallclock 收尾。
- 参数放 `RunnerOptions`（`retry_backoff_base`、`retry_backoff_cap`、`max_retry_attempts`、`max_retry_duration`），不进 session 配置。**命名（R6）**：这是宿主层的“重试次数”，与 llm_context `error_policy.max_consecutive_errors`（run 内部连续错误策略）是两回事，文档里并列说明，不共用名字。
- 观测：每次退避记 `opendan.run.retry_scheduled {session_id, run_id, attempts, not_before_ms, error_kind}`；用尽记 `opendan.run.retries_exhausted`。
- 不读 provider 的 `retry_after_ms`（xllm 单 Q2 定为不往上带）；30 s 起步已覆盖本次 DV 的 60 s 提示的量级。
- **部署顺序（R7）**：xllm 单 P2 把 AICC 标 `retriable` 的 503 / 429 变成可重试之后，在 H1 落地前子会话会每秒重打过载的上游。两单的代码可以分别合入，但部署时 P2 不先于 H1；xllm 单里同步补这一句。

### H2 窗口上限（Q2 = C，即上一单 §8.3）

- 定案：`RunnerOptions.max_context_window = Some(256_000)`；`runner/live.rs` `resolve_context_window` 对 AICC 查到的值与 `default_context_window` 取 `min`，显式 `context_window`（agent.toml / session / behavior 的 `llm_context`）不受限。opendan loader、xagent CLI、测试宿主都走缺省值。
- 效果：256k 的 80% 触发线约 205k，本次 DV 的 226k 尖峰正好会触发一次压缩，顺带验证 H3 第 1 条。
- 未选项备查：A 维持按模型真实窗口（1.05M 在 84 万 token 时才压缩，每轮重发几十万 token）；B Jarvis `agent.toml [llm_context] context_window` 显式写一个值（只管 Jarvis，与上一单 B2“不写”相反）。

### H3 遗留 DV（上一单 §6 未覆盖的部分）

本次 DV 已覆盖：窗口解析（第 1 条的前半）、`<<budget>>` 告警（第 2 条的前半）、预算 100 / 60 的用量（第 3 条）。未覆盖、需要专门构造：

1. 中途压缩：H2 定了上限后，用一个会读大量材料的研究 Todo 让上下文过 80%，看 `context_rewritten` / `compaction`、`compactions` 计数、重写后输入回到窗口 30% 附近，之后若干轮不再压缩；MessageHub 卡片在压缩前后连续（上一单 §6 第 1、4 条）。
2. 工具预算收尾：把 `work_simple` 预算临时设为 8 跑一个 Todo，用完后 `report(is_end)` 成功（上一单 §6 第 2 条的后半）。
3. 墙钟：临时把 `agent.toml [llm_context] timeout` 设为 600，跑一个总时长超过 10 分钟的 plan，确认 plan 不以 `budget_exhausted: Wallclock` 结束；再构造一段连续运行超过 600 s 的 Todo，看 `control_applied{budget_wrap_up}` 与收尾后的真实结局（上一单 §6 第 6 条）。
4. 可重试错误（H1 + xllm 单 P2）：停掉 codex-proxy（`systemctl stop cli-proxy-api`）一段时间再恢复，确认 Todo session 按退避重试、恢复后同一个 run 继续；停得够久时以 `retries_exhausted` 失败。
5. Q6（第 25 轮 226k 尖峰）：对照 AICC `aicc.llm.output` 的 `usage` 与 provider 响应，确认内置搜索是否计入输入（上一单 §6 第 5 条）。

证据放 `/home/lzc/project/work_context_budget_dv_runs/dv_followup/`。

## 4. 待裁决

| Q | 问题 | 选项 | 建议 |
| --- | --- | --- | --- |
| Q1 | H1 的参数 | 退避 base / cap；上限次数 / 时长 | base 30 s、cap 10 min、6 次或 30 分钟（先到为准）。AICC 自己已经做过同模型重试和 fallback，宿主这一层的重试是给“过一会儿就好”的故障，不需要秒级 |
| Q2 | H2 窗口上限 | A / B / C（§3 H2） | **C，256k**。一处管住所有 Agent，`agent.toml` 显式值仍能放开；256k 时触发线约 20 万 token，与本次 DV 的长 run 规模相当（缓存命中高、每个 run 约 $1）。A 让单轮输入可以涨到 80 万 token 才压缩；B 只管 Jarvis |
| Q3 | 子会话之外，`Paused` 的 run 被用户消息唤醒时是否无视退避立即重试 | A 立即重试（用户在场）/ B 仍按退避 | **A**。只对带用户消息的输入批次；定时器、任务结束等自动输入仍按退避 |

**裁决（2026-11-02）**：Q1、Q2、Q3 均按建议——base 30 s / cap 10 min / 6 次或 30 分钟；H2 = C，256k；Q3 = A。

**review 回写（R1–R7，已并入 §3）**：R1 `not_before_ms` 投影进 `SessionStatus`；R2 计数在任何有进展的 outcome 清空；R3 退避不计入墙钟并加测试；R4 host 只管子会话、顶层与 Q3-A 在 runner 按 `is_user_msg` 判断；R5 `last_error.kind = provider_unavailable` 让 `todos.rs observe()` 看见；R6 命名避开 llm_context 的 `max_consecutive_errors`；R7 P2 不先于 H1 部署。

## 5. 实施清单（xllm 单 P2 合入后）

- [x] H1-1 `protocol/state.rs` `SessionState.retry`；`protocol/agent_state.rs` `SessionStatus.retry_not_before_ms` + `session/live.rs` `status()` 投影（R1）；`runner/outcome.rs` 可重试分支写入、其它所有 outcome 清空（R2）；用尽时转为不可恢复（Turn 失败 + `fail_unattended`），`last_error` 按 R5 的形状；`todos.rs` `observe()` 带出 `provider_unavailable`。
- [x] H1-2 host 侧（R4）：`host/mod.rs` `ChildDriver::advanceable` 在 `now < retry_not_before_ms` 时返回 false；`serve_one` 不变（仍按 wake_event / poll 唤醒）。runner 侧：`drive` 进入时 run 处在退避期 → 不推理只处理控制与输入；本批输入含 `is_user_msg` → 越过退避（Q3 = A）。
- [x] H1-3 `runner/mod.rs` `RunnerOptions` 四个参数（R6 命名）；日志 `opendan.run.retry_scheduled` / `retries_exhausted`。
- [x] H1-4 测试（`tests/work_simple_async.rs` 或新 `tests/retry.rs`）：脚本化模型连续返回 `Transient` → 退避期内不再推理、到期重试成功后 `retry` 清空；中间有成功推理的间歇性 `Transient` 不累计（R2）；连续失败到上限 → `last_error.kind = provider_unavailable` + `retries_exhausted`、Todo 观察为 Failed 并带原因（R5）；子会话不再每秒重驱（统计推理次数）且退避中不占并发名额；用户消息批次越过退避、定时器输入不越过（Q3）；暂停 10 分钟后重试不触发 Wallclock 收尾（R3）；宿主重启后计数延续。
- [x] H2（Q2 = C）`runner/mod.rs` `RunnerOptions.max_context_window = Some(256_000)`；`runner/live.rs` `resolve_context_window` 对 AICC 值与缺省值取 `min`，显式值不受限；`tests/context_limit.rs` `a_run_gets_the_window_of_its_model` 加上限用例；cmd_skill `opendan_agent_dev.md` 的“Run limits”一句补上限；`doc/opendan/protocol/Session Directory Protocol.md` 窗口来源一段同步；上一单 §8.3 回写裁决。
- [ ] H3 DV 1–5。
- [x] 文档：`doc/opendan/protocol/Session Directory Protocol.md`（run 的预算一段补“可重试错误的退避与上限”，与 llm_context `max_consecutive_errors` 并列说明（R6）；注册表一段补 `retry_not_before_ms`（R1））、`doc/opendan/xAgent.md`（drive 步骤 h 的可重试错误）；cmd_skill `opendan_agent_dev.md` 若写到会话失败的条件，补一句。
- [x] xllm 单 [xllm-aicc-error-mapping-todo.md](./xllm-aicc-error-mapping-todo.md) 补“P2 不先于 H1 部署”（R7）。
- [x] 全量 `cargo test -p libopendan -p opendan -- --test-threads=1`（§7.3）。

## 6. 关联

- 上一单：[opendan-work-context-budget-fix-todo.md](./opendan-work-context-budget-fix-todo.md)（§8.3 窗口、§6 DV）。
- 先行：[xllm-aicc-error-mapping-todo.md](./xllm-aicc-error-mapping-todo.md)（P1 413 → 重写、P2 可重试判定、P3 上下文超限错误码）。
- AICC：[aicc-request-body-limit-todo.md](./aicc-request-body-limit-todo.md)。
- 后续（不在本单）：`thinking` 块在 Responses 编码时被丢弃却随 kRPC 发送（本次约占上下文的 11%），能否按目标协议在客户端裁掉，需先确认 Claude 等协议对 thinking 签名回放的要求。

## 7. 实施记录（2026-11-02，未提交）

### 7.1 落点

- **H1-1**：`protocol/state.rs` `SessionState.retry: Option<RetryState>`（`attempts`：已排的重试次数；`since_ms`；`not_before_ms`；`applied_input_seq`，见 7.2-2；缺省省略），`RetryState::backing_off`；`protocol/agent_state.rs` `SessionStatus.retry_not_before_ms`，`session/live.rs` `status()` 投影。新模块 `runner/retry.rs`：`on_retryable`（排下一次或判用尽）、`backing_off`、`has_user_msg`、`backoff_error`、`exhausted_line`、`PROVIDER_UNAVAILABLE`（`runner::PROVIDER_UNAVAILABLE` 公开）。`runner/outcome.rs` 可重试分支算出的 `RetryState` 放进 `Next.retry`，由 `commit_kept` 写入 state（其它经 `commit_kept` 的 outcome——保活、上下文上限、中断、pause——都带 `None`，即清空）；`commit_run_end`、交接 `suspend_run`、`end_open_work` 无 run 的两支、`fail_workspace` 清空。用尽 → `last_error = {kind: "provider_unavailable", retries_exhausted: true, attempts, since_ms, at_ms, message: 最后一次错误, recoverable: false}`，之后走原来的不可恢复路径（子 context 返回 `failed`；否则 Turn `failed` + `fail_unattended`）。`todos.rs` `observe()`：这种 `last_error` 的 `error` 与 `summary` 都是 `provider_unavailable: the AI provider stayed unavailable; gave up after N retries over M min (last error: …)`。
- **H1-2**：host `ChildDriver::advanceable` 在 `parent_stopped` 之后看 `retry_not_before_ms`，未到就 false（不 spawn、不占 `max_child_concurrency`）；`serve_one` 未改。runner `drive_inner`：`backing_off = state 在退避期 && 候选里没有用户消息`，与 `paused` 一样并入 `hold_inputs` / `triggered` / continuation / `resumable` 四处；无事可做时直接返回 `DriveResult::Error { last_error + retry_not_before_ms }`（任何 `until` 都一样）。控制（stop / turn stop）在这之前照常处理。
- **H1-3**：`RunnerOptions.retry_backoff_base` 30 s / `retry_backoff_cap` 10 min / `max_retry_attempts` 6 / `max_retry_duration` 30 min（注释与 llm_context `max_consecutive_errors` 区分）；日志 `opendan.run.retry_scheduled {session_id, run_id, error_kind, attempts, not_before_ms}`（warn）、`opendan.run.retries_exhausted {…, attempts, since_ms}`（error）。
- **H2**：`RunnerOptions.max_context_window = Some(256_000)`；`resolve_context_window` 对 AICC 查到的值与 `default_context_window` 都取 `min`，显式 `context_window` 不受限（§3 H2 里写的“与 `default_context_window` 取 min”按效果理解为与上限取 min）。`window.rs` 模块注释同步。
- **文档**：`Session Directory Protocol.md`（版本行、state.json 的 `retry`、Turn 表“继续”、窗口一段补上限、run 的预算补“可重试错误的退避与上限”并与 `max_consecutive_errors` 并列）、`Agent State Protocol.md`（登记表 `retry_not_before_ms` + 变更行）、`xAgent.md`（§9.3 步骤 b / h、Finished 一行、§9.6 Error 一行）、cmd_skill `opendan_agent_dev.md`（Run limits 补 256k；When it fails 补 `provider_unavailable`）、`opendan/web` `types.ts` `retry_not_before_ms`、schema 重生成；上一单 §8.3 回写裁决。xllm 单的 R7 部署顺序已在（§3 P2、§6）。

### 7.2 自定偏差 / 补充

1. **R2 的“有进展”按推理成功算，不只看 outcome**：一个长 run 恢复后成功推理、调工具若干轮再碰 503，中间没有别的 outcome，只按 outcome 清空会把它算成连续。`RoundCounter` 加 `progressed`（`infer` 成功置位，`take_progress` 在每个 outcome 取走一次，与检查点取走的计数分开）；本段有过成功推理时，这次可重试错误从 1 重新起算（`since_ms` 也重置）。
2. **`RetryState.applied_input_seq`**：错误时 live run 已收的批次数；只有 run 之后没再收批次时才算退避期。原因：用户消息越过退避、批次已提交、推理前进程崩溃（`crash.rs` 两例），恢复后这条消息已在 run 里、不在候选里，只看候选会再挡 30 s。退避期内只有用户消息的批次能提交，所以与 Q3 = A 等价，且崩溃安全。
3. **`attempts` 计已排的重试**：第 1–6 次错误各排一次重试（30 s … 10 min，累计 25.5 分钟），第 7 次错误才放弃，与 §3 “6 次累计约 25.5 分钟”一致；`last_error` 多一个 `at_ms`（放弃时刻，Todo 的“M min”用它算，不随观察时间变）。
4. **退避期 drive 返回 `Error` 而不是 `Idle`**：`Idle` 会让 `serve_one` 开始计 `idle_unload`；`Error`（recoverable）让宿主循环保持，与今天可重试错误之后一样。代价：顶层会话退避期内每次被唤醒都会恢复一次 run（重建工具集、不推理），与 `control(pause)` 的会话相同；`serve_one` 自己的 5 s→60 s 退避使到期后的重试最多晚约 60 s（6 次累计最多多约 6 分钟，30 分钟上限可能先到）。
5. **生产形态的子会话不经 `ChildDriver`**：opendan 服务用 `Supervisor`，所有会话（含 Todo 的 work session）都由各自的 `serve_one` 托管，所以那里的退避完全靠 runner 层（上面第 4 条）；`ChildDriver` 的改动管 xagent CLI 的 `run_session`。§2.1 说的“每 ≤ 1 s 重驱”只在 `run_session` 形态成立，`Supervisor` 形态原来是 5 s→60 s。
6. 测试宿主：`tests/goal_session.rs` 的 `host_deps` 把 `retry_backoff_base` 设为 50 ms（`a_run_opportunity_joining_an_open_activation_is_one_of_its_reasons` 靠定时 Activation 在错误后几秒内重试）。`tests/context_limit.rs` 原有的 400k 用例显式不设上限。
7. schema 重生成顺带补上上一单漏生成的 `behavior_config` / `host_meta` / `session_config`（`wrap_up_tools`、`budget_warned`、`budget_wrap_up_turn`、`context_target_ratio`）。

### 7.3 测试

- 新 `tests/retry.rs` 7 例：退避期内各种 `until` 都不推理、到期重试、再失败退避翻倍、同一 run 成功后清空；中间有成功推理的间歇错误不累计（R2）；重试用尽 → `provider_unavailable` + Todo 观察 Failed 带原因（R5），每次换新 runner、计数从 state 延续（重启）；用户消息越过退避并与等着的定时器事件同批、定时器不越过（Q3）；退避长于 `timeout` 不触发墙钟收尾（R3）；`ChildDriver` 下退避中的子会话 1.5 s 只被驱动一次、唯一的并发名额给了另一个子会话，到期才再驱动。`runner/retry.rs` 3 个单测（30/60/…/600 s、6 次后用尽、时长先到）。`context_limit` 的 `a_run_gets_the_window_of_its_model` 加上限用例（1.05M → 256k、200k 不变、缺省 400k → 256k、显式 500k 不受限）。
- 全量 `cargo test -p libopendan -p opendan --no-fail-fast -- --test-threads=1`：45 个目标 492 过、1 失败——`turn_task::the_live_line_is_the_call_intent_in_the_task_and_the_state`（20 ms 轮询 state.json 没看到一闪而过的 `running shell`，与重试无关），单独连跑 3 次都过，属负载下的计时抖动。格式化后 `retry` 与 lib 单测再跑一次全过。

### 7.4 未做

- **H3 DV 1–5**：要 `./build_aios --local-test` 后重建 `buckyos-app-jarvis` 容器，DV 3 临时改 Jarvis `agent.toml` / behavior 预算，DV 4 要 `systemctl stop cli-proxy-api`（与本人 Codex CLI 共用），DV 1 跑一个大研究任务（上次 12 个 run $9.21）。等用户安排。**部署顺序（R7）**：xllm 单 P2 已提交（`1e0de4f2e`），镜像要包含本单 H1 才能部署。
