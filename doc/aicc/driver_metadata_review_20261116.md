# AICC 模型目录与默认路由维护记录（2026-11-16）

执行依据：[notepads/aicc-model-catalog-maintenance-todo.md](../../notepads/aicc-model-catalog-maintenance-todo.md)。基线提交 `af0c007f9`（任务文档基线 `120d1811b` 之后只新增了该任务文档）。后续轮次按 [LLM 模型目录例行维护](maintenance/llm_catalog_routine_maintenance.md) 执行。

## 1. Review 复核

任务文档 §2 的源码观察均成立，补充以下差异：

- 单模型缺口比 §2.1 的三个例子更普遍。用生产 registry（`ModelRegistry::build` + builtin overlay + `resolve_candidates`）对 113 个非 `direct_only` 内置 LLM 逐个单独接入，修改前有 110 个至少一个用途既无候选、也没有能力拒绝记录（纯目录漏配）；§2.1 的三个模型结果与任务文档完全一致。
- 缺口全部来自规格引用：没有一例是 effort 未映射、inventory 缺失或 Provider 禁用造成的（单模型 inventory 带齐全部规格绑定的 effort）。真实能力拒绝只有 plan/code 缺 JSON Schema 或 tools、vision 缺视觉、summarize/vision 上下文不足四类。
- `model_defaults.rs` 头部注释树此前漏写 chat 的 `kimi_flagship`、`qwen_max`，plan 的 `kimi_flagship`，vision 的 `kimi_flagship`、`minimax_standard`，以及规格目录 `kimi-flagship`。原有一致性测试只检查“注释 → 代码”方向，现改为双向（条目总数必须等于实际目录）。
- 仓库已有 54 个规格（16 个 `direct_only`）。OpenAI GPT-6.x 价格已带 272K 以上的长上下文阶梯；Claude Haiku 5.5 已带 100K 阶梯价格。
- `MetadataSources::resolve` 整文件选择、builtin 嵌入二进制、`BUILTIN_CATALOG_REVISION_SEQ = 22`、云更新默认关闭，均与任务文档一致。现有约定是每个改动的 catalog `revision_seq` 加 1，builtin 序号加 1。

## 2. 最终规则

- 七个内置用途（chat、plan、code、swift、summarize、translate、vision）的定义改为显式 `fallback: {mode: target_logical, target: llm.fallback}`；`llm.fallback` 保持 `disabled`（终点），scheduler 改为 `cost_first`。用途自身的规格偏好和权重不变。
- `llm.fallback` 引用全部 38 个非 `direct_only` 规格，分四层：
  - 4.0 廉价快速档：gpt-nano、claude-haiku、gemini-flash-lite、qwen-flash、glm-flash、deepseek-flash、doubao-mini、minimax-highspeed；
  - 3.0 性价比档：gpt-mini、gemini-flash、qwen-plus、doubao-lite、kimi-general、qwen-moe-35b、qwen-dense-27b；
  - 2.0 标准档：gpt-standard、claude-sonnet、glm-standard、deepseek-pro、kimi-flagship、minimax-standard、qwen-max、doubao-pro、qwen-moe-120b、qwen-moe-400b；
  - 1.0 高档与特化档：gpt-pro、gpt-max、gpt-codex、claude-opus、claude-fable、gemini-pro、qwen-moe-2t、kimi-code、doubao-code、glm-vision、glm-vision-code、glm-vision-flash、deepseek-vision。
