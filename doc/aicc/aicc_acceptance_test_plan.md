# AICC 测试验收方案

## 1. 目标

本文基于 `doc/aicc` 目录下的 AICC 设计文档，制定 AICC 模块的分层测试验收方案。目标是在不依赖真实模型、不产生不可控费用的前提下，先用确定性的 Mock 模型覆盖协议、路由、任务、资源、配置、异常和安全语义；再通过 gateway 访问由 `buckyos-devkit` 临时启动的 group 环境，使用真实模型完成端到端验收，并在验收结束后清理该临时环境。

验收方案需要覆盖：

1. AICC 文档中说明的各类能力、控制面和运行时行为。
2. 主流模型接口协议，包括 OpenAI、Claude、Google / Gemini、OpenAI-compatible / OpenRouter、fal 等 Provider 的请求参数、响应格式、错误格式和 streaming 差异。
3. 文本、结构化数据、图片、音频、视频等输入输出格式。
4. 非结构化数据的 `url`、`base64`、`named_object`、artifact 输出策略。
5. AICC 的异步任务与进度观察语义，以及 Provider-native streaming 到 task data / final summary 的适配。
6. 路由、Provider、协议、任务、权限、预算、资源、配置等异常路径。
7. 低成本、确定性 Mock 模型前期测试，以及真实模型的 gateway 验收。
8. gateway 验收必须覆盖 `openai`、`fal`、`google`、`claude`、`openrouter`、`sn` 六类 Provider；其中 `sn` Provider 不需要 API key。
9. gateway 真实模型验收按 Provider 与其支持模型的笛卡尔积生成用例，每个用例执行一条复杂 workflow。

## 2. 设计依据

主要依据：

- `doc/aicc/aicc_api设计.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/aicc 逻辑模型目录.md`
- `doc/aicc/aicc_requirements.md`
- `doc/aicc/aicc_provider_plan.md`
- `doc/aicc/krpc_aicc_calling_guide.md`
- `doc/aicc/update_aicc_settings_via_system_config.md`
- `doc/aicc/aicc_usage_log_db_requirements.md`
- `doc/aicc/how_to_add_provider.md`

关键协议约束：

- kRPC `method` 是 schema discriminator，例如 `llm.chat`、`image.txt2img`、`audio.asr`。
- 正式 request body 放在 `payload.input_json`，`payload.resources` 只用于资源复用或旧调用方兼容。
- `ResourceRef` JSON tag 使用 `url`、`base64`、`named_object`。
- AICC 不暴露独立 streaming 协议；长任务、进度、Provider streaming 中间态统一通过 task-manager event / task data 观察。
- AI method response 只有 `succeeded`、`running`、`failed` 三类状态；失败细节写入 task event / task data。
- 精确模型名格式为 `<provider_model_id>@<provider_instance_name>`。
- 精确模型默认不 fallback，除非显式开启。
- fallback 不得跨 API namespace。
- `local_only`、隐私、预算、能力、上下文长度、Provider health 是硬过滤条件。
- 使用量记录只在 Provider 成功完成且存在 usage 时写入；缺 usage 的成功结果应视为 provider protocol error。

## 3. 测试分层

| 层级 | 入口 | 位置 | 模型 | 执行方式 | 目标 |
|---|---|---|---|---|---|
| L1 白盒单测 | AICC 内部模块 | `src/frame/aicc/tests`；如需测试非 `pub` 程序块，可嵌入对应实现文件 | Rust Mock Provider | `cargo test -p aicc` | 精细覆盖路由、调度、协议转换、任务、usage log、异常分支 |
| L2 AiccClient 黑盒 | `AiccClient` | `src/kernel/buckyos-api/tests/aicc_client_test.rs` | In-process Mock AICC server | `cargo test -p buckyos-api --test aicc_client_test` | 验证 SDK client 的 request/response、错误、任务接口语义 |
| L3 本地 kRPC 黑盒 | `/kapi/aicc` | `test/aicc_test` | TypeScript Mock Provider | 启动本机 BuckyOS + AICC 后运行 TS 用例 | 验证真实服务进程、配置重载、kRPC、task-manager、资源链路 |
| L4 Gateway 验收 | gateway 远程访问 | `test/aicc_test` | 真实模型 | `buckyos-devkit` 临时 group + 自动 runner | 验证真实部署链路；覆盖 Provider × 支持模型的笛卡尔积；每个用例执行一条复杂 workflow |

分层原则：

- L1/L2/L3 使用 Mock 模型，必须确定性执行，适合 CI 和开发阶段反复运行。
- L4 使用真实模型，受网络、模型状态、额度和内容不确定性影响，不要求 100% 通过，但必须报告失败原因。
- Mock 阶段不得访问真实模型，不得依赖外网，不得依赖真实 API key。
- 真实模型验收只验证协议事实、任务状态、artifact、usage、trace 和错误分类，不断言自然语言结果全文。
- L4 的被测 BuckyOS 环境由 runner 通过 `buckyos-devkit` 构造为临时 group；执行脚本的宿主机只作为客户端经 gateway 访问，不在被测 group 内直接调用本地服务。
- L4 runner 必须在结束时清理新构造的 group 环境；清理失败作为 warning 写入报告，不能掩盖原始测试失败。

## 4. 用例优先级

| 优先级 | 含义 | 验收要求 |
|---|---|---|
| P0 | 核心协议、路由、任务、安全和 Mock Provider 行为 | L1/L2/L3 必须 100% 通过，阻塞合入 |
| P1 | 完整功能覆盖，包括所有 method、Provider 协议细节、usage 查询、配置闭环 | 可阶段性 pending，但必须在报告中标注缺口 |
| P2 | 真实模型、性能、兼容性、边界增强和长期稳定性 | gateway / nightly / 手工验收，不阻塞普通开发合入 |

## 5. 功能覆盖矩阵

| 功能域 | 必测点 | 主要层级 |
|---|---|---|
| Method schema | `llm.chat`、`llm.completion`、`embedding.text`、`embedding.multimodal`、`rerank`、`image.*`、`vision.*`、`audio.*`、`video.*`、`agent.computer_use` 占位语义 | L1/L3/L4 |
| Provider inventory | `provider_instance_name`、`provider_type`、`provider_driver`、`exact_model`、`api_types`、`logical_mounts`、capabilities、pricing、health | L1/L3 |
| 路由解析 | 逻辑模型、精确模型、旧 alias 兼容、非法模型名、目录不存在 | L1/L2/L3 |
| Fallback | `strict`、`parent`、`target_exact`、`target_logical`、`disabled`、环路检测、最大深度 | L1/L3 |
| 调度 | `cost_first`、`latency_first`、`quality_first`、`balanced`、`local_first`、`strict_local`、权重优先、同权重 profile 评分 | L1 |
| Session | session 粘性、session config、patch、revision conflict、TTL expired、policy locked | L1/L3 |
| Task | 同步成功、异步 running、失败 task、cancel、无权限查询/取消、重复 idempotency | L1/L2/L3 |
| 资源 | `ResourceRef::Url`、`Base64`、`NamedObject`、FileObject meta、artifact 输出、大批量 embedding artifact | L1/L3/L4 |
| Streaming | Provider-native streaming 转最终 summary；中间态写 task data；AICC response 只返回 `succeeded` 或 `running` | L1/L3/L4 |
| Usage log | 成功调用写一条 durable event；幂等不重复写；缺 usage 视为 provider protocol error；按 1d/7d/provider/model 查询 | L1/L3 |
| 配置 | system_config 写入、全量/局部更新、`reload_settings`、`models.list` 生效验证 | L3/L4 |
| 安全 | `local_only` 硬过滤、`proxy_unknown` 非本地、trace 脱敏、密钥不入日志、跨租户隔离 | L1/L3/L4 |

## 6. 需求追踪矩阵

| 需求来源 | 覆盖用例族 | 层级 |
|---|---|---|
| `aicc_router.md` R-001 精确模型名解析 | `routing_exact_model_*` | L1/L2/L3 |
| R-002 逻辑模型目录树 | `routing_logical_tree_*` | L1/L3 |
| R-003 多 Provider 挂载 | `routing_multi_provider_*` | L1/L3 |
| R-004 Provider 声明式元数据 | `provider_inventory_*` | L1/L3 |
| R-005 候选列表生成 | `routing_candidates_*` | L1 |
| R-006 硬性过滤 | `routing_hard_filter_*` | L1/L3 |
| R-007 fallback 策略 | `routing_fallback_*` | L1/L3 |
| R-008 fallback 环路检测 | `routing_fallback_loop_*` | L1 |
| R-009 权重与 profile 调度 | `scheduler_weight_profile_*` | L1 |
| R-010 session 粘性 | `session_sticky_*` | L1/L3 |
| R-011 运行时 failover | `runtime_failover_*` | L1/L3/L4 |
| R-012 route trace | `trace_route_*` | L1/L3/L4 |
| R-013 配置化策略合并 | `session_config_merge_*` | L1 |
| R-014 精确模型默认不 fallback | `routing_exact_no_fallback_*` | L1/L2/L3 |
| R-015 目录 item 权重 | `scheduler_item_weight_*` | L1 |
| R-016 精确模型权重 | `scheduler_exact_model_weight_*` | L1 |
| R-017 session config 状态 | `session_config_state_*` | L1/L3 |
| R-018 session config 继承与覆盖 | `session_config_inherit_patch_*` | L1 |
| R-019 目录软链接环检测 | `routing_logical_tree_loop_*` | L1 |
| R-020 session config 并发一致性 | `session_config_revision_*` | L1 |
| R-021 用户友好 trace summary | `trace_user_summary_*` | L1/L3/L4 |
| `aicc_api设计.md` ResourceRef | `resource_ref_*` | L1/L3 |
| `aicc_api设计.md` idempotency | `idempotency_*` | L1/L2/L3 |
| `aicc_usage_log_db_requirements.md` usage event | `usage_log_*` | L1/L3 |
| `update_aicc_settings_via_system_config.md` reload | `settings_reload_*` | L3/L4 |

