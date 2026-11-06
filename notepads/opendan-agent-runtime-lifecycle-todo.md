# OpenDAN Agent Runtime：工作模型与生命周期 TODO

> 2026-11-04，三轮迭代合并。第一轮从 [skills 体系改进计划 §3.6.1](opendan-skills-system-todo.md) 的“cmd 装到哪个 runtime”出发整理 runtime 的创建 / 使用 / 更新 / 删除；第二轮用脑图加了用户视角与能力视角两个分类；第三轮（§0）确定 **Agent 工作模型**：私有 runtime 永远在、chat 只知道两台、plan 看全表，并据此把 `kind` 换成 connector + caps + grant（§1）。范围因此从“runtime 登记的生命周期”扩成“Agent 怎么用 runtime”。第四轮（§0.6、§4.6）核对用户添加 runtime 的入口，改成“用户级目录 → 分配给 Agent → Agent 首页看当前与历史”两步。相关：[AgentRuntime 设计 §6.4](<../doc/llm_context/AgentRuntime 设计.md>)、[Agent State Protocol §10](<../doc/opendan/protocol/Agent State Protocol.md>)、[Session Directory Protocol §9](<../doc/opendan/protocol/Session Directory Protocol.md>)、[Session Runtime](<../doc/opendan/OpenDAN Agent-Session-Runtime.md>)、[primary-runtime TODO](opendan-primary-runtime-tools-todo.md)、[object-url TODO](agent-object-url-todo.md)（路径解释 D1–D3）。上一版（只有生命周期 + 两视角）的内容全部并入，编号 R-C / R-U / R-M / R-D 保持不变，新增 R-W（工作模型）与 R-K（记录模型）。

## 0. 工作模型（方向）

### 0.1 出发点（owner 的设计思考，2026-11-04 原文要点）

1. chat session 中，Agent 总是在**自己的私有 runtime** 里调用命令、访问 `cyfs://` 等 BuckyOS 路径。用户给的传统绝对路径结合上下文尝试，一般从 owner 的默认 runtime 开始。
2. 用户创建 chat session 时可以选一个 runtime 作为该 chat 的 default runtime（先不支持）。
3. 用户明确要求时，`analyze_intent` 建的 work session 用 default runtime 去工作；这样 chat session 不用背着一大堆 runtime list。
4. 私有 runtime 在非 chat session 里总是可用：Agent 在私有 runtime 里用 cmd_skills 完成构造，再到 workspace runtime / session runtime 里完成最后的工作。即 work session **绑定了私有 runtime，也绑定了别的 runtime 上的 workspace**。
5. plan 模式能看到所有 workspace 和 runtime，为 work session 填正确的 default runtime 和 workspace。

### 0.2 Review

**同意并作为方向的三点**

- **三级知情**：chat 只知道“私有 + owner 默认”两台；work 知道自己绑的那几台；plan 看全表。这直接解决 R-U2 的矛盾（谁该看到 runtime 表）和 chat 提示词的 cache 稳定问题，也是 2026-10-25 “Workspace 不决定执行位置”那条裁决的自然延伸。
- **私有 runtime 永远在**：cmd_skill 自带的命令、`agent-session` 家族、对象库访问都只需要在一处保证。它把 §3（skills 对 runtime 的要求）里一半的复杂度拿掉了：skill 命令缺省装在私有 runtime，不再需要跟着 work runtime 跑平台矩阵；节点 Agent 家目录 `tools/` 那笔欠账从前置降成可选。
- **两种 session 形态**（2026-11-04 owner 澄清）：session 的 default runtime 决定 `shell()` 在哪执行；**`agent-session` 只在私有 runtime 上有**。所以 default 为私有的 session 是“多 runtime session”，能经 `agent-session run / copy` 访问别的 runtime；default 为 host 等的 session 是“单机 session”，只有 `shell / read / write` 等 runtime 层工具，没有 `agent-session`，不能访问别的 runtime。不存在“双绑定”：binding 不变，节点上也**不再需要安装 helper**（primary-runtime TODO §6.1 那条欠账改为不做）。

**需要先裁定的歧义**（结论见 §0.3，待裁决见 §7）

- “default runtime” 一词在原文里有三个意思：owner 的 `default_runtime_id`、chat session 的可选 default、work session 的默认落点。本文分别叫 **owner 默认 runtime**、**chat 工作 runtime（未来）**、**工作 runtime**（= 现在的 session-runtime / `runtime.frozen`）。
- ~~`shell` 默认跑在哪一侧~~ 已澄清：default runtime 决定 `shell()`；单机 session 里没有 `agent-session`，也就没有“另一侧”。由此 plan session 必须是私有 default（它靠 `agent-session todo` 工作），待裁决 5 一并关闭。
- 绝对路径“从 owner 默认 runtime 开始”与 object-url TODO D2（无来源的 `file:///path` 按 **Zone 级 default device** 解释）是两套默认。本文取：**用户在 chat 里打的路径有来源，来源就是 owner 本人，解释为 owner 默认 runtime 所在设备**；D2 的 Zone default device 只管没有任何来源的持久引用。两者要在 key url.md 里写成一段。
- “chat 总是私有”和现状冲突：Windows / macOS Desktop 上 `host_by_default` 让 chat session 跑在 host。改成私有之后，**chat 里直接“截个屏 / 打开那个文件”会变成一个 work**（analyze_intent 建 `--runtime host` 的 work）。这是产品行为变化，接受它，换来 chat 提示词不随 runtime 变。
- Desktop Lite / Agent Service 原生运行（没有容器）时“私有 runtime”是什么：就是 Agent Service 进程自己的环境。现状里这时 `container.exec = null` 被当成不可用，新模型要求它**永远可用**（见 R-W1）。

**代价**：新 helper `agent-session run`、单机 session 的边界写进协议（binding 不升版，节点不装 helper）、三份 behavior（ui / analyze_intent / plan_simple）重写、`freeze_runtime` 按 Session 类别分叉。都在 §6 P0。

### 0.3 定名与规则

| 名字 | 定义 | 现状对应 |
| --- | --- | --- |
| **私有 runtime**（`private`） | Agent Service 进程自己的环境：aios 容器，或原生运行时的宿主进程环境。永远存在、永远可用、声明 `extended`、PATH 含 Agent Bin（cmd_skill 的 `tools/<skill>/bin`）。**一定拥有 `$BUCKYOS_ROOT` 目录结构**（见下“目录约定”第 5 条） | `container` 记录（`exec: native`），`canonical_runtime` 已有 `local → container` 别名，再加 `private` |
| **owner 默认 runtime** | owner 对该 Agent 设的 `default_runtime_id`。语义改为两件事：用户说“在我电脑上”时指的机器；用户给的绝对路径优先在哪解释。**不再决定 chat session 跑在哪** | `AgentSettings.default_runtime_id` → 记录 `default` 标记 |
| **session 的 default runtime**（本文简称**工作 runtime**） | **`shell()` 在哪执行**，cwd 也在它上面。chat：恒为私有。work：创建者选（analyze_intent 只会选私有或 owner 默认；plan 任选）。冻结后不变 | `runtime.frozen`（session-runtime），`freeze_runtime` 三级规则 |
| **workspace** | 项目文件所在的 runtime + 目录，独立于以上三者 | 不变 |

Session 的三处绑定：

```text
default = 私有（多 runtime session）：shell() 在私有；有 agent-session，可 run --runtime <id> / copy 访问任何已登记且可用的 runtime
    chat session : 恒私有；轻量访问 owner 默认 runtime（M7 读路径、短命令），真正的活建 work
    plan session : 恒私有（靠 agent-session todo 工作）；看全部 runtime 与 workspace，为每个 Todo 填 --runtime / --workspace
    work session : 缺省形态；私有构造，经 run / copy 到 workspace runtime 完成最后一步
default = 其它 runtime（单机 session）：shell() 在那台机器；只有 runtime 层工具，没有 agent-session，不能访问别的 runtime
    work session : 用户说“在我电脑上做”或 plan 指定；材料与命令都在那台机器上时才这样选
workspace     : 任意可用 runtime 上的目录，与以上独立（现有）
```

规则：

