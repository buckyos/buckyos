# Work Session 上下文压缩不触发、工具预算耗尽后无法 report 与墙钟上限的修正 TODO

日期：2026-11-02

状态：**已 review、已裁决，可开工（先做先行 TODO）。** 来源：用户在本机 DV（`jarvis.test.buckyos.io`，容器形态，提交 `dd6b62945`）跑了一个深度研究任务，Claude 核对 session 目录、run 记录、日志与源码后写成本单。2026-11-02 用户 review 裁决：压缩按 [OpenDAN Agent Session架构设计.md §4.2](../doc/opendan/OpenDAN%20Agent%20Session架构设计.md) 的 **80% 触发、压回 30%**；其余按 Claude 建议（§4 已回写：Q1 B、Q2 改为按窗口比例、Q3 收尾模式替代宽限、Q4 `work_simple` 100 / `explore` 60、Q5 保持开启、Q6 查清不阻塞）。同日补入**问题三（墙钟上限）**：用户等完整个任务，plan 在 7 个 Todo 全部完成后以 `budget_exhausted: Wallclock` 失败；§1 / §2 / §3.4 / Q7；同日第二轮 review 修订（behavior `[budget] max_wallclock_ms` 已存在、墙钟兼任工具 deadline、`max_total_tokens` 是已有的会话级成本上限、触顶收尾走宿主转换复用 P2 不加 P3；**Q7 用户已按建议裁决：a = A、b = 不设、c = 宿主转换**）。先行 TODO P1 / P2 已在执行。

范围：libopendan 宿主侧（run 的上下文窗口、压缩目标、工具预算的告警与收尾、墙钟的计法与触顶处理）与 Jarvis 包配置，外加几份文档的顺手修正（§5 C）。按用户规则（llm_context / agent_tool 修改单独成 TODO、先于宿主），xllm 的触发比例配置与 llm_context 的收尾模式工具集放在**先行 TODO** [llm-context-context-budget-prereq-todo.md](./llm-context-context-budget-prereq-todo.md)，排在本单之前。

路径均相对 `buckyos/`。

## 1. DV 现象（2026-11-02）

任务：plan `work-20261102-5`「Personal Agent竞品研究」（`plan_simple`），派发 T01 `work-20261102-6`「扫描热门项目与热度证据」、T02 `work-20261102-7`「核查BuckyOS三层能力」（都是 `work_simple`）。目录：`/opt/buckyos/data/home/devtest/agents/jarvis.test.buckyos.io/sessions/`。

| 现象 | 证据 |
| --- | --- |
| 全部 7 个 session 都**没有压缩** | worklog 没有 `compaction`、没有 `outcome kind=context_rewritten`；各 run 的 `run.json` `compactions = 0`、`limit_reason = null`；容器 `/opt/buckyos/logs/opendan/opendan.1.log` 没有一条 `opendan.context.compaction.*` / `opendan.context.rewrite.*` |
| T02 上下文一路涨 | `usage.jsonl` 26 个 Round 的输入：8k → 34k → 65k → 102k → 129k → **226,706**（第 25 轮）→ 148,839（第 26 轮）。run 快照 `state.accumulated` 53 条消息、约 84.5 万字符 |
| 一条消息占了三分之一 | 第 50 条 assistant 消息 27.3 万字符：`thinking` + 多个 `provider_state`（provider 内置网页搜索 `web.run` 的 `open_page` 等动作与加密内容）。T02 的 `tool_policy.disable_capabilities = []`，内置搜索是开着的 |
| T02 失败：`missing_end_report` | `state.last_error = {"kind":"missing_end_report","message":"an explicit final report is required; this session has no input queue"}`；快照 `tool_iterations_left = 0`（`work_simple` 每 Turn 24 次已用完）。最后的回复里模型说“当前可调用工具仅 web.run，不能调用原生 report”，把 report 写成了一段 JSON 文本；runner 的纠正（nudge）要求它 `report(is_end=true)`，它仍然调不了。研究产物其实已写在 session 目录（`BUCKYOS_THREE_LAYER_REPORT.md`、`evidence/*.json`） |
| 第 25 轮的 226k 尖峰 | **未核实**。历史只增不减，第 26 轮却回落到 149k；推测 provider 把那一轮内置搜索的 token 计进了输入 |
| 4 个 Todo session 以 `missing_end_report` 失败 | 后续派发的 Todo 中 `work-20261102-6` / `-7` / `-8` / `-10` 都是这个结局（问题二）；plan 靠重试把 7 个 Todo 全部做完（`control_applied{todo_summary}`：`7 todos: 7 completed`），韧性本身成立 |
| **plan 以 `budget_exhausted: Wallclock` 失败（问题三）** | `work-20261102-5` 的 `state.last_error = {"kind":"budget_exhausted","message":"Wallclock"}`，`run.json` `status = limit_reached`、`limits.timeout_secs = 3600`。时间线（UTC）：run 创建 23:01:10 → 第 14 个 Turn 结束 23:58:58 → plan 空闲等 T07，**00:01:10 过了 1 小时线** → T07 结束事件 00:04:01 打开第 15 个 Turn，同一毫秒 worklog 记 `outcome kind=budget`、`turn_ended status=budget_exhausted`，一次推理都没做。7 个 Todo 都已完成，plan 没写最终报告，会话失败，从创建到结束 62.8 分钟。超时完全来自空闲等待 |