## 7. Method 验收清单

### 7.1 LLM

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `llm.chat` | 纯文本 messages、content part、image/audio resource、tools、response_format JSON schema、generation params | `text`、`tool_calls`、`finish_reason`、usage、route trace | tool schema 非法、JSON schema 不满足、context too long、feature unsupported |
| `llm.completion` | `prompt`、`suffix` | `text`、`finish_reason` | legacy wrapper 到 chat 失败、空 prompt |

### 7.2 Embedding / Rerank

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `embedding.text` | text items、resource item、chunking、dimensions、normalize、`prefer_artifact` | 小批量 inline embedding、大批量 `data_resource` artifact、embedding meta | embedding space 不匹配、dimensions 不支持、resource invalid |
| `embedding.multimodal` | text + image pair、dimensions、normalize | 与 `embedding.text` 同结构，记录 `embedding_space_id` | fallback 到不同 embedding space 被拒绝 |
| `rerank` | query、text document、resource document、`n`、`return_documents` | `results[index,id,score]` | 文档为空、n 越界、不同 reranker 分数混排禁止 |

### 7.3 Image / Vision

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `image.txt2img` | prompt、negative_prompt、n、aspect_ratio、quality、seed、output | image artifacts，FileObject meta 写 media type / size | output media type 不支持、预算超限 |
| `image.img2img` | source image、prompt、strength、output | image artifacts | source image invalid、strength 越界 |
| `image.inpaint` | image、mask、prompt、mask_semantics | image artifacts | mask 缺失、mask semantics 不兼容 |
| `image.upscale` | image、scale、target size、preserve_faces | image artifact | 目标分辨率不满足、fallback 不能满足硬约束 |
| `image.bg_remove` | image、mode、output | rgba image artifact | 输入非图片、输出 alpha 缺失 |
| `vision.ocr` | document、level、language_hints、return_layout、return_artifacts | text、`extra.ocr`、OCR artifacts | 不支持语言、文档 meta 缺失 |
| `vision.caption` | image、style、language、n | captions、summary text | n 越界、图片无效 |
| `vision.detect` | image、classes、score_threshold、bbox_spec | detections with bbox | bbox unit 不支持 |
| `vision.segment` | image、prompt、mask_format、return_bitmap_mask | masks、bitmap artifact | mask_format 不支持 |

### 7.4 Audio / Video

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

### 7.5 Agent Runtime Support

`agent.computer_use` 当前作为占位方向，不作为普通 AICC v0 模型调用的强制真实 Provider 验收项。Mock 阶段只验证 schema、路由目录和安全约束：

- screenshot resource。
- viewport。
- allowed actions。
- action array response。
- 不允许无 sandbox / environment 上下文的真实执行。

## 8. Provider 协议覆盖

| Provider | 输入格式 | 输出格式 | Streaming / 异步 | Mock 重点 |
|---|---|---|---|---|
| OpenAI | Responses API、image generation/edit、audio transcription/speech、embedding | text、tool calls、JSON schema、image/audio artifact、usage | SSE delta 归并；图片/音频直接 artifact | tool call、JSON schema、vision content part、rate limit、context too long |
| Claude | Messages API、content blocks、tool use、vision block | text block、tool_use、stop_reason、usage | SSE event stream 归并 | content block 转换、tool schema、vision fallback、overloaded/rate limit |
| Google / Gemini | `generateContent`、多模态 parts、embedding、image/video/audio | candidates、function_call、safety、media outputs | streamGenerateContent / 长任务 operation | parts 映射、safety block、multimodal embedding space、video operation |
| OpenAI-compatible / OpenRouter | Chat completions 或 responses-like | OpenAI-like，但字段可能缺失或扩展 | SSE 兼容差异 | 兼容字段缺失、模型名映射、provider-specific error |
| fal | 图片/音频/视频工具型任务 | artifact URL / operation status | 异步 submit + poll | upscale、bg_remove、audio.enhance、video.upscale、operation timeout |
| SN AI Provider | AICC 内置 SN provider settings，经 SN 转发到兼容模型服务 | OpenAI-like 或 SN 归一响应 | 由 SN provider 能力决定 | 无 API key 参数、SN 链路可达性、provider instance 命名、usage / trace 归因 |

P0 Provider 最小集合按 `aicc_provider_plan.md`：

- `openai.rs`
- `claude.rs`
- `google.rs` / 当前实现中的 Gemini adapter
- `fal.rs`

OpenRouter 在 Mock 和 Provider adapter 单测中仍可作为 P1 optional provider；在 L4 gateway 真实模型验收中必须纳入 Provider 覆盖矩阵，用于验证 OpenAI-compatible 长尾模型、成本 fallback 和兼容性。

### 8.1 L4 真实 Provider 与模型矩阵

L4 不再按“每 Provider 一条用例”收敛，而是由 runner 在测试开始时读取当前临时 group 中的 `models.list` / Provider inventory，生成以下矩阵：

```text
case_set = {
  provider in [openai, fal, google, claude, openrouter, sn]
} x {
  model in provider.supported_models
}
```

矩阵生成规则：

1. `supported_models` 以 AICC 实际注册并可被 `models.list` 观察到的模型为准，包含精确模型名和其支持的 `api_types`。
2. 同一个 Provider 下同一个物理模型如果支持多个 `api_types`，runner 应选择能覆盖该模型主要能力的复杂 workflow；必要时一个模型可拆成 `llm` / `media` / `embedding` 等子用例，但报告必须仍能按 Provider × model 聚合。
3. Provider 已启用但没有任何可用模型时，生成一个 `skipped` 诊断用例，原因记为 `provider_has_no_models`。
4. `sn` Provider 不需要 API key；如果临时 group 的 SN provider 没有注册成功，应判为环境或配置失败，而不是 key 缺失。
5. `openai`、`fal`、`google`、`claude`、`openrouter` 缺少对应 API key 时，该 Provider 的全部真实模型用例标记为 `skipped`，并在报告中按 Provider 汇总。
6. 每个真实模型用例最多执行 3 次 attempt：首次失败后只重跑同一个 Provider × model × workflow 用例 2 次；任意一次 attempt 成功则该用例最终为 `passed`。
7. attempt 失败原因必须全部保留在报告中，最终成功的用例也要记录之前失败 attempt 的 `failure_class`、错误码和耗时，便于分析不稳定性。

## 9. Mock Provider 契约

Mock Provider 必须提供统一、确定、低成本的行为控制能力。

### 9.1 通用原则

- 固定 seed、固定响应、固定 usage、固定 cost、固定 latency bucket。
- 不访问外网，不依赖真实 API key。
- 所有非确定行为必须由测试显式配置。
- 支持 provider health、quota、pricing、capabilities 的动态切换。
- 支持非结构化输出策略：小结果 inline，大结果 `named_object` artifact。
- 支持 Provider-native streaming 模拟：按固定 chunk 输出，Adapter 聚合后写最终 `AiResponseSummary`，中间 progress 写 task data。

### 9.2 行为控制

Mock 行为可通过 request `payload.options.mock_behavior`、测试专用 header 或 Mock 管理接口控制。推荐字段：

```json
{
  "mock_behavior": {
    "scenario": "success",
    "latency_ms": 20,
    "stream_chunks": ["hello", " ", "world"],
    "error": null,
    "usage": {
      "input_tokens": 10,
      "output_tokens": 5,
      "total_tokens": 15
    },
    "artifact_size": "small"
  }
}
```

必须支持的 `scenario`：

| scenario | 含义 |
|---|---|
| `success` | 固定成功 |
| `stream_success` | 固定 streaming chunks，最终成功 |
| `async_success` | 返回 running，随后写最终 task result |
| `rate_limit` | 返回 429 / provider rate limit |
| `quota_exhausted` | 返回 quota exhausted |
| `provider_5xx` | 返回 Provider 5xx |
| `timeout` | 超时 |
| `malformed_response` | 返回格式错误 |
| `missing_usage` | 成功响应缺 usage |
| `invalid_resource` | 资源无效 |
| `safety_blocked` | Provider 内容安全拒绝 |
| `health_unavailable` | Provider health 不可用 |

## 10. 测试数据与资源 Fixture

`test/aicc_test/fixtures` 和 Rust test fixture 中应维护固定数据：

| 类型 | Fixture | 用途 |
|---|---|---|
| 图片 | 小 PNG、大 PNG、透明 PNG、JPEG、mask PNG | `image.*`、`vision.*`、多模态 `llm.chat` |
| 音频 | 短 wav/mp3、长音频、噪声音频 | `audio.tts`、`audio.asr`、`audio.enhance` |
| 视频 | 短 mp4 或 mock video object | `video.*` |
| 文档 | text chunk、PDF/image page mock、OCR 样例 | `embedding.text`、`vision.ocr`、`rerank` |
| 结构化数据 | JSON schema、tool schema、rerank docs | `llm.chat`、`rerank` |
| 大批量数据 | 101 条 embedding items 或超过 1MB 预估响应 | artifact 输出策略 |

