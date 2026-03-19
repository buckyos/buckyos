# OpenDAN Workspace FS Path 规范

本文档定义 OpenDAN 在开发阶段采用的确定性 Workspace 文件系统布局与路径读取规则。目标是消除 `collect_workspace_path_candidates` 这一类“枚举多个 key + 向上探测祖先目录”的弹性解析，改为单一规范。

## 1. 术语

- `agent_env_root`
  Agent 的文件系统根目录。对应 `AgentWorkshopConfig.agent_env_root`，也是未绑定本地 workspace 时 session 的默认工作目录。
- `local_workspace_root`
  某个 workspace 的根目录，必须位于 `agent_env_root/workspaces/<workspace_id>`。
- `session workspace root`
  当前 session 的工作区根目录。
  如果 session 已绑定本地 workspace，则等于 `local_workspace_root`。
  如果 session 未绑定本地 workspace，则等于 `agent_env_root`。

## 2. 标准目录树

`agent_env_root` 必须满足以下布局：

```text
<agent_env_root>/
  index.json
  skills/
  sessions/
  workspaces/
    <workspace_id>/
      skills/
      worklog/
        worklog.db
  todo/
    todo.db
  worklog/
    worklog.db
  tools/
  artifacts/
  memory/
```

其中：

- `index.json` 是 workshop 级 workspace 索引。
- `todo/todo.db` 是 workshop 级 todo 数据库。
- `worklog/worklog.db` 是 workshop 级 worklog 数据库。
- `workspaces/<workspace_id>/worklog/worklog.db` 是 workspace 级 worklog 数据库。

## 3. Session 绑定对象

session 绑定本地 workspace 后，`workspace_info.binding` 必须至少包含以下字段：

```json
{
  "binding": {
    "local_workspace_id": "ws-demo",
    "workspace_path": "/abs/path/to/agent_env_root/workspaces/ws-demo",
    "workspace_rel_path": "workspaces/ws-demo",
    "agent_env_root": "/abs/path/to/agent_env_root"
  }
}
```

字段定义：

- `binding.local_workspace_id`
  当前绑定的 workspace id。
- `binding.workspace_path`
  当前绑定的 workspace 根目录绝对路径。
- `binding.workspace_rel_path`
  workspace 根目录相对于 `agent_env_root` 的相对路径。当前固定为 `workspaces/<workspace_id>`。
- `binding.agent_env_root`
  当前 session 所属的 `agent_env_root` 绝对路径。

## 4. 确定性读取规则

路径解析必须遵循以下顺序，禁止再做“候选 key 列表 + 祖先扫描”：

### 4.1 读取 session workspace root

只读取：

- `workspace_info.binding.workspace_path`
- 否则回退到 `session_cwd`

### 4.2 读取 agent_env_root

只读取：

- `workspace_info.binding.agent_env_root`
- 否则回退到 `session_cwd`

### 4.3 读取 local workspace root

只允许两种方式：

- 当前绑定 workspace：直接使用 `workspace_info.binding.workspace_path`
- 已知 `workspace_id` 且已知 `agent_env_root`：
  `agent_env_root/workspaces/<workspace_id>`

### 4.4 读取数据库路径

- agent env todo DB：
  `<agent_env_root>/todo/todo.db`
- agent env worklog DB：
  `<agent_env_root>/worklog/worklog.db`
- local workspace worklog DB：
  `<local_workspace_root>/worklog/worklog.db`

## 5. 禁止项

以下行为从本规范开始视为不再允许的新实现：

- 同时兼容 `/workspace_root`、`/workspace/root`、`/path`、`/root_path` 等多个 JSON key
- 从任意 `cwd` 开始向上遍历祖先目录，猜测哪个目录是 agent_env_root
- 通过“某目录下是否存在 `todo.db` / `worklog.db` / `index.json`”来反推出根目录语义

## 6. 兼容性

本规范不保留向前兼容路径解析，也不再接受旧字段或旧目录层级。
