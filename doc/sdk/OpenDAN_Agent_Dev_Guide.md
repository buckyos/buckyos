# OpenDAN Agent 开发指南

面向 Agent、Behavior、Skill 和运行环境的开发者。本文规定能力扩展的设计目标，可以领先现有实现；标为“当前实现”的内容以 2026-10-11 的仓库源码为准，实现差距见 §6。

核心原则：**尽量让所有 Agent 能力扩展都表现为 CLI 环境的扩展，保持模型可见的 tool 定义、action 协议和公共 system prompt 前缀稳定。**

## 1. 稳定模型接口，通过 CLI 扩展能力

### 1.1 为什么同时限制 tool 和 action 的变化

原生 function-call tool 的名称、参数 schema、description、排列顺序，以及 action 的标签、参数表达和用法说明，都会参与模型输入的构造。频繁增删或重写这些内容会改变可缓存的前缀。Action 还依赖提示词中的 XML 用法，扩展时需要同步维护提示词和解析约定。

因此，原生 tool 有参数 API 支持，也不意味着它适合作为日常能力扩展入口。设计目标是：

- tool 和 action 都作为少量、稳定的协议入口维护；新增业务能力默认不增加 tool 或 action。
- 新能力通过安装 CLI、增加子命令、提供脚本或连接后端服务进入执行环境。
- 能力发现和详细用法通过 CLI 查询、`--help`、Skill 文档按需读取，作为当前上下文的增量信息返回。
- 新增 CLI、Skill、MCP 服务或可调用 behavior，不自动重建 tool/action 定义，也不把完整能力清单重新写入公共 system prompt。
- 保持已有稳定入口也是目标；CLI 化不要求为了统一形式反复删改已经稳定的 tool/action 集合。

**只把 tool schema 改写成 system prompt 里的 CLI 手册，仍然会改变公共前缀。** 需要一并稳定工具定义和公共提示词，把动态发现结果留在运行中的消息里。不同任务或 behavior 所需的正文可以变化，但能力扩展不应附带重写公共协议。这里的目标是减少无谓的前缀变化，不承诺具体模型或 provider 的 cache 命中率。

### 1.2 各层职责

| 层级 | 负责什么 | 能力扩展方式 |
|---|---|---|
| tool / action 协议 | 模型如何请求执行、传递内容、接收结果 | 少量固定入口，作为协议变更管理 |
| CLI 环境 | 当前执行环境中有哪些命令及其用法 | PATH、命令、子命令、脚本、服务客户端 |
| Skill | 何时使用能力、工作流程、领域知识 | 按需读取说明和资源，调用 CLI |
| Behavior | 当前目标、决策规则、上下文切换方式 | 调整任务编排，不自动扩张工具集合 |
| runtime / 宿主 | 执行、授权、task、状态提交和恢复 | 提供稳定的执行与控制接口 |

工具列表的稳定性不代替权限控制。命令能否执行、能访问哪些资源，应由 runtime、命令后端和宿主校验；拒绝结果通过稳定的结果协议返回。不同权限环境可以使用预先定义的固定接口集合，不应因某个 Skill 的加载临时增删模型工具。

## 2. CLI 能力的开发约定

### 2.1 命令入口与发现

一个新能力应优先提供可独立调用的命令或已有 CLI 的子命令，并提供简短帮助、明确参数和退出码。命令由实际执行环境提供；不能只把命令名写进提示词而没有可执行入口。

公共提示词只需要说明稳定的执行入口和能力发现方式。动态命令目录、可用 behavior 列表、服务清单和详细参数，通过命令查询或读取文件获得。本文不规定新的发现命令名，具体命令及其可用性由实现提供。

MCP 和进程内 host tool 也遵循同一方向：优先由 CLI 客户端或宿主代理访问后端能力。外部服务增加一个方法，不应自动让模型请求增加一个 function-call 定义或 XML action。

### 2.2 输入、输出和宿主桥接

