# OpenDAN：Work Session 窗口回到模型上限、压缩质量修正（TODO）

日期：2026-11-03

状态：**已实施（2026-11-03，未提交）：U1b + U2 + U4 + U5 + U6 + C3 + C5 + C6 + W1、文档与 schema，单测过（agent_tool / libopendan / llm_context / opendan），DV 未做；U3 另起一单**，实施记录见 §8。原状态：已裁决（2026-11-03，用户在 §4 批注：Q1 A、Q3 4K、Q4 `llm.summarize`、Q5/Q7/Q8/Q9 按建议、Q6 不做；Q2/Q10 撤销或已定），待实施。 实施顺序按 Q7-B：U1b + U2 + U4 + U5 + U6 + C3 + C5 先行，W1 随后，U3 另起一单。来源与过程：2026-11-03 Zone DV 看压缩 → 源码核对 → 用户原则"只用 llm_compress 一种压缩逻辑" → 探针验证（§7）→ 用户裁决"去掉机械压缩只留 LLM 压缩"、hint 定义（U6）。

范围：libopendan 宿主侧（`runner/window.rs` 的窗口上限、`runner/history.rs` 的历史重建与 LLM 摘要、`runner/live.rs` 的重写次数）与 Jarvis 包配置。llm_context / agent_tool **不改**（触发线 `context_yield_ratio` 维持 0.80）。

路径均相对 `buckyos/src/frame/lib_opendan/src/`，另注明的除外。

## 1. DV 现象（2026-11-03，UTC）

任务：plan `work-20261103-3`（`plan_simple`，gpt-6-astra）派发 T01 `work-4`、T02 `work-5`、T03 `work-6`、T04 `work-7`（`explore`，`gpt-6.1-sol:reasoning-xhigh@codex-proxy`）与 T05 `work-8`。窗口按 H2 封顶 256,000，触发线 204,800（80%），目标线 76,800（30%）。日志：`/opt/buckyos/logs/apps/<jarvis 实例 id>/opendan/opendan.1.log`；每轮实际 token：各 session 的 `.opendan_agent_session/usage.jsonl`。

| 项 | 结果 |
| --- | --- |
| 触发 | 4 次 `llm_context.context_limit {reason: approaching_window}`，估算 205k–209k，**实际 input 167k–182k**（估算高 15–20%）；每次重写 17–23 ms；`run.json` `compactions` 正确（work-5 = 1、work-6 = 2、work-7 = 1）；没有 413、没有 provider 超窗错误；四个会话都交付了 report |
| **压缩后只剩 11k–16k** | 目标是 76.8k。`work-5` 压缩后的快照（`runs/…-1c4591/snapshots/0123.json`）里 `<session_history>` 只有 31 KB 文本；`work-6` 第 2 次压缩后 16.2k。相当于压回窗口的 **5%**，不是 30% |
| **LLM 摘要一次没跑** | 全部会话没有 `summary.json`，worklog 没有 `compaction` 条目，日志没有 `opendan.context.compaction.started`。原因见 §2.1 |
| 截掉的是最有价值的内容 | function call 模式 assistant 文本为空，模型读到的东西只在工具结果里，旧结果一律截到 280 字；中途 `report(is_end=false)` 写下的发现（T02 三次进度报告各 1–3 KB）在 `render_calls` 里被截到 280 字（全文模式也只有 400 字）；保留全文的"最后 2 个 response"在 work-5 恰好是一整页 arXiv 搜索结果 |
| 模型压缩后重读文件 | 压缩后第一步的 intent：work-5「恢复已完成的论文与复核证据」、work-6「恢复研究进度，核对已保存的版本元数据」，随后 `sed -n` 重读同一批 PDF 文本。work-5 压缩后 9 轮（约 2 分钟）回到 123k，work-6 6 轮回到 110k、13 分钟后再次触发 |
| 成本 | work-5 56 轮 $1.83、work-6 56 轮 $1.80（名义值，codex-proxy 为订阅）；缓存命中 93% 左右，重读的钱不多，耗的是轮数、工具预算和时间 |

压缩前后各轮实际 input（`usage.jsonl`）：