## 2. 现状（源码核对）

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| run 中途压缩的触发 | llm_context 只在三种情况 yield `ContextLimitReached`：估算达到 `BudgetSpec.context_yield_threshold`、估算加完成预留（`max_completion_tokens`，Jarvis 为 8192）超过 `context_window_tokens`、provider 结构化拒收（`ProviderRefused`）。前两项都没设时只剩 provider 拒收 | `llm_context/src/request.rs:146-195`、`llm_context/src/context_window.rs` |
| xllm 的触发线 | `RunLimits.context_window_tokens`（`.llm_context` 的 `context_window`）有值时，阈值固定为窗口的 **75%**（`DEFAULT_CONTEXT_YIELD_RATIO`，硬编码，`.llm_context` 没有可调键）；没有窗口就没有阈值 | `agent_tool/src/xllm.rs:147-157`、`:1429`、`:1479`、`:2167-2173`、`:7104`、`:7505`、`:7955` |
| libopendan 不给窗口 | libopendan 组好 `llm_ctx` 后调 `XllmTask::prepare_hosted`，不写 `context_window`；Jarvis `agent.toml [llm_context]` 只有 `model` / `max_tokens`。DV 里 run.json 的 `config.limits` 只有 `max_tokens / max_tool_iterations / timeout_secs / llm_timeout_secs` | `lib_opendan/src/runner/live.rs:572`、`apps/jarvis_agent/agent/agent.toml:9-17` |
| yield 之后宿主怎么做 | 已有完整路径：`ContextLimitReached` → `rewrite_for_limit`：把 run 历史 flush 进 worklog（`outcome kind=context_rewritten`）→ `compact_for_limit` 摘要（worklog `compaction`）→ 以 `system 前缀 + 一条 history 消息` resume 同一个 run；连续 3 次（`MAX_LIMIT_COMPACTIONS`）仍放不下就暂停 run。测试 `tests/context_limit.rs` 4 例 | `lib_opendan/src/runner/live.rs:41,877-935,1029-1180`、`runner/history.rs:659-710` |
| **目标线是绝对值，不是窗口比例** | 重写时 keep = `history_budget_tokens`（session `prompt.history_budget_tokens`，缺省 `RunnerOptions` 的 24k）右移 `attempt` 位：第 1 次 ≤24k、第 2 次 ≤12k、第 3 次 ≤6k。system 前缀（system prompt、runtime context、工具 schema）不在这个预算内。对 128k 窗口第 1 次只回到约 19%，200k 窗口约 12%，400k 窗口约 6%，与文档的“压回约 30%”不符；研究类 Work 被压得过狠 | `runner/live.rs:1134-1142`、`runner/mod.rs:63`、`runner/history.rs:355-400` |
| 文档里的目标 | 架构设计 §4.2：约 30% 涨到约 80% 触发，一次压回约 30%，不做小幅反复压缩；[LLM Compress.md](../doc/llm_context/LLM%20Compress.md) §（`trigger_ratio` 0.80 / `target_ratio` 0.50）写的是机械压缩目标线 50%、至少释放 30%。**本单以架构设计为准（80% → 30%）**，LLM Compress.md 顺手对齐（§5 C3） | `doc/opendan/OpenDAN Agent Session架构设计.md:200-215`、`doc/llm_context/LLM Compress.md:342-400,683-690,1030,1508` |
| flush 只写文本 | run 历史 flush 时 assistant 消息经 `text_content()` 只落正文与 tool_calls，`Thinking` / `ProviderState` 不进 worklog。所以一旦重写，§1 那条 27 万字符的消息就不在历史里了 | `lib_opendan/src/runner/flush.rs:110-120` |
| history 预算只在开 run 时生效 | 24k 的 history 预算在**给新 run 组装历史**时压缩；多 Turn Work 整个会话是一个 run（第一刀 Q12），走不到 | `runner/history.rs:40-80`、`runner/live.rs:584-590` |
| 模型窗口的事实来源 | AICC 模型目录的 `capabilities.max_context_tokens`（内置缺省 128,000）；路由按它过滤（`min_context_tokens`、`estimated_input_tokens + estimated_output_tokens`，不够则 `context_length_insufficient`）；管理接口有 `resolve_route` | `aicc/src/routing/mod.rs:905-945`、`aicc/src/service/management.rs:63`、`aicc/src/model/mod.rs:2368,2519` |
| 工具预算耗尽后不再给工具 | `allow_tool_calls = mode != None && tool_iterations_left > 0`：剩 0 次时请求里没有任何 function tool（provider 内置能力不受影响）；模型若仍发工具调用则 `BudgetExhausted{ToolIterations}`。耗尽前没有任何告警 | `llm_context/src/context_loop.rs:1163-1166`、`:556-565`、behavior 模式 `:1595` |
| 预算何时补满 | 多 Turn Work 只在**新 Turn 打开**时把 `tool_iterations_left` 补满（`open_turn.is_none()` 且没有 continuation）；nudge 是同一 Turn 里的纠正批次，不补 | `lib_opendan/src/runner/live.rs:798-807`、`runner/ticks.rs:143-170` |
| 收尾失败的分支 | 回复既没结束也没在等工作 → nudge 一次；再一次 → `ExplicitReport` 且 `nobody_answers` → `missing_end_report` 失败 | `runner/outcome.rs:509-525`、`runner/ticks.rs` |
| 各 behavior 的每 Turn 预算 | `work_simple` 24、`explore` 40、`analyze_intent` 40、`plan_simple` 160、`goal` / `chat_route` / `groupchat_route` / `memory_maintenance` 24 | `apps/jarvis_agent/agent/behaviors/*.toml [budget]` |
| 推理前注入点 | `InferenceHook::before_inference` 返回 `Option<Injection>`（消息追加到 `state.accumulated`），现在只做边界检查 / todo 同步 / 观察登记，返回 None；半订阅背景走输入模板的 `{{ background_context }}` | `lib_opendan/src/runner/hook.rs:189-205`、`llm_context/src/deps.rs:329` |
| 内置搜索从哪来、怎么关 | AICC 只对逻辑模型 `llm.chat` / `llm.plan` / `llm.vision` 默认打开 provider 内置网页搜索，其它别名（如 `explore` 的 `llm.code`）默认关闭；请求带 `disable_capabilities: ["web_search"]` 时关闭。会话 run 的请求由 libopendan 自己组：`prepare_hosted` 把 `EffectiveConfig.disable_capabilities` 固定为空，随后 libopendan 调 `hosted_request` 得到 `let mut request`，可以直接改 `request.tool_policy.disable_capabilities`；它随快照保存，经 provider（`ModelDisable`）传到 AICC。`.llm_context` 里没有这个键（要从配置文件读才需要改 xllm） | `aicc/src/routing/mod.rs:410-418,1270-1295`、`agent_tool/src/xllm.rs:7589,7913,5309-5312`、`lib_opendan/src/runner/live.rs:651-669`、对照 `lib_opendan/src/share.rs:197` |
| Responses 的两份表示与估算口径 | AICC 解码 OpenAI Responses 时，每个输出项产出两块：通用块（`Text` / `ToolUse` / `Thinking`）和原样保存该项的 `ProviderState`，所以推理和正文在 `state.accumulated` 里各有两份。§1 那条消息里：加密推理两份共约 24 万字符，正文两份约 1.4 万字符，32 次内置搜索 / 打开页面的动作记录约 1.5 万字符，没有页面正文（拆解见 `/home/lzc/project/work_context_budget_dv_runs/t02_round25_assistant_dump.md`）。**回放不翻倍**：编码时只要有同来源的原生 `ProviderState`，就只回放原始项；`Thinking` 在 Responses 编码里被忽略，它自己那份 `encrypted_content` 编码侧没有任何地方读，只占快照体积。**估算口径**：`Thinking` 只按 summary / text 计（这次都为空），`ProviderState` 按整段 JSON 计。加密推理只算一次，但 base64 密文按普通文本 token 估、正文两份都算，所以估算偏高，80% 线偏早触发（方向偏保守）。重写走 `RewrittenHistory` 时，llm_context 会剥掉两种形态的 thinking（[thinking block TODO](./llm-context-thinking-block-todo.md) §3.1 / §3.3），内置搜索的动作记录不在剥离范围，但重写后的历史从 worklog 文本重建，也不会带过去 | `aicc/src/protocol/openai_responses.rs:816-862,912,1273-1276,1356-1388`、`llm_context/src/context_window.rs:98-121`、`llm_context/src/suspension.rs:270-300` |
| 墙钟上限从哪来 | xllm `DEFAULT_TIMEOUT_SECS = 3600`；`prepare_hosted` 取 `.llm_context` 的 `timeout`，没有就用缺省（Jarvis 没配），写进请求的 `budget.max_wallclock_ms`。llm_context 在**每次推理前**检查 `now − state.started_at_ms > max_wallclock_ms` → `BudgetExhausted{Wallclock}`（function call `check_wallclock_budget` 在 `:399`，behavior 在 `:1575`，判定在 `:1275-1287`）。**墙钟还兼任工具 deadline**：`tool_call_ctx` 把 `started_at + max` 作为 deadline 传给每个工具调用，超时的工具被取消（`CancelCause::Deadline`）后同样是 `BudgetExhausted{Wallclock}`，发生在批次中间。只有 context-limit 挂起的时间不计（`resume_clock` 只在 `RewrittenHistory` / `RewrittenSteps` 两种 resume 里调） | `agent_tool/src/xllm.rs:128,8037,9026`、`llm_context/src/context_loop.rs:399,918-926,960-972,1275-1287,1575`、`llm_context/src/suspension.rs:43-58,87-92` |
| **多 Turn Work 的墙钟按整个会话算** | 多 Turn Work 整个会话是一个 run；新 Turn 从快照恢复（`resume_live_run`，`ResumeFill::ResumeFromMidRun`）时 libopendan 只补满 `tool_iterations_left`，**从不重置 `started_at_ms`**，`ResumeFromMidRun` 也不走 `resume_clock`，所以墙钟从会话开始一路累计，Turn 之间等 Todo / 任务的空闲时间也算在内，1 小时就是会话寿命上限。对照：xllm 自己恢复 run 时重置 `started_at_ms` 并重写 `max_wallclock_ms`（注释“本次命令的执行时长从恢复启动重新计算”）。`work_simple` 等所有多 Turn Work 都受影响。恢复路径核对：`open_state_live_run`（重入 parked run）也经 `resume_live_run`（`live.rs:1204`），所以不经它的只有同一 drive 内的批次续跑 | `lib_opendan/src/runner/live.rs:756,798-807,1204`、对照 `agent_tool/src/xllm.rs:8677-8679` |
| behavior 已能覆盖墙钟，Jarvis 没用 | `[budget]`（`BehaviorBudget`，`deny_unknown_fields`）有 `max_tool_iterations / max_consecutive_errors / max_total_tokens / max_completion_tokens / max_wallclock_ms`；`max_wallclock_ms` 由 `prepare_hosted` 之后的 frozen budget 写进 `request.budget.max_wallclock_ms`（子 context 同样处理），覆盖 `.llm_context` 的 `timeout`。Jarvis 的 behaviors 都没写 `max_wallclock_ms` / `max_total_tokens`。**`max_total_tokens` 同样按整个 run 累计、不按 Turn 重置**（`check_token_budget`），实际上是一个已有的会话级成本上限，只是没人设 | `lib_opendan/src/protocol/behavior.rs:90-103`、`runner/live.rs:330-331,679-690`、`llm_context/src/context_loop.rs:518,1289` |
| 触顶即失败、不给收尾 | `BudgetExhausted`（`Wallclock` / `TotalTokens` / `ToolIterations` 同一分支）→ `run_ended`、`status = LimitReached`、Turn `budget_exhausted`，`fail_unattended`：会话策略 `wait_user_msg = finish_failed`（plan 与 Todo session 都是）时直接 `finished + failed`；子 context 则把失败返回给调用方。墙钟在推理**之前**检查，触顶那一刻连一次写报告的推理都没有。出 outcome 时宿主仍持有 ctx（`lc.ctx.snapshot()` 在 `live.rs:971` 就是这样用的），可以取快照改状态后 resume | `lib_opendan/src/runner/outcome.rs:399-404,730-747`、`runner/live.rs:971` |

