# Render Prompt 模板变量说明

本文档按当前代码实现更新。当前通用模板引擎位于 `src/frame/llm_context/src/prompt_engine.rs`，类型为 `PromptRenderEngine`。它是 `llm_context` 的通用能力，不认识 OpenDAN 的 session、workspace、owner、todo 等业务概念。

重要现状：

- 实际指令名是 `__ENV`、`__INCLUDE`、`__EXEC`、`__VAR`，不是历史文档里的 `__OPENDAN_ENV`、`__OPENDAN_CONTENT`、`__OPENDAN_EXEC`、`__OPENDAN_VAR`。
- `AgentSession::render_system_messages` 当前仍把 `behavior.system_prompt_template` 原样作为 System message 使用，尚未调用 `PromptRenderEngine` 渲染。
- 当前仓库没有 OpenDAN 专用的 `ValueLoader` 主路径实现，因此 `$session.*`、`$workspace.*`、`$owner.*`、`$new_msg`、`$workspace_todolist` 等变量不是已接入的运行时变量。
- OpenDAN 当前每轮会手工构造一个 `[environment]` user block，包含 behavior、session、workspace、recent activity、clock；这不是本文模板变量系统的一部分。

## 1. 渲染流程

`PromptRenderEngine::render(template, vars, loader)` 分两步执行：

1. **预处理阶段**
   - 处理 `__ENV($expr)__`、`__INCLUDE(path)__`、`__EXEC(cmd)__`、`__VAR(name, $expr)__`。
   - 预处理最多执行 32 轮，因此 include 进来的内容里也可以继续包含这些指令。
   - 预处理后如果仍存在 `__ENV(`、`__INCLUDE(`、`__EXEC(`、`__VAR(` 或历史 `__OPENDAN_` 指令，会返回语法错误。

2. **upon 渲染阶段**
   - 使用 upon 引擎渲染 `{{ name }}`、`{% if %}`、`{% for %}` 等模板语法。
   - 渲染上下文来自 `RenderVars.vars` 和 `__VAR` 注册出来的变量。
   - 最终输出超过 `EngineConfig.max_total_bytes` 时会被截断，并设置 `truncated = true`。

## 2. 动态表达式解析

`__ENV` 和 `__VAR` 的参数必须是 `$expr`。`$` 后面的表达式按如下顺序解析：

1. 在 `RenderVars.env` 中查找，支持点路径，例如 `$owner.name`。
2. 在 `RenderVars.vars` 中查找，支持点路径和数组下标，例如 `$items.0`。
3. 调用调用方传入的 `ValueLoader::load(expr)`。

`ValueLoader::load` 返回 `Ok(None)` 表示“不认识这个变量”，引擎把它当作软缺失处理，不会直接报错。

值转成文本时的规则：

- JSON string 会先 `trim`，空字符串视为缺失。
- JSON null 视为缺失。
- JSON object、array、number、bool 会序列化成紧凑 JSON 字符串。

## 3. 指令

### 3.1 `__ENV($expr)__`

把 `$expr` 解析成值，并把结果以内联文本替换到当前位置。

```text
session=__ENV($session_id)__
owner=__ENV($owner.name)__
```

规则：

- 参数不以 `$` 开头会返回语法错误。
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
- 开启后通过 `sh -lc <cmd>` 执行。
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

这意味着“所有 upon 变量都必须显式写 `__VAR`”已经不是当前实现事实。建议行为：

- 静态、调用方已知的值放进 `RenderVars.vars`。
- 昂贵或异步加载的值显式写 `__VAR`，让模板依赖更清楚。
- 不要依赖自动注册来表达复杂业务依赖，尤其是 OpenDAN 接入后需要控制加载成本的变量。

转义字面量大括号：

- `\{{` 渲染为 `{{`
- `\}}` 渲染为 `}}`

## 5. EngineConfig 默认值

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `max_include_bytes` | 64 KiB | 单次 include 内容和 exec stdout 上限 |
| `max_total_bytes` | 256 KiB | 最终渲染结果上限 |
| `exec_timeout` | 10 秒 | 单条 `__EXEC` 超时 |
| `allow_exec` | `false` | 是否允许 `__EXEC` |
| `include_roots` | 空 | 为空时 `__INCLUDE` 全部失败 |
| `max_recursion_depth` | 8 | include 嵌套预处理深度 |

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