| 会话 | 触发时刻 | 压缩前实际 | 压缩后 | 之后 |
| --- | --- | ---: | ---: | --- |
| work-5 | 06:26:27 | 175,015 | 13,469 | 9 轮回到 123k，06:36 交付 |
| work-6 | 06:26:29 | 182,274 | 11,307 | 6 轮回到 110k |
| work-7 | 06:32:29 | 170,228 | 13,494 | 涨到 71k 后交付 |
| work-6 第 2 次 | 06:39:09 | 166,694 | 16,218 | 收尾交付 |

## 2. 现状（源码核对）

### 2.1 为什么 LLM 摘要没跑、压缩后只剩 5%

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| 重写流程 | `rewrite_for_limit_inner`：先把 run 的历史 flush 进 worklog，算 `LimitLine`（target = ⌈窗口 × 0.30⌉，history_budget = max(target − 前缀, 24k)），`keep = history_budget >> attempt.min(8)`，调 `compact_for_limit` | `runner/live.rs:1232-1300` |
| **第一次就砍半** | `attempt` 从 1 起算，所以第一次重写 `keep = history_budget / 2`（256k 下约 35k），第二次 1/4 | `runner/live.rs:1036,1289` |
| `compact_for_limit` | 先 `read_window(keep)`：按**机械渲染**从新往旧读 worklog，算 token，装得下就 `reached_start = true` → **跳过 `compact()`**（LLM 摘要），再 `build_history(history_budget)` 重建历史消息 | `runner/history.rs:697-760` |
| 机械渲染 | `MechanicalCompress` 缺省：`recent_full_responses = 2`（只有最新 2 个 response 及其后的条目全文，上限 `max_result_chars = 4096`），更老的条目一律截到 `summary_chars = 280`；工具调用参数全文模式 400 字、否则 280 字 | `protocol/config.rs:360-404`、`runner/history.rs:134-145,149-260` |
| 结果 | 一个 50 轮的 run 机械渲染后只有约 10k token，远小于 keep（35k），于是"装得下"——永远到不了 LLM 摘要；重建出来的历史也就是这 10k，不会去填满 76.8k 的预算。**`history_budget` 在今天的默认配置下是一个够不着的数** | — |
| **机械渲染不认 AgentToolResult 协议** | 两套"机械压缩"并存。agent_tool `llm_compress`（xllm 自己的 run / OneShot）设计上解析 tool result 里的 AgentToolResult JSON 信封，按年龄降到 `Medium` / `Min` 级重渲染（summary / title），只处理 `AiRole::Tool` 消息、不碰 user / assistant；**但 F22 之后这条路径已失效**：进历史的文本是 `observation_text` + hints（`xllm.rs:4476-4491`），信封只在 `Observation.host`，`try_decode_agent_tool_envelope` 永远匹配不上，≥ 2048 字的旧结果走 `plain_text_truncate_v1`（sha256 + "rerun or reread"）。libopendan 这套则在 flush 时把 `ToolResult.content` 的文本（模型看到的 `observation_text`）原样写进 worklog `action_result.result`，信封留在 `Observation.host` 里没有落盘；机械渲染只有 `trunc`（取前 N 个字符加 `…`），对 result / assistant / user input / 工具参数一视同仁。`write_file` 的结果本来有现成 summary（"created fetch.py, wrote 1893 bytes"），shell 结果按 Min 级是一行命令 + 状态，都比 280 字前缀有用 | `agent_tool/src/llm_compress.rs:460-515,638-648`、`runner/flush.rs:130-150`、`runner/history.rs:100-106` |
| 跳过时只记 debug | `compaction.skipped {skip_reason: within_keep_budget}` 是 `log::debug!`，INFO 日志里看不出摘要为什么没跑 | `runner/history.rs:739` |
| LLM 摘要器 | `LlmSummarizer`：用 run 自己的模型（这次是 `reasoning-xhigh`），一次请求把 `previous + segment` 全部送进去，`max_completion_tokens = 2048`；segment 是 `recent_full_responses = u32::MAX` 的全文渲染。窗口越大，一旦真的走到这里 segment 越大（1.05M 窗口下可达 50 万 token），2048 的摘要装不下 | `runner/history.rs:441-487,531-545` |

