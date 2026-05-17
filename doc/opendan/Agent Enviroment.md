# AgentSession Prompt Environment

OpenDAN `AgentSession` 把会话 / behavior / workspace / 运行时状态映射成 `llm_context::PromptRenderEngine` 可消费的变量。本文是这一映射的**实现手册** —— 列出 behavior 模板今天可以直接写的变量、模板引擎接的是哪一套语法、include 边界在哪里。

> 文件名中的 `Enviroment` 是历史拼写，语义上是 **AgentSession Prompt Environment**。

实现集中在 [`src/frame/opendan/src/prompt_env.rs`](../../src/frame/opendan/src/prompt_env.rs) 和 [`src/frame/opendan/src/agent_session.rs`](../../src/frame/opendan/src/agent_session.rs) 的 `render_system_messages` / `compose_environment_message` / `build_prompt_env`。

## 1. 渲染管线

```
behavior.prompt.on_init  ─┐
                          ├─▶ PromptRenderEngine.render() ─▶ System message
{{ role_md / self_md }}  ─┘     (upon + __VAR__/__ENV__/__INCLUDE__)

ENVIRONMENT_BLOCK_TEMPLATE ─▶ PromptRenderEngine.render() ─▶ env block
                                                            (wrapped in <background_environment>
                                                             by compose_turn_message)
```

- 渲染时机：在 `AgentSession::build_or_resume` 把 turn 真正打包送 `llm_context` **之前**完成。不在 behavior 加载时预渲染。
- 每次渲染独立：每个 turn 拿一份 `AgentSessionEnv` 快照（构造时一次性 `meta.lock()` 读出所有字段），然后用快照新建 `PromptRenderEngine` + `AgentSessionValueLoader` + `RenderVars`。没有跨 turn 复用的可变状态。
- 失败兜底：
  - `render_system_messages` 渲染失败 → `log::warn` + 回退到内置组合（`role.md` + `self.md` + objective + readme）。
  - `compose_environment_message` 渲染失败 → `log::warn` + 返回 `None`（该 turn 不附带 env block）。

## 2. 模板语法

模板语法由 `llm_context::PromptRenderEngine` 决定，不是 OpenDAN 自家方言：

| 形式 | 来源 | 说明 |
| --- | --- | --- |
| `{{ session.id }}` / `{% if workspace.has_id %}{% endif %}` | upon 0.10 | 普通占位 + 控制结构。聚合对象已 seed 到 `RenderVars.vars`，可直接 `{{ name.field }}`。 |
| `__ENV($expr)__` | PromptRenderEngine | 静态 `RenderVars.env` 优先，未命中走 `ValueLoader`。 |
| `__VAR(name, $expr)__` | PromptRenderEngine | 异步 loader 解析 `$expr` 并注册为 upon 变量 `name`。`{{ session.id }}` 这种"裸名形如已 seed 变量"会被预处理自动补一条 `__VAR__` 声明。 |
| `__INCLUDE(/abs/path)__` | PromptRenderEngine | 把文件内容内联（受 `include_roots` 白名单约束）。 |
| `__EXEC(cmd)__` | PromptRenderEngine | 默认 **关闭**（`allow_exec = false`），OpenDAN 不打开。 |
| `\{{ … \}}` | PromptRenderEngine | 字面双花括号转义。 |

OpenDAN 旧的单花括号 `{name}` 替换**已删除**，behavior 作者只学一套语法。

## 3. 变量参考

下表是 `AgentSessionValueLoader::load` / `build_render_vars` 当前实际暴露的全部变量。表达式列对应 loader 的输入；upon 占位列对应模板里能直接写的形式。

### 3.1 `session`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$session` | `{{ session }}` | object | 下列字段的聚合 |
| `$session.id` | `{{ session.id }}` | string | `SessionMeta.session_id` |
| `$session.kind` | `{{ session.kind }}` | string | `"ui"` / `"work"` |
| `$session.title` | `{{ session.title }}` | string | `SessionMeta.title.trim()`（空串即"无标题"） |
| `$session.objective` | `{{ session.objective }}` | string | `SessionMeta.objective`；为空时回退到 `behavior.meta.objective` |
| `$session.owner` | `{{ session.owner }}` | string | `SessionMeta.owner` |
| `$session.has_title` | `{{ session.has_title }}` | bool | `!session_title.is_empty()` |

