# OpenDAN AgentRootFS 与配置文件改进计划

> **本文性质**：改进计划，不是最终 spec。落盘节奏跟 beta2.2 / beta2.3 的 breaking change 一起走。
>
> **上游心智**：[OpenDAN Agent 配置开发指导](./OpenDAN%20Agent配置开发指导.md) ——
> Agent Runtime（Gateway）/ Session / Behavior 三层心智，是本计划要让"配置文件长成什么样"对齐的总锚。
>
> **下游事实**：当前实现在 [agent_config.rs](../../src/frame/opendan/src/agent_config.rs) /
> [behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs) /
> [agent_session.rs](../../src/frame/opendan/src/agent_session.rs) /
> [agent.rs](../../src/frame/opendan/src/agent.rs) ——本文用"现状 vs 改进"的格式给出迁移路径。

---

## 0. 这是什么、不是什么

| | |
|---|---|
| ✅ 本文给出 | AgentRootFS 目录契约（增量调整）、`agent.toml` 新 schema、`behaviors/<name>` 新 schema、配置 DSL 的最小动词集、迁移路径 |
| ❌ 本文不给 | 具体解析器实现、Rust 数据结构最终签名、脚本引擎选型、UI / msg-center 接口变更 |

---

## 1. 立论：为什么现在动配置格式

三个驱动：

1. **指导文档新立心智** —— Gateway / Session / Behavior 三层已经定下来，但当前 `agent.toml` 太薄
   （5 个字段），Gateway 行为（事件接入、dispatcher）与 Session 类定义都没有显式落盘，全部硬编码在
   `AIAgent::resolve_ui_session` 等地方。Session 类多一种（比如邮件、群、tunnel-A），都得动 Rust。
2. **Action 集合已经在 beta2.2 重新固化为 v2 七元集**（见 [Agent Actions](./Agent%20Actions.md)），
   Behavior 配置应当围绕"事件 → action"组织，而不是 v1 的"prompt + tool_whitelist"。
3. **异常路径需要显式落盘** —— `ContextLimitReached` / provider 失败 / suspend-resume 当前散落在 Rust
   逻辑里，指导文档 §3.3 明确"响应方式跟当前在干什么强相关，写在 Behavior 里"，需要 schema 支撑。

判据（贯穿全文）：

- **配置层级 ∝ 改动频率**。framework 不许 leak 到 behavior，business 不许 leak 到 agent.toml。
- **显式大于隐式**。"何时停 / 切到哪 / 上下文满了怎么办"必须在配置里看得到。
- **配置即架构**。读 `agent.toml` + 数 `behaviors/` 目录，应当能复述 Agent 的对外形态与全部分支。

---

## 2. 三层落盘对照（决策表）

| 心智层 | 改动频率 | 配置载体 | 物理路径 |
|---|---|---|---|
| Identity + Runtime | 🟦 极少 | `[identity]` `[runtime]` `[[channel]]` | `<agent_root>/agent.toml` |
| Dispatcher（事件 → Session） | 🟦 极少 | `[dispatch]` `[[dispatch.rule]]` | `<agent_root>/agent.toml` |
| Session 类 | 🟨 偶尔 | `[session.<class>]` | `<agent_root>/agent.toml` |
| Behavior（业务） | 🟥 经常 | 每 behavior 一份 | `<agent_root>/behaviors/<name>.toml` 或 `<name>/behavior.toml` |
| Tool plan | 🟥 经常 | 策略文件 | `<agent_root>/tool_plans/<plan>.toml` |
| Skills / Tools 声明 | 🟥 经常 | 见 §6 4 层 Bin | `<agent_root>/tools/` / `<agent_root>/skills/` |

**改谁查这一张表，不要靠记忆。**

---

## 3. AgentRootFS 目录布局（增量调整）

