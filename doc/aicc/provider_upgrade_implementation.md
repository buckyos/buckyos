# AICC Provider upgrade 实现记录

日期：2026-09-25。对应 [任务单](../../notepads/aicc-provider-upgrade-todo.md)，基于 Model Driver v2 内核继续完成 Provider 身份、库存、预设和计费链路。Beta 2.2 breaking change，不迁移旧字段或旧 inventory 缓存。

## 已实现的边界

- `catalog/mod.rs`：精确身份索引、受限最长包含、歧义候选、归一化查询辅助函数；`resolve_model` 仅取精确模型事实。删除 driver 白名单、origin DSL、猜测默认模型和通配模型成员关系。有限 pattern 编译成精确条目，显式 models 优先。
- `provider/inventory.rs`：instance override → Provider matcher → 完全匹配 → 受限包含；失败只隔离一个模型。排除不记 warning。库存保留 identity_source、inventory_source，输出 unmatched 和缺少渠道预设映射的诊断；运行时按原因变化去重 warning。
- 静态库存只来自明确声明/实例 snapshot。动态查询成功只补充 `supplemental_inventory_api_types` 中的 API；GLM 使用当前有效 catalog，不能把动态下线的 LLM 补回。OpenAI/Kimi/兼容渠道共享 models HTTP/JSON/ID/ETag 处理；认证和格式错误不能被静态 fallback 吞掉。显式 fallback 为 Degraded。
- `catalog::executable_variants` 由 inventory 和 call 共用；集合为模型 supported_efforts ∩ 渠道映射 ∩ 已知渠道限制。native 使用 base，其他使用 reasoning-{effort}。Registry 拒绝非法 exact/缓存预设。lowering 在请求/canonical/request_rules 后锁定思考参数，保留无关 generation_config 参数。
- 默认树保留 95 条 LLM 功能引用、49 个规格，恢复 55 条图片/视频家族引用及权重。图片/视频父任务 Manual，explicit family 路径接纳 metadata exact；自动 exact 不能绕过家族偏好。音频/视觉/agent_runtime 的明确 LLM 规格引用权重为 1.0，仍必须满足请求 API。

包含匹配采用任务单建议：比较时忽略大小写；前一字符是起点或非字母数字；后缀只允许空、-YYYY-MM-DD、-YYYYMMDD、-YYMMDD、-MMDD 形状。点号与连字符不全局等价，特殊归一化必须由局部 Provider matcher 或实例 override 完成。

## 计费契约

`AiUsage.input_tokens` 含缓存读写；`output_tokens` 含 reasoning，其他计数是子集。Claude 把原始 input、cache_read、cache_creation 相加；Gemini Interactions 把 total_thought_tokens 加入输出，使用 total_cached_tokens；OpenAI Chat/Responses 保持包含缓存/思考的总数。新增 audio/image 输入输出 token 和 cache_write_1h_input_tokens（cache_write_input_tokens 的子集）。缓存与多模态重叠无法拆分时返回 unknown，避免重复计费。

Pricing、tier、time-window 支持上述独立费率。非零维度缺费率、计数矛盾、缺关键 usage 或超出最后阶梯均返回 None；显式 0 才表示免费。自报 reported_cost 只有在 Provider Rules 声明 `reported_cost: {currency, semantics: "total_request_cost"}` 时才优先采用。声明及锁定价格随 PinnedProviderTask 持久化。

路由和结算复用 PinnedPricingSnapshot 的时段/阶梯解析。LLM 默认参考 1,000 输入 + 1,000 输出，请求已有估量时覆盖对应维度。同家族、同固定 effort 内，已知 USD 可比价优先，再比较延迟、可靠性；未知价排最后，硬成本预算拒绝未知。规格偏好与版本顺序仍优先于实例价格。

汇率复用 Known Provider catalog 的可选 exchange_rates：source_url、observed_at_ms、expires_at_ms、usd_per_unit。只在 [observed_at_ms, expires_at_ms) 生效；无有效汇率视为不可比价，不按汇率 1 计算，也不影响其它 USD 候选。没有内置猜测汇率，部署方通过现有 cloud/local/system 配置下发。