- **M1 chat 恒私有**。`freeze_runtime` 对 UI / chat 类 Session 固定选私有，`selected_by: private`；不读 `default`。R-W2。
- **M2 chat 的 runtime 知识只有两行**：私有（含 baseline / platform）与 owner 默认（id、os、是否可用、ComputerUse 档位）。不列其它 runtime；要看全表的事交给 plan。R-W3。
- **M3 analyze_intent 只做二选一**：不写 `--runtime`（= 私有）或用户明确要求时 `--runtime <owner 默认>`；用户点名别的机器、或任务需要多台，建 `--class plan_simple` 让 plan 去选。R-W4。
- **M4 多 runtime 访问只属于私有 default 的 session**：`agent-session run --runtime <id|workspace> -- <cmd>`（新，走宿主侧 R-U3 的 `runtime_exec`）与现有 `agent-session copy --from <session|workspace|id>:<path>`。单机 session 没有 `agent-session`，节点上不装 helper；它要的材料由创建者（plan / analyze_intent 所在的私有 session）事先放到位，或 workspace 就在那台机器上。R-W5。
- **M5 cmd_skill 只声明适配平台与基线**（现有 `baseline` / `platforms`，不加字段）；冻结按 session default runtime 过滤（现有）。基线 = 一次装好的 ExtTools 包，全平台同一组命令（§3，R-S1）。
- **M6 plan 看全表并逐 Todo 填位置**：`<runtimes>` 块（R-U2 的表）+ workspace 列表进 plan_simple 首个输入；Todo 的 `--runtime` 缺省私有（即继承 plan 自己的），`--workspace` 缺省 `inherit`。R-W7。
- **M7 用户给的绝对路径**：chat 里出现的 `/Users/...`、`C:\...` 一类路径，按 owner 默认 runtime 的设备解释为 `file://<device_id>/...`（D1），读取经 read 工具 / `agent-session copy --from <owner 默认>:<path>`；owner 默认 runtime 不可用或路径不存在时再试私有，并把实际解释写进回复。R-W8。
- **目录约定**（2026-11-04 owner 列出，四条就够，不加字段）：
  1. 没有 workspace 时 session 有目录可用，在 Agent RootFS 下（现有 SessionDir）。
  2. Agent 的私有 workspace 建在 Agent RootFS 下（现有容器 roots 建议的第一项 `<agent_root>/workspaces`）。
  3. 协作类 workspace 用户不指定时放 BuckyOS owner 的 home（容器内 `/home/<owner>`；host 上是 OS 账户 home）。
  4. Agent RootFS 本身在 owner 的 home 下（`/home/<owner>/agents/<agent_id>`，path_usage.md）。
  5. **私有 runtime 一定拥有 `$BUCKYOS_ROOT` 目录结构**（owner 强调）：容器内是 `/opt/buckyos`，原生运行（Desktop Lite）时是 BuckyOS 安装根。布局固定：`bin/`（标准命令）、`data/home/<owner>/`（owner home，含 `agents/<agent_id>` 即 Agent RootFS）、`instance/`（可写持久卷：`pkg/`、`cache/`、`tools/bin`、`tools/<agent_id>/<sid>`）、`tools/`（ExtTool 只读卷：`store/`、`deno-cache`）、`libexec/buckyos-tool/`、`/config` 投影（path_usage.md）。所以私有 runtime 上一切路径都可以相对 `$BUCKYOS_ROOT` 推出来，standard / extended 基线、Agent RootFS、Session 目录、持久卷在哪都不用探测。**这是私有与其它 runtime 的本质区别**：host / 节点 / 设备只有执行器和一个账户 home，不要求有 `$BUCKYOS_ROOT`（跑了 node-daemon 的节点恰好有一份，form B 设备没有）。
  合起来一句话：**Agent 在任何 runtime 上的目录都在该 runtime 上 owner 的 home 下**；私有 runtime 的 owner home 就是 `$BUCKYOS_ROOT/data/home/<owner>`（容器内映射为 `/home/<owner>`）。私有 = `<owner home>/agents/<agent_id>`；节点 = `<OS 账户 home>/.opendan/<agent_id>`（今天写作 `<roots[0]>/.opendan/<agent_id>`，因为节点策略的种子 roots 就是账户 home）。`roots` 只是权限边界，不是定位 Agent 目录的锚点；待裁决 12 据此关闭。
- **M8 chat 工作 runtime 可选（先不支持）**：`SessionSpec.runtime.requirement.runtime_id` 已能表达，只是 UI / 创建入口不开；开放时它覆盖 M1，并同时成为该 chat 内 M3 / M7 的“owner 默认”。R-W9，不排期。

### 0.4 与现状的差异

| 现状 | 目标 | 落点 |
| --- | --- | --- |
| UI session 按 `freeze_runtime` 三级规则取 owner 默认（Desktop 上是 host） | 恒私有（M1） | `api.rs freeze_runtime`、`template.rs [session.ui]` |
| 提示词 `<runtime>` 渲染当前 session-runtime 的 `RuntimeInfo` + workspace；要看别的得 `agent-session runtime list` | chat：私有 + owner 默认两行；plan：全表 `<runtimes>`；work：工作 runtime + 私有一行 + workspace | `runner/assembler.rs`、`runtime_status` 生成处 |
| analyze_intent 手册教模型 `runtime list` 后自己挑 | 二选一（M3），复杂的交给 plan | `behaviors/analyze_intent.toml` §Workspace and runtime |
| plan_simple：Todo 缺省继承 plan 的 session-runtime；plan 的 runtime 又来自创建者 | plan 恒私有；Todo 缺省私有；plan 用全表填 `--runtime` | `behaviors/plan_simple.toml`、`runner/todos.rs` |
| cmd_skill 按 session-runtime 的 baseline / platform 过滤；`extended` 在 host / 节点上只是声明，没有安装器 | 过滤不变；补 ExtTools 全平台包与安装器（R-S1） | `baseline.rs`、`runtime/mod.rs`、publish/exttool |
| `agent-session copy` 支持 `session: / workspace: / <runtime_id>:`；没有跨 runtime 执行；节点 Session 期望将来装 helper（§6.1 欠账） | 新增 `agent-session run --runtime <id|workspace>`；helper 只在私有，节点不装 | `transfer.rs`、`xagent_cli.rs`、`session.helper`、Session Directory Protocol §9 |
| 原生运行时 `container.exec = null`，私有不可用 | 私有永远可用，`connector: native` | `protocol/runtime.rs builtin_runtimes`、Loader |
| binding `/5`：runtime_id + workdir + workspace | `/6`：加 `private {workdir}`（控制侧 SessionDir） | Session Directory Protocol §9、`runtime/mod.rs` |

### 0.5 在非工作 runtime 上执行命令

Agent 让命令跑到“不是当前 default runtime”的机器上，只有四条路。前三条是产品面，第四条是现有 runtime 层工具，R-C8 之后收敛进第二条。**第二、三条只在私有 default 的 session 里存在**（`agent-session` 只装在私有 runtime）；单机 session 只有第一条，由创建者替它决定。

| 路 | 谁用 | 粒度 | 现状 |
| --- | --- | --- | --- |
| 建 work session 时 `--runtime <id>`（analyze_intent 的 `create-worksession`、plan 的 Todo `--runtime`） | chat → work，plan → Todo | 整个 Session，冻结后不变 | ✅ |
| `agent-session run --runtime <id\|workspace> [--cwd <dir>] [--timeout <s>] -- <cmd>` | 私有 default 的 session（chat 轻量、plan 只读、work） | 单条命令 | ❌ R-W5 |
| `agent-session copy --from <session\|workspace\|id>:<path> --to …` | 私有 default 的 session | 文件 | ✅ |
| `executor_call` 工具（agent_tool runtime 层，对在线 daemon 目标） | 现有 ComputerUse / daemon 路径 | 单个 executor op | ✅，但不经 runtime 登记与 grant |

`run` 的规则（R-W5 的细则）：

- **目标**：`workspace` 解析为本 Session 绑定的 workspace 所在 runtime；`id` 必须是本 Agent 登记、`available` 且 `grant.ops ∋ Exec` 的 runtime。不可用拒绝不回退，拒绝信息同 R-U5。
- **cwd**：`workspace` → workspace 目录（只读 workspace 仍可跑，但写文件由执行器按 grant 拒）；`id` → **必须给 `--cwd`**，且在该 runtime 的 `grant.roots` 内；不在目标上替它建 Session 目录。
- **环境**：目标 runtime 自己的 PATH 与 shell（`caps.shell`）；不把私有侧 PATH、Agent Bin、cmd_skill 命令目录带到别的机器（与 binding §9 的规则一致）。Agent Bin 里 skill 自带的命令和 `agent-session` 只在私有 `shell()` 里可用，不能 `run` 到别处。
- **执行**：经 `session.helper` 到宿主的 `runtime_exec`（R-U3），记录在 `runtime_ops` 目录，有调用 id；超时后转 task，`run` 返回 task id，`agent-session task wait|cancel` 跟进；输出截断规则同 `shell`。Session 恢复后按调用 id 查询，不重放（与 ComputerUse 草案“未知结果不盲重放”同一条）。
- **不改变绑定**：`run` 不写 `binding.json`，不换 default runtime，不改 Session 的 cwd；每次调用独立做 grant 检查并写 worklog（哪个 runtime、什么命令、调用 id）。
- **不改公共 tool schema**：`shell` 工具不加 `runtime` 参数，全部走 `agent-session run`。
- **chat 也有这条路**（它是私有 default）：机制上不限制；`ui_core.md` 引导为 M7 的读文件与短命令（看一眼目录、取一个文件），持续的活建 work（M3）。待裁决 17 据此关闭。
- **与 workspace 的关系**：私有 session 的 workspace 在另一台 runtime 上时，典型序列是 `copy` 材料进来 → 私有侧构造 → `copy` 回去 → `run --runtime workspace -- <构建/校验命令>`。这是 §0.1 第 4 条“私有构造、目标完成”的具体形态。
- **单机 session 的材料**：它不能 `copy`，所以 plan 把 Todo 放到非私有 runtime 时，要么 workspace 就在那台机器上，要么先用一个私有 Todo 把材料 `copy` 过去（Todo 链），并在 Todo 正文写明路径。

### 0.6 用户入口：两步流程

**目标流程**（owner 的设计）：

1. 用户管理**自己的** agent runtime：一张用户级列表，每条能看到被分配给了哪些 Agent、各自拿到什么 grant。
2. 在 Agent 的 home page 看**这个 Agent** 的 runtime 使用情况：当前列表里的，和历史上用过的。

**现状**：runtime 进入 Agent 视野的入口有六处，分属两个产品、三种对象，互相不通：

| 入口 | 在哪 | 做什么 | 问题 |
| --- | --- | --- | --- |
| 节点开放（`owner_agents` 策略） | node_daemon 启动时 `seed_owner_agents` 种下；之后只能改 system config | 决定哪些节点对 owner 的 Agent 开放、roots、ComputerUse 上限 | Desktop 没有 UI（`grep owner_agents` 在 desktop/src 只命中 API 类型）；owner 在自己的 Desktop 上既是管理员又是用户，却改不了 |
| 添加设备（daemon URL，form B） | Desktop → Settings → Cluster Manager → Add device（`AgentDevices.tsx`，`executor.device.*`） | 签发 URL；表单里**当场选一个 Agent**（`profiles.<p>.principals = [agent]`）、roots、只读 / 读写、ComputerUse、有效期；列表有在线 / 到期 / Reset URL / Revoke / Delete | **不写任何 Agent 的 `settings.runtimes`**（代码里没有写 `AgentRuntimeTarget::Executor` 的地方）；Agent 只能经 `executor_list` 工具看到它，Session 绑不了；分配关系藏在 executor profile 的 `principals` 里 |
| Agent 创建向导 | Desktop → 新建 Agent → runtime 步 | `agent.runtime_directory` 取目录，只处理 `host`（开 / 关 + 节点 ComputerUse 上限），`apply_create_defaults` | 目录里的其它节点不可选；创建后没有同等的编辑页 |
| Agent 详情（Desktop） | users-agents → `AgentDetailPage` runtime 卡片 | 只读展示 `settings.runtimes` 与 `default_runtime_id` | 不能改；`agent.update` 的 `runtimes` 参数只有向导在用 |
| opendan WebUI | Workspaces 页 `RuntimeList`（`runtimes.settings_set`） | 对已有条目启停 / 默认 / ComputerUse / 声明基线 / `check` | 明确不能新增目标（“control panel adds runtimes”）；放在 Workspaces 页下，不在 Agent 首页 |
| CLI | `buckyos agent list\|get\|profile`、`buckyos executor list`、`xagent runtime register`（开发用） | — | `buckyos` 没有 runtime 模块；`remote_ssh` 只能用 `xagent` 直接登记到某个 Agent 的 Agent State，用户级看不到 |

