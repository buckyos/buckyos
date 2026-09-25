# AICC Provider 价格事实源与落库策略

价格 metadata 只保存能由当前 schema 精确表达、且可用于实际结算的价格。价格受地区、
账号套餐、模态或动态 endpoint 影响时保持 unknown；unknown 可以参与保守路由策略和验收
报告中的估算敞口，但不能生成 `finance_complete=true` 的实际账单。

Model Driver 不保存 `model_pricing`，也不提供原厂默认价或成本估值兜底。2026-09-25 已删除
全部 builtin Model Driver 价表，未将旧价格自动迁入 Provider Rules。价格只能来自当前渠道的
discovery、响应，或已确认适用的 Provider Rules；缺失时宁可 unknown，也不使用未经确认的价格。
运行时仍有旧的 Model Driver 价格字段与 fallback，删除它们尚待 Review 后实现。

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

以下链接供维护 Provider 渠道价格时核验，不表示相关价格已经配置或仍然有效。本轮保留
`glm.provider.json` 原有的渠道价格，其余已删除的 Model Driver 价表均未迁移；未重新核验任何
线上报价。即使官方模型 ID 相同，也不能将官网价格直接用作第三方渠道价格。

| Provider | 核验入口 |
|---|---|
| OpenAI | [API pricing](https://openai.com/api/pricing/) |
| Claude | [Models and pricing](https://platform.claude.com/docs/en/about-claude/models/overview) |
| Gemini | [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing) |
| Fal | [Model API pricing](https://fal.ai/docs/documentation/model-apis/pricing) |
| OpenRouter | [Usage accounting](https://openrouter.ai/docs/cookbook/administration/usage-accounting) |
| MiniMax | [Pay-as-you-go pricing](https://platform.minimax.io/docs/guides/pricing-paygo) |
| Kimi | [Moonshot platform](https://platform.moonshot.ai/docs/) |
| GLM | [Official pricing](https://bigmodel.cn/pricing) |
| DeepSeek | [Models and pricing](https://api-docs.deepseek.com/quick_start/pricing) |
| Doubao | [Volcano Ark documentation](https://www.volcengine.com/docs/82379) |
| Qwen | [Model Studio pricing](https://help.aliyun.com/en/model-studio/model-pricing) |
| SN | Provider inventory/usage response |

## 结算规则

1. Provider 响应中的带币种实际费用优先。
2. 没有实际费用时，只有完整匹配本次请求计费条件的 pinned pricing 才可计算费用。该价格只能
   来自 Provider discovery 或适用的 Provider Rules，不能来自 Model Driver；分时价在请求
   时刻钉住，分档价在结算时按真实用量选档。
3. 不同币种分别聚合，不换算、不直接比较。
4. 缺失或无法准确表达的价格保持 unknown，不按零处理；免费模型显式写 0，两者不可混淆。
5. 官方价格变更时必须更新事实源检查日期、metadata golden 和相应协议/计费测试。

核验入口沿用 2026-09-18 的记录；本次价格边界修订日期：2026-09-25，不作为报价核验日期。
