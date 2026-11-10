# OpenDAN Agent Runtime：工作模型与生命周期 TODO

> 2026-11-04，三轮迭代合并。第一轮从 [skills 体系改进计划 §3.6.1](opendan-skills-system-todo.md) 的“cmd 装到哪个 runtime”出发整理 runtime 的创建 / 使用 / 更新 / 删除；第二轮用脑图加了用户视角与能力视角两个分类；第三轮（§0）确定 **Agent 工作模型**：私有 runtime 永远在、chat 只知道两台、plan 看全表，并据此把 `kind` 换成 connector + caps + grant（§1）。范围因此从“runtime 登记的生命周期”扩成“Agent 怎么用 runtime”。第四轮（§0.6、§4.6）核对用户添加 runtime 的入口，改成“用户级目录 → 分配给 Agent → Agent 首页看当前与历史”两步。相关：[AgentRuntime 设计 §6.4](<../doc/llm_context/AgentRuntime 设计.md>)、[Agent State Protocol §10](<../doc/opendan/protocol/Agent State Protocol.md>)、[Session Directory Protocol §9](<../doc/opendan/protocol/Session Directory Protocol.md>)、[Session Runtime](<../doc/opendan/OpenDAN Agent-Session-Runtime.md>)、[primary-runtime TODO](opendan-primary-runtime-tools-todo.md)、[object-url TODO](done/agent-object-url-todo.md)（路径解释 D1–D3）。上一版（只有生命周期 + 两视角）的内容全部并入，编号 R-C / R-U / R-M / R-D 保持不变，新增 R-W（工作模型）与 R-K（记录模型）。
>
> **2026-11-09 状态**：P0 / P1 已提交（`c88404aaf`，实施记录在 §8）。之后[扩展包 TODO](opendan-exttools-cross-platform-todo.md) 一、二期和 [skills TODO](opendan-skills-system-todo.md) 的实施做掉了 R4、R13、R16，[Agent UI 升级 TODO](agent-ui-upgrade-todo.md) 的裁决 D3 / D4 改了 Agent 首页一侧的分工。§1、§3、§4、§6 的状态列已按当天的代码更新；剩余任务、与另外两单的分工、验证状态见 §9；**§7.1 的 8 条裁决和 §7.2 的 8 条答复都已回写**。两轮答复叠起来之后的模型在 §10，前文与它冲突的地方以它为准；§7.3 的两点也已确认，目前没有待定的设计问题。§0.4、§2 的“现状”是实施前的快照，留作对照；§0.6 的变化标在行内。

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

“现状”列是 2026-11-04 实施前的状态，各行已按 §8.1 落地。两处与表里写的不同：cmd_skill 一行的 ExtTools 安装器由扩展包 TODO 二期交付（node_daemon 的 reconciler），不是本文的 R16；最后一行的 binding `/6` 实际只把 `kind` 换成 `connector`，没有加 `private {workdir}`（单机 session 没有私有侧，多 runtime session 的 workdir 本来就在私有，§0.2）。

| 现状 | 目标 | 落点 |
| --- | --- | --- |
| UI session 按 `freeze_runtime` 三级规则取 owner 默认（Desktop 上是 host） | 恒私有（M1） | `api.rs freeze_runtime`、`template.rs [session.ui]` |
| 提示词 `<runtime>` 渲染当前 session-runtime 的 `RuntimeInfo` + workspace；要看别的得 `agent-session runtime list` | chat：私有 + owner 默认两行；plan：全表 `<runtimes>`；work：工作 runtime + 私有一行 + workspace | `runner/assembler.rs`、`runtime_status` 生成处 |
| analyze_intent 手册教模型 `runtime list` 后自己挑 | 二选一（M3），复杂的交给 plan | `behaviors/analyze_intent.toml` §Workspace and runtime |
| plan_simple：Todo 缺省继承 plan 的 session-runtime；plan 的 runtime 又来自创建者 | plan 恒私有；Todo 缺省私有；plan 用全表填 `--runtime` | `behaviors/plan_simple.toml`、`runner/todos.rs` |
| cmd_skill 按 session-runtime 的 baseline / platform 过滤；`extended` 在 host / 节点上只是声明，没有安装器 | 过滤不变；补 ExtTools 全平台包与安装器（R-S1） | `baseline.rs`、`runtime/mod.rs`、publish/exttool |
| `agent-session copy` 支持 `session: / workspace: / <runtime_id>:`；没有跨 runtime 执行；节点 Session 期望将来装 helper（§6.1 欠账） | 新增 `agent-session run --runtime <id\|workspace>`；helper 只在私有，节点不装 | `transfer.rs`、`xagent_cli.rs`、`session.helper`、Session Directory Protocol §9 |
| 原生运行时 `container.exec = null`，私有不可用 | 私有永远可用，`connector: native` | `protocol/runtime.rs builtin_runtimes`、Loader |
| binding `/5`：runtime_id + workdir + workspace | `/6`：加 `private {workdir}`（控制侧 SessionDir） | Session Directory Protocol §9、`runtime/mod.rs` |

### 0.5 在非工作 runtime 上执行命令

Agent 让命令跑到“不是当前 default runtime”的机器上，只有四条路。前三条是产品面，第四条是现有 runtime 层工具，R-C8 之后收敛进第二条。**第二、三条只在私有 default 的 session 里存在**（`agent-session` 只装在私有 runtime）；单机 session 只有第一条，由创建者替它决定。

| 路 | 谁用 | 粒度 | 现状 |
| --- | --- | --- | --- |
| 建 work session 时 `--runtime <id>`（analyze_intent 的 `create-worksession`、plan 的 Todo `--runtime`） | chat → work，plan → Todo | 整个 Session，冻结后不变 | ✅ |
| `agent-session run --runtime <id\|workspace> [--cwd <dir>] [--timeout <s>] -- <cmd>` | 私有 default 的 session（chat 轻量、plan 只读、work） | 单条命令 | ✅（W4） |
| `agent-session copy --from <session\|workspace\|id>:<path> --to …` | 私有 default 的 session | 文件 | ✅ |
| `executor_call` 工具（agent_tool runtime 层，对在线 daemon 目标） | 现有 ComputerUse / daemon 路径 | 单个 executor op | ✅，但不经 runtime 登记与 grant |

`run` 的规则（R-W5 的细则）：

- **目标**：`workspace` 解析为本 Session 绑定的 workspace 所在 runtime；`id` 必须是本 Agent 登记、`available` 且 `grant.ops ∋ Exec` 的 runtime。不可用拒绝不回退，拒绝信息同 R-U5。
- **cwd**：`workspace` → workspace 目录（只读 workspace 仍可跑，但写文件由执行器按 grant 拒）；`id` → **必须给 `--cwd`**，且在该 runtime 的 `grant.roots` 内；不在目标上替它建 Session 目录。
- **环境**：目标 runtime 自己的 PATH 与 shell（`caps.shell`）；不把私有侧 PATH、Agent Bin、cmd_skill 命令目录带到别的机器（与 binding §9 的规则一致）。Agent Bin 里 skill 自带的命令和 `agent-session` 只在私有 `shell()` 里可用，不能 `run` 到别处。
- **执行**：经 `session.helper` 到宿主的 `runtime_exec`（R-U3），记录在 `runtime_ops` 目录，有调用 id；到 `--timeout`（缺省 120 s，上限 30 min）终止命令，返回 124 和已有输出，**不转 task**（裁决 29）：要跑很久或要持续跟进的命令建 work（单机 session，M3）；输出截断规则同 `shell`。Session 恢复后按调用 id 查询，不重放（与 ComputerUse 草案“未知结果不盲重放”同一条）。
- **不改变绑定**：`run` 不写 `binding.json`，不换 default runtime，不改 Session 的 cwd；每次调用独立做 grant 检查并写 worklog（哪个 runtime、什么命令、调用 id）。
- **不改公共 tool schema**：`shell` 工具不加 `runtime` 参数，全部走 `agent-session run`。
- **chat 也有这条路**（它是私有 default）：机制上不限制；`ui_core.md` 引导为 M7 的读文件与短命令（看一眼目录、取一个文件），持续的活建 work（M3）。待裁决 17 据此关闭。
- **与 workspace 的关系**：私有 session 的 workspace 在另一台 runtime 上时，典型序列是 `copy` 材料进来 → 私有侧构造 → `copy` 回去 → `run --runtime workspace -- <构建/校验命令>`。这是 §0.1 第 4 条“私有构造、目标完成”的具体形态。
- **单机 session 的材料**：它不能 `copy`，所以 plan 把 Todo 放到非私有 runtime 时，要么 workspace 就在那台机器上，要么先用一个私有 Todo 把材料 `copy` 过去（Todo 链），并在 Todo 正文写明路径。

### 0.6 用户入口：两步流程

**目标流程**（owner 的设计）：

1. 用户管理**自己的** agent runtime：一张用户级列表，每条能看到被分配给了哪些 Agent、各自拿到什么 grant。
2. 在 Agent 的 home page 看**这个 Agent** 的 runtime 使用情况：当前列表里的，和历史上用过的。

**现状**（2026-11-04 核对；2026-11-09 的变化标在行内）：runtime 进入 Agent 视野的入口有六处，分属两个产品、三种对象，互相不通：

| 入口 | 在哪 | 做什么 | 问题 |
| --- | --- | --- | --- |
| 节点开放（`owner_agents` 策略） | node_daemon 启动时 `seed_owner_agents` 种下；之后只能改 system config | 决定哪些节点对 owner 的 Agent 开放、roots、ComputerUse 上限 | Desktop 没有 UI（`grep owner_agents` 在 desktop/src 只命中 API 类型）；owner 在自己的 Desktop 上既是管理员又是用户，却改不了。Linux 节点只有激活时选了 OS 账号才会种（节点本地的 `etc/host_account.json`，策略带 `run_as`），没选的节点没有策略，目录页显示为未开放（裁决 25、§7.2 推定 3） |
| 添加设备（daemon URL，form B） | Desktop → Settings → Cluster Manager → Add device（`AgentDevices.tsx`，`executor.device.*`） | 签发 URL；表单里**当场选一个 Agent**（`profiles.<p>.principals = [agent]`）、roots、只读 / 读写、ComputerUse、有效期；列表有在线 / 到期 / Reset URL / Revoke / Delete | **不写任何 Agent 的 `settings.runtimes`**（代码里没有写 `AgentRuntimeTarget::Executor` 的地方，`agent.update` 只接受目录里的节点）；分配关系藏在 executor profile 的 `principals` 里，也没有修改已有设备 profile 的接口。2026-11-09：手工把 `exec-<id>` 写进 settings 之后的投影与 Session 绑定已有（R11，未在真实 daemon 上验证），不再只能经 `executor_list` 工具看到 |
| Agent 创建向导 | Desktop → 新建 Agent → runtime 步 | `agent.runtime_directory` 取目录，只处理 `host`（开 / 关 + 节点 ComputerUse 上限），`apply_create_defaults` | 目录里的其它节点不可选；创建后没有同等的编辑页 |
| Agent 详情（Desktop） | users-agents → `AgentDetailPage` runtime 卡片 | 只读展示 `settings.runtimes` 与 `default_runtime_id` | 不能改；`agent.update` 的 `runtimes` 参数只有向导在用 |
| opendan WebUI | Home 页 `RuntimeList`（`runtimes.settings_set`；2026-11-09 已从 Workspaces 页搬来，只在 `Home.tsx` 渲染） | 对已有条目启停 / 默认 / ComputerUse / 声明基线 / `check`；显示 connector / caps / grant / usage，不能改 grant | 明确不能新增目标（“control panel adds runtimes”）；每台一张长卡片，形态待改（UI TODO RT2） |
| CLI | `buckyos agent list\|get\|profile`、`buckyos executor list`、`xagent runtime register`（开发用） | — | `buckyos` 没有 runtime 模块（2026-11-09 多了 `ext-tools`、`skill`，仍没有）；`remote_ssh` 只能用 `xagent` 直接登记到某个 Agent 的 Agent State，用户级看不到，而且登记了也不能执行（裁决 23：本轮不做 ssh） |

**历史使用**：Session 的 `runtime.frozen` 在 `session_config` 里（SessionDetail 的 “Runtime & hosting” 面板能看单个），但 `SessionSummary` / sessions 列表不带 `runtime_id`；Workspace 记录带 `location.runtime_id`；没有按 runtime 聚合的“哪些 Session / Workspace / skill 用过它”视图，`runtimes.impact` 也还没有（R-D1）。被 `unregister` 的记录直接消失。2026-11-09：记录已有 `usage`（Session 数、最近一次、skill 命令）与 `lifecycle: retired`，`unregister` 改为 retire、记录保留；`SessionSummary` 仍不带 `runtime_id`，Workspace 不记账，`runtimes.impact` 仍没有。

