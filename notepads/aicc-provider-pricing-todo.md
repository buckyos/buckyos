# AICC Provider 计费链路 TODO

日期：2026-09-25。面向后续 CodeAgent。本文是实现任务单：本轮只完成了现状 Review 并整理出差距，尚未动手实现。AICC Provider 还有其他待改项，届时与本文一起修改，其他项可追加到第 6 节。

## 1. 目标原则

1. Provider 有义务提供价格信息，最好能通过网络直接更新。
2. Provider 实现的核心是：把 usage 算对，并拿到单价（此时可以实时折算单价）。
3. 价格信息宁可没有，也不能写错。
4. 同一个模型有多个 Provider 时，优先使用单价最低的。

## 2. 当前链路（Review 时的状态）

`静态 JSON / discovery → inventory pricing（Discovery > ProviderRules > ModelDriver）→ 调用时锁定 PinnedPricingSnapshot → 完成后 usage × 单价（provider 自报 cost 优先）→ 路由按 estimated_cost 打分`

- 11 份 builtin Model Driver 的 `model_pricing` 已删除（见 [aicc-model-driver-logical-tree-todo.md](aicc-model-driver-logical-tree-todo.md)），**旧价格没有搬到 Provider Rules**。目前只有 GLM（`providers/glm.provider.json`）和 OpenRouter（discovery 动态价）有价格。
- `src/provider/builtin/openai.rs`、`claude.rs` 中仍有测试断言 Model Driver 上存在价格，需要随本任务处理。

以下路径均相对于 `src/frame/aicc/`。行号以 Review 时为准，动手前请重新定位。

## 3. P0：会算错钱的问题

- [ ] **各协议对 cache token 的语义不一致。** `TokenRates::apply`（`src/execution/mod.rs` 约 414 行）假设 `cache_read ⊂ input_tokens`，这是 OpenAI/Gemini 的语义。Claude 的 `input_tokens` 不包含 cache read 和 cache creation（`src/protocol/claude_messages.rs` `decode_usage`），因此会被多扣一次 cache 部分，`total_tokens` 也偏小。要求：适配层统一输出含义固定的计费 usage，并为每个协议补测试。
- [ ] **cache 写入没有计费。** `AiUsage.cache_write_input_tokens` 已解析，但 `Pricing`、`PricingTierStep`、`PricingTimeWindow` 都没有 `cache_write_input_token` 单价，`apply` 也未使用（例如 Claude 的 cache 写入按 1.25 倍 input 价计）。
- [ ] **缺失单价被当成免费。** `apply` 中 `input_token` 或 `output_token` 为 `None` 时按 0 计。应改为：usage 中有该维度、价格中没有对应单价时，整体返回 `None`（不出价）。embedding 这类确实没有 output 的情况，由 usage 为 0 或 None 自然处理，不靠单价缺失来表达。
- [ ] **阶梯价超出最后一档时回落到基础价。** `tier_rates` 中 `find` 返回 `None` 后，外层 `.unwrap_or(base)` 会取基础价。校验（`src/catalog/validation.rs` `validate_pricing_tiers`）只要求"只有最后一档可省略 `up_to`"，没有强制最后一档必须省略。二选一：强制最后一档无上限，或超档返回 `None`。
- [ ] **自报 cost 一律按 USD 处理。** `src/protocol/openai_chat_completions.rs` 和 `openai_responses.rs` 的 `decode_usage` 只要发现 `usage.cost` 就当 USD，而 provider 自报 cost 的优先级最高。改为由 Provider 声明自报 cost 的币种和语义；未声明时忽略。

## 4. P1：让比价真正生效（原则 4）

- [ ] **LLM 的 estimated_cost 实际总是 `None`。** `estimate_model_cost`（`src/service/inference.rs` 约 661 行）使用了 `estimated_input_tokens?`，而推理路径上 `estimated_input_tokens` 全为 `None`，所以 token 计价模型永远没有估算成本，只有按次计价的模型能参与比价。改为不依赖真实 token 数的可比单价指数，例如固定参考量或请求画像比例。
- [ ] **币种不一致时整体放弃比价。** `score_candidates`（`src/routing/mod.rs` 约 993 行）只要有一个候选币种不同，所有候选的成本都变成 `None`，CNY 与 USD 混合时比价失效。需要一个汇率层：汇率表可随 cloud catalog 下发，并带有效期；过期即视为无价。
- [ ] **没有"同一 origin 模型组内按价格优先"的规则。** 现在 cost 只是加权打分的一项（Balanced 下为 0.25），会被延迟、质量、偏好压过。改为两层：逻辑模型之间沿用现有加权打分；同一 origin 模型的多个 Provider 实例之间按估算单价排序，价格相同时再看延迟和可靠性。
- [ ] 确认无价候选的策略。目前 cost 分记为 1.0（最差），但 `budget_with_unknown_cost_allows_candidate` 允许无价候选通过预算限制，需要决定是否与原则 3 一致。

## 5. P2：架构与数据

- [ ] **去掉 `PricingSource::ModelDriver`**（`src/provider/inventory.rs`、`src/call/mod.rs`）。价格只能来自 Provider Rules 或 discovery：同一 origin 模型在不同 Provider（转售、Azure、Bedrock 等）上价格不同，不能继承 Model Driver 的价格。同时清理 `ModelSemantics.pricing`、`ModelDriverCatalog.model_pricing` 及其校验和测试。
- [ ] **把价格补回 Provider Rules。** 从官方页面逐个核对后填入 `providers/*.provider.json`，不能照搬已删除的旧数据。旧数据中已发现疑似错误：`claude-fable-5-1` 的 `cache_input_token` 为 2.5e-07，而 `claude-fable-5` 为 1e-06。
- [ ] **价格可追溯。** `Pricing` 增加 `source_url` 和 `verified_at`（或 `effective_at`）。
- [ ] **价格合理性 lint。** 检查 cache/input、output/input 等比例；比例异常必须显式标注，否则校验失败。
- [ ] **价格独立发布。** 考虑把价格拆成独立的 catalog kind，走 `cloud_update` 单独发布和回滚，不必与技术规则共用一个版本。
- [ ] **计费职责下放到 Provider。** 目前 `ProviderExecutionPort::completion_cost` 只有默认实现，没有 Provider 覆盖。可以让 Provider 输出归一化后的计费 usage，或允许其覆盖 `completion_cost`（可与 P0 第一条一起做）。
- [ ] **discovery 价格与 catalog 同等校验。** `src/provider/mod.rs` `validate_pricing` 只校验基础字段，没有校验 `tiers` 和 `time_windows`。
- [ ] **多模态 token**（音频、图片输入 token）按单独单价计费；缺少对应单价时返回 `None`。
- [ ] **价格覆盖率可观测。** 管理接口列出"已上架但无价"的模型。
- [ ] 为更多 Provider 实现 discovery 动态拿价（目前只有 OpenRouter）。

## 6. 其他 Provider 待改项

（待补充，与上面各项一起修改。）

## 7. 验收

- 每个协议适配器都有 usage 语义测试：包含 cache read、cache write、reasoning 的样例响应，经计算得到确定的成本。
- 单价缺失、超出阶梯档位、币种未声明时结果为 `None`，而不是 0 或错误的价格。
- 同一 origin 模型挂在两个不同价格的 Provider 上时（包括 CNY/USD 混合），路由选中较便宜的一个。
- `cargo test -p aicc` 全部通过（包括清理 Model Driver 价格断言后的 `openai.rs` / `claude.rs` 测试）。
