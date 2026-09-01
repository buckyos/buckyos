# AICC Provider 协议覆盖组件

定义第一版 11 个内置 Provider 以及 SN 扩展 Provider 的协议覆盖要求和 L4 五维矩阵。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. Provider 协议覆盖

| Provider | 输入格式 | 输出格式 | Streaming / 异步 | Mock 重点 |
|---|---|---|---|---|
| OpenAI 官方 | `openai-responses`；其它资源 API 按 operation | Responses item、tool calls、JSON schema、artifact、usage | Responses SSE delta 归并 | 新接口 contract、tool、vision、rate limit、context too long |
| Claude 官方 | `claude-messages` | content block、tool_use、stop_reason、usage | Messages SSE event stream | Messages contract、tool schema、vision、overloaded/rate limit |
| Google Gemini 官方 | `gemini-interactions`；其它媒体/embedding API 按 operation | interaction outputs、function call、safety、media outputs | Interactions stream / 长任务 operation | 新接口 contract、safety、multimodal、video operation |
| 首版历史接口 | `openai-chat-completions` | Chat completion、tool calls、usage | Chat Completions SSE | 由 OpenRouter/Kimi/GLM 的真实需求触发，只维护一份基础 Adapter；其它历史代际仍按需加入 |
| fal | 图片/音频/视频工具型任务 | artifact URL / operation status | 异步 submit + poll | upscale、bg_remove、audio.enhance、video.upscale、operation timeout |
| OpenRouter / Kimi / GLM | 共享 `openai-chat-completions` + 各自 dialect | 归一化 message/tool/reasoning/usage | 共享基础 SSE，各自验证扩展 event | 基础合同只维护一次；分别验证路由参数、partial/cache、thinking/tool_stream |
| MiniMax | `claude-messages` + MiniMax dialect；媒体走原生 operation | Messages content block / media artifact | Messages SSE / 原生异步任务 | 复用 Claude 基础合同，只增加 `base_resp` 和兼容差异 |
| DeepSeek / 豆包 / Qwen | 共享 `openai-responses` + 各自 dialect | Responses item/tool/reasoning/usage | 共享基础 SSE，各自验证扩展 event | 分别验证 thinking、方舟工具、Qwen 参数子集/session cache |
| SN AI Provider | 统一 Provider Instance，`sn-openai` 派生 Adapter | OpenAI 基础协议语义 | 复用 OpenAI stream/响应处理 | API Key 与动态登录双模式、token 刷新、SN 错误隔离、usage / trace / free credit 归因 |

第一版内置 Provider 集合按 `aicc_provider_plan.md`：

- `openai`、`claude`、`gemini`、`fal`
- `openrouter`、`minimax`、`kimi`、`glm`
- `deepseek`、`doubao`、`qwen`

Mock、基础协议复用、派生 Adapter 和 builtin 装配测试必须覆盖全部首版 Provider。缺少某家 key 时只允许跳过该 Provider 的 live smoke test，不能跳过离线协议和装配测试。

### 1.1 L4 真实 Provider、逻辑目录与物理模型矩阵

L4 不再按“每 Provider 一条用例”或“Provider × model 一条用例”收敛，而是由 runner 在测试开始时读取当前临时 group 中的 `models.list` / Provider inventory / 逻辑目录配置，生成完整覆盖矩阵：

```text
case_set = {
  api_type in canonical ApiType
} x {
  method in methods_supporting(api_type)
} x {
  logical_path in standard_logical_paths where logical_path.api_type == api_type
} x {
  provider in enabled_official_providers
} x {
  model in provider.supported_models
    where model.api_types contains api_type
      and model is mounted to logical_path or admitted by logical_path min_line
}
```

矩阵来源：

