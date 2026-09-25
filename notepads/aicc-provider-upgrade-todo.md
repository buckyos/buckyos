# AICC Provider 改造 TODO

日期：2026-09-25（基于提交 `b8e294c7 upgrade aicc driver model and logical tree`，补充 `model_defaults.rs` 注释契约与 Provider 实现的交叉 Review）。A/B/C 的生产链路、metadata、管理诊断及离线验收已实现；下文“现状”保留改造前 Review 背景。实现入口、价格来源、验证结果与代码量见 [Provider upgrade 实现记录](../doc/aicc/provider_upgrade_implementation.md)。

本任务包含三部分，需一并完成：

- **A. Provider inventory → Model Driver 匹配机制简化**（§1–§4）。这是核心，先做。
- **B. 计费链路修正**（§5–§8）。
- **C. Provider 职责收敛、思考预设与默认树契约补齐**（§9）。在 A 确定身份后统一生成合法预设、复用 discovery，并修复本次 Review 发现的实际缺口。

**本版本是 breaking change，不考虑兼容旧数据。** 不为旧的 Provider Rules 字段、旧 inventory 缓存、旧内部类型或旧测试保留兼容层；旧字段直接删除，出现即校验失败。设计目标是边界清晰、实现简单：删除的代码应多于新增的代码。

源码简写路径相对于 `src/frame/aicc/src/`，`providers/*.provider.json` 相对于 `src/frame/aicc/driver_metadata/`；`doc/`、`notepads/` 从仓库根定位。行号以 Review 时为准，动手前请重新定位。

---

# A. inventory → Model Driver 匹配

## 1. 新设计

匹配只回答一个问题：**Provider inventory 返回的一个 `provider_model_id`，对应哪个 Model Driver 的哪个 `models[].id`？** 结果只有三种：匹配成功 `(model_driver_id, model_id)`、失败、歧义。

匹配顺序：

1. **Provider 自定义匹配（可选）。** Provider 实现可以自己完成匹配，例如 OpenRouter 的 `anthropic/claude-sonnet-5`，按厂商前缀和别名表匹配到 `(claude, claude-sonnet-5)`。它返回以下之一：
   - `Matched(model_driver_id, model_id)`：AICC 仍要校验该 driver 存在、且 `model_id` 是该 driver 中的精确 ID；校验不通过就记为失败，不再落入通用匹配。
   - `NotHandled`：进入通用匹配。
   - `Failed(reason, candidates)`：Provider 已识别该命名规则，但结果歧义、浮动别名尚未确认或映射无效；记为 unmatched，**不再落入通用匹配**。不能用 `NotHandled` 掩盖已知的不确定映射。
2. **通用完全匹配。** `provider_model_id` 与某个 Model Driver 的 `models[].id` 完全相等。
3. **通用包含匹配**（仅在完全匹配失败时执行）。如果 `provider_model_id` 包含某个 Model Driver 的 `models[].id`，即匹配成功，例如 `gpt-5.6-2026-05-01` 匹配到 `gpt-5.6`。
   - 多个 ID 都被包含时，**取最长的**（例如 `gpt-5.6-mini` 同时包含 `gpt-5.6` 和 `gpt-5.6-mini`，取后者）。
   - 最长匹配不止一个（跨 driver 或同长度）时为**歧义**，按失败处理，并记录所有候选。
   - 已采用 §4 第 1 条建议（大小写和边界规则）。
4. **失败或歧义：** 该模型**不进入** inventory（不可路由，不挂到逻辑树），打一条 warning，并记录到 unmatched 列表，可以通过 AICC 管理接口查看（§3）。

候选范围是**全部已加载的 Model Driver**，不再按 Provider 声明 driver 白名单。某个 Provider 需要缩小范围或做特殊处理时，就实现自定义匹配。

唯一的人工修正入口：实例配置 `instance_rules.model_driver_overrides: { provider_model_id → "driver/model_id" }`，优先级最高，同样要校验目标存在。它替代现有的 `origin_model_overrides`。实例级的 `exclude_models` 以及 Provider Rules 中的 `exclude` 属于"主动排除"，不算匹配失败，不产生 warning。

完整优先级：`model_driver_overrides` → Provider 自定义匹配 → 完全匹配 → 包含匹配 → unmatched。

**浮动 API 名须先绑定版本。** 对 Provider 已知会重定向的名称，自定义匹配必须先按该 Provider 已确认的实际底层模型解析；即使名称与某个旧 `models[].id` 完全相同，也不能直接归回旧家族。未确认或目标 metadata 尚未接入时记为 `unresolved_alias`。绑定按 Provider 生效，不传播到其他 Provider 的固定旧版本。显式实例 override 仍是最高优先级的人工绑定，必须保留其来源供诊断。

### 1.1 匹配示例

以下示例使用当前 builtin Model Driver 中真实存在的 ID，例如 openai 的 `gpt-5.6`、`gpt-5.6-sol`、`gpt-5.4-mini`（**没有** `gpt-5` 和 `gpt-5.6-mini`），anthropic 的 `claude-sonnet-5`、`claude-haiku-4-5-20251001`，kimi 的 `kimi-k2.6`，minimax 的 `MiniMax-M2.7`。

