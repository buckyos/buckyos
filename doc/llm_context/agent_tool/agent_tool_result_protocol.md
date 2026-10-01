# AgentToolResult 协议说明

`AgentToolResult` 是 OpenDAN AgentTool 的统一执行结果协议。

它的定位是：**面向 Agent Loop / StepRecord prompt 渲染，并带 Runtime 控制语义的工具执行结果协议**。

它不是一个通用的 tool-to-tool 业务数据交换协议。工具之间如果需要交换稳定的业务数据，应定义各自的 typed schema、artifact reference 或专用 API；不要从 `title` / `summary` / `output` 里解析业务语义。

## 设计目标

### 1. 支撑 StepRecord 的分级渲染

Behavior Loop 在下一个 Step 的 prompt 中消费的不是孤立的工具 stdout，而是 `StepRecord`：一个 Step 的决策（Step 内可能经过多次推理 Round）加上 dispatcher 执行后的 action results。标准 function call loop 没有 Step，工具结果以按 `call_id` 配对的 tool result 消息回灌（见“调用身份、结果归属与 Pending 回填”）。

`AgentToolResult` 的核心设计目标是让同一个工具结果可以按 StepRecord 所处位置稳定渲染：

- hot tail / 最近 step：保留完整命令表达和完整主返回体
- compact history：保留可独立理解的多行摘要
- digest / list：保留一行标题
- history summary：不再保留单个工具结果，只保留跨 step 的任务语义

因此协议同时提供：

- `cmd_name` / `cmd_args`：Full 渲染所需的原始命令表达
- `output` / `detail`：Full 渲染所需的完整主返回体
- `summary`：Medium 渲染所需的多行压缩
- `title`：Min 渲染所需的一行压缩
- `status` / `task_id` / `pending_reason` / `check_after` / `return_code`：Runtime 控制字段

### 2. 保留关键控制语义

所有工具结果都收敛到三种状态：

- `success`
- `error`
- `pending`

Agent Loop、WorkLog、`check_task`、审批等待、长任务等待都可以基于这些控制字段工作：

- `status`
- `task_id`
- `pending_reason`
- `check_after`
- `return_code`
- `partial_output`

这些字段是 Runtime 控制字段，不是 prompt 渲染字段。

### 3. 保留命令原始表达

`cmd_name` / `cmd_args` 表示这次工具调用的命令形态。

它们不是摘要字段，也不应该被 `title` / `summary` 替代。对写入类操作，如果完整写入意图本身就是命令的一部分，可以放在 `cmd_args` 中；Full / 不压缩展示时应能看到 `cmd_name + cmd_args`。

注意：

- `cmd_name` / `cmd_args` 在协议语义上是原始字段，工具开发者填进去是什么就是什么
- 渲染层不应对 `cmd_name` / `cmd_args` 做摘要或改写
- 在 `Min` / `Medium` 等压缩档下，渲染层可以选择不展示 `cmd_name` / `cmd_args`
- 当压缩档不展示原始命令字段时，`title` / `summary` 应包含必要的命令信息
- prompt 渲染器仍可以因为全局 token budget 对最终文本做硬裁剪
- `cmd_args` 是参数文本，不是 JSON 数组

### 4. 明确完整返回体：`output` 或 `detail`

同一份主结果不应同时出现在 `output` 和 `detail` 中。默认情况下，一个结果只填其中一个字段作为主返回体。

允许一个工具同时填写 `output` 和 `detail`，但前提是二者承载不同信息，并且工具文档必须明确说明各自用途。例如：`detail` 放结构化数据，`output` 放可在 bash 终端复现的人读文本。

`output` 表示：

> 这个命令如果在 bash 中执行，用户会在终端里完整看到的文本输出(内置命令需要加特殊参数，默认执行是返回agent_tool_result)。


因此：

- 普通 bash 命令使用 `output`
- 文本型 Agent Tool 如果主结果就是终端文本，可以使用 `output`，也可以使用字符串 `detail`
- `output` 是纯文本，不要求 JSON，也不应要求 consumer 反序列化
- `exec_bash` 默认把 tmux / stdout / stderr 视角下用户会看到的输出收敛到 `output`

