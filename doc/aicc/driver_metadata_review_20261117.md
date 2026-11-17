# AICC 模型目录维护记录：Qwen / DeepSeek / Kimi / MiniMax / Doubao（2026-11-17）

按 [LLM 模型目录例行维护](maintenance/llm_catalog_routine_maintenance.md) 执行，处理 [2026-11-16 记录](driver_metadata_review_20261116.md) §5.4 标为“需要更新”的五个厂商。

- 基线：提交 `af0c007f9`，工作区带 2026-11-16 轮未提交改动（`BUILTIN_CATALOG_REVISION_SEQ = 23`）；本轮改动叠加其上，同样未提交。
- 官方页面核对日期：2026-11-16（部分页面自带时间戳为北京时间 2026-11-17）。只采信原厂页面；第三方资料只用于找 URL。
- 本轮范围：上述五家，含原生渠道及 Doubao（火山方舟）上的 DeepSeek 第三方 ID。OpenAI / Anthropic / Gemini 已于 2026-11-16 完成；GLM、xAI 在 11-16 抽查无变化，本轮未复查。

## 1. 抽查线索复核

| 厂商 | 11-16 抽查说法 | 官方核对结论 | 处理 |
| --- | --- | --- | --- |
| Qwen | `qwen4-max` GA：1M、tools、JSON、effort low/high/xhigh、10/30 CNY | 成立，有修正：2026-11-16 发布，默认 effort 为 `high`，另有 `none` 可关闭思考；思考模式输出 262,144、非思考 131,072；同时支持 JSON Object / JSON Schema；输入含图片、视频 | 新增 |
| Qwen | `qwen4-max-preview` 2026-11-25 结束 | 成立：仅 Token Plan、不支持按量付费，2026-11-11 至 11-25 24:00（UTC+8），之后下线，建议换 qwen4-max | 不入库，记录 |
| Qwen | 存在更便宜的 `qwen-flash` 未入库 | 成立但不入库：Qwen3 代旧版别名（等同 `qwen-flash-2025-07-28`），官方列入“旧版模型、不再作为首选推荐”；见 §3.1 | 不入库，记录 |
| DeepSeek | 官方 API ID 为 `deepseek-flash`，不是 `deepseek-v4.1-flash` | 成立；`deepseek-v4.1-flash` 不出现在任何官方页面。但仓库早已用 `model_driver_overrides` 把 `deepseek-flash` 映射到家族 `deepseek-v4.1-flash`，测试 `deepseek_inventory_uses_provider_metadata_for_aliases_and_exclusions` 已覆盖 | 无需改映射 |
| DeepSeek | `deepseek-v4-flash-vision-exp`、`deepseek-v4-flash` 已退役并转到 Flash | 成立：2026-09-10 退役，名称仍可调用，按 Flash 计价，“临时”路由无截止日。仓库已映射到 `deepseek-v4.1-flash` | 无需改映射 |
| Kimi | 新增 `kimi-k3.1-pro-highspeed` | 成立 | 新增 |
| Kimi | K2 系列（含 `kimi-k2.8-preview`）关闭新用户、12 月底下线 | 部分成立：只涉及 `kimi-k2.6`、`kimi-k2.7-code`、`kimi-k2.7-code-highspeed`；`kimi-k2.8-preview` 不是 Kimi API 型号（只出现在 Kimi Code 订阅产品中） | 记录日期，保留定义 |
| MiniMax | `MiniMax-M3.1-Flash` GA，0.15/0.60 USD，200+ TPS | 成立：2026-10-19 发布（国际与国内同日）；200+ TPS 出自官方博客，P50 为 170+ TPS | 新增 |
| MiniMax | M2.7/M2.5 按量付费 2026-11-18 停止 | 成立：2026-10-19 起不对新用户开放，11-18 下线；Token Plan 请求自 10-19 起自动转到 M3.1-Flash。官方表格已把全部 M2.x 列为 Retired，价格页不再列出 M2.x | 删除原生渠道价格，保留定义 |
| Doubao | 模型列表与价格页 ID 形式不同 | 成立：模型列表（及 API `model` 参数）用带日期的 Model ID（如 `doubao-seed-2-1-pro-260915`），价格页按产品名（`doubao-seed-2.1-pro`）一行计价。仓库早已用产品名作 canonical ID，并用 `model_driver_overrides` 映射日期 ID | 无需改映射；补价格 |
| Doubao | 2.0 部分 `-260215` 版本即将下线 | 成立，见 §3.5 | 记录日期 |
| Doubao | `doubao-seed-evolving` 未入库 | 抽查错误：已在 `doubao-pro`；官方 ID 无日期，自动升级（最近一次 2026-11-05） | 补价格 |

## 2. 配置变更