价格继续作为 Provider Rules 的独立 model_pricing 表发布，复用既有 cloud 文档版本和回滚，不增加 catalog kind 或第二套配置 DSL。可以单独更新 Provider 文档而不修改 Model Driver；整份 Provider 文档的原子替换仍是发布边界。

## 核验的数据与来源

以下来源核对于 2026-09-25；静态价格是普通按量渠道价，按 token 原始单位存储。未确认/不能准确表达的条件不填价。

| Provider | 本次写入/修订 | 官方来源 |
| --- | --- | --- |
| OpenAI | gpt-6-astra 标准价、272K 输入阈值长上下文价；low/medium/high/xhigh/max | [模型页](https://developers.openai.com/api/docs/models/gpt-6-astra) |
| Claude | Fable 5.1/5、Opus 5、Sonnet 5、Haiku 4.5；缓存读取、5m/1h 写入 | [价格](https://platform.claude.com/docs/en/about-claude/pricing)、[thinking](https://platform.claude.com/docs/en/build-with-claude/extended-thinking) |
| MiniMax | M2.7/2.5/2.1/M2、highspeed、已确认 TTS 和 image-01；只用于 global 区域 | [按量价格](https://platform.minimax.io/docs/guides/pricing-paygo) |
| Gemini | 3.1/3.5 Flash-Lite 文本、音频、图像输入价；3.1 Flash-Lite 四档，medium 正确映射 medium | [价格](https://ai.google.dev/gemini-api/docs/pricing)、[thinking](https://ai.google.dev/gemini-api/docs/thinking)、[usage](https://ai.google.dev/api/interactions-api) |
| DeepSeek | 固定 V4 Pro 的工作日峰谷价；官方 Flash 旧调用名优先解析重定向 | [价格](https://api-docs.deepseek.com/quick_start/pricing/)、[更新](https://api-docs.deepseek.com/updates/) |
| GLM | 核对既有价格并补来源，只用于 china 区域；ASR 改为官方 token 费率 | [价格](https://docs.bigmodel.cn/cn/guide/start/pricing)、[thinking](https://docs.bigmodel.cn/cn/guide/capabilities/thinking) |
| Qwen/Kimi | 按 supported_efforts 精确映射；Qwen enable_thinking，Kimi thinking.type | [Qwen Responses](https://docs.qwencloud.com/api-reference/chat/openai-responses)、[Kimi](https://platform.kimi.com/docs/api/chat) |
| OpenRouter/FAL | OpenRouter 保留动态 token 价；FAL discovery 查询带账户认证的单位报价 | [OpenRouter models](https://openrouter.ai/api/v1/models)、[FAL pricing](https://fal.ai/docs/platform-apis/v1/models/pricing) |

Fable 5.1 的缓存读取价 0.25 USD/百万 token 与 Fable 5 的 1 USD/百万不同，官方当前表证实该差异，未强行拉齐。GLM-4.7、GLM-4.5-Air 的输入/输出双维分档当前单维 tiers 无法准确表达，撤下旧的不准确报价；当前官方表没有确认的其它旧价格也撤下。

FAL 的单位报价可能按尺寸/长度成比例调整，当前 queue codec 没有精确账单单位。因此动态结果只存参考 estimated_cost；不根据图片数或秒数猜总费用。未知单位也不强行换算。静态价格、动态价格共享包含 tiers/time_windows 和比例 lint 的校验；异常比例必须填写 ratio_exception。

## 明确保留的接入缺口

- Gemini 2.5 的 none/thinking 在当前 Interactions adapter 没有准确的开关映射，返回 unavailable_presets。官方 [GenerateContent 文档](https://ai.google.dev/gemini-api/docs/generate-content/thinking) 的 thinkingBudget=0/-1 属于另一协议，未冒充 Interactions 参数，没有新增另一协议的 Adapter。
- DeepSeek 官方旧 Flash ID 当前重定向到 V4.1；V4.1 Model Driver 尚未接入时标 unresolved_alias。其它 Provider 的固定 V4 Flash 模型仍保留旧家族。实例 override 可明确人工绑定，来源可见。
- 未确认价格的模型、需要额外服务费或无法完整观测计费单位的场景仍不能给出完整实际费用；没有填充推测价格。没有使用真实 API Key 或执行付费推理。

## 管理接口与验证

`list_providers.inventory` 新增 unmatched_models（ID/原因/候选/时间）、unavailable_presets（模型/effort/原因）、unpriced_models。`refresh_provider_models` 增加 unmatched_count。实例 origin override 改为 model_driver_overrides，值为 driver/model_id。Rust 公共 SDK 和 usage 消费者已同步；仓库 tracked TS 源中没有对应独立声明。Cargo 依赖不变。

关键测试位于 provider/builtin/upgrade_tests.rs、catalog/tests.rs、protocol/* 的 billing_usage_tests、execution 的 billing_boundary_tests、routing 和 service tests。闭环使用真实 builtin 三类 metadata，经共享 discovery/mock HTTP、InventoryBuilder、ModelRegistry、Router、CallResolver 到 codec 最终 JSON；没有手工注入预设替代关键验收。

实际完成的验证：

- `cargo test -p aicc --offline`：486 passed，0 failed；包含最后补充的全部 95 条 LLM 引用、49 个规格、55 条媒体引用及权重断言，以及别名更新后的渠道隔离测试。
- `cargo test -p aicc -p llm_context -p buckyos-api --offline`：AICC 486、buckyos-api 217 + 4 集成测试、llm_context 156，共 863 项通过；二进制与 doc-test 均通过。
- `cargo check -p aicc -p llm_context -p buckyos-api --all-targets --offline` 通过。
- 受影响 AICC Rust 文件的 `rustfmt --check`、`git diff --check` 通过。旧身份字段仅在拒绝旧输入的测试中保留字面量，生产代码与 builtin metadata 已删除。
- 共享字段的其它 Rust 构造调用方使用 `Default`，不需要修改。未运行整仓构建、DV 部署或真实厂商付费调用；现有 dead-code warning 不在本次清理范围。

## 代码量与范围

基准为任务开始的 HEAD；任务单原本未跟踪，使用任务开始内容作其差分基准。仅统计 AICC、本次必要修改的公共 API/llm_context、AICC 文档与本任务单，不纳入用户已有的旧 pricing TODO 删除或其它并行文件修改。Rust 按语法树剔除所有 cfg(test) 项及独立测试文件，生产行排除空行与注释；测试单列物理行。JSON 与文档单独统计，不能抵消生产增长。

| 范围（按文件主职责分组） | 新增 | 删除 | 净变化 |
| --- | ---: | ---: | ---: |
| A/C 发现器与身份解析收敛 | 485 | 882 | -397 |
| B 计费、动态价格与实例比价 | 443 | 85 | +358 |
| 新增契约、预设隔离、诊断、默认树及共享类型 | 861 | 282 | +579 |
| **生产合计** | **1789** | **1249** | **+540** |
| 测试 | 1587 | 637 | +950 |
| metadata | 1931 | 269 | +1662 |
| 文档与任务单 | 241 | 239 | +2 |

第一组包括 catalog/mod、error、provider/mod 和除 FAL 外的 builtin 发现器；第二组包括 execution、protocol、routing、service/inference 和 FAL；其它生产修改归第三组。文件内有职责交叉，因此这些是可复算的文件分组，不声称按每一行功能作精确归因。测试辅助包装保持计入测试，没有借删除测试夸大生产收敛。

发现器/身份解析达到净减少，但全任务生产代码净减少的设计目标未达到：新增计费维度、汇率/价格校验、预设隔离、诊断与默认树契约的实现增加了代码。旧 origin DSL、猜测 fallback、driver 白名单和重复发现流程已实际删除，没有保留兼容双轨。metadata 增长包含精确预设表、来源与地区价格条件及 JSON 展开，不计入生产代码。