**历史使用**：Session 的 `runtime.frozen` 在 `session_config` 里（SessionDetail 的 “Runtime & hosting” 面板能看单个），但 `SessionSummary` / sessions 列表不带 `runtime_id`；Workspace 记录带 `location.runtime_id`；没有按 runtime 聚合的“哪些 Session / Workspace / skill 用过它”视图，`runtimes.impact` 也还没有（R-D1）。被 `unregister` 的记录直接消失。

**“分配给了哪些 Agent”**：`agent.runtime_directory` 是 owner 级目录 × 单个 Agent 的 settings 叠出来的视图，没有跨 Agent 的反查；要知道 host 开给了哪几个 Agent，只能逐个 Agent 看。

**结论：要改，而且是把“对象”理顺，不只是加页面。** 现状的层次其实已经是两层（用户级的节点 / 设备 → 每个 Agent 的选择 → Agent State 投影），缺的是：用户级没有一张表、daemon 设备没有走进第二层、Agent 侧没有历史。目标模型：

```text
用户级 runtime 目录（owner 一份）           ← 第 1 步在这里管
  ├─ 节点（owner_agents 策略开放的）          来源：system config nodes/<n>/settings
  ├─ 设备（daemon URL，form B）               来源：executor 记录（含到期）
  └─ ssh 登记（从 Agent State 上提到用户级）  来源：新增 users/<owner>/agent_runtimes/<id>
每条：connector、caps 上限、grant 上限、assigned_to: [{agent_id, enabled, grant, default}]

分配 = AgentSettings.runtimes[id]（存储不变）    ← 两边都能改：runtime 页“分配给 Agent”，Agent 页“启用 runtime”
  → Agent State 投影（§1 记录）                 ← 第 2 步看这里 + 使用账本
```

两步流程下各入口的去向：

| 入口 | 去向 |
| --- | --- |
| 节点开放策略 | 进用户级目录页：zone owner 在 Desktop 上能开 / 关节点、改 roots 与 ComputerUse 上限（写 `owner_agents`，R-E2） |
| 添加设备 | 拆成两步：签发 URL 时 Agent **可选**（不选只签发；选了就是签发 + 分配连着做）；profile 的 `principals` 由分配推导，与节点的 scheduler 做法一致；“分配给 Agent”是目录页上的动作，写 `settings.runtimes[exec-<id>] = {target: Executor}`（R-C8） |
| 创建向导 | 不变，但目录里的任何节点 / 设备都能在向导里勾（不止 `host`） |
| Agent 详情（Desktop） | 保持只读摘要，加链接跳到 opendan 的 Agent Home runtime 面板（编辑在那里做） |
| opendan WebUI `RuntimeList` | 搬到 Agent 自己的 Home page，成为“这个 Agent 的 runtime”面板：当前（§5.3 列）+ 历史（R-E5）+ 可编辑；这是第 2 步唯一的落点，Desktop 详情页只链过来；Workspaces 页只留 workspace 与 runtime 的关联 |
| CLI | `buckyos runtime list\|add-ssh\|assign\|unassign\|revoke`（用户级）与 `buckyos agent runtime list\|enable\|disable\|default`（Agent 级）；`xagent runtime register` 保留给开发 |

## 1. 记录模型：connector + caps + grant（去掉 kind）

`RuntimeKind { Container, HostNative, RemoteNode, RemoteSsh }` 在 lib_opendan / control_panel 的约 37 处使用全部只问 `is_node()`（走进程内还是走 Node Executor）；没有一处区分 `HostNative` 与 `RemoteNode`。这一个 bit 在 `exec`（agent_tool `RuntimeConfig.kind`）里本来就有。kind 剩下的只是语义暗示，而暗示用字段说更准。第二轮提的 `role` 犯同样的错（也是“几个字段的常见组合”），降为 UI 分组 / 登记预设，代码不读。

| 块 | 回答什么 | 谁写 | 代码分支 |
| --- | --- | --- | --- |
| `connector` | 怎么连：`native`、`node_executor {node_did \| executor_id, profile_id}`、`ssh {host, user, port, identity_file}` | 投影 / 登记 | 是（唯一合法的 type tag） |
| `caps` | 这台机器能做什么：`os / arch / shell`、`desktop: bool`、`baseline`（声明）、`baseline_check`、`tools_check`（核验过的命令） | `check` 学来；baseline 是声明 | 是（skill 候选、手册过滤） |
| `grant` | 允许本 Agent 做什么：`ops`、`roots`、`home`、`computer`、`install: none \| agent_bin \| system`、`workspace: bool`、`read_only`（由 ops 推）、**`expires_at_ms`** | 节点策略上限 × owner 缩窄 × daemon URL 期限，投影时算交集 | 是（freeze、`workspaces.create`、skill 安装、到期） |
| `desc` | 给人和 LLM 的一句话：“主人的 Windows 工作机，与主人共用，可控屏，到 11-10” | owner / 投影模板 | **否** |

保留：`runtime_id`、`name`、`builtin`、`default`、`availability`、`revision`、`registered_by`。`private` 是 `connector: native` 的那条内建记录。

| # | 需求 | 现状 |
| --- | --- | --- |
| R-K1 | `runtime_record` 版本 4：去 `kind`，加 `connector / caps / grant / desc`；旧记录按 kind 推 connector（Container → native，HostNative / RemoteNode → node_executor，RemoteSsh → ssh）；`FrozenRuntime.kind` 同步换成 `connector` | ❌ |
| R-K2 | `is_native_runtime` / `is_node()` 改读 connector；37 处机械替换；`binding.json` 的 `kind` 字段改 `connector` | ❌ |
| R-K3 | caps 与 grant 不合并：caps 是探出来的事实，grant 是人给的授权（daemon TODO §5.4 末条：capability 是声明不是授权，范围取交集） | 原则 |
| R-K4 | `grant.expires_at_ms` 对所有来源统一：daemon URL 的 `expires_at` 投影进来；owner 也能给 host 一个有期限的 grant（“这周允许装东西和控屏”）；到期处理见 R-M7 | ❌（只有 executor 记录有期限） |
| R-K5 | `grant.roots` 只表示执行器强制的权限边界（容器为空表示不限）；Agent 目录按 §0.3 目录约定定位，**不加 `home` 字段、不迁移**。节点侧把“账户 home”从 `describe` 拿（`ExecutorRuntimeInfo` 现在只有 os / arch / hostname / shell / path，加 `home`），不再取 `roots[0]`；`home` 不在任何 root 内时绑定拒绝并提示管理员。`remote_ssh` 登记要求显式 `--root` | ❌ |
| R-K6 | 登记 / 签发表单的**预设**（不是强制）：`device_type ≠ desktop` 的节点或设备预填 `ops = 文件读 + computer_observe`、`workspace = false`、`install = none`；desktop 节点与私有预填 `workspace = true`；管理员 / owner 可改 | ❌ |
| R-K7 | `xagent runtime register --kind` 改为 `--connector`；WebUI RuntimeList、Agent State Protocol §10、Workspace Protocol 中的 kind 引用同步 | ❌ |

## 2. 现状一页纸（2026-11-04 工作树）

runtime 对 Agent 来说分三层，每层的写入方不同：

| 层 | 内容 | 谁写 | 落点 |
| --- | --- | --- | --- |
| 目录与选择 | zone 里哪些节点对 owner 的 Agent 开放（`owner_agents` 策略 → `NodeRuntimeOffer`：id、node_did、os、ComputerUse 上限、roots）；owner 对每个 Agent 的选择 `AgentSettings.runtimes[id] = {enabled, computer, target, baseline}` 与 `default_runtime_id`；`target` 有 `Node {node_id, node_did}` 与 `Executor {executor_id}`（daemon URL，form B）两种 | control panel `agent.runtime_directory` / `agent.update`；opendan `runtimes.settings_set`（只改已有条目） | system config（AgentSettings） |
| 登记（投影） | `state/runtimes/<id>.json`：kind、exec、roots、platform、baseline、availability、`default`、`computer`、`baseline_check` | Loader `project_runtimes` → `ensure_builtin`（启动与 settings 变化时幂等补建 `container`、`host`、`node-*`）；`check_runtimes` 后台核验标准工具；owner 可手工 `runtimes.register` 非内建的 `remote_ssh`；`Executor` 目标投影出的记录标 “not wired into Sessions yet” | Agent State |
| 使用 | Session 创建 `api::freeze_runtime`：显式 `--runtime` > 继承父 Session > `default`；不可用拒绝，不回退；同一 Session 不换 runtime。Workspace 独立选 runtime，跨 runtime 用 `agent-session copy`（`session: / workspace: / <runtime_id>:` 前缀）。节点 Session 的 cwd 是 `<roots[0]>/.opendan/<agent>/sessions/<sid>`，libopendan 每次绑定把 `session.json` 与 token 写过去；节点 PATH 不含控制侧的 Agent Bin | libopendan | `session_config.runtime.frozen`、`binding.json /5` |

入口现状：
- WebUI（opendan）Workspaces 页 `RuntimeList`：启停、默认、ComputerUse、声明基线、`check`；不能新增目标。Desktop Agent 详情只展示。`buckyos` CLI 没有 runtime 模块；`xagent runtime list|get|check|baseline|register|update|unregister` 是 Session 内 / 开发用 CLI。
- Agent 自己看得到什么：提示词 `<runtime>` 只有当前 session-runtime 的 `RuntimeInfo` + workspace；其它 runtime 要主动 `agent-session runtime list`（ui_core、analyze_intent、plan_simple 三份手册都这样教）。`## Commands` 末尾列出因基线 / 平台不满足的手册。Todo 可写 `--runtime <id>` / `--workspace`，PLAN 紧凑视图显示每个 Todo 的 runtime 与基线。
- 已知欠账（primary-runtime TODO §10.4）：节点 Session 没有 `<agent_root>/tools/<skill>/bin`；`skill install` 在节点 Session 里不可用；Windows / macOS 实机未验证；真实 Zone DV 未跑。

