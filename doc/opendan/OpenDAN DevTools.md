# OpenDAN Agent DevTools 设计文档

版本：v1.0（基于口述记录整理）  
状态：设计草案  
范围：Agent 开发、调试、运行时控制、Sub-Agent 协同

---

## 1. 背景

OpenDAN 的 Agent 不再只是“输入 Prompt，得到回复”的简单模型调用，而是一个持续运行的系统。它会在一个长期会话（longtime runtime）中持续推进自己的工作，包括：

- 读取和拼接 Prompt
- 调用 LLM 进行推理
- 规划和执行 Tool
- 读写 Workspace
- 维护 Memory / Worklog
- 通过 To-Do System 协调任务
- 生成或管理 Sub-Agent

这类系统的开发难点，与传统程序调试相比有明显差异：

1. **可观察性差**：开发者很难知道某一轮推理到底看到了什么 Prompt，为什么做出某个决策。
2. **可中断性弱**：运行时一旦开始推进，很多步骤是直接跑完的，缺少中途停下来的机制。
3. **可修复性差**：如果 Tool 返回异常、Prompt 拼错、Memory 污染，往往只能重启整个 Session。
4. **可复现性不足**：相同 Agent 在不同时间点、不同状态下表现不同，问题难以回放。
5. **子 Agent 协同难以调试**：主 Agent 与 Sub-Agent 的关系不是传统函数调用，而是通过任务系统解耦，调试视角需要重构。

因此，OpenDAN 需要一套面向 **Agent Loop** 的开发工具链。其核心不是“做一个很重的 UI”，而是把 Agent 变成一个 **可调试程序**：

> 可以停、可以看、可以改、可以继续跑。

---

## 2. 设计目标

本设计希望构建一个统一的 OpenDAN Agent DevTools 体系，满足以下目标：

### 2.1 核心目标

1. **支持单步调试**
   - 每轮 LLM 推理完成后自动进入可观察状态。
   - Tool 调用前后可单步推进。
   - 下一轮 Prompt 生成前可以预览并修改。

2. **支持条件断点**
   - 可按事件类型、Loop 次数、Tool 名称、Prompt 内容等条件停下。

3. **支持运行时状态编辑**
   - 在调试暂停时修改 Prompt、Memory、Workspace、Tool Result、To-Do 等状态。

4. **支持 Skill 注入**
   - 可手工向当前 Session 注入一个或多个 Skill，用于验证 Prompt 能力、临时扩展功能、模拟配置变化。

5. **支持 Sub-Agent 调试与创建**
   - 可以在运行前或运行中从某个 Agent Environment 派生一个 Sub-Agent。
   - 主 Agent 与 Sub-Agent 可并行运行、独立暂停。

6. **支持两种开发模式**
   - 面向普通用户的 Web UI / 可视化控制台。
   - 面向高级开发者的 CLI + 文本文件 + IDE 集成方式。

7. **尽量减少对 Runtime 的侵入**
   - 通过统一的断点检查与状态补丁机制完成，不重写整个 Agent Runtime。

### 2.2 非目标

第一阶段不追求以下能力：

- 完整替代现有 IDE
- 立即实现强确定性的全量重放
- 一开始就支持复杂的分布式多 Agent 调试拓扑
- 为所有 Tool 提供统一 GUI

这些能力可以作为后续扩展。

---

## 3. 设计原则

### 3.1 文件优先，而不是 UI 优先

Agent 的开发主体应以 **文本文件编辑** 为主。原因是：

- 开发者已经习惯 VSCode、JetBrains、Vim 等现有工具
- 文本文件天然适配 Git、Diff、Merge、Code Review
- 自研 UI 很难达到主流 IDE 的完整度

因此整体方向是：

> 文件系统 + CLI Debugger + 可选 Web UI + 可选 IDE 插件

### 3.2 调试能力必须下沉到 Runtime

调试器不能只是外围观察工具，而必须成为 Runtime 的一部分。只有 Runtime 自身支持：