- 大段文本通过 stdin 或文件传入，避免把复杂内容全部塞进 argv。`write_file` 的 `--content-stdin`、`edit_file` 的 `--new-string-stdin` 已提供这种入口。
- 面向 Agent 的结构化结果遵循 [AgentToolResult 协议](../llm_context/agent_tool/agent_tool_result_protocol.md)，保留成功、失败、task ID 和等待状态；诊断信息写 stderr。
- 普通命令可以输出文本。需要让宿主解释结构化结果的命令，stdout 应只包含协议约定的结果，不能混入日志。
- 需要当前 run 的内存状态或控制权时，通过宿主桥接访问。CLI 是接口形态，不要求所有后端状态都存在于 CLI 子进程中。
- 桥接必须保留调用所属的 run、call 身份，以及存在时的 Session 身份；由拥有状态的执行器校验请求、提交变更和处理恢复，CLI 不直接改写驱动者持锁维护的状态。

当前 `shell` 已支持识别简单命令 stdout 中的 `AgentToolResult` 并转发；含管道等 shell 运算符的复合命令不走同一结构化直传路径。依赖 `Pending` 等控制语义时，应使用直接的 CLI 调用，见 [ShellTool 实现](../../src/frame/agent_tool/src/llm_bash.rs)。

### 2.3 访问 BuckyOS 的工具：TypeScript + buckyos-websdk

访问 BuckyOS 的新工具优先用 TypeScript 基于 buckyos-websdk 编写（Deno 运行），源码随 runtime 交给 Agent：Agent 读工具源码就学会了怎么调 BuckyOS，可以在此基础上写派生脚本组合能力，比只读手册或用管道串 CLI 更可控。不优先新增 Rust `agent_tool` 子命令或 shell 薄包装。落点分工：

| 工具类型 | 落点 | 运行时可读位置（aios 容器） |
|---|---|---|
| 人也会用的系统、App、发布、Agent 管理命令 | websdk `cli/modules/*.ts`（`buckyos` 命令，唯一源码真相源） | `/opt/buckyos/libexec/buckyos-tool/cli/`（宿主同版本只读挂载），SDK 类型 `dist/node.d.ts` |
| Agent 专用的配方类工具（AIGC、媒体合成等） | `src/tools/buckyos-agent`（每个命令是 6 步配方，`commands/gen_image.ts` 是范本） | 容器：`/opt/buckyos/bin/opendan/buckyos-agent/`，`deno.json` 的 `buckyos` 指向挂载的 SDK；宿主节点：`$BUCKYOS_ROOT/libexec/buckyos-agent/`，由 `bin/agent-tools/` 下的 `opendan` 入口用 Tool 自带的 Deno 运行 |
| 某个 cmd_skill 自带的命令 | skill 包的 `tools/<skill>/bin` | `$OPENDAN_AGENT_ROOT/tools/<skill>/bin`，只在选了该 cmd_skill 的 Session PATH 上 |

xllm 等 runtime 层能力保留 Rust 实现。Agent 派生的脚本写到 `$OPENDAN_AGENT_ROOT/tools/`（在 PATH 上、长期保留）或当前 Session 目录（一次性），TS 脚本用 `deno run --config <安装态 deno.json>` 复用 SDK 映射，不改只读安装副本。命令能被发现不代表当前身份有权限或 provider 支持；手册要说明真实的拒绝 / 不支持结果。

### 2.4 cmd_skills：随 Session 冻结的命令手册

cmd_skill 是一类事的命令手册（`cmd_skills/<name>.md`，front matter 写 `name` / `description` / `requires` / `baseline` / `platforms`）加可选的命令（`tools/<name>/bin`）。它是捷径不是规定：手册写“可以这样做、这样做更快更稳”，Agent 有更合适的做法时可以不用。

