# Built-in Agent Tool 手册

本文给出 OpenDAN 当前所有 builtin agent tool 的使用手册。每个工具按统一格式列出：

1. **Prompt**：注入到 LLM `ToolSpec.description` 的提示词，以及 schema / usage（function calling 见到的内容）。
2. **Bash 支持**：`CallingConventions` 决定的入口（`BASH` / `ACTION` / `LLM`）。
3. **CLI 命令解释 + 最常使用例子**：在 Session 的 bash 环境里如何调用。
4. **可能的输出结果**：`AgentToolResult` 协议示例（参见 [agent_tool_result_protocol.md](agent_tool_result_protocol.md)）。

> 输出协议固定字段：`agent_tool_protocol / status / cmd_name / cmd_args / title / summary / detail|output`，详细规则见协议文档。下文示例只展示 `detail` 关键字段，渲染压缩字段 `title` / `summary` 仅给出代表性写法。

---

## 环境变量契约

所有 builtin agent tool（无论是 Session Exec Bin 里的 CLI 软链接，还是 `agent_tool <tool>` 全名调用）共享同一套**最小环境变量契约**。完整设计与推导规则见 [OpenDAN AgentTool 开发指南.md](OpenDAN%20AgentTool%20%E5%BC%80%E5%8F%91%E6%8C%87%E5%8D%97.md) 第 6 节，这里给出工具使用者视角的速查。

### 生产契约（稳定，仅 4 个）

| env | 必需 | 用途 | 缺失处理 |
|-----|------|------|----------|
| `OPENDAN_AGENT_ROOT` | 是 | 当前 Agent RootFS / state root（取代旧 `OPENDAN_AGENT_ENV`） | 生产报错；CLI dev 回退到当前 `cwd` |
| `OPENDAN_SESSION_ID` | 是 | 当前 session id（不新增 `OPENDAN_AGENT_SESSIONID`） | 生产报错；CLI dev 回退到 `cli-session` |
| `BUCKYOS_APPCLIENT_SESSION_TOKEN` | 是 | AgentTool 作为 AppClient 访问 BuckyOS runtime / kRPC 的 token | 需要 RPC 的工具报错；纯文件 dev 工具可不要求 |
| `OPENDAN_TRACE_ID` | 否 | trace id / 日志关联 | 缺失时生成默认 trace |

工具进程启动时由这 4 个变量构造统一的 `RuntimeContext`（`agent_tool::runtime_context`）。其余上下文一律**从 RuntimeContext 推导，不再通过 env 注入**：

| 上下文 | 推导来源 |
|--------|----------|
| `agent_id` / app id | Agent RootFS identity metadata（`<agent_root>/.meta/agent_identity.json` → `agent.toml [identity]` → 规范路径），缺失时走 BuckyOS runtime / `app_instance_config` |
| owner user id | 同上 |
| session root | `<agent_root>/sessions/<session_id>/` |
| behavior / step / wakeup | session state / last step record |
| tool bin / session tool path | BuckyOS tools 路径规则 + `agent_id + session_id` |
| memory / notebook / todo / workspace | Agent RootFS 固定目录规则（见 [Agent RootFS.md](../opendan/Agent%20RootFS.md)） |

### Dev-only override（仅本地调试，不进生产契约）

| env | 用途 |
|-----|------|
| `AGENT_MEMORY_ROOT` | 覆盖 memory root（否则用 `<agent_root>/memory/`） |
| `AGENT_NOTEBOOK_ROOT` | 覆盖 notebook root |
| `OPENDAN_WORKFLOW_URL` / `WORKFLOW_SERVICE_URL` | 直连 workflow service |
| `OPENDAN_TASK_MANAGER_URL` / `TASK_MANAGER_URL` | 直连 task-manager |
| `OPENDAN_SESSION_TOKEN` / `SESSION_TOKEN` | dev 直连 RPC token |

这些只在 dev 路径生效（cli_dev 的 `allow_dev_overrides()` / dcrontab 的 `allow_dev_env_overrides()` gate）；生产工具通过 Agent RootFS 路径和 BuckyOS runtime client 获取资源。

### beta2.2 已移除的历史变量

beta2.2 是 breaking-change 版本，下列旧上下文变量**已整体移除**，工具不再读取，命令行也不要再通过 `--agent-env` / `--session-id` / `--agent-id` 重复传：

`OPENDAN_AGENT_ENV`、`OPENDAN_AGENT_ID`、`OPENDAN_AGENT_OWNER`、`OPENDAN_OWNER_USER_ID`、`BUCKYOS_OWNER_USER_ID`、`OPENDAN_AGENT_TOOL`、`OPENDAN_AGENT_BIN`、`OPENDAN_SESSION_TOOL_PATH`、`OPENDAN_BEHAVIOR`、`OPENDAN_STEP_IDX`、`OPENDAN_WAKEUP_ID`。

> 注意：`OPENDAN_AGENT_ROOT=/tmp/foo` 这类任意 override 路径在没有 identity metadata 时 `identity = None`，工具不会静默向上扫目录或检查 `todo.db` / `worklog.db` 来猜身份。

---

## 0. 工具一览

