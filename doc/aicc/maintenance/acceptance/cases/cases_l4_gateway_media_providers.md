# AICC L4 Gateway 媒体 Provider 用例

定义 fal 和媒体能力 Provider 的 image/audio/video 真实 gateway workflow、artifact、task、usage、trace 和 partial 判定。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. Method 验收清单

本节同时覆盖标准 AI 推理 method、分层 API method 和控制/管理 method。`method` 是 kRPC schema discriminator；`api_type` 只用于 `route.resolve`、Provider inventory 和逻辑目录过滤。

当前实现里的 canonical `ApiType` 序列化值以代码枚举为准：LLM 为 `llm`，不是 `llm.chat`；chat 的真实调用 method 仍是 `llm.chat`。验收用例必须同时验证：

- `route.resolve(api_type="llm")` 可路由到支持 chat 的模型。
- `route.resolve(api_type="llm.chat")` 的行为必须与当前协议约定一致：若实现尚未接受该别名，应返回稳定、可判断的错误，并在报告中标注为命名兼容缺口。
- `embedding.multilingual`、`embedding.code` 当前不是正式 `ApiType` 枚举项；如文档或 inventory 中出现，应作为 capability / logical mount / metadata 标签处理，不能当作已支持的标准 `api_type` 误判为缺测。

### 1.1 LLM

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `route.resolve` | `api_type`、逻辑模型名 `logical_model`、requirements、disable、policy | 完整渠道/模型身份、operation、fallback、capabilities、trace | 传入 exact model 被拒、无匹配、多 Driver 冲突 |
| `chat.completions.create` | `exact_model`、content-block `messages`、tools、response_format | `message: AiMessage`、`tool_calls`、`finish_reason`、usage、route trace | 传入逻辑模型名被拒、primary quota exhausted 不 fallback、无注册 operation |
| `helper.llm_chat` | 逻辑模型名 + messages | 等价于 `route.resolve` + `chat.completions.create` | 与两阶段行为一致性 |
| `llm.chat`（legacy） | content-block messages、image/document/tool_use block、tools、response_format JSON schema、generation params | `text`/`message`、`tool_calls`、`finish_reason`、usage、route trace | tool schema 非法、JSON schema 不满足、context too long、feature unsupported |

### 1.2 Embedding / Rerank

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `embedding.text` | text items、resource item、chunking、dimensions、normalize、`prefer_artifact` | 小批量 inline embedding、大批量 `data_resource` artifact、embedding meta | embedding space 不匹配、dimensions 不支持、resource invalid |
| `embedding.multimodal` | text + image pair、dimensions、normalize | 与 `embedding.text` 同结构，记录 `embedding_space_id` | fallback 到不同 embedding space 被拒绝 |
| `rerank` | query、text document、resource document、`n`、`return_documents` | `results[index,id,score]` | 文档为空、n 越界、不同 reranker 分数混排禁止 |

### 1.3 Image / Vision

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `images.generate`（typed inference） | `exact_model`、prompt、negative_prompt、size、quality、seed、output | image artifacts，FileObject meta 写 media type / size | 传入逻辑模型名被拒、primary 不 fallback |
| `helper.text_to_image` | 逻辑模型名 + prompt | 等价于 `route.resolve` + `images.generate` | 与两阶段行为一致性 |
| `image.txt2img`（legacy） | prompt、negative_prompt、n、aspect_ratio、quality、seed、output | image artifacts，FileObject meta 写 media type / size | output media type 不支持、预算超限 |
| `image.img2img` | source image、prompt、strength、output | image artifacts | source image invalid、strength 越界 |
| `image.inpaint` | image、mask、prompt、mask_semantics | image artifacts | mask 缺失、mask semantics 不兼容 |
| `image.upscale` | image、scale、target size、preserve_faces | image artifact | 目标分辨率不满足、fallback 不能满足硬约束 |
| `image.bg_remove` | image、mode、output | rgba image artifact | 输入非图片、输出 alpha 缺失 |
| `vision.ocr` | document、level、language_hints、return_layout、return_artifacts | text、`extra.ocr`、OCR artifacts | 不支持语言、文档 meta 缺失 |
| `vision.caption` | image、style、language、n | captions、summary text | n 越界、图片无效 |
| `vision.detect` | image、classes、score_threshold、bbox_spec | detections with bbox | bbox unit 不支持 |
| `vision.segment` | image、prompt、mask_format、return_bitmap_mask | masks、bitmap artifact | mask_format 不支持 |

