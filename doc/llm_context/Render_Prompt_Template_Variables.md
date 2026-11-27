# Render Prompt 模板变量说明

本文档是 `agent_tool::prompt_engine::PromptRenderEngine` 的实现参考，写给修改 / 扩展模板引擎自身的人。引擎位于 [`src/frame/agent_tool/src/prompt_engine.rs`](../../src/frame/agent_tool/src/prompt_engine.rs)，类型 `PromptRenderEngine`，是 `agent_tool` 的通用能力 —— 不认识 OpenDAN 的 session、workspace、owner、todo 等业务概念。

> 旧 opendan Runtime（`src/frame/opendan/src/prompt_env.rs`）这一侧实际暴露的变量契约、`AgentSessionValueLoader` 的字段映射、`include_roots` 当前设置、behavior 模板可以写什么，全部在 [Agent Enviroment.md](../opendan/Agent%20Enviroment.md)，待下一阶段 opendan 重构接入。本文不重复那一层。其中 `current_context.last_step` / `current_context.step_history` 是从 LLMContext 快照取出的 `StepRecord`，没有 Round / Turn 编号变量。
>
> libopendan 只在 behavior 配了模板时经过本引擎（`runner/assembler.rs::render_template`：system 模板、`on_init` / `on_input` / `on_context_switch` 输入模板、Goal 头模板），变量见 §8；没配模板的输入批次由 `SessionAssembler::render_input` 直接拼成 `<session_input hook=… time=…>`。Step 历史和动作回响（`<<step_history>>`、`<<last_step_action_results>>`）由 `XmlStepRenderer` 在每次推理前渲染；Session 历史由 Runner 从 worklog 重建为一串消息（压缩后含一条 `<history_summary>` 摘要消息）。这些都不是模板变量。

实际指令名是 `__ENV`、`__INCLUDE`、`__EXEC`、`__VAR`，不是更早设计稿里的 `__OPENDAN_ENV` 等前缀名。

## 1. 渲染流程

`PromptRenderEngine::render(template, vars, loader)` 分两步执行：

1. **预处理阶段**
   - 处理 `__ENV($expr)__`、`__INCLUDE(path)__`、`__EXEC(cmd)__`、`__VAR(name, $expr)__`。
   - 预处理最多执行 32 遍（`MAX_PREPROCESS_PASSES`），因此 include 进来的内容里也可以继续包含这些指令。
   - 预处理后如果仍存在 `__ENV(`、`__INCLUDE(`、`__EXEC(`、`__VAR(` 或历史 `__OPENDAN_` 指令，会返回语法错误。

2. **upon 渲染阶段**
   - 使用 upon 引擎渲染 `{{ name }}`、`{% if %}`、`{% for %}` 等模板语法。
   - 渲染上下文来自 `RenderVars.vars` 和 `__VAR` 注册出来的变量。
   - 引擎设置了自己的默认 formatter：`{{ x }}` 遇到 map / list 不再报 upon 的 “unformattable type”，按 §2.1 渲染。
   - 最终输出超过 `EngineConfig.max_total_bytes` 时会被截断，并设置 `truncated = true`。

## 2. 动态表达式解析

`__ENV` 和 `__VAR` 的参数必须是 `$expr`。`$` 后面的表达式按如下顺序解析：

1. 在 `RenderVars.env` 中查找，支持点路径，例如 `$owner.name`。
2. 在 `RenderVars.vars` 中查找，支持点路径和数组下标，例如 `$items.0`。
3. 调用调用方传入的 `ValueLoader::load(expr)`。

`ValueLoader::load` 返回 `Ok(None)` 表示"不认识这个变量"，引擎把它当作软缺失处理，不会直接报错。

`__ENV` 的值转成文本时的规则：

- JSON string 会先 `trim`，空字符串视为缺失。
- JSON null 视为缺失。
- number、bool 输出其字面值。
- object、array 按 §2.1 的默认渲染输出可读文本，不是 JSON。

### 2.1 对象的默认渲染

模板里引入的对象不插完整 JSON，按对象自己的字段给出人可读的文本（`RenderExtensions::value_text`）。`{{ x }}`、`__ENV($x)__` 和文本 filter（`xml`、`attr`、`truncate`、`oneline`、`quote`、`join` 的元素）都走这一条规则：