**“分配给了哪些 Agent”**：`agent.runtime_directory` 是 owner 级目录 × 单个 Agent 的 settings 叠出来的视图，没有跨 Agent 的反查；要知道 host 开给了哪几个 Agent，只能逐个 Agent 看。

**结论：要改，而且是把“对象”理顺，不只是加页面。** 现状的层次其实已经是两层（用户级的节点 / 设备 → 每个 Agent 的选择 → Agent State 投影），缺的是：用户级没有一张表、daemon 设备没有走进第二层、Agent 侧没有历史。目标模型：

```text
用户级 runtime 目录（owner 一份）           ← 第 1 步在这里管
  ├─ 节点（owner_agents 策略开放的）          来源：system config nodes/<n>/settings
  ├─ 设备（daemon URL，form B）               来源：executor 记录（含到期）
  └─ ssh 登记（从 Agent State 上提到用户级）  来源：新增 users/<owner>/agent_runtimes/<id>   ← 本轮不做（裁决 23）
每条：connector、它自己的配置（ops、roots、ComputerUse、期限、基线）、assigned_to: [{agent_id, enabled, default}]

分配 = AgentSettings.runtimes[id] = {enabled, target}    ← runtime 页“分配给 Agent”；Agent 页启用 / 停用 / 移除。没有按 Agent 的 grant（§10）
  → Agent State 投影（§1 记录）                 ← 第 2 步看这里 + 使用账本
```

两步流程下各入口的去向：

| 入口 | 去向 |
| --- | --- |
| 节点开放策略 | 进用户级目录页：zone owner 在 Desktop 上能开 / 关节点、改 roots 与 ComputerUse 上限（写 `owner_agents`，R-E2）；对有策略的节点都可用，Linux 节点保留 `run_as`（裁决 25、§7.2 推定 3） |
| 添加设备 | 拆成两步：签发 URL 时 Agent **可选**（不选只签发；选了就是签发 + 分配连着做）；profile 的 `principals` 由分配推导，与节点的 scheduler 做法一致；“分配给 Agent”是目录页上的动作，写 `settings.runtimes[exec-<id>] = {target: Executor}`（R-C8）。一台设备同时只有一个 Agent 启用，配置在签发时定死（§10） |
| 创建向导 | 不变，但目录里的任何节点 / 设备都能在向导里勾（不止 `host`） |
| Agent 详情（Desktop） | 保持只读摘要，加链接跳到 opendan 的 Agent Home runtime 面板（编辑在那里做） |
| opendan WebUI `RuntimeList` | 搬到 Agent 自己的 Home page，成为“这个 Agent 的 runtime”面板：当前（§5.3 列）+ 历史（R-E5）+ 可编辑；这是第 2 步唯一的落点，Desktop 详情页只链过来；Workspaces 页只留 workspace 与 runtime 的关联。2026-11-09：已搬到 Home；形态按 [Agent UI 升级 TODO](agent-ui-upgrade-todo.md) 的裁决 D4 改为“瘦列表（三类）+ 详情页 + 历史”，由那边的 RT1–RT4 实施（§9.2）；runtime 的配置（权限、ComputerUse、基线）在 Desktop 的目录页管，这里只读（裁决 24、§10） |
| CLI | `buckyos runtime list\|add-ssh\|assign\|unassign\|revoke`（用户级）与 `buckyos agent runtime list\|enable\|disable\|default`（Agent 级）；`xagent runtime register` 保留给开发；`add-ssh` 本轮不做（裁决 23） |

## 1. 记录模型：connector + caps + grant（去掉 kind）

**2026-11-09 第二轮（§7.2 推定 2）：没有“按 Agent 授权”这一层了。** 下表的 `grant` 一行改读为“这台 runtime 自己允许什么”：只剩 `ops / roots / computer / expires_at_ms`，唯一来源是对 runtime 的探测；`install`、`workspace`、“owner 缩窄”和“投影时算交集”都取消。完整的模型见 §10。

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
| R-K1 | `runtime_record` 版本 4：去 `kind`，加 `connector / caps / grant / desc`；旧记录按 kind 推 connector（Container → native，HostNative / RemoteNode → node_executor，RemoteSsh → ssh）；`FrozenRuntime.kind` 同步换成 `connector` | ✅（R1）。旧记录没有按 kind 推：v3 读取失败，内建由宿主重写，非内建重新登记（§8.3） |
| R-K2 | `is_native_runtime` / `is_node()` 改读 connector；37 处机械替换；`binding.json` 的 `kind` 字段改 `connector` | ✅（R1） |
| R-K3 | caps 与 grant 不合并：caps 是探出来的事实，grant 是人给的授权（daemon TODO §5.4 末条：capability 是声明不是授权，范围取交集） | 作废（§7.2 推定 2）：没有按 Agent 的授权，`grant` 只剩 runtime 自己的配置这一个来源，不存在交集 |
| R-K4 | `grant.expires_at_ms` 对所有来源统一：daemon URL 的 `expires_at` 投影进来；owner 也能给 host 一个有期限的 grant（“这周允许装东西和控屏”）；到期处理见 R-M7 | ⚠️ 字段、daemon 期限的投影、到期判定已做。期限只来自 runtime 自己（daemon URL）；“owner 给 host 设一个有期限的 grant”不做（§7.2 推定 2：要临时的就建一台临时 runtime）。到期后的处理见 R-M7 |
| R-K5 | `grant.roots` 只表示执行器强制的权限边界（容器为空表示不限）；Agent 目录按 §0.3 目录约定定位，**不加 `home` 字段、不迁移**。节点侧把“账户 home”从 `describe` 拿（`ExecutorRuntimeInfo` 现在只有 os / arch / hostname / shell / path，加 `home`），不再取 `roots[0]`；`home` 不在任何 root 内时绑定拒绝并提示管理员。`remote_ssh` 登记要求显式 `--root` | ✅（R1）：`describe` 报 `home`，home 不在 roots 内时绑定拒绝；`register` 要求 `--root` |
| R-K6 | 登记 / 签发表单的**预设**（不是强制）：`device_type ≠ desktop` 的节点或设备预填 `ops = 文件读 + computer_observe`、`workspace = false`、`install = none`；desktop 节点与私有预填 `workspace = true`；管理员 / owner 可改 | 取消（§7.2 推定 2）：预设是 grant 的预填，grant 层没有了；签发表单的缺省值随 E2 |
| R-K7 | `xagent runtime register --kind` 改为 `--connector`；WebUI RuntimeList、Agent State Protocol §10、Workspace Protocol 中的 kind 引用同步 | ✅（R1） |

## 2. 实施前的现状（2026-11-04 工作树，留作对照）

本节写在 P0 / P1 落地之前：`kind`、三级 `freeze_runtime`、binding `/5`、“`Executor` 目标 not wired into Sessions”、提示词只有当前 session-runtime，这些都已经被 §8 改掉。三层的划分和各层的写入方没有变。当前状态看各节的状态列与 §9。

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

**2026-11-09 裁决 28（owner，讨论后）：skill 不能带二进制，只是纯文本；这是分发的安全边界。带 bin 的 skill 更接近一个 ext tool 包（包里顺手带了 skill）。** 命令因此只来自两层：`standard` 与扩展包。下表的第三层取消，R-U1 的 `skill` 级、R-U4、R-U9、R-M2、R-M5 随之取消；已经实现的部分（skills S18–S20 里的 cmd 安装，runtime 侧的 `skill_runtime`、`usage.skill_commands`、Session PATH 的 `tools/<skill>/bin` 层）要拆，见 R17 和 skills TODO §0。§7.2 已确认：安装脚本也不保留，安装 skill 就是确定性的复制；skill 包依赖扩展包，两者各发各的 pikg。skill 与 runtime 只剩一处关系：冻结时按 Session runtime 的 `baseline / platforms` 过滤手册。

命令来自三层（裁决 28 之后只剩前两层）：

| 层 | 内容 | 谁装 | 现状 |
| --- | --- | --- | --- |
| `standard` | 28 个 Agent 标准命令（`agent_tool`、`agent-session`、`xagent`、`buckyos`、AICC 命令…，清单 `baseline::STANDARD_TOOLS`） | 私有：`$BUCKYOS_ROOT/bin/`（镜像或安装根自带）；host / 节点由 node-daemon 的 `install-agent-tools` 布到其 buckyos root 的 `bin/agent-tools` 并前置 PATH | ✅ |
| `extended` = standard + ExtTools 包 | ffmpeg / ffprobe、ImageMagick、poppler、tesseract、pandoc、python / uv、node、deno…（Session Runtime 文档 §3.2 / §3.3） | node_daemon 的 reconciler 按 Zone 的期望状态把扩展包装到节点的 `$BUCKYOS_ROOT/tools`（容器只读挂载同一目录，不再进镜像）；管理员在 Desktop “设置 > 扩展工具”或 `buckyos ext-tools` 里选。form B 设备不参与，`extended` 只能由 owner 声明 | ✅（扩展包 TODO 二期，2026-11-06） |
| ~~skill 自带~~ | ~~`tools/<skill>/bin`~~ | 取消（裁决 28）：skill 是纯文本，不带命令；要带命令的做成扩展包。现有实现（复制进 Agent Bin、`install-cmd --runtime` 放到节点）待拆 | 取消 |

由此 §3.6.1 C “cmd 装到哪个 runtime”收窄成两件事：

| # | 需求 | 现状 |
| --- | --- | --- |
| R-S1 | **ExtTools 全平台包 + 安装器**（2026-11-06 改写，方案见 [扩展包 TODO](opendan-exttools-cross-platform-todo.md) §3.3）：扩展包是 `app_type=ext_tools` 的 pikg（格式已落地），**安装器是 node_daemon 的 reconciler**，对照 Zone 级期望状态写 `$BUCKYOS_ROOT/tools/`（容器节点写 `buckyos-tools` 卷）；管理员在管理页勾选，Agent 侧只读，不再有 `runtime ext-tools install`，也不经 `runtime_put` / `runtime_exec`（后者没有写安装根的权限，也没有持久 task）。owner 声明 `extended` 的规则不变（声明与探测分离）。form B 设备不参与，baseline 只能声明 | ✅ 扩展包 TODO 一、二期已做（格式、期望状态、reconciler、管理页与 CLI）；三期 standard 分发未排期（裁决 22） |
| R-S2 | 官方 skill 只用 `standard` / `extended` 清单里的命令，手册 `requires` 写具体命令、`platforms` 写适配平台；第三方 skill 自带 `install.toml` 脚本的路径保留为例外（§3.6.1 C），不是主路径 | ✅ 清单已固化（R-S3），官方手册补了 `commands`（扩展包 X5）。“第三方 skill 自带 `install.toml`”这个例外随裁决 28 取消（§7.2 推定 4） |
| R-S3 | `extended` 清单机器可读（`EXTENDED_TOOLS`，对应 R-S1 的包内容），`check` 用它核验，plan 全表显示 | ✅ `baseline::EXTENDED_TOOLS`（与 `ext-core` 的 `ext-manifest.json` 单测绑定）、`check_baseline` 按声明选集合、`runtime_line` 的 `ext:` 段 |

冻结规则**不变**：按 session default runtime 的 `baseline` / `platforms` 过滤（`unmet_on`），不满足的不注入并说明。`agent-session` 的特殊性（§0 第 2 点：只在私有 session 里使用）不靠 skill 字段表达：用到它的官方手册只有 `ui_core`（`scope: ui`，UI session 恒私有）与 `opendan_agent_dev`；它在 `STANDARD_TOOLS` 里且被布到 host 这件事怎么处理见待裁决 22。

安装记录永远写实际 runtime_id（§3.6.1 的 `pinned_runtime = null` 约定取消）；`pinned_runtime` 只对带自装命令的 skill 有意义。（2026-11-09：安装记录与 `pinned_runtime` 随裁决 28 取消。）

| 需要 | 现状（2026-11-09） | 缺口 |
| --- | --- | --- |
| 在指定 runtime 上装 ExtTools / 跑安装脚本并核验 | ExtTools 走 node_daemon 的 reconciler；skill 不再有安装脚本（裁决 28） | 无 |
| 把文件放到目标 runtime | `runtime_put`（R2）保留为 Agent 级原语；skill 不再用它往节点放 `tools/` | 无（R3 取消） |
| runtime 变化反映到 skill 状态 | 手册 `commands` 的可用性由 `skills.status` 每次现查（`runtimes.check --commands`），不落盘 | 无（R6 取消） |
| 创建 Session 时按 skill 落点选 runtime | 取消（裁决 28）；已实现的 `skill_runtime` 待拆 | R17 |

## 4. 分环节需求

编号 R-W（工作模型）、R-C（创建）、R-U（使用）、R-M（更新）、R-D（删除）；“现状”标 ✅ 已有 / ⚠️ 部分 / ❌ 没有。