### 1.4 Audio / Video

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `audio.tts` | text、voice contract、speed、output | audio artifact | voice_id 不可 fallback、sample_rate 不支持 |
| `audio.asr` | audio、language、timestamps、diarization、output_formats | transcript、segments、vtt/srt/json artifacts | output format 不支持、音频 meta 缺失 |
| `audio.music` | prompt、duration、instrumental、lyrics、seed、output | async task、audio artifact、structure | duration 越界、异步任务失败 |
| `audio.enhance` | audio、task、strength、return_stems | enhanced audio artifact、stems | task 不支持 |
| `video.txt2video` | prompt、duration、aspect_ratio、resolution、generate_audio、seed | async task、video artifact | operation timeout、Provider started 后不跨 Provider 重试 |
| `video.img2video` | image、prompt、duration、resolution | async task、video artifact | image invalid |
| `video.video2video` | video、prompt、preserve_motion、time_range | async task、video artifact | time_range 越界 |
| `video.extend` | video、prompt、continuation_handle、duration | async task、video artifact | continuation_handle 缺失或不匹配 |
| `video.upscale` | video、target_resolution、denoise、sharpen、output | async task、video artifact | target_resolution 不支持 |

### 1.5 Agent Runtime Support

`agent.computer_use` 当前作为占位方向，不作为普通 AICC v0 模型调用的强制真实 Provider 验收项。Mock 阶段只验证 schema、路由目录和安全约束：

- screenshot resource。
- viewport。
- allowed actions。
- action array response。
- 不允许无 sandbox / environment 上下文的真实执行。

### 1.6 Control / Management

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `cancel` | `task_id`、tenant/session 上下文 | accepted / rejected、原 task 状态可观察、task data / event 记录 cancel 语义 | unknown task、跨 tenant cancel、provider 不支持取消、已完成任务重复取消 |
| `service.reload_settings` | 空 params | reload 结果、Provider registry / ModelRegistry 重建摘要 | settings 非法、凭据缺失、保留上一版可用配置 |
| `models.list` | 空 params、可选诊断过滤参数 | Provider inventory、完整模型/渠道身份、operations、逻辑目录、health 摘要 | registry 为空、敏感字段泄露、损坏 catalog 不应导致服务不可诊断 |
| `usage.query` | 时间窗口、provider/model/method/api_type 过滤 | 聚合 usage、明细数量、成本/usage 字段、空结果 | 非法时间窗口、无权限、重复幂等记录不应重复计费 |
| `quota.query` | capability / method、tenant/session 上下文 | 剩余额度、预算状态、限制来源 | 未配置 quota、跨 tenant 查询、非法 method |
| `provider.list` | 可选 provider/type/driver 过滤 | Provider 列表、inventory 摘要、health、capability、pricing 脱敏视图 | 无权限、凭据泄露、Provider 状态异常仍可诊断 |
| `provider.health` | provider instance / driver | health 状态、最近错误摘要、latency / quota / availability | Provider 不存在、health 过期、敏感错误未脱敏 |
| `provider.validate` | Provider Instance 草案、endpoint、Profile、Adapter、auth | schema 校验结果、可连接性 / mock 可达性、脱敏诊断 | 凭据缺失、endpoint 非法、未知 Profile/Adapter、不得写入 system_config |
| `provider.add` | provider settings、tenant/session 上下文 | system_config 写入、reload 后 `models.list` 可见、审计记录 | 重名冲突、无权限、schema 非法、写入失败回滚 |
| `provider.delete` | provider instance name、tenant/session 上下文 | system_config 删除、reload 后候选消失、相关 routing 诊断 | 删除不存在、仍被 policy 锁定引用、无权限 |
| `provider.refresh_models` | provider instance / driver、刷新策略 | inventory 更新、metadata resolver 生效、`models.list` 反映新 revision | Provider 不可达、返回损坏 metadata、刷新失败不破坏旧 inventory |