```text
<agent_root>/
  agent.toml                          # ⬅ 改 schema（§4）
  role.md                             # 自我介绍片段，按约定文件名由 behavior 提示词模板 include 引入
  self.md                             # 内部能力 / 边界声明，同上
  .meta/
    rootfs_sync.json                  # 启动期 package → root 同步 manifest（sha256，保护本地修改）
  users/
    <user_id>.md                      # 按 from_did 选择的系统提示词片段
    group_<gid>.md
  memory/                             # AgentMemory 初始化目录
  notepads/<notepad_name>/
  skills/<category>/<skill_dir>/
  tools/                              # Agent Bin 层（见 §6）
  tool_plans/<plan_name>.toml         # behavior 引用（见 §6.3）
  i18n/<language>.toml                # runtime-facing 短文案，如 session one-line status
  behaviors/
    <name>.toml                       # ⬅ 扁平形态（§5）
    <name>/                           # ⬅ 新增结构化形态（§5.5）
      behavior.toml
      system.md                       # [prompt].on_init 模板
      input_msg.md                    # [prompt].on_input_msg 模板（可缺）
      input_event.md                  # [prompt].on_input_event 模板（可缺）
  workspace/<workspace_id>/
  sessions/<session_id>/
  archive/
    skills/
    sessions/<session_id>/
    workspace/<workspace_id>/
    worklog.db
```

**仍然有效的硬约束**（与当前实现一致，不变）：

- AgentRootFS 内禁止任何二进制可执行文件（ELF / Mach-O / PE / `.so` / `.dylib` / `.dll`），通过 4 层
  Bin 的执行视图来承载二进制。
- `sessions/<id>/` 内不放 `./bin/`，执行视图渲染到 App Instance Volume 下的
  `<instance_volume>/tools/<agent_id>/<session_id>/`。Docker 正式环境中通常是
  `/opt/buckyos/instance/tools/<agent_id>/<session_id>/`。
- 路径解析单一来源 [`paths.rs`](../../src/frame/opendan/src/paths.rs)，禁止"候选 key 列表 + 祖先扫描"。

**新增约束**：

- `behaviors/<name>/` 与 `behaviors/<name>.toml` **互斥**——同名同时存在视为配置错误，启动期拒绝加载。
- `behaviors/<name>/behavior.toml` 内的 `[prompt].on_init` / `on_input_msg` / `on_input_event` 引用
  的文件路径**必须解析到同目录之下**——禁止跨目录引用。要共享 prompt 片段（比如 `role.md` /
  `self.md`）只能通过模板引擎的 `include` 机制，由 prompt compiler 处理，见
  [Agent Prompt Compiler](./Agent%20Prompt%20Compiler.md)。

---

## 4. `agent.toml` 新 schema —— Gateway + Session classes

### 4.1 现状（5 字段，[agent_config.rs](../../src/frame/opendan/src/agent_config.rs)）

```toml
agent_did              = ""
display_name           = ""
default_ui_behavior    = ""
default_work_behavior  = ""
subscribe_events       = []
cancel_reason          = ""
preserve_attachment_tag_in_egress = false
```

问题：Gateway 行为不可表达、session 类不可表达、订阅是扁平数组不挂在具体 session 类上、dispatcher
完全隐式（在 `AIAgent::dispatch_inbound` 里硬编码）。

### 4.2 改进 schema 草案

```toml
# ─── Identity ────────────────────────────────────────────────
[identity]
agent_did    = ""                       # 空 ⇒ 启动期从 system_config 拉的 AgentDocument 回填
display_name = ""                       # 空 ⇒ 从目录名推断
# role.md / self.md 不在这里声明——它们是约定俗成的文件名，由具体 behavior 的
# system prompt 模板通过 prompt compiler 的 `{{ include "role.md" }}` 显式引入。
# agent.toml 不掺合"哪个 behavior 用哪段身份描述"。

# ─── Runtime ─────────────────────────────────────────────────
[runtime]
cancel_reason = "user requested cancel" # Observation::Cancelled 文案兜底
language = "en"                         # 选择 i18n/<language>.toml，空值等同 en
# 支持 zh / zh-TW / en / es / fr / de / ko / ja / ru；zh-CN、en-US 等 alias 会归一化。
preserve_attachment_tag_in_egress = false
filesystem_policy = "workspace"         # workspace / unrestricted

# ─── Channels：Gateway 监听的事件源 ──────────────────────────
[[channel]]
type = "msg_center"
# msg_center 上来的所有 inbound Msg 都进 dispatcher
[[channel]]
type    = "kevent"
filters = ["task_mgr/**", "kvdoc/**"]    # 订阅前缀

# ─── Dispatcher：事件类型 → Session class ───────────────────
# v0 故意做"窄"：纯事件类型过滤器到 session class 的固定映射。
# 没有 when 表达式、没有 session_id 模板，没有任何"基于事件字段计算"的能力。
# 复杂分发等积累完真实需求再考虑升级（见 §10 开放问题 #X）。
[dispatch]
default_class = "ui"                    # 没有任何 rule 命中时的兜底 session 类

[[dispatch.rule]]
on            = "msg.chat"              # 单事件类型，允许尾部通配 `msg.*`
session_class = "ui"

[[dispatch.rule]]
on            = "msg.group"
session_class = "group"

[[dispatch.rule]]
on            = "task_mgr.*"            # task_mgr 下所有事件
session_class = "work"

# ─── Session 类 ──────────────────────────────────────────────
[session.ui]
loop_mode           = "agent"           # 普通 Agent Loop，default behavior 固定
default_behavior    = "ui_default"
subscribe_events    = ["msg.incoming"]  # 这个 class 默认订阅的 kevent 通配
session_id_strategy = "per_peer"        # 见下方枚举
switch_mode         = "normal"          # behavior 切换语义；agent loop 下不会触发，写出来仅为对称
process_stack_limit = 4
keep_alive          = true              # 永远算 active，重启后 restore
inject_background_environment = true    # 每轮 user input 前注入 <background_environment>

[session.group]
loop_mode           = "agent"
default_behavior    = "group_default"
session_id_strategy = "per_group"
switch_mode         = "normal"
keep_alive          = true

[session.work]
loop_mode           = "behavior"        # Behavior Loop，状态机
default_behavior    = "work_default"
subscribe_events    = ["task_mgr.*", "kvdoc.*"]
session_id_strategy = "per_event_session"
switch_mode         = "normal"          # "normal" | "fork" | "independent"，整个 class 统一
process_stack_limit = 8
keep_alive          = false             # status != Ended 才算 active
inject_background_environment = false   # worksession 通常只用 prompt 模板里的 session/workspace 变量
report_delivery     = "final_only"      # "final_only" | "top_level" | "all"
```

