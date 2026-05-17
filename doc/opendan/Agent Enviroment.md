# AgentSession Prompt Environment

本文说明 OpenDAN `AgentSession` 在接入 `llm_context::PromptRenderEngine` 时，应向提示词渲染层提供哪些环境变量。

历史上本文档描述的是一个独立的 `agent_environment` 模板替换引擎，包括 `{{var}}`、`{{workspace/path}}`、`TEXT / INPUT_BLOCK`、Null 跳过推理等规则。当前架构已经改变：模板渲染能力已下沉为通用的 `llm_context::prompt_engine`，OpenDAN 不再维护一套专用模板语法。`AgentSession` 的职责应收敛为：实现 OpenDAN 私有的 `ValueLoader`，并把 session、workspace、memory、输入事件等状态以稳定变量暴露给通用渲染引擎。

文件名中的 `Enviroment` 是历史拼写，语义上应理解为 **AgentSession Prompt Environment**。

## 1. 当前定位

`PromptRenderEngine` 是通用文本模板引擎，不认识 OpenDAN 的 session、workspace、owner、todo、message queue。它只提供：

- `RenderVars.env`：静态环境值；
- `RenderVars.vars`：静态模板变量；
- `ValueLoader::load(expr)`：调用方提供的异步动态取值接口；
- `__ENV($expr)__` / `__VAR(name, $expr)__` / `__INCLUDE(path)__` / `__EXEC(cmd)__`；
- upon 模板语法，例如 `{{ session.id }}`、`{% if workspace.id %}`。

因此 OpenDAN 的接入层应提供：

1. `AgentSessionValueLoader`：根据 `$session.id`、`$workspace.root`、`$input.messages` 等表达式返回 JSON 值。
2. `RenderVars` 初始化：放入本轮已经确定的轻量静态值。
3. `EngineConfig.include_roots`：限制 `__INCLUDE` 能读取的目录。
4. `SectionSpec` 构造：把 system、environment、memory、workspace、user input 等 section 交给 `prompt_compose` 预算装配。

## 2. 当前实现状态

当前 OpenDAN 主路径尚未真正接入 `PromptRenderEngine`。

已经存在的事实：

- `AgentSession::render_system_messages` 对 `behavior.system_prompt_template` 仍是直接作为 System message 使用，尚未渲染模板。
- `AgentSession::compose_environment_message` 已手工构造一个 `[environment]` block，包含 behavior、session、workspace、recent activity、clock。
- session 元数据保存在 `SessionMeta`，包括 `session_id`、`kind`、`current_behavior`、`status`、`owner`、`workspace_id`、`title`、`objective`、`process_stack` 等。
- session 运行时拥有 `session_dir`、`agent_root`、workspace 绑定和 snapshot。
- `PendingInput` 中保存本轮待消费的消息、事件和 interrupt。

本文下面定义的是接入 `PromptRenderEngine` 时应稳定暴露的变量契约。实现可以分阶段完成，但变量命名应尽量一次定稳，避免 behavior prompt 与外部 agent 模板反复迁移。

## 3. 命名规则

动态表达式统一使用点路径：

```text
$session.id
$session.title
$behavior.name
$workspace.id
$input.text
$paths.session_root
```

模板里推荐用两种方式：

```text
__ENV($session.id)__
```

或先注册变量：

```text
__VAR(session, $session)__
当前会话：{{ session.id }}
```

推荐约定：

- 顶层 namespace 使用小写名词：`session`、`behavior`、`workspace`、`agent`、`paths`、`input`、`runtime`、`memory`。
- 返回值优先使用 JSON object，而不是把复杂结构提前拼成字符串。
- 同一个变量名必须在 UI session、work session、恢复路径中保持语义一致。
- 不存在或当前不可用的值返回 `None`，由模板引擎按缺失处理。
- 不要把绝对路径直接暴露给不需要路径的模板；需要 include 时通过 `paths.*` 和 `include_roots` 控制。

## 4. 静态变量与动态变量

应区分两类变量。

**静态变量**适合放入 `RenderVars.env` 或 `RenderVars.vars`：

