# AICC L2 AiccClient 用例

定义 AiccClient 黑盒测试、SDK request/response、错误、任务接口和控制 method 语义。

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
| `route.resolve` | `api_type`、逻辑模型名 `logical_model`、requirements、disable、policy | `selected_exact_model`、provider 信息、`provider_options`、`fallback_attempts`、`enabled/disabled_capabilities`、`route_trace` | 传入 exact model 被拒（错误码明确）、无候选 |
| `chat.completions.create` | `exact_model`、content-block `messages`、tools、response_format、provider_options | `message: AiMessage`、`tool_calls`、`finish_reason`、usage、route trace | 传入逻辑模型名被拒、primary quota exhausted 不 fallback |
| `helper.llm_chat` | 逻辑模型名 + messages | 等价于 `route.resolve` + `chat.completions.create` | 与两阶段行为一致性 |
| `llm.chat`（legacy） | content-block messages、image/document/tool_use block、tools、response_format JSON schema、generation params | `text`/`message`、`tool_calls`、`finish_reason`、usage、route trace | tool schema 非法、JSON schema 不满足、context too long、feature unsupported |
| `llm.completion`（legacy） | `prompt`、`suffix` | `text`、`finish_reason` | legacy wrapper 到 chat 失败、空 prompt |

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
| `reload_settings` | 空 params 或兼容旧调用 | reload 结果、Provider registry / ModelRegistry 重建摘要 | settings 非法、凭据缺失、保留上一版可用配置 |
| `service.reload_settings` | 同 `reload_settings` | 同 `reload_settings` | 同 `reload_settings` |
| `models.list` | 空 params、可选诊断过滤参数 | Provider inventory、exact model、`api_types`、`logical_mounts`、逻辑目录、legacy aliases、health 摘要 | registry 为空、敏感字段泄露、损坏 metadata 不应导致服务不可诊断 |
| `service.models.list` | 同 `models.list` | 同 `models.list` | 同 `models.list` |
| `usage.query` | 时间窗口、provider/model/method/api_type 过滤 | 聚合 usage、明细数量、成本/usage 字段、空结果 | 非法时间窗口、无权限、重复幂等记录不应重复计费 |
| `quota.query` | capability / method、tenant/session 上下文 | 剩余额度、预算状态、限制来源 | 未配置 quota、跨 tenant 查询、非法 method |
| `provider.list` | 可选 provider/type/driver 过滤 | Provider 列表、inventory 摘要、health、capability、pricing 脱敏视图 | 无权限、凭据泄露、Provider 状态异常仍可诊断 |
| `provider.health` | provider instance / driver | health 状态、最近错误摘要、latency / quota / availability | Provider 不存在、health 过期、敏感错误未脱敏 |
| `provider.validate` | provider settings 草案、base_url、auth mode、模型声明 | schema 校验结果、可连接性 / mock 可达性、脱敏诊断 | 凭据缺失、base_url 非法、未知 driver、不得写入 system_config |
| `provider.add` | provider settings、tenant/session 上下文 | system_config 写入、reload 后 `models.list` 可见、审计记录 | 重名冲突、无权限、schema 非法、写入失败回滚 |
| `provider.delete` | provider instance name、tenant/session 上下文 | system_config 删除、reload 后候选消失、相关 routing 诊断 | 删除不存在、仍被 policy 锁定引用、无权限 |
| `provider.refresh_models` | provider instance / driver、刷新策略 | inventory 更新、metadata resolver 生效、`models.list` 反映新 revision | Provider 不可达、返回损坏 metadata、刷新失败不破坏旧 inventory |

## 2. 分层用例清单

本节把前文功能域拆成更接近实现任务的用例清单。用例 ID 可在实现时继续细化，但应保持前缀稳定。

### 2.1 L1 白盒单测

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

### 2.2 L2 AiccClient 黑盒测试

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