1. 宿主为这类值注册了默认格式（`RenderExtensions::with_default_format(格式名, 判定函数)`，第一个判定为真的生效）时，用该命名格式的输出。libopendan 为输入视图、消息、事件、附件列表注册了 `input.xml`、`message.xml`、`event.xml`、`attachments.list`。
2. 否则 object 最多两行，取对象已有的字段、沿用原字段名，不给对象加统一的展示字段：
   - 第一行：`title`、`name`、`subject` 中第一个有值的，只写值。
   - 第二行：`summary`、`description`、`status` 中第一个有值的，写成 `<字段名>: <值>`。
   - “有值”指非空字符串、数字或 bool；字段值里的换行和连续空白合成一个空格。
   - 例：`{"title": "Weekly report", "status": "active", …}` → `Weekly report` + 换行 + `status: active`。
3. array 每个元素一行：字符串、数字、bool 是其字面值，null 跳过，object 把上面两行用 ` — ` 接成一行，嵌套 array 用 `, ` 接成一行。空 array 输出空串。
4. 取不到可读字段时（某个 object 一个上述字段都没有，或数组里有这样的元素）：整个值回退成紧凑 JSON，`log::warn!` 记下模板名（`EngineConfig.name`）和变量路径（如 `handover.sub_task`；表达式算出的值找不到路径时写 `(an expression)`），`RenderStats.json_fallback += 1`。渲染照常完成，不报错，这批输入照常消费。
5. 结果（可读文本或回退的 JSON）受单结果预算约束：超过 16K token（`agent_tool::text_budget`，默认按 4 字节一个 token 估算）时保留头尾，中间写 `…[N bytes omitted]…`。

明确要 JSON 时写 `{{ x | json }}`：输出紧凑 JSON，不记日志、不计数。`render_format` 和宿主 filter 拿到的仍是原值。

## 3. 指令

### 3.1 `__ENV($expr)__`

把 `$expr` 解析成值，并把结果以内联文本替换到当前位置。

```text
session=__ENV($session_id)__
owner=__ENV($owner.name)__
```

规则：

- 参数不以 `$` 开头会返回语法错误。
- object / array 按 §2.1 渲染；回退成 JSON 时同样记日志并计入 `stats.json_fallback`。
- 命中变量时，`stats.env_expanded += 1`。
- 未命中变量时，`stats.env_not_found += 1`，输出 HTML 注释形式的失败标记，例如 `<!-- __ENV__ failed: not found -->`。

### 3.2 `__VAR(name, $expr)__`

把 `$expr` 解析成 JSON 值，并注册到 upon 渲染上下文中。指令本身输出为空。

```text
__VAR(session, $session)__
当前会话：{{ session.id }}
```

规则：

- `name` 必须是合法变量名：首字符为 ASCII 字母或 `_`，后续可包含 ASCII 字母、数字、`_`、`.`、`-`。
- `$expr` 不以 `$` 开头会返回语法错误。
- 命中变量时写入 upon 上下文，`stats.var_registered += 1`，`resolved_vars[name] = true`。
- 未命中变量时注册一个空字符串占位，`stats.var_failed += 1`，`resolved_vars[name] = false`。

### 3.3 `__INCLUDE(path)__`

读取文件内容并内联到当前位置。当前实现中的指令名是 `__INCLUDE`，不是 `__OPENDAN_CONTENT`。

```text
__INCLUDE(/absolute/path/to/prompt.md)__
__INCLUDE($paths.role_prompt)__
```

规则：

- 如果 `path` 以 `$` 开头，先按动态表达式解析成路径文本。
- 如果 `path` 不以 `$` 开头，直接作为路径文本使用。
- `__INCLUDE` 不做局部变量拼接：`$paths.agent_root/role.md` 会被整体当作动态表达式 `paths.agent_root/role.md` 解析；除非 `ValueLoader` 显式支持这个 key，否则不会自动变成 `$paths.agent_root` + `/role.md`。
- 路径会展开 `$HOME`、`${HOME}` 和开头的 `~`。
- 展开后必须是绝对路径。
- `EngineConfig.include_roots` 必须非空，且目标路径必须位于白名单目录内。
- 路径中包含 `..` 会被拒绝。
- 文件内容按 UTF-8 读取，超过 `EngineConfig.max_include_bytes` 会截断，默认 64 KiB。
- include 的内容会继续递归预处理，递归深度由 `EngineConfig.max_recursion_depth` 限制，默认 8。
- include 失败不会让整个 render 失败；会输出 `<!-- __INCLUDE__ failed: ... -->`，并记录 `stats.content_failed += 1`。

### 3.4 `__EXEC(cmd)__`

执行 shell 命令并内联 stdout。

```text
__EXEC("printf hello")__
__EXEC(tree -L 2 $paths.workspace_root)__
```

规则：

