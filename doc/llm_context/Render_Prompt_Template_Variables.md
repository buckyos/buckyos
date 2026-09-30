# Render Prompt 模板变量说明

本文档是 `llm_context::PromptRenderEngine` 的实现参考，写给修改 / 扩展模板引擎自身的人。引擎位于 [`src/frame/llm_context/src/prompt_engine.rs`](../../src/frame/llm_context/src/prompt_engine.rs)，类型 `PromptRenderEngine`，是 `llm_context` 的通用能力 —— 不认识 OpenDAN 的 session、workspace、owner、todo 等业务概念。

> OpenDAN 这一侧实际暴露的变量契约、`AgentSessionValueLoader` 的字段映射、`include_roots` 当前设置、behavior 模板可以写什么，全部在 [Agent Enviroment.md](Agent%20Enviroment.md)。本文不重复那一层。

实际指令名是 `__ENV`、`__INCLUDE`、`__EXEC`、`__VAR`，不是更早设计稿里的 `__OPENDAN_ENV` 等前缀名。

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

`ValueLoader::load` 返回 `Ok(None)` 表示"不认识这个变量"，引擎把它当作软缺失处理，不会直接报错。

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

这意味着"所有 upon 变量都必须显式写 `__VAR`"已经不是当前实现事实。建议行为：

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

## 7. 修改 / 扩展引擎时的注意事项

- 新指令名要与现有 `__XXX__` 前缀风格一致，并在 §1 的预处理"未消化指令"检测里加新前缀，否则会被当成语法错误。
- 任何新指令默认要"失败可恢复"：失败时打 stats + 输出 `<!-- ... failed: ... -->` 标记，不要让整个 render 终止。
- `ValueLoader::load` 的成功 / 软缺失 / 硬错误三态语义不要破坏：调用方依赖 `Ok(None)` 来表达 loader 不持有这个名字。
- 新增 `EngineConfig` 字段必须给 `Default` 实现，并在本文 §5 表格里补充。
- 触发 IO / 进程的新指令要带超时 + 字节上限；不要假设调用方已经做了沙箱。
- 业务变量映射（`$session.*`、`$workspace.*` 等）一律不进引擎，由调用方的 `ValueLoader` 提供 —— OpenDAN 的实现见 [Agent Enviroment.md](Agent%20Enviroment.md)。