- 适用环境：`baseline: standard | extended`（缺省 standard）说明手册在哪种 session-runtime 上有效——standard 只用标准环境的命令（清单见 `xagent runtime baseline`：`agent-session`、`llm_understand_media`、`agent_tool xllm`、`xagent`、`buckyos`、`aicc-tool` 及其媒体命令），用到 ffmpeg、ImageMagick、构建工具链等第三方工具的写 `extended`；`platforms: linux, macos, windows`（逗号分隔，缺省全部）。`requires` 仍是给人读的说明，具体工具名写在正文，由 Agent 运行时自查。这两个字段不是这几个固定词时安装被拒。
- 生效：Session 创建冻结 cmd_skills 时按其 session-runtime 声明的基线与平台过滤（Workspace 的 runtime 不参与）；不满足的手册不注入，在 `## Commands` 末尾列出原因。Work / Todo 显式点名的手册不满足时直接拒绝创建（Todo 留在 pending 并记下原因）；继承来的在子 Session 的 runtime 上重新解析。容器声明为 extended，节点 runtime 由 owner 在 Agent 设置里声明（缺省 standard）。

- 选择：Work 在创建时选定，最多 8 个（`agent-session create-worksession --cmd-skill <name>`，可重复），由意图分析、Goal、PLAN/DO 从 `agent-session skill list` 中挑选；未指定时子 Work 继承父 Work 的选择，UI 的 `ui_core` 与 Goal 的 goal_skills 不会自动注入 Work。UI Session 用 `agent.toml [session.<class>] default_cmd_skills`。
- 装配：选中的手册按顺序整篇进入系统段末尾的 `## Commands`，随 Session 冻结；运行中不切换、不追加，安装 / 更新 / 删除只影响之后创建的 Session。
- 安装：skill 包是 `app_type = agent_skills` 的 PIKG（`buckyos pikg init --kind agent-skills`），`agent_skills` 子包内分 `cmd_skills/`、`goal_skills/`、`tools/`。用户用 `buckyos skill install <x.pikg> --agent <agent_id>`（opendan kRPC `skills.install`），Agent 用 `agent-session skill install <x.pikg | dir>`；安装记录在 `.meta/skills.json`，Agent 模板升级不覆盖、不删除已安装的 skill。Agent 模板也可以在自己的 PIKG 里带 `agent_skills` 子包，随模板同步。
- 维护：手册是对应功能的交付物，改 CLI / 协议 / 运行路径时同步更新，见仓库根 `AGENTS.md` 的“cmd_skills 联动维护”。

## 3. xllm 是最小可用层

### 3.1 独立 xllm 应具备的能力

用户可以只使用 xllm 推进一个 LLM context，不创建 OpenDAN Session。CLI 优先的能力体系必须从这一层成立：

- 执行命令、读取和写入文件。
- 启动长命令，并取得 task ID。
- 查询、等待和取消当前 runtime 管理的 task。
- 按需发现执行环境中的命令和用法。

这些能力不应以 `agent-session`、SessionRunner 或在线 BuckyOS task-mgr 为前提。需要上层状态的 CLI 可以依赖对应宿主，但通用 runtime task 的 CLI 入口应由 xllm/runtime 层提供。

### 3.2 当前 wait_task 的语义

**当前实现：独立 xllm 已允许模型调用 `wait_task`。** 启用工具、选择内置 `bash` 组且 `shell.mode = auto` 时，会同时注册 `wait_task`、`get_task_state`、`cancel_task`。默认 shell 模式为 `auto`。

| 情况 | 行为 |
|---|---|
| `auto` 模式下 `shell` 命令在 `wait_ms` 内完成 | 直接返回结果 |
| `auto` 模式等待到期，命令仍未完成 | 返回 task ID 和已有输出，模型可继续查询或等待 |
| 独立 xllm 调用 `wait_task` | 默认等 30 秒，单次工具内等待上限 30 分钟，同时受 run deadline 和打断约束；完成则返回结果，否则返回当时状态 |
| `allow_deferred = false` | 在调用内等待，不把当前 run 挂起交给上层；不表示禁用 `wait_task` |
| 宿主允许 deferred，且请求等待超过工具内上限 | `wait_task` 可以返回 `Pending`，由宿主承担后续等待和恢复 |

