# Agent Workspace 的产品目标

## 1. 背景与定位

**Agent Workspace** 是面向 Agent 系统的一个「任务管理 + 执行过程可视化」工作台视图。
它的定位不是传统意义上的 To-do 工具，而是用于**呈现 Agent Loop 的运行细节**，让用户（或开发/运维/产品人员）能够以 Agent 为单位理解它正在如何推进工作。

## 2. 核心目标

Agent Workspace 的核心目标是：

* **以 Agent 为单位呈现工作推进情况**（一个 Agent 对应一个 Workspace）
* 让用户能清晰看到：

  * Agent Loop 由哪些 Step 构成
  * 每个 Step 发生了哪些关键行为
  * 当前进度与历史演进路径
  * 串行与并行（Sub-Agent）是如何展开的

一句话：**让“Agent 在做什么、做到哪一步、做过什么、并行做了什么”可见。**

## 3. 运行模型与状态来源

### 3.1 Agent Loop 的触发与结构

* Agent 会在被事件触发（例如“唤醒”事件）后开始执行一次 **Agent Loop**
* 一个 Loop 由多个 **Step** 组成
* 每个 Step 内部可能进行 **1～6 次 LLM 推理**

### 3.2 Task：LLM 推理驱动的子任务

* **每一次 LLM 推理都会产生一个 Task**
* 这些 Task 属于某个更高层级行为（Behavior）的一部分

  * 可理解为：Behavior 是一次完整行为过程，Task 是其中由推理产生的子任务

### 3.3 WorkLog：异步与外部交互的全过程记录

WorkLog 是 Workspace 中用于记录“发生过什么”的核心日志结构，覆盖以下类型：

1. **Message（消息发送/接收）**

* Agent 会发送消息，且消息是**纯异步**：

  * 发送后不能期望立刻返回
* Message 与 Message Reply 都应记录到 WorkLog

2. **Function Call（Step 内同步依赖调用）**

* Function Call 通常成本较高
* 常发生于 Step 内部的某段逻辑中：

  * 必须拿到结果后才能继续推进到下一段
* 调用过程与结果需要被记录到 WorkLog

3. **Action（Step 之间的动作集合）**

* 通常在 Step 结束时生成一组 Action
* 支持按配置 **并行执行/串行执行**
* 不要求全部成功（允许部分失败）
* Action 的结果会汇总进入 **下一个 Step 的运行环境**
* 执行过程与结果需要记录到 WorkLog

### 3.4 并行能力：Sub-Agent（从串行到并行）

Agent Loop 支持通过创建 **Sub-Agent** 将串行逻辑扩展为并行推进：

* 创建 Sub-Agent 后，通过 Agent 间异步消息通信与其交互
* 消息也可以发给外部/平行的 Agent（非子层级）
* Sub-Agent 的生命周期行为需要记录到 WorkLog，包括：

  * 创建 / 销毁
  * 激活 / 休眠（部分 Sub-Agent 是长期存在的）
  * 与 Sub-Agent 的通信过程

该机制在概念上类似传统系统里的“创建线程/任务分叉”。

### 3.5 Todo List：每个 Agent 的待办推进

* 每个 Agent（包括 Sub-Agent）都有独立的 **Todo List**
* Todo Item 会随着 Agent Loop 推进而：

  * 新建
  * 标记完成

## 4. Workspace 的状态集合定义

Agent Workspace 需要统一管理并可视化展现的状态集合为：

* **Task 管理**
* **WorkLog 管理**

  * 包含 Message / Message Reply
  * 包含 Function Call & Action 的调用与结果
  * 包含 Sub-Agent 生命周期与通信记录
* **Todo List**

这些共同构成一个 Agent 在运行过程中的“可观察状态”。

## 5. UI 侧的产品关注点（目标导向）

Agent Workspace 的 UI 不以“列表堆砌”为目标，而是专注解决：

* 以 Agent 为单位呈现**当前工作进度**
* 展现 Agent Loop 的结构与演进：

  * Step → 其中的推理/调用/消息/动作
  * 由此产生的 Task、WorkLog、Todo 的变化
* 展现并行展开：

  * 哪些 Sub-Agent 被创建
  * 它们在做什么
  * 与主 Agent 的交互如何发生、何时发生

最终效果：用户打开一个 Agent Workspace，就能快速理解它的工作推进链路与当前状态。


// worklog用sqlite记录workspace里的工作日志
// 通过worklog，可以看到agent(包括sub agent)是如何完成工作的
// worklog除了常规字段为，先增加thread-id和tag字段，方便用不同的方法查询和汇聚worklog
// worklog的主要接口是 append 和 必要的查询接口
//
// 下面是worklog中可能会出现的log类型
// create task / task complete/finish 
// send msg / recv msg
// create/active subagent, delete/disable subagent
// toos usage record (function call & action)
// create todo / update todo (注意记录todo的父todo)
//