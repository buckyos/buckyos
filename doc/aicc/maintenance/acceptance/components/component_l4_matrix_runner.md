# AICC L4 矩阵 Runner 组件

定义 L4 api_type × method × logical_path × Provider × model 五维矩阵、attempt、not_applicable/skipped/partial 和真实模型判定。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. Provider 协议覆盖

| Provider | 输入格式 | 输出格式 | Streaming / 异步 | Mock 重点 |
|---|---|---|---|---|
| OpenAI | Responses API、image generation/edit、audio transcription/speech、embedding | text、tool calls、JSON schema、image/audio artifact、usage | SSE delta 归并；图片/音频直接 artifact | tool call、JSON schema、vision content part、rate limit、context too long |
| Claude | Messages API、content blocks、tool use、vision block | text block、tool_use、stop_reason、usage | SSE event stream 归并 | content block 转换、tool schema、vision fallback、overloaded/rate limit |
| Google Gemini | `generateContent`、多模态 parts、embedding、image/video/audio | candidates、function_call、safety、media outputs | streamGenerateContent / 长任务 operation | parts 映射、safety block、multimodal embedding space、video operation |
| OpenAI-compatible / OpenRouter | Chat completions 或 responses-like | OpenAI-like，但字段可能缺失或扩展 | SSE 兼容差异 | 兼容字段缺失、模型名映射、provider-specific error |
| fal | 图片/音频/视频工具型任务 | artifact URL / operation status | 异步 submit + poll | upscale、bg_remove、audio.enhance、video.upscale、operation timeout |
| SN AI Provider | AICC `settings.sn-ai-provider`，经 SN 转发到兼容模型服务 | OpenAI-like 或 SN 归一响应 | 由 SN AI Provider 能力决定 | 无普通 API key 参数、`runtime_session` / SN 链路可达性、provider instance 命名、usage / trace / free credit 归因 |

P0 Provider 最小集合按 `aicc_provider_plan.md`：

- `openai.rs`
- `claude.rs`
- `gemini.rs` / `google-gemini` driver（代码中保留历史拼写，配置与 metadata 统一按 Google Gemini 语义验收）
- `fal.rs`

OpenRouter 在 Mock 和 Provider adapter 单测中仍可作为 P1 optional provider；在 L4 gateway 发布强覆盖验收中纳入 Provider 覆盖矩阵，用于验证 OpenAI-compatible 长尾模型、成本 fallback 和兼容性。普通开发验收缺少 OpenRouter key 时应 skipped，不阻塞 P0。

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
2. `methods_supporting(api_type)` 以本文件 §7 Method 验收清单和 `aicc_api设计.md` 为准。一个 api_type 可以对应多个 method，例如 `llm` 需要覆盖 `route.resolve`、`chat.completions.create`、`helper.llm_chat`、`llm.chat`、`llm.completion` 中适用的调用形态。
3. `standard_logical_paths` 以当前运行版本加载的 `LocalLogicalTreeConfig.logical_definitions`、`SessionConfig.logical_tree` 全部可寻址节点和 `models.list` 暴露的逻辑目录为准；该配置默认来自 `build_builtin_local_logical_tree_config()`，并可被 system_config 中的官方 routing 配置叠加。runner 必须把最终生效的标准逻辑目录路径写入报告，并标明每个路径的来源、继承到的 api_type、items、fallback 和 admission 结果。
4. `enabled_official_providers` 发布强覆盖默认至少包含 `openai`、`fal`、`google-gemini`、`claude`、`openrouter`、`sn-ai-provider`；如果官方配置或本次发布基线新增 Provider driver，必须自动纳入矩阵或在报告中标记为未覆盖缺口。
5. `supported_models` 以 AICC 实际注册并可被 `models.list` 观察到的模型为准，包含精确模型名、provider instance、`api_types`、`logical_mounts`、capabilities、health 和 pricing 摘要。

矩阵生成规则：

