# OpenDAN AgentTool 开发指南


阅读完本文，你应当能：

- 知道一个 AgentTool 在 Agent 的执行环境中如何被 `exec`/tmux/bash 调用
- 知道同一份工具实现如何同时服务 CLI、Action、LLM tool call、bash namespace
- 知道 `TypedTool` 与 `AgentTool` 的边界，新增工具时该放在哪里注册
- 知道 CLI stdout 的 `AgentToolResult` 协议、纯文本例外和 exit code 语义
- 知道 AgentTool 的最小环境变量契约，以及其它上下文如何从 Agent RootFS / session 路径推导
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

5. **环境变量只承载最小启动契约**
   目标设计中，AgentTool 进程只依赖 `OPENDAN_AGENT_ROOT`、`OPENDAN_SESSION_ID`、`BUCKYOS_APPCLIENT_SESSION_TOKEN` 和可选 `OPENDAN_TRACE_ID`。`agent_id`、owner、tool bin、workspace、memory、notebook 等上下文都应从 Agent RootFS、session state 或 BuckyOS runtime 推导，不再通过一组 `OPENDAN_*` 变量重复注入。

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
build_exec_env_vars 注入 PATH 与 AgentTool 最小环境变量契约
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
Glob
Grep
dcrontab
read
x-call
x_call
read_file
write_file
edit_file
todo
get_session
create_workspace
bind_workspace
agent-memory
agent-notebook
agent-skills
check_task
cancel_task
finish_task
```

注意：

- `read` / `x-call` / `x_call` / `check_task` / `cancel_task` / `finish_task` 是 CLI pseudo-tool，不在 `AgentToolManager` 注册表中。
- `read` / `x-call` 通过 `agent-did-object-lib` 加载 `ObjectRouteConfig` 并调用 `AgentDIDObjectRuntime`，用于替代旧 `agent_tool::read_tool` 的 CLI 接入路径。
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
    "finish_task",
]
```

`loaded_tools` 的语义不是“默认集 + 增量扩展”。当前实现是：

- session 没有 `loaded_tools` 时，使用默认 CLI 工具 + always-available 工具
- behavior 配置为 `tools.mode = all` 时，`loaded_tools` 置空，因此仍使用默认集
- behavior 配置为 allow-list 时，只从 allow-list 中保留 `EXEC_BASH_AGENT_CLI_TOOL_NAMES` 里的名字，再追加 `check_task` / `cancel_task` / `finish_task`
- behavior 配置为 none 时，会写入占位值，最终只剩 `check_task` / `cancel_task` / `finish_task`

因此，当前 session 软链接目录只管理内置 CLI 工具子集；MCP 或其它 Runtime 注册工具不会自动出现在这个软链接目录里。



### 2.3 注入到 tmux 的环境变量

目标设计下，`exec` / tmux 只注入最小启动契约：

| 变量 | 必需 | 含义 |
|------|------|------|
| `PATH` | 是 | 前置 session 工具软链接目录。它是进程执行环境，不作为 AgentTool 上下文来源。 |
| `OPENDAN_AGENT_ROOT` | 是 | 当前 Agent RootFS / state root。 |
| `OPENDAN_SESSION_ID` | 是 | 当前 session id。沿用现有变量名，不新增 `OPENDAN_AGENT_SESSIONID`。 |
| `BUCKYOS_APPCLIENT_SESSION_TOKEN` | 是 | AgentTool 作为 AppClient 访问 BuckyOS runtime / kRPC 服务的 token。 |
| `OPENDAN_TRACE_ID` | 否 | trace id。缺失时由调用端或工具侧生成默认值。 |

其它历史变量不再作为外部契约：