结论：压缩的整条路径已经在（llm_context 的 yield、xllm 的阈值、libopendan 的 `rewrite_for_limit`），**差两个输入**：窗口（没有它永远不触发）和按窗口算的目标线（现在是绝对值 24k，压得过狠）。工具预算耗尽是另一回事：24 这个数是早期 function-calling 时代的量级，而且耗尽即摘掉全部工具，nudge 要求的 `report` 在那一刻根本不在请求里。墙钟（问题三）是同一类问题：工具预算已经按 Turn 补满（第一刀 Q11），墙钟却还按整个 run 算，而多 Turn Work 的 run 就是整个会话，于是本该管“超长任务”的 plan 被一个“单次运行失控保护”的 1 小时卡住寿命。

## 3. 修正方案（已裁决）

### 3.1 让 run 知道模型窗口，并按 80% → 30% 压缩（问题一）

1. **窗口来源**（Q1 = B）：libopendan 在 `prepare_hosted` 之前给 `llm_ctx` 写 `context_window`（宿主侧，不改 xllm；xllm 据此设 `context_window_tokens` 与触发线）。取值顺序：
   1. session / behavior / `agent.toml [llm_context]` 显式写了 `context_window` → 用它；
   2. 否则按 run 的模型别名（`llm.chat` / `llm.plan` / `llm.code`）向 AICC 查路由选中模型的 `max_context_tokens`，按别名在进程内缓存（10 分钟），查不到就跳过；
   3. 都没有 → 保守缺省 128k（`RunnerOptions.default_context_window`，可配）。