`detail` 表示：

> Agent Tool 内部的完整返回。

因此：

- 内置 Agent Tool 可以使用 `detail`
- `detail` 是 JSON value，可以是 object、array、string 等
- 结构化工具通常使用 object / array
- `read_file` 这类完整结果本质是文本的工具，可以选择 `output`，也可以选择字符串 `detail`
- 选择字符串 `detail` 时，不需要再把同一份内容放进 `output`
- 选择 object / array `detail` 时，不需要搞 JSON-in-string
- `detail` 的业务 schema 或文本语义由具体工具定义
- 普通 bash 命令不要把主输出塞进 `detail`

`output` 和 `detail` 的选择规则：

| 工具类型 | 主返回体 |
| --- | --- |
| 普通 bash | `output` |
| bash-like 文本工具 | `output` 或字符串 `detail` |
| 结构化 Agent Tool | object / array `detail` |

`return_code`、`task_id`、`partial_output` 等控制字段独立于主返回体存在，不参与 `output` / `detail` 的选择。

### 5. 明确渲染压缩字段：`title` / `summary`

和 prompt/history 压缩渲染有关的专用字段是：

- `title`
- `summary`

`title` 是对 `cmd_name` / `cmd_args` 和结果的一行压缩。

`summary` 是对 `cmd_name` / `cmd_args` 和结果的多行压缩。

二者都是给人和 LLM 读的展示字段，不是 Runtime 控制字段，也不是机器可读业务字段。它们服务于 `Min` / `Medium` 压缩展示，不替代 Full 渲染中的 `cmd_name` / `cmd_args` / `output` / `detail`。

推荐规则：

- `title` 应该短、一行、稳定，例如 `cargo test => failed (exit=101)`
- `summary` 应该能独立读懂，包含关键结论和必要上下文
- `summary` 可以重复 `title` 中的信息，不需要为了避免重叠而牺牲可读性
- consumer 不应从 `title` / `summary` 中 parse 控制语义
- `title` / `summary` 可以包含人类可读的状态、退出码、task id 等冗余描述，但 Runtime 判断状态时必须读取 `status` / `return_code` / `task_id` 等控制字段

## StepRecord 渲染规则

`AgentToolResult` 的渲染设计首先服务于 StepRecord prompt。单个工具结果的 `Min` / `Medium` / `Full` 规则只是基础；真正进入下一个 Step 的 LLM 输入时，它会被包进 StepRecord 历史结构里。

当前基线实现是 `llm_context::XmlStepRenderer`。本节基于现有实现描述 StepRecord 的渲染基线，并补充目标形态。它使用 XML-like 的边界标记，但渲染文本只给 LLM 消费，不要求是标准 XML 文档。外层 root wrapper 使用 `<<tag_name>>` / `<</tag_name>>`，用于提示 LLM 这是 prompt 协议边界而不是严格 XML；内部字段只在必要位置做转义。

### Message 序列

一个 Behavior Step 开始推理前的理想 message 序列是（Step 内后续的推理还会看到该 Step 的 inner transcript）：

```text
system
user: step_history
assistant: hot step intent
user: hot step action results
assistant: hot step intent
user: hot step action results
user: behavior init / on_behavior_switch UserMessage
```

其中：

- `step_history` 是一条 user message，承载已经沉淀的 StepRecord 历史。
- hot tail 是最近若干个完整 `(assistant, user)` step pair。
- 当 context 不够时，hot tail 中较旧的一部分会合并进 `step_history`，并在 `step_history` 内压缩或裁剪。
- `step_history` 是可选的；Behavior 刚开始且没有历史时，可以单独给一条 behavior init user message。
- 如果既有 `step_history` 又需要注入 behavior init / on_behavior_switch UserMessage，应先渲染 `step_history`，再追加该 UserMessage，保证沉淀历史在时间顺序上更早。

### Full Step

一个完整 step 渲染为一组严格相邻的 `(assistant, user)` message：