### 2.2 重写次数的累计

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| 上限 3 次 | `run_compacting` 的 `attempt` 在**一次输入批次的 `run()` 循环内**累计，中间推理有进展也不清零；注释写"连续 3 次"，实现是"累计 3 次"。第 4 次碰线 → `opendan.context.rewrite.paused {pause_reason: compaction_attempts_exhausted}`，run 暂停、`FinishKind::ContextLimit` | `runner/live.rs:43,981-1047`、`runner/outcome.rs:882-889` |
| 影响 | 多 Turn Work 一个批次往往就是整个任务（work-6 第 2 次压缩已是 attempt = 2，keep 只剩 1/4）。今天没触发第 4 次，但一个更长的任务会。与 H1 退避"推理成功才算有进展"（`RoundCounter.progressed`）是同一类判断 | `runner/rounds.rs` |

### 2.3 窗口

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| 上限 | `RunnerOptions.max_context_window = Some(256_000)`，`resolve_context_window` 对 AICC 查到的值取 `min`；显式 `context_window`（agent.toml / session / behavior 的 `llm_context`）不受限 | `runner/mod.rs:183,254`、`runner/window.rs`、`runner/live.rs` |
| 模型真实窗口 | AICC `driver_metadata/models/openai.model.json`：`gpt-6.1-sol` / `gpt-6-astra` `max_context_tokens = 1_050_000`；80% 触发线 84 万 | — |
| 请求体上限 | AICC 已改 32 MiB（[aicc-request-body-limit-todo.md](./done/aicc-request-body-limit-todo.md)，已提交，2026-11-06 DV 通过）；今天 209k 估算（约 1 MB 请求体）没有 413，说明 DV 镜像里已带这份修改，但要确认部署镜像也带（`aicc/src/protocol/transport.rs:48`、`openai_chat_completions.rs:31`） | `buckyos/src/frame/aicc/` |
| 估算偏差 | llm_context 估算比 provider 实际高 15–20%，1.05M 下 84 万估算 ≈ 70 万实际 | — |
| 设计文档 | §4.2：80% → 30%，Work Session 同样适用；§11.2：Work Session "可以围绕任务保留较多执行信息"，压缩时"应保留任务约束、已验证结论、当前执行状态与产物引用" | `doc/opendan/OpenDAN Agent Session架构设计.md:200-216,819` |

结论：今天看到的"压缩后重读"不是窗口太小，而是压缩没做它该做的事——预算 76.8k 只用了 13k、LLM 摘要从未介入、模型自己写的进度报告被截成 280 字。这个缺陷和窗口大小无关，窗口放大只会放大它：1.05M 下的那一次压缩会从 70 万砸到 2 万。所以先修压缩，再放开窗口。

## 3. 方案（建议，待裁决）

### W 窗口回到模型上限

- **W1** `RunnerOptions.max_context_window` 缺省改为 `None`（跟随 AICC 查到的模型窗口；查不到仍是 `default_context_window` 128k）。**保留旋钮不删**：xagent CLI、测试宿主、以及以后想限成本的 Agent 可以设；`agent.toml [llm_context] context_window` 的显式覆盖不变。Jarvis 不写。
- **W2** 前置核对：部署镜像里 AICC 请求体上限是 32 MiB（§2.3）。1.05M 窗口的 84 万估算 token 约 4 MB 请求体，1 MiB 会直接 413。
- **W3** 代价要写进文档（§6），不是拦着：每轮 prefill 50–80 万 token 的延迟；Codex 额度与用户本人 Codex CLI 共用；快照文件按上下文等比例涨（175k 时 run 快照 1 MB，每次工具调用写两份）。
- **W4** 不改触发线 0.80、不改目标线 0.30：1.05M 下压缩后历史约 31 万 token 原始记录，正好对应设计文档 §11.2 "Work Session 保留较多执行信息"。

### U 只保留一种压缩逻辑：llm_compress（用户原则，2026-11-03）