2. **触发线 80%**：xllm 的 `DEFAULT_CONTEXT_YIELD_RATIO` 从 0.75 改为 0.80，并给 `.llm_context` 加 `context_yield_ratio` 可调键（先行 TODO P1）。硬上限不变：估算 + `max_tokens` 预留超过窗口也会 yield。本单 A1 不依赖 P1（先按 75% 也能验收），但 DV 以 P1 合入后的 80% 为准。
3. **目标线 30%**（Q2）：新增 `context_target_ratio`，缺省 0.30，放 `RunnerOptions` 并给 session config `prompt.context_target_ratio` 一个入口（**不**复用 `compact_ratio`：那是 run 结束后 `maybe_compact` 的触发比例，语义相反）。重写时：
   - `target = ceil(window × context_target_ratio)`；
   - `prefix = estimate(snapshot.request.input 里的 System 消息 + 工具 schema)`，用 llm_context 的 `estimate_messages` / `estimate_request` 同一估算器；
   - `history_budget = max(target − prefix, history_budget_tokens)`，即 24k 是下限不是目标；窗口未知时退回 24k；
   - `keep = history_budget >> attempt` 保留（第 2、3 次再砍半），连续 3 次仍放不下 → 暂停 run（现状）。

   按窗口算的触发 / 目标：

   | 窗口 | 触发 80% | 目标 30% | 现状第 1 次（≤24k） |
   | --- | ---: | ---: | ---: |
   | 128k | 102k | 38k | 19% |
   | 200k | 160k | 60k | 12% |
   | 400k | 320k | 120k | 6% |

4. 窗口确定后不在 run 中途变化；AICC failover 落到更小窗口的模型时，由 AICC 路由的 `context_length_insufficient` 和 provider 拒收兜底（现有 `ProviderRefused` 路径）。
5. 内置搜索的大消息：80% 在 128k 窗口留约 25k 余量，而 §1 那条消息约 70k token，一轮就能从触发线下方直接撞到硬上限。硬上限同样 yield `ContextLimitReached`，重写时 flush 只写文本（§2），所以能恢复；代价是那一轮白跑。写进 §3.3 的观察项，不另做处理。
6. 重写对多 Turn Work 的影响需要用例确认：Turn 编号、`turn_tasks` 绑定、`one_line_status`、卡片的 `usage`（`usage.jsonl` 不受影响）、MessageHub 连续日志（多一条 `context_rewritten` / `compaction` 记录，UI 已当普通事件行显示）。

### 3.2 工具预算：从“配额”改成“告警 + 收尾”的失控保护（问题二）

