# Render Prompt 模板变量说明

本文档描述 Render Prompt 的目标渲染流程，以及模板变量在各阶段的展开规则。

## 概述

Render Prompt 逻辑分为两个阶段：

1. **阶段一：OpenDAN 预处理**
   - 处理四种指令：`__OPENDAN_ENV($key)__`、`__OPENDAN_CONTENT(path)__`、`__OPENDAN_EXEC(cmd)`、`__OPENDAN_VAR(var_name, $exp)`。
   - `__OPENDAN_ENV($key)__` 是**纯文本替换**：通过动态变量获取求值后，将结果直接替换到当前位置，不做路径识别、不读文件。参数必须以 `$` 开头，表示动态变量引用。
   - `__OPENDAN_CONTENT(path)__` 是**文件内容内联**：解析路径后，读取文件全文并替换到当前位置。参数可以是 `$` 开头的动态变量引用，也可以是绝对路径。
   - `__OPENDAN_EXEC(cmd)` 是**命令执行内联**：执行 shell 命令后，将标准输出替换到当前位置。命令里可以直接引用 OpenDAN 动态路径变量，例如 `__OPENDAN_EXEC("tree -L 3 $agent_root/memory")__`。
   - `__OPENDAN_VAR(var_name, $exp)` 是**上下文注册指令**：求值后将结果注入 upon 渲染上下文，指令本身替换为空字符串（因此可以写在模板任意位置）。
   - 四者共享同一套动态变量获取机制（见第四节）。
   - 阶段完成后，文本中不应再存在任何 `__OPENDAN_*` 指令，否则报错。
   - 阶段一的输出可以继续包含 upon 占位符，留待第二阶段渲染。

2. **阶段二：标准 upon 渲染**
   - 只走标准 upon 引擎，不再做任何动态变量获取。
   - 使用阶段一构建好的上下文，完成最终的模板替换。
   - upon 上下文中的每一个变量都必须由 `__OPENDAN_VAR` 显式声明，没有隐式注入。

从职责上讲：

- **阶段一**负责"获取值"——动态、异步、需要运行时计算的值都在这一步解决。
- **阶段二**负责"渲染模板"——纯粹的标准模板引擎，只做变量替换。
- 两个阶段之间的边界是**upon 上下文**：阶段一写入，阶段二只读。

四种指令各司其职，互不重叠：

| 指令 | 职责 | 输出 |
|------|------|------|
| `__OPENDAN_ENV($key)__` | 动态值 → 文本替换 | 求值结果内联到文本 |
| `__OPENDAN_CONTENT(path)__` | 路径 → 读取文件 → 内容替换 | 文件内容内联到文本 |
| `__OPENDAN_EXEC(cmd)` | 执行命令 → 读取 stdout → 文本替换 | 命令输出内联到文本 |
| `__OPENDAN_VAR(var_name, $exp)` | 动态值 → 注册上下文 | 文本中替换为空 |

## 一、`__OPENDAN_ENV($key)__`：纯文本替换

### 1.1 语义

`__OPENDAN_ENV($key)__` 将 `$key` 对应的动态变量求值后，直接替换到模板文本的当前位置。不做路径识别，不读取文件。

**约定**：参数必须以 `$` 开头，表示这是一个动态变量引用。若参数不以 `$` 开头，报错。

### 1.2 适用场景

- 将动态计算或异步获取的文本值内联到 prompt 中
- 在进入 upon 之前完成一轮字符串替换

### 1.3 展开规则

1. 检查参数是否以 `$` 开头，若不是则报错
2. 通过动态变量获取机制（见第四节）解析 `$key` 对应的值
3. 将求值结果直接替换到当前位置
4. 若展开后的文本中包含 upon 占位符，保留到第二阶段继续渲染
5. 若 `$key` 无法解析，按空字符串处理

### 1.4 示例

```text
__OPENDAN_ENV($params.todo)__
__OPENDAN_ENV($session_id)__
```

## 二、`__OPENDAN_CONTENT(path)__`：文件内容内联

