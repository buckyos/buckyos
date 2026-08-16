# Backup 模块需求

> 状态：Draft  
> 对应 module：`backup`

## 1. 目标与边界

为企业版升级和灾难恢复提供一致性备份、校验、恢复和保留策略。NamedData/RepoService 可以是
备份存储底座，但“若干对象已经复制”不等于系统备份完成。

## 2. 资源模型

- backup operation：范围、冻结点、预计容量、目标和风险；
- snapshot/manifest：组件版本、对象清单、hash、加密和一致性标记；
- restore operation：目标版本、依赖顺序、冲突和回滚点；
- retention policy 和 verify report。

## 3. 初始命令

| 命令 | 访问级别 | 执行模式 | 说明 |
| --- | --- | --- | --- |
| `backup dry-run --operation <op>` | privileged | task/either | 对 create/restore/prune 做无副作用预演 |
| `backup apply <operation-id>` | operation-defined | task | 按 create/restore/prune 风险级别执行 operation |
| `backup list` | read | sync | 分页列出备份 |
| `backup get <backup-id>` | read | sync | 获取 manifest 摘要和状态 |
| `backup verify <backup-id>` | read | task | 校验完整性和可恢复前置条件 |

## 4. 安全与升级集成

- 默认加密，密钥不写入 manifest 和 CLI 输出。
- `backup dry-run --operation create --reason pre-upgrade` 产生的 operation 可成为 system upgrade
  的 gate。
- create operation 执行成功必须包含完整 manifest 和 verify 状态；Task 成功不等于 restore
  已演练。
- restore/prune 必须 dry-run/apply、scoped sudo、显式确认和审计。
- local-link 内容必须被识别为非持久或被 materialize，不能静默遗漏。

## 5. 当前状态与待决策

仓库目前没有统一的系统 Backup/Restore 服务协议。实现 CLI 前需要先确定一致性边界、组件
freeze hook、备份目标、加密密钥和 restore orchestrator。