- 断点检查
- 暂停 / 恢复
- 状态覆盖
- 检查点导出

上层 UI 和 CLI 才有实现基础。

### 3.3 Source 与 Runtime 分离

调试时对 Prompt 的修改，不应默认直接改写源文件，而应区分：

- **Source Edit**：修改磁盘上的 Agent 配置/Prompt 文件
- **Runtime Override**：仅对当前 Session 生效的运行时补丁

这样可以在“不污染源码”的前提下完成调试实验。

### 3.4 Agent 与 Sub-Agent 解耦

Sub-Agent 与主 Agent 的沟通原则上只通过 **To-Do System** 完成，不直接形成硬依赖。这样可保证：

- 主 Agent 卡住时，Sub-Agent 仍可继续
- Sub-Agent 等授权时，不阻塞主 Agent
- 多 Agent 调试可以并行进行

### 3.5 可观察、可控制、可修复、可演进

调试器应同时满足四个层次：

- **可观察**：看见 Prompt、Tool、Memory、Workspace、Task
- **可控制**：停下、继续、单步、切换 Agent
- **可修复**：直接补状态，不必整局重来
- **可演进**：后续可扩展到 Replay、Time Travel、Fork

---

## 4. 核心概念

### 4.1 Agent Loop

Agent 的标准执行循环，可抽象为：

```text
读取环境 / 状态
→ 拼接 Prompt
→ LLM 推理
→ 解析输出
→ 计划 Tool 调用
→ 执行 Tool
→ 更新 Memory / Worklog / To-Do / Workspace
→ 生成下一轮 Prompt
→ 进入下一轮循环
```

### 4.2 Session

Session 是 Agent 的一次运行实例，包含当前会话状态、运行游标、调试标志、临时覆盖等。

### 4.3 Agent Environment

Agent Environment 是可落盘、可导出、可加载的运行环境，通常包含：

- Prompt 源文件
- Memory
- Workspace
- Worklog
- Session 状态
- To-Do 数据
- Skill 配置
- 调试覆盖信息（可选）

### 4.4 Breakpoint Point

Runtime 中预定义的一组“可停点”，如：

- LLM 推理完成后
- Tool 执行前
- Tool 执行后
- 下一轮 Prompt 生成前
- 新一轮 Loop 开始时

### 4.5 Debug Pause

Session 因断点命中而暂停的状态。在此状态下，允许查询与修改运行时状态。

### 4.6 Runtime Override

只对当前 Session 生效的运行时状态补丁，例如：

- 覆盖下一轮 Prompt
- 覆盖某次 Tool 返回值
- 修改某段 Memory
- 注入临时 Skill

### 4.7 Sub-Agent

通过 To-Do System 与主 Agent 协同的独立 Agent 实例，可拥有独立 Session 和 Workspace，也可选择共享 Workspace。

---

## 5. 用户角色与开发模式

OpenDAN 需要同时服务两类开发者。

### 5.1 初级开发者模式（Beginner Mode）

面向：

- 普通用户
- 非专业开发者
- 刚开始尝试构建 Agent 的用户

提供方式：

- OpenDAN 平台自带 Web UI
- Agent 首页内建控制台（Agent Console）
- 可直接以调试模式启动 Agent

该模式下，用户可通过图形界面完成：

- 启动 / 暂停 / 继续 Agent
- 查看当前 Prompt
- 查看推理输出
- 查看 Tool 调用与结果
- 修改下一轮 Prompt
- 查看 Workspace / To-Do / Memory
- 手动创建 Sub-Agent

该模式是“保底方案”，保证不会使用 CLI 或 IDE 的用户也能完成 Agent 调试。

### 5.2 高级开发者模式（Advanced Mode）

面向：

- 专业开发者
- 需要与现有 IDE 深度整合的用户
- 需要版本控制、Diff、批量编辑、自动化脚本的团队

提供方式：

