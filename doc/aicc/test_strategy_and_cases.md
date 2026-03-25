# AICC 测试规划与模拟环境方案（cargo test）

## 1. 目标与发布门槛

本方案用于 AICC 模块发布前质量验收，核心目标：

1. 覆盖高风险语义：路由、fallback、长短任务边界、多租户隔离、错误码一致性
2. 覆盖会议明确高风险项：Streaming、协议兼容、多模态 URL/Base64 路径、调度组合策略
3. 用低成本模拟环境替代真实大模型调用，并补充正式环境脚本化验证
4. 形成可重复执行的 `cargo test` + 正式环境 smoke 用例体系

发布门槛建议：

- L0 Streaming 语义层：100% 通过（必须）
- L1 核心语义层：100% 通过（必须）
- L2 Provider 适配器协议层：100% 通过（必须）
- L3 协议规范层：100% 通过（必须）
- L4 稳定性与并发层：>= 90% 通过（允许登记已知风险后发布）
- L5 调度组合与复杂任务编排层：>= 80% 通过（允许登记已知风险后发布）
- L6 正式环境与发布前脚本层：100% 通过（必须）

---

## 2. 测试分层设计

### L0：Streaming 语义层（必须全绿）

覆盖以下语义：

- 协议声明支持 `stream=true` 时，`Started` 任务必须可轮询并看到增量输出
- 增量数据刷入 task data 时，必须单调追加，不得覆盖已确认分片
- 事件顺序必须满足：`Queued -> Started -> Final/Error/CancelRequested`
- `Started` 后不得触发 fallback 重试
- cancel 后增量停止，状态最终一致（Canceled 或 Failed，且错误码可机读）
- 跨租户轮询与读取必须拒绝

### L1：核心语义层（必须全绿）

覆盖以下语义：

- Router：硬过滤（capability / must_features / allow-deny / alias映射）
- 打分：成本/延迟/负载/错误率
- 执行：`Immediate` / `Started` / `Queued`
- fallback：仅启动阶段可重试错误可触发；`Started` 后必须停止重试
- 多租户：跨租户 cancel 拒绝
- 资源校验：base64 大小、mime 白名单、URL 合法性
- 错误码：`bad_request` / `no_provider_available` / `model_alias_not_mapped` / `provider_start_failed` / `resource_invalid`

### L2：Provider 协议层（必须全绿）

对 `openai.rs` / `gimini.rs` 的请求-响应语义做协议测试：

- 2xx 正常返回
- 4xx 不可重试错误
- 429/5xx 可重试错误
- 响应体缺失关键字段
- 超时/网络抖动
- 不支持 capability 的错误路径

### L3：协议规范层（必须全绿）

覆盖各 capability 的输入输出协议规范：

- **LLM（llm_router）**
  - `payload.text/messages/input_json/options` 组合规则
  - tool calling 字段合法性（`tool_specs`）
  - temperature/top_p/max_tokens 参数边界
- **Text2Image（text2image）**
  - prompt 来源优先级（text/messages/input_json/options）
  - 图片输出 artifact 引用格式（URL/Base64）
  - size/quality/style 参数映射
- **Voice2Text / Video2Text / Image2Text（voice2text / video2text / image2text）**
  - `ResourceRef::Base64`：mime 白名单 + 大小上限 + 可解码
  - `ResourceRef::Url`：scheme 必须存在 + URL 合法性
  - language/hotword 等配置参数
- **Text2Voice（text2voice）**
  - 文本输入长度限制
  - voice/model 配置参数格式
  - 音频输出格式（URL 引用）
- **通用协议规范**
  - 敏感字段不落日志（prompt/base64 原文）
  - `idempotency_key` 透传
  - `session_token` 与 tenant 绑定

### L4：稳定性层（建议）

- 并发 complete 的 task_id 唯一性
- Registry 热更新并发下 route 稳定性
- 指标更新（EWMA/error/in_flight）收敛与不越界

### L5：复杂任务编排层（建议）

- 任务拆解：LLM 生成 DAG 计划
- 串行执行：依赖步骤阻塞
- 并行执行：同 parallel_group 步骤并发
- 循环重规划：质量不达标时触发二次拆解
- fallback 执行：子任务失败时使用备选 alias

### L6：正式环境与发布前脚本层（必须全绿）

- 物理机环境部署（非虚拟机）可稳定运行
- 基于分配 URL 执行 smoke 脚本并产出日志
- 监控告警链路可触发且可恢复
- 缺陷记录包含现场上下文，可用于自动化复现
- `POST /kapi/aicc` 的 kRPC 调用链路可用（`complete/cancel/reload_settings`）
- `POST /kapi/system_config` 的配置变更链路可用（`sys_config_get/set/set_by_json_path`）
- `system_config` 更新后经 `reload_settings` 可在线生效且结果可回归验证

---

## 3. 模拟测试环境（低成本）

### 3.1 组件构成

