# Decision API（2026-09-25）

公共契约：API Type `decision`，Capability `decision`，RPC `decision.evaluate`。
先 `route.resolve(api_type=decision, logical_model=decision, requirements=...)`，
再用返回的 `selected_exact_model` 调用 typed inference。没有 Helper。

请求保留 typed inference 的 `exact_model`、`execution_mode`、`trace_id`、
`idempotency_key`、`task_options`、`session_id`。`state` 为 string/object/array。
`questions` 为非空数组，每题有唯一 `id`、`type` 和结构化 `instructions`：

- `choice`：`options: [{id, description}]`，description 接受 string/object/array/null。
- `score`：`levels: [string|object|array]`，数组位置就是从零开始的等级索引。
- `boolean`：可选 `criteria: {true?: string|object|array, false?: string|object|array}`。

同批问题独立求值、共享状态，不能依赖同批答案。问题及选项 ID 为 1–128 个
ASCII 字母、数字、下划线、点或连字符。公共保护上限：1024 题、1024 选项或
等级、1 MiB 的序列化 state/questions；实际上限还须满足模型与渠道声明。

响应公共字段沿用 typed response，业务字段为 `answers` 数组及可选
`model`（经渠道验证的 origin 版本）。答案有匹配的问题 `id`、`type`、可选 `confidence`：

- `choice`：`selected` 和以选项 ID 为键的完整 `probabilities`。
- `score`：`score`、以等级索引字符串为键的 `probabilities`、完整 `levels`。
  score 是零起点等级索引的概率期望值，不是百分比。
- `boolean`：`probability_true`，不丢弃原概率来生成布尔值。

概率必须有限且在 [0,1]，分布和容差为 1e-4；评分与期望值误差最多
`1e-4 * max(1, 等级数-1)`。不归一化、不补题、不补候选。缺题、额外题、
重复题、类型不符、非法分布或越界评分使调用失败。原始概率、评分、confidence
保持原值。confidence 缺失保持缺失，不承诺跨模型可比较，也不承诺判断正确。

```json
{
  "exact_model": "jev-1.13.0@typesafe-main",
  "state": {"message": "Payment failed twice; please help now"},
  "questions": [
    {"id": "team", "type": "choice", "instructions": "Choose a team",
     "options": [{"id": "billing", "description": "Payments"}, {"id": "support", "description": "Other issues"}]},
    {"id": "urgency", "type": "score", "instructions": {"task": "Rate urgency"},
     "levels": ["Routine", "Urgent", "Critical"]},
    {"id": "retry", "type": "boolean", "instructions": "Has a retry already failed?"}
  ]
}
```

对应成功响应示例：

```json
{
  "task_id": "decision-task-1",
  "status": "succeeded",
  "model": "jev-1.13.0",
  "answers": [
    {"id": "team", "type": "choice", "selected": "billing",
     "probabilities": {"billing": 0.9, "support": 0.1}, "confidence": 0.53},
    {"id": "urgency", "type": "score", "score": 1.2,
     "levels": ["Routine", "Urgent", "Critical"],
     "probabilities": {"0": 0.1, "1": 0.6, "2": 0.3}},
    {"id": "retry", "type": "boolean", "probability_true": 0.8}
  ],
  "usage": {"input_tokens": 318, "output_tokens": 72, "total_tokens": 390}
}
```

示例数值用于说明公共形态，不是线上实测。`routing.preview` 可传同一
`requirements`；WebSDK `decisionRequirements(request)` 与 Rust 使用相同输入推导。

路由 `requirements.decision` 描述 `question_types`、`structured_state`、
`structured_rules`、`question_count`、`max_options`、`max_levels`、`input_bytes`、
`max_state_question_bytes`。Rust `DecisionEvaluateRequest::requirements()` 从实际
请求生成这些要求；exact 调用也重新计算。模型和 codec 必须同时声明
`decision.probabilities` 与各问题类型特性，容量不足或缺失的候选被过滤。
preview、fallback 和 failover 使用同一硬过滤路径。

