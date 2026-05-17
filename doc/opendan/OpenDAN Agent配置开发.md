# OpenDAN Agent 配置开发

本文档面向**Agent 开发者**：你想做一个新的 Agent（比如 `summarizer`、`finance_helper`、`code_reviewer`），希望像发布 App 一样把它打包、分发、安装到任意一个 BuckyOS Zone 的 `opendan` 进程里运行。

读者预期：

- 已经读过 [Agent RootFS](Agent%20RootFS.md)（数据布局）和 [Agent Session](Agent%20Session.md)（运行模型）。
- 知道 OpenDAN 不是"写一段 system prompt 就完事"——它是一个有状态的、可恢复的、多 behavior 的 agent loop。

本文档不重复 RootFS / Session / Behavior 的规范，只回答开发者真正会问的问题：**"一个 Agent 包到底由哪些文件组成？每个文件该怎么写？怎么调试？怎么发布？"**

---

## 1. 一个 Agent 包是什么

一个 Agent 包是一个**普通目录**，被 BuckyOS 的安装器（`node_daemon`）拷贝/同步到目标 Zone 上当作 `agent_package_root`。`opendan` 启动时按 [Agent RootFS §9.3](Agent%20RootFS.md) 的 `sync_agent_rootfs_from_package` 把包内容同步进 `agent_root`（保护本地修改），随后整套就以 AgentRootFS 的形态运行。

包目录的最小骨架：

```text
<agent_package_root>/
  agent.toml                # Agent 自身配置（身份、默认 behavior、订阅事件）
  role.md                   # 角色定义，进 system prompt
  self.md                   # 自我能力声明，进 system prompt
  behaviors/
    ui_default.toml         # UI session 默认 behavior
    work_default.toml       # Work session 默认 behavior（可选）
    <name>.toml             # 其它 behavior（可选）
  skills/                   # 默认 skill 包（可选）
    <skill_name>/
      meta.json
      skill.md
  tools/                    # Agent Bin 层脚本工具（可选）
    <name>.sh / <name>.py / <name>/tool.toml
  tool_plans/               # 工具策略文件（可选）
    <plan>.toml
  users/                    # 针对特定调用者的 system prompt 片段（可选）
    <user_id>.md
    group_<gid>.md
  readme                    # 包说明，仅给人读
```

**没有可执行二进制**：Agent 包是平台无关的纯文本（脚本 + 配置 + Markdown）。需要二进制工具走 ExtTool Volume / Crafter 走 Runtime Bin 层，见 [Agent RootFS §3](Agent%20RootFS.md)。

---

## 2. `agent.toml` — Agent 身份与启动配置

落地位置：`<agent_root>/agent.toml`。结构来自 [agent_config.rs](../../src/frame/opendan/src/agent_config.rs) 的 `AgentTomlFile`。**所有字段都有默认值**，最小可启动的 `agent.toml` 可以是空文件，但生产 Agent 至少应该填写下面这些：

```toml
# Agent 在 BuckyOS Zone 内的全局 DID。
# 留空时由 opendan 启动期从 buckyos identity 自动写回，不要手填。
agent_did = ""

# 给人看的名字，用于 worklog / UI / 多 agent 协作里的可读标识。
display_name = "Jarvis"

# UI session 创建时默认走哪个 behavior（必须有对应的 behaviors/<name>.toml）。
# 留空 ⇒ "ui_default"
default_ui_behavior = "ui_default"

# Work session 由 try_create_worksession 派生时的默认 behavior。
# 留空 ⇒ "work_default"
default_work_behavior = "work_default"

# 这个 Agent 关心的事件类型，会注册到 task_mgr 并通过
# session_event_pump 推到对应 session。事件类型详见 Agent Session 的事件订阅。
subscribe_events = [
  "msg.incoming",
  "task.completed",
]

# 被中断时 Observation::Cancelled.reason 的兜底文案。
# 留空 ⇒ "user requested cancel"
cancel_reason = "用户取消了当前推理"

# 是否把 incoming 消息上携带的 attachment 标签透传到 outgoing 消息。默认 false。
preserve_attachment_tag_in_egress = false
```