### 4.1 工作模型（§0 的规则落成需求）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-W1 | 私有 runtime 永远可用：内建记录 `private`（别名 `container` / `local`），`connector: native`；原生运行时也如此，不再 `exec = null`；`availability` 恒 `available`；`caps.baseline = extended`（容器）或 owner 声明（原生）。私有 runtime **必须拥有 `$BUCKYOS_ROOT` 布局**（目录约定第 5 条）：Loader 启动时核验 `bin/`、`data/home/<owner>/agents/<agent_id>`、`instance/`、`tools/` 存在，缺了是部署错误，不是“不可用” | ✅（W1） |
| R-W2 | M1：UI / chat 类 Session `freeze_runtime` 固定私有，`selected_by: private`；`SessionSpec` 需要知道类别（UI 模板建的 Session 带 `class: ui`） | ✅（W2；UI 类 Session 按 `SessionKind::Ui` 判定） |
| R-W3 | M2：chat 的 `<runtime>` 块两行（私有、owner 默认），其余 runtime 不出现；`ui_core.md` 删掉“看 runtime list 自己挑”的指引，改为“要在别的机器上做的事建 work” | ✅（W5、W6） |
| R-W4 | M3：`analyze_intent.toml` 的 runtime 段改为二选一；结果 schema `runtime_id` 允许 `private`；用户点名其它机器或多台 → `--class plan_simple` | ✅（W6） |
| R-W5 | M4：`agent-session run --runtime <id\|workspace> [--cwd] [--timeout] -- <cmd>`：经 `session.helper` 到宿主的 `runtime_exec`（R-U3）；输出与退出码原样回传，到 `--timeout` 终止并返回 124，不转 task（裁决 29）；只在私有 default 的 session 里存在；细则见 §0.5 | ✅（W4） |
| R-W6 | M5：不加 skill 字段，冻结规则不变；需求转为 §3 的 R-S1 – R-S3 | — |
| R-W7 | M6：plan_simple 首个输入带 `<runtimes>` 全表（列见 §5.3）+ workspace 列表；Todo `--runtime` 缺省私有；手册改写 | ✅（W5、W6） |
| R-W8 | M7：chat 里绝对路径的解释规则写进 `ui_core.md` 与 key url.md；read 工具接受 `file://<device_id>/` 后，私有侧读 owner 默认 runtime 的文件经 `agent-session copy --from <id>:<path> --to session:<f>` 或 read 直接取 | ✅（W8） |
| R-W9 | M8：chat 工作 runtime 可选，先不做；只保证 `freeze_runtime` 的 UI 分支尊重显式 `requirement.runtime_id` | 不排期（显式 `requirement.runtime_id` 已被尊重） |
| R-W10 | 单机 session 的边界写成协议：binding 不升版；节点上**不安装** `agent-session` helper（关闭 primary-runtime TODO §6.1 与 Session Directory Protocol §9 “helper 需在节点上安装”的表述）；节点 `.opendan/session.json` 与 token 保留，给标准工具（`buckyos`、AICC 命令）取服务身份用；单机 session 的 `report` 走 runner 的 report 工具（不依赖 helper），产物按现有节点读取路径上收 | ✅（W3） |
| R-W11 | 行为上的后果写进手册：chat 里的“截屏 / 打开文件 / 在我电脑上跑”都是 work（`--runtime <owner 默认>`），不是 chat 自己做 | ✅（W6） |

### 4.2 创建（runtime 进入 Agent 的视野）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-C1 | 三种来源保持不变：系统投影的内建（`private`、`host`、`node-*`、`exec-*`）、owner 手工登记（`remote_ssh`）、skill 安装过程**不**产生新 runtime | ✅ |
| R-C2 | 每条登记在可用时必须有完整的 `caps.platform` 与 `baseline`；`register` / 投影后立刻 `check` 一次 | ✅（R1：`register` 后立刻 check） |
| R-C3 | `caps.tools_check`：除标准工具外，记录“这个 runtime 上核验过存在的命令”（来源：skill 安装核验、显式 `check --commands`），结构同 `baseline_check` | ✅（R1；写入方是 `runtimes.check --commands` 与 skill 安装） |
| R-C4 | owner 能从 WebUI / `buckyos` CLI 新增与登记 runtime：目录里的节点（`agent.update`）和手工 `remote_ssh`（`runtimes.register`） | ⚠️ 没有变化：WebUI / CLI 都不能新增（E1、E5、E7）。`remote_ssh` 本轮不做，只留开发登记（裁决 23） |
| R-C5 | Agent 自己不能登记 runtime。**申请流程先不做**（已裁决）：候选为空时只在拒绝信息里说“没有可用的 <os> runtime，请 owner 在目录里开一个”（R-U5） | 不排期 |
| R-C6 | 默认 runtime 规则不变（`host_by_default`）；语义改为 §0.3 的“owner 默认”；owner 改默认时提示“装在旧默认上的 skill 命令 / ExtTools：N 个” | ⚠️ 语义已改（W2）；改默认时的提示未做（提示里不再有 skill 命令，裁决 28）。停用默认 runtime 时保留 `default_runtime_id` 归 UI TODO RT1（其裁决 D3），现在的代码会清掉 |
| R-C7 | 登记预设（原 `role`）：WebUI / CLI 登记时的四个模板 `agent_home / owner_desktop / managed_device / temporary` 只是 grant 的预填，不入记录；列表按 §5.1 四类分组展示，分组由 connector + grant 推导 | 取消（§7.2 推定 2）：四个模板是 grant 的预填。分组只用于界面：Agent 首页按 UI TODO RT2 的三类，目录页按来源（节点 / 设备） |
| R-C8 | form B executor（daemon URL）接入 Session：`AgentRuntimeTarget::Executor` 投影为 `exec-<executor_id>`，`connector: node_executor {executor_id}`，`grant.expires_at_ms` 来自 URL；`freeze_runtime` / `open_session_env` 走同一条 node_executor 路。daemon TODO §5.4 已做 executor target 与 ledger，缺 runtime 配置与 Session 绑定 | ⚠️ 投影与 Session 绑定已做（R11），未在真实 daemon 上验证；分配入口未做（E2） |
| R-C9 | 设备能作为执行器添加成功（节点开了 `owner_agents`，或 daemon URL 接入）就进目录，grant 按策略 / 签发的 profile，不另设门槛；Device DID Object Card 类设备（多半经 HA 一类服务接入）走对象库，添加路径不同，不在本文范围 | ⚠️ |

### 4.3 使用（Session、skill、Agent 级操作）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-U1 | work session 的选择规则：显式 `--runtime` > 继承 > **点名的、带自装命令的 skill 的落点**（都装在同一 runtime 才自动取；装在多台且都可用时取默认优先；互相冲突拒绝并列出） > 私有。skill 自装命令缺省装私有，这一级很少触发。`selected_by` 加 `skill` / `private` | ⚠️ 显式、继承、私有三级已做（W2）。“skill 落点”这一级按裁决 28 取消；skills S20 已实现的 `skill_runtime` 待拆（R17） |
| R-U2 | plan 的 runtime 全表 → 并入 R-W7；chat 不要 | — |
| R-U3 | Agent 级执行原语（宿主侧，不依赖 Session）：`runtime_exec(runtime_id, script, cwd, timeout)` 与 `runtime_put(runtime_id, local_dir, remote_dir)`，复用 `run_probe` 的 `runtime_ops` 目录与调用 id，输出存档。R-W5 的 `agent-session run`、skill 安装、`tools_check`、R-M3 都走它 | ✅（R2） |
| R-U4 | 节点上的 Agent 目录 `<OS 账户 home>/.opendan/<agent_id>/`（§0.3 目录约定）：`sessions/<sid>`（已有）、`tools/<skill>/bin`（skill 自带命令）、`ops/`（安装日志）。节点 Session 的 PATH 前置 `tools/*/bin`：节点上没有 helper（R-W10），改由 libopendan 在每次 exec 的 `env_overrides.PATH` 里前置（`ExecutorExecRequest` 已有该字段，不改协议）；cwd 与此无关，由 Session 目录 / workspace 目录决定。**优先级降为 P2**：多数 skill 命令来自 ExtTools | 取消（裁决 28）：skill 不带命令，节点上不需要 `tools/<skill>/bin`。skills S19 已实现的“放到节点”待拆（R17） |
| R-U5 | 不可用时“拒绝不回退”；拒绝信息说清下一步（“runtime host 不可达，最近一次 check …；装在它上面的 skill：…；可以改在私有 runtime 做的部分：…”） | ⚠️；信息里“装在它上面的 skill”一项随裁决 28 去掉 |
| R-U6 | Workspace 与工作 runtime 的关系不变；skill 安装不涉及 Workspace | ✅ |
| R-U7 | 私有 runtime 上的安装就是现在的 Agent Bin 复制 | ✅ |
| R-U8 | 能力投影 = §1 的 `caps / grant`，由投影 / check 算一次；skill 候选、`freeze_runtime`、`workspaces.create`、plan 全表都读它 | ✅（R1）；R19 之后记录里只有 runtime 自己声明的一份，没有按 Agent 的值 |
| R-U9 | 私有（容器）上的 `system` 安装必须落持久卷：`install.toml` 声明 `writes = [...]`，核验不在 `<instance_volume>` / `<agent_root>` 下的记 `volatile`，容器重启后由 R-M2 的 `tools_check` 重核验。**ExtTools 清单（R-S3）里已有的命令拒绝由 skill 自己装**：报“由 ExtTools 提供，手册改声明 `baseline: extended`” | 取消（裁决 28）：skill 不带命令，也不带安装脚本（§7.2 推定 4）。skills S19 已实现的 `writes` / `volatile`、ExtTools 拒装待拆（skills TODO） |
| R-U10 | `workspaces.create / import` 的规则改为派生（§10）：runtime 可用、不是只读、没有期限才能建，不再有 `grant.workspace` 开关；拒绝信息给出可用的 runtime（“临时 runtime 上不建长期产物；在 host 建 Workspace，用 `agent-session copy` 搬文件”） | ⚠️ 不可用、停用、到期、retired 的已拒绝（R12）；只读与“有期限”两条未查；`grant.workspace` 字段待删（R19） |
| R-U11 | 只读 runtime（`grant.read_only`）：需要写的 work `freeze_runtime` 时拒绝；plan 可用；`--runtime` 点名只读 runtime 的 Todo 留 pending 并记录原因 | ⚠️ `freeze_runtime` 的拒绝已做（W2，只读行为放行）；“Todo 留 pending 并记录原因”没有核对 |
| R-U12 | `agent-session copy` 与 `run` 的权限按目标 runtime 的 grant 判定（roots、ops），私有侧按 Agent 自己的目录规则；两侧都记进 `state/transfers` / `runtime_ops` | ✅（W4：`run` 记在 `state/runtime_ops`） |

### 4.4 更新（设置、可用性、机器本身变了）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-M1 | settings 变化（启停 / ComputerUse / 声明基线 / 默认 / grant 缩窄）→ 投影 → 已有 Session 不变 | ✅；R19 之后 Agent 一侧的 settings 只剩启停与默认，runtime 自己的配置变化由探测带进记录 |
| R-M2 | runtime 事件 → skills 状态重算：不可用 → 装在其上的安装记录标 `stale`（别的 runtime 还 `ok` 则 skill 仍 `enabled`；全 stale 降 `unavailable`）；恢复 → 自动 `tools_check`。私有侧的 `volatile` 安装在容器重启后同样重核验 | 取消（裁决 28）：没有安装记录可重算；手册 `commands` 的可用性由 `skills.status` 每次现查 |
| R-M3 | `runtimes.check {runtime_id, commands?}` 带命令清单；`buckyos skill check --runtime <id>` 重核验 | ⚠️ `runtimes.check {commands}` 保留；`buckyos skill check --runtime`（重核验安装记录）随裁决 28 待拆 |
| R-M4 | 声明基线从 `extended` 降为 `standard`：不改安装记录，过滤自然变；提示影响的 skill 数 | ⚠️；声明基线挪到 runtime 上（R19） |
| R-M5 | 节点的账户 home 变了（`describe` 报的 home 与记录不同）才视为“新机器”，安装记录标 `stale`；roots 变化只影响边界，不动 Agent 目录 | 取消（裁决 28）：节点上不再有安装记录 |
| R-M6 | `update` 的 CAS 与 `.locks/runtimes.lock` 保持；`tools_check` 写入走同一把锁 | ✅ |
| R-M7 | `grant.expires_at_ms` 到期（裁决 26）：现算为 `runtime_unavailable`，记录仍是 `active`，列表里标“已到期”；不写 settings、不转 retired；URL 重置（executor_id 不变）或延长 grant 后自动恢复。到期时通知 owner，并按 R-D2 标记进行中的 Session | ⚠️ 现算的不可用与自动恢复已做；通知与 Session 标记未做（R14，与 R8 一起） |
| R-M8 | 共用机器（owner 的 desktop）：runtime 表与 `desc` 标“与 owner 共用”；`system` 安装确认文案点明“会装到你的电脑上”；ComputerUse 冲突沿用草案 §8.1 租约与人工接管 | ⚠️ runtime 列表与 `desc` 的“与 owner 共用”未做；`system` 安装的确认文案随裁决 28 失效 |

