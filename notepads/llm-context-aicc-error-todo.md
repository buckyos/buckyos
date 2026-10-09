# llm_context / xllm：AICC 调用失败的原因

> 来源：2026-10-07 DV 验证许愿格时，分析阶段失败，界面只显示 `aicc helper.llm_chat failed: task_id=…, event_ref=…`；真实原因只在 task-manager 的 `error_json` 中：`invalid_request: Claude Messages requires max_output_tokens or resolved max_tokens`。

## 已做（待 review）

- [x] `agent_tool/src/xllm.rs` 的 AICC 适配：`status = Failed` 时把响应里的 `error`（`AiccError`，AICC 在同步响应中已经返回）拼进错误文本：`aicc helper.llm_chat failed (invalid_request: …): task_id=…, event_ref=…`。验证：`cargo test -p agent_tool --lib xllm -- --test-threads=1`。

## 待确认

- [ ] 失败分类：现在一律 `ProviderFailure::Unknown`。可按 `AiccError.retriable` / `code` 映射为 `Transient` / `Permanent`，影响 llm_context 的重试行为，需要确认。
- [ ] 输出上限的默认值：`.llm_context` 不写 `max_tokens` 时，路由到 Claude 必然失败。Claude 编码器也接受 `resolved_parameters.max_tokens`，但 builtin 元数据没有给出默认值。由 AICC 按模型元数据给默认值，还是由每个宿主必须写 `max_tokens`（Jarvis 包与 aiworkspace 现在都显式写了），需要确认。
