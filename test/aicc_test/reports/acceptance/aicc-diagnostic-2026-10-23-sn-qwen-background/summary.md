# SN Qwen Responses 后台能力诊断

日期：2026-10-23。用户明确授权一次最小诊断；Ubuntu，通过 Zone Gateway 和真实 AICC 鉴权提交推理；结果查询为使用同一 SN Key 调用 SN 公开只读接口。此轮为协议诊断，不替代 OCR、图片成功/质量验收，也不改变原 99 单元矩阵的结论。

| 用例 | 请求/期望 | 实际结果 | 结论 |
|---|---|---|---|
| qwen3.7-plus 文本 Responses 后台提交 | `background=true, store=false, reasoning.enabled=false, max_output_tokens=32`；输入要求仅回复 OK | 2156 ms 后返回完整 `OK`，finish_reason=stop，返回 response ID；输入 16、输出 1 token | 未观察到 queued/in_progress；短任务本身不能证明后台标志被忽略 |
| 查询该 response ID | 同一 SN Key 调用 `GET /api/v1/ai/responses/{id}` | 454 ms，HTTP 404；LiteLLM 返回 OpenrouterException，内层 `Not Found`, code=404 | 该已完成响应的 ID 查询失败；不是已观测到 queued/in_progress 的任务 ID，不能据此判定长任务异步支持 |

任务：`t-5442371d985145c59cf0188a73232b3a`。返回 ID 仅保留在本地原始诊断记录，不在此复制长编码串。

`store=false` 是 OpenRouter 官方协议允许的值；官方概览明确拒绝 store=true，官方 SDK 将 store 定义为 false。因此不能将此轮 GET 失败外推为全部 Qwen 上游或所有模型都没有异步能力；本轮只证明这个短请求直接完成，且该完成响应不能查询；长任务是否返回可查询的排队 ID 未验证。

只读线上 model/info 确认 qwen3.7-plus → openrouter/qwen/qwen3.7-plus，未配置 use_chat_completions_api 或 supported_endpoints 覆盖。LiteLLM 1.103.0 官方源码有 OpenRouterResponsesAPIConfig，默认选择原生 Responses，不会因为该模型的 mode=chat 就必然桥接 Chat；本轮按源码提取的选择函数验证两种 provider 都选择原生适配。实际部署 wheel 版本未获得。

资料：

- [LiteLLM 1.103.0 Provider 选择](https://raw.githubusercontent.com/BerriAI/litellm/v1.103.0/litellm/utils.py)
- [OpenRouter 原生 Responses 适配](https://raw.githubusercontent.com/BerriAI/litellm/v1.103.0/litellm/llms/openrouter/responses/transformation.py)
- [OpenRouter Responses 概览](https://openrouter.ai/docs/api/reference/responses/overview)
- [OpenRouter 官方 Responses schema](https://raw.githubusercontent.com/OpenRouterTeam/typescript-sdk/main/src/models/responsesrequest.ts)

真实推理 1 次，客户端零重试；追加只读结果查询 1 次。未生成媒体、未发送外部消息。临时实例已删除；settings 与 metadata 恢复，激活 SN 保留 device_jwt 动态登录，网关未修改。准备阶段的临时配置校验失败未发出推理。

## 结论适用范围修正

本轮输入过短，未观察到 queued/in_progress，不能用这次完成响应的 GET 404 推断 OpenRouter 不支持后台长任务。最终诊断状态为“未判定”。后续有区分力的验证应使用原失败的 Qwen OCR 负载或能持续足够时间的输入，记录初始 HTTP 返回的 status、background 与 ID；只有实际出现 queued/in_progress 时再验证查询，避免把普通完成响应 ID 等同于后台任务句柄。本轮一次推理授权已用完，没有追加真实调用。
