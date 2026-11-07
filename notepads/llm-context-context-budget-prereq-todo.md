# llm_context / xllm：上下文触发比例可配 + 工具预算收尾模式（先行 TODO）

日期：2026-11-02

状态：**已实施（2026-11-02，未提交）**，单测与 xllm 本机 DV 过，记录见 §5；主 TODO 可以开工（A4 依赖的 `wrap_up_tools` 已在）。§5.3 有一处自定修改和一个待裁决的发现。这是 [Work Session 上下文压缩与工具预算修正 TODO](./opendan-work-context-budget-fix-todo.md)（下称“主 TODO”）§3.1 第 2 条与 §3.2 第 3 条需要的 llm_context / agent_tool 层改动，按约定单列并**先于** libopendan 侧实施（[feedback-llm-context-first]）。两项彼此独立，可并行；各自带单测与 fixtures 更新。都是增量字段，缺省值保持旧行为，不动 [llm_context 冻结](./done/llm-context-freeze-todo.md) 已定的六件事边界（P2 只改“在哪里停”的一个判断条件，不加新的等待或提示语义）。

路径均相对 `buckyos/src/frame/`。

## 1. 背景

- 主 TODO §1 的 DV：work session 上下文涨到 226k 不压缩（宿主没给窗口，属主 TODO），而且 xllm 的触发线是硬编码 75%（`agent_tool/src/xllm.rs:147`），架构设计文档写的是 80%。
- 工具预算用完后 `allow_tool_calls = false`（`llm_context/src/context_loop.rs:1163-1166`），请求里没有任何 function tool；模型发工具调用则 `BudgetExhausted`（`:556-565`，behavior 模式 `:1595`）。宿主的纠正（nudge）要求模型 `report(is_end=true)`，而那一刻 `report` 不在请求里，模型只能把 report 写成文本，会话以 `missing_end_report` 失败。

## 2. P1：`context_yield_ratio`（agent_tool / xllm）

现状：`context_yield_threshold(limits)` 在 `RunLimits.context_window_tokens` 有值时固定返回 `ContextThreshold::Ratio { value: DEFAULT_CONTEXT_YIELD_RATIO }`（0.75）。llm_context 的 `BudgetSpec.context_yield_threshold` 已支持任意比例（`request.rs:146-195`、`context_window.rs:52-66`），不用改 llm_context。

- [x] P1-1 `DEFAULT_CONTEXT_YIELD_RATIO` 0.75 → **0.80**（对齐架构设计 §4.2）。
- [x] P1-2 `.llm_context` 新键 `context_yield_ratio`（f32，`(0, 1]`，越界报配置错误）：解析（`xllm.rs:1429` 的键白名单、`:1479` 附近 `get_f32`）、`MergedConfig` / 覆盖链（`:769`、`:1688`、`:1785`）、`RunLimits` 新增 `context_yield_ratio: Option<f32>`（`#[serde(default, skip_serializing_if = "Option::is_none")]`，随 run.json 保存，`RUN_RECORD_VERSION` 不升：旧记录缺省即 0.80）；`context_yield_threshold` 改为取 `limits.context_yield_ratio.unwrap_or(DEFAULT_CONTEXT_YIELD_RATIO)`。
- [x] P1-3 没有 `context_window` 时 `context_yield_ratio` 无效：配置了比例但没有窗口 → warn 一次（`sources` 里记来源），行为与现在相同（只剩 provider 拒收）。
- [x] P1-4 CLI 覆盖：`xllm run --context-yield-ratio`（与 `--context-window` 并列；若现在没有 `--context-window` 也不补，保持 `.llm_context` 为唯一入口）。
- [x] P1-5 测试：`xllm.rs` 现有 `context_window: 50` 的例子（`:13115`）旁加比例例：`context_window: 100` + `context_yield_ratio: 0.5` → 快照 `request.budget.context_yield_threshold = Ratio 0.5`；缺省 → 0.8；`1.5` / `0` → 配置错误；run.json `config.limits.context_yield_ratio` 回读。
- [x] P1-6 文档：`doc/llm_context/xllm_rust_sdk.md:46`（`context_window` 段的“75%”改“缺省 80%，`context_yield_ratio` 可调”）与 `:61`（`RunLimits` 字段列表补 `context_yield_ratio`）；cmd_skill `opendan_agent_dev.md` 的 `.llm_context` 字段由主 TODO B3 一起改。