### 2.1 语义

`__OPENDAN_CONTENT(path)__` 解析参数得到文件路径，读取文件全文，将内容替换到模板文本的当前位置。

**参数支持两种形式**：
- **动态变量引用**：以 `$` 开头，通过动态变量获取机制求值得到路径（如 `$agent_root/system_prompt.md`）。
- **绝对路径**：直接写绝对路径（如 `/opt/prompts/system_prompt.md`），不经过动态变量求值。

### 2.2 适用场景

- 将外部文件（system prompt、README、配置文件等）的全文嵌入到 prompt 中
- 需要根据运行时动态决定加载哪个文件

### 2.3 展开规则

1. 判断参数形式：
   - 若以 `$` 开头，通过动态变量获取机制（见第四节）解析得到文件路径
   - 若以 `/` 开头，视为绝对路径直接使用
   - 其他情况，**报错**
2. 若路径中仍包含 `$` 开头的环境变量引用（如 `$HOME`），先用系统环境变量展开
3. 读取文件全文，将内容替换到当前位置
4. 若文件内容中包含 upon 占位符，保留到第二阶段继续渲染
5. 若最终路径不合法或文件不存在，**报错**（不静默替换为空）

### 2.4 路径安全

- 相对路径不允许 `..` 等越界访问
- 文件内容截断至 `MAX_INCLUDE_BYTES`（64KB）

### 2.5 示例

```text
__OPENDAN_CONTENT($agent_root/system_prompt.md)__
__OPENDAN_CONTENT($workspace/README.md)__
__OPENDAN_CONTENT(/opt/prompts/shared_rules.md)__
__OPENDAN_CONTENT($dynamic.prompt_source)__
```

示例流程：

- 第一个：`$agent_root` 求值得到 agent 根目录，拼接后读取 `system_prompt.md`
- 第三个：绝对路径，直接读取 `/opt/prompts/shared_rules.md`
- 第四个：`$dynamic.prompt_source` 异步求值得到 `$HOME/prompts/todo.md`，系统环境变量展开 `$HOME` → `/home/user/prompts/todo.md`，读取文件并内联

## 三、`__OPENDAN_VAR(var_name, $exp)`：上下文注册指令

### 3.1 语义

`__OPENDAN_VAR(var_name, $exp)` 将 `$exp` 求值后的结果，以 `var_name` 为键注入到 upon 渲染上下文中。指令本身在文本中替换为空字符串。

等价于：

```
upon_context[var_name] = evaluate($exp)
```

这是 upon 上下文变量的**唯一注入方式**。模板中未通过 `__OPENDAN_VAR` 声明的变量，在 upon 渲染阶段不可用。

### 3.2 适用场景

- 为 upon 模板准备所需的上下文变量
- 控制 session 数据的拉取参数（如消息条数上限）
- 按需声明，避免不必要的数据加载

### 3.3 语法

```text
__OPENDAN_VAR(var_name, $exp)
```

- `var_name`：注入到 upon 上下文中的变量名，在 upon 模板中通过 `{{var_name}}` 引用。
- `$exp`：动态变量获取表达式（见第四节），用于求出实际值。

### 3.4 示例

```text
__OPENDAN_VAR(new_msg, $new_msg.16)
__OPENDAN_VAR(todo_list, $workspace.todolist)
__OPENDAN_VAR(session_id, $session_id)
```

效果：

| 指令 | upon 上下文结果 |
|------|----------------|
| `__OPENDAN_VAR(new_msg, $new_msg.16)` | `new_msg` = 最近 16 条新消息 |
| `__OPENDAN_VAR(todo_list, $workspace.todolist)` | `todo_list` = 当前 todo 列表 |
| `__OPENDAN_VAR(session_id, $session_id)` | `session_id` = 当前会话 ID |

### 3.5 放置位置

由于 `__OPENDAN_VAR` 处理后替换为空字符串，它可以放在模板的任意位置。建议集中放在模板开头，便于一目了然地看到当前模板依赖了哪些上下文变量：