#### 例 1：`stable/gpt-5-6`（自定义名字，与 driver ID 不完全一致）

场景：用户添加了一个 OpenAI 兼容网关（`openai_responses_compatible`），它的 inventory 返回 `stable/gpt-5-6`：`stable/` 是网关的渠道前缀，版本号的 `.` 被写成了 `-`。期望匹配到 `(openai, gpt-5.6)`。

按流程逐步处理：

1. **`model_driver_overrides`**：没有配置 → 继续。
2. **Provider 自定义匹配**：通用兼容 Provider 没有实现 → `NotHandled`。
3. **完全匹配**：没有 driver ID 等于 `stable/gpt-5-6` → 失败。
4. **包含匹配**：所有 driver ID 都不是 `stable/gpt-5-6` 的子串（`gpt-5.6` 中是 `.`，provider 名中是 `-`）→ 失败。
   - 注意：如果 driver 中恰好有 `gpt-5`，纯子串匹配会命中 `gpt-5`，这是错误结果。§4 的后缀规则可以挡住它：`gpt-5` 后面剩下的 `-6` 不是日期快照后缀。
5. **unmatched**：记录 `{provider_model_id: "stable/gpt-5-6", reason: no_match}`，打 warning，在 `list_unmatched_models` 中可见。该模型不可路由，同一网关的其他模型不受影响。

让它匹配正确的两种修正方式（按情况二选一）：

- **a. 用户配置 override（不改代码，适合个别自建网关）：**
  `instance_rules.model_driver_overrides: { "stable/gpt-5-6": "openai/gpt-5.6" }`。重新刷新后，第 1 步命中，校验 `openai` driver 中存在 `gpt-5.6` → 匹配成功。如果写成不存在的 `openai/gpt-5-6`，会记为 `invalid_override`，**不会**继续落入后续匹配。
- **b. Provider 实现自定义匹配（适合某个 builtin Provider 系统性地这样命名）：**
  该 Provider 的 `match_model_driver` 先去掉渠道前缀 `stable/`，得到 `gpt-5-6`；再用归一化键查询 catalog，归一化规则为转小写、`.` 替换成 `-`：`gpt-5.6` → `gpt-5-6`。
  - 恰好命中一个 → 返回 `Matched(openai, gpt-5.6)`，AICC 校验通过。
  - 命中多个（例如 driver 中同时有 `x-5-1` 和 `x-5.1`）→ 返回歧义，记为 unmatched。
  - 这个归一化**只放在 Provider 自定义匹配中**，不进通用匹配：`-` 在 `claude-sonnet-5`、`claude-haiku-4-5-20251001` 这类 ID 中本来就是合法字符，全局做 `.`/`-` 等价会扩大误匹配面。catalog 可以提供 `find_by_normalized_id` 这样的公共辅助函数，供各 Provider 复用。

#### 例 2：其他典型输入

| Provider 返回 | 命中步骤 | 结果 | 说明 |
| --- | --- | --- | --- |
| `gpt-5.6` | 完全匹配 | `(openai, gpt-5.6)` | |
| `gpt-5.6-2026-05-01` | 包含匹配 | `(openai, gpt-5.6)` | 剩余后缀是日期快照，允许。 |
| `gpt-5.6-sol-2026-05-01` | 包含匹配 | `(openai, gpt-5.6-sol)` | 同时包含 `gpt-5.6` 和 `gpt-5.6-sol`，取最长的。 |
| `accounts/fw/models/kimi-k2.6` | 包含匹配 | `(kimi, kimi-k2.6)` | 前缀是路径或命名空间，允许。 |
| `minimax-m2.7` | 包含匹配（忽略大小写） | `(minimax, MiniMax-M2.7)` | 依赖 §4 的"转小写"决定。 |
| `anthropic/claude-sonnet-5`（OpenRouter） | Provider 自定义匹配 | `(claude, claude-sonnet-5)` | 厂商前缀 `anthropic` 经别名表映射到 driver `claude`，再对 `claude-sonnet-5` 做精确校验。 |
| `gpt-5.6-mini` | 包含匹配命中 `gpt-5.6`，但被后缀规则挡住 | unmatched `no_match` | driver 中没有 `gpt-5.6-mini`。纯子串匹配会把它当成 `gpt-5.6`，这是错误结果：mini 是另一个规格。 |
| `claude-haiku-4-5`（省略了日期） | 无 | unmatched `no_match` | provider 名比 driver ID 短，包含匹配的方向不成立。用 override 修正，或在 driver 中补一个不带日期的 ID。 |
| `my-private-finetune` | 无 | unmatched `no_match` | 符合预期：没有 Model Driver 事实的模型不进 inventory。 |

## 2. 现状与需要删除的内容

当前链路（`provider/inventory.rs` 中的 `InventoryBuilder::build`，约 647 行起）有 5 个来源叠加决定 origin 和 driver，边界不清：