**`session_id_strategy` 枚举**（v0 固化、不可扩展）：

| 策略 | session_id 形态 | 适用 |
|---|---|---|
| `per_peer` | `<class>-<event.from_did>` | 一个对话方一个 session（UI 类典型） |
| `per_group` | `<class>-<event.group_id>` | 一个群一个 session |
| `per_event_session` | `<event.session_id>` | event 自带 session_id（task_mgr 完成事件、worksession 路由） |
| `singleton` | `<class>` | 整个 class 全局只有一个 session（系统级、调度类） |

策略选择由 session class 决定，**不由 dispatcher rule 决定**——这是 §1 立论里"配置层级 ∝ 改动频率"
的具体落地：dispatcher 想加新 rule 不应当被迫想清楚 session_id 怎么取。

未命中策略所需字段（如 `per_peer` 收到一条没有 `from_did` 的 event）⇒ drop + warn。

### 4.3 改进点与现状映射

| 现状字段 | 新位置 | 说明 |
|---|---|---|
| `agent_did` / `display_name` | `[identity]` | 同语义，分组到 identity 段 |
| `default_ui_behavior` | `[session.ui].default_behavior` | session 类拥有自己的 default |
| `default_work_behavior` | `[session.work].default_behavior` | 同上 |
| `subscribe_events` | `[session.<class>].subscribe_events` | 订阅挂在 session 类上，而不是 Agent 全局 |
| `cancel_reason` | `[runtime].cancel_reason` | 同语义 |
| `preserve_attachment_tag_in_egress` | `[runtime]` | 同语义 |
| — | `[runtime].filesystem_policy` | **新增**，控制 read / attachment 的本地文件路径策略 |
| — | `[[channel]]` | **新增**，Gateway 接入显式化 |
| — | `[dispatch]` | **新增**，dispatcher 显式化（当前硬编码在 `AIAgent::dispatch_inbound`） |
| — | `[session.<class>].loop_mode` | **新增**，对齐指导文档 "UI Session 是 loop_mode=agent 的特例" |
| — | `[session.<class>].session_id_strategy` | **新增**，固化 session_id 派生策略 |
| — | `[session.<class>].switch_mode` | **新增**（从 behavior 上提），整个 class 统一 |
| — | `[session.<class>].keep_alive` | **新增**，固化"UI 永远活、Work 看状态"的差异 |
| — | `[session.<class>].inject_background_environment` | **新增**，控制是否在每轮输入前动态注入 `<background_environment>` |
| — | `[session.<class>].report_delivery` | **新增**，控制 WorkSession report 上行到 UI session 的默认范围 |

### 4.4 兼容性

- beta2.2 启动期：若 `agent.toml` 包含旧字段，runtime 做一次内存映射（warn + 提示运行
  `opendan migrate-config`），不再向新代码扩散。