1. `MockProvider`（脚本化）
   - 输入：预置 `VecDeque<Result<ProviderStartResult, ProviderError>>`
   - 输出：按序返回 `Immediate/Started/Queued/retryable/fatal`
   - 记录：start 调用次数、cancel 记录

2. `MockTaskMgrHandler`（内存任务管理）
   - 已存在基础实现，可继续扩展断言字段（status/progress/data/events）

3. `FakeResourceResolver`
   - 可配置成功、权限失败、解析失败、超时

4. `TaskEventSink`（内存事件收集）
   - 用于断言事件序列：Queued -> Started -> Final/Error/CancelRequested

5. `Fake HTTP Server`（用于 adapter 测试）
   - 返回可编排 HTTP 响应：200/400/429/503/无效JSON/超时
   - 不访问外部 API，无模型成本

### 3.2 环境原则

- 不依赖真实 OpenAI/Gemini
- 所有测试默认离线可跑
- 测试数据固定，避免随机波动
- 单元测试优先，集成测试补充

---

## 4. 典型测试用例清单（建议最小集）

### 4.0 Streaming 语义（8）

- `stream_01_started_then_poll_receives_incremental_chunks`：输入 `stream=true` 且 provider 返回 `Started` 的请求，经过轮询流程，最终应输出按时间到达的增量片段。
- `stream_02_incremental_chunks_are_append_only`：输入含多分片流式结果的任务，经过 task data 写入流程，最终应输出仅追加不覆盖的分片序列。
- `stream_03_event_sequence_order_is_stable`：输入可触发 `Queued/Started/Final` 的流式任务，经过事件管道处理，最终应输出固定顺序事件序列。
- `stream_04_started_must_not_fallback`：输入首个 provider 已 `Started` 的请求，经过 fallback 判定流程，最终应输出“不再切换候选”的路由结果。
- `stream_05_cancel_stops_incremental_output`：输入运行中流式任务并发送 cancel，经过取消与事件收敛流程，最终应输出分片停止且状态收敛。
- `stream_06_cross_tenant_poll_rejected`：输入跨租户轮询请求，经过租户鉴权流程，最终应输出拒绝错误（无敏感数据泄露）。
- `stream_07_stream_timeout_classified_retryable_before_started`：输入启动前网络超时场景，经过错误分类流程，最终应输出可重试错误并允许回退。
- `stream_08_stream_final_snapshot_consistent_with_chunks`：输入存在最终汇总的流式任务，经过 final 合并流程，最终应输出与已接收分片一致的最终快照。

### 4.1 路由与映射（8）

- `route_01_mapped_primary_with_fallback`：输入已映射 alias 与多个候选 provider，经过路由打分流程，最终应输出主备顺序正确的决策。
- `route_02_alias_unmapped_returns_model_alias_not_mapped`：输入未映射 alias，经过模型映射流程，最终应输出 `model_alias_not_mapped`。
- `route_03_must_features_filtered_out`：输入 `must_features` 无候选满足，经过能力过滤流程，最终应输出 `no_provider_available`。
- `route_04_tenant_allow_provider_types`：输入租户 allow 白名单策略，经过租户策略过滤流程，最终应输出仅命中白名单 provider。
- `route_05_tenant_deny_provider_types`：输入租户 deny 黑名单策略，经过租户策略过滤流程，最终应输出避开黑名单 provider。
- `route_06_max_cost_filter`：输入低成本上限请求，经过成本约束过滤流程，最终应输出超预算候选被剔除。
- `route_07_max_latency_filter`：输入低时延上限请求，经过时延约束过滤流程，最终应输出超时延候选被剔除。
- `route_08_tenant_mapping_override_global`：输入租户级别名映射，经过映射解析流程，最终应输出租户映射覆盖全局映射。

### 4.2 启动与 fallback（6）

- `start_01_retryable_error_then_fallback_success`：输入首选启动返回可重试错误，经过回退流程，最终应输出候选切换成功并进入 `running/succeeded`。
- `start_02_fatal_error_no_fallback`：输入首选启动返回不可重试错误，经过错误分流流程，最终应输出直接失败且不回退。
- `start_03_started_must_stop_fallback`：输入首选已返回 `Started`，经过状态机流程，最终应输出停止回退并绑定当前实例。
- `start_04_queued_no_fallback`：输入首选返回 `Queued`，经过状态机流程，最终应输出保持排队语义且不回退。
- `start_05_all_candidates_failed_provider_start_failed`：输入全部候选启动失败，经过聚合错误流程，最终应输出 `provider_start_failed`。
- `start_06_fallback_respects_limit`：输入可无限回退风险场景，经过 `fallback_limit` 控制流程，最终应输出在上限内终止尝试。

### 4.3 生命周期与事件（4）

- `task_01_immediate_persists_completed`：输入 provider 立即完成响应，经过落库流程，最终应输出任务状态 `Completed`。
- `task_02_started_persists_running_and_binding`：输入 provider 返回 `Started`，经过实例绑定流程，最终应输出 `Running` 且可正确 cancel。
- `task_03_queued_persists_pending_and_position`：输入 provider 返回 `Queued(position)`，经过持久化流程，最终应输出 `Pending` 且保留队列位置。
- `task_04_emit_error_event_with_code`：输入可触发失败路径的请求，经过事件发射流程，最终应输出带机读 `code` 的 Error 事件。