## 3. cmd_skills 与 runtime

**原则**（2026-11-04 owner 裁决）：cmd_skill 只声明**适配的平台与基线**，就是现有 front matter 的 `baseline: standard | extended` 与 `platforms`，不发明新字段。基线的本质是**简化 cmd 安装**：在一个 runtime 上装一次 ExtTools，就得到一打命令，而且全平台一致；官方 skill 按基线写手册，不逐个 skill 往 runtime 上装命令。

命令来自三层：

| 层 | 内容 | 谁装 | 现状 |
| --- | --- | --- | --- |
| `standard` | 28 个 Agent 标准命令（`agent_tool`、`agent-session`、`xagent`、`buckyos`、AICC 命令…，清单 `baseline::STANDARD_TOOLS`） | 私有：`$BUCKYOS_ROOT/bin/`（镜像或安装根自带）；host / 节点由 node-daemon 的 `install-agent-tools` 布到其 buckyos root 的 `bin/agent-tools` 并前置 PATH | ✅ |
| `extended` = standard + ExtTools 包 | ffmpeg / ffprobe、ImageMagick、poppler、tesseract、pandoc、python / uv、node、deno…（Session Runtime 文档 §3.2 / §3.3） | 私有：aios 镜像预装 + `$BUCKYOS_ROOT/tools` 只读卷（Lite 原生时同一位置）；**host / 节点没有安装器**，`extended` 现在只是 owner 的声明 | ⚠️ 只有容器 |
| skill 自带 | `tools/<skill>/bin` | `skill install` 复制进 Agent Bin（私有）；节点家目录见 R-U4 | ⚠️ 私有有，节点没有 |

由此 §3.6.1 C “cmd 装到哪个 runtime”收窄成两件事：

| # | 需求 | 现状 |
| --- | --- | --- |
| R-S1 | **ExtTools 全平台包 + 安装器**：`runtime ext-tools install <id>`（owner 在 Agent Home / CLI 触发，经 R-U3 的 `runtime_put` / `runtime_exec` 到目标 runtime），Linux / macOS / Windows 各一份清单、同一组命令名；装完自动 `check`，`baseline_check` 反映实际；owner 声明 `extended` 的规则不变（声明与探测分离），安装器只是让声明成立的手段。落点：私有与跑了 node-daemon 的节点装到各自 `$BUCKYOS_ROOT/tools/`（容器上即持久卷，R-U9）；form B 设备没有 `$BUCKYOS_ROOT`，首期不支持安装，baseline 只能声明 | ❌ |
| R-S2 | 官方 skill 只用 `standard` / `extended` 清单里的命令，手册 `requires` 写具体命令、`platforms` 写适配平台；第三方 skill 自带 `install.toml` 脚本的路径保留为例外（§3.6.1 C），不是主路径 | ⚠️（手册已这样写，清单未固化） |
| R-S3 | `extended` 清单机器可读（`EXTENDED_TOOLS`，对应 R-S1 的包内容），`check` 用它核验，plan 全表显示 | ❌（只有 `STANDARD_TOOLS`） |

冻结规则**不变**：按 session default runtime 的 `baseline` / `platforms` 过滤（`unmet_on`），不满足的不注入并说明。`agent-session` 的特殊性（§0 第 2 点：只在私有 session 里使用）不靠 skill 字段表达：用到它的官方手册只有 `ui_core`（`scope: ui`，UI session 恒私有）与 `opendan_agent_dev`；它在 `STANDARD_TOOLS` 里且被布到 host 这件事怎么处理见待裁决 22。

安装记录永远写实际 runtime_id（§3.6.1 的 `pinned_runtime = null` 约定取消）；`pinned_runtime` 只对带自装命令的 skill 有意义。

| 需要 | 现状 | 缺口 |
| --- | --- | --- |
| 在指定 runtime 上装 ExtTools / 跑安装脚本并核验 | 只有 Session 驱动路径与宿主基线探测 `run_probe` | Agent 级原语 R-U3，安装器 R-S1 |
| 把文件放到目标 runtime | `RuntimeFiles` 做 Session 间复制 | `runtime_put`（R-U3）；节点家目录（R-U4） |
| runtime 变化反映到 skill 状态 | `availability` / `baseline_check` 变，无订阅者 | R-M2 |
| 创建 Session 时按 skill 落点选 runtime | 三级规则不认识 skill | R-U1，只对带自装命令的 skill |

## 4. 分环节需求

编号 R-W（工作模型）、R-C（创建）、R-U（使用）、R-M（更新）、R-D（删除）；“现状”标 ✅ 已有 / ⚠️ 部分 / ❌ 没有。

### 4.1 工作模型（§0 的规则落成需求）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-W1 | 私有 runtime 永远可用：内建记录 `private`（别名 `container` / `local`），`connector: native`；原生运行时也如此，不再 `exec = null`；`availability` 恒 `available`；`caps.baseline = extended`（容器）或 owner 声明（原生）。私有 runtime **必须拥有 `$BUCKYOS_ROOT` 布局**（目录约定第 5 条）：Loader 启动时核验 `bin/`、`data/home/<owner>/agents/<agent_id>`、`instance/`、`tools/` 存在，缺了是部署错误，不是“不可用” | ⚠️（容器时可用；原生时不可用） |
| R-W2 | M1：UI / chat 类 Session `freeze_runtime` 固定私有，`selected_by: private`；`SessionSpec` 需要知道类别（UI 模板建的 Session 带 `class: ui`） | ❌ |
| R-W3 | M2：chat 的 `<runtime>` 块两行（私有、owner 默认），其余 runtime 不出现；`ui_core.md` 删掉“看 runtime list 自己挑”的指引，改为“要在别的机器上做的事建 work” | ❌ |
| R-W4 | M3：`analyze_intent.toml` 的 runtime 段改为二选一；结果 schema `runtime_id` 允许 `private`；用户点名其它机器或多台 → `--class plan_simple` | ❌ |
| R-W5 | M4：`agent-session run --runtime <id\|workspace> [--cwd] [--timeout] -- <cmd>`：经 `session.helper` 到宿主的 `runtime_exec`（R-U3）；输出与退出码原样回传，长命令走 task；只在私有 default 的 session 里存在；细则见 §0.5 | ❌（copy 有 `<runtime_id>:`；无 run） |
| R-W6 | M5：不加 skill 字段，冻结规则不变；需求转为 §3 的 R-S1 – R-S3 | — |
| R-W7 | M6：plan_simple 首个输入带 `<runtimes>` 全表（列见 §5.3）+ workspace 列表；Todo `--runtime` 缺省私有；手册改写 | ❌（R-U2 原需求并入） |
| R-W8 | M7：chat 里绝对路径的解释规则写进 `ui_core.md` 与 key url.md；read 工具接受 `file://<device_id>/` 后，私有侧读 owner 默认 runtime 的文件经 `agent-session copy --from <id>:<path> --to session:<f>` 或 read 直接取 | ❌（依赖 object-url TODO A 段） |
| R-W9 | M8：chat 工作 runtime 可选，先不做；只保证 `freeze_runtime` 的 UI 分支尊重显式 `requirement.runtime_id` | 不排期 |
| R-W10 | 单机 session 的边界写成协议：binding 不升版；节点上**不安装** `agent-session` helper（关闭 primary-runtime TODO §6.1 与 Session Directory Protocol §9 “helper 需在节点上安装”的表述）；节点 `.opendan/session.json` 与 token 保留，给标准工具（`buckyos`、AICC 命令）取服务身份用；单机 session 的 `report` 走 runner 的 report 工具（不依赖 helper），产物按现有节点读取路径上收 | ⚠️（现状即如此，但文档写的是“将来装 helper”） |
| R-W11 | 行为上的后果写进手册：chat 里的“截屏 / 打开文件 / 在我电脑上跑”都是 work（`--runtime <owner 默认>`），不是 chat 自己做 | ❌ |

### 4.2 创建（runtime 进入 Agent 的视野）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-C1 | 三种来源保持不变：系统投影的内建（`private`、`host`、`node-*`、`exec-*`）、owner 手工登记（`remote_ssh`）、skill 安装过程**不**产生新 runtime | ✅ |
| R-C2 | 每条登记在可用时必须有完整的 `caps.platform` 与 `baseline`；`register` / 投影后立刻 `check` 一次 | ⚠️ 投影后有后台 check；手工 `register` 没有 |
| R-C3 | `caps.tools_check`：除标准工具外，记录“这个 runtime 上核验过存在的命令”（来源：skill 安装核验、显式 `check --commands`），结构同 `baseline_check` | ❌ |
| R-C4 | owner 能从 WebUI / `buckyos` CLI 新增与登记 runtime：目录里的节点（`agent.update`）和手工 `remote_ssh`（`runtimes.register`） | ⚠️ |
| R-C5 | Agent 自己不能登记 runtime。**申请流程先不做**（已裁决）：候选为空时只在拒绝信息里说“没有可用的 <os> runtime，请 owner 在目录里开一个”（R-U5） | 不排期 |
| R-C6 | 默认 runtime 规则不变（`host_by_default`）；语义改为 §0.3 的“owner 默认”；owner 改默认时提示“装在旧默认上的 skill 命令 / ExtTools：N 个” | ⚠️ |
| R-C7 | 登记预设（原 `role`）：WebUI / CLI 登记时的四个模板 `agent_home / owner_desktop / managed_device / temporary` 只是 grant 的预填，不入记录；列表按 §5.1 四类分组展示，分组由 connector + grant 推导 | ❌ |
| R-C8 | form B executor（daemon URL）接入 Session：`AgentRuntimeTarget::Executor` 投影为 `exec-<executor_id>`，`connector: node_executor {executor_id}`，`grant.expires_at_ms` 来自 URL；`freeze_runtime` / `open_session_env` 走同一条 node_executor 路。daemon TODO §5.4 已做 executor target 与 ledger，缺 runtime 配置与 Session 绑定 | ❌（只有 `executor_call`） |
| R-C9 | 设备能作为执行器添加成功（节点开了 `owner_agents`，或 daemon URL 接入）就进目录，grant 按策略 / 签发的 profile，不另设门槛；Device DID Object Card 类设备（多半经 HA 一类服务接入）走对象库，添加路径不同，不在本文范围 | ⚠️ |