| 现有机制 | 位置 | 处理 |
| --- | --- | --- |
| Provider Rules 的 `origin_mappings`（正则提取 `vendor/model`）+ `origin_provider_aliases` + trim/lowercase/alias 变换 | `catalog/mod.rs` 中的 `resolve_provider_origin`、`apply_origin_mapping`、`CompiledOriginMapping` 及其校验；`providers/*.provider.json` 当前只有 OpenRouter 的 `origin_mappings` 非空 | **删除。** OpenRouter 这类带厂商前缀的，改由 Provider 自定义匹配实现（别名表写在 Rust 中，或保留为该 Provider 私有的配置）；其余 Provider 走通用匹配，已确认的浮动别名按 §1 显式处理。 |
| Provider Rules 的 `metadata_drivers` 白名单，以及 origin 必须落在白名单内的校验 | `catalog/validation.rs` 约 233、942 行；`provider/inventory.rs` 约 500、714 行 | **删除。** 候选范围改为全部 driver。 |
| `DiscoveredModel.origin_model_id`，以及它与 mapping 冲突时整个刷新失败 | `provider/inventory.rs` 约 347、689 行 | **删除该字段**，改为 Provider 自定义匹配的返回值。 |
| `instance_rules.origin_model_overrides`（只改 origin，不指定 driver） | `provider/inventory.rs` 约 704 行 | 改名为 `model_driver_overrides`，同时指定 driver 和 model，并做校验。 |
| `resolve_model` 中的 pattern、只有一个候选 driver 时的 `defaults`、`ConservativeFallback` → `"unclassified"` 三个分支；`unclassified` 的模型仍会以猜测的 api_types（默认 `llm`）进入 inventory | `catalog/mod.rs` 约 460–560 行，`catalog/schema.rs` 中的 `ModelSemantics::conservative`、`ModelMatchKind`，`model/mod.rs` 约 871 行 | **删除。** 匹配失败的模型不再进入 inventory。`resolve_model` 只保留"给定 `(driver, model_id)` 取语义"这一个功能。 |
| `AmbiguousModelDrivers` 等错误导致**整个 Provider 刷新失败** | `catalog/mod.rs` | 改为单个模型记为 unmatched，其他模型照常进入 inventory。 |
| Model Driver 的通配 `patterns` | 目前只剩 `cohere.model.json` 的 `rerank-*` | **删除**，改为精确 `models[].id` 条目。Model Driver 只用精确 ID 参与匹配；v2 中有限 ID 数组形式的 pattern 展开成精确 ID 使用，或一并改写为 `models[]`。 |

Provider Rules 中按 `provider_model_id` 匹配的 `models`/`patterns`（operations、request_rules、pricing、exclude）是 **Provider 侧规则**，不属于匹配 driver 的职责，A 部分不删除这套机制。B 部分调整价格，C 部分收敛 effort 参数转换；这些规则均不再承担 origin、driver、规格或版本先后的推断。

## 3. 可观测：unmatched 列表

- [x] `ProviderInventorySnapshot` 增加 `unmatched_models: Vec<UnmatchedInventoryModel>`，每项包含：`provider_model_id`、`reason`（`no_match` / `ambiguous { candidates }` / `invalid_provider_match { model_driver_id, model_id }` / `invalid_override` / `unresolved_alias`）、`discovered_at_ms`。
- [x] 每次刷新时，每个 `(provider_instance, provider_model_id)` 打一条 `warn!`。与上次快照相比没有变化的不重复打，避免每次定时刷新都刷屏。
- [x] 管理接口：`service/management.rs` 新增 `list_unmatched_models`（可按 `provider_instance_name` 过滤），或在 `list_providers` / `provider_health` 的返回里带上各实例的 unmatched 列表。二选一，推荐新增独立方法，返回结构更清晰。
- [x] `refresh_provider_models` 的返回值中带上本次的 unmatched 数量，方便用户添加 Provider 后立即发现问题。

## 4. 任务清单

- [x] **包含匹配的细节已落实**（采用本文建议的默认规则：包含比较忽略大小写、前缀有边界、后缀只允许日期形状；精确匹配仍优先）：
  - 大小写：建议比较前两边都转小写。
  - 边界：纯子串匹配有风险，§1.1 中的 `gpt-5.6-mini`、`stable/gpt-5-6`（driver 中有 `gpt-5` 时）都会误匹配到错误的规格，违反"宁可没有也不能写错"。只禁止"后面紧跟数字或 `.` + 数字"不够：它挡不住 `-6`、`-mini`，还会误伤合法的 `-2026-05-01`。
  - 建议规则：**前缀任意，后缀受限。** 匹配段前面可以是任意内容（命名空间、渠道前缀），但前一个字符必须是字符串开头或非字母数字；匹配段后面剩余的部分只能是空，或者是日期快照后缀（`-YYYY-MM-DD`、`-YYYYMMDD`、`-YYMMDD`、`-MMDD`）。其他后缀一律视为不同的模型，记为 unmatched。
  - 如果确认采用该规则，`gpt-5.6-2026-05-01` 能匹配，`gpt-5.6-mini` 和 `gpt-5-6` 不能匹配。后者交给 override 或 Provider 自定义匹配处理（见 §1.1 例 1）。