### 4.5 删除（禁用、取消登记、Agent 删除）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-D1 | 内建只能禁用，不能 `unregister`；**私有不能禁用**。影响清单 `runtimes.impact`（对齐 `workspaces.runtime_impact`）：活跃 Session、pending 且写了 `--runtime` 的 Todo、绑定的 Workspace、它是否是 owner 默认。**停用**（裁决 27）：清单里没有活跃 Session 和 pending Todo 时直接生效，有就先列出再确认；Workspace 只显示不拦。**从 Agent 移除（`runtimes.remove`）与目录页的 revoke** 始终先展示清单再确认。CLI 同一规则，`--yes` 跳过 | ⚠️ 内建不能 `unregister`、私有不能停用已有；`runtimes.impact` 与确认未做（R7），现在只有 `workspaces.runtime_impact` |
| R-D2 | 禁用后的进行中 Session：Loader 投影时标 `runtime_unavailable` 并通知 owner，不自动迁移 | ❌ |
| R-D3 | `unregister`（非内建）：同 R-D1 + `--purge-remote`（删节点上 `.opendan/<agent>/`，缺省不删）；skills 记录里该 runtime 条目删除 | ⚠️ `unregister` = retire 已做（R1）；影响清单、`--purge-remote` 未做；“清 skills 记录”随裁决 28 取消。Agent 视角的“删除”是 UI TODO RT1 的 `runtimes.remove`（§9.2） |
| R-D4 | Agent 被删除（裁决 30）：节点上的 `.opendan/<agent_id>/` 不自动清，与裁决 10 一致；删除确认框列出留有它目录的 runtime 与路径（来源是它的 `settings.runtimes`），由 owner 决定是否手工删。skills / runtime 记录随 Agent 目录消失 | ❌ 确认框未做（R18） |

### 4.6 入口（§0.6 的两步流程）

| # | 需求 | 现状 |
| --- | --- | --- |
| R-E1 | 用户级 runtime 目录 `agent.runtime_catalog`（每个用户一份）。本轮两种来源（裁决 23，ssh 不做）：节点 offer（只有 zone owner 有）与 daemon 设备；非 zone owner 的用户目录里只有自己签发的设备。每条带 connector、它自己的配置（os、desktop、ops、roots、ComputerUse、到期、声明基线），以及 `assigned_to: [{agent_id, enabled, default}]`（扫 owner 全部 Agent 的 settings）。`agent.runtime_directory` 保留为它按单个 Agent 的投影 | ❌（directory 无反查、无设备） |
| R-E2 | zone owner 在 Desktop 目录页上改节点这台 runtime 的配置：开 / 关、roots、ComputerUse 档位、声明基线。新 kRPC `node.owner_agents_set`（写 `nodes/<n>/settings.executor.owner_agents`，只允许 zone owner），不动 node_daemon 的 seed 逻辑。对有策略的节点都可用：Desktop 节点，以及激活时选了 OS 账号的 Linux 节点（缺省与该账号权限一致，改的时候保留 `run_as`；裁决 25、§7.2 推定 3）。只改已有策略，不替节点新建；没有策略的节点显示为未开放 | ❌ |
| R-E3 | daemon 设备（§10）：签发表单定的就是这台 runtime 的配置（roots、只读 / 读写、ComputerUse、期限、基线），之后不改，要别的配置就再签一台。`executor.device.create` 的 Agent 选项**可选**，选了就是签发 + 分配一次做完。分配走 `agent.update {runtimes: {"exec-<id>": {enabled}}}`；同一台设备同时只有一个 Agent 启用，executor profile 的 `principals` 就是启用的那个 Agent，由 control panel 按分配重算，不重签 URL（裁决 18 已核） | ❌。服务端允许 `principals` 为空；Desktop 表单在签发时当场选一个 Agent；没有按分配重算 `principals` 的逻辑，也没有独占检查 |
| R-E4 | ssh 登记上提到用户级。**本轮不做**（裁决 23）：没有 ssh 执行器，登记了也恒为不可用；私钥上提后怎么存放没有设计。原方案留作以后参考：`users/<user>/agent_runtimes/<id>`（connector ssh、roots 必填、grant、到期），RBAC 为本人读写、zone owner 可读；只能分给自己的 Agent；Agent State 里的 `remote_ssh` 记录改为投影。`xagent runtime register --connector ssh` 留给开发，标 `registered_by: dev` | 本轮不做 |
| R-E5 | Agent 侧使用账本：`RuntimeRecord.usage {sessions_total, active_sessions, last_used_at_ms, last_session_id, workspaces}`，在 `freeze_runtime` / 绑定 / Workspace 创建时更新（`skill_commands` 随裁决 28 去掉）；`SessionSummary` 加 `runtime_id`，sessions 列表可按 runtime 过滤。从 Agent 移除、设备被撤回、取消登记的记录**永久保留**（已裁决），改 `lifecycle: retired`（保留 `usage`，`unregister` 的语义即 retire），这就是“历史上用过的”；retired 记录不参与候选、不出现在 plan 全表。**到期不算 retired**（裁决 26，见 R-M7） | ⚠️ `usage`（Session）与 `lifecycle` 已做；Workspace 记账、`active_sessions`、`SessionSummary.runtime_id`、按 runtime 过滤未做；已实现的 `skill_commands` 待拆（R17） |
| R-E6 | Agent 自己的 Home page（opendan WebUI Home，已裁决只做这一处）的 runtime 面板：当前列（§5.3）+ retired 区 + 每条的 usage 与最近 Session 链接；启用 / 停用 / 默认 / 移除在这里做。**runtime 的配置不在这里改**（裁决 24、§7.2 推定 2）：权限、ComputerUse、声明基线是 runtime 自己的，在 Desktop 的 Settings → Agent Runtimes 里管；这个面板的列表来自那里的分配，配置只读显示。Desktop `AgentDetailPage` 的卡片保持只读，加一个跳到该面板的链接 | ⚠️ `RuntimeList` 已搬到 Home。形态改为“瘦列表（三类）+ 详情页 + 历史”，由 UI TODO RT1–RT4 实施（§9.2） |
| R-E7 | 创建向导的 runtime 步列出目录里全部可用条目（不止 `host`），缺省勾选按 `host_by_default`；已被别的 Agent 启用的设备不可选；向导不再有按 Agent 的 ComputerUse 选项（§10） | ⚠️ |
| R-E8 | CLI：`buckyos runtime list\|assign\|unassign\|revoke`（`assign` 只选 Agent；`add-ssh` 本轮不做，裁决 23）、`buckyos agent runtime list\|enable\|disable\|default`；输出与 WebUI 同一视图 | ❌ |
| R-E9 | 分配 / 撤销走 R-D1 的影响清单：撤销前列出该 Agent 在其上的活跃 Session、pending Todo、Workspace；用户级 revoke 一台设备时列出正在启用它的那个 Agent 的影响（同时只有一个，§10）；把设备换给另一个 Agent = 先按同一规则停用原来的 | ❌ |

## 5. 两个分类视角（脑图，2026-11-04）

分类只用于 UI 分组、登记预设和这份文档的讨论，不是记录字段（§1）。 2026-11-09 第二轮：登记预设取消（§10）；§5.1 的“缺省 grant”一列读作这类 runtime 自己通常的配置，其中 `install`、`workspace` 两项作废。

### 5.1 用户视角：四类 runtime

| 类 | 它是什么 | 对应 | 缺省 grant（登记预设） | 期限 |
| --- | --- | --- | --- | --- |
| **Agent 自用** | Agent 自己的电脑 = §0 的私有 runtime，拥有 `$BUCKYOS_ROOT` 布局 | `private`（`connector: native`） | 全部；`workspace = true`；`install = agent_bin + system(持久卷)` | 永久 |
| **主人的主要工作环境** | owner 日常用的 PC；**Agent 和人共用一台机器**（claude code / codex 模式）。没有 PC 的纯娱乐用户不存在这一类 | `host`，及 `device_type = desktop` 的 `node-*` | 节点策略上限；`workspace = true`；`install = agent_bin + system(owner 确认)` | 长期 |
| **可管理设备** | TV、路由器、NAS。鼓励用 Device DID Object Card 管理（`obj://…/devices/<id>` → Card / Profile → 属性、动作、事件），不是开 shell | 跑了执行器的非 desktop 节点；Android TV daemon 未实现（daemon TODO §7.4）；`buckyos/src` 下尚无 `impl DIDObjectServer` | `ops = 文件读 + computer_observe`；`workspace = false`；`install = none` | 长期 |
| **临时管理设备** | 帮朋友修电脑：对方跑 `buckyos_agent_daemon` 用 URL 接入（form B）；或 owner 手工 `remote_ssh`（本轮不做，裁决 23） | `exec-<executor_id>`、`remote_ssh` | 按 URL 签发的 profile / `--root`；`workspace = false`；`install = none`；**`expires_at_ms` 必填** | 临时 |

### 5.2 能力视角：六个问题 → 字段

| 问题 | 字段 | 现状与结论 |
| --- | --- | --- |
| computer_use? | `grant.computer`、`caps.desktop` | `none / observe / control` 三档已有；草案决策 5：缺接口时的最后手段。**网络 KVM 不需要新 connector**：它是 `computer = control` 且 `ops` 只含 `Computer*` 的 runtime |
| 能安装 cmd? | `grant.install`、`caps.tools_check` | 裁决 28 之后只有一条路：装扩展包（R-S1），由管理员在节点上装，不按 Agent 授权。skill 不带 bin，也不带安装脚本；`grant.install` 删除（§7.2 待定 2，R19） |
| read only? | `grant.read_only`（由 `ops` 推） | 记录上已能看到（`grant.read_only()`，R1），`freeze_runtime` 据此拒绝会写的 work → R-U11 |
| filesystem? | `grant.roots`（权限边界） | Agent 目录按 §0.3 目录约定，不占字段；roots 现混用“边界”和“建议位置”→ R-K5；WebUI 当“文件夹权限列表”展示，Agent 级只能缩 |
| run cmd? | `ops ∋ Exec` | 不另设字段；Agent 级非 Session 执行是 R-U3 |
| 能否建 workspace? | 派生，不设字段 | workspace 本身就是产物、长期维护：runtime 可用、不是只读、没有期限才能建（§10）→ R-U10 |

### 5.3 plan 全表（R-W7 的 `<runtimes>` 块列定义）

`id / 分组 / os / baseline / 可用 / 默认 / computer / read_only / roots / ExtTools 已装 / 到期 / desc`。私有一行永远在最前。原有的 install、“装了哪些 skill 命令”两列随 §10 去掉（R17、R19）。

## 6. 实施任务（分期）

**P0 工作模型**（§0；先做，别的都建立在它上面）

| # | 任务 | 落点 | 状态（2026-11-09） |
| --- | --- | --- | --- |
| W1 | 私有 runtime 永远可用（R-W1）：`private` 别名、原生运行时 `connector: native` | `protocol/runtime.rs`（`builtin_runtimes`、`canonical_runtime`）、Loader | ✅ |
| W2 | `freeze_runtime` 按类别分叉：UI 固定私有；work 四级（R-U1）；`selected_by` 加 `private / skill` | `api.rs`、`template.rs`、`session_config.schema.json` | ✅；`skill` 这一级按裁决 28 取消，skills S20 已实现的待拆（R17） |
| W3 | 单机 session 边界落文档与代码注释（R-W10）：Session Directory Protocol §9、primary-runtime TODO §6.1 改为不做；确认 `report` 在节点 session 不依赖 helper | Session Directory Protocol、`runner/reports.rs` | ✅ |
| W4 | `agent-session run --runtime <id\|workspace>`（R-W5）；依赖 R2 的 `runtime_exec` | `xagent_cli.rs`、`session.helper` 路由、`home.rs` | ✅（超时不转 task，裁决 29） |
| W5 | 提示词：chat 两行 / work 工作 runtime + 私有 / plan 全表 `<runtimes>`（R-W3、R-W7） | `runner/assembler.rs`、`runtime_status` | ✅ |
| W6 | 三份 behavior 与 `ui_core.md` 重写（R-W3、R-W4、R-W7、R-W11） | `behaviors/analyze_intent.toml`、`plan_simple.toml`、UI 模板、`cmd_skills/ui_core.md` | ✅ |
| W7 | （取消：不加 skill 字段）→ 见 P2 的 R16 | — | — |
| W8 | 绝对路径解释（R-W8）：与 object-url TODO A 段合并推进，key url.md 补“chat 输入路径 = owner 默认 runtime 设备”一段 | `read_tool.rs`、`doc/key url.md`、`ui_core.md` | ✅ |