### 4.3 使用（Session、skill、Agent 级操作）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-U1 | work session 的选择规则：显式 `--runtime` > 继承 > **点名的、带自装命令的 skill 的落点**（都装在同一 runtime 才自动取；装在多台且都可用时取默认优先；互相冲突拒绝并列出） > 私有。skill 自装命令缺省装私有，这一级很少触发。`selected_by` 加 `skill` / `private` | ❌ |
| R-U2 | plan 的 runtime 全表 → 并入 R-W7；chat 不要 | — |
| R-U3 | Agent 级执行原语（宿主侧，不依赖 Session）：`runtime_exec(runtime_id, script, cwd, timeout)` 与 `runtime_put(runtime_id, local_dir, remote_dir)`，复用 `run_probe` 的 `runtime_ops` 目录与调用 id，输出存档。R-W5 的 `agent-session run`、skill 安装、`tools_check`、R-M3 都走它 | ❌ |
| R-U4 | 节点上的 Agent 目录 `<OS 账户 home>/.opendan/<agent_id>/`（§0.3 目录约定）：`sessions/<sid>`（已有）、`tools/<skill>/bin`（skill 自带命令）、`ops/`（安装日志）。节点 Session 的 PATH 前置 `tools/*/bin`：节点上没有 helper（R-W10），改由 libopendan 在每次 exec 的 `env_overrides.PATH` 里前置（`ExecutorExecRequest` 已有该字段，不改协议）；cwd 与此无关，由 Session 目录 / workspace 目录决定。**优先级降为 P2**：多数 skill 命令来自 ExtTools | ❌ |
| R-U5 | 不可用时“拒绝不回退”；拒绝信息说清下一步（“runtime host 不可达，最近一次 check …；装在它上面的 skill：…；可以改在私有 runtime 做的部分：…”） | ⚠️ |
| R-U6 | Workspace 与工作 runtime 的关系不变；skill 安装不涉及 Workspace | ✅ |
| R-U7 | 私有 runtime 上的安装就是现在的 Agent Bin 复制 | ✅ |
| R-U8 | 能力投影 = §1 的 `caps / grant`，由投影 / check 算一次；skill 候选、`freeze_runtime`、`workspaces.create`、plan 全表都读它 | ❌ |
| R-U9 | 私有（容器）上的 `system` 安装必须落持久卷：`install.toml` 声明 `writes = [...]`，核验不在 `<instance_volume>` / `<agent_root>` 下的记 `volatile`，容器重启后由 R-M2 的 `tools_check` 重核验。**ExtTools 清单（R-S3）里已有的命令拒绝由 skill 自己装**：报“由 ExtTools 提供，手册改声明 `baseline: extended`” | ❌ |
| R-U10 | `workspaces.create / import` 按 `grant.workspace` 拒绝；拒绝信息给出可用的 runtime（“临时 runtime 上不建长期产物；在 host 建 Workspace，用 `agent-session copy` 搬文件”） | ❌ |
| R-U11 | 只读 runtime（`grant.read_only`）：需要写的 work `freeze_runtime` 时拒绝；plan 可用；`--runtime` 点名只读 runtime 的 Todo 留 pending 并记录原因 | ❌ |
| R-U12 | `agent-session copy` 与 `run` 的权限按目标 runtime 的 grant 判定（roots、ops），私有侧按 Agent 自己的目录规则；两侧都记进 `state/transfers` / `runtime_ops` | ⚠️（copy 有记录；run 没有） |

### 4.4 更新（设置、可用性、机器本身变了）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-M1 | settings 变化（启停 / ComputerUse / 声明基线 / 默认 / grant 缩窄）→ 投影 → 已有 Session 不变 | ✅（grant 部分随 R-K1） |
| R-M2 | runtime 事件 → skills 状态重算：不可用 → 装在其上的安装记录标 `stale`（别的 runtime 还 `ok` 则 skill 仍 `enabled`；全 stale 降 `unavailable`）；恢复 → 自动 `tools_check`。私有侧的 `volatile` 安装在容器重启后同样重核验 | ❌ |
| R-M3 | `runtimes.check {runtime_id, commands?}` 带命令清单；`buckyos skill check --runtime <id>` 重核验 | ❌ |
| R-M4 | 声明基线从 `extended` 降为 `standard`：不改安装记录，过滤自然变；提示影响的 skill 数 | ⚠️ |
| R-M5 | 节点的账户 home 变了（`describe` 报的 home 与记录不同）才视为“新机器”，安装记录标 `stale`；roots 变化只影响边界，不动 Agent 目录 | ❌ |
| R-M6 | `update` 的 CAS 与 `.locks/runtimes.lock` 保持；`tools_check` 写入走同一把锁 | ✅ |
| R-M7 | `grant.expires_at_ms` 到期：投影为 `runtime_unavailable` + 自动禁用，走 R-D1 影响清单与 R-D2 的 Session 标记；URL 重置（executor_id 不变）恢复可用；记录永久保留为 `retired`（R-E5），不清理 | ❌ |
| R-M8 | 共用机器（owner 的 desktop）：runtime 表与 `desc` 标“与 owner 共用”；`system` 安装确认文案点明“会装到你的电脑上”；ComputerUse 冲突沿用草案 §8.1 租约与人工接管 | ❌（文案层） |

### 4.5 删除（禁用、取消登记、Agent 删除）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-D1 | 内建只能禁用，不能 `unregister`；**私有不能禁用**；禁用前给影响清单（对齐 `workspaces.runtime_impact`）：活跃 Session、绑定的 Workspace、装在其上的 skill 命令、pending 且写了 `--runtime` 的 Todo、它是否是 owner 默认。WebUI 与 CLI 先展示再确认 | ⚠️（Workspace 有 impact） |
| R-D2 | 禁用后的进行中 Session：Loader 投影时标 `runtime_unavailable` 并通知 owner，不自动迁移 | ❌ |
| R-D3 | `unregister`（非内建）：同 R-D1 + `--purge-remote`（删节点上 `.opendan/<agent>/`，缺省不删）；skills 记录里该 runtime 条目删除 | ❌ |
| R-D4 | Agent 被删除：节点家目录清理由 control panel 侧按 `owner_agents` profile 做；skills / runtime 记录随 Agent 目录消失 | ⚠️（需确认 control panel 是否清节点目录） |

### 4.6 入口（§0.6 的两步流程）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-E1 | 用户级 runtime 目录 `agent.runtime_catalog`（每个用户一份）：合并节点 offer（只有 zone owner 有）、daemon 设备、ssh 登记三种来源；非 zone owner 的用户目录里只有自己签发的设备与 ssh；每条带 connector、caps 上限（os、desktop、roots）、grant 上限、到期，以及 `assigned_to: [{agent_id, enabled, computer, baseline, default}]`（扫 owner 全部 Agent 的 settings）。`agent.runtime_directory` 保留为它按单个 Agent 的投影 | ❌（directory 无反查、无设备、无 ssh） |
| R-E2 | zone owner 在 Desktop 目录页上开 / 关节点、改 roots 与 ComputerUse 上限：新 kRPC `node.owner_agents_set`（写 `nodes/<n>/settings.executor.owner_agents`，只允许 zone owner），不动 node_daemon 的 seed 逻辑 | ❌ |
| R-E3 | daemon 设备分两步（已裁决）：`executor.device.create` 的 Agent 选项**可选**，选了就是签发 + 分配一次做完，不选只签发；分配走 `agent.update {runtimes: {"exec-<id>": {enabled, computer}}}`，executor profile 的 `principals` 由 control panel 按分配重算（与节点 `profile_for` 同一思路），重算不重签 URL | ❌（principals 在签发时定死） |
| R-E4 | ssh 登记上提到用户级（已裁决：任何有 Agent 的用户都能加）：`users/<user>/agent_runtimes/<id>`（connector ssh、roots 必填、grant、到期），RBAC 为本人读写、zone owner 可读；分配同 R-E3，只能分给自己的 Agent；Agent State 里的 `remote_ssh` 记录改为投影出来的（builtin 化），`xagent runtime register` 保留给开发并标 `registered_by: dev` | ❌（只在 Agent State） |
| R-E5 | Agent 侧使用账本：`RuntimeRecord.usage {sessions_total, active_sessions, last_used_at_ms, last_session_id, workspaces, skill_commands}`，在 `freeze_runtime` / 绑定 / Workspace 创建 / skill 安装时更新；`SessionSummary` 加 `runtime_id`，sessions 列表可按 runtime 过滤；被撤回 / 取消登记 / 到期的记录**永久保留**（已裁决），改 `lifecycle: retired`（保留 `usage`，`unregister` 的语义即 retire），这就是“历史上用过的”；retired 记录不参与候选、不出现在 plan 全表 | ❌ |
| R-E6 | Agent 自己的 Home page（opendan WebUI Home，已裁决只做这一处）的 runtime 面板：当前列（§5.3）+ retired 区 + 每条的 usage 与最近 Session 链接；启用 / 默认 / grant 缩窄也在这里改（`runtimes.settings_set` 扩到 grant）。Desktop `AgentDetailPage` 的卡片保持只读，加一个跳到该面板的链接 | ❌（RuntimeList 在 Workspaces 页） |
| R-E7 | 创建向导的 runtime 步列出目录里全部可用条目（不止 `host`），缺省勾选按 `host_by_default` | ⚠️ |
| R-E8 | CLI：`buckyos runtime list\|add-ssh\|assign\|unassign\|revoke`、`buckyos agent runtime list\|enable\|disable\|default`；输出与 WebUI 同一视图 | ❌ |
| R-E9 | 分配 / 撤销走 R-D1 的影响清单：撤销前列出该 Agent 在其上的活跃 Session、Workspace、skill 命令；用户级 revoke 一台设备时列出所有被分配的 Agent 的影响 | ❌ |

## 5. 两个分类视角（脑图，2026-11-04）