- [x] Provider 扩展点：在 `ProviderDiscovery` 或 Provider 注册信息上增加可选方法 `match_model_driver(&self, provider_model_id, catalog) -> ProviderModelMatch`，支持 §1 的 Matched/NotHandled/Failed，默认返回 `NotHandled`。OpenRouter 实现"厂商前缀 + 别名"匹配；已知浮动别名实现渠道版本绑定；SN 等只有确实存在特殊命名时才增加自定义匹配。
- [x] 通用匹配器：在 catalog 上建一个精确 ID 索引（`id → driver`），然后实现完全匹配和"包含 + 最长"匹配，歧义时返回全部候选。这个匹配器是纯函数，单独测试。
- [x] 按 §2 删除旧机制，同时更新 `catalog/validation.rs`：出现旧字段（`origin_mappings`、`origin_provider_aliases`、`metadata_drivers`）直接拒绝。
- [x] 迁移 `driver_metadata/providers/*.provider.json`，删除上述字段；`cohere.model.json` 的 `rerank-*` 改写为精确 ID。
- [x] 静态 inventory（GLM 的 `static_inventory_models`、`catalog_only_inventory`）也走同一个匹配器，不单独处理。删除 `catalog_only_inventory` 经 `metadata_drivers` 枚举 Model Driver 全部模型并标为 Available 的分支；仅消费 Provider 明确声明或实例显式配置的库存（详见 §9.4）。
- [x] 按 §3 实现 unmatched 列表、warning 和管理接口。
- [x] 同步文档：`doc/aicc/driver_metadata_schema.md`、`doc/aicc/frozen_model_driver_and_logical_model_fs.md` 中关于 origin mapping、metadata_drivers 和 conservative fallback 的描述。

**验收：**

- §1.1 中的每一行都有对应测试，结果与表格一致。
- `stable/gpt-5-6` 的三种情况：不做任何配置时为 unmatched；配置 override 后匹配到 `(openai, gpt-5.6)`；override 目标写错时为 `invalid_override`，且不落入后续匹配。另外需要有一个测试 Provider 实现"去前缀 + `.`/`-` 归一化"的自定义匹配，并覆盖唯一命中和歧义两种情况。
- OpenRouter 的 `anthropic/claude-sonnet-5` 通过 Provider 自定义匹配到 `(claude, claude-sonnet-5)`。自定义匹配返回的 ID 不存在时，记为 `invalid_provider_match`，且不会落入通用匹配。
- Provider 自定义匹配的 `Failed` 不继续通用匹配；浮动 API 名切换后归入已确认的新家族，未确认时为 `unresolved_alias`；另一 Provider 使用相同调用 ID 的固定旧版本仍保留旧归属。
- 无法匹配或有歧义的模型：不在 inventory 中，不在逻辑树中，也不可路由；会出现在 unmatched 列表里，并有 warning；同一 Provider 的其他模型照常可用，刷新不失败。
- 代码中不再出现 `unclassified`、`ConservativeFallback`、`origin_mappings`、`metadata_drivers`。
- 按边界规则的决定，补充对应的正例和负例测试（例如 driver 中没有 `gpt-5.6` 时的行为）。

---

# B. 计费链路

## 5. 目标原则

1. Provider 有义务提供价格信息，最好能通过网络直接更新。
2. Provider 实现的核心是：把 usage 算对，并拿到单价（此时可以实时折算单价）。
3. 价格信息宁可没有，也不能写错。
4. 同一个模型有多个 Provider 时，优先使用单价最低的。

当前链路：`静态 JSON / discovery → inventory pricing（Discovery > ProviderRules）→ 调用时锁定 PinnedPricingSnapshot → 完成后 usage × 单价（provider 自报 cost 优先）→ 路由按 estimated_cost 打分`

- Model Driver 的 `model_pricing` 已在 `b8e294c7` 删除，**但价格没有搬到 Provider Rules**。目前只有 GLM（`providers/glm.provider.json`）和 OpenRouter（discovery 动态价）有价格。
- `PricingSource::ModelDriver` 枚举项和 `service/inference.rs` 中对它的映射仍然存在（新的 InventoryBuilder 已不再产出这个来源），需要清理。

## 6. P0：会算错钱的问题

- [x] **各协议对 cache token 的语义不一致。** `TokenRates::apply`（`execution/mod.rs` 约 414 行）假设 `cache_read ⊂ input_tokens`，这是 OpenAI/Gemini 的语义。Claude 的 `input_tokens` 不包含 cache read 和 cache creation（`protocol/claude_messages.rs` 中的 `decode_usage`），因此会被多扣一次 cache 部分，`total_tokens` 也偏小。要求：适配层统一输出含义固定的计费 usage，并为每个协议补测试。
- [x] **cache 写入没有计费。** `AiUsage.cache_write_input_tokens` 已解析，但 `Pricing`、`PricingTierStep`、`PricingTimeWindow` 都没有 `cache_write_input_token` 单价，`apply` 也未使用（例如 Claude 的 cache 写入按 1.25 倍 input 价计）。
- [x] **缺失单价被当成免费。** `apply` 中 `input_token` 或 `output_token` 为 `None` 时按 0 计。应改为：usage 中有该维度、价格中没有对应单价时，整体返回 `None`（不出价）。
- [x] **阶梯价超出最后一档时回落到基础价。** `tier_rates` 中 `find` 返回 `None` 后，外层 `.unwrap_or(base)` 会取基础价。校验（`catalog/validation.rs` 中的 `validate_pricing_tiers`）没有强制最后一档必须省略 `up_to`。二选一：强制最后一档无上限，或超档时返回 `None`。
- [x] **自报 cost 一律按 USD 处理。** `protocol/openai_chat_completions.rs` 和 `openai_responses.rs` 的 `decode_usage` 只要发现 `usage.cost` 就当 USD，而 provider 自报 cost 的优先级最高。改为由 Provider 声明自报 cost 的币种和语义，未声明时忽略。