- 排序规则：沿用现有逐层展开，`llm.fallback` 取第一个仍有合格候选的权重组，所以分层顺序优先于单价；组内由 scheduler 排序。直接请求 `llm.fallback` 用 `cost_first`；从用途回退时 `CandidateSet.scheduler_profile` 仍取原用途（plan 为 `quality_first`），能力门槛和禁用项也沿用原用途。未知价格在归一化里按最差成本处理。
- 低优先级候补：plan 加 `claude_sonnet`（1.0）、code 加 `claude_sonnet`（1.05）、vision 加 `gemini_flash`（0.95），都低于该用途原有全部条目，只在原用途没有其他候选时生效。理由：回退按“廉价优先”，Sonnet 是 Anthropic 定位于编程/Agent 的主力档、Gemini Flash 原生多模态，只接这类模型和更便宜的同系列模型时，质量型用途应先用它们而不是回退到最便宜的一档。
- 隔离：overlay 替换用途 `items` 而未写 `fallback` 时回退关闭（沿用原语义）；`strict`/`disabled` 关闭；只改权重保留回退；规格、家族直选为 `disabled`；`direct_only` 规格进入 `llm.fallback` 会被 `validate_llm_tree` 拒绝。Router 的 `allow_fallback=false` 仍拒绝经回退得到的路由。
- 运行期边界（本轮未改）：目录回退只在路由阶段发生。`service/inference.rs` 的运行期 failover 只遍历本轮已排序的 `fallback_candidates`（同一权重组），上游执行错误不会重新解析到 `llm.fallback`；运行状态在路由时已不可用的候选会被跳过，从而进入下一组或回退。
- 不改 spec effort 或 `default_effort`：回退使用规格绑定的现有预设。例如 gemini-flash-lite 绑定 `:high`（官方默认 minimal），Claude Haiku 5.5 绑定 `:medium`；更低预设需要后续通过 `additional_specs` 等合法绑定表达。

修改入口：`src/frame/aicc/src/service/model_defaults.rs`（定义、overlay、头部契约）；测试在 `service/tests.rs`、`routing/mod.rs`、`provider/builtin/upgrade_tests.rs`、`provider/builtin/gemini.rs`；契约文档 `aicc 逻辑模型目录.md`、`driver_metadata_schema.md`、`frozen_model_driver_and_logical_model_fs.md`。

## 3. 单模型 × 用途覆盖

生产 registry 逐个单独接入每个内置 LLM（带齐其全部非 `direct_only` 规格绑定的 effort），按用途统计。“回退”指经 `llm.fallback` 到达；“拒绝”指模型被路由到但能力不满足，`admissions` 记录了缺失项。

| 用途 | 修改前：用途 / 拒绝 / 不可达 | 修改后（含本轮新增 3 个模型，共 116 个）：用途 / 回退 / 拒绝 |
| --- | --- | --- |
| chat | 66 / 0 / 47 | 67 / 49 / 0 |
| plan | 37 / 16 / 60 | 43 / 36 / 37 |
| code | 35 / 15 / 63 | 39 / 40 / 37 |
| swift | 34 / 0 / 79 | 35 / 81 / 0 |
| summarize | 37 / 1 / 75 | 38 / 77 / 1 |
| translate | 38 / 0 / 75 | 39 / 77 / 0 |
| vision | 42 / 12 / 59 | 53 / 37 / 26 |

修改后不可达为 0。剩余拒绝：plan/code 缺 JSON Schema（32，含 Gemma 4、gpt-5.4-pro、多款 GLM/MiniMax）、缺 tools+JSON（3）、上下文与能力都不足（2）；vision 缺视觉（24）及上下文不足（2）；summarize 上下文不足（1，`doubao-seed-translation` 4K）。`direct_only` 家族（grok、qwen-legacy、GLM 特化线等）单独接入时任何用途和 `llm.fallback` 都没有候选或准入记录。

主流三家逐模型结果：

