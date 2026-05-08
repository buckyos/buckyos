# OpenDAN AgentTool 开发指南

本文面向想要为 OpenDAN 增加新工具的开发者，定义一个工具的最小实现契约。指南依据当前 `src/frame/agent_tool` 的实现编写，与 [agent_tool独立.md](../../src/frame/agent_tool/agent_tool独立.md)、[readme.md](../../src/frame/agent_tool/readme.md)、[agent_tool_result_protocol.md](../../src/frame/agent_tool/agent_tool_result_protocol.md)、[builtin_agent_tools.md](../../src/frame/agent_tool/builtin_agent_tools.md) 配套使用。

阅读完本文，你应当能：

- 知道一个 AgentTool 在 Agent 的执行环境（tmux + bash）中是如何被调用的
- 知道同一份代码如何同时承担「CLI 命令」与「进程内 function call」两种入口
- 知道 CLI stdout 的 JSON 协议是什么样、什么时候允许偏离
- 知道如何利用注入的环境变量定位 Agent Session RootFS，避免给工具加一堆冗余参数

---

## 1. 设计基线

OpenDAN 对 AgentTool 的 4 条基线，必须同时成立：

1. **完整 CLI 工具**
   工具最终以可执行文件形式出现在 Agent 的 `$PATH` 中，能被 bash、管道、子命令、脚本自由组合。**不允许**只能作为 Runtime 内嵌函数被调用、却不能在 bash 里直接跑的「半 CLI」。
2. **可作为内置工具进程内接入 function call**
   同一份工具实现要能注册到 `AgentToolManager`，让 Runtime 在 LLM `tool_calls` / `action` 路径上不经过子进程直接调用。
3. **CLI 默认输出 JSON**
   stdout 默认是 `AgentToolResult` 协议（带 `is_agent_tool=true`），方便 Runtime 的 WorkLog 压缩渲染。只有少数命令（例如 `read_file` 的 pipe 模式）才允许在明确的条件下退化成纯文本。
4. **优先用环境变量定位 Session RootFS**
   工具不要求 caller 传 `--agent-env / --session-id / --agent-id` 这一类反复重复的参数；它从 `OPENDAN_*` 环境变量自取，缺失时再回退到 `cwd`。这是把工具命令行保持干净的关键。

后续章节就是在拆解这 4 条。

---

## 2. 调用模型：Agent → tmux → 工具

OpenDAN 的执行模型是「一个 session 一个 tmux」，所有 bash 命令都通过 `exec_bash` 提交到该 session 的 pane 上跑。AgentTool CLI 化之后，工具就是这条 bash 路径上的一个普通可执行文件。

### 2.1 单二进制 + 别名分发（busybox 模式）

部署形态固定为「一个 `agent_tool` 主二进制 + 多个命令别名」： 

> TODO:这一条是对我们的内置AgentTool来说的，第三方不受该约束，第三方也能通过调整把write_file等内置命令Link到他的自定义工具上）

- 真实可执行文件只有 `agent_tool`
- 其它命令名（`read_file` / `write_file` / `edit_file` / `todo` / `get_session` / `create_workspace` / `bind_workspace` / `set_memory` / `remove_memory` / `check_task` / `cancel_task` …）都是指向同一二进制的软链接
- 进程入口靠 `argv[0]` 的 `file_name()` 分发到具体工具，参考 `cli::parse_command`：

  ```rust
  let argv0 = args.first()
      .and_then(|v| Path::new(v).file_name())
      .and_then(|v| v.to_str())
      .unwrap_or(MAIN_BINARY_NAME);
  if is_tool_name(argv0) { … }
  ```

- 直接 `agent_tool <tool_name> <args…>` 也是合法入口，便于在没有别名目录的开发环境调试

新增一个工具时，只在 `cli.rs::TOOL_NAMES` 里把工具名注册进去，dispatch 自然生效；不要再单独构造一个 bin。

### 2.2 Runtime 给 tmux 准备的环境

每个 session 在第一次跑 bash 前，`exec_bash` 会准备一份「该 session 专属」的工具目录和环境变量（参考 `agent_bash.rs::build_exec_env_vars` / `prepare_session_tool_env`）：

> 考虑到OpoenDAN Runtime会升级，这个环境的确认工作其实在每次使用session前都会进行

