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
  agent.app.json
  agent.json.doc
  index.json
  role.md
  self.md
  behaviors/
  skills/
  sessions/
  workspaces/
    session_workspace_bindings.json
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

- `agent.app.json` 是启动后缓存到本地的 agent spec 快照。
- `agent.json.doc` 是启动后缓存到本地的 agent instance doc 快照。
- `index.json` 是 workshop 级 workspace 索引。
- `role.md`、`self.md`、`behaviors/` 是 agent 核心提示词与行为配置的 overlay 根；优先从 `agent_env_root` 读取，再回退到 agent package。
- `todo/todo.db` 是 workshop 级 todo 数据库。
- `worklog/worklog.db` 是 workshop 级 worklog 数据库。
- `workspaces/session_workspace_bindings.json` 记录 session 到本地 workspace 的绑定关系。
- `workspaces/<workspace_id>/worklog/worklog.db` 是 workspace 级 worklog 数据库。

说明：

- `agent_env_root` 顶层基础目录会在启动时被预创建。
- `todo.db`、`worklog.db`、workspace 下的 `worklog/` 等文件/子目录允许按首次访问延迟创建；本规范描述的是稳定的逻辑布局，不要求所有文件在首次启动时全部物化。

## 3. Session 绑定对象

session 绑定本地 workspace 后，`workspace_info.binding` 必须至少包含以下字段：

```json
{
  "binding": {
    "session_id": "sess-demo",
    "local_workspace_id": "ws-demo",
    "workspace_path": "/abs/path/to/agent_env_root/workspaces/ws-demo",
    "workspace_rel_path": "workspaces/ws-demo",
    "agent_env_root": "/abs/path/to/agent_env_root",
    "bound_at_ms": 1710000000000
  }
}
```

字段定义：

- `binding.session_id`
  当前绑定关系所属的 session id。
- `binding.local_workspace_id`
  当前绑定的 workspace id。
- `binding.workspace_path`
  当前绑定的 workspace 根目录绝对路径。
- `binding.workspace_rel_path`
  workspace 根目录相对于 `agent_env_root` 的相对路径。当前固定为 `workspaces/<workspace_id>`。
- `binding.agent_env_root`
  当前 session 所属的 `agent_env_root` 绝对路径。
- `binding.bound_at_ms`
  绑定建立时间戳（毫秒）。

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

## 6. Agent 核心配置

当前实现里，Agent 的核心配置分成三层：

### 6.1 启动层配置（`src/frame/opendan/src/main.rs`）

启动层负责确定当前实例的：

- `agent_id`
- `agent_env_root`
- `agent_package_root`
- `agent_did`
- `agent_owner_did`
- `service_port`

其中 `agent_env_root` 的来源优先级是：

1. CLI 参数 `--agent-env`
2. 环境变量 `OPENDAN_AGENT_ENV` / `AGENT_ENV` / `AGENT_ROOT`
3. agent spec 中的显式字段
4. agent doc 中的显式字段
5. agent spec 的 `install_config.data_mount_point[root|agent_root|agent_env|workspace]`
6. 默认回退到 `<buckyos_root>/agents/<agent_id>`

`agent_package_root` 的来源与此类似，优先使用 CLI / 环境变量 / spec / doc 中的显式字段，否则根据 agent package 名推导安装目录。

### 6.2 Workshop 层配置（`AgentWorkshopConfig`）

`AgentWorkshopConfig` 定义 workshop/runtime 的基础运行参数，核心字段包括：

- `agent_env_root`
- `agent_did`
- `bash_path`
- `default_timeout_ms`
- `max_output_bytes`
- `default_max_diff_lines`
- `default_max_file_write_bytes`
- `tools_json_rel_path`，默认是 `tools/tools.json`
- `local_workspace_lock_ttl_ms`

如果 `tools/tools.json` 不存在，则使用内置默认工具集：

- `exec_bash`
- `edit_file`
- `write_file`
- `read_file`
- `todo_manage`
- `create_workspace`
- `bind_workspace`

### 6.3 Agent 层配置（`AIAgentConfig`）

`AIAgentConfig` 定义 Agent 自身行为加载与循环执行参数，默认值包括：

- `agent_root = agent_env_root`
- `behaviors_dir_name = behaviors`
- `role_file_name = role.md`
- `self_file_name = self.md`
- `worklog_file_rel_path = worklog/agent-loop.jsonl`
- `session_worker_threads = 1`

