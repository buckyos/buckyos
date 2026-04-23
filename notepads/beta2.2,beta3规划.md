# BuckyOS Beta2.2 版本和Beta3版本规划

计划: 4月底Beta2.2进入发布轨道，Beta3预计5月初进入开发，5月底发布。
在Beta3开发期间，视Beta2.2的发布效果，会考虑是否发布Beta 2.3(App体验优化和Agent增强)

## Beta 2.2 目标

- 进入完全AI-Native的开发循环
- 从向下兼容的角度，完成BuckyOS规划的所有内核组件。Beta2.2 计划提供稳定的概念抽象和模块边界

## Beta 3 目标

- 能将BuckyOS安装在商用硬件上
- 集成严肃的备份恢复流程
- 集成USDB
- 在特定环境下，集成cyfs(文件系统)

## 了解Beta 2.2

一些关键的设计稳定下来

- cyfs:// 协议除大容器外的部分定稿 （推荐阅读协议文档)
- Named Data Mgr (分布式对象存储) 正式发布。
- rtcp:// 协议设计定稿，支持中转，并完善所有过去已知的问题
- 调度器核心功能全部完成（0到1，还有巨大的策略优化空间）
    - 定义Function Instance支持OP Task和FaaS类任务
    - 通过paios统一镜像支持Scirpt类AppService,降低App开发门槛
    - 定义RDB Instance,统一管理RDB
- 加入workflow
- 通过workflow+taskmgr,完成对Agent-human-loop的支持，完成对Agent意图引擎的支持
- 正式发布buckyos-webk-sdk
- 实现正式的WebUI Deskotp + ControlPanel
- Message Center 支持Self-host group,严肃的完成TG MsgTunnel和Lark MsgTunnel 

完成两个重要的内置App
- FileBrowser
- MessageHub



## Beta 2.2的工作开展

- AI不适合渐进式迭代，更适合 `设计一步到位+分模块实现`
- 人类主导设计验收，包括“验收什么”，“在什么地方验收”，再给Agent执行
- 修改需要通过PR提交，并在流程中展现“构造代码用的提示词”
- 通过Agent改进关键的项目流程
    - 自动Review PR，分析修改的“爆炸半径“并要求人类参与
    - 对线上服务的状态进行自动跟踪
    - 持续的进行自动化验证





