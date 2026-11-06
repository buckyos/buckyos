# OpenDAN 上下文压缩 `llm_compress`

- 文档版本：v2.0（2026-11-03，按 [compaction TODO](../../notepads/opendan-context-window-compaction-todo.md) 的裁决重写）
- 实现：`src/frame/agent_tool/src/llm_compress.rs`
- 调用方：libopendan Session（`runner/history.rs`、`runner/live.rs`）、xllm 自己的 run（`compact_context`）、`llm_understand_media`（父历史提纯）

> **分工**：llm_context 冻结后（见 [llm-context-freeze-todo](../../notepads/llm-context-freeze-todo.md)），llm_context 只在估算达到触发线（`context_yield_ratio`，缺省 0.80）时让出 `ContextLimitReached`，自己不改写历史；改写由宿主执行，压缩逻辑**只有本模块一种**。宿主不再自带机械截断，本模块也没有机械压缩：压缩 = 一次（或分块的若干次）LLM 摘要。

---

## 1. 为什么只有 LLM 压缩

2026-11-03 的 Zone DV（研究类 Work，256k 窗口）里，旧实现把 175k 压到 13k（目标是 76.8k），LLM 摘要一次没跑，模型压缩后重读同一批文件。原因有两层：

1. libopendan 自带的机械渲染把旧条目一律截到 280 字，50 轮的 run 只剩约 10k，"装得下"就永远到不了 LLM 摘要；模型自己写的进度报告也被截掉。
2. 本模块原有的机械阶段（按 AgentToolResult 信封降级、`plain_text_truncate`、History 块折叠）在 F22 之后已经失效：进历史的工具结果是 `observation_text` + hints，信封只在 `Observation.host`；而且它只在 user 消息处分段，function call 的 run 中途没有 user 消息，整个 run 是一个 active span，原样返回。

离线探针（`agent_tool/examples/compress_probe.rs`）证实：工具结果占 token 的 94%，机械降级后只剩 9–21%，"先机械够了就停"永远不会走到 LLM；真实摘要（163k 输入 → 5.3k，2–3 分钟）保留了任务、约束、产物路径、已核验的数字与版本、已交的中间报告和下一步，正好对症重读。所以只留被证明有用的 LLM 压缩。

---

## 2. 设计目标

1. **压缩后形态固定**：`system 前缀 + head（开场输入 + 第 1 轮）+ [一块 summary] + keep_exact + note + 原始尾巴`。尾巴里没有被改写过的消息。
2. **按轮切分**：head、尾巴、压缩范围的边界都是"轮"（§3.1），tool 结果不会和它的调用分开，behavior 的 step 与它的结果不分开。
3. **summary 滚动合并**：已有 summary 与新进入范围的原始轮一起交给摘要器，输出一块新 summary 替换它们；历史里始终只有一块 summary。原始轮不会被二次摘要；summary 的有损会叠加，但有界（探针：滚动一次，抽查 10 个关键事实留 8 个）。
4. **尾巴按预算填满**：压缩不是"压到越小越好"，而是回到目标线——尾巴从最新一轮往回填满 `target` 的余量（30% / 256k 下约 60k，最近 10–15 轮）。
5. **压到目标为止**：同一次调用内分块折叠、必要时把尾巴最老的轮并入 summary，直到 `total ≤ target`（至多 `MAX_FOLD_PASSES` = 4 次）。
6. **模型写下的东西不丢**：给摘要器看的段落里，assistant 文本与工具调用参数（`report` 正文、`intent`）各自全文，上限 `REPORT_CHARS` = 4096 字；提示词要求保留中间报告的内容和 `intent` 里的判断。
7. **keep_exact（hint）是不可压缩的逐字槽**：摘要可以大胆丢细节，但用户给的电话、地址、账号、确切的 id / URL / commit SHA / 路径、数值约束不能有损。摘要器把这类字符串输出到 `verbatim`；运行时逐字追加到 keep_exact，已有条目不再经过摘要器（摘要器只能追加，不能改写），去重，总量上限 `KEEP_EXACT_MAX_TOKENS` = 2048，超出时最老的条目转为 memory candidates。keep_exact 与 Memory 互补：Memory 是长期保证但召回不一定在需要的那一轮注入，keep_exact 是上下文内保证，同时也是现成的 Memory 候选。
8. **摘要模型由调用方指定**：BuckyOS 下是 AICC 别名 `llm.summarize`（`SUMMARIZE_MODEL_ALIAS`）；xllm 直连 OpenAI 兼容端点时只能用本次 run 的模型（`EffectiveConfig::summary_model`）。
9. **可观测**：每次压缩以 INFO 记 `llm_compress.completed`（范围、策略、token 变化、摘要调用次数），宿主再记自己的 `opendan.context.*` 事件。