- 软链接目录：
  `<buckyos_root>/agent_bash_tools/<sanitized_agent_id>/<session_id>/`
  里面是该 session 当前可见的所有 AgentTool 命令别名，全部指向 `agent_tool` 主二进制。
- 注入到 tmux pane 的环境变量：

> TODO：需要简化，只需要定位到了AgentRootFS和SessionRoot,就能读取到
> CLI内部也应用多用 CWD/PWD 目录去定位目录，防止出现没有设置环境变量就无法工作的情况

  | 变量 | 含义 |
  |------|------|
  | `PATH` | 在前面拼上软链接目录，使 `read_file` 等命令在 bash 里直接可用 |
  | `OPENDAN_AGENT_BIN` | `agent_tool` 所在的 bin 目录 |
  | `OPENDAN_AGENT_TOOL` | `agent_tool` 主二进制绝对路径 |
  | `OPENDAN_SESSION_TOOL_PATH` | 当前 session 的软链接目录 |
  | `OPENDAN_AGENT_ENV` | session 所属 `agent_env_root` 绝对路径 |
  | `OPENDAN_AGENT_ID` | agent DID |
  | `OPENDAN_SESSION_ID` | session id |
  | `OPENDAN_BEHAVIOR` | 当前 behavior 名 |
  | `OPENDAN_STEP_IDX` | 当前 step 序号 |
  | `OPENDAN_WAKEUP_ID` | 当前 wakeup id |
  | `OPENDAN_TRACE_ID` | trace id |

  这些变量正好对应 `SessionRuntimeContext`（见 `agent_tool::SessionRuntimeContext`），是工具感知 Agent 上下文的唯一稳定来源。

### 2.3 一次工具调用的完整链路

```
LLM 输出 bash 一行
   │
   ▼
opendan exec_bash → tmux send-keys
   │
   ▼
tmux pane 里的 bash 解析命令
   │
   ▼
PATH 命中软链接 → 启动 agent_tool 主进程
   │
   ▼
argv[0] = "read_file"，cli::run_process 分发到 ReadFileTool
   │
   ▼
工具读取 OPENDAN_* env 构造 SessionRuntimeContext
   │
   ▼
执行业务（可能本地完成，也可能走 kRPC 回调 opendan/buckyos-api）
   │
   ▼
stdout 输出 JSON（AgentToolResult）+ exit_code
   │
   ▼
exec_bash 解析 JSON → AgentToolResult → 喂回 agent_loop
```

### 2.4 调试入口：tmux 调试 session

仓库提供 `src/frame/agent_tool/create_tmux_debug_session.sh`，本机即可拉起一个与 Runtime 等价的 tmux 调试环境：

```bash
./create_tmux_debug_session.sh /path/to/agent_tool od-debug /tmp/od-agent-env
```

它做的事情和 `exec_bash` 完全对齐：建好软链接目录、注入 `OPENDAN_*` 环境变量、`PATH` 指向软链接目录、`cd` 到当前工作目录、启动 tmux session 并 attach。新增工具时，**先在这个 tmux session 里能跑通，再让 Agent 调用**。

---

## 3. 进程内接入 function call

CLI 不是工具的唯一入口。OpenDAN Runtime 在 LLM `tool_calls` / `action` 阶段会直接在自己进程里调用工具，**不经过子进程**。这是为了：

> TODO 需要先检查是否存在用户盖link,如果存在就要对齐到调用用户工具上

- 避免短任务每次 fork/exec 的延迟
- 让 `action`（写操作）的 side-effect 与 bash 路径下完全一致
- 让 Tool Policy / `gate_tool_calls` 之类策略层只对本进程内的 tool call 生效

进程内入口的契约由 `agent_tool::AgentTool` trait 给出（见 `lib.rs`）：

```rust
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    fn support_bash(&self) -> bool;
    fn support_action(&self) -> bool;
    fn support_llm_tool_call(&self) -> bool;

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError>;

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        _shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> { … }
}
```

### 3.1 三种入口模式

每个工具按自身定位声明它支持哪几种入口：

| 模式 | 用途 | 谁来调 | 典型例子 |
|------|------|--------|----------|
| `support_bash` | 命令行风格调用 | tmux bash / `exec_bash` 解析 | `read_file`, `todo`, `get_session` |
| `support_action` | LLM 单 step 末尾的「写」动作（结构化 JSON） | Action 解析器 | `write_file`, `edit_file` |
| `support_llm_tool_call` | 标准 OpenAI 风格 function calling | Runtime LLM 路径 | `read_file`, `load_memory`, `worklog_manage` |