### 4.4 多租户与安全（4）

- `sec_01_cancel_reject_cross_tenant`：输入跨租户 cancel 请求，经过权限校验流程，最终应输出拒绝。
- `sec_02_cancel_accept_same_tenant`：输入同租户 cancel 请求，经过权限校验流程，最终应输出接受或可解释的受控拒绝。
- `sec_03_resource_invalid_from_resolver`：输入资源解析失败请求，经过资源校验流程，最终应输出 `resource_invalid`。
- `sec_04_base64_policy_enforced`：输入违规 Base64 资源，经过 MIME/大小策略流程，最终应输出拒绝。

### 4.5 可观测（2）

- `obs_01_error_code_mapping_consistent`：输入多类错误场景，经过错误归一化流程，最终应输出稳定一致的错误码映射。
- `obs_02_log_redaction_no_prompt_or_base64`：输入含敏感 prompt/base64 的请求，经过日志脱敏流程，最终应输出无原文泄露日志。

### 4.6 并发稳定性（2）

- `conc_01_task_id_uniqueness_under_concurrency`：输入并发 complete 请求，经过任务创建流程，最终应输出唯一 `task_id` 集合。
- `conc_02_registry_hot_update_route_consistency`：输入注册表热更新与路由并发场景，经过读写并发流程，最终应输出路由稳定可用。

### 4.7 Adapter 协议测试（OpenAI/Gimini 各 6）

- `adapter_xx_01_http_200_success`：输入 provider 返回 200 合法响应，经过 adapter 解析流程，最终应输出成功启动。
- `adapter_xx_02_http_429_retryable`：输入 provider 返回 429，经过错误分类流程，最终应输出可重试错误。
- `adapter_xx_03_http_503_retryable`：输入 provider 返回 503，经过错误分类流程，最终应输出可重试错误。
- `adapter_xx_04_http_400_fatal`：输入 provider 返回 400，经过错误分类流程，最终应输出不可重试错误。
- `adapter_xx_05_invalid_json_fatal`：输入 200 但 body 非法 JSON，经过解析流程，最终应输出不可重试错误。
- `adapter_xx_06_timeout_or_network_error_classified`：输入网络超时/连接失败，经过错误分类流程，最终应输出可重试错误。

### 4.8 协议规范层 - Base64 资源（8）

- `proto_b64_01_image_valid_png`：输入合法 PNG Base64，经过资源校验流程，最终应输出通过。
- `proto_b64_02_image_valid_jpeg`：输入合法 JPEG Base64，经过资源校验流程，最终应输出通过。
- `proto_b64_03_audio_valid_wav`：输入合法 WAV Base64，经过资源校验流程，最终应输出通过。
- `proto_b64_04_audio_valid_mp3`：输入合法 MP3 Base64，经过资源校验流程，最终应输出通过。
- `proto_b64_05_video_valid_mp4`：输入合法 MP4 Base64，经过资源校验流程，最终应输出通过。
- `proto_b64_06_invalid_mime_rejected`：输入不在白名单的 MIME，经过资源校验流程，最终应输出 `resource_invalid`。
- `proto_b64_07_size_limit_exceeded_rejected`：输入超出大小上限的 Base64，经过资源校验流程，最终应输出 `resource_invalid`。
- `proto_b64_08_malformed_base64_rejected`：输入不可解码 Base64，经过资源校验流程，最终应输出 `resource_invalid`。

### 4.9 协议规范层 - URL 资源（6）

- `proto_url_01_https_valid`：输入合法 HTTPS URL，经过 URL 校验流程，最终应输出通过。
- `proto_url_02_http_allowed`：输入策略允许下的 HTTP URL，经过 URL 校验流程，最终应输出通过。
- `proto_url_03_missing_scheme_rejected`：输入缺失 scheme 的 URL，经过 URL 校验流程，最终应输出 `resource_invalid`。
- `proto_url_04_empty_url_rejected`：输入空 URL，经过 URL 校验流程，最终应输出 `resource_invalid`。
- `proto_url_05_invalid_url_format_rejected`：输入格式非法 URL，经过 URL 解析流程，最终应输出 `resource_invalid`。
- `proto_url_06_resource_unreachable_simulated`：输入不可达 URL，经过资源解析流程，最终应输出 `resource_invalid`。

### 4.10 协议规范层 - 各模型特定格式（10）

**LLM 特定：**
- `proto_llm_01_messages_format_valid`：输入标准 messages 结构，经过请求构建流程，最终应输出可被 provider 正确接受的报文。
- `proto_llm_02_input_json_format_valid`：输入 `input_json` 结构，经过协议转换流程，最终应输出字段映射正确的请求体。
- `proto_llm_03_tool_specs_format_valid`：输入 `tool_specs` 配置，经过工具参数转换流程，最终应输出合法 tool calling 字段。
- `proto_llm_04_temperature_boundary_valid`：输入边界温度参数，经过参数约束流程，最终应输出边界值处理正确结果。