**字段语义和兜底**全部走 `AgentConfig::open()` 的逻辑，开发者要记的只有一点：**没有 `agent.toml` 也能启动**，但默认 behavior 名要么显式配，要么得保证 `behaviors/ui_default.toml` 存在。当 `behaviors/ui_default.toml` 不存在且 toml 也没改默认值时，运行时会用 [`AgentConfig::builtin_ui_default()`](../../src/frame/opendan/src/agent_config.rs) 兜底——仅含 v2 七个固化 Action（`exec_bash` / `write_file` / `edit_file` / `read` / `subscribe_event` / `unsubscribe_event`），适合"先跑起来再说"的最小演示。

---

## 3. `role.md` 和 `self.md` — 进 system prompt 的人格层

两者都是纯 Markdown，启动时被 `Render_Prompt_Template_Variables` 的对应变量拼进 system prompt：

- **`role.md`**：**面向 LLM 的角色定义**。"You are X. You serve Y. You will / will not Z." 这是 Agent 人格的稳定骨架，跨 behavior 不变。
- **`self.md`**：**Agent 自己写给自己的 self-reflection**。可以是空的、可以由 Agent 在运行期通过 `write_file` 自我更新（自我成长机制的简化形态）。它放在 system prompt 里是让 Agent 看见自己上次留下的便条。

例子（参考 `src/rootfs/bin/buckyos_jarvis/role.md`）：

```markdown
You are Jarvis, the user's primary personal Agent running on OpenDAN.

You are not an app. You are not a chatbot. You are the user's long-term
companion intelligence — a unified entry point through which they interact
with the digital world. ...

You serve exactly one person: your owner. ...
```

`self.md` 通常一开始很短，留给 Agent 自己生长：

```markdown
<self-thinking>
I still don't know much about my owner; I need to find a way to take the
initiative and get him to talk more about himself.
</self-thinking>
```

写作要点：

- **稳定**：role.md 不是任务说明，是身份定义。写得跨任务通用。
- **不要塞工具描述**：工具说明走 `behaviors/<name>.toml` 的 `tool_whitelist` + 提示词模板变量自动注入。
- **不要塞具体业务流程**：那是 behavior 的事。

---

## 4. `behaviors/<name>.toml` — 行为级提示词与执行策略

每一份 `.toml` 是一个 behavior。Session 在任一时刻只有一个 active behavior；通过 `<switch_behavior>` Action 在 behavior 之间迁移。结构来自 [behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs) 的 `BehaviorCfg`。

### 4.1 一份典型 behavior 文件

```toml
# behaviors/ui_default.toml
name      = "ui_default"
objective = "interactive UI session — listen, route, dispatch"

# system prompt 模板。会被 PromptRenderEngine 渲染，可以用
# __ENV / __INCLUDE / __OPENDAN_VAR / __OPENDAN_CONTENT 等指令。
# 详见 Agent Prompt Compiler / Render_Prompt_Template_Variables。
system_prompt_template = """
__INCLUDE(role.md)__

__OPENDAN_CONTENT(session_skills)__

# Current task
__OPENDAN_VAR(session_objective)__
"""

# Action / tool 白名单。空数组 ⇒ 全部可见。
# 写入这里的名字必须是注册过的 Action，或可被 bin overlay 解析的可执行名。
tool_whitelist = [
  "exec_bash",
  "write_file",
  "edit_file",
  "read",
  "report",
  "subscribe_event",
  "unsubscribe_event",
]

# 需要人工/Owner 审批才能执行的 Action（弹审批，被批准前不发出）。
approval_required = ["exec_bash"]

# 可选：引用 tool_plans/<name>.toml，做更细的 deny/allow 策略。
tool_plan = "minimal_safe"

# Behavior loop 模式（XML <actions> 协议）或传统 Agent loop（provider tool_calls）。
# 默认 "behavior"，常规 OpenDAN agent 用这个。
mode = "behavior"

# behavior 模式下的 parser / renderer。当前仅支持 "xml"。
parser   = "xml"
renderer = "xml"
parser_strict = false

# 渲染调参：影响 step 历史压缩。
[renderer_cfg]
recent_full_steps = 3       # 最近 N 个 step 保持完整
summary_chars     = 800     # 老 step 压缩到 N 字符以内
max_result_chars  = 2000    # 单个 tool result 最长 N 字符

# 输出协议：纯文本或结构化 JSON。
[output]
type   = "text"             # "text" 或 "json"
# 当 type = "json" 时可加：
# schema = { ... }
# strict = true

# 安全围栏。
max_rounds              = 32   # 单次 behavior loop 最多 N 轮
max_consecutive_errors  = 3    # 连续错误 ≥ N 终止

# behavior 切换语义：normal | fork | independent。MVP 仅 normal / independent 生效。
switch_mode = "normal"

# 模型选择（LlmClient 适配层用）。
[model]
preferred              = "claude-sonnet-4-6"
fallbacks              = ["claude-opus-4-7"]
temperature            = 0.2
max_completion_tokens  = 4096
# provider_options     = { ... }  # 厂商自定义参数

# 可选：硬预算。超出预算 behavior loop 直接 Stop。
[budget]
max_total_tokens      = 200_000
max_completion_tokens = 50_000
max_wallclock_ms      = 600_000
max_cost_units        = 100
```