注册时如果三个都没有，`AgentToolManager::register_tool` 直接报 `InvalidArgs`。

### 3.2 注册到 Runtime

```rust
let mgr = AgentToolManager::new();
mgr.register_tool(ReadFileTool::new(cfg))?;
mgr.register_tool(WriteFileTool::new(cfg))?;
…
```

注册后由 manager 维护三个 namespace：

- `bash_cmds`：以工具名作为 bash 命令解析的字典（参考 `resolve_bash_registered_tool_name`）
- `llm_tools`：参与 LLM `tool_calls` 的工具集，`list_tool_specs()` 会按名字稳定排序后送进 prompt
- `all_tools` + `support_action()`：Action 路径

### 3.3 `call` 与 `exec` 的关系

- `call(ctx, args: Json)`：**唯一的核心实现**。Runtime 内部直接调用，CLI 走完参数解析后也调它。
- `exec(ctx, line, shell_cwd)`：bash 一行参数到 JSON 的桥接器；trait 提供了一个 `parse_default_bash_exec_args` 的默认实现，工具可以覆写来定制 bash 解析（例如 `read_file` 需要支持 `path range first_chunk` 这种位置参数）。

**实现工具时只写 `call`；只有当需要给 bash 模式做特化解析时才覆写 `exec`。**

### 3.4 与 OpenDAN runtime 边界

- 4 个元工具（`read_file` / `write_file` / `edit_file` / `exec_bash`）和 `create_sub_agent` 由 OpenDAN 注入策略层（参考 `src/frame/opendan/src/agent_tool.rs::AgentPolicy`），其它工具默认不再做 tool-policy 限制
- 工具实现里**不允许**回调 OpenDAN runtime 的内部组件；如果需要操作 `todo` / `worklog`，请通过 `agent_tool` 自己提供的 lib（例如 `TodoTool::new(...)`）。
  这是 [agent_tool独立.md](../../src/frame/agent_tool/agent_tool独立.md) 反复强调的边界：「OpenDAN 依赖 AgentTool」，反向不成立。
- 需要 Runtime 能力（例如读 session view、读 task 状态）时通过 trait + backend 注入：`SessionViewBackend`、`WorkspaceRuntimeBackend`、`ExternalWorkspaceRuntimeBackend` 等。Runtime 在初始化 manager 时把后端实例传进去，工具在 `call` 里只看到 trait。

---

## 4. CLI 输出协议：默认 JSON

CLI 进程的 stdout 是 Agent 拿到结果的唯一通道，所以协议必须稳定。

### 4.1 默认协议：`AgentToolResult`

stdout 默认是一段单行 JSON（结构定义见 `lib.rs::AgentToolResult` 与 `agent_tool_result_protocol.md`），最少字段：

>TODO:is_agent_tool 应该修正为agent_tool_result的版本schema

```json
{
  "is_agent_tool": true,
  "cmd_name": "read demo.txt range=1-20",
  "status": "success",
  "summary": "read 128 bytes",
  "detail": { "tool": "read_file", "content": "hello" }
}
```

约定：

- `is_agent_tool` **必须**显式 `true`，让 `exec_bash` 不至于把碰巧像 JSON 的普通 stdout 误判成 AgentTool
- `status` ∈ `success | error | pending`
- `summary` 永远填，给人读、给 prompt 压缩用
- `detail` 是结构化业务结果；普通 bash 的 `output` 字段对 builtin tool 默认不填
- exit_code：`success → 0`，`error → 非 0`（默认 `1`），`pending → 非 0`（约定的「未完成」码）；对应 `cli_exit_code_for_error` 与 `CLI_EXIT_*` 常量
- 出错时不要把 `is_agent_tool` 置 `false`：错误也是 AgentTool 的输出

### 4.2 `pending` 与长任务

工具如果立刻完成不了，应直接返回：

```json
{
  "is_agent_tool": true,
  "status": "pending",
  "summary": "PENDING (long_running, check_after=5s)",
  "task_id": "12345",
  "pending_reason": "long_running",
  "check_after": 5,
  "partial_output": "building target..."
}
```

`pending_reason` 仅取 `long_running | user_approval | wait_for_install`（历史值 `external_callback` 仅做兼容别名）。Agent loop 看到 `pending` 后会调用 `check_task <task_id>` / `cancel_task <task_id>` 跟进。

