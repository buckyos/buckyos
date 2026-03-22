# AgentToolResult 协议说明

`AgentToolResult` 是 OpenDAN AgentTool 的统一结果协议。

它同时服务三类场景：

1. Runtime 内部的工具结果传递
2. `agent_tool` CLI 的 stdout JSON 输出
3. `exec_bash` 对内置工具结果与普通 bash 结果的统一封装

## 设计目标

### 1. 统一三态

所有工具结果都收敛到三种状态：

- `success`
- `error`
- `pending`

Agent Loop、WorkLog、`check_task`、审批等待、长任务等待都基于这三个状态工作。

### 2. 区分结构化结果与 bash 文本输出

`detail` 和 `output` 的职责不同：

- `detail`：给内置工具和 Runtime 使用的结构化数据
- `output`：给 bash 语义使用的主输出文本

这两个字段不能混用。

尤其是：

- 对普通 bash 命令，主结果应该看 `output`
- 对内置工具，结构化数据应该看 `detail`

现在可以通过 `output` 和 `detail` 的组合，明确分辨结果来源：

- `detail` 明显承载业务结构化数据：通常是内置工具结果
- `output` 是主内容，`detail` 只带少量元信息：通常是普通 bash 结果

### 3. 避免隐式 JSON 误判

`exec_bash` 不应因为某个普通 bash 命令碰巧输出了一段 JSON，就把它自动当成内置工具结果。

当前实现规则是：

- 只有明确命中内置 AgentTool 命令时，`exec_bash` 才尝试把 stdout 解析为 `AgentToolResult`
- 普通 bash 命令始终按文本输出处理，落到 `output`

这避免了 `echo '{"a":1}'` 之类输出触发隐式协议转换。

## JSON 协议

序列化到 CLI / bash stdout 后，`AgentToolResult` 的协议如下：

```json
{
  "status": "success|error|pending",
  "summary": "human readable summary",
  "output": "primary text output for bash",
  "detail": {},

  "return_code": 0,
  "cmd_name": "",
  "cmd_args": "",

  "task_id": "optional",
  "partial_output": "",
  "pending_reason": "long_running|user_approval|wait_for_install",
  "check_after": 5
}
```

说明：

- `output`、`return_code`、`cmd_name`、`cmd_args`、`task_id`、`partial_output`、`pending_reason`、`check_after` 都是可选字段
- Rust 内部结构体字段名仍然是 `details`，但对外序列化字段名固定为 `detail`
- 历史值 `external_callback` 仍作为兼容别名接受，但新协议统一写作 `wait_for_install`

## 字段定义

### `status`

结果状态，取值固定为：

- `success`
- `error`
- `pending`

### `summary`

给人读的短摘要。

要求：

- 所有状态都应该尽量提供
- 用于 WorkLog、Prompt 压缩、任务列表展示
- 不要求可机读，但要稳定、简短、可理解

### `output`

面向 bash 语义的主文本输出。

规则：

- 普通 bash 命令的主要结果放这里
- `exec_bash` 默认回退逻辑会把 tmux 视角下的混合输出放这里
- 内置工具如果主要价值是结构化结果，可以省略该字段

`output` 不要求是 JSON，也不要求可反序列化。

### `detail`

结构化明细，只给内置工具 / Runtime / Agent Loop 使用。

规则：

- 内置工具的业务结构化结果放这里
- 普通 bash 命令不要依赖 `detail` 传主结果
- `exec_bash` 对普通命令仍可在 `detail` 中附带少量元信息，例如 `pwd`、`cwd`、`session_id`、`line_results`

不要把 `detail` 当成 bash 文本输出容器。

### `return_code`

命令退出码。

规则：

- 有 shell / bash 退出码语义时填写
- 内置工具没有明确退出码时可以省略

### `cmd_name` / `cmd_args`

命令名和参数文本。

规则：

- 主要用于 `exec_bash` 结果和调试
- 内置工具默认可以省略
- `cmd_args` 是参数文本，不是 JSON 数组

### `task_id`

当结果为 `pending` 时，用于后续 `check_task` 轮询。

### `partial_output`

`pending` 时的阶段性输出。

规则：

- 用于长任务在未完成时向 Agent 暴露当前进展
- 不要求完整
- 不替代最终 `output`

### `pending_reason`

当前仅使用以下值：

- `long_running`
- `user_approval`
- `wait_for_install`

兼容说明：

- 历史值 `external_callback` 会被兼容解析为 `wait_for_install`

### `check_after`

建议 Agent 多少秒后再次轮询。

仅在 `pending` 时有意义。

## 使用约定

### 内置工具

内置工具默认直接返回 `AgentToolResult`。

推荐模式：

- `summary`：给人读的摘要
- `detail`：完整结构化结果
- `output`：只有在确实需要暴露 bash 文本输出时才填写

示例：

```json
{
  "status": "success",
  "summary": "read 128 bytes",
  "detail": {
    "tool": "read_file",
    "path": "/tmp/a.txt",
    "content": "hello"
  }
}
```

### 普通 bash / exec_bash 默认回退

当命令不是内置工具，或 stdout 不应按内置工具协议解释时，`exec_bash` 生成统一结果：

- `output`：tmux 视角下的混合输出
- `return_code`：shell exit code
- `detail`：附带运行元信息

示例：

```json
{
  "status": "error",
  "summary": "FAILED (exit=2)",
  "output": "ls: /missing: No such file or directory",
  "detail": {
    "pwd": "/workspace",
    "session_id": "s1",
    "engine": "tmux",
    "line_results": []
  },
  "return_code": 2,
  "cmd_name": "ls",
  "cmd_args": "/missing"
}
```

### Pending 结果

示例：

```json
{
  "status": "pending",
  "summary": "PENDING (long_running, check_after=5s)",
  "detail": {
    "pwd": "/workspace",
    "session_id": "s1"
  },
  "task_id": "12345",
  "partial_output": "building target...",
  "pending_reason": "long_running",
  "check_after": 5
}
```

## Agent 侧消费规则

建议统一按以下顺序处理：

1. 先看 `status`
2. 再看 `summary`
3. 如果是 bash 语义结果，优先看 `output`
4. 如果是内置工具结果，读取 `detail`
5. 如果是 `pending`，保存 `task_id`，按 `check_after` 轮询

不要依赖下面这些不稳定模式：

- 看到 stdout 是 JSON 就推断它是内置工具结果
- 从 `detail` 中提取 bash 主输出
- 仅通过 `return_code` 推断最终状态

## 与 Rust 内部实现的关系

当前 Rust 内部仍继续使用 `AgentToolResult` 结构体。

需要注意的兼容点：

- 结构体字段名是 `details` 
- 对外 JSON 字段名是 `detail`
- `render_prompt()` 优先展示 `output`，其次才展示 `stdout`
- `CliResultEnvelope` 最终也会转换回 `AgentToolResult` 再输出

这意味着：

- Runtime 内部仍可以继续用结构化字段工作
- CLI / bash / WorkLog 看到的是统一协议

## 文档边界

本文只定义 `AgentToolResult` 协议本身。

不覆盖以下内容：

- 每个具体工具的业务字段定义
- TaskManager 的完整任务模型
- WorkLog 压缩策略
- 审批流 / 安装流的上层编排策略