**Text2Image 特定：**
- `proto_t2i_01_prompt_from_text`：输入 `payload.text`，经过 prompt 提取优先级流程，最终应输出以 text 为 prompt 的请求。
- `proto_t2i_02_prompt_from_messages`：输入 `messages` 且无 text，经过 prompt 回退流程，最终应输出以 messages 内容为 prompt。
- `proto_t2i_03_prompt_from_options`：输入 `options.prompt` 且无 text/messages，经过 prompt 回退流程，最终应输出以 options.prompt 为 prompt。
- `proto_t2i_04_artifact_url_format`：输入文生图成功响应，经过产物转换流程，最终应输出 URL 型 artifact。

**Voice2Text/Video2Text 特定：**
- `proto_v2t_01_language_param_respected`：输入带 language 参数请求，经过参数透传流程，最终应输出 language 被正确保留。
- `proto_v2t_02_hotword_param_respected`：输入带 hotword 参数请求，经过参数透传流程，最终应输出 hotword 被正确保留。

**Text2Voice 特定：**
- `proto_t2v_01_voice_param_format_valid`：输入语音角色与模型参数，经过参数校验流程，最终应输出合法 TTS 请求。
- `proto_t2v_02_output_artifact_url_format`：输入 TTS 成功响应，经过结果归一化流程，最终应输出 URL 型音频 artifact。

### 4.11 协议规范层 - 混合资源模式（4）

- `proto_mix_01_url_and_base64_in_same_task`：输入同一任务中 URL+Base64 混合资源，经过统一校验流程，最终应输出可解析且不冲突。
- `proto_mix_02_multiple_images_mixed`：输入多张混合模式图片，经过批量资源处理流程，最终应输出顺序与数量一致结果。
- `proto_mix_03_workflow_mixed_resource_modes`：输入工作流中多步骤混合资源，经过步骤编排流程，最终应输出跨步骤可用资源引用。
- `proto_mix_04_cross_capability_resource_passthrough`：输入跨 capability 资源传递场景，经过协议适配流程，最终应输出引用不丢失不篡改。

### 4.12 协议规范层 - 安全与脱敏（4）

- `proto_sec_01_no_base64_in_logs`：输入含 Base64 原文请求，经过日志输出流程，最终应输出不包含原文日志。
- `proto_sec_02_no_prompt_in_logs`：输入含敏感 prompt 请求，经过日志输出流程，最终应输出不包含 prompt 原文日志。
- `proto_sec_03_no_artifact_bytes_in_events`：输入含二进制产物任务，经过事件发射流程，最终应输出事件内无原始字节。
- `proto_sec_04_idempotency_key_preserved`：输入含 `idempotency_key` 请求，经过任务落库流程，最终应输出 key 透传一致。

### 4.13 复杂任务编排（8）

- `workflow_01_plan_generates_valid_dag`：输入复杂目标描述，经过 planner 拆解流程，最终应输出结构合法 DAG。
- `workflow_02_serial_dependency_blocks_until_ready`：输入含依赖链任务，经过调度流程，最终应输出前置未完成时后继不执行。
- `workflow_03_parallel_group_executes_concurrently`：输入并行组任务，经过并发调度流程，最终应输出同组步骤并行执行。
- `workflow_04_replan_triggered_on_quality_threshold`：输入质量低于阈值结果，经过重规划流程，最终应输出触发 replan。
- `workflow_05_retryable_subtask_uses_fallback_alias`：输入子任务可重试失败，经过回退流程，最终应输出使用 fallback alias 成功续跑。
- `workflow_06_started_subtask_never_retries_cross_instance`：输入子任务已 `Started` 后失败场景，经过状态机流程，最终应输出不跨实例重试。
- `workflow_07_each_step_routes_to_correct_capability`：输入多 capability 步骤 DAG，经过逐步路由流程，最终应输出每步命中正确能力。
- `workflow_08_event_sequence_reflects_dag_structure`：输入有串并混合的 DAG，经过事件汇聚流程，最终应输出事件序列与 DAG 拓扑一致。

### 4.14 调度组合策略（8）

- `sched_01_effect_priority_prefers_higher_quality_when_budget_allows`：输入预算充足且多候选质量差异场景，经过策略打分流程，最终应输出优先高质量 provider。
- `sched_02_cost_priority_prefers_lower_cost_under_same_capability`：输入同能力多候选成本差异场景，经过策略打分流程，最终应输出低成本 provider。
- `sched_03_free_quota_priority_prefers_quota_provider_first`：输入有免费额度候选场景，经过策略打分流程，最终应输出优先免费额度 provider。
- `sched_04_agent_tier_policy_routes_to_expected_provider_group`：输入 agent 分层策略，经过路由策略流程，最终应输出命中预期 provider 组。
- `sched_05_master_feature_local_required_filters_non_local`：输入强制 `local` 主特性请求，经过特性过滤流程，最终应输出非本地候选被剔除。
- `sched_06_optional_features_do_not_break_primary_selection`：输入可选特性组合请求，经过路由筛选流程，最终应输出主选择稳定不被无关可选项破坏。
- `sched_07_multi_provider_same_model_priority_stable`：输入同模型多 provider 场景，经过重复调度流程，最终应输出稳定优先级顺序。
- `sched_08_tenant_policy_overrides_global_strategy`：输入租户策略与全局策略冲突场景，经过策略合并流程，最终应输出租户策略优先。

