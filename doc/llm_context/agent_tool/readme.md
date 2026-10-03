# OpenDAN AgentTool 实体化

## 文档索引

本目录：

- [OpenDAN AgentTool 开发指南](<OpenDAN AgentTool 开发指南.md>)：分层、派发、取消与 task、CLI、环境变量，新增工具的步骤
- [agent_tool_result_protocol.md](agent_tool_result_protocol.md)：`AgentToolResult` 字段定义、StepRecord 渲染、到 `Observation` 的映射与 Pending 处理
- [builtin_agent_tools.md](builtin_agent_tools.md)：当前 builtin tools 与 task 工具的输入 / 输出约定
- [agent-tool实例分析.md](agent-tool实例分析.md)：以 `read_file` / `write_file` 为例讲 `TypedTool` 的写法
- [无需返回的agent-tool.md](无需返回的agent-tool.md)：结果不需要进入后续输入的工具
- [Agent 计划任务cli工具需求.md](<Agent 计划任务cli工具需求.md>)：`dcrontab` 需求

上级目录：

- [run_local_llm SDK 化：目录与命令行协议基线](../local_llm_context_protocol.md)：工具 SDK 化范围、Rust 协议现状与 TS 设计参考
- [xllm Rust SDK 参考](../xllm_rust_sdk.md)：`.llm_context` 配置、Run 目录、状态机、退出码、提示词组装、共享 AgentRuntime
- [LLM Context 设计](<../LLM Context 设计.md>)：waist、`ToolManager`、打断与平滑结束

## 背景

AgentTool 基于 tool_calls 机制提供 Agent 的工具能力。每个 tool 支持两种调用模式：

- **Function 模式**：标准的 function calling，一个 run 中可多次调用；每次推理（Round）返回的原生 tool calls 组成一个工具批次，结果按 `call_id` 回灌
- **Action 模式**：Behavior Loop 中在一个 Step 的决策输出之后执行，结果记入该 Step 的 `StepRecord`，供下一个 Step 读取，通常用于写操作

### 核心假设

根据我们对 Agent 使用工具的长期规划，我们相信：**所有 Agent 最终都将通过标准 Linux Bash 来使用全部外置能力。** Bash 无平台相关性，是 Agent 获得真正能力的统一入口。

如果内置工具只存在于宿主进程里，LLM 构造组合命令（管道、子命令、脚本）时就无法调用它们，内置工具与 Bash 原生命令之间存在不可组合的断层。

## 分层

```text
宿主：xllm CLI / libopendan SessionRunner / xagent
  ▼
llm_context（waist）：ToolManager::call_tool(call, ToolCallCtx) -> Observation
  ▼
agent_tool::runtime：AgentRuntime（native | tmux | remote_ssh）= 执行环境（sandbox）
  ├─ 内置 bash 组：shell / read_file / write_file / edit_file（+ task 工具）
  ├─ MCP 工具
  └─ 宿主工具
        ▼  shell 命令中
  PATH 上的 agent_tool CLI（单二进制 + 命令别名）
```

- Runtime 决定工具在哪里执行。内置工具、模板里的命令都经同一个执行体，同一份实现可以跑在本机进程、tmux 或远端 SSH 上。
- AgentTool 同时有两种形态：在派发器里被直接调用，或作为 CLI 命令在 `shell` 里被调用。两种形态共用同一份实现和同一个结果协议。

细节见 [开发指南](<OpenDAN AgentTool 开发指南.md>) 第 1、2 节。

## 方案：AgentTool CLI 化（BusyBox 模式）

把 AgentTool 打包为**真实存在于 Bash 环境中的可执行文件**，类似 BusyBox：一个二进制，多个命令别名。

### 基本特征

- 最终部署形态是**一个主二进制 + 多个命令别名**，而不是为每个工具长期维护独立二进制
- 命令名通过软链接、硬链接、wrapper 或 `argv[0]` 分发暴露给 Bash
- 可执行文件真实存在于 Bash 的 `$PATH` 中，可被 Bash 原生组合调用

### 部署约束

- **最终交付以单主二进制为准**，即 `agent_tool`
- `todo`、`get_session`、`read_file` 这类名字对 Bash 直接可见，但底层复用同一份可执行文件
- 开发或迁移阶段允许临时产出多个 `bin` 以便联调、测试、灰度验证；这属于过渡措施，不应成为最终部署模型
- 原因：部署、升级、版本一致性、回滚与制品管理都会显著简单

### 执行流程

