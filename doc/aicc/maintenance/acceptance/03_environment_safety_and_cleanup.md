# AICC 验收环境、安全与清理

定义 preflight、cleanup、真实模型开关、临时 group 生命周期、报告保留和清理失败处理。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. 预检与清理流程

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
12. 如果是 L4，调用 `models.list` 并读取最终生效逻辑目录，生成 `api_type × method × logical_path × Provider × model` 矩阵，并在真正执行前把矩阵摘要写入报告。

执行后应做 cleanup：

1. 停止 runner 启动的 Mock Provider。
2. 恢复或清理测试写入的 AICC settings。
3. 清理测试写入的 route overlay/settings。
4. 清理未完成的 Mock task 或记录到报告。
5. 保留报告、输入、输出和脱敏后的 Provider 请求摘要。
6. 如果是 L4，停止并清理本次 runner 新建的临时 group / VM。
7. 如果 `keep_on_failure=true`，保留临时 group，但必须在报告中写入 group 名、节点名和手工清理命令。

清理失败不能覆盖原始测试失败原因，应作为单独 warning 写入报告。