### 2.3 L3 本地 kRPC 黑盒测试

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l3_settings_reload_mock_*` | P0 | system_config 写入 Mock settings、reload、models.list |
| `l3_provider_admin_*` | P0 | provider.validate/add/delete/refresh_models 的 system_config 写入、reload 和回滚语义 |
| `l3_models_list_*` | P0 | `models.list` / `service.models.list` inventory、逻辑目录、health、legacy aliases 脱敏诊断 |
| `l3_quota_query_*` | P1 | `quota.query` 按 tenant、capability、method 返回预算状态和拒绝路径 |
| `l3_krpc_llm_chat_*` | P0 | 纯文本、多模态 content part、tool call、JSON schema |
| `l3_krpc_resource_*` | P0 | `url`、`base64`、`named_object` 输入和 artifact 输出 |
| `l3_krpc_stream_*` | P0 | Mock streaming chunks、task data progress、final summary |
| `l3_krpc_async_*` | P0 | image/audio/video 类异步 task 状态闭环 |
| `l3_krpc_usage_*` | P0 | usage event 写入和查询 |
| `l3_krpc_failover_*` | P0 | Provider timeout / 5xx / quota exhausted 后 failover |
| `l3_krpc_security_*` | P0 | local_only、跨用户访问拒绝、脱敏扫描 |
| `l3_krpc_legacy_*` | P1 | legacy alias、旧字段兼容或迁移提示 |

### 2.4 L4 Gateway 真实模型验收

| 用例族 | 优先级 | 覆盖点 |
|---|---|---|
| `l4_gateway_openai_<model>_complex_workflow` | P2 | OpenAI 每个支持模型的文本、JSON schema、tool call、usage、trace |
| `l4_gateway_claude_<model>_complex_workflow` | P2 | Claude 每个支持模型的多模态或 vision、tool use、usage、trace |
| `l4_gateway_gemini_<model>_complex_workflow` | P2 | Google Gemini 每个支持模型的多模态、safety / function call / operation 语义 |
| `l4_gateway_openrouter_<model>_complex_workflow` | P2 | OpenRouter 每个支持模型的 OpenAI-compatible 协议兼容、usage、trace |
| `l4_gateway_fal_<model>_media_workflow` | P2 | fal 每个支持模型的 image/video/audio 工具型异步任务和 artifact |
| `l4_gateway_sn_ai_provider_<model>_complex_workflow` | P2 | SN AI Provider 每个支持模型的无普通 API key 链路、usage、trace、provider 归因 |
| `l4_gateway_models_list` | P2 | 真实环境 inventory、逻辑目录和 Provider health 可诊断 |

L4 用例 ID 中的 `<model>` 必须使用稳定可读的 slug，由精确模型名归一化得到；报告中必须保留原始精确模型名。

## 3. 首批 P0 最小用例集

为避免第一轮实现范围过大，M0/M1 阶段先落地以下最小 P0 用例集。该集合不追求覆盖全部 method，而是优先打通协议、路由、Mock、任务、资源、usage、trace 和异常主链路。

### 3.1 M0 最小集

| case id | 层级 | 目标 |
|---|---|---|
| `l1_routing_exact_model_success` | L1 | 精确模型名解析成功 |
| `l1_routing_exact_model_no_fallback` | L1 | 精确模型不可用且未开启 fallback 时失败 |
| `l1_routing_logical_model_candidates` | L1 | 逻辑模型展开候选列表 |
| `l1_routing_parent_fallback_success` | L1 | parent fallback 生效 |
| `l1_routing_fallback_loop_rejected` | L1 | fallback 环路被拒绝 |
| `l1_scheduler_weight_priority` | L1 | 目录 item weight 优先级生效 |
| `l1_scheduler_profile_cost_first` | L1 | 同优先级候选按 cost profile 选择 |
| `l1_request_overlay_override_route` | L1 | request overlay 覆盖系统配置并改变最终物理路由 |
| `l1_request_overlay_stateless` | L1 | 不同 request overlay 互不污染，AICC 不保存 session config |
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

### 3.2 M1 最小集

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