24 来自早期 function-calling 时代（Cursor 当年“每次请求 25 次工具调用”就是这个量级，后来提到 200）。现在 Claude Code / Codex CLI / Claude Agent SDK 都没有每回合工具次数硬顶，边界是上下文窗口、超时、重复调用检测和用户打断；OpenAI Agents SDK 的 `max_turns` 是失控保护不是工作配额。经验法则：上限是典型用量的 5–10 倍，不按“够用”定。DV 的 T02 用满 24 次只做了一半。

裁决（Q3 / Q4）：

1. **预算值**（B1）：`work_simple` 100、`explore` 60、`plan_simple` 维持 160、其余（UI 对话类、goal、memory_maintenance）维持 24。
2. **告警**（A3）：一个 Turn 里 `tool_iterations_left` 首次 ≤ 20% × `max_tool_iterations` 时，宿主在 `before_inference` 返回一条 `Injection`：一行 `<<budget>>` 文本（“本 Turn 还剩 N 次工具调用；收尾或 `report`”）。每 Turn 一次，记在 `HostMeta`（随快照提交，崩溃不重复），不是独立事件、不进 worklog 正文（按“半订阅只在 input 里”的同一精神：模型视野里的一行提示）。
3. **收尾模式**（A4，依赖先行 TODO P2）：`tool_iterations_left == 0` 时请求不再是“无工具”，而是只带收尾工具集 `report` + `ask_user`；调用它们不计次，最多 2 个批次（`wrap_up_rounds`），批次里出现其它工具 → `BudgetExhausted` 照旧。宿主在 `hosted_request` 之后写 `request.tool_policy.wrap_up_tools`（行为切换重建请求时同样处理，resume 沿用快照）。这样 nudge 要求的 `report` 一定可调用，原方案 B1 的“2 次宽限”**不做**。
4. 预算用完时模型在回复里自己写出的 `{"name":"report",...}` 文本**不**当作 report（不猜文本）。
5. 多 Turn Work 每 Turn 补满预算不变（第一刀 Q11）；nudge 照旧每 Turn 一次。
6. 真正的失控保护是重复调用检测（同工具同规范化参数连续 N 次）和现有 `timeout_secs`，放 §7 后续，不在本单。
7. 成本提醒：预算放到 100 后每轮都重发上下文，100 轮 × 100k 就是千万级输入 token，所以 3.1 的压缩和稳定前缀（缓存命中）比次数本身更要紧；DV 看 `usage.jsonl` 的 `cache_read` 比例。

### 3.3 provider 内置网页搜索（Q5：保持开启）

T02 里内置搜索单条消息 27 万字符，不计工具次数，worklog 里只有 `provider_state`，看不到搜了什么，还让上下文估算跳变。3.1 做完后压缩会兜住体积（flush 丢弃 `Thinking` / `ProviderState`）。保持开启，记录观察：触发线与硬上限之间被单条大消息穿透的次数（日志 `opendan.context.rewrite.*` 的 reason 分布）。

若要对某些 behavior 关掉：开关放在 behavior 配置里（例如 `[model] disable = ["web_search"]`，由 libopendan 的 behavior 解析读取），libopendan 在 `hosted_request` 之后写进 `request.tool_policy.disable_capabilities`；behavior 切换重建请求时同样处理，resume 沿用快照里已有的值。纯宿主侧，不改 xllm / AICC。本单不做，需要时另起。

### 3.4 墙钟：从“会话寿命”改回“单次连续运行的失控保护”（问题三，Q7 已裁决；2026-11-02 第二轮 review 后修订：behavior 键已有、墙钟兼任工具 deadline、触顶收尾走宿主转换不动 llm_context、`TotalTokens` 一并覆盖）

1. **计法**（Q7a）：每次从快照恢复 run（`resume_live_run`）时把 `snapshot.state.started_at_ms` 置为现在，与 xllm 恢复 run 的语义一致，也和补满工具预算在同一处（`live.rs:798-807`）。墙钟于是只量一段连续运行：Turn 之间的空闲、Turn 内等任务 / 子 session 的保活等待都不计；一个 Turn 里不停调工具跑满 1 小时仍会触顶。因为工具 deadline 也从 `started_at_ms` 推导，重置一处两条路径（推理前检查、工具取消）一起修好。所有恢复都重置，含崩溃恢复（停机时间不累计）；上下文上限 resume 的 `resume_clock` 只是再加上挂起时长，与之兼容。不经 `resume_live_run` 的只有同一 drive 内的批次续跑，那里不重置是对的（同一段连续运行）。
2. **上限值**：缺省 3600 s 不变（单段连续运行 1 小时足够当失控保护）；需要时用 `.llm_context` 的 `timeout` 改，behavior 已有 `[budget] max_wallclock_ms` 可按 behavior 覆盖（§2），不新增键；**不建议**用它把墙钟调大来绕过问题，第 1 条才是修法。
3. **会话寿命**（Q7b）：plan / work 默认**不设**寿命上限。成本由工具预算（每 Turn）、压缩（3.1）、token 用量与用户 stop 控制。若以后要上限，用已有的 `[budget] max_total_tokens`（按整个 run 累计、不按 Turn 重置，限的是成本不是时间，比新加 `max_lifetime_secs` 合理），触顶走第 4 条的收尾。本单不加新键，不给 Jarvis 设值。
4. **触顶处理**（Q7c）：单段运行真的跑满 1 小时（或 `max_total_tokens` 用完）时，现状直接 `budget_exhausted` → 失败。建议给收尾，**走宿主转换，不改 llm_context**（先行 TODO 不加 P3）：顶层 run 出 `BudgetExhausted{Wallclock | TotalTokens}` 时，宿主在 `handle_context_outcome` 收口为失败之前取 `lc.ctx.snapshot()`，把 `state.started_at_ms` 置为现在、`tool_iterations_left` 置 0、`wrap_up_rounds_used` 置 0（`TotalTokens` 时再把 `budget.max_total_tokens` 抬到当前用量 + 一个收尾余量，例如 2 × `max_tokens`），注入一行“时间 / token 预算已用完，现在 `report`”的纠正输入（与 nudge 同款、同一 Turn 只做一次），再 resume：先行 TODO P2 的收尾模式就自动生效（只带 `report` / `ask_user`，最多 `wrap_up_rounds` 批次），上限由重置后的墙钟与 2 个批次共同兜住。收尾批次里 `report` 成功即按真实结局结束；收尾后仍 `BudgetExhausted` → 照旧失败。只对顶层 run 做，子 context 照旧把失败返回给调用方。工具 deadline 取消那条路径的快照里被取消的调用已配对为 observation（`pair_rest`），A8 用例确认。
5. 本次 DV 的情形（空闲等 T07 时过线、Turn 15 一打开就失败）在第 1 条之后不会再发生；第 4 条兜的是真正跑满一段的情况。