## 7. 当前 OpenDAN 主路径变量状态

以下变量是历史文档或设计文档中提到过的 OpenDAN 业务变量，但当前主路径没有接入 `PromptRenderEngine`，因此不能视为已实现模板变量。

| 历史变量 | 当前状态 |
| --- | --- |
| `$session_title` | 未作为模板变量接入；session title 当前只在手工 `[environment]` block 和默认 system prompt 拼装里使用 |
| `$current_behavior` | 未作为模板变量接入；当前 behavior name 只在 `[environment]` block 里手工输出 |
| `$params` | 未接入 |
| `$new_msg` / `$new_msg.$n` | 未接入 |
| `$owner` / `$owner.did` / `$owner.show_name` | 未接入模板渲染；相关 owner 信息存在于 session/meta/contact 逻辑中 |
| `$session_list` / `$session_list.$n` | 未接入 |
| `$workspace_list` / `$workspace_list.$n` | 未接入模板渲染；worksession 工具有自己的 workspace list 拼装逻辑 |
| `$workspace_todolist` | 未接入 |
| `$workspace_current_todo_id` | 未接入 |
| `$workspace_current_todo` | 未接入 |
| `$workspace_next_ready_todo` | 未接入 |
| `$workspace_todolist.$todo_id` | 未接入 |
| `$workspace/<rel_path>` | 未接入；通用引擎只支持最终为绝对路径且位于 `include_roots` 内的 `__INCLUDE(path)__` |
| `$last_step` / `$last_steps.$num` | 未接入模板变量；snapshot/step record 数据存在，但没有 `ValueLoader` 映射 |
| `$step_index` / `$step_num` / `$step_limit` | 未接入 |
| `$llm_result` / `$trace` | 未接入，也不建议作为普通 prompt 变量依赖 |

如果要让 OpenDAN behavior prompt 使用这些变量，需要先实现 OpenDAN 专用 `ValueLoader`，并在 `AgentSession::render_system_messages` 或新的 prompt compose 路径中调用 `PromptRenderEngine`。

## 8. 当前 OpenDAN system prompt 规则

当前 `AgentSession::render_system_messages` 的行为是：

1. 如果 `behavior.system_prompt_template` 非空，直接把它作为一条 System message 返回，不做模板渲染。
2. 如果 `system_prompt_template` 为空，则尝试读取 agent root 下的 `role.md` 和 `self.md`。
3. 如果 work session 有 objective，则追加 `## Objective` block。
4. 如果 session 目录下有 `readme.md`，追加其内容。
5. 如果以上都为空，生成一条兜底 System prompt。

因此，今天在 behavior TOML 里写：

```text
system_prompt_template = "当前会话：{{ session.id }}"
```

LLM 实际看到的仍是字面量 `当前会话：{{ session.id }}`。

## 9. 接入建议

后续如果要把 OpenDAN 接入当前通用渲染引擎，建议按以下边界实现：

1. 新建 OpenDAN 私有 `AgentSessionValueLoader`，把 session、behavior、workspace、paths、input、runtime、memory 等命名空间映射为 JSON。
2. 在调用 `PromptRenderEngine` 时设置 `EngineConfig.include_roots`，至少限制在 agent root、session dir、绑定 workspace root 等明确目录内。
3. 保持 `__EXEC` 默认关闭；behavior prompt 不应依赖 shell 动态生成内容。
4. 将 `render_system_messages` 的 `system_prompt_template` 分支改为渲染后再作为 System message。
5. 迁移旧变量名时优先使用点路径命名，例如 `$session.title`、`$behavior.name`、`$paths.agent_root`，避免继续扩散 `$workspace_todolist` 这类扁平历史变量。
6. 如果模板需要 include 固定文件，优先让 `ValueLoader` 暴露完整文件路径变量，例如 `$paths.role_prompt`、`$paths.session_readme`，不要假设通用引擎会拼接路径片段。