## 7. P1：让比价真正生效（原则 4）

- [x] **LLM 的 estimated_cost 实际总是 `None`。** `estimate_model_cost`（`service/inference.rs`）使用了 `estimated_input_tokens?`，而推理路径上这个值全为 `None`，所以 token 计价模型永远没有估算成本。改为不依赖真实 token 数的可比单价指数，例如固定参考量或请求画像比例。
- [x] **币种不一致时整体放弃比价。** `score_candidates`（`routing/mod.rs`）只要有一个候选币种不同，所有候选的成本都变成 `None`。需要一个汇率层：汇率表可随 cloud catalog 下发，并带有效期，过期即视为无价。
- [x] **同一家族、同一固定 effort 的多实例按价格优先。** `b8e294c7` 的路由已改为"规格权重 → 规格内版本/家族 → 同家族实例"三层。在最后一层的合格实例之间按估算单价排序，价格相同时再看延迟和可靠性。这依赖 A 的统一模型身份和 C 的统一预设身份；不能因为更便宜而跨规格、跨版本或切换 effort。
- [x] 无价候选排在可比价格之后；硬成本预算拒绝无价，普通无预算场景仍可用。原 `budget_with_unknown_cost_allows_candidate` 使用的默认约束不是硬预算，不再把它误读为硬预算允许未知。

## 8. P2：架构与数据

- [x] **删除 `PricingSource::ModelDriver`**（`provider/inventory.rs`、`call/mod.rs`、`service/inference.rs`）。价格只能来自 Provider Rules 或 discovery。
- [x] **把价格补回 Provider Rules。** 从官方页面逐个核对后填入 `providers/*.provider.json`，不能照搬已删除的旧数据。旧数据中已发现疑似错误：`claude-fable-5-1` 的 `cache_input_token` 为 2.5e-07，而 `claude-fable-5` 为 1e-06。
- [x] **价格可追溯。** `Pricing` 增加 `source_url` 和 `verified_at`（或 `effective_at`）。
- [x] **价格合理性 lint。** 检查 cache/input、output/input 等比例；比例异常必须显式标注，否则校验失败。
- [x] **价格独立发布已评估并落实。** 保留 Provider Rules 独立 `model_pricing` 表，复用现有 Provider 文档的 cloud 发布/回滚；无需修改 Model Driver，不另增 catalog kind 或配置 DSL。
- [x] **计费职责下放到 Provider。** 目前 `ProviderExecutionPort::completion_cost` 只有默认实现。可以让 Provider 输出归一化后的计费 usage，或允许其覆盖 `completion_cost`（可与 P0 第一条一起做）。
- [x] **discovery 价格与 catalog 同等校验。** `provider/mod.rs` 中的 `validate_pricing` 没有校验 `tiers` 和 `time_windows`。
- [x] **多模态 token**（音频、图片输入 token）按单独单价计费；缺少对应单价时返回 `None`。
- [x] **价格覆盖率可观测。** 管理接口列出"已匹配但无价"的模型，与 §3 的 unmatched 列表放在一起。
- [x] 为更多 Provider 实现 discovery 动态拿价（目前只有 OpenRouter）。

**验收：**

- 每个协议适配器都有 usage 语义测试：包含 cache read、cache write、reasoning 的样例响应，经计算得到确定的成本。
- 单价缺失、超出阶梯档位、币种未声明时结果为 `None`，而不是 0 或错误的价格。
- 同一模型挂在两个价格不同的 Provider 上时（包括 CNY/USD 混合），路由选中较便宜的一个。

---

# C. Provider 职责收敛与契约补齐

## 9. 按新设计简化 Provider

承接 [aicc-model-driver-logical-tree-todo.md](aicc-model-driver-logical-tree-todo.md) 和 [Model Driver v2 实现记录](../doc/aicc/model_driver_v2_implementation.md)。本节覆盖模型匹配之后的 Provider 接入，不能仅增加更多手写 variants 表就视为完成。

### 9.1 Review 基线：已完成与实际缺口

已完成：`service/model_defaults.rs` 的 7 个 LLM 功能目录共 95 条规格引用及权重与头部注释逐项一致；49 个规格均已由 metadata 声明。零库存、动态家族清理、direct_only、禁止 Parent fallback、规格权重与版本分层排序已有实现及测试。本次不重写这些机制。

