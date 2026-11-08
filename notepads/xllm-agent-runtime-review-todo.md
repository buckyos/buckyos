# 从 xllm 看 Agent Runtime：现状梳理 + 工具等待策略 + BuckyOS 三档 TODO

日期：2026-11-01

状态：**整理中，待用户 review / 裁决（Q2 已于 2026-11-06 裁决为取小；其余 Q1、Q3–Q9 给的是建议值）。** 起因是 llm_context 冻结重构（[llm-context-freeze-todo](./done/llm-context-freeze-todo.md) §8.4 T7）之后在 xllm 里跑通了 `ask_user`，暴露出两件事：

1. 调用内等待预算是一刀切的 `task_wait_ms`（默认 30 秒、硬顶 30 分钟），而 `ask_user` 这类"等人"的工具应该能配到 2 小时这种级别。应当能通过 policy 针对特定工具配置最长等待。
2. xllm 最常用的 native runtime 对"有没有 BuckyOS"的判定是隐式、分散的（有 token 就接 TaskMgr，provider 另算一套）。应当有显式三档：`yes` / `no` / `auto`（有则用）。

本单的主要目的是**站在 xllm 的角度把现有 agent runtime 的实现梳理一遍**（§1–§2），再在这个基础上给出这两项配置的形状（§3）。实施排在裁决之后；agent-session 侧只写接入点，不在这里改。

## 1. 现状（2026-11-01 源码核对）

### 1.1 分层与各层对"等待"知道多少

| 层 | 代码 | 对任务 / 等待的认知 |
| --- | --- | --- |
| waist `llm_context` | [deps.rs:111](../src/frame/llm_context/src/deps.rs) `ToolCallCtx { abort, deadline_ms }` | 不认识任务（冻结原则 8）。只给工具一个打断令牌和 run 总时长的截止点；`ToolPolicy`（[request.rs:63](../src/frame/llm_context/src/request.rs)）管的是白名单、迭代数、`finish_grace_ms`，**没有**按工具的时间字段 |
| `agent_tool` 工具层 | [task_model.rs](../src/frame/agent_tool/src/task_model.rs)、[tasks.rs](../src/frame/agent_tool/src/tasks.rs)、[tasks_zone.rs](../src/frame/agent_tool/src/tasks_zone.rs)、[task_bridge.rs](../src/frame/agent_tool/src/task_bridge.rs) | 任务概念在这里：`RunningTaskResolver`（state / wait / cancel / can_resolve / watch）、`TaskState`、进程内 `InProcessTaskManager`（`local:` id）、Zone TaskMgr 的 `ZoneShellTasks`（`t-` id）、`wait_task / get_task_state / cancel_task` 工具与 task CLI bridge |
| `agent_tool` 派发层 | [xllm.rs:4180](../src/frame/agent_tool/src/xllm.rs) `XllmToolManager::call_tool` → `wait_in_tool`（:4124） | 工具返回 `pending + task_id` 时，在**这次调用里**等 `check_after` 或 `task_wait_ms`，到点把任务当时状态当结果返回；等待被 `ToolCallCtx.deadline_ms` 和常量 `MAX_IN_TOOL_WAIT_MS` 再截一刀（`wait_until_ms`，task_model.rs:322） |
| `agent_tool` AgentRuntime | [runtime/mod.rs:32](../src/frame/agent_tool/src/runtime/mod.rs) `RuntimeConfig`、`AgentRuntime` trait、`RuntimeRegistry`、`open_runtime` | 执行体（native / remote_ssh / remote_node）+ 唯一的策略字段 `task_wait_ms`（F13 第四轮："不建预设注册表，只加这一个字段"） |
| 宿主 | xllm CLI [run_local_llm.rs](../src/frame/agent_tool/src/run_local_llm.rs)；libopendan [live.rs:120](../src/frame/lib_opendan/src/runner/live.rs)；opendan [tasks.rs:281](../src/frame/opendan/src/tasks.rs) `zone_xllm_deps` | 决定 `XllmDeps` 里注入什么：任务后端、node connector、宿主工具、task CLI、url_reader。"仍在运行"之后谁跟任务由宿主定（`TaskFollowup`：xllm 让模型 `wait_task`，Session 在循环外跟） |

