# AICC 脱敏与诊断组件

定义安全脱敏验收、失败分类、诊断字段、报告扫描和敏感信息泄露阻断要求。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 安全与脱敏验收

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

## 2. 失败分类与诊断信息

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