| 文件 | 变更 | revision |
| --- | --- | --- |
| `models/qwen.model.json` | 新增 `qwen4-max`（`qwen-max`，weight 40，effort/default `high`，supported none/low/high/xhigh，1,000,000 / 262,144，视觉、tools、JSON Schema） | 9 → 10 |
| `providers/qwen.provider.json` | qwen4-max 加入 none/low/xhigh 变体，新增 `reasoning-high`；北京与新加坡价格；删除 2026-10-10 已退役的 `qwen3.6-max-preview`、`qwen3-max`、`qwen3-max-2025-09-23`、`qwen3-max-2026-01-23` 及 `qwen3-max*` 通配价格 | 14 → 15 |
| `providers/deepseek.provider.json` | 价格复核未变，`verified_at` 更新 | 5 → 6 |
| `models/kimi.model.json` | 新增 `kimi-k3.1-pro-highspeed` 与 `direct_only` 规格 `kimi-flagship-highspeed` | 8 → 9 |
| `providers/kimi.provider.json` | highspeed 精确规则（移除固定采样参数与 `thinking`）、low/high/max 变体、价格；K3.1 Pro / K3 / K2 价格复核未变，`verified_at` 更新 | 10 → 11 |
| `models/minimax.model.json` | 新增 `MiniMax-M3.1-Flash`（主规格 `minimax-standard:max` weight 32，`additional_specs` 绑定 `minimax-highspeed:low` weight 32）；因使用 `additional_specs`，`schema_revision` 0 → 1 | 9 → 10 |
| `providers/minimax.provider.json` | M3.1-Flash 加入 adaptive low–max 变体；新增 M3.1-Flash 国际/国内价格、M3 国际价格；删除 M2.7/M2.7-highspeed/M2.5/M2.5-highspeed/M2.1/M2.1-highspeed/M2 原生价格（共 9 条）；M3 国内价格 `verified_at` 更新 | 7 → 8 |
| `providers/doubao.provider.json` | 映射 `deepseek-v4-1-flash-261017`、`deepseek-v4-1-flash-economy-261017` → `deepseek-v4.1-flash`；新增 2.1-pro/evolving、2.1-lite、2.0-mini、character、translation、DeepSeek economy 价格；更正 2.0-lite 缓存命中分档（0.12/0.18/0.36）；`deepseek-v4-1-flash-260910` 与 261017 共用一条价格；删除已下架的 `deepseek-v4-flash-260425` 价格 | 14 → 15 |
| `src/settings/mod.rs` | `BUILTIN_CATALOG_REVISION_SEQ` | 23 → 24 |
| `src/service/model_defaults.rs` | 头部归档说明与规格目录（`kimi-flagship-highspeed [direct_only]`、qwen4-max、M3.1-Flash） | — |
| `test/aicc_test/acceptance/provider_protocol_contracts.json` | MiniMax 官方 ID 改为 M3.1-Flash、M3（去掉 Preview 与已退役 M2.x），测试模型与变体规则改为 M3.1-Flash；Kimi 加入 highspeed 及其变体；Qwen 加入 qwen4-max 与 none/low/high/xhigh 变体规则 | — |

规格总数 54 → 55（新增 `kimi-flagship-highspeed`，`direct_only`）。用途表与 `llm.fallback` 的条目、权重不变（仍为 141 条）。

测试改动：新增 `provider::builtin::upgrade_tests::cn_vendor_models_reviewed_on_2026_11_16_close_the_discovery_to_route_loop`（原生 Qwen/Kimi/MiniMax 及方舟渠道的发现 → 身份匹配 → 预设/价格 → 路由，另断言 MiniMax 原生渠道不再给 M2.x 计价）；更新 `provider_pricing_contains_all_rebased_model_defaults`（doubao 18、kimi 6、minimax 17、qwen 96）、`every_builtin_provider_price_has_provenance`（334）、`builtin_logical_tree_is_not_an_inventory_snapshot`（规格 55）、`reviewed_vendor_token_limits_match_official_hard_limits`（加三个新模型）、`provider_rules_are_loaded_without_rust_generated_revisions`（doubao 15、deepseek 6、qwen 15）。

## 3. 官方核对结论

### 3.1 Qwen（阿里云百炼）