- 本轮已加载的 behavior 名称；
- session id；
- session title；
- workspace id；
- 当前用户输入文本；
- 当前时间戳。

**动态变量**适合由 `AgentSessionValueLoader` 解析：

- workspace 文件摘要；
- memory 召回结果；
- 最近历史片段；
- owner/contact 详情；
- pending input 的结构化批次；
- 当前 snapshot 中的最近 accumulated/step 信息。

这样可以避免每次渲染都提前加载昂贵数据，也方便 section 预算裁剪：没有被模板引用的变量不需要取值。

## 5. 基础变量

### 5.1 `session`

`$session` 应返回当前 session 的稳定元数据。

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$session` | object | 当前 session 元信息 |
| `$session.id` | string | session id |
| `$session.kind` | string | `ui` 或 `work` |
| `$session.status` | string | `idle` / `running` / `waiting_input` / `waiting_tool` / `ended` / `error` |
| `$session.title` | string? | session 标题 |
| `$session.objective` | string? | work session 目标或任务说明 |
| `$session.owner` | string? | owner 标识，当前来自 `SessionMeta.owner` |
| `$session.peer_did` | string? | UI 对端 DID |
| `$session.peer_tunnel_did` | string? | 对端 tunnel DID |
| `$session.bootstrap_done` | bool | bootstrap 是否完成 |

建议 `$session` object 至少包含：

```json
{
  "id": "ui_xxx",
  "kind": "ui",
  "status": "running",
  "title": "chat",
  "objective": "",
  "owner": "alice",
  "peer_did": "did:..."
}
```

### 5.2 `behavior`

`$behavior` 描述本轮正在运行的 behavior 配置。

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$behavior` | object | 当前 behavior 元信息 |
| `$behavior.name` | string | behavior 名称 |
| `$behavior.objective` | string? | behavior 配置中的 objective |
| `$behavior.mode` | string | `agent` 或 `behavior` |
| `$behavior.parser` | string | 当前 parser 名称 |
| `$behavior.renderer` | string | 当前 renderer 名称 |
| `$behavior.switch_mode` | string | `normal` / `fork` / `independent` |
| `$behavior.max_rounds` | number | tool/behavior 最大轮数 |
| `$behavior.tool_plan` | string? | tool plan 名称 |

注意：模型策略、provider options、budget 等运行策略通常不应直接进 prompt，除非某个 behavior 明确需要向模型暴露。

### 5.3 `process`

`$process` 描述 behavior process 栈。

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$process.entry` | string | 当前 process 入口 behavior |
| `$process.current` | string | 当前 behavior |
| `$process.stack` | array | fork/independent process 栈 |

`process.stack` 中每项建议形如：

```json
{ "entry": "research", "current": "summarize" }
```

## 6. 输入变量

`$input` 描述触发本轮推理的输入。它只应包含本轮 drain 出来的 pending inputs，不代表完整历史。

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$input` | object | 本轮输入摘要 |
| `$input.text` | string? | 本轮用户消息合并后的文本 |
| `$input.messages` | array | 本轮 `PendingInput::Msg` 列表 |
| `$input.events` | array | 本轮非消息事件列表 |
| `$input.interrupts` | array | 本轮 interrupt 列表 |
| `$input.keys` | array<string> | 本轮 pending input dedup keys |
| `$input.has_user_text` | bool | 是否有用户文本 |
| `$input.has_events` | bool | 是否有事件 |

`$input.messages` 每项建议包含：

```json
{
  "record_id": "...",
  "from": "...",
  "from_did": "did:...",
  "from_name": "Alice",
  "tunnel_did": "did:...",
  "text": "hello"
}
```

`$input.events` 每项建议包含：

```json
{
  "event_id": "/task_mgr/123/done",
  "data": {}
}
```

当前 `build_or_resume` 已经把本轮 `human_texts` 合并成 user message。接入模板后，`$input.text` 应替代手写的 `compose_human_text` 结果，或者作为 `user.input` section 的 local var。

## 7. Workspace 与路径变量

