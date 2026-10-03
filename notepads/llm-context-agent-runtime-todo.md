# AgentRuntime 下移到 llm_context 层 TODO

日期：2026-10-02（按本轮 review 收敛首版范围）

状态：首版已实施。native、tmux、remote_ssh 已共享配置、构造、工具入口、环境信息与恢复检查；xAgent §5、SDK、PRD、模板文档及协议 schema/fixtures 已回写。policy/grant/approval 和其它执行体仍为后续规划。

## 1. 目标与边界

把 Agent Runtime 从 libopendan 的 Session 层下移到 llm_context 层（`llm_context` crate + `agent_tool` / xllm 这一层），首版完成三个需求：

1. **用简单配置定义执行体，并由统一入口构造。** `.llm_context` 增加 `runtime`，不配置就是 native。xllm、libopendan、xagent、以后的 OpenDAN 复用同一份配置与构造入口。首版实现 `native`、`tmux`、`remote_ssh`。
2. **统一工具的派发与执行位置。** 所有工具调用先进入 Runtime；内置 exec 与文件工具使用同一执行体，模板引擎的 `__EXEC__` 也在该执行体执行。MCP 与宿主进程内工具保留各自的实际执行位置，由 Runtime 管理注册与派发，不能因为统一入口就宣称它们都在同一机器上。
3. **提供实际执行环境的信息。** `RuntimeInfo` 描述执行体的 OS、工作目录、时间等，供提示词模板引用，不能再默认来自 Runner 所在机器。

Runtime 长期包含**执行体与 policy** 两部分：执行体回答“在哪里、通过什么机制执行”，policy 回答“允许做什么、如何隔离和限制”。首版先完成执行体；统一入口为后续 sandbox policy 留出接入位置，`native` / `tmux` 本身不提供 OS 级隔离。

本期边界：

- 首版配置去掉 `fs_view`、`path_layers`、`limits`。文件工具的现有 `tools.filesystem_policy` 暂时保留；exec 超时与输出上限仍归 `LlmBashConfig` / 执行请求，模板执行超时仍归模板引擎，均不新增 runtime 配置项。
- `container`、`container_host`、BuckyOS Node 远程执行、`http_proxy_runtime` 只列入规划，暂不冻结完整配置、设计协议或实现。
- policy、`ActionGuard` / DenyList、`ToolSpec.effect` 的迁移、临时授权（RuntimeGrant）、人工审批流留在后续阶段，不作为本期前置任务。既有 Session 协议门槛、执行跟踪、恢复检查与工具路径约束继续生效。
- 遵循 beta 2.2 规则：不做旧接口兼容；持久格式发生变化时显式升版并拒绝旧格式，相关 schema / fixtures 在实施时同步。
- 下面 §2 保留实施前基线，当前实现与验证见 §10。

相关：[xllm Rust SDK 参考](<../doc/llm_context/xllm_rust_sdk.md>)、[Agent Session SDK 实现计划 §7](<../doc/opendan/Agent Session SDK 实现计划.md>)、[NewOpenDANRuntime §2 四层 bin](<../doc/opendan/NewOpenDANRuntime.md>)、[prompt_render_engine](<../doc/llm_context/prompt_render_engine.md>)。

## 2. 实施前基线（历史记录）

| 位置 | 当前行为 | 缺口 |
|---|---|---|
| `llm_context/src/prompt_engine.rs` `__EXEC(...)__` | `allow_exec` 默认关，开启后在本进程用 `Command::new("sh")` 执行，默认 10s 超时 | 执行位置与工具 exec 不一致；未接入工具执行跟踪 |
| `agent_tool/src/xllm.rs` `TemplateEnv::for_new_run` | `runtime.current_time / timezone / os / cwd` 来自 Runner；`build_contexts_system_text` 把未被模板引用的项补进 system 第 20 段 | 无法表示远端实际环境 |
| `agent_tool/src/llm_bash.rs` | `ExecBashTool`、`LlmBashConfig`、`BashRunner` / `LocalProcessBashRunner`；`exec_tracking::TrackedBashRunner` 做启动握手与执行标识 | 只有 exec 可换执行器；`BashTargetSpec::Raw` 未实现 |
| `agent_tool/src/xllm.rs` `build_toolset` / `XllmToolManager` | 文件工具用 `FileToolConfig::new(workdir)` 读写本地 fs；exec 可注入 `XllmDeps.bash_runner` | 文件工具不能随执行体切换 |
| `.llm_context`（`parse_llm_context_file`） | 无 `runtime` 键；`tools.filesystem_policy` 支持 workspace / unrestricted，控制内置文件工具路径与 exec.cwd | 该策略不隔离 shell 命令自身的文件访问，也不决定执行位置 |
| `lib_opendan/src/runtime/`、`src/protocol/runtime.rs` | 已有 `AgentRuntime`、`NativeRuntime`、`TmuxRuntime`、`bin_overlay`、`binding.json`；descriptor 包含 `fs_view` / `path_layers` | 位于 Session 层，xllm 无法独立使用；Runtime 只管 exec |
| `lib_opendan/src/runtime/tmux.rs` | 创建或复用按 Session ID 生成的 `od_<sid>` 会话，命令经脚本注入 pane；记录执行标识并复用本机恢复逻辑 | 尚无面向 `.llm_context` 的指定 session / socket 配置 |
| `opendan/src/agent_bash.rs` | 另有 `TmuxBashRunner`、四层 PATH overlay、`session_exec_base_env` | 本期不改旧 opendan；优先复用 libopendan 已移植实现 |