用户裁决方向：**系统只用 agent_tool `llm_compress` 这一种压缩逻辑**，libopendan 不再自带一套机械截断。源码核对后的现状（§2.1）：llm_compress 的协议分级路径在 llm_context 冻结（F22）之后也已经失效——工具结果进历史的文本是 `observation_text` + hints（`agent_tool/src/xllm.rs:4476-4491`），信封只在 `Observation.host`，而 llm_compress 收到的是 `Vec<AiMessage>`，`try_decode_agent_tool_envelope` 要求文本是 AgentToolResult JSON，永远匹配不上；≥ 2048 字的旧结果走 `plain_text_truncate_v1`（只留 sha256 + "rerun or reread"），小的原样不动。所以统一不是"libopendan 改调 llm_compress"一句话，分三步：

- **U1（撤销）** 原"信封跟着历史走"是为机械分级服务的；机械压缩去掉后不需要。worklog `action_result` 不加 `host`，llm_compress 输入仍是 `&[AiMessage]`。
- **U1b llm_compress 的分段按"轮"而不是按 user 消息（§7 实验 1；保留）。** `build_message_spans` 只在 `AiRole::User` 处切 span，function call 的一个 run 中途没有 user 消息 → 整个 run 是一个 active span → `select_compress_range` 没有候选，`desired_tool_result_level` 的年龄（后面的 user 消息数）恒为 0；四个 DV run 都原样返回。xllm 自己的 `compact_context` 因此三次空转后 `context_capacity` 失败。改为：一轮 = assistant（含 tool_calls）+ 它的 tool 结果（+ 紧随的 user 输入），head / hot tail / 年龄都按轮算；behavior 模式按 step。
- **U2 libopendan 的渲染器退化为忠实重建，不再是压缩器。** `runner/history.rs` 的 `read_window` / `render_entry` 改为 worklog → AiMessage pair（assistant 调用 + tool 结果 + 信封，user 输入、report 原样），不截断；`MechanicalCompress`（`recent_full_responses` / `summary_chars` / `max_result_chars`）与 `trunc` 删除，`drop_kinds` 保留（审计类条目本来就不进历史）。超预算的判断与压缩全部交给 llm_compress：目标预算 = `LimitLine.history_budget`（30% 线减前缀），head keep / hot tail / 机械降级 / LLM 摘要都是它的规则。
- **U3 llm_compress 的输出成为 session 摘要的持久形态。** summary.json 从一段 `history_summary` 文本改为 llm_compress 产出的 compressed pair（含 `MechanicalCompressedMeta` / summary 元数据、稳定边界）+ `start_seq/start_offset`；下一个 run 重建历史 = compressed pair + 之后的原始 pair，而不是一整块 `<session_history>` 文本。这是 Session Directory 协议改动，连带：UI 续聊历史（`build_chat_history`）、behavior 的 `RewrittenSteps` / `<<step_history>>`（llm_compress 已有 `try_fold_agent_loop_history_block`）、Goal 恢复结构（架构设计 §11.3 Current Skills 进靠前稳定区）要逐一对账。
- **U4 llm_compress 只留 LLM 路径（用户裁决 2026-11-03）：滚动 summary + 按需选范围 + 循环到目标 + 尾巴按预算。** 删除机械阶段的全部代码：`try_mechanical_compress`、`desired_tool_result_level`、`mechanically_compress_tool_result(_text)`、`try_fold_agent_loop_history_block` 及其辅助、`MechanicalCompressedMeta` / `MECHANICAL_COMPRESS_META_MARKER`、`MECHANICAL_*` 常量、`try_decode_agent_tool_envelope`、`plain_text_truncate_v1`；`AgentHistoryShowLevel` 的 Min / Medium 渲染仍归 StepRecord 用，不在压缩里用。源码核对（2026-11-03）与设计讨论的两处偏差：(1) 已有 compressed pair 是稳定边界，新压缩从它之后开始、再插一个新 pair，summary 块越积越多，不是"上一个 summary + 要压缩的消息对 → 新 summary"；(2) 一次调用只压 `MAX_LLM_COMPRESS_INPUT_TOKENS = 16_000` 的输入、做一次就返回。改法：(a) 滚动合并——范围 = 上一个 compressed pair（若有）+ 它之后到尾巴之前的原始轮，输出一个新 compressed pair 替换两者，历史里始终只有一块 summary（Q8）；(b) 尾巴按预算：`tail_budget = target − system − head − summary_budget − hint 上限`，从最新一轮往回填满（30% / 256k 下约 60k ≈ 最近 10–15 轮，Q9），其余全部进摘要范围，不设 16k 硬上限；(c) 输入超过摘要模型窗口一半时分块折叠（每块 ≤ `summary_chunk_tokens` 建议 100k，前一块摘要作为下一块的 previous），同一调用内循环到 `total ≤ target` 再返回；(d) summary 上限 2048 → 4096（§7 实测 4–5k 估算 token，上限按模型 `max_completion_tokens` 给 6144 更稳）；(e) 摘要模型由调用方传（接口已有 `model_alias`），libopendan 与 xllm 都传 `llm.summarize`（Q4；agent_tool 现有的 `llm.summary` 一并改名）；(f) 摘要提示词按 §7 探针的版本（有记忆系统、可以大胆、保留文件路径与已验证结论、合并 previous）。压缩后的历史 = `system + head（开场输入 + 第 1 轮）+ [summary] + hint（U6 逐字槽 + C6 固定一行）+ 原始尾巴`，尾巴里没有被改写过的消息。 其它调用方：`llm_understand_media.rs:339` 也调 `compress`（父历史提纯快照），机械阶段删除后它只走 LLM 路径，行为变化要在该模块的测试里确认。
- **U5 模型自己写的东西进摘要时不丢。** 摘要提示词明确要求保留 `report(is_end=false)` 的内容、工具调用 `intent` 里的判断和 `ReportDelivery` 文本；渲染给摘要器的段落里这些字段全文（上限 `report_chars` = 4096，Q3）。
- **C3 重写次数按"连续"算（libopendan，保留）。** `run_compacting` 的 `attempt` 在一次推理成功（`RoundCounter.progressed`）后清零；上限 3 次是"连续 3 次放不下"。第一次重写目标 = `history_budget`（不砍半），第 2 次起 `>> (attempt − 1)`。
- **C5 日志（保留）。** `rewrite.completed` 加 `history_tokens_after`、`summarized: bool`、`summary_tokens`；llm_compress 的压缩范围 / 策略 / token 变化（它的设计目标 10）以 INFO 进 opendan 日志。
- **U6 hint = 不可压缩的关键信息槽（用户 2026-11-03 定义）。** hint 不是运行时提示，而是摘要旁边一条**逐字保留**的车道：用户说过的电话、地址、账号、确切的 id / URL / commit SHA / 路径、数值约束等，摘要可以大胆丢细节，这些不能有损。实现：llm_compress 的摘要输出 JSON 加 `verbatim: [string]`（已有 `important_entities` 的语义收紧为"必须逐字保留的字符串"）；hint 块 = 累积的 verbatim 列表，滚动合并时**不再经过摘要器**（摘要器只能追加，不能改写已有条目），去重、设上限（建议 2k token，超出的最老条目转 `memory_candidates`）；随 summary.json 持久化（U3），渲染在 summary 之后、尾巴之前：`<history_summary>…</history_summary>\n<keep_exact>…</keep_exact>`。与 memory 的关系是互补：memory 是长期保证但召回不一定在需要的那一轮注入，hint 是上下文内保证，同时也是现成的 memory 候选。
- **C6 运行时固定一行（可选，Q5）。** 在 hint 块之后加固定一句 `Everything before this point was compressed; files in the session directory are intact — re-read only what you need`。不是 hint 的一部分，不进 worklog。