| 历史变量 | 新规则 |
|----------|--------|
| `OPENDAN_AGENT_ENV` | 由 `OPENDAN_AGENT_ROOT` 取代。 |
| `OPENDAN_AGENT_ID` | 从 Agent RootFS identity metadata 或 BuckyOS runtime 推导。 |
| `OPENDAN_OWNER_USER_ID` / `OPENDAN_AGENT_OWNER` / `BUCKYOS_OWNER_USER_ID` | 从 Agent RootFS identity metadata、`app_instance_config` 或 BuckyOS runtime 推导。 |
| `OPENDAN_AGENT_TOOL` / `OPENDAN_AGENT_BIN` / `OPENDAN_SESSION_TOOL_PATH` | 由 runtime 根据路径规则计算，并通过 `PATH` 或 generated shell hook 使用，不要求工具进程读取。 |
| `OPENDAN_BEHAVIOR` / `OPENDAN_STEP_IDX` / `OPENDAN_WAKEUP_ID` | 运行时 step 上下文，写入 session state；需要时由工具按 `OPENDAN_AGENT_ROOT + OPENDAN_SESSION_ID` 读取。 |
| `AGENT_MEMORY_ROOT` / `AGENT_NOTEBOOK_ROOT` | 降级为 dev-only CLI override；生产 AgentTool 从 Agent RootFS 固定路径推导。 |
| `OPENDAN_WORKFLOW_URL` / `OPENDAN_TASK_MANAGER_URL` 等服务 URL fallback | 降级为 dev-only fallback；生产环境通过 BuckyOS runtime client 发现服务。 |

当前代码仍处于多变量兼容状态，`CliRuntimeEnv::from_process()` 还会读取旧 `OPENDAN_*` 变量。实现迁移时应先新增统一的 runtime context resolver，再逐步删除旧变量读取点。

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

当前 `exec` 长任务会通过 task manager 创建任务，并返回 `pending`。CLI 里的 `check_task <task_id>` / `cancel_task <task_id>` / `finish_task <task_id> [failed]` 通过 `TaskManagerClient` 查询、取消、完成或失败结束任务。

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

### 5.5 DID Object 访问 CLI

`read` 和 `x-call` 是 `agent-did-object-lib` 的 CLI 接入准备层。二者加载同一份 `ObjectRouteConfig`，构造 `AgentDIDObjectRuntime` 后直接返回库生成的 `AgentToolResult`，CLI 不再二次拼接 read / x-call 结果。

配置加载顺序：

1. 命令行 `--config <route.toml>` 或 `--route-config <route.toml>`。
2. 环境变量 `AGENT_DID_OBJECT_ROUTE_CONFIG`。
3. 环境变量 `OPENDAN_AGENT_OBJECT_ROUTE_CONFIG`。
4. 内置 dev 默认配置：`file://` read 走 filesystem，`http://` / `https://` read 走 web，`https://` x-call 走 did_object，`agent://` 走 agent_runtime。

示例：

```bash
agent_tool read ./demo.txt --content-only --offset 1 --limit 20
agent_tool read uri=file:///tmp/demo.txt content_only=true
agent_tool x-call --config ./object-routes.toml obj://demo/item reserve qty=2
agent_tool x-call https://device.example.com/cam01 restart --params '{"delay_ms":1000}'
```

CLI 层只做 Gateway 第一层的本地便利转换：没有 `://` 的 `read` / `x-call` object 会按当前工作目录解析成 canonical `file://` URL。进入 `agent-did-object-lib` 后仍只接受 URL，不接受 DID URI、alias 或裸路径。

### 5.6 stdout / stderr 分工

- stdout 只写最终协议 JSON，或纯文本模式下的主内容。
- stderr 写进度、警告、调试日志。
- `agent_tool_cli_dev::main` 会分别 print stdout 和 stderr，并用 `CliRunOutput.exit_code` 退出。

---

## 6. 环境变量与 Agent RootFS

AgentTool 环境变量的目标设计是“少传上下文，多按路径和 runtime 推导”。生产运行时只承认下面几个变量为稳定契约：

| env | 必需 | 用途 | 缺失处理 |
|-----|------|------|----------|
| `OPENDAN_AGENT_ROOT` | 是 | 当前 Agent RootFS / state root | 生产环境报错；CLI dev 可回退到当前 `cwd` |
| `OPENDAN_SESSION_ID` | 是 | 当前 session id | 生产环境报错；CLI dev 可回退到 `cli-session` |
| `BUCKYOS_APPCLIENT_SESSION_TOKEN` | 是 | 访问 BuckyOS runtime / kRPC 服务 | 需要 RPC 的工具报错；纯文件 dev 工具可不要求 |
| `OPENDAN_TRACE_ID` | 否 | trace id / 日志关联 | 缺失时生成或使用默认 trace |

