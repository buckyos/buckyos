# OpenDAN AgentTool 开发指南


阅读完本文，你应当能：

- 知道一个 AgentTool 在 Agent 的执行环境中如何被 `exec`/tmux/bash 调用
- 知道同一份工具实现如何同时服务 CLI、Action、LLM tool call、bash namespace
- 知道 `TypedTool` 与 `AgentTool` 的边界，新增工具时该放在哪里注册
- 知道 CLI stdout 的 `AgentToolResult` 协议、纯文本例外和 exit code 语义
- 知道如何用 `OPENDAN_*` 环境变量定位 Agent 环境，避免重复传 `--agent-env` / `--session-id`
- 知道当前内置 CLI 工具、Runtime 工具、MCP 工具扩展机制的真实状态

---

## 1. 设计基线

OpenDAN 当前 AgentTool 实现遵循以下基线：

1. **工具首先是可注册的 Runtime tool**
   核心接口在 `agent_tool` crate 中。Runtime 通过 `AgentToolManager` 注册工具，并按 `CallingConventions` 暴露到 bash、Action、LLM tool call 等 namespace。

2. **推荐用 `TypedTool` 实现新工具**
   `TypedTool` 提供 typed `Args` / `Output`、`schemars` schema、统一 JSON 序列化、默认 `AgentToolResult` 包装。只有需要自定义 `pending`、特殊错误包装、非标准结果 envelope 时，才直接实现底层 `AgentTool` trait。

3. **CLI 前端是单二进制 busybox 模式**
   最终可执行文件名是 `agent_tool`，由 `agent_tool_cli_dev` crate 生成。`read_file` / `write_file` / `todo` 等命令通过软链接指向同一二进制，进程入口根据 `argv[0]` 或 `agent_tool <tool>` 分发。

4. **CLI 默认 stdout 输出 `AgentToolResult` JSON**
   自有 AgentTool 的标准 stdout 是单行 JSON，带 `agent_tool_protocol: "1"`。当前唯一明确的纯文本例外是 `read_file` 在非 Agent 环境且 stdout 非 TTY 时输出文件内容，便于管道使用。

5. **上下文来自环境变量，不来自重复 CLI 参数**
   CLI 通过 `OPENDAN_AGENT_ENV`、`OPENDAN_SESSION_ID`、`OPENDAN_AGENT_ID` 等变量还原 `SessionRuntimeContext`。缺少 `OPENDAN_AGENT_ENV` 时，开发态回退到当前工作目录。

---

## 2. 调用模型：Agent → exec → tmux → CLI 工具

OpenDAN 的 bash 执行入口是 Runtime 工具 `exec`，实现位于 `src/frame/opendan/src/agent_bash.rs`，常量名是 `TOOL_EXEC_BASH`，对外 tool name 是 `"exec"`。

一次典型调用链路：

```text
LLM / Action 产生 exec command
  |
  v
Runtime 调用 ExecBashTool::call
  |
  v
prepare_session_tool_env 刷新当前 session 的工具软链接目录
  |
  v
build_exec_env_vars 注入 PATH 与 OPENDAN_* 环境变量
  |
  v
tmux pane 中的 bash 执行命令
  |
  v
PATH 命中 read_file/write_file/... 软链接，启动 agent_tool
  |
  v
agent_tool_cli_dev::run_process 根据 argv[0] / 子命令分发
  |
  v
CLI 构造 CliRuntimeEnv，调用 agent_tool crate 内的工具实现
  |
  v
stdout 输出 AgentToolResult JSON，exec 再按协议解析或作为普通 bash 输出包装
```

### 2.1 单二进制 + 软链接分发

当前 CLI 前端在 `src/frame/agent_tool_cli_dev/src/lib.rs`：

- `MAIN_BINARY_NAME = "agent_tool"`
- `TOOL_NAMES` 是 CLI 可识别命令清单
- `parse_command()` 先检查 `argv[0]` 是否是工具名
- 也支持 `agent_tool <tool_name> <args...>`
- `__command_not_found__` 是 command-not-found proxy 的占位入口，未知命令返回 `127`

当前 CLI `TOOL_NAMES` 包含：

```text
read_file
write_file
edit_file
get_session
set_memory
remove_memory
todo
create_workspace
bind_workspace
check_task
cancel_task
```

注意：