### 3.2 `behavior`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$behavior` | `{{ behavior }}` | object | 下列字段的聚合 |
| `$behavior.name` | `{{ behavior.name }}` | string | `BehaviorCfg.meta.name` |
| `$behavior.objective` | `{{ behavior.objective }}` | string | `BehaviorCfg.meta.objective` |
| `$behavior.mode` | `{{ behavior.mode }}` | string | 恒为 `"behavior"`（占位，后续区分 `agent` 时启用） |

### 3.3 `workspace`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$workspace` | `{{ workspace }}` | object | 下列字段的聚合 |
| `$workspace.id` | `{{ workspace.id }}` | string | `SessionMeta.workspace_id`（未绑定 → 空串） |
| `$workspace.root` | `{{ workspace.root }}` | string | `agent_config.layout.workspaces_dir / workspace_id`（未绑定 → 空串） |
| `$workspace.has_id` | `{{ workspace.has_id }}` | bool | workspace_id 是否非空 |

### 3.4 `paths`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$paths` | `{{ paths }}` | object | 下列字段的聚合 |
| `$paths.agent_root` | `{{ paths.agent_root }}` | string | `agent_config.layout.root` |
| `$paths.session_root` | `{{ paths.session_root }}` | string | `AgentSession.session_dir` |
| `$paths.workspace_root` | `{{ paths.workspace_root }}` | string | 同 `$workspace.root` |

这些路径主要用作 `__INCLUDE__` 的拼接锚点。直接把绝对路径塞进模型上下文一般不必要，能避免就避免。

### 3.5 `input`

| 表达式 | upon 占位 | 类型 | 备注 |
| --- | --- | --- | --- |
| `$input` | `{{ input }}` | object | 下列字段的聚合 |
| `$input.text` | `{{ input.text }}` | string | **当前固定为空串**（见 §6） |
| `$input.has_user_text` | `{{ input.has_user_text }}` | bool | **当前固定为 `false`** |
| `$input.has_events` | `{{ input.has_events }}` | bool | **当前固定为 `false`** |

`$input.*` 的契约已确定，但 `build_prompt_env` 目前不注入真实值 —— `user.input` section 仍由 [`compose_human_text`](../../src/frame/opendan/src/agent_session.rs) 直接构造，不经过模板。在模板里引用 `$input.*` 会得到空串 / false。

### 3.6 `runtime`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$runtime` | `{{ runtime }}` | object | 下列字段的聚合 |
| `$runtime.clock_unix_ms` | `{{ runtime.clock_unix_ms }}` | number | `SystemTime::now().duration_since(UNIX_EPOCH).as_millis()` |
| `$runtime.recent_activity` | `{{ runtime.recent_activity }}` | string | `OneLineStatusSink` 当前值（trim 后） |
| `$runtime.has_activity` | `{{ runtime.has_activity }}` | bool | `!recent_activity.is_empty()` |

## 4. Render-time extras

`render_system_messages` 在调用 `prompt_env::render_template` 时会额外注入两个 extras，仅对 `on_init` 可见：

| 占位 | 来源 |
| --- | --- |
| `{{ role_md }}` | `agent_root/role.md` 文件内容（`unwrap_or_default()`） |
| `{{ self_md }}` | `agent_root/self.md` 文件内容（`unwrap_or_default()`） |

extras 通过 `render_template(template, env, extras)` 的第三个参数传入，覆盖同名 Phase-1 变量。这是给 `on_init` 路径专用的桥 —— 后续把这两个值改成 `__INCLUDE($paths.agent_root/role.md)__` 时这层 extras 会消失。

## 5. 环境块（`ENVIRONMENT_BLOCK_TEMPLATE`）