```text
__OPENDAN_VAR(new_msg, $new_msg.32)
__OPENDAN_VAR(session_id, $session_id)
__OPENDAN_VAR(current_todo, $workspace.todolist.next_ready_todo)

你是一个任务助手。当前会话：{{session_id}}
{%- if new_msg %}

最新消息：
{{new_msg}}
{%- endif %}
{%- if current_todo %}

当前待办：
{{current_todo}}
{%- endif %}
```

## 四、动态变量获取机制

`__OPENDAN_ENV`、`__OPENDAN_CONTENT`、`__OPENDAN_VAR` 共享同一套动态变量获取机制。这不是一个表达式引擎——不支持组合运算、条件判断——而是一个**可扩展的动态变量获取器**，每个 key 对应一种取值方式。

### 4.1 取值规则

给定一个 key（如 `$new_msg.16`、`$params.todo`、`$workspace/README.md`），系统匹配已注册的取值器并调用对应逻辑获取值。若无法匹配，返回空字符串。

### 4.2 当前支持的变量类型

以下按 **session context** 视角重新整理当前可用变量，便于把模板依赖映射到 prompt 上下文槽位。

状态说明：

- **已有**：当前文档中已经定义了可直接取值的动态变量
- **间接**：没有独立结构化变量，但可以通过通用路径变量或近似字段拼装
- **暂无**：当前文档未定义对应动态变量

#### 4.2.1 System Identity

| 上下文项 | 状态 | 可用变量 | 说明 |
|----------|------|----------|------|
| `role.md` | 间接 | `$agent_root/<rel_path>` | 可通过 `__OPENDAN_CONTENT($agent_root/role.md)__` 读取；前提是该文件实际存在 |
| `self.md` | 间接 | `$agent_root/<rel_path>` | 可通过 `__OPENDAN_CONTENT($agent_root/self.md)__` 读取；前提是该文件实际存在 |

#### 4.2.2 Task-Session Context

鼓励用文件获得全局信息，
`__OPENDAN_CONTENT($session_root/summary.md)` (这个默认都有，一般说明session的目标，或是sesion的文件地图)


规则里，会说明session的文件地图

与任务上下文直接相关的现有变量：

| 变量 | 说明 |
|------|------|
| `$session_title` |  session的title，String |
| `$current_behavior` | 当前behavior_name |
| `$params` | 请求参数 JSON 对象 |
| `$new_msg` | 新消息（默认最多 32 条） |
| `$new_msg.$n` | 指定拉取上限，`$n` 为 1–4096 |
| `$owner` | 当前 owner 信息，Json 对象 |
| `$owner.did` | 当前 owner 的 DID |
| `$owner.show_name` | 当前 owner 的显示名 |
| `$owner.contact` | 当前 owner 的联系人信息 Json |
| `$session_list` | 最近会话列表（默认最多 16 条） |
| `$session_list.$n` | 指定拉取上限 |
| `$workspace_list` | 最近本地工作区列表（默认最多 16 条）,Json数组 |
| `$workspace_list.$n` | 指定拉取上限 ，Json数组|

#### 4.2.3 Workspace Context


注意workspace就是原来session bind的唯一的local workspace

`__OPENDAN_CONTENT($workspace/summary.md)`

可以得到workspace的文件地图


| 变量 | 说明 |
|------|------|
| `$workspace_todolist` | todo 列表信息, 已经渲染好的字符串 |
| `$workspace_current_todo_id` | 当前todo 的代码（如 T01） |
| `$workspace_current_todo` | 当前 todo 的Json |
| `$workspace_next_ready_todo` | 下一个就绪 todo 的json |
| `$workspace_todolist.$todo_id` | 通过todo_id得到todo的json ||
| `$workspace/<rel_path>` | 当前 `workspace` 目录下的文件路径 |

**路径安全**：`rel_path` 必须是相对路径，不允许 `..` 等越界访问。

#### 4.2.4 Memory Context

使用基于token limit的机械压缩控制，支持启用3种
- Agent Memory
- History Message
- Session Step Records