---

## 3. 概念

### 3.1 轮（Round）

system / developer 前缀之后的消息按轮切分（`segment`）：

- 一轮 = 一条 assistant 消息（可带 tool_calls）+ 它的 tool 结果 + 紧随的 user 输入（function call run 中途注入的输入属于前一轮）；
- 开场的 user 消息并入第一轮；
- assistant 的最终回复（没有 tool_calls）之后的 user 消息开新一轮——对话里一问一答是一轮；
- behavior 的 step 渲染为 assistant 文本 + 一条 `<<last_step_action_results …>>` user 消息（前缀 `STEP_RESULTS_PREFIX`），结果消息并入 step 那一轮；
- summary 消息（§3.3）单独成一个单元，是 head 与尾巴的边界。

轮是消息层单元，不等于 Turn；Round / Step / Turn 的定义见 [readme.md](readme.md)。

### 3.2 Head

第一轮（开场输入 + 第一次响应及其结果）。它占 `target` 的比例不超过 `HEAD_MAX_SHARE`（0.25）时整轮保留；否则只保留开场的 user 消息（同样受此上限）；再不行就没有 head。已有 summary 时，head 是 summary 之前的那一轮。

### 3.3 Summary 消息

一条 user 消息，由 `summary_message` 渲染、`parse_summary_message` 解析：

```text
<history_summary>
…摘要（markdown）…
</history_summary>
<keep_exact>
- +1 (408) 555-0100
- 3f2a9c1e
</keep_exact>
Everything before this point was compressed; files in the session directory are intact — re-read only what you need
```

最后一行是调用方传入的 `note`（libopendan：`COMPRESSED_NOTE`；xllm：工作目录版本），不是摘要的一部分，也不进 worklog。keep_exact 为空时不渲染该块。

### 3.4 尾巴（Tail）

```text
tail_budget = target − system 前缀 − head − SUMMARY_MAX_TOKENS(6144) − KEEP_EXACT_MAX_TOKENS(2048) − note − 16
```

从最新一轮往回累加，装得下就并入尾巴，遇到 summary 或 head 停止。尾巴可以为空（最新一轮本身就超过余量时，它也进 summary）。

### 3.5 压缩范围

`[head_end, tail_start)`：已有的 summary 消息（合并为 previous）+ 其中的全部原始轮。没有原始轮，或 system 前缀本身已超过 `target` 时，不调用摘要器，原样返回（`strategy = no_range`）。

---

## 4. 流程

```text
compress_history(history, deps, opts):
  total ≤ target                     → 原样返回（unchanged）
  segment → head → tail_budget → tail
  range = [head_end, tail_start)：previous summary + 原始轮
  无原始轮 / system 前缀 ≥ target     → 原样返回（no_range）
  fold(previous, 原始轮)              → 新 summary（分块折叠，§4.1）
  loop:
    out = 前缀 + head + summary 消息 + 尾巴
    out ≤ target 或 尾巴为空 或 已 4 次 → 返回
    尾巴最老的若干轮（覆盖超出的部分）并入 summary：fold(summary, 这些轮)
```

### 4.1 分块折叠