- 默认关闭：`EngineConfig.allow_exec = false`。关闭时输出失败标记，不执行命令。
- 开启后通过 `EngineConfig.executor: Option<Arc<dyn PromptExec>>` 执行。未注入执行器返回 `RenderError::Loader`；不自动启动本地 shell。`SandboxPromptExec` 与 exec 工具共用 Runtime 和执行跟踪。
- 命令参数可以用一层单引号或双引号包裹；不匹配的引号会报错。
- 命令中的 `$expr` 会按动态表达式解析；未命中的 `$expr` 保留原文本。动态 token 可包含 ASCII 字母、数字、`_`、`.`、`/`、`-`，因此 `$paths.workspace_root/subdir` 同样会被整体当作一个表达式。
- 超时由 `EngineConfig.exec_timeout` 控制，默认 10 秒。
- stdout 超过 `max_include_bytes` 会截断。
- 非 0 退出、超时、spawn 失败都会输出 `<!-- __EXEC__ failed: ... -->`，并记录 `stats.exec_failed += 1`。

## 4. upon 变量与自动注册

upon 上下文的来源有两类：

- `RenderVars.vars` 会在渲染开始时直接放入上下文，所以静态变量可以直接用 `{{ name }}`。
- `__VAR(name, $expr)__` 会在预处理阶段把动态值注入上下文。

当前实现还有一个自动注册行为：渲染前会扫描简单的 upon 输出占位符。如果模板里出现 `{{ user }}` 或 `{{ owner.name }}`，且没有显式 `__VAR` 声明，引擎会自动在模板前加：

```text
__VAR(user, $user)__
__VAR(owner, $owner)__
```

这意味着"所有 upon 变量都必须显式写 `__VAR`"已经不是当前实现事实。建议行为：

- 静态、调用方已知的值放进 `RenderVars.vars`。
- 昂贵或异步加载的值显式写 `__VAR`，让模板依赖更清楚。
- 不要依赖自动注册来表达复杂业务依赖，尤其是 OpenDAN 接入后需要控制加载成本的变量。

转义字面量大括号：

- `\{{` 渲染为 `{{`
- `\}}` 渲染为 `}}`

`agent_tool::runtime::RuntimeValueLoader` 支持 `__ENV($runtime.hostname)__` 等表达式，值来自执行体。`{{runtime.*}}` 可使用 id、kind、os、arch、hostname、shell、cwd、tools、current_time、timezone（宿主负责把 RuntimeInfo 放入 RenderVars）。Session 的 system 只放稳定字段，时间和时区进入输入批次；一次性 xllm 可在 system 使用打开时的值。cwd 是工具执行处路径；模板和 INCLUDE 路径仍在控制侧。

## 5. EngineConfig 默认值

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `max_include_bytes` | 64 KiB | 单次 include 内容和 exec stdout 上限 |
| `max_total_bytes` | 256 KiB | 最终渲染结果上限 |
| `exec_timeout` | 10 秒 | 单条 `__EXEC` 超时 |
| `allow_exec` | `false` | 是否允许 `__EXEC` |
| `executor` | `None` | 注入的 PromptExec 执行器 |
| `include_roots` | 空 | 为空时 `__INCLUDE` 全部失败 |
| `max_recursion_depth` | 8 | include 嵌套预处理深度 |
| `extensions` | 空 | 宿主的命名格式、默认格式（§2.1）与额外 filter |
| `name` | 空串 | 模板名，用在 §2.1 的警告里；libopendan 写 `<behavior>.<hook>`、`<behavior>.system`、`goal_activation` |

## 6. 返回值与统计

`PromptRenderEngine::render` 返回 `RenderResult`：

| 字段 | 说明 |
| --- | --- |
| `rendered` | 最终渲染文本 |
| `stats` | 渲染统计 |
| `resolved_vars` | `__VAR` 注册变量的解析结果，`true` 表示命中 |
| `truncated` | 最终输出是否因 `max_total_bytes` 被截断 |

`RenderStats` 字段：

| 字段 | 说明 |
| --- | --- |
| `env_expanded` | `__ENV` 成功展开数量 |
| `env_not_found` | `__ENV` 未找到数量 |
| `content_loaded` | `__INCLUDE` 成功加载数量 |
| `content_failed` | `__INCLUDE` 失败数量 |
| `exec_run` | `__EXEC` 成功执行数量 |
| `exec_failed` | `__EXEC` 失败数量 |
| `var_registered` | `__VAR` 成功注册数量 |
| `var_failed` | `__VAR` 未解析到值的数量 |
| `json_fallback` | 取不到可读字段、回退成紧凑 JSON 的 object / array 数量（§2.1） |

