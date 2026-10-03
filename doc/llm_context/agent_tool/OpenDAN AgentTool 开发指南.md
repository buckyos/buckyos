# OpenDAN AgentTool 开发指南


阅读完本文，你应当能：

- 知道一个 AgentTool 在一次 run 中处在哪一层，如何被装配和派发
- 知道同一份工具实现如何同时服务 LLM tool call、Behavior action、bash 与 CLI
- 知道 `TypedTool` 与 `AgentTool` 的边界
- 知道工具如何响应打断 / 平滑结束 / deadline，会在调用返回后继续运行的工作怎样交给 task
- 知道 CLI stdout 的 `AgentToolResult` 协议、纯文本例外和 exit code 语义
- 知道 AgentTool 的环境变量契约与 PATH 分层

---

## 1. 分层

```text
宿主：xllm CLI（agent_tool xllm）/ libopendan SessionRunner / xagent
  │  合并 .llm_context 配置，选择 runtime，注入宿主工具、task resolver、env 与 PATH 前缀
  ▼
llm_context（waist）
  │  ToolManager::call_tool(call, ToolCallCtx) -> Observation
  │  ToolPolicy：工具迭代额度、allow_deferred、finish_grace_ms
  ▼
agent_tool::runtime   AgentRuntime(native | tmux | remote_ssh).open(...) -> XllmToolManager（Sandbox）
  ├─ 内置 bash 组：shell / read_file / write_file / edit_file（+ wait_task / get_task_state / cancel_task）
  ├─ MCP 工具：在所配置的 MCP 服务执行
  └─ 宿主工具：XllmDeps.host_tools，在宿主进程执行
        │
        ▼  shell 命令中
  PATH 上的 agent_tool CLI（单二进制 + 命令别名）
```

要点：

