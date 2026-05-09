# AgentToolResult 协议说明

`AgentToolResult` 是 OpenDAN AgentTool 的统一执行结果协议。

它的定位是：**带 Runtime 控制语义的工具执行结果协议**。

它不是一个通用的 tool-to-tool 业务数据交换协议。工具之间如果需要交换稳定的业务数据，应定义各自的 typed schema、artifact reference 或专用 API；不要从 `title` / `summary` / `output` 里解析业务语义。

## 设计目标

### 1. 保留关键控制语义

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

### 2. 保留命令原始表达

`cmd_name` / `cmd_args` 表示这次工具调用的命令形态。

它们不是摘要字段，也不应该被 `title` / `summary` 替代。对写入类操作，如果完整写入意图本身就是命令的一部分，可以放在 `cmd_args` 中；Full / 不压缩展示时应能看到 `cmd_name + cmd_args`。

注意：

- `cmd_name` / `cmd_args` 在协议语义上是原始字段，工具开发者填进去是什么就是什么
- 渲染层不应对 `cmd_name` / `cmd_args` 做摘要或改写
- 在 `Min` / `Medium` 等压缩档下，渲染层可以选择不展示 `cmd_name` / `cmd_args`
- 当压缩档不展示原始命令字段时，`title` / `summary` 应包含必要的命令信息
- prompt 渲染器仍可以因为全局 token budget 对最终文本做硬裁剪
- `cmd_args` 是参数文本，不是 JSON 数组

### 3. 明确完整返回体：`output` 或 `detail`

同一份主结果不应同时出现在 `output` 和 `detail` 中。默认情况下，一个结果只填其中一个字段作为主返回体。

允许一个工具同时填写 `output` 和 `detail`，但前提是二者承载不同信息，并且工具文档必须明确说明各自用途。例如：`detail` 放结构化数据，`output` 放可在 bash 终端复现的人读文本。

`output` 表示：

> 这个命令如果在 bash 中执行，用户会在终端里完整看到的文本输出。

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

### 4. 明确渲染压缩字段：`title` / `summary`

和 prompt 渲染有关的字段只有：

- `title`
- `summary`

`title` 是对 `cmd_name` / `cmd_args` 和结果的一行压缩。

`summary` 是对 `cmd_name` / `cmd_args` 和结果的多行压缩。

二者都是给人和 LLM 读的展示字段，不是 Runtime 控制字段，也不是机器可读业务字段。

推荐规则：

- `title` 应该短、一行、稳定，例如 `cargo test => failed (exit=101)`
- `summary` 应该能独立读懂，包含关键结论和必要上下文
- `summary` 可以重复 `title` 中的信息，不需要为了避免重叠而牺牲可读性
- consumer 不应从 `title` / `summary` 中 parse 控制语义
- `title` / `summary` 可以包含人类可读的状态、退出码、task id 等冗余描述，但 Runtime 判断状态时必须读取 `status` / `return_code` / `task_id` 等控制字段

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
  "detail": {},

  "return_code": 0,

  "task_id": "optional",
  "partial_output": "optional pending progress text",
  "pending_reason": "long_running|user_approval|wait_for_install",
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
- 普通 bash 输出即使碰巧长得像 JSON，也不能仅凭内容猜成 AgentToolResult
- `exec_bash` 只有在明确命中内置 AgentTool 命令，且 stdout JSON 带有合法协议标志时，才应把 stdout 解析为 AgentToolResult

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

## 渲染规则

协议只定义单个 `AgentToolResult` 在不同展示级别下应使用哪些字段。

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

如果 `exec_bash` 执行的是内置 AgentTool 命令，并且 stdout 明确是合法 AgentToolResult，`exec_bash` 可以把该结果转发为结构化工具结果。

普通 bash 的 stdout 即使碰巧是 JSON，也不能被隐式当成 `detail` 或 AgentToolResult。

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

本文只定义 `AgentToolResult` 协议本身。

不覆盖以下内容：

- 每个具体工具的 `detail` 业务字段或文本语义定义
- TaskManager 的完整任务模型
- WorkLog 的存储 schema
- 审批流 / 安装流的上层编排策略