## 3. P2：工具预算收尾模式（llm_context）

目标：`tool_iterations_left == 0` 时不是“无工具”，而是只提供一小组收尾工具（宿主指定，典型为 `report`、`ask_user`），调用它们**不计次**、**有限批次**；批次里出现其它工具仍 `BudgetExhausted`。缺省（空集）= 现行为。

### 3.1 协议

`ToolPolicy`（`llm_context/src/request.rs:63`）新增：

```rust
/// Tools still offered once `max_tool_iterations` is used up: the model
/// can only end (report) or hand over (ask the user). Calls to them do not
/// consume iterations. Empty (default) keeps the old behavior: no tools.
pub wrap_up_tools: Vec<String>,
/// How many tool batches may run in wrap-up mode before the context gives
/// up with `BudgetExhausted`. Default 2.
pub wrap_up_rounds: u32,
```

`LLMContextState`（`state.rs:19`）新增 `wrap_up_rounds_used: u32`（`#[serde(default)]`，快照版本 `SNAPSHOT_FORMAT_VERSION` 不升：旧快照缺省 0）。

### 3.2 行为

- `build_inference_request`（`context_loop.rs:1163`）：`tool_iterations_left == 0` 且 `wrap_up_tools` 非空且 `wrap_up_rounds_used < wrap_up_rounds` → `allow_tool_calls = true`，tool specs 过滤为 `wrap_up_tools ∩ 已解析工具`（名字按 `resolve_tool_specs` 的规范名比较；白名单里没有的收尾工具忽略并 warn 一次）。其它情况同现在。
- 工具批次入口（`:556-565`）：`tool_iterations_left == 0` 时，若批次的每个调用都在 `wrap_up_tools` 里且 `wrap_up_rounds_used < wrap_up_rounds` → `wrap_up_rounds_used += 1`，照常派发，**不**扣 `tool_iterations_left`（`:889` 的扣减跳过）；否则 `BudgetExhausted{ToolIterations}` 同现在（`partial` 带本轮文本）。
- behavior 模式（`:1595`，action 调度）：同一规则，按 step 的 `actions` 名判断；`:1689` 的扣减同样跳过。
- 收尾批次里 `report(is_end=true)` → `Done`，照旧；`ask_user` 返回任务 → 照旧 `Yield`（宿主关 Turn）；收尾工具返回结果后模型再发非收尾工具 → `BudgetExhausted`。
- `wrap_up_rounds_used` 随 `tool_iterations_left` 一起被宿主补满时清零：宿主写 `snapshot.state.tool_iterations_left = max` 时同时写 `wrap_up_rounds_used = 0`（主 TODO A4 负责；llm_context 不主动清）。
- 子 context / fork 的 `tool_iterations_left` 同步点（`:675`、`:2037`、`:2052`）同步 `wrap_up_rounds_used`。
- 事件 / 日志：进入收尾模式时发一条 `llm_context.budget.wrap_up { left_rounds }` 日志（与现有 `BudgetExhausted` 日志同级），方便宿主排查。

### 3.3 xllm

- `.llm_context` 不加键（收尾工具集是宿主运行时的事）。xllm 自己的 hosted / CLI run 缺省空集 = 现行为；`xllm run` 若有 `--report` 类显式结束要求，可把 `report` 放进 `wrap_up_tools`（看现状，不强求）。
- `hosted_request` 返回的 `request.tool_policy.wrap_up_tools` 由 libopendan 写（主 TODO A4）。

### 3.4 测试

- [x] P2-1 `llm_context` 单测：`max_tool_iterations = 1`、`wrap_up_tools = ["report"]`：第 1 批 `shell` 扣到 0 → 第 2 轮请求只带 `report` → 模型调 `report(is_end)` → `Done`，`tool_iterations_left` 仍 0、`wrap_up_rounds_used = 1`。
- [x] P2-2 收尾模式里模型调 `shell` → `BudgetExhausted`，`partial` 带文本。
- [x] P2-3 `wrap_up_rounds = 1`：第 1 个收尾批次调 `ask_user`（返回非终结结果）后，第 2 轮请求无工具，再调 `report` → `BudgetExhausted`。
- [x] P2-4 `wrap_up_tools = []`：与现在逐字节一致（现有 budget 测试不变）。
- [x] P2-5 behavior 模式同 P2-1 / P2-2。
- [x] P2-6 快照：旧版本快照（无 `wrap_up_rounds_used`）能 resume；`fixtures` 里的请求 / 快照样例补字段。
- [x] P2-7 `agent_tool` 的 hosted 快照 round-trip 测试补 `wrap_up_tools`。

