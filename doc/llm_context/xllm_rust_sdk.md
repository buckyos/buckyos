# xllm Rust SDK 参考

- 日期：2026-09-18；2026-10-02 同步 AgentRuntime、`RunRecord.version = 3` 与快照 v3
- 实现：`src/frame/agent_tool/src/xllm.rs`（SDK）、`src/frame/agent_tool/src/run_local_llm.rs`（CLI，`agent_tool xllm ...`）
- 依据：[xllm PRD](../../product/xllm/PRD.md)。本文只记录 Rust 实现落实 PRD 时固定下来的协议决定，供 websdk 的 TS 版本对照；产品行为以 PRD 为准。

## 1. 分层与入口

| 层 | 类型 / 函数 | 职责 |
| --- | --- | --- |
| 配置 | `discover_config_files` / `load_config_layers` / `merge_config_layers` → `MergedConfig` | 从工作目录向上查找 `.llm_context`，按“祖先 → 子目录”合并；每个字段记录来源文件 |
| 准备 | `XllmTask::prepare(workdir, TaskInput, TaskOverrides, &XllmDeps) → PreparedTask` | 选组、展开工具（含 MCP 发现）、读取附件、渲染模板、组装提示词、能力校验；**不建立 Run** |
| 执行 | `XllmRun::start(prepared, deps)` → `execute()` → `RunOutcome` | 分配 runid、加锁、首次模型请求前写记录、驱动 waist `LLMContext` 到本次停止点 |
| 恢复 | `XllmRun::resume(store, run_id?, workdir?, ResumeLimits, deps) → ResumeStart::{Terminal, Run}` | 终态只返回记录；非终态重建工具与 Provider，装回最新快照 |
| 查询 | `list_runs` / `latest_run` / `load_run` / `export_result` / `build_result_view` | 只读记录，不触发执行 |

`XllmDeps` 注入：`LlmClientFactory`（默认 buckyos→AICC、openai→Chat Completions）、具名工具（`tools: - name: x`）、`RunObserver`（进度事件）、锁目录。

## 2. `.llm_context` 解析要点

- YAML 手工转换，未知字段、类型错误、别名/行号冲突都报 `XllmError::Config { file, field, reason }`。
- `runs_dir: none`（或 null）表示不落盘（内存 `RunStore`）。相对路径相对声明它的文件；`~` 展开为主目录。
- `prompt.groups.<name>` 为字符串时是外部组文件（根节点即组对象），加载时展开并记录来源。
- section 键：数字行号或固定别名（role=10、contexts/env=20、rules=30、cmd_manual=40、output_format=100）；`text: ""` 显式清空；`name` 与固定别名冲突报错。
- 合并规则：标量覆盖；`tools` 对象按字段合并，`tools`/`actions`/`bash_tools` 列表整体替换；组按名合并、section 按行号合并；`mode: custom` 与同层 `select`/`sections` 冲突报错。
- 凭据只保存引用（`SecretRef::ConfigFile{path, field}` / `Env{name}`），resume 时重新解析。

## 3. 有效配置优先级

`默认值 → 合并后的文件配置 → 选中组 → prompt.tools → CLI（TaskOverrides）`。工具开关：`tools.enabled` 默认 false；`--tools/--no-tools` 最高。仅启用未配置列表时使用 `bash` 组（`read_file`、`write_file`、`edit_file`、`exec`）。`exec` 默认超时 1800s、上限 3600s，输出按头 1/4 + 尾 3/4 保留 64KB；命令在独立进程组中运行，超时、中断或到达总时长时整组 SIGKILL，超时返回 Error 结果（`timed_out`、已有输出与重试提示）；非 0 退出的 Error 观察同时带 summary 与输出。运行中的工具调用受 `--timeout`（默认 3600s）和 Ctrl-C 取消。function_call 下配置 `actions` 或 `tools2actions` 报能力错误；behavior + `tools2actions` 把 tools 转为 actions，原生列表置空。