### 1.2 AgentRuntime 今天的公开面

`RuntimeConfig`（`deny_unknown_fields`）：`kind`（缺省 native）、`id`、`workdir`、`env`、`remote_ssh`、`remote_node`、`baseline`、`task_wait_ms`。

- **身份**：`runtime_id` 由 native 的 `host + uid`（[mod.rs:525](../src/frame/agent_tool/src/runtime/mod.rs) `native_host_id` = `AGENT_TOOL_HOST_ID` 或 `/etc/hostname + /etc/machine-id`）或远端目标派生，进 `RuntimeDescriptor`，恢复时逐字段核对。策略字段不参与（`task_wait_ms` 已这样处理）。
- **kind 与依赖**：`remote_node` 要 `XllmDeps.node_executor`，拿不到报 Capability（[mod.rs:794](../src/frame/agent_tool/src/runtime/mod.rs) `from_config_with`）；`container / container_host / http_proxy_runtime` 报"规划中"；TS 侧规划 `site` kind（[xllm-ts-runtime-todo](./xllm-ts-runtime-todo.md) Q4）。
- **合并**：祖先 → 子目录，同 kind 逐字段覆盖，换 kind 重置整段；CLI `--runtime` 只换 kind。
- **打开**：`open_runtime(config, ctx, tools, deps)` → `runtime.open` → `(EffectiveTools, XllmToolManager)`；`ShellSettings::from_config(cfg.shell, task_wait_ms)`（xllm.rs:592）把预算塞给 shell 当 `wait_ms`。
- **Capabilities**（descriptor）：只有 `tools / network / os`，没有"任务后端是什么""有没有 BuckyOS"。
- **run.json**：`EffectiveConfig.runtime`（整份 RuntimeConfig，含 `task_wait_ms`）+ `runtime_descriptor`；`xllm status` 不显示任务后端与 BuckyOS 状态。

### 1.3 等待时间今天有几道墙

想让一次 `ask_user` 调用在调用内等 2 小时，要过的墙：

| 墙 | 值 | 位置 |
| --- | --- | --- |
| `RuntimeConfig.task_wait_ms` | 默认 30 s，`clamp(1, 30 min)` | runtime/mod.rs:88；`XllmToolManager::set_task_wait_ms` 再 clamp 一次（xllm.rs:4026） |
| `AgentToolResult.check_after`（秒） | 工具给；**替代**预算而不是取小 | xllm.rs:4252 `check_after.map(…).unwrap_or_else(task_wait_ms)`；随后仍被下一行截 |
| `MAX_IN_TOOL_WAIT_MS` | 常量 30 min | task_model.rs:24；被 `wait_until_ms`（:325）、`wait_task` 工具（tasks.rs:832）、task CLI bridge（task_bridge.rs:113）、`ZoneShellTasks::wait`（tasks_zone.rs:661）、`ShellTask::wait`（tasks.rs:283）、`MAX_AUTO_WAIT_MS`（llm_bash.rs:51）、两处 clamp 共 **8 处**引用 |
| `ToolCallCtx.deadline_ms` | run `timeout` 默认 3600 s | xllm.rs:128 `DEFAULT_TIMEOUT_SECS`；`wait_until_ms` 取 `min(deadline)` |
| `tools.shell.timeout_ms / max_timeout_ms` | 30 min / 60 min | 命令本身的超时，不是等待；只管 shell |
| `ask_user --expires` | 默认 10800 s = 3 h | websdk [ask_user.ts](../src/tools/buckyos-agent/commands/ask_user.ts)；问题有效期，与等待预算无关 |

结论：**预算是单值 + 硬顶**，没有按工具的维度；就算把 `task_wait_ms` 调到 2 小时，常量和 run `timeout` 也会把它截回 30 分钟 / 1 小时。

### 1.4 "工具"在 xllm 眼里是谁：按工具名配不到 `ask_user`

`ask_user` 是 websdk CLI（[feedback-agent-tools-ts-websdk]），在 xllm 里**经 `shell` 调用**：shell 把简单命令 stdout 里的 AgentToolResult 信封原样转发（[llm_bash.rs:1522](../src/frame/agent_tool/src/llm_bash.rs) `try_forward_inner_agent_tool_result`，要求 `command_is_simple_for_protocol_forward`）。所以到 `XllmToolManager::call_tool` 时 `call.name == "shell"`，能区分它的只有：