分类只用于 UI 分组、登记预设和这份文档的讨论，不是记录字段（§1）。

### 5.1 用户视角：四类 runtime

| 类 | 它是什么 | 对应 | 缺省 grant（登记预设） | 期限 |
| --- | --- | --- | --- | --- |
| **Agent 自用** | Agent 自己的电脑 = §0 的私有 runtime，拥有 `$BUCKYOS_ROOT` 布局 | `private`（`connector: native`） | 全部；`workspace = true`；`install = agent_bin + system(持久卷)` | 永久 |
| **主人的主要工作环境** | owner 日常用的 PC；**Agent 和人共用一台机器**（claude code / codex 模式）。没有 PC 的纯娱乐用户不存在这一类 | `host`，及 `device_type = desktop` 的 `node-*` | 节点策略上限；`workspace = true`；`install = agent_bin + system(owner 确认)` | 长期 |
| **可管理设备** | TV、路由器、NAS。鼓励用 Device DID Object Card 管理（`obj://…/devices/<id>` → Card / Profile → 属性、动作、事件），不是开 shell | 跑了执行器的非 desktop 节点；Android TV daemon 未实现（daemon TODO §7.4）；`buckyos/src` 下尚无 `impl DIDObjectServer` | `ops = 文件读 + computer_observe`；`workspace = false`；`install = none` | 长期 |
| **临时管理设备** | 帮朋友修电脑：对方跑 `buckyos_agent_daemon` 用 URL 接入（form B）；或 owner 手工 `remote_ssh` | `exec-<executor_id>`、`remote_ssh` | 按 URL 签发的 profile / `--root`；`workspace = false`；`install = none`；**`expires_at_ms` 必填** | 临时 |

### 5.2 能力视角：六个问题 → 字段

| 问题 | 字段 | 现状与结论 |
| --- | --- | --- |
| computer_use? | `grant.computer`、`caps.desktop` | `none / observe / control` 三档已有；草案决策 5：缺接口时的最后手段。**网络 KVM 不需要新 connector**：它是 `computer = control` 且 `ops` 只含 `Computer*` 的 runtime |
| 能安装 cmd? | `grant.install`、`caps.tools_check` | 主路径是装 ExtTools 包让 runtime 成为 `extended`（R-S1）；skill 自带 bin 进 Agent Bin；第三方脚本是例外。容器 rootfs 层装的东西重启丢，`system` 必须落持久卷（R-U9） |
| read only? | `grant.read_only`（由 `ops` 推） | 现状散在 `AgentPermissionMode` / 节点 `ops` / `WorkspaceAccess` 三处，记录看不到 → R-U11 |
| filesystem? | `grant.roots`（权限边界） | Agent 目录按 §0.3 目录约定，不占字段；roots 现混用“边界”和“建议位置”→ R-K5；WebUI 当“文件夹权限列表”展示，Agent 级只能缩 |
| run cmd? | `ops ∋ Exec` | 不另设字段；Agent 级非 Session 执行是 R-U3 |
| 能否建 workspace? | `grant.workspace` | workspace 本身就是产物、长期维护，只建在私有与 owner 的工作机上 → R-U10 |

### 5.3 plan 全表（R-W7 的 `<runtimes>` 块列定义）

`id / 分组 / os / baseline / 可用 / 默认 / computer / install / read_only / roots / ExtTools 已装 / 装了哪些 skill 命令 / 到期 / desc`。私有一行永远在最前。

## 6. 实施任务（分期）

**P0 工作模型**（§0；先做，别的都建立在它上面）

| # | 任务 | 落点 |
| --- | --- | --- |
| W1 | 私有 runtime 永远可用（R-W1）：`private` 别名、原生运行时 `connector: native` | `protocol/runtime.rs`（`builtin_runtimes`、`canonical_runtime`）、Loader |
| W2 | `freeze_runtime` 按类别分叉：UI 固定私有；work 四级（R-U1）；`selected_by` 加 `private / skill` | `api.rs`、`template.rs`、`session_config.schema.json` |
| W3 | 单机 session 边界落文档与代码注释（R-W10）：Session Directory Protocol §9、primary-runtime TODO §6.1 改为不做；确认 `report` 在节点 session 不依赖 helper | Session Directory Protocol、`runner/reports.rs` |
| W4 | `agent-session run --runtime <id|workspace>`（R-W5）；依赖 R2 的 `runtime_exec` | `xagent_cli.rs`、`session.helper` 路由、`home.rs` |
| W5 | 提示词：chat 两行 / work 工作 runtime + 私有 / plan 全表 `<runtimes>`（R-W3、R-W7） | `runner/assembler.rs`、`runtime_status` |
| W6 | 三份 behavior 与 `ui_core.md` 重写（R-W3、R-W4、R-W7、R-W11） | `behaviors/analyze_intent.toml`、`plan_simple.toml`、UI 模板、`cmd_skills/ui_core.md` |
| W7 | （取消：不加 skill 字段）→ 见 P2 的 R16 | — |
| W8 | 绝对路径解释（R-W8）：与 object-url TODO A 段合并推进，key url.md 补“chat 输入路径 = owner 默认 runtime 设备”一段 | `read_tool.rs`、`doc/key url.md`、`ui_core.md` |

**P1 记录模型**（§1）

| # | 任务 | 落点 |
| --- | --- | --- |
| R1 | `runtime_record` v4：去 kind，加 connector / caps / grant / desc；`tools_check`；`check` 带 `commands?`；`register` 后立即 check（R-K1–R-K7、R-C2、R-C3） | `protocol/runtime.rs`、`state/runtimes.rs`、`schema/runtime_record.schema.json`、Agent State Protocol §10 |
| R2 | Agent 级原语 `runtime_exec` / `runtime_put`（R-U3），封装 `run_probe` + `RuntimeFiles` | `lib_opendan/src/runtime/mod.rs` |
| R10 | grant 交集在 control panel 算好写进 settings（已裁决 8），opendan 投影只读；登记预设（R-K6、R-U8）| `control_panel/agent_runtimes.rs`、`state/runtimes.rs` |
| R11 | form B executor 接入 Session（R-C8）：`RemoteNodeConfig` 加 `executor_id` 分支，`node_exec` 对应改，`expires_at_ms` 投影 | `agent_tool/runtime/node.rs`、`protocol/runtime.rs` |
| R12 | `workspaces.create / import` 与 `freeze_runtime` 读 grant（R-U10、R-U11） | `state/workspaces.rs`、`api.rs` |

**P2 skills 接口**（§3；与 skills TODO §3.6.1 C / D 同步）

| # | 任务 | 落点 |
| --- | --- | --- |
| R3 | 节点家目录 `tools/<skill>/bin` + PATH 前置（R-U4） | `runtime/mod.rs`、Session Directory Protocol、node executor 侧 |
| R13 | 私有 `system` 安装持久性核验（R-U9） | `skills.rs`、`install.toml` schema |
| R16 | ExtTools 全平台包（Linux / macOS / Windows 清单与安装脚本）+ `runtime ext-tools install` + `EXTENDED_TOOLS` 清单（R-S1、R-S3）；[standard Agent 工具 + ExtTools 全平台统一分发方案与实施 TODO（待 Review）](opendan-exttools-cross-platform-todo.md) | `publish/exttool/`、`baseline.rs`、`xagent_cli.rs`、Agent Home 面板 |
| R4 | R-U1 第四级 `skill` | `api.rs` |

**P3 更新与删除**

| # | 任务 | 落点 |
| --- | --- | --- |
| R6 | runtime 事件 → skills 状态重算（R-M2） | `skills.rs`、`loader.rs`、`state/runtimes.rs` |
| R7 | `runtimes.impact` + 禁用 / 取消登记前确认 + `--purge-remote`（R-D1、R-D3） | `home.rs`、`service.rs`、`web/src/pages/Workspaces.tsx`、`buckyos runtime` 模块 |
| R8 | 禁用时标记进行中 Session 并通知 owner（R-D2） | `loader.rs`、`notify` |
| R14 | 到期处理（R-M7） | `loader.rs` |

**P4 入口**

| # | 任务 | 落点 |
| --- | --- | --- |
| R9 | owner 从 WebUI / CLI 新增目录节点与 `remote_ssh`（R-C4）；并入 E5 / E7 | WebUI、`buckyos-tool/cli/modules/` |
| R15 | runtime 列表按四类分组、登记预设、列对齐 §5.3；`remote_ssh` 要求 `--root`（R-C7、R-K5） | `Workspaces.tsx`、`xagent_cli.rs`、`buckyos runtime` |
| E1 | 用户级目录 `agent.runtime_catalog` + `assigned_to` 反查（R-E1）；`node.owner_agents_set`（R-E2） | `control_panel/agent_runtimes.rs`、`agent_mgr.rs`、node settings RBAC |
| E2 | daemon 设备两步化：`principals` 由分配推导；`exec-<id>` 分配写 settings（R-E3，依赖 R11） | `control_panel/agent_daemon.rs`、`agent_runtimes.rs apply_update`、`AgentDevices.tsx` |
| E3 | ssh 登记上提到用户级并投影（R-E4） | `users/<owner>/agent_runtimes` 键与 RBAC、Loader `project_runtimes`、`xagent_cli.rs` |
| E4 | 使用账本与 retired（R-E5）：`usage` 字段、`SessionSummary.runtime_id`、`lifecycle` | `protocol/runtime.rs`、`api.rs freeze_runtime`、`protocol/summary.rs`、`state/runtimes.rs` |
| E5 | Desktop 目录页（Settings → Agent Runtimes，合并 Cluster Manager 的 Add device，Agent 选项可选）、Agent 详情加链接、向导全目录（R-E3、R-E7） | `desktop/src/app/settings/pages/`、`AgentDetailPage.tsx`、`agent-setup/` |
| E6 | opendan Home 的 runtime 面板（当前 + retired + 可编辑）+ Workspaces 页瘦身（R-E6） | `opendan/web/src/pages/Home.tsx`、`Workspaces.tsx`、`home.rs set_runtime_settings` |
| E7 | CLI 两组命令（R-E8） | `buckyos-websdk/cli/modules/runtime.ts`、`agent.ts` |