### 3.5 文档

- [x] P2-8 `doc/llm_context/readme.md` “标准 function_call Loop”一节与 `ToolPolicy` 字段说明：补收尾模式（三句话：剩 0 次只给收尾工具、不计次、最多 `wrap_up_rounds` 批）。
- [x] P2-9 `doc/llm_context/LLM Compress.md` 不涉及；`doc/opendan/xAgent.md` 由主 TODO C5 改。

## 4. 验证

- `cargo test -p llm_context`、`cargo test -p agent_tool`（并发跑多个 cargo 时加 `-j 8`）。
- xllm 本机 DV（跑法见 [llm-context-freeze-todo.md](./done/llm-context-freeze-todo.md) §8.4）：`.llm_context` 写 `context_window: 20000` + `context_yield_ratio: 0.5`，跑一个多轮工具任务，run.json `limit_reason = context` 出现在估算过 10k 时。

## 5. 实施记录（2026-11-02，未提交）

### 5.1 落点

**P1**（`agent_tool/src/xllm.rs`）

- `DEFAULT_CONTEXT_YIELD_RATIO = 0.80`；`.llm_context` 顶层键 `context_yield_ratio`（新 `YamlCtx::get_f32`，非数字 / 越出 `(0, 1]` 报 `XllmError::Config`，exit 2）→ `LlmContextFile` / `MergedConfig`（`sources["context_yield_ratio"]` 记来源文件）→ `RunLimits.context_yield_ratio: Option<f32>`（`None` 不写进 run.json，旧记录按 0.80）。`RunLimits` 因 f32 去掉了 `Eq`（没有使用方依赖它）。CLI（`XllmTask::prepare`）与宿主装配（`prepare_hosted`）两条路径都填；`context_yield_threshold` 取 `unwrap_or(DEFAULT_CONTEXT_YIELD_RATIO)`。
- P1-3：`unused_context_yield_ratio`。CLI 新 run 在 `build_initial_context` 发一条 `RunEvent::Warning`（stderr 可见，注明来源文件）；宿主装配的 run 在 `prepare_hosted` 记 `log::warn!`。行为不变。
- P1-4：`xllm` 现在没有 `--context-window`，按本单不加 CLI 参数。
- 测试：`context_yield_ratio_sets_the_compaction_threshold`（0.5 → 快照 `Ratio 0.5`、run.json 回读；缺省 → 0.8 且不写键；旧 `RunLimits` JSON → 0.8）、`context_yield_ratio_must_be_in_zero_one`（1.5 / 0 / -0.2 / 非数字报错，1 与 0.25 接受）、`context_yield_ratio_without_a_window_only_warns`。
- 文档：`xllm_rust_sdk.md` §4 / §5、`local_llm_context_protocol.md`（配置表、§9.3、`RunLimits` 字段）、`LLM Context 设计.md`（§3.3 宿主值、§11 伪代码、§9 宿主接入一句）里的 0.75 / 75% 全部改掉。

**P2**（`llm_context`）