| 已确认的问题 | 现状与影响 | 后续任务 |
| --- | --- | --- |
| effort 来源仍不统一 | `provider/inventory.rs::InventoryBuilder::build` 从 Provider Rules 的 variants 生成库存，只过滤 driver；`model/mod.rs::materialize_families` 才过滤 supported_efforts。非法 variant 仍已注册为 exact model，exact 直选不经过家族过滤。 | §9.3 在生成入口统一约束，并覆盖 exact 直选。 |
| Qwen 宽泛模板与模型事实冲突 | `providers/qwen.provider.json` 为全部 `qwen3*` 生成 none/mini/low/medium/high/xhigh/max；Qwen3.8-2.4T metadata 只允许 low/medium/xhigh。Qwen3.7-Plus、Qwen3.5-27B 要求 thinking，Provider 表却没有 thinking，库存中有模型仍无法执行规格指定预设。 | 区分语义 effort 和渠道参数转换，补齐合法映射、删除伪造身份。 |
| 图片/视频默认树不符合注释 | 注释声明 55 条图片/视频家族引用及不同权重；实现给图片任务增加了权重均为 1.0 的 `llm.gpt-*` 引用，视频没有对应家族权重 overlay。Auto 挂载的 exact 权重 1.0 不能替代这套家族偏好。 | §9.5 恢复注释契约，保留实际 API 能力筛选。 |
| 动态树示例与对外表示不同 | 规格→家族引用实际 weight 为 1.0，版本通过 `LlmOrder` 的数值元组独立排序；注释示例却把 560/550 写在原本表示边权的位置。排序语义已实现。 | 修正示例，区分路由权重与派生版本，不把版本写回通用 weight。 |
| 进度注释过时 | 头部仍写“尚待写入 metadata 并落实第 7 条校验”和“glm-ocr 待去掉 llm”，实际已有 metadata/校验，glm-ocr 已只保留 vision.ocr。 | 更新完成状态，继续明确标注真正缺失的模型与渠道映射。 |

Review 执行过 `cargo test -p aicc --lib --offline`：473 passed。部分 LLM 测试直接手工提供 `reasoning-{effort}` 库存，不能证明真实 Provider Rules 能生成同样的库存及请求参数；后续验收必须贯通真实 metadata、InventoryBuilder、路由和 lowering。

### 9.2 职责边界

| 层 | 保留的职责 | 应删除或移走的重复职责 |
| --- | --- | --- |
| Model Driver | 官方精确模型 ID、规格/家族、supported_efforts/default_effort/effort、稳定性、模型能力事实。 | 价格、调用参数模板、Provider 可用性、功能权重。 |
| 通用匹配与 InventoryBuilder | §1 的统一身份匹配；模型事实与渠道/Adapter 限制求交；生成合法 exact/variant；统一诊断。 | 多套 origin 推断、unknown 当 LLM、直接照抄渠道 variants 作为模型预设事实。 |
| Provider discovery | 获取本渠道模型 ID、上下架/健康、已观察到的能力限制、渠道价格；必要时实现自定义身份匹配。 | 规格归档、版本排序、功能挂载、按型号猜 effort，以及把所有已知模型当作本渠道库存。 |
| Adapter / Provider 参数转换 | 把已选定的合法 effort 转成该协议及渠道实际支持的参数；usage 归一化。协议结构由 Adapter 负责，渠道差异由 Provider 转换规则负责。 | 重复定义模型支持哪些 effort，或用一个厂商通配模板覆盖全部产品线。 |
| ModelRegistry / Router | 静态规格、动态家族、任务能力门槛、分层选择、显式 fallback、同预设实例调度。 | Provider 名称分支、渠道 ID 猜测、协议参数转换。 |

优先复用现有 `HttpTransport`、`CatalogOnlyDiscovery`、`AnthropicModelsDiscovery`、Provider Rules 和 Adapter，不引入第二套通用插件框架、配置 DSL 或依赖。Provider Rules 可继续承载渠道参数映射；其 `variants` 即使暂时保留为映射载体，也不再决定模型侧合法预设集合。非 LLM 的实际参数映射消费者须逐项核对后再删，不能按字段名一刀切。

### 9.3 P0：统一合法 effort 的生成与执行