首版 TypeSafe System One Adapter 只支持 immediate。渠道使用明确静态库存
`jev-1.13.0`；已核验的 `jev-latest`/`jev-preview` 身份映射固定到该版本，
不通配未来模型。别名可通过显式配置库存启用；上游响应版本必须与解析身份一致，漂移时明确失败，
待 metadata 重新核验更新后再开放。未知身份不能成为可执行库存。

TypeSafe `POST /v1/systemone` 使用 Bearer key；questions/answers 是 ID map，
公共 boolean 映射到 noul，options/levels 映射到 criteria。Jev 支持最多 255
Choice 选项、2–10 个 Score 等级，64k 总 token 和 32k state+最长题。
没有官方 tokenizer 时使用明确的保守本地 UTF-8 JSON 字节上限 64000/32000，
不把它当作真实 token usage；实际服务仍负责其 token 限制。最大 1024 题为
AICC 本地保护限额，不是厂商承诺。Jev confidence 来自分布集中程度，
boolean 上游没有 confidence。usage 保留真实 input/output token。

原厂渠道价格 $0.042/百万输入 token、输出费率明确为 0，价格只放 Provider
Rules。渠道缺少价格则 unknown；不按答案大小估算 token。

官方协议核验日期 2026-09-25：
[API](https://docs.typesafe.ai/api)、[Models](https://docs.typesafe.ai/models)、
[结构化规则](https://docs.typesafe.ai/primitives/advanced)、
[Score](https://docs.typesafe.ai/primitives/score)、
[Confidence](https://docs.typesafe.ai/confidence)。fixture 根据这些官方协议制作。

## OpenRouter alpha Decisions

沿用已有 `openrouter-default` 的 OpenRouter key 与 `https://openrouter.ai/api/v1`。
部署包含该 codec 的 AICC 后正常 `provider.refresh_models`，经核验的 Jev 就会进入
`decision` 目录；不需要 `include_alpha`、第二实例或 TypeSafe key。
`~/.buckycli/buckyos_boot.toml` 中的 settings JSON 字符串是启动覆盖，修改它不会
即时应用到运行中实例；运行中设置沿用正常管理接口，本接入无需改 base URL。

| 请求渠道 ID | origin model | 接受的响应 build |
| --- | --- | --- |
| `typesafe/jev-1.13` | `typesafe/jev-1.13.0` | `typesafe/jev-1.13-20260917` |
| `~typesafe/jev-latest` | 同上；目录 alias_target 必须指向前一行 | 同上 |

身份链依据 [OpenRouter 完整目录](https://openrouter.ai/api/v1/models?output_modalities=all)、
[官方教程](https://openrouter.ai/blog/tutorials/how-to-use-jev/) 与
[TypeSafe Models](https://docs.typesafe.ai/models)，于 2026-09-25 核验；不推导其它后缀。
版本、alias target、context 或 modality 漂移需重新核验 metadata，未知响应 build 失败。
原始响应 model/id/provider 保存在任务结果的 `provider_metadata`，公共 `model` 返回
验证后的 `jev-1.13.0`，用于现有执行层身份核对。

operation 为 `decisions.create`，同源 `POST /api/alpha/decisions`，保留代理前缀。
只支持 Immediate；一次混合请求是一次批量上游调用。官方 OpenAPI 要求：若提供
boolean 的 criteria，必须同时有 true 与 false；单侧 criteria 在渠道编码时拒绝。
缺失概率、题目或候选不补造；confidence 与概率分开保留。

渠道 context 为 32000 tokens；本地额外防护为 32000 UTF-8 JSON bytes（总输入及
state+最长题），不是 tokenizer 或实际 usage。255 选项、2–10 等级由官方教程核验；
1024 题和 HTTP body 上限是 AICC 本地防护，不声明为上游保证。
输入价从 OpenRouter discovery 读取，核验值 USD 0.042/百万 token、输出零费率。
`usage.input_tokens/output_tokens` 保留实际数值；可选 `usage.cost` 按 USD
`total_request_cost` 入账并覆盖估算，不重复相加。缺失 cost 为未报告，零 cost 是已报告零；
usage 整体缺失、负数、非有限值或 token 溢出均失败。