被 U 取代的原建议（备查）：C1 机械渲染按预算填满两档、C2 report 参数不截、C7 截断档按信封 summary 渲染、以及 U1 信封随历史走——都是在机械压缩上修补，用户 2026-11-03 裁决去掉机械压缩后全部不做。

### 不做

- 不改 llm_context（触发线、估算器、Observation 形状）。估算偏高 15–20% 只观察。
- 不给 Work Session 另立一套比例（§11.2 说的"保留较多"由 1.05M × 30% 自然得到）。
- 不动 `maybe_compact`（run 结束后按 `compact_ratio` 的压缩）的触发语义，只把它的执行换成 llm_compress。

## 4. 裁决（2026-11-03，用户批注）

| # | 问题 | 裁决 |
| --- | --- | --- |
| Q1 | 窗口上限 | **A**：跟随模型（W1，`max_context_window` 缺省 None，旋钮保留）；U 先于 W 落地 |
| Q2 | 信封通道 | 撤销（机械压缩去掉后不需要） |
| Q3 | U5 的 `report_chars` | **4096** |
| Q4 | 摘要模型 | **`llm.summarize`**（AICC 已有该别名：`aicc/src/routing/mod.rs:2513`、`service/model_defaults.rs:617`、`doc/aicc/aicc_router.md`）。注意 agent_tool 里 `llm_explore.rs:55` / `llm_understand_media.rs:37` 用的是 `llm.summary`，实施时统一成 `llm.summarize` |
| Q5 | C6 固定一行 | 加，一行，不进 worklog、只在重建的历史头部 |
| Q6 | 256k vs 1.05M 对照 DV | **不做**（用户：模型窗口只会越来越大） |
| Q7 | U3 分步 | **B**：先 U1b + U2 + U4（摘要文本仍写回 `history_summary`），U3 另起一单 |
| Q8 | 滚动合并 vs 堆叠 | **合并**；`LLM Compress.md` 目标 7 改为"原始对不被二次摘要，summary 块可滚动合并" |
| Q9 | 尾巴大小 | **B**：由预算算，填满 target 余量 |
| Q10 | 中途重写只走 LLM 摘要 | 已裁决：机械压缩整体去掉 |