| 类别 | 结论 | 来源 |
| --- | --- | --- |
| 新增 GA | `qwen4-max`：2026-11-16 在北京、新加坡及弗吉尼亚/法兰克福/东京/香港上线；上下文 1,000,000；最大输入 991,808（思考 983,616）；最大输出 131,072（非思考）/ 262,144（思考），思维链 262,144；文本/图片/视频输入；tools、并行工具、JSON Object 与 JSON Schema；Chat Completions / Responses / Anthropic Messages / DashScope 均支持；`reasoning_effort`（Responses 为 `reasoning.effort`）low/high/xhigh、默认 high，`none` 关闭思考，minimal→low、medium→high、max→xhigh 为别名；从 qwen4-max 起 `max_tokens` 与 `max_completion_tokens` 同义，限制思考 + 回答总量 | [qwen4-max](https://help.aliyun.com/zh/model-studio/qwen4-max)、[英文页](https://www.alibabacloud.com/help/en/model-studio/qwen4-max)、[新模型发布](https://help.aliyun.com/zh/model-studio/newly-released-models)、[深度思考](https://help.aliyun.com/zh/model-studio/deep-thinking)、[API 兼容升级说明](https://help.aliyun.com/zh/model-studio/api-compatibility-upgrade-notice) |
| 价格 | 北京 10 / 30 CNY，缓存命中 0.5，显式缓存写入 5 分钟 12.5 / 1 小时 20；新加坡（中文页 CNY）10.938 / 32.813，命中 0.547，写入 13.672 / 21.875；英文页新加坡 1.5 / 4.5 USD。思考与非思考同价，0–1M 单档 | [模型价格](https://help.aliyun.com/zh/model-studio/model-pricing)、[Qwen4 显式缓存](https://help.aliyun.com/zh/model-studio/explicit-cache-qwen4) |
| 渠道未开放 | `qwen4-max-preview`：仅 Token Plan，effort low/medium/xhigh 默认 xhigh，2026-11-25 24:00（UTC+8）后下线 | [qwen4-max-preview](https://help.aliyun.com/zh/model-studio/qwen4-max-preview) |
| 不入库 | `qwen-flash`：Qwen3 系列 Flash 别名，等同 `qwen-flash-2025-07-28`，官方“旧版模型、不再作为首选推荐”；仅文本输入，输出 32,768；JSON 仅 Object（且仅非思考）；tools 仅北京支持、新加坡/法兰克福/弗吉尼亚不支持；思考默认关闭、无 `reasoning_effort` 文档；北京 ≤128K 0.15 / 1.5 CNY | [qwen-flash](https://help.aliyun.com/zh/model-studio/qwen-flash)、[文本生成模型](https://help.aliyun.com/zh/model-studio/text-generation-model)、[结构化输出](https://help.aliyun.com/zh/model-studio/qwen-structured-output) |
| 下线 | `qwen3.6-max-preview`、`qwen3-max`、`qwen3-max-2025-09-23`、`qwen3-max-2026-01-23` 于 2026-10-10 退役（替代 qwen3.7-max）：保留定义，删除原生价格。`qwen3.5-plus`（含 -2026-02-15、-2026-04-20）与 `qwen3.5-397b-a17b`：2026-11-18 停止新购，2027-01-19 停服（替代 qwen3.8-flash）：只记录 | [模型下线](https://help.aliyun.com/zh/model-studio/model-depreciation)、公告 www.aliyun.com/notice/118344、118345、118728 |
| 已知不变 | qwen3.8-max（1M、131,072、effort none/low/medium/xhigh 默认 xhigh、12/36、命中 1.5）、qwen3.8-flash（0.8/2.7、命中 0.1）与目录一致 | [qwen3.8-max](https://help.aliyun.com/zh/model-studio/qwen3-8-max)、[qwen3.8-flash](https://help.aliyun.com/zh/model-studio/qwen3-8-flash) |

`qwen-flash` 不入库的理由：同规格按版本权重选最新家族，它不会在 qwen3.8-flash 存在时被选中；`qwen-flash` 作为家族名与规格 `qwen-flash` 路径冲突，只能以 `qwen-flash-2025-07-28` 建模并映射浮动别名；tools 能力随地域变化，现有规则无法按地域收窄。若以后需要它作廉价候补，应单独评估。

`qwen3-max-preview` 的状态未核实，保留其价格。官方价格页说明“缓存命中按 10% 计费”，但 qwen4-max 表内命中价为输入的 5%；按表内数值录入。

### 3.2 DeepSeek（api-docs.deepseek.com）

| 类别 | 结论 | 来源 |
| --- | --- | --- |
| 当前 ID | `model` 枚举只有 `deepseek-flash`（→ DeepSeek-V4.1-Flash-1017，2026-10-17 起，API 与价格不变）与 `deepseek-v4-pro`（→ V4-Pro-0813） | [pricing](https://api-docs.deepseek.com/quick_start/pricing)、[updates](https://api-docs.deepseek.com/updates)、[create-response](https://api-docs.deepseek.com/api/create-response) |
| 退役（名称仍可用） | `deepseek-v4-flash`、`deepseek-v4-flash-vision-exp`：2026-09-10 退役，临时由 V4.1-Flash-1017 按 Flash 价格服务；`deepseek-chat` / `deepseek-reasoner` 2026-07-24 15:59 UTC 后不可用 | [news260910](https://api-docs.deepseek.com/news/news260910)、[updates](https://api-docs.deepseek.com/updates) |
| 能力 | 两者 1M 上下文、393,216 输出；Flash 支持图片输入，V4-Pro 不支持；effort none/low/high/max，默认 high；JSON Schema 仅 Responses API（Chat Completions 只有 json_object）——原生渠道使用 `responses.create`，与 `json_schema: true` 一致 | [vision](https://api-docs.deepseek.com/guides/vision)、[thinking_mode](https://api-docs.deepseek.com/guides/thinking_mode)、[json_mode](https://api-docs.deepseek.com/guides/json_mode) |
| 价格 | 闲时：Flash 0.15 / 0.60 / 命中 0.003 USD，V4-Pro 0.66 / 1.98 / 0.022；高峰 2×：周一至周五 01:00–04:00、06:00–10:00 UTC（= 北京 09–12、14–18），两模型同一时段；周末与中国法定节假日全天闲时 | [pricing](https://api-docs.deepseek.com/quick_start/pricing)、[news260813](https://api-docs.deepseek.com/news/news260813) |
| 官方内部矛盾 | news260910 称 2026-09-14 起 `deepseek-v4-pro` 转到 Flash；更新日志与价格页均显示 V4-Pro 继续按 Pro 计价。按较新的价格页处理 | 同上 |

目录与价格均一致，只更新 `verified_at`。`time_windows` 无法表达法定节假日（节假日按高峰时段计价会偏高）。

### 3.3 Kimi（platform.kimi.com / platform.kimi.ai）

| 类别 | 结论 | 来源 |
| --- | --- | --- |
| 新增 | `kimi-k3.1-pro-highspeed`：2026-10-24（官方博客）；K3.1 Pro 高速版，约 2–3 倍速度、同等智能，无 tokens/s 数字；1,048,576 上下文，最大输出 1,048,576（默认 131,072）；tools；`reasoning_effort` low/high/max 默认 max，不支持 `thinking`；采样参数固定（temperature 1.0、top_p 0.95、n 1、两种 penalty 0） | [models](https://platform.kimi.com/docs/models)、[models-overview](https://platform.kimi.com/docs/api/models-overview)、[chat](https://platform.kimi.com/docs/api/chat)、[K3.1 Pro 博客](https://www.kimi.com/en/blog/kimi-k3-1-pro) |
| 未声明 | 视觉指南与结构化输出指南只点名 `kimi-k3.1-pro`、`kimi-k3`；highspeed 未声明图片/视频输入和 JSON Schema，按规则不推断，目录不写 vision / json_schema | [vision](https://platform.kimi.com/docs/guide/use-kimi-vision-model)、[response_format](https://platform.kimi.com/docs/guide/response_format) |
| 价格 | highspeed（CNY/百万）：输入 26、输出 136、缓存命中 1.3、写入 5 分钟 26 / 1 小时 52；国际 4 / 20 / 0.20 USD。K3.1 Pro 13/68/1.3/13/26、K3 20/100/2/20/40、K2.7-code 6.5/27/1.3、K2.7-code-highspeed 13/54/2.6、K2.6 6.5/27/1.1 均未变 | [pricing/chat](https://platform.kimi.com/docs/pricing/chat)、[国际](https://platform.kimi.ai/docs/pricing/chat) |
| 下线 | `kimi-k2.6`、`kimi-k2.7-code`、`kimi-k2.7-code-highspeed`：2026-10-31 起不对新注册用户开放（11-01 前注册的用户可继续用），预计 2026 年 12 月底全面下线，未给具体日期；迁移指引指向 K3.1 Pro / K3 | [models](https://platform.kimi.com/docs/models)、[pricing/chat](https://platform.kimi.com/docs/pricing/chat) |
| 非 API 型号 | `kimi-k2.8-preview` 不在 Kimi API 型号、价格或枚举中，只出现在 Kimi Code 订阅（ID `kimi-for-coding`）。目录定义保留（百炼、方舟 Agent Plan 渠道仍使用） | [Kimi Code models](https://www.kimi.com/code/docs/en/kimi-code/models.html) |

highspeed 单列 `kimi-flagship-highspeed` 并标记 `direct_only`，与 `kimi-code-highspeed` 的处理一致：同等智能、价格翻倍的加速版不应在默认路由或回退中被自动选中（包括 K3.1 Pro 暂不可用时）。

### 3.4 MiniMax（platform.minimax.io / platform.minimax.cn）

`platform.minimaxi.com` 现 302 跳转到 `platform.minimax.cn`。

| 类别 | 结论 | 来源 |
| --- | --- | --- |
| 新增 GA | `MiniMax-M3.1-Flash`：2026-10-19 发布；可用 LLM 仅 M3.1-Flash 与 M3；1,000,000 上下文，`max_tokens` 最大 524,288（含思考）；文本/图片/视频输入；tools（Anthropic 兼容 `tool_choice` 仅 auto）；`thinking: {type: adaptive}` + `output_config.effort` low/medium/high/xhigh/max，默认 max，`none` 与 `thinking.disabled` 返回 400；结构化输出未声明 | [models FAQ](https://platform.minimax.io/docs/faq/models)、[release notes](https://platform.minimax.io/docs/release-notes/models)、[text-chat-anthropic](https://platform.minimax.io/docs/api-reference/text-chat-anthropic)、[text-generation](https://platform.minimax.io/docs/guides/text-generation) |
| 速度依据 | 官方博客：日常使用 200+ TPS，P50 170+ TPS | [M3.1 Flash 博客](https://www.minimax.io/blog/minimax-m31f) |
| 价格 | M3.1-Flash：0.15 / 0.60 / 命中 0.015 USD；1 / 4 / 0.1 CNY；无输入长度分档，未列缓存写入。M3 标准档（“永久五折”）：国际 ≤512K 0.30 / 1.20 / 0.06，>512K 0.60 / 2.40 / 0.12 USD；国内 2.1 / 8.4 / 0.42、4.2 / 16.8 / 0.84 CNY | [国际价格](https://platform.minimax.io/docs/guides/pricing-paygo)、[国内价格](https://platform.minimax.cn/docs/guides/pricing-paygo) |
| 下线 | M2.5 / M2.7 系列：2026-10-19 起不对新用户开放，2026-11-18 下线（未给时刻与时区）；Token Plan 请求自 10-19 起自动转到 M3.1-Flash。官方表格将 M2.7、M2.7-highspeed、M2.5、M2.5-highspeed、M2.1、M2.1-highspeed、M2、M2-her 均列为 Retired；价格页不再列出 M2.x | [models FAQ](https://platform.minimax.io/docs/faq/models)、[国内 FAQ](https://platform.minimax.cn/docs/faq/models) |
| 未声明 | `MiniMax-M3.1-Flash-Preview` 不再出现在任何官方 API 页面，退役/别名状态未知；M3 未写 JSON Schema；缓存写入价格均未列出 | — |
| 差异未改 | M3 新增 `service_tier: priority`（1.5× 标准价），目录未建模；M3 省略 `thinking` 时的默认状态，Anthropic 与 OpenAI 兼容文档说法相反 | 同上 |

M3.1-Flash 归 `minimax-standard`（沿用 11-16 前对 Preview 的归档，weight 32 高于 Preview 31、M3 30），并通过 `additional_specs` 以 `:low` 绑定 `minimax-highspeed`。理由：M2.x highspeed 本周全部退役后，原生渠道的 `minimax-highspeed`（swift 与 `llm.fallback` 4.0 层的 MiniMax 入口）将没有候选；M3.1-Flash 是 MiniMax 当前最便宜、官方给出最高吞吐数字的模型，用合法的 `additional_specs` 绑定低档预设，不改共享规格 effort。结构化输出未声明，所以 M3.1-Flash 进入 plan/code 时按缺少 JSON Schema 拒绝（与 Preview 相同）。

原生渠道 M2.x 价格全部删除：官方表格已列为 Retired、价格页不再提供、无法复核；定义保留，百炼聚合的 `MiniMax/MiniMax-M2.x` 价格不受影响。

### 3.5 Doubao（火山方舟）

| 类别 | 结论 | 来源 |
| --- | --- | --- |
| ID 形式 | 模型列表与 API 文档使用带日期的 Model ID（可传 Endpoint ID）；价格页按产品名一行；点号或无日期形式能否调用未声明。仓库以产品名为 canonical ID、日期 ID 经 `model_driver_overrides` 映射，与官方一致 | [模型列表](https://docs.volcengine.com/docs/ark/model-list?lang=zh)、[价格](https://docs.volcengine.com/docs/ark/model-pricing?lang=zh)、[Chat API](https://docs.volcengine.com/docs/ark/chat-api?lang=zh) |
| 在售 Seed LLM | evolving、2-1-pro-260915、2-1-lite-260915、2-1-turbo-260628、2-1-pro-260628、2-0-lite-260428（LTS）、2-0-mini-260428（LTS）、character-260628 / 251128、translation-250915：均已在目录中；没有 2.1 mini、2.2 或新的 evolving / code 快照 | 模型列表 |
| 下线 | 第 10 批：2026-09-24 停止新购，**2026-11-24 14:00（UTC+8）** 停服：`doubao-seed-2-0-pro-260215`（替代 2-1-lite-260915，自动替换待定）、`doubao-seed-2-0-code-preview-260215`（→ evolving）、`deepseek-v4-pro-260425`（→ deepseek-v4-1-flash-261017，自动替换为 economy 版）、`glm-5-1-260428`（→ evolving）。`doubao-seed-2-0-lite-260215`、`-mini-260215` 停服日为 2026-10-22，但模型列表仍标“即将下线”（两页矛盾）。LTS 2-0-lite/mini-260428 最早 2027-04-30 下线，至少提前 3 个月通知 | [下线公告](https://docs.volcengine.com/docs/ark/model-deprecation-notice?lang=zh)、[LTS](https://docs.volcengine.com/docs/ark/lts-mechanism?lang=zh) |
| 价格（CNY/百万，在线推理常规） | evolving 与 2.1-pro 6 / 30 / 命中 1.2；2.1-lite 0.8 / 2.7 / 0.16（音频输入 12，未建模）；2.1-turbo 3 / 15 / 0.6（不变）；2.0-lite 分档 [0,32K] / (32,128K] / (128,256K]：0.6/0.9/1.8、输出 3.6/5.4/10.8、命中 0.12/0.18/0.36（原目录命中三档都写 0.12，已更正）；2.0-mini 0.2/0.4/0.8、2/4/8、0.04/0.08/0.16；character 0.8/1.2、2/6、命中 0.16；translation 1.2 / 3.6 无缓存 | 价格 |
| 第三方 ID | 新增 `deepseek-v4-1-flash-261017`（推荐版本）与 `deepseek-v4-1-flash-economy-261017`（官方说明与标准版权重完全相同、能力一致，价格更低、生成略慢）：闲时 1 / 4 / 0.02 与 0.8 / 3.2 / 0.016，高峰（工作日北京 09–12、14–18）2×；1024K 上下文、384K 输出。`deepseek-v4-flash-260425` 已从列表移除（9-21 停服），删除其价格；`deepseek-v4-1-flash-260910`、`deepseek-v4-flash-ga-260731`、`deepseek-v4-pro-ga-260813`、GLM 两个 ID 仍在列表 | 模型列表、价格、[Agent Plan 模型发布](https://docs.volcengine.com/docs/zh/ark/agent-plan-personal-model-release.md) |
| 未接入 | `kimi-k3-1-pro-261024`（1024K / 1024K，13 / 68 / 1.3 CNY）：方舟上的 Kimi 预设参数与固定采样参数处理未核实，本轮不映射；`doubao-seed-2-0-pro-260215` 即将停服，不入库 | 同上 |

方舟价格页的 2.1-pro 只有一行，未说明 260628 与 260915 是否同价；沿用现有做法（产品行价格用于该产品各日期版本，2-1-pro-260628 另有 256K 上下文收窄）。

## 4. 用途覆盖变化

- `qwen4-max` 进入 `qwen-max`（weight 40），该规格的首选由 qwen3.8-max:xhigh（12/36）变为 qwen4-max:high（10/30），影响 chat/plan/code/vision 中的 `qwen_max` 条目与 `llm.fallback` 2.0 层。
- `MiniMax-M3.1-Flash` 成为 `minimax-standard` 首选（:max）与 `minimax-highspeed` 首选（:low）；swift、summarize、translate 的 `minimax_highspeed` 与 `llm.fallback` 4.0 层因此在 M2.x 退役后仍有 MiniMax 候选。
- `kimi-k3.1-pro-highspeed` 只能按规格名或家族名直选（`llm.kimi-flagship-highspeed`、`llm.kimi-k3-1-pro-highspeed`），不进入任何用途或回退。
- Doubao 各 Seed 规格的首选不变，但方舟渠道补齐价格后，`cost_first` 排序不再把它们当作未知价格（最差成本）。
- 单模型覆盖测试 `builtin_single_llm_serves_every_usecase_it_is_capable_of` 覆盖了全部新模型：每个用途要么有候选，要么有明确能力拒绝（M3.1-Flash 在 plan/code 因缺 JSON Schema 被拒），`direct_only` 的 highspeed 没有泄漏。

## 5. fallback 选型（五个系列）

费用为原生渠道标准价（输入 / 输出 / 缓存命中，每百万 token），不代表 SN、OpenRouter 等渠道实际报价。

| 系列 | 4.0 档首选（规格 → 家族:预设） | 费用 | 速度依据 | 后备顺序 | 来源（核对 2026-11-16） |
| --- | --- | --- | --- | --- | --- |
| Qwen | qwen-flash → qwen3.8-flash:xhigh | 0.8 / 2.7 / 0.1 CNY（北京） | 官方列为成本优先推荐，无延迟数字 | qwen-plus / moe-35b / dense-27b → qwen-max（qwen4-max:high，10 / 30 / 0.5）/ moe-120b / 400b → moe-2t | §3.1 |
| DeepSeek | deepseek-flash → deepseek-v4.1-flash:high（原生 ID `deepseek-flash`） | 0.15 / 0.60 / 0.003 USD 闲时，工作日 01–04、06–10 UTC 2×；方舟 economy 0.8 / 3.2 / 0.016 CNY | 无官方延迟数字；方舟 economy 官方称生成略慢 | deepseek-pro → deepseek-vision | §3.2、§3.5 |
| Kimi | 无独立廉价档；最低 kimi-general → kimi-k2.6:medium | 6.5 / 27 / 1.1 CNY | 无 | kimi-flagship（kimi-k3.1-pro:max，13 / 68 / 1.3）→ kimi-code | §3.3；K2 自 10-31 不对新用户开放、12 月底下线，之后原生渠道最低为 kimi-flagship |
| MiniMax | minimax-highspeed → MiniMax-M3.1-Flash:low | 0.15 / 0.60 / 0.015 USD；1 / 4 / 0.1 CNY | 官方博客 200+ TPS（P50 170+） | minimax-standard（M3.1-Flash:max，其次 M3） | §3.4 |
| Doubao | doubao-mini → doubao-seed-2.0-mini:none | ≤32K：0.2 / 2.0 / 0.04 CNY（分三档） | 无官方延迟数字；:none 为非思考 | doubao-lite（2.1-turbo:none，3 / 15 / 0.6）→ doubao-pro（evolving:high，6 / 30 / 1.2）→ doubao-code（2.0-code-preview:medium） | §3.5、§9 |

## 6. 渠道与生效

- 未改 SN：`minimax-m3.1-flash` 仍映射 `MiniMax-M3.1-Flash-Preview`，`deepseek-v4.1-flash-1017` 等映射保持；SN 实际底层模型无法从外部核实。
- 未改 OpenRouter：拉取 `openrouter.ai/api/v1/models` 得到的是不符合 OpenRouter 模型列表格式的 10 条记录，无法确认新模型是否上架。OpenRouter 按 `vendor/model` 自动匹配目录 ID，因此 `qwen/qwen4-max`、`minimax/minimax-m3.1-flash`、`moonshotai/kimi-k3.1-pro-highspeed` 一旦上架会直接匹配；highspeed 届时需要与 K3.1 Pro 相同的固定采样参数移除规则。
- 未改百炼聚合渠道（`kimi/*`、`MiniMax/*`、`vanchin/*`）与方舟 Agent Plan：厂商原生下线不自动传播到这些渠道，本轮未核实其状态。
- 有效来源与 revision：builtin metadata 随二进制发布，本轮未构建、未部署，未回读 AICC 的 metadata 来源视图与逻辑目录；部署后需确认没有旧的 local / system-config 文件遮蔽新的 builtin revision 24。

## 7. 验证

- `cargo test -p aicc --lib -- --test-threads=1`（`src/`，Windows）：790 通过、11 失败。11 个失败与改动前基线完全相同，全部在 `service::cloud_update`（5）与 `storage`（6），是本机沙箱的临时目录 / SQLite 权限问题。改动前基线为 789 通过、11 失败；新增 1 个用例。
- `pnpm run acceptance:preflight`：通过；`pnpm run acceptance:self-test`：162/162。运行前把 `doc/aicc/aicc_e2e_test_requirements.md` 临时转为 LF，结束后从备份恢复原 CRLF 文件。
- 新价格与关键事实另做了独立复核：Kimi highspeed 价格行和方舟 Seed / DeepSeek 价格行直接核对官方页面原始导出；MiniMax 国际/国内价格页与 qwen4-max 中英文页由第二次独立抓取确认。
- 未执行：完整 workspace 测试、`buckyos-build.py` 构建、部署后回读、T1 / T1.5 / T2、真实 Provider 调用（费用 0）。

## 8. 风险与后续

到期复核：

- 2026-11-18：MiniMax M2.5/M2.7 下线；百炼 `qwen3.5-plus` 系列停止新购。
- 2026-11-24：方舟 `doubao-seed-2-0-pro-260215`、`2-0-code-preview-260215`、`deepseek-v4-pro-260425`、`glm-5-1-260428` 停服。
- 2026-11-25：`qwen4-max-preview` 结束。
- 2026-12 月底：Kimi K2 系列下线。
- 2027-01-19：`qwen3.5-plus`、`qwen3.5-397b-a17b` 停服。

本轮发现、未处理：

- Doubao 结构化输出：方舟只对 2-1-pro-260628、2-0-lite-260428、character-260628 明确推荐 `json_schema`，其他型号只写“结构化输出”（Beta）；目录仍未声明 `json_schema`。
- 原生 Qwen 的 `static_inventory_models` 中 `qwen3.8-2.4t-a95b`（low/medium/xhigh）与 `qwen3.5-27b`（none/thinking）没有渠道变体映射，库存会报告 `unavailable_presets`（改动前已存在）。
- Kimi 国际站 USD 价格、MiniMax M3 priority 档、方舟 2.1-lite / 2.0-lite / 2.0-mini 的音频输入价未建模；方舟 `kimi-k3-1-pro-261024` 未映射。
- MiniMax 国内价格的旧条目仍引用 `platform.minimaxi.com`（现跳转到 `platform.minimax.cn`）。
- 回退办法：回滚上述 JSON、`BUILTIN_CATALOG_REVISION_SEQ`（25 → 23；只撤销 §9 时 25 → 24）、`model_defaults.rs` 头部说明及对应测试；`kimi-flagship-highspeed` 为 `direct_only`，删除时无用途或 overlay 引用需要清理。

## 9. 续：Doubao Seed 思考档位（2026-11-17）

§8 中记录的缺口在同日补齐。官方依据于 2026-11-17 重新拉取，与 11-16 一致：[深度思考](https://docs.volcengine.com/docs/ark/deep-thinking?lang=zh)、[Responses API 深度思考](https://docs.volcengine.com/docs/ark/responses-api-deep-thinking?lang=zh)（Markdown 导出 `docs.volcengine.com/docs/zh/ark/<页面>.md`）。

**官方事实**

- `thinking.type` 支持 enabled（默认）/ disabled。
- `reasoning_effort`（Chat）与 `reasoning.effort`（Responses）接受 none、minimal、low、medium、high、xhigh、max 七个值。对 Seed 模型：minimal 关闭思考，none 映射到 minimal，xhigh/max 映射到 high。实际可区分的档位为关闭 / low / medium / high。
- 默认 high：evolving、2-1-pro-260915/260628、2-1-lite-260915、2-1-turbo-260628。
- 默认 medium：2-0-lite/mini-260428 与 -260215、2-0-code-preview-260215、character-260628。
- character-251128 不支持深度思考（模型列表只标文本与函数调用）；translation-250915 不支持思考。
- Agent Plan 文档未单独重述思考参数。它走同一套方舟 API（`/api/plan/v3`、Responses），现有 DeepSeek/GLM/Kimi 预设也用 `reasoning.effort`，按此前做法沿用方舟深度思考文档。

**配置**

| 文件 | 变更 | revision |
| --- | --- | --- |
| `models/doubao.model.json` | 思考型 Seed 模型 `supported_efforts` 改为 none/low/medium/high，`default_effort` 按官方（2.1/evolving high，2.0 与 2.0-code-preview medium）。规格绑定：`doubao-pro` none → high，`doubao-code` none → medium，`doubao-mini`、`doubao-lite` 保持 none。2.0-code-preview 补 `reasoning: true`。`doubao-seed-character`、`doubao-seed-translation` 改为 native | 10 → 11 |
| `providers/doubao.provider.json` | Seed 模型新增 reasoning-low/medium/high（`reasoning.effort` 同名值）；reasoning-none（minimal）补上 2.0-code-preview | 15 → 16 |
| `providers/doubao-agent-plan.provider.json` | 同上，覆盖套餐内 6 个 Seed 模型（及原列表中的 2-0-lite-260215） | 12 → 13 |
| `src/settings/mod.rs` | `BUILTIN_CATALOG_REVISION_SEQ` | 24 → 25 |
| `src/service/model_defaults.rs` | 头部说明 Doubao 各规格绑定的预设 | — |
| `provider_protocol_contracts.json` | doubao、doubao-agent-plan 的 Seed 变体规则加入 low/medium/high，变体来源加入 Responses 深度思考页 | — |

**取舍**

- 规格绑定只把 pro/code 改为官方默认思考档。pro 面向复杂 Agent，code 面向编程。mini/lite 服务 swift、translate、summarize 和回退 3.0/4.0 层，保持非思考，不增加这些用途的时延与输出 token。
- 家族直选（如 `llm.doubao-seed-2-1-pro`）按 `default_effort` 走官方默认思考档。
- 显式预设（如 `llm.doubao-seed-2-0-mini:low`）均可用。xhigh/max 在方舟上等同 high，不另建预设。
- character 的 canonical 家族同时承载 251128（无思考）与 260628（默认 medium）。按家族无法给出一致的可调档位，改为 native，即不发送思考参数，由上游按版本默认处理。
- translation 不支持思考，改为 native。
- 按 inventory 规则（缺少渠道映射即报告 `missing_channel_mapping`），原先 character/translation/2.0-code-preview 的 `none` 预设在原生方舟渠道上没有变体，属于不可用预设。本次改为 native 或补齐映射后不再出现。这是从规则推断的，没有用改动前的配置实测。

**验证**

- 新增 `provider::builtin::upgrade_tests::doubao_seed_thinking_presets_follow_official_efforts_on_both_ark_channels`。它在两个渠道分别检查各模型预设集合、无不可用预设、四个变体的上游取值，以及默认档。路由检查覆盖 `llm.doubao-pro` → :high、`llm.doubao-mini` → :none、`llm.doubao-code` → :medium、家族直选与显式 `:low`。
- `cn_vendor_models_reviewed_on_2026_11_16_close_the_discovery_to_route_loop` 中 Doubao 部分同步为新预设。
- `provider_rules_are_loaded_without_rust_generated_revisions` 更新为 doubao 16、agent plan 13。
- `cargo test -p aicc --lib -- --test-threads=1`：791 通过、11 失败（与基线相同的 `service::cloud_update` / `storage` 沙箱失败）。单模型覆盖测试通过。
- `pnpm run acceptance:preflight` 通过，`acceptance:self-test` 162/162（同样临时转 LF 后从备份恢复）。
- 未执行真实方舟 / Agent Plan 调用；Agent Plan 对 low/medium/high 的实际接受情况未经实测。