- [x] 通用层从匹配后的 `LlmSemantics.supported_efforts` 枚举语义预设；本渠道可执行集合 = **模型声明 ∩ Adapter/Provider 可准确转换的预设 ∩ 已知渠道限制**。没有映射就标记不可执行，不以 base、medium 或 high 冒充 thinking。
- [x] `native` 使用 base；其余统一为 `reasoning-{effort}`。删除 `reasoning-mini` 等旧身份；`thinking` 与 `medium`、`native` 与 `none` 均不得互换。Provider wire 参数的合法别名转换不增加新的语义 effort。
- [x] 编译/解析一次有效渠道映射，由 inventory 和 call lowering 共用，避免 inventory 认为可用、调用时又查出 `MissingProviderVariant`。映射选择使用已匹配的模型身份及渠道限制，不能仅凭 `qwen3*` 等调用 ID 前缀套模板。
- [x] 校验并阻止非法 reasoning variant 进入 ModelRegistry；覆盖手工库存和持久缓存输入。exact 直选不能绕过 supported_efforts 检查，合法预设缺失时也不能悄悄改用另一个 effort。
- [x] 经规格或家族 selector 选中的 effort 在 lowering 后保持固定。请求选项、canonical mapping、request_rules 的应用顺序不得改变该语义；冲突须明确处理，不能显示 high 实际发送 medium。
- [x] 补齐并核验 Qwen/Kimi/GLM 的 thinking、Haiku 4.5 的 none/thinking、Gemini 3.1 Flash-Lite 的 minimal/low/medium/high、Gemini 2.5 的 none/thinking。Gemini medium 已正确映射 medium；2.5 在当前 Interactions 协议无准确映射，已生成不可执行诊断，准确调用接入仍列入下方缺口。
- [x] 核对 OpenAI `gpt-6-astra`：metadata 已声明 effort，但现有 builtin Provider variants 主要匹配 `gpt-5*`。已有模型定义不等于已有可执行的渠道映射；按确认的渠道能力补齐，不能拓宽通配符后假定所有参数通用。
- [x] Qwen 托管版与开放权重版分别匹配转换规则；2.4T 只允许 low/medium/xhigh，不生成 none/high。这里只以已确认的模型/渠道事实修复，不按同厂商相似名字推断能力。
- [x] 增加“身份已匹配、但预设缺少渠道映射”的诊断，显示 Provider、模型、effort 和原因；与 §3 的身份 unmatched 区分，保留该模型的其他有效 API/预设。

### 9.4 P1：具体 Provider 的代码收敛

注册层已经通过有效 Known Provider catalog 构建 profile/连接信息，多个 `*_profile`、`*_model_driver`、`*_catalog_files` 函数仅用于 `cfg(test)`。不要把这些测试辅助函数计作生产重复逻辑，也不要把已经共享的代码再重构一遍。

| 当前入口 | 可做的简化 | 必须保留的差异 |
| --- | --- | --- |
| `builtin/openai.rs`、`openai_responses_compatible.rs`、`kimi.rs` | 合并重复的 GET models、认证应用、HTTP/JSON 错误、ETag、ID 校验和排序流程。优先以现有 compatible discovery 为基础，Kimi 仅保留额外字段解析。 | 各 base_url 的路径拼接语义、响应限制、Kimi 的 supports_image/video/reasoning 观测；未知字段与明确不支持不能混同。 |
| GLM、DeepSeek | 二者已有 compatible discovery 复用；移除 GLM 为抵消通用层虚构 LLM/Responses 能力而做的清空步骤，统一只输出真实发现的信息。 | GLM 静态补充库存、认证/区域配置；DeepSeek 已知浮动别名的版本绑定。 |
| Claude、MiniMax | 已共用 `AnthropicModelsDiscovery`；沿用轻量参数化，核对可删除的注册包装，避免再建另一套共享发现器。 | version header、分页/游标终止、各自协议与 usage 差异。 |
| Qwen、Doubao、FAL 等 catalog-only 路径 | 统一使用 `CatalogOnlyDiscovery` 和显式 Provider 库存；移除重复的 catalog 模型构造/验证包装。 | 渠道明确的库存、参数转换、价格以及 FAL 队列任务协议。 |
| OpenRouter | origin 拆分/别名只留 §1 的一个自定义匹配入口；通用 HTTP 和 ID 校验可复用。 | 动态价格、modalities/supported_parameters、渠道状态和真实底层身份的解析。 |
| Gemini | 可共用 transport/错误处理辅助函数，保留独立响应解析；规格、effort 和逻辑挂点交给通用层。 | 分页、模型资源名、supportedGenerationMethods、协议及 thinking 参数差异。 |
| SN | 删除与统一身份匹配和预设生成重复的分支。 | 动态登录、凭证缓存/刷新、SN 专用 discovery/调用及配额行为。 |

- [x] 通用 `/models` discovery 不再默认填 `api_types=[llm]`、`remote_methods=[responses.create]`；只返回 ID 的接口将这些字段留空，由模型事实与 Adapter 支持求交。Discovery 确实报告的渠道限制必须保留，不为共用代码抹掉。
- [x] 将 `glm.rs::merge_static_glm_inventory_models` 的静态补充逻辑移入现有公共库存装配流程，使用**当前有效 CatalogSnapshot**。删除生产路径直接 `include_bytes!(...glm.provider.json)` 的读取，保证 cloud/local/system 配置更新、exclude 和库存变更能够生效。
- [x] 静态库存只来自明确的 Provider 库存声明或实例配置。删除从 `metadata_drivers` 枚举全部模型的做法，也不能把仅描述价格/参数的 Provider Rules `models[]` 自动视为上线清单。迁移当前依赖这些隐式默认库存的配置，明确填写渠道实际提供的 ID，禁止直接复制整个 Model Driver 列表伪装为库存。
- [x] 动态查询成功后删除的模型不能被旧静态表自动补回；只有明确用于该查询未覆盖 API 的渠道库存才可合并，并记录来源。发现失败后的显式 fallback 继续标为 Degraded；Model Driver 中保留旧模型定义不代表 Provider 仍提供它。
- [x] 合并发现器时迁移已有 URL、认证、响应校验、分页和失败处理测试。厂商专有行为保留小型解析/配置差异；仅在确有共享消费者时提取辅助函数，不为减少文件数扩大重构。

