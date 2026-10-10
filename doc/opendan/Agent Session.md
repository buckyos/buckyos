# Agent Session

本文内容已合入[《OpenDAN Agent Session 架构设计》](<OpenDAN Agent Session架构设计.md>)，后续以该文档的新设计目标为准。

- [§3.4 执行基础设施分层](<OpenDAN Agent Session架构设计.md#34-执行基础设施的分层>)：宿主、Session Runtime / SDK、LLMContext 与 Agent State。
- [§6 Work Session](<OpenDAN Agent Session架构设计.md#6-work-session有界执行明确失败与结果交付>)：创建与初始化、有界执行、Task 修订与 Final Report。
- [§11.6 持久状态](<OpenDAN Agent Session架构设计.md#116-持久状态与恢复依据>)、[§11.7 输入提交与恢复](<OpenDAN Agent Session架构设计.md#117-输入提交与崩溃恢复>)、[§11.8 Behavior 与模板合同](<OpenDAN Agent Session架构设计.md#118-behavior上下文切换与模板合同>)。
- [§12.12 宿主装载、状态与中断](<OpenDAN Agent Session架构设计.md#1212-宿主装载运行状态与中断>)、[§12.13 Task 服务映射](<OpenDAN Agent Session架构设计.md#1213-task-服务映射与状态同步>)。
- [§14.7 Runtime / Workspace 绑定](<OpenDAN Agent Session架构设计.md#147-runtime-与-workspace-绑定>)、[§14.8 回复与报告投递](<OpenDAN Agent Session架构设计.md#148-回复报告与可靠投递>)。
- [§15.6 原设计整合与迁移](<OpenDAN Agent Session架构设计.md#156-原-session-设计的整合与迁移>)：旧类型、目录、输入消费、恢复及 driver 配置与现有实现的差异。

旧稿的 Work 消息转发、SelfCheck / SelfImprove 分类和历史字段不再作为目标规范；Work 的 Task 修订通过正式 Task 对象半订阅送达，Memory 只提供辅助线索。持久提交、恢复与未知工具副作用的处理以新文档为准。