- `check_task` / `cancel_task` 是 CLI pseudo-tool，不在 `AgentToolManager` 注册表中。
- `load_memory` 当前是 Runtime 注册的 LLM/bash-capable tool，但没有加入 `agent_tool_cli_dev::TOOL_NAMES`，也没有默认 session 软链接。
- `worklog_manage` 的 `TypedTool` 仍在 `agent_tool` crate 中，但当前 OpenDAN workshop 不再把它暴露成 Runtime tool，只保留 `tools.json` 参数解析给写审计复用。

### 2.2 session 工具目录与 PATH

`ExecBashTool::prepare_session_tool_env()` 每次执行 `exec` 前都会刷新工具目录：

```text
<BUCKYOS_ROOT>/tools/<sanitized_agent_id>/<session_id>/
```

实现细节：

- 路径由 `resolve_session_tool_dir()` 生成。
- `<sanitized_agent_id>` 来自 `sanitize_token_for_id()`，非字母数字字符会被压成 `_`。
- `sync_session_tool_links()` 会删除目录里不再可见的链接，并补齐当前可见工具的软链接。
- 每个软链接都指向解析到的 `agent_tool` 主二进制。
- 非 Unix 平台当前会返回错误，因为实现依赖 Unix symlink。

当前默认 session CLI 工具来自：

```rust
EXEC_BASH_AGENT_CLI_TOOL_NAMES = [
    "read_file",
    "write_file",
    "edit_file",
    "get_session",
    "set_memory",
    "remove_memory",
    "todo",
    "create_workspace",
    "bind_workspace",
]

EXEC_BASH_ALWAYS_AVAILABLE_CLI_TOOL_NAMES = [
    "check_task",
    "cancel_task",
]
```

`loaded_tools` 的语义不是“默认集 + 增量扩展”。当前实现是：

- session 没有 `loaded_tools` 时，使用默认 CLI 工具 + always-available 工具
- behavior 配置为 `tools.mode = all` 时，`loaded_tools` 置空，因此仍使用默认集
- behavior 配置为 allow-list 时，只从 allow-list 中保留 `EXEC_BASH_AGENT_CLI_TOOL_NAMES` 里的名字，再追加 `check_task` / `cancel_task`
- behavior 配置为 none 时，会写入占位值，最终只剩 `check_task` / `cancel_task`

因此，当前 session 软链接目录只管理内置 CLI 工具子集；MCP 或其它 Runtime 注册工具不会自动出现在这个软链接目录里。



### 2.3 注入到 tmux 的环境变量

`build_exec_env_vars()` 会合并用户传入 env，再写入 OpenDAN 上下文变量。关键变量：

| 变量 | 含义 |
|------|------|
| `PATH` | 前置 session 工具软链接目录 |
| `OPENDAN_AGENT_BIN` | `agent_tool` 所在目录 |
| `OPENDAN_AGENT_TOOL` | `agent_tool` 主二进制绝对路径 |
| `OPENDAN_SESSION_TOOL_PATH` | 当前 session 工具软链接目录 |
| `OPENDAN_AGENT_ENV` | 当前 agent env root |
| `OPENDAN_AGENT_ID` | 当前 agent DID / name |
| `OPENDAN_SESSION_ID` | 当前 session id |
| `OPENDAN_BEHAVIOR` | 当前 behavior 名 |
| `OPENDAN_STEP_IDX` | 当前 step 序号 |
| `OPENDAN_WAKEUP_ID` | 当前 wakeup id |
| `OPENDAN_TRACE_ID` | 当前 trace id |

CLI 侧 `CliRuntimeEnv::from_process()` 只读取 `OPENDAN_AGENT_ENV`、`OPENDAN_AGENT_ID`、`OPENDAN_SESSION_ID`、`OPENDAN_BEHAVIOR`、`OPENDAN_STEP_IDX`、`OPENDAN_WAKEUP_ID`、`OPENDAN_TRACE_ID` 来构造调用上下文。

### 2.4 `exec` 如何识别 AgentTool JSON

tmux 命令执行结束后，`decode_exec_bash_json_result()` 只有在命令看起来是内部 AgentTool 命令时才尝试解析 stdout：

- 命令名是 `agent_tool`
- 或命令名在 `default_agent_cli_tool_names()` 中
- stdout 是 JSON
- JSON 顶层 `agent_tool_protocol == "1"`