- llm_context 只认识 `ToolManager` 和 `Observation`，不认识 `AgentToolResult`。`AgentToolResult` 到 `Observation` 的映射在派发器里完成，映射表见 [agent_tool_result_protocol.md](agent_tool_result_protocol.md#结果到-observation-的映射)。
- Runtime 是执行环境，也是 sandbox：内置工具和模板执行（`PromptExec`）都在 runtime 里执行；MCP 与宿主工具保留各自的执行位置，但都经同一个派发器。policy / grant / approval 之后在这一层加。
- `agent_tool::xllm::XllmToolManager` 是 xllm 与 libopendan 共用的派发器。
- 配置字段、RuntimeInfo、恢复检查等细节以 [xllm Rust SDK](<../xllm_rust_sdk.md>) §2、§3、§10 为准，本文只写工具开发者需要知道的部分。

### 1.1 Runtime

- `RuntimeConfig`（`.llm_context` 的 `runtime` 键）：`kind`（缺省 `native`，另有 `tmux`、`remote_ssh`）、`id`、`workdir`、`env`、`tmux`、`remote_ssh`。`RuntimeRegistry::from_config` 构造；宿主注入的 runtime 必须与有效配置一致。
- `AgentRuntime`：
  - `descriptor()`：runtime id、kind、实际 target、workdir、能力
  - `info()` → `RuntimeInfo`：id、kind、os、arch、hostname、shell、cwd、tools、current_time、timezone，供提示词模板 `{{runtime.*}}` 引用
  - `open(ctx, tools, deps)` → `(EffectiveTools, XllmToolManager)`：按工具配置装配本 run 的工具
  - `describe_interrupted(run, action)`：崩溃后为没有结果的调用生成“被打断”的说明
- 执行体：native 用本地进程（独立进程组）；tmux 每条命令一个 window；remote_ssh 用系统 ssh / sftp。内置文件工具经 `runtime::files::FileBackend`（本地或 SFTP）读写，命令经 `BashRunner` 执行，所以同一份实现可以跑在三种 runtime 上。
- `Sandbox`：`ToolManager` 加 `workdir()`、`env_check()`、`exec(BashRunRequest)`；`SandboxPromptExec` 用它执行模板里的命令。
- native / tmux 不提供 OS 隔离。`filesystem_policy: workspace`（缺省）只限制内置文件工具的路径和 `shell` 的 `cwd`，不隔离 shell 命令自身的文件访问。

### 1.2 工具来源

`.llm_context` 的 `tools` 对象决定本 run 有哪些工具：

```yaml
tools:
  enabled: true
  tools:                       # 原生 tool call；behavior 模式下的 action 用 actions 列表
    - groupname: bash          # 内置组
    - mcp: http://127.0.0.1:3000/mcp
    - name: get_session        # 宿主注册的单个工具
  bash_tools:                  # 写进 shell 命令手册（system 第 40 段）的 CLI 命令
    - name: todo
      description: 工作项管理
      command: todo
      usage: "todo add <title> | todo ls"
  shell:
    mode: auto                 # auto | wait
    wait_ms: 30000
```

- `groupname`：目前只有 `bash` 组：`shell`、`read_file`、`write_file`、`edit_file`；`shell` 处于 auto 模式时另加 `wait_task`、`get_task_state`、`cancel_task`。启用工具但不配置列表时默认使用它，显式空列表则不提供原生工具。
- `mcp`：展开为该 MCP 服务的全部工具（HTTP JSON-RPC `tools/call`），在服务端执行。
- `name`：宿主用 `XllmDeps::with_host_tool` 注册的进程内工具，在宿主进程执行。具名宿主工具缺失时拒绝运行或接管。
- `bash_tools` 不注册工具，只让模型从命令手册里知道 shell 里有哪些命令可用。

---

## 2. 一次调用的派发

```text
LLM tool call / behavior action
  ▼
llm_context：ToolManager::call_tool(call, ToolCallCtx { abort, deadline_ms, allow_deferred })
  ▼
XllmToolManager::call_tool
  ├─ 宿主有 run 记录时，登记 in-flight
  ├─ 在 task-local CURRENT_TOOL_CTX / CURRENT_TOOL_CALL 作用域内调用 AgentTool::call(&SessionRuntimeContext, args)
  ├─ 工具不可取消时，与 ToolCallCtx 的打断 / deadline 竞争
  └─ AgentToolResult / AgentToolError → Observation
```

- `SessionRuntimeContext`：`trace_id`、`agent_name`、`behavior`、`tool_call_index`、`wakeup_id`、`session_id`、`read_token_limit`。`tool_call_index` 只是本 run 内的调用序号（CLI 进程中为 0），不是 Step 或 Turn 编号；识别一次调用用 `call_id`。
- `call_id` 和 `ToolCallCtx` 不在 `call` 的参数里，经 task-local 取得：`agent_tool::runtime::CURRENT_TOOL_CALL`、`CURRENT_TOOL_CTX`。用 `try_with` 读取；不在派发器下调用（CLI、单元测试）时它们不存在，工具应按“永不取消、没有 call_id”处理。

---

## 3. 工具接口：`TypedTool`、`AgentTool`、`CallingConventions`

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
    fn cancellable(&self) -> bool { false }
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
    fn cancellable(&self) -> bool { false }

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

- 工具需要返回 `task_id`、`partial_output` 或 `pending` 等 typed pipeline 不方便表达的字段
- 工具需要非常特殊的 envelope 或错误包装
- 工具本身就是执行控制工具，例如 `shell`、`wait_task`

### 3.3 `CallingConventions`

工具通过 bitflag 声明入口：

```rust
CallingConventions::BASH
CallingConventions::ACTION
CallingConventions::LLM
CallingConventions::ALL
```

| Convention | 说明 |
|------------|------|
| `BASH` | 工具能从 bash 风格参数解析执行 |
| `ACTION` | 工具可出现在 Behavior Step 决策输出的结构化 action 里，结果记入该 Step 的 `action_results`，按 `call_id` 关联 |
| `LLM` | 工具作为标准 LLM tool call 暴露 |

`AgentToolManager` 是工具注册表，CLI 前端用它分发命令：注册时要求 `calling()` 非空，只维护一份 `tools` map，按 namespace 过滤查询（`get_tool()` 只返回 `LLM` 工具，`get_bash_cmd()` 只返回 `BASH` 工具，`get_action()` 只返回 `ACTION` 工具，`get_any_tool()` 忽略 namespace）。

### 3.4 `call`、`exec`、`parse_cli_args` 的关系

- `execute(ctx, typed_args)` 是 `TypedTool` 的核心业务实现。
- `call(ctx, Json)` 是结构化调用入口，由 `TypedToolHandle` 负责 JSON 反序列化后调用 `execute()`。
- `exec(ctx, line, shell_cwd)` 是 bash command line 入口，会先 tokenize，再调用 `parse_bash_args()`。
- `parse_cli_args(tokens, shell_cwd)` 是外部 `agent_tool` CLI 的 argv 解析入口，返回：
  - `CliInvocation::Bash { line }`：复用 `exec()` 解析
  - `CliInvocation::Json { args, content_input }`：直接走 `call()`

如果工具 CLI 只是 bash 风格的位置参数，通常不用覆写 `parse_cli_args()`；如果要支持 `--flag value`、`--content-stdin` 这类 CLI 语法，在工具自身实现里覆写，而不是在 CLI dispatcher 里新增分支。

### 3.5 `ToolHost`：backend 注入

`ToolHost` 把多个 backend 能力收敛成一个访问面：

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

`AgentToolManager::with_host()` / `set_host()` 配置 host，`register_typed_tool()` 会把当前 host 捕获进 `TypedToolHandle`。多数现有工具仍直接持有 backend `Arc`；新代码沿用所在模块的模式即可。

---

## 4. 取消、时限与长工作

术语（打断、平滑结束、deadline）的定义见 [LLM Context 设计](<../LLM Context 设计.md>) §8。“stop” 是 Session 层术语，工具层不使用。

### 4.1 `ToolCallCtx`

`llm_context::deps::ToolCallCtx` 是每次调用的上下文，工具和派发器都不自己维护取消监听或 deadline：

| 成员 | 说明 |
|------|------|
| `abort` | 与推理共用的打断 / 平滑结束信号 |
| `deadline_ms` | run 的 wallclock 截止时间（epoch ms） |
| `allow_deferred` | 宿主能否在 `pending` 上挂起 run（`ToolPolicy.allow_deferred`） |
| `cancelled().await -> CancelCause` | 打断、平滑结束或到期时返回原因 |
| `cancelled_hard().await` | 只在打断或到期时返回，平滑结束不触发 |
| `cause()` | 当前是否已有取消原因 |
| `wait_until_ms(wait_ms)` | 调用内等待的截止时间：run deadline、30 分钟上限、`wait_ms` 三者取最早 |

`CancelCause`：`Interrupted`（立即停；不能取消的工作放弃等待）、`Finishing`（能取消就取消，不能取消就继续，派发器等 `ToolPolicy.finish_grace_ms`，默认 30s，之后升级为打断）、`Deadline`。

### 4.2 声明可取消

- `cancellable()` 默认 `false`：派发器在打断或 deadline 时放弃这次调用，记为 `Cancelled { effect_unknown: true }`；平滑结束时等它做完。
- 返回 `true` 的工具自己监听 `CURRENT_TOOL_CTX`，停止工作后返回 `AgentToolError::Cancelled { message, effect_unknown }`：
  - `effect_unknown: false`：工作已经停止，或明确知道它仍在运行，`message` 说明是哪种
  - `effect_unknown: true`：中途放弃，效果未知
- 只有 `ToolCallCtx` 已有取消原因时返回 `Cancelled` 才合法；被取消不要包装成业务 `error`。
- 可取消只是工具的声明，不假设可靠取消：例如 `shell` 在 tmux 下被打断时只停止等待，命令继续运行，返回的 `message` 写明这一点。
- 基础设施故障（如远端连接断开）用 `AgentToolError::Transport { message, effect_unknown }`，它映射为派发失败，不是业务错误。

### 4.3 长工作交给 task

调用内等待受工具内上限（`llm_context::tasks::MAX_IN_TOOL_WAIT_MS`，30 分钟）和 run deadline 约束。工作可能超过这个时长时，交给 task 并立即返回 `task_id`，由模型决定是否等待：

- 进程内的工作：实现 `agent_tool::tasks::LocalTask`（`brief`、`started_at_ms`、`cancellable`、`state`、`wait`、`cancel`），用 `InProcessTaskManager::register(kind, task)` 登记，得到 `local:<kind>:<n>`；宿主经 `XllmToolManager::tasks()` 取得管理器。参考 `ShellTool` 在 auto 模式下的 `detach_into_task`。
- 进程内 task 不持久化，执行器重启后状态为 unknown。需要崩溃后能读回结果的，工具自己落盘（`shell` 用执行目录）。
- 服务接口本身返回 buckyos task id 时直接返回它，不要为了用 task-mgr 把本地工作改造成远程任务。宿主注入 `XllmDeps.buckyos_tasks` 后，LLM 的 task 工具能跟进这类 task；没有注入时，模型在 `shell` 中用 CLI `check_task` / `cancel_task` / `finish_task`（见 [builtin_agent_tools.md](builtin_agent_tools.md#6-task-工具)）。
- 返回文本里的下一步提示用 `llm_context::tasks::next_step_hint(task_id, cancellable)` 生成，与 task 工具保持一致；只有可取消的 task 才提示 `cancel_task`。
- 执行方式（等多久、是否转 task）由配置决定，不给模型加调用前要选择的参数（如 `background`）。
- `pending` 只在调用语义就是“等待”时返回，必须带 `task_id`。规则见 [agent_tool_result_protocol.md](agent_tool_result_protocol.md#长任务与-pending-结果)。

### 4.4 进程语义

- 执行器只管理正在执行的命令：native 在 Unix 上让命令在独立进程组中运行，打断或超时时整组结束。
- 命令返回后留下的进程（`&`、nohup、setsid、守护进程）在打断、超时、run 结束、恢复时都不停止、不追杀。用 nohup 起服务本身可能就是这次调用的目的。
- 恢复时不核验、不停止进程，不重放调用。没有结果的调用由 `AgentRuntime::describe_interrupted` 生成说明，交给模型判断。新增执行型工具时要想清楚：执行记录写在哪里，恢复时怎么描述。

---

## 5. CLI 与 shell 中的工具

### 5.1 单二进制 + 软链接分发

CLI 前端在 `src/frame/agent_tool_cli_dev/src/lib.rs`：

- 主二进制名是 `agent_tool`
- `TOOL_NAMES` 是 CLI 可识别的命令清单
- `parse_command()` 先检查 `argv[0]` 是否是工具名，也支持 `agent_tool <tool_name> <args...>`
- `__command_not_found__` 是 command-not-found proxy 的占位入口，未知命令返回 `127`

当前 `TOOL_NAMES`：

```text
Glob  Grep  dcrontab  read  xcall
read_file  write_file  edit_file
todo  get_session  create_workspace  bind_workspace
agent-memory / agent_memory  agent-notebook / agent_notebook  agent-skills / agent_skills
check_task  cancel_task  finish_task
read_session_history  commit_session_history_improved
BeginAttentionSignalExtraction  CompleteAttentionSignalExtraction
ListPendingAttentionSignals  MarkAttentionSignalConsumed
DiscoverEvent  DiscoverObjectObservation  DiscoverRelationship  DiscoverSkillCoverageGap
```

注意：

- `read` / `xcall` / `check_task` / `cancel_task` / `finish_task` 是 CLI pseudo-tool，不在 `AgentToolManager` 注册表中。
- `read` / `xcall` 通过 `agent-did-object-lib` 加载 `ObjectRouteConfig` 并调用 `AgentDIDObjectRuntime`。
- CLI `cancel_task` 与 LLM 工具 `cancel_task` 同名但互不冲突：前者是 shell 命令，操作 buckyos task-mgr；后者是 function call，先由 runtime 处理自己的 task。

### 5.2 PATH 分层

命令能在 `shell` 中直接调用，是因为宿主在 runtime 的 PATH 前面加了工具目录（`RuntimeOpenCtx.path_prefix`）：

| 层 | 目录 | 内容 |
|----|------|------|
| Session Bin | `<session_dir>/.runtime/bin` | tool plan 的 tombstone（遮蔽被禁用的工具）与 `agent-session` 等 Session helper，带 `.manifest.json` 校验 |
| Agent Bin | `<agent_root>/tools` | 指向 `agent_tool` 的命令别名 |
| runtime 原有 PATH | | |

- libopendan 在绑定 runtime 时准备并校验 Session Bin，再把这两层加到 PATH 前面。完整的 4 层 Bin 规则见 [Agent RootFS](<../../opendan/Agent RootFS.md>) §6，目录协议见 [Session Directory Protocol](<../../opendan/protocol/Session Directory Protocol.md>) §9。
- xllm 独立运行时，PATH 前缀由宿主经 `XllmDeps.runtime_path_prefix` 注入。
- remote_ssh 目前不支持 PATH 前缀（远端 Session helper 未部署，报 Capability 错误）。

### 5.3 `shell` 如何识别 AgentTool JSON

命令是一条不含 shell 操作符（管道、`;`、`&&`、重定向等）的简单命令，且 stdout 整体是带 `agent_tool_protocol: "1"` 的合法 AgentToolResult 时，`shell` 直接把它作为结果返回，缺少 `return_code` 时补上命令退出码。`pending` 但没有 `task_id` 的结果不转发。其它情况一律按普通命令包装，混合输出放进 `output`。

所以 CLI 工具想让结构化结果直达模型，stdout 必须只有那一行 JSON。

### 5.4 标准输出：`AgentToolResult`

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

- `agent_tool_protocol` 必须是字符串 `"1"`，反序列化只接受当前协议版本。
- `status` 取值为 `success` / `error` / `pending`。
- `title` / `summary` 是 prompt / history 压缩字段。
- `detail` 是内置工具结构化返回体；`output` 用于普通 bash 或明确需要终端文本输出的场景。
- `cmd_name` / `cmd_args` 表示 bash 风格命令形态。
- `return_code` 是进程或 bash 命令退出码。

`TypedToolHandle` 会把 typed output 序列化成 `detail`，再通过 `build_builtin_tool_result()` 生成 `cmd_name` / `cmd_args` / `summary` / 默认 `title`。

### 5.5 exit code

CLI exit code 常量在 `agent_tool` crate：

| 常量 | 值 | 用途 |
|------|----|------|
| `CLI_EXIT_SUCCESS` | `0` | 成功 |
| `CLI_EXIT_ERROR` | `1` | 执行失败 / timeout / already exists / cancelled |
| `CLI_EXIT_USAGE` | `2` | 参数错误 / tool not found |
| `CLI_EXIT_COMMAND_NOT_FOUND` | `127` | command-not-found proxy |

`cli_exit_code_for_error()` 当前映射：

- `InvalidArgs` / `NotFound` → `2`
- `AlreadyExists` / `ExecFailed` / `Timeout` / `Cancelled` 等 → `1`

### 5.6 `pending` 与 task

CLI 工具把工作交给 buckyos task-mgr 时，可以返回带 `task_id` 的 `pending`：

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

`shell` 转发它之后，派发器按 `allow_deferred` 处理：不能挂起时在调用内等 task（需要宿主注入 buckyos resolver 才能查到状态），能挂起时 run 以 `PendingTool` 挂起。如果工作不需要调用方等待，返回 `success` 加 `task_id` 更合适。CLI 里的 `check_task` / `cancel_task` / `finish_task` 通过 `TaskManagerClient` 查询、取消、结束 buckyos task。

### 5.7 纯文本例外

`read_file` 是当前唯一显式 opt-in：

- `ReadFileTool::cli_plain_text_stdout() == true`
- CLI 环境 `!has_agent_env && !stdout_is_terminal`

满足时，`agent_tool_cli_dev` 会把 `detail.content` 直接写到 stdout，而不是输出 JSON，让本地开发时的管道用法自然工作：

```bash
read_file ./demo.txt | wc -l
```

新工具如果要加入纯文本模式，必须同时满足：

- 切换条件由环境变量 / TTY 状态明确决定
- 不用内容启发式判断
- exit code 语义不变
- 在 [builtin_agent_tools.md](builtin_agent_tools.md) 写清 JSON 模式与纯文本模式的切换条件

### 5.8 DID Object 访问 CLI

Bash 的 `read` 是从标准输入读取变量的内建命令，优先于 PATH 中的同名可执行文件。对象读取在 shell 中使用 `agent_tool read ...`；LLM 的 `read` 工具调用不经过 shell，不受此同名问题影响。文件读取仍可使用 `read_file ...`。

`xcall` 是唯一的对象 action CLI 命令名，可使用 `agent_tool xcall ...`，也可以通过 `xcall` 别名直接调用。返回结果的 `tool` / `cmd_name` 均为 `xcall`。Rust API 和路由配置方法名仍为 `x_call`，本地 adapter HTTP endpoint 仍为 `/adapter/x-call`。

`read` 和 `xcall` 加载同一份 `ObjectRouteConfig`，构造 `AgentDIDObjectRuntime` 后直接返回库生成的 `AgentToolResult`，CLI 不再二次拼接结果。

配置加载顺序：

1. 命令行 `--config <route.toml>` 或 `--route-config <route.toml>`。
2. 环境变量 `AGENT_DID_OBJECT_ROUTE_CONFIG`。
3. 环境变量 `OPENDAN_AGENT_OBJECT_ROUTE_CONFIG`。
4. 内置 dev 默认配置：`file://` read 走 filesystem，`http://` / `https://` read 走 web，`https://` xcall 走 did_object，`agent://` 走 agent_runtime。

示例：

```bash
agent_tool read ./demo.txt --content-only --offset 1 --limit 20
agent_tool read uri=file:///tmp/demo.txt content_only=true
agent_tool xcall --config ./object-routes.toml obj://demo/item reserve qty=2
agent_tool xcall https://device.example.com/cam01 restart --params '{"delay_ms":1000}'
```

CLI 层只做 Gateway 第一层的本地便利转换：没有 `://` 的 `read` / `xcall` object 会按当前工作目录解析成 canonical `file://` URL。进入 `agent-did-object-lib` 后仍只接受 URL，不接受 DID URI、alias 或裸路径。

### 5.9 stdout / stderr 分工

- stdout 只写最终协议 JSON，或纯文本模式下的主内容。
- stderr 写进度、警告、调试日志。
- `agent_tool_cli_dev::main` 分别输出 stdout 和 stderr，并用 `CliRunOutput.exit_code` 退出。

---

## 6. 环境变量与 Agent RootFS

### 6.1 工具侧契约

AgentTool 进程只依赖以下变量，由 `agent_tool::runtime_context` 构造统一的 `RuntimeContext`：

| env | 必需 | 用途 | 缺失处理 |
|-----|------|------|----------|
| `OPENDAN_AGENT_ROOT` | 是 | 当前 Agent RootFS / state root | 生产环境报错；CLI dev 可回退到当前 `cwd` |
| `OPENDAN_SESSION_ID` | 是 | 当前 session id | 生产环境报错；CLI dev 可回退到 `cli-session` |
| `BUCKYOS_APPCLIENT_SESSION_TOKEN` | 是 | 访问 BuckyOS runtime / kRPC 服务 | 需要 RPC 的工具报错；纯文件 dev 工具可不要求 |
| `OPENDAN_TRACE_ID` | 否 | trace id / 日志关联 | 缺失时生成或使用默认 trace |

新工具不要通过 `--agent-env` / `--session-id` / `--agent-id` 重复传上下文参数。命令行参数只表达业务语义，需要上下文时从 `RuntimeContext` 读取。

### 6.2 宿主侧注入

shell 的环境由两部分组成：runtime 配置的 `env`，以及宿主注入的变量（xllm 为 `XllmDeps.runtime_env`）。libopendan 按 [Agent Session SDK 实现计划](<../../opendan/Agent Session SDK 实现计划.md>) §7.3 注入（`lib_opendan::runtime::session_env_vars`）：

| env | 说明 |
|-----|------|
| `OPENDAN_AGENT_DID` | Agent DID |
| `OPENDAN_SESSION_DIR` | session 目录（session 可以在 AgentRoot 之外） |
| `OPENDAN_SESSION_ID` / `OPENDAN_TRACE_ID` / `OPENDAN_RUNTIME_ID` | 标识 |
| `OPENDAN_AGENT_ROOT` | 有 Agent RootFS 时注入 |
| `OPENDAN_INPUT_QUEUE` | 有输入队列时注入 |
| `BUCKYOS_APPCLIENT_SESSION_TOKEN` | 宿主进程有该变量时透传 |

`agent_tool` CLI 目前仍按 `OPENDAN_AGENT_ROOT` + `OPENDAN_SESSION_ID` 推导 session 目录，尚未读取 `OPENDAN_SESSION_DIR` / `OPENDAN_INPUT_QUEUE`。

### 6.3 RuntimeContext 推导规则

| 上下文 | 推导来源 |
|--------|----------|
| `agent_root` | `OPENDAN_AGENT_ROOT` |
| `session_id` | `OPENDAN_SESSION_ID` |
| `trace_id` | `OPENDAN_TRACE_ID`，缺失时生成默认值 |
| `agent_id` / owner user id | `<agent_root>/.meta/agent_identity.json` → `<agent_root>/agent.toml` 的 `[identity]` → 规范路径 `$BUCKYOS_ROOT/data/home/<owner>/.local/share/<agent_id>/` |
| session root | `<agent_root>/sessions/<session_id>/` |
| behavior / step / wakeup | session state / last step record，不从进程 env 读取 |
| memory / notebook / todo / workspace | Agent RootFS 固定目录规则 |

不允许只从任意 override 路径字符串猜测身份：例如 `OPENDAN_AGENT_ROOT=/tmp/foo` 且没有 metadata 时 `identity = None`。

### 6.4 Agent RootFS 常用路径

Agent RootFS 布局以 [Agent RootFS](<../../opendan/Agent RootFS.md>) 为准。常用路径：

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

### 6.5 Dev-only override

以下变量只在 CLI dev 路径生效（如 `CliRuntimeEnv::allow_dev_overrides()`），不进入生产契约：

| env | 用途 |
|-----|------|
| `AGENT_MEMORY_ROOT` | 覆盖 memory root |
| `AGENT_NOTEBOOK_ROOT` | 覆盖 notebook root |
| `OPENDAN_WORKFLOW_URL` / `WORKFLOW_SERVICE_URL` | 直连 workflow service |
| `OPENDAN_TASK_MANAGER_URL` / `TASK_MANAGER_URL` | 直连 task-manager |
| `OPENDAN_SESSION_TOKEN` / `SESSION_TOKEN` | dev 直连 RPC token |

---

## 7. 新增工具的步骤

### 7.1 宿主工具（function call / action）

1. 在 `src/frame/agent_tool/src/` 中实现，优先 `TypedTool`：
   - 定义 `Args: Deserialize + JsonSchema`、`Output: Serialize + JsonSchema`
   - 实现 `name()` / `description()` / `calling()` / `execute()`
   - 必要时实现 `usage()` / `build_cmd_line()` / `build_summary()` / `parse_bash_args()` / `parse_cli_args()`
2. 按第 4 节决定 `cancellable()`，以及工作是否可能超过调用内等待上限、需要交给 task。
3. 需要 backend 能力时优先复用现有 backend trait 或 `ToolHost` slot。
4. 宿主用 `XllmDeps::with_host_tool` 注册，`.llm_context` 的 `tools.tools` / `tools.actions` 里以 `- name: <tool>` 引用。
5. 宿主工具在宿主进程执行，读写的是宿主所在机器。工具要跟随 runtime 操作目标机器时，改为在 shell 中调用的 CLI 命令（7.2），或者经 runtime 的 `FileBackend` / `Sandbox::exec`。

内置 `bash` 组只放执行环境的基础能力（命令、文件、task），由 `xllm::expand_tool_sources` 装配，一般不往里加新工具。

### 7.2 CLI 命令

工具需要能在 shell 中以命令形式运行时：

1. 把工具注册到 `agent_tool_cli_dev::build_cli_tool_manager()`。
2. 把命令名加入 `agent_tool_cli_dev::TOOL_NAMES`。
3. 让命令出现在 PATH 上：在 Agent Bin（`<agent_root>/tools`）放指向 `agent_tool` 的别名。
4. 需要模型从命令手册里知道它时，在 `.llm_context` 的 `tools.bash_tools` 加一条（`name` / `description` / `command` / `usage`）。
5. stdout 只输出一行 AgentToolResult JSON，确保 `shell` 能转发（5.3）。
6. 更新 `src/frame/agent_tool/create_tmux_debug_session.sh` 的调试软链接列表。
7. 直接运行 `agent_tool <tool>` 验证 stdout JSON 与 exit code。

### 7.3 MCP 工具

不需要修改 Rust 代码：在 `.llm_context` 的 `tools.tools` 加 `- mcp: <endpoint>`，派发器展开该服务的全部工具，在服务端执行。

### 7.4 通用要求

- 添加单元测试，至少覆盖 typed args 解析、成功结果、错误结果；可取消或会转 task 的工具补取消和 task 用例（参考 `runtime/tests.rs`、`llm_bash.rs` 的测试）。
- 更新 [builtin_agent_tools.md](builtin_agent_tools.md) 的工具清单与输入输出约定。

---

## 8. 调试与验证

### 8.1 构建与测试

在 `src/` 目录下优先使用仓库脚本：

```bash
uv run buckyos-build.py --skip-web
```

只验证 Rust crate 时：

```bash
cargo test -p llm_context
cargo test -p agent_tool --lib          # SSH 用例默认 ignored
cargo test -p agent_tool_cli_dev
cargo test -p libopendan -- --test-threads=1
```

### 8.2 在真实 run 中验证

用 `agent_tool xllm` 在工作目录的 `.llm_context` 配置下跑一个 run，可以检查工具装配、shell 的 auto / wait 行为和 run 目录（`runs/<run_id>/`，含执行目录 `exec/<call_id>/`）。CLI 参数与退出码见 [xllm Rust SDK](<../xllm_rust_sdk.md>) §7。

### 8.3 CLI 直接调试

构建出 `agent_tool` 后可以直接：

```bash
agent_tool read_file ./demo.txt
read_file ./demo.txt
write_file ./demo.txt --mode write --content "hello"
```

没有 `OPENDAN_AGENT_ROOT` 时，CLI dev 用当前目录作为开发态 state root；生产 AgentTool 不应依赖这个回退。

### 8.4 tmux 调试 session

```bash
src/frame/agent_tool/create_tmux_debug_session.sh <agent_tool_binary> [session_name] [agent_root]
```

脚本会创建临时工具软链接目录，给 `agent_tool`、`read_file`、`write_file`、`edit_file` 等命令建软链接，只注入第 6.1 节的最小契约（并 seed `agent.toml [identity]` 让身份可推导），前置 PATH 后 attach 到 tmux session。

---

## 9. 反模式速查

- 给模型加调用前要选择执行方式的参数（如 `background`），而不是由配置决定。
- 工作可能超过 30 分钟，却在调用里一直等；或者因为“耗时长”就返回 `pending`，而不是返回 `task_id`。
- 返回 `pending` 但不带 `task_id`。
- 工具自己维护取消监听或 deadline，而不是读 `ToolCallCtx`。
- 声明 `cancellable() == true` 却不响应取消；或在没有取消原因时返回 `Cancelled`；或把取消包装成业务 `error`。
- 执行器退出、打断或恢复时追杀命令留下的后台进程，或在恢复时核验进程状态。
- 内置组工具绕过 runtime 直接读写本地文件系统，导致在 remote_ssh 上读写错机器。
- 给工具新增 `--agent-env` / `--session-id` / `--agent-id` 这类重复上下文参数。
- 在 stdout 混写调试文本和 JSON，导致 `shell` 无法转发结果。
- 在 `agent_tool_cli_dev` 的 dispatcher 中堆工具专用解析分支，而不是让工具覆写 `parse_cli_args()`。
- 只注册进 `AgentToolManager`，却以为它会自动变成 PATH 里的命令，或自动出现在 run 的工具列表里。
- 通过向上扫描目录、检查 `todo.db` / `worklog.db` 是否存在来猜 `agent_root`。
- 输出不带 `agent_tool_protocol: "1"` 的自有工具 JSON。
- 新增工具后只改代码，不更新 [builtin_agent_tools.md](builtin_agent_tools.md)。

---

## 10. 参考资料

- [readme.md](readme.md)：AgentTool CLI 化与长任务模型的设计背景
- [agent_tool_result_protocol.md](agent_tool_result_protocol.md)：`AgentToolResult` 字段定义、渲染规则、到 `Observation` 的映射
- [builtin_agent_tools.md](builtin_agent_tools.md)：当前 builtin tools 输入 / 输出约定
- [LLM Context 设计](<../LLM Context 设计.md>)：waist、ToolManager、打断与平滑结束
- [xllm Rust SDK](<../xllm_rust_sdk.md>)：`.llm_context` 配置、shell 配置、共享 AgentRuntime
- [Agent RootFS](<../../opendan/Agent RootFS.md>)：Agent root 目录布局与 4 层 Bin
- [Agent Session SDK 实现计划](<../../opendan/Agent Session SDK 实现计划.md>) §7：libopendan 的 runtime 绑定与环境契约
- `src/frame/agent_tool/src/lib.rs`：`AgentToolResult`、`AgentTool`、`AgentToolManager`、`AgentToolError`
- `src/frame/agent_tool/src/tool.rs`：`TypedTool`、`CallingConventions`、`ToolHost`
- `src/frame/agent_tool/src/runtime/`：`RuntimeConfig`、`AgentRuntime`、`Sandbox`、native / tmux / SSH 与文件后端、`CURRENT_TOOL_CTX`
- `src/frame/agent_tool/src/llm_bash.rs`：`shell`、`BashRunner`、执行目录
- `src/frame/agent_tool/src/tasks.rs`：`InProcessTaskManager`、`CompositeTaskResolver`、task 工具
- `src/frame/agent_tool/src/xllm.rs`：`ToolsConfig`、`XllmToolManager`、工具装配
- `src/frame/llm_context/src/deps.rs`、`tasks.rs`：`ToolCallCtx`、`CancelCause`、`RunningTaskResolver`
- `src/frame/agent_tool_cli_dev/src/lib.rs`：CLI 分发、`CliRuntimeEnv`、pseudo-tool
- `src/frame/lib_opendan/src/runtime/`：Session 的 runtime 绑定、Session Bin、环境注入