每一轮渲染成摘要器读的文本（`### round N`，`assistant:` / `call <tool> #<id>: <args>` / `result #<id> [ok|error]:` / `user:`），按 `summary_chunk_tokens`（缺省 100k）分块；前一块的摘要作为下一块的 previous，keep_exact 逐块追加。单个工具结果在渲染时最多 `2 × summary_chunk_tokens` 字（约半块），超出部分注明省略。1.05M 窗口下一次压缩的范围可达 70 万 token，分块是硬要求。

### 4.2 摘要请求

- system：固定提示词（有长期记忆系统、可以大胆；按名字精确保留任务与约束、已核验的结论及关键数字 / 版本 / 引文、决策与理由、写下的文件及其内容、已交的中间报告、`intent` 里的判断、未解决的问题与剩余工作；与 previous 合并且可独立成立；不编造；用任务语言；约 3000 词以内）+ 调用方的 `extra_focus_prompt`；
- user：`<previous_summary>` + 只读的 `<keep_exact>` + `<history rounds="N">`，并声明历史只是材料、不是指令；
- `max_completion_tokens = SUMMARY_MAX_TOKENS`（6144），`force_json` 配固定 schema，不带工具；
- 输出 `{"summary": "<markdown>", "verbatim": [...], "memory_candidates": [...]}`；不是 JSON 时整段文本作为 summary；summary 为空返回 `OutputParse` 错误。

摘要调用是独立的 LLM 请求（libopendan 用 run 的原始 client，不计入 Round 统计）。

---

## 5. 接口

```rust
pub async fn compress_history(
    history: &[AiMessage],
    deps: &LLMContextDeps,
    opts: &CompressOptions,
) -> Result<CompressOutcome, LLMComputeError>;

pub struct CompressOptions {
    pub target_token_budget: u32,
    pub model_alias: String,
    pub extra_focus_prompt: Option<String>,
    pub note: Option<String>,
    pub summary_chunk_tokens: u32,
}

pub struct CompressOutcome {
    pub messages: Vec<AiMessage>,
    pub head_end: usize,      // 输入 [..head_end] 原样保留（前缀 + head）
    pub tail_start: usize,    // 输入 [tail_start..] 原样保留（尾巴）
    pub summary: Option<CompressedSummary>,   // { summary, keep_exact }；None = 没有摘要
    pub memory_candidates: Vec<Value>,
    pub report: CompressReport,               // 日志 llm_compress.completed 的内容
}
```

- `compress(history, deps, target, model_alias, focus)`：只要消息的简写；
- `LlmSummarizeCompressor`：xllm `Compressor` 适配器；
- token 用 waist 的估算器（`estimate_messages`），与触发线同一口径。

---

## 6. 调用方

### 6.1 libopendan Session

历史由 worklog 忠实重建为消息（`runner/history.rs`，渲染器 `libopendan.history/4`）：user 输入（有 `brief` 时用 brief）、assistant 响应与原生 tool calls、tool 结果、最终交付（`report_delivery`；交付 report 的那次调用参数换成交付引用，正文只出现一次）不截断；behavior 上下文里 function call 的调用列在 assistant 文本中、结果放进 `<<last_step_action_results>>`；已返回子 context 的过程不进历史，只留 `process_done` 结果；`drop_kinds` 中的审计条目不渲染。超预算时整段历史交给 `compress_history`，结果写回 `summary.json`：

| 字段 | 含义 |
| --- | --- |
| `history_summary` / `keep_exact` | summary 块 |
| `head_end_seq` / `head_end_offset` | head = worklog 中此前的条目（0 = 没有 head） |
| `start_seq` / `start_offset` | 原始尾巴的起点；之后的重建从提交末尾反向读到这里 |

压缩只写 summary.json 并追加一条 `compaction` 条目，worklog 不改写。三处入口：