每个 fixture 应有固定 digest、media type、size、必要 metadata，便于验证 FileObject meta 和 artifact 输出。

## 11. 异常路径

必测异常：

| 类别 | 场景 |
|---|---|
| 路由 | 非法模型名、模型不存在、无候选、策略拒绝、fallback loop、logical tree loop、精确模型不可用 |
| 策略 | `local_only` 无本地候选、预算超限、feature unsupported、context too long、provider 被禁用 |
| Provider | 401/403、429、5xx、timeout、quota exhausted、malformed response、missing usage、unsupported media type |
| 任务 | cancel unknown task、cancel forbidden、provider 不支持取消、异步任务最终失败 |
| 幂等 | 重复 key 命中 running/succeeded/failed/cancelled；相同 key 不同 body 返回 conflict |
| 配置 | settings schema 非法、凭据缺失、reload 后 provider 数量为 0、inventory 为空 |
| 安全 | 跨 tenant 查询/取消拒绝，trace/log 不包含 token、prompt 原文、原始文件内容 |

错误验收要求：

- 错误码可机器判断。
- 用户可见 message 可理解。
- route trace 记录候选过滤、fallback、failover 原因。
- Provider 原始错误只保留脱敏摘要。
- 早期 kRPC error 不创建 AICC task；已创建 task 的失败写入 task data / event。

## 12. 配置与重载验收

必须覆盖完整闭环：

```text
system_config 写入 settings
  -> service.reload_settings
  -> models.list
  -> route
  -> provider call
  -> usage / trace
```

用例：

1. 写入 Mock OpenAI settings，`base_url` 指向本地 Mock，reload 后 `models.list` 出现 `openai-mock-1`。
2. 禁用 Provider，reload 后候选消失，调用返回无候选或策略拒绝。
3. 修改 Provider capabilities，reload 后 `must_features` 硬过滤结果变化。
4. 修改 `provider_type` 为 `local_inference` / `cloud_api` / `proxy_unknown`，验证 `local_only` 过滤。
5. 全量覆盖 settings 和局部更新 settings 都能生效。
6. settings 非法时 reload 失败，不破坏上一版可用配置。

## 13. Usage Log 验收

用例：

1. 成功同步调用写入 exactly one usage event。
2. 成功异步最终完成写入 exactly one usage event。
3. 相同 `tenant_id + method + idempotency_key` 重试不重复写 usage event。
4. Provider 成功但缺 usage，调用应失败为 provider protocol error，不写成功 usage event。
5. TaskMgr completed task 删除后 usage event 仍可查询。
6. `last_1d` 按 provider model 汇总。
7. `last_7d` 按 provider model 汇总。
8. 自定义时间范围按 request model + provider model 汇总。
9. raw events 支持 `limit` / `cursor`。
10. finance snapshot 存在时写入，缺失时不影响成功。

## 14. 安全与脱敏验收

增加专门用例扫描报告、trace、task data、日志摘要，确认不出现：

- API key。
- session token。
- 原始 prompt 全文。
- 原始文件内容。
- Provider 原始敏感响应。

隐私策略用例：

1. `local_only=true` 时云端 Provider 被硬过滤。
2. `provider_type=proxy_unknown` 不被视为本地。
3. Provider inventory 自声明 `attributes.local=true` 但 system_config 中 `provider_type=cloud_api` 时，不能通过本地过滤。
4. 用户级策略不能覆盖组织级 locked policy。
5. 跨用户、跨租户查询和取消任务被拒绝。

## 15. Gateway 真实模型验收

真实模型成本受控：

- 每个真实 Provider 按其支持模型展开用例；每个 Provider × model 用例执行一条复杂 workflow。
- 每条 workflow 尽量覆盖该 Provider / model 最有代表性的能力。
- 首次失败后只重跑同一个 Provider × model × workflow 用例，最多累计 3 次 attempt。
- 任意 attempt 成功则该用例成功，所有 attempt 摘要都写入报告。
- 不断言自然语言全文，只断言协议事实。
- 未配置 API key 或 Provider 未启用时用例标记为 `skipped`，不算失败。
- `sn` Provider 不需要 API key，缺 key 不得作为 skip 原因。
- 真实模型返回可理解错误时，报告记录为 `failed` 或 `partial`，保留错误码、Provider 摘要、trace id。

每条真实模型 workflow 至少断言：

1. response schema 正确。
2. task 状态闭环。
3. artifact 可读取。
4. usage 存在。
5. route trace 存在。
6. 错误被分类。
7. 成本调用次数受控。

建议 workflow：

| Provider | Workflow |
|---|---|
| OpenAI | 每个模型执行 `llm.chat` 多轮 + JSON schema + tool call + image/audio 或 embedding 子步骤 |
| Claude | 每个模型执行多模态 `llm.chat` + tool use + vision caption/OCR fallback |
| Google | 每个模型执行多模态 `llm.chat` + embedding/multimodal 或 image/video operation |
| fal | 每个模型执行 `image.upscale` / `image.bg_remove` / `audio.enhance` / `video.upscale` 中匹配能力的异步任务 + artifact 读取 |
| OpenRouter | 每个模型执行 `llm.chat` 复杂 JSON 输出 + OpenAI-compatible 兼容字段检查 |
| SN AI Provider | 每个模型执行无 key 的 gateway 转发 workflow，验证 provider 归因、usage 和 trace |

## 16. 统一执行脚本

规划在 `test/aicc_test` 增加统一 runner：

```text
test/aicc_test/run_acceptance.{ts|py}
```

执行顺序：

1. `cargo test -p aicc`
2. `cargo test -p buckyos-api --test aicc_client_test`
3. 检查本机 BuckyOS / AICC 状态。
4. 启动 TS Mock Provider。
5. 写入 Mock settings，调用 `service.reload_settings`。
6. 调用 `models.list` 验证 Mock Provider 生效。
7. 执行本地 kRPC TS 用例。
8. 如 TOML 配置启用 gateway，则通过 `buckyos-devkit` 创建临时 group。
9. 从宿主机经 gateway 登录临时 group，并写入真实 Provider settings。
10. 调用 `models.list` 生成 Provider × model 矩阵。
11. 执行真实模型 workflow；失败 case 最多累计 3 次 attempt。
12. 生成报告。
13. 清理本次 runner 创建的临时 group。

报告输出：

```text
test/aicc_test/reports/acceptance/<run_id>/
  summary.md
  summary.json
  cargo_aicc.log
  cargo_buckyos_api.log
  local_krpc/
  gateway/
    matrix.json
    attempts/
  artifacts/
  cleanup.log
```

## 17. 报告 Schema

`summary.json` 推荐结构：

```json
{
  "run_id": "20260509-acceptance-001",
  "started_at": "2026-05-09T00:00:00Z",
  "finished_at": "2026-05-09T00:10:00Z",
  "status": "failed",
  "summary": {
    "total": 120,
    "passed": 115,
    "failed": 2,
    "skipped": 2,
    "partial": 1
  },
  "layers": [
    {
      "layer": "L1",
      "status": "passed",
      "total": 60,
      "passed": 60,
      "failed": 0
    }
  ],
  "cases": [
    {
      "case_id": "routing_exact_model_no_fallback",
      "layer": "L1",
      "priority": "P0",
      "method": "llm.chat",
      "model": "gpt-5.2@openai-mock-1",
      "provider": "openai-mock-1",
      "status": "passed",
      "attempts_total": 1,
      "passed_after_attempt": 1,
      "error_code": null,
      "trace_id": "trace-aicc-001",
      "duration_ms": 20,
      "cost": {
        "amount": 0,
        "currency": "USD"
      },
      "artifacts": [],
      "skip_reason": null,
      "failure_reason": null,
      "attempts": [
        {
          "attempt": 1,
          "status": "passed",
          "failure_class": null,
          "error_code": null,
          "duration_ms": 20,
          "task_id": null,
          "trace_id": "trace-aicc-001"
        }
      ]
    }
  ]
}
```

报告状态：

| 状态 | 含义 |
|---|---|
| `passed` | 用例通过 |
| `failed` | 用例失败 |
| `skipped` | 前置条件缺失，例如未配置真实模型 key |
| `partial` | 真实模型协议链路成功，但模型内容或 Provider 状态导致部分断言不可稳定成立 |

退出码：

| 退出码 | 含义 |
|---:|---|
| 0 | 无失败 |
| 1 | 有失败 |
| 2 | 无失败但有 partial |

## 18. CI 与手工验收边界

| 范围 | 执行环境 | 是否阻塞合入 |
|---|---|---|
| L1 `cargo test -p aicc` | CI / 本地 | 是 |
| L2 `cargo test -p buckyos-api --test aicc_client_test` | CI / 本地 | 是 |
| L3 本地 kRPC + Mock Provider | CI 或 nightly；本地可手工 | P0 阶段应阻塞 |
| L4 gateway + 真实模型 | nightly / 手工 | 不阻塞普通合入，阻塞发布验收 |

真实模型 key 缺失时，L4 用例必须 `skipped`，不能算失败。

## 19. 阶段性验收门槛

### 19.1 Mock 阶段

