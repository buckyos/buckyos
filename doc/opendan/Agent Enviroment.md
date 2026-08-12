# AgentSession Prompt Environment

OpenDAN `AgentSession` 把会话、workspace、pending input、行为上下文和运行时状态映射成 `llm_context::PromptRenderEngine` 可消费的变量。本文是 Behavior 开发人员查看可用模板变量的主参考。

如果只是在写 `behaviors/*.toml`，优先看本文。另一个 [`Render_Prompt_Template_Variables.md`](Render_Prompt_Template_Variables.md) 是模板引擎自身的实现参考，面向修改 `llm_context::PromptRenderEngine` 的开发者，不列 OpenDAN 业务变量契约。

> 文件名中的 `Enviroment` 是历史拼写，语义上是 **AgentSession Prompt Environment**。

实现集中在 [`src/frame/opendan/src/prompt_env.rs`](../../src/frame/opendan/src/prompt_env.rs) 和 [`src/frame/opendan/src/agent_session.rs`](../../src/frame/opendan/src/agent_session.rs)。

## 1. 渲染管线

Behavior 模板不是在加载 behavior 时预渲染，而是在 Session worker 的 hook point 上用一份冻结的环境快照渲染。

```
Session driver hook
  -> freeze AgentSessionEnv
  -> PromptRenderEngine.render()
  -> AiMessage
  -> LLMContext
```

当前有四类重要 hook point：

| Hook point | 触发边界 | 典型用途 |
| --- | --- | --- |
| `on_init` | session / context 初始化 | 渲染 system prompt |
| `on_behavior_switch` | behavior switch / fork / independent 切换 | 给新 behavior 构造入口 user message |
| `on_behavior_step_ob` | Behavior Loop step 观察阶段 | 在 step 边界观察追加输入或事件 |
| `on_wakeup` | idle / waiting 状态下被 pending input 唤醒 | UI / Work session 被输入唤醒后的下一轮驱动 |

背景环境块也分两层配置：

| 配置点 | 含义 |
| --- | --- |
| 触发边界 | 哪个 hook point 会构造背景环境 |
| 内容模板 | 触发后向 LLM 提供哪些 session / workspace / runtime 信息 |

## 2. 模板语法

模板语法由 `llm_context::PromptRenderEngine` 决定，不是 OpenDAN 自家方言。Behavior 作者只需要使用 upon 语法和 `PromptRenderEngine` 指令。

| 形式 | 用途 |
| --- | --- |
| `{{ session.id }}` | 输出一个标量字段 |
| `{% if workspace.has_id %}...{% endif %}` | 条件判断 |
| `{% for event in input.events %}...{% endfor %}` | 遍历 list |
| `__VAR(name, $expr)__` | 通过 loader 解析 `$expr` 并注册为模板变量 |
| `__INCLUDE(/abs/path)__` | 内联文件内容，受 `include_roots` 白名单约束 |
| `__ENV($expr)__` | 读取静态 env / loader 值，主要给底层模板引擎使用 |
| `__EXEC(cmd)__` | 默认关闭，OpenDAN 不打开 |
| `\{{ ... \}}` | 输出字面双花括号 |

常见写法：

```upon
{% if session.has_title %}
Session title: {{ session.title }}
{% endif %}

{% for event in input.events %}
- event_id: {{ event.event_id }}
{% if event.data.reason %}  reason: {{ event.data.reason }}{% endif %}
{% endfor %}
```

常见类型：

| 类型 | 模板里怎么用 | 注意事项 |
| --- | --- | --- |
| `string` / `number` / `bool` | 可以直接 `{{ value }}` 输出 | 字符串为空时通常配合 `has_*` 判断 |
| `object` | 用点路径访问字段，如 `{{ session.id }}` | 不要直接输出整个 object |
| `array` | 用 `{% for item in items %}` 遍历，或 `items.0` 取第一项 | 不要直接输出整个 array |
| `null` | 用 `{% if value %}` 判断存在性 | 直接输出通常为空或失败 |

