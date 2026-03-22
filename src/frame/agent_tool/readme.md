# OpenDAN AgentTool 实体化

## 背景

当前 OpenDAN Runtime 中的 AgentTool 采用传统模式实现，基于 tool_calls 机制提供基本的 Agent 工具能力。在此基础上，每个 tool 支持两种调用模式：

- **Function 模式**：标准的 function calling，可在 LLM 推理过程中多次调用
- **Action 模式**：在一次 LLM 调用的末尾执行，决定是否进入下一个 step，通常用于写操作

### 核心假设

根据我们对 Agent 使用工具的长期规划，我们相信：**所有 Agent 最终都将通过标准 Linux Bash 来使用全部外置能力。** Bash 无平台相关性，是 Agent 获得真正能力的统一入口。

### 当前问题

目前在 Tmux 中执行命令时，Runtime 会先解析命令，判断是否为内置命令：

- **是内置命令** → 走内置 AgentTool 逻辑
- **否则** → 交给 Tmux 的 Bash 执行

这导致一个关键问题：**当 LLM 构造组合性语法（如管道、子命令等）时，无法在组合命令中调用内置的 AgentTool。** 内置工具与 Bash 原生命令之间存在不可组合的断层。

## 方案：AgentTool CLI 化（BusyBox 模式）

将原有 Runtime 内置的 AgentTool 提取出来，打包为**真实存在于 Bash 环境中的可执行文件**，类似 BusyBox 的思路——一个二进制，多个命令别名。

### 基本特征

- 最终部署形态是**一个主二进制 + 多个命令别名**，而不是为每个工具长期维护独立二进制
- 命令名通过软链接、硬链接、wrapper 或 `argv[0]` 分发暴露给 Bash
- 可执行文件真实存在于 Bash 的 `$PATH` 中，可被 Bash 原生组合调用
- 支持不同的命令名 / 别名（类似 `ls`、`cat` 等）

### 部署约束

- **最终交付以单主二进制为准**，例如 `agent_tool`
- `todo`、`get_session`、`read_file` 这类名字对 Bash 仍然直接可见，但底层应尽量复用同一份可执行文件
- 开发或迁移阶段中，允许临时产出多个 `bin` 以便联调、测试、灰度验证；这属于过渡措施，不应成为最终部署模型
- 这样做的原因很直接：部署、升级、版本一致性、回滚与制品管理都会显著简单

### 执行流程

```
LLM 生成 Bash 命令
    ↓
Tmux / Bash 执行
    ↓
AgentTool CLI 可执行文件启动
    ↓
读取环境变量（由 OpenDAN 启动 Tmux 时注入）
    ↓
┌─ 纯本地工具（如 editfile）→ 直接执行，无需回调
└─ 需要 Runtime 能力的工具  → 通过本地 RPC 回调 Runtime 进程
    ↓
返回结果至 Bash stdout/stderr
```

### 环境变量注入

由于 AgentTool CLI 运行在 OpenDAN 启动的 Tmux 环境中，我们可以通过环境变量传入充足的上下文信息（Session ID、RPC 地址、认证信息等），使 CLI 工具能够定位并回调 Runtime。

### 本地 RPC 回调

对于需要 Runtime 能力的工具，CLI 进程通过本地 RPC 调用回 Runtime 主进程，由 Runtime 完成实际的执行操作后将结果返回。

## Tool Result 统一协议

借 AgentTool 实体化的机会，统一设计工具调用返回结果的协议。

协议文档已单独整理，见：

- [agent_tool_result_protocol.md](/Users/liuzhicong/project/buckyos/src/frame/agent_tool/agent_tool_result_protocol.md)

本节只保留设计动机。具体字段定义、兼容规则、`output/detail` 分工、`exec_bash` 的判定规则都以单独文档为准。

### 动机

当前 Bash 环境下只能依赖 stdout / stderr 获取结果，格式不统一。自有的 AgentTool CLI 化后，我们可以要求所有自有工具遵循统一的返回协议，带来两个关键好处：