## 4. 裁决（2026-11-02，Q1–Q7 全部已定）

| Q | 问题 | 裁决 | 说明 |
| --- | --- | --- | --- |
| Q1 | 窗口从哪来 | **B**：宿主按别名向 AICC 查 `max_context_tokens`，`agent.toml` / behavior / session 显式值覆盖，128k 缺省兜底 | 实施前核对 AICC 对 codex-proxy 的 `gpt-6.1-sol` / `gpt-6-astra` 是否填了 `max_context_tokens`（`openrouter.provider.json` 里只有 variant 匹配，没看到窗口），以及宿主用 `resolve_route` 还是模型列表 |
| Q2 | 压缩目标 | **按窗口比例**：触发 80%、目标 30%（架构设计 §4.2）；`history_budget_tokens` 24k 降为下限 | 原来的“24k 还是 48k”作废；LLM Compress.md 的 50% 目标线对齐到 30%（C3） |
| Q3 | 预算耗尽后怎么收尾 | **收尾模式**（原选项 A 的现代形态）：剩 0 次只给 `report` / `ask_user`，不计次，最多 2 批次；改 llm_context，先行 TODO P2 | 原 B1“2 次宽限”不做；B2“耗尽即关 Turn”不做 |
| Q4 | 每 Turn 预算 | **`work_simple` 100、`explore` 60**，`plan_simple` 160 维持，其它 24 维持；加 20% 告警 | 预算是失控保护不是配额 |
| Q5 | 内置网页搜索 | **保持开启**，记录观察 | 3.3 的宿主开关留作后续 |
| Q6 | 第 25 轮 226k 尖峰 | **查清**，写进 §8，不阻塞 | 核对 provider 是否把内置搜索计入 `input_tokens`；若是，卡片 token 显示与阈值估算都偏。已知线索：这一轮在一次响应里做了 32 次服务端搜索 / 打开页面，页面正文不在上下文里，第 26 轮回落到 149k。另：本地估算对加密推理按整段 JSON 文本计、正文两份都算，估算比 provider 计的偏高、触发偏早（偏保守），见 §2「Responses 的两份表示与估算口径」 |
| Q7 | 墙钟（问题三） | a 计法：**A** 每次从快照恢复 run 时重置（只量连续运行段，同 xllm；工具 deadline 随之）/ B 每个 Turn 打开时重置（Turn 内的保活等待仍计）/ C 维持按整个 run 计，只把 `timeout` 调大；b 会话寿命：**不设缺省上限**（要时用已有 `max_total_tokens`）/ 加 `[session.<class>] max_lifetime_secs`；c 触顶：**收尾模式，宿主转换**（取快照、重置墙钟、`tool_iterations_left = 0` 后 resume，复用 P2，不动 llm_context）/ 收尾模式改 llm_context（加 P3）/ 维持直接失败 | **已裁决（2026-11-02）：a = A、b = 不设、c = 宿主转换**，一并覆盖 `TotalTokens`。见 §3.4 |

## 5. 实施清单（先做先行 TODO P1 / P2，再做本单）

### A. libopendan