## 5. 测试与 DV

- 单测：llm_compress（`agent_tool`）U1b FC 历史按轮分段、找到范围；U4 范围按 need 选、同一调用循环到目标以下、滚动合并后历史只有一块 summary、尾巴按预算填、分块折叠；U5 report / intent 进摘要段落全文；U6 verbatim 条目跨两次滚动合并逐字不变、超上限最老条目转 memory_candidates；机械相关测试删除。libopendan（`tests/context_limit.rs`）U2 重建的轮与原始 tool call / result 一一对应；压缩后历史 token 落在 `[0.6, 1.0] × history_budget`；C3 推理成功后再次碰线 attempt 从 1 起；摘要 fixture 用 mock LlmClient。
- DV（容器形态，`PLAN_DO_CASE_ROOT=… PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py`，或直接复用这次的研究任务提示）：
  1. 256k + U 修正：压缩后 `usage.jsonl` 下一轮 input ≥ 60k；压缩后第一步的 intent 不是"恢复/核对已完成…"；日志有 llm_compress 的压缩范围 / 策略记录，机械降级与 LLM 摘要各自的 token 变化可读。
  2. 1.05M（W1）：同一任务跑一次，确认 84 万估算 token 的请求不 413、第一次压缩的摘要分块与尾巴预算正常（不做 256k 对照，Q6）。
- 部署顺序：U1b + U2 + U4 + C3 先于 W；W1 不先于 AICC 32 MiB。

## 6. 文档

- `doc/opendan/OpenDAN Agent Session架构设计.md` §4.2：窗口"来自 AICC 路由选中模型的 `max_context_tokens`"后补一句上限旋钮缺省不设、以及代价（W3）。
- `doc/opendan/protocol/Session Directory Protocol.md` §7：重建规则、压缩交给 llm_compress（形态 system + head + summary + hint + 尾巴）；U3 落地时 summary.json 形态。
- `doc/llm_context/LLM Compress.md`：去掉"机械压缩 / LLM 压缩分级是宿主实现的参考"一句，改为唯一实现；§2.2 机械压缩、设计目标 4 / 6 / 7（Message Pair 边界、优先机械、Compressed Pair 稳定边界）按裁决改写为：只有 LLM 压缩、summary 滚动合并、尾巴按预算；补分块与 head/summary/hint/tail 形态。 hint 的定义（U6）写进 §3 设计目标。
- `doc/llm_context/LLM Context Switch (histoiry samples).md`"机械压缩与 StepRecord 摘要"一节对齐。
- [opendan-dv-followup-todo.md](./opendan-dv-followup-todo.md) H2、[opendan-work-context-budget-fix-todo.md](./opendan-work-context-budget-fix-todo.md) §8.3 第 1 项：加一行指向本单（裁决改为跟随模型）。

