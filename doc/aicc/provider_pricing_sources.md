# AICC Provider 价格事实源与落库策略

价格 metadata 只保存能由当前 schema 精确表达、且可用于实际结算的价格。价格受地区、
账号套餐、输入长度、模态、时间段或动态 endpoint 影响时保持 unknown；unknown 可以参与
保守路由策略和验收报告中的估算敞口，但不能生成 `finance_complete=true` 的实际账单。

| Provider | 官方事实源 | 当前策略 |
|---|---|---|
| OpenAI | [API pricing](https://openai.com/api/pricing/) | 可精确表达的模型静态 USD 单价；响应实际金额优先 |
| Claude | [Models and pricing](https://platform.claude.com/docs/en/about-claude/models/overview) | 可精确表达的模型静态 USD token/cache 单价 |
| Gemini | [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing) | 同一模型按模态、服务档位、上下文和媒体规格变化，无法精确表达的部分为 unknown |
| Fal | [Model API pricing](https://fal.ai/docs/documentation/model-apis/pricing) | endpoint、账号和输出单位动态变化；无权威响应金额时为 unknown，不把 gallery 价格固化为账单 |
| OpenRouter | [Usage accounting](https://openrouter.ai/docs/cookbook/administration/usage-accounting) | `/models` USD 动态价格用于估算，响应 `usage.cost` 用于实际结算并优先于估算 |
| MiniMax | [Pay-as-you-go pricing](https://platform.minimax.io/docs/guides/pricing-paygo) | schema 可表达的固定价格进入 metadata；套餐、分段或 credits/units 价格为 unknown |
| Kimi | [Moonshot platform](https://platform.moonshot.ai/docs/) | 未取得可与当前模型及账号计费方式唯一对应的官方价格时为 unknown |
| GLM | [Official pricing](https://bigmodel.cn/pricing) | CNY 价格按模型、输入/输出长度和服务方式分档；当前 schema 无法精确表达的价格为 unknown |
| DeepSeek | [Models and pricing](https://api-docs.deepseek.com/quick_start/pricing) | USD 价格存在峰谷时段变化；未实现时间价格规则前为 unknown |
| Doubao | [Volcano Ark documentation](https://www.volcengine.com/docs/82379) | 价格与区域、模型/endpoint 及服务规格绑定；由实例规则或实际账单提供，否则为 unknown |
| Qwen | [Model Studio pricing](https://help.aliyun.com/en/model-studio/model-pricing) | CNY 价格随区域、上下文和推理模式变化；当前 schema 无法精确表达的价格为 unknown |
| SN | Provider inventory/usage response | 以 SN 返回的实际渠道模型和费用为事实源；未返回费用时为 unknown |

## 结算规则

1. Provider 响应中的带币种实际费用优先。
2. 没有实际费用时，只有完整匹配本次请求计费条件的 pinned pricing 才可计算费用。
3. 不同币种分别聚合，不换算、不直接比较。
4. 缺失或无法准确表达的价格保持 unknown，不按零处理。
5. 官方价格变更时必须更新事实源检查日期、metadata golden 和相应协议/计费测试。