1. `assistant` message：该 Step 的 LLM 决策原文，即 `step.assistant_text`。
2. `user` message：该 Step 的 action 执行结果，使用 `<<last_step_action_results>>` wrapper。

当前 wrapper 名称为复数：`last_step_action_results`。

````text
<<last_step_action_results behavior="<behavior_name>" step="<step_index>">>
- AgentToolResult.title

```output
AgentToolResult.output | AgentToolResult.detail
```

- AgentToolResult.title

```output
AgentToolResult.output | AgentToolResult.detail
```
<</last_step_action_results>>
````

规则：

- `behavior` 来自 `step.meta.behavior_name`。
- `step` 来自 `step.meta.step_index`。
- `step.actions[i]` 与 `step.action_results[i]` 按 index 配对渲染。
- 如果存在未配对的 `action_results`，以 `Step result` / `Step error` 形式渲染。
- `messages_sent` 追加渲染为 `Message sent to <target>`，body 为 `Message sent.`。
- Full step 对应 `AgentToolResult.Full`，应展示原始命令表达和完整主返回体：`cmd_name + cmd_args + output/detail`。

### Action 标题

当前实现会把 action 渲染成紧凑的一行 command。后续如果 action result 已经是合法 `AgentToolResult`，标题应优先使用 `AgentToolResult.title`；否则按 action 参数降级构造。

Behavior XML action 的 ID 由运行时执行前分配，LLM 不需要输出。最近 step 以 assistant/user pair 回灌时，assistant action 标签会补 `call_id="<id>"`，对应 action result 标题前会补 `#<id>`，用于把命令和执行结果稳定关联起来。

| Action | 降级标题规则 |
| --- | --- |
| `exec_bash` | 使用 `command` 参数，压缩空白并截断到 160 字符 |
| `read` | `read <path-or-uri> [first_chunk=...] [range=...]` |
| `write_file` | `write_file <path> mode=<mode>` |
| `edit_file` | `edit_file <path> [old_string="..."]` |
| 其它 action | `<name> key=value ...`，key 排序；跳过 `content` / `old_string` / `new_string` / `from_user_did` |

降级标题最终通常是：

```text
Run <command>
```

### Action Result Body

当 action result 是合法 `AgentToolResult` 时，body 按展示级别选择：

| Level | Body |
| --- | --- |
| `Full` | `output` 或 `detail` |
| `Medium` | `summary` |
| `Min` | `title` |

当前实现中的 `Observation` 还不是完整的 `AgentToolResult` 展示器，因此会按 `Observation` 降级映射：

| Observation | Full body |
| --- | --- |
| `Success` | `content` 如果是 string 则直接使用，否则 JSON stringify；空内容显示 `Success` |
| `Error` | `Error: <message>` |
| `Pending` | `Pending` |
| `Cancelled` | `Cancelled: <reason>` |
| `Unresolved` | `Unresolved (result unknown, side effects unconfirmed): <reason>`，未执行时为 `Not executed: <reason>` |

如果结果被截断，在 body 末尾追加：

```text
[truncated]
```

当前 full step 中 success body 仍有保护性上限，默认最多渲染 4096 字符。这个上限属于实现参数，不是协议字段。

### Step History

`StepRecord` 进入历史后按当前 behavior 和新旧程度分级渲染：

| 场景 | 渲染形态 |
| --- | --- |
| 当前 behavior 的最近 step | 与 Full Step 相同 |
| 当前 behavior 的较旧 step | 仍渲染 `(assistant, user)` pair，但 assistant text 和 action result body 会截断 |
| 跨 behavior 继承的 step | 渲染进 `step_history`，不再作为 hot tail pair |
| History summary | 渲染进 `step_history`，只保留跨 step 摘要语义 |

`step_history` 是一条 user message，推荐外层形态：

````xml
<<step_history>>
<step_record behavior="<behavior_name>" index="<step_index>" started_at_ms="<started_at_ms>" ended_at_ms="<ended_at_ms>" compression="<full|compact|summary>">
<observation>...</observation>
<thought>...</thought>
<actions>
- AgentToolResult.title