解析成功后，Runtime 会把它作为 `AgentToolResult` 返回，并补上 `cmd_name` / `cmd_args` / `return_code`。解析失败或普通 bash 命令则走 `build_default_exec_bash_result()`，把 tmux 捕获到的混合输出放进 `output`。

---

## 3. Runtime 工具模型：`TypedTool`、`AgentTool`、`CallingConventions`

### 3.1 推荐接口：`TypedTool`

新工具优先实现 `src/frame/agent_tool/src/tool.rs` 中的 `TypedTool`：

```rust
#[async_trait]
pub trait TypedTool: Send + Sync + 'static {
    type Args: DeserializeOwned + JsonSchema + Send;
    type Output: Serialize + JsonSchema + Send;

    fn name(&self) -> &str;
    fn description(&self) -> &str { "" }
    fn calling(&self) -> CallingConventions { CallingConventions::ALL }
    fn args_schema(&self) -> Json { ... }
    fn output_schema(&self) -> Json { ... }
    fn usage(&self) -> Option<String> { None }
    fn build_cmd_line(&self, _args: &Self::Args) -> Option<String> { None }
    fn build_summary(&self, _output: &Self::Output) -> String { "ok".to_string() }
    fn build_title(&self, _output: &Self::Output) -> Option<String> { None }
    fn parse_bash_args(&self, tokens: &[String], shell_cwd: Option<&Path>) -> Result<Json, AgentToolError> { ... }
    fn parse_cli_args(&self, tokens: &[String], shell_cwd: Option<&Path>) -> Result<CliInvocation, AgentToolError> { ... }
    fn cli_plain_text_stdout(&self) -> bool { false }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Self::Args) -> Result<Self::Output, AgentToolError>;
}
```

`AgentToolManager::register_typed_tool()` 会把它包装成 `TypedToolHandle<T>`，对外实现底层 `AgentTool` trait。

### 3.2 底层接口：`AgentTool`

底层 trait 位于 `src/frame/agent_tool/src/lib.rs`：

```rust
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn calling(&self) -> CallingConventions;

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError>;

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> { ... }

    fn parse_cli_args(
        &self,
        tokens: &[String],
        shell_cwd: Option<&Path>,
    ) -> Result<CliInvocation, AgentToolError> { ... }

    fn cli_plain_text_stdout(&self) -> bool { false }
}
```

只有以下情况建议直接实现 `AgentTool`：

- 工具需要返回 `status: pending`、`task_id`、`pending_reason`、`partial_output` 等 typed pipeline 不方便表达的字段
- 工具需要非常特殊的 envelope 或错误包装
- 工具本身就是 Runtime 控制工具，例如 `exec`

### 3.3 `CallingConventions`

当前不再使用旧的 `support_bash()` / `support_action()` / `support_llm_tool_call()` 三个 bool，而是使用 bitflag：

```rust
CallingConventions::BASH
CallingConventions::ACTION
CallingConventions::LLM
CallingConventions::ALL
```

含义：

| Convention | 谁使用 | 说明 |
|------------|--------|------|
| `BASH` | `get_bash_cmd()` / `call_tool_from_bash_line()` / bash namespace | 表示工具能从 bash 风格参数解析执行 |
| `ACTION` | `execute_actions()` | 表示工具可出现在 LLM step 末尾的结构化 action 里 |
| `LLM` | `list_tool_specs()` / `call_tool()` policy | 表示工具会作为标准 LLM tool call 暴露 |

`AgentToolManager::register_tool_arc()` 要求 `calling()` 非空。注册后只维护一份 `tools` map，具体 namespace 查询通过 `calling()` 过滤：

- `get_tool()`：只返回 `LLM` 工具
- `get_bash_cmd()`：只返回 `BASH` 工具
- `get_action()`：只返回 `ACTION` 工具
- `get_any_tool()`：忽略 namespace，CLI 前端使用它
- `list_tool_specs()`：只列出 `LLM` 工具
- `list_bash_cmd_specs()`：只列出 `BASH` 工具
- `list_action_tool_specs()`：只列出 `ACTION` 工具

### 3.4 `call`、`exec`、`parse_cli_args` 的关系

推荐实现心智模型：