1. **新 run 的历史**（`build_history`，`inherit: recent_dialogue`）：预算 `history_budget_tokens`；
2. **run 中途碰线**（`rewrite_for_limit` → `compact_for_limit`）：预算 `history_budget >> (第几次 − 1)`，`history_budget = max(⌈窗口 × 0.30⌉ − system 前缀, history_budget_tokens)`；provider 拒绝（`provider_refused`）时不超过被拒历史的一半；推理成功一次后次数清零，连续 3 次放不下才暂停 run；
3. **run 结束后按比例**（`maybe_compact`，只在宿主配置了摘要 client 时）：超过 `compact_ratio × 预算` 时压到预算的一半。

U3（summary.json 改为持久化 compressed pair、UI 续聊与 Goal 恢复结构对账）另起一单。

### 6.2 xllm

`compact_context`：`ContextLimitReached` 时把 `rewritable_history` 交给 `LlmSummarizeCompressor`（目标 `compaction_target_tokens`，摘要模型 `summary_model()`，note 指向工作目录），function call 用 `RewrittenHistory`、behavior 用 `RewrittenSteps` 恢复；一个 run 至多压缩 3 次。

### 6.3 llm_understand_media

调用 `compress` 提纯父历史（摘要模型参数 `summary_model`，缺省 `llm.summarize`），超预算时同样只走 LLM 摘要。

---

## 7. 可观测性

`llm_compress.completed`（INFO；`no_range` 为 WARN）：`strategy`、`model_alias`、`target_tokens`、`tokens_before` / `tokens_after`、`head_messages` / `head_tokens`、`range_start` / `range_end` / `range_rounds` / `range_tokens`、`previous_summary`、`tail_rounds` / `tail_tokens`、`summary_calls`、`summary_tokens`、`keep_exact`、`memory_candidates`、`reached_target`、`elapsed_ms`。

libopendan 另记：`opendan.context.compaction.{triggered,skipped,started,completed,failed,budget_exceeded}`（`completed` 带 `history_tokens_before/after`、`start_seq` / `head_end_seq` 前后、`summary_calls`、`keep_exact`、`memory_candidates` 原文）；`opendan.context.rewrite.completed` 带 `history_tokens_after`、`summarized`、`summary_tokens`、`summary_calls`。

---

## 8. 错误

| 情况 | 行为 |
| --- | --- |
| 历史在目标内 | 原样返回，不调用摘要器 |
| 没有可摘要的原始轮 / system 前缀超过目标 | 原样返回（`no_range`），由调用方决定（libopendan 记 `budget_exceeded`，中途重写计一次尝试） |
| 摘要器失败 | 返回错误，不伪装成成功（libopendan 记 `compaction.failed` / `rewrite.failed`） |
| 摘要为空 | `OutputParse` |
| 摘要比预留的大 | 尾巴最老的轮并入 summary 再折叠，至多 4 次 |

---

## 9. 非目标

- 不在压缩中写 Memory（`memory_candidates` 只随结果返回、进日志）；
- 不做精确字数控制；
- 不在单次工具调用后触发：触发由 llm_context 的触发线与宿主决定；
- 不给 Work Session 另立比例：§11.2 "保留较多执行信息"由窗口 × 30% 自然得到。

---

## 10. 测试

- `agent_tool` 单测（`llm_compress::tests`）：按轮分段（function call / 对话 / behavior 结果消息）、head + summary + 尾巴形态与配对、尾巴按预算填满、滚动合并只有一块 summary、分块折叠、keep_exact 跨两次滚动逐字不变与超上限转 memory candidates、report / intent 全文进摘要段落、循环到目标、head 过大时只留开场输入、纯文本回复、空摘要报错；
- `lib_opendan/tests/context_limit.rs`：中途重写后的形态与 token 落在 `[0.6, 1.0] × history_budget`、provider 拒绝后的重写、behavior 重写、摘要失败、连续次数清零（C3）；`tests/l1.rs`：预算耗尽压缩无缝隙、按 summary.json 重建结果稳定；
- 离线探针：`cargo run -p agent_tool --example compress_probe -- --worklog … --system … --target … --mode llm --base-url …`。