## 3. Runtime 类型与实际用途

`kind` 按具体执行机制命名。`remote` 是远程执行这一类能力的统称，首版使用明确的 `remote_ssh`，不提供含义不清的 `kind: remote` / `host`。

| kind | 执行位置与机制 | 实际用途 | 本期范围 |
|---|---|---|---|
| `native` | Runner 当前所在 OS / 容器内直接启动进程，使用当前用户权限；文件工具访问同一文件系统 | 本地开发、默认执行；Runner 在容器里时命令也在该容器里 | 实现，默认值 |
| `tmux` | Runner 所在环境的指定 tmux session 内执行命令；文件工具使用与 native 相同的文件系统 | 保留可观察的终端会话，人工 attach 调试，创建或继承既有 session | 迁移已有实现并补齐指定 session 配置 |
| `remote_ssh` | 经 SSH 到指定机器 / SSH alias 执行命令与文件操作 | 使用开发机、服务器或另一台设备上的环境 | 细化并实现；首版远端目标为 Linux + bash |
| `container` | 从 Runner 所在机器通过容器引擎进入本机一个指定的已有容器，例如 `docker exec` | 使用已有容器的工具链、依赖与文件系统；不负责创建新容器 | 规划，暂不实现 |
| `container_host` | Runner / llm_context 在容器内，命令与文件操作在容器外的宿主机执行 | OpenDAN 在容器内运行时使用宿主环境；尤其包括 Windows 桌面上的宿主命令 | 规划；后续定义宿主连接机制与路径语义 |
| `remote_node`（规划名） | 通过 BuckyOS 规划中的 Node 命令执行能力，在指定 Node 上执行 | 使用 Zone 中任意 Node 的环境；属于 remote 家族，与 SSH 是不同接入方式 | 规划，等待 Node 能力与认证接口明确 |
| `http_proxy_runtime` | 将执行与文件操作转发给实现约定 HTTP 协议的应用扩展 | 应用按协议提供自定义执行体即可接入 | 规划，暂不设计协议或实现 |

区分原则：`container` 是“进入容器”，`container_host` 是“从容器到宿主”。Windows 下需明确目标是用户的 Windows 主机还是容器平台的 Linux VM，不能仅根据“容器外”推断实际 OS。`container_host` 后续可复用 SSH 或宿主服务作为传输机制，但用途与目标定义独立于传输。

BuckyOS Node 的命令执行接口不能直接假定具有完整文件工具能力；接入时需补齐目标侧文件访问，或明确报告缺失能力，不能将文件工具退回 Runner 本地。

## 4. P0：简单配置与统一构造