```output
AgentToolResult.summary | AgentToolResult.title
```
</actions>
</step_record>
<history_summary steps="<start>..<end>" count="<count>" started_at_ms="<started_at_ms>" ended_at_ms="<ended_at_ms>" behaviors="<behavior_names>">...</history_summary>
<</step_history>>
````

较旧 step 的 compact action result 仍可以使用 `<<last_step_action_results>>` wrapper，但 body 更短：

- compact history step 优先展示 `AgentToolResult.summary`。
- inherited step record 可以展示 `summary` 或 `title`。
- success body 截断后如果为空，显示 `Success`。
- error / cancelled body 按 compact 上限截断。
- pending 显示 `Pending`。

跨 behavior 继承的 step 不应继续作为新 behavior 的完整 assistant/user hot tail 出现，否则新 behavior 会在错误的 system prompt 和 action view 下读取旧 intent。

## JSON 协议

序列化到 CLI / bash stdout 后，`AgentToolResult` 的字段全集示意如下。

不要把下面的字段全集示意理解为每个结果都要填写全部字段。实际结果中，`output` 和 `detail` 默认只填一个；如果同时填写，二者必须承载不同信息，不能重复同一份主结果。

```json
{
  "agent_tool_protocol": "1",
  "status": "success|error|pending",

  "cmd_name": "bash-style command name",
  "cmd_args": "bash-style argument text",

  "title": "one line compressed view",
  "summary": "multi-line compressed view",

  "output": "complete terminal text output",
  "detail": {}, // 通常不写

  "return_code": 0,

  //下面的字段只有在pending的时候才有
  "task_id": "optional",
  "partial_output": "optional pending progress text",
  //可以随意扩展
  "pending_reason": "long_running|user_approval|wait_for_install",
  //预期多久后完成
  "check_after": 5
}
```

字段分组：

| 分组 | 字段 | 说明 |
| --- | --- | --- |
| 协议识别 | `agent_tool_protocol` | 标识这是 AgentToolResult 协议结果 |
| 控制语义 | `status`, `task_id`, `pending_reason`, `check_after`, `return_code`, `partial_output` | Runtime / Agent Loop / `check_task` 使用 |
| 命令表达 | `cmd_name`, `cmd_args` | 原始调用意图 |
| 渲染压缩 | `title`, `summary` | prompt / history 压缩视图 |
| 完整返回体 | `output` 或 `detail` | Full / 不压缩展示和调试使用 |

说明：

- `agent_tool_protocol` 同时承担协议识别和 schema 版本声明
- 当前协议版本为 `"1"`

## 字段定义

### `agent_tool_protocol`

协议标志位和 schema 版本号。

规则：

- 自有 AgentTool 输出的协议 JSON 必须显式带上该字段
- 当前版本为 `"1"`
- 普通 bash 输出即使碰巧长得像 JSON，也不能仅凭 JSON 结构猜成 AgentToolResult
- `exec_bash` 只有在 stdout 是带合法 `agent_tool_protocol` 的 AgentToolResult envelope 时，才应把 stdout 解析为 AgentToolResult

版本演进规则：

- 主版本号变更表示 breaking change
- 在同一主版本内只能新增可选字段，不能改变已有字段语义

### `status`

结果状态，取值固定为：

- `success`
- `error`
- `pending`

`status` 是控制字段。Agent Loop 可以用它判断是否失败、是否等待、是否继续后续行为。

不要通过 `summary`、`output` 或 `return_code` 反推最终状态。它们只能作为辅助信息。

### `cmd_name` / `cmd_args`

命令名和参数文本。

规则：

- 使用 bash 风格文本
- `cmd_name` 表示命令名或工具名
- `cmd_args` 表示参数文本
- 二者拼接后形成完整 command line
- 对写入类操作，完整写入意图可以保留在 `cmd_args`
- Full / 不压缩展示时应优先展示 `cmd_name + cmd_args`
- `Min` / `Medium` 压缩档可以不展示这两个字段，此时 `title` / `summary` 应承担压缩后的命令表达