1. runner 必须先生成 `api_type × method × logical_path × provider × model` 的候选矩阵，再按模型实际能力、逻辑目录 `min_line`、`disable_line`、`mount_mode`、health、quota、policy 和 key 可用性决定 `planned` / `skipped` / `not_applicable`。
2. `skipped` 只用于环境缺失或凭据缺失；模型不支持该 api_type、未挂载到该逻辑目录或不满足 `min_line` 时，应记录为 `not_applicable`，不能混入 skipped 通过率。
3. 每个 `planned` 用例必须执行两段验证：逻辑模型段用 `logical_path` 发起路由或 helper/legacy 调用，断言 route trace 中的 `requested_model_type=logical`、`resolved_logical_path`、`selected_exact_model` 和 provider；物理模型段使用同一个 `selected_exact_model` 或矩阵中的 exact model 发起 typed inference / exact model 调用，断言 `requested_model_type=exact`、不发生隐式 fallback、usage 和 trace 正确。
4. 如果某个 method 只允许 exact model，例如 typed inference，逻辑模型段必须拆成 `route.resolve(logical_path)`，再把结果传给该 method；如果某个 legacy/helper method 接受逻辑模型名，则必须直接用逻辑路径调用一次。
5. 同一个 Provider 下同一个物理模型如果支持多个 `api_types`，不得只用一条“代表性 workflow”替代全部 api_type 覆盖；可以把昂贵能力合并到同一 workflow 中执行，但报告必须保留每个 `api_type × method × logical_path × provider × model` 维度的覆盖状态。
6. Provider 已启用但没有任何可用模型时，生成一个 `skipped` 诊断用例，原因记为 `provider_has_no_models`。
7. `sn-ai-provider` 不需要普通 API key；如果临时 group 的 `settings.sn-ai-provider` 没有注册成功，应判为环境或配置失败，而不是 key 缺失。
8. `openai`、`fal`、`google-gemini`、`claude`、`openrouter` 缺少对应 API key 时，该 Provider 的全部真实模型用例标记为 `skipped`，并在报告中按 Provider 汇总；发布强覆盖模式可在 preflight 直接失败。
9. 每个真实模型用例最多执行 3 次 attempt：首次失败后只重跑同一个 `api_type × method × logical_path × Provider × model` 用例 2 次；任意一次 attempt 成功则该用例最终为 `passed`。
10. attempt 失败原因必须全部保留在报告中，最终成功的用例也要记录之前失败 attempt 的 `failure_class`、错误码和耗时，便于分析不稳定性。

## 2. Gateway 真实模型验收

真实模型成本受控：

- 每个真实 Provider 按 `api_type × method × 标准逻辑目录路径 × Provider × model` 展开用例；每个 planned 用例必须覆盖逻辑模型路径和精确物理模型路径。
- 每条 workflow 可以承载多个矩阵用例以控制成本，但报告必须逐项标记每个矩阵坐标的覆盖结果，不能用“代表性 workflow”替代未执行维度。
- 首次失败后只重跑同一个 `api_type × method × logical_path × Provider × model` 用例，最多累计 3 次 attempt。
- 任意 attempt 成功则该用例成功，所有 attempt 摘要都写入报告。
- 不断言自然语言全文，只断言协议事实。
- 未配置 API key 或 Provider 未启用时用例标记为 `skipped`，不算失败。
- `sn-ai-provider` 不需要普通 API key，缺 key 不得作为 skip 原因。
- 真实模型返回可理解错误时，报告记录为 `failed` 或 `partial`，保留错误码、Provider 摘要、trace id。

每条真实模型 workflow 至少断言：

1. 矩阵坐标中的 `api_type`、`method`、`logical_path`、`provider`、`exact_model` 被写入报告。
2. 逻辑模型段 route trace 正确，包含 `requested_model_type=logical`、`resolved_logical_path`、`selected_exact_model` 和 provider。
3. 物理模型段 response schema 正确，并确认 exact model 调用不发生隐式 fallback。
4. task 状态闭环。
5. artifact 可读取。
6. usage 存在。
7. route trace 存在且能关联逻辑段与物理段。
8. 错误被分类。
9. 成本调用次数受控。