- [x] **位置**：新增 `agent_tool::runtime`。已有 `BashRunner`、`exec_tracking`、文件工具都在 `agent_tool`，无需新增 crate / 依赖方向。迁移 libopendan 的 native / tmux 执行实现；`binding.json` 与 Session 专属的 `bin_overlay / prepare_session_bin / verify_session_env` 留在 libopendan。
- [x] **配置**：`.llm_context` 新增根键 `runtime`（JSON 形式同 schema，libopendan 的 `prompt.llm_context` 自动获得）。公共字段只保留 `kind`、可选 `id`、`workdir`、`env`，加当前 kind 对应的连接配置。

  最小配置是 `runtime: { kind: native }`，也可以完全省略。需要指定本地工作目录或环境时：

  ```yaml
  runtime:
    kind: native
    workdir: .
    env: { LANG: en_US.UTF-8 }
  ```

  指定 tmux session：

  ```yaml
  runtime:
    kind: tmux
    tmux:
      session: agent-dev
      mode: create_or_attach  # create | attach | create_or_attach；默认 create_or_attach
  ```

  SSH 远程执行：

  ```yaml
  runtime:
    kind: remote_ssh
    workdir: /home/dev/workspace
    remote_ssh:
      host: dev-box          # 主机名、地址或本机 OpenSSH config 中的 alias
  ```

  字段规则：

  - `id` 可省，按 kind 与规范化目标生成稳定身份；tmux 目标包含 socket / session，SSH 目标包含有效主机 / 用户 / 端口。显式 `id` 不能代替实际目标核验。
  - native / tmux 的 `workdir` 缺省为宿主传入的工作目录；显式相对路径按声明字段的配置文件目录解析。kind 为 `remote_ssh` 时，`runtime.workdir` 首版要求显式给出远端绝对路径，不能在 Runner 上 canonicalize 或拼接本地配置目录。
  - `env` 是注入执行体的环境覆盖，命令的单次 env 再覆盖它；远端不默认继承 Runner 的环境。PATH 默认来自执行体，Session 所需 helper 路径由宿主内部装配，首版不暴露 `path_layers`。
  - `tmux.session` 首版必填；`socket` 可选，表示本机 tmux socket 路径，相对路径按本地配置来源解析，省略时使用默认 socket。`create` 在会话已存在时报错，`attach` 在不存在时报错，`create_or_attach` 创建或复用。结束 run 不销毁继承的 session。
  - `remote_ssh.host` 必填；`user`、`port`、`identity_file` 可选，未指定时遵循本机 OpenSSH 配置。`identity_file` 是 Runner 上的路径，按本地配置来源解析；不把私钥内容或密码写入 `.llm_context` / run.json。
  - 同 kind 按现有配置优先级逐字段覆盖，`env` 按键覆盖，保留路径字段的来源目录；kind 变化时整段重置。与 kind 无关的连接块、未知字段 / kind 返回 `Config` 错误。规划中的 kind 被选择时返回明确的 `Capability` 错误，不静默退回 native，也不要求先支持其完整连接配置。

- [x] **构造**：提供 `RuntimeRegistry::from_config(&RuntimeConfig) -> Result<Arc<dyn AgentRuntime>>`。连接探测与执行环境准备放在打开阶段；缺少 tmux / SSH 客户端、目标不可达或必要能力不足，在首次模型请求前明确失败。
- [x] **身份与持久化**：`RuntimeDescriptor` 移到 `agent_tool::runtime`，保留 runtime_id / kind / 实际目标 / 必要能力信息，去掉首版的 `fs_view` / `path_layers` 字段。规范化配置与打开后的目标身份写入 run.json；resume 使用保存值，不重读 `.llm_context`。核对 runtime_id、kind、实际目标与 workdir，不仅比较 kind；SSH alias 重定向也不能让旧 run 换到另一执行体。
- [x] **现有约束**：保留 `tools.filesystem_policy` 的 workspace / unrestricted 行为，workspace 约束针对执行体内的 workdir 生效。它不是完整沙箱 policy，也不由已移除的 `fs_view` 推导；本期不改成新的白名单配置体系。

## 5. P0：统一工具入口与 SSH 执行体

- [x] 接口职责如下（Rust 示意，具体类型以实施时为准）：

  ```rust
  #[async_trait]
  pub trait AgentRuntime: Send + Sync {
      fn descriptor(&self) -> &RuntimeDescriptor;
      async fn info(&self) -> Result<RuntimeInfo>;
      async fn open(
          &self,
          ctx: &RuntimeOpenCtx,
          tools: &ToolsConfig,
          host_tools: Vec<Arc<dyn AgentTool>>,
      ) -> Result<(EffectiveTools, Arc<dyn Sandbox>)>;
      async fn reconcile_execution(&self, rec: &ExecutionRecord) -> Result<()>;
  }

  #[async_trait]
  pub trait Sandbox: llm_context::deps::ToolManager {
      fn workdir(&self) -> &str;  // 执行体内的路径，不能按 Runner 的 OS 路径语义解释
      fn env_check(&self) -> serde_json::Value;
      async fn exec(&self, req: BashRunRequest, ctx: &SessionRuntimeContext)
          -> Result<BashRunOutput>;
  }
  ```

  `RuntimeOpenCtx` 不依赖 libopendan 的 SessionDir / Binding；只接收通用 run 身份、执行记录接口与宿主装配的环境。`open` 接收未展开的工具配置，复用 `build_toolset` 的解析逻辑，返回有效工具集与派发器，避免要求 Runtime 在工具尚未展开时接收 `EffectiveTools`。