### 7.1 `workspace`

`$workspace` 描述 session 绑定的 primary workspace。

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$workspace` | object? | 当前 workspace；未绑定时为 `None` |
| `$workspace.id` | string | workspace id |
| `$workspace.root` | string | workspace 根目录，主要供 include path resolution 使用 |
| `$workspace.summary` | string? | workspace 摘要，建议优先读取 `summary.md` 或等价文件 |
| `$workspace.readme` | string? | workspace README 或任务说明 |

`workspace.root` 是敏感路径，不建议直接输出给 LLM。模板若要引用 workspace 文件，应优先通过 `__INCLUDE`，并由 `EngineConfig.include_roots` 限制范围。

### 7.2 `paths`

`$paths` 暴露 AgentSession 可安全使用的路径锚点。

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$paths.agent_root` | string | AgentRootFS 根目录 |
| `$paths.session_root` | string | 当前 session 目录 |
| `$paths.workspace_root` | string? | 当前 workspace 根目录 |
| `$paths.memory_root` | string | agent memory 目录 |
| `$paths.notepads_root` | string | notepads 目录 |
| `$paths.skills_root` | string | skills 目录 |
| `$paths.tools_root` | string | agent tools 目录 |

推荐 include 写法：

```text
__INCLUDE($paths.session_root/readme.md)__
__INCLUDE($paths.workspace_root/summary.md)__
__INCLUDE($paths.agent_root/role.md)__
```

实现时要注意：当前 `PromptRenderEngine` 的 `__INCLUDE` 只接受最终为绝对路径的 path，并要求路径位于 `include_roots` 白名单内。因此 `AgentSession` 必须把允许 include 的根目录传给 `EngineConfig.include_roots`，至少包括：

- `agent_config.layout.root`
- `session_dir`
- 当前 workspace root

是否允许 `memory_root`、`notepads_root`、`skills_root` 进入 include roots，需要按具体 behavior 权限决定，不应默认全部开放。

## 8. Agent 与 Owner 变量

### 8.1 `agent`

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$agent.id` | string | agent id |
| `$agent.name` | string | agent display/runtime name |
| `$agent.did` | string? | agent DID |
| `$agent.root` | string | AgentRootFS 根目录 |

`role.md`、`self.md` 当前由 `render_system_messages` 直接读取。接入模板后，可以通过 `__INCLUDE($paths.agent_root/role.md)__` 和 `__INCLUDE($paths.agent_root/self.md)__` 显式引入。

### 8.2 `owner`

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$owner` | object? | 当前 owner/contact 信息 |
| `$owner.id` | string? | owner id |
| `$owner.did` | string? | owner DID |
| `$owner.show_name` | string? | 显示名 |
| `$owner.contact` | object? | contact manager 返回的原始联系人信息 |

`owner` 可能需要异步查询 contact manager，适合由 `ValueLoader` 懒加载，不应放在每次 render 的静态 env 中。

## 9. Runtime 环境变量

`$runtime` 描述当前运行态，而不是长期 session 元数据。

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$runtime.clock_unix_ms` | number | 当前时间戳，毫秒 |
| `$runtime.recent_activity` | string? | `one_line_status`，由工具通过 `OneLineStatusSink` 更新 |
| `$runtime.pending_tool_calls` | array? | snapshot 中待恢复的 pending tool calls，通常不进 prompt |
| `$runtime.llm_task_ids` | array? | 当前 snapshot 中 provider task ids，通常仅调试 |

当前手写 `[environment]` block 已包含：

- behavior name；
- session id / title；
- workspace id；
- recent activity；
- `clock: unix_ms=...`。

接入模板后，这些应改为 `$behavior.name`、`$session.id`、`$session.title`、`$workspace.id`、`$runtime.recent_activity`、`$runtime.clock_unix_ms`。

## 10. History 与 Snapshot 变量

历史变量要谨慎暴露。完整 `state.accumulated` 可能很大，也可能包含 tool observation，不应默认完整进入 prompt。

建议提供以下受控变量：

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$history.recent_messages` | string 或 array | 最近 user/assistant 消息片段，已截断 |
| `$history.last_user_message` | string? | 最近用户消息 |
| `$history.last_assistant_message` | string? | 最近 assistant 文本 |
| `$history.summary` | string? | 历史摘要，如果已有旁路压缩结果 |
| `$history.round_summary` | object? | 当前或最近 round summary |