- 信封字段：`pending_reason: "user_input"`（result_view.rs:51；今天**没有任何代码读它**，只是给模型看）、`check_after`；
- 任务 id 形态：`t-…`（Zone TaskMgr）/ `nt-…`（通知）；任务的 `schema_id`（`human.input/v1`）要再查 TaskMgr 才知道；
- shell 命令的程序名（`ask_user`），`command_is_simple_for_protocol_forward` 已经在判这个形状；
- `bash_tools` 手册名（如果写进手册）。

libopendan 里 `ask_user` 同样是 shell 命令；function call 形态的具名工具（`report`、`call_behavior`、MCP 工具）才是按 `call.name` 可见的。**按工具配策略，键至少要同时覆盖"工具名"和"shell 简单命令名"**，否则 xllm 最常用的形态配不到。

### 1.5 BuckyOS 支持面：xllm CLI 今天怎样"有则用"

| 依赖 BuckyOS 的能力 | 今天的触发条件 | 失败时 |
| --- | --- | --- |
| provider `buckyos` → AICC | `provider.type` 缺省就是 buckyos；`DefaultLlmClientFactory`（xllm.rs:4938）调 `ensure_buckyos_runtime`（:4985）：① `BUCKYOS_APPCLIENT_SESSION_TOKEN` → AppClient；② OOD 本机设备密钥 → KernelService；③ `~/.buckycli` 用户私钥 → AppClient | `Capability` 错，run 不开 |
| Zone TaskMgr 任务后端（`XllmDeps.buckyos_tasks`，能解析 `t-` 任务，`ask_user` 的答案走这里） | **只看** `BUCKYOS_APPCLIENT_SESSION_TOKEN` 非空（run_local_llm.rs:1163 `with_zone_tasks`，start 与 resume 两处） | 静默不注入；`t-` 任务 → `Unknown`，模型拿不到答案 |
| shell 长命令交 Zone TaskMgr（`shell_tasks`） | CLI 从不设（`None` → 进程内 `local:`）；只有 opendan `zone_xllm_deps` 设 | — |
| `remote_node` 执行体 | 总是注入 `BuckyosNodeConnector`，按需连接 | 连接时报错 |
| websdk CLI 工具（`ask_user`、`import_*`、`aicc-tool` 等） | 靠 PATH 与**继承的环境变量**（token、`BUCKYOS_HOST_GATEWAY`、`BUCKYOS_NODE_GATEWAY_PORT`），xllm 不感知 | 工具自己报错 |
| `task_cli`、`url_reader`（`obj:// cyfs://`） | CLI 不设 | `read` 拒绝这些 URL |

三处不一致：

1. **provider 与任务后端判定不同**。设备密钥登录（②）成功时 AICC 能用，但任务后端因为没有 token 不接，`ask_user` 的 `t-` 任务解析不到——11-01 DV T7 正是靠手工拿 devtest 会话当 token 才通的。
2. **"没有 BuckyOS"要靠显式 `provider.type: openai` 表达**，否则默认 buckyos 直接报错；没有一个开关说"这台机器就是没有 BuckyOS，别去试"。
3. **auto 已经部分存在但不可见**：status / run.json 都不记录探测结果，排查只能看 stderr 的 warn。

### 1.6 xllm 与 libopendan 的 deps 差别（现状对照，§3.9 预设讨论的事实基础）

| `XllmDeps` 字段 | xllm CLI | libopendan（live.rs:120–175） | opendan `zone_xllm_deps` |
| --- | --- | --- | --- |
| `buckyos_tasks` | 有 token 才 `ZoneShellTasks` | `host_task_resolver(sh)`（session 子任务 + Zone） | `ZoneShellTasks.resolver` |
| `shell_tasks` | `None`（进程内） | 由 opendan 注入 | `ZoneShellTasks`（挂 Turn task 下） |
| `shell_task_context` | 默认 | `session_id + parent_task` 闭包 | — |
| `task_cli` | `None`（模型用 `wait_task` 工具） | UI 会话 `agent-session task` | — |
| `host_prompt_sections` | 全开 | UI function call 时 `NONE` | — |
| `host_tools` | 无 | `report`、`call_behavior`、consolidation | — |
| `node_executor` | `BuckyosNodeConnector` | 同 | 同 |
| `task_wait_ms` | 配置，缺省 30 s | 未设（缺省 30 s；第四轮建议 opendan 应 auto 30 s） | — |