1. **结构化返回**：统一以 JSON 格式输出结果，便于程序化解析和后续处理
2. **适配 WorkLog 压缩渲染**：工具调用的历史记录会以不同压缩比显示在 Agent 的 WorkLog 中——越早的记录压缩率越高。结构化的返回协议使我们能更智能地做 Tool Result 的提示词压缩（摘要、截断、字段裁剪等），而非粗暴地截断纯文本

### 核心洞察：同步、长任务、用户授权是同构的

工具调用从 Agent 的视角看，只有三种状态——**立即完成、还没完成、出错了**。而"还没完成"的原因可能各不相同，但对 Agent 的处理逻辑完全一致：

| 完成模式 | 等待对象 | 示例 |
|----------|----------|------|
| 同步完成 | 无 | `ls`、`cat`、`editfile` |
| 异步等待（机器） | 进程 / 系统 | `build`、`test`、`deploy` |
| 异步等待（人类） | 用户审批 / 输入 | 删除敏感文件、执行危险操作、需要人类确认方案 |

**用户授权本质上就是一种"非同步完成"的工具调用——和长时间 build 命令在结构上完全同构。** Agent 不需要知道它在等什么，只需要知道"这个操作还没完成，我先去做别的"。

### 当前协议要点

- 顶层协议对象统一为 `AgentToolResult`
- `output` 是 bash 主输出
- `detail` 是内置工具结构化数据
- `pending_reason` 当前统一使用 `long_running | user_approval | wait_for_install`
- 历史值 `external_callback` 仅作为兼容别名继续接受

### Agent 决策流程

1. 拿到 `status: "success"` 或 `"error"` → 正常处理结果，继续下一步
2. 拿到 `status: "pending"` → 记下 `task_id`
3. 看 `pending_reason` 决定行为策略：
   - `user_approval` → 催也没用，优先去做其他独立任务
   - `long_running` → 按 `check_after` 周期 poll
   - `wait_for_install` → 等待外部流程完成
4. 在后续 loop 中调用 `check_task <task_id>` 获取最终结果

### 状态可流转

一个操作的状态可以多次流转，Agent 的 poll 逻辑完全不用变——每次 check 都拿到当前 status 即可：

```
pending (user_approval)  →  pending (long_running)  →  success
   用户批准后                   开始执行 build              完成
```

### 设计要点

- 所有自有 AgentTool 的 stdout 输出遵循此统一 JSON 协议
- `summary` 字段在任何 status 下都有，支持 WorkLog 的不同粒度压缩渲染
- 外部原生命令（非自有工具）的输出仍为纯文本，由 Runtime 侧做通用处理

## 异步执行与长任务支持

### 问题

AgentTool CLI 化并合入 Bash 后，Agent 可以一次性向 Tmux 提交多条命令（不再是逐行执行）。但部分命令（如 `build`）可能耗时数分钟，当前逻辑下 Agent 必须同步等待上一条命令执行完毕才能执行下一条，造成阻塞。

现有的解决方案是让 Agent 用 SubAgent 来跑长任务，但存在两个问题：

- 调用前无法预知某个命令是长任务还是短任务
- 对于简单的长命令，启动 SubAgent 过于重量级

### 方案：基于统一协议的异步模式

异步执行不需要单独的机制——它自然地落在 Tool Result 统一协议的 `pending` 状态上。Agent Loop 的底层只需要能理解 `pending` 并做相应调度即可：

```
Agent 发起命令
    ↓
工具返回 { status: "pending", task_id: "xxx", pending_reason: "long_running" }
    ↓
Agent 继续执行其他操作
    ↓
Agent 在后续 loop 中调用 check_task <task_id>
    ↓
拿到 { status: "success", detail: {...} } → 处理结果
```

Agent 知道自己在一个工作 Session 中有且仅有一个 Tmux，可以选择：

- **同步模式**（默认）：等待命令执行完毕后继续
- **异步模式**：命令后台执行，返回 `pending`，Agent 可继续做其他事情，按需 poll 结果

