# OpenDAN：可重试错误的退避与上限、窗口上限裁决、遗留 DV（宿主 TODO）

日期：2026-11-02

状态：**待 review / 裁决**（§4 Q1–Q3）。来源：[Work Session 上下文压缩与工具预算修正](./opendan-work-context-budget-fix-todo.md) 实施后的 DV（2026-11-02，`jarvis.test.buckyos.io` 容器形态，plan `work-20261103-3` 与它派发的 9 个 Todo session）。同一次 DV 拆出的另外两单：AICC 请求体 1 MiB 上限 [aicc-request-body-limit-todo.md](./aicc-request-body-limit-todo.md)（另行处理中）；xllm 的 AICC 错误归类 [xllm-aicc-error-mapping-todo.md](./xllm-aicc-error-mapping-todo.md)（先行，本单 H1 依赖它的 P2）。

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

### H1 可重试错误的退避与上限（依赖 xllm 单 P2）

- 会话状态加 `retry`（`SessionState`，缺省不序列化）：`{ attempts, since_ms, not_before_ms }`，在可重试错误的提交里写入；任何非可重试错误的 outcome（成功、Turn 结束、不可恢复失败）清空。随 state 持久化，宿主重启不会把计数清零，停机时间也计入 `since_ms`。
- 退避：`not_before_ms = now + min(base × 2^(attempts−1), cap)`（Q1 建议 base 30 s、cap 10 min）。`serve_one` 与 `ChildDriver::advanceable` 在 `now < not_before_ms` 时不驱动；`ChildDriver` 不再每秒重驱。用户的输入、stop、resume 照常能进来，驱动后若 run 仍处在退避期，不推理只处理控制与输入。
- 上限：`attempts` 达到 `max_attempts` 或 `now − since_ms` 超过 `max_retry_duration`（Q1 建议 6 次 / 30 分钟）→ 按不可恢复处理：Turn 失败，`last_error` 保留最后一次的错误并加 `"retries_exhausted": true`；无人应答的会话照旧 `fail_unattended`，plan 收到 Todo 失败后自己决定重试。
- 参数放 `RunnerOptions`（`retry_backoff_base`、`retry_backoff_cap`、`max_retry_attempts`、`max_retry_duration`），不进 session 配置。
- 观测：每次退避记 `opendan.run.retry_scheduled {session_id, run_id, attempts, not_before_ms, error_kind}`；用尽记 `opendan.run.retries_exhausted`。
- 不读 provider 的 `retry_after_ms`（xllm 单 Q2 定为不往上带）；30 s 起步已覆盖本次 DV 的 60 s 提示的量级。

### H2 窗口上限（Q2，即上一单 §8.3）

- A 维持：按模型真实窗口（1.05M 在 84 万 token 时才压缩）；
- B Jarvis `agent.toml [llm_context] context_window` 显式写一个值（例如 200k），对所有别名生效，与上一单 B2“不写”相反；
- C `RunnerOptions.max_context_window`（例如 256k），窗口取 `min(AICC 值, 上限)`，对所有 Agent 生效，`agent.toml` 显式值仍可越过它。

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

## 5. 实施清单（xllm 单 P2 合入后）

- [ ] H1-1 `protocol/state.rs` `SessionState.retry`；`runner/outcome.rs` 可重试分支写入、其它分支清空；用尽时转为不可恢复（Turn 失败 + `fail_unattended`）。
- [ ] H1-2 `host/mod.rs`：`serve_one` 用 `not_before_ms` 定下次唤醒；`ChildDriver::advanceable` 在退避期返回 false；Q3 = A 时用户消息批次越过退避。
- [ ] H1-3 `runner/mod.rs` `RunnerOptions` 四个参数；日志 `opendan.run.retry_scheduled` / `retries_exhausted`。
- [ ] H1-4 测试（`tests/work_simple_async.rs` 或新 `tests/retry.rs`）：脚本化模型连续返回 `Transient` → 退避期内不再推理、到期重试成功后 `retry` 清空；连续失败到上限 → `retries_exhausted`、Todo 失败；子会话不再每秒重驱（统计推理次数）；宿主重启后计数延续。
- [ ] H2（Q2 = C）`runner/mod.rs` `RunnerOptions.max_context_window = Some(256_000)`；`runner/live.rs` `resolve_context_window` 对 AICC 值与缺省值取 `min`，显式值不受限；`tests/context_limit.rs` `a_run_gets_the_window_of_its_model` 加上限用例；cmd_skill `opendan_agent_dev.md` 的“Run limits”一句补上限；`doc/opendan/protocol/Session Directory Protocol.md` 窗口来源一段同步；上一单 §8.3 回写裁决。
- [ ] H3 DV 1–5。
- [ ] 文档：`doc/opendan/protocol/Session Directory Protocol.md`（run 的预算一段补“可重试错误的退避与上限”）、`doc/opendan/xAgent.md`（drive 步骤 h 的可重试错误）；cmd_skill `opendan_agent_dev.md` 若写到会话失败的条件，补一句。
- [ ] 全量 `cargo test -p libopendan -p opendan -- --test-threads=1`。

## 6. 关联

- 上一单：[opendan-work-context-budget-fix-todo.md](./opendan-work-context-budget-fix-todo.md)（§8.3 窗口、§6 DV）。
- 先行：[xllm-aicc-error-mapping-todo.md](./xllm-aicc-error-mapping-todo.md)（P1 413 → 重写、P2 可重试判定、P3 上下文超限错误码）。
- AICC：[aicc-request-body-limit-todo.md](./aicc-request-body-limit-todo.md)。
- 后续（不在本单）：`thinking` 块在 Responses 编码时被丢弃却随 kRPC 发送（本次约占上下文的 11%），能否按目标协议在客户端裁掉，需先确认 Claude 等协议对 thinking 签名回放的要求。

## 7. 实施记录

（实施后填写。）