### 4.15 正式环境脚本化 smoke（6）

- `smoke_01_complete_basic_succeeds_on_assigned_url`：输入分配 URL 的最小 complete 请求，经过网关与服务链路，最终应输出成功或运行中状态。
- `smoke_02_json_output_path_succeeds_on_assigned_url`：输入 `json_output` 请求，经过 provider 处理流程，最终应输出可解析 JSON 结果。
- `smoke_03_cancel_endpoint_reachable_on_assigned_url`：输入 cancel 请求，经过远程接口流程，最终应输出 `accepted` 布尔结果。
- `smoke_04_stream_poll_basic_path_on_assigned_url`：输入流式任务并轮询，经过线上任务链路，最终应输出可观测增量或可解释状态。
- `smoke_05_monitor_alarm_trigger_and_recovery`：输入可触发告警的受控异常，经过监控流程，最终应输出告警触发与恢复记录。
- `smoke_06_bug_context_capture_template_complete`：输入故障样例请求，经过缺陷记录流程，最终应输出完整现场信息模板。

### 4.16 kRPC + gateway 远程调用链路（12）

- `krpc_01_gateway_complete_minimal_llm_success`：输入最小 `method=complete` kRPC 报文，经过 gateway `/kapi/aicc` 转发流程，最终应输出合法 result 结构。
- `krpc_02_gateway_complete_with_sys_seq_token_trace_success`：输入完整 `sys=[seq,token,trace]` 报文，经过 kRPC 解析流程，最终应输出成功且 trace 可追踪。
- `krpc_03_gateway_complete_without_token_with_trace_uses_null_placeholder`：输入 `sys=[seq,null,trace]` 报文，经过 kRPC 解析流程，最终应输出请求被正确受理。
- `krpc_04_gateway_complete_invalid_sys_shape_returns_bad_request`：输入非法 `sys` 结构报文，经过参数校验流程，最终应输出 `bad_request`。
- `krpc_05_gateway_cancel_cross_tenant_rejected`：输入跨租户 cancel 报文，经过租户鉴权流程，最终应输出拒绝。
- `krpc_06_gateway_cancel_same_tenant_accepted_or_graceful_false`：输入同租户 cancel 报文，经过 cancel 流程，最终应输出 `accepted=true` 或受控 `false`。
- `krpc_07_gateway_reload_settings_aliases_compatible`：输入 `reload_settings/service.reload_settings/reaload_settings` 报文，经过方法兼容流程，最终应输出热加载成功。
- `cfg_01_sys_config_get_aicc_settings_success`：输入 `sys_config_get` 请求，经过 `/kapi/system_config` 流程，最终应输出 `services/aicc/settings` 当前值。
- `cfg_02_sys_config_set_full_value_then_reload_effective`：输入 `sys_config_set` 全量配置，经过写入+`reload_settings` 流程，最终应输出新配置生效可被 complete 验证。
- `cfg_03_sys_config_set_by_json_path_partial_update_then_reload_effective`：输入 `sys_config_set_by_json_path` 局部更新，经过局部写入+热加载流程，最终应输出增量配置生效。
- `cfg_04_sys_config_write_without_permission_rejected`：输入无权限 token 的配置写请求，经过 RBAC 流程，最终应输出拒绝。
- `cfg_05_sys_config_value_not_json_string_rejected`：输入非 JSON 字符串 `value` 的配置写请求，经过参数校验流程，最终应输出请求失败且错误可机读。

### 4.17 协议资源一致性补充（10）

