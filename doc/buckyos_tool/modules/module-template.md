# BuckyOS Tool 模块需求模板

> 状态：Draft / Implementing / Available / Deprecated  
> 对应 module：`<module>`

## 1. 目标

说明模块解决的生产运维问题和目标资源。

## 2. 边界

### 2.1 负责

- 列出本模块负责的领域能力。

### 2.2 不负责

- 列出容易与其它模块混淆的能力，并链接到对应模块文档。

## 3. 资源模型

说明命令操作的是哪类稳定 ID、desired state、observed state 和 revision。

## 4. 命令清单

| 命令 | 主要输入 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- | --- |
| `<module> list` |  | read | sync |  |

命令应优先使用主 PRD 的标准动词。每个命令实现前必须补充输入/输出 JSON schema。
`apply <operation-id>` 可以使用 `operation-defined`，其它命令应声明固定访问级别。

## 5. 权限与安全

说明普通用户、Owner、Admin、sudo 和 Host 权限要求，以及 secret 脱敏规则。

## 6. 服务映射

说明使用哪个正式 BuckyOS service/client；禁止以 raw `system-config` 读写代替领域服务。

## 7. 输出与异步任务

说明资源 ID、分页、task、dry-run operation/revision 和部分成功语义。

## 8. 当前实现基础

列出仓库中已存在、可复用但可能过时的 API，不把它们自动视为最终协议。

## 9. 验收标准

- 至少一个只读命令的 parser、schema、mock client 单测。
- 所有写命令覆盖权限、幂等、确认和错误输出。
- 长操作覆盖默认跟踪到完成和 `--no-wait`。
- 文档中的示例能由当前命令 schema 生成或校验。

## 10. 待决策项

- 只保留会影响协议或实现边界的问题。