差别全在 deps 注入与一个预算字段上，和 F13 第四轮"差别只剩等待预算和任务后端两项"一致。本单两项新配置正好落在这两项上：**tool_policy 是预算的细化，buckyos 三档是任务后端（及其它 BuckyOS 依赖）的显式开关**。

### 1.7 相关文档现状

- [AgentRuntime 设计](<../doc/llm_context/AgentRuntime 设计.md>) §4.1 字段表、§7 长命令与 task（已写"每个任务在预算内等，不按工具分类"）。
- [xllm_rust_sdk.md](../doc/llm_context/xllm_rust_sdk.md) §8 登录方式、§10 共享 AgentRuntime。
- [LLM Context 设计](<../doc/llm_context/LLM Context 设计.md>) §11 xllm 工作流伪代码。
- [llm-context-freeze-todo](./done/llm-context-freeze-todo.md) §3.2.1 等待规则（"等待规则只有一条：每个任务都在 Runtime 预算内等，不按工具分类……`pending_reason` 可当跳过等待的可选优化"）、§3.9 预设。**本单 §3.1 与这句"不按工具分类"是有意的修订**：规则仍是"一条"（预算内等），只是预算可按工具给。
- [xllm-ts-runtime-todo](./xllm-ts-runtime-todo.md) Q3/Q4：TS 的 `RuntimeConfig` 要同步新字段。

## 2. 问题清单（xllm 视角）

| # | 问题 | 来源 |
| --- | --- | --- |
| P1 | 等待预算单值、硬顶 30 分钟、常量散落 8 处；`check_after` 是"替代"不是"取小"，工具可以把等待拉到顶 | §1.3 |
| P2 | 按工具配策略时，`ask_user` 等 shell 包裹的命令没有可配置的身份 | §1.4 |
| P3 | 2 小时等待撞 run `timeout`（1 h）；且 xllm 的 `paused (tasks_running)` + `--resume` 路径（DV T4）本来就能表达"不等了，人来续"，要定清楚什么时候等、什么时候停 | §1.3 |
| P4 | `ZoneShellTasks::wait` 对非本进程任务是 1 秒轮询 TaskMgr（tasks_zone.rs:675）；等 2 小时 = 7200 次 RPC | tasks_zone.rs |
| P5 | BuckyOS 支持判定分散（provider 一套、任务后端一套）、隐式、不可见 | §1.5 |
| P6 | 无 BuckyOS 的机器上 xllm 默认配置直接报错，缺少"明确没有"的表达 | §1.5 |
| P7 | 子进程工具（websdk CLI）依赖的 BuckyOS 环境由 xllm 隐式继承，`remote_node` 还会滤掉 `XLLM_*`（agent-session TODO R9） | §1.5 |
| P8 | 恢复 / 接手时不核对"接手方有没有 BuckyOS"：带 `t-` 任务的 run 被无 token 的进程接手 → `Unknown`（long-tool TODO 曾用 `can_resolve` 拒绝接手，B 之后只剩 `Unknown`） | §1.2 |
| P9 | 文档里"预算不按工具分类"与本意不符 | §1.7 |

## 3. 方案（建议值，待裁决）

### 3.1 工具等待策略 `runtime.tool_policy`

形状（配置层，不是新 kind；同样进 `.llm_context` 合并与 run.json）：

```yaml
runtime:
  kind: native
  task_wait_ms: 30000            # 默认预算（保留）
  max_task_wait_ms: 1800000      # 预算上限（新；取代常量 MAX_IN_TOOL_WAIT_MS；缺省 30 分钟）
  tool_policy:
    ask_user:                    # 键 = 工具名，或 shell 简单命令的程序名
      task_wait_ms: 7200000      # 这个工具的调用内预算（受 max_task_wait_ms 约束）
    cargo:
      task_wait_ms: 600000
```

语义：