### 4.3 允许的偏离：纯文本模式

`read_file` 在「**没有 agent 环境** 且 **stdout 不是 TTY**」时会切换成纯文本模式，直接把 `detail.content` 写到 stdout（参考 `CliRuntimeEnv::use_plain_text_read_output`）。这是为了让管道、`cat` 替代场景仍然好用。

新工具如果想加这种偏离，必须满足：

- 切换条件能用环境变量 + `IsTerminal` 显式判断，不基于内容启发
- 切换后仍要保持 exit_code 语义
- 在 `builtin_agent_tools.md` 显式写清「JSON 模式 vs 纯文本模式」与切换条件

不满足上述条件就老老实实输出 JSON。

### 4.4 stderr 与 stdout 分工

- stdout 只写最终协议 JSON（或纯文本模式下的内容）
- 进度日志、警告、调试输出走 stderr；Runtime 不解析 stderr
- 工具不要混用，`run_process` 已经把 stdout / stderr 分别打印

---

## 5. 用环境变量定位 Session RootFS

减少参数数量是这套协议能保持「像普通 CLI」的关键。

### 5.1 必读环境变量

CLI 启动时必须立即从 env 构造 `SessionRuntimeContext` 与 `agent_env_root`，**不允许**通过 `--agent-id / --session-id / --agent-env` 这一类参数重复传入。当前实现见 `cli::CliRuntimeEnv::from_process`：

| env | 用途 | 缺失时回退 |
|-----|------|------------|
| `OPENDAN_AGENT_ENV` | `agent_env_root`，是文件 IO / db / RootFS 解析的根 | 当前 `cwd` |
| `OPENDAN_AGENT_ID` | agent DID | `did:opendan:cli` |
| `OPENDAN_SESSION_ID` | session id | `cli-session` |
| `OPENDAN_BEHAVIOR` | behavior 名 | `cli` |
| `OPENDAN_STEP_IDX` | step 序号 | `0` |
| `OPENDAN_WAKEUP_ID` | wakeup id | `cli-wakeup` |
| `OPENDAN_TRACE_ID` | trace id | `cli-trace` |

> 「`OPENDAN_AGENT_ENV` 缺失就回退到 `cwd`」是开发态的便利约定。请用 `CliRuntimeEnv::has_agent_env` 区分「真的运行在 Agent 环境里」还是「人在终端里手动调」，例如 `read_file` 的纯文本切换就是依赖这个标记。

### 5.2 RootFS 路径解析

`agent_env_root` 拿到之后，文件布局参考 [Agent RootFS.md](Agent RootFS.md) 第 2 节，关键路径：

| 资源 | 路径 |
|------|------|
| agent env todo DB | `<agent_env_root>/todo/todo.db` |
| agent env worklog DB | `<agent_env_root>/worklog/worklog.db` |
| local workspace 根 | `<agent_env_root>/workspaces/<workspace_id>/` |
| local workspace worklog DB | `<local_workspace_root>/worklog/worklog.db` |
| session 持久化 | `<agent_env_root>/sessions/<session_id>/session.json` |
| memory 根 | `<agent_env_root>/memory/` |

实现工具时直接拼这几条；**禁止**枚举多个候选 key 或向上回溯祖先目录猜根（[Agent RootFS.md](Agent RootFS.md) 第 5 节明确禁止）。

### 5.3 命令行参数应该被压缩到「业务参数」

环境变量负责「我是谁、我在哪、是哪一次」；命令行只剩业务语义。例：

| ❌ 反例 | ✅ 正解 |
|---------|---------|
| `read_file --agent-env=/x --session-id=s1 --path=demo.txt --range=1-20` | `read_file demo.txt 1-20` |
| `todo --session-id=s1 add "标题"` | `todo add "标题"` |
| `bind_workspace --agent-env=/x ws-001` | `bind_workspace ws-001` |

只在「环境变量没有合理的默认值，且确实需要用户指定」时才加新参数。

### 5.4 CLI 与进程内调用的等价性

进程内调用时 Runtime 直接构造 `SessionRuntimeContext` 传给 `call()`；CLI 模式下 `CliRuntimeEnv` 从 env 还原同一个 `SessionRuntimeContext`。两条路径共享同一份 `call` 实现，因此对工具实现而言「我是 CLI 还是 in-process」是透明的——这正是协议设计要保持的不变量。

---

## 6. 新增一个 AgentTool 的最小步骤