**P1 记录模型**（§1）

| # | 任务 | 落点 | 状态（2026-11-09） |
| --- | --- | --- | --- |
| R1 | `runtime_record` v4：去 kind，加 connector / caps / grant / desc；`tools_check`；`check` 带 `commands?`；`register` 后立即 check（R-K1–R-K7、R-C2、R-C3） | `protocol/runtime.rs`、`state/runtimes.rs`、`schema/runtime_record.schema.json`、Agent State Protocol §10 | ✅ |
| R2 | Agent 级原语 `runtime_exec` / `runtime_put`（R-U3），封装 `run_probe` + `RuntimeFiles` | `lib_opendan/src/runtime/mod.rs` | ✅ |
| R10 | grant 交集在 control panel 算好写进 settings（已裁决 8），opendan 投影只读；登记预设（R-K6、R-U8）| `control_panel/agent_runtimes.rs`、`state/runtimes.rs` | 取消（§7.2 推定 2）：没有按 Agent 的 grant，不需要交集；已做的 settings 侧随 R19 删 |
| R11 | form B executor 接入 Session（R-C8）：`RemoteNodeConfig` 加 `executor_id` 分支，`node_exec` 对应改，`expires_at_ms` 投影 | `agent_tool/runtime/node.rs`、`protocol/runtime.rs` | ⚠️ 已做，未在真实 daemon 上验证 |
| R12 | `workspaces.create / import` 与 `freeze_runtime` 读 grant（R-U10、R-U11） | `state/workspaces.rs`、`api.rs` | ⚠️ 可用性与只读已接；Workspace 规则改为派生（不是只读、没有期限），`grant.workspace` 随 R19 删 |
| R19 | 去掉按 Agent 的授权层（§10）：`AgentRuntimeSettings` 收成 `{enabled, target}`，删 `AgentRuntimeGrant` / `InstallGrant`；ComputerUse 档位与声明基线改从 runtime 的来源取（节点策略、设备记录各加一个可选的 `baseline`；scheduler 的 `profile_for` 用策略的 `computer`）；记录的 `grant` 删 `install`、`workspace`，只留探测来的 `ops / roots / computer / expires_at_ms`；`runtimes.settings_set` 与 `agent.update` 的参数收成启停与默认；创建向导去掉按 Agent 的 ComputerUse；`runtime_record` 升 v5，schema、fixtures、WebUI 类型、Agent State Protocol §10 同步 | `buckyos-api`（`app_schema.rs`、`node_executor.rs`）、`scheduler/executor_auth.rs`、`control_panel`（`agent_runtimes.rs`、`agent_daemon.rs`）、`lib_opendan`（`protocol/runtime.rs`、`state/runtimes.rs`）、`opendan/src/home.rs`、`opendan/web`、Desktop 向导 | ❌ |

**P2 skills 接口**（§3；与 skills TODO §3.6.1 C / D 同步）

| # | 任务 | 落点 | 状态（2026-11-09） |
| --- | --- | --- | --- |
| R3 | 节点家目录 `tools/<skill>/bin` + PATH 前置（R-U4） | `runtime/mod.rs`、Session Directory Protocol、node executor 侧 | 取消（裁决 28）；已实现的“放到节点”待拆（R17） |
| R13 | 私有 `system` 安装持久性核验（R-U9） | `skills.rs`、`install.toml` schema | 取消（裁决 28）；skills S19 已实现的待拆（skills TODO） |
| R16 | ExtTools 全平台包 + `EXTENDED_TOOLS` 清单（R-S1、R-S3）。已由[扩展包 TODO](opendan-exttools-cross-platform-todo.md) 一、二期交付：安装器是 node_daemon 的 reconciler，入口是 Desktop “设置 > 扩展工具”与 `buckyos ext-tools`，**没有** `runtime ext-tools install` | `publish/exttool/`、`baseline.rs`、node_daemon、Desktop 设置页 | ✅ 扩展包 TODO |
| R4 | R-U1 第四级 `skill` | `api.rs` | 取消（裁决 28）；skills S20 已实现的 `skill_runtime` 待拆（R17） |
| R17 | 拆掉 runtime 侧为 skill 自带命令做的东西（裁决 28），与 skills TODO 的拆除一起做：`freeze_runtime` 的 `skill` 级与 `RuntimeSelection::Skill`、`usage.skill_commands` 与 `RuntimeUsageEvent::SkillCommand*`、plan 全表的 skill 命令列、Session PATH 的 `tools/<skill>/bin` 层、`runtime_exec` 前置的 `<agent_root>/tools`。`grant.install` 随 R19 删除 | `api.rs`、`protocol/runtime.rs`、`state/runtimes.rs`、`runtime/mod.rs` | ❌（§7.2 推定 4、5 已确认：删除） |

**P3 更新与删除**

| # | 任务 | 落点 | 状态（2026-11-09） |
| --- | --- | --- | --- |
| R6 | runtime 事件 → skills 状态重算（R-M2） | `skills.rs`、`loader.rs`、`state/runtimes.rs` | 取消（裁决 28）；skills S21 同时取消 |
| R7 | `runtimes.impact` + 禁用 / 取消登记前确认 + `--purge-remote`（R-D1、R-D3） | `home.rs`、`service.rs`、`web/src/pages/Workspaces.tsx`、`buckyos runtime` 模块 | ❌；确认规则按裁决 27 |
| R8 | 禁用时标记进行中 Session 并通知 owner（R-D2） | `loader.rs`、`notify` | ❌ |
| R14 | 到期处理（R-M7） | `loader.rs` | ⚠️ 到期 → 不可用已做；剩通知与 Session 标记，与 R8 一起做（裁决 26） |
| R18 | Agent 删除确认框列出留有 `.opendan/<agent_id>/` 的 runtime 与路径（R-D4，裁决 30） | `desktop/src/app/users-agents/components/detail/AgentDetailPage.tsx` | ❌ |

**P4 入口**

| # | 任务 | 落点 | 状态（2026-11-09） |
| --- | --- | --- | --- |
| R9 | owner 从 WebUI / CLI 新增目录节点与 `remote_ssh`（R-C4）；并入 E5 / E7 | WebUI、`buckyos-tool/cli/modules/` | ❌（并入 E5 / E7；`remote_ssh` 不做，裁决 23） |
| R15 | runtime 列表按四类分组、登记预设、列对齐 §5.3；`remote_ssh` 要求 `--root`（R-C7、R-K5） | `Workspaces.tsx`、`xagent_cli.rs`、`buckyos runtime` | ❌；登记预设取消（§10），只剩列表分组与列对齐 |
| E1 | 用户级目录 `agent.runtime_catalog` + `assigned_to` 反查（R-E1）；`node.owner_agents_set`（R-E2） | `control_panel/agent_runtimes.rs`、`agent_mgr.rs`、node settings RBAC | ❌；来源只有节点与 daemon 设备（裁决 23）；`node.owner_agents_set` 对有策略的节点都可用（裁决 25、§7.2 推定 3） |
| E2 | daemon 设备两步化：`principals` 由分配推导；`exec-<id>` 分配写 settings（R-E3，依赖 R11） | `control_panel/agent_daemon.rs`、`agent_runtimes.rs apply_update`、`AgentDevices.tsx` | ❌；配置签发时定死，同时只有一个 Agent 启用，独占检查放在 `agent.update`（§10） |
| E3 | ssh 登记上提到用户级并投影（R-E4） | `users/<owner>/agent_runtimes` 键与 RBAC、Loader `project_runtimes`、`xagent_cli.rs` | 本轮不做（裁决 23） |
| E4 | 使用账本与 retired（R-E5）：`usage` 字段、`SessionSummary.runtime_id`、`lifecycle` | `protocol/runtime.rs`、`api.rs freeze_runtime`、`protocol/summary.rs`、`state/runtimes.rs` | ⚠️ `usage` / `lifecycle`、Session 记账已做；Workspace 记账、`SessionSummary.runtime_id` 未做；`skill_commands` 待拆（R17） |
| E5 | Desktop 目录页（Settings → Agent Runtimes，合并 Cluster Manager 的 Add device，Agent 选项可选）、Agent 详情加链接、向导全目录（R-E3、R-E7） | `desktop/src/app/settings/pages/`、`AgentDetailPage.tsx`、`agent-setup/` | ❌；分配只选 Agent，没有 grant 对话框（§10）；UI TODO RT3 的深链落在这页（§9.2） |
| E6 | opendan Home 的 runtime 面板（当前 + retired + 可编辑）+ Workspaces 页瘦身（R-E6） | `opendan/web/src/pages/Home.tsx`、`Workspaces.tsx`、`home.rs set_runtime_settings` | 移交 UI TODO RT1–RT4（§9.2） |
| E7 | CLI 两组命令（R-E8） | `buckyos-websdk/cli/modules/runtime.ts`、`agent.ts` | ❌；没有 `add-ssh`（裁决 23） |

依赖：W1 → W2 → W3 → W5 / W6 可并行；W4 依赖 R2；R16 依赖 R2；W8 跟 object-url TODO。R1 可与 P0 并行但要在 W2 合并前定 `FrozenRuntime` 的新形状。P2 在 R1 / R2 后；P3 / P4 最后。入口里 E1 / E4 不依赖别的，可以先做（E4 的 `usage` 字段随 R1 的 schema 升版一起）；E2 依赖 R11；E3 依赖 R1；E5 / E6 / E7 在 E1–E4 后。 2026-11-09：剩余任务的顺序见 §9.3。

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

7. ✅ 已裁决：`role` 降为登记预设 / UI 分组，不是字段（R-C7）。 2026-11-09：预设也取消，只留界面分组（§7.2 推定 2）。
8. ✅ 已裁决：control panel 算好交集写进 settings，opendan 投影只读不再乘（R10 落点在 `control_panel/agent_runtimes.rs`）。 2026-11-09：作废（§7.2 推定 2：没有按 Agent 的 grant，不需要交集）。
9. ✅ 已裁决：能作为执行器添加成功就进目录，不另设门槛（R-C9，R-K6 改为预设）；Card 设备大概率经 HA 一类服务接入，添加路径不同，不在本文范围。
10. ~~临时 runtime 到期后的记录保留多久~~ → 已裁决（见 21）：永久保留为 retired；远端 `.opendan/<agent>/` 不删，只在 `--purge-remote` 时删。
11. ✅ 已裁决：允许但必须落持久卷；ExtTools 里已有的命令不允许 skill 自己装（R-U9）。 2026-11-09：被裁决 28 取代（skill 不带二进制，也就没有自装）。
12. ✅ 已裁决（2026-11-04）：以前搞复杂了。不加 `home` 字段、不迁移，按 §0.3 的四条目录约定：Agent 在任何 runtime 上的目录都在该 runtime 上 owner 的 home 下；`roots` 只是边界。唯一要补的是 executor `describe` 报账户 home（R-K5）。
生命周期（沿用）：

13. ✅ 已裁决：节点上没有 helper，PATH 前置走每次 exec 的 `env_overrides.PATH`，不改协议（R-U4）；cwd 是另一件事，由 Session 目录 / workspace 目录决定，shell 的当前目录就是它。 2026-11-09：PATH 前置这一半随裁决 28 取消（节点上没有 skill 的 `bin` 可前置）；cwd 的说法不变。
14. ✅ 已裁决：只在 skill 安装、ExtTools 安装与显式 `check --commands` 时跑，`check_runtimes` 后台只核标准清单（R-C3）。
15. ✅ 已裁决：取默认优先；skill 自装命令缺省装私有，这一级很少触发（R-U1）。 2026-11-09：被裁决 28 取代（没有 skill 落点这一级）。
16. ✅ 已裁决：先不做；只在拒绝信息里提示 owner 去目录开（R-C5 不排期）。
17. ✅ 已裁决（随第 2 点澄清）：chat 是私有 default，天然有 `agent-session run`；机制上不限制，`ui_core.md` 引导为轻量读取，持续的活建 work（M3）。

入口：

