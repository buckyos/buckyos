# OpenDAN Agent Prompt 编排架构

本文说明当前 OpenDAN / `llm_context` 的提示词编排理念和接入方式。

早期版本把这一层称为 **Prompt Compiler**，并讨论过“固定三段式 input”“memory 大块”“step 三级压缩”等方案。当前实现已经收敛到更清晰的分层：提示词模板渲染和业务上下文装配属于上层 L4，`llm_context` 只接收已经编译好的 `AiMessage` 列表，并负责 LLM 执行、工具循环、snapshot/resume 和上下文超限信号。

对外部开发者来说，核心不是实现一个新的全局 Prompt Compiler，而是理解：

- 哪些内容应在进入 `llm_context` 前被编译成 prompt；
- 哪些历史由 `llm_context` 的运行态自然累积；
- context window 压力出现时，谁负责压缩、如何 resume；
- 如何用模板、section、预算和 loader 组合出稳定的提示词输入。

## 1. 当前理念

当前架构遵循几个原则。

**`llm_context` 是 waist，不是业务 prompt 编译器。**

`llm_context` 的输入是 `LLMContextRequest.input: Vec<AiMessage>`。这组 message 被视为“已经编译好的对话历史 / 初始提示词”。模板展开、读取工作区、整理 session 信息、选择 memory 片段等业务行为，都应在调用 `LLMContext::new` 之前完成。

**Prompt 编排是 L4 的职责。**

OpenDAN Agent、OneShot、本地工具调用、未来 Workflow DSL 都可以复用同一个 `llm_context`，但它们对 prompt 的组织方式不同。因此 prompt 编译策略必须留在各自 L4 中，而不是塞进 waist。

**历史不是单一来源。**

当前系统里至少有三类历史：

- `state.accumulated`：LLM 实际看过的 message 历史，包含 system/user/assistant/tool。
- `state.steps` / `state.last_step`：Behavior Loop 的结构化 step 历史。
- OpenDAN round history / worklog / session 文件：审计、调试、恢复和后续召回的外部事实源。

这些历史不应被强行合并成一个“memory 大块”。应该按用途分别处理。

**压缩策略属于调度层。**

当模型上下文接近上限时，`llm_context` 只负责产出 `ContextLimitReached` 这个事实信号。具体是 LLM 摘要、保留尾部、硬截断、失败退出，还是换大窗口模型，应由 L4 决定，并通过 `ResumeFill::RewrittenHistory` 喂回。

## 2. 当前分层

```text
Agent / Workflow / OneShot L4
  ├─ 读取配置、session、workspace、memory、业务状态
  ├─ 渲染模板：PromptRenderEngine + ValueLoader
  ├─ 可选：按 section 做 token budget 装配
  └─ 输出 Vec<AiMessage>

llm_context
  ├─ 接收 LLMContextRequest.input
  ├─ 运行 LLM / tool loop / behavior loop
  ├─ 维护 accumulated、steps、last_step
  ├─ 产出 snapshot / outcome
  └─ 在 context 压力下产出 ContextLimitReached

L4 outcome driver
  ├─ 持久化 snapshot
  ├─ 处理 PendingTool / Interrupted / Done / Error
  ├─ 处理 ContextLimitReached
  └─ 需要时压缩历史并 resume
```

这个分层的结果是：外部开发者不需要理解 OpenDAN 内部所有 session 数据结构，也能基于 `llm_context` 构建自己的 prompt 编排层。

## 3. `LLMContextRequest.input`

`LLMContextRequest.input` 是进入 waist 的边界。它通常包含：

- 一段或多段 `System` message：角色、行为约束、输出协议、工具使用规则。
- 当前用户输入或任务输入：通常是 `User` message。
- 必要的环境上下文：例如当前 workspace、behavior、session 标识、最近活动。
- 需要直接进入模型上下文的业务资料：例如文档片段、任务说明、外部检索结果摘要。

`llm_context` 不会再帮这些内容做模板展开。它只会把 `input` 克隆为初始 `state.accumulated`，并在运行过程中继续追加 assistant/tool/user observation。

因此，接入方要保证传入的 `input` 已经满足模型可读性、安全性和预算要求。

## 4. 模板渲染工具

当前通用模板引擎在 `llm_context::prompt_engine` 中。

它不是 OpenDAN 专用引擎，不认识 session、workspace、todo、owner。所有动态值都通过调用方提供的 `ValueLoader` 注入。

支持的指令是：

| 指令 | 作用 |
| --- | --- |
| `__ENV($key)__` | 从 `RenderVars.env`、`RenderVars.vars` 或 `ValueLoader` 取值并内联 |
| `__INCLUDE(path)__` | 在 `include_roots` 白名单内读取文件并内联 |
| `__EXEC(cmd)__` | 执行 shell 并内联 stdout，默认关闭 |
| `__VAR(name, $expr)__` | 解析动态值并注册为 upon 变量 |
| `{{ name }}` | upon 模板变量输出，支持条件和循环 |