`tools.filesystem_policy` 可在顶层、选中组或 `prompt.tools` 中配置，按同样的字段覆盖顺序生效。`workspace`（省略时的默认值）将内置文件工具路径和 `exec.cwd` 限制在工作目录内；`unrestricted` 清空读写路径白名单并允许 `exec.cwd` 指向其它目录，实际文件访问由运行用户的操作系统权限决定，相对路径仍以工作目录为基准。[通用模板](../../product/xllm/PRD.md#49-通用配置模板参考-pi-mono) 显式选择 `unrestricted`。该策略对 function_call 和 behavior 中的内置 `bash` 组均生效，不改变 MCP 或宿主工具的策略；`workspace` 也不隔离 shell 命令自身的文件访问。实际策略保存在 `RunRecord.config.tools.filesystem_policy` 中，resume 使用保存值，不重新读取目录配置。

## 4. 提示词组装

system = 按行号升序的非空 section（`## <name>` 标题 + 用户文本 + 系统说明）+ `## runtime_protocol`。系统说明：20 补齐未被模板引用的 RuntimeInfo 核心字段；30 列出实际可用 tools/actions（或声明没有工具）；40 只在 exec 启用时输出命令手册（含 `bash_tools`），exec 未启用时整段省略。custom 模式 = 用户整段 + `## capabilities` + `## runtime_protocol`。

模板：`{{runtime.id|kind|os|arch|hostname|shell|cwd|tools|current_time|timezone}}`、`{{env.NAME}}`；`\{{` 转义；缺失变量在模型请求前报 `XllmError::Template`。渲染结果与变量随 Run 保存。

user = 任务要求（显式 > 组默认 > stdin）→ 附件（命令顺序；文本用 `<material name="...">`，图片用 `[image N: label]` + 图片块）→ `<stdin>` 材料。配置 `file_model` 且有图片时先由文件模型分析，主模型收到 `<image_analysis model="...">` 而不再收到图片。

behavior 协议（`XllmActionParser`）：`<response><thinking/><actions>…</actions><report/></response>`；动作标签集由本次生效 actions 决定，属性→参数，正文→`command`/`content` 等正文参数或 JSON；**没有动作且带 `<report>` 的回复即为最终答案**。

正文/CDATA 直接承载参数值，不加字段名或 `字段名:` 前缀。例如读取文件使用 `<read_file><![CDATA[fixture.txt]]></read_file>`，也可使用 `<read_file path="fixture.txt"/>`。同一个正文参数已通过属性给出时保留属性值，空 CDATA 不会覆盖它；未通过属性给出的正文参数仍支持空字符串（例如写入空文件）和原始空白。

`context_window`（顶层，token，可选）是模型上下文窗口，记入 `RunLimits.context_window_tokens`。设置后，待发送请求的估算达到窗口的 75% 时 waist 让出上下文压缩，估算加 `max_tokens` 超过窗口的请求从不发送；未设置时只在 Provider 以结构化错误码拒绝（OpenAI 兼容接口的 `context_length_exceeded`）时压缩。压缩用本次模型（`LlmSummarizeCompressor`），function_call 以 `RewrittenHistory`、behavior 以 `RewrittenSteps`（物化历史折叠进 input，编号继续）续跑，压缩后的上下文先保存快照再继续，每个 run 最多 3 次。

`max_tool_iterations`（默认 8；CLI `--max-tool-iterations`，`TaskOverrides` / `RunLimits` / `ResumeLimits` 同名字段）是原生 tools 与 behavior actions 共用的工具预算，映射到 waist `ToolPolicy.max_tool_iterations`。一次工具迭代 = 一批实际派发的原生工具调用（一次 response 的全部调用），或一个派发 action 的 behavior Step（同一 Step 多个 action 只计一次）；工具业务失败也计入，behavior Step 内层的原生工具批次与外层 action 共用剩余额度。额度耗尽后仍允许模型返回无工具的最终答案，再请求工具或 action 则进入 `limit_reached`（`tool iteration limit (N) reached`），不会执行超额调用。`max_calls_per_round = 16` 另限一次 response（一个 Round）的原生调用数，不限制 Step 的 action 数。

工具预算不计推理次数：无工具的最终回答、解析纠错后的重试都是 Round，但不消耗工具迭代。推理尝试数记在 `RunRecord.usage.llm_requests`（§5），xllm 不设推理次数上限。

## 5. Run 记录

```
<runs_dir>/<run_id>/run.json        RunRecord
<runs_dir>/<run_id>/snapshots/NNNN.json   waist LLMContextSnapshot（每次推理前 + outcome 边界；快照 v3）
<runs_dir>/<run_id>/.lock           该 Run 的执行互斥
<lock_dir>/<hash(workdir)>.lock     启用工具的任务按工作目录互斥（默认 ~/.xllm/locks）
```

`run_id` = `YYYYMMDD-HHMMSS-<6hex>`。`RunRecord` 关键字段：`version`（当前 3）、`status`、`workdir`、`input`（请求、来源、附件摘要与 sha256、stdin 角色）、`config`（有效 runtime 配置与 runtime_descriptor/Provider/模型/loop/限制 `limits: RunLimits {max_tokens, max_tool_iterations, timeout_secs, llm_timeout_secs, context_window_tokens}`/result_format/工具展开结果/配置文件与字段来源）、`prompt`（section 渲染结果、runtime_protocol 与版本、最终 system、模板变量）、`file_model_stage`、`pending_input`（首次快照前保留的输入，之后清空）、`latest_snapshot_idx`、`last_error`（phase/kind/message/recoverable/condition）、`result`（raw 原文、按 result_format 的提取值、`--json` 校验结果）、`artifacts`、`usage`（主模型/文件模型分别记录；`llm_requests` = 本 Run 经宿主 `LlmClient::infer` 发起的推理尝试数，含失败尝试、文件模型阶段以及 xllm 自己执行时的上下文压缩摘要请求，每个执行段只累加、不重置）、`limit_reason`、`interrupt_reason`。

## 6. 状态机与错误分类

| 状态 | 终态 | 触发 |
| --- | --- | --- |
| running | 否 | 执行中；记录为 running 但无进程持锁时，查询显示为“已中断（进程退出）” |
| interrupted | 否 | `XllmInterrupter::interrupt`（Ctrl-C）或进程退出 |
| paused | 否 | Provider 超时 / Transient / 疑似凭据问题（Permanent 且信息含 token、401、expired 等）/ 存储 checkpoint 失败 / 工具基础设施失败 |
| completed | 是 | waist `Done` |
| failed | 是 | Provider Permanent（非凭据）/ Unknown、输出解析或工具错误连续超限、上下文压缩 3 次仍超限、deferred tool |
| limit_reached | 是 | 工具迭代（`max_tool_iterations`）/ 总时长（`timeout`，映射为 waist wallclock 预算）/ token 预算 |

resume：终态只返回记录（附带限制参数则报 `RunTerminal`）；非终态重置 wallclock 起点，工具迭代沿用已消耗值（`ResumeLimits.max_tool_iterations` 显式调高时剩余 = 新总额 − 已消耗，只增加差额），`usage.llm_requests` 继续累加。`RunRecord.version` 不是 3 或快照版本不是 3 的 Run 不能恢复（不迁移旧格式）。保存于上下文上限挂起态的快照先压缩再续跑；等待 deferred 工具结果的快照报 `NotResumable`（xllm 不提供 deferred 结果）。

## 7. CLI（`agent_tool xllm`）

CLI 自身的帮助、状态标签（含结构化结果中的 `status_label`）、阶段、进度日志和诊断信息统一使用英文。用户输入、模型结果和外部工具返回内容按原文保留。

参数面与 PRD §6 一致。stdin 只在是管道（FIFO）或文件重定向时自动读到 EOF；终端、socket、`/dev/null` 视为无管道输入；管道为空报错且不建立 Run。

退出码：0 完成/查询成功；1 终态失败或查询目标不存在；2 参数/配置/输入预检错误（无 Run）；3 可恢复暂停；4 用户中断；5 `--output` 写入失败；6 结果提取或 `--json` 校验失败（原文已保存）。

`--format json` 输出 `XllmResult`（runid、状态、是否终态/可恢复、answer、artifacts、usage、error、Provider/模型/实际返回模型、resume 命令）。

`run_logs: info/debug` 启动时在 stderr 显示实际合并的 `.llm_context` 数量及路径，按祖先到工作目录的覆盖顺序排列；未找到配置时显示 0。resume 显示沿用的原 Run 配置来源，不重新合并。`exec` 的开始、完成和失败日志在括号中显示实际 command，多行和控制字符转义为单行；call_id 保留在内部事件与运行记录中。日志级别取生效配置，显式 `--run-logs` 可覆盖；warn/result 隐藏这些常规进度。

## 8. buckyos Provider 的登录方式

`model` / `file_model`（包括 `--model` / `--file-model` 覆盖）支持两种选择器：`llm.chat`、`llm.vision` 等逻辑名走 AICC `helper.llm_chat`；`model@provider` 或 `model:variant@provider` 精确选择器走 `chat.completions.create`，原样保留模型、variant 和 provider，不进行逻辑路由或 fallback。例如：`agent_tool xllm --model 'gpt-5.6-sol@openai-main' '你好'`。精确选择器中的模型和 provider 必须存在于当前 AICC 环境；格式错误由 AICC SDK 拒绝。

`ensure_buckyos_runtime` 按顺序选择身份（`BUCKYOS_APP_ID` 可覆盖默认的 `buckycli`）：

1. 设置了 `BUCKYOS_APPCLIENT_SESSION_TOKEN`：AppClient，直接使用 verify-hub 签发的 session token（OpenDAN 给工具注入的方式）。`BUCKYOS_APP_ID` 应与 token 的 appid 一致；应用 owner 从 token 的应用实例 claims 补齐，无需额外设置 `BUCKYOS_OWNER_USER_ID`。`control-panel` 等 system 会话使用自己的 system target。
2. 在 OOD 本机且能读到设备密钥（`/opt/buckyos/security/<device>/authentication.private.pem`）：以 KernelService 语义初始化（服务地址走 127.0.0.1），用设备密钥签 `sub = iss = 设备名` 的登录断言，经 node gateway（默认 3180 端口）上的 verify-hub 换取正式会话后登录。`buckycli` 在 RBAC 中属于 kernel 角色，system-config 与 AICC 均接受。DV Test 环境下 root 直接运行即可，不需要 dev 目录或额外环境变量。
3. 否则：AppClient，用 `$BUCKYOS_DEV_HOME` / `~/.buckycli` 下的用户私钥签断言，经 verify-hub 换会话；该路径要求所选 app 已安装在 zone 中（否则 verify-hub 返回 `AppAccessDenied`）。

## 9. 验证

- `cargo test -p agent_tool --lib xllm`（SDK，ScriptedLlm 驱动，覆盖配置合并、section 组装、工具优先级、behavior/function_call 循环、暂停与恢复、锁、文件模型阶段等）。
- `cargo test -p agent_tool --lib run_local_llm`（CLI 参数规则）。
- 真实环境：在 DV Test 的 OOD 上以 root 运行 `agent_tool xllm "问题"`，即走上面第 2 种登录方式。
- 端到端（无 BuckyOS）：用一个 OpenAI 兼容的 mock HTTP 服务（返回 `choices[0].message` 与可选 `tool_calls`），在 `.llm_context` 里配置 `provider.type: openai`、`base_url`、`api_key_env`，即可跑通新任务、管道串联、工具循环、`--json`、暂停/resume、`--output`、list/status/result。

## 10. 宿主装配的 Run 与恢复检查（2026-09-29，Agent Session SDK §8.7）

libOpenDAN 把 session 的 `runs/` 直接作为 xllm 的 run 目录。为此增加以下可选能力，xllm 自己的 Run 行为不变：

- **宿主装配（X2）**：`XllmTask::prepare_hosted(workdir, llm_context_json, origin, host_system, deps) -> HostedTask`。宿主给出 `.llm_context` 的 JSON 形式（同 schema、严格键）和自己的 system 文本；xllm 计算有效配置、展开工具，并在宿主文本后追加 `capabilities` / `cmd_manual` / `runtime_protocol` 段（`protocol_version = xllm/1`）。宿主 system 支持稳定 runtime.* 与具名 env.* 模板，current_time/timezone 在输入批次提供，system 引用新鲜量会报 Template；`HostedTask::new_record` / `build_request`、`hosted_request`、`hosted_waist_deps`（behavior 用 `XllmActionParser` + `XmlStepRenderer`）让宿主驱动 waist，而 run 目录保持 xllm 可接手。`rebuild_toolset` / `create_run_llm` 按保存的记录重建工具与 Provider。
- **RunStore（X1）**：`create_run`、`lock_run`、`remove_run`、`prune_snapshots` 公开；`run.json` 与快照写入先 fsync 再原子发布（目录也 fsync）。
- **RunRecord 新字段**（均可缺省）：`host`（`assembled_by`、`session_id`、`runtime_kind`、`runtime_id`、`env_check`）、`host_commit_pending`、`inflight[]`、`executions[]`。
- **resume 检查（X3 / X6）**：`version` 不等于当前版本（`RUN_RECORD_VERSION = 3`）→ 拒绝；`host_commit_pending` 非空 → 拒绝（须由宿主补交输入）；按保存的 runtime 构造执行体并核对完整 descriptor（kind、id、实际 target、cwd）；Session 接管校验保存的环境、PATH、bin manifest 与 helper 内容，凭据重新读取环境引用；取得 run 锁后先确认 `executions[]` 中旧执行已停止（`exec_tracking::stop_execution`，无法确认则拒绝），再把没有持久结果的 `inflight[]` 物化为“结果未知”（`materialize_unresolved`）并落盘，**不重放工具**。
- **执行跟踪（X6）**：`agent_tool::exec_tracking`：`TrackedBashRunner`（启动握手：执行标识经 `ExecutionRegistrar` 持久化后才放行命令；子进程继承 `OPENDAN_EXECUTION_ID`）、`probe_execution` / `stop_execution`（按环境标记而不是可复用的 PID 核对，无法核对返回 Unknown）。`XllmDeps.runtime` 注入共享 `AgentRuntime`，`execution_registrar`、`runtime_env` 与 `runtime_path_prefix` 由宿主装配；`XllmDeps.skip_workdir_lock` 让宿主自行协调共享 workspace。
- **用量累加**：宿主驱动时由宿主在每个 outcome 后把本段推理尝试数加到 `usage.llm_requests`（libOpenDAN 如此），xllm 接手后在其上继续累加；任何执行段都不覆盖已有值。
- 快照版本与宿主元数据见《LLM Context 设计》§9.4；session 协议见 `doc/opendan/protocol/`。

## 10. 共享 AgentRuntime

`agent_tool::runtime` 提供 `RuntimeConfig`、`RuntimeRegistry::from_config(&cfg)`、`AgentRuntime` 与 `Sandbox: ToolManager`。`prepare` / `prepare_hosted` 调用 `runtime.open(ctx, tools_cfg, host_tools)`，返回 `(EffectiveTools, XllmToolManager)`；后者实现 Sandbox，接受未展开工具配置并复用既有工具解析。`RuntimeOpenCtx` 只接收控制侧 workdir、run_id、LoopModel、工具来源、registrar 和宿主环境，不依赖 Session 类型。Runtime 的 `descriptor()` 在打开后才表示实际目标；`info()` 更新执行处时间与时区。

`.llm_context` 顶层 runtime 默认 native：

```yaml
runtime:
  kind: native
  workdir: ./workspace
  env: { LANG: en_US.UTF-8 }
```

```yaml
runtime:
  kind: tmux
  tmux:
    session: review
    mode: create_or_attach  # create / attach / create_or_attach
    socket: ./review.sock
```

```yaml
runtime:
  kind: remote_ssh
  workdir: /srv/review
  remote_ssh:
    host: review-host
    user: alice
    port: 22
    identity_file: ~/.ssh/review_key
```

同 kind 逐字段覆盖，env 按键覆盖；切换 kind 重置整段。native/tmux 的 workdir、tmux.socket、identity_file 按声明配置文件目录解析；SSH workdir 必须显式绝对路径。id 是身份，不是 profile 查找键。未知字段和无关连接块报 Config；规划中的 container、container_host、remote_node、http_proxy_runtime 报 Capability。runtime 不接受 fs_view、path_layers、limits 或 policy。文件工具的 workspace 策略在目标侧解析真实路径，防止符号链接逃逸；shell 的文件访问仍由系统权限控制。

exec、read/write/edit、模板执行共用执行体。MCP 仍在所配置服务执行，进程内宿主工具仍在 Runner 执行；都由 Sandbox 派发，具名宿主工具缺失时拒绝接管。SSH 使用系统 ssh/sftp、非交互认证和 known_hosts，要求远端 Linux/bash/SFTP；探测与身份核验在首次模型请求前完成。脚本和文件内容经数据通道传送，目标侧临时文件替换写入，不回退本地执行。exec 在 go 放行前持久化身份，断线结果记为未知；超时、取消及恢复在目标侧核验停止，不重放副作用。无法核验时返回 RecoveryBlocked。

run.json 的 workdir、配置文件、日志和快照属于控制侧；config.runtime.workdir 是执行侧路径。恢复沿用保存配置，不重读 .llm_context；SSH alias 重定向或 tmux session 被替换都会拒绝。native/tmux 保留实际 cwd 的 flock；SSH 不取远端路径的本地锁，跨 Runner 并发由宿主协调。Session 未部署远端 helper 时明确报 Capability；SSH 可独立用于 xllm。

新任务可用 `--runtime <kind>` 覆盖 kind；连接字段仍需来自配置。resume 与查询不接受该参数；status 显示实际 runtime、target、cwd 和 env_check。核心测试：`cargo test -p agent_tool --lib`、`cargo test -p llm_context`；真实传输测试：仓库根目录 `bash test/runtime_ssh/run.sh`（需 ssh、sftp、sshd），覆盖文件、取消、认证失败、断线、强杀与目标变更。