- [x] `Sandbox::call_tool`：现有参数 / 路径约束 → 必要的启动握手与记录 → 执行 → 归一 Observation。工具自身失败返回 `Observation::Error`；传输或派发设施失败使用既有 `ToolDispatchError`，可能已开始执行时设置 `effect_unknown = true`，不自动重试有副作用的调用。后续 policy 接在执行前，本期不要求 guard / approval 实现。
- [x] **Native / tmux**：native 复用 `TrackedBashRunner` 的本机启动握手与恢复逻辑；tmux 复用已有脚本、输出采集与执行标识机制。继承 session 时选择专用执行 pane，不向用户正在使用的 pane 注入命令；同一执行 pane 内串行派发。每次命令应用有效 cwd / env，文件工具使用同一 cwd 所在的本机文件系统。
- [x] **文件工具后端**：从 `file_tools.rs` 拆出可替换的 I/O 后端；read / write / edit 的参数、匹配、diff 与审计行为继续复用。路径解析、路径约束与必要的真实路径核验在目标文件系统上完成，remote 不能调用 Runner 的本地 fs 校验后就声称远端路径已通过。
- [x] **SSH 传输**：首版使用系统 OpenSSH `ssh` / `sftp` CLI 与现有进程管理依赖，不引入 SSH crate 或常驻 runtime daemon。`ssh` 运行远端 bash，`sftp` 提供文件传输；首版以 Linux、bash 与 SFTP 为目标能力要求，以复用现有 Linux 执行跟踪思路，其它目标 OS 后续扩展。复用本机 ssh_config、agent / key 与 known_hosts，以非交互方式连接，连接或认证失败明确报告。
- [x] **SSH 命令与文件语义**：命令在远端 workdir 执行，返回 exit_code / stdout / stderr / timeout / truncation；文件工具经远端后端读写同一目录。命令脚本与文件内容通过数据通道传送，不能将文件内容拼进 shell 命令；路径与 env 使用明确的转义规则。远端写入采用目标侧临时文件与替换，不能因 SSH 失败把写入改为本地文件。
- [x] ~~**SSH 执行跟踪与恢复**：复用 `ExecutionRecord` / registrar 契约……无法证明执行已停止时返回 RecoveryBlocked。~~ **2026-10-02 被 [长命令 TODO](./llm-context-long-tool-todo.md) §3.2 推翻**：不再持久化进程身份、不核验、不停止；远端命令由包装脚本后台启动，把 `pid`、`stdout`、`stderr`、`exit` 写进远端执行目录 `/tmp/llm-runtime-<uid>/<run_id>/<call_id>`，后续 SSH 会话轮询；超时 / 取消用记录的 pid `kill`；恢复只读执行目录（`AgentRuntime::describe_interrupted`），连不上时如实说明，不返回 RecoveryBlocked。没有持久结果的调用记为“被打断、结果未知”，不重放。
- [x] **xllm 装配**：`prepare / prepare_hosted` 改为 `runtime.open(..)`；`XllmDeps.bash_runner` 改由 Runtime 提供，`XllmDeps.runtime` 支持宿主 / 测试注入，仍核对有效配置与目标。resume 由对应 runtime 完成恢复，替换“只接受 native”的硬编码。
- [x] **控制侧与执行侧目录**：配置来源、run.json、快照与日志仍属于 Runner 的控制侧；`runtime.workdir` 是工具执行侧路径，二者显式区分。run 锁与 native / tmux 的工作目录锁保留；SSH 不对远端路径取本地 flock。首版不提供跨 Runner 的远端工作目录锁，同一远端目录的并发使用需由宿主协调。
- [x] **MCP / 宿主工具**：全部经 Runtime 派发，MCP 仍由所配置的服务执行，层 ③ 工具仍在宿主进程执行；其位置与依赖不能伪装成远端能力。宿主工具依赖记入 run.json，xllm 缺少它们时按既有 app_tools 规则拒绝接手。

## 6. P0：环境信息与模板引擎