Behavior Loop 可额外提供：

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$steps.last` | object? | `state.last_step` 的安全视图 |
| `$steps.recent` | array | 最近若干 step 的安全视图 |
| `$steps.summary` | string? | step 历史摘要 |

原则：

- 不直接暴露完整 snapshot。
- 不默认暴露 tool result 全文。
- 对进入 prompt 的历史必须有 token/字符上限。
- 如果需要模型看完整工具结果，应通过 observation 或明确的 section 传入。

## 11. Memory 变量

`$memory` 应代表“本轮 prompt 需要的长期可复用信息”，不是 memory 数据库全量 dump。

建议变量：

| 表达式 | 类型 | 说明 |
| --- | --- | --- |
| `$memory.recall` | string? | 针对当前输入召回的 memory 摘要 |
| `$memory.session_summary` | string? | session 级摘要 |
| `$memory.agent_profile` | string? | agent 长期自我配置或偏好 |
| `$memory.user_profile` | string? | 用户偏好摘要 |

Memory 召回可以很贵，必须懒加载，并且应由 section budget 控制是否进入最终 prompt。

## 12. 与模板 section 的关系

推荐 OpenDAN prompt section 使用如下变量：

### `system.identity`

```text
__INCLUDE($paths.agent_root/role.md)__

__INCLUDE($paths.agent_root/self.md)__
```

### `system.behavior`

```text
__VAR(behavior, $behavior)__
Current behavior: {{ behavior.name }}
Objective: {{ behavior.objective }}
```

### `environment`

```text
__VAR(session, $session)__
__VAR(workspace, $workspace)__
__VAR(runtime, $runtime)__