`compose_environment_message` 渲染常量 [`ENVIRONMENT_BLOCK_TEMPLATE`](../../src/frame/opendan/src/prompt_env.rs)：

```upon
behavior: `{{ behavior.name }}`
session: `{{ session.id }}`{% if session.has_title %} ("{{ session.title }}"){% endif %}{% if workspace.has_id %}
workspace: `{{ workspace.id }}`{% endif %}{% if runtime.has_activity %}
recent activity: {{ runtime.recent_activity }}{% endif %}
clock: unix_ms={{ runtime.clock_unix_ms }}
```

输出形如：

```
behavior: `chat_route`
session: `ui_xxx` ("hello")
workspace: `ws1`
recent activity: tool running
clock: unix_ms=1747400000000
```

`compose_turn_message` 接着把它包成 `<background_environment>…</background_environment>` 再 prepend 到 user message。可选行（title / workspace / recent_activity）由 `has_*` 布尔字段控制，与历史手写版本字节级等价。

## 6. EngineConfig 与安全边界

`build_engine_config` 当前的取值：

| 字段 | 当前值 | 说明 |
| --- | --- | --- |
| `include_roots` | `[agent_root, session_root]` + `workspace_root`（若绑定） | `__INCLUDE__` 的白名单。memory / notepads / skills / tools 目录**不在**白名单。 |
| `allow_exec` | `false` | `__EXEC__` 全程关闭。 |
| `max_include_bytes` | 64 KiB | 单次 `__INCLUDE__` 字节上限（`EngineConfig::default()`） |
| `max_total_bytes` | 256 KiB | 渲染输出上限，超出标记 `truncated=true`（`EngineConfig::default()`） |
| `max_recursion_depth` | 8 | `__INCLUDE__` 嵌套深度上限（`EngineConfig::default()`） |
| `exec_timeout` | 10s | 仅在 `allow_exec=true` 时生效，当前无影响 |

边界注意：

- `__INCLUDE__` 解析为绝对路径，且必须在某个 `include_root` 之下，否则记一次 `content_failed` + 在输出里留下 `<!-- __INCLUDE__ ... -->` 标记。
- loader 返回 `Ok(None)` 表示"不认识这个名字"，引擎计 1 次 soft miss，不报错。返回 `Err(_)` 才是硬失败（当前 OpenDAN loader 不会返 `Err`）。

## 7. 已规划但未实现

下列变量在历史设计中提到过，但 [`resolve_phase1`](../../src/frame/opendan/src/prompt_env.rs) 当前没有它们的分支。在模板里引用会得到 `None`（loader miss）/ 上游字段缺失：

- `session`: `peer_did`、`peer_tunnel_did`、`status`、`bootstrap_done`
- `behavior`: `parser`、`renderer`、`switch_mode`、`max_rounds`、`tool_plan`
- `process`: `entry`、`current`、`stack`
- `input`: `messages`、`events`、`interrupts`、`keys` —— `text` / `has_*` 已暴露但取值固定（见 §3.5）
- `workspace`: `summary`、`readme`
- `agent`: 整套（`id`、`name`、`did`、`root`）
- `owner`: 整套（异步查询 contact manager）
- `history`: `recent_messages`、`last_user_message`、`last_assistant_message`、`summary`、`round_summary`
- `steps`: `last`、`recent`、`summary`
- `memory`: `recall`、`session_summary`、`agent_profile`、`user_profile`
- `runtime`: `pending_tool_calls`、`llm_task_ids`

引入新变量需要：
1. 加 `AgentSessionEnv` 字段并在 `build_prompt_env` 填值（异步 IO 字段考虑延迟到 loader 实例方法里，避免每次渲染都付出开销）。
2. 在 `resolve_phase1` 加 match 分支。
3. 视情况扩展 `session_object` / `behavior_object` / … 的 JSON 聚合。
4. 加单元测试。

### 7.1 历史扁平命名

早期设计稿（以及一份已合并进本手册的 `Render_Prompt_Template_Variables.md`）出现过下面这些 `$xxx` 扁平命名。今天的契约统一走"namespace + 点路径"形式，列在这里只为帮助读老 PR / 老设计稿的人对得上号：