- L1/L2/L3 P0 必须 100% 通过。
- 不允许访问真实模型。
- 测试执行环境必须可重复，失败必须可复现。
- usage、task、trace、resource、routing 的核心断言必须稳定。
- 报告必须能定位失败原因。

### 19.2 Gateway 阶段

- gateway 链路、鉴权、配置读取、报告生成必须稳定。
- 被测环境必须由 `buckyos-devkit` 临时 group 启动，宿主机 runner 作为客户端通过 gateway 访问。
- Provider 覆盖必须包含 `openai`、`fal`、`google`、`claude`、`openrouter`、`sn`。
- 用例覆盖必须按 Provider × 支持模型的笛卡尔积生成；每个用例执行一条复杂 workflow。
- 每个失败用例必须额外重试 2 次，累计最多 3 次；任意一次成功则最终判定为成功。
- 真实模型内容不要求固定，但协议、任务状态、错误分类、trace、usage 记录必须可判定。
- 真实模型失败不能吞掉原因，必须进入报告。
- 测试完成后必须清理临时 group；如果清理失败，报告记录 `cleanup_failed` warning。

## 20. 待实现任务清单

1. 在 `src/frame/aicc/tests` 补齐 L1 Mock Provider 和路由、调度、任务、usage、资源、异常测试。
2. 在 `src/kernel/buckyos-api/tests/aicc_client_test.rs` 增加 AiccClient 黑盒测试。
3. 在 `test/aicc_test` 增加 TypeScript Mock Provider。
4. 在 `test/aicc_test` 增加本地 kRPC 用例。
5. 在 `test/aicc_test` 增加 gateway TOML 配置和真实模型 workflow。
6. 增加统一 runner 和验收报告输出，支持 `buckyos-devkit` 临时 group 生命周期、Provider × model 矩阵生成和失败用例最多 3 次 attempt。
7. 增加 fixture 目录和固定测试资源。
8. 增加日志、trace、报告脱敏扫描。

## 21. 验收里程碑

验收落地建议拆成三个里程碑，避免一次性实现全部用例导致范围过大。

| 里程碑 | 范围 | 完成标准 |
|---|---|---|
| M0 | L1 白盒单测 + L2 AiccClient 黑盒测试 | Rust Mock Provider 可用；`cargo test -p aicc` 和 `cargo test -p buckyos-api --test aicc_client_test` 的 P0 用例 100% 通过 |
| M1 | L3 本地 kRPC + TypeScript Mock Provider | 本机 BuckyOS + AICC 启动后，可通过 TS Mock Provider 完成配置重载、`models.list`、各类 method、task、usage、trace 和异常路径测试 |
| M2 | L4 gateway + 真实模型验收 | runner 能启动临时 group，经 gateway 执行 Provider × model 矩阵 workflow，报告可区分 passed / failed / skipped / partial；真实模型失败原因和重试 attempt 可追踪 |

里程碑边界：

- M0 只要求进程内确定性测试，不启动完整 BuckyOS。
- M1 要求真实 AICC 服务进程和本地 kRPC 链路可用，但仍不访问真实模型。
- M2 允许访问真实模型，主要用于发布前验收和远程部署验证；M2 完成后必须清理 runner 创建的临时 group。

## 22. 当前实现现状与缺口

当前仓库中已有的相关内容：

| 路径 | 现状 |
|---|---|
| `src/frame/aicc/tests` | 已有多组 AICC Rust 测试文件，覆盖 adapter protocol、routing、scheduler、security、stream、task lifecycle、workflow 等方向 |
| `test/aicc_test` | 已有 TypeScript smoke、`models.list`、fal provider 测试和报告目录说明 |
| `src/kernel/buckyos-api/src/aicc_client.rs` | 已有 AiccClient 相关实现入口 |
| `doc/aicc` | 已有 API、路由、Provider、配置、kRPC 调用、usage log 等设计文档 |

仍需补齐的缺口：

| 缺口 | 目标位置 | 说明 |
|---|---|---|
| AiccClient 黑盒测试文件 | `src/kernel/buckyos-api/tests/aicc_client_test.rs` | 当前未看到独立 `tests` 目录，需要新增测试入口 |
| 统一 Mock Provider 契约 | `src/frame/aicc/tests`、`test/aicc_test` | Rust Mock 和 TS Mock 的行为控制字段应保持一致 |
| TypeScript Mock Provider | `test/aicc_test` | 用于 L3 本地 kRPC 确定性测试 |
| 统一验收 runner | `test/aicc_test/run_acceptance.{ts|py}` | 串联 cargo test、本地 kRPC、gateway、报告输出 |
| `buckyos-devkit` group 管理 | `test/aicc_test/run_acceptance.{ts|py}` 或独立 helper | 创建、启动、探测、清理临时 group；支持从空白虚拟机模板 clone 多节点 |
| L4 Provider × model 矩阵生成 | `test/aicc_test/cases/gateway_cases.toml` 或 runner 动态生成 | 根据 `models.list` / inventory 生成真实模型用例，覆盖 openai/fal/google/claude/openrouter/sn |
| fixture 目录 | `test/aicc_test/fixtures`，必要时 Rust tests 内也保留最小 fixture | 固定图片、音频、视频、文档、mask、embedding 大批量输入 |
| 报告 schema 固化 | `test/aicc_test/reports/acceptance` | 输出 `summary.json` 和 `summary.md`，方便 CI / 发布验收读取 |
| 脱敏扫描 | runner 或独立检查脚本 | 扫描报告、trace、task data、日志摘要 |

## 23. 用例命名规范

用例 ID 采用稳定、可检索、可映射到需求的格式：

```text
<layer>_<domain>_<feature>_<scenario>
```

字段约定：

| 字段 | 示例 |
|---|---|
| `layer` | `l1`、`l2`、`l3`、`l4` |
| `domain` | `routing`、`scheduler`、`provider`、`protocol`、`resource`、`task`、`usage`、`security`、`settings`、`gateway` |
| `feature` | `exact_model`、`fallback`、`stream_merge`、`reload_settings`、`named_object` |
| `scenario` | `success`、`no_fallback`、`rate_limit`、`policy_rejected`、`conflict` |

示例：

```text
l1_routing_exact_model_success
l1_routing_exact_model_no_fallback
l1_scheduler_weight_profile_cost_first
l1_provider_openai_stream_merge
l2_client_idempotency_conflict
l3_krpc_reload_settings_mock_openai
l3_resource_named_object_image_txt2img
l4_gateway_openai_gpt_5_4_complex_workflow
l4_gateway_fal_video_upscale_workflow
```

命名要求：

- case id 一旦进入报告，不应随意改名。
- case id 应能从名字看出层级、功能域和主要断言。
- 需求追踪矩阵中的用例族可用前缀表达，例如 `l1_routing_exact_model_*`。

## 24. Mock Provider 配置样例

L3 TypeScript Mock Provider 建议使用 TOML 或 JSON 配置。示例：

```toml
[server]
host = "127.0.0.1"
port = 18080

[[providers]]
provider_instance_name = "openai-mock-1"
provider_type = "cloud_api"
provider_driver = "openai"
base_path = "/v1"
health = "available"
quota_state = "normal"

[[providers.models]]
provider_model_id = "gpt-5-mini"
exact_model = "gpt-5-mini@openai-mock-1"
api_types = ["llm.chat", "llm.completion", "embedding.text", "image.txt2img"]
logical_mounts = ["llm.gpt5", "llm.chat", "embedding.text", "image.txt2img"]
features = ["json_output", "tool_calling", "web_search", "vision", "streaming"]
max_context_tokens = 128000
quality_score = 0.90
latency_ms = 20
cost_per_1k_input_tokens = 0.0
cost_per_1k_output_tokens = 0.0

[[scenarios]]
name = "success"
status = 200
latency_ms = 20
usage_input_tokens = 10
usage_output_tokens = 5

[[scenarios]]
name = "rate_limit"
status = 429
provider_code = "mock/rate_limit"

[[scenarios]]
name = "stream_success"
status = 200
stream_chunks = ["hello", " ", "world"]
usage_input_tokens = 10
usage_output_tokens = 3
```

配置要求：

- Provider settings 中的 `base_url` 指向 Mock Provider。
- Mock Provider 返回的 inventory、health、usage 和错误码必须可由配置控制。
- 同一个 scenario 在 Rust Mock 和 TS Mock 中语义一致。
- scenario 触发方式必须稳定，推荐通过 `payload.options.mock_behavior.scenario` 指定。

## 25. 发布验收标准

发布前建议满足以下硬指标：

1. P0 Mock 用例 100% 通过。
2. `cargo test -p aicc` 通过。
3. `cargo test -p buckyos-api --test aicc_client_test` 通过。
4. 本地 kRPC Mock 验收能完成 `reload_settings -> models.list -> route -> provider call -> task / usage / trace` 闭环。
5. gateway runner 能读取 TOML 配置并生成 `summary.md` 和 `summary.json`。
6. gateway runner 能通过 `buckyos-devkit` 启动临时 group，并从宿主机经 gateway 完成访问。
7. 已配置真实 key 的 Provider 必须覆盖其全部可用模型；`sn` Provider 必须无 key 覆盖；未配置 key 的 Provider 在普通开发验收中标记为 `skipped`，发布强覆盖验收中应 preflight 失败。
8. 报告、trace、task data、日志摘要中不得出现 API key、session token、原始 prompt 全文和原始文件内容。
9. 真实模型调用次数、attempt 次数和成本在报告中可见。
10. 所有 failed / partial 用例都有明确失败原因、错误码或 Provider 摘要。
11. runner 创建的临时 group 已清理，或报告中明确记录保留原因和清理命令。