| 模型 | chat | plan | code | swift | summarize | translate | vision |
|---|---|---|---|---|---|---|---|
| `claude/claude-fable-5` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 回退 |
| `claude/claude-fable-5-1` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 回退 |
| `claude/claude-fable-5-2` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 回退 |
| `claude/claude-haiku-4-5-20251001` | 回退 | 回退 | 回退 | 用途 | 用途 | 用途 | 回退 |
| `claude/claude-haiku-5-5` | 回退 | 回退 | 回退 | 用途 | 用途 | 用途 | 回退 |
| `claude/claude-opus-4-5-20251101` | 回退 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-opus-4-6` | 回退 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-opus-4-7` | 回退 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-opus-4-8` | 回退 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-opus-5` | 回退 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-opus-5-5` | 回退 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-sonnet-4-5-20250929` | 用途 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-sonnet-4-6` | 用途 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-sonnet-5` | 用途 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `claude/claude-sonnet-5-5` | 用途 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `gemini/gemini-2.5-flash` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemini-2.5-flash-lite` | 回退 | 回退 | 回退 | 用途 | 回退 | 回退 | 回退 |
| `gemini/gemini-2.5-pro` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 用途 |
| `gemini/gemini-3-flash-preview` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemini-3.1-flash-lite` | 回退 | 回退 | 回退 | 用途 | 回退 | 回退 | 回退 |
| `gemini/gemini-3.1-pro-preview` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 用途 |
| `gemini/gemini-3.1-pro-preview-customtools` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 用途 |
| `gemini/gemini-3.5-flash` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemini-3.5-flash-lite` | 回退 | 回退 | 回退 | 用途 | 回退 | 回退 | 回退 |
| `gemini/gemini-3.6-flash` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemini-3.7-flash` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemini-3.8-flash` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemini-4-argon` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 用途 |
| `gemini/gemini-4-carbon` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 用途 |
| `gemini/gemini-4-hydrogen` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemini-robotics-er-2-preview` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemma-4-26b-a4b-it` | 用途 | 拒绝:json | 拒绝:json | 用途 | 用途 | 用途 | 用途 |
| `gemini/gemma-4-31b-it` | 用途 | 拒绝:json | 拒绝:json | 用途 | 用途 | 用途 | 用途 |
| `openai/gpt-5.3-codex` | 回退 | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 |
| `openai/gpt-5.4` | 用途 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `openai/gpt-5.4-mini` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 回退 |
| `openai/gpt-5.4-nano` | 回退 | 回退 | 回退 | 用途 | 回退 | 回退 | 回退 |
| `openai/gpt-5.4-pro` | 回退 | 拒绝:json | 拒绝:json | 回退 | 回退 | 回退 | 回退 |
| `openai/gpt-5.5` | 用途 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `openai/gpt-5.5-pro` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 回退 |
| `openai/gpt-5.6` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 回退 |
| `openai/gpt-5.6-luna` | 回退 | 回退 | 回退 | 用途 | 回退 | 回退 | 回退 |
| `openai/gpt-5.6-sol` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 回退 |
| `openai/gpt-5.6-terra` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 回退 |
| `openai/gpt-6-astra` | 回退 | 用途 | 回退 | 回退 | 回退 | 回退 | 回退 |
| `openai/gpt-6-luna` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 回退 |
| `openai/gpt-6-sol` | 用途 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `openai/gpt-6.1-sol` | 用途 | 用途 | 用途 | 回退 | 回退 | 回退 | 用途 |
| `openai/gpt-6.1-terra` | 用途 | 回退 | 回退 | 用途 | 用途 | 用途 | 回退 |

本轮新增模型由 `models_added_on_2026_11_16_close_the_discovery_to_route_loop` 覆盖原生渠道的发现 → 身份匹配 → 预设/价格 → 路由闭环。

## 4. fallback 运营选型

费用为原厂原生渠道标准价（USD/百万 token，输入 / 输出 / 缓存读），不代表 SN、OpenRouter 等渠道实际报价；没有一个厂商公布延迟或吞吐数字时写“无官方延迟数字”。同一规格内按版本权重选最新家族，不按价格。