- **匹配键（Q1）**：按顺序取第一个命中——(a) `call.name`（function call 工具名 / behavior 动作名 / MCP 工具名）；(b) `call.name == "shell"` 且命令是 `command_is_simple_for_protocol_forward` 认可的简单命令时，取其程序名（basename，去 `.exe`）。不按 `pending_reason` 配（它是工具的自述，不是宿主可控的身份）；不按任务 schema 配（要多一次查询，而且预算在派发时就要定）。
- **与 `check_after` 的关系（Q2，2026-11-06 用户裁决：取小）**：`check_after` 从"替代"改为"只能缩短"：`wait = min(check_after（有才算）, policy.task_wait_ms ?? runtime.task_wait_ms, max_task_wait_ms, deadline)`。工具不能单方面把等待拉过宿主预算。这里原来的公式是 `min(check_after ?? policy ?? runtime, max, deadline)`，工具仍能把等待拉到上限，不是取小，已改。这一项随 [TS TODO](./xllm-ts-runtime-todo.md) T2-0 先做，上限在 `max_task_wait_ms` 落地前仍是常量 `MAX_IN_TOOL_WAIT_MS`。
- **与 run `timeout` 的关系（Q3）**：`timeout` 保持 run 总时长语义，不豁免等待。配置校验：任一 `tool_policy.*.task_wait_ms` 或 `task_wait_ms` > `timeout` → `Config` 错，提示把 `timeout` 调大或预算调小；`max_task_wait_ms` > `timeout` 只 warn（上限可以比总时长大，具体预算不能）。
- **预算到期的行为不变**：返回"仍在运行 t-…"；xllm 场景下模型可 `wait_task`（其 `wait_ms` 上限也改为 `max_task_wait_ms`），或回复 → `paused (tasks_running)` 等人 `--resume`。libopendan 可为 `ask_user` 配 0 预算（有事件源，等人 30 秒没意义）——"0 等待"这才有了配置落点（第四轮曾建议由 `pending_reason` 声明，第六轮删了；本单用 tool_policy 承接，Q4）。
- **身份与记录**：`tool_policy / max_task_wait_ms` 与 `task_wait_ms` 一样是策略，不进 `runtime_id`；写进 run.json 的 `config.runtime`，恢复沿用保存值。
- **等待实现（P4，Q5）**：`ZoneShellTasks::wait` 的轮询改为退避（1 s → 2 s → 5 s → 15 s，上限 30 s）；TaskMgr 若有等待 / 订阅接口则优先。`InProcessTaskManager` 是进程内 notify，不受影响。

落点：`RuntimeConfig`（字段、`validate`、`merge_over`——`tool_policy` 按键合并、JSON schema）；`XllmToolManager::call_tool`（按 call 选预算；`set_task_wait_ms` 的 clamp 改读上限）；`task_model::wait_until_ms` 增加上限参数；`wait_task` 工具、task CLI bridge、`ShellSettings::from_config`、`ZoneShellTasks::wait` 的常量全部改读配置（常量只剩缺省值一个用途）；`xllm status` 显示生效预算。

### 3.2 native runtime 的 BuckyOS 三档 `runtime.buckyos: yes | no | auto`

形状：

```yaml
runtime:
  kind: native
  buckyos: auto                  # yes | no | auto（缺省 auto）
```

| 档 | 行为 |
| --- | --- |
| `yes` | 开 run 前 `ensure_buckyos_runtime` 必须成功（三级登录顺序不变），失败 → `Capability` 错。注入：Zone TaskMgr 任务解析器（`buckyos_tasks`，不再只看 token——登录成功就接）、`BuckyosNodeConnector`、`url_reader`（若 CLI 侧有可用实现，Q8）。给子进程导出登录得到的 BuckyOS 环境（token / gateway 地址，Q7），websdk CLI 工具因此可用。`RuntimeInfo` 增加 `buckyos: "yes"`，提示词可用 `{{runtime.buckyos}}`，`## capabilities` 段说明 `ask_user` 等依赖 BuckyOS 的命令可用 |
| `no` | 不登录、不注入任何 BuckyOS 依赖；`provider.type` 缺省或 `buckyos` → `Config` 错（"runtime.buckyos: no 需要 provider.type: openai"）；`t-` 任务 → `Unknown{reason: "no BuckyOS in this runtime"}`；`remote_node` → `Config` 错（它隐含 yes）；`shell_note` 写明"没有 BuckyOS，`ask_user` 等命令不可用" |
| `auto`（缺省） | 尝试登录：成功 → 同 `yes`；失败 → 同 `no`，但 provider 规则维持今天的行为（缺省 buckyos 仍报 Capability，错误文本带上探测原因），并把结果记为 `buckyos_effective: unavailable (<reason>)` |