示例：

```json
{
  "cmd_name": "write_file",
  "cmd_args": "/workspace/demo.txt --mode=write <<'EOF'\nhello\nEOF"
}
```

### `title`

一行压缩视图。

用途：

- 最小历史展示
- WorkLog digest
- 调试列表

要求：

- 一行
- 简短
- 能表达命令和结果状态
- 不作为控制字段解析
- 可以包含状态、退出码、task id 等人读信息；consumer 需要这些值时必须读取对应控制字段

示例：

```text
cargo test => failed (exit=101)
read_file demo.txt range=1-20 => success
check_task 123 => pending (long_running)
```

### `summary`

多行压缩视图。

用途：

- Medium 档 prompt
- LLM 可读的主要结果摘要
- 任务列表或工作日志中的人读摘要

要求：

- 能独立读懂
- 可以多行
- 应包含关键结论、错误摘要、重要路径或下一步提示
- 不要求可机读
- 不作为控制字段解析

`summary` 可以和 `title` 有信息重叠。Medium 档只显示 `summary`，因此 `summary` 不需要为了避免重复而省略关键信息。

`summary` 可以包含状态、退出码、task id 等人读信息；consumer 需要这些值时必须读取对应控制字段。

### `output`

bash 语义的完整文本输出。

定义：

> 这个命令如果在 bash 中执行，用户会在终端里完整看到的文本输出。

规则：

- 普通 bash 命令主结果放这里
- `exec_bash` 默认回退逻辑把用户会看到的混合输出放这里
- `output` 不要求是 JSON
- consumer 不应把 `output` 当结构化数据解析
- 如果同一份主结果已经放在 `detail`，不要再重复放进 `output`
- 如果同时填写 `output` 和 `detail`，`output` 必须承载不同于 `detail` 的信息，并由工具文档说明用途

### `detail`

Agent Tool 内部的完整返回。

规则：

- `detail` 是 JSON value，可以是 object、array、string 等
- 结构化 Agent Tool 通常把主结果放在 object / array `detail`
- 文本型 Agent Tool 可以选择把主结果放在字符串 `detail`
- `read_file` 这类工具可以用 `output` 返回完整文本，也可以用字符串 `detail` 返回完整文本，效果等价
- 如果同一份主结果已经放在 `detail`，不要再重复放进 `output`
- 如果同一份主结果已经放在 `output`，不要再重复放进 `detail`
- 如果同时填写 `output` 和 `detail`，二者必须承载不同信息，并由工具文档说明用途
- 不要把 `summary` 这种人读摘要塞进 `detail` 来替代顶层 `summary`

`detail` 可以用于 Runtime 内部、CLI、WorkLog 或测试读取工具的完整返回。但它不是跨所有工具统一的业务数据交换 schema。

### `return_code`

命令退出码。

规则：

- 有 shell / bash 退出码语义时填写
- 普通 bash 命令应填写
- 内置结构化工具没有明确退出码时可以省略
- `return_code` 不替代 `status`

### `task_id`

当 `status = pending` 时，用于后续 `check_task` 轮询。

### `partial_output`

`pending` 时的阶段性输出。

规则：

- 用于暴露长任务当前进展
- 不要求完整
- 不替代最终 `output`

### `pending_reason`

当前使用以下值：

- `long_running`
- `user_approval`
- `wait_for_install`

### `check_after`

建议 Agent 多少秒后再次轮询。

仅在 `status = pending` 时有意义。

## 单个 AgentToolResult 渲染规则

本节定义单个 `AgentToolResult` 在不同展示级别下应使用哪些字段。

| Level | 使用字段 | 说明 |
| --- | --- | --- |
| `Min` | `title` | 一行压缩视图 |
| `Medium` | `summary` | 多行压缩视图；不需要再拼 `title` |
| `Full` | `cmd_name + cmd_args + output/detail` | 不压缩展示命令和完整返回体 |

要点：

