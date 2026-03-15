# Render Prompt 模板变量说明

本文档描述 `AgentEnvironment::render_prompt` 在渲染 prompt 时支持的模板变量和语法。

## 概述

`render_prompt` 通过 `render_text_template` 实现，解析流程分为两个阶段：

1. **阶段一**：展开 `__OPENDAN_ENV(path)__` 占位符（仅从 `env_context` 取值）
2. **阶段二**：替换 `{{key}}` 占位符（优先 `env_context`，未命中则从 session 加载）

## 一、`__OPENDAN_ENV(path)__` 语法

**格式**：`__OPENDAN_ENV(key)__`

- 仅从 `env_context`（`HashMap<String, Json>`）中取值
- 若 key 不存在，替换为空字符串

**示例**：
```
__OPENDAN_ENV(params.todo)__
__OPENDAN_ENV(step_summary)__
```

## 二、`{{key}}` 占位符

**格式**：`{{key}}`

解析顺序：

1. 先在 `env_context` 中查找（支持 JSON 路径）
2. 若未找到，再调用 `load_value_from_session` 从 session 加载

### 2.1 来自 env_context 的变量

`env_context` 由调用方传入，常见字段包括（视调用场景而定）：

| 变量 | 说明 |
|------|------|
| `params` | 请求参数 JSON 对象 |
| `params.<path>` | 参数中的嵌套路径，如 `params.todo`、`params.x` |
| `role_md` | 角色描述 Markdown |
| `self_md` | 自身描述 Markdown |
| `session_id` | 会话 ID |
| `loop.session_id` | 循环中的会话 ID |
| `step.index` | 当前步骤索引 |
| `step_summary` | 上一步摘要 |
| `step_index` | 步骤索引 |
| `step_limit` | 步骤上限 |
| `llm_result` | LLM 返回结果 |
| `trace` | 执行追踪信息 |

**JSON 路径**：支持 `.` 访问嵌套字段和数组下标，例如：

- `params.todo` → 对象字段
- `params.items.0` → 数组第一个元素

### 2.2 来自 Session 的变量（load_value_from_session）

当 `env_context` 中无对应 key 时，从 session 加载：

#### 会话元数据

| 变量 | 说明 |
|------|------|
| `session_id` | 当前会话 ID |
| `step_index` | 当前步骤索引 |
| `last_step_summary` | 上一步摘要文本 |

#### 消息与事件

| 变量 | 说明 |
|------|------|
| `new_msg` | 新消息（从 kmsgqueue 拉取，默认最多 32 条） |
| `new_msg.$n` | 同上，`$n` 为 1–4096 的拉取上限，如 `new_msg.8` |

#### 会话与工作区列表

| 变量 | 说明 |
|------|------|
| `session_list` | 最近会话列表（默认最多 16 条） |
| `session_list.$n` | 同上，`$n` 为拉取上限 |
| `local_workspace_list` | 最近本地工作区列表（默认最多 16 条） |
| `local_workspace_list.$n` | 同上，`$n` 为拉取上限 |

#### Todo 相关

| 变量 | 说明 |
|------|------|
| `current_todo` | 下一个就绪 todo 的代码（如 T01） |
| `workspace.todolist.next_ready_todo` | 下一个就绪 todo 的渲染详情 |

#### 工作区信息（workspace_info）

| 变量 | 说明 |
|------|------|
| `workspace.<path>` | 从 session 的 `workspace_info` JSON 中按路径取值 |
| `current_todo` | 也可映射到 `workspace.current_todo` |

支持的路径包括但不限于：

- `workspace.current_todo`
- `workspace.todolist`
- `workspace.todolist.<path>`

#### 文件包含

| 变量 | 说明 |
|------|------|
| `$workspace/<rel_path>` | 从工作区根目录读取文件内容 |
| `$cwd/<rel_path>` | 从当前会话工作目录读取文件内容 |

**路径安全**：`rel_path` 必须是相对路径，不允许 `..` 等越界访问。

**示例**：
```
{{$workspace/readme.txt}}
{{$cwd/config.yaml}}
```

## 三、转义

若需输出字面量 `{{` 或 `}}`，使用反斜杠转义：

- `\{{` → `{{`
- `\}}` → `}}`

## 四、限制

- **总输出长度**：渲染结果截断至 `MAX_TOTAL_RENDER_BYTES`（256KB）
- **文件包含**：单文件内容截断至 `MAX_INCLUDE_BYTES`（64KB）
- **new_msg 拉取**：默认 32 条，最大 4096
- **session_list / local_workspace_list**：默认 16 条

## 五、返回值

`AgentTemplateRenderResult` 包含：

| 字段 | 说明 |
|------|------|
| `rendered` | 渲染后的文本 |
| `env_expanded` | `__OPENDAN_ENV` 成功展开数量 |
| `env_not_found` | `__OPENDAN_ENV` 未找到数量 |
| `successful_count` | `{{key}}` 成功替换数量 |
| `failed_count` | `{{key}}` 未找到数量 |

## 六、相关代码位置

- `AgentEnvironment::render_prompt`：`src/frame/opendan/src/agent_environment.rs:396`
- `render_text_template`：`src/frame/opendan/src/agent_environment.rs:114`
- `load_value_from_session`：`src/frame/opendan/src/agent_environment.rs:204`
- `expand_opendan_env_tokens`：`src/frame/opendan/src/agent_environment.rs:629`
- `resolve_env_context_value`：`src/frame/opendan/src/agent_environment.rs:675`