| 系列 | 4.0 档首选（规格 → 家族:预设） | 费用 | 速度依据 | 后备顺序 | 来源（核对 2026-11-16） |
| --- | --- | --- | --- | --- | --- |
| GPT | gpt-nano → gpt-6-luna:none（其次 gpt-5.6-luna、gpt-5.4-nano） | 0.10 / 0.50 / 0.01；>272K 输入 2×、输出 1.5× | 无官方延迟数字；官方称 luna 为“focused, high-volume”高效模型，`none` 不产生推理 token | gpt-mini（gpt-6.1-terra:low，1 / 5 / 0.05，官方称 fast, efficient）→ gpt-standard（gpt-6.1-sol:medium）→ gpt-pro/max/codex | [pricing](https://developers.openai.com/api/docs/pricing)、[models](https://developers.openai.com/api/docs/models) |
| Claude | claude-haiku → claude-haiku-5-5:medium（其次 haiku-4-5:none） | 提示 ≤100K：0.10 / 0.50 / 0.01；>100K：0.50 / 2.50 / 0.05 | 官方比较延迟标为 “Fastest”，无数字；`low` effort 更快更省但未绑定规格 | claude-sonnet（sonnet-5-5:high，2 / 10 / 0.10）→ opus / fable | [pricing](https://platform.claude.com/docs/en/about-claude/pricing)、[models overview](https://platform.claude.com/docs/en/about-claude/models/overview) |
| Gemini | gemini-flash-lite → gemini-3.5-flash-lite:high（其次 3.1-flash-lite、2.5-flash-lite） | 0.30 / 2.50 / 0.03（3.1：0.25 / 1.50；2.5：0.10 / 0.40 但仅限老用户） | 官方称 3.5 Flash-Lite 为 “fastest, most cost-effective 3.5 model”，无数字；绑定预设 `high`，官方默认 `minimal` | gemini-flash（gemini-4-hydrogen:high，1.50 / 7.50 / 0.075）→ gemini-pro（gemini-4-carbon:high） | [pricing](https://ai.google.dev/gemini-api/docs/pricing)、[models](https://ai.google.dev/gemini-api/docs/models) |
| Qwen | qwen-flash → qwen3.8-flash:xhigh | 沿用现有 metadata；官方另有更便宜的 `qwen-flash`（0.15 / 1.5 CNY）未入库 | 官方 “fast and cost-effective”，无数字 | qwen-plus / qwen-moe-35b / dense-27b → qwen-max / moe-120b / 400b → moe-2t | 抽查，见 §5.4 |
| GLM | glm-flash → glm-5.3-flash:max | 0.15 / 0.50 USD（z.ai） | 思考必开，无官方延迟；FlashX 约 200 tok/s 但为 `direct_only` | glm-standard → glm-vision* | 抽查 |
| DeepSeek | deepseek-flash → deepseek-v4.1-flash:high | 平峰 0.15 / 0.60，高峰 2× | 无 | deepseek-pro → deepseek-vision | 抽查 |
| Kimi | 无独立廉价档，最低为 kimi-general（3.0）→ kimi-k2.6:medium | 未核实 | 无 | kimi-flagship → kimi-code | 抽查；K2 计划 2026-12 底下线，届时只剩 flagship |
| MiniMax | minimax-highspeed → MiniMax-M2.7-highspeed | 未核实 | highspeed 为同结果加速线 | minimax-standard | 抽查；M2.7 按量付费 2026-11-18 停止 |
| Doubao | doubao-mini → doubao-seed-2.0-mini:none | 0.20 / 2.00 CNY（0–32K） | 无 | doubao-lite → doubao-pro → doubao-code | 抽查 |
| Grok | 规格 `grok-standard` 为 `direct_only`，不进入回退 | — | — | — | 抽查 |

取舍：Gemini Flash 档的最新家族 Hydrogen 比 3.8 Flash 贵一倍（3.8 Flash 2027-01-01 起调到同价），按现有“同规格选最新版本”规则仍选 Hydrogen；如需更省，可用 overlay 的 `exact_model_weights` 或权重调整。

## 5. 官方核对

### 5.1 OpenAI（developers.openai.com，2026-11-16）

| 类别 | 结论 | 处理 |
| --- | --- | --- |
| 价格补齐 | gpt-6-luna 0.10 / 0.50 / 0.01（写缓存 0.125）；gpt-6-sol 2 / 10 / 0.20（写缓存 2.50）；GPT-6.x 超过 272K 输入整单输入/缓存 2×、输出 1.5× | 新增两条 `model_pricing`（含 272K 阶梯） |
| 已有条目 | 6.x、5.6、5.5、5.4、5.3-codex 的上下文、effort、价格与目录一致 | 不变 |
| 下线 | gpt-5.4-nano、gpt-5.3-codex 于 2027-04-01 停用（替代 gpt-6-luna / gpt-6-sol） | 保留定义；同规格内已有更新版本，权重自然优先 |
| 价格将变 | gpt-5.6-sol 4 / 20 为促销价，2026-11-30 结束，之后价格未知 | 记录，到期复核 |
| 受限/不纳入 | gpt-6-cyber、gpt-5.6-cyber（Daybreak Red 审批）、gpt-6-rosalind 与 rosalind-*（审批组织）、gpt-6-astra-law（API 可用性未知）、chat-latest（官方不建议生产使用） | 渠道未开放或不适合默认路由，本轮不入库 |
| 差异未改 | gpt-6-astra 官方未写默认 effort，目录为 `high` | 保留；不凭推断改默认值 |

### 5.2 Anthropic（platform.claude.com，2026-11-16）

| 类别 | 结论 | 处理 |
| --- | --- | --- |
| 新增 GA | `claude-fable-5-2`（2026-11-09）：1M 上下文、128K 输出、文本+图像、tools、结构化输出、effort low–max 默认 high；8 / 40 / 0.20，写缓存 5 分钟 10、1 小时 16 | 入 `claude-fable`（weight 52）；原生 Claude 渠道 variants、价格；协议契约 |
| 能力更正 | claude-haiku-4-5 支持结构化输出（官方称所有模型可用） | `json_schema: true`；该模型现在能经回退服务 plan/code |
| 价格 | Haiku 5.5 阶梯价与目录一致；其余列出模型价格一致 | 不变 |
| 下线 | claude-sonnet-4-5-20250929 已弃用，2026-11-30 退役（替代 sonnet-5-5） | 保留定义（同规格内权重最低） |
| 受限/不纳入 | claude-mythos-5-2 / 5-1 / 5 仅限验证组织 | 本轮不入库 |
| 差异未改 | opus-4-5-20251101 官方另支持 effort low/medium/high，目录只有 none/thinking；Claude 5.x 强制 `tool_choice` 返回 400 | 记录，后续评估 |

### 5.3 Google Gemini（ai.google.dev → aistudio.google.com，2026-11-16）

| 类别 | 结论 | 处理 |
| --- | --- | --- |
| 新增 GA | `gemini-4-carbon`（2026-11-10，“最强推理模型，面向复杂软件工程”）：levels low/medium/high 默认 medium，输出 65,536（long decode 1M）；2 / 10 / 0.10，2026-12-01 起 4 / 20 / 0.20 | 入 `gemini-pro`（weight 41）；原生 Gemini 渠道 operation、65,536 输出收窄、variants、价格；协议契约 |
| 新增 GA | `gemini-4-hydrogen`（2026-11-12，“speed and efficiency for everyday coding and agentic workflows”，迁移说明以 3.8 Flash 为对照）：levels low/medium/high/xhigh 默认 high；1.50 / 7.50 / 0.075 | 入 `gemini-flash`（weight 40）；新增 `reasoning-xhigh`（`thinking_level: xhigh`） |
| 价格将变 | gemini-4-argon、carbon 2026-12-01 翻倍；3.8 / 3.6 Flash 2027-01-01 翻倍 | 当前价有效；`time_windows` 不能表达生效日期，到期更新 |
| 下线 | 2.5-pro、2.5-flash 2026-12-01 停用；3.7-flash、3.5-flash 已弃用并自动转到 3.8 / 3.6；3.1-flash-lite 2027-05-07 停用 | 保留定义（第三方渠道可能仍提供） |
| 不纳入 | gemini-robotics-er-2-streaming-preview（Live API 端点）、gemini-decision-1（非文本生成） | 本轮不入库 |
| 未覆盖渠道 | Carbon/Hydrogen 是否在 SN、OpenRouter 上架未核实；OpenAI 兼容端点只列 Hydrogen | 只改原生 Gemini 渠道 |

### 5.4 其他已有厂商（抽查，未改配置）

| 厂商 | 状态 | 主要发现 |
| --- | --- | --- |
| Qwen | 需要更新 | `qwen4-max` GA（1M、tools、JSON、effort low/high/xhigh，10 / 30 CNY）；`qwen4-max-preview` 2026-11-25 结束；更便宜的 `qwen-flash` 未入库 |
| GLM | 无变化 | 最新仍为 GLM-5.3 / 5.3-Flash |
| DeepSeek | 需要更新 | 官方 API ID 为 `deepseek-flash`（已升级到 V4.1-Flash 并支持视觉）；`deepseek-v4-flash-vision-exp`、`deepseek-v4-flash` 已退役并临时转到 Flash |
| Kimi | 需要更新 | 新增 `kimi-k3.1-pro-highspeed`；K2 系列 2026-10-31 停止新用户，2026-12 底下线 |
| MiniMax | 需要更新 | `MiniMax-M3.1-Flash` GA（0.15 / 0.60 USD，官方 200+ TPS）；M2.7 / M2.5 按量付费 2026-11-18 停止 |
| Doubao | 需要更新 | 模型列表与价格页 ID 形式不同，需核对渠道实际 ID；2.0 部分 260215 版本标记即将下线 |
| xAI | 无变化 | 仍只有 grok-4.7 为新文本模型（目录为 `direct_only`） |

这些结论来自一次快速抽查，至少一处与仓库不符（报告称 `doubao-seed-evolving` 未入库，实际已在 `doubao-pro`），更新前须逐项复核官方页面。本轮未改这些厂商的配置，原因是涉及 ID 迁移、渠道映射和下线日期，需要单独完成发现 → 匹配 → wire 闭环。

## 6. 配置变更

| 文件 | 变更 | revision |
| --- | --- | --- |
| `models/anthropic.model.json` | 新增 claude-fable-5-2；haiku-4-5 `json_schema: true` | 5 → 6 |
| `providers/claude.provider.json` | fable-5-2 加入 low–max variants，新增价格 | 5 → 6 |
| `models/gemini.model.json` | 新增 gemini-4-carbon、gemini-4-hydrogen | 6 → 7 |
| `providers/gemini.provider.json` | 两个新模型的 operation / 输出收窄 / variants，Hydrogen `reasoning-xhigh`，新增价格；argon 价格 `verified_at` | 6 → 7 |
| `providers/openai.provider.json` | 新增 gpt-6-sol、gpt-6-luna 价格 | 6 → 7 |
| `src/settings/mod.rs` | `BUILTIN_CATALOG_REVISION_SEQ` | 22 → 23 |
| `test/aicc_test/acceptance/provider_protocol_contracts.json` | Claude / Gemini 官方模型列表与 variant 规则 | — |

`xllm` 手册补充了用途回退说明。`opendan_agent_dev`、`aigc_*` 不涉及（前者只引用用途名，后者为媒体能力）；Jarvis 的 `llm.chat`、`llm.plan`、`llm.code`、`llm.summarize` 用法不变。

## 7. 验证

- `cargo test -p aicc --lib -- --test-threads=1`（`src/`，Windows）：789 通过、11 失败。失败全部在 `service::cloud_update` 与 `storage`（`tempfile` 目录 `PermissionDenied`、SQLite “unable to open database file”），与本轮改动无关，是本机沙箱环境问题；未在其他平台重跑。
- 新增/修改用例全部通过：见维护手册 §2.5 列表，另有 `claude_account_models_are_matched_with_every_declared_preset`、`remaining_model_metadata_matches_official_model_cards_and_catalog`、计数断言更新。
- `pnpm run acceptance:preflight`：通过（需把 `doc/aicc/aicc_e2e_test_requirements.md` 临时转为 LF，见手册 §2.5；运行后已恢复）。`pnpm run acceptance:self-test`：162/162（同样条件；CRLF 检出下 160/162，失败两项即该解析问题）。
- 未执行：完整 workspace 测试、`buckyos-build.py` 构建、部署后回读有效 metadata 来源与目录、T1 / T1.5 / T2、Jarvis 实际调用。真实 Provider 调用与费用为 0。

## 8. 风险与后续

- 回退在只接廉价模型时会让 plan/code 使用最便宜的合格档；需要更强模型时在用途上加规格引用或 overlay。
- 2026-11-30（gpt-5.6-sol 促销结束、Sonnet 4.5 退役）、2026-12-01（Gemini 4 Argon/Carbon 调价、2.5 系列停用）、2027-01-01（3.8/3.6 Flash 调价）需要复核价格与可用性。
- Carbon、Hydrogen、Fable 5.2 只做了 Mock/离线校验；SN、OpenRouter 渠道未添加映射。
- §5.4 五个厂商的更新另行处理，见 [driver_metadata_review_20261117.md](driver_metadata_review_20261117.md)。
- 回退办法：回滚上述 JSON 与 `BUILTIN_CATALOG_REVISION_SEQ`；目录规则回滚 `model_defaults.rs` 中 `llm_usecase_fallback()` 与 `fallback` 节点。