- 以调试模式启动 Agent Runtime
- 通过 CLI 工具 attach 到 Session
- 在现有 IDE 中直接编辑 Agent Environment 文件
- 用 CLI 发出调试命令控制运行时推进

该模式的核心理念类似传统调试器：

> Runtime 负责停住，CLI 负责控制，IDE 负责编辑。

---

## 6. 整体架构

整体架构分为四层：

```text
Web UI / VSCode 插件 / 其他可视化工具
                ↓
          CLI Debugger
                ↓
       Debug Protocol / Runtime API
                ↓
      OpenDAN Longtime Runtime Core
```

### 6.1 Runtime Core

负责：

- 管理 Session 生命周期
- 执行 Agent Loop
- 提供断点检查 Hook
- 暂停 / 恢复 Session
- 暴露可读写状态

### 6.2 Debug Protocol / Runtime API

负责：

- 查询状态
- 修改状态
- 设置断点
- 恢复执行
- 导出 Environment / Snapshot
- 创建或控制 Sub-Agent

实现方式可以是：

- 进程内 RPC
- 本地 IPC
- 本地 HTTP/Unix Socket

此处不强制绑定具体协议，重点是要形成统一的调试控制面。

### 6.3 CLI Debugger

提供高级开发者主要入口。例如：

```bash
opendan agent run --debug
opendan debug attach <session-id>
```

CLI 负责：

- 断点设置
- 状态查看
- 单步推进
- 运行时补丁
- 导出快照
- 管理 Sub-Agent

### 6.4 上层 UI

可以包括：

- Web UI Agent Console
- VSCode Extension
- 后续第三方 IDE 集成

UI 不直接实现核心调试逻辑，只消费 Runtime 暴露的调试能力。

---

## 7. Runtime 状态模型

### 7.1 新增 Session 调试状态

在 OpenDAN 的 longtime runtime 中增加一个新的 Session 状态或状态标志：

```text
WAIT_FOR_DEBUG
```

或者在数据结构上表示为：

```text
session.debug.wait_for_debug = true
```

其语义是：

> 当前 Session 处于“遇到断点条件时应暂停”的调试模式。

### 7.2 推荐状态机

建议 Session 至少具备如下状态：

```text
INIT
RUNNING
WAIT_FOR_DEBUG
PAUSED_DEBUG
STOPPED
ERROR
```

其中：

- `RUNNING`：正常推进 Agent Loop
- `WAIT_FOR_DEBUG`：带断点检查地运行
- `PAUSED_DEBUG`：已经命中断点并暂停
- `STOPPED`：执行结束或被主动停止

### 7.3 调试状态含义

- `WAIT_FOR_DEBUG` 不代表“当前已经停住”，而是“当前运行在可命中断点的模式下”。
- 一旦命中断点，Session 转入 `PAUSED_DEBUG`。
- 当收到 `next / continue / step-tool` 等命令后，再恢复到 `WAIT_FOR_DEBUG` 或 `RUNNING`。

---

## 8. 断点模型

### 8.1 可用断点位置

Runtime 需要定义统一的“可用断点位置”（Breakpoint Points），建议第一阶段至少包括：

1. **loop_start**：新一轮 Agent Loop 开始时
2. **post_infer**：LLM 推理完成后
3. **pre_tool**：某个 Tool 即将执行前
4. **post_tool**：某个 Tool 执行完成后
5. **pre_next_prompt**：下一轮 Prompt 拼接完成、即将进入下一次推理前

其中，`post_infer`、`post_tool`、`pre_next_prompt` 是第一阶段最关键的三个节点。

### 8.2 默认单步调试行为

在默认单步调试模式下，推荐行为如下：

1. **LLM 推理完成自动停下**
   - 展示本次推理输入 Prompt
   - 展示 LLM 输出结果
   - 展示计划调用的 Tool，但此时 Tool 尚未真正执行