[environment]
session: `{{ session.id }}`{% if session.title %} ("{{ session.title }}"){% endif %}
behavior: `{{ behavior.name }}`
{% if workspace.id %}workspace: `{{ workspace.id }}`{% endif %}
{% if runtime.recent_activity %}recent activity: {{ runtime.recent_activity }}{% endif %}
clock: unix_ms={{ runtime.clock_unix_ms }}
```

### `workspace.context`

```text
{% if workspace.summary %}
## Workspace Summary
{{ workspace.summary }}
{% endif %}
```

### `memory.recall`

```text
{% if memory.recall %}
## Relevant Memory
{{ memory.recall }}
{% endif %}
```

### `user.input`

```text
__VAR(input, $input)__
{{ input.text }}
```

## 13. 安全边界

`AgentSessionValueLoader` 和 `EngineConfig` 必须共同保证安全。

### 13.1 文件 include

- `__INCLUDE` 必须限制在 `include_roots` 内。
- 默认允许：agent root、session dir、当前 workspace root。
- memory、notepads、skills、tools 是否允许 include，应由 behavior/tool policy 决定。
- 不允许任意绝对路径。
- 不允许路径穿越。
- 单文件 include 受 `max_include_bytes` 限制。
- 总输出受 `max_total_bytes` 限制。

### 13.2 命令执行

`__EXEC` 默认关闭。OpenDAN behavior prompt 不应依赖 `__EXEC`。需要动态内容时，优先通过 `ValueLoader` 或工具调用获得。

### 13.3 隐私与审计

- 绝对路径、DID、contact 原始数据进入 prompt 前要确认是否必要。
- prompt 渲染结果应可在 round history 或调试日志中复现。
- loader 查询失败应产生统计或审计事件，不能静默掩盖关键错误。

## 14. 缺失值语义

当前通用 `PromptRenderEngine` 没有旧文档里的 `INPUT_BLOCK` Null 模式。

建议新接入采用更简单的规则：

- 缺失变量在模板里表现为空字符串或 failed marker，取决于指令类型。
- 是否跳过 LLM 推理由 `AgentSession` 的 turn 构造逻辑决定，而不是模板引擎决定。
- 没有真实用户输入、没有事件、没有需要恢复的 snapshot 时，不构造 user input section。
- 中途 resume 不注入 synthetic user message。

也就是说，“零 LLM 空转”应由 `AgentSession::build_or_resume` 的输入驱动逻辑保证，而不是由模板渲染结果的 Null 语义保证。

## 15. 实现路线图

### 15.1 第一期接入范围（MVP，目标：系统能跑通且行为等价）

**原则**

- 不改变现行 prompt 的可观察内容，只把"字面字符串拼接"换成"模板渲染"。
- 渲染时机统一在 turn 真正打包送 `llm_context` 之前（`agent_session` 的 egress 边界），不在 behavior 加载时预渲染。
- 只暴露**同步可得、零 IO**的变量。任何需要异步查询、文件读、外部服务调用的变量留到后续阶段。
- 行为模板仍可保持纯字面文本不写任何 `{{ }}` / `__VAR__`，渲染对它们应是 no-op。

**第一期 Loader 必须支持的变量**

| 表达式 | 来源 | 备注 |
| --- | --- | --- |
| `$session.id` | `SessionMeta.session_id` | |
| `$session.kind` | `SessionMeta.kind` | `ui` / `work` |
| `$session.title` | `SessionMeta.title` | 缺失 → 空串 |
| `$session.objective` | `SessionMeta.objective` | 缺失 → 空串 |
| `$session.owner` | `SessionMeta.owner` | 缺失 → 空串 |
| `$behavior.name` | 当前 behavior | |
| `$behavior.objective` | behavior cfg | 缺失 → 空串 |
| `$behavior.mode` | behavior cfg | `agent` / `behavior` |
| `$workspace.id` | session 绑定 workspace | 未绑定 → `None` |
| `$workspace.root` | 同上 | 仅供 include path resolution |
| `$paths.agent_root` | `agent_config.layout.root` | |
| `$paths.session_root` | `session_dir` | |
| `$paths.workspace_root` | 当前 workspace root | 未绑定 → `None` |
| `$input.text` | 本轮 `human_texts` 合并 | 与现有 `compose_human_text` 等价 |
| `$input.has_user_text` | 同上 | |
| `$input.has_events` | `PendingInput` | |
| `$runtime.clock_unix_ms` | `SystemTime::now` | |
| `$runtime.recent_activity` | `OneLineStatusSink` | 缺失 → 空串 |

第一期**不实现**的变量（明确推后）：

- `$session.peer_did` / `$session.peer_tunnel_did` / `$session.status` / `$session.bootstrap_done`
- `$behavior.parser` / `$behavior.renderer` / `$behavior.switch_mode` / `$behavior.max_rounds` / `$behavior.tool_plan`
- `$process.*`（fork/independent 栈）
- `$input.messages` / `$input.events` / `$input.interrupts` / `$input.keys`（结构化批次，第一期模板只需 `$input.text`）
- `$workspace.summary` / `$workspace.readme`（需要文件 IO）
- `$agent.*` / `$owner.*`（需要 contact manager 异步查询）
- `$history.*` / `$steps.*`（snapshot 历史视图，需要裁剪策略）
- `$memory.*`（贵的召回路径）
- `$runtime.pending_tool_calls` / `$runtime.llm_task_ids`

**第一期接入的 section**

只接两个：

1. `system.identity`：把 `behavior.system_prompt_template` 经 `PromptRenderEngine` 渲染后作为 System message，替换 `render_system_messages` 中的字面输出。
2. `environment`：把现行 `compose_environment_message` 手写拼装迁到模板，输出与现版本字节级或仅 whitespace 等价的 `[environment]` block。

`user.input` 第一期**不动**，继续走现有 `compose_human_text` 路径，避免一次改动牵动 message ordering / dedup 逻辑。

**第一期 EngineConfig**

- `include_roots`：`[agent_config.layout.root, session_dir]` + 当前 workspace root（若绑定）
- `__EXEC` 全程关闭
- memory / notepads / skills / tools 目录**不进** include_roots
- `max_include_bytes` / `max_total_bytes` 采用 `EngineConfig::default()`，等真有 behavior 触发限制再调

**第一期不做**

- 不接 `prompt_compose::SectionSpec` 的 budget 装配（两个 section 都是小段，第一期直接 render-then-concat，第二期再上 budget）
- 不动 round_history / snapshot 持久化格式
- 不引入新的 behavior cfg 字段
- 不写 `__INCLUDE($paths.agent_root/role.md)__` 这类新模板；现有 behavior 模板保持原样

**验收条件**

- 现有 `chat_route` / `do` / `plan` / `groupchat_route` 四个 behavior 不修改任何配置文件即可跑通。
- diff 渲染前后的 System message + environment block：字节级等价，或差异仅来自 whitespace / 模板引擎对空值的处理。
- Loader 单元测试覆盖：缺 workspace、缺 title、空 input、有 input、`workspace.root` 未绑定时的 include_roots。

**当前实现状态**

- [x] `AgentSessionValueLoader` 新建（`src/frame/opendan/src/prompt_env.rs`）
- [x] `render_system_messages` 改走 `PromptRenderEngine`，保留单花括号 `{name}` 兜底以不破坏现有 behavior
- [x] `compose_environment_message` 改走 `PromptRenderEngine`（`ENVIRONMENT_BLOCK_TEMPLATE` 常量，输出与历史版本字节级等价）
- [x] include_roots 注入（agent_root + session_root + workspace_root，若绑定）
- [x] Loader 单元测试（10 项，覆盖 key 解析 / 聚合对象 / 完整 env block / 最小 env block / 部分 env block / legacy 单花括号 / engine+legacy 组合 / include_roots 边界）
- [x] 四个现有 behavior 的等价性回归（`cargo test -p opendan --lib` 通过 121 项，未修改任何 `behaviors/*.toml`）

### 15.2 后续阶段

1. 引入 `prompt_compose::SectionSpec` 与 token budget 装配；`user.input` 迁入模板。
2. 扩展 loader：`$workspace.summary` / `$workspace.readme` / `$agent.*` / `$owner.*`（带异步 IO）。
3. `$history.*` / `$steps.*`：先实现 `last_user_message` / `last_assistant_message` / `round_summary` 三件套并设字符上限。
4. `$memory.*`：与 memory 召回路径联动，按 section budget 决定是否进 prompt。
5. `$process.*`、`$input.messages` 等结构化字段按 behavior 实际需求逐项放开。
6. 评估是否把 `memory_root` / `notepads_root` / `skills_root` 按 behavior policy 加入 include_roots。

## 16. 当前必须避免的旧设计

以下旧设计不再作为目标：

- 独立 `agent_environment` 模板引擎。
- `{{workspace/path}}` / `{{cwd/path}}` 自定义 include 语法。
- `TEXT` / `INPUT_BLOCK` 两种渲染模式。
- 由模板引擎返回 `None` 来决定 session WAIT。
- 在模板引擎中直接接 PolicyEngine、SubAgent fs_scope、session worker 状态机。

这些能力现在分别由通用 `PromptRenderEngine`、`EngineConfig.include_roots`、`AgentSession` turn 构造逻辑、工具/文件权限层和 L4 outcome driver 承担。

## 17. 总结

新的 AgentSession Environment 文档不是模板引擎需求，而是 OpenDAN 对提示词渲染层的变量契约。

核心分工是：

- `llm_context::prompt_engine` 负责通用模板语法和安全 include。
- `AgentSessionValueLoader` 负责把 OpenDAN session 状态映射成 `$session.*`、`$workspace.*`、`$input.*` 等变量。
- `prompt_compose` 负责 section 渲染和预算装配。
- `AgentSession` 负责决定本轮是否需要推理、如何 resume、如何处理 context limit。

对 behavior 作者和外部开发者来说，这份变量契约就是编写 prompt 模板时最重要的 API 面。