- `Min` 不读取 `summary`
- `Medium` 只读取 `summary`
- `Full` 不依赖 `title` / `summary`，而是展示原始命令表达和完整返回体
- 如果某个字段为空，渲染器可以做降级展示，但工具开发者不应依赖降级逻辑

具体字符截断长度、`output` 取头还是取尾、代码块格式、空字段降级策略等属于实现细节，不在本协议范围内。协议只约束各展示级别的主要信息来源。

推荐 Full 展示形态：

普通 bash：

````text
$ cargo test
```output
...
```
````

结构化 Agent Tool：

````text
read_file demo.txt range=1-20
```json
{
  "content": "..."
}
```
````

## `exec_bash` 约定

`exec_bash` 本身也是一个标准工具。它负责执行 bash 命令，并把 bash 的执行结果转换成 `AgentToolResult`。

普通 bash 命令的推荐输出：

```json
{
  "agent_tool_protocol": "1",
  "status": "error",
  "cmd_name": "cargo",
  "cmd_args": "test",
  "title": "cargo test => failed (exit=101)",
  "summary": "cargo test failed with exit code 101.\nLast error: unresolved import `foo` in src/lib.rs.",
  "output": "$ cargo test\n...\nerror[E0432]: unresolved import `foo`\n...",
  "return_code": 101
}
```

如果 `exec_bash` 执行的命令在 stdout 明确输出合法 AgentToolResult，`exec_bash` 可以把该结果转发为结构化工具结果。

普通 bash 的 stdout 即使碰巧是 JSON，也不能在缺少合法 `agent_tool_protocol` 时被隐式当成 `detail` 或 AgentToolResult。

## 内置 Agent Tool 约定

Agent Tool 的推荐输出：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "read_file",
  "cmd_args": "demo.txt range=1-20",
  "title": "read_file demo.txt range=1-20 => success",
  "summary": "Read 20 lines from demo.txt.",
  "detail": {
    "path": "demo.txt",
    "range": "1-20",
    "content": "..."
  }
}
```

约定：

- 必须填写 `status`
- 应填写 `cmd_name` / `cmd_args`
- 应填写 `title` / `summary`
- 主结果是结构化 JSON 时填写 object / array `detail`
- 主结果是文本时，可以填写 `output`，也可以填写字符串 `detail`
- 不要同时把同一份主结果塞进 `output` 和 `detail`

文本型 Agent Tool 也可以这样返回：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "read_file",
  "cmd_args": "demo.txt range=1-20",
  "title": "read_file demo.txt range=1-20 => success",
  "summary": "Read 20 lines from demo.txt.",
  "detail": "line 1\nline 2\n..."
}
```

等价地，也可以选择把完整文本放在 `output` 中；关键约束是不要两边都有同一份主结果。

## Pending 结果

长任务或等待用户审批时返回 `pending`。

示例：

```json
{
  "agent_tool_protocol": "1",
  "status": "pending",
  "cmd_name": "cargo",
  "cmd_args": "test",
  "title": "cargo test => pending (long_running)",
  "summary": "cargo test is still running. Partial output shows compilation in progress.",
  "return_code": 0,
  "task_id": "12345",
  "partial_output": "Compiling opendan v0.1.0 ...",
  "pending_reason": "long_running",
  "check_after": 5
}
```

规则：

- `task_id` 用于后续 `check_task`
- `check_after` 是建议轮询间隔
- `partial_output` 是阶段性输出
- 最终完成后，`check_task` 应返回新的 `success` 或 `error` 结果

## 调用身份、结果归属与 Pending 回填

本节说明宿主（llm_context / xllm / libopendan）如何归属一次工具调用的结果。Round / Step / Turn 的定义见 [../readme.md](../readme.md)。

| 对象 | 身份 | 说明 |
| --- | --- | --- |
| 一次工具调用 | `call_id` | function call 模式是模型返回的原生 tool call id；behavior 模式由运行时在执行前为每个 action 分配。结果回填、in-flight 记录、Session worklog 的 `action_result` 都按它关联 |
| Behavior Step | `(run_id, step_index)` | 一个 Step 可以携带多个 action；`step_index` 在 Session 内不全局唯一（independent context 各自编号） |
| Turn | libopendan Session 的 `turn` | 由 Session 判定开启和完成，工具调用本身不改变它 |

