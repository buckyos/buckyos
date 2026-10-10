# Agent Session 的事件订阅

本文内容已合入[《OpenDAN Agent Session 架构设计》](<OpenDAN Agent Session架构设计.md>)，后续以该文档的设计目标为准。

- [§12.7 三种等待模式](<OpenDAN Agent Session架构设计.md#127-事件订阅与三种等待模式>)：PendingTool、主动订阅与环境半订阅。
- [§12.8 订阅与输入装配](<OpenDAN Agent Session架构设计.md#128-订阅唤醒与输入装配>)、[§12.9 并发与可靠消费](<OpenDAN Agent Session架构设计.md#129-串行推进事件合并与可靠消费>)：唤醒、超时、队列、Coalesce / Override 与事件批次。
- [§12.10 Topic 与上下文成本](<OpenDAN Agent Session架构设计.md#1210-topic环境浮现与上下文成本>)、[§12.11 模式选择反例](<OpenDAN Agent Session架构设计.md#1211-模式选择与误用反例>)。
- [§15.5 实现基础与迁移差距](<OpenDAN Agent Session架构设计.md#155-事件订阅的实现基础与迁移差距>)。

半订阅的目标时机已统一为“默认每次推理前观察”，包括无普通 Message input 的 Work；不再限定为用户发来新消息时。当前实现与目标的差距单独记录，不作为收缩设计目标的依据。