## 26. 性能与并发最低要求

性能和并发不作为第一阶段的主要目标，但需要设置最低验收线，避免破坏基础可用性。

| 项目 | 最低要求 | 主要层级 |
|---|---|---|
| 路由解析耗时 | Mock 环境下单次普通路由不应成为主要耗时瓶颈；建议记录 p50 / p95，不先设置硬阈值 | L1/L3 |
| 并发 session config patch | 同一 `session_id` 的并发 patch 必须按 revision 保持一致性；冲突返回明确错误 | L1 |
| 幂等重试 | 并发重复提交同一 `idempotency_key` 不得重复执行 Provider，不得重复写 usage | L1/L3 |
| usage 写入 | 多个异步任务并发完成时，usage event 不串任务、不重复、不丢失 | L1/L3 |
| artifact 输出 | 并发生成 artifact 时 ObjectId、meta、task result 不串 | L3 |
| failover | 多候选并发失败时，trace 能区分每次 attempt，不污染其它 request | L1/L3 |
| task 状态 | 多个异步任务并发运行和完成时，状态、event_ref、最终 result 对应正确 | L1/L3 |

并发测试建议：

1. 同一 session 多个 request 并发读 route binding。
2. 同一 session 多个 request 并发提交 session config patch。
3. 同一 idempotency key 并发提交相同 request。
4. 同一 idempotency key 并发提交不同 request。
5. 多个异步 Mock video/audio/image task 并发完成。
6. Provider 先失败后 failover 的请求与普通成功请求并发执行。

## 27. 分层用例清单

本节把前文功能域拆成更接近实现任务的用例清单。用例 ID 可在实现时继续细化，但应保持前缀稳定。

### 27.1 L1 白盒单测

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l1_routing_exact_model_*` | P0 | 精确模型解析、Provider instance 校验、API type 校验、默认不 fallback |
| `l1_routing_logical_tree_*` | P0 | 逻辑目录展开、items target、目录软链接、候选去重 |
| `l1_routing_fallback_*` | P0 | `strict`、`parent`、`target_exact`、`target_logical`、`disabled` |
| `l1_routing_loop_*` | P0 | fallback loop、logical tree loop、最大 fallback depth |
| `l1_scheduler_weight_*` | P0 | item weight、exact model weight、weight 0 硬过滤、同权重 profile 评分 |
| `l1_scheduler_profile_*` | P0 | `cost_first`、`latency_first`、`quality_first`、`balanced`、`local_first`、`strict_local` |
| `l1_session_sticky_*` | P0 | session route binding、binding TTL、Provider 不可用后的重新选择 |
| `l1_session_config_*` | P0 | full config、patch、inherit、revision conflict、expired、policy locked |
| `l1_provider_protocol_openai_*` | P0 | OpenAI request/response 转换、tool call、JSON schema、SSE 聚合 |
| `l1_provider_protocol_claude_*` | P0 | Claude content block、tool use、vision block、stop reason、usage |
| `l1_provider_protocol_gemini_*` | P0 | Gemini parts、function call、safety block、operation 状态 |
| `l1_provider_protocol_fal_*` | P1 | fal submit/poll、artifact URL、operation timeout |
| `l1_resource_ref_*` | P0 | `url`、`base64`、`named_object`、FileObject meta 推导 |
| `l1_task_lifecycle_*` | P0 | immediate、async running、final succeeded、failed、cancel |
| `l1_usage_log_*` | P0 | 成功写 usage、幂等去重、缺 usage 报错、查询聚合 |
| `l1_security_*` | P0 | `local_only`、`proxy_unknown`、locked policy、trace 脱敏 |
| `l1_concurrency_*` | P1 | session patch 并发、幂等并发、异步任务并发完成 |

### 27.2 L2 AiccClient 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l2_client_llm_chat_success` | P0 | AiccClient 构造标准 `llm.chat` 请求并解析成功响应 |
| `l2_client_exact_model_no_fallback` | P0 | 精确模型不可用时透传可判断错误 |
| `l2_client_idempotency_*` | P0 | running / succeeded / failed / conflict 语义 |
| `l2_client_async_task_*` | P0 | running response、event_ref、最终 task 查询 |
| `l2_client_cancel_*` | P0 | cancel 成功、unknown task、forbidden |
| `l2_client_resource_ref_*` | P1 | client 侧 `ResourceRef` JSON tag 和反序列化 |
| `l2_client_error_mapping_*` | P0 | kRPC error 与 AICC task failed error 的边界 |

### 27.3 L3 本地 kRPC 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l3_settings_reload_mock_*` | P0 | system_config 写入 Mock settings、reload、models.list |
| `l3_krpc_llm_chat_*` | P0 | 纯文本、多模态 content part、tool call、JSON schema |
| `l3_krpc_resource_*` | P0 | `url`、`base64`、`named_object` 输入和 artifact 输出 |
| `l3_krpc_stream_*` | P0 | Mock streaming chunks、task data progress、final summary |
| `l3_krpc_async_*` | P0 | image/audio/video 类异步 task 状态闭环 |
| `l3_krpc_usage_*` | P0 | usage event 写入和查询 |
| `l3_krpc_failover_*` | P0 | Provider timeout / 5xx / quota exhausted 后 failover |
| `l3_krpc_security_*` | P0 | local_only、跨用户访问拒绝、脱敏扫描 |
| `l3_krpc_legacy_*` | P1 | legacy alias、旧字段兼容或迁移提示 |

### 27.4 L4 Gateway 真实模型验收

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l4_gateway_openai_<model>_complex_workflow` | P2 | OpenAI 每个支持模型的文本、JSON schema、tool call、usage、trace |
| `l4_gateway_claude_<model>_complex_workflow` | P2 | Claude 每个支持模型的多模态或 vision、tool use、usage、trace |
| `l4_gateway_google_<model>_complex_workflow` | P2 | Google / Gemini 每个支持模型的多模态、safety / function call / operation 语义 |
| `l4_gateway_openrouter_<model>_complex_workflow` | P2 | OpenRouter 每个支持模型的 OpenAI-compatible 协议兼容、usage、trace |
| `l4_gateway_fal_<model>_media_workflow` | P2 | fal 每个支持模型的 image/video/audio 工具型异步任务和 artifact |
| `l4_gateway_sn_<model>_complex_workflow` | P2 | SN AI Provider 每个支持模型的无 key 链路、usage、trace、provider 归因 |
| `l4_gateway_models_list` | P2 | 真实环境 inventory、逻辑目录和 Provider health 可诊断 |

L4 用例 ID 中的 `<model>` 必须使用稳定可读的 slug，由精确模型名归一化得到；报告中必须保留原始精确模型名。

## 28. 执行命令约定

推荐统一 runner 最终屏蔽底层命令，但文档仍保留基础命令，便于开发者单独定位问题。

### 28.1 Rust 单测

在仓库根目录执行：

```bash
cargo test -p aicc
cargo test -p buckyos-api --test aicc_client_test
```

如果按 AGENTS.md 建议在 `src` 目录运行，也可使用 workspace 相对命令；最终 runner 应固定工作目录，避免不同目录下路径解析不一致。

### 28.2 本地 kRPC Mock 验收

推荐流程：

```bash
uv run src/check.py
cd test/aicc_test
pnpm install
pnpm run acceptance:local
```

`acceptance:local` 预期完成：

1. 启动或连接 TS Mock Provider。
2. 写入 Mock settings。
3. 调用 `service.reload_settings`。
4. 调用 `models.list` 验证配置生效。
5. 运行 L3 用例。
6. 输出 `reports/acceptance/<run_id>`。

### 28.3 Gateway 真实模型验收

```bash
cd test/aicc_test
pnpm run acceptance:gateway -- --config ./aicc_acceptance.toml
```

真实模型验收必须显式传入配置文件；不应从开发者环境变量中隐式读取 key 后直接发起调用，避免误触发费用。

推荐最终提供一个全量自动化入口，用于发布前一次性执行 L1/L2/L3/L4 并输出报告：

```bash
cd test/aicc_test
pnpm run acceptance:all -- \
  --openai-key "<openai-api-key>" \
  --fal-key "<fal-api-key>" \
  --google-key "<google-api-key>" \
  --claude-key "<claude-api-key>"
```

`acceptance:all` 的职责：

1. 固定从仓库根目录或 `src` 目录解析路径，避免工作目录差异。
2. 执行 L1/L2 Rust 单测。
3. 执行 L3 本地 Mock 验收。
4. 使用 `buckyos-devkit` 创建并启动 L4 临时 group，通过 gateway 访问该 group。
5. 将传入的 4 个 key 写入临时 group 的 AICC settings；`sn` Provider 不需要 key。
6. 对 `openrouter`，runner 优先读取配置文件或临时 group settings 中的 `openrouter` key；如果发布验收要求强覆盖但缺 key，应在 preflight 阶段失败。普通开发验收可将 openrouter 矩阵标记为 `skipped`。
7. 动态读取 `models.list`，生成 Provider × model 矩阵。
8. 每个矩阵用例执行复杂 workflow，失败后最多额外执行 2 次。
9. 输出 `summary.md`、`summary.json` 和脱敏后的 attempt 明细。
10. 清理 runner 新建的临时 group。

为了让参数尽可能少，`sn` 不设置 key；OpenRouter key 不作为默认必填命令行参数，但发布验收若要求 OpenRouter 强覆盖，必须通过 `--openrouter-key` 或配置文件提供。

## 29. Gateway TOML 配置约定

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
matrix_mode = "provider_model_cartesian"
providers = ["openai", "fal", "google", "claude", "openrouter", "sn"]

[providers.openai]
enabled = true
api_key = ""

[providers.claude]
enabled = true
api_key = ""

[providers.google]
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

[providers.sn]
enabled = true
api_key = ""
requires_api_key = false
```