另外还包含 `max_steps_per_wakeup`、`max_behavior_hops`、`max_walltime_ms`、sleep/hp/memory 限制等运行控制参数。

Agent package / env root 下的本地 `agent.json` 还可以覆盖部分运行时默认值：

- `default_ui_behavior_name`
  UI session 的默认 behavior 名称；未设置时继续按现有逻辑自动选择，优先 `resolve_router`。
- `default_work_behavior_name`
  work session 的默认 behavior 名称；未设置时继续按现有逻辑自动选择，优先 `plan` / `do`。
- `self_check_timer`
  agent 自检定时器周期，单位秒；默认 `10`。设置为 `0` 表示关闭。当前版本只会启动线程并输出日志，后续再补真实自检行为。

## 7. Agent 加载逻辑

### 7.1 启动器到 opendan 进程

本地部署流程里，`node_daemon` 启动 agent 时会：

- 先解析 agent package 根目录
- 取 app data 目录作为 `agent_env_root`
- 通过 `--agent-id`、`--agent-env`、`--agent-bin`、`--service-port` 启动 `opendan`

因此在正常部署链路中，`agent_env_root` 的首选来源是 loader 传入的 `--agent-env`，而不是运行时自行猜测。

### 7.2 启动时的元数据加载

`opendan` 启动后会先读取两类上游元数据：

- agent instance doc：`agents/<agent_id>/doc`
- agent spec：`users/<owner>/agents/<agent_id>/spec`

解析完成后，会把它们分别缓存为：

- `<agent_env_root>/agent.json.doc`
- `<agent_env_root>/agent.app.json`

这样后续排障时，只看 `agent_env_root` 就能知道该实例启动时拿到的上游配置快照。

### 7.3 RootFS / workshop 初始化

确定 `agent_env_root` 之后，运行时会先做绝对路径归一化，然后确保以下顶层目录存在：

- `memory/`
- `skills/`
- `sessions/`
- `workspaces/`
- `worklog/`
- `todo/`
- `tools/`
- `artifacts/`

随后：

- `AgentWorkshop::new/create_workshop/load_workshop` 会初始化 workshop
- `LocalWorkspaceManager` 会创建或加载 `index.json`
- 若存在 `workspaces/session_workspace_bindings.json`，则会恢复 session 绑定表

其中：

- `create_workshop` 在 `index.json` 缺失时会创建新索引
- `load_workshop` 要求 `index.json` 已存在，否则报错

### 7.4 Session 与本地 workspace 绑定恢复

当 session 绑定到本地 workspace 时，会同时持久化两份信息：

- workshop 级绑定表：`workspaces/session_workspace_bindings.json`
- session 自身的 `workspace_info.binding`

`binding` 中至少包括：

- `local_workspace_id`
- `workspace_path`
- `workspace_rel_path`
- `agent_env_root`
- `bound_at_ms`

绑定成功后，session 的运行态会同步更新：

- `local_workspace_id`
- `pwd = binding.workspace_path`
- `workspace_info.workspace_id`
- `workspace_info.local_workspace_id`
- `workspace_info.workspace_type = local`
- `workspace_info.binding`

因此后续路径解析只需要读取 session 当前状态，不再需要通过目录结构反推。

### 7.5 行为、提示词与 package overlay

Agent 核心资源的加载顺序是“环境根优先，package 回退”：

- `role.md`：先读 `<agent_env_root>/role.md`，再回退 package 内同名文件或 `prompts/role.md`
- `self.md`：先读 `<agent_env_root>/self.md`，再回退 package 内同名文件或 `prompts/self.md`
- `behaviors/`：先读 `<agent_env_root>/behaviors/`，再追加 package 下的 `behaviors/`

如果两边都没有 `behaviors/`，运行时会在 `<agent_env_root>/behaviors/` 下创建空目录作为 fallback 根。

### 7.6 Workshop 默认数据库与路径

若工具配置没有覆盖默认路径，则当前实现固定使用：

- workshop todo DB：`<agent_env_root>/todo/todo.db`
- workshop worklog DB：`<agent_env_root>/worklog/worklog.db`
- local workspace worklog DB：`<local_workspace_root>/worklog/worklog.db`

这些路径与第 4 节的确定性读取规则保持一致。

## 8. 兼容性与边界

需要区分两个阶段：