- `proto_res_01_named_object_passthrough_preserved`：输入 `ResourceRef::NamedObject`，经过协议转换流程，最终应输出对象引用不丢失、不改写。
- `proto_res_02_cyfs_url_scheme_policy_allowed`：输入 `cyfs://` 资源 URL，经过 URL 策略校验流程，最终应输出在策略允许下通过。
- `proto_res_03_cyfs_url_scheme_policy_rejected`：输入策略不允许的 `cyfs://` 资源 URL，经过 URL 策略校验流程，最终应输出 `resource_invalid`。
- `proto_res_04_named_object_requires_resolver_when_provider_needs_bytes`：输入 `NamedObject` 且下游能力需要字节流，经过资源解析流程，最终应输出未配置 resolver 时拒绝并给出可机读错误。
- `proto_res_05_equivalent_resource_semantics_base64_url_named_object`：输入同一资源的 Base64/URL/NamedObject 三种表示，经过统一解析流程，最终应输出语义等价结果。
- `proto_res_06_base64_to_url_translation_for_url_only_provider`：输入 Base64 到仅支持 URL 的 provider，经过资源桥接流程，最终应输出成功转换或明确失败原因。
- `proto_res_07_provider_base64_unsupported_error_classified`：输入 Base64 到不支持 Base64 的 provider，经过 provider 适配流程，最终应输出稳定错误分类且不误标为路由错误。
- `proto_res_08_named_object_and_url_mixed_order_stable`：输入 NamedObject+URL 混合资源列表，经过批量资源处理流程，最终应输出顺序与数量稳定一致。
- `proto_res_09_mime_hint_consistency_after_translation`：输入带 `mime_hint` 的 URL 资源，经过协议转换流程，最终应输出 MIME 提示不丢失或有可解释降级。
- `proto_res_10_no_sensitive_resource_literal_in_provider_logs`：输入含敏感 URL 签名/对象标识/Base64 的请求，经过日志输出流程，最终应输出脱敏日志且无原文泄露。

---

## 5. 执行命令建议（cargo test）

在 `src/` 目录执行：

```bash
# 全量测试
cargo test -p aicc -- --test-threads=1

# Streaming 语义层
cargo test -p aicc stream_

# 按组执行
cargo test -p aicc route_
cargo test -p aicc start_
cargo test -p aicc task_
cargo test -p aicc sec_
cargo test -p aicc obs_
cargo test -p aicc conc_
cargo test -p aicc adapter_openai_
cargo test -p aicc adapter_gimini_

# 协议规范层
cargo test -p aicc proto_b64_
cargo test -p aicc proto_url_
cargo test -p aicc proto_llm_
cargo test -p aicc proto_t2i_
cargo test -p aicc proto_v2t_
cargo test -p aicc proto_t2v_
cargo test -p aicc proto_mix_
cargo test -p aicc proto_sec_
cargo test -p aicc proto_res_

# 复杂任务编排
cargo test -p aicc workflow_

# 调度组合策略
cargo test -p aicc sched_
```

说明：

- 初始阶段建议 `--test-threads=1` 保证稳定
- 并发类测试可单独启用多线程执行
- 协议层测试建议在 MockProvider 中增加"协议校验器"模式

正式环境 smoke（物理机）建议：

```bash
cd src/frame/aicc
python3 test_llm.py
```

可选环境变量：

- `AICC_URL`：正式环境 URL（例如 `http://<ip>:4040/kapi/aicc`）
- `AICC_MODEL_ALIAS`：用于 smoke 的默认模型别名
- `AICC_RPC_TOKEN`：鉴权 token
- `AICC_TIMEOUT_SECONDS`：超时时间

kRPC + gateway 链路建议（远程）：

- 网关入口：`POST /kapi/aicc`
- 配置入口：`POST /kapi/system_config`
- 关键校验点：
  - `sys` 数组语义：`[seq, token?, trace_id?]`
  - `sys_config` 写入后必须显式调用 `reload_settings`
  - `reload_settings` 后以一次 `complete` 做配置生效回归

---

## 6. 与其他模块的协作依赖（需联调验收）

1. TaskMgr
   - 任务状态机语义一致（Pending/Running/Completed/Canceled/Failed）
   - task data 字段持久化一致

2. 事件通道（TaskMgr/MsgQueue）
   - 事件 schema 和顺序语义对齐（Started/Queued/Final/Error/CancelRequested）

3. IAM/Auth
   - tenant 身份来源可信，跨租户校验可生效

4. Resource 服务
   - 资源权限校验与审计落地

5. 配置中心
   - alias 映射与策略更新原子性，避免漂移窗口

---

## 7. PR 验收证据要求（发布卡点）

提交实现时需附：

- 测试清单与风险映射表（case -> risk）
- `cargo test -p aicc` 全量通过结果
- 安全场景通过证据：
  - 跨租户 cancel 拒绝
  - base64 超限/非法 mime 拒绝
- 协议测试证据：
  - openai/gimini 各至少 6 条
- 失败路径证据：
  - `model_alias_not_mapped`
  - `provider_start_failed`
- Streaming 证据：
  - `Started` 后可轮询增量输出
  - cancel 后增量停止且状态收敛
  - 事件顺序与 task data 快照一致
- kRPC + gateway 证据：
  - `/kapi/aicc` 远程 `complete/cancel/reload_settings` 全链路成功
  - `sys` 结构与可选 token/trace 组合均覆盖
  - 错误路径（非法 `sys`、跨租户 cancel）可稳定复现
- system_config 证据：
  - `sys_config_get/set/set_by_json_path` 覆盖并具备请求-响应记录
  - 配置变更 + `reload_settings` + `complete` 回归三段证据齐全
- 正式环境证据：
  - 物理机部署记录（设备、系统、网络）
  - 基于分配 URL 的脚本化 smoke 报告
  - 监控告警触发与恢复记录

无证据按未实现处理，不建议发布。

---

## 8. 当前代码基线观察（便于增量实现）

