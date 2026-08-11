# AICC L3 配置 Reload 与管理用例

定义 system_config 写入、reload_settings、models.list、provider 管理、usage/quota 查询和非法配置回滚。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 配置与重载验收

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
7. `provider.validate` 只做校验和脱敏诊断，不写入 system_config。
8. `provider.add` 写入后 reload，`models.list` / `provider.list` / `provider.health` 可见，且路由可命中新增 Provider。
9. `provider.refresh_models` 更新 inventory revision 后，`models.list` 中 exact model、`api_types`、`logical_mounts` 和 metadata resolver 结果同步变化。
10. `provider.delete` 后 reload，候选和 provider list 中删除目标消失；若仍被 locked policy 引用，必须返回明确错误或诊断。

## 2. Usage Log 验收

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

## 3. 安全与脱敏验收

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

## 4. 分层用例清单

本节把前文功能域拆成更接近实现任务的用例清单。用例 ID 可在实现时继续细化，但应保持前缀稳定。

### 4.1 L1 白盒单测

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

### 4.2 L2 AiccClient 黑盒测试

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

### 4.3 L3 本地 kRPC 黑盒测试

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

### 4.4 L4 Gateway 真实模型验收

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