- `ToolPolicy.wrap_up_tools`（空不写）、`wrap_up_rounds`（`DEFAULT_WRAP_UP_ROUNDS = 2`，等于缺省不写，`lib.rs` 导出常量）；`LLMContextState.wrap_up_rounds_used`（0 不写）。缺省时请求与快照的序列化与之前逐字节一致，快照版本、RunRecord 版本都不升。
- `build_inference_request`（改为 `&mut self`）：`in_wrap_up()` = 剩 0 次 ∧ `wrap_up_tools` 非空 ∧ `wrap_up_rounds_used < wrap_up_rounds` 时，specs 过滤为 `wrap_up_tools ∩ resolve_tool_specs`，`allow_tool_calls = mode != None ∧ 过滤后非空`；否则同现在（P2-4）。每次收尾推理记 `log::info!("llm_context.budget.wrap_up {owner, trace_id, wrap_up_tools, offered, left_rounds}")`（与 `llm_context.context_limit` 同级）；策略不提供的收尾工具每个 LLMContext 实例 warn 一次（behavior 模式下 action 策略允许的名字也算提供）。
- 批次入口：function call `:556` 与 behavior `:1595` 都用 `admits_wrap_up(names)`（全部调用 / action 名在 `wrap_up_tools` 里），不满足照旧 `BudgetExhausted{ToolIterations}`（`partial` 带本轮文本）。名字在这里只比 `wrap_up_tools`，白名单仍由随后的 policy gate 判。
- 计数时机：**派发时**加 1（function call 第 5 步推入 `tool_batch` 前；behavior 第 6 步原本扣额度处），不在入口检查时加：被 `max_calls_per_round` 或 policy gate 拒绝的批次不占收尾批数，与“被拒批次不扣额度”一致（偏差 1）。批次完成处的 `saturating_sub(1)` 在 0 上本来就不扣，只补了注释；behavior 第 6 步改成“0 则计收尾批数，否则扣 1”。
- 同步点：`tool_checkpoint_snapshot`（`:675`）、`run_inner_for_step` 进出（`:2037` / `:2052`）。fork 走 `from_request`，计数从 0 开始（与 fork 补满额度同理），不需要同步。
- xllm：`build_initial_context` 与 `hosted_request` 的 `ToolPolicy` 字面量显式写空集（`hosted_request` 注明由宿主写）；`xllm run` 没有 `--report` 类开关，CLI 的最终答案是无工具文本，额度用完照样能答，不加（§3.3 “不强求”）。
- 测试（`tests/budgets.rs`，新增 6 例；`mocks.rs` 的 `ScriptedRecordingLlm` 记录每次请求广告的工具；测试解析器 `tests/protocol.rs` 认 `<ask_user>`）：
  - P2-1 `used_up_iterations_leave_only_the_wrap_up_tools`：第 2 轮只广告 `report`，`tool_iterations_left = 0`、`wrap_up_rounds_used = 1`。
  - P2-2 `other_tools_in_wrap_up_mode_exhaust_the_budget`：纯 `shell` 与 `report + shell` 混合批次都 `BudgetExhausted`，`partial` 带文本，批次一个调用都不派发、不计批数。
  - P2-3 `wrap_up_rounds_bound_the_wrap_up_batches`：`wrap_up_rounds = 1`，`ask_user` 后第 3 轮不广告工具，再调 `report` → `BudgetExhausted`。
  - P2-4 `without_wrap_up_tools_used_up_iterations_offer_nothing`：剩 0 次不广告工具；缺省 `ToolPolicy` / state 序列化里没有新键。现有 budget 测试不变。
  - P2-5 `behavior_wrap_up_steps_run_only_wrap_up_actions`：`<ask_user>` Step 不计次 → `<report end="true">` Done；Step 内层推理也只广告收尾工具（同步生效）；`<ask_user>` + `<shell>` → `BudgetExhausted`。
  - P2-6 `wrap_up_state_survives_snapshots`：去掉新键的旧快照反序列化为缺省值；宿主把快照 `tool_iterations_left` 置 0 后 resume 直接进收尾模式（主 TODO A7 的墙钟收尾正是这条路）；`wrap_up_rounds_used = 2` 的快照 resume 后不再广告工具。fixtures 不用补：68 个 libopendan 快照样例不含新键、照常加载（`cargo test -p libopendan` 的 fixtures 用例过）；主 TODO A4 写入 `wrap_up_tools` 后重生成即可。
  - P2-7 `host_wrap_up_tools_round_trip_through_the_run_snapshot`（agent_tool）：`prepare_hosted` → `build_request` → 宿主写 `wrap_up_tools = ["shell"]`、剩 0 次 → `RunStore` 存取一致 → `XllmRun::resume` 两轮都只广告 `shell`，批次派发后 `wrap_up_rounds_used = 1`。
- P2-1 的“→ `Done`”：llm_context 里 `report(is_end)` 的结束是宿主报告工具调 `LLMContextInterruptHandle::finish`（libopendan `reports.rs:535`），waist 返回 `Settled`；测试按这条真实路径断言 `Settled { reason: "accepted final report" }`。
- 文档：`readme.md`（名词段的收尾模式三句话、function_call 伪代码的预算检查）、`LLM Context 设计.md`（§3.3 `ToolPolicy` 字段与收尾模式段、§6 `LLMContextState` 字段、§11 两段伪代码、xllm 请求示意）、`xllm_rust_sdk.md` §4、`local_llm_context_protocol.md` §9.2；[xllm TS 运行时 TODO](./xllm-ts-runtime-todo.md) Q7 与 L2 行同步（收尾模式、0.80、budgets 12 例）。
- cmd_skills：`xllm.md` 不列 `.llm_context` 键也不写压缩阈值（指向 `--help`、`status` 与未知键报错），无需修改；`opendan_agent_dev.md` 由主 TODO B3 改。