典型接入方式：

```rust
let engine = PromptRenderEngine::new(EngineConfig {
    include_roots: vec![agent_root.clone(), workspace_root.clone()],
    allow_exec: false,
    ..EngineConfig::default()
});

let vars = RenderVars::new()
    .with_env("session_id", session_id)
    .with_env("behavior", behavior_name);

let result = engine
    .render(template, &vars, &open_dan_value_loader)
    .await?;
```

如果模板要访问 OpenDAN 私有数据，例如 `$new_msg`、`$workspace/README.md`、`$owner.show_name`，应由 OpenDAN 自己实现 `ValueLoader`。不要把这些变量名放进 `llm_context`。

## 5. Section 组合与预算

复杂 prompt 不建议拼成一个大字符串。更推荐拆成多个 section：

- `system.identity`
- `system.behavior`
- `environment`
- `memory.summary`
- `workspace.context`
- `user.input`
- `debug.trace`，如有必要

每个 section 可以有自己的：

- role：`System` / `User` / `Assistant` / `Tool`
- template
- priority
- min token floor
- truncation 策略：保留头部、尾部或首尾
- local vars

`llm_context::prompt_compose` 提供了 `SectionSpec` 和 `compose(...)`，会按顺序完成：

1. 对每个 section 调用 `PromptRenderEngine::render`；
2. 把渲染结果交给 `PromptBudgeter`；
3. 在总 token budget 内裁剪或丢弃低优先级 section；
4. 输出最终 `Vec<AiMessage>`。

这适合外部开发者构建自己的 Agent、Workflow 或报告生成器。OpenDAN 自己也应在接入 prompt 渲染时复用这条流水线，而不是重新发明一套模板系统。

## 6. Behavior Loop 的历史

Behavior mode 与传统 chat/tool loop 不同。它把 LLM 输出解析成 `StepRecord`，并维护：

- `state.steps`：已经沉淀的历史 step；
- `state.last_step`：最新 hot step，会在下一轮完整渲染；
- `StepRenderer`：把 step 历史重新渲染成 `AiMessage`。

内层推理请求大致是：

```text
request.input
  + StepRenderer.render_history(state.steps)
  + StepRenderer.render(state.last_step)
```

这保留了早期文档里“最近 step 权重最高”的思想，但实现方式已经不同：不是一个固定 XML 大块，也不是全局 Prompt Compiler，而是 behavior loop 内部的 renderer 机制。

Behavior Loop 还预留了 `HistoryCompressor` 扩展点。它用于压缩 `state.steps` 这个 step 维度的历史。这个 compressor 是可选依赖，不是 `llm_context` 的内置策略。

## 7. `accumulated` 历史与 context limit

传统 Agent Loop 和 Behavior Loop 最终都会维护 `state.accumulated`。这是 provider 实际看到过的 message 历史，也是 snapshot/resume 的核心部分。

当 `llm_context` 产出 `ContextLimitReached` 时，L4 应选择一种压缩策略：

- OneShot 当前默认使用旁路 LLM，把中间历史摘要成 `[Conversation summary]`，保留 leading system 和最近若干轮对话。
- OpenDAN session 当前使用启发式 message-level 压缩：保留 leading system，丢弃中间，保留最近 tail，并插入一条说明压缩发生的 synthetic user message。
- Workflow 可以选择直接失败、换模型或走上游 retry。

压缩完成后，L4 通过：

```rust
LLMContext::resume(
    snapshot,
    ResumeFill::RewrittenHistory { history: rewritten },
    deps,
)
```

继续执行。

因此，context 压缩不是 prompt compiler 的一部分，而是 outcome driver 的一部分。

## 8. Memory 的定位

当前理念中，memory 不应被理解为“把所有历史塞回 prompt 的地方”。

更合理的分工是：

- `accumulated` 保存模型真实经历过的短期上下文；
- `steps` / `last_step` 保存 Behavior Loop 的结构化执行链；
- memory 保存跨轮次、跨 session 可复用的稳定事实；
- worklog / round history 保存审计和调试事实；
- session/workspace 文件保存任务目标、文件地图、长期产物。

进入 prompt 的 memory 应该是经过选择和预算控制的片段，而不是完整数据库 dump。

外部开发者在实现自己的 memory 接入时，应优先回答：

- 这段信息是否必须让模型本轮看到？
- 它应该作为 system 约束、user context，还是 tool observation？
- 它被截断后是否仍然安全？
- 它是否可以通过 `ValueLoader` 按需加载？
- 它是否应该由旁路 LLM 先摘要？

## 9. 外部开发者接入建议

如果你要为 OpenDAN 写新的 Agent、Workflow 或 LLM 应用，推荐按下面的方式组织。

### 9.1 定义 prompt sections

先把 prompt 拆成稳定 section，而不是直接拼字符串。

示例：

```text
system.identity       高优先级，不截断
system.behavior       高优先级，不截断
environment           中高优先级，保留尾部
workspace.summary     中优先级，保留头部
memory.recall         中优先级，允许截断
user.input            最高优先级，不截断
```