当前基线（2026-03）：

- `src/frame/aicc/tests/core_semantics_tests.rs` 已覆盖 route/start/task/sec/obs/conc 的主要语义用例
- `src/frame/aicc/tests/adapter_protocol_tests.rs` 已覆盖 OpenAI/Gimini 各 6 条 adapter 场景与部分 T2I 协议场景
- `src/frame/aicc/src/openai_protocol.rs`、`src/frame/aicc/src/claude_protocol.rs`、`src/frame/aicc/src/openai.rs` 已有协议转换与参数映射单测

当前相对本方案的关键缺口（优先补齐）：

- Streaming 用例簇（`stream_*`）尚未成体系
- L3 缺口：`proto_llm_*`、`proto_v2t_*`、`proto_t2v_*`、`proto_mix_*`、`proto_sec_01~03`
- L5 缺口：`workflow_*` 与 `sched_*` 组合策略用例
- 资源语义缺口：`proto_res_*`（NamedObject/cyfs:///跨模式等价与转换）用例
- 正式环境 smoke 与监控告警验收尚未并入统一发布 gate

为避免“总清单有、实现缺口看不见”，补充一组按当前实现差异追踪的目标用例（本轮新增）：

- Streaming：`stream_01~stream_08`
- URL 规范：`proto_url_05_invalid_url_format_rejected`
- V2T 参数透传：`proto_v2t_01`、`proto_v2t_02`
- T2V 协议：`proto_t2v_01`、`proto_t2v_02`
- 混合资源：`proto_mix_01~proto_mix_04`
- 安全脱敏：`proto_sec_03_no_artifact_bytes_in_events`
- LLM 格式：`proto_llm_01~proto_llm_04`
- 调度策略：`sched_01~sched_08`
- 编排策略：`workflow_01~workflow_08`
- kRPC/gateway：`krpc_01~krpc_07`
- system_config：`cfg_01~cfg_05`
- 正式环境 smoke：`smoke_04~smoke_06`（`smoke_01~03` 已有脚本基础）

建议在当前基础上优先补齐 `L0/L3/L6`，再完善 `L5`。

---

## 9. 复杂任务规划提示词示例（生产可用版）

### 9.1 Planner 系统提示词

```
你是任务编排规划器。请把用户目标拆解为可执行子任务图（DAG），并严格输出 JSON。

必须遵守：
1. 每个步骤必须包含 id、title、capability、model_alias、depends_on、parallel_group、acceptance_criteria。
2. 仅允许以下 capability：
   - llm_router（文本生成/分析）
   - text2image（文生图）
   - text2voice（文生语音）
   - image2text（图像识别）
   - voice2text（语音转写）
   - video2text（视频理解）
3. 能并行的步骤必须放同一 parallel_group；有依赖的步骤必须在 depends_on 声明目标步骤的 id。
4. 若存在高风险步骤，给出 fallback_alias 和 retry_policy。
5. 若任务可能失败或质量不足，必须给出 replan_trigger 条件。
6. 若涉及二进制资源，明确指定资源传递方式：url 或 base64。
7. 输出仅 JSON，不要解释文字。
```

### 9.2 Planner 输出 JSON Schema

```json
{
  "plan_id": "string",
  "goal": "string",
  "max_replan_rounds": 3,
  "steps": [
    {
      "id": "step_1",
      "title": "string",
      "capability": "llm_router | text2image | text2voice | image2text | voice2text | video2text",
      "model_alias": "string",
      "must_features": ["string"],
      "inputs": {
        "key": "${step_x.output.key}"
      },
      "depends_on": ["step_x"],
      "parallel_group": "group_a",
      "resource_mode": "url | base64",
      "retry_policy": {
        "max_retries": 2,
        "backoff_ms": 1000,
        "fallback_alias": "string"
      },
      "acceptance_criteria": [
        "score >= 0.8",
        "output contains 'xyz'"
      ],
      "replan_trigger": {
        "condition": "score < 0.6",
        "target_step": "step_1"
      }
    }
  ],
  "global_acceptance": [
    "all_steps_completed",
    "total_cost < 1.0"
  ]
}
```

### 9.3 典型复杂任务示例：产品发布多媒体包

**输入：**
- 产品 PRD 文档（文本）
- 演示视频文件（video）
- 采访音频文件（audio）
- 截图素材（images）

**目标产出：**
- 发布文案（LLM）
- 1 张主视觉海报（T2I）
- 30 秒旁白（T2Voice）
- FAQ（LLM）
- 视频要点摘要（V2T + LLM）

**拆解后的 DAG：**

| Step | Capability | Model Alias | Mode | Depends On | Parallel Group |
|------|------------|-------------|------|------------|----------------|
| S1 | llm_router | llm.plan.default | - | - | - |
| S2 | video2text | v2t.default | url | - | group_input |
| S3 | voice2text | asr.default | base64 | - | group_input |
| S4 | image2text | i2t.default | url | - | group_input |
| S5 | llm_router | llm.default | - | S2,S3,S4 | - |
| S6 | text2image | t2i.default | - | S5 | - |
| S7 | text2voice | t2v.default | - | S5 | - |
| S8 | llm_router | llm.default | - | S5 | group_qa |
| S9 | llm_router | llm.qa.default | - | S8 | - |