## 7. 修改 / 扩展引擎时的注意事项

- 新指令名要与现有 `__XXX__` 前缀风格一致，并在 §1 的预处理"未消化指令"检测里加新前缀，否则会被当成语法错误。
- 任何新指令默认要"失败可恢复"：失败时打 stats + 输出 `<!-- ... failed: ... -->` 标记，不要让整个 render 终止。
- `ValueLoader::load` 的成功 / 软缺失 / 硬错误三态语义不要破坏：调用方依赖 `Ok(None)` 来表达 loader 不持有这个名字。
- 新增 `EngineConfig` 字段必须给 `Default` 实现，并在本文 §5 表格里补充。
- 触发 IO / 进程的新指令要带超时 + 字节上限；不要假设调用方已经做了沙箱。
- 业务变量映射（`$session.*`、`$workspace.*` 等）一律不进引擎，由调用方的 `ValueLoader` 提供 —— OpenDAN 的实现见 [Agent Enviroment.md](../opendan/Agent%20Enviroment.md)。
- 新的对象变量先给可读视图（对象自己的字段、宿主注册的默认格式，或现成的 `*_text` 文本），原始数据只在确实需要时以显式的名字（如 `runtime.raw`）保留。

## 8. libopendan 输入模板的变量

`SessionAssembler::render_input`（`lo/runner/assembler.rs`）给 `on_init` / `on_input` / `on_context_switch` 模板的变量（`RenderVars.vars`，模板名 `<behavior>.<hook>`）。“直接放”指 `{{ 变量 }}` 不加 filter 时的输出，规则见 §2.1。

| 变量 | 是什么 | 直接放 |
| --- | --- | --- |
| `input` | 这批输入的视图（`hook`、`time`、`messages[]`、`events[]`、`items[]`、`text`、`count`，字段见 `runner/input_view.rs`） | `input.xml`，即 `input.text` |
| `input.messages[]` / `input.events[]` / `input.items[]` | 消息、事件视图；`items` 按消费顺序混排，带 `is_msg` / `is_event` 等布尔字段 | 每条一个 `message.xml` / `event.xml` 元素 |
| `msg.attachments` | 附件列表 | `attachments.list`，每个附件一行 |
| `msg.from` / `msg.conversation` | 说话人 / 会话 | `name` 字段 |
| `session` | `id`、`kind`、`objective`、`timezone`、`is_bootstrap`、`todos`（TodoList 紧凑视图，文本），以及恒为 false / 空串的 `background_hint_changed` / `default_changed_background_hint_text` | 命名空间，用其中的字段；整体放会回退成 JSON |
| `runtime.status` | `<runtime>` 块里的那几行（`- ` 开头，工作 runtime、与之不同时的私有 runtime、workspace） | 文本 |
| `runtime.clock_text` | 批次时间（UTC RFC 3339） | 文本 |
| `runtime.raw` | 宿主的原始状态（RuntimeInfo + `workspace` + `runtimes`），给要按 os / shell 分支的模板 | 回退成 JSON 并记警告；要 JSON 写 `\| json` |
| `handover` | 交接：`to`、`process_result`、`sub_task`；不是交接批次时为 null | 命名空间，整体放会回退成 JSON；要整段文本用 `handover_text` |
| `handover.process_result` | 子 behavior 交回的结果：`behavior`、`status`、`result`（文本）、`submission` | `status: <status>` |
| `handover.process_result.submission` | 子 behavior 提交的结构化结果，紧凑 JSON 文本，受单结果预算约束（超出时写明 `agent-session read-session <sid> --turn <n>` 能读到全文） | 文本（JSON） |
| `handover.sub_task` | 被调起时的 `mode` 和 `task` | 回退成 JSON；用 `handover.sub_task.task` |
| `hints` | 这批的 Memory 线索，每条一行文本 | 每条一行 |
| `background` | 这批要显示的背景行的文本 | 每条一行 |
| `background_context` | `<<background_context>>` 块（含 `- ` 前缀），无行时为空 | 文本 |
| `task_text`、`handover_text`、`todos_text`、`hints_text`、`active_sessions_text`、`runtime_text` | 内建模板的各块，空或以换行结尾；`runtime_text` 是 `<runtime>` 块，plan 还带 `<runtimes>` 表和第一批的 `<skill_candidates>` | 文本 |
| `builtin` | 内建模板的完整输出 | 文本 |

system 模板（`<behavior>.system`）的变量是 `identity`、`behavior`、`session`、`workspace.id`；Goal 头模板（`goal_activation`）的变量是 `goal`（`GoalView`，`goal.text` 是内建 Goal 头）。