工具内等待、宿主持久挂起是两个层次。独立 xllm 无须依赖上层 Session 就能等待 task；支持等待也不自动承诺 task 在执行器退出后继续运行或可恢复。对 `wait_task` 而言，等待到期或取消等待不等于取消 task；停止任务应通过显式取消请求完成，不支持取消时应明确返回这一结果。

### 3.3 task 能力 CLI 化的目标

目标是让模型通过稳定的 shell 入口调用任务命令。**已实现**（2026-10-17，[先行 TODO](../../notepads/llm-context-chat-prompt-prereq-todo.md) P3）：在 Agent Session 中是

```xml
<actions>
  <shell>agent-session task state local:shell:example</shell>
  <shell>agent-session task wait local:shell:example --wait-ms 25000</shell>
</actions>
```

（还有 `agent-session task cancel <id>`）。run 的 task CLI 桥（`agent_tool::task_bridge`）在 Unix socket 上服务本 run 的 task resolver，端点经 `XLLM_TASK_ENDPOINT` 传给命令，结果文本与 task 工具逐字一致；内置组 `bash_core` 在宿主给出 `XllmDeps.task_cli` 时不再带三个 task 工具。独立 xllm 没有这个命令，仍用 `bash` 组的 task 工具。

命令必须连接当前 xllm/runtime 的同一个 task resolver，复用状态、输出、等待和取消语义。不能简单把 `get_task_state` 别名到现有 `check_task`：后者查询 BuckyOS task-mgr，不覆盖当前 runtime 的 `local:` task；现有同名 CLI `cancel_task` 也有这一差别。

SessionRunner 可以为这个基础层增加宿主任务解析、进程外等待和持久恢复。桥接入口不应反向要求独立 xllm 先创建 Session。

实现 CLI 入口并验证等价性之后，再安排模型工具/action 集合的一次性收敛；不能先移除 task 工具而让长命令失去跟进入口。

## 4. 宿主控制能力如何 CLI 化

“依赖宿主内存”不是无法 CLI 化的理由。当前没有发现必须随业务增长而新增 tool/action 的内置能力；需要单独设计的是执行入口、结果协议和当前上下文的控制语义。

| 能力 | CLI 化后仍须保留的语义 | 当前边界 |
|---|---|---|
| runtime task 查询、等待、取消 | 同一 resolver、task 身份、等待上限、打断与结果读取 | 模型已有工具；缺少等价 CLI 桥 |
| `report` / `<report end="true">` | 提交报告、固化产物；最终报告还需校验未完成工作、完成当前上下文并停止执行后续调用 | 没有提交当前 run 报告的 CLI；写出 `report.md` 不等于提交 |
| `call_behavior` | 当前 Session 内的子 context 调用栈、fork 历史、调用身份、挂起与结果回填 | 没有等价 CLI；`create-worksession` 创建独立子 Session，不能替代 fork / create_sub_context |
| `next_behavior` / `WAIT_USER_MSG` | 在规定的提交点切换、等待或关闭 Turn，遵循当前 Session 策略 | 当前属于回复协议字段，没有等价 CLI |
| MCP / 自定义 host tool | 后端调用、错误和结果语义；需要时关联当前 run | 默认 CLI 尚无通用动态接入桥 |

`report` 和 `next_behavior` 不只是普通业务动作：前者同时有原生 tool 和回复字段形式，后者是回复控制字段。可以设计 CLI 提交控制请求，但应由宿主确认和执行；普通 stdout 文本不能被当作已经完成提交或切换。

控制请求还需定义幂等、checkpoint、恢复和同一批次中后续调用的处理。本文不新增控制命令名称，也不要求立即撤掉现有稳定控制协议。