### 9.5 默认树与文档修复

- [x] 恢复 `model_defaults.rs` 注释约定的图片/视频家族引用与偏好权重；零 Provider 时这些静态引用仍存在，缺库存的家族无 exact 候选。不能用通用 Auto 候选绕开家族偏好，需一起检查候选排序与非 LLM 挂载。
- [x] 核对已增加的 image/audio/vision/agent_runtime → LLM 规格引用。保留有明确请求 API 支持的用途并同步注释；如果属于设计扩展，应单独写出权重和选择规则，不能无说明替换既有图片/视频家族表。不得从“LLM 支持 vision”推导图片生成或其他 API 能力。
- [x] 动态示例将真实边权写为 1.0，把 `version=(5,6,0)` / 派生展示值 560 单独标注；保留数值元组排序和 `LlmOrder`，不恢复 `version_order` 或跨层权重相乘。无需为修正文档新增公共版本字段。
- [x] 删除已完成的“metadata/第 7 条校验尚待落实”“glm-ocr 待去掉 llm”表述。未接入的模型、未确认的浮动别名和缺少渠道映射仍明确列为待办，不能用补注释代替实现。
- [x] 同步 `doc/aicc/driver_metadata_schema.md`、`doc/aicc/frozen_model_driver_and_logical_model_fs.md`、`doc/aicc/model_driver_v2_implementation.md`，区分已完成的 v2 内核和本任务才补齐的 Provider 链路。

## 10. 整体验证

- 建议顺序：A 统一身份及显式库存 → C 的 effort/调用闭环 → C 的 Provider 复用与默认树修复；B 的计费语义可独立修正，同模型比价在身份和预设一致后验收。
- **真实配置闭环**：使用 builtin Model Driver、Provider Rules、Known Provider JSON 和模拟 discovery HTTP 响应，经生产 InventoryBuilder → ModelRegistry → Router → CallResolver/Codec 检查最终 wire 参数。关键验收不能只手工构造 `InventoryModelVariant`。
- **effort 正反例**：Qwen3.8-2.4T 不产生 none/high exact variants，构造非法 exact 请求不可路由；Qwen3.7-Plus、Qwen3.5-27B 的 thinking 在有已确认映射时可执行，无映射时有诊断；thinking 不降级为 medium，native 使用 base；请求参数不覆盖固定预设。
- **多 Provider 生命周期**：相同模型/effort 在两个渠道共享家族且不叠加家族权重；移除官方库存仍保留第三方实例；最后一个实例移除后清空动态部分，保留规格/功能权重。仅增加 metadata 不能让任一 Provider 自动增加 Available 模型。
- **有效配置刷新**：覆盖 GLM 静态库存、exclude、渠道映射的 builtin/cloud/local/system 替换；新快照改变后不再读取旧编译内置值，动态下线不被静态默认值复活。
- **树契约**：逐项检查 95 条 LLM 功能引用、49 个规格及 55 条图片/视频家族引用；验证高权重空分支被跳过、版本与权重分层、无隐式根回退、direct_only 及任务能力限制。清单随显式设计变更同步更新。
- **发现器回归**：合并前后的 URL、认证、ETag、响应大小限制、模型去重、分页终止和渠道能力观测保持有测试覆盖；身份失败仅隔离单个模型，不吞掉认证、传输或整份响应格式错误。
- `cargo test -p aicc --offline` 全部通过，`cargo check -p aicc --all-targets --offline` 通过。
- 交付时统计生产代码的新增和删除行数（口径同 `doc/aicc/model_driver_v2_implementation.md`），单列测试、metadata 和文档。A 与 C 的 Provider 收敛部分预期生产代码净减少；新增契约校验和计费修正单列，不能用删除测试抵消生产增长。

## 11. 本次交付与剩余接入边界

- A/B/C 的代码与离线验收完成；管理诊断选择扩展 `list_providers.inventory`，没有新增独立 RPC。库存 schema 升为 2，旧 origin 配置与旧缓存不做兼容。
- Gemini 2.5 的 none/thinking 仍缺当前 Interactions 协议的准确映射；GenerateContent 的 thinkingBudget 不可直接搬用。模型身份可匹配，但预设不可执行并有诊断。这里的完成表示核验、隔离和诊断完成，不表示该渠道调用已接通。
- DeepSeek 官方旧 Flash 名按渠道处理；V4.1 metadata 尚缺时为 unresolved_alias。新增有效 metadata 后会绑定新身份，其它 Provider 的固定 V4 保留旧家族，均有测试。
- 缺官方依据、双维阈值无法表达或账单单位不可观测的价格保持 unknown。FAL 动态价格只作参考估价；GLM 中国站和 MiniMax 国际站价格按 region 限定。汇率由有效 catalog 下发，未内置猜测值。
- A/C 发现器与身份解析生产代码净减少 397 行；完整任务因计费和契约补齐净增加 540 行，整体净减少目标未达到。测试、metadata 与文档不抵消生产增长。
- 未增加依赖，未使用真实 API Key 或进行付费推理；未执行整仓部署构建。全部验证及生产/测试/metadata/文档统计见实现记录。