建议 workflow：

| Provider | Workflow |
|---|---|
| OpenAI | 每个模型执行 `llm.chat` 多轮 + JSON schema + tool call + image/audio 或 embedding 子步骤 |
| Claude | 每个模型执行多模态 `llm.chat` + tool use + vision caption/OCR fallback |
| Google Gemini | 每个模型执行多模态 `llm.chat` + embedding/multimodal 或 image/video operation |
| fal | 每个模型执行 `image.upscale` / `image.bg_remove` / `audio.enhance` / `video.upscale` 中匹配能力的异步任务 + artifact 读取 |
| OpenRouter | 每个模型执行 `llm.chat` 复杂 JSON 输出 + OpenAI-compatible 兼容字段检查 |
| SN AI Provider | 每个模型执行无普通 API key 的 gateway 转发 workflow，验证 provider 归因、usage、trace 和 free credit 归因 |

## 3. Gateway TOML 配置约定

真实模型验收通过 TOML 配置驱动。建议配置结构：

```toml
gateway_host = "https://example-zone.example"
report_dir = "reports/acceptance"
mode = "gateway"

[environment]
managed_by_devkit = true
group_name = "aicc-acceptance-${run_id}"
group_template = "2zone_sn"
blank_vm_template = "aicc-blank"
cleanup_on_exit = true
keep_on_failure = false

[auth]
token = ""
username = ""
password = ""
login_appid = "buckycli"

[runner]
app_id = "aicc-acceptance"
default_model_alias = "llm.plan"
timeout_ms = 300000
max_attempts_per_case = 3
allow_real_model_calls = false
fail_on_partial = false
matrix_mode = "full_cartesian"
providers = ["openai", "fal", "google-gemini", "claude", "openrouter", "sn-ai-provider"]

[providers.openai]
enabled = true
api_key = ""

[providers.claude]
enabled = true
api_key = ""

[providers.google-gemini]
enabled = true
api_key = ""

[providers.fal]
enabled = true
api_key = ""
image_url = ""
video_url = ""

[providers.openrouter]
enabled = true
api_key = ""

[providers.sn-ai-provider]
enabled = true
api_key = ""
requires_api_key = false
```

配置规则：

- `allow_real_model_calls` 默认为 `false`。只有显式设为 `true` 才允许发起真实模型调用。
- `matrix_mode=full_cartesian` 时，runner 必须按 `api_type × method × 标准逻辑目录路径 × Provider × model` 生成 L4 用例。
- 兼容旧配置 `matrix_mode=provider_model_cartesian` 时，runner 必须在报告中标记为降级模式，并明确列出未覆盖的 `api_type`、`method`、`logical_path` 维度；发布强覆盖不得使用该降级模式。
- `max_attempts_per_case` 默认为 `3`；只有首轮失败的用例才继续执行第 2 / 第 3 次 attempt。
- Provider `enabled=true` 但缺 key 时，用例标记 `skipped`；发布强覆盖模式下，缺 key 可在 preflight 直接失败。
- `google-gemini` 对应 AICC 配置中的 `settings.gemini` / `settings.google_gemini` 兼容入口，生效的 `provider_driver` 应归一为 `google-gemini`。
- `sn-ai-provider` 对应 AICC 配置中的 `settings.sn-ai-provider`，`requires_api_key=false`，缺普通 API key 不应导致 skipped。
- Provider key 不写入报告和日志。
- runner 应把最终生效配置的脱敏摘要写入报告。
- `managed_by_devkit=true` 时，runner 负责创建、启动、探测和清理 group；`keep_on_failure=true` 只用于人工排查，报告必须明确标注遗留环境名。

### 3.1 `buckyos-devkit` 临时 group 生命周期

L4 runner 应把被测环境视为一次性资源，推荐流程：