数组 / object 不能直接当字符串输出。`{{ input.events }}`、`{{ input.bg_events }}`、`{{ current_context.step_history }}` 这类写法会在 upon 格式化阶段失败；模板应访问标量字段、循环展开，或使用后续提供的 `*_json` 辅助字段。

## 3. 变量参考

### 3.1 `session`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$session` | `{{ session }}` | object | 下列字段的聚合 |
| `$session.id` | `{{ session.id }}` | string | `SessionMeta.session_id` |
| `$session.kind` | `{{ session.kind }}` | string | `"ui"` / `"work"` / `"self_check"` / `"self_improve"` |
| `$session.title` | `{{ session.title }}` | string | `SessionMeta.title.trim()` |
| `$session.objective` | `{{ session.objective }}` | string | `SessionMeta.objective`；为空时回退到当前 behavior objective |
| `$session.owner` | `{{ session.owner }}` | string | `SessionMeta.owner` |
| `$session.current_behavior` | `{{ session.current_behavior }}` | string | 当前 behavior name |
| `$session.current_todo` | `{{ session.current_todo.todo_id }}` | object / null | `todos.json` 中第一个非终态 Todo |
| `$session.current_todo_list` | `{{ session.current_todo_list }}` | string | `todos.json` 的简表；缺失时为 `(empty)` |
| `$session.background_hint_changed` | `{{ session.background_hint_changed }}` | bool | 本 hook 是否通过 `load_background_hits` 加载到变化的背景 hint |
| `$session.default_changed_background_hint_text` | `{{ session.default_changed_background_hint_text }}` | string | `session.background_hints` 的默认纯文本渲染，格式为 `- hint text` 列表 |
| `$session.default_changed_backgrand_hint_text` | `{{ session.default_changed_backgrand_hint_text }}` | string | 上一项的历史拼写别名 |
| `$session.background_hints` | `{% for hint in session.background_hints %}` | array of `BackgroundHint` | 本 hook 新加载到的变化背景 hint；没有配置加载或无变化时为空数组 |
| `$session.has_title` | `{{ session.has_title }}` | bool | title 是否非空 |
| `$session.has_current_todo` | `{{ session.has_current_todo }}` | bool | 当前 session 是否存在非终态 Todo |

`BackgroundHint`：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `path` | string | hint 的稳定路径，如 `event/presence.changed`、`notepad/<id>`、`memory/<key>` |
| `kind` | string | `event` / `notepad` / `memory` |
| `text` | string | 给 LLM 阅读的简短提示文本 |
| `fingerprint` | string | 用于判断本次和上次加载相比是否变化 |
| `data` | object / null | 结构化原始数据；模板通常不需要直接展开 |

### 3.2 `workspace`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$workspace` | `{{ workspace }}` | object | 下列字段的聚合 |
| `$workspace.id` | `{{ workspace.id }}` | string | `SessionMeta.workspace_id`，未绑定为空串 |
| `$workspace.root` | `{{ workspace.root }}` | string | `agent_config.layout.workspaces_dir / workspace_id`，未绑定为空串 |
| `$workspace.has_id` | `{{ workspace.has_id }}` | bool | workspace id 是否非空 |

### 3.3 `paths`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$paths` | `{{ paths }}` | object | 下列字段的聚合 |
| `$paths.agent_root` | `{{ paths.agent_root }}` | string | agent root |
| `$paths.session_root` | `{{ paths.session_root }}` | string | 当前 session 目录 |
| `$paths.workspace_root` | `{{ paths.workspace_root }}` | string | 同 `$workspace.root` |

这些路径主要用作 `__INCLUDE__` 的拼接锚点。直接把绝对路径塞进模型上下文一般不必要。

### 3.4 `input`