命名约定：

- session id 继续使用现有 `OPENDAN_SESSION_ID`，不新增 `OPENDAN_AGENT_SESSIONID`。
- `OPENDAN_AGENT_ROOT` 取代旧 `OPENDAN_AGENT_ENV`。迁移期间 CLI 可以同时接受旧变量，但新代码只应写入 / 文档化 `OPENDAN_AGENT_ROOT`。
- 除上表外，其它 `OPENDAN_*` 变量都不是 AgentTool 的生产契约。

### 6.1 RuntimeContext 推导规则

AgentTool CLI 启动时应先构造统一的 `RuntimeContext`：

```text
OPENDAN_AGENT_ROOT
  + OPENDAN_SESSION_ID
  + BUCKYOS_APPCLIENT_SESSION_TOKEN
  + optional OPENDAN_TRACE_ID
  -> RuntimeContext
```

推导规则：

| 上下文 | 推导来源 |
|--------|----------|
| `agent_root` | `OPENDAN_AGENT_ROOT` |
| `session_id` | `OPENDAN_SESSION_ID` |
| `trace_id` | `OPENDAN_TRACE_ID`，缺失时生成默认值 |
| `agent_id` / app id | Agent RootFS identity metadata；缺失时通过 BuckyOS runtime 或 `app_instance_config` 推导 |
| owner user id | Agent RootFS identity metadata；缺失时通过 BuckyOS runtime 或 `app_instance_config` 推导 |
| session root | `<agent_root>/sessions/<session_id>/` |
| behavior / step / wakeup | session state / last step record，不再从进程 env 读取 |
| tool bin / session tool path | BuckyOS tools 路径规则和 `agent_id + session_id` 计算 |
| memory / notebook / todo / workspace | Agent RootFS 固定目录规则 |

为保证 `OPENDAN_AGENT_ROOT` 能可靠推导身份，Agent RootFS 必须满足以下条件之一：

- 使用规范路径：`$BUCKYOS_ROOT/data/home/<owner>/.local/share/<agent_id>/`。
- 或包含身份 metadata，例如 `<agent_root>/.meta/agent_identity.json`，至少记录 `owner_user_id` 和 `agent_id`。

不允许只从任意 override 路径字符串猜测身份。例如 `OPENDAN_AGENT_ROOT=/tmp/foo` 时，必须依赖 metadata 或 runtime。

### 6.2 Agent RootFS 常用路径

Agent RootFS 布局以 [../opendan/Agent RootFS.md](../opendan/Agent%20RootFS.md) 为准。常用路径：

| 资源 | 路径 |
|------|------|
| todo DB | `<agent_root>/todo/todo.db` |
| worklog DB | `<agent_root>/worklog/worklog.db` |
| session 记录 | `<agent_root>/sessions/<session_id>/session.json` |
| workspace index | `<agent_root>/index.json` |
| session workspace 绑定 | `<agent_root>/workspaces/session_workspace_bindings.json` |
| local workspace | `<agent_root>/workspaces/<workspace_id>/` |
| local workspace worklog DB | `<local_workspace_root>/worklog/worklog.db` |
| memory 根 | `<agent_root>/memory/` |

### 6.3 Dev-only override

以下变量只保留为本地调试 / CLI dev override，不进入生产 AgentTool 契约：

| env | 用途 |
|-----|------|
| `AGENT_MEMORY_ROOT` | 覆盖 memory root |
| `AGENT_NOTEBOOK_ROOT` | 覆盖 notebook root |
| `OPENDAN_WORKFLOW_URL` / `WORKFLOW_SERVICE_URL` | 直连 workflow service |
| `OPENDAN_TASK_MANAGER_URL` / `TASK_MANAGER_URL` | 直连 task-manager |
| `OPENDAN_SESSION_TOKEN` / `SESSION_TOKEN` | dev 直连 RPC token |

新工具不要通过 `--agent-env` / `--session-id` / `--agent-id` 重复传上下文参数。命令行参数应只表达业务语义。需要上下文时从统一 `RuntimeContext` 读取。

### 6.4 环境变量契约迁移（已完成）

beta2.2 已完成迁移。因为 beta2.2 是 breaking-change 版本，无需保留向前兼容，旧上下文变量直接移除而不是留废弃分支。落地情况：