- beta3.0：旧字段不再识别，启动期拒绝。
- `agent_did` 留 empty 由 runtime 回填的现有契约保留不变。

---

## 5. Behavior 配置 —— 从 "prompt + tools" 到 "event + action"

### 5.1 现状（[behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs)）

```toml
name = "explorer"
objective = "..."
system_prompt_template = "..."
tool_whitelist = ["exec_bash", "..."]
approval_required = []
tool_plan = "minimal_safe"
mode = "behavior"
parser = "xml"
renderer = "xml"
parser_strict = false
[renderer_cfg]   ...
output = { ... }
max_rounds = 16
max_consecutive_errors = 3
switch_mode = "normal"
[model]   ...
[budget]  ...
```

问题：**事件不见了，action 不见了，next 不见了**。Behavior 看起来像一份 LLM 调用模板，而不是
指导文档说的"对一组事件的响应表 + 状态机的一个节点"。

### 5.2 改进 schema 草案

Behavior 配置的心智模型很简单：**走到这里，session 已经决定要推理了**，behavior 只关心三件事——
**渲染什么提示词、能用什么能力、异常路径有没有打开**。状态机的边、事件过滤、switch_mode 都不在
behavior 里：

- "什么事件触发推理" 是 session 类的事（见 §4.2 `[session.<class>].subscribe_events`）
- "下一个 behavior 是哪个" 由 LLM 在 `<next_behavior>` 输出端决定，runtime 不预先约束
- "切的时候用 normal / fork / independent" 是 session 类的事（见 §4.2 `[session.<class>].switch_mode`）

```toml
# ─── meta ───────────────────────────────────────────────────
[meta]
name      = "explorer"
objective = "explore unknown territory"

# ─── prompt 渲染：三个时机，三段模板 ───────────────────────
# 模板引擎（Agent Prompt Compiler）在每个时机都能访问当前 session / behavior / 环境状态，
# 具体可见变量见 Render_Prompt_Template_Variables.md。
[prompt]
# 进入这个 behavior 的瞬间，渲染 system 段
on_init        = "system.md"

# 收到一条 user/peer message，即将拼进本轮 input 时渲染
# 缺省 ⇒ 走 runtime 内建的最小消息模板（仅原文 + 发送者）
on_input_msg   = "input_msg.md"

# 收到一条业务 event，即将拼进本轮 input 时渲染
# 缺省 ⇒ 走 runtime 内建的最小事件模板（type + 关键字段）
on_input_event = "input_event.md"

parser = "xml"                          # LLM 输出 parse 方式；renderer 等细节全部走默认值

# ─── 能力声明（占位，§5.3 详述）────────────────────────────
# behavior 在这次推理中能用到的能力，三类合在一起：
#   - v2 内建 Action（exec_bash / write_file / ... / subscribe_event）
#   - skills bundle（<agent_root>/skills/ 下加载的成套能力）
#   - 传统 function-call tool（ToolManager 注册的命名函数）
# 详细 schema 待 §5.3 收敛；当前 5.2 草案先占位，不写细节字段。
[capabilities]
# TODO

# ─── Budget / Model ─────────────────────────────────────────
[budget]
max_rounds              = 16
max_consecutive_errors  = 3
max_total_tokens        = 200_000

[model]
preferred   = "claude-opus-4-7"


# ─── 旁路 LLM 逻辑（异常路径的开关）─────────────────────────
# 这一段的作用是"一眼看出来哪些旁路打开了"。v0 不追求灵活性，每个 on_xxx 就是一个固定模式：
# 写出来 ⇒ 旁路启用，按固定策略执行；删掉 ⇒ 旁路关闭，命中时按 runtime 默认（通常是结束 process）。
# 想调策略细节先改 src/，等真有第二种合理策略再扩 schema。
[on_context_limit_reached]
mode = "compress_then_continue"         # 唯一支持的模式；填上即启用

[on_llm_message_compress]
mode = "context_window_ratio"           # 一轮完成后按 context window 使用率触发
trigger_ratio = 0.80
target_ratio = 0.50
hard_limit_ratio = 0.95
min_turns_between_compress = 2
preserve_cache_stability = true

[on_provider_failed]
mode   = "fallback_behavior"
target = "explorer_safe_mode"

[on_interrupt_graceful]
mode = "cancel_pending_tools_then_continue"
```