2. **点击下一步时逐个执行 Tool**
   - 每执行完一个 Tool 后再次停下
   - 展示 Tool 输入参数、日志、真实返回结果

3. **Tool 执行完成后，预览下一轮 Prompt**
   - 系统自动拼接下一轮 Prompt
   - 在进入下一轮推理前再次停下
   - 允许开发者手工修改 Prompt

这一流程形成完整的调试链：

```text
看推理结果
→ 执行 Tool
→ 看 Tool 结果
→ 看下一轮 Prompt
→ 修改
→ 继续推理
```

### 8.3 条件断点

断点不应只支持“停在某个固定事件”，还应支持“满足条件才停”。

推荐支持的条件类型：

1. **Loop 条件**
   - 第 N 轮停下
   - 每隔 N 轮停一次

2. **Prompt 条件**
   - Prompt 中包含某个关键字时停下
   - Prompt 长度超过阈值时停下

3. **Tool 条件**
   - 当 Tool 名称匹配某个值时停下
   - 当 Tool 参数包含特定字段时停下

4. **Session / State 条件**
   - Memory 某个字段变更时停下
   - To-Do 状态变化时停下

5. **Sub-Agent 条件**
   - 某个 Sub-Agent 被创建时停下
   - 某个 Sub-Agent 完成任务时停下

#### 示例

```text
break on post_infer when prompt contains "delete file"
break on pre_tool when tool == "shell_execute"
break on loop_start when loop_index == 5
```

### 8.4 断点检查伪代码

```rust
fn check_debug_breakpoint(session: &Session, event: BreakpointEvent, ctx: &RuntimeContext) {
    if !session.debug.wait_for_debug {
        return;
    }

    if breakpoint_match(session.debug.breakpoints, event, ctx) {
        pause_session(session, event, ctx);
    }
}
```

Runtime 在每个可用断点位置调用该检查函数即可。

---

## 9. 调试暂停后的可观察能力

当 Session 进入 `PAUSED_DEBUG` 后，开发者需要能看到至少以下内容。

### 9.1 Prompt 视图

应同时支持查看：

- 原始 Prompt 源文件
- 本轮实际拼接出的完整 Prompt
- Prompt 各组成部分来源
  - system prompt
  - skills
  - memory
  - worklog / history
  - tool results
  - user input
- 下一轮 Prompt 预览

重点不是只看“模板文件”，而是看：

> 这一次真正送给 LLM 的完整输入。

### 9.2 推理结果视图

应展示：

- LLM 原始返回
- 结构化解析结果
- Agent 的 message / reasoning / tool calls
- 解析错误（如果有）

### 9.3 Tool 视图

应展示：

- Tool 名称
- 输入参数
- 调用日志
- 标准输出 / 标准错误
- 执行状态
- 返回结果

### 9.4 状态视图

应展示：

- Session 元信息
- Loop Index
- Memory
- Worklog
- Workspace 文件变化
- To-Do 状态
- Sub-Agent 列表与运行状态

### 9.5 时间线视图

应能按事件顺序查看：

```text
Loop Start
→ Prompt Compiled
→ Inference Done
→ Tool Planned
→ Tool Executed
→ Next Prompt Ready
```

这能帮助开发者快速定位问题发生的环节。

---

## 10. 调试暂停后的可修改能力

调试器的价值不只是“看见”，更是“当场修”。因此在暂停状态下，应允许对当前 Session 的关键状态进行修改。

### 10.1 可修改内容

第一阶段建议支持：

- 下一轮 Prompt
- 某次 Tool 返回结果
- Memory
- Workspace 文件
- To-Do 内容与状态（谨慎）
- Skill 集合

### 10.2 Prompt 修改

Prompt 修改是最核心能力之一。系统需要支持两种方式。

#### 方式一：Runtime Override

开发者在暂停状态下直接改“下一轮将要使用的 Prompt”。其特点是：

- 只作用于当前 Session
- 默认不写回源文件
- 适合调试实验与快速修复

可抽象为：