```
LLM 调用 shell
    ↓
runtime 执行 bash 命令（PATH 前置 Session Bin 与 Agent Bin）
    ↓
AgentTool CLI 启动，按 argv[0] 分发
    ↓
读取环境变量（宿主注入），构造 RuntimeContext
    ↓
┌─ 纯本地工具（如 edit_file）→ 直接执行
└─ 需要 Session / Agent 状态的工具 → 读写 Session 目录、Agent RootFS，或访问 BuckyOS 服务
    ↓
stdout 输出 AgentToolResult JSON；简单命令的结果由 shell 直接转发给模型
```

PATH 分层与环境变量契约见 [开发指南](<OpenDAN AgentTool 开发指南.md>) 第 5、6 节。

## Tool Result 统一协议

借 AgentTool 实体化的机会，统一设计工具调用返回结果的协议。字段定义、兼容规则、`output` / `detail` 分工、`shell` 的转发规则见 [agent_tool_result_protocol.md](agent_tool_result_protocol.md)，本节只保留设计动机。

### 动机

Bash 环境下只能依赖 stdout / stderr 获取结果，格式不统一。自有 AgentTool CLI 化后，所有自有工具遵循统一的返回协议，带来两个好处：

1. **结构化返回**：统一以 JSON 格式输出结果，便于程序化解析和后续处理
2. **适配 WorkLog 压缩渲染**：工具调用的历史记录会以不同压缩比显示在 Agent 的历史中，越早的记录压缩率越高。结构化的返回协议让我们能更智能地压缩 Tool Result（摘要、截断、字段裁剪等），而非粗暴地截断纯文本

### 核心洞察：同步、长任务、用户授权是同构的

工具调用从 Agent 的视角看，只有三种情况：**已经完成、还没完成、出错了**。“还没完成”的原因各不相同，但对 Agent 的处理逻辑一致：

| 完成模式 | 等待对象 | 示例 |
|----------|----------|------|
| 同步完成 | 无 | `ls`、`cat`、`edit_file` |
| 异步等待（机器） | 进程 / 系统 | `build`、`test`、`deploy` |
| 异步等待（人类） | 用户审批 / 输入 | 删除敏感文件、执行危险操作、需要人类确认方案 |

**用户授权本质上是一种“非同步完成”的工具调用，和长时间 build 命令在结构上同构。** Agent 不需要知道它在等什么，只需要知道“这个工作还没完成，有一个 task 可以跟进”。

### 协议要点

- 顶层协议对象统一为 `AgentToolResult`
- builtin tool 的固定字段是 `agent_tool_protocol / cmd_name / status / summary / detail`
- `detail` 是内置工具结构化数据；`output` 只在明确需要 bash 主文本输出时使用
- 调用返回后仍在运行的工作用 `task_id` 表示；`pending` 只在这次调用的结果必须等 task 结束才有时使用，且必须带 `task_id`

### 设计要点

- 所有自有 AgentTool 的 stdout 输出遵循此统一 JSON 协议
- builtin tool 应稳定提供 `cmd_name` 与 `summary`，支持 WorkLog 的不同粒度压缩渲染
- 外部原生命令（非自有工具）的输出仍为纯文本，由 `shell` 做通用包装

## 长任务

### 问题

部分命令（如 `build`）可能耗时数分钟甚至更久。如果工具调用必须同步等完，Agent 就只能空转；而调用前无法预知一个命令会跑多久，让 LLM 在调用前选择“同步 / 后台”也会出错。为简单的长命令启动 SubAgent 又过于重量级。

### 方案：配置决定执行方式，task 承接未完成的工作

- **执行方式由配置决定，不由 LLM 选择。** `shell` 的 `tools.shell.mode` 默认 `auto`：在调用内等 `wait_ms`（默认 30s），到期仍在运行的命令转为进程内 task，调用立即返回“仍在运行”、`task_id`、已有输出和跟进方式；`wait` 模式则等到结束或超时。
- **task 工具跟进。** 模型用 `wait_task` / `get_task_state` / `cancel_task` 跟进 task；`cancel_task` 只对声明了可取消的 task 有效。buckyos task 也可以在 shell 里用 CLI `check_task` / `cancel_task` / `finish_task`。
- **后台状态半自动呈现。** 每次推理前，本 context 启动的 task 以 `<background_tasks>` 简介追加在请求末尾，不进历史；完整输出由模型按需取。
- **调用内等待有上限。** 任何工具内等待最长 30 分钟后必须把控制权还给 LLM，task 继续运行，由 LLM 决定是否继续等或取消。更长的等待只能由能挂起 run 的宿主（Session 层）在进程外进行。
- **llm_context 只做两种机械判断。** 结果交给 LLM；或者工具返回 `pending` + `task_id`，run 挂起等 task 后按 `call_id` 回填。

