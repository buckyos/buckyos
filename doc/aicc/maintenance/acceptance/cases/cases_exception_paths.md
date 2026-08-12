# AICC 异常路径用例

定义路由、Provider、Task、Resource、Usage、配置、安全和真实模型 gateway 的异常路径覆盖。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 异常路径

必测异常：

| 类别 | 场景 |
|---|---|
| 路由 | 非法模型名、模型不存在、无候选、策略拒绝、fallback loop、logical tree loop、精确模型不可用 |
| 策略 | `local_only` 无本地候选、预算超限、feature unsupported、context too long、provider 被禁用 |
| Provider | 401/403、429、5xx、timeout、quota exhausted、malformed response、missing usage、unsupported media type |
| 任务 | cancel unknown task、cancel forbidden、provider 不支持取消、异步任务最终失败 |
| 幂等 | 重复 key 命中 running/succeeded/failed/cancelled；相同 key 不同 body 返回 conflict |
| 配置 | settings schema 非法、凭据缺失、reload 后 provider 数量为 0、inventory 为空 |
| 管理 | provider validate/add/delete/refresh 无权限、重名、损坏 metadata、删除被引用 Provider、`models.list` / `provider.list` 敏感字段泄露 |
| 命名 | `route.resolve.api_type` 与正式 `ApiType` 枚举不一致、历史别名行为不稳定、inventory 中出现非正式 api_type |
| 安全 | 跨 tenant 查询/取消拒绝，trace/log 不包含 token、prompt 原文、原始文件内容 |

错误验收要求：

- 错误码可机器判断。
- 用户可见 message 可理解。
- route trace 记录候选过滤、fallback、failover 原因。
- Provider 原始错误只保留脱敏摘要。
- 早期 kRPC error 不创建 AICC task；已创建 task 的失败写入 task data / event。