- 启动阶段：为了兼容历史部署数据，`opendan main` 仍会从多个 CLI/env/spec/doc 字段里解析 `agent_env_root` 与 `agent_package_root`
- 运行阶段：一旦实例已经启动，session/workspace/todo/worklog 的路径解析必须遵守本文第 3、4、5 节，不再做祖先扫描或多 key 猜测

因此：

- 本规范不保留运行期路径解析的向前兼容
- 历史字段兼容只允许保留在启动入口，不应扩散到 workshop/session/path resolver 的新实现中


## 9. COW

```text
Agent RootFS = OverlayFS(Package[RO], Data[RW])
```

每个 Agent 运行时看到一个统一的文件系统视图，由两层 overlay 合并而成：

- `Package`：不可变发布包，只读
- `Data`：Agent 实例的全部可写状态

`Data` 包含：

- self-improve 产物
- 覆盖后的 `role.md` / `self.md` / `behaviors/`
- `sessions/`
- `todo/`
- `worklog/`
- 本地 workspace
- 运行时缓存文件

本节的目标不是让 host 直接管理 OverlayFS，而是定义一个 **Docker-first COW** 方案：host 只负责启动容器和提供持久卷，OverlayFS 由容器内的 Linux 运行时负责。

---

### 9.1 为什么采用 Docker-first

host 大部分情况下是 Windows / macOS，直接在 host 上统一做 OverlayFS 生命周期管理，跨平台成本高且行为不稳定。

因此采用以下边界：

- host / `node_daemon`：
  负责 `docker run`、卷挂载、端口、环境变量、实例生命周期
- container：
  负责把 `Package + Data` 挂成统一的 `agent_env_root`
- `opendan`：
  只消费已经准备好的 `--agent-env` 与 `--agent-bin`

这样可以保持 `opendan` 的路径契约不变，同时把 COW 对内核能力的依赖收敛到 Docker 容器内部。

---

### 9.2 为什么必须用 OS 层 OverlayFS

在应用层用 `open_file()` / `list_dir()` 模拟 overlay 有一个无法修补的问题：Bash、shell 脚本、第三方工具直接走内核 syscall，绕过 API。结果就是 Agent 代码看到的文件系统和 Bash 看到的不一致。

OverlayFS 在内核层完成合并，`open()`、`readdir()`、`stat()` 全部自动生效，任何进程看到的视图完全一致。这一点对 OpenDAN 当前大量依赖 bash/tooling 的执行模型是必要条件。

---

### 9.3 四路径模型

实现时必须区分 4 个路径，而不是把 merged root 和 upperdir 视为同一路径：

| 角色 | 容器内示例路径 | 说明 |
|------|----------------|------|
| `package_root` | `/opt/agent/package` | lowerdir，只读镜像内容或只读挂载 |
| `data_upper` | `/opt/agent/data` | upperdir，全部可写状态落在这里 |
| `overlay_work` | `/opt/agent/data/.overlay_work` | OverlayFS workdir，必须与 upperdir 同文件系统 |
| `agent_env_root` | `/opt/agent/rootfs` | merged root，运行时统一视图，也是传给 `--agent-env` 的路径 |

关键约束：

- `data_upper` 不是 `agent_env_root`
- `agent_env_root` 是挂载点，不是持久数据目录
- `overlay_work` 不应暴露给 Agent 作为业务目录使用

---

### 9.4 目录语义

在 merged root 里，OpenDAN 仍然看到第 2 节定义的标准布局，但这些路径的真实来源分成两类：

- 来自 `Package`
- 来自 `Data`

推荐约定如下：

- `role.md` / `self.md` / `behaviors/`
  默认来自 `Package`，一旦修改则 copy-up 到 `Data`
- `sessions/`、`todo/`、`worklog/`、`artifacts/`、`memory/`
  实际上应长期驻留在 `Data`
- `workspaces/local/`
  本地 workspace 根
- `external_workspaces/`
  外部 workspace 挂载点或链接目录

说明：

- 当前实现里 local workspace 与 external workspace 都复用了 `workspaces/`
- Docker 化前，建议把语义收敛为：
  - `workspaces/local/` 给 workshop/local workspace
  - `external_workspaces/` 给 runtime external bindings

这是一个目标设计；代码当前尚未完成该拆分。

---

### 9.5 OverlayFS 行为速查