```
LLM 调用 shell（cargo test）
    ↓ 30s 后仍在运行
返回 { status: "success", task_id: "local:shell:<call_id>", output: "still running ... Call wait_task ..." }
    ↓
LLM 继续做别的；每次推理前看到 <background_tasks>
    ↓
LLM 调用 wait_task / get_task_state
    ↓
拿到最终结果
```

用户审批将走同一模型：审批是一个 task，批准后再开始执行，Agent 的跟进方式不变。

### 并发层次总览

| 层次 | 机制 | 当前状态 |
|------|------|----------|
| Session 级 | 同一 Agent 跑多个 Session（不共享 Workspace 即可并发） | ✅ 已支持 |
| SubAgent 级 | 通过 SubAgent 实现子任务并发 | ✅ 已支持 |
| 工具调用级 | 单个 run 内的 task：`shell` auto 模式 + task 工具 + `<background_tasks>` | ✅ 已支持（进程内 task） |
| Session 挂起等待 | `allow_deferred`：run 以 `PendingTool` 挂起，宿主在进程外等 task 后回填 | ⬜ llm_context 已支持，xllm / libopendan 未开启 |

### 统一模型带来的额外好处

**权限分级自然落地。** 可以在 Runtime 层统一检查工具调用，需要用户 approval 时返回一个审批 task。Agent 不需要知道权限策略的细节，它只知道“这个工作还没完成”。

**AHL（Agent-Human-Loop）天然融入。** human-in-the-loop 不再是特殊路径：人类审批、人类提供输入、人类确认方案，都和等一个 docker build 一样是一个 task。

## 设计原则

### 1. 能不绕回宿主就不绕回

- **纯本地工具**（如 `edit_file` 等文件系统操作）：直接在 CLI 进程内完成
- **需要 Session / Agent 状态的工具**：把 session、todo、worklog 当作 Session 目录、Agent RootFS 里的文件或 BuckyOS 服务来操作，尽量不回调宿主进程

尽量减少对宿主的依赖，保持工具的独立性和轻量性。

### 2. Runtime 是执行环境

工具在哪里执行由 run 的 runtime 决定（native / tmux / remote_ssh），工具实现不假设自己跑在本机。Runtime 同时是 sandbox：之后的安全检查、授权都在这一层统一进行。

### 3. 不给 LLM 增加调用前的决策

能交给配置或运行结果的，就不给 LLM 加参数，也不给协议加字段。工具返回结果时附带下一步提示（渐进式披露），细节由 LLM 按需调用工具获取。

### 4. 标准父子进程语义

执行器只管理正在执行的命令。命令留下的后台进程（`&`、nohup、setsid）在打断、超时、run 结束、恢复时都不被追杀；恢复时不核验进程，把“被打断、结果未知”如实交给 LLM 判断。

### 5. 自有工具遵循统一协议

所有 AgentTool CLI 的输入输出遵循统一协议，为历史压缩渲染和结构化处理提供基础。

## 收益

### 1. Bash 原生可组合

AgentTool 成为真正的 CLI 命令后，LLM 可以自由地将其与其他 Bash 命令组合使用（管道、重定向、子命令、脚本等），不再有内置命令与 Bash 命令之间的断层。

### 2. 调试便利性

CLI 化后，可以直接在终端中独立运行和调试单个工具，而不必拉起整个 Agent 宿主。单主二进制 + 命令别名同样满足独立调试的需求。

### 3. 为 Agent 自演化铺路

当 AgentTool 以独立可执行文件形式存在时，Agent 未来可以“阅读”自己工具的源码，理解其实现方式，从而具备自主创建或改进工具的能力。

### 4. 长任务不再阻塞

长命令转为 task 后，Agent 不必空转，可以继续推进其他工作。

### 5. AHL 与权限控制统一

用户授权、人类审批、人类输入与长任务等待是同一个 task 模型。权限策略在 Runtime 层统一处理，Agent 无需感知权限细节。

## 实现语言选择

| 阶段 | 语言 | 理由 |
|------|------|------|
| **当前阶段** | **Rust** | 稳定可靠，适合系统级 CLI 工具 |
| 未来（自演化阶段） | TypeScript | Agent 可读、可理解、可修改自身工具的源码 |

当前阶段距离 Agent 自演化还较远，优先选择 Rust 以保证工具的稳定性和性能。进入自演化阶段后，再考虑切换到 TypeScript，以降低 Agent 理解和修改工具代码的门槛。