已有可复用的例子是 `agent-session create-worksession --wait` 和 `agent-session wait <sid>`：CLI 返回带 `session:<sid>` 的 `Pending` 结果，shell 转发给宿主处理。因此，“需要挂起”本身也不构成 CLI 化障碍。见 [xagent 实现](../../src/frame/lib_opendan/src/bin/xagent.rs)。

## 5. 当前配置：用于理解存量实现

本节记录当前配置语义，不把“每个 behavior 动态组装不同工具集”作为能力扩展目标。

### 5.1 Behavior 的 capabilities

```toml
[prompt]
mode = "behavior"

[capabilities]
tool_whitelist = []
action_whitelist = ["group:bash"]
```

当前 Jarvis 的 plan / do 使用这类配置，且在 Agent 的基础 `[llm_context.tools]` 中设置 `enabled = true`。

| 字段或写法 | 当前含义 |
|---|---|
| `tool_whitelist` | 映射到 `.llm_context` 的 `tools.tools`，作为原生 function call 暴露 |
| `action_whitelist` | 映射到 `tools.actions`，作为 behavior XML action 暴露 |
| 不写某个 whitelist | 保留 Session 中对应的基础配置 |
| 显式 `[]` | 清空对应列表，不补回默认组 |
| `group:bash` | 唯一内置组：`shell`、`read_file`、`write_file`、`edit_file`；auto 模式另加三个 task 工具 |
| 普通名称 | 查询宿主注册的具名工具，不是在内置组中挑选同名成员 |
| `call_intent = true` | 只用于 function_call：宿主给每个原生函数的 schema 加 required 的 `intent`，执行前校验（缺失 / 空白 / 非字符串 → 可修正错误，调用不执行），交给工具前去掉，原始参数保留在快照与 worklog；Jarvis `plan_simple` 使用 |

因此，上述配置的实际效果是无原生 tools，整个 bash 组作为 actions 提供，包含 task 工具。**这是当前实现与 CLI 优先目标之间的差距，不能据此继续把所有新能力展开成 action。**

当前独立 xllm 默认关闭工具；只启用工具但不指定原生列表时，默认使用 bash 组。function_call 模式拒绝非空 actions 和 `tools2actions = true`；behavior 模式可以同时使用两类入口。不要从 Jarvis 的基础配置推断所有 xllm 调用都默认开放工具。

普通名称只查宿主注册表，所以直接写 `"shell"` 或 `"read_file"` 不能替代 `"group:bash"`，除非宿主另行注册。MCP 来源需要通过底层 `.llm_context` 的 `{mcp: "服务地址"}` 配置，这两个 whitelist 字符串列表没有直接 MCP 写法。

配置映射见 [BehaviorConfig](../../src/frame/lib_opendan/src/protocol/behavior.rs)，展开和校验见 [xllm](../../src/frame/agent_tool/src/xllm.rs)。完整配置见 [Agent State Protocol §7](<../opendan/protocol/Agent State Protocol.md>) 和 [本地 llm_context 配置协议](../llm_context/local_llm_context_protocol.md)。

## 6. 实现差距与迁移顺序

### 6.1 已具备的基础

- `agent_tool` CLI 已覆盖文件读写、搜索、对象访问、todo、memory、notebook、attention 相关能力；另有模型调用和媒体分析子命令。
- `agent-session` 已提供子 Session 创建、等待、输入投递，以及 `ctl subscribe/unsubscribe` 等控制命令。事件订阅不能再仅因涉及 Session 状态就被视为必须保留独立 action。
- shell 已有结构化结果转发；CLI 发起子 Session 等待已有实际路径。

### 6.2 尚需收敛的地方