- [x] `RuntimeInfo` 采用一致的核心字段，来自打开后的执行体：

  | 模板键 | 含义 | 新鲜度 |
  |---|---|---|
  | `runtime.id`、`runtime.kind` | 实际 runtime 身份与执行机制 | 稳定 |
  | `runtime.os`、`runtime.arch`、`runtime.hostname`、`runtime.shell` | 命令实际执行处的系统信息；SSH 时在远端探测 | 稳定，恢复时核验必要身份 |
  | `runtime.cwd` | 执行体内的有效工作目录，对应 `runtime.workdir` 配置 | 稳定 |
  | `runtime.tools` | 经配置解析并核验的可用工具 / 必需命令摘要，列表有上限 | 当前打开的执行环境内稳定 |
  | `runtime.current_time`、`runtime.timezone` | 执行处的时间与时区；不可用时明确表示未知 | 新鲜；libopendan 放输入批次 S-20，xllm 一次性任务可放 system |

  tmux session、SSH 连接目标等类型专属信息放在可选的目标摘要中。不增加 `runtime.fs_view`、`runtime.path_layers` 或 `runtime.limits.*`。

- [x] `{{runtime.*}}` 与 `build_contexts_system_text` 从 `RuntimeInfo` 取值；`TemplateEnv::for_new_run` 不再自己采集 Runner 环境。保留现有 `runtime.cwd` 模板键，避免配置 workdir 与模板 cwd 的含义混淆。
- [x] `__ENV($runtime.*)__` 经宿主 `ValueLoader` 获取同一份信息。`__EXEC(...)__` 经注入的执行接口调用同一 Sandbox，保留 `allow_exec`（默认关）与模板自己的 `exec_timeout`，接入执行跟踪并记录命令与耗时。
- [x] **依赖方向**：`llm_context` 定义小型 `PromptExec` trait 与请求 / 输出类型，模板引擎接收 `Option<Arc<dyn PromptExec>>`；`agent_tool` 用 Sandbox 实现适配。不能在 `llm_context::EngineConfig` 中直接引用 `agent_tool::Sandbox` / `BashRunRequest`，否则与既有 `agent_tool -> llm_context` 依赖形成环。启用 `__EXEC__` 而未注入执行器时明确报错。
- [x] 提示词素材与 `__INCLUDE__` 的路径仍按现有模板来源与 include_roots 解析；它们属于控制侧素材，不因远端 workdir 改变而自动变成远端文件。
- [x] 实施时同步 `doc/llm_context/prompt_render_engine.md`、`Render_Prompt_Template_Variables.md`、xllm Rust SDK 与 PRD §4.9，补充核心键、控制侧 / 执行侧路径与新鲜量规则。

## 7. 后续规划：Runtime policy

policy 参考 Docker 的环境与资源配置，也参考 NVIDIA OpenShell 对执行边界、策略决策与强制机制的分工。这里只补充后续设计依据，不在首版添加 `policy` schema、依赖或要求实现。

### 7.1 设计维度

| 维度 | 后续需要回答的问题 |
|---|---|
| 文件系统 | 允许哪些读写根、哪些挂载 / 目录只读，如何处理符号链接与路径映射 |
| 身份与权限 | 用哪个用户 / UID / GID 执行，允许哪些 capability / 特权，是否清理继承的环境 |
| 网络与请求 | 哪个进程可访问哪些主机 / 端口，是否进一步限制 HTTP / RPC 方法与资源路径，如何覆盖 bash 子进程 |
| 凭据 | 谁持有真实凭据，允许向哪些目标与请求注入，如何与网络放行和 BuckyOS 身份 / RBAC 分开核验 |
| 资源与生命周期 | CPU / 内存 / 进程数量、临时目录与清理如何约束；exec 超时 / 输出预算继续与工具级配置区分 |
| 工具与授权 | 工具可见性、Do 前检查、ActionGuard / DenyList、临时 grant 与审批如何组合 |
| 策略生效与审计 | 哪些项在启动时固定、哪些可动态更新；如何记录有效 revision、强制机制、拒绝原因与撤权结果 |

不同执行体的强制能力不同：native / tmux 上的工具路径检查不能等同于容器隔离；SSH / HTTP 的策略必须由目标侧执行或提供可验证的能力，不能仅凭配置宣称已限制远端。后续设计有效 policy 与能力不足时报错的规则，再决定配置字段与 guard 接口。

`ToolSpec.effect` 统一与 libopendan `classify_effect` 迁移可在该阶段推进；本期继续使用现有分类记录 inflight。grant 与人工审批仍在 xAgent 阶段衔接，见 xAgent.md §5.4。