## 2. Provider 协议覆盖

| Provider | 输入格式 | 输出格式 | Streaming / 异步 | Mock 重点 |
|---|---|---|---|---|
| OpenAI | Responses API、image generation/edit、audio transcription/speech、embedding | text、tool calls、JSON schema、image/audio artifact、usage | SSE delta 归并；图片/音频直接 artifact | tool call、JSON schema、vision content part、rate limit、context too long |
| Claude | Messages API、content blocks、tool use、vision block | text block、tool_use、stop_reason、usage | SSE event stream 归并 | content block 转换、tool schema、vision fallback、overloaded/rate limit |
| Google Gemini | `generateContent`、多模态 parts、embedding、image/video/audio | candidates、function_call、safety、media outputs | streamGenerateContent / 长任务 operation | parts 映射、safety block、multimodal embedding space、video operation |
| OpenAI-compatible / OpenRouter | Chat completions 或 responses-like | OpenAI-like，但字段可能缺失或扩展 | SSE 兼容差异 | 兼容字段缺失、模型名映射、provider-specific error |
| fal | 图片/音频/视频工具型任务 | artifact URL / operation status | 异步 submit + poll | upscale、bg_remove、audio.enhance、video.upscale、operation timeout |
| SN AI Provider | 统一 Provider Instance，`sn-openai` 派生 Adapter | OpenAI 基础协议语义 | 复用 OpenAI stream/响应处理 | API Key 与动态登录双模式、token 刷新、SN 错误隔离、usage / trace / free credit 归因 |

P0 Provider 最小集合按 `aicc_provider_plan.md`：

- `openai.rs`
- `claude.rs`
- `gemini.rs` / `google-gemini` driver（代码中保留历史拼写，配置与 metadata 统一按 Google Gemini 语义验收）
- `fal.rs`

OpenRouter 在 Mock 和 Provider adapter 单测中仍可作为 P1 optional provider；在 L4 gateway 发布强覆盖验收中纳入 Provider 覆盖矩阵，用于验证 OpenAI-compatible 长尾模型、成本 fallback 和兼容性。普通开发验收缺少 OpenRouter key 时应 skipped，不阻塞 P0。

### 2.1 L4 真实 Provider、逻辑目录与物理模型矩阵

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

## 3. Gateway 真实模型验收

真实模型成本受控：

- 每个真实 Provider 按 `api_type × method × 标准逻辑目录路径 × Provider × model` 展开用例；每个 planned 用例必须覆盖逻辑模型路径和精确物理模型路径。
- 每条 workflow 可以承载多个矩阵用例以控制成本，但报告必须逐项标记每个矩阵坐标的覆盖结果，不能用“代表性 workflow”替代未执行维度。
- 首次失败后只重跑同一个 `api_type × method × logical_path × Provider × model` 用例，最多累计 3 次 attempt。
- 任意 attempt 成功则该用例成功，所有 attempt 摘要都写入报告。
- 不断言自然语言全文，只断言协议事实。
- 未配置 API key 或 Provider 未启用时用例标记为 `skipped`，不算失败。
- `sn-ai-provider` 支持 `api_key` 和 `dynamic_login`；只按所选认证模式检查对应凭据，不能假定它永远无 API Key。
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
| SN AI Provider | 每个模型分别执行 API Key 和动态登录 workflow，验证 token 刷新、Provider 归因、usage、trace 和 free credit 归因 |

