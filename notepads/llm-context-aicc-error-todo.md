# llm_context / xllm：AICC 调用失败的原因

> 来源：2026-10-07 DV 验证许愿格时，分析阶段失败，界面只显示 `aicc helper.llm_chat failed: task_id=…, event_ref=…`；真实原因只在 task-manager 的 `error_json` 中：`invalid_request: Claude Messages requires max_output_tokens or resolved max_tokens`。

## 已做（待 review）

- [x] `agent_tool/src/xllm.rs` 的 AICC 适配：`status = Failed` 时把响应里的 `error`（`AiccError`，AICC 在同步响应中已经返回）拼进错误文本：`aicc helper.llm_chat failed (invalid_request: …): task_id=…, event_ref=…`。验证：`cargo test -p agent_tool --lib xllm -- --test-threads=1`。

## 待确认

- [ ] 失败分类：现在一律 `ProviderFailure::Unknown`。可按 `AiccError.retriable` / `code` 映射为 `Transient` / `Permanent`，影响 llm_context 的重试行为，需要确认。
- [ ] 输出上限的默认值：`.llm_context` 不写 `max_tokens` 时，路由到 Claude 必然失败。Claude 编码器也接受 `resolved_parameters.max_tokens`，但 builtin 元数据没有给出默认值。由 AICC 按模型元数据给默认值，还是由每个宿主必须写 `max_tokens`（Jarvis 包与 aiworkspace 现在都显式写了），需要确认。

## 2026-10-22：额度与资源策略拒绝（AI Center Token 额度 PRD §7.4）

- [x] `llm_context`：`ProviderFailure::Refused(RefusalKind)`，`RefusalKind` = `QuotaExhausted` / `QuotaBudgetExceeded` / `ResourceNotPermitted` / `AttributionUnavailable`。不在 `infra_retry_safe` 之列：waist 与宿主都不自动重试。
- [x] `xllm`：AICC 这五个码以 kRPC `Detailed{code, data}` 上线；`provider_error_from_rpc` 先按 `AiccError::from_krpc_error` 的 `code` 映射（`quota_unavailable` → `Transient`），`status = Failed` 时按响应 `error.code` 映射；`classify_error` 给出 `kind` = `quota_exhausted` 等（`refusal_kind_name`），`recoverable: false`。
- [x] `xllm` buckyos Provider 新增 `on_behalf_of`（`.llm_context` 的 `provider.on_behalf_of`，`AiccOnBehalfOf`）：系统服务代用户调用时由 AICC 校验 RBAC（`obj://aicc/on_behalf_of`）。
- 宿主接入：OpenDAN Turn 失败记 `error.kind = "ai_quota"`，回复用 Agent 包的 `[outbound] ai_quota_limited`；许愿格映射为 `AI_QUOTA_*` 子码（`aiworkspace/server/src/wish/mod.rs`）。
- 验证：`cargo test -p llm_context -p agent_tool -p aiworkspace -p libopendan -p opendan`。
