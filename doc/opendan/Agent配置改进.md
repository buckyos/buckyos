# OpenDAN AgentRootFS 与配置文件改进计划

> **本文性质**:改进计划,不是最终 spec。
>
> **本版定位**:**breaking change**。不维护对旧 `agent.toml` / `BehaviorCfg` 的兼容映射,
> beta2.2 上线即新 schema,旧配置不再识别。
>
> **上游心智**:[OpenDAN Agent 配置开发指导](./OpenDAN%20Agent配置开发指导.md) ——
> Agent Runtime(Gateway)/ Session / Behavior 三层心智,是本计划要让"配置文件长成什么样"对齐的总锚。
>
> **下游事实**:当前实现在 [agent_config.rs](../../src/frame/opendan/src/agent_config.rs) /
> [behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs) /
> [agent_session.rs](../../src/frame/opendan/src/agent_session.rs) /
> [agent.rs](../../src/frame/opendan/src/agent.rs)。本文给出新 schema,旧字段直接淘汰。

---

## 0. 这是什么、不是什么

| | |
|---|---|
| ✅ 本文给出 | AgentRootFS 目录契约(增量调整)、`agent.toml` 新 schema、`behaviors/<name>.toml` 新 schema、配置 DSL 的最小动词集、最小可运行样例 |
| ❌ 本文不给 | 具体解析器实现、Rust 数据结构最终签名、脚本引擎选型、UI / msg-center 接口变更、兼容旧配置的映射 |

---

## 1. 立论:为什么现在动配置格式

三个驱动:

1. **指导文档新立心智** —— Gateway / Session / Behavior 三层已经定下来,但当前 `agent.toml` 太薄
   (5 个字段),Gateway 行为(事件接入、dispatcher)与 Session 类定义都没有显式落盘,全部硬编码在
   `AIAgent::resolve_ui_session` 等地方。Session 类多一种(比如邮件、群、tunnel-A),都得动 Rust。
2. **Action 集合已经在 beta2.2 重新固化为 v2 七元集**(见 [Agent Actions](./Agent%20Actions.md)),
   Behavior 配置应当围绕"事件 → action"组织,而不是 v1 的"prompt + tool_whitelist"。
3. **异常路径需要显式落盘** —— `ContextLimitReached` / provider 失败 / suspend-resume 当前散落在 Rust
   逻辑里,指导文档 §3.3 明确"响应方式跟当前在干什么强相关,写在 Behavior 里",需要 schema 支撑。

**为什么是 breaking change 而不是渐进迁移**:三层概念(Gateway / Session / Behavior)的物理落盘
**重新切分了责任**——Gateway 字段上移、subscribe 从 agent 全局下放到 session class、switch_mode 从
behavior 上提到 session class。这些不是字段重命名,是责任层重新分配,做兼容映射会让两套心智在
代码里共存几个 minor 版本,得不偿失。

判据(贯穿全文):

- **配置层级 ∝ 改动频率**。framework 不许 leak 到 behavior,business 不许 leak 到 agent.toml。
- **显式大于隐式**。"何时停 / 切到哪 / 上下文满了怎么办"必须在配置里看得到。
- **配置即架构**。读 `agent.toml` + 数 `behaviors/` 目录,应当能复述 Agent 的对外形态与全部分支。
- **窄到底,不要半 DSL**。v0 不引入任何表达式;真到表达力不够的那天,**整体跳到 Workflow LLMContext
  DSL**,不在本 schema 上叠加 `when` / 模板插值。详见 §7.1。

---

## 2. 三层落盘对照(决策表)