```text
effective_prompt = source_prompt + debug_override
```

#### 方式二：Source File Edit

开发者直接修改磁盘上的 Prompt 文件。由于 Agent 每轮都会重新加载 Prompt，因此：

- 下一轮自然生效
- 适合正式修复
- 需要 IDE / Git / Diff 支持

### 10.3 Tool Result 修改

很多时候问题不在 LLM，而在 Tool 返回值异常。调试器应允许：

- 手工覆盖 Tool Result
- 标记某个 Tool 为“执行成功但返回指定值”
- 标记某个 Tool 为“模拟失败”
- 跳过某个 Tool

这样可以快速验证“如果 Tool 正常 / 异常，会怎样”。

### 10.4 Memory / Workspace 修改

应允许在暂停时：

- 修补污染的 Memory
- 修正文档、配置、缓存等 Workspace 内容
- 对比修改前后差异

### 10.5 状态修改的审计

所有调试期修改都应写入审计日志，至少记录：

- 修改时间
- 修改人 / 命令来源
- 修改对象
- 修改前后摘要

这样可避免“调试过程中改了什么没人知道”的问题。

---

## 11. Skill Injection 设计

### 11.1 能力定义

调试模式下允许手工导入一个 Skill。其本质通常是：

- 向 Prompt 中注入新的能力描述
- 增加可调用 Tool 集合
- 临时调整 Agent 的策略约束

### 11.2 使用场景

- 验证新 Skill 的 Prompt 是否有效
- 比较“注入前后”的 Agent 行为差异
- 临时补充某项能力，而不立即改正式配置

### 11.3 默认策略

建议 Skill Injection 默认仅对当前 Session 生效，不自动写回源配置。

同时可提供以下选项：

- `session-only`
- `write-back`
- `export-patch`

### 11.4 Prompt 辅助工具

为了避免手工逐字符拼接，可提供常见模板：

- Tool result 插入模板
- Memory 片段模板
- Skill 描述模板
- 常见调试约束模板

---

## 12. Sub-Agent 设计

### 12.1 Sub-Agent 的角色定位

Sub-Agent 在 OpenDAN 中不是“普通函数调用”，而是一个通过任务系统与主 Agent 协作的独立 Agent。其特点是：

- 有自己的 Session
- 可以有自己的 Workspace
- 可以共享主 Agent 的 Workspace
- 通过 To-Do System 与主 Agent 沟通
- 原则上不与主 Agent 形成直接阻塞依赖

从模型上看，Sub-Agent 更像：

> 在某个 Agent 环境中 fork 出来的子进程式工作体。

### 12.2 与主 Agent 的通信机制

两者之间唯一的正式沟通渠道是：

```text
To-Do System
```

Sub-Agent 接受一个 Task / To-Do 后，向主 Agent 汇报的核心状态通常只有：

- `Pending`
- `Running`
- `WaitingUser`
- `Completed`
- `Failed`
- `Cancelled`

主 Agent 关心的核心事实是：

- 子任务是否完成
- 是否失败
- 是否需要人工授权
- 完成后输出了什么

### 12.3 创建方式

Sub-Agent 需要支持两种创建模式。

#### 模式一：启动时创建

在加载 Agent Environment 时，用户可选择：

- 以主 Agent 模式启动
- 以 Sub-Agent 模式启动

这种方式适用于“预先就知道要跑一个子 Agent”的场景。

#### 模式二：运行中创建

在主 Agent 正常运行时，可以在任意时刻导出当前环境，并基于该环境创建一个 Sub-Agent。

这意味着：

- Sub-Agent 可以从某个明确的 Loop 节点派生
- 很像从某个运行态快照 fork 出一个新 Agent

### 12.4 Workspace 策略

建议支持两种工作区模式：

1. **独立 Workspace**
   - 适合隔离执行
   - 互不污染

2. **共享 Workspace / Observer 模式**
   - Sub-Agent 观察或协作主环境
   - 需配合权限控制，默认建议只读