- `execute(ctx, typed_args)` 是 `TypedTool` 的核心业务实现。
- `call(ctx, Json)` 是 Runtime 结构化调用入口，由 `TypedToolHandle` 负责 JSON 反序列化后调用 `execute()`。
- `exec(ctx, line, shell_cwd)` 是 bash command line 入口，会先 tokenize，再调用 `parse_bash_args()`。
- `parse_cli_args(tokens, shell_cwd)` 是外部 `agent_tool` CLI 的 argv 解析入口，返回：
  - `CliInvocation::Bash { line }`：复用 `exec()` 解析
  - `CliInvocation::Json { args, content_input }`：直接走 `call()`

如果工具 CLI 只是 bash 风格的位置参数，通常不用覆写 `parse_cli_args()`；如果工具要支持 `--flag value`、`--content-stdin` 这类 CLI 语法，就应该在工具自身实现里覆写，而不是在 CLI dispatcher 里新增分支。

### 3.5 `ToolHost`：Runtime backend 注入

当前实现用 `ToolHost` 把多个 backend 能力收敛成一个访问面：

```rust
pub trait ToolHost: Send + Sync {
    fn session_view(&self) -> Option<&dyn SessionViewBackend> { None }
    fn workspace_runtime(&self) -> Option<&dyn WorkspaceRuntimeBackend> { None }
    fn workspace_tool(&self) -> Option<&dyn WorkspaceToolBackend> { None }
    fn external_workspace(&self) -> Option<&dyn ExternalWorkspaceBackend> { None }
    fn memory_load(&self) -> Option<&dyn MemoryLoadBackend> { None }
    fn memory_mutation(&self) -> Option<&dyn MemoryMutationBackend> { None }
    fn worklog_action(&self) -> Option<&dyn WorklogActionBackend> { None }
    fn file_write_audit(&self) -> Option<&dyn FileWriteAuditBackend> { None }
}
```

`AgentToolManager::with_host()` / `set_host()` 可以配置 host，`register_typed_tool()` 会把当前 host 捕获进 `TypedToolHandle`。多数现有工具仍直接持有 backend `Arc`；新代码应优先沿用当前模块里的模式，不为了统一而做大范围重构。

---

## 4. 当前工具注册位置

### 4.1 OpenDAN Agent 初始化注册

`src/frame/opendan/src/agent.rs::new` 当前会：

1. 创建 `AgentToolManager`
2. 创建 `AgentEnvironment`
3. 调用 `environment.register_workshop_tools_with_task_mgr(...)`
4. 创建 `AgentMemory` 并调用 `memory.register_tools(&tools)`
5. 注册 `GetSessionTool`
6. 注册 `LoadSkillTool` / `UnloadSkillTool`

`ai_runtime.rs::register_tools()` 还会注册：

- `create_sub_agent`
- `bind_external_workspace`
- `list_external_workspaces`

### 4.2 Workshop 默认工具

`AgentWorkshopToolsConfig::default()` 默认启用：

```text
exec
edit_file
write_file
read_file
todo_manage
create_workspace
bind_workspace
```

注册逻辑在 `AgentWorkshop::register_tools_with_task_mgr()`：

- `exec` 直接实现底层 `AgentTool`
- `read_file` / `write_file` / `edit_file` 注册 typed file tools
- `todo_manage` 注册 `TodoTool`，实际 tool name 是 `todo`
- `create_workspace` / `bind_workspace` 注册 workspace typed tools
- `worklog_manage` 当前不注册为 Runtime tool，只解析配置用于 file write audit
- `kind = "mcp"` 会注册 `MCPTool`

### 4.3 Memory 工具

`AgentMemory::register_tools()` 会注册：

- `load_memory`
- `set_memory`
- `remove_memory`

其中 `load_memory` 当前支持 `BASH | LLM`，`set_memory` / `remove_memory` 支持 `BASH | LLM`；但 CLI 前端只暴露 `set_memory` / `remove_memory`。

### 4.4 Skill 工具

`LoadSkillTool` / `UnloadSkillTool` 位于 `src/frame/opendan/src/skill_tool.rs`，当前：

- `calling() = ACTION | LLM`
- 不作为默认 CLI 软链接出现
- 修改 session 的 skill 加载状态后持久化 session

### 4.5 MCP 工具

`tools/tools.json` 里 `kind = "mcp"` 的工具会注册成 `MCPTool`。当前 `MCPTool`：