### 7.2 OpenShell 参考（2026-10-02 调研）

以下依据官方架构、latest / dev policy 文档与 main 的隔离接口。右列是对本项目的设计建议；实施时需重新核对选定版本的实际能力。

| OpenShell 的处理方式与依据 | 对 AgentRuntime 的启发 |
|---|---|
| Compute driver 负责部署、通信通道与外围网络边界；可信 supervisor 在 workload 边界外作出 policy 决策；隔离 backend 提供共同的执行接口。[Architecture](https://docs.nvidia.com/openshell/about/architecture)、[Isolation Backends](https://docs.nvidia.com/openshell/extensibility/isolation-backends) | 执行体、policy 决策、强制机制分别建模；native / tmux / SSH / Node 的接入差异不应变成各自一套授权语义。 |
| Linux workload 使用 Landlock 限制文件访问、降权与 seccomp 等进程机制，并由受控通道与代理处理出站连接。[Architecture](https://docs.nvidia.com/openshell/about/architecture) | ActionGuard 管工具调用授权；执行体约束覆盖命令及其子进程的实际 I/O。允许 `exec` 之后，文件和网络限制仍须生效；仅检查 `write_file` 参数覆盖不了 shell 中的写入。 |
| 网络规则组合目标、端口、调用程序与可选请求检查；REST 可限制 method / path，MCP 可限制 method / tool，但当前 MCP 规则不匹配工具参数。请求规则有独立的 audit / enforce 模式。[Network Rules](https://docs.nvidia.com/openshell/dev/how-it-works/policies/network-rules) | 网络 policy 可细化到“允许这个程序调用这个 API”；业务资源、工具参数与 BuckyOS 权限仍需上层授权。观察到违规与真正阻断必须区分，不能把 audit 当成隔离。 |
| Agent 环境中的凭据值使用占位符，由代理在允许的 HTTP 请求中解析；网络放行与凭据绑定是两次独立授权。[Providers](https://docs.nvidia.com/openshell/how-it-works/providers/overview) | 凭据应成为独立维度；能访问某服务不意味着可向其发送任意 token。后续评估目标绑定的凭据代理 / 服务授权，避免默认把宿主 token 放进任意 bash 的环境。 |
| 文件系统与进程设置在启动时固定；网络规则可更新，更新后旧连接关闭。管理端接受 revision 与目标报告 Loaded 是不同状态。[Manage Policies](https://docs.nvidia.com/openshell/dev/how-it-works/policies/manage-policies) | grant 生效、撤销和 resume 需要目标确认的有效策略；明确哪些修改要求重建执行环境，哪些可热更新，以及旧连接 / 已运行进程如何处理。 |
| 隔离接口按绑定、确认边界、启动的顺序推进；确认结构核验文件约束、出站拦截、请求归属与权限基线等属性。[contract.rs](https://github.com/NVIDIA/OpenShell/blob/main/crates/openshell-isolation-interface/src/contract.rs) | 配置要求、backend 支持能力与实际生效结果是三件事；后续在执行前核验所需约束，并记录目标身份、有效策略与执行机制。 |
| Agent 可提议窄范围网络规则，由可信侧审查；prover 检查其模型所覆盖的授权属性，对不支持的规则返回 unsupported。模型内通过不等于运行环境已强制执行，也不等于任务本身安全。[Policy Advisor](https://docs.nvidia.com/openshell/dev/how-it-works/policies/advisor)、[Policy Prover](https://docs.nvidia.com/openshell/dev/how-it-works/policies/prover) | Agent 提议权限与批准权限分离。先做结构校验、授权差异与生效核验；形式化验证作为更后的增强项，不阻塞首版或初步 policy 实现。 |

### 7.3 后续落地顺序与边界

1. **先区分两种 policy 职责。** 工具授权处理调用、参数、业务资源、BuckyOS RBAC 与审批；执行体约束处理文件、进程、网络与凭据。二者协作，进程内 ActionGuard 不能独自保证任意 shell 程序的 I/O 范围。
2. **定义要求、能力与生效结果。** 后续 policy 应明确所需强制能力；backend 无法提供时拒绝对应的受约束执行，不能静默降级。现有 tmux pane 与普通 SSH 连接不能仅靠配置获得完整隔离；SSH 的命令与 SFTP 文件通道也需纳入同一授权边界，不能只约束远端 bash。
3. **明确组合与更新语义。** 记录声明策略、provider / grant 等贡献与目标实际采用的有效策略。最大授权边界与 Session / grant 的组合需单独定义，不能套用 runtime 连接字段的覆盖规则；OpenShell 的 global policy 是替换语义，本项目不直接照搬为权限上限。[Sandbox Policies](https://docs.nvidia.com/openshell/dev/how-it-works/policies/overview) 然后设计 revision / 生效确认 / 撤权 / 审计，区分启动固定项和动态项。
4. **再评估实现复用。** OpenShell 可作为后续受约束执行 backend 的候选，也可仅借鉴 policy 与边界契约。其当前 workload 隔离依赖 Linux 机制；Windows 平台支持经 WSL 2，不能据此推断它已支持本项目的 Windows `container_host` 宿主命令隔离。[Support Matrix](https://docs.nvidia.com/openshell/about/support-matrix) 是否接入、接哪一层，待该阶段验证；本期仍按 native / tmux / remote_ssh 推进。

## 8. P1：libopendan、CLI 与后续类型接入

- [x] libopendan 的 `RunnerDeps.runtime` 换成 `agent_tool::runtime::AgentRuntime`；`drive` 打开 Sandbox，`SessionToolManager` 保留 lease / 提交门槛 / inflight / touching 等 Session 协议纪律，内部调用 Sandbox。Session bin 与 helper 仍由 Session 装配，通过通用环境接口注入执行体，不重新增加 `runtime.path_layers` 配置。
- [x] `session_config.runtime.requirement.runtime_id` 是 Session 的绑定要求，`.llm_context.runtime` 是构造配置；`binding.json` 核对打开后的 runtime 身份、目标与 workdir。不一致报 RuntimeMismatch，拒绝发生在推理或工具执行前。
- [x] libopendan 的远端接入需明确 Session helper 的目标侧可用性。不能把 Runner 上的 `.runtime/bin` 路径直接加到远端 PATH；所需 helper 未部署或无法访问 Session 服务时明确报能力不足。SSH runtime 在 xllm 中独立可用，不等待这一步。
- [x] CLI 可增加 `xllm --runtime <kind>` 作为配置覆盖，仍须提供该 kind 必需的连接字段；`id` 表示身份，不作为未定义的 profile 查找入口。`xllm status` 显示实际 runtime、目标与 env_check。
- [x] 实施完成后回写 xAgent.md §5 的类型、职责与阶段安排；§3.1 G5（deadline / cancel）、G7（文件工具绑本地 fs）按本文处理。
- [x] **规划已明确（本期不实现）**：后续依次评估已有容器执行、container_host、BuckyOS Node 与 HTTP 应用扩展。远端 wire protocol 只传可序列化的工具请求 / 结果与调用上下文；本地 trait 的 registrar / host_tools 等进程内对象不属于远端协议。本期无需提前设计统一 daemon / HTTP 协议。

## 9. 实施顺序与验收

以下验证用于实现阶段验收，命令在 `src/` 目录执行。

1. `agent_tool::runtime`：简单 config / registry / descriptor / RuntimeInfo / AgentRuntime / Sandbox；迁移 native / tmux，拆出文件 I/O 后端，保留 Session bin 职责。验证 `cargo test -p agent_tool --lib runtime`。
2. xllm 接入 `.llm_context.runtime` 与统一装配；run.json 保存有效配置与目标；模板信息改源，`__EXEC__` 使用 PromptExec 适配。验证 `cargo test -p agent_tool --lib`、`cargo test -p llm_context`。
3. 实现 `remote_ssh` 命令、文件、环境探测与恢复；增加可显式运行的本机 SSH 测试目标，覆盖实际传输与断线，不将远程服务器作为默认单元测试依赖。SSH 完成前不算首版 runtime 完成。
4. libopendan 切换 native / tmux 并核对绑定与 Session helper；删除已迁移的执行实现。验证 `cargo test -p libopendan -- --test-threads=1`，同步持久版本、schema / fixtures 与相关文档。

验收实验（可先于 xagent 做）：

- 不配置 runtime 与显式 native 行为一致；首版有效配置不含 fs_view / path_layers / limits，现有 exec 与模板超时仍生效。
- tmux 能创建指定 session、继承已有 session，attach 不存在的 session 明确失败；命令在专用 pane 执行，文件工具结果与同 cwd 的 native 一致。
- SSH 在远端 cwd 执行命令，read / write / edit 与 exec 看到同一文件内容；覆盖带空格、引号与中文的路径 / 内容，证明未改写 Runner 本地同名文件。
- workspace / unrestricted 在 native、tmux、SSH 上按目标路径生效；文档与提示词不把 workspace 宣称为 shell 隔离。
- 模板 `{{runtime.os}}` / `__ENV($runtime.hostname)__` 取自执行体；`__EXEC(uname -n)__` 与 exec 工具输出一致；未启用或未注入模板执行器时不会执行。
- SSH 认证失败、目标不可达、取消、超时、执行中断线与 Runner 被 kill 后恢复均有明确结果；不能确认远端旧进程已停止时阻塞恢复，有副作用的调用不重放。
- run.json 保存有效 runtime 与目标；resume 不重读目录配置，实际目标 / kind / workdir 变化则拒绝；含宿主工具的 run 仍遵循接手限制。
- 选择规划中的 kind 明确报 Capability 错误；libopendan 既有用例通过，worklog 形状不变；policy / grant / approval 未实现不阻塞上述验收。

## 10. 实施记录与验证

实现入口：`src/frame/agent_tool/src/runtime/`（配置/registry、RuntimeInfo、Sandbox、native/tmux/SSH 与文件后端），xllm 的 prepare/prepare_hosted/resume 使用共享 runtime；libopendan 只保留 Session 绑定、bin/helper 与协议纪律，原 native/tmux 执行模块已删除。无新 crate 或第三方依赖。

具体接口以源码为准：AgentRuntime.open 返回 `(EffectiveTools, XllmToolManager)`，后者实现 Sandbox，可以作为 ToolManager 或 Arc<dyn Sandbox> 使用。RuntimeOpenCtx 不依赖 Session 类型。RuntimeConfig 的 optional 字段允许分层局部覆盖，打开后配置规范化并保存实际 descriptor。Session 的 binding/session_config/session_state 和 xllm RunRecord 升至 3；summary、机械渲染、快照仍沿用原版本，worklog 形状不变。旧格式明确拒绝。

Session 接管额外保存并核验 runtime、PATH、环境和 bin manifest/helper 内容；凭据只保存环境变量名，恢复时重新读取。宿主 system 渲染稳定 RuntimeInfo 字段，新鲜 current_time/timezone 放输入批次；模板 EXEC 使用注入的 PromptExec，与工具执行器及 registrar 共用路径。启用而未注入时明确错误，控制侧 INCLUDE 行为保留。

已验证：

- `cargo test -p agent_tool --lib`：215 通过，7 项显式测试默认忽略；包含配置覆盖、目标绑定、文件策略、默认 native 与 tmux pane、模板执行与缺失执行器检查。
- `cargo test -p llm_context`：185 通过。
- `cargo test -p libopendan -- --test-threads=1`：83 通过，1 项故障注入 child 入口默认忽略；包括 20 项崩溃恢复测试及 13 个重生成 fixture 场景。接管测试验证 Session PATH/环境恢复及 manifest 变化拒绝，模型调用前完成核验。
- `bash test/runtime_ssh/run.sh`：5 通过。使用隔离 ssh_config/known_hosts、临时 sshd 和两个 loopback 目标，覆盖 exec/read/write/edit、空格/引号/中文/换行路径或内容、cwd/workspace/unrestricted、超时/取消、认证/连接失败、执行中断线、Runner SIGKILL 后恢复、SSH alias 目标改变；证明本地同名文件未被改写，副作用未重放。
- `cargo check --workspace` 与 `uv run buckyos-build.py --skip-web` 通过。整仓 `cargo test --workspace -- --test-threads=1` 在链接阶段因现有磁盘空间不足（ENOSPC）未完成；已清理本轮生成的增量缓存/链接产物，相关模块测试重新通过。
- schema 和 fixtures 由 Rust 参考实现重新生成；fixture 路径替换同步重算 worklog 字节边界，测试专用主机身份占位符不改变生产恢复核验。

首版限制：SSH 目标需 Linux/bash/SFTP；libopendan 未部署远端 Session helper，选择 SSH 时明确能力错误，独立 xllm 可用。native/tmux 的 workspace 路径检查不构成 OS 隔离。SSH 不提供跨 Runner 的远端工作目录锁。policy、grant、approval 与其它类型按 §7/§8 后续评估；不提前冻结 daemon/HTTP 协议。2026-10-02 起恢复不做进程核验（长命令 TODO §3.2），native / tmux / SSH 都不再依赖 /proc 与 setsid；tmux 因此可在 macOS 上使用。