下面是开发新工具的标准动作清单，按顺序执行：

1. **在 `src/frame/agent_tool/src/` 下新增模块**（或在已有模块下加文件）。实现一个结构体 `MyTool`，给它实现 `AgentTool` trait：
   - `spec()` 返回 `ToolSpec`：`name` / `description` / `args_schema` / `output_schema` / `usage`
   - `support_bash` / `support_action` / `support_llm_tool_call` 三选一以上
   - `call(ctx, args)` 实现核心业务，返回 `AgentToolResult`
   - 必要时覆写 `exec(ctx, line, shell_cwd)` 来做 bash 风格的位置参数解析
2. **业务结果走 `AgentToolResult`**，至少填 `cmd_name` / `status` / `summary` / `detail`，并显式 `with_is_agent_tool(true)`。
3. **工具名注册**：把工具名加入 `cli::TOOL_NAMES` 数组，让 argv[0] 分发能命中；如果加入了 bash 解析的子命令，记得在 `parse_tool_command` 里走通。
4. **进程内注册**：在 OpenDAN Runtime 初始化 `AgentToolManager` 的位置 `register_tool(MyTool::new(...))`。如果工具需要 Runtime 能力，定义一个 backend trait 并把实例从 Runtime 注入进去。
5. **session 工具可见性**：把工具名加进 session 默认可见的 CLI 工具集合（`default_agent_cli_tool_names`），这样软链接才会自动建。
6. **更新文档**：把工具加进 [builtin_agent_tools.md](../../src/frame/agent_tool/builtin_agent_tools.md) 的「当前 builtin tools 一览」+ 单独一节写输入字段、`cmd_name` / `summary` / `detail` 的输出约定。
7. **测试**：
   - 单元测试覆盖 `call` 的成功 / 失败 / pending 路径
   - 至少一个 e2e 用例：先 `cargo build -p agent_tool`，再用 `create_tmux_debug_session.sh` 拉起 tmux，在 bash 里跑这个工具，确认 stdout JSON 与 exit_code 都符合协议
   - 进程内集成测试通过 `AgentToolManager.register_tool + get_tool().call(...)` 跑一遍

---

## 7. 反模式速查

下列做法和上面的设计原则冲突，PR 评审会被打回：

- 给工具加 `--agent-env` / `--session-id` / `--agent-id` 这类与环境变量重复的开关
- 在 stdout 输出非 JSON 内容（`println!` 调试日志、混合文本 + JSON、JSON 多行格式化）
- 工具实现里直接 `use opendan::...` 反向依赖 Runtime
- 用 stdout 的内容启发式猜测「这是不是 AgentTool 输出」，而不是看 `is_agent_tool=true`
- 同名工具同时构造多个独立 bin，或长期维护多个 `[[bin]]`
- 在进程内调用 CLI 子进程作为 fallback（要不就实现 in-process 路径，要不就明确说不支持）
- 把长任务用 sleep + 同步阻塞的方式实现，不走 `pending + check_task` 协议
- 通过 `todo.db` / `worklog.db` 是否存在来反推 `agent_env_root`

---

## 8. 参考资料

- [agent_tool独立.md](../../src/frame/agent_tool/agent_tool独立.md)：AgentTool 与 OpenDAN 的边界
- [readme.md](../../src/frame/agent_tool/readme.md)：CLI 化与异步执行模型的设计动机
- [agent_tool_result_protocol.md](../../src/frame/agent_tool/agent_tool_result_protocol.md)：`AgentToolResult` 字段定义
- [builtin_agent_tools.md](../../src/frame/agent_tool/builtin_agent_tools.md)：当前 builtin tools 输入 / 输出约定
- [Agent RootFS.md](Agent RootFS.md)：`agent_env_root` 的目录布局与确定性读取规则
- [Agent Session.md](Agent Session.md)：Session 状态机与输入语义
- [Agent Skill.md](Agent Skill.md)：Skill 与 Tool 的边界
- `src/frame/agent_tool/src/cli.rs`：CLI 分发实现
- `src/frame/agent_tool/src/lib.rs`：`AgentTool` trait、`AgentToolManager`、`AgentToolResult`
- `src/frame/opendan/src/agent_bash.rs`：tmux 注入软链接目录与 `OPENDAN_*` 环境变量的实现
- `src/frame/agent_tool/create_tmux_debug_session.sh`：本地 tmux 调试脚本