## 7. 实验记录（2026-11-03，DV 现场离线探针）

探针：`src/frame/agent_tool/examples/compress_probe.rs`（保留，U 的测试可从它长出来）。从 worklog 重建 function call 历史（system 取自 run 快照），对 work-5 / 6 / 7 压缩前的那段（到第一个 `context_rewritten`）：(1) 调真实 `llm_compress::compress`（mock 摘要器）；(2) 原型 `system + head + [summary] + hint + 尾巴 2 轮`，摘要用 gpt-6.1-sol 直连本机 codex-proxy（`http://127.0.0.1:8317/v1`，`OPENAI_API_KEY` 取 `/opt/cli-proxy-api/keys.env`）。摘要原文在 scratchpad `work-20261103-{5,6,7}.summary1.md`、`work-20261103-6.summary2.md`。

1. **现有 llm_compress 对 FC 历史原样返回**（四个 run 都是 `unchanged=true`、摘要器 0 次调用）：见 U1b。
2. **机械压缩"总能成功"**：工具结果占 token 94%（work-5：tool 165k / assistant 8k / system 2k / user 0.7k）。中段结果全换 Min 行后：work-5 22k（13%）、work-6 15k（9%）、work-7 35k（21%），任何 ≥35k 的目标都在机械阶段停下。见 U4 策略改动。
3. **真实摘要**（中段输入 → 耗时 → 摘要大小 → 重写后总量，尾巴 2 轮）：work-5 163k → 168 s → 5.3k → 18.5k（11%）；work-6 166k → 143 s → 5.0k → 10.6k（6%）；work-7 140k → 175 s → 4.6k → 27.9k（17%）。work-5 摘要内容：任务与约束、每个产物文件的路径与内容、七个来源各自核验过的数字/版本/SHA、已交三次中间 report 的 id、下一步，并明确"足够交付，不必再扩检"——对症今天的重读。
4. **滚动合并**（work-6 第 2 段：上次摘要 5k + 新中段 137k → 101 s → 3.7k）：抽查 10 个关键事实（版本号、commit、协议语句）留下 8 个，丢了 `python-1.24.0`、"SendMessage MAY be idempotent"。有损叠加可见、有界（Q8）。
5. 代价：每次摘要一次全量输入（≈ 一轮满上下文推理）、2–3 分钟在 run 关键路径上；1.05M 窗口下中段可达 70 万，U4(c) 分块是硬要求。
6. 探针偏差：第 2 段的 head 取的是重写后第一轮（`ls; cat`，9k），真实算法 head 仍是原开场；尾巴固定 2 轮（Q9 建议改预算）。

## 8. 实施记录（2026-11-03，未提交）

### 8.1 落点