| 历史命名 | 现行映射 |
| --- | --- |
| `$session_title` | `$session.title` |
| `$current_behavior` | `$behavior.name` |
| `$step_index` / `$step_num` / `$step_limit` | 未实现；规划属于 `$steps.*` namespace |
| `$last_step` / `$last_steps.$num` | 未实现；规划属于 `$steps.last` / `$steps.recent` |
| `$new_msg` / `$new_msg.$n` | 未实现；规划属于 `$input.messages` |
| `$owner` / `$owner.did` / `$owner.show_name` | 未实现；规划属于 `$owner.*`（异步 contact manager 查询） |
| `$session_list` / `$session_list.$n` | 未实现；不在 prompt env 计划内（worksession 工具自己拼装） |
| `$workspace_list` / `$workspace_list.$n` | 未实现；不在 prompt env 计划内（worksession 工具自己拼装） |
| `$workspace_todolist` 及变体 (`$workspace_current_todo_id` 等) | 未实现；不在当前 prompt env 计划内 |
| `$workspace/<rel_path>` | 不再支持作为变量。引擎只认 `__INCLUDE(<absolute_path>)__`，且路径必须在 `include_roots` 白名单内 |
| `$params` | 未实现；该位置由 behavior cfg 直接覆盖，不进 loader |
| `$llm_result` / `$trace` | 不计划暴露为 prompt 变量 |

## 8. `prompt_compose` / token budget

`llm_context::prompt_compose::compose` 与 `SectionSpec` / `PromptBudgeter` 已就绪，但 `render_system_messages` / `compose_environment_message` 当前**不走** `compose` —— 两个 section 都是小段，直接 `render` 后塞进 `Vec<AiMessage>`。引入 section budget 是后续工作，会改造 `build_or_resume` 把 system / environment / user.input 三段统一交给 `compose`。

## 9. 测试入口

- 单元测试在 [`prompt_env::tests`](../../src/frame/opendan/src/prompt_env.rs) 模块（10 项，覆盖 loader 解析 / 聚合对象 / 环境块三种布局 / extras overlay / include_roots 边界）。
- `cargo test -p opendan --lib prompt_env::` 可单独跑。
- 完整 opendan 单元测试：`cargo test -p opendan --lib`（121 项）。

## 10. 后续阶段（路线图）

按"代价从小到大"排：

1. 接 `SectionSpec` + budget；`user.input` 迁入模板（让 `$input.text` 等真正生效）。
2. 扩 loader：`$workspace.summary` / `$workspace.readme` / `$agent.*` / `$owner.*`，引入异步 IO loader。
3. `$history.*` / `$steps.*`：先做 `last_user_message` / `last_assistant_message` / `round_summary` 三件套并设字符上限。
4. `$memory.*`：与 memory 召回联动，按 section budget 决定是否进 prompt。
5. `$process.*`、`$input.messages` 等结构化字段按 behavior 实际需求逐项放开。
6. 评估是否把 `memory_root` / `notepads_root` / `skills_root` 按 behavior policy 加入 `include_roots`。

每一步都要保持本手册 §3 的契约稳定 —— 新增字段优先，重命名 / 删除需要走 deprecation。

## 11. 与旧设计的差异

历史版本的本文档以"应/建议"风格描述了一个独立的 `agent_environment` 模板引擎（`{{var}}` / `{{workspace/path}}` / `TEXT / INPUT_BLOCK` / Null 跳过推理）。这些设计现在分别由：

- 通用 `PromptRenderEngine`（语法、include 安全）
- `AgentSession::build_or_resume` 的输入驱动逻辑（决定是否需要推理，是否注入 synthetic user message）
- 工具 / 文件权限层（fs scope）
- L4 outcome driver（session 状态机）

承担。模板引擎本身**不**感知 session、policy、subagent。本手册描述的是 OpenDAN 这一侧给通用引擎填的"变量适配器"，不是新引擎。