1. ~~在 xllm/runtime 层补齐 runtime task CLI 桥，复用现有 resolver~~（已完成：`agent_tool::task_bridge` + `agent-session task`）；上层再提供 Session 控制桥。
2. 为动态能力提供 CLI 接入和发现路径。当前 `agent_tool` CLI 仍有静态命令表；独立可执行命令可通过执行环境提供，不能把新增能力仅注册到模型 tool 列表。
3. 将动态目录移出公共模型接口：当前 `call_behavior` 的 description 会拼入可调用 behavior 列表；`bash_tools` 手册会写入 system prompt。这两处即使不增加 tool 名，也会改变模型输入前缀。UI Session 已改为宿主关掉 xllm 的 `capabilities` / `cmd_manual` / `runtime_protocol` 段（`XllmDeps.host_prompt_sections`），命令手册改由随 Session 冻结的 cmd_skills（`## Commands`，见 §2.4）承担：冻结后同一 Session 内前缀不变，换 Session 才变；Work 也按创建时选定的 cmd_skills 装配（UI 的运行时调整见 `notepads/opendan-cmd-skill-todo.md`）。
4. CLI 功能与控制语义等价后，再统一调整固定 tool/action 集合、提示词和恢复协议；不在每次能力扩展时改变它们。

媒体分析目前已有 CLI，结果是侧模型生成的文本。若将来要求当前主模型直接接收图片等原生媒体，应扩展稳定的结果协议和宿主处理；当前标准 tool 与 shell 的模型结果通道都是文本，不能把此缺口当作必须新增业务 tool 的理由。

### 6.3 设计验收标准

- 新增一个 CLI / Skill / MCP 能力，不改变既定 tool schema、description、顺序、XML action 用法和公共 system prompt 前缀；动态信息通过运行中的查询结果获得。
- 不依赖 OpenDAN Session，独立 xllm 就能通过 CLI 启动长任务、查询、等待和取消；未知 task、超时、打断和不可取消任务的行为明确。
- 接到上层宿主后，等待和控制命令保留身份、提交顺序、幂等与恢复语义；不把独立子 Session 当作 fork 的替代。
- 文档和提示词只声明实际存在的命令；设计中的 CLI 示例明确标注，迁移前保留可用的任务跟进路径。

## 7. 代码与文档入口

| 内容 | 入口 |
|---|---|
| xllm 工具来源、提示词与结果装配 | [xllm.rs](../../src/frame/agent_tool/src/xllm.rs) |
| runtime task 工具与 resolver | [tasks.rs](../../src/frame/agent_tool/src/tasks.rs) |
| shell 执行与 CLI 结果转发 | [llm_bash.rs](../../src/frame/agent_tool/src/llm_bash.rs) |
| Agent CLI 命令与分发 | [agent_tool_cli_dev](../../src/frame/agent_tool_cli_dev/src/lib.rs) |
| Session CLI | [xagent.rs](../../src/frame/lib_opendan/src/bin/xagent.rs) |
| cmd_skills / goal_skills 的列表、安装、删除 | [skills.rs](../../src/frame/lib_opendan/src/skills.rs)、[behaviors.rs](../../src/frame/lib_opendan/src/state/behaviors.rs)（冻结） |
| Jarvis 内置 cmd_skills | [cmd_skills/](../../src/apps/jarvis_agent/agent/cmd_skills/) |
| 访问 BuckyOS 的 Agent 工具（TS + websdk） | [buckyos-agent](../../src/tools/buckyos-agent/readme.md)、websdk `cli/modules/` |
| 当前报告与子 context 控制 | [reports.rs](../../src/frame/lib_opendan/src/runner/reports.rs)、[tools.rs](../../src/frame/lib_opendan/src/runner/tools.rs) |

- [OpenDAN AgentTool 开发指南](<../llm_context/agent_tool/OpenDAN AgentTool 开发指南.md>)：接口、CLI、执行环境和结果契约。
- [xllm Rust SDK](../llm_context/xllm_rust_sdk.md)：独立 xllm、runtime 和等待行为。
- [Agent Actions](<../llm_context/Agent Actions.md>)：Action 保持小集合的设计背景；其中历史固定动作表和旧实现路径不能作为当前能力清单。
- [OpenDAN 协议目录](../opendan/protocol/README.md)：Session、Agent State、持久格式与恢复约定。