## 4. 发布验收标准

发布前建议满足以下硬指标：

1. P0 Mock 用例 100% 通过。
2. `cargo test -p aicc` 通过。
3. `cargo test -p buckyos-api --test aicc_client_test` 通过。
4. 本地 kRPC Mock 验收能完成 `reload_settings -> models.list -> route -> provider call -> task / usage / trace` 闭环。
5. gateway runner 能读取 TOML 配置并生成 `summary.md` 和 `summary.json`。
6. gateway runner 能通过 `buckyos-devkit` 启动临时 group，并从宿主机经 gateway 完成访问。
7. 已配置真实凭据的 Provider 必须覆盖其全部可用模型；`sn-ai-provider` 必须覆盖 API Key 和动态登录两种模式；缺少所选模式凭据的 Provider 在普通开发验收中标记为 `skipped`，发布强覆盖验收中应 preflight 失败。
8. 报告、trace、task data、日志摘要中不得出现 API key、session token、原始 prompt 全文和原始文件内容。
9. 真实模型调用次数、attempt 次数和成本在报告中可见。
10. 所有 failed / partial 用例都有明确失败原因、错误码或 Provider 摘要。
11. runner 创建的临时 group 已清理，或报告中明确记录保留原因和清理命令。

### 4.1 新模型维护更新验收

当验收目标来自 `maintenance/aicc_maintenance_roles.md` 中的新模型、新 Provider、新逻辑目录挂载、metadata、运营策略或 routing 维护动作时，除满足常规发布标准外，还必须执行本节闭环。

维护更新类型：

| 类型 | 交付物 | 必验内容 |
|---|---|---|
| 已有 Provider 新增协议兼容模型 | 模型事实 metadata、运营策略、必要的 routing_config | `models.list` 出现新 exact model；`api_types`、`capabilities`、上下文长度、`logical_mounts` 正确；成本、健康度、权重和 fallback 策略生效 |
| 新增 OpenAI-compatible Provider Instance | Provider Profile、Protocol Adapter、`endpoint`、授权、discovery 策略 | Provider 启用后 inventory 可见；完整身份链正确；exact model 可调用；逻辑目录可路由；缺 key / 错 key / discovery 不兼容时错误可诊断 |
| 新增非兼容 Provider adapter 或新 API type | 版本包、adapter、schema、metadata 基线、默认路由策略 | 新 adapter 的协议转换、错误映射、streaming / task 语义、usage、fallback 和 helper / typed inference 链路通过相关用例 |
| 仅更新运营策略 | 策略配置、成本 / quota / health / 权重 / 熔断 / 灰度规则 | 不改变模型事实；route trace 显示策略命中；回滚策略后路由恢复；不需要回滚 metadata |
| 随版本内置缓存更新 | 版本包内 builtin metadata / 默认策略 | 新安装或无云端更新环境中仍能识别发布时已知模型，并生成可用默认路由 |
| 云端 metadata 更新 | 配置的 HTTPS/NDN 发布源；客户端内部 activation 位于 `$BUCKYOS_ROOT/data/srv/aicc/driver_metadata/remote_cache/v1/<source-key>/` | 完整验证后原子生效；损坏候选不破坏 LKGS；revision 回退和冲突被拒绝 |
| 人工运行时覆盖 | `$BUCKYOS_ROOT/etc/aicc/driver_metadata/local/<driver>.json` 或 `system-config/<driver>.json` | `reload_settings` 后生效；优先级高于 cloud activation；损坏配置被拒绝且不破坏可用基线 |

统一验收顺序：