- [ ] A1 窗口解析：`runner/live.rs` `prepare_hosted` 前写 `llm_ctx["context_window"]`；取值顺序按 §3.1 第 1 条；AICC 查询放在宿主依赖里（`RunnerDeps`，测试可注入），按别名缓存 10 分钟；查询失败只记 warn，不阻塞 run；`RunnerOptions.default_context_window = 128_000`。
- [ ] A2 目标线：`RunnerOptions.context_target_ratio = 0.30`、session config `prompt.context_target_ratio`；`rewrite_for_limit_inner` 按 §3.1 第 3 条算 `history_budget`（system 前缀估算用 llm_context 的估算器，窗口取 `snapshot.request.budget.context_window_tokens`）；日志 `opendan.context.compaction.triggered` 加 `window_tokens` / `target_tokens` / `prefix_tokens` 字段。
- [ ] A3 预算告警：`InferenceHook::before_inference` 在剩余 ≤ 20% 且本 Turn 未提示时返回 `Injection`（一行 `<<budget>>` user 消息）；已提示标记放 `HostMeta`，随快照提交，Turn 打开补满预算时清掉。
- [ ] A4 收尾工具集：`hosted_request` 之后写 `request.tool_policy.wrap_up_tools = ["report", "ask_user"]`（只对 `ExplicitReport` 的会话；behavior `[budget] wrap_up_tools` 可覆盖）；behavior 切换重建请求时同样写；`BudgetExhausted{ToolIterations}` 的宿主处理不变。
- [ ] A5 测试：
  - `tests/context_limit.rs` 加多 Turn Work 例：设一个小窗口，Turn 1 中途超过触发线 → `context_rewritten` + `compaction`，重写后历史 ≤ 30% 窗口（断言 `compact_for_limit` 收到的 keep），同一个 run 继续，Turn 编号、`turn_tasks`、`one_line_status` 正常，后续 Turn 照常补满预算；
  - 窗口解析：显式值优先、AICC 值、缺省兜底、AICC 失败不阻塞；
  - 目标线：窗口 128k / 未知两种，`history_budget` 分别为 38k − prefix 与 24k；
  - `tests/work_simple_async.rs` 加预算例：剩 20% 时出现一次 `<<budget>>` 注入且本 Turn 不重复；用完全部迭代后回复文本 → nudge → 模型在收尾模式调 `report(is_end)` 成功结束；收尾模式里调 `shell` → `BudgetExhausted` → 照旧 `missing_end_report`。
- [ ] A6 墙钟按连续运行段计（问题三，Q7a = A）：`resume_live_run` 在补满预算的同一处把 `snapshot.state.started_at_ms` 置为现在（所有恢复都重置，不限新 Turn）；核对内存续跑路径不重置。
- [ ] A7 预算触顶收尾（Q7c = 宿主转换，依赖先行 TODO P2，不加 P3）：`handle_context_outcome` 对顶层 run 的 `BudgetExhausted{Wallclock | TotalTokens}` 先走收尾：取 `lc.ctx.snapshot()` → `started_at_ms = now`、`tool_iterations_left = 0`、`wrap_up_rounds_used = 0`（`TotalTokens` 再抬 `max_total_tokens`）→ 注入纠正输入（`source = system:runner, event = budget_wrap_up`，同一 Turn 一次，标记放 `HostMeta`）→ `LLMContext::resume(ResumeFromMidRun)` → 发布快照后继续；收尾批次里 `report` 成功即按真实结局结束；收尾后再 `BudgetExhausted` → 现状失败分支。worklog 记一条 `control_applied{budget_wrap_up, which}`。子 context 不做。
- [ ] A8 测试（问题三）：`tests/work_simple_async.rs` 或 `tests/plan_simple_async.rs` 用 `.llm_context` `timeout` 设一个小值（如 3 s）：
  - 会话总时长远超 `timeout`、但每段连续运行都短（Turn 之间等 `sleep` 任务 / 子 session）→ 不触顶，正常 `report` 结束；
  - 单段连续运行超过 `timeout`，两种形态都要：推理前检查触顶（脚本化模型连续多轮慢工具后再推理）、工具 deadline 取消（一个超过 deadline 的 `shell`，批次中间被取消）→ 都进入收尾：快照里 `started_at_ms` 已重置、请求只带 `report` / `ask_user`，模型 `report(is_end)` → 真实结局；收尾里再调 `shell` → `BudgetExhausted` → `budget_exhausted` 失败；
  - `[budget] max_total_tokens` 设小值 → `TotalTokens` 触顶同样收尾；
  - 子 context 触顶仍把失败返回调用方，不收尾；
  - 宿主重启后恢复 run 同样重置（崩溃恢复不累计停机时间）。

### B. Jarvis 包与手册

- [ ] B1 `behaviors/work_simple.toml [budget] max_tool_iterations = 100`、`behaviors/explore.toml` 60。
- [ ] B2 `agent.toml [llm_context]` 不写 `context_window`（Q1 = B，靠 AICC）；注释写明来源与覆盖方式。
- [ ] B3 cmd_skill `opendan_agent_dev.md`：`[budget]` 一句改为“`max_tool_iterations` 计工具批次，多 Turn Work 每 Turn 补满；剩 20% 时收到一行 `<<budget>>` 提示；用完后只剩 `report` / `ask_user` 可调（最多 2 批次）”；`.llm_context` 字段补 `context_window`（来源：显式 → AICC → 128k）与 `context_yield_ratio`（缺省 0.80）、`prompt.context_target_ratio`（缺省 0.30）。按 AGENTS.md 联动规则写明 `ui_core` 等不受影响。
- [ ] B4 cmd_skill `opendan_agent_dev.md`（问题三）：`.llm_context` 的 `timeout` 一句写明“单段连续运行的墙钟上限（缺省 3600 s），每次恢复 run 重新计时，Turn 之间与等待任务的时间不计；会话本身没有寿命上限；用完后与工具预算一样进入收尾模式”；`[budget]` 一句补 `max_wallclock_ms` / `max_total_tokens` 是按整个 run 的覆盖项，一般不设。

### C. 文档（顺手修正）