- function call 模式：一次推理（Round）返回的全部原生 tool calls 组成一个工具批次，每个结果以 tool result 消息按 `call_id` 回灌；整批完成消耗一次工具迭代（`ToolPolicy.max_tool_iterations`），单个响应的调用数受 `max_calls_per_round` 限制（它不限制 Step 的 action 数）。
- behavior 模式：action 结果按序写入当前 Step 的 `action_results`，全部就绪后该 Step 折叠为 `StepRecord`，下一个 Step 通过 `<<last_step_action_results>>` 看到它们；一个分派 action 的 Step 也消耗一次工具迭代。
- 工具运行上下文 `SessionRuntimeContext.tool_call_index` 是宿主在每次工具调用时递增的调用序号（xllm 由 `XllmToolManager` 维护，只在内存中，run 恢复后重新计数；CLI 进程中为 0），只用于日志和请求 id。它不是 `step_index`，也不是 Turn 编号，不能用来推导 Step 或 Turn。
- 工具内部再发起的模型调用（如 `llm_understand_media`、`llm_explore` 内部的 xllm run）是这次调用的嵌套推理：有自己的 run 和用量统计，不计入父 run 的 Round，不是父 Step，也不开启、推进或完成 Session Turn。对父循环而言只有一次调用和一个结果。
- 调用进行中宿主崩溃：libopendan 在调用前登记 in-flight 记录，只有包含结果的 checkpoint 才清除它；恢复时没有已持久化结果的调用被回填为 `Unresolved`（效果未知），不会自动重放。

Pending 回填：`status = pending` 的结果在宿主中映射为 `Observation::Pending`，llm_context 的处理取决于 `ToolPolicy.allow_deferred`：

- `allow_deferred = true`：分派停在该调用，run 以 `PendingTool` 挂起，批次或 Step 中剩余的调用保留在状态里。宿主拿到结果后用 `ResumeFill::ToolResults { results: [(call_id, Observation)] }` 按 `call_id` 回填（缺失、重复或未知的 id 会被拒绝），再继续剩余调用。回填不重新推理已完成的决策、不重放已执行的工具、不重复扣工具迭代，也不新开 Turn；behavior 模式下结果仍属于原来那个 Step。
- `allow_deferred = false`：`Pending` 视为违约，该调用记为效果未知，run 以 `Internal` 错误结束。当前 xllm 和 libopendan 托管执行都使用 `false`（libopendan 恢复时遇到 `PendingTool` 挂起也会拒绝继续），所以本文 `task_id` + `check_task` 的轮询模型尚未在这两个宿主中接通。

## Agent 侧消费规则

建议 consumer 按字段分层消费：

1. Runtime 控制只读取控制字段：`status`、`task_id`、`pending_reason`、`check_after`、`return_code`
2. Prompt / history 压缩只读取渲染字段：`title`、`summary`
3. Full / 不压缩展示读取 `cmd_name` / `cmd_args` 和主返回体 `output` 或 `detail`
4. 结构化业务数据只读取具体工具定义的 object / array `detail` schema
5. 普通 bash 文本输出只读取 `output`

不要依赖以下模式：

- 从 `summary` 里 parse 状态、task id、退出码
- 看到 stdout 是 JSON 就推断它是 AgentToolResult
- 把 `output` 当 JSON 解析
- 把 object / array `detail` 当终端输出文本
- 把同一份主结果同时当作 `output` 和 `detail` 消费

## 文档边界

本文主要定义 `AgentToolResult` 协议本身，并补充当前 `StepRecord` prompt 渲染基线。

不覆盖以下内容：

- 每个具体工具的 `detail` 业务字段或文本语义定义
- TaskManager 的完整任务模型
- WorkLog 的存储 schema
- WorkLog 的持久化压缩策略
- 审批流 / 安装流的上层编排策略