### 4.2 写 behavior 的几条经验法则

- **一个 behavior = 一种工作模式**：`resolve_router`（决定路由）、`plan`（规划）、`execute`（执行）、`chatonly`（纯聊天）是典型分法。不要把"所有事"塞到一个 behavior 里。
- **`tool_whitelist` 不是装饰**：白名单越窄，LLM 越稳。chatonly 类 behavior 应只放 `report`；plan 类 behavior 应只放 `write_file`（写 plan 文件）+ `report`；execute 类 behavior 才放 `exec_bash`。
- **`approval_required` 用来挡 dangerous Action**：任何会写远程系统、改 zone 状态、花钱的 Action 都该列进来。
- **`tool_plan` 用来挡 bash 路径**：`tool_whitelist` 挡的是"LLM 看得见什么"，`tool_plan` 挡的是"LLM 跑 `exec_bash` 时 shell 真的能执行什么"。两者互补，缺一会留漏洞。详见 [Agent RootFS §3.4](Agent%20RootFS.md)。
- **`max_rounds` / `max_consecutive_errors` 不要拉太高**：跑飞的 behavior loop 是最常见的预算事故源。32 / 3 是合理起点。
- **system prompt 用 `__INCLUDE(role.md)__` 复用**：不要把 role.md 内容复制粘贴到每个 behavior 里。

### 4.3 模板变量

`system_prompt_template` 走 [Agent Prompt Compiler](Agent%20Prompt%20Compiler.md) 的 `PromptRenderEngine`，支持：

- `__INCLUDE(rel/path.md)__` — 拼入 agent_root 下的文件
- `__OPENDAN_CONTENT(<key>)__` — 拼入运行期内容块（`session_skills` / `session_environment` / `session_worklog` / ...）
- `__OPENDAN_VAR(<key>[, $expr])__` — 注入运行期变量
- `__ENV($key)__` — 注入静态键值
- `{{ name }}` — upon 模板变量、条件、循环

完整列表见 [Render_Prompt_Template_Variables.md](Render_Prompt_Template_Variables.md)。

---

## 5. `tool_plans/<plan>.toml` — 工具策略

可选。和 behavior 是"白名单 vs 黑名单"的互补关系，详见 [Agent RootFS §3.4](Agent%20RootFS.md)。

```toml
# tool_plans/minimal_safe.toml
mode = "deny"

[[deny]]
name   = "rm"
reason = "use trash-cli instead"

[[deny]]
name   = "curl"
reason = "use authenticated http via fetch_url Action"
```

`mode = "allow"` 时换 `[[allow]]`，未列的工具一律墓碑。

---

## 6. `tools/` — Agent 自带脚本工具

Agent Bin 层。详见 [Agent RootFS §3](Agent%20RootFS.md)。两种声明形态：

**扁平**（一文件交付，靠 docblock 推断元数据）：

```bash
# tools/dedup_csv.py
#!/usr/bin/env python3
# @description: dedup rows in a CSV by first column
# @input_schema: {"path": {"type": "string"}}
import sys, csv
...
```

**结构化**（多文件 / 显式 schema）：

```text
tools/summarize_pdf/
  tool.toml
  summarize.sh
  prompts/sys.md
```

**写脚本工具的要点**：

- **只放文本**，不放二进制（启动器按 magic bytes 拒收）。
- **单文件 ≤ 64 KB**，整个 `tools/` 文件数 ≤ 几百。
- **shebang 必须正确**：跨平台的 Linux 容器里 `/usr/bin/env python3`、`/usr/bin/env bash` 是常态。
- **stderr 上写人类可读 + JSON 双行**：和 tombstone stub 同风格，便于 LLM 解析失败原因。
- **退出码语义化**：`0` 成功、`127` 不存在/被策略挡、`2` 用法错误、其它表示业务失败。