配置规则：

- `allow_real_model_calls` 默认为 `false`。只有显式设为 `true` 才允许发起真实模型调用。
- `matrix_mode=provider_model_cartesian` 时，runner 必须按 Provider × 该 Provider 支持模型生成 L4 用例。
- `max_attempts_per_case` 默认为 `3`；只有首轮失败的用例才继续执行第 2 / 第 3 次 attempt。
- Provider `enabled=true` 但缺 key 时，用例标记 `skipped`；发布强覆盖模式下，缺 key 可在 preflight 直接失败。
- `sn` Provider 的 `requires_api_key=false`，缺 key 不应导致 skipped。
- Provider key 不写入报告和日志。
- runner 应把最终生效配置的脱敏摘要写入报告。
- `managed_by_devkit=true` 时，runner 负责创建、启动、探测和清理 group；`keep_on_failure=true` 只用于人工排查，报告必须明确标注遗留环境名。

### 29.1 `buckyos-devkit` 临时 group 生命周期

L4 runner 应把被测环境视为一次性资源，推荐流程：

1. 生成唯一 `run_id` 和 `group_name`，例如 `aicc-acceptance-20260511-153000`。
2. 检查 `buckyos-devkit` / `buckyos-devtest`、Multipass、Python、`uv`、`cargo`、`pnpm` 是否可用。
3. 构造或复用空白 VM 模板；如果本次需要多个虚拟机，先构造一个空白虚拟机，再 clone 出 SN、OOD、普通节点等实例，然后按 group 配置修改 hostname、hosts、端口映射和 app 参数。
4. 使用 group template 生成临时 group 配置，最小建议为 `sn + alice-ood1`；需要多 Provider 节点或 gateway 冗余时再扩展节点。
5. 执行 `create_vms` / `install` / `start`，并等待 gateway、system-config、verify-hub、scheduler、task-manager、AICC 全部可访问。
6. 宿主机 runner 通过 gateway 登录并获取测试 token，后续所有 L4 调用都经 gateway 访问 `/kapi/aicc` 和相关 task / artifact 接口。
7. 写入真实 Provider settings，触发 `reload_settings`，调用 `models.list` 生成 Provider × model 矩阵。
8. 运行 L4 矩阵用例并收集报告。
9. 默认执行 `stop` / `clean_vms` 清理临时 group；除非显式 `keep_on_failure=true`，失败环境也必须清理。

清理约束：

- runner 只能清理自己创建且带有本次 `run_id` 标签或命名前缀的 group / VM。
- 清理前应把必要日志、AICC settings 脱敏摘要、`models.list` 输出和失败 attempt 摘要复制到报告目录。
- 清理失败不能覆盖测试结论，应记录为 `cleanup_failed` warning，并列出残留 group / VM 名称。

## 30. 真实模型判定规则

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

1. 重试粒度是单个 Provider × model × workflow 用例，不得扩大到整个 Provider 或整个测试批次。
2. 第 1 次 attempt 失败后，runner 应立即重跑同一用例；第 2 次仍失败时再重跑第 3 次。
3. 任意 attempt 满足通过断言，则该 case 最终状态为 `passed`，并在报告中标注 `passed_after_attempt=N`。
4. 三次 attempt 全部失败时，该 case 最终状态为 `failed`，主失败原因取最后一次 attempt，同时保留全部 attempt 明细。
5. `skipped` 不重试；preflight / 配置错误不重试；明显安全失败不重试。
6. 对已经返回 `running` 或已提交异步任务的 attempt，不允许在同一个 task 上静默重复提交；重试必须创建新的 case attempt id，并在报告中记录可能产生的真实费用。

## 31. 测试环境确定性要求

Mock 阶段必须保证执行环境确定：

1. Mock Provider 固定端口或由 runner 分配端口后写入 settings。
2. Mock Provider 启动成功后必须有 health check。
3. 每次运行使用独立 `run_id` 和独立报告目录。
4. fixture 数据固定 digest；runner 在开始时校验 digest。
5. Mock scenario 不依赖系统时间，除非用例明确测试 TTL / timeout。
6. timeout 用例应使用虚拟时钟或短固定延迟，避免 CI 偶发失败。
7. 并发用例必须设置最大等待时间，并在失败报告中输出未完成任务列表。
8. 所有 Mock usage、cost、latency 都应由配置或用例明确指定。
9. 测试结束后清理 Mock settings 或恢复测试前 settings，避免影响开发环境。

## 32. 风险与处理策略

| 风险 | 影响 | 处理策略 |
|---|---|---|
| 真实模型输出不稳定 | gateway 用例误报 | 只断言协议事实，不断言自然语言全文 |
| 真实模型费用失控 | 成本风险 | `allow_real_model_calls=false` 默认值、按 Provider × model 生成前先输出预计用例数；失败用例最多 3 次 attempt；报告统计真实调用次数和估算成本 |
| L4 临时环境残留 | 占用本机资源、污染后续测试 | group 名带 `run_id`，默认 `cleanup_on_exit=true`；只清理 runner 创建的 group；清理失败写 warning |
| VM 多节点构造慢 | 发布验收耗时长 | 先构造空白 VM，再 clone 出所需节点；只在需要 gateway / SN / 多节点路径时扩展节点数 |
| Mock 与真实 Provider 差异过大 | Mock 通过但真实失败 | Mock 按 Provider 原生协议构造请求/响应，不只 mock AICC 内部 trait |
| Streaming 语义混乱 | UI 或 task 状态不一致 | AICC 协议只验最终 summary；中间态只验 task data / event |
| 使用量重复写入 | 账单和统计错误 | 幂等并发测试、usage 唯一约束测试 |
| 配置 reload 破坏旧状态 | 服务不可用 | 非法 settings reload 失败后必须保留上一版配置 |
| trace 泄露敏感信息 | 安全风险 | 脱敏扫描作为 P0 |
| 并发测试偶发失败 | CI 不稳定 | 固定 Mock 行为、短 timeout、失败输出足够诊断信息 |

## 33. 文档联动要求

后续实现测试或修改 AICC 协议时，需要同步检查：

- `doc/aicc/aicc_api设计.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/aicc 逻辑模型目录.md`
- `doc/aicc/krpc_aicc_calling_guide.md`
- `doc/aicc/update_aicc_settings_via_system_config.md`
- `doc/aicc/aicc_provider_plan.md`
- `doc/aicc/aicc_usage_log_db_requirements.md`
- `src/kernel/buckyos-api/src/aicc_client.rs`
- `src/frame/aicc/src`
- `test/aicc_test`

触发文档联动的变更包括：

1. 新增或改名 method。
2. 修改 request / response schema。
3. 修改 `ResourceRef` JSON 表达。
4. 修改 Provider settings 字段。
5. 修改 exact model 命名规则。
6. 修改 fallback、session config、policy 字段。
7. 修改 usage log schema。
8. 修改 task data / event 中 AICC 字段。

## 34. 用例 Manifest 约定

为便于统一 runner 执行和生成报告，建议把 L3/L4 用例声明为 manifest。Rust L1/L2 可以不强制使用 manifest，但报告中的 case metadata 应与 manifest 字段保持一致。

推荐文件：

```text
test/aicc_test/cases/
  local_mock_cases.toml
  gateway_cases.toml
```

Manifest 样例：

```toml
[[cases]]
case_id = "l3_krpc_llm_chat_json_schema_success"
layer = "L3"
priority = "P0"
method = "llm.chat"
model_alias = "llm.plan"
provider = "openai-mock-1"
scenario = "success"
timeout_ms = 30000
requires = ["mock_provider", "aicc_service", "task_manager"]
fixtures = []
expect_status = "succeeded"
expect_artifacts = false
expect_usage = true
expect_trace = true

[cases.input]
template = "llm_chat_json_schema.json"

[cases.assertions]
json_schema = "assertions/llm_chat_summary.schema.json"
no_sensitive_log = true

[[cases]]
case_id = "l4_gateway_openai_${model_slug}_complex_workflow"
layer = "L4"
priority = "P2"
method = "workflow"
provider = "openai"
model = "${exact_model}"
timeout_ms = 300000
requires = ["gateway", "real_model", "api_key:openai"]
max_attempts = 3
expect_status = "partial_or_passed"
expect_usage = true
expect_trace = true
```

字段说明：

| 字段 | 说明 |
|---|---|
| `case_id` | 稳定用例 ID，进入报告后不随意变更 |
| `layer` | `L1`、`L2`、`L3`、`L4` |
| `priority` | `P0`、`P1`、`P2` |
| `method` | AICC method 或 `workflow` |
| `model_alias` | 请求模型名，可为逻辑模型或精确模型 |
| `provider` | 期望命中的 Provider；路由类用例可为空 |
| `scenario` | Mock 行为场景 |
| `requires` | 前置能力；缺失时用例 `skipped` |
| `fixtures` | 所需 fixture 列表 |
| `expect_status` | `succeeded`、`running`、`failed`、`partial_or_passed` |
| `expect_usage` | 是否必须存在 usage |
| `expect_trace` | 是否必须存在 route trace |
| `max_attempts` | L4 单 case 最大 attempt 数；真实模型默认 3 |
| `matrix_source` | L4 动态矩阵来源，推荐 `models.list` |
| `model_slug` | 由精确模型名归一化得到的稳定用例 ID 片段 |