1. 生成唯一 `run_id` 和 `group_name`，例如 `aicc-acceptance-20260511-153000`。
2. 检查 `buckyos-devkit` / `buckyos-devtest`、Multipass、Python、`uv`、`cargo`、`pnpm` 是否可用。
3. 构造或复用空白 VM 模板；如果本次需要多个虚拟机，先构造一个空白虚拟机，再 clone 出 SN、OOD、普通节点等实例，然后按 group 配置修改 hostname、hosts、端口映射和 app 参数。
4. 使用 group template 生成临时 group 配置，最小建议为 `SN + alice-ood1`；需要多 Provider 节点或 gateway 冗余时再扩展节点。
5. 执行 `create_vms` / `install` / `start`，并等待 gateway、system-config、verify-hub、scheduler、task-manager、AICC 全部可访问。
6. 宿主机 runner 通过 gateway 登录并获取测试 token，后续所有 L4 调用都经 gateway 访问 `/kapi/aicc` 和相关 task / artifact 接口。
7. 写入真实 Provider settings，触发 `reload_settings`，调用 `models.list` 并读取最终生效逻辑目录，生成 `api_type × method × logical_path × Provider × model` 矩阵。
8. 运行 L4 矩阵用例并收集报告。
9. 默认执行 `stop` / `clean_vms` 清理临时 group；除非显式 `keep_on_failure=true`，失败环境也必须清理。

清理约束：

- runner 只能清理自己创建且带有本次 `run_id` 标签或命名前缀的 group / VM。
- 清理前应把必要日志、AICC settings 脱敏摘要、`models.list` 输出和失败 attempt 摘要复制到报告目录。
- 清理失败不能覆盖测试结论，应记录为 `cleanup_failed` warning，并列出残留 group / VM 名称。

## 4. 真实模型判定规则

真实模型输出不可完全确定，验收断言必须避开自然语言全文匹配。

| 类型 | 可稳定断言 | 不应断言 |
|---|---|---|
| `llm.chat` | status、非空 text 或 tool_calls、usage、finish_reason、route trace | 回答全文、具体措辞 |
| JSON schema | JSON 可解析、包含 required 字段、字段类型正确 | 字段内容完全一致 |
| tool call | tool name 在允许集合内、args 可解析、required args 存在 | args 的自然语言细节完全一致 |
| image/audio/video artifact | artifact 存在、media type 正确、可读取、size > 0 | 视觉/听觉内容完全一致 |
| async task | running -> succeeded/failed 有闭环、失败有 error code | Provider 完成时间固定 |
| usage/cost | usage 存在且数值非负、真实调用次数受控 | token 数精确一致 |
| trace | final_model、provider、fallback/failover 标志、trace id 存在 | score 细节完全固定 |

真实模型可接受的 `partial`：

- Provider 成功返回，但内容安全策略导致模型拒答，协议链路和错误分类正确。
- Provider 临时不可用，AICC 返回明确 provider error 或 failover trace。
- artifact 生成成功但模型内容不满足人工预期，协议事实正确。

真实模型不可接受的 `partial`，应判为 `failed`：

- task 卡住且没有超时错误。
- usage 缺失但被当作成功。
- artifact 引用不可读取。
- trace 缺失或泄露敏感信息。
- Provider key、token、原始 prompt 出现在报告中。

真实模型重试判定：

1. 重试粒度是单个 `api_type × method × logical_path × Provider × model` 用例，不得扩大到整个 Provider 或整个测试批次。
2. 第 1 次 attempt 失败后，runner 应立即重跑同一用例；第 2 次仍失败时再重跑第 3 次。
3. 任意 attempt 满足通过断言，则该 case 最终状态为 `passed`，并在报告中标注 `passed_after_attempt=N`。
4. 三次 attempt 全部失败时，该 case 最终状态为 `failed`，主失败原因取最后一次 attempt，同时保留全部 attempt 明细。
5. `skipped` 不重试；preflight / 配置错误不重试；明显安全失败不重试。
6. 对已经返回 `running` 或已提交异步任务的 attempt，不允许在同一个 task 上静默重复提交；重试必须创建新的 case attempt id，并在报告中记录可能产生的真实费用。