- `calling() = BASH | ACTION`
- 不支持 `LLM`
- 不会自动加入 `agent_tool_cli_dev::TOOL_NAMES`
- 不会自动加入 session 软链接目录

因此 MCP 的 `BASH` namespace 当前主要服务 Runtime 的 in-process bash/action 管理能力，不等同于一个真实落地到 `$PATH` 的 CLI 命令。

---

## 5. CLI 输出协议

### 5.1 标准输出：`AgentToolResult`

CLI 默认输出一行 JSON。结构定义见 `src/frame/agent_tool/src/lib.rs::AgentToolResult` 与 [agent_tool_result_protocol.md](agent_tool_result_protocol.md)。

示例：

```json
{
  "agent_tool_protocol": "1",
  "tool": "read_file",
  "cmd_name": "read_file",
  "cmd_args": "demo.txt 1-20",
  "status": "success",
  "title": "read_file demo.txt 1-20 => success",
  "summary": "succeeded, read 128 bytes across 20 lines",
  "detail": {
    "content": "hello"
  }
}
```

关键规则：

- `agent_tool_protocol` 必须是字符串 `"1"`。
- 反序列化只接受当前协议版本；历史别名不应再输出。
- `status` 取值为 `success` / `error` / `pending`。
- `title` / `summary` 是 prompt/history 压缩字段。
- `detail` 是内置工具结构化返回体。
- `output` 通常用于普通 bash 或明确需要终端文本输出的场景。
- `cmd_name` / `cmd_args` 表示 bash 风格命令形态。
- `return_code` 是进程或 bash 命令退出码，`exec` 解码 AgentTool JSON 后会补上 tmux 命令的退出码。

`TypedToolHandle` 会把 typed output 序列化成 `detail`，再通过 `build_builtin_tool_result()` 生成 `cmd_name` / `cmd_args` / `summary` / 默认 `title`。

### 5.2 exit code

CLI exit code 常量在 `agent_tool` crate：

| 常量 | 值 | 用途 |
|------|----|------|
| `CLI_EXIT_SUCCESS` | `0` | 成功 |
| `CLI_EXIT_ERROR` | `1` | 执行失败 / timeout / already exists |
| `CLI_EXIT_USAGE` | `2` | 参数错误 / tool not found |
| `CLI_EXIT_COMMAND_NOT_FOUND` | `127` | command-not-found proxy |

`cli_exit_code_for_error()` 当前映射：

- `InvalidArgs` / `NotFound` → `2`
- `AlreadyExists` / `ExecFailed` / `Timeout` → `1`

### 5.3 `pending`

协议支持 `pending`：

```json
{
  "agent_tool_protocol": "1",
  "status": "pending",
  "summary": "PENDING (long_running, check_after=5s)",
  "task_id": "12345",
  "pending_reason": "long_running",
  "check_after": 5,
  "partial_output": "building target..."
}
```

`pending_reason` 当前枚举：

- `long_running`
- `user_approval`
- `wait_for_install`

反序列化兼容历史值 `external_callback`，但新输出不要再使用。

当前 `exec` 长任务会通过 task manager 创建任务，并返回 `pending`。CLI 里的 `check_task <task_id>` / `cancel_task <task_id>` 通过 `TaskManagerClient` 查询或取消任务。

### 5.4 纯文本例外

`read_file` 当前唯一显式 opt-in：

- `ReadFileTool::cli_plain_text_stdout() == true`
- CLI 环境 `!has_agent_env && !stdout_is_terminal`

满足时，`agent_tool_cli_dev` 会把 `detail.content` 直接写到 stdout，而不是输出 JSON。这是为了让本地开发时的管道用法自然工作：

```bash
read_file ./demo.txt | wc -l
```

新工具如果要加入纯文本模式，必须同时满足：

- 切换条件由环境变量 / TTY 状态明确决定
- 不用内容启发式判断
- exit code 语义不变
- 在 [builtin_agent_tools.md](builtin_agent_tools.md) 写清 JSON 模式与纯文本模式的切换条件

### 5.5 stdout / stderr 分工

- stdout 只写最终协议 JSON，或纯文本模式下的主内容。
- stderr 写进度、警告、调试日志。
- `agent_tool_cli_dev::main` 会分别 print stdout 和 stderr，并用 `CliRunOutput.exit_code` 退出。

---

## 6. 环境变量与 Agent RootFS