### 5.3 `[capabilities]` 段：待收敛的占位

这是当前最不确定的一段，先讲清楚边界：

- **v2 内建 Action**（[Agent Actions](./Agent%20Actions.md) §1）是**提示词耦合的固化集合**——
  prompt 模板里见过哪些标签 LLM 才会输出。这部分天然要在 behavior 层声明（不同 behavior 可能屏蔽
  不同 action）。
- **Skills**（`<agent_root>/skills/` 下加载的成套能力）是 self-improvable 的业务能力包，每个 bundle
  自带 prompt 片段、tool 注册、可能还有自己的 sub-behavior。behavior 要做的是"声明本次推理把哪些
  bundle 挂进来"。
- **传统 function-call tool**（ToolManager 注册的命名函数，走 provider native tool_calls 通道）
  是 v2 Action 的并存通道——`exec_bash` 之外的工具调用要么走 shim 进 bash，要么走这条 native tool
  通道。behavior 要声明白名单。

三类都需要一个"上层引用 + 下层细节定义"的解耦：

| 类别 | 上层引用 | 下层细节定义 |
|---|---|---|
| v2 内建 Action | `[capabilities].actions = [...]`（待定） | runtime 内建，schema 在 Agent Actions.md |
| Skill bundle | `[capabilities].skills = [...]`（待定） | `<agent_root>/skills/<category>/<bundle>/` |
| Function tool | `[capabilities].tools = [...]` 或预设名 | ToolManager 注册 + tool_plan |

**5.2 草案有意只写 `[capabilities] # TODO`**——把三类如何统一表达留作单独的设计点。现状 `BehaviorCfg`
里的 `tool_whitelist` / `tool_plan` 字段在新 schema 里都进 `[capabilities]`，但具体 key 名要等
skill / function tool 配置一起定。Open question #X 跟踪。

### 5.4 改进点与现状映射

| 现状字段 | 新位置 | 备注 |
|---|---|---|
| `name` / `objective` | `[meta]` | — |
| `mode` | `[session.<class>].loop_mode` | **上提到 session 类**：agent loop vs behavior loop 是 session 决定的 |
| `system_prompt_template` | `[prompt].on_init` | 渲染时机命名化 |
| `parser` | `[prompt].parser` | — |
| `renderer` / `parser_strict` / `renderer_cfg` | 不再配置 | 走默认值，要调直接改 src |
| `output` | `[prompt].output` | 没变（不常用，按需保留） |
| `tool_whitelist` / `approval_required` / `tool_plan` | `[capabilities]`（待收敛） | 见 §5.3 |
| `max_rounds` / `max_consecutive_errors` / `budget` | `[budget]` | — |
| `switch_mode` | `[session.<class>].switch_mode` | **上提到 session 类** |
| `model` | `[model]` | — |
| — | `[prompt].on_input_msg` / `on_input_event` | **新增**：消息/事件渲染时机的模板槽 |
| — | `[on_xxx]` 旁路开关 | **新增**：异常路径从 Rust 默认逻辑里提出来做成可见开关 |

### 5.5 文件物理形态

**扁平**（适合简单 behavior，单文件 < 200 行）：
```
behaviors/explorer.toml
```

**结构化**（适合 prompt 模板需要拆三段、内容较长时）：
```
behaviors/explorer/
  behavior.toml
  system.md             # [prompt].on_init 指向
  input_msg.md          # [prompt].on_input_msg 指向（可缺）
  input_event.md        # [prompt].on_input_event 指向（可缺）
  notes.md              # 自由文档（不被 runtime 读，给开发者看）
```

加载顺序：先看 `behaviors/<name>/behavior.toml`，没有再看 `behaviors/<name>.toml`。同时存在 → error。

### 5.6 `[on_xxx]` 与 LLM 输出 `<actions>` 的层级澄清

| | 谁决定 | 出现时机 | 例子 |
|---|---|---|---|
| `[on_xxx]`（**配置**） | 配置开发者 | LLMContext 抛出异常事件 | "上下文满了 ⇒ 压缩后继续" |
| `<action>`（**LLM 输出**） | LLM 在一轮推理内 | LLM 决定本轮要写一个文件 | "本轮输出 `<write_file>`" |

`[on_xxx]` 是**旁路开关**（异常路径打不打开、按哪个固定策略走），`<actions>` 是 LLM 在正常推理内
的**输出**（这一轮想做什么）。两者属于不同维度，不冲突也不重叠。