---

## 7. `skills/<skill>/` — 默认 skill 包

Skill 是**只读的提示词片段**，不是工具；它通过 `__OPENDAN_CONTENT(session_skills)__` 注入到 system prompt。详见 [Agent Skill](Agent%20Skill.md)。

每个 skill 最少两个文件：

```text
skills/<category>/<skill_name>/
  meta.json         # 出现在 session_skill_list 里的简介
  skill.md          # 完整 skill 内容（被 session_skills 拼进 prompt）
```

例：

```json
// skills/planner/meta.json
{
  "name": "planner",
  "summary": "Use todo CLI to initialize the session todo list during planning."
}
```

```markdown
<!-- skills/planner/skill.md -->
Use the session todo CLI to initialize the todo list during the planning phase.

- Build the todo list in one pass when the task scope is already clear.
- Prefer multiple `todo add` commands to define the full initial plan.
...
```

加载顺序：

1. **包默认 skills**：放在 `<agent_package_root>/skills/`，启动时同步到 `<agent_root>/skills/`。
2. **behavior 级 skills**：behavior 配置里通过 `load_skills` 字段加载（行为切换时同步切换）。
3. **session 级 skills**：运行期通过 `load_skill <name> session` Action 加载。

三者求并集进 `session_skills`（详细规则见 Agent Skill）。

---

## 8. `users/` — 针对调用者的 prompt 片段

可选。文件命名按 `from_did` / `group_<gid>` 选择，运行时按 sender 自动挑选并拼入 system prompt。用于"对老板说话和对客户说话语气不同"这类场景。

```text
users/
  did:web:alice.example.md       # alice 调用本 agent 时附加的提示
  group_team_finance.md          # finance 群里发消息时附加的提示
```

---

## 9. 一个最小可发布 Agent 的例子

```text
hello_agent/
  agent.toml
  role.md
  self.md
  behaviors/
    ui_default.toml
```

`agent.toml`:

```toml
display_name = "Hello"
default_ui_behavior = "ui_default"
subscribe_events = ["msg.incoming"]
```

`role.md`:

```markdown
You are Hello, a friendly demo agent. You answer in one sentence,
in the same language the user used.
```

`self.md`: 空文件。

`behaviors/ui_default.toml`:

```toml
name      = "ui_default"
objective = "minimal demo"

system_prompt_template = """
__INCLUDE(role.md)__
"""

tool_whitelist = ["report"]
mode = "behavior"
max_rounds = 4

[model]
preferred = "claude-sonnet-4-6"
```

把这四个文件打包成 BuckyOS app package，安装到任一 Zone，`opendan` 启动后就能跑。

---

## 10. 开发循环

### 10.1 本地起一个隔离的 agent_root

不需要跑完整 BuckyOS。直接给 `opendan` 一个临时目录当 `agent_root`：

```bash
mkdir -p /tmp/hello_root
cp -r ./hello_agent/* /tmp/hello_root/

OPENDAN_AGENT_ROOT=/tmp/hello_root \
  cargo run -p opendan -- \
    --agent-root /tmp/hello_root \
    --appid hello \
    --owner-id did:dev:local
```

`agent_did` / msg_center / kevent / task_mgr 三个边界客户端可以为 None（[ai_runtime.rs](../../src/frame/opendan/src/ai_runtime.rs) 的"只接受 `submit_text` 注入"模式），CLI 单测一样跑得起来。

### 10.2 用 CLI 注入消息

`opendan` CLI（或 `agent_tool_cli_dev`）暴露了 `submit_text` 入口，可以直接把用户输入塞进 UI session 看 behavior loop 的产出。每一轮的 `<actions>` XML、tool 结果都会写进 worklog DB，方便 diff prompt 和回放。

### 10.3 改了 behavior toml 怎么办

- **运行中改**：当前实现下 `BehaviorCfg` 在每次进入该 behavior 时重新加载，所以下一次 `switch_behavior` 进入它时就生效。**不需要重启进程**。
- **改了 `role.md` / `self.md`**：影响 system prompt，下一次 turn 重新渲染时生效。
- **改了 `tools/` 内的脚本**：Agent Bin 层会在下一次 `exec_bash` 起手时按 mtime 触发 re-render（见 [Agent RootFS §3.5](Agent%20RootFS.md)）。