CLI 启动时通过 `CliRuntimeEnv::from_process()` 构造：

| env | 用途 | 缺失回退 |
|-----|------|----------|
| `OPENDAN_AGENT_ENV` | agent env root / CLI state root | 当前 `cwd` |
| `OPENDAN_AGENT_ID` | `SessionRuntimeContext.agent_name` | `did:opendan:cli` |
| `OPENDAN_SESSION_ID` | session id | `cli-session` |
| `OPENDAN_BEHAVIOR` | behavior 名 | `cli` |
| `OPENDAN_STEP_IDX` | step 序号 | `0` |
| `OPENDAN_WAKEUP_ID` | wakeup id | `cli-wakeup` |
| `OPENDAN_TRACE_ID` | trace id | `cli-trace` |

CLI state root 规则：

- 有 `OPENDAN_AGENT_ENV`：state root 就是 `agent_env_root`
- 没有 `OPENDAN_AGENT_ENV`：state root 是 `<cwd>/.opendan-cli`
- 文件工具在无 Agent 环境时会清空读写 root 限制，方便本地调试

Agent RootFS 布局以 [../opendan/Agent RootFS.md](../opendan/Agent%20RootFS.md) 为准。常用路径：

| 资源 | 路径 |
|------|------|
| todo DB | `<agent_env_root>/todo/todo.db` |
| worklog DB | `<agent_env_root>/worklog/worklog.db` |
| session 记录 | `<agent_env_root>/sessions/<session_id>/session.json` |
| workspace index | `<agent_env_root>/index.json` |
| session workspace 绑定 | `<agent_env_root>/workspaces/session_workspace_bindings.json` |
| local workspace | `<agent_env_root>/workspaces/<workspace_id>/` |
| local workspace worklog DB | `<local_workspace_root>/worklog/worklog.db` |
| memory 根 | `<agent_env_root>/memory/` |

新工具不要通过 `--agent-env` / `--session-id` / `--agent-id` 重复传上下文参数。命令行参数应只表达业务语义。

---

## 7. 新增工具的最小步骤

### 7.1 新增 Runtime tool

1. 在 `src/frame/agent_tool/src/` 或对应 OpenDAN runtime 模块中新增工具实现。
2. 优先实现 `TypedTool`：
   - 定义 `Args: Deserialize + JsonSchema`
   - 定义 `Output: Serialize + JsonSchema`
   - 实现 `name()` / `description()` / `calling()` / `execute()`
   - 必要时实现 `usage()` / `build_cmd_line()` / `build_summary()` / `parse_bash_args()` / `parse_cli_args()`
3. 如果需要 Runtime 能力，优先复用现有 backend trait 或 `ToolHost` slot；确实缺能力时再新增 trait，并由 Runtime 注册时注入。
4. 在正确的 Runtime 注册点注册：
   - workshop 默认工具：`AgentWorkshop::register_tools_with_task_mgr()`
   - memory 工具：`AgentMemory::register_tools()`
   - runtime 级工具：`AiRuntime::register_tools()`
   - agent 初始化固定工具：`Agent::new`
5. 更新 [builtin_agent_tools.md](builtin_agent_tools.md) 的工具清单与输入输出约定。
6. 添加单元测试，至少覆盖 typed args 解析、成功结果、错误结果；涉及 Runtime 注册的补集成测试。

### 7.2 新增真实 CLI 命令

如果工具还必须能在 tmux bash 的 `$PATH` 中以命令形式运行，需要额外做：

1. 把工具注册到 `agent_tool_cli_dev::build_cli_tool_manager()`。
2. 把命令名加入 `agent_tool_cli_dev::TOOL_NAMES`。
3. 如果希望默认 session 有软链接，把命令名加入 `agent_bash.rs::EXEC_BASH_AGENT_CLI_TOOL_NAMES`。
4. 如果命令应始终可用，即使 behavior tools 为 none，也加入 `EXEC_BASH_ALWAYS_AVAILABLE_CLI_TOOL_NAMES`。
5. 如需 `exec` 直接把 stdout JSON 解码成 AgentToolResult，确认 `is_internal_agent_tool_command()` 能识别该命令。当前它基于 `default_agent_cli_tool_names()`。
6. 更新 `src/frame/agent_tool/create_tmux_debug_session.sh` 的调试软链接列表。
7. 在本地用 debug tmux session 或直接运行 `agent_tool <tool>` 验证 stdout JSON 与 exit code。