**正常路径下"什么事件触发推理"不在 behavior 里**——那是 session 类 `subscribe_events` 决定的（§4.2）。
behavior 永远是"已经决定推理，正在准备这一轮"的状态。

---

## 6. 4 层 Bin / Tool Plan（保留现有契约，简述）

完整描述见原 §3。本节只复述硬约束，方便单文件阅读。

| 层 | 物理路径 | 持久化 |
|---|---|---|
| System Bin | `<buckyos_root>/tools/store/` | ExtTool 共享只读卷 |
| Runtime Bin | `<instance_volume>/tools/bin/` | App Instance Volume |
| Agent Bin | `<agent_root>/tools/` | AgentRootFS 持久化 |
| Session Bin 声明 | `<agent_root>/sessions/<sid>/tools/` | AgentRootFS 持久化 |
| Session Bin 执行视图 | `<instance_volume>/tools/<agent_id>/<sid>/` | App Instance Volume |

Docker 正式环境中:

- `<buckyos_root>` 固定为 `/opt/buckyos`。
- `<instance_volume>` 来自 `BUCKYOS_INSTANCE_VOLUME`,当前 worker 容器内为 `/opt/buckyos/instance`。
- `/opt/buckyos/tools` 是 `buckyos-exttool` 共享只读卷。OpenDAN 只能读取其中的 `store/`、预热 cache 等内容,不能在该目录下创建 session-bin。
- Runtime Bin 和 Session Bin 都必须落在 `/opt/buckyos/instance/tools/...`,避免写入只读 ExtTool 卷。

### 6.1 PATH overlay
```
PATH = SessionExecBin : AgentBin : RuntimeBin : SystemBin : <inherited>
```

### 6.2 Session `./tools/` 硬约束
- 只放文本（脚本源码、`tool.toml`、prompts、schema），禁止二进制。
- 单文件建议 ≤ 64 KB，整目录文件数建议 ≤ 几百（hot path，每次 `exec_bash` 起手要做 mtime 同步）。

### 6.3 Tool Plan
位置：`<agent_root>/tool_plans/<plan>.toml`；schema 与渲染时机不变。

**改进点（与 behavior schema 联动）**：
- `tool_plan` 字段从 behavior 顶层移到 `[actions].tool_plan`，强调 "tool plan 是 action 配套，不是
  behavior 独立维度"。

---

## 7. DSL 设计 —— v0 故意不开表达式

### 7.1 原则（v0）

1. **完全没有表达式**。整份配置里**所有**字符串字段要么是事件类型 / 文件路径，要么是固定枚举常量。
   不存在 `when = "msg.kind == 'chat'"` 这种写法、也不存在 `session_id = "ui-${msg.from_did}"` 这种
   模板插值。一旦哪天觉得不够表达，**直接整体跳到完整的 process-chain DSL**，而不是"小步演进"——
   小步加表达式的代价是产生第二种半成品语义层，跟将来的完整 DSL 不兼容。
2. **配置字段全是枚举或字面量**。整份 schema 里 runtime 需要"解释"的部分只有：dispatcher 的事件
   类型字符串（含尾部通配）、各种 enum、文件路径、命名引用（behavior 名 / tool plan 名 / skill 名）。
   不存在条件判断、不存在算术、不存在变量替换。
3. **事件类型只支持尾部通配**。`msg.chat` 精确匹配，`msg.*` 匹配 `msg.` 任一后缀。**没有**中间通配
   （禁止 `*.completed`），没有交集 / 并集，没有否定。
4. **session_id 派生是 4 选 1 的枚举**（见 §4.2 `session_id_strategy`），不是表达式。
5. **旁路只做"开/关 + 唯一策略"**。`[on_xxx]` 每段只允许一个固定 `mode`（§7.2），不允许 `if/else` /
   多策略并列。要第二种策略 ⇒ 写第二个 `[on_xxx]` 段（但当前 schema 没给）。
6. **分支判断让 LLM 决定**。"下一个 behavior 是谁" / "走哪条业务路径" 由 LLM 在 `<next_behavior>` /
   `<report>` / 其它输出端表达——这是 prompt 工程的职责，不是配置 DSL 的职责。
7. **未来升级路径**：见 §10 开放问题 #1。**不允许小步加 `when`**——要升级就一次性引入完整的
   process-chain DSL 替换本节，确保只有一种表达层语义。

### 7.2 旁路 `mode` 清单（v0 固化）