Runner 要求：

- manifest 解析失败应直接终止，不能静默跳过。
- `requires` 不满足时标记 `skipped`，并记录 `skip_reason`。
- 同一 manifest 内 `case_id` 必须唯一。
- 报告中的 case 顺序应与 manifest 顺序一致，便于人工阅读。
- L4 动态矩阵用例可以由模板 case 展开；展开后的 `case_id` 必须唯一，并保留 `provider`、`model`、`api_types`、`matrix_source`。
- L4 attempt 明细必须挂在同一个 case 下，不能展开成多个独立 case 影响通过率统计。

## 35. Mock Provider HTTP 接口约定

L3 TypeScript Mock Provider 应尽量模拟 Provider 原生接口，而不是只模拟 AICC 内部 trait。这样可以测试 Provider Adapter 的真实协议转换。

### 35.1 管理接口

Mock Provider 需要提供测试管理接口：

| Method | Path | 说明 |
|---|---|---|
| `GET` | `/__mock/health` | 健康检查 |
| `POST` | `/__mock/reset` | 清空请求记录和动态状态 |
| `POST` | `/__mock/scenario` | 设置默认 scenario 或按 request id 设置 scenario |
| `POST` | `/__mock/provider_state` | 设置 health、quota、latency、capabilities |
| `GET` | `/__mock/requests` | 返回已收到的脱敏请求记录 |
| `GET` | `/__mock/metrics` | 返回调用次数、错误次数、stream chunk 计数 |

管理接口不应暴露真实 key、session token 和原始敏感资源内容。

### 35.2 OpenAI-like 接口

建议支持：

| Method | Path | 覆盖能力 |
|---|---|---|
| `POST` | `/v1/responses` | `llm.chat`、tool call、JSON schema、stream |
| `POST` | `/v1/chat/completions` | OpenAI-compatible / legacy 兼容 |
| `POST` | `/v1/embeddings` | `embedding.text` |
| `POST` | `/v1/images/generations` | `image.txt2img` |
| `POST` | `/v1/images/edits` | `image.img2img`、`image.inpaint` |
| `POST` | `/v1/audio/transcriptions` | `audio.asr` |
| `POST` | `/v1/audio/speech` | `audio.tts` |

### 35.3 Claude-like 接口

建议支持：

| Method | Path | 覆盖能力 |
|---|---|---|
| `POST` | `/v1/messages` | `llm.chat`、content block、tool use、vision |
| `POST` | `/v1/messages?stream=true` | SSE streaming |

### 35.4 Gemini-like 接口

建议支持：

| Method | Path | 覆盖能力 |
|---|---|---|
| `POST` | `/v1beta/models/{model}:generateContent` | `llm.chat`、multimodal parts、function call |
| `POST` | `/v1beta/models/{model}:streamGenerateContent` | streaming |
| `POST` | `/v1beta/models/{model}:embedContent` | `embedding.text`、`embedding.multimodal` |
| `GET` | `/v1beta/operations/{operation}` | video / long running operation |

### 35.5 fal-like 接口

建议支持：

| Method | Path | 覆盖能力 |
|---|---|---|
| `POST` | `/fal-ai/esrgan` | `image.upscale` |
| `POST` | `/fal-ai/imageutils/rembg` | `image.bg_remove` |
| `POST` | `/fal-ai/deepfilternet3` | `audio.enhance` |
| `POST` | `/fal-ai/video-upscaler` | `video.upscale` |
| `GET` | `/queue/requests/{request_id}/status` | 异步状态 |
| `GET` | `/queue/requests/{request_id}` | 异步结果 |

## 36. Fixture Manifest 约定

固定资源应有 manifest，runner 开始时校验存在性、大小、digest 和 media type。

推荐文件：

```text
test/aicc_test/fixtures/manifest.toml
```

示例：

```toml
[[fixtures]]
id = "image_png_small"
path = "images/small.png"
media_type = "image/png"
size_bytes = 1024
sha256 = "..."
used_by = ["image.img2img", "vision.caption", "llm.chat"]

[[fixtures]]
id = "mask_png_alpha"
path = "images/mask_alpha.png"
media_type = "image/png"
sha256 = "..."
attributes = { width = 512, height = 512, has_alpha = true }
used_by = ["image.inpaint", "vision.segment"]

[[fixtures]]
id = "audio_wav_short"
path = "audio/short.wav"
media_type = "audio/wav"
sha256 = "..."
attributes = { duration_seconds = 2.0, sample_rate = 16000 }
used_by = ["audio.asr", "audio.enhance"]
```

Fixture 要求：

- 小文件可以直接进入仓库。
- 大文件应尽量使用可生成的 deterministic fixture，或在 runner 中按脚本生成。
- L4 真实模型使用外部 URL 时，必须在 TOML 中显式配置，不应默认访问不受控 URL。
- fixture 内容不应包含真实用户数据。

## 37. 预检与清理流程

统一 runner 执行前应做 preflight：

1. 确认当前工作目录和仓库根目录。
2. 确认必要命令存在：`cargo`、`uv`、`pnpm`、`deno` 或 `node`。
3. L3 前确认 BuckyOS 已启动，`uv run src/check.py` 返回可用状态。
4. 检查 AICC 服务是否可访问。
5. 检查 task-manager 是否可访问。
6. 检查 Mock Provider 端口是否可用；如端口占用，自动选择新端口并写入临时 settings。
7. 校验 fixture manifest。
8. 创建本次 `run_id` 和报告目录。
9. 如果是 L4，确认 `allow_real_model_calls=true`，否则跳过真实调用。
10. 如果是 L4，确认 `buckyos-devkit`、Multipass 和临时 group template 可用。
11. 如果是 L4，创建或 clone 临时 group VM，启动后通过 gateway 完成登录和 `/kapi/aicc` 连通性检查。
12. 如果是 L4，调用 `models.list` 生成 Provider × model 矩阵，并在真正执行前把矩阵摘要写入报告。

执行后应做 cleanup：

1. 停止 runner 启动的 Mock Provider。
2. 恢复或清理测试写入的 AICC settings。
3. 清理测试 session config 和 session route binding。
4. 清理未完成的 Mock task 或记录到报告。
5. 保留报告、输入、输出和脱敏后的 Provider 请求摘要。
6. 如果是 L4，停止并清理本次 runner 新建的临时 group / VM。
7. 如果 `keep_on_failure=true`，保留临时 group，但必须在报告中写入 group 名、节点名和手工清理命令。

清理失败不能覆盖原始测试失败原因，应作为单独 warning 写入报告。

## 38. 失败分类与诊断信息

报告中的失败原因应使用稳定分类，便于统计和自动处理。

| failure_class | 含义 |
|---|---|
| `preflight_failed` | 环境或依赖检查失败 |
| `config_failed` | TOML、settings、manifest 或 fixture 配置错误 |
| `service_unavailable` | AICC、task-manager、gateway 或 Mock Provider 不可访问 |
| `routing_failed` | 路由解析、候选、fallback、调度不符合预期 |
| `provider_protocol_failed` | Provider request/response 转换错误 |
| `provider_runtime_failed` | Provider 运行时返回错误、超时或状态异常 |
| `task_lifecycle_failed` | task 状态、event_ref、cancel、final result 不符合预期 |
| `resource_failed` | ResourceRef、artifact、FileObject meta 或读取失败 |
| `usage_failed` | usage 缺失、重复、查询不正确 |
| `security_failed` | 权限、隐私、脱敏失败 |
| `assertion_failed` | 用例断言失败 |
| `cleanup_failed` | 清理阶段失败 |

每个 failed case 至少记录：

- `case_id`
- `failure_class`
- `error_code`
- `message`
- `trace_id`
- `task_id`
- `provider`
- `model`
- `duration_ms`
- 脱敏后的 request 摘要
- 脱敏后的 response / error 摘要
- 相关日志片段位置

## 39. 实现任务拆分建议

建议按以下任务顺序实现，减少互相阻塞。

| 顺序 | 任务 | 主要文件 |
|---:|---|---|
| 1 | 固化 `summary.json` schema 和报告目录结构 | `test/aicc_test` |
| 2 | 增加 fixture manifest 和最小 fixture | `test/aicc_test/fixtures` |
| 3 | 实现 TS Mock Provider 管理接口和 OpenAI-like 最小接口 | `test/aicc_test` |
| 4 | 实现 L3 runner preflight、settings reload、models.list | `test/aicc_test` |
| 5 | 增加 L3 `llm.chat`、resource、usage、trace P0 用例 | `test/aicc_test` |
| 6 | 增加 L2 `aicc_client_test.rs` | `src/kernel/buckyos-api/tests` |
| 7 | 对齐 Rust Mock Provider 与 TS Mock scenario 契约 | `src/frame/aicc/tests`、`test/aicc_test` |
| 8 | 增加 Provider-specific protocol P0 用例 | `src/frame/aicc/tests` |
| 9 | 增加 gateway TOML、`buckyos-devkit` 临时 group 管理和真实模型 workflow | `test/aicc_test` |
| 10 | 增加 Provider × model 矩阵生成、失败 case 三次 attempt 和 attempt 报告 | `test/aicc_test` |
| 11 | 接入脱敏扫描和发布验收报告 | `test/aicc_test` |