1. 准备更新说明，列出 provider、model、api type、逻辑目录、模型事实变更、运营策略变更、routing 变更、是否需要 adapter 发版，以及影响的旧用例族。
2. 新增或更新命名可检索的相关用例，并在 manifest tags 中标明更新类型、provider、model、api type 和逻辑目录。
3. 在测试环境发布云端配置、运行时覆盖文件或版本包，触发 `reload_settings`。
4. 先执行本次新增用例和受影响旧用例，覆盖 inventory、metadata 解析、exact model、logical model、fallback、成本估算、禁用策略和错误返回。
5. 相关用例通过后执行 AICC 全量用例，确认旧 Provider、旧模型和旧路由策略未回归。
6. 发布环境上线后重复相关用例，再执行发布环境全量用例；发布环境的授权、网络、Provider 实际状态和报告摘要必须可诊断。
7. 如本次支持回滚，至少执行一次目标回滚用例：模型事实错误时回滚 metadata / override；路由错误时优先回滚运营策略或 routing_config；回滚后重新 `reload_settings`，确认 `models.list`、route trace 和关键调用恢复预期。

角色边界：

- BuckyOS 项目方更新公共协议、默认模型事实基线、默认运营策略基线、默认逻辑目录和随版本缓存时，必须同时提交或更新对应 L1/L3/L4 用例。
- 商业服务商跟随 BuckyOS 更新或维护自有 Provider 网关、模型事实包、运营策略包和产品默认 routing_config 时，必须保留服务商维度的用例 tags，报告中能按服务商 Provider / model 聚合。
- 产品用户通过 system_config、local metadata override 或 `session_overlay` 做临时接入时，验收只要求配置生效、可回滚和安全边界正确；不要求修改公共基线。
- 模型服务商主动提供 BuckyOS metadata / inventory / cost / quota / health 信息目前按畅想处理；如接入试点，应作为服务商或第三方 Provider 包验收，不作为 P0 默认要求。

## 5. 分层用例清单

本节把前文功能域拆成更接近实现任务的用例清单。用例 ID 可在实现时继续细化，但应保持前缀稳定。

### 5.1 L1 白盒单测

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l1_routing_exact_model_*` | P0 | 精确模型解析、Provider instance 校验、API type 校验、默认不 fallback |
| `l1_routing_logical_tree_*` | P0 | 逻辑目录展开、items target、目录软链接、候选去重 |
| `l1_routing_fallback_*` | P0 | `strict`、`parent`、`target_exact`、`target_logical`、`disabled` |
| `l1_routing_loop_*` | P0 | fallback loop、logical tree loop、最大 fallback depth |
| `l1_scheduler_weight_*` | P0 | item weight、exact model weight、weight 0 硬过滤、同权重 profile 评分 |
| `l1_scheduler_profile_*` | P0 | `cost_first`、`latency_first`、`quality_first`、`balanced`、`local_first`、`strict_local` |
| `l1_request_overlay_*` | P0 | overlay 合并、逻辑目录覆盖、policy locked、互不污染 |
| `l1_provider_protocol_openai_*` | P0 | OpenAI request/response 转换、tool call、JSON schema、SSE 聚合 |
| `l1_provider_protocol_claude_*` | P0 | Claude content block、tool use、vision block、stop reason、usage |
| `l1_provider_protocol_gemini_*` | P0 | Gemini parts、function call、safety block、operation 状态 |
| `l1_provider_protocol_fal_*` | P1 | fal submit/poll、artifact URL、operation timeout |
| `l1_resource_ref_*` | P0 | `url`、`base64`、`named_object`、FileObject meta 推导 |
| `l1_task_lifecycle_*` | P0 | immediate、async running、final succeeded、failed、cancel |
| `l1_usage_log_*` | P0 | 成功写 usage、幂等去重、缺 usage 报错、查询聚合 |
| `l1_method_api_type_canonical_*` | P0 | `method` 与 `api_type` 边界、`llm` vs `llm.chat`、非正式 api_type 拒绝或降级诊断 |
| `l1_control_method_*` | P0 | cancel、reload、models list、usage/quota/provider 查询的 schema 和权限边界 |
| `l1_security_*` | P0 | `local_only`、`proxy_unknown`、locked policy、trace 脱敏 |
| `l1_concurrency_*` | P1 | session patch 并发、幂等并发、异步任务并发完成 |

### 5.2 L2 AiccClient 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l2_client_llm_chat_success` | P0 | AiccClient 构造标准 `llm.chat` 请求并解析成功响应 |
| `l2_client_exact_model_no_fallback` | P0 | 精确模型不可用时透传可判断错误 |
| `l2_client_idempotency_*` | P0 | running / succeeded / failed / conflict 语义 |
| `l2_client_async_task_*` | P0 | running response、event_ref、最终 task 查询 |
| `l2_client_cancel_*` | P0 | cancel 成功、unknown task、forbidden |
| `l2_client_control_method_*` | P0 | reload、models list、usage/quota/provider 查询响应解析和错误映射 |
| `l2_client_resource_ref_*` | P1 | client 侧 `ResourceRef` JSON tag 和反序列化 |
| `l2_client_error_mapping_*` | P0 | kRPC error 与 AICC task failed error 的边界 |