### 并发层次总览

| 层次 | 机制 | 当前状态 |
|------|------|----------|
| Session 级 | 同一 Agent 跑多个 Session（不共享 Workspace 即可并发） | ✅ 已支持 |
| SubAgent 级 | 通过 SubAgent 实现子任务并发 | ✅ 已支持 |
| **工具调用级** | **单个 Session 内的 pending → poll 异步调用** | ⬜ 待实现 |

核心观点：Agent 不应假设所有工具都是同步的。从基础能力上，Agent Loop 应支持异步操作，让 Agent 能够同时做多件事情。

### 统一模型带来的额外好处

**权限分级自然落地。** 可以在 tool 定义里标注哪些操作需要用户 approval，Runtime 拦截后直接返回 `pending + user_approval`。Agent 不需要知道权限策略的细节，它只知道"这个操作还没完成"。

**AHL（Agent-Human-Loop）天然融入。** OpenDAN 的 AHL Workflow Engine 中，human-in-the-loop 不再是特殊路径——它就是一个返回 `pending` 的工具调用，和等一个 docker build 没有本质区别。人类审批、人类提供输入、人类确认方案，都走同一个 `pending → poll → result` 流程。

**状态可组合。** 一个操作可能先 `pending: user_approval`（等用户批准），批准后变成 `pending: long_running`（开始执行），最终变成 `success`。Agent 的 poll 逻辑完全不用变。

## 设计原则

### 1. 能不绕回 Runtime 就不绕回

- **纯本地工具**（如 `editfile` 等文件系统操作）：直接在 CLI 进程内完成，不经过 RPC
- **需要 Runtime 上下文的工具**：才通过本地 RPC 回调

尽量减少对 Runtime 的依赖，保持工具的独立性和轻量性。

### 2. Tmux 基座保持不变

Tmux 作为 Agent Bash 环境的底层基座已经非常稳固，提供了良好的 Session 管理和屏幕捕获能力。本次重构在 Tmux 之上进行，不改变底层架构。

### 3. 自有工具遵循统一协议

所有 AgentTool CLI 的输入输出遵循统一协议，为 WorkLog 压缩渲染和结构化处理提供基础。

## 收益

### 1. Bash 原生可组合

AgentTool 成为真正的 CLI 命令后，LLM 可以自由地将其与其他 Bash 命令组合使用（管道、重定向、子命令、脚本等），不再有内置命令与 Bash 命令之间的断层。

### 2. 调试便利性

此前调试一个 AgentTool 需要拉起整个 OpenDAN Runtime，调用链很长。CLI 化后，可以直接在终端中独立运行和调试单个工具，开发体验大幅改善。  
这里的“独立运行”不要求最终发布多个独立二进制；单主二进制 + 命令别名同样满足调试需求。

### 3. 为 Agent 自演化铺路

当 AgentTool 以独立可执行文件形式存在时，Agent 未来可以"阅读"自己工具的源码，理解其实现方式，从而具备自主创建或改进工具的能力。

### 4. 长任务不再阻塞

异步执行模式让 Agent 在等待长时间命令时不必空转，可以继续推进其他工作，整体效率提升。

### 5. AHL 与权限控制统一

用户授权、人类审批、人类输入不再是 Agent 系统中的特殊路径，而是与长任务等待完全同构的 `pending → poll → result` 流程。权限分级策略可以在 tool 定义层标注，Runtime 拦截后透明地返回 `pending`，Agent 无需感知权限细节。

## 实现语言选择

| 阶段 | 语言 | 理由 |
|------|------|------|
| **当前阶段** | **Rust** | 稳定可靠，适合系统级 CLI 工具 |
| 未来（自演化阶段） | TypeScript | Agent 可读、可理解、可修改自身工具的源码 |

当前阶段距离 Agent 自演化还较远，优先选择 Rust 以保证工具的稳定性和性能。待进入自演化阶段后，再考虑切换到 TypeScript 以降低 Agent 理解和修改工具代码的门槛。