`input` 表示本 hook point 从 pending input 中消费或观察到的 msg / event。`pull_msg` / `pull_event` 只影响 `input`，不影响 `current_context`。

当 driver 只消费单条记录时，模板不必每次写循环：`input.msg` 是 `input.msgs.0` 的快捷入口，`input.event` 是 `input.events.0` 的快捷入口。没有对应记录时为 `null`。

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$input` | `{{ input }}` | object | 下列字段的聚合 |
| `$input.text` | `{{ input.text }}` | string | 本次消息输入合并文本；没有消息时为空串 |
| `$input.msg` | `{{ input.msg.text }}` | `MsgRef` / null | `input.msgs.0` 的快捷入口 |
| `$input.msgs` | `{% for msg in input.msgs %}` | array of `MsgRef` | `pull_msg` 拉出的 message |
| `$input.event` | `{{ input.event.event_id }}` | `EventRef` / null | `input.events.0` 的快捷入口 |
| `$input.events` | `{% for event in input.events %}` | array of `EventRef` | `pull_event` 拉出的 event |
| `$input.bg_events` | `{% for event in input.bg_events %}` | array of `BgEventSnapshot` | 半订阅事件快照，不消费 pending queue |
| `$input.has_user_text` | `{{ input.has_user_text }}` | bool | 是否有用户文本 |
| `$input.has_msgs` | `{{ input.has_msgs }}` | bool | `input.msgs` 是否非空 |
| `$input.has_events` | `{{ input.has_events }}` | bool | `input.events` 是否非空 |
| `$input.has_bg_events` | `{{ input.has_bg_events }}` | bool | `input.bg_events` 是否非空 |

`MsgRef`：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `record_id` | string | message record id |
| `from` | string | 展示用发送者名称；应优先由 contact manager / channel adapter 归一化 |
| `from_did` | string / null | 发送者 DID |
| `tunnel_did` | string / null | 消息所在 tunnel / channel DID |
| `created_at_ms` | number / null | 消息创建 / 发送时间；没有上游时间时为空 |
| `received_at_ms` | number | AgentSession 接收并冻结该输入的时间 |
| `raw_text` | string / null | channel 入口解析出的原始纯文本；没有纯文本时为空 |
| `text` | string | 系统从 `content` 渲染出的默认纯文本视图，包含标准 attachment marker |
| `content` | array of `MsgContentBlock` | 结构化消息内容块，转自 `AiMessage.content` |
| `attachments` | array of `AttachmentRef` | `content` 中图片、文档等非文本附件块的快捷索引 |

`raw_text`、`text` 和 `content` 的区别：

| 字段 | 适合场景 |
| --- | --- |
| `raw_text` | 需要还原 channel 入口的用户原话 |
| `text` | 普通 Behavior 模板默认使用；它是 `content` 的系统默认文本渲染，附件会以标准 marker 出现 |
| `content` | 需要精确处理多模态块、附件、结构化 payload 或 runtime auto message |

系统会为复杂结构提供默认文本渲染，命名统一使用 `default_*_text` 或直接使用对象的 `text` 字段。`input.msg.text` 是 `MsgRef.content` 的默认文本视图：文本块按顺序输出，附件块以 `AttachmentRef.text_marker` 形式输出。模板只需要自然语言输入时优先使用 `input.msg.text` / `input.text`；需要精确判断图片、文档或 object id 时再读 `input.msg.content` / `input.msg.attachments`。

附件不应只靠普通字符串表示。当前 MessageHub 入站模型中，文本在 `MsgContent.content`，附件在 `MsgContent.refs`；降到 LLMContext 后会成为 `AiContent::Image` / `AiContent::Document`。`AiMessage::text_content()` 只返回文本块，会跳过图片和文档。因此 `MsgRef.text` 必须由 OpenDAN 自己渲染，并保留结构化 `content` / `attachments`。

`MsgContentBlock`：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `type` | string | `text` / `image` / `document` / `audio` / `video` / `file` / `machine` |
| `text` | string / null | `type = "text"` 时的文本 |
| `attachment` | `AttachmentRef` / null | 附件类 block 的结构化引用 |
| `machine` | object / null | 机器可读 payload |

`AttachmentRef`：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `kind` | string | `image` / `document` / `audio` / `video` / `file` |
| `source.type` | string | `named_object` / `url` / `base64` |
| `source.obj_id` | string / null | `source.type = "named_object"` 时的 CYFS object id |
| `source.url` | string / null | `source.type = "url"` 时的 URL |
| `mime` | string / null | MIME hint，例如 `image/png` |
| `title` | string / null | 文件标题或展示名 |
| `label` | string / null | 上游 ref label |
| `text_marker` | string | 给纯文本 prompt 使用的稳定占位，例如 `[image: screenshot.png]` |

图片消息示例：

```json
{
  "record_id": "msg-1",
  "from": "Alice",
  "from_did": "did:example:alice",
  "raw_text": "看看这个截图",
  "text": "看看这个截图\n[image: screenshot.png]",
  "content": [
    {
      "type": "text",
      "text": "看看这个截图"
    },
    {
      "type": "image",
      "attachment": {
        "kind": "image",
        "source": {
          "type": "named_object",
          "obj_id": "file:010203"
        },
        "mime": "image/png",
        "title": "screenshot.png",
        "label": "screenshot.png",
        "text_marker": "[image: screenshot.png]"
      }
    }
  ],
  "attachments": [
    {
      "kind": "image",
      "source": {
        "type": "named_object",
        "obj_id": "file:010203"
      },
      "mime": "image/png",
      "title": "screenshot.png",
      "label": "screenshot.png",
      "text_marker": "[image: screenshot.png]"
    }
  ]
}
```

模板中建议这样使用：

```upon
{{ input.msg.text }}
{% for attachment in input.msg.attachments %}
- attachment: {{ attachment.kind }} {{ attachment.title }}
{% endfor %}
```

`EventRef`：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `event_id` | string | 事件名 / 事件路径，例如 `timer.reminder_check` 或 `/task_mgr/<id>/status` |
| `data` | object / null | 事件 payload |
| `reason` | string / null | 订阅或调度方给出的可读原因；session 主动订阅事件时通常应填写 |
| `observed_at_ms` | number | AgentSession 观察到事件并冻结输入的时间 |

`event_id` 命名约定：

| 形态 | 用途 |
| --- | --- |
| `timer.reminder_check` | 本地逻辑事件，通常不以 `/` 开头，适合 driver filter 命名空间 |
| `/task_mgr/...` / `/msg_center/...` | KEvent path，保留 `/` 开头，语义参考 kevent event path 设计 |

`BgEventSnapshot`：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `event_id` | string | 事件名 |
| `data` | object / null | 最新快照 payload |
| `reason` | string / null | 订阅方给出的可读原因 |
| `observed_at_ms` | number | 观察时间 |

Driver policy 对 `input` 的影响：

| policy | 影响 |
| --- | --- |
| `pull_msg = "none"` | `input.msgs = []`，`input.msg = null`，`input.text = ""` |
| `pull_msg = "one"` | `input.msgs` 最多一条；有消息时 `input.msg` 和 `input.text` 可直接使用 |
| `pull_msg = "all"` | `input.msgs` 包含本 hook 可消费的所有 message；`input.msg` 指第一条，`input.text` 是合并文本 |
| `pull_event = "none"` | `input.events = []`，`input.event = null` |
| `pull_event = "all"` | `input.events` 包含本 hook 可消费的所有 event；`input.event` 指第一条 |
| `pull_event = "<filter_name>"` | `input.events` 只包含匹配 filter 的 event，例如 `timer.*`；`input.event` 指第一条匹配事件 |
| `load_background_hits = "none"` | `session.background_hints = []`，`session.background_hint_changed = false` |
| `load_background_hits = "all"` | 调用 `load_changed_background_hits`，加载本 hook 相比上次调用发生变化的背景 hint；若同 session 过去 60 秒内返回过非空结果，本次直接返回空 |

### 3.5 `current_context`

`current_context` 是 Driver 在 hook point 上冻结出来的 LLMContext 运行态摘要。它描述当前 behavior context 本身，不承载 pending input。

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$current_context` | `{{ current_context }}` | `LLMContext` | 下列字段的聚合 |
| `$current_context.behavior_name` | `{{ current_context.behavior_name }}` | string | 当前 behavior name |
| `$current_context.last_step` | `{{ current_context.last_step.observation }}` | `StepRecord` / null | 最近一个 step |
| `$current_context.last_report` | `{{ current_context.last_report }}` | string / null | 当前 context 最近一次 `<report>` |
| `$current_context.step_history` | `{% for step in current_context.step_history %}` | array of `StepRecord` | 当前 context 的 step history |