### 5.2 验证

- `cargo test -p llm_context -p agent_tool --no-fail-fast -- --test-threads=1`：llm_context 105 过；agent_tool 339 过、5 ignored（原有的忽略项）。
- `cargo test -p libopendan --no-fail-fast -- --test-threads=1`（最终代码）：31 个测试目标全过（含 fixtures）；`cargo check -p libopendan -p opendan --tests` 过。
- xllm 本机 DV（buckyos provider → 本机 Zone AICC `llm.chat`），证据在 `/home/lzc/project/work_context_budget_dv_runs/prereq_xllm/`：

| 场景 | 结果 |
| --- | --- |
| t1b behavior loop，`context_window: 20000` + `context_yield_ratio: 0.5`，依次 `cat` 三个 19 KB 文件 | run.json `limits.context_yield_ratio = 0.5`，快照阈值 `Ratio 0.5`；第 3 个文件读完后估算 10,086 ≥ 10,000 → `approaching_window` → 1 次压缩（有摘要请求）→ 历史折进 input、Step 编号接着走 → completed。中途一次 AICC 请求 611 s 超时（`provider_timeout`，exit 3），`--resume` 后继续，与本单无关 |
| t1 function_call，同样配置 | 读完第 2 个文件后估算 11,652 → 按 0.5 让出（P1 正确）；**压缩是空操作，3 次后 `context_capacity` 失败**，见 §5.3 |
| t2 只写 `context_yield_ratio: 0.5`，无窗口 | completed；stderr 一条 `warning: context_yield_ratio (from …/.llm_context) has no effect without context_window…` |
| t3 `context_yield_ratio: 1.5` | exit 2，`config error: …/.llm_context: context_yield_ratio: must be in (0, 1]`，不建 run |

§4 写的“run.json `limit_reason = context`”与实现不符：xllm 的上下文让出不写 `limit_reason`（它只给 `BudgetExhausted`），证据是快照里的 `suspended.which = approaching_window` 与 run.json `compactions`。

### 5.3 自定修改与待裁决（请 review）

1. **自定修改：xllm 压缩目标随窗口。** 原来 `compact_context` 的目标固定 `DEFAULT_AUTO_COMPRESS_TARGET_TOKENS = 32768`；压缩器在历史 ≤ 目标时原样返回（`llm_compress.rs:172`），所以窗口 < 约 41k（0.80；原 0.75 时约 43k）时目标高于触发线，压缩必然空转、3 次后 `context_capacity`。改为 `compaction_target_tokens`：已知窗口时取 `min(窗口 × 30%, 触发线 / 2, 32768)`（`DEFAULT_CONTEXT_TARGET_RATIO = 0.30`，对齐架构设计 §4.2 的 80% → 30%；“触发线一半”只在比例配到 0.6 以下时起作用），无窗口仍 32768。单测 `compaction_lands_below_the_yield_threshold`；t1b 的压缩就靠它（旧目标下估算 10k < 32k，会空转）。文档 `local_llm_context_protocol.md` §9.3、`xllm_rust_sdk.md` §4 已写。
2. **待裁决：xllm function_call 单输入 run 无法压缩。** `llm_compress` 以 user 消息切段，保留开头一段与进行中的一段；xllm 一次性 run 只有一条 user，之后全是 assistant / tool，整段历史只有一段，`select_compress_range` 找不到可压缩范围（`no_compressible_message_range`），原样返回（t1 的 3 次压缩都没有摘要请求，`usage.llm_requests = 2`）。behavior run 的物化历史按 Step 成段，不受影响（t1b）。libopendan 的中途重写走自己的 worklog 窗口（`runner/history.rs` `compact_for_limit` → `read_window` / `compact`），不用 `llm_compress`，主 TODO 不受影响。可选：(a) 压缩器把单段内的 assistant/tool 往返也当可压缩单元（保留最后一个往返）；(b) xllm function_call 的上下文让出改成 provider 拒收前的直接失败、提示拆材料；(c) 维持现状，只在文档写明（已写进 `local_llm_context_protocol.md` §9.3）。建议 (a)，属 agent_tool 压缩器，另起一单。