整份配置里 v0 唯一的"枚举值字段"集中在 behavior `[on_xxx]` 旁路开关上。每个 `[on_xxx]` section
都对应一个固定的 `mode`，写出来即启用：

| `[on_xxx]` 段 | 允许的 `mode` | 含义 |
|---|---|---|
| `[on_context_limit_reached]` | `compress_then_continue` | 上下文满 ⇒ 压缩后继续 |
| `[on_llm_message_compress]` | `context_window_ratio` | 一轮完成后按 context window 使用率自动压缩；不写则不启用自动压缩 |
| `[on_provider_failed]` | `fallback_behavior`（带 `target = "<name>"`） | provider 失败 ⇒ 切到 target |
| `[on_interrupt_graceful]` | `cancel_pending_tools_then_continue` | 收到 graceful 中断 ⇒ 注入 Cancelled 后继续 |
| `[on_interrupt_discard]` | `end` | 收到 discard 中断 ⇒ 直接结束 process |

每个 `[on_xxx]` 段当前**只允许一种 mode**——这是"先做开关，不做策略灵活性"原则的具体体现。需要
第二种合理策略时再扩；不预先开口子。`[on_llm_message_compress]` 的可调参数是该固定策略的阈值：
`trigger_ratio`、`target_ratio`、`hard_limit_ratio`、`min_turns_between_compress`、
`preserve_cache_stability`，以及调试/兜底用的 `context_window_tokens`。生产环境应优先依赖
AICC `models.list` 返回的 `capabilities.max_context_tokens`。**未列出的 section 或 mode ⇒ parse error**。

其它枚举字段（`loop_mode` / `switch_mode` / `session_id_strategy` / dispatcher `on` 通配 / `mode = "agent"|"behavior"`）
分散在各自的 schema 段中，本节不重复罗列。

### 7.3 留白：把"想写代码"的诉求踢给注释

需要表达 v0 动词集合覆盖不到的逻辑时，**唯一允许的形式是 Rust 风格注释**——它对 runtime 完全
透明，只是给读配置的人看：

```toml
[on_error.context_limit_reached]
verb   = "compress_then_continue"
keep_n = 6
# NOTE: 真要做"先看 pending_inputs 多不多再决定压几轮"这种逻辑，
#       目前的去处是直接改 src/frame/opendan/src/llm_context_helper.rs::compact_history，
#       不要在这里加 when / script。等 process-chain DSL 上线再统一搬过来。
```

**这一节是故意没给"小型脚本钩子"的**。前一版本草案里有 `script = "handlers.rhai::..."` 留白，
被砍掉了——原因：留个半残钩子比不留更糟，社区会基于它写一堆"半 DSL 配置"，等真要升级到完整
process-chain DSL 时就尾大不掉。要么不要，要么一次性全要。

---

## 8. 部署 / FS / COW（保留现有契约，简述）

完整版见 git 历史。这里只列**不变事实**，方便单文件阅读：

- **数据 / 运行时分离**：AgentRootFS 在宿主机普通文件系统；AgentRuntime 在 Linux 容器内。
- **跨平台契约只落在 AgentRootFS**：目录结构、配置、session 数据平台无关；执行视图（PATH 里的 bin、
  tmux pane、临时挂载）允许纯 Linux 形态。
- **`agent_root` 来源**：`opendan` 启动 BuckyOS AppService runtime 后，用 runtime 解析出的
  `app_id` / `owner_id` 调 `get_buckyos_app_data_dir(app_id, owner_id)` 取得当前 app 数据目录。
- **Docker 中的 `agent_root`**：容器内为 `/home/<owner>/.local/share/<agent_id>`,host 侧对应
  `$BUCKYOS_ROOT/data/home/<owner>/.local/share/<agent_id>`。它是 AgentRootFS/user app data,不承载
  session-bin。
- **Docker 中的可写执行层**：`BUCKYOS_INSTANCE_VOLUME=/opt/buckyos/instance`,OpenDAN 的 Runtime Bin 和
  Session Exec Bin 都渲染到 `/opt/buckyos/instance/tools/...`。
- **Docker 中的 ExtTool 层**：`BUCKYOS_EXTTOOL_DIR=/opt/buckyos/tools`,由 `buckyos-exttool` 共享只读卷提供,
  OpenDAN 只把 `/opt/buckyos/tools/store` 作为 System Bin 读取。