每个任务完成后至少应能回答：

- 增加了哪些 case id。
- 覆盖了哪些需求追踪项。
- 如何单独运行。
- Mock 与真实模型是否都会触发。
- 报告中如何定位失败。

## 40. 评审清单

新增或修改 AICC 验收用例时，评审应检查：

1. case id 是否符合命名规范。
2. 是否标明 layer、priority、method、provider、scenario。
3. 是否可以稳定复现。
4. 是否避免真实模型默认调用。
5. 是否有明确断言，而不是只检查“不报错”。
6. 是否覆盖成功和至少一个失败路径。
7. 是否检查 usage、trace、task 或 artifact 中与该用例相关的关键字段。
8. 是否避免记录密钥、token、原始 prompt 和原始文件内容。
9. 是否在失败时输出足够诊断信息。
10. 是否更新需求追踪矩阵或 manifest。
11. 是否需要同步更新 `doc/aicc` 其它协议文档。
12. 是否会引入新的依赖；如需要，应先单独确认。

## 41. 首批 P0 最小用例集

为避免第一轮实现范围过大，M0/M1 阶段先落地以下最小 P0 用例集。该集合不追求覆盖全部 method，而是优先打通协议、路由、Mock、任务、资源、usage、trace 和异常主链路。

### 41.1 M0 最小集

| case id | 层级 | 目标 |
|---|---|---|
| `l1_routing_exact_model_success` | L1 | 精确模型名解析成功 |
| `l1_routing_exact_model_no_fallback` | L1 | 精确模型不可用且未开启 fallback 时失败 |
| `l1_routing_logical_model_candidates` | L1 | 逻辑模型展开候选列表 |
| `l1_routing_parent_fallback_success` | L1 | parent fallback 生效 |
| `l1_routing_fallback_loop_rejected` | L1 | fallback 环路被拒绝 |
| `l1_scheduler_weight_priority` | L1 | 目录 item weight 优先级生效 |
| `l1_scheduler_profile_cost_first` | L1 | 同优先级候选按 cost profile 选择 |
| `l1_session_sticky_hit` | L1 | 同 session 后续请求复用绑定 |
| `l1_session_config_revision_conflict` | L1 | session config revision 冲突返回明确错误 |
| `l1_security_local_only_rejects_cloud` | L1 | `local_only` 硬过滤云端 Provider |
| `l1_provider_openai_chat_success` | L1 | OpenAI-like `llm.chat` 协议转换成功 |
| `l1_provider_openai_stream_merge` | L1 | Provider streaming chunks 聚合为最终 summary |
| `l1_resource_ref_json_tags` | L1 | `url`、`base64`、`named_object` JSON tag 正确 |
| `l1_task_immediate_success` | L1 | 同步成功任务写入 result |
| `l1_task_async_success` | L1 | 异步任务 running 到 succeeded 闭环 |
| `l1_usage_success_write_once` | L1 | 成功调用写入 exactly one usage event |
| `l1_usage_missing_usage_rejected` | L1 | 成功响应缺 usage 被判为协议错误 |
| `l2_client_llm_chat_success` | L2 | AiccClient 调用 `llm.chat` 成功 |
| `l2_client_idempotency_conflict` | L2 | 同 key 不同 body 返回 idempotency conflict |
| `l2_client_cancel_unknown_task` | L2 | 取消不存在任务返回可判断错误 |

### 41.2 M1 最小集

| case id | 层级 | 目标 |
|---|---|---|
| `l3_settings_reload_mock_openai` | L3 | 写入 Mock settings 后 reload 生效 |
| `l3_models_list_mock_inventory` | L3 | `models.list` 可看到 Mock Provider inventory |
| `l3_krpc_llm_chat_text_success` | L3 | kRPC `llm.chat` 纯文本成功 |
| `l3_krpc_llm_chat_json_schema_success` | L3 | JSON schema 输出可解析 |
| `l3_krpc_resource_base64_image` | L3 | base64 图片资源输入成功 |
| `l3_krpc_resource_named_object_artifact` | L3 | named_object artifact 输出可读取 |
| `l3_krpc_stream_progress_and_final` | L3 | streaming 中间态写 task data，最终 summary 正确 |
| `l3_krpc_async_task_success` | L3 | 异步任务状态闭环 |
| `l3_krpc_provider_5xx_failover` | L3 | Provider 5xx 后按策略 failover |
| `l3_krpc_provider_timeout_failed` | L3 | Provider timeout 返回明确错误 |
| `l3_krpc_usage_query_last_1d` | L3 | usage 可按 last_1d 查询 |
| `l3_krpc_security_no_secret_in_report` | L3 | 报告和 trace 脱敏扫描通过 |

首批 P0 最小集通过后，再扩展到完整 P0/P1/P2 用例矩阵。

## 42. 验收报告示例

`summary.md` 建议面向人工阅读，保留失败定位信息和跳过原因。示例：

```markdown
# AICC Acceptance Report

- Run ID: 20260509-acceptance-001
- Mode: acceptance_all
- Gateway group: aicc-acceptance-20260509-001
- Started: 2026-05-09T10:00:00Z
- Finished: 2026-05-09T10:04:31Z
- Status: failed

## Summary

| Layer | Passed | Failed | Skipped | Partial | Duration |
|---|---:|---:|---:|---:|---:|
| L1 | 18 | 0 | 0 | 0 | 42s |
| L2 | 3 | 0 | 0 | 0 | 8s |
| L3 | 10 | 1 | 1 | 0 | 3m41s |
| L4 | 18 | 1 | 5 | 1 | 41m12s |

Total: 49 passed, 2 failed, 6 skipped, 1 partial.

## L4 Matrix

| Provider | Models | Passed | Failed | Skipped | Partial | Attempts |
|---|---:|---:|---:|---:|---:|---:|
| openai | 4 | 4 | 0 | 0 | 0 | 5 |
| claude | 3 | 3 | 0 | 0 | 0 | 3 |
| google | 4 | 3 | 1 | 0 | 0 | 7 |
| fal | 4 | 4 | 0 | 0 | 0 | 4 |
| openrouter | 5 | 0 | 0 | 5 | 0 | 0 |
| sn | 3 | 3 | 0 | 0 | 0 | 3 |

## Failed Cases

### l3_krpc_provider_5xx_failover

- Failure class: routing_failed
- Method: llm.chat
- Model: llm.plan
- Provider: openai-mock-1
- Error code: AICC_ROUTE_NO_CANDIDATE
- Trace ID: trace-aicc-20260509-0008
- Task ID: aicc-20260509-0008
- Duration: 3021ms
- Reason: expected failover to openai-mock-2, but no fallback candidate remained after hard filter.
- Artifacts: local_krpc/l3_krpc_provider_5xx_failover/

## Skipped Cases

| Case | Reason |
|---|---|
| l4_gateway_openai_* | allow_real_model_calls=false |
| l4_gateway_claude_* | missing api_key:claude |
| l4_gateway_openrouter_* | missing api_key:openrouter |

## Cost

| Provider | Real calls | Attempts | Estimated cost |
|---|---:|---:|---:|
| openai | 5 | 5 | USD 0.42 |
| claude | 3 | 3 | USD 0.31 |
| google | 7 | 7 | USD 0.28 |
| fal | 4 | 4 | USD 0.16 |
| openrouter | 0 | 0 | USD 0 |
| sn | 3 | 3 | USD 0 |

## Cleanup

- Temporary group: cleaned
- Cleanup warnings: 0

## Security Scan

- API keys: passed
- Session tokens: passed
- Raw prompt leakage: passed
- Raw file content leakage: passed
```

报告要求：

- `summary.md` 面向人工阅读。
- `summary.json` 面向 CI、脚本和后续分析。
- 失败用例必须提供 trace id、task id、failure class 和脱敏输入输出目录。
- skipped 不能只显示数量，必须显示原因。
- L4 报告必须显示真实模型调用次数、attempt 次数、Provider × model 覆盖矩阵和估算成本。
- L4 报告必须显示临时 group 是否已清理；如未清理，必须显示保留原因和手工清理命令。

## 43. 第一阶段明确不做

第一阶段目标是建立确定、可执行、可报告的验收体系。以下内容明确不做，避免范围失控：

1. 不压测真实模型。
2. 不把真实模型用例放入普通 CI 必跑。
3. 不比较真实模型自然语言质量。
4. 不断言真实模型自然语言全文一致。
5. 不实现复杂账单、发票、余额或对账逻辑。
6. 不把 Provider 原始完整响应写入报告。
7. 不把原始 prompt、原始文件内容、API key、session token 写入报告或普通日志。
8. 不要求所有需要 API key 的真实模型 Provider 在无 key 环境下通过；`sn` Provider 例外，它本身不需要 API key。
9. 不要求 `agent.computer_use` 在第一阶段接入真实桌面或浏览器环境。
10. 不要求所有视频、音乐等高成本能力在 Mock 阶段之外真实执行。
11. 不引入新的通用测试框架或依赖，除非先单独确认。
12. 不在验收 runner 中自动修改生产环境真实 Provider 配置，除非配置文件显式允许。