| 操作 | 行为 |
|---|---|
| 读文件 | 优先读 `Data`，没有则透到 `Package` |
| 写文件（Package 中已有） | 自动 copy-up 到 `Data` 后再写 |
| 写新文件 | 直接写入 `Data` |
| 删除 Package 文件 | 在 `Data` 中生成 whiteout |
| 列目录 | 内核自动合并两层视图 |
| Package 升级 | 只对未被 copy-up / whiteout 的路径生效 |

---

### 9.6 与当前 OpenDAN 启动协议的关系

当前 `opendan` 启动入口接受的关键参数是：

- `--agent-id`
- `--agent-env`
- `--agent-bin`
- `--service-port`

因此 COW 方案不应改写 `opendan` 的核心启动接口，而应让 container entrypoint 先完成 overlay 挂载，再以现有协议启动 `opendan`：

- `--agent-env = agent_env_root`
- `--agent-bin = package_root`

对应到当前实现，`node_daemon` 未来需要从“直接起本地进程”切换为“启动容器并传同等实例参数”，但 `opendan` 本身不需要理解 Docker 细节。

---

### 9.7 Docker 容器内实现

#### entrypoint.sh

```bash
#!/bin/sh
set -eu

PACKAGE_ROOT=/opt/agent/package
DATA_UPPER=/opt/agent/data
OVERLAY_WORK=/opt/agent/data/.overlay_work
AGENT_ENV_ROOT=/opt/agent/rootfs

mkdir -p "$DATA_UPPER" "$OVERLAY_WORK" "$AGENT_ENV_ROOT"

mount -t overlay overlay \
  -o lowerdir="$PACKAGE_ROOT",upperdir="$DATA_UPPER",workdir="$OVERLAY_WORK" \
  "$AGENT_ENV_ROOT"

exec /opt/agent/opendan \
  --agent-id "${OPENDAN_AGENT_ID}" \
  --agent-env "$AGENT_ENV_ROOT" \
  --agent-bin "$PACKAGE_ROOT" \
  --service-port "${OPENDAN_SERVICE_PORT}"
```

说明：

- 这里只是示意启动协议，不代表最终镜像路径必须完全一致
- 如果容器内不允许 `mount -t overlay`，可以退化为 `fuse-overlayfs`
- entrypoint 的职责是“准备 RootFS 并启动 opendan”，不是重新定义 workspace 启动接口

---

### 9.8 Host / node_daemon 的职责

在 Docker-first 方案中，host 不负责 OverlayFS 挂载，只负责：

- 准备只读 Package 来源
- 准备 Agent 实例级 Data volume
- 注入 `agent_id`、`service_port` 等实例参数
- 管理容器启停、日志、健康检查

也就是说，`node_daemon` 的改造重点是 Docker 编排，而不是跨平台 mount 细节。

容器启动后，entrypoint 负责生成真正的 `agent_env_root`，再把该路径传给 `opendan`。

---

### 9.9 docker run 示意

```bash
docker run \
  --cap-add SYS_ADMIN \
  -e OPENDAN_AGENT_ID=jarvis \
  -e OPENDAN_SERVICE_PORT=3180 \
  -v jarvis_data:/opt/agent/data \
  opendan/agent-runtime:latest
```

如果 `Package` 放在镜像内，则不必额外挂只读卷；如果需要把 `Package` 与镜像解耦，也可以额外挂只读 volume 到 `/opt/agent/package`。

`--cap-add SYS_ADMIN` 是容器内直接使用内核 OverlayFS 的最小权限要求。若安全策略不允许，可评估：

- `fuse-overlayfs`
- 预先在镜像构建期生成基础层，运行期只做 volume 覆盖

但这两种都属于兼容实现，不是主设计。

---

### 9.10 Package 更新语义

Package 更新的推荐合同应收敛为：

- **默认语义：下次容器重启生效**

也就是说：

- 更新 `Package` 后，新容器实例会看到最新 lowerdir
- 已 copy-up 到 `Data` 的文件继续优先使用实例自己的版本
- 不承诺“运行中立刻热切换到新 Package”

如果未来需要在线热切换，应单独定义 remount / restart 策略，而不是把它作为当前基础合同的一部分。

---

## 一句话总结

> COW 主要依靠 Docker 容器内的 Linux OverlayFS 实现。host 只负责编排容器；container 内把 `Package[RO] + Data[RW]` 挂成独立的 `agent_env_root`，再按现有 `opendan --agent-env/--agent-bin` 协议启动。