| 心智层 | 改动频率 | 配置载体 | 物理路径 |
|---|---|---|---|
| Identity + Runtime | 🟦 极少 | `[identity]` `[runtime]` | `<agent_root>/agent.toml`(v0 channels 硬编码,见 §4.2 / §10 #4) |
| Dispatcher(事件 → Session) | 🟦 极少 | `[dispatch]` `[[dispatch.rule]]` | `<agent_root>/agent.toml` |
| Session 类 | 🟨 偶尔 | `[session.<class>]` | `<agent_root>/agent.toml` |
| Behavior(业务) | 🟥 经常 | 每 behavior 一份单文件 | `<agent_root>/behaviors/<name>.toml` |
| Skills / Tools 声明 | 🟥 经常 | 见 §6 4 层 Bin | `<agent_root>/tools/` / `<agent_root>/skills/` |

**改谁查这一张表,不要靠记忆。**

---

## 3. AgentRootFS 目录布局(增量调整)

```text
<agent_root>/
  agent.toml                          # ⬅ 改 schema(§4)
  role.md                             # 自我介绍片段,约定俗成的文件名,由 behavior 提示词 include 引入
  self.md                             # 内部能力 / 边界声明,同上
  .meta/
    rootfs_sync.json                  # 启动期 package → root 同步 manifest(sha256,保护本地修改)
  users/
    <user_id>.md                      # 按 from_did 选择的系统提示词片段
    group_<gid>.md
  memory/                             # AgentMemory 初始化目录
  notepads/<notepad_name>/
  skills/<category>/<skill_dir>/
  tools/                              # Agent Bin 层(见 §6)
  tool_plans/<plan_name>.toml         # behavior 引用(见 §6.3)
  i18n/<language>.toml                # runtime-facing 短文案,如 session one-line status
  behaviors/
    <name>.toml                       # ⬅ 单文件形态(§5)
  workspace/<workspace_id>/
  sessions/<session_id>/
  archive/
    skills/
    sessions/<session_id>/
    workspace/<workspace_id>/
    worklog.db
```

**关于 behavior 单文件**:

本版**只支持单文件**。一个 behavior 的主体是它的提示词,实操中开发者写一个 behavior 时提示词永远是
整体构思的——拆成 `system.md` / `input_msg.md` / `input_event.md` 多个文件只会割裂思考的连贯性。
提示词写在 toml 里(用 TOML 多行字符串),工具能 include 的片段(`role.md` / `self.md` / `users/<x>.md`)
由 prompt compiler 在渲染期处理,不在文件层做拆分。

**仍然有效的硬约束**(与当前实现一致,不变):

- AgentRootFS 内禁止任何二进制可执行文件(ELF / Mach-O / PE / `.so` / `.dylib` / `.dll`),通过 4 层
  Bin 的执行视图来承载二进制。
- `sessions/<id>/` 内不放 `./bin/`,执行视图渲染到容器临时目录 `<buckyos_root>/tools/<agent_id>/<session_id>/`。
- 路径解析单一来源 [`paths.rs`](../../src/frame/opendan/src/paths.rs),禁止"候选 key 列表 + 祖先扫描"。

---

## 4. `agent.toml` 新 schema —— Gateway + Session classes

### 4.1 现状(5 字段,[agent_config.rs](../../src/frame/opendan/src/agent_config.rs))

```toml
agent_did              = ""
display_name           = ""
default_ui_behavior    = ""
default_work_behavior  = ""
subscribe_events       = []
cancel_reason          = ""
preserve_attachment_tag_in_egress = false
filesystem_policy = "workspace"         # workspace / unrestricted
```

问题:Gateway 行为不可表达、session 类不可表达、订阅是扁平数组不挂在具体 session 类上、dispatcher
完全隐式(在 `AIAgent::dispatch_inbound` 里硬编码)。

### 4.2 改进 schema 草案

```toml
# ─── Identity ────────────────────────────────────────────────
[identity]
agent_did    = ""                       # 空 ⇒ 启动期从 system_config 拉的 AgentDocument 回填
display_name = ""                       # 空 ⇒ 从目录名推断
# role.md / self.md 不在这里声明——它们是约定俗成的文件名。当前实现中,
# 由 runtime 读取后作为 `[prompt].on_init` 的 `{{ role_md }}` / `{{ self_md }}` 变量注入
# (upon 语法,通过 `PromptRenderEngine` 渲染)。
# agent.toml 不掺合"哪个 behavior 用哪段身份描述"。

# ─── Runtime ─────────────────────────────────────────────────
[runtime]
cancel_reason = "user requested cancel" # Observation::Cancelled 文案兜底
language = "en"                         # 选择 i18n/<language>.toml,空值等同 en
# 支持 zh / zh-TW / en / es / fr / de / ko / ja / ru; zh-CN、en-US 等 alias 会归一化。
preserve_attachment_tag_in_egress = false

# ─── Channels:Gateway 监听的事件源 ──────────────────────────
# v0 把这两个 channel 硬编码在 runtime 里:
#   - msg_center: 所有 inbound Msg 进 dispatcher
#   - kevent:     task_mgr / kvdoc 等 kevent 流 → dispatcher
# `[[channel]]` 段在 v0 不识别;插件化 channel 接入留到 §10 开放问题 #4。

# ─── Dispatcher:事件类型 → Session class ───────────────────
# v0 故意做"窄":纯事件类型过滤器到 session class 的固定映射。
# 没有 when 表达式、没有 session_id 模板,没有任何"基于事件字段计算"的能力。
# 复杂分发等积累完真实需求再考虑升级(见 §10 开放问题 #1)。
[dispatch]
default_class = "ui"                    # 没有任何 rule 命中时的兜底 session 类

[[dispatch.rule]]
on            = "msg.chat"              # 单事件类型,允许尾部通配 `msg.*`
session_class = "ui"

[[dispatch.rule]]
on            = "msg.group"
session_class = "group"

[[dispatch.rule]]
on            = "task_mgr.*"            # task_mgr 下所有事件
session_class = "work"

# ─── Session 类 ──────────────────────────────────────────────
# session class 由两段组成:
#   - 顶层 `[session.<class>]`:身份 / 拓扑(kind / loop_mode / default_behavior /
#     session_id_strategy / process_stack_limit / enabled)。
#   - `[session.<class>.driver]`:运行期行为(switch_mode /
#     inject_background_environment / report_delivery + 4 个 hook point 子表)。
# 详细对应关系见 [Agent Session §3 + §8](./Agent%20Session.md)。
#
# 关于 active/restore:UI vs Work 的差异(UI 永远 active、Work 看 status)在 v0
# 由 `kind` 隐式区分,无独立 `keep_alive` 字段。真要按 class 显式覆盖时再加。

[session.ui]
kind                = "ui"
loop_mode           = "agent"           # 普通 Agent Loop,default behavior 固定
default_behavior    = "ui_default"
session_id_strategy = "per_peer"        # 见下方枚举
process_stack_limit = 4

[session.ui.driver]
switch_mode                      = "normal" # agent loop 下不会触发,写出来仅为对称
inject_background_environment    = true     # 每轮 user input 前注入 <background_environment>
report_delivery                  = "final_only"

[session.ui.driver.on_init]
filter     = "all"
pull_msg   = "none"
pull_event = "none"

[session.ui.driver.on_behavior_switch]
filter     = "top"
pull_msg   = "all"
pull_event = "none"

[session.ui.driver.on_wakeup]
filter     = "top"
pull_msg   = "all"
pull_event = "none"
load_background_hits = "all"

[session.group]
kind                = "ui"
loop_mode           = "agent"
default_behavior    = "group_default"
session_id_strategy = "per_group"

[session.group.driver]
switch_mode                   = "normal"
inject_background_environment = true
report_delivery               = "final_only"

[session.work]
kind                = "work"
loop_mode           = "behavior"        # Behavior Loop,状态机
default_behavior    = "work_default"
session_id_strategy = "per_event_session"
process_stack_limit = 8

[session.work.driver]
switch_mode                   = "fork"   # "normal" | "fork" | "independent",整个 class 统一
inject_background_environment = false
report_delivery               = "final_only"   # "final_only" | "top_level" | "all"

[session.work.driver.on_behavior_switch]
filter     = "top"
pull_msg   = "all"
pull_event = "all"

[session.work.driver.on_behavior_step_ob]
filter     = "top"
pull_msg   = "all"
pull_event = "all"

[session.work.driver.on_wakeup]
filter     = "top"
pull_msg   = "one"
pull_event = "none"

# ─── 特化 Session 类:SelfCheck / SelfImprove ───────────────
# 这两类 session 在调度上是 Work Session 的特化形式,通过 `kind` 字段标记;
# 启用 / 禁用由 `enabled` 控制(默认 enabled = true,Jarvis 模板里两者都先关掉)。
# 详细语义见 [Agent Session §3.3 / §3.4](./Agent%20Session.md)。

[session.self_check]
enabled             = false
kind                = "self_check"
loop_mode           = "behavior"
default_behavior    = "self_check"
session_id_strategy = "singleton"       # 整个 class 一个 session
process_stack_limit = 2

[session.self_check.driver]
switch_mode                   = "normal"
inject_background_environment = false
report_delivery               = "final_only"

[session.self_check.driver.on_init]
filter     = "all"
pull_msg   = "none"
pull_event = "timer.*"

[session.self_check.driver.on_behavior_switch]
filter     = "top"
pull_msg   = "none"
pull_event = "timer.*"     # 只接受 timer.* 命名空间事件

[session.self_improve]
enabled             = false
kind                = "self_improve"
loop_mode           = "behavior"
default_behavior    = "self_improve"
session_id_strategy = "singleton"
process_stack_limit = 2

[session.self_improve.driver]
switch_mode                   = "normal"
inject_background_environment = false
report_delivery               = "final_only"

[session.self_improve.driver.on_init]
filter     = "all"
pull_msg   = "none"
pull_event = "none"       # SelfImprove 不消费 pending queue 外部输入
                          # history / global_state 由框架直接注入 env

[session.self_improve.driver.on_behavior_switch]
filter     = "top"
pull_msg   = "none"
pull_event = "none"
```

**Hook point × pull policy 详解**见 [Agent Session §8](./Agent%20Session.md);
`pull_event = "timer.*"` 这类 filter 命名空间当前固定在
[`TimerEventKind`](../../src/frame/opendan/src/session_model.rs) 列举的几个事件类型,
未识别的 filter 会在 startup 期被 `validate_driver_filters` 拒绝。

**关于 `switch_mode` 放在 session class 而非 behavior**:

这是**有意识的限制**,不是简化遗漏。本版主张:**LLM 不应该选择切换模式**——它只通过
`<next_behavior>` 表达"我想去哪",**怎么去**(普通切换 / fork / independent)是 session class
的固定属性。

这等价于把 v0 的状态机能力**钉死在"单 mode 状态机"**上。真要表达复杂分支(planner → executor 用
normal、planner → summarizer 用 fork),本版的回答是:**这种诉求不应该长在 behavior 配置里,
应该用 Workflow LLMContext DSL 去描述**。Workflow 与 behavior 是两套不同的编排载体——behavior 是
"LLM 自己驱动的状态机",Workflow 是"显式编排的执行图"。本版 behavior 选择前者并停在简化形态,
不模糊两者边界。

**`session_id_strategy` 枚举**(v0 固化、不可扩展):

| 策略 | session_id 形态 | 适用 |
|---|---|---|
| `per_peer` | `<class>-<event.from_did>` | 一个对话方一个 session(UI 类典型) |
| `per_group` | `<class>-<event.group_id>` | 一个群一个 session |
| `per_event_session` | `<event.session_id>` | event 自带 session_id(task_mgr 完成事件、worksession 路由) |
| `singleton` | `<class>` | 整个 class 全局只有一个 session(系统级、调度类) |

策略选择由 session class 决定,**不由 dispatcher rule 决定**——这是 §1 立论里"配置层级 ∝ 改动频率"
的具体落地:dispatcher 想加新 rule 不应当被迫想清楚 session_id 怎么取。

未命中策略所需字段(如 `per_peer` 收到一条没有 `from_did` 的 event)⇒ drop + warn。

### 4.3 改进点与现状映射(供 code change 时参考,不做运行期兼容)

| 现状字段 | 新位置 | 说明 |
|---|---|---|
| `agent_did` / `display_name` | `[identity]` | 同语义,分组到 identity 段 |
| `default_ui_behavior` | `[session.ui].default_behavior` | session 类拥有自己的 default |
| `default_work_behavior` | `[session.work].default_behavior` | 同上 |
| `subscribe_events` | `[session.<class>.driver.<hook>].pull_event` | 订阅从 Agent 全局下放到 session class 的 hook point,粒度可按 hook 不同 |
| `cancel_reason` | `[runtime].cancel_reason` | 同语义 |
| `preserve_attachment_tag_in_egress` | `[runtime]` | 同语义 |
| — | `[runtime].filesystem_policy` | **新增**,控制 read / attachment 的本地文件路径策略 |
| — | `[dispatch]` | **新增**,dispatcher 显式化(当前由 `AIAgent::dispatch_inbound` + `FixedRulesDispatch` 消费) |
| — | `[session.<class>].kind` | **新增**,显式标识 `ui` / `work` / `self_check` / `self_improve` |
| — | `[session.<class>].enabled` | **新增**,关停一整个 session class(SelfCheck/SelfImprove 默认 `false`) |
| — | `[session.<class>].loop_mode` | **新增**,对齐指导文档 "UI Session 是 loop_mode=agent 的特例" |
| — | `[session.<class>].session_id_strategy` | **新增**,固化 session_id 派生策略 |
| — | `[session.<class>].process_stack_limit` | **新增**,Fork / Independent 切换前由 runtime 检查;0 ⇒ 不设上限;超限按 §10 #3 拒绝并写 worklog |
| — | `[session.<class>.driver].switch_mode` | **新增**(从 behavior 上提),整个 class 统一 |
| — | `[session.<class>.driver].inject_background_environment` | **新增**,控制是否在每轮 user message 前动态注入 `<background_environment>` |
| — | `[session.<class>.driver].report_delivery` | **新增**,控制 WorkSession report 上行到 UI session 的默认范围 |
| — | `[session.<class>.driver.{on_init, on_behavior_switch, on_behavior_step_ob, on_wakeup}]` | **新增**,Driver hook point × filter × pull policy(详见 [Agent Session §8](./Agent%20Session.md)) |

> 旧 schema 直接在 `[session.<class>]` 顶层挂 `subscribe_events` / `switch_mode` /
> `inject_background_environment` / `report_delivery` 等字段,这些位置在
> beta2.2 起被 [`warn_deprecated_session_keys`](../../src/frame/opendan/src/agent_config.rs) 警告,
> 必须迁到 `[session.<class>.driver]` 下。
>
> `keep_alive` 在改进计划早期草案出现过,v0 实际未实现也未解析——active/restore 差异由 `kind` 隐式区分,
> 写在 toml 里只会被静默丢弃。
> 同理 `[[channel]]` 段在 v0 不识别,Gateway 接入是 runtime 内硬编码的 `msg_center` + `kevent`。

---

## 5. Behavior 配置 —— 从 "prompt + tools" 到 "prompt + capabilities + 旁路开关"

### 5.1 现状([behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs))

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

问题:**事件不见了,action 不见了,next 不见了**。Behavior 看起来像一份 LLM 调用模板,而不是
指导文档说的"对一组事件的响应表 + 状态机的一个节点"。

### 5.2 改进 schema —— 单文件形态

Behavior 配置的心智模型很简单:**走到这里,session 已经决定要推理了**,behavior 只关心三件事——
**渲染什么提示词、能用什么能力、异常路径有没有打开**。状态机的边、事件过滤、switch_mode 都不在
behavior 里:

- "什么事件触发推理" 是 session 类的事(见 §4.2 `[session.<class>].subscribe_events`)
- "下一个 behavior 是哪个" 由 LLM 在 `<next_behavior>` 输出端决定,runtime 不预先约束
- "切的时候用 normal / fork / independent" 是 session 类的事(见 §4.2 `[session.<class>].switch_mode`)

完整单文件示例(`behaviors/explorer.toml`):

```toml
# ─── meta ───────────────────────────────────────────────────
[meta]
name      = "explorer"
objective = "explore unknown territory"

# ─── prompt 渲染:当前实现是简单 `{var}` 替换 ───────────────
# `on_init` / `on_behavior_switch` 通过 `llm_context::PromptRenderEngine` 渲染
# (upon `{{ var }}` + OpenDAN `__VAR__` / `__ENV__` / `__INCLUDE__`)。完整变量契约见
# `doc/opendan/Agent Enviroment.md` §15.1。提示词内联在 toml 多行字符串里
# ——一个 behavior 的提示词是一个整体,不拆文件;长内容需要外置时用
# `__INCLUDE($paths.agent_root/<file>)__`。
[prompt]
parser = "xml"                          # LLM 输出 parse 方式;renderer 等细节全部走默认值

# 进入这个 behavior 的瞬间,渲染 system 段
# 常用变量(详见 Agent Enviroment.md §15.1):
#   {{ session.id }} {{ session.title }} {{ session.objective }}
#   {{ behavior.name }} {{ behavior.objective }}
#   {{ workspace.id }} {{ paths.agent_root }} {{ paths.session_root }}
#   {{ runtime.clock_unix_ms }}
# render-time extras (由 render_system_messages 注入):
#   {{ role_md }} {{ self_md }}
on_init = """
{{ role_md }}
{{ self_md }}

You are now in behavior `{{ behavior.name }}`. Your objective: {{ behavior.objective }}.

## Available actions
Use only the tools allowed by this behavior's capabilities.
"""

# 当 next_behavior 切换到这个 behavior 后,渲染为一条 synthetic User input,
# 触发切换后的继续运行。当前可用同一套 PromptRenderEngine 变量。
# 不写 ⇒ 切换后等待下一条真实输入。
on_behavior_switch = """
Continue in behavior `{{ behavior.name }}`.
"""

# v0 没有 `on_input_msg`:PendingInput::Msg 始终作为原始 User AiMessage 拼进本轮 input,
# 想统一改写消息文本得改 src(或者等 §10 #5 prompt 模板默认值统一升级)。

# 收到一条业务 event,即将拼进本轮 input 时渲染
# 行为级 fallback:只有没有命中 subscription 自带 message_template 时才使用。
# 可用变量:{event_id} {data},以及 JSON payload 顶层 scalar 字段的 {key}。
# 不写 ⇒ 走 runtime 内建的最小事件模板(event_id + payload)
on_input_event = """
[event {event_id}]
{data}
"""

# ─── 能力声明(占位,§5.3 详述)────────────────────────────
# v0 暂时直接沿用现状字段语义,只是物理位置移到本段下。
# v0 之后会与 skill / function tool 配置一起重新设计,届时 break change。
[capabilities]
tool_whitelist    = ["exec_bash", "write_file", "edit_file", "read", "sendmsg"]
disable_capabilities = ["web_search"]
approval_required = []
tool_plan         = "minimal_safe"

# ─── Budget / Model ─────────────────────────────────────────
[budget]
max_rounds              = 16
max_consecutive_errors  = 3
max_total_tokens        = 200_000

[model]
preferred = "claude-opus-4-7"

# ─── 旁路 LLM 逻辑(异常路径的开关)─────────────────────────
# 这一段的作用是"一眼看出来哪些旁路打开了"。v0 不追求灵活性:每个 on_xxx 就是一个固定模式,
# 写出来 ⇒ 旁路启用;删掉 ⇒ 旁路关闭,命中时按 runtime 默认(通常是结束 process)。
# 想调策略细节先改 src/,等真有第二种合理策略再扩 schema。
[on_context_limit_reached]
mode = "compress_then_continue"         # 唯一支持的模式;填上即启用

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

# [on_interrupt_discard]
# mode = "end"                          # 不写 ⇒ 用 runtime 默认
```

### 5.3 `[capabilities]` 段:v0 占位

`[capabilities]` 段的最终形态需要跟 skill bundle / 传统 function-call tool 的配置形态一起定。当前
v0 **直接沿用现状 `BehaviorCfg.tool_whitelist` / `approval_required` / `tool_plan` 三个字段的语义**,
只是物理位置挪到本段下:

| 字段 | 语义 | 与现状关系 |
|---|---|---|
| `tool_whitelist` | v2 Action 七元集 + 已注册 function-call tool 的命名白名单 | 同 `BehaviorCfg.tool_whitelist` |
| `disable_capabilities` | 禁用 AICC 默认注入的模型能力，例如 `web_search` | 新增 |
| `approval_required` | 调用前需要人审的工具名清单 | 同 `BehaviorCfg.approval_required` |
| `tool_plan` | PATH overlay 工具策略(见 §6.3) | 同 `BehaviorCfg.tool_plan` |

**这一段在 v0 之后会重做**,等三类能力(v2 内建 Action / Skill bundle / Function-call tool)的统一
配置形态收敛(Open question #2)。届时是一次 schema break,会在升级文档里说明。

### 5.4 改进点与现状映射

| 现状字段 | 新位置 | 备注 |
|---|---|---|
| `name` / `objective` | `[meta]` | — |
| `mode` | `[session.<class>].loop_mode` | **上提到 session 类**:agent loop vs behavior loop 是 session 决定的 |
| `system_prompt_template` | `[prompt].on_init` | 渲染时机命名化;**内联在 toml 多行字符串里** |
| `parser` | `[prompt].parser` | — |
| `renderer` / `parser_strict` / `renderer_cfg` | 不再配置 | 走默认值,要调直接改 src |
| `output` | `[prompt].output` | 没变(不常用,按需保留) |
| `tool_whitelist` / `approval_required` / `tool_plan` | `[capabilities]` | 字段名语义不变,见 §5.3 |
| `max_rounds` / `max_consecutive_errors` / `budget` | `[budget]` | — |
| `switch_mode` | `[session.<class>].switch_mode` | **上提到 session 类** |
| `model` | `[model]` | — |
| — | `[prompt].on_behavior_switch` | **新增**:切换到该 behavior 后自动注入 synthetic User input,触发继续运行 |
| — | `[prompt].on_input_event` | **新增**:行为级事件 fallback —— 没有命中 subscription 自带 template 时使用 |
| — | `[on_xxx]` 旁路开关 | **新增**:异常路径从 Rust 默认逻辑里提出来做成可见开关 |

### 5.5 `[on_xxx]` 与 LLM 输出 `<actions>` 的层级澄清

| | 谁决定 | 出现时机 | 例子 |
|---|---|---|---|
| `[on_xxx]`(**配置**) | 配置开发者 | LLMContext 抛出异常事件 | "上下文满了 ⇒ 压缩后继续" |
| `<action>`(**LLM 输出**) | LLM 在一轮推理内 | LLM 决定本轮要写一个文件 | "本轮输出 `<write_file>`" |

`[on_xxx]` 是**旁路开关**(异常路径打不打开、按哪个固定策略走),`<actions>` 是 LLM 在正常推理内
的**输出**(这一轮想做什么)。两者属于不同维度,不冲突也不重叠。

**正常路径下"什么事件触发推理"不在 behavior 里**——那是 session 类 `subscribe_events` 决定的(§4.2)。
behavior 永远是"已经决定推理,正在准备这一轮"的状态。

---

## 6. 4 层 Bin / Tool Plan(保留现有契约,简述)

完整描述见原 §3。本节只复述硬约束,方便单文件阅读。

| 层 | 物理路径 | 持久化 |
|---|---|---|
| System Bin | `<buckyos_root>/tools/store/` | 随 Worker Image |
| Runtime Bin | `<buckyos_root>/tools/bin/` | 容器临时 |
| Agent Bin | `<agent_root>/tools/` | AgentRootFS 持久化 |
| Session Bin 声明 | `<agent_root>/sessions/<sid>/tools/` | AgentRootFS 持久化 |
| Session Bin 执行视图 | `<buckyos_root>/tools/<agent_id>/<sid>/` | 容器临时 |

### 6.1 PATH overlay
```
PATH = SessionExecBin : AgentBin : RuntimeBin : SystemBin : <inherited>
```

### 6.2 Session `./tools/` 硬约束
- 只放文本(脚本源码、`tool.toml`、prompts、schema),禁止二进制。
- 单文件建议 ≤ 64 KB,整目录文件数建议 ≤ 几百(hot path,每次 `exec_bash` 起手要做 mtime 同步)。

### 6.3 Tool Plan
位置:`<agent_root>/tool_plans/<plan>.toml`;schema 与渲染时机不变。

**改进点(与 behavior schema 联动)**:
- `tool_plan` 字段从 behavior 顶层移入 `[capabilities].tool_plan`,强调"tool plan 是 capability
  配套,不是 behavior 独立维度"。

---

## 7. DSL 设计 —— v0 故意不开表达式

### 7.1 原则(v0)

1. **完全没有表达式**。整份配置里**所有**字符串字段要么是事件类型 / 文件路径,要么是固定枚举常量。
   不存在 `when = "msg.kind == 'chat'"` 这种写法、也不存在 `session_id = "ui-${msg.from_did}"` 这种
   模板插值。一旦哪天觉得不够表达,**直接整体跳到 Workflow LLMContext DSL**,而不是"小步演进"——
   小步加表达式的代价是产生第二种半成品语义层,跟将来的完整 DSL 不兼容。
2. **配置字段全是枚举或字面量**。整份 schema 里 runtime 需要"解释"的部分只有:dispatcher 的事件
   类型字符串(含尾部通配)、各种 enum、文件路径、命名引用(behavior 名 / tool plan 名 / skill 名)。
   不存在条件判断、不存在算术、不存在变量替换。
3. **命名引用是字面量,不是表达式**。`target = "explorer_safe_mode"` / `default_behavior = "ui_default"` /
   `tool_plan = "minimal_safe"` 这种通过名字引用其它配置实体的字段,在 schema 里都是字符串字面量。
   runtime 在**加载期做存在性校验**,运行期不做名字解析或拼接。
4. **事件类型只支持尾部通配**。`msg.chat` 精确匹配,`msg.*` 匹配 `msg.` 任一后缀。**没有**中间通配
   (禁止 `*.completed`),没有交集 / 并集,没有否定。
5. **session_id 派生是 4 选 1 的枚举**(见 §4.2 `session_id_strategy`),不是表达式。
6. **旁路只做"开/关 + 唯一策略"**。`[on_xxx]` 每段只允许一个固定 `mode`(§7.2),不允许 `if/else` /
   多策略并列。要第二种策略 ⇒ 写第二个 `[on_xxx]` 段(但当前 schema 没给)。
7. **分支判断让 LLM 决定**。"下一个 behavior 是谁"由 LLM 在 `<next_behavior>` 输出端表达——这是
   prompt 工程的职责,不是配置 DSL 的职责。**LLM 不决定切换模式**,只决定 target,模式由 session
   class 统一(§4.2 `switch_mode`)。
8. **复杂状态机走 Workflow LLMContext DSL,不在 behavior schema 里长**。本计划的 behavior 配置
   有意停在"单 mode 状态机"——planner / executor / summarizer 这种节点间需要不同切换模式的场景,
   是 Workflow 的领域,不是 behavior 的领域。两套载体边界清晰,本 schema 不模糊化。
9. **未来升级路径**:见 §10 开放问题 #1。**不允许小步加 `when`**——要升级就一次性引入完整的
   Workflow LLMContext DSL 替换本节,确保只有一种表达层语义。

### 7.2 旁路 `mode` 清单(v0 固化)

整份配置里 v0 唯一的"枚举值字段"集中在 behavior `[on_xxx]` 旁路开关上。每个 `[on_xxx]` section
都对应一个固定的 `mode`,写出来即启用:

| `[on_xxx]` 段 | 允许的 `mode` | 含义 |
|---|---|---|
| `[on_context_limit_reached]` | `compress_then_continue` | 上下文满 ⇒ 压缩后继续 |
| `[on_llm_message_compress]` | `context_window_ratio` | 一轮完成后按 context window 使用率自动压缩；不写则不启用自动压缩 |
| `[on_provider_failed]` | `fallback_behavior`(带 `target = "<name>"`) | provider 失败 ⇒ 切到 target |
| `[on_interrupt_graceful]` | `cancel_pending_tools_then_continue` | 收到 graceful 中断 ⇒ 注入 Cancelled 后继续 |
| `[on_interrupt_discard]` | `end` | 收到 discard 中断 ⇒ 直接结束 process |

每个 `[on_xxx]` 段当前**只允许一种 mode**——这是"先做开关,不做策略灵活性"原则的具体体现。需要
第二种合理策略时再扩;不预先开口子。`[on_llm_message_compress]` 的可调参数是该固定策略的阈值:
`trigger_ratio`、`target_ratio`、`hard_limit_ratio`、`min_turns_between_compress`、
`preserve_cache_stability`，以及调试/兜底用的 `context_window_tokens`。生产环境应优先依赖
AICC `models.list` 返回的 `capabilities.max_context_tokens`。**未列出的 section 或 mode ⇒ parse error**。

其它枚举字段(`loop_mode` / `switch_mode` / `session_id_strategy` / dispatcher `on` 通配)分散在
各自的 schema 段中,本节不重复罗列。

### 7.3 留白:把"想写代码"的诉求踢给注释

需要表达 v0 动词集合覆盖不到的逻辑时,**唯一允许的形式是 TOML 注释**——它对 runtime 完全
透明,只是给读配置的人看:

```toml
[on_context_limit_reached]
mode = "compress_then_continue"
# NOTE: 真要做"先看 pending_inputs 多不多再决定压几轮"这种逻辑,
#       目前的去处是直接改 src/frame/opendan/src/llm_context_helper.rs::compact_history,
#       不要在这里加 when / script。等 Workflow LLMContext DSL 上线再统一搬过来。
```

**这一节是故意没给"小型脚本钩子"的**。前一版本草案里有 `script = "handlers.rhai::..."` 留白,
被砍掉了——原因:留个半残钩子比不留更糟,社区会基于它写一堆"半 DSL 配置",等真要升级到完整
Workflow DSL 时就尾大不掉。要么不要,要么一次性全要。

---

## 8. 部署 / FS / COW(简述)

这里只列**不变事实**,方便单文件阅读:

- **数据 / 运行时分离**:AgentRootFS 在宿主机普通文件系统;AgentRuntime 在 Linux 容器内。
- **跨平台契约只落在 AgentRootFS**:目录结构、配置、session 数据平台无关;执行视图(PATH 里的 bin、
  tmux pane、临时挂载)允许纯 Linux 形态。
- **`agent_root` 来源**:`opendan` 启动 BuckyOS AppService runtime 后,用 runtime appid / owner 调用
  `get_buckyos_app_data_dir(appid, owner)` 获取。
- **COW 由容器内 OverlayFS 实现**:`OverlayFS(Package[RO], Data[RW])`,host 不做 overlay 生命周期。
- **`opendan` 进程不理解 Docker**,只消费 BuckyOS runtime 解析出的 app data dir。

确定性读取规则(不变):

- 不允许从 cwd 向上扫祖先目录推 `agent_root`。
- 不允许"哪个目录有 `worklog.db` 就是根"这类反推。
- session workspace root:绑定 ⇒ `<agent_root>/workspace/<id>/`;未绑定 ⇒ `<agent_root>`。

---

## 9. 落地节奏

| 阶段 | 内容 |
|---|---|
| **beta2.2** | 新 schema 全量替换。`AgentCfg` / `BehaviorCfg` 直接重写,旧字段不识别。`AIAgent::dispatch_inbound` 从硬编码改读 `[dispatch]`;`AIAgent::resolve_*_session` 从硬编码改读 `[session.<class>]`。 |
| **beta2.3** | `[capabilities]` 段配合 skill bundle / function-call tool 收敛重做(再一次 break)。 |
| **beta2.4+** | 若 §10 #1 触发条件之二同时成立 ⇒ 启动 Workflow LLMContext DSL 设计,**一次性**替换 §4.2 dispatcher / §5.2 behavior 中的窄设计。 |

**没有兼容映射、没有 `opendan migrate-config`、没有 strict 模式开关**。手上的现有 agent 都是
团队内部维护的,beta2.2 上线时一次性手改 + lint 通过即可。这是 break change 应有的姿态:
快、利落、不留半模。

---

## 10. 待定事项(Open questions)

按优先级排:

1. **何时升级到 Workflow LLMContext DSL**:v0 dispatcher 是纯"事件类型 → session class"映射,
   behavior 不带表达式,switch_mode 钉死在 session class。这套窄到底的设计有意识地拒绝"小步加
   表达式"——见 §7.1。升级触发条件(提前定下来避免临时拍):
   - 同一事件类型需要按 payload 字段路由到不同 session class(例如 `msg.chat` 按
     `recipient_group_id` 分发到不同 group session);
   - 收到事件需要根据 payload 决定要不要触发推理(例如 "from blacklist 的 user_msg 直接丢");
   - 单 session class 内需要按当前 behavior 选不同 switch_mode(planner→executor 用 normal,
     planner→summarizer 用 fork)。

   三类需求都不出现就不引入表达式。一旦其中**两类**同时变现实需求,**一次性**用 Workflow LLMContext
   DSL 替换 §4.2 / §5.2 / §7 的窄设计,不在 v0 上叠加 `when`。

2. **`[capabilities]` 最终 schema**:§5.3 v0 直接沿用现状三个字段,但需要在同一段下表达三类能力
   (v2 内建 Action / skill bundle / 传统 function-call tool),且每类都有"上层引用 + 下层细节定义"
   的解耦诉求。需要等 [Agent Skill](./Agent%20Skill.md) 与 ToolManager 的配置形态一起定下来再收敛。
   beta2.3 重做。

3. **`process_stack_limit` 超限语义**(已落地为 v0 行为,保留为开放问题以备调优):
   `[session.<class>].process_stack_limit` 在 Fork / Independent 切换前由 `switch_behavior`
   检查;超限 ⇒ 拒绝切换、`warn!` 一行、写一条
   `type = "agent.session.behavior_switch_refused"` 的 worklog 记录、把当前进程优雅
   结束(UI 回 `WaitForMsg`,Work 回 `End`)。0 ⇒ 不设上限。
   未来可能演化的方向(留作 open):换成 observation 让 LLM 自处理、或自动 unwind
   最早的非 entry frame——但都需要先看到生产数据。

4. **Channel 类型的对外接口**:v0 把 Gateway inbound channel 硬编码在 runtime(`msg_center` +
   `kevent`),`[[channel]]` schema 段不识别。后续要插件化 channel(`http` 等)时,需要
   定一个准入表,跟 [Agent Session 的事件订阅](./Agent%20Session的事件订阅.md) §2 三种模式
   对齐,再决定是恢复 `[[channel]]` 段还是另起更窄的 schema。

5. **prompt 模板默认值**:当前 `[prompt].on_init` 使用简单 `{var}` 替换;`on_input_event` 作为
   subscription template 之后的 fallback,同样使用简单 `{var}` 替换。若后续要切到
   [Render_Prompt_Template_Variables](./Render_Prompt_Template_Variables.md) 中的通用
   `PromptRenderEngine`,需要先补 OpenDAN 专用变量 loader,并把可访问变量清单重新定稿。
   同时可考虑是否要补一个 `on_input_msg`(早期草案保留过,v0 未实现亦未保留 schema 字段)。

---

## 11. 最小可运行样例

最小 echo agent(对人对话回声),展示新 schema 的"地板"形态。两个文件即可跑起来:

**`<agent_root>/agent.toml`**:

```toml
[identity]
display_name = "echo-bot"

[runtime]
cancel_reason = "user canceled"
filesystem_policy = "workspace"

# v0 不需要写 [[channel]]:msg_center + kevent 是 runtime 硬编码的。

[dispatch]
default_class = "ui"

[[dispatch.rule]]
on            = "msg.chat"
session_class = "ui"

[session.ui]
loop_mode           = "agent"
default_behavior    = "ui_default"
session_id_strategy = "per_peer"

[session.ui.driver]
switch_mode = "normal"
```

**`<agent_root>/behaviors/ui_default.toml`**:

```toml
[meta]
name      = "ui_default"
objective = "echo whatever the user says"

[prompt]
parser  = "xml"
on_init = """
You are an echo bot. For each user message, output `<report>` with the
exact text the user sent, then `<next_behavior>END</next_behavior>`.
"""

[capabilities]
tool_whitelist = []
tool_plan      = "minimal_safe"

[budget]
max_rounds = 2

[model]
preferred = "claude-haiku-4-5-20251001"
```

**到这里就是一个能跑的 agent**。没有 `[on_xxx]` 旁路(异常按默认行为走),没有 `role.md` / `self.md`
(echo bot 不需要身份段),消息文本直接进 input,event 走 runtime 内建默认模板。

更复杂的 agent 在此之上**只增不改结构**:加 `[[dispatch.rule]]`、加 `[session.<class>]`、
在 `behaviors/` 下加文件、把 `[prompt].on_init` 写得更厚——schema 形态本身不变。

---

## 一句话总结

> **`agent.toml` 承载 Session 类骨架(事件类型 → session class 的固定映射,loop / switch /
> session_id 派生策略 / process_stack_limit 全归 session 类;Gateway channel v0 硬编码);
> `behaviors/<name>.toml` 单文件承载"已经决定要推理之后,渲染哪段提示词、能用哪些能力、
> 有没有打开异常旁路"——prompt 模板字段内联在 toml 里(`on_init` / `on_behavior_switch` /
> `on_input_event` 当前生效)+ `[capabilities]` + `[on_xxx]` 开关。v0 整份配置不带表达式,
> LLM 不选择 switch_mode,复杂状态机走 Workflow LLMContext DSL,不在本 schema 上叠加。**