- [ ] C1 `doc/opendan/protocol/Session Directory Protocol.md` §“上下文上限中途重写”第 2 条：“起点前移到只剩约 `history_budget >> 第几次`”改为“只剩约 `max(窗口 × context_target_ratio − system 前缀, history_budget_tokens) >> 第几次`”；§6 `history_budget_tokens` 的说明补“中途重写时是下限”。
- [ ] C2 `doc/opendan/OpenDAN Agent Session架构设计.md` §4.2：补一句“Work Session 同样适用；触发线 `context_yield_ratio`（xllm，缺省 0.80）、目标线 `context_target_ratio`（libopendan，缺省 0.30）、窗口来自 AICC `max_context_tokens`”，并把 §12 末尾“约 30%→80%→30%”一句指向 §4.2。
- [ ] C3 `doc/llm_context/LLM Compress.md`：`target_ratio` 缺省 0.50 → 0.30（`:354,373,379,397,683-690,1031,1509` 与示例表），“至少释放约 30%”改为“约 50%”；文首加一段注明：llm_context 冻结后压缩由宿主执行（libopendan `rewrite_for_limit`），本文的机械压缩 / LLM 压缩分级是宿主实现的参考，不是 llm_context 的行为；与 [llm-context-freeze-todo.md](./llm-context-freeze-todo.md) 的边界一致。
- [ ] C4 `doc/opendan/Agent RootFS.md` / `doc/opendan/Agent配置改进.md` 的 `[on_llm_message_compress] mode = "context_window_ratio"`、`context_window_tokens` 段：核对是否仍有实现；没有则标注“已由 §3.1 的 `context_window` / `context_yield_ratio` / `context_target_ratio` 取代”，不留两套参数名。
- [ ] C5 `doc/opendan/xAgent.md` `Budget { max_tool_iterations, … }` 伪代码与 `:1095` 的“上下文上限”一句：补收尾模式与告警；`src/frame/opendan/README.md` 无 run 配置描述，不改。
- [ ] C6 `doc/llm_context/readme.md`（先行 TODO 负责 llm_context 侧的 `wrap_up_tools` 文档；本单只核对宿主侧引用）。
- [ ] C7 预算语义（问题三）：`doc/opendan/protocol/Session Directory Protocol.md` 与 `doc/opendan/xAgent.md` 里 run 预算 / `timeout_secs` / `max_total_tokens` 的描述补“宿主每次恢复 run 重新计时；多 Turn Work 的会话没有墙钟寿命上限；`Wallclock` / `TotalTokens` / `ToolIterations` 触顶都先进收尾模式再失败”；grep `timeout_secs` / `max_wallclock` 的其它文档一并核对。

## 6. 验证与 DV

- 单测：先行 TODO 的 `cargo test -p llm_context`、`cargo test -p agent_tool`；再 `cargo test -p libopendan --test context_limit --test work_simple_async -- --test-threads=1`，再全量 `cargo test -p libopendan -- --test-threads=1`、`cargo test -p opendan -- --test-threads=1`（并发跑多个 cargo 时加 `-j 8`）。
- DV（本机 Zone，容器形态需 `./build_aios --local-test` + `uv run src/start.py --skip-update`，或 `./debug_jarvis.sh devtest --agent <agent_id>` 宿主形态）：
  1. 复跑同类深度研究（plan + 2～3 个研究 Todo）。检查：run.json `config.limits.context_window_tokens` 有值且 `context_yield_ratio = 0.8`；输入估算到窗口 80% 时 worklog 出现 `context_rewritten` 与 `compaction`，日志 `opendan.context.compaction.triggered` 的 `target_tokens ≈ 30% 窗口`，`compactions` 计数增加；压缩后同一个 run 继续，`usage.jsonl` 的输入回落到窗口 30% 附近，之后若干轮不再压缩。
  2. 把 `work_simple` 预算临时设小（如 8）跑一个 Todo：剩 20% 时 worklog 之外能在快照里看到 `<<budget>>` 注入；用完后模型调 `report(is_end=true)` 成功，session 以 succeeded / failed 的真实结局结束，不再是 `missing_end_report`。
  3. 预算 100 跑研究 Todo：看 `usage.jsonl` 的 `cache_read` 比例与总输入 token，与 §1 的 T02 对比。
  4. MessageHub：卡片在压缩前后连续（token 不归零、一句话正常），点卡看到的连续日志里有压缩记录。
  5. Q6：对照 AICC 日志 / provider 响应 `usage`，确认第 25 轮尖峰的来源。
  6. 问题三：复跑一个总时长超过 1 小时的 plan（或临时把 `agent.toml [llm_context] timeout` 设为 600 加速），确认 plan 在 Todo 全部完成后写出最终报告、以真实结局结束；worklog 里没有 `outcome kind=budget`，run.json 不是 `limit_reached`。
- 证据放 `/home/lzc/project/work_context_budget_dv_runs/`。

## 7. 关联与后续

- 先行：[llm-context-context-budget-prereq-todo.md](./llm-context-context-budget-prereq-todo.md)（P1 `context_yield_ratio`、P2 收尾模式工具集；**已在执行**。Q7c 走宿主转换，不再需要 P3）
- [MessageHub 运行视图改为以 Work Session 为主](./messagehub-worksession-ui-todo.md)（本次 DV 的环境与卡片）
- [简单 Work Session 全异步化](./opendan-simple-worksession-async-todo.md)（多 Turn Work、同一 run、每 Turn 补满预算 Q11 / Q12、nudge）
- [plan_simple 多 Turn 化与 TodoList 自动调度](./opendan-plan-simple-async-todo.md)
- [llm_context 冻结](./llm-context-freeze-todo.md)（`ContextLimitReached` 与宿主重写的边界）
- 后续刀（不在本单）：重复调用检测（同工具同规范化参数连续 3 次 → 拒绝并提示，llm_context 层）；3.3 的 behavior 级 `disable = ["web_search"]` 开关；`history_budget_tokens` / `context_target_ratio` 的会话模板 `[session.<class>]` 入口。

## 8. 实施记录

（实施后填写。）