**特点：**
- 并行：S2/S3/S4 同时处理输入媒体
- 串行：S5 依赖所有输入处理完成
- 并行：S6/S7/S8 依赖 S5 结果
- 循环：S8 质量评分 < 0.6 时触发 replan回到 S5

---

## 10. 模拟环境增强：协议校验器

### 10.1 SimProvider 协议校验模式

```rust
enum ProtocolMode {
    Normal,                    // 正常执行
    ValidateOnly,              // 仅校验输入协议，不真正执行
    InjectError(ProviderError), // 注入指定错误
}

struct SimProvider {
    // ... 现有字段 ...
    protocol_mode: ProtocolMode,
    input_validator: Option<Box<dyn Fn(&CompleteRequest) -> Result<(), ProviderError>>>,
}
```

### 10.2 协议校验器示例

```rust
// Base64 校验器
fn base64_validator(mime_whitelist: &[&str], max_bytes: usize) -> impl Fn(&CompleteRequest) -> Result<(), ProviderError> {
    move |req| {
        for resource in &req.payload.resources {
            match resource {
                ResourceRef::Base64 { mime, data_base64 } => {
                    if !mime_whitelist.contains(&mime.as_str()) {
                        return Err(ProviderError::fatal(format!("mime '{}' not allowed", mime)));
                    }
                    let decoded = general_purpose::STANDARD.decode(data_base64)
                        .map_err(|_| ProviderError::fatal("invalid base64"))?;
                    if decoded.len() > max_bytes {
                        return Err(ProviderError::fatal(format!("size {} exceeds limit {}", decoded.len(), max_bytes)));
                    }
                }
                ResourceRef::Url { url, .. } => {
                    if !url.contains("://") {
                        return Err(ProviderError::fatal("url must contain scheme"));
                    }
                }
                // ...
            }
        }
        Ok(())
    }
}
```

### 10.3 模拟资源解析器

```rust
enum ResourceResolverMode {
    Success,
    PermissionDenied,
    NotFound,
    Timeout,
    Malformed,
}

struct FakeResourceResolver {
    mode: ResourceResolverMode,
    // 模拟资源映射表
    resource_map: HashMap<String, Vec<u8>>,
}
```

### 10.4 模拟存储（用于 artifact URL）

```rust
struct FakeArtifactStorage {
    artifacts: Mutex<HashMap<String, Vec<u8>>>,
}

impl FakeArtifactStorage {
    fn store(&self, data: Vec<u8>, mime: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.artifacts.lock().unwrap().insert(id.clone(), data);
        format!("sim://artifact/{}", id)
    }

    fn get(&self, url: &str) -> Option<Vec<u8>> {
        url.strip_prefix("sim://artifact/")
            .and_then(|id| self.artifacts.lock().unwrap().get(id).cloned())
    }
}
```

---

## 11. PR 验收证据要求（发布卡点）- 更新版

提交实现时需附：

- 测试清单与风险映射表（case -> risk）
- `cargo test -p aicc` 全量通过结果
- 安全场景通过证据：
  - 跨租户 cancel 拒绝
  - base64 超限/非法 mime 拒绝
  - URL 无 scheme 拒绝
- 协议测试证据：
  - openai/gimini 各至少 6 条
  - Base64 协议测试至少 8 条
  - URL 协议测试至少 6 条
  - 各模型特定格式测试至少 10 条
  - Streaming 协议测试至少 8 条
- 失败路径证据：
  - `model_alias_not_mapped`
  - `provider_start_failed`
  - `resource_invalid` 各场景
- 复杂任务编排证据（如实现）：
  - DAG 生成与解析正常
  - 并行步骤确实并发执行
  - 串行步骤正确阻塞
  - 循环重规划触发正常
- 正式环境发布证据：
  - 物理机 smoke 全量通过
  - 监控告警演练通过
  - Bug 现场信息模板完整（请求参数、租户、trace_id、provider、错误码、日志片段）
  - kRPC + gateway 远程调用链路通过（含 system_config 更新与 reload 生效）

无证据按未实现处理，不建议发布。

---

## 12. 本次修改内容概要

1. 将会议纪要中的关键风险纳入测试分层：新增 `L0 Streaming` 与 `L6 正式环境`。
2. 扩展测试用例清单：新增 `stream_*`、`sched_*`、`smoke_*` 三组用例。
3. 补充执行与验收闭环：在 `cargo test` 外增加物理机 `test_llm.py` smoke 建议与环境变量说明。
4. 更新 PR 验收标准：新增 Streaming 证据与正式环境发布证据要求。
5. 修正“约 7 条基础单测”的过时描述，改为当前基线与缺口列表，便于后续增量补齐。
6. 新增 kRPC + gateway 远程调用与 system_config 在线更新链路测试方案与验收证据。