18. ✅ 已裁决（2026-11-04）：签发时 Agent 可选，不选也可以。剩一项技术确认：daemon 侧鉴权只看 profile 的 `principals`，重算后不需要重签 URL。 ✅ 已核（2026-11-09）：control panel 每个心跳周期重新生成配置快照（设备记录的 `profiles` 加各 Agent 的 binding）并推给在线的 daemon，离线的在重连时拿到；daemon 的授权只读快照里的 `profiles.principals` 与 `principal_bindings`；URL 凭据是单独的摘要，只有 `executor.device.issue` 会换。所以重算 `principals` 只要改设备记录，不用重签。要补的是修改已有设备 `profiles` 的入口（现在只有 create / issue / revoke / delete）。
19. ✅ 已裁决：任何有 Agent 的用户都能加 ssh / 设备；`users/<u>/agent_runtimes` 本人读写、zone owner 可读，只能分给自己的 Agent（R-E4）。 2026-11-09（裁决 23）：ssh 本轮不做，这条目前只对 daemon 设备生效。
20. ✅ 已裁决：Agent 自己的 Home page（opendan WebUI Home）是第 2 步唯一落点；Desktop `AgentDetailPage` 只读 + 链接（R-E6）。
21. ✅ 已裁决：retired 记录永久保留，`unregister` 的语义即 retire（R-E5、R-M7）。 2026-11-09（裁决 26）：到期不算 retired，见 R-M7。

skills：

22. **方向已定，排期待安装包改造**（owner 2026-11-04；2026-11-06 补：[扩展包 TODO](opendan-exttools-cross-platform-todo.md) 的三期（standard 分发）就是这里说的安装包改造中 Agent 工具的部分，其它服务的安装包重构另起，R-S1 的扩展包不再等它）：Desktop Lite 会是主流，私有 runtime 就是 host，所以 **host 要满足 Agent runtime 的 baseline**，`agent-session` 留在标准清单并照常布到 host（它在 Lite 下本来就是私有侧的命令）。不是私有的 host 上它装着但手册不引导。整个构建 / 安装包的修改目标还没定（aiws 那边还有不少“有则用”的逻辑），本轮改造完后综合各基础服务的需求统一调整安装包；R-S1 的 ExtTools 全平台包并入那次调整。难点：ffmpeg 一类命令的全平台化比 Docker 镜像打包麻烦。 2026-11-09：扩展包（R-S1 / R16）已经先行交付，这条只剩 standard 分发（扩展包 TODO 三期，未排期）。

### 7.1 2026-11-09 新增裁决（owner 当日答复）

下面 8 条是核对现状时发现的：有的是本文前后不一致，有的是别的单实施后与本文对不上，有的是代码里的事实让原计划站不住。每条保留当时写的现状与选项，owner 的批注原样留在下面，“✅ 已裁决”是整理后的结论和回写位置。回写时的推定与还没定的问题在 §7.2。

23. **ssh runtime 本轮做不做。** 挡着 E3、E1 的目录来源、E7 的 `add-ssh`、R-C4。
    - 现状（已核）：`connector: ssh` 的记录可以登记，但没有 ssh 执行器。`state/runtimes.rs` 的 `observe` 对它恒给 `runtime_unavailable`（“this runner cannot host sessions on ssh runtimes yet”），所以不能绑 Session，`runtime_exec`、`check`、`agent-session run / copy` 也都到不了它。
    - R-E4 / E3 要做的是把 ssh 登记上提到用户级，加 RBAC、投影、CLI，全部建在一个不能执行的 connector 上。另外 `identity_file` 现在是 Agent 环境里的一个路径；上提以后一条登记要分给多个 Agent，私钥放在哪里、怎么进各个 Agent 的容器，本文没有设计。
    - A（建议）：本轮拿掉 E3 和 `add-ssh`。用户级目录先只有节点和 daemon 设备两种来源；“帮朋友修电脑”由 daemon URL 覆盖（§5.1 第四类）。ssh 等有执行器时另起一单，私钥存放一并定。`xagent runtime register --connector ssh` 留给开发。
    - B：本轮先补 ssh 执行器（exec、文件读写、describe，对齐 Node Executor 的操作集），再做 E3，并定私钥存放。

> 按你的建议

   ✅ 已裁决（2026-11-09）：选 A。本轮不做 ssh：E3 与 `buckyos runtime add-ssh` 拿掉，用户级目录只有节点与 daemon 设备两种来源；`xagent runtime register --connector ssh` 留给开发。有 ssh 执行器时另起一单，私钥存放一并定。已回写 R-E1、R-E4、R-E8、R-C4、E1、E3、E7。

24. **每个 Agent 的 grant 在哪里改。** 挡着 R10、E5 的分配对话框、UI TODO RT2 的详情页。
    - 现状（已核）：`AgentRuntimeSettings.grant {install, workspace, roots, expires_at_ms}` 有两条写入路径。control panel 的 `agent.update` 不认 `grant`（新条目写 `grant: None`），裁决 8 要它算的交集没有做。opendan 的 `runtimes.settings_set` 接受 `grant` 并原样写进 settings，不对照节点策略；WebUI 没有用它，卡片只显示 grant。也就是说现在没有任何界面能改 grant，唯一能改的接口绕过了裁决 8。（`computer` 也是两条路径都能写，只有 control panel 对照节点上限。）
    - R-E6 原文是“grant 缩窄也在 Agent 首页改”；UI TODO 的 D4 说 Agent 首页只谈 enable / disable / delete。
    - A（建议）：grant 是“分配”的一部分，只在用户级目录里改：目录页的“分配给 Agent”对话框和 `buckyos runtime assign`，走 `agent.update`，交集在 control panel 算（R10）。Agent 首页的详情页只读显示 grant；opendan 的 `runtimes.settings_set` 去掉 `grant` 参数。
    - B：两边都能改（R-E6 原文）。opendan 这条路只允许缩窄，为此要让它读得到上限：control panel 把上限一并写进 settings，或 opendan 每次改之前去问 control panel。

> 在settings的agent runtime里管理（现状一个agent runtime只能分配给一个agent), agent首页里能看到的runtime list来自这里

   ✅ 已裁决（2026-11-09）：在 Desktop 的 Settings → Agent Runtimes（E5 的目录页）里管理；Agent 首页的 runtime 列表来自这里的分配，首页不改 grant。owner 注：现状一条 agent runtime 只分配给一个 Agent。§7.2 推定 2 的答复进一步去掉了按 Agent 的 grant：在 Settings 里管的是 runtime 自己的配置，分配只是把它交给哪个 Agent（§10）；R10 取消，opendan `runtimes.settings_set` 的 `grant / computer / baseline` 参数随 R19 去掉。已回写 §0.6、R-E6、R-E8、E5 与 UI TODO 的 RT1、RT2。

25. **`node.owner_agents_set` 管到哪一步（R-E2）。** 挡着 E1 的这一半和 E5 的节点行。
    - 现状（已核）：`owner_agents` 策略由 node_daemon 种一次。Desktop 节点按交互用户的 home 种，`computer: control`，不带 `run_as`。Linux 节点只有激活时选了 OS 账号（节点本地的 `etc/host_account.json`）才种：roots 是该账号的 home（root 账号再加 `/`），`computer: none`，非 root 账号带 `run_as {uid, gid}`；没选账号的 Linux 节点没有策略，也不会补种。种过以后 `enabled = false` 被尊重，不会重开。
    - R-E2 写的是“开 / 关节点、改 roots 与 ComputerUse 上限”，没有区分这几种情形。风险在 Linux：策略里没有 `run_as` 就是以 node_daemon 的身份（root）执行，所以这个接口不能替没有策略的 Linux 节点造一条出来，改已有策略时也不能丢掉 `run_as`。
    - A（建议）：接口只改已有策略的 `enabled`、`workdir_roots`、`computer`，其余字段（`run_as`、`env`、`ops`）原样保留；节点没有策略时拒绝，目录页显示“未开放：激活时没有选择给 Agent 用的账号”。在 Desktop 上给 Linux 节点选账号另开一项，本单不做。
    - B：本单一并做“在 Desktop 上为 Linux 节点选账号”。要新增节点侧接口（列出可选账号、写 `host_account.json`、补种），并定谁有权选 root。

> Linux下没desktop版本，所以没有 owner default runtime

   ✅ 已裁决（2026-11-09）：Linux 没有 Desktop 版本，也就没有 owner 默认 runtime，这个情形不用处理，B 不做。按 §7.2 推定 3 的答复，Linux 节点作为 runtime 仍要支持：激活时选了账号的节点照常进目录，`node.owner_agents_set` 对它同样可用（保留 `run_as`）。接口只改已有策略，不替节点新建。已回写 §0.6、R-E2、E1。

26. **到期的 runtime 算什么状态（R-M7、R-E5、裁决 21）。** 挡着 R14 和 E4 的 retired 语义。
    - 本文前后不一致：R-M7 写“到期 → 自动禁用……URL 重置恢复可用；记录永久保留为 retired”，R-E5 也把“到期”列进 retired。自动禁用（写 settings）和 retired 都会让“重置后恢复可用”做不到，除非再定义复活规则。
    - 现实现（已核）：到期是每次现算的不可用（`grant.expired_at(now)`，提示续期后恢复），不写 settings、不转 retired；URL 重置或延长 grant 后，下一次投影 / check 自动恢复。
    - A（建议）：按现实现定。到期的记录仍是 `active`，留在当前列表里标“已到期”，可以续期；retired 只来自三件事：从 Agent 移除（`runtimes.remove`）、设备在目录里被 revoke / delete、`unregister`。R14 剩下的是到期时通知 owner、标记进行中的 Session（与 R8 同一条路）。
    - B：到期即 retired，URL 重置时由投影把同 id 的记录改回 `active`（usage 延续）。当前列表更干净，但多一条状态迁移，而且“到期”和“撤回”在历史里分不出来。

> 按你的建议

   ✅ 已裁决（2026-11-09）：选 A。到期是现算的不可用，记录仍是 `active`、标“已到期”，续期后自动恢复；retired 只来自从 Agent 移除、设备被 revoke / delete、`unregister`。已回写 R-M7、R-E5、R14、裁决 21 与 UI TODO 的 RT2。

27. **停用 runtime 要不要先确认（R-D1 对 UI TODO RT2）。** 挡着 R7 的交互和 RT2。
    - R-D1 写“禁用前给影响清单，WebUI 与 CLI 先展示再确认”。UI TODO RT2 写“停用可逆，开关直接生效，不弹确认”，并注明等本文的 R7。R7 做完以后两边要对齐。
    - A（建议）：清单里没有“正在进行的东西”（活跃 Session、pending 且点名它的 Todo）时开关直接生效；有就弹一次确认并列出。绑定的 Workspace、装在上面的 skill 命令因为停用可逆，只显示不拦。从 Agent 移除（`runtimes.remove`）和目录页的 revoke 始终先展示清单再确认。CLI 同一规则，`--yes` 跳过。
    - B：停用一律先展示清单再确认（R-D1 原文）。
    - C：停用一律不确认（RT2 原文），清单只用于移除和 revoke。

> 按你的建议

   ✅ 已裁决（2026-11-09）：选 A。停用时清单里没有活跃 Session 和 pending 且点名它的 Todo 就直接生效，有就先列出再确认；Workspace 只显示不拦。从 Agent 移除与目录页的 revoke 始终先展示清单再确认。CLI 同一规则，`--yes` 跳过。已回写 R-D1、R7 与 UI TODO 的 RT2。

28. **ExtTools 已有的命令，skill 包能不能自带同名的（R-U9、裁决 11）。** 只影响 R-U9 的文字，或 skills 的一处实现。
    - 现状（已核）：skills 的实施只拦安装脚本：`install.toml` 的安装脚本要装的命令在 ExtTools 清单里就拒绝。包在 `tools/<name>/bin` 里自带同名命令是允许的，有用例固定（`a_skill_cannot_shadow_a_standard_command_but_may_shadow_an_extension_one`）；标准命令不允许覆盖。R-U9 的字面是“拒绝由 skill 自己装”，没有区分这两种。
    - 覆盖范围：PATH 只前置本 Session 冻结的那些 skill 的 `bin`，所以同名命令只在点名了这个 skill 的 Session 里生效，不碰系统位置，也不影响别的 Session。
    - A（建议）：接受现实现，R-U9 改写为“安装脚本不得安装 ExtTools 已有的命令；包内自带的同名命令允许，只在冻结了该 skill 的 Session 里覆盖”，并在安装计划里加一条提示（覆盖了哪个扩展包命令），让 owner 在确认页看得到。
    - B：一律拒绝。改 skills 的实现和那条用例；想用特定版本的 skill 只能换个命令名。

> 我们讨论了一下: 应该规定skills不能带二进制（只是纯文本），这是一个分发的安全边界的问题。带bin的skills更接近一个ext tool包（这个包里顺手带了skills)

   ✅ 已裁决（2026-11-09，owner，讨论后）：A、B 都不取。**skill 不能带二进制，只是纯文本**，这是分发的安全边界；带 bin 的 skill 更接近一个 ext tool 包（包里顺手带了 skill）。命令因此只来自 `standard` 与扩展包两层。本文受影响的：§3 的第三层、R-U1 的 `skill` 级（R4）、R-U4（R3）、R-U9（R13）、R-M2 / R-M5（R6）取消，已经实现的部分待拆（R17）。这条改的主要是 skills 体系和扩展包格式：已在 skills TODO §0 和扩展包 TODO §3.9.1 各记了一条，那两份文档的正文还没有回写。§7.2 已确认：安装脚本也不保留，已实现的删除；skill 包依赖扩展包，各发各的 pikg；`grant.install` 删除。