`StepRecord` 是给 Behavior 模板读取 step 历史的稳定结构，至少应能暴露：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `step_index` | number | step 序号 |
| `behavior_name` | string | step 所属 behavior |
| `observation` | string / null | 上一步 action / tool 结果观察 |
| `thinking` | string / null | LLM 本步思考摘要 |
| `actions` | array | 本步发起的 action / tool call |
| `report` | string / null | 本步产生的 report |
| `next_behavior` | string / null | 本步声明的 next behavior |

`on_behavior_step_ob` 还应提供系统默认的 action result 文本渲染，避免每个 Behavior 自己格式化复杂的 action / observation 结构：

```toml
on_behavior_step_ob = """
<<last_step_action_results>>
{{ default_last_step_action_results_text }}
<</last_step_results>>
"""
```

| 表达式 | 类型 | 含义 |
| --- | --- | --- |
| `$default_last_step_action_results_text` | string | 当前 last step action results 的默认纯文本渲染，可直接放进 prompt |
| `$default_last_step_action_results_content` | array / object | 当前 last step action results 的结构化内容，不能直接用 `{{ }}` 格式化输出 |

Behavior 模板需要自然语言 observation 时统一使用 `default_last_step_action_results_text`；只有需要按类型处理 action result 时才读取结构化 `default_last_step_action_results_content`。

