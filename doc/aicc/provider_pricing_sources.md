# AICC Provider 价格事实源与落库策略

价格 metadata 只保存能由当前 schema 精确表达、且可用于实际结算的价格。价格受地区、
账号套餐、模态或动态 endpoint 影响时保持 unknown；unknown 可以参与保守路由策略和验收
报告中的估算敞口，但不能生成 `finance_complete=true` 的实际账单。

## 当前 schema 可表达的计价口径

价格声明在 `model_pricing` 表（见 `provider_profile_schema.md` 5.5），下列口径都已可表达，
不再因为"schema 装不下"而留空：

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

## Provider 事实源

| Provider | 官方事实源 | 当前策略 |
|---|---|---|
| OpenAI | [API pricing](https://openai.com/api/pricing/) | 可精确表达的模型静态 USD token 单价与按张图像价；响应实际金额优先 |
| Claude | [Models and pricing](https://platform.claude.com/docs/en/about-claude/models/overview) | 可精确表达的模型静态 USD token/cache 单价 |
| Gemini | [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing) | token 单价、图像按张价（分档已验证，不再按每百万输出 token 误当每张）与 TTS 按 token 价；同一模型按模态、服务档位和媒体规格变化的其余部分为 unknown |
| Fal | [Model API pricing](https://fal.ai/docs/documentation/model-apis/pricing) | 四个 endpoint 按 compute-second / audio-second / megapixel 计价；动态变化的 endpoint 仍为 unknown |
| OpenRouter | [Usage accounting](https://openrouter.ai/docs/cookbook/administration/usage-accounting) | `/models` USD 动态价格用于估算；响应自动携带 `usage.cost`，无需旧式 `usage.include`。OpenRouter dialect 按官方合同为无币种的数值 cost 补 `USD`，实际费用优先于估算；基础 OpenAI-compatible codec 不猜币种 |
| MiniMax | [Pay-as-you-go pricing](https://platform.minimax.io/docs/guides/pricing-paygo) | token 单价、按字符的语音价、按张图像价与按算力秒视频价进入 metadata（国际站 USD）；国内站 CNY 价格需另行核对 |
| Kimi | [Moonshot platform](https://platform.moonshot.ai/docs/) | 未取得可与当前模型唯一对应的官方价格时为 unknown |
| GLM | [Official pricing](https://bigmodel.cn/pricing) | CNY 价已按官网逐模型录入：token 单价、按次/按秒价与按输入长度分档价 |
| DeepSeek | [Models and pricing](https://api-docs.deepseek.com/quick_start/pricing) | 峰谷时段用 `time_windows` 表达；官方费率冲突未定论时保持 unknown |
| Doubao | [Volcano Ark documentation](https://www.volcengine.com/docs/82379) | 按输入长度分档的 CNY 价已录入；与区域、endpoint 及服务规格绑定的部分仍为 unknown |
| Qwen | [Model Studio pricing](https://help.aliyun.com/en/model-studio/model-pricing) | 按上下文长度分档的 CNY 价已录入；与区域和推理模式绑定的部分仍为 unknown |
| SN | Provider inventory/usage response | 以 SN 返回的实际渠道模型和费用为事实源；未返回费用时为 unknown |

## 结算规则

1. Provider 响应中的带币种实际费用优先。
2. 没有实际费用时，只有完整匹配本次请求计费条件的 pinned pricing 才可计算费用；分时价在请求
   时刻钉住，分档价在结算时按真实用量选档。
3. 不同币种分别聚合，不换算、不直接比较。
4. 缺失或无法准确表达的价格保持 unknown，不按零处理；免费模型显式写 0，两者不可混淆。
5. 官方价格变更时必须更新事实源检查日期、metadata golden 和相应协议/计费测试。

事实源核对日期：2026-09-18。