1. `canonical ApiType` 以 `src/frame/aicc/src/model_types.rs` 中的 `ApiType` 序列化值为准。当前 `llm` 是 canonical api_type，`llm.chat` 是 method；`vision.ocr`、`vision.caption`、`vision.detect`、`vision.segment` 是 api_type，但其标准逻辑目录路径在内置树中是 `image.ocr`、`image.caption`、`image.detect`、`image.segment`。
2. `methods_supporting(api_type)` 以本文件 §7 Method 验收清单和 `aicc_api设计.md` 为准。一个 api_type 可以对应多个 method，例如 `llm` 需要覆盖 `route.resolve`、`chat.completions.create`、`helper.llm_chat`、`llm.chat` 中适用的调用形态。
3. `standard_logical_paths` 以当前运行版本加载的 `LocalLogicalTreeConfig.logical_definitions`、`SessionConfig.logical_tree` 全部可寻址节点和 `models.list` 暴露的逻辑目录为准；该配置默认来自 `build_builtin_local_logical_tree_config()`，并可被 system_config 中的官方 routing 配置叠加。runner 必须把最终生效的标准逻辑目录路径写入报告，并标明每个路径的来源、继承到的 api_type、items、fallback 和 admission 结果。
4. `enabled_official_providers` 发布强覆盖默认至少包含 `openai`、`fal`、`google-gemini`、`claude`、`openrouter`、`sn-ai-provider`；如果官方配置或本次发布基线新增 Provider driver，必须自动纳入矩阵或在报告中标记为未覆盖缺口。
5. `supported_models` 以 AICC 实际注册并可被 `models.list` 观察到的模型为准，包含精确模型名、provider instance、`api_types`、`logical_mounts`、capabilities、health 和 pricing 摘要。

矩阵生成规则：

1. runner 必须先生成 `api_type × method × logical_path × provider × model` 的候选矩阵，再按模型实际能力、逻辑目录 `min_line`、`disable_line`、`mount_mode`、health、quota、policy 和 key 可用性决定 `planned` / `skipped` / `not_applicable`。
2. `skipped` 只用于环境缺失或凭据缺失；模型不支持该 api_type、未挂载到该逻辑目录或不满足 `min_line` 时，应记录为 `not_applicable`，不能混入 skipped 通过率。
3. 每个 `planned` 用例必须执行两段验证：逻辑模型段用 `logical_path` 发起路由或 helper/legacy 调用，断言 route trace 中的 `requested_model_type=logical`、`resolved_logical_path`、`selected_exact_model` 和 provider；物理模型段使用同一个 `selected_exact_model` 或矩阵中的 exact model 发起 typed inference / exact model 调用，断言 `requested_model_type=exact`、不发生隐式 fallback、usage 和 trace 正确。
4. typed inference 只允许 exact model；逻辑模型段必须调用 `route.resolve(logical_model)`，再把结果传给 typed method。Helper 的逻辑模型调用作为独立组合链路验收。
5. 同一个 Provider 下同一个物理模型如果支持多个 `api_types`，不得只用一条“代表性 workflow”替代全部 api_type 覆盖；可以把昂贵能力合并到同一 workflow 中执行，但报告必须保留每个 `api_type × method × logical_path × provider × model` 维度的覆盖状态。
6. Provider 已启用但没有任何可用模型时，生成一个 `skipped` 诊断用例，原因记为 `provider_has_no_models`。
7. `sn-ai-provider` 必须按 `auth.mode` 判断前置条件：`api_key` 缺 key 可 skipped，`dynamic_login` 缺登录凭据或链路可达性属于对应模式的环境失败。
8. `openai`、`fal`、`google-gemini`、`claude`、`openrouter` 缺少对应 API key 时，该 Provider 的全部真实模型用例标记为 `skipped`，并在报告中按 Provider 汇总；发布强覆盖模式可在 preflight 直接失败。
9. 每个真实模型用例最多执行 3 次 attempt：首次失败后只重跑同一个 `api_type × method × logical_path × Provider × model` 用例 2 次；任意一次 attempt 成功则该用例最终为 `passed`。
10. attempt 失败原因必须全部保留在报告中，最终成功的用例也要记录之前失败 attempt 的 `failure_class`、错误码和耗时，便于分析不稳定性。