(worklog后期将只用于workspace的审计，不再在promp中使用)

#### 4.2.5 Execution Context


与执行上下文直接相关的现有变量：

| 变量 | 说明 |
|------|------|
| `$last_step` | 引用上一步的StepRecord |
| `$last_steps.$num` | 前n步的StepRecord Array,按顺序从远到近排列 |
| `$step_index` | 当前step |

| `$step_num` | 当前session的step_num 
| `$step_limit` | 当前behavior的步骤上限 |
| `$llm_result` | 去掉，可以在StepRecord中访问 |
| `$trace` | 执行追踪信息，去掉，提示词渲染不看这个 |

### 4.2 实现差异与待补齐

- `$workspace_list`：当前实现可补齐为最近本地 workspace 列表的 alias，但返回值仍是渲染后的文本，不是本节表述中的 Json 数组。
- `$workspace_current_todo`、`$workspace_next_ready_todo`、`$workspace_todolist.$todo_id`：当前实现可提供 todo 详情文本，但不是本节表述中的结构化 Json。
- `$owner`：当前实现通过 contact mgr 查询 owner 联系人，返回的 Json 至少包含 `user_id`、`show_name`、`contact.did/groups/tags/bindings`；它不是完整的 `users/$id/settings` 镜像。
- `$last_step`、`$last_steps.$num`：当前实现只能稳定提供 step record 的渲染文本；若要严格对齐为 `StepRecord` / `StepRecord Array`，需要把 session 动态变量取值链路扩展为支持结构化非字符串返回。
- `$step_limit`：当前只能在调用方显式放入 render `env_context` 时获取；`agent_environment.rs` 仅靠 session 还无法独立反查当前 behavior config。
- 4.2.4 `Memory Context` 目前只有压缩策略说明，尚未整理出一组已经稳定暴露给 `agent_environment.rs` 的动态变量名，需要后续补全。


### 4.3 扩展

新增一种动态变量时，只需注册对应的取值器即可。三种指令会自动支持新变量，无需修改渲染流程。

## 五、阶段二：标准 upon 渲染

阶段一完成后，模板进入标准 upon 渲染流程。

这一阶段的约束：

- **只使用已构建好的 upon 上下文**，不再做任何动态变量获取
- 不承担 OpenDAN 专有的预处理职责
- 只做标准 upon 模板替换

### 5.1 模板语法

upon 语法风格接近 Liquid / Jinja，支持表达式输出和控制块。

**表达式输出**：`{{ var_name }}`，将变量值输出到结果中。

```text
{{session_id}}
{{new_msg}}
```

**条件块**：`{% if %}...{% endif %}`，当变量为 falsy（`None`、`false`、`0`、空串、空列表、空 map）时跳过。

```text
{%- if new_msg %}
最新消息：
{{new_msg}}
{%- endif %}
```

**循环**：`{% for item in seq %}...{% endfor %}`，遍历列表或 map。

```text
{% for msg in messages %}
- {{msg.sender}}: {{msg.content}}
{% endfor %}
```

**空白控制**：`{%-` / `-%}` 去掉标签一侧的相邻空白，用于避免条件块和循环引入多余空行。

所有 `var_name` 必须是通过 `__OPENDAN_VAR` 注册过的。upon 引擎的完整语法参见 `upon::syntax`。

### 5.2 转义

若需输出字面量 `{{` 或 `}}`，使用反斜杠转义：

- `\{{` → `{{`
- `\}}` → `}}`

## 六、完整示例

### 6.1 模板源文件

```text
__OPENDAN_VAR(new_msg, $new_msg.16)
__OPENDAN_VAR(session_id, $session_id)
__OPENDAN_VAR(current_todo, $workspace.todolist.next_ready_todo)

__OPENDAN_CONTENT($agent_root/system_prompt.md)__

你是一个任务助手。当前会话：{{session_id}}

项目说明：
__OPENDAN_ENV($params.project_name)__ 的工作区
{%- if new_msg %}

最新消息（最近 16 条）：
{{new_msg}}
{%- endif %}
{%- if current_todo %}

当前待办：
{{current_todo}}
{%- endif %}

请根据以上信息回复用户。
```