1. ✅ 新增统一 `RuntimeContext` resolver（`agent_tool::runtime_context`），输入只使用 `OPENDAN_AGENT_ROOT`、`OPENDAN_SESSION_ID`、`BUCKYOS_APPCLIENT_SESSION_TOKEN`、可选 `OPENDAN_TRACE_ID`。
2. ✅ Agent RootFS identity metadata 按三路推导：`<agent_root>/.meta/agent_identity.json` → `<agent_root>/agent.toml` 的 `[identity]` (`owner_user_id` + `agent_id`) → 规范路径 `data/home/<owner>/.local/share/<agent_id>`。任意 override 路径（如 `/tmp/foo`）无 metadata 时 `identity = None`，不静默猜身份。
3. ✅ `exec` / tmux 环境注入只把最小契约和 `PATH` 写入子进程，并过滤掉请求里夹带的 `OPENDAN_*` / dev 变量。`OPENDAN_AGENT_TOOL` 不再作为工具进程契约，改由 runtime 用 `resolve_agent_tool_cli_path()` 按路径规则算出并直接写入 generated shell hook。
4. ✅ `agent_tool_cli_dev::CliRuntimeEnv::from_process()` 改走 `RuntimeContext` resolver。旧 `OPENDAN_AGENT_ENV`、`OPENDAN_AGENT_ID`、`OPENDAN_BEHAVIOR`、`OPENDAN_STEP_IDX`、`OPENDAN_WAKEUP_ID` 已整体移除（breaking change，不保留兼容分支）；`opendan` 主进程也删掉了 `OPENDAN_AGENT_ID` / `OPENDAN_AGENT_OWNER` / `OPENDAN_AGENT_BIN` 等别名。
5. ✅ `AGENT_MEMORY_ROOT`、`AGENT_NOTEBOOK_ROOT`、`OPENDAN_WORKFLOW_URL`、`OPENDAN_TASK_MANAGER_URL`、`OPENDAN_SESSION_TOKEN` 等只在 dev-only 路径生效（cli_dev 用 `allow_dev_overrides()`、dcrontab 用 `allow_dev_env_overrides()` gate）；生产工具通过 Agent RootFS 路径和 BuckyOS runtime client 获取资源，缺 token 时显式报错。
6. ✅ `src/frame/agent_tool/create_tmux_debug_session.sh` 只注入最小契约，并自动 seed `agent.toml [identity]` 让身份可推导；旧变量已从脚本移除。
7. ✅ 测试覆盖（`runtime_context.rs`）：
   - 最小契约能构造完整 `RuntimeContext`（`minimal_contract_builds_complete_context`）。
   - `OPENDAN_AGENT_ROOT=/tmp/foo` 且无 metadata 时不会静默猜 identity（`arbitrary_root_without_metadata_has_no_identity`）。
   - 需要 RPC 的工具缺少 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 时给出明确错误（`rpc_token_missing_yields_clear_error`）。
   - identity 三路解析与空 session id 拒绝各有用例。
   - 注：旧变量已移除，不再有"兼容路径仍工作"一项；`runtime_exec_env` 的过滤行为由 `opendan::agent_bash` 的 `runtime_env_overrides_user_session_context` 测试覆盖。

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

没有 `OPENDAN_AGENT_ROOT` 时，CLI dev 可以使用当前目录作为开发态 state root；生产 AgentTool 不应依赖这个回退。

### 8.3 tmux 调试 session

仓库提供：

```bash
src/frame/agent_tool/create_tmux_debug_session.sh <agent_tool_binary> [session_name] [agent_root]
```

示例：

```bash
src/frame/agent_tool/create_tmux_debug_session.sh /opt/buckyos/bin/opendan/agent_tool od-debug /tmp/od-agent-env
```

脚本会：

- 创建临时工具软链接目录
- 给 `agent_tool`、`read_file`、`write_file`、`edit_file` 等命令建软链接
- 注入调试所需环境变量。脚本在迁移完成前可能仍包含旧 `OPENDAN_*` 变量，但新实现应以第 6 节的最小契约为准。
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
- 通过向上扫描目录、检查 `todo.db` / `worklog.db` 是否存在来猜 `agent_root`。
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