### 3.6 `runtime`

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$runtime` | `{{ runtime }}` | object | 下列字段的聚合 |
| `$runtime.clock_unix_ms` | `{{ runtime.clock_unix_ms }}` | number | Driver 冻结环境时的 Unix ms |
| `$runtime.clock_text` | `{{ runtime.clock_text }}` | string | Driver 冻结环境时的本地时间，格式 `DD-MM HH:MM`，24 小时制 |
| `$runtime.recent_activity` | `{{ runtime.recent_activity }}` | string | `OneLineStatusSink` 当前值 |
| `$runtime.has_activity` | `{{ runtime.has_activity }}` | bool | recent activity 是否非空 |
| `$runtime.workspace_list_text` | `{{ runtime.workspace_list_text }}` | string | `render_workspace_inventory` 渲染出的 workspace 列表文本，按当前 `session_id` 上次访问后的 `updated_at_ms` 增量窗口读取当前 Agent workspace registry；首次访问按 `updated_at_ms` 倒序读取 |
| `$runtime.last_schedule_task_list_text` | `{{ runtime.last_schedule_task_list_text }}` | string | Schedule-Task 增量摘要的预渲染文本；包含从上一次访问到本次环境冻结之间需要 Agent 关注的计划任务变化 |

`runtime.last_schedule_task_list_text` 面向 prompt 直接阅读，不要求 Behavior 再遍历结构化 task 列表。文本只包含下列三类 Schedule-Task：

| 类别 | 触发条件 | 备注要求 |
| --- | --- | --- |
| 新计划任务 | 上一次访问后新创建的 Schedule-Task | 必须注明是否 create from Notebook Item；如果是，应包含来源 notebook item 信息 |
| 运行失败的计划任务 | 上一次访问后执行失败的 Schedule-Task | 应包含失败原因、最近一次 execution report 或可读错误摘要 |
| 有用户手工 Noted 过的计划任务 | 上一次访问后用户手工追加过 note / remark 的 Schedule-Task | 应包含 note / remark 摘要、作者和时间；只统计用户手工记录，不统计系统自动执行日志 |

每条 task 至少包含：动作、`task_id`、`task_title`、相关备注。建议格式保持简短稳定，例如：

```text
- created task_id=task-1 task_title="daily mail check" note="create from Notebook Item notebook=user/actions item=item-9"
- failed task_id=task-2 task_title="send weekly report" note="last run failed: smtp timeout"
- user_noted task_id=task-3 task_title="book train reminder" note="Alice noted: postpone to Friday"
```

没有匹配任务时返回可读空结果，例如 `Recent schedule tasks: none.`。

### 3.7 `notebook`

`notebook` 表示当前 Agent 可见的 Agent Notebook 摘要。这里注入的是给 prompt 使用的纯文本索引，不是 notebook 正文；需要确认事实时，Behavior 仍应调用 `agent-notebook read` 读取对应 notebook / item。

| 表达式 | upon 占位 | 类型 | 来源 |
| --- | --- | --- | --- |
| `$notebook` | `{{ notebook }}` | object | 下列字段的聚合 |
| `$notebook.list_text` | `{{ notebook.list_text }}` | string | Agent Notebook registry 的默认纯文本渲染；列出当前一共有多少个 Notebook、每个 Notebook 有多少条记录，以及最后修改时间 |
| `$notebook.last_items_text` | `{{ notebook.last_items_text }}` | string | Agent Notebook 最近变化 item 的默认纯文本渲染；按当前 `session_id` 上次访问后的增量窗口读取，最多列出最近 8 条 item；每条 item 直接输出 `item_id status updated_at source_session_id` 和正文内容；带 active `self_check` remark 且标记 `keep_observing` 的 item 不受该窗口限制 |

推荐在 system prompt 中直接插入这两个字段：

```upon
{{ notebook.list_text }}
{{ notebook.last_items_text }}
```

`notebook.list_text` 用于帮助 Agent 选择应该读取哪本 notebook；`notebook.last_items_text` 用于直接暴露最近变化 item 的状态、更新时间、来源 session 和内容，避免只拿到标题后再补读。

## 4. `on_behavior_switch` 变量

`on_behavior_switch` 除了通用变量，还会注入切换来源信息：

| 占位 | 类型 | 含义 |
| --- | --- | --- |
| `{{ switch.from }}` / `{{ from_behavior }}` | string | 切换前 behavior name |
| `{{ switch.to }}` | string | 切换后 behavior name |
| `{{ switch.from_context }}` | `LLMContext` / null | 切换前 context 的冻结摘要 |
| `{{ switch.to_context }}` / `{{ current_context }}` | `LLMContext` | 切换后的当前 context |

Fork / independent child 返回父 context 时，`switch.from_context.last_report` 是父 behavior 观察 child 输出的主要入口。

## 5. 关键事件变量

事件统一进入 `input.events`。为了让常见事件更容易写模板，下列派生变量按事件类型分桶；没有匹配事件时均为空数组。

| 变量 | 类型 | 来源 event |
| --- | --- | --- |
| `input.timer_events` | array of `TimerEvent` | `timer.*` |
| `input.reminder_events` | array of `TimerEvent` | `timer.reminder_check` |
| `input.hard_barrier_events` | array of `TimerEvent` | `timer.hard_barrier` |
| `input.scheduled_task_events` | array of `TimerEvent` | `timer.scheduled_task_check` |
| `input.worksession_reports` | array of `WorksessionReportEvent` | `worksession_report` |

`TimerEvent.data` 使用 `TimerReason`：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `trigger_type` | string | `hard_barrier` / `precise_trigger` |
| `target_type` | string | `reminder` / `scheduled_task` / `other` / named target |
| `target_id` | string | 目标对象 id |
| `expected_trigger_time` | string | 期望触发时间 |
| `reason` | string | timer 原因 |

示例：

```upon
{% for timer in input.reminder_events %}
- reminder {{ timer.data.target_id }}: {{ timer.data.reason }}
{% endfor %}
```

`WorksessionReportEvent.data` 的关键字段：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `report_id` | string | report 唯一 id |
| `source_session_id` | string | 来源 WorkSession |
| `target_session_id` | string | 目标 UI Session |
| `title` | string | WorkSession 标题 |
| `objective` | string | WorkSession 目标 |
| `workspace_id` | string / null | workspace id |
| `phase` | string | `checkpoint` / `final` |
| `report` | string | report 正文 |
| `is_final` | bool | 是否 final report |

## 6. 环境块

环境块由一组 session 相关的 background hints 组成。OpenDAN 不再自动插入 `<background_environment>` 块；是否插入、插入到哪个 User Message 模板，由 Behavior 开发者显式决定。

Driver 需要在对应 hook 上打开 hint 加载，例如：

```toml
[session.ui.driver.on_wakeup]
filter = "top"
pull_msg = "all"
pull_event = "none"
load_background_hits = "all"
```

然后在相关 User Message 模板中手工插入：

```upon
{% if session.background_hint_changed %}
<background_environment>
{{ session.default_changed_background_hint_text }}
</background_environment>
{% endif %}
```

`session.default_changed_background_hint_text` 是 `session.background_hints` 的默认渲染，格式为：

```text
- hint1
- hint2
```

需要注意的是，`load_changed_background_hits` 会记录上一次加载状态，只返回相比上次调用发生变化的部分。为了让事件和其它背景变化有时间汇聚，函数还有一个 session 级 60 秒硬间隔：同一 session 只要返回过一次非空结果，未来 60 秒内每次调用都会直接返回空，不读取也不刷新 hint 指纹。没有配置 `load_background_hits = "all"` 的 hook 不会读取 hints，`session.background_hint_changed` 为 `false`。

## 7. Include 与安全边界

Behavior 模板应使用 `__INCLUDE__` 引入文件内容，不再依赖 `role_md` / `self_md` 这类 render-time extras。

| 字段 | 当前值 | 说明 |
| --- | --- | --- |
| `include_roots` | `[agent_root, session_root]` + `workspace_root`（若绑定） | `__INCLUDE__` 白名单 |
| `allow_exec` | `false` | `__EXEC__` 全程关闭 |
| `max_include_bytes` | 64 KiB | 单次 `__INCLUDE__` 字节上限 |
| `max_total_bytes` | 256 KiB | 渲染输出上限，超出标记 `truncated=true` |
| `max_recursion_depth` | 8 | `__INCLUDE__` 嵌套深度上限 |

`__INCLUDE__` 解析为绝对路径，且必须在某个 `include_root` 之下，否则会留下失败标记，不会读取任意路径。

## 8. 测试入口

- 单元测试在 [`prompt_env::tests`](../../src/frame/opendan/src/prompt_env.rs) 模块。
- 变量契约主测试是 `prompt_env::tests::contract_renders_main_variables_and_control_flow`。本文任意新增、删除、重命名或改变语义的 prompt env 变量，必须同步更新这个测试；新增常用模板写法（尤其是 `if` / `for` / 数组索引）也必须在该测试或同级契约测试中覆盖。
- `cargo test -p opendan --lib prompt_env::` 可单独跑。
- 完整 opendan 单元测试：`cargo test -p opendan --lib`。