这里用到了 upon 的条件块语法：`{% if new_msg %}...{% endif %}`。当 `new_msg` 为空（falsy）时，整个"最新消息"段落不会出现在最终 prompt 中。`{%-` 的减号用于消除条件块引入的多余空行。

### 6.2 渲染过程

**阶段一：**

1. `__OPENDAN_VAR(new_msg, $new_msg.16)` → 拉取最近 16 条消息，注入 upon 上下文 `new_msg`，指令替换为空
2. `__OPENDAN_VAR(session_id, $session_id)` → 获取会话 ID，注入上下文 `session_id`，指令替换为空
3. `__OPENDAN_VAR(current_todo, $workspace.todolist.next_ready_todo)` → 获取下一个就绪 todo，注入上下文 `current_todo`，指令替换为空
4. `__OPENDAN_CONTENT($agent_root/system_prompt.md)__` → 求值得到文件路径，读取文件全文（如 "你是 OpenDAN 的智能助手。"），内联到当前位置
5. `__OPENDAN_ENV($params.project_name)__` → 求值得到项目名（如 "MyProject"），直接替换

**阶段一输出（upon 占位符和控制块保留）：**

```text
你是 OpenDAN 的智能助手。

你是一个任务助手。当前会话：{{session_id}}

项目说明：
MyProject 的工作区
{%- if new_msg %}

最新消息（最近 16 条）：
{{new_msg}}
{%- endif %}
{%- if current_todo %}

当前待办：
{{current_todo}}
{%- endif %}

请根据以上信息回复用户。
```

**阶段二（假设 new_msg 有内容、current_todo 为空）：**

upon 引擎执行变量替换和条件判断。因为 `current_todo` 为空（falsy），对应的 `{% if current_todo %}` 块被跳过，最终输出：

```text
你是 OpenDAN 的智能助手。

你是一个任务助手。当前会话：sess_abc123

项目说明：
MyProject 的工作区

最新消息（最近 16 条）：
[用户A] 请帮我看一下这个 bug
[用户B] 已经修复了，PR 在这里

请根据以上信息回复用户。
```

## 七、限制

| 限制项 | 默认值 |
|--------|--------|
| 总输出长度 | `MAX_TOTAL_RENDER_BYTES`（256KB） |
| 单文件内容（`__OPENDAN_CONTENT`） | `MAX_INCLUDE_BYTES`（64KB） |

## 八、返回值

`AgentTemplateRenderResult` 包含：

| 字段 | 说明 |
|------|------|
| `rendered` | 渲染后的最终文本 |
| `env_expanded` | `__OPENDAN_ENV` 成功展开数量 |
| `env_not_found` | `__OPENDAN_ENV` 未找到数量 |
| `content_loaded` | `__OPENDAN_CONTENT` 成功加载数量 |
| `content_failed` | `__OPENDAN_CONTENT` 加载失败数量 |
| `var_registered` | `__OPENDAN_VAR` 成功注册数量 |
| `var_failed` | `__OPENDAN_VAR` 求值失败数量 |

## 九、实现边界说明

当前代码中仍能看到历史上的自定义模板渲染实现和隐式变量注入逻辑，但目标架构应当收敛为：

1. **阶段一**：OpenDAN 预处理——三种指令各司其职，共享同一套动态变量获取机制。
2. **阶段二**：纯标准 upon 渲染——只使用阶段一构建好的上下文。

核心原则：

- **职责单一**：`__OPENDAN_ENV` 只做文本替换，`__OPENDAN_CONTENT` 只做文件内联，`__OPENDAN_VAR` 只做上下文注册。
- **显式声明**：upon 上下文中的每个变量都必须由 `__OPENDAN_VAR` 显式声明，不存在隐式注入。
- **职责分离**：动态取值和模板渲染是两个独立阶段，不交叉。
- **统一求值**：三种指令使用同一套动态变量获取器，扩展一次，三处受益。