### 5.3 L3 本地 kRPC 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l3_settings_reload_mock_*` | P0 | system_config 写入 Mock settings、reload、models.list |
| `l3_provider_admin_*` | P0 | provider.validate/add/delete/refresh_models 的 system_config 写入、reload 和回滚语义 |
| `l3_models_list_*` | P0 | `models.list` inventory、完整身份链、逻辑目录、operations、health 脱敏诊断 |
| `l3_quota_query_*` | P1 | `quota.query` 按 tenant、capability、method 返回预算状态和拒绝路径 |
| `l3_krpc_llm_chat_*` | P0 | 纯文本、多模态 content part、tool call、JSON schema |
| `l3_krpc_resource_*` | P0 | `url`、`base64`、`named_object` 输入和 artifact 输出 |
| `l3_krpc_stream_*` | P0 | Mock streaming chunks、task data progress、final summary |
| `l3_krpc_async_*` | P0 | image/audio/video 类异步 task 状态闭环 |
| `l3_krpc_usage_*` | P0 | usage event 写入和查询 |
| `l3_krpc_failover_*` | P0 | Provider timeout / 5xx / quota exhausted 后 failover |
| `l3_krpc_security_*` | P0 | local_only、跨用户访问拒绝、脱敏扫描 |
| `l3_krpc_removed_api_*` | P1 | 已删除 method、旧字段和别名必须被稳定拒绝 |

### 5.4 L4 Gateway 真实模型验收

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l4_gateway_openai_<model>_complex_workflow` | P2 | OpenAI 每个支持模型的文本、JSON schema、tool call、usage、trace |
| `l4_gateway_claude_<model>_complex_workflow` | P2 | Claude 每个支持模型的多模态或 vision、tool use、usage、trace |
| `l4_gateway_gemini_<model>_complex_workflow` | P2 | Google Gemini 每个支持模型的多模态、safety / function call / operation 语义 |
| `l4_gateway_openrouter_<model>_complex_workflow` | P2 | OpenRouter 每个支持模型的 OpenAI-compatible 协议兼容、usage、trace |
| `l4_gateway_fal_<model>_media_workflow` | P2 | fal 每个支持模型的 image/video/audio 工具型异步任务和 artifact |
| `l4_gateway_sn_ai_provider_<model>_complex_workflow` | P2 | SN AI Provider 每个支持模型的 API Key/动态登录双链路、token 刷新、usage、trace、provider 归因 |
| `l4_gateway_models_list` | P2 | 真实环境 inventory、逻辑目录和 Provider health 可诊断 |

L4 用例 ID 中的 `<model>` 必须使用稳定可读的 slug，由精确模型名归一化得到；报告中必须保留原始精确模型名。

## 6. 真实模型判定规则

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