### 10.4 调试要看的地方

| 看什么                                | 在哪里                                                            |
| ------------------------------------- | ----------------------------------------------------------------- |
| 系统 prompt 实际渲染产物              | session worklog 里每一轮的 `prompt` 字段                          |
| LLM 返回的 `<actions>` 原文           | session worklog 里每一轮的 `assistant_raw` 字段                   |
| Action 执行结果 / Observation         | session worklog `observations` 字段                               |
| 工具策略合成结果                      | `<agent_root>/sessions/<sid>/tool_plan.resolved.toml`             |
| Session bin 执行视图                  | `<buckyos_root>/tools/<agent_id>/<session_id>/`                   |
| Behavior loop 当前 step 历史          | `state.snap` / `behavior_<name>.snap`                             |

---

## 11. 打包与发布

> 这一节描述当前形态。后续 BuckyOS App Store 接入还会迭代。

### 11.1 包的形态

Agent 包就是一个目录树，按 §1 骨架组织，**没有额外的 manifest 文件**——`agent.toml` 同时是 BuckyOS 视角下的 app metadata 源。BuckyOS 安装器把包目录作为 app 的 `bin_dir`，启动时通过 `--agent-bin <package_root>` 传给 `opendan`，由 `sync_agent_rootfs_from_package` 同步进 `<agent_root>`。

### 11.2 同步语义（升级保护）

`sync_agent_rootfs_from_package` 是**有状态的同步**，不是简单覆盖：

- 每个文件记录两个 hash 到 `.meta/rootfs_sync.json`：
  - `source_sha256`：当前包里的文件 hash
  - `installed_sha256`：上次成功同步时落到 root 的 hash
- 同步策略：
  - **首次出现**：直接 copy
  - **本地未改**（local hash == installed_sha256）：覆盖为新版
  - **本地改过**（local hash != installed_sha256）：保留本地，log warn
- 用户/Agent 自己写过的 `role.md` / `self.md` / `behaviors/*.toml` 在升级时**不会被覆盖**——这是设计目标。

发布新版的开发者要意识到：**用户改过的文件你升级不动**，所以新增能力优先以"新文件 / 新 behavior"形态交付，不要修改已发布过的关键文件的语义。

### 11.3 发布前的自检清单

- [ ] `agent.toml` 留空 `agent_did`（由 zone 运行时回填）
- [ ] `display_name` 是人类可读的产品名
- [ ] `default_ui_behavior` / `default_work_behavior` 指向的 `.toml` 真的存在
- [ ] `subscribe_events` 只订阅必要事件（事件订阅会推动 session 唤醒，多余订阅 = 多余 wakeup）
- [ ] 每个 behavior 的 `tool_whitelist` 都已收敛，没有"全开"
- [ ] dangerous Action 都列进了 `approval_required`，或者被 `tool_plan` deny 掉
- [ ] `role.md` 不含敏感凭据 / Owner 私人信息
- [ ] `tools/` 下没有二进制、没有大文件、shebang 正确
- [ ] `skills/` 下每个 skill 都有 `meta.json` + `skill.md`
- [ ] 至少跑过一次本地 `agent_root` 模式，确认四个核心 behavior（chat / plan / execute / report）的 happy path

---

## 12. 与其它文档的关系

- 数据布局 / 4 层 Bin / Session bin 渲染：[Agent RootFS](Agent%20RootFS.md)
- Session 状态机 / pending_inputs / behavior 切换语义：[Agent Session](Agent%20Session.md)
- 提示词模板指令完整列表：[Render_Prompt_Template_Variables.md](Render_Prompt_Template_Variables.md)
- v2 Action 协议（`<actions>` / `<report>` 等固化 Action）：[Agent Actions](Agent%20Actions.md)
- skill 的注入语义：[Agent Skill](Agent%20Skill.md)
- 提示词整体编排理念：[Agent Prompt Compiler](Agent%20Prompt%20Compiler.md)

实现入口：

- `agent.toml` 解析：[agent_config.rs](../../src/frame/opendan/src/agent_config.rs)
- behavior toml 解析：[behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs)
- 启动 / 同步：[main.rs](../../src/frame/opendan/src/main.rs) 的 `ensure_agent_rootfs_layout` / `sync_agent_rootfs_from_package`
- 工具策略：[tool_plan.rs](../../src/frame/opendan/src/tool_plan.rs)