### 9.2 实现 `ValueLoader`

把业务变量统一封装到 loader：

```text
session.title
session.current_behavior
workspace.summary
workspace.file.<path>
memory.recall
owner.show_name
recent_messages
```

loader 可以读 DB、文件、远程服务，也可以调用旁路 LLM。模板引擎不关心来源。

### 9.3 配置安全边界

`__INCLUDE` 必须设置 `include_roots`，不要允许任意绝对路径。

`__EXEC` 默认关闭。只有在明确需要、且执行环境可控时才打开。

### 9.4 保持输入可审计

最终进入 `LLMContextRequest.input` 的 message 应能被记录和复现。不要在 provider adapter 内部临时拼不可见 prompt。

### 9.5 把压缩当成运行期策略

不要试图在初始 prompt compiler 里解决所有历史膨胀问题。初始 prompt 只负责当前轮需要的静态和动态上下文；运行过程中产生的历史，由 snapshot、ContextLimitReached 和 L4 compressor 处理。

## 10. 与早期“三种范式”的关系

早期文档讨论过三种范式：

1. 传统消息追加式；
2. 固定三段式编排；
3. 分层时间压缩式。

当前实现吸收了其中部分理念，但没有采用旧设计作为协议。

保留下来的思想：

- 最近历史和 last step 应有更高权重；
- 长期记忆和短期强时序历史应分开；
- prompt 需要预算控制；
- 历史压缩应明确可审计。

已经放弃或下沉的部分：

- 不再要求全局固定三段式 input；
- 不再把 memory 作为所有历史的唯一承载区；
- 不再定义 OpenDAN 专用 Prompt Compiler 作为 waist 的组成部分；
- 不再把 context 压缩写死在 prompt 编译阶段。

现在的核心架构是：

```text
L4 编译当前输入
  → llm_context 运行并累积历史
  → snapshot 持久化
  → context 超限时 L4 压缩并 resume
```

这比早期 Prompt Compiler 方案更适合多种上层应用共用 `llm_context`：Agent 可以保留行为状态机，OneShot 可以使用旁路 LLM 摘要，Workflow 可以选择失败或重试，而不会被同一个 prompt 编排协议绑死。

## 11. 当前实现状态

已经存在的通用能力：

- `llm_context::prompt_engine`：纯模板渲染。
- `llm_context::prompt_budget`：section token 预算与裁剪。
- `llm_context::prompt_compose`：render + budget + `AiMessage` 装配。
- `LLMContextSnapshot`：完整保存 request 与 runtime state。
- `ResumeFill::RewrittenHistory`：支持 L4 压缩后恢复。
- Behavior Loop 的 `StepRenderer` / 可选 `HistoryCompressor`。
- OneShot 的旁路 LLM summary compressor。
- OpenDAN session 的 message-level context limit compressor。

仍需接入或完善的部分：

- OpenDAN behavior 的 `system_prompt_template` 当前仍需要接入 `PromptRenderEngine`。
- OpenDAN 私有 `ValueLoader` 需要明确支持哪些 session/workspace/memory key。
- OpenDAN prompt section 构造层需要把 behavior、environment、memory、workspace 等拆成 `SectionSpec`。
- Behavior Loop 的 `HistoryCompressor` 是否接入 OpenDAN 主路径，需要根据实际 token 压力和模型表现再决定。

## 12. 术语小结

| 术语 | 当前含义 |
| --- | --- |
| Prompt 编排 | L4 把业务上下文编译成 `Vec<AiMessage>` 的过程 |
| PromptRenderEngine | 通用模板引擎，不含 OpenDAN 业务语义 |
| ValueLoader | 调用方提供的动态变量解析器 |
| SectionSpec | 可预算、可裁剪、可分 role 的 prompt 片段 |
| accumulated | LLMContext 运行中真实累积的 message 历史 |
| StepRecord | Behavior Loop 的结构化 step 记录 |
| last_step | Behavior Loop 最新 step，下一轮完整保留 |
| snapshot | 可序列化恢复点，包含 request 和 state |
| ContextLimitReached | waist 发出的上下文压力事实信号 |
| RewrittenHistory | L4 压缩后喂回 waist 的新 message 历史 |

## 13. 结论

当前 OpenDAN 的提示词编排不再追求一个中心化、OpenDAN 专用的 Prompt Compiler。新的方向是：

- 用通用模板引擎解决文本渲染；
- 用 section 和预算解决 prompt 组成；
- 用 `ValueLoader` 解决业务数据接入；
- 用 `LLMContext` 管理执行、历史、工具和 snapshot；
- 用 L4 outcome driver 决定压缩和恢复策略。

这套分层让外部开发者可以清楚地选择自己的接入位置：简单应用只需要准备 `Vec<AiMessage>`；复杂 Agent 可以使用模板和 section；长期运行的 Agent 再接入 memory、snapshot 和 compressor。