### 7.3 新增 MCP 工具

MCP 工具当前通过 `tools/tools.json` 配置，不需要修改 Rust 代码。配置项最终构造成 `MCPToolConfig`：

- `name`
- `endpoint`
- `mcp_tool_name`
- `description`
- `args_schema`
- `output_schema`
- `headers`
- `timeout_ms`

MCP 工具当前注册为 `BASH | ACTION`，不自动暴露为 LLM tool call，也不自动获得 `$PATH` 软链接。

---

## 8. 调试与验证

### 8.1 构建

在 `src/` 目录下优先使用仓库脚本：

```bash
uv run buckyos-build.py --skip-web
```

只验证 Rust crate 时可运行：

```bash
cargo test -p agent_tool
cargo test -p agent_tool_cli_dev
cargo test -p opendan
```

### 8.2 CLI 直接调试

构建出 `agent_tool` 后，可以直接：

```bash
agent_tool read_file ./demo.txt
read_file ./demo.txt
write_file ./demo.txt --mode write --content "hello"
```

没有 `OPENDAN_AGENT_ENV` 时，CLI 使用当前目录和 `.opendan-cli` 作为开发态 state root。

### 8.3 tmux 调试 session

仓库提供：

```bash
src/frame/agent_tool/create_tmux_debug_session.sh <agent_tool_binary> [session_name] [agent_env_root]
```

示例：

```bash
src/frame/agent_tool/create_tmux_debug_session.sh /opt/buckyos/bin/opendan/agent_tool od-debug /tmp/od-agent-env
```

脚本会：

- 创建临时工具软链接目录
- 给 `agent_tool`、`read_file`、`write_file`、`edit_file` 等命令建软链接
- 注入 `OPENDAN_*` 环境变量
- 前置 PATH
- attach 到 tmux session

---

## 9. 反模式速查

以下做法和当前实现方向冲突：

- 给工具新增 `--agent-env` / `--session-id` / `--agent-id` 这类重复上下文参数。
- 在 stdout 混写调试文本和 JSON。
- 新工具优先直接实现底层 `AgentTool`，却没有 `pending` 或特殊 envelope 需求。
- 在 `agent_tool_cli_dev` 的 dispatcher 中堆工具专用解析分支，而不是让工具覆写 `parse_cli_args()`。
- 只把工具注册进 `AgentToolManager`，却误以为它会自动变成 `$PATH` 里的 CLI 命令。
- 把 MCP 工具当成自动可见的 CLI 软链接工具。
- 通过向上扫描目录、检查 `todo.db` / `worklog.db` 是否存在来猜 `agent_env_root`。
- 输出不带 `agent_tool_protocol: "1"` 的自有工具 JSON。
- 新增工具后只改代码，不更新 [builtin_agent_tools.md](builtin_agent_tools.md)。

---

## 10. 参考资料

- [readme.md](readme.md)：AgentTool CLI 化与异步执行模型的设计背景
- [agent_tool_result_protocol.md](agent_tool_result_protocol.md)：`AgentToolResult` 字段定义与渲染规则
- [builtin_agent_tools.md](builtin_agent_tools.md)：当前 builtin tools 输入 / 输出约定
- [../opendan/Agent RootFS.md](../opendan/Agent%20RootFS.md)：Agent env root 目录布局与路径规则
- [../opendan/Agent Session.md](../opendan/Agent%20Session.md)：Agent Session 需求
- [../opendan/Agent Skill.md](../opendan/Agent%20Skill.md)：Skill 与 Tool 边界
- `src/frame/agent_tool/src/lib.rs`：`AgentToolResult`、`AgentTool`、`AgentToolManager`
- `src/frame/agent_tool/src/tool.rs`：`TypedTool`、`CallingConventions`、`ToolHost`
- `src/frame/agent_tool_cli_dev/src/lib.rs`：CLI 分发、`CliRuntimeEnv`、pseudo-tool
- `src/frame/opendan/src/agent_bash.rs`：`exec`、tmux、session 工具目录、环境变量注入
- `src/frame/opendan/src/workspace/workshop.rs`：workshop 工具注册、`tools/tools.json`、MCP 注册
- `src/frame/agent_tool/create_tmux_debug_session.sh`：本地 tmux 调试脚本