- **生效值记录**：`EffectiveConfig.runtime.buckyos` 记配置档，`runtime_descriptor` 旁新增 `buckyos_effective: yes | no`（探测结果，Q6 是否进 `Capabilities`）；`xllm status` 显示两者。
- **恢复 / 接手（P8）**：接手方按保存的配置档重新探测；`yes` 的 run 在探测失败的进程里 → `NotResumable`（与"注入 runtime 不匹配"同类）；`auto` 的 run 若原来 `yes` 现在 `no`，且 run 有在跟的 `t-` 任务 → 拒绝接手并说明；没有 `t-` 任务则放行并记录降级。
- **身份**：不进 `runtime_id`（同一台机器有没有登录成功不该改变执行体身份）。
- **`remote_ssh`**：同字段同语义（控制侧登录；远端工具的 BuckyOS 环境另议，本单不做）。`remote_node` 固定为 `yes`，配其它值报 `Config`。
- **shell 长命令是否也交 Zone TaskMgr（`shell_tasks`，Q9）**：建议 **不**。CLI 退出时的进程内任务归属已由 T4b 的 `release_running` 解决；交 TaskMgr 需要 session / Turn 上下文，是 opendan 的事。`yes` 只接解析器，不改 shell 的交接目标。

落点：`RuntimeConfig` 新字段 + `validate`（kind 联动）；`run_local_llm.rs` 的 `with_zone_tasks` 改为 `apply_buckyos_mode(cfg)`，start / resume 两处共用；`DefaultLlmClientFactory` 的 provider 错误带探测结果；`RuntimeInfo.buckyos`；`ensure_buckyos_runtime` 成功后把会话导出到子进程环境（`XllmToolManager` 的 `runtime_env`）；`xllm status`。

### 3.3 梳理结论：xllm 视角下 AgentRuntime 的形状

- **身份**：`kind` + 连接块（`remote_ssh / remote_node`）+ `workdir` + 探测到的 host / uid → `runtime_id`。不变。
- **策略**（不进身份，进 run.json，恢复沿用）：`task_wait_ms`、`max_task_wait_ms`、`tool_policy`、`buckyos`。本单加三个。
- **能力**（descriptor / RuntimeInfo，供提示词与接手核对）：现有 `tools / network / os` + `buckyos_effective`。
- **预设**：维持第四轮结论，不建注册表。"xllm 默认 / session 本机 / session 节点"三份示例配置写进 AgentRuntime 设计 §4，其中 session 本机例子就是 `buckyos: yes` + `tool_policy.ask_user.task_wait_ms: 0`。
- **常量收口**：`MAX_IN_TOOL_WAIT_MS` 只作 `max_task_wait_ms` 的缺省值，其余 7 处改读配置。

## 4. 待裁决

| # | 问题 | 选项 | 建议 |
| --- | --- | --- | --- |
| Q1 | `tool_policy` 的匹配键 | (a) 工具名；(b) + shell 简单命令程序名；(c) + `pending_reason` | **a + b**。c 不做：`pending_reason` 是工具自述 |
| Q2 | `check_after` 与 policy 的关系 | 替代（现状）/ 取小 | **已裁决：取小**（2026-11-06）。工具只能把等待缩短 |
| Q3 | 预算 > run `timeout` | 静默截断（现状）/ Config 错 / 等待不计入 timeout | **Config 错**。不改 `timeout` 语义 |
| Q4 | libopendan 的 `ask_user` 预算 | 由 `pending_reason` 跳过 / tool_policy 配 0 / 维持 30 s | **tool_policy 配 0**（opendan 的 agent 模板里给） |
| Q5 | Zone 任务等待的轮询 | 维持 1 s / 退避到 30 s / TaskMgr 订阅 | **退避**；有订阅接口再换 |
| Q6 | `buckyos_effective` 放哪 | `Capabilities` / RunRecord 新字段 / 只进 RuntimeInfo | **RuntimeInfo + RunRecord 新字段**；`Capabilities` 进 descriptor 会影响接手比对，不放 |
| Q7 | `yes/auto` 成功后是否把会话导出给子进程 | 导出 token + gateway 变量 / 只导出已有变量 / 不导出 | **导出**（设备密钥登录拿到的会话也要给 websdk 工具，否则 `ask_user` 只在有现成 token 时可用）；`remote_node` 滤 `XLLM_*` 的问题（R9）一起看 |
| Q8 | `yes` 是否注入 `url_reader` | 注入 CLI 实现 / 不注入 | 若 agent_tool 里已有可独立构造的实现则注入；否则本单不做 |
| Q9 | `yes` 是否让 shell 长命令交 Zone TaskMgr | 交 / 不交 | **不交**（§3.2） |