### 12.5 并行执行模型

由于主 Agent 与 Sub-Agent 通过 To-Do 解耦，因此两者应可并行运行：

- 主 Agent 等待用户授权时，Sub-Agent 仍可继续
- Sub-Agent 等待用户授权时，主 Agent 也可继续
- 任一方暂停调试，不自动阻塞另一方

### 12.6 调试控制要求

调试器需要支持：

- 查看所有 Agent / Sub-Agent 列表
- 选择 attach 到某个 Session
- 单独暂停 / 恢复某个 Agent
- 在主 Agent 暂停时继续运行 Sub-Agent
- 在调试面板中查看其 To-Do 状态与输出

---

## 13. Agent Environment 与持久化结构

Agent 的运行环境应尽量以文件形式持久化，便于 IDE、Git 与调试器协同。

### 13.1 推荐目录结构

```text
agent/
  agent.yaml
  prompt/
    system.md
    skills/
  memory/
    memory.json
  session/
    session.json
    worklog.jsonl
    todos.json
  workspace/
  debug/
    breakpoints.json
    overrides/
    patches.log
```

### 13.2 持久化要求

至少需要保证以下内容可导出：

- 当前 Session 元信息
- 当前 Loop Index
- 当前 Memory
- 当前 To-Do
- 当前 Workspace
- 当前 Prompt 源文件与编译结果
- 当前调试覆盖信息

### 13.3 Snapshot

系统应支持从任意调试暂停点导出 Snapshot，用于：

- 问题复现
- 创建 Sub-Agent
- 交给他人接手调试
- 后续 Replay / Time Travel 扩展

---

## 14. 开发循环（Development Loop）

这是 OpenDAN Agent DevTools 最关键的使用流程之一。

### 14.1 标准开发循环

```text
编辑 Agent 文件
→ 以 Debug 模式启动 Agent
→ Runtime 在断点处停下
→ 查看 Prompt / Tool / 状态
→ 修改 Prompt 或环境
→ step / next / continue
→ 观察结果
→ 重复
```

也可概括为：

```text
Edit → Run → Inspect → Modify → Continue
```

### 14.2 关键判断

OpenDAN 不应试图构造一个“万能 Web IDE”。更合理的路径是：

- 文件系统承担配置与源码管理
- Runtime 承担暂停与恢复
- CLI 承担调试控制
- IDE 承担编辑体验
- Web UI 作为低门槛入口与可视化面板

---

## 15. CLI 调试器设计

### 15.1 启动方式

建议支持以下形式：

```bash
OPENDAN_DEBUG=1 opendan agent run
```

或：

```bash
opendan agent run --debug
```

启动后，Runtime 在可用断点处会按照调试配置检查是否暂停。

### 15.2 Attach 方式

```bash
opendan debug attach <session-id>
```

### 15.3 基础命令集合

#### 状态查询

```bash
opendan debug status
opendan debug prompt show
opendan debug tool show
opendan debug memory show
opendan debug workspace ls
opendan debug todo show
opendan debug agent list
```

#### 执行控制

```bash
opendan debug next
opendan debug step-tool
opendan debug continue
opendan debug pause
opendan debug stop
```

建议语义：

- `next`：推进到下一个可用断点
- `step-tool`：若当前存在待执行 Tool，则执行一个 Tool 后停下
- `continue`：继续运行直到下一个满足条件的断点
- `pause`：主动请求暂停
- `stop`：结束 Session

#### 断点控制

```bash
opendan debug break add --event post_infer
opendan debug break add --event pre_tool --tool shell_execute
opendan debug break add --event post_infer --prompt-contains "delete file"
opendan debug break list
opendan debug break clear
```

#### 状态修改

```bash
opendan debug prompt edit
opendan debug prompt override --file patched_prompt.md
opendan debug memory edit
opendan debug tool patch-result <tool-call-id> --file result.json
opendan debug workspace open <path>
opendan debug skill inject <skill-id>
```