依赖：W1 → W2 → W3 → W5 / W6 可并行；W4 依赖 R2；R16 依赖 R2；W8 跟 object-url TODO。R1 可与 P0 并行但要在 W2 合并前定 `FrozenRuntime` 的新形状。P2 在 R1 / R2 后；P3 / P4 最后。入口里 E1 / E4 不依赖别的，可以先做（E4 的 `usage` 字段随 R1 的 schema 升版一起）；E2 依赖 R11；E3 依赖 R1；E5 / E6 / E7 在 E1–E4 后。

## 7. 待裁决

工作模型：

1. ✅ 已裁决（2026-11-04）：session 的 default runtime 决定 `shell()` 在哪执行；只有 default 为私有的 session 有 `agent-session`，能经 `run / copy` 访问别的 runtime（§0.5）。
2. **chat 恒私有的产品后果**：Desktop 上“截个屏”从 chat 直接做变成一个 work。接受（建议），还是给 chat 开 M8 的“工作 runtime = host”作为 Desktop 缺省？后者会让 chat 提示词又随 runtime 变。
> 按我们的设计，Desktop Lite用户才会有这种需求，刚好agent的私有runtime就是用户的defaul runtime. 标准产品里buckyos运行在独立的personal server上，提到的桌面一般就是buckyos的用户桌面

   ✅ 已裁决：接受，M1 不变、不开 M8 缺省。Desktop Lite 上私有 = owner 默认，问题不存在；标准产品里“桌面”指 BuckyOS 用户桌面，owner PC 上的事本来就是 work。
3. **绝对路径的默认**：chat 输入路径 = owner 默认 runtime 的设备（建议），与 object-url D2 的 Zone default device 分工写进 key url.md；还是统一到 D2 一个默认？
> 按建议。（另一个todo: 在buckyos体系里app,说到路径的时候都是url格式的或者objid格式的，这样传统的路径格式能区分出来）

   ✅ 已裁决：按建议写进 key url.md（R-W8）。附带规则：BuckyOS 体系内的 app 与工具说到路径一律 URL（`file://<device>/…`、`cyfs://`、`obj://`）或 ObjId，裸的传统路径只会来自用户输入，因此可以按 M7 解释。
4. **原生运行时的私有 runtime**：进程环境本身（建议）；此时 `host` 记录与私有同机，是否仍保留 `host`（建议保留：它代表经 Node Executor、带 grant 的那一侧）。
> 我的想法 host runtime这个词是“实现上的”，agent视角还是自己的私有default和owner的default

   ✅ 已裁决：`host` 只是实现 / 登记上的 id，保留。Agent 视角（提示词、手册、R-W3 的两行）只有两个名字：**你的私有 runtime** 与 **owner 的默认 runtime**（后面括注 id 与 os）；原生运行时两者同机时提示词说明“同一台机器，经不同身份”。
5. ✅ 已裁决（随第 2 点澄清）：plan 恒私有，因为它靠 `agent-session todo` 工作；只读探索要在目标机上做时，是一个单机 Todo。
6. ✅ 已裁决（2026-11-04）：**不加 `runs_on`**。cmd_skill 只声明适配平台与基线（现有 `baseline` / `platforms`）；官方 skill 按基线写，基线靠一次性安装 ExtTools 包满足、全平台同一组命令（§3，R-S1 – R-S3）。

记录模型：

7. ✅ 已裁决：`role` 降为登记预设 / UI 分组，不是字段（R-C7）。
8. ✅ 已裁决：control panel 算好交集写进 settings，opendan 投影只读不再乘（R10 落点在 `control_panel/agent_runtimes.rs`）。
9. ✅ 已裁决：能作为执行器添加成功就进目录，不另设门槛（R-C9，R-K6 改为预设）；Card 设备大概率经 HA 一类服务接入，添加路径不同，不在本文范围。
10. ~~临时 runtime 到期后的记录保留多久~~ → 已裁决（见 21）：永久保留为 retired；远端 `.opendan/<agent>/` 不删，只在 `--purge-remote` 时删。
11. ✅ 已裁决：允许但必须落持久卷；ExtTools 里已有的命令不允许 skill 自己装（R-U9）。
12. ✅ 已裁决（2026-11-04）：以前搞复杂了。不加 `home` 字段、不迁移，按 §0.3 的四条目录约定：Agent 在任何 runtime 上的目录都在该 runtime 上 owner 的 home 下；`roots` 只是边界。唯一要补的是 executor `describe` 报账户 home（R-K5）。
生命周期（沿用）：

13. ✅ 已裁决：节点上没有 helper，PATH 前置走每次 exec 的 `env_overrides.PATH`，不改协议（R-U4）；cwd 是另一件事，由 Session 目录 / workspace 目录决定，shell 的当前目录就是它。
14. ✅ 已裁决：只在 skill 安装、ExtTools 安装与显式 `check --commands` 时跑，`check_runtimes` 后台只核标准清单（R-C3）。
15. ✅ 已裁决：取默认优先；skill 自装命令缺省装私有，这一级很少触发（R-U1）。
16. ✅ 已裁决：先不做；只在拒绝信息里提示 owner 去目录开（R-C5 不排期）。
17. ✅ 已裁决（随第 2 点澄清）：chat 是私有 default，天然有 `agent-session run`；机制上不限制，`ui_core.md` 引导为轻量读取，持续的活建 work（M3）。

入口：

18. ✅ 已裁决（2026-11-04）：签发时 Agent 可选，不选也可以。剩一项技术确认：daemon 侧鉴权只看 profile 的 `principals`，重算后不需要重签 URL。
19. ✅ 已裁决：任何有 Agent 的用户都能加 ssh / 设备；`users/<u>/agent_runtimes` 本人读写、zone owner 可读，只能分给自己的 Agent（R-E4）。
20. ✅ 已裁决：Agent 自己的 Home page（opendan WebUI Home）是第 2 步唯一落点；Desktop `AgentDetailPage` 只读 + 链接（R-E6）。
21. ✅ 已裁决：retired 记录永久保留，`unregister` 的语义即 retire（R-E5、R-M7）。

skills：

22. **方向已定，排期待安装包改造**（owner 2026-11-04）：Desktop Lite 会是主流，私有 runtime 就是 host，所以 **host 要满足 Agent runtime 的 baseline**，`agent-session` 留在标准清单并照常布到 host（它在 Lite 下本来就是私有侧的命令）。不是私有的 host 上它装着但手册不引导。整个构建 / 安装包的修改目标还没定（aiws 那边还有不少“有则用”的逻辑），本轮改造完后综合各基础服务的需求统一调整安装包；R-S1 的 ExtTools 全平台包并入那次调整。难点：ffmpeg 一类命令的全平台化比 Docker 镜像打包麻烦。

## 8. 实施记录（2026-11-04，未提交）

本轮落了 **P0 工作模型 + P1 记录模型**（W1–W6、W8 的文档段、R1、R2、R11、R12 的 runtime 侧、E4 的 `usage / lifecycle`），P2–P4 与 control panel / Desktop / CLI 侧未做，见 §8.3。

### 8.1 改了什么