29. **`agent-session run` 超时以后怎么办（§0.5、R-W5）。** 只影响 W4 的收尾。
    - §0.5 写“超时后转 task，`run` 返回 task id，`agent-session task wait|cancel` 跟进”。现实现到 `--timeout`（缺省 120 s，上限 30 min）就终止命令，返回 124 和已有输出。另外现在的 task 模型里没有 `wait`：task 结束是作为输入回到 Session 的。
    - A（建议）：不转 task，把 §0.5 那一句改成现实现；规则是“要跑很久或要持续跟进的命令建 work（单机 session）”，与 M3 一致。
    - B：实现转 task：超时不终止，登记为 Session 的 task，结束时结果作为输入回到 Session，用 `task state | cancel` 跟进。宿主要一直持有这条执行，Session 恢复后按 op id 续查。

> 按你的建议

   ✅ 已裁决（2026-11-09）：选 A。不转 task，到 `--timeout` 终止并返回 124；要跑很久或要持续跟进的命令建 work。已回写 §0.5、R-W5、W4。

30. **Agent 被删除后，节点上的目录清不清（R-D4）。** 不挡任何任务。
    - 现状（已核）：`agent.delete` 把记录置为 removed，并删掉 system config 里的 spec、key、settings、profile、info（`begin_agent_removal`）。control_panel 和 node_daemon 里都没有清节点 `<账户 home>/.opendan/<agent_id>/` 的代码。留下的是各 Session 的目录（含 `session.json` 和 token）与 skill 放过去的 `tools/`。
    - 裁决 10 定过“远端目录不删，只在 `--purge-remote` 时删”，那是 retire 的情形，Agent 还在，还能发起清理。Agent 删除以后 Agent Service 和它在节点上的 profile 都没了，没有人能再清。
    - A（建议）：不自动清，与裁决 10 一致。删除确认框列出“这些机器上留有它的目录”和路径（来源是它的 `settings.runtimes`），由 owner 决定要不要手工删。
    - B：删除流程加一步尽力清理：在撤掉身份之前让 Agent Service 对每个可达的节点删自己的目录，离线的跳过并记进删除结果。
    - C：节点侧回收：node_daemon 发现某个 Agent 的 profile 从本机配置里消失后删它的目录。daemon 设备不受管，覆盖不到。

> 按你的建议

   ✅ 已裁决（2026-11-09）：选 A。不自动清；Agent 删除确认框列出留有它目录的 runtime 与路径，由 owner 决定是否手工删。已回写 R-D4，新增任务 R18。

### 7.2 第一轮回写的推定与待定（owner 已答复）

**推定。** 下面五条是第一轮回写时按批注的意思推出来的。owner 的批注原样留着，“✅ 已答复”是结论和回写位置。

1. **裁决 24 的“一条 agent runtime 只分配给一个 Agent”按 daemon 设备的现状来读。** 目录页的分配对话框对设备做成单选。数据层的 `assigned_to` 仍是列表，因为节点今天不是一对一：Desktop 上新建的 Agent 缺省都启用 `host`（`apply_create_defaults`）。如果一对一是规则、节点也要遵守，要改 R-E1、R-E3 和新建 Agent 的缺省。（R-E1、R-E3、E5）
> 1对1是现状，规划会1对多（但只能有一个是enable的）

   ✅ 已答复：一对一是现状，规划是一对多、同时只有一个启用。按“同一台 runtime 同时只有一个 Agent 启用”写进 §10：设备 runtime 的独占检查放在 control panel，`assigned_to` 仍是列表。节点（`host`）不受这条约束（§7.3 已确认）。

2. **Agent 首页详情页现有的 ComputerUse、声明基线两项保持可改。** 它们不在 `grant` 里，裁决 24 收进 Settings 的只有 grant（install / workspace / roots / 期限）。opendan 这条路改 ComputerUse 时不对照节点上限，这个缺口还在，没有排任务。（UI TODO RT2）
> 从设计的角度，agent runtime已经有自己的capability声明了。用户应该是根据权限配置创建不同的agent runtime(本质这上是个sandbox),而不是再接一层，通过grant要求agent有限的使用agent runtime.

   ✅ 已答复：不保持。runtime 自己声明能力和权限，要不同的权限就建不同的 runtime（它本质上是个 sandbox），不再按 Agent 接一层 grant。ComputerUse 档位、声明基线、grant 都不再按 Agent 设，Agent 首页上这几项只读。裁决 8（control panel 算交集）作废，R10 取消，新增 R19。模型见 §10。

3. **Linux 节点在目录里只读。** 激活时选了账号而种下策略的 Linux 节点（主要是开发环境）照常作为可分配的节点列出，`node.owner_agents_set` 对它拒绝；没有策略的 Linux 节点不出现。（R-E2、E1、E5）
> 这个场景还是要支持的，默认和owner的账号权限一致即可

   ✅ 已答复：这个场景要支持，缺省与 owner 的账号权限一致。激活时选了账号的 Linux 节点照常是 runtime，`node.owner_agents_set` 对它同样可用（保留 `run_as`）；缺省配置沿用现有 seed（以该账号的身份执行）。已回写 R-E2、E1。

4. **裁决 28 的“纯文本”读作 skill 包里只有 `SKILL.md` 和 `references/`。** `tools/<name>/bin` 和 `tools/<name>/install.toml`（安装脚本）都不再允许：安装脚本虽然是文本，作用是往 runtime 上装可执行文件，同样越过这条边界。如果安装脚本要保留，R13、R-U9 的持久卷部分、R6、`grant.install` 要恢复。（§3、R17、skills TODO）
> 不用保留安装脚本。 安装skills就是确定性的复制

   ✅ 已答复：不保留安装脚本，安装 skill 就是确定性的复制。R13、R-U9、R6 的取消维持。

5. **已经实现并提交的那部分要拆，不是留着不用。** 涉及 skills S18–S20 里的 cmd 安装（`install.toml`、`install-cmd`、按 runtime 的安装状态、`volatile`、ExtTools 拒装）和本文的 R17；包格式、lint、样例包（`test/skills_pikg_sample/skills/tools/`）、`skill_authoring` / `opendan_agent_dev` 手册、`AGENTS.md` 里“skill 自带的命令放 `tools/<skill>/bin`”一并改。内置的 Jarvis 手册今天没有带 `tools/` 的，不受影响。拆除的清单记在 skills TODO §0，等第 4 条确认后再排。
> 删除

   ✅ 已答复：删除。R17 可以排；skills 一侧是 skills TODO §0 的 S24。

**待定。** 下面三条当时没有按任何一种写，owner 已答复。

1. **扩展包怎么“顺手带 skill”。** 现在 `app_type = ext_tools` 的包只接受本类子包，校验会拒绝里面的 `agent_skills`。要定的有两件：格式（扩展包里加 skill 子包，还是 skill 包声明它依赖哪个扩展包），以及带来的 skill 由谁装。扩展包是管理员在 Zone 级勾选的，skill 是 owner 逐个 Agent 确认的（skills TODO 裁决 4），两条路的授权人不同。这条归扩展包 TODO 和 skills TODO。
> skill包依赖ext_tools就好了，做ext tool的人顺手再发个skills pikg

   ✅ 已答复：skill 包依赖 ext_tools 就可以，做 ext tool 的人顺手再发一个 skills pikg。扩展包格式不改，两个 pikg 各发各的。依赖怎么表达：手册的 `commands` 加扩展包目录的 `provides` 已经能从 skill 反推到扩展包（扩展包 X5），先不加包级的依赖字段。

2. **`grant.install` 留不留。** 它只被 skill 的 cmd 安装读，按推定 4 拆掉以后没有使用方。建议连同 `InstallGrant` 一起去掉，§5.1 的预设和 plan 全表的 install 列随之删除。
> 按上面的推定，skills纯文本后和runtime无关了，这个也不需要了?

   ✅ 已答复：不需要。skill 是纯文本以后与 runtime 无关，`grant.install` 连同 `InstallGrant` 删除（R19）。

3. **`host_by_default` 的 Linux 分支。** 代码里 Linux 节点在“没有容器可跑”时仍会把 `host` 设成新 Agent 的默认 runtime；按裁决 25，Linux 不该有 owner 默认。改不改没有定，本文没有排任务。
> Linux下必定要有容器，这个分支不用处理...

   ✅ 已答复：Linux 下必定有容器，这个分支不处理。

### 7.3 第二轮回写后确认的两点（owner 已答复）

owner 2026-11-09 口头答复：两点都按建议。

1. **节点（`host`）受不受“同时只有一个 Agent 启用”的约束。** 我按“不受”写（§10.2）：节点的 `owner_agents` 策略是一份模板，每个 Agent 在节点上有自己的 profile（`agent-<hash>`），各算一个 sandbox，所以 Desktop 上新建的 Agent 仍然缺省启用 `host`。如果节点也要独占，`apply_create_defaults` 要改：owner 的第二个 Agent 不再自动拿到 `host`，要在目录页从第一个 Agent 那里换过来。（§10.2、E1、R-E7）

   ✅ 已答复：不受。独占只对设备 runtime 生效；Desktop 上新建的 Agent 仍然缺省启用 `host`，`apply_create_defaults` 不改。

2. **Linux 节点缺省的 roots。** 现有 seed 把 roots 定在该账号的 home（root 账号再加 `/`），比“与账号权限一致”窄。要不要放宽成整个文件系统、由 OS 权限兜底，没有改。

   ✅ 已答复：维持现状，不放宽。缺省的 roots 仍是该账号的 home（root 账号再加 `/`）；要更宽的范围由 zone owner 在目录页用 `node.owner_agents_set` 改（R-E2）。


## 8. 实施记录（2026-11-04；已于 2026-11-05 提交 `c88404aaf`）

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

本节是 2026-11-04 的清单。其中 R16 之后由扩展包 TODO 做掉；R4、R13 由 skills TODO 做掉后又被裁决 28 取消，R3、R6 也随之取消；`agent-session run` 的超时按裁决 29 就是现在这样。当前的剩余任务见 §9.3。

- **P2 / P3 / P4 全部未做**：R3 节点 `tools/<skill>/bin`、R13 持久卷核验、R16 ExtTools 全平台包与 `EXTENDED_TOOLS`、R4 `skill` 选择级、R6 skills 状态重算、R7 `runtimes.impact` 与确认、R8 禁用通知、R14 到期处理（现只做了到期 → `runtime_unavailable`，没有自动禁用与通知）、R9 / R15 登记预设与分组、E1–E3 / E5–E7 用户级目录、daemon 两步化、ssh 上提、Desktop 目录页、Home 面板、`buckyos runtime` CLI。
- `agent-session run` 超时未“转 task”：到 `--timeout`（缺省 120 s，上限 30 min）返回 124 与已有输出；长命令请加 `--timeout` 或建 work。
- `grant.workspace = false` 尚未在 `workspaces.create / import` 里强制（R-U10 的拒绝信息也没写），只有 WebUI 候选过滤与手册引导。
- control panel 侧的 grant 交集（R10）、daemon URL 设备分配写 `settings.runtimes[exec-<id>]`（R-E3）未做，所以 `AgentRuntimeTarget::Executor` 现在只能靠手工写 settings 触发；`ExecutorResolver` 的真实路由未在 daemon 上跑过。
- `RuntimeSelection::Skill` 只是枚举值，R-U1 的第四级（按带自装命令的 skill 落点选 runtime）未实现。
- `SessionSummary.runtime_id`、sessions 列表按 runtime 过滤、Workspace / skill 安装的 `usage` 记账未做。
- 旧的 `state/runtimes/*.json`（version 3）读取失败：内建记录由宿主重写，非内建的 `remote_ssh` 登记要重新 `register`（开发态，不做迁移）。

## 9. 现状核对（2026-11-09）

P0 / P1 各项的状态取自 §8.1，这次只抽查了入口是否还在（`canonical_runtime`、`freeze_runtime` 的分叉、`runtime_exec / runtime_put`、`cmd_runtime_run`、`runtimes_view`，`RuntimeKind` 已无引用）。各表里标 ⚠️ / ❌ 的项和 §7.1 的“已核”都对着当天的代码看过。

### 9.1 §8 之后变了什么

