# 添加自定义 Provider：只填 endpoint 和 token，协议客户端自动探测

2026-11-27 立项，未实施。来源：协议客户端重构（`notepads/aicc-provider-protocol-client-todo.md`）实施后发现的问题，owner 当天的判断。

## 1. 目标

owner 原话：

> 常见的 provider 也有可能和官方 provider 一样是“多个协议客户端组合来的”，因此只选一个，其它的就要靠猜。UI 上我计划未来用户选 endpoint + token 就好，协议客户端应该选 auto，通过一个标准的探测流程来获取。

- 用户添加自定义 Provider 时只填 endpoint 和 token。
- 实例用哪些协议客户端、每种 API 类型走哪个客户端，由一个标准的探测流程得出，不让用户选，也不靠猜。
- 本版本先关闭添加入口（见 §4）。

## 2. 现状

- 自定义实例由用户在向导里选一个默认客户端。客户端集合是这个默认客户端，加上同协议族里所有对自定义开放的客户端（`custom_provider_adapters`）。代码要求显式选择，不探测（`service/settings_runtime.rs`）。
- 库存构建时，对每个模型、每种 API 类型解析“走哪个客户端的哪个 operation”（`protocol/binding.rs` 的 `resolve_binding`），结果写进库存，路由只读这个结果。
- 问题：默认客户端没有某种 API 类型的绑定时，要在其余客户端里选。几个客户端同分就报歧义。实测：默认选 `openai-images` 的自定义网关只要列出一个 LLM 模型，`openai-responses`、`openai-chat-completions`、`legacy-openai-chat-completions` 三者并列，整个实例的库存构建失败，连图片模型也用不了。默认选 `openai-audio`、`openai-embeddings`、`openai-videos` 同样如此；选 `openai-responses` 或 `openai-chat-completions` 时不出现。
- 根子在于一个网关实际支持哪几种 API（Responses 还是 Chat，官方口径还是生态口径，有没有图片、音频接口），只选一个默认客户端表达不出来。

## 3. 与已有裁决的关系

- 协议客户端重构第一步的约束是“不新增运行时探测或回退”。本 TODO 的探测只在添加和验证时跑一次，结果写进配置，运行时仍然不探测、不回退。这一点需要 owner 确认。
- 协议客户端重构 TODO §6 的第二步思路是“只填协议、endpoint、token 的自定义 Provider 基本可用”。本 TODO 更进一步，连协议也不填，§6 并入这里。
- 目标第 7 条：只有 API 类型和协议客户端升级需要发版本。探测方法属于客户端的一部分，跟客户端一起发版本；每个客户端声明自己怎么探测，探测流程本身不写厂商分支。
- `doc/aicc/` 现在写的是“自定义 Provider 显式选择客户端，不探测”，这是当前代码的行为。本 TODO 实施时一并改。

## 4. 本版本已做：关闭添加入口

- Desktop AI Center 的“添加 Provider”向导去掉 Custom Provider 卡片（`components/providers/wizard/StepChooseType.tsx`）。
- `buckyos_admin` 手册写明当前不能新建 custom Provider，不要引导用户用 RPC 绕过。
- 没有关：后端 `provider.add` 仍接受 `custom`；已有的 custom 实例照常加载、可查看、可编辑规则。
- e2e：`tests/e2e/pages/ai-center.spec.ts` 里经向导新建 custom Provider 的 4 个用例改成 `test.skip`，首个用例改为断言卡片不存在。入口重开时恢复或重写这 4 个用例（其中 2 个测的是已有 custom 实例的规则编辑，可以改成预置实例）。

## 5. 待 owner 定的问题

1. **探测什么。** 至少要得出：协议族（OpenAI、Claude、Gemini）；凭据放在哪个头（`Authorization: Bearer`、`x-api-key`、`x-goog-api-key`）；base 路径要不要补 `/v1`；族内哪些客户端可用（Responses、Chat、生态口径 Chat，图片、音频、向量、视频接口）。每个模型支持哪些 API 类型，探测到什么粒度？
2. **能花多少钱。** 只用列表接口和不产生费用的请求（例如故意缺参数，看返回 400 还是 404），还是允许每个客户端发一次最小的真实请求（LLM 限 1 个输出 token）？
3. **探测结果不确定时怎么办。** 例如 `/responses` 和 `/chat/completions` 都通、或者都不确定：仍然不猜，标成“需要用户确认”给出选项？还是按固定的偏好顺序选？
4. **结果存在哪里、什么时候重跑。** 存进实例 settings（客户端集合 + 每种 API 类型的绑定，用户可改），还是存进库存？刷新模型时要不要重跑探测？
5. **先支持哪些网关。** LiteLLM（SN 就是）、one-api / new-api、vLLM、Ollama、各家的兼容接口、OpenRouter 风格的聚合网关。
6. **高级选项。** 是否保留手动指定客户端，作为探测失败时的出路？
7. **已有的 custom 实例。** 按 breaking change 不迁移，还是用新的探测流程重建一次？
8. **入口重开之前**，库存构建里的歧义要不要先改成“只去掉这个模型的这个 API 类型，并记下原因”，不让整个实例失败？现在入口关了，只有已有实例会遇到。

## 6. 任务草案（§5 定了再细化）

- 客户端描述里加探测声明，按协议族组织标准探测流程。
- 后端新增探测 RPC，向导的“验证”步骤调用它，结果预填进草稿。
- 向导改成 endpoint + token，展示探测结果（族、客户端集合、每种 API 类型的绑定），不确定项让用户确认。
- 恢复 Custom Provider 入口和 e2e 用例。
- `doc/aicc/` 相关文档、`buckyos_admin` 手册同步。
- 验收：每个支持的网关类型至少一个真实 endpoint 跑通探测，并做一次真实调用。
