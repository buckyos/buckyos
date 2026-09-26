# AICC Provider 价格事实源与落库策略

价格 metadata 只保存能由当前 schema 精确表达、且可用于实际结算的价格。价格受地区、
账号套餐、模态或动态 endpoint 影响时保持 unknown；unknown 可以参与保守路由策略和验收
报告中的估算敞口，但不能生成 `finance_complete=true` 的实际账单。

Model Driver 不保存 `model_pricing`，也不提供原厂默认价或成本估值兜底。原厂直连 Provider
原来位于 builtin Model Driver 的默认价已迁入对应 Provider Rules，并按当前官方渠道重新核验；
第三方渠道没有因此继承原厂直连价格。价格只能来自当前渠道的 discovery、响应，或已确认适用的
Provider Rules；缺失时宁可 unknown，也不使用未经确认的价格。

## 当前 schema 可表达的计价口径

静态价格只声明在 Provider Rules 的 `model_pricing` 表（见 `provider_profile_schema.md` 5.5）。
下列口径可以表达，但能表达不等于价格已核实；只有确认渠道、币种和计费条件后才能填写：

| 口径 | 表达方式 |
|---|---|
| 按 token | `input_token` / `output_token` / `cache_input_token`，每 token 单价 |
| 按次、按张、按音视频秒、按字符 | `unit` + `amount` |
| 按算力秒、按百万像素 | `unit: "second"` / `"megapixel"` + `amount` |
| 按输入长度等用量分档 | `tiers`（`volume` 整单取档 / `graduated` 逐档累进） |
| 按峰谷时段 | `time_windows`（请求时刻钉住） |
| 按请求参数（质量、尺寸等） | `rules`（仅 Provider Rules 侧） |

仍按 unknown 处理的情形：

1. 与地区、账号套餐或 credits/units 折算绑定的价格；
2. 需要同时按多个维度分档、单一 `tiers` 表达不了的价格；
3. `second` / `megapixel` 计价：口径可以声明，但当前没有 adapter 上报对应 usage 计数器，
   `completion_cost` 返回 unknown，不会算成 0；
4. 按次计费的 `tiers`：档位只在 token 计量下参与选档。

## Provider 价格核验入口

以下链接是 builtin Provider 静态价格的核验入口。每条静态 `model_pricing` 都同时保存
`source_url` 和 `verified_at`；维护价格时必须同步核对币种、区域、模态、阶梯和请求参数条件。
即使官方模型 ID 相同，也不能将官网价格直接用作第三方渠道价格。OpenRouter 等动态价格渠道
仍以 discovery/响应为准，不伪造静态来源。

| Provider | 核验入口 |
|---|---|
| OpenAI | [API pricing](https://developers.openai.com/api/docs/pricing) |
| Claude | [Models and pricing](https://platform.claude.com/docs/en/about-claude/models/overview) |
| Gemini | [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing) |
| Fal | 各模型页（例如 [ESRGAN](https://fal.ai/models/fal-ai/esrgan)） |
| OpenRouter | [Usage accounting](https://openrouter.ai/docs/cookbook/administration/usage-accounting) |
| MiniMax | [国内按量价格](https://platform.minimaxi.com/docs/guides/pricing-paygo)、[国际按量价格](https://platform.minimax.io/docs/guides/pricing-paygo) |
| Kimi | [Chat pricing](https://platform.kimi.com/docs/pricing/chat) |
| GLM | [Official pricing](https://docs.bigmodel.cn/cn/guide/start/pricing) |
| DeepSeek | [Models and pricing](https://api-docs.deepseek.com/quick_start/pricing/) |
| Doubao | [Model pricing](https://ark.volcengine.com/docs/82379/1099320) |
| Qwen | [Model Studio pricing](https://help.aliyun.com/zh/model-studio/model-pricing) |
| SN | Provider inventory/usage response |

## 结算规则

1. Provider 响应中的带币种实际费用优先。
2. 没有实际费用时，只有完整匹配本次请求计费条件的 pinned pricing 才可计算费用。该价格只能
   来自 Provider discovery 或适用的 Provider Rules，不能来自 Model Driver；分时价在请求
   时刻钉住，分档价在结算时按真实用量选档。
3. 不同币种分别聚合，不换算、不直接比较。
4. 缺失或无法准确表达的价格保持 unknown，不按零处理；免费模型显式写 0，两者不可混淆。
5. 官方价格变更时必须更新事实源检查日期、metadata golden 和相应协议/计费测试。

本轮补齐与复核日期：2026-09-26。具体条目的核验日期以其 `verified_at` 为准。