## 5. 实施顺序与验证（裁决后）

1. `agent_tool` 配置层：`RuntimeConfig` 三字段 + `validate` + `merge_over` + schema；`task_model` 上限参数化；8 处常量改读配置。`cargo test -p agent_tool --lib`。
2. 派发层：`XllmToolManager::call_tool` 按键选预算、`check_after` 取小；`wait_task` / bridge / `ZoneShellTasks::wait` 退避。新单测：shell 简单命令 `ask_user` 命中 policy、`check_after` 超上限被截、预算 > timeout 报 Config。
3. CLI：`apply_buckyos_mode`、provider 错误文本、`RuntimeInfo.buckyos`、子进程环境导出、`xllm status`。
4. 恢复：接手时的 `buckyos` 核对。`cargo test -p llm_context -p libopendan -p agent_tool -- --test-threads=1`（libopendan 只需编译与现有测试过，不改语义）。
5. 本机 DV（复用 freeze TODO §8.4 的跑法）：
   - `buckyos: no` + `provider.type: openai` + mock → 普通 run 通过；`buckyos: no` 缺 provider → Config 错；
   - `buckyos: auto` 无 token、设备密钥可用 → 任务后端已接（status 显示 `yes`）；
   - `tool_policy.ask_user.task_wait_ms: 7200000` + `timeout: 10800` → `ask_user` 在调用内等到用户在 MessageHub 作答（≥ 31 分钟后作答，证明越过了旧硬顶）；
   - `timeout: 3600` 配同样预算 → Config 错；
   - SIGKILL 后在无 BuckyOS 的进程里 `--resume` 一个带 `t-` 任务的 `yes` run → NotResumable。
6. 文档回写：AgentRuntime 设计 §4.1 字段表 / §7 等待规则（"预算可按工具给"）/ §4 三份示例；xllm_rust_sdk.md §8（三档）/ §10；freeze TODO §3.2.1 那句"不按工具分类"改为引用本单；TS TODO Q3（`RuntimeConfig` 新字段）/ Q4（`site` kind 的 `buckyos` = 是否有浏览器 session）。

## 6. 不在本单

- agent-session 侧对 `tool_policy` 的使用（Q4 的模板配置）与 `ask_user` 撤销、`<<task_result>>` 渲染：[opendan-agent-session-async-task-todo](./done/opendan-agent-session-async-task-todo.md)。
- 远端 runtime（`remote_ssh` / `remote_node`）上工具子进程的 BuckyOS 环境。
- 审批 / 授权类 policy（`ActionGuard`、RuntimeGrant）：仍是 [llm-context-agent-runtime-todo](./done/llm-context-agent-runtime-todo.md) 的"后续规划"，本单的 `tool_policy` 只有时间字段，命名上为它留位置。

## 7. 关联

[llm-context-freeze-todo](./done/llm-context-freeze-todo.md)（§3.2.1、§3.9、§8.4）、[llm-context-long-tool-todo](./done/llm-context-long-tool-todo.md)、[llm-context-agent-runtime-todo](./done/llm-context-agent-runtime-todo.md)、[xllm-ts-runtime-todo](./xllm-ts-runtime-todo.md)、[opendan-agent-session-async-task-todo](./done/opendan-agent-session-async-task-todo.md)、[AgentRuntime 设计](<../doc/llm_context/AgentRuntime 设计.md>)、[xllm_rust_sdk.md](../doc/llm_context/xllm_rust_sdk.md)。