| 来源 | 对本文的影响 |
| --- | --- |
| `c88404aaf`（11-05） | §8 的 P0 / P1 提交入库 |
| 扩展包 TODO 一、二期（11-06） | R16 / R-S1 交付，形态是 node_daemon 的 reconciler；R-S3 的 `EXTENDED_TOOLS`；多了 `runtimes.ext_tools`、`runtimes.ext_tools_check` 两个接口 |
| skills TODO 实施（11-09，`174dc06f7`，websdk `1052876`） | R4（`skill_runtime`）、R13（`writes` / `volatile`）、R-M3（`skills::check_commands`、`buckyos skill check`）、E4 里 skill 命令的记账、R-M8 的安装确认文案。skill 的 `tools/<name>/` 能经 `runtime_put` 放到节点。安装记录有了 `stale`，R6 只差触发 |
| owner 对 §7.1 的答复（11-09） | 23–30 全部裁决并回写。裁决 23 拿掉 E3；裁决 28 取消 R3、R4、R6、R13，新增 R17；裁决 30 新增 R18 |
| owner 对 §7.2 的答复（11-09，第二轮） | 去掉按 Agent 的 grant 层（R10 取消，新增 R19）；同一台 runtime 同时只有一个 Agent 启用；确认删除 skill 的 cmd 安装（R17）；Linux 节点照常支持。合起来的模型在 §10 |
| Agent UI 升级 TODO 的裁决（11-09，未实施） | D3：停用默认 runtime 时保留 `default_runtime_id`。D4：设备目录与分配以本文为准，Agent 首页一侧由那边的 RT1–RT4 做（§9.2） |

### 9.2 与 Agent UI 升级 TODO 的分工

这一小节是那份文档 DX4 要求的回写。

- **R-E6 / E6 移交。** Agent 首页的 runtime 区块按规划改为“瘦列表（Provider / Owner 默认 / 其他已分配三类）+ `#/runtime/<id>` 详情页 + 折叠的历史（retired）”，由 UI TODO 的 RT1、RT2、RT4 实施，本文不再单列 E6。本文给它提供数据：`usage`、`lifecycle`（已有）、`runtimes.impact`（R7）、`SessionSummary.runtime_id`（E4）。
- **Agent 视角的“删除”是 `runtimes.remove`**（UI TODO RT1）：从这个 Agent 的 settings 里移除条目，记录转 `retired`，设备还在用户级目录里，可以再分配；拒绝私有及其别名。R-D1 / R-D3 的影响清单与确认同样适用于它。`unregister` 只对手工登记的记录有效（内建的会被拒绝），也就是只剩开发用途。
- **E5 的目录页是 Agent 首页“添加 Runtime”的落点**（UI TODO RT3）：按钮深链到 `${desktop_url}/?settings=<page>`。E5 要给目录页定一个稳定的页面 id，并在落地时把 RT3 的临时目标（Settings → Cluster）换掉。RT4 的删除入口等 E5 可用后才放出，否则删掉了没有界面能加回。
- **D3 影响 R-C6 / R-D1。** 停用 owner 默认 runtime 时保留 `default_runtime_id`，停用期间实际默认落到私有，重新启用后恢复（UI TODO RT1；现在的 `set_runtime_settings` 会清掉）。R-D1 影响清单里的“它是否是 owner 默认”因此只是提示，不拦。
- **两单都会改的文件**：`opendan/src/home.rs`（`set_runtime_settings`）、`state/runtimes.rs`（`ensure_builtin`）、`control_panel/agent_runtimes.rs`（`apply_update`）、`opendan/web/src/pages/Home.tsx`。RT1 先做，本文的 R19、E2 在它改过的 `apply_update` 上继续。
- 裁决 24、26、27 已写回 UI TODO 的 RT2：详情页的 grant 只读；到期的条目留在当前列表里，不进历史；停用按影响清单决定要不要确认。§7.2 推定 2 之后，详情页上的 ComputerUse 与声明基线也改为只读，`runtimes.settings_set` 的参数只剩启停与默认（R19），已写回 RT1、RT2。

### 9.3 剩余任务与建议顺序

按 §7.1、§7.2 两轮答复重排（2026-11-09）。模型见 §10。

**先做：两项删除，同一次 schema 升版（v5）。** 后面的入口任务都建在删完之后的 settings 和记录上。

| 任务 | 要做的 | 落点 |
| --- | --- | --- |
| R17 | 拆 runtime 侧为 skill 自带命令做的东西，与 skills TODO 的 S24 一起做 | `api.rs`、`protocol/runtime.rs`、`state/runtimes.rs`、`runtime/mod.rs` |
| R19 | 去掉按 Agent 的授权层：settings 收成 `{enabled, target}`，ComputerUse 与基线挪到 runtime 上，记录删 `install` / `workspace`。在 UI TODO RT1 改过的 `apply_update`、`set_runtime_settings` 上做 | 见 §6 |

**第一批：互相独立。**

| 任务 | 要做的 | 落点 |
| --- | --- | --- |
| R7 | `runtimes.impact {runtime_id}`：活跃 Session、pending 且点名它的 Todo、Workspace、是否 owner 默认；停用 / 移除的确认规则按裁决 27，CLI `--yes`。界面接入在 UI TODO RT2 / RT4 | `opendan/src/home.rs`、`service.rs` |
| R8 + R14 | 停用、不可用、到期时标记进行中的 Session 并通知 owner | `opendan/src/loader.rs`、`notify` |
| R12 的尾巴 | Workspace 的派生规则（不是只读、没有期限）与 R-U10 的拒绝信息 | `state/workspaces.rs` |
| E4 的尾巴 | `SessionSummary.runtime_id`、sessions 按 runtime 过滤、Workspace 记账 | `protocol/summary.rs`、`state/workspaces.rs` |
| E1 | `agent.runtime_catalog`（节点 + daemon 设备，`assigned_to` 反查）；`node.owner_agents_set`（有策略的节点，保留 `run_as`） | `control_panel/agent_runtimes.rs`、`agent_mgr.rs` |
| R18 | Agent 删除确认框列出留有目录的 runtime 与路径 | Desktop `AgentDetailPage.tsx` |
| R11 的 DV | 手工把 `exec-<id>` 写进 settings，在真实 daemon 上验一次 form B 的投影、绑定和执行。E2 建在它上面 | — |

**第二批：依赖前面的。**

| 任务 | 依赖 | 备注 |
| --- | --- | --- |
| E2 daemon 设备两步化 | E1、R19、R11 的 DV | 配置签发时定死；同时只有一个 Agent 启用，检查在 `agent.update`；`principals` 按分配重算 |
| E5 Desktop 目录页、E7 CLI | E1、E2 | R9、R15（列表分组与列对齐）并入；分配只选 Agent；E5 给目录页定页面 id，交给 UI TODO RT3 |
| R-E9 用户级 revoke 的影响清单 | R7、E1 | 只涉及正在启用它的那个 Agent |

**不做或不再属于本文**：E3（裁决 23，本轮不做）；R3、R4、R6、R13（裁决 28 取消）；R10 与登记预设（§7.2 推定 2 取消）；E6（移交 UI TODO）；R16（扩展包 TODO 已交付）；W7（已取消）。

### 9.4 验证状态

- 2026-11-09：buckyos 与 buckyos-websdk 的工作树干净；`cargo check -p libopendan -p opendan -p control_panel -p node_executor --tests` 通过。全量测试这次没有重跑。
- §8.2 列的未验证项没有新的记录：原生运行的 private、host 上的 `agent-session run`、form B executor 的路由都没有真实 Zone 的 DV；`node_executor --test remote_node_runtime` 需要 root，仍未实测。11-08 有过 Windows / macOS 的 runtime 修复提交（`bfd98bc49`、`4d258a09a`），本文没有对应的验证记录。
- skills 一侧新增的 runtime 路径（把命令装到非私有 runtime、`skill_runtime` 这一级选择）只有单测和集成测试，没有在真实节点上跑过（skills TODO §0）。

## 10. 简化后的模型（2026-11-09，两轮答复之后）

§7.1 和 §7.2 的答复合起来给了三条原则。本节写它们叠在一起之后的样子；§0.6、§1、§4 里与本节冲突的写法以本节为准，各表的状态列已经按本节更新。

### 10.1 三条原则

1. **skill 是纯文本**（裁决 28、§7.2 推定 4）。安装就是确定性的复制，不带命令，不跑脚本。带命令的东西是扩展包；skill 包依赖扩展包，两者各发各的 pikg。skill 与 runtime 只剩一处关系：冻结时按 Session runtime 的 `baseline / platforms` 过滤手册（M5）。
2. **agent runtime 是一个 sandbox，自己声明能力和权限**（§7.2 推定 2）。要不同的权限就建不同的 runtime，不再按 Agent 加一层 grant。
3. **一台 runtime 同时只给一个 Agent 用**（§7.2 推定 1）。今天是一对一；规划是可以分配给多个 Agent，但同时只有一个启用。

### 10.2 三个对象，各管一件事

| 对象 | 存什么 | 在哪里改 |
| --- | --- | --- |
| runtime（用户级目录里的一条） | 怎么连、是什么机器、允许做什么：`ops`、`roots`、ComputerUse 档位、期限、声明基线 | Desktop 的 Settings → Agent Runtimes |
| 分配 | `AgentSettings.runtimes[id] = {enabled, target}`，加 `default_runtime_id` | 目录页（分配、换人）；Agent 首页（启用 / 停用 / 默认 / 移除） |
| 记录（Agent State 里的投影） | 上面两者的只读投影，加 `availability`、`usage`、`lifecycle` | 不改 |

- **runtime 的配置就是它来源上的那一份。** 节点是 `owner_agents` 策略（`enabled / workdir_roots / computer / run_as`，加一个 `baseline`）；设备是签发时的 profile（roots、ops、期限，加一个 `baseline`）。设备的配置签发时定死，要别的配置就再签一台。Linux 节点的缺省配置是激活时选的 OS 账号：以它的身份执行。
- **分配里没有权限。** `AgentRuntimeSettings` 从 `{enabled, computer, target, baseline, grant}` 收成 `{enabled, target}`。ComputerUse 档位和声明基线是 runtime 的，不按 Agent 设。
- **记录里的 `grant` 只有一个来源。** 它只剩 `ops / roots / computer / expires_at_ms`，全部来自对 runtime 的探测（`login`、`describe`、策略、URL 期限），含义是“这台 runtime 允许什么”。结构名不改，省一轮机械替换。
- **两个开关改成派生。** 只读 = `ops` 里没有写操作（现有）。能建 Workspace = runtime 可用、不是只读、没有期限。
- **独占。** 设备 runtime 同时只有一个 Agent `enabled`，executor profile 的 `principals` 就是那一个。检查放在 control panel 的 `agent.update`，只有它看得到 owner 的全部 Agent。opendan 一侧的停用、移除照常在本地做；启用转给 control panel（它已有 control-panel 的客户端，权限口径实施时核）。换人 = 先停用原来的（按裁决 27 的确认规则），再启用新的。节点的 `owner_agents` 策略是模板，每个 Agent 在节点上各有自己的 profile，按各自一个 sandbox 处理，不做跨 Agent 的独占（§7.3 已确认）。
- **选 runtime 只有三级。** 显式 `--runtime` > 继承 > 私有。

### 10.3 因此删掉的

| 删掉的 | 依据 | 任务 |
| --- | --- | --- |
| skill 自带命令的全套：`tools/<name>/bin`、`install.toml`、`install-cmd`、按 runtime 的安装状态与 `stale`、`volatile`、ExtTools 拒装、Session PATH 的 skill `bin` 层、`skill_runtime` 这一级、`usage.skill_commands`、plan 全表的 skill 命令列 | 裁决 28、§7.2 推定 4、5 | R17，skills TODO 的 S24 |
| 节点上的 `tools/<skill>/bin` 与 PATH 前置；runtime 事件触发的 skill 重核验；持久卷核验 | 同上 | R3、R6、R13 取消 |
| 按 Agent 的授权：settings 的 `grant`（`AgentRuntimeGrant`、`InstallGrant`）、按 Agent 的 `computer` 与 `baseline`；记录的 `grant.install`、`grant.workspace` | §7.2 推定 2、待定 2 | R19 |
| control panel 算 grant 交集；登记预设；“owner 给 host 设一个有期限的 grant” | §7.2 推定 2 | R10、R-K6、R-C7 取消，R-K4 收窄 |
| 目录页的 grant 对话框、CLI 的 grant 参数、Agent 首页上 ComputerUse 与基线的编辑、创建向导里按 Agent 的 ComputerUse | §7.2 推定 2 | 随 E5、E7、R19 与 UI TODO RT2 |
| 修改已有设备 profile 的入口（§9 第一版说要补的） | 设备的配置签发时定死 | 不做；`principals` 的重算是 control panel 内部的事 |
| ssh 上提与 `add-ssh` | 裁决 23 | E3 本轮不做 |

留下来的任务清单在 §9.3。