- **COW 由容器内 OverlayFS 实现**：`OverlayFS(Package[RO], Data[RW])`，host 不做 overlay 生命周期。
- **`opendan` 进程不理解 Docker**，只消费 BuckyOS runtime 解析出的 app 数据目录。

确定性读取规则（不变）：

- 不允许从 cwd 向上扫祖先目录推 `agent_root`。
- 不允许"哪个目录有 `worklog.db` 就是根"这类反推。
- session workspace root：绑定 ⇒ `<agent_root>/workspace/<id>/`；未绑定 ⇒ `<agent_root>`。



---

## 10. 待定事项（Open questions）

按优先级排：

1. **何时升级到完整 process-chain DSL**：v0 dispatcher 是纯"事件类型 → session class"映射，
   behavior 不带 `[on.*]` / `[next.rule]` / `when`。这套窄到底的设计有意识地拒绝"小步加表达式"——
   见 §7.1。升级触发条件（提前定下来避免临时拍）：
   - 同一事件类型需要按 payload 字段路由到不同 session class（例如 `msg.chat` 按
     `recipient_group_id` 分发到不同 group session）；
   - 收到事件需要根据 payload 决定要不要触发推理（例如 "from blacklist 的 user_msg 直接丢"）；
   - behavior 切换需要 runtime 自动判定，不再让 LLM 在 `<next_behavior>` 里决定。

   三类需求都不出现就不引入表达式。一旦其中两类同时变现实需求，**一次性**用完整 process-chain DSL
   替换 §4.2 / §5.2 / §7 的窄设计，不在 v0 上叠加 `when`。

2. **`[capabilities]` 的最终 schema**：§5.3 留空。需要在同一个段下表达三类能力（v2 内建 Action /
   skill bundle / 传统 function-call tool），且每类都有"上层引用 + 下层细节定义"的解耦诉求。需要
   等 [Agent Skill](./Agent%20Skill.md) 与 ToolManager 的配置形态一起定下来再收敛。在那之前 §5.2
   草案里这段就只是占位。

3. **session class 内多 behavior 时的事件路由**：dispatcher 把事件送进 session，session 把事件交给
   `current_behavior`，behavior 通过 prompt 表达自己的反应——不需要"event → behavior"的二级路由。
   等真有"同一 session class 内多个 behavior 长期共存且都要被外部直接寻址"的场景再回头处理。

4. **`subscribe_event` 静态 vs 动态**：v2 Action 集合里有 `subscribe_event` / `unsubscribe_event`
   （LLM 主动注册），而 `[session.<class>].subscribe_events` 是配置静态注册。两者关系明确：静态订阅
   在 session 一启动就生效；动态订阅在 SessionMeta 里持久化、重启后从 meta 还原。**两条路径都走
   `session_event_pump`**，配置语义不冲突。

5. **`process_stack_limit` 超限语义**：当前
   [`SessionMeta.process_stack`](../../src/frame/opendan/src/session_model.rs) 无上限，新 schema 加上
   limit 后需要决定超限语义（拒绝 fork / 拒绝 independent switch / 自动 unwind）。

6. **`AgentLayout` 兼容扁平 + 结构化两种 behavior 形态**：`layout.behavior_path()` 当前签名只返回
   `.toml`，需要扩到 enum 或返回候选列表，并把"两种形态同名存在"的错误检查放在加载入口。

7. **Channel 类型的对外接口**：`type = "msg_center" | "kevent" | "http" | ...` 的清单需要定一个准入
   表，跟 [Agent Session 的事件订阅](./Agent%20Session的事件订阅.md) §2 三种模式对齐。

8. **prompt 三段模板的默认值**：`[prompt].on_input_msg` / `on_input_event` 缺省走 runtime 内建的
   最小模板——这两个内建模板的具体形态需要跟 [Render_Prompt_Template_Variables](./Render_Prompt_Template_Variables.md)
   一起定，并明确"自定义模板可以访问哪些上下文变量"的清单。

---

## 一句话总结

> **`agent.toml` 承载 Gateway + Session 类骨架（事件类型 → session class 的固定映射、loop / switch /
> session_id 派生策略全归 session 类）；`behaviors/<name>/` 承载"已经决定要推理之后，渲染哪段提示词、
> 能用哪些能力、有没有打开异常旁路"——三段 prompt 模板（on_init / on_input_msg / on_input_event）+
> `[capabilities]` + `[on_xxx]` 开关。v0 整份配置不带表达式，要表达式就一次性升级到完整
> process-chain DSL。**