| Tool | 入口 | 主要用途 | 代码位置 |
|---|---|---|---|
| [`exec_bash`](#1-exec_bash) | bash / action / llm_tool_call | 执行 bash 命令 | `src/llm_bash.rs` |
| [`read`](#2-read) | bash / action / llm_tool_call | 按 URI 读取（v2 Action） | `src/read_tool.rs` |
| [`write_file`](#3-write_file) | action | 写文件 | `src/file_tools.rs` |
| [`edit_file`](#4-edit_file) | action | 基于唯一字符串替换文件 | `src/file_tools.rs` |
| [`read_file`](#5-read_file-legacy) | bash / llm_tool_call | legacy 文本读取，支持纯文本 stdout | `src/file_tools.rs` |
| [`Glob`](#6-glob) | bash / llm_tool_call | 文件名 glob 匹配 | `src/glob_tool.rs` |
| [`Grep`](#7-grep) | bash / llm_tool_call | ripgrep 内容搜索 | `src/grep_tool.rs` |
| [`todo`](#8-todo) | bash / action / llm_tool_call | session 级 PDCA todo 管理 | `src/todo_tools.rs` |
| [`delegateTask`](#9-delegatetask) | bash / action / llm_tool_call | 委托系统级 Task | `src/todo_tools.rs` |
| [`get_session`](#10-get_session) | bash | 读 session 快照 | `src/lib.rs` |
| [`create_workspace`](#11-create_workspace) | bash | 创建 workspace 并绑定 | `src/lib.rs` |
| [`bind_workspace`](#12-bind_workspace) | bash | 绑定 workspace 到 session | `src/lib.rs` |
| [`bind_external_workspace`](#13-bind_external_workspace) | bash | 注册外部 workspace | `src/lib.rs` |
| [`list_external_workspaces`](#14-list_external_workspaces) | bash | 列出外部 workspace | `src/lib.rs` |
| [`worklog_manage`](#15-worklog_manage) | bash | append-only 审计日志（不入 prompt） | `src/lib.rs` |
| [`subscribe_event`](#16-subscribe_event) | llm_tool_call | 订阅 KEvent | `opendan/src/buildin_tool.rs` |
| [`unsubscribe_event`](#17-unsubscribe_event) | llm_tool_call | 取消订阅 KEvent | `opendan/src/buildin_tool.rs` |
| [`check_task`](#18-check_task) | CLI 伪工具 | 轮询 pending task | `agent_tool_cli_dev/src/lib.rs` |
| [`cancel_task`](#19-cancel_task) | CLI 伪工具 | 取消 pending task | `agent_tool_cli_dev/src/lib.rs` |
| [`finish_task`](#20-finish_task) | CLI 伪工具 | 结束 task（完成/失败） | `agent_tool_cli_dev/src/lib.rs` |
| [`agent-memory`](#21-agent-memory) | CLI | 长期记忆 KV 存取 | `agent_tool_cli_dev/src/lib.rs` |
| [`agent-notebook`](#22-agent-notebook) | CLI | Agent notebook 读写 | `agent_tool_cli_dev/src/lib.rs` |

CLI 伪工具不走 `AgentTool` trait 注册，只在 `agent_tool` 二进制内分发。`agent-memory` / `agent-notebook` 是 CLI-only 子命令，没有对应的 `TypedTool` 注册。

---

## 1. `exec_bash`

### Prompt

- `description`: `Run bash command at target node (bash -c $command)`
- `args_schema`:
  ```json
  {
    "type": "object",
    "properties": {
      "command": { "type": "string", "description": "shell command to execute" },
      "target":  { "type": "string", "description": "MUST select known node. Blank = current environment." }
    },
    "required": ["command"]
  }
  ```
- `usage`: `exec_bash command='<shell>' [target=local]`

可选字段（顶层 args）：`cwd`（必须落在 workspace 内）、`timeout_ms`、`env`（受 `allow_env` 约束）。

### Bash 支持

`CallingConventions::ALL` —— bash / action / llm_tool_call 都支持。最常见入口是 LLM 直接调度，runtime 把它转成 tmux + bash 执行。

### CLI 命令解释 + 常用例子

`exec_bash` 自身不直接出现在 Session Exec Bin 里——bash 本身就是底座。常见入口是 Behavior 的 `<exec_bash>` action：

```xml
<exec_bash command="ls -la"/>
<exec_bash command="cargo build" cwd="/workspace/proj" timeout_ms="120000"/>
<exec_bash command="read_file src/foo.rs 1-50"/>
```

JSON 调度形态：

```json
{ "command": "ls -la" }
{ "command": "cargo build", "cwd": "/workspace/proj", "timeout_ms": 120000 }
{ "command": "./run.sh",   "env": { "DEBUG": "1" } }
```

### 输出示例

`exec_bash` 不自己产生业务结果，结果形态取决于 **实际跑的命令**：

#### A) 普通 bash 命令

`exec_bash` 把 stdout / stderr / exit_code 包成自己的 envelope，`cmd_name="exec_bash"`：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "exec_bash",
  "cmd_args": "ls -la",
  "title": "exec_bash ls -la => exit=0",
  "summary": "exit=0 in 12ms",
  "return_code": 0,
  "output": "$ ls -la\ntotal 8\n...",
  "detail": {
    "command": "ls -la",
    "target": "local",
    "cwd": "/workspace/proj",
    "exit_code": 0,
    "stdout": "total 8\n...",
    "stderr": "",
    "output": "...",
    "output_truncated": false,
    "duration_ms": 12,
    "engine": "local"
  }
}
```

失败时 `status=error` / `return_code` 非零；超时 / `max_output_bytes` 命中时 `output_truncated=true`。

#### B) 内层输出 AgentToolResult envelope

当内层命令自己在 stdout 输出合法 `agent_tool_protocol` envelope 时（典型来源是 Session Exec Bin 中的 AgentTool 链接，如 `read_file` / `todo` / `Glob` / `Grep` …），按协议契约（见 [agent_tool_result_protocol.md](agent_tool_result_protocol.md) `exec_bash 约定` 节）：

- `exec_bash` 应把内层 stdout 上的合法 envelope **透传**为本次 `AgentToolResult`
- 此时 `cmd_name` / `cmd_args` / `detail` 由内层工具决定，不再是 `exec_bash`

例如 `<exec_bash command="read_file src/foo.rs 1-50"/>` 预期返回：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "read_file",
  "cmd_args": "src/foo.rs 1-50",
  "title": "read_file src/foo.rs range=1-50 => success",
  "summary": "succeeded, read 1234 bytes across 50 lines at 1-50",
  "detail": { "content": "...", "matched": true, "line_range": "1-50", "bytes": 1234, "...": "..." }
}
```

> 普通 bash 的 stdout 即使碰巧长得像 JSON，也不会被当作 AgentToolResult；透传依赖 stdout 是带合法 `agent_tool_protocol` 的 envelope。

---

## 2. `read`

v2 Behavior 协议的 `<read uri="..."/>` 规范读取工具。读取目标必定是结构化数据，返回内容必定是 LLM 可处理的文本。当前仅实现 `file://` scheme，bare path 等价于 `file://`，其他 scheme 返回 `InvalidArgs`。

### Prompt

- `description`: `Read structured data by uri and return LLM-readable text.`
- `args_schema`:
  ```json
  {
    "type": "object",
    "properties": {
      "uri":    { "type": "string",  "description": "Structured data target to read. Bare paths default to file reads." },
      "offset": { "type": "integer", "minimum": 0, "description": "Line offset to start reading at; defaults to 0." },
      "limit":  { "type": "integer", "minimum": 1, "description": "Max lines to read." }
    },
    "required": ["uri"]
  }
  ```
- `usage`: `read uri="<path-or-uri>" [offset=<line>] [limit=<lines>]`

`offset` 是 0-based 行位置，`limit` 是行数。原 token limit 不再是本函数参数，而是 session 属性 `read_token_limit`，默认 20K；返回内容超过该预算时会设置 `detail.token_truncated = true`。同 session + 同窗口的重复 read，命中"未变化"短路时 `detail.unchanged = true` 且 `content` 替换为 `和上一次read相比没有变化`，状态文件落在系统临时目录。

### Bash 支持

`CallingConventions::ALL`。

### CLI 命令解释 + 常用例子

```bash
# bare path
read src/foo.rs

# 显式 URI
read uri=file:///workspace/demo.txt

# 分段读：先头 200 行
read src/big.log offset=0 limit=200

# 翻下一页
read src/big.log offset=200 limit=200
```

XML action 形态：

```xml
<read uri="src/foo.rs" offset="0" limit="200"/>
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "read",
  "cmd_args": "file:///workspace/demo.txt offset=0 limit=200",
  "title": "read file:///workspace/demo.txt => read 50 lines (EOF)",
  "summary": "read 50 lines at offset 0 of 50 (EOF)",
  "detail": {
    "uri": "file:///workspace/demo.txt",
    "scheme": "file",
    "path": "/workspace/demo.txt",
    "content": "...",
    "offset": 0,
    "limit": 200,
    "lines_read": 50,
    "total_lines": 50,
    "eof": true,
    "unchanged": false,
    "token_limit": 20480,
    "token_truncated": false
  }
}
```

未变化时：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "read",
  "title": "read src/foo.rs => read 0 lines",
  "output": "和上一次read相比没有变化",
  "detail": { "content": "和上一次read相比没有变化", "lines_read": 0, "unchanged": true, "eof": false, "offset": 0, "limit": 200, "total_lines": 50 }
}
```

---

## 3. `write_file`

### Prompt

- `description`: `Write file.`
- `args_schema`（由 `WriteFileArgs` 派生）：
  ```json
  {
    "type": "object",
    "properties": {
      "path":    { "type": "string" },
      "content": { "type": "string" },
      "mode":    { "type": "string", "enum": ["new", "append", "write"] }
    },
    "required": ["path", "content"]
  }
  ```
- `usage`: `write_file <path> [--mode new|append|write] (--content <text> | --content-stdin)`

`mode` 默认 `write`（覆盖）。`new` 仅当文件不存在；`append` 追加。

### Bash 支持

`CallingConventions::ACTION` —— 仅作为 v2 Action 入口，不暴露成 bash 别名（写入意图必须显式经过 Action）。

### CLI 命令解释 + 常用例子

XML action 形态：

```xml
<write_file path="src/foo.rs" mode="write">
  <content>fn main() { println!("hello"); }</content>
</write_file>
```

CLI 形态（dispatcher 内部）：

```bash
write_file src/foo.rs --mode write --content "fn main() {}"
# 从 stdin 读取内容：
cat new.txt | write_file src/foo.rs --mode write --content-stdin
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "write_file",
  "cmd_args": "src/foo.rs --mode=write",
  "title": "write_file src/foo.rs mode=write => success",
  "summary": "wrote src/foo.rs, wrote 42 bytes across 3 lines",
  "detail": {
    "created": true,
    "changed": true,
    "bytes_written": 42,
    "line_count": 3
  }
}
```

`detail` 不回显输入 `content`，完整写入意图保留在 `cmd_args`。

---

## 4. `edit_file`

唯一字符串替换：`old_string` 必须在文件中只出现一次，然后替换成不同的 `new_string`。

### Prompt

- `description`: `Edit file.`
- `args_schema`（由 `EditFileArgs` 派生）：
  ```json
  {
    "type": "object",
    "properties": {
      "path":        { "type": "string" },
      "old_string":  { "type": "string" },
      "new_string":  { "type": "string" }
    },
    "required": ["path", "old_string", "new_string"]
  }
  ```
- `usage`: `edit_file <path> --old-string <text> (--new-string <text> | --new-string-stdin)`

### Bash 支持

`CallingConventions::ACTION`。

### CLI 命令解释 + 常用例子

XML action 形态：

```xml
<edit_file path="src/foo.rs">
  <old_string><![CDATA[println!("hello");]]></old_string>
  <new_string><![CDATA[println!("hi");]]></new_string>
</edit_file>
```

CLI 形态：

```bash
edit_file src/foo.rs \
  --old-string 'println!("hello");' \
  --new-string 'println!("hi");'
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "edit_file",
  "cmd_args": "edit_file src/foo.rs old_string=\"println!(\\\"hello\\\");\"",
  "title": "edit_file src/foo.rs => success (line 7)",
  "summary": "edited src/foo.rs with replace at line 7\n```diff\n-    println!(\"hello\");\n+    println!(\"hi\");\n```",
  "detail": {
    "matched": true,
    "changed": true,
    "mode": "replace",
    "line": 7,
    "diff": "-    println!(\"hello\");\n+    println!(\"hi\");\n",
    "diff_truncated": false
  }
}
```

`matched=false` 时 `title` 为 `... => anchor not found`，`changed=false`，`detail.line=null`。

---

## 5. `read_file`（legacy）

行号导向的 legacy 文本读取工具，仍作为 Session Exec Bin 链接保留，并提供一个 CLI 纯文本短路。

### Prompt

- `description`: `Read file.`
- `args_schema`（由 `ReadFileArgs` 派生）：包含 `path`、`first_chunk?`、`range?`（1-based，支持负数 / `$` / `+N`）。
- `usage`:
  ```text
  read_file <path> [range] [first_chunk]
    range: 1-based; supports negative/$/+N, and applies within first_chunk slice
  ```

### Bash 支持

`CallingConventions::from_legacy(true, false, true)` —— `BASH | LLM`。
`cli_plain_text_stdout = true`：当无 agent 环境且 stdout 不是 TTY 时，CLI 改为输出文件内容本身（接近 `cat`）。

### CLI 命令解释 + 常用例子

```bash
# 整文件
read_file src/foo.rs

# 1-based 范围
read_file src/foo.rs 1-50

# 从锚点开始的子片段
read_file src/foo.rs '1-50' 'fn main'

# 流式管道（非 TTY 自动切换到纯文本）
read_file src/big.log | grep ERROR
```

### 输出示例

JSON 模式：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "read_file",
  "cmd_args": "src/foo.rs 1-50",
  "title": "read_file src/foo.rs range=1-50 => success",
  "summary": "succeeded, read 1234 bytes across 50 lines at 1-50\n```content\n...预览...\n```",
  "detail": {
    "content": "...实际读到的内容...",
    "matched": true,
    "line_range": "1-50",
    "bytes": 1234,
    "preview": "...",
    "start_line": 1,
    "end_line": 50,
    "line_count": 50
  }
}
```

纯文本模式（非 TTY 旁路）：stdout 直接是文件内容，无 JSON 包装。

---

## 6. `Glob`

ripgrep / glob 风格的文件名匹配。

### Prompt

`description`:
```text
- Fast file pattern matching tool that works with any codebase size
- Supports glob patterns like "**/*.js" or "src/**/*.ts"
- Returns matching file paths sorted by modification time
- Use this tool when you need to find files by name patterns
- When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the Agent tool instead
```

- `args_schema`:
  ```json
  {
    "type": "object",
    "properties": {
      "pattern": { "type": "string" },
      "path":    { "type": "string" }
    },
    "required": ["pattern"]
  }
  ```
- `usage`: `Glob <pattern> [path]` —— `pattern: glob such as "**/*.rs" or "src/**/*.ts"`

结果默认按 mtime 倒序，最多 100 条（`truncated=true` 表示截断）。

### Bash 支持

`BASH | LLM`。

### CLI 命令解释 + 常用例子

```bash
# 在 workspace 根下找 Rust 源码
Glob '**/*.rs'

# 限定子目录
Glob 'src/**/*.ts' path=frontend
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "Glob",
  "cmd_args": "pattern='**/*.rs'",
  "title": "Glob => found 42 files",
  "summary": "found 42 files in 7ms\n```files\nsrc/lib.rs\nsrc/main.rs\n...\n```",
  "detail": {
    "durationMs": 7,
    "numFiles": 42,
    "filenames": ["src/lib.rs", "src/main.rs", "..."],
    "truncated": false
  }
}
```

---

## 7. `Grep`

基于 `rg` 的内容搜索。

### Prompt

`description`:
```text
A powerful search tool built on ripgrep

Usage:
- Search file contents with regular expressions.
- Supports full ripgrep regex syntax, glob filtering, file type filtering, line numbers, context lines, pagination, and multiline matching.
- Output modes: "content" shows matching lines, "files_with_matches" shows only file paths (default), "count" shows match counts.
- Use this tool for code/content search instead of invoking grep or rg manually.
```

`args_schema`（关键字段）：`pattern` (required)、`path`、`glob`、`output_mode` (`content|files_with_matches|count`，默认 `files_with_matches`)、`-B/-A/-C`、`context`、`-n`、`-i`、`type`、`head_limit`、`offset`、`multiline`。

`usage`: `Grep <pattern> [path]` —— `pattern: ripgrep regex; optional key=value args include glob, type, output_mode, head_limit, offset`

### Bash 支持

`BASH | LLM`。

### CLI 命令解释 + 常用例子

```bash
# 找文件（默认）
Grep 'fn main'

# 显示匹配行 + 上下文
Grep 'TODO' output_mode=content -C=2

# 只在 .rs 中找
Grep 'unsafe' type=rust output_mode=content

# 分页
Grep 'panic!' output_mode=content head_limit=50 offset=50
```

### 输出示例

`files_with_matches`：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "Grep",
  "cmd_args": "pattern='fn main'",
  "title": "Grep => 3 files",
  "summary": "found 3 files in 9ms",
  "detail": {
    "mode": "files_with_matches",
    "durationMs": 9,
    "numFiles": 3,
    "filenames": ["src/bin/a.rs", "src/bin/b.rs", "tests/it.rs"]
  }
}
```

`content` 模式额外带 `content / numLines / appliedLimit / appliedOffset`。`count` 模式带 `numMatches`。

---

## 8. `todo`

Session 级 PDCA 工具：plan 阶段加 todo，do 阶段消费 current，完成时回写 summary / report。

### Prompt

`description`:
```text
Session 级 Todo 工具：plan 写入下一步执行分支、DO 拉取 current todo、
分支结束写 summary/report 回流到主干。
```

`usage`:
```text
todo add "<title>" --content <text> [--skill <name>]... [--context <text>]
todo current
todo list
todo done "<summary>" [--report <text> | --report-file <path>]
todo failed "<summary>" [--report <text> | --report-file <path>]
todo finish --status <completed|failed|timeout|blocked> "<summary>"
   [--report <text> | --report-file <path>] [--id <todoID>]
todo show [<todoID>]
todo clean
```

`args_schema` 是一个 `TodoArgs` tagged enum（每个 subcommand 一个 variant），LLM tool_call 路径直接发对应的 JSON 即可。

数据落地：

- session 级 `todos.json` ← `<agent_rootfs>/sessions/<session_id>/todos.json`
- workspace 级 `tasks.json` ← `<workspace>/.agent/tasks.json`

### Bash 支持

`CallingConventions::ALL`。Session Exec Bin 通过软链暴露 `todo` 命令。

### CLI 命令解释 + 常用例子

```bash
todo add "实现 read_tool 的 unchanged 短路" \
  --content '复用 hash 检查' --skill read --skill cache

todo current                  # 当前 running / 第一个 pending
todo list                     # 全部 todo + delegate task
todo show                     # 当前 todo 详情
todo show T001                # 指定 id

# 完成（report 可走文件以便长内容）
todo done "短路实现完成" --report-file ./reports/read_unchanged.md
todo failed "命中边界条件" --report '...'

# 通用 finish
todo finish --status timeout "等待超时" --id T002

# 清理 pending（已 terminal 的 todo 不动）
todo clean
```

### 输出示例

`todo add` 结果：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "todo",
  "cmd_args": "todo add",
  "title": "todo add => ok",
  "summary": "added T003 (current)",
  "detail": {
    "Add": {
      "todo_id": "T003",
      "is_current": true,
      "order_index": 3,
      "title": "实现 read_tool 的 unchanged 短路"
    }
  }
}
```

`todo current` 无 current：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "todo",
  "cmd_args": "todo current",
  "title": "todo current => ok",
  "summary": "no current todo",
  "detail": { "Current": { "todo": null } }
}
```

`todo done`：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "todo",
  "summary": "T003 -> Completed",
  "detail": { "Done": { "todo_id": "T003", "status": "completed" } }
}
```

---

## 9. `delegateTask`

把一段自然语言任务委托给 task_management，登记到当前 workspace 的 `.agent/tasks.json`。

### Prompt

`description`:
```text
委托一个系统级 Task（task_management 创建 / 路由），并把 taskID 登记到
当前 workspace 的 .agent/tasks.json。
```

`usage`: `delegateTask "<task>" [--to <target>] [--context <text>]`

### Bash 支持

`CallingConventions::ALL`。

### CLI 命令解释 + 常用例子

```bash
delegateTask "替我把 build 产物上传到 OSS" --to deploy-agent --context "release v1.2"
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "delegateTask",
  "cmd_args": "delegateTask",
  "title": "delegateTask => success",
  "summary": "delegated tsk_ab12cd34ef56 (pending)",
  "detail": {
    "task_id": "tsk_ab12cd34ef56",
    "status": "pending",
    "purpose": "替我把 build 产物上传到 OSS"
  }
}
```

后续通过 `check_task <task_id>` 拉最终结果。

---

## 10. `get_session`

读 session 快照（runtime 在每轮 LLM 之前会调一次）。

### Prompt

- `description`: `Read current session state and status. Used by runtime before each LLM round.`
- `args_schema`:
  ```json
  { "type": "object", "properties": { "session_id": { "type": "string" } } }
  ```
- `usage`: `get_session [session_id]`

### Bash 支持

`CallingConventions::BASH`。

### CLI 命令解释 + 常用例子

```bash
get_session                  # 走 ctx 里的 session
get_session sid-abc-123      # 指定 session
get_session session_id=sid-abc-123
get_session --session-id sid-abc-123   # CLI 长选项
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "get_session",
  "cmd_args": "get_session sid-abc-123",
  "title": "get_session => ok",
  "summary": "ok",
  "detail": {
    "ok": true,
    "session": {
      "session_id": "sid-abc-123",
      "workspace_id": "ws-001",
      "...": "..."
    }
  }
}
```

---

## 11. `create_workspace`

创建本地 workspace、写 `SUMMARY.md`、把当前 session 绑过去。

### Prompt

- `description`: `创建session的wrokspace并设置为session的default workspace`
- `args_schema`:
  ```json
  {
    "type": "object",
    "properties": { "name": {"type":"string"}, "summary": {"type":"string"} },
    "required": ["name", "summary"]
  }
  ```
- `usage`: `create_workspace <name> <summary>`

### Bash 支持

`CallingConventions::BASH`。

### CLI 命令解释 + 常用例子

```bash
create_workspace beta22_release "Track beta2.2 release prep"
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "create_workspace",
  "cmd_args": "create_workspace beta22_release Track beta2.2 release prep",
  "title": "create_workspace beta22_release => created",
  "summary": "ok",
  "detail": {
    "ok": true,
    "workspace": { "id": "ws-7f...", "name": "beta22_release" },
    "binding": { "session_id": "sid-abc", "workspace_id": "ws-7f..." },
    "summary_path": "/workspaces/ws-7f.../SUMMARY.md",
    "session_updated": true
  }
}
```

---

## 12. `bind_workspace`

切换 session 当前 workspace。

### Prompt

- `description`: `设置agent_session的当前workspace`
- `args_schema`:
  ```json
  {
    "type": "object",
    "properties": { "workspace": {"type":"string"} },
    "required": ["workspace"]
  }
  ```
- `usage`: `bind_workspace <workspace_id|workspace_path>`

### Bash 支持

`CallingConventions::BASH`。

### CLI 命令解释 + 常用例子

```bash
bind_workspace ws-7f1a2b3c
bind_workspace /workspaces/beta22_release    # 走 path
bind_workspace workspace_id=ws-7f1a2b3c
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "bind_workspace",
  "cmd_args": "bind_workspace ws-7f1a2b3c",
  "title": "bind_workspace ws-7f1a2b3c => bound",
  "summary": "bound session to workspace ws-7f1a2b3c",
  "detail": {
    "ok": true,
    "binding": { "session_id": "sid-abc", "local_workspace_id": "ws-7f1a2b3c" },
    "session_updated": true
  }
}
```

---

## 13. `bind_external_workspace`

把用户指定的目录注册成 agent 可见的 external workspace。

### Prompt

- `description`: `Bind an external workspace directory so this agent can access it from runtime.`
- `args_schema`:
  ```json
  {
    "type": "object",
    "properties": {
      "name":            { "type": "string" },
      "workspace_path":  { "type": "string" },
      "agent_did":       { "type": "string" }
    },
    "required": ["name", "workspace_path"]
  }
  ```

### Bash 支持

`CallingConventions::BASH`。

### CLI 命令解释 + 常用例子

```bash
bind_external_workspace home_repo /Users/alice/code/repo
bind_external_workspace home_repo /Users/alice/code/repo agent_did=did:opendan:alice
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "bind_external_workspace",
  "cmd_args": "bind_external_workspace home_repo /Users/alice/code/repo",
  "title": "bind_external_workspace home_repo => bound",
  "summary": "bound external workspace home_repo",
  "detail": {
    "ok": true,
    "binding": { "name": "home_repo", "workspace_path": "/Users/alice/code/repo", "agent_did": "did:opendan:alice" }
  }
}
```

---

## 14. `list_external_workspaces`

列出 agent 可见的 external workspace。

### Prompt

- `description`: `List bound external workspaces visible to current agent.`
- `args_schema`:
  ```json
  { "type": "object", "properties": { "agent_did": { "type": "string" } } }
  ```

### Bash 支持

`CallingConventions::BASH`。

### CLI 命令解释 + 常用例子

```bash
list_external_workspaces
list_external_workspaces agent_did=did:opendan:alice
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "list_external_workspaces",
  "title": "list_external_workspaces => 2",
  "summary": "listed 2 external workspace(s)",
  "detail": {
    "ok": true,
    "workspaces": [
      { "name": "home_repo", "workspace_path": "/Users/alice/code/repo" },
      { "name": "scratch",   "workspace_path": "/tmp/scratch" }
    ]
  }
}
```

---

## 15. `worklog_manage`

Append-only 审计日志读写。**不进入 prompt**，仅供调试 / 事后分析。

### Prompt

- `description`: `Append-only audit log of agent runtime events. Used for debugging and post-hoc analysis; does not feed into prompts.`
- `args_schema`: 顶层 `action` enum：`append_worklog | list_worklog | get_worklog`，外加 `record / id / step_id / owner_session_id / workspace_id / type / status / keyword / limit / offset`。

> 历史接口 `append_step_summary / mark_step_committed / list_step / build_prompt_worklog / render_for_prompt` 在 beta2.2 已下线，调用会返回 `unsupported action`。详见 `notepads/worklog简化.md`。

### Bash 支持

`CallingConventions::BASH`。

### CLI 命令解释 + 常用例子

```bash
worklog_manage action=append_worklog \
  record='{"type":"agent.file.write","status":"success","msg":"wrote demo.txt"}'

worklog_manage action=list_worklog type=agent.file.write limit=20

worklog_manage action=get_worklog id=wl_01HXYZ
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "worklog_manage",
  "cmd_args": "worklog_manage action=list_worklog",
  "title": "worklog_manage list_worklog => ok",
  "summary": "list_worklog",
  "detail": {
    "ok": true,
    "records": [ { "id": "wl_01HXYZ", "type": "agent.file.write", "...": "..." } ],
    "total": 134
  }
}
```

---

## 16. `subscribe_event`

把当前 Agent Session 订阅到一条 KEvent 路径模式，匹配事件会作为 user wakeup message 回灌。

### Prompt

- `description`: `Subscribe this Agent Session to a KEvent path pattern. Matching events are batched and delivered as natural-language user wakeup messages.`
- `args_schema`:
  ```json
  {
    "type": "object",
    "properties": {
      "pattern": {
        "type": "string",
        "description": "KEvent path pattern, for example /task_mgr/42 or /approval/**."
      },
      "message_template": {
        "type": "string",
        "description": "Optional natural-language rendering used when a matching event wakes the session. Supports {event_id}, {data}, and top-level JSON fields such as {status} or {message}."
      }
    },
    "required": ["pattern"]
  }
  ```

### Bash 支持

`CallingConventions::LLM` —— 仅 function calling，不暴露 bash 别名。

### CLI 命令解释 + 常用例子

只通过 LLM tool_call 调度：

```json
{ "pattern": "/task_mgr/42", "message_template": "task {event_id} -> {status}: {message}" }
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "subscribe_event",
  "cmd_args": "subscribe_event /task_mgr/42 message_template=<set>",
  "title": "subscribe_event /task_mgr/42 => success",
  "summary": "subscribed to /task_mgr/42",
  "detail": { "subscribed": true, "pattern": "/task_mgr/42" }
}
```

重复订阅返回 `subscribed=false`，`title` 改成 `=> already active`。

---

## 17. `unsubscribe_event`

取消订阅。

### Prompt

- `description`: `Remove a KEvent subscription from this Agent Session.`
- `args_schema`: `{ "type": "object", "properties": { "pattern": { "type": "string" } }, "required": ["pattern"] }`

### Bash 支持

`CallingConventions::LLM`。

### CLI 命令解释 + 常用例子

```json
{ "pattern": "/task_mgr/42" }
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "unsubscribe_event",
  "cmd_args": "unsubscribe_event /task_mgr/42",
  "title": "unsubscribe_event /task_mgr/42 => success",
  "summary": "unsubscribed from /task_mgr/42",
  "detail": { "unsubscribed": true, "pattern": "/task_mgr/42" }
}
```

订阅不存在时 `unsubscribed=false`，`title` 为 `=> not found`。

---

## 18. `check_task`

CLI 伪工具：从 `task_management` 拉一次任务状态，转成 `AgentToolResult`。

### Prompt

不走 `ToolSpec` 注册，没有 LLM 看得到的 description。Agent 通过 prompt 模板里的轮询规则去用它。

`usage`: `check_task <task_id>`

### Bash 支持

CLI-only。Session Exec Bin 不为它建链接（链接列表见 `BUILTIN_AGENT_TOOL_BINS = ["todo", "Glob", "Grep", "read_file"]`），所以一般通过 `agent_tool check_task ...` 全名调用，或由 runtime 主动调度。

### CLI 命令解释 + 常用例子

```bash
agent_tool check_task 12345
agent_tool check_task --task-id 12345
agent_tool check_task task_id=12345
```

### 输出示例

任务还在跑：

```json
{
  "agent_tool_protocol": "1",
  "status": "pending",
  "cmd_name": "check_task",
  "cmd_args": "check_task 12345",
  "title": "check_task 12345 => pending (long_running)",
  "summary": "task 12345 is running (estimated 30s)",
  "task_id": "12345",
  "pending_reason": "long_running",
  "check_after": 5,
  "detail": {
    "task_id": "12345",
    "task_status": "Running",
    "task_name": "build release",
    "task_progress": 0.42,
    "task": { "...": "完整 task 对象" }
  }
}
```

任务跑的是 `tool.exec_bash` 时，会顺带把 `output` / `return_code` 顶层带出来；非 exec_bash 任务则不带 `output`。

任务完成：

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "check_task",
  "title": "check_task 12345 => success",
  "summary": "task 12345 completed",
  "task_id": "12345",
  "detail": { "task_status": "Completed", "task": { "...": "..." } }
}
```

`pending_reason` 当前值集合：`long_running | user_approval | wait_for_install`。

---

## 19. `cancel_task`

CLI 伪工具：取消 pending task，可递归。

### Prompt

无 LLM `ToolSpec`。`usage`: `cancel_task <task_id> [--recursive]`

### Bash 支持

CLI-only，调用方式同 `check_task`。

### CLI 命令解释 + 常用例子

```bash
agent_tool cancel_task 12345
agent_tool cancel_task 12345 --recursive
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "cancel_task",
  "cmd_args": "cancel_task 12345",
  "title": "cancel_task 12345 => success",
  "summary": "canceled task 12345",
  "task_id": "12345",
  "detail": {
    "task_id": "12345",
    "task_status": "Canceled",
    "task": { "...": "..." },
    "recursive": false
  }
}
```

底层 interrupt 失败时 `detail.interrupt_error` 含原始错误信息，`summary` 追加 `(interrupt failed: ...)`，顶层 `status` 仍是 `success`（cancel 流程已经走完）。

---

## 20. `finish_task`

CLI 伪工具：把指定 task 结束为 `Completed` 或 `Failed`。默认是 `Completed`，失败结束时写入 TaskManager error/message。

### Prompt

无 LLM `ToolSpec`。`usage`: `finish_task <task_id> [failed] [--message <text>]`

### Bash 支持

CLI-only，调用方式同 `check_task`。

### CLI 命令解释 + 常用例子

```bash
agent_tool finish_task 12345
agent_tool finish_task --task-id 12345
agent_tool finish_task task_id=12345
agent_tool finish_task 12345 failed
agent_tool finish_task 12345 --failed --message "cannot route task"
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "finish_task",
  "cmd_args": "finish_task 12345",
  "title": "finish_task 12345 finished => success",
  "summary": "finished task 12345",
  "task_id": "12345",
  "detail": {
    "task_id": "12345",
    "task_status": "Completed",
    "task_progress": 100.0,
    "finish_outcome": "completed",
    "task": { "...": "..." }
  }
}
```

失败结束时顶层 `status` 仍是 `success`，表示 CLI 已成功把 task 标记为失败；业务失败状态在 `detail.task_status` / task 数据中体现。

---

## 21. `agent-memory`

CLI-only，Agent Memory Graph v2.10 入口。可执行文件名 `agent-memory` / `agent_memory`。

Memory 的真相源是 `.meta/occasions.jsonl`；`set` / `remove` 仍保留旧平铺手感，但写入的是 `kind = "free"` 的 Memory Item。`object` / `observe` / `relate` / `set-status` 等图操作会经 occasion log replay 成 canonical JSON、path index 和 `memory.sqlite` 派生缓存。

### Prompt

CLI-only，无 LLM `ToolSpec`，但通过 prompt 模板里的 memory 章节驱动。

`usage`:
```text
agent-memory [--root <path>] [--quiet] <init|occasion|object|observe|relate|set-status|set|remove|get|list|load|verify|compact> [...]
```

`--root` 默认从 RuntimeContext 推导为 `<OPENDAN_AGENT_ROOT>/memory/`；dev 态可用 `AGENT_MEMORY_ROOT` override（见[环境变量契约](#环境变量契约)）。actor / owner 上下文来自 RuntimeContext（Agent RootFS identity → BuckyOS runtime / `app_instance_config`），session 来自 `OPENDAN_SESSION_ID`。

> beta2.2 起，旧的 `OPENDAN_OWNER_USER_ID` / `OPENDAN_AGENT_ID` 已移除，不再作为 actor / owner 来源；不要再用 `--agent-id` 之类参数重复传上下文。

### Bash 支持

CLI-only。Session Exec Bin 不为它建链接，但可作为 `agent_tool agent-memory ...` 调用，或 runtime 提供子命令直接 expose。

### CLI 命令解释 + 常用例子

```bash
# 初始化目录
agent-memory init

# 写入 free hint（content 走 argv 或 stdin）
agent-memory set "/user/name" "Liu Zhicong" --reason "init profile" --tags "profile"
cat profile.md | agent-memory set "/user/profile" --reason "from file"

# 图写入
agent-memory occasion add --type session.turn --summary "User discussed BuckyOS memory graph." --tags "memory graph"
agent-memory observe add --occasion occ_0000000000000001 --kind explicit_statement --entities obj_user --confidence 0.8 "User prefers inspectable memory."
agent-memory object upsert --occasion occ_0000000000000001 --kind user --name "User" --object obj_user --alias "user" --alias-type name --evidence obs_0000000000000002_00 --confidence 0.9
agent-memory relate --occasion occ_0000000000000001 --subject obj_user --predicate prefers --object obj_memory_graph --weight 0.8 --confidence 0.7 --evidence obs_0000000000000002_00 --reason "May affect future architecture suggestions."

# 读 free content 或完整 JSON
agent-memory get "/user/name"
agent-memory get item item_0000000000000004_00
agent-memory get object obj_user

# 列出（可加 prefix）
agent-memory list /user/
agent-memory list objects --kind user

# 删
agent-memory remove "/user/name" --reason "stale"

# 触发召回 / 验证 / 压缩
agent-memory load --tags "memory graph" --objects obj_user --max-records 10 --max-bytes 8192
agent-memory verify
agent-memory compact
```

### 输出示例

`agent-memory get "/user/name"` 直接输出 free item 的 content；`agent-memory get item <item_id>` / `get object <object_id>` / `get observation <obs_id>` 输出完整 JSON。

`agent-memory occasion add`：

```text
OCCASION occ_0000000000000001
SEQ 1
```

`agent-memory load`：

```text
ITEM item_0000000000000004_00
KIND relation
ENTITIES obj_user,obj_memory_graph
WEIGHT 0.800
CONFIDENCE 0.700
SOURCE_OCCASION occ_0000000000000004
NOTICED_AT 2026-06-03T10:00:00Z
EVIDENCE obs_0000000000000002_00
MATCHED entity:obj_user,tag:memory graph
SIZE 35
TRUNCATED 0
---
obj_user prefers obj_memory_graph
END
```

`--quiet` 开启时抑制错误以外的辅助日志，不改变退出码。

---

## 22. `agent-notebook`

CLI-only，对应 `agent_tool::agent_notebook`。Agent 私有笔记 / system context 拼装入口。

### Prompt

CLI-only，由 prompt 模板里的 notebook 章节驱动。

`usage`（节选）：
```text
agent-notebook [--root <path>] \
  <create|update|append|read|list|build-system-context|build-registry-context|hints|mark-status|promote> [...]
```

`--root` 默认从 RuntimeContext 推导为 `<OPENDAN_AGENT_ROOT>/notebook/` 对应固定目录；dev 态可用 `AGENT_NOTEBOOK_ROOT` override（见[环境变量契约](#环境变量契约)）。Agent Notebook 归属当前 Agent，SQL 数据按当前 notebook DB 路径隔离；命令行参数只表达业务语义，不需要传 owner/session/actor/reason 类上下文参数。

### Bash 支持

CLI-only。

### CLI 命令解释 + 常用例子

```bash
agent-notebook create --bookid nb_01HXY --kind project --title "buckyos beta2.2"
agent-notebook append "read_tool unchanged 短路" --bookid nb_01HXY --stdin <<EOF
今天梳理了 read_tool 的 unchanged 短路。
EOF

agent-notebook read --bookid nb_01HXY
agent-notebook list
agent-notebook status itm_01HXY stale --reason "no longer applies"
agent-notebook remarks append itm_01HXY red "needs confirmation"
agent-notebook remarks list itm_01HXY --type red
agent-notebook remarks remove itm_01HXY rmk_01HXY

# 装配 prompt 用的 context
agent-notebook build-system-context
agent-notebook hints --topic-tags agent-notebook,state-management
```

### 输出示例

```json
{
  "agent_tool_protocol": "1",
  "status": "success",
  "cmd_name": "agent-notebook",
  "cmd_args": "agent-notebook read --bookid nb_01HXY",
  "title": "agent-notebook read nb_01HXY => ok",
  "summary": "read nb_01HXY (1.2 KB)",
  "detail": {
    "ok": true,
    "notebook": {
      "id": "nb_01HXY",
      "kind": "project",
      "title": "buckyos beta2.2",
      "items": [ { "status": "active", "text": "..." } ]
    }
  }
}
```

`build-system-context` / `build-registry-context` 返回的 `detail.text` 是已经拼好的 prompt 片段，可以直接喂回 system message。

---

## 附录

- 环境变量最小契约、`RuntimeContext` 推导规则、dev-only override、beta2.2 已移除变量的完整设计见 [OpenDAN AgentTool 开发指南.md](OpenDAN%20AgentTool%20%E5%BC%80%E5%8F%91%E6%8C%87%E5%8D%97.md) 第 6 节。
- 协议字段 / `status` / `pending` / `task_id` / `output` vs `detail` 等更多约束见 [agent_tool_result_protocol.md](agent_tool_result_protocol.md)。
- builtin tool 的设计约定（统一执行模型、function call schema、`detail` 与 arguments 的分工）见 [builtin_agent_tools.md](builtin_agent_tools.md)。
- CLI 命令完整解析逻辑见 `src/frame/agent_tool_cli_dev/src/lib.rs`；每个工具的 `parse_bash_args / parse_cli_args` 是 argv 转 JSON args 的权威实现。
- Session Exec Bin 当前对外暴露的 builtin 链接：`["todo", "Glob", "Grep", "read_file"]`（见 `src/frame/opendan/src/agent_bash.rs:76`）。其余工具通过 `agent_tool <tool>` 主入口调用或由 Action / function-call 调度。