| 项 | 实现 |
| --- | --- |
| U1b / U4 / U5 / U6 | `agent_tool/src/llm_compress.rs` 重写：`segment` 按轮分段（assistant + tool 结果 + 紧随 user；开场 user 并入第一轮；最终回复后的 user 开新轮；`<<last_step_action_results` 结果消息并入 step 轮）；`choose_head`（第一轮 ≤ target × 0.25，否则只留开场 user）；尾巴预算 = target − 前缀 − head − 6144 − 2048 − note；范围 = head 之后、尾巴之前的 previous summary + 全部原始轮；`fold` 按 `summary_chunk_tokens`（100k）分块滚动；`loop` 把尾巴最老的轮并入直到 ≤ target（至多 4 次）；摘要请求 `max_completion_tokens = 6144`、JSON `{summary, verbatim, memory_candidates}`，assistant 文本与调用参数各 ≤ 4096 字（`REPORT_CHARS`）；keep_exact 去重追加、≤ 2048 token、溢出的最老条目进 `memory_candidates`。summary 消息 `<history_summary>…</history_summary>\n<keep_exact>…</keep_exact>\n<note>`；新接口 `compress_history` / `CompressOptions` / `CompressOutcome{messages, head_end, tail_start, summary, memory_candidates, report}` / `summary_message` / `parse_summary_message`；日志 `llm_compress.completed`。机械阶段（信封降级、`plain_text_truncate_v1`、History 块、`MechanicalCompressedMeta`）与对应测试全部删除 |
| Q4 | `SUMMARIZE_MODEL_ALIAS = "llm.summarize"`；`llm_explore` / `llm_understand_media` 缺省改用它；`EffectiveConfig::summary_model()`：buckyos provider 用别名，openai 直连用 run 的模型；xllm `compact_context` 与 libopendan 都用它 |
| U2 | `lib_opendan/src/runner/history.rs` 重写：`Render` 把 worklog 忠实重建为消息（`HistoryShape::Native` 原生 tool calls / `Text` 给 behavior 上下文）；`drop_kinds` 保留（`MechanicalCompress` → `HistoryRender{drop_kinds}`，`prompt.mechanical_compress` → `prompt.history_render`，缺省加 `turn_started` / `input_batch`）；超预算整段交给 `compress_history`；`summary.json` 加 `keep_exact`、`head_end_seq/offset`，删 `mechanical`，schema `opendan.session_summary/3`，renderer `libopendan.history/4`。`Summarizer` 由 trait 改为 `{llm, model}`，`RunnerDeps.summarizer` 改为 `Option<Arc<dyn LlmClient>>` |
| C3 | `RoundCounter.successes()`（不消费 H1 的 `progressed`）；`run_compacting` 两次重写之间有成功推理就把 attempt 清零；预算 `history_budget >> (attempt − 1)` |
| C5 | `rewrite.completed` 加 `history_tokens_after` / `summarized` / `summary_tokens` / `summary_calls`；`compaction.completed` 带前后 token、`start_seq` / `head_end_seq` 前后、`keep_exact`、`memory_candidates` 原文；中途重写的 `compaction.skipped` 记 INFO |
| C6 | `COMPRESSED_NOTE` 作为 note 渲染在 keep_exact 之后，只在重建的历史里 |
| W1 | `RunnerOptions.max_context_window` 缺省 `None` |
| 文档 | `LLM Compress.md` 按实现重写（v2.0），删 `llm_compress_implementation_guide.md`（机械压缩实施指南）；Session Directory Protocol §6/§7、架构设计 §4.2、LLM Context Switch 样例、readme / why behavior loop / append-only / prompt_render_engine / Render 变量 / xAgent / SDK 实现计划里的 `<session_history>` 说法；`opendan_agent_dev` 手册的窗口与压缩说明；schema `session_config` / `session_summary` 重新导出；fixtures 的 `mechanical_compress` 与 11 号 `summary.json` 改成新形态（生成器输出不确定，只改这些字段） |

### 8.2 实施中自定的两点

1. **provider 拒绝时的预算**：忠实重建后，一个估算远低于窗口却被 provider 拒绝（`context_length_exceeded`）的历史重建出来还是同样大，会被拒三次后暂停（旧的 280 字截断会顺带缩小它）。`rewrite_for_limit` 在 `which = provider_refused` 时把预算限制为被拒历史的一半（`Session Directory Protocol` §7 已写）。
2. **测试脚本的 call_id**：Goal 的两组测试（`goal_session`、`memory_maintenance`）的脚本每次 Activation 都用同一个 call_id（`r1`），旧的文本历史里看不到上一次的结果，现在是原生 tool 结果，脚本改为只看当前 Activation 头之后的消息。真实模型的 call_id 唯一；若某个 provider 不保证跨 run 唯一，原生重建会出现重复 id（UI 续聊原有路径同样如此），记为风险。

### 8.3 验证

- `cargo test -p agent_tool`：345 过（含 llm_compress 14 个新单测，§5 列的各项）；
- `cargo test -p libopendan -p agent_tool -p opendan -p llm_context --no-fail-fast -- --test-threads=1`（CI 的跑法）：945 过、0 失败、6 ignored；`context_limit.rs` 新增：重写后 head + summary + 尾巴且历史落在 `[0.6, 1.0] × history_budget`、推理成功后 attempt 从 1 起；`--test-threads=4` 下 `plan_simple_async` 有 4 个失败（Todo 的 work session 在 Todo 消息到达前先以 on_init 推理），单线程全过；没有在改动前的代码上对照。
- DV（§5 第 1、2 条）未做。