#### Sub-Agent 控制

```bash
opendan debug subagent create --from current
opendan debug subagent create --from snapshot <snapshot-id>
opendan debug subagent list
opendan debug subagent attach <session-id>
```

### 15.4 与 IDE 的配合

CLI 的最大价值在于它非常容易与 IDE 集成。开发者可以：

- 在 IDE 中编辑 Prompt、Memory、Workspace 文件
- 用终端或插件调用 CLI 控制运行
- 在 VSCode 中以熟悉的方式查看状态、Diff、Patch

因此，VSCode 插件不需要重造调试核心，只需要围绕 CLI 和 Runtime API 做集成。

---

## 16. Web UI / Agent Console 设计

Web UI 的定位不是替代 CLI，而是提供低门槛调试入口和可视化观察面板。

### 16.1 主要能力

进入某个 Agent 的首页后，应可看到：

- 当前 Session 状态
- 当前 Loop Index
- Prompt 面板
- 推理结果面板
- Tool 调用面板
- Workspace 面板
- Memory / Worklog 面板
- To-Do / Sub-Agent 面板
- 断点配置面板
- 控制按钮：Step / Next / Continue / Pause / Stop

### 16.2 使用定位

Web UI 适合：

- 初级开发者
- 演示场景
- 轻量排障
- 教学和 onboarding

高阶调试仍建议以文件与 CLI 为主。

---

## 17. Runtime 执行流程

以下给出一次典型调试流程。

### 17.1 执行阶段

1. Runtime 载入 Agent Environment
2. Session 进入 `WAIT_FOR_DEBUG`
3. Agent Loop 正常推进
4. 在每个可用断点位置调用 `check_debug_breakpoint()`
5. 若命中条件，则 Session 进入 `PAUSED_DEBUG`

### 17.2 暂停阶段

1. Runtime 固化当前可观察上下文
2. 暴露当前 Prompt、Tool、State、Workspace 等内容
3. 接收来自 CLI / UI 的查询与修改命令
4. 将运行时补丁记录到审计日志

### 17.3 恢复阶段

1. 接收 `next / step-tool / continue`
2. 应用修改后的运行时状态
3. 恢复 Session 执行
4. 进入下一个断点检查位置

### 17.4 伪代码

```rust
loop {
    emit(loop_start);
    check_debug_breakpoint(session, loop_start, ctx);

    prompt = compile_prompt(session, env);
    emit(prompt_compiled);

    infer_result = llm_infer(prompt);
    emit(post_infer, infer_result);
    check_debug_breakpoint(session, post_infer, ctx);

    tool_calls = parse_tool_calls(infer_result);
    for tool_call in tool_calls {
        emit(pre_tool, tool_call);
        check_debug_breakpoint(session, pre_tool, ctx);

        tool_result = exec_tool(tool_call);
        emit(post_tool, tool_result);
        check_debug_breakpoint(session, post_tool, ctx);
    }

    next_prompt = build_next_prompt(session, infer_result, tool_results);
    emit(pre_next_prompt, next_prompt);
    check_debug_breakpoint(session, pre_next_prompt, ctx);

    commit_state(session, next_prompt, tool_results);
}
```

---

## 18. 对现有 Runtime 的影响

引入调试模式会对 Runtime 带来几个工程层面的变化。

### 18.1 执行过程必须可 Hook

原先一些步骤可能是“直接跑完”的，现在必须允许在中间插入：

- 事件发射
- 断点检查
- 暂停 / 恢复

### 18.2 状态必须可序列化

若想支持暂停、导出、修改、重放，运行态必须具备良好的序列化能力。

### 18.3 Prompt 编译必须可追踪

不能只保留最终字符串，最好记录：

- 每段 Prompt 来源
- 拼接顺序
- 编译结果
- Override 覆盖点

### 18.4 Tool 调用必须经过统一封装

所有 Tool 执行都应进入统一包装层，以便：