| 任务 | 落点 | 说明 |
| --- | --- | --- |
| W1 R-W1 | `protocol/runtime.rs`、`opendan/loader.rs` | 内建 `private`（别名 `container` / `local`，`canonical_runtime` 解析），`connector: native`，原生运行（`OPENDAN_SERVICE_MODE=native`）时同样 `available`、`exec = {kind: native}`，baseline 取 owner 对 `host` 的声明；Loader 启动核验 `$BUCKYOS_ROOT` 的 `bin/`、`data/home/<owner>/agents/<agent_id>`、`instance/`、`tools/`，缺了 `log::error`（部署错误），不改可用性 |
| W2 R-W2 R-U1 R-U11 | `api.rs freeze_runtime`、`protocol/config.rs` | UI / chat 恒私有（`selected_by: private`，显式 `requirement.runtime_id` 仍尊重 = R-W9 的保证）；其它：显式 > 继承 > 私有，owner `default` 不参与；`RuntimeSelection` 去 `default`、加 `private / skill`（`skill` 这一级的逻辑是 P2 R4，未做）；只读 grant 拒绝会写的 work（行为 `capabilities.workspace = read_only` 的放行）；创建后 `note_usage(Session)` |
| W3 R-W10 | Session Directory Protocol §9、primary-runtime TODO §6.1 | 单机 session 边界写入协议：节点不装 helper，`.opendan/session.json` + token 保留给标准工具取身份；`report` 本来就是 runner 工具（`runner/reports.rs` 对节点 Session 经冻结的执行器读产物，不依赖 helper），只改文档 |
| W4 R-W5 R-U12 | `xagent_cli.rs cmd_runtime_run`、`runtime/mod.rs runtime_exec` | `agent-session run --runtime <id\|workspace> [--cwd] [--timeout] -- <cmd>`：只在工作 runtime 为私有的 Session 里（否则拒绝并说明单机 session）；`workspace` → workspace 所在 runtime 与目录；id → 必须 `--cwd` 且在 `grant.roots` 内；目标自己的 PATH / shell；grant 到期 / 无 Exec / 不可用拒绝不回退；记录在 Agent 的 `state/runtime_ops/<op_id>.json` 与 Session 的 `state/runtime_ops/<op_id>.json`（带 call id）；超时返回 124 并带已有输出（**“超时转 task”未做**，见 §8.3） |
| W5 R-W3 R-W7 | `runner/drive.rs`、`runner/background.rs`、`runner/assembler.rs`、`runtime/mod.rs runtimes_view`、`template.rs`、`agent.toml` | `SessionPolicy.runtime_view = chat \| work \| all`（内建 ui → chat，其它 work；`[session.plan_simple] runtime_view = "all"`）。chat 背景两行：“your private runtime …”与“owner's default runtime `host` (os, available, desktop access)…”；work 的 `<runtime>` 带 `runtimes.private`；plan 多一个 `<runtimes>` 块，每 runtime 一行（§5.3 列：id / name / connector / platform / baseline / availability / computer / install / flags private·owner default·read-only·no workspaces·desktop·expires / roots / 核验过的命令 / skill 命令 / desc） |
| W6 R-W4 R-W11 | `behaviors/analyze_intent.toml`、`plan_simple.toml`、`work_simple.toml`、`goal.toml`、`cmd_skills/ui_core.md`、`aigc_*.md`、`opendan_agent_dev.md` | analyze_intent 二选一（不写 `--runtime` = 私有 / 用户明确要求 → owner 默认 / 点名别的机器或多台 → `--class plan_simple`）；plan 读 `<runtimes>` 逐 Todo 填位置、单机 Todo 的材料由私有 Todo 先 `copy`；ui_core 删“看 runtime list 自己挑”，写“截屏 / 打开文件 / 在我电脑上跑 = work”、M7 路径规则与 `run` 的轻量用法 |
| W8 R-W8 | `doc/key url.md` §3、`ui_core.md` | “chat 里用户打的裸路径 = owner 默认 runtime 的设备”写进 key url.md，与 D2 分工；`read` 工具接受 `file://<device_id>/` 本来就有（`opendan/urls.rs`，现按 connector 找节点） |
| R1 R-K1–K7 R-C2 R-C3 | `protocol/runtime.rs`、`state/runtimes.rs`、`state/mod.rs`、`state/krpc.rs`、schema、Agent State Protocol §10 | `runtime_record` v4：去 `kind`，加 `connector {native \| node_executor{node_did\|executor_id, profile_id} \| ssh}`、`caps {os, arch, shell, desktop, identity_mode, baseline, baseline_check, tools_check}`、`grant {ops, roots, computer, install, workspace, expires_at_ms}`、`desc`、`enabled`、`lifecycle`、`usage`；`FrozenRuntime {runtime_id, connector, revision, selected_by, caps, grant}`；`is_node()` 全部改读 connector；`binding.json` `/6`：`kind` → `connector`（`native \| node_executor`）；`check {runtime_id, commands?}` 写 `tools_check`；`register` 后立刻 check、要求 `--root`；`unregister` = retire（永久保留）；新 kRPC `runtimes.note_usage`；`xagent runtime register --connector ssh\|node_executor`，`runtime op <op_id>`；探测把执行器 `login.caps` 写进 `grant.ops`、`describe` 的 roots（owner 未缩窄时）、平台、`identity_mode`、`desktop` 写进记录 |
| R2 R-U3 | `runtime/mod.rs runtime_exec / runtime_put / runtime_op` | Agent 级原语：任一可用 runtime 上跑一条命令 / 放一个文件或目录（分块 `write_chunk`）；私有 PATH 前置 `<agent_root>/tools`；输出存档 `state/runtime_ops/` |
| R11 R-C8 | `agent_tool runtime/mod.rs`（`RemoteNodeConfig.executor_id`、`connect_target()`）、`runtime/mod.rs ExecutorResolver / connector_for`、`buckyos_api AgentRuntimeTarget::Executor{profile_id?}` | form B executor 投影为 `connector: node_executor {executor_id, profile_id}`；到达时经 control panel `executor.online.list` 取 generation / store_id / profiles（profile 为空且只有一个时自动取）建 `WssExecutorConnector`，`expires_at_ms` 来自在线列表；`freeze_runtime` / `open_session_env` / `runtime_for_session` / `check` 走同一条 node_executor 路。**未在真实 daemon 上验证** |
| R12 R-U10 | `state/workspaces.rs`、`api.rs` | `workspaces.create / import` 要求 runtime `usable()`（含 `enabled`、未到期、未 retire）；`freeze_runtime` 读 grant 的 read_only。**`grant.workspace = false` 拒绝建 workspace 未接到 `workspaces.rs`**（只在 WebUI 候选过滤与手册里体现），见 §8.3 |
| R10 的 settings 侧 | `buckyos_api AgentRuntimeSettings.grant: Option<AgentRuntimeGrant {install, workspace, roots, expires_at_ms}>`、`InstallGrant`、`opendan/home.rs settings_set {grant}` | 投影只读 settings 的 grant；control panel 算交集写进来的那半（`agent_runtimes.rs apply_update`）**未做** |
| R-K5 | `buckyos_api ExecutorRuntimeInfo.home`、`node_executor stream.rs describe`、`runtime/mod.rs node_agent_dir / node_workdir` | 执行器 `describe` 报账户 home（profile env `HOME` > passwd > `USERPROFILE`）；节点 Session 目录改为 `<home>/.opendan/<agent_id>/sessions/<sid>`，home 不在 roots 内或未报告时绑定拒绝并提示管理员 |
| E4 R-E5（部分） | `protocol/runtime.rs RuntimeUsage / RuntimeLifecycle`、`state/runtimes.rs note_usage / unregister` | `usage {sessions_total, last_used_at_ms, last_session_id, workspaces, skill_commands}`、`lifecycle: active \| retired`；`freeze_runtime` 记 Session 使用；Workspace 创建 / skill 安装处的记账与 `SessionSummary.runtime_id` **未做** |
| WebUI | `opendan/web`：`types.ts`、`Workspaces.tsx RuntimeList`、`mock.ts`、`i18n.ts`、`SessionDetail.tsx`、e2e `home.spec.ts`；`desktop` worklog model / mock | 卡片按 connector / caps / grant / usage / lifecycle 渲染，workspace 候选只给 `grant.workspace` 的可用 runtime；仍在 Workspaces 页（E6 搬 Home 未做） |

共享类型联动：`buckyos_api`（`PRIVATE_RUNTIME_ID`、`AgentRuntimeTarget::Executor.profile_id`、`AgentRuntimeGrant` / `InstallGrant`、`AgentRuntimeSettings.grant`、`ExecutorRuntimeInfo.home`、`ExecutorOp: JsonSchema`）；scheduler / control_panel 的 `AgentRuntimeSettings` 字面量补 `grant: None`；settings 词表里 `default_runtime_id = "container"` 不改（libopendan 边界按别名解析，`runtimes.settings_set` 接受 `private`）。

### 8.2 验证（2026-11-04，`buckyos/src/`，非 root）

- `cargo check -p libopendan -p opendan -p node_executor -p control_panel -p scheduler -p node_daemon --tests` 通过。
- `cargo test -p libopendan`（lib 62 + 全部 test target）、`-p opendan`、`-p buckyos-api` 通过（`--test-threads=1`）；新增 / 改写用例：`runtimes.rs`（private 别名、register 要 root 且立刻 check、retire、kRPC `check --commands` / `note_usage`、原生运行时 private 仍可用、显式 `host` 的重检）、`session_runtime.rs`（未指定的 work 落私有而非 owner 默认、chat 恒私有、`container` 别名）、`remote_report.rs`（binding `connector`）、`tests/common` 新增 `MockNode`（答 login / describe，含 `home`）。
- 全量跑完后按新模型改了四处断言（`workspace_binding` 的 `$OPENDAN_WORKSPACE_RUNTIME = private`、`xagent_cli` 的 `runtime register --connector ssh --root` / retire 流程、`opendan urls` 单测的 v4 记录、`loader` 的内建列表 `private`）。两处失败在改动前的树上同样失败（`git stash` 核对）：`session_runtime::a_session_token_reaches_only_its_own_session_methods` 与 `opendan loader::a_session_process_elsewhere_reaches_its_session_through_the_service`，都是 helper `todo add` 在没有队列客户端的 CLI 进程里派发 Todo 报 “the session needs an input queue but the host has no queue client”，与本次无关、未处理。
- `agent_tool --lib`：330 通过，3 失败为 `grep_tool` 找不到 `rg`（本机未装 ripgrep，与本次无关）。
- `node_executor --test remote_node_runtime`：改了用例（`RuntimeConnector`、`grant.roots`、`check --commands`、`runtime_exec` 在节点上跑命令与越界拒绝），但它需要 root，本机只能 `skipping: needs root`，**未实测**。
- OpenDAN WebUI `pnpm check`、`pnpm lint` 通过；Desktop `pnpm check` 通过；e2e 未跑。
- schema 用 `xagent schema doc/opendan/protocol/schema` 重生成（`runtime_record`、`session_config`、`binding`）；黄金 fixtures 用 `cargo run -p libopendan --example fixtures -- ../doc/opendan/protocol/fixtures` 重生成（`runtime.frozen` 新形状、`binding/6`；run 目录按时间戳命名所以整批换新），`cargo test -p libopendan --test fixtures` 随之通过。
- `control_panel` 单测 `sso_identity::…carries_ceiling_method_and_password_state` 失败（Aicc scope 断言，文件未动，与本次无关）；scheduler、node_daemon、node_executor（非 root 部分）通过。
- 未做：真实 Zone DV（原生运行的 private、host 上 `agent-session run`、form B executor 路由）、Windows / macOS 实机。

### 8.3 未做 / 风险

- **P2 / P3 / P4 全部未做**：R3 节点 `tools/<skill>/bin`、R13 持久卷核验、R16 ExtTools 全平台包与 `EXTENDED_TOOLS`、R4 `skill` 选择级、R6 skills 状态重算、R7 `runtimes.impact` 与确认、R8 禁用通知、R14 到期处理（现只做了到期 → `runtime_unavailable`，没有自动禁用与通知）、R9 / R15 登记预设与分组、E1–E3 / E5–E7 用户级目录、daemon 两步化、ssh 上提、Desktop 目录页、Home 面板、`buckyos runtime` CLI。
- `agent-session run` 超时未“转 task”：到 `--timeout`（缺省 120 s，上限 30 min）返回 124 与已有输出；长命令请加 `--timeout` 或建 work。
- `grant.workspace = false` 尚未在 `workspaces.create / import` 里强制（R-U10 的拒绝信息也没写），只有 WebUI 候选过滤与手册引导。
- control panel 侧的 grant 交集（R10）、daemon URL 设备分配写 `settings.runtimes[exec-<id>]`（R-E3）未做，所以 `AgentRuntimeTarget::Executor` 现在只能靠手工写 settings 触发；`ExecutorResolver` 的真实路由未在 daemon 上跑过。
- `RuntimeSelection::Skill` 只是枚举值，R-U1 的第四级（按带自装命令的 skill 落点选 runtime）未实现。
- `SessionSummary.runtime_id`、sessions 列表按 runtime 过滤、Workspace / skill 安装的 `usage` 记账未做。
- 旧的 `state/runtimes/*.json`（version 3）读取失败：内建记录由宿主重写，非内建的 `remote_ssh` 登记要重新 `register`（开发态，不做迁移）。