- 打点
- 捕获输入输出
- 支持跳过 / 覆盖结果
- 支持断点

### 18.5 并发与锁

当一个 Session 处于 `PAUSED_DEBUG` 时，应避免多个控制端并发写同一状态造成冲突。建议：

- attach 后获取调试控制权
- 关键修改走事务式提交
- 非修改查询可共享

---

## 19. 安全性与一致性要求

### 19.1 默认只在开发 / 调试环境开放

调试接口应默认只对本地开发环境或明确开启的受控环境开放。

### 19.2 所有修改可审计

所有运行时修改都应写入补丁日志，便于追踪“为什么行为变了”。

### 19.3 写回动作需明确确认

Runtime Override 默认不写回源文件。若要写回 Source，应显式执行保存动作。

### 19.4 Sub-Agent 共享 Workspace 的权限控制

共享 Workspace 时，建议默认只读；若允许写入，必须有明确权限模型。

---

## 20. MVP 范围

第一阶段建议只做最关键、最闭环的能力。

### 20.1 MVP 必做

1. Runtime 增加 `wait_for_debug` 状态
2. 增加统一 `check_debug_breakpoint()` Hook
3. 实现三个核心断点：
   - `post_infer`
   - `post_tool`
   - `pre_next_prompt`
4. CLI 支持 attach / status / next / continue
5. 支持查看完整 Prompt、推理结果、Tool 结果
6. 支持对下一轮 Prompt 做 Runtime Override
7. 支持基础条件断点（Loop / Tool / Prompt Contains）
8. 支持导出当前 Session Snapshot

### 20.2 第二阶段

1. Web UI Agent Console
2. Memory / Workspace / To-Do 编辑
3. Tool Result Patch
4. Skill Injection
5. 更完整的断点表达式

### 20.3 第三阶段

1. Sub-Agent 图形化管理
2. 从 Snapshot Fork Sub-Agent
3. VSCode Extension
4. Replay / Time Travel Debugging
5. 多 Agent 运行时间线视图

---

## 21. 关键收益

如果这套设计落地，OpenDAN 将获得一套区别于普通 Prompt 平台的核心开发能力：

### 21.1 对开发者

- 更快定位 Prompt、Tool、Memory 问题
- 不必反复重启整局 Session
- 可在真实运行态中直接修复问题
- 可用熟悉的 IDE 和 Git 工作流开发 Agent

### 21.2 对平台

- 降低 Agent 开发门槛
- 提高调试效率与问题复现能力
- 为后续高级能力打基础
  - Replay
  - Snapshot
  - Fork
  - Time Travel
  - IDE Extension

### 21.3 对产品定位

这会让 OpenDAN 的 Agent 能力从“可运行”进化为“可工程化开发”。

---

## 22. 总结

本设计的核心判断是：

> OpenDAN Agent 的调试器，本质上不是一个花哨 UI，而是一个下沉到 Runtime 的 Agent Loop Debugger。

它的最小闭环非常明确：

```text
设置调试模式
→ 在断点处自动停下
→ 查看当前 Prompt / Tool / 状态
→ 手工修改运行时状态
→ 继续执行到下一步
```

也就是：

```text
Stop → Inspect → Modify → Continue
```

从实现上看，最关键的基础只有三件事：

1. 在 Session 中引入 `wait_for_debug`
2. 在 Runtime 的可用断点位置统一调用 `check_debug_breakpoint()`
3. 在暂停状态下允许查询与修改可编辑状态

只要这三件事成立，CLI、Web UI、VSCode 插件、Sub-Agent 调试、Snapshot、Replay 等能力都可以在其上逐步生长。

最终，OpenDAN 的 Agent 开发体验应当形成如下格局：

```text
文本文件负责定义 Agent
Runtime 负责承载 Agent
Debugger 负责控制 Agent
IDE / UI 负责让人更舒服地使用这些能力
```

这就是 OpenDAN Agent DevTools 的整体方向。
