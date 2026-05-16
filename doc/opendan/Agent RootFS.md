# OpenDAN Agent RootFS 规范

本文档定义 OpenDAN 运行时所看到的文件系统视图，覆盖：

- AgentRootFS 的角色与跨平台契约
- AgentRootFS 标准目录布局
- 4 层 Bin（System / Runtime / Agent / Session）的物理路径与渲染规则
- 启动加载与 Session 绑定的确定性读取规则
- COW（容器内 OverlayFS）落地方案

目标是消除"枚举多个 JSON key + 向上扫祖先目录"这类弹性解析，改为单一来源。

> 本文档与 [NewOpenDANRuntime.md](../../notepads/NewOpenDANRuntime.md) §1 / §2 / §1.5 对齐。当二者冲突时，
> 本文档负责"数据层文件系统视图"的规范，NewOpenDANRuntime.md 负责"运行时 + 调度"的规范。

---

## 0. 运行时部署模型（前置假设）

整套设计建立在 **数据 / 运行时分离** 上：

| 维度        | AgentRootFS（数据）                                                | AgentRuntime（执行）                          |
| ----------- | ------------------------------------------------------------------ | --------------------------------------------- |
| 形态        | 宿主机普通文件系统目录                                             | Linux Docker 容器                             |
| 位置        | `/opt/buckyos/data/home/<user_id>/.local/share/<agent_id>/`        | 容器内 mount AgentRootFS + 临时卷             |
| OS 跨度     | Windows / macOS / Linux 宿主机均可承载，目录结构平台无关           | **始终 Linux**（bash + tmux + POSIX 是刚性依赖） |
| 生命周期    | 长期持久化，跨容器重建保留；可在宿主机之间整目录迁移               | 可随时销毁重建，无状态                        |
| 路径风格    | 平台无关、相对路径优先                                             | `/opt/buckyos/...` 绝对路径、POSIX symlink    |

推论（后续章节默认成立）：

- **AgentRootFS 是 Agent 的核心状态**——目录结构、配置、session 数据、工具声明全部以平台无关的方式
  存在宿主机文件系统里，可在宿主机之间原样拷贝迁移。
- **跨平台兼容性的要求只落在 AgentRootFS 的数据上**；容器内生成的派生物（PATH 里的执行视图、tmux pane、
  临时挂载点等）可以是纯 Linux 形态。
- **"session 目录不带 `./bin/`" 的根因不是"宿主机可能没有 symlink"**——而是"绑死 Linux 镜像内部路径会
  让数据脱离镜像后失去意义"。即使宿主是 Linux，session 数据里的 symlink 指向容器内路径仍是反模式。
- 所以："数据语义" 由 AgentRootFS 持有；"执行视图" 由容器在启动时按当前镜像 + 当前 session 重新渲染。

---

## 1. 术语

- **`agent_root`（=AgentRootFS 根）**
  Agent 的文件系统根目录。`opendan` 进程通过 `--agent-root` / `--agent-env` / `OPENDAN_AGENT_ROOT`
  环境变量等指定，对应 `AgentLayout.root`。未绑定本地 workspace 时 session 的默认工作目录也是它。
- **`local_workspace_root`**
  某个本地 workspace 的根目录，必须位于 `agent_root/workspace/<workspace_id>`。
- **`session_root`**
  某个 session 的根目录，必须位于 `agent_root/sessions/<session_id>`。
- **`session workspace root`**
  当前 session 的工作区根目录。
  - 已绑定本地 workspace：等于 `local_workspace_root`
  - 未绑定：等于 `agent_root`
- **`buckyos_root`**
  容器内 BuckyOS 公共目录（默认 `/opt/buckyos`，可由 `BUCKYOS_ROOT` 覆盖）；承载 4 层 Bin 的 System /
  Runtime / Session 执行视图（见 §3）。**不是 AgentRootFS 的一部分**。

> `agent_env_root` 是旧术语，已与 `agent_root` 合并；CLI 参数 `--agent-env` 仅为向后兼容别名保留。

---

## 2. AgentRootFS 标准目录树

`agent_root` 必须满足以下布局（与 [agent_config.rs](../../src/frame/opendan/src/agent_config.rs)
`AgentLayout` 一致；`opendan` 启动时由 `init_agent_rootfs` 预创建）：

```text
<agent_root>/
  agent.toml                          # Agent 自身配置（agent_did / display_name / 默认 behavior 等）
  role.md                             # 自我介绍，进 system prompt
  self.md                             # 内部能力 / 边界声明，进 system prompt
  .meta/
    rootfs_sync.json                  # 启动时 package → root 的同步 manifest（保存 sha256，保护本地修改）
  users/
    <user_id>.md                      # 针对调用者的系统提示词片段（按 from_did 选择）
    group_<gid>.md                    # 群组维度的系统提示词片段
  memory/                             # AgentMemory 模块初始化目录
  notepads/
    <notepad_name>/                   # 多本 notepad
  skills/
    <category>/<skill_dir>/           # Agent 实际加载的 skills（可 self-improve）
  tools/                              # Agent Bin 层：Agent 自写脚本工具（见 §3）
  tool_plans/
    <plan_name>.toml                  # 工具策略文件（白名单 / 黑名单），由 behavior 引用（见 §3.4）
  behaviors/
    <behavior_name>.toml              # Behavior 模板（system prompt + 工具白名单 + parser/renderer 配置）
  workspace/
    <workspace_id>/                   # 本地 workspace 根（见 §4）
  sessions/
    <session_id>/                     # session 根（见 §5）
  archive/
    skills/                           # 导入的原始 skills，Agent 不直接看
    sessions/<session_id>/            # 已归档 session
    workspace/<workspace_id>/         # 已归档 workspace
    worklog.db                        # 归档 SQLite
```

说明：

- **目录预创建**：`agent_root` 顶层基础目录会在 `opendan` 启动时由 `ensure_agent_rootfs_layout`
  预创建（[main.rs `ensure_agent_rootfs_layout`](../../src/frame/opendan/src/main.rs)）。
- **延迟物化**：`workspace/<id>/`、`sessions/<id>/`、`archive/worklog.db` 等允许按首次访问延迟创建；
  本规范描述的是稳定的逻辑布局，不要求所有文件在首次启动时全部物化。
- **Worklog 持久化路径**：当前默认 worklog SQLite 由环境变量 `OPENDAN_WORKLOG_DB`
  指定（默认 `/opt/buckyos/opendan/worklog.db`），不在 AgentRootFS 内；归档时按需写到 `archive/worklog.db`。
  这是当前实现的过渡形态，未来可能合并到 AgentRootFS。

**禁止项**：

- **不要在 AgentRootFS 内放可执行二进制**（ELF / Mach-O / PE / `.so` / `.dylib` / `.dll`）。理由：
  - 破坏跨宿主机迁移性
  - 二进制不可被 LLM / 审计者直接阅读
  - 二进制工具应走 ExtTool Volume / Crafter，落到 Runtime Bin 层（§3）
- **不要在 session 目录里放 `./bin/`**。详见 §5。

---

## 3. 4 层 Bin 与 PATH overlay

工具集由 4 层合成（同名后者优先）：

| 层         | 范围             | 物理路径                                                    | 权限       | 持久化                |
| ---------- | ---------------- | ----------------------------------------------------------- | ---------- | --------------------- |
| System Bin | 所有 Agent 可见  | `<buckyos_root>/tools/store/`                               | rx，共享   | 随 Worker Image       |
| Runtime Bin| 特定 Agent 可见  | `<buckyos_root>/tools/bin/`                                 | rx，App-scoped | 容器临时（按 manifest 重建） |
| Agent Bin  | 特定 Agent 可见  | `<agent_root>/tools/`                                       | rwx 给 Agent | AgentRootFS 持久化   |
| Session Bin| 特定 session 可见 |                                                             |            |                       |
| └─ 声明    | session 数据     | `<agent_root>/sessions/<session_id>/tools/`                 | rwx        | AgentRootFS 持久化   |
| └─ 执行视图| 容器临时         | `<buckyos_root>/tools/<agent_id>/<session_id>/`             | rwx        | 容器临时（session 启动时重建） |

物理路径单一来源：[paths.rs](../../src/frame/opendan/src/paths.rs)
（`system_bin_dir` / `runtime_bin_dir` / `session_exec_bin_dir`）。
`<buckyos_root>` 解析顺序：

1. `BUCKYOS_ROOT` 环境变量
2. Linux 默认：`/opt/buckyos`
3. macOS 默认：`$HOME/.buckyos`
4. 其它：`/tmp/buckyos`（仅 dev）

生产容器里固定 `export BUCKYOS_ROOT=/opt/buckyos`。

### 3.1 PATH overlay 顺序

```
PATH = SessionExecBin : AgentBin : RuntimeBin : SystemBin : <inherited>
```

由 [agent_bash.rs](../../src/frame/opendan/src/agent_bash.rs) 的 `SessionBinLayout::to_overlay()`
输出 `BinOverlayConfig::layered([session, agent, runtime, system])`；agent_tool 侧
`prepare_overlay_env` 反向遍历 layers 拼接。前者优先，同名覆盖。

### 3.2 Session Bin 的两个视图

| 视图       | 路径                                                        | 内容                                         | 跨宿主机迁移 |
| ---------- | ----------------------------------------------------------- | -------------------------------------------- | ------------ |
| 声明        | `<agent_root>/sessions/<session_id>/tools/`                 | 文本：脚本源码、`tool.toml`、prompts、schema | ✅ 平台无关  |
| 执行视图    | `<buckyos_root>/tools/<agent_id>/<session_id>/`             | hard-link / wrapper / tombstone stub         | ❌ 容器临时  |

**Session `./tools/` 的硬约束**：

- 只放**文本**。脚本（`.sh` / `.py` / `.ts` / `.js` / `.rb` / ...）、`tool.toml`、prompts、JSON Schema、
  README 都可以；**禁止任何二进制**（启动器在 render 时按 magic bytes 探测，发现二进制 → 跳过 + warn）。
- 必须保持**小且文件数有限**：单文件建议 ≤ 64 KB，整个 `./tools/` 文件数建议 ≤ 几百。
  这是 hot path——每次 `exec_bash` 后启动器会做一次 mtime 同步检查（见 §3.5）。
- 数据集 / 日志 / 抓取结果 / 模型权重 / 缓存等**不要塞进 `./tools/`**，它们属于 session 根、workspace
  或 `./archive/`。

### 3.3 Session 自带工具的两种声明形态

**(a) 扁平脚本形态**——一文件交付：

```
./tools/query_weather.ts
./tools/parse_invoice.sh
./tools/dedup_csv.py
```

元数据自动推断：
- `name` = 文件名去扩展名（同名冲突 → 拒绝 render）
- `interpreter` = 按扩展名映射，回落 shebang
- `description` / `input_schema` = 从文件头 docblock 提取（`# @description:` / `# @input_schema:`
  或顶部 `/** ... */`），缺省时给 LLM 兜底说明
- `version` = 文件 mtime 哈希，仅用于 plan 缓存判定

**(b) 结构化形态**——需要显式 schema / 多文件 / 引用上层工具时：

```
./tools/summarize_pdf/tool.toml
./tools/summarize_pdf/summarize.sh
./tools/summarize_pdf/prompts/sys.md
```

`tool.toml` schema 见 [NewOpenDANRuntime.md §1.5.1](../../notepads/NewOpenDANRuntime.md)。

扁平形态可以"升级"到结构化形态（移到同名子目录 + 写 `tool.toml`），启动器按 name 一致视作同一工具的演化。

### 3.4 Tool Plan（工具策略文件）

工具策略由 Agent 层定义，behavior 引用：

- **位置**：`<agent_root>/tool_plans/<plan_name>.toml`（不是 session 临时决定，是 owner / operator 策略）
- **引用**：Behavior config 字段 `tool_plan = "<plan_name>"`（缺省 = 全部可见）
- **schema**：

  ```toml
  mode = "deny"     # "deny" | "allow"，默认 "deny"
  [[deny]]
  name   = "rm"
  reason = "use trash-cli instead"
  # mode = "allow" 时改用 [[allow]]，未列的工具一律墓碑
  ```

- **解析产物**：session 启动时把"实际生效的合成策略"落到
  `<agent_root>/sessions/<session_id>/tool_plan.resolved.toml`，跟 tmux pane scrollback 一起供审计。

**墓碑 stub 形态**——shebang 脚本，stderr 双行（机器可读 JSON + 人类可读），`exit 127`：

```sh
#!/bin/sh
# auto-generated by opendan tool plan renderer
echo '{"blocked_by":"tool_plan","tool":"rm","reason":"use trash-cli instead","plan":"minimal_safe"}' >&2
echo 'rm is blocked by tool plan: use trash-cli instead' >&2
exit 127
```

`exit 127` 是 shell "command not found" 标准码，调用方脚本无需特别处理。

### 3.5 渲染时机

| 触发点                                            | 动作                                          | 是否打断推理 |
| ------------------------------------------------- | --------------------------------------------- | ------------ |
| session worker cold start / resume                | 同步 render，未完成不进 run loop              | 启动期，无推理可打断 |
| 每次 `exec_bash` 起手                              | walk Agent Bin `tools/` mtime；有改动则 re-render | 否，紧贴 bash 调用 |
| 显式写工具的 session 工具命中 `./tools/`            | 直接标记 dirty                                | 否           |
| `tool_grants` 变化（用户授权 / 撤权）              | 标记 dirty                                    | 否           |
| 任一 dirty 触发后，**下一个 turn 边界**            | re-render                                     | 否，turn 间空档 |
| behavior 切换                                     | **不 re-render**，只调 `tool_policy.whitelist` gate | —            |

实现：[tool_plan.rs `SessionBinRenderer`](../../src/frame/opendan/src/tool_plan.rs)。

- `render_initial(session_dir)`：mkdir → hard-link Agent tools 顶层 + 一层子目录的可执行文件到
  session_bin（跨 fs / 非 Unix 退回 `fs::copy`） → 写 tombstone stub → 序列化 `tool_plan.resolved.toml`
- `maybe_resync()`：snapshot Agent tools 算 `max_mtime_ns`；无变化跳过；有变化 re-link + 重写 tombstone
- Tombstone last-writer-wins：apply_snapshot 主动跳过和 tombstone 同名的 Agent tools 文件

---

## 4. Workspace 子目录契约

```
<agent_root>/workspace/<workspace_id>/
  .workspace.json     # WorkspaceRecord：workspace_id / name / created_by_session / current_session / created_at_ms / updated_at_ms / status
  readme.md           # 目录结构说明，作为环境上下文片段进入提示词
  ...                 # 用户 / Agent 写入的工作文件
```

参考 [local_workspace.rs](../../src/frame/opendan/src/local_workspace.rs) 的 `WorkspaceRecord` /
`LocalWorkspaceManager`：

- 无内存 binding 表——session 绑定由 `AgentSession.meta.workspace_id` 持有（持久化进
  `sessions/<sid>/.meta/session.json`）。
- workspace 记录里的 `current_session` 仅作冲突检测 hint，**不是真理源**。
- `validate_workspace_id` 拒 `..` / `/` / 空 id（防 path traversal）。

---

## 5. Session 子目录契约

```
<agent_root>/sessions/<session_id>/
  .meta/
    session.json                     # SessionMeta：id / agent_did / owner / current_behavior / status /
                                     #   one_line_status / pending_inputs[] / pending_task_calls[] /
                                     #   process_entry / process_stack[] / event_subscriptions[] /
                                     #   workspace_id / title / objective / bootstrap_done / ...
    state.snap                       # 最新 LLMContextSnapshot（由 turn_hook 写入；栈顶 process 镜像）
    behavior_<name>.snap             # independent process 挂起时的快照（仅在挂起时存在）
  readme.md                          # session 目录说明，进环境上下文
  tools/                             # Session Bin 声明层（见 §3.2 / §3.3）
  tool_plan.resolved.toml            # 启动时落地的合成策略，供审计（见 §3.4）
  report.md                          # worksession 完成后的工作报告
  archive/                           # 完整 history（包括 worklog 子集），可翻看
```

**禁止**：

- 不要在 session 目录里放 `./bin/`：`./bin/` + 指向镜像内部路径的 symlink 会让 session 数据绑死特定镜像
  版本、阻碍跨平台迁移、让 Agent 有机会 link 到镜像或宿主机路径绕过授权。
- **所有"进 PATH 的 bin"都在容器临时目录里由启动器渲染**（见 §3.2 执行视图）。

### 5.1 Session 类型

- **UI Session**：永远活跃，每个 UI tunnel 对应一个；天然带 `try_create_worksession` / `forward_msg` 等工具
- **Work Session**：状态机，非 END 状态下都算活跃；由 UI session 通过 `try_create_worksession` 派生

### 5.2 持久化字段

`SessionMeta` 是状态管理的核心，**任何"已经从系统取走、但还没被 LLM 真正消费"的 msg / event
必须落到 `meta.pending_inputs` 持久化字段**。落盘策略：

- `enqueue_pending` → push → `flush_meta()`（tmp + rename crash-consistent）
- 落盘成功才 ack 上游 msg-center；保证 at-least-once
- worker turn 成功才 `discard_consumed` + `flush_meta`；失败保留以便重启重放

详见 [agent_session.rs](../../src/frame/opendan/src/agent_session.rs) 与
[NewOpenDANRuntime.md §1 / §4](../../notepads/NewOpenDANRuntime.md)。

---

## 6. 确定性读取规则

路径解析必须遵循以下顺序，禁止再做"候选 key 列表 + 祖先扫描"。

### 6.1 读取 agent_root

- `AIAgent::open(root, ...)` 拿到的 root（来自启动参数 / 环境变量）
- 不允许从 cwd 向上扫描

### 6.2 读取 session workspace root

- 已绑定：`AgentSession.meta.workspace_id` → `LocalWorkspaceManager::workspace_dir(id)`
  = `<agent_root>/workspace/<id>/`
- 未绑定：`<agent_root>`

### 6.3 读取 local workspace root

只允许两种方式：

- 当前绑定 workspace：直接使用 `LocalWorkspaceManager::workspace_dir(meta.workspace_id)`
- 已知 `workspace_id`：`<agent_root>/workspace/<workspace_id>`

### 6.4 4 层 Bin 路径

| 角色             | 解析函数（`paths.rs`）                          | 默认路径                                                     |
| ---------------- | ----------------------------------------------- | ------------------------------------------------------------ |
| System Bin       | `system_bin_dir()`                              | `<buckyos_root>/tools/store/`                                |
| Runtime Bin      | `runtime_bin_dir()`                             | `<buckyos_root>/tools/bin/`                                  |
| Agent Bin        | `<agent_layout>.tools_dir`                      | `<agent_root>/tools/`                                        |
| Session Exec Bin | `session_exec_bin_dir(agent_id, session_id)`    | `<buckyos_root>/tools/<agent_id>/<session_id>/`              |

`agent_id` / `session_id` 通过 `sanitize_path_segment` 收敛到 `[A-Za-z0-9_-]`，跨 OS 安全。

### 6.5 数据库路径

- worklog（全局）：`OPENDAN_WORKLOG_DB` 环境变量；默认 `/opt/buckyos/opendan/worklog.db`
- 归档 worklog：`<agent_root>/archive/worklog.db`（未来可能合并）

---

## 7. 禁止项

以下行为视为不再允许的实现：

- 同时兼容 `/workspace_root`、`/workspace/root`、`/path`、`/root_path` 等多个 JSON key
- 从任意 cwd 开始向上遍历祖先目录，猜测哪个目录是 agent_root
- 通过"某目录下是否存在 `worklog.db` / `index.json`"反推根目录语义
- 把执行视图（symlink / wrapper）写进 AgentRootFS（session `./tools/` 或 agent `tools/`）
- 在 AgentRootFS 内放任何二进制可执行文件

---

## 8. Agent 核心配置

### 8.1 启动层（[main.rs](../../src/frame/opendan/src/main.rs)）

启动层负责确定当前实例的：

- `appid` / `agent_id`
- `owner_id`
- `agent_root`
- `agent_package_root`（可选）
- `agent_did`（从 system_config 拉的 `AgentDocument` 取）

`agent_root` 来源优先级：

1. CLI 参数 `--agent-root` / `--agent-env`（向后兼容别名）
2. 环境变量 `OPENDAN_AGENT_ROOT`
3. 环境变量 `BUCKYOS_DATA_DIR`
4. 默认回退到 `/opt/buckyos/opendan/agent`

`agent_package_root` 来源：

1. CLI 参数 `--agent-bin`
2. 环境变量 `OPENDAN_AGENT_BIN` / `BUCKYOS_PKG_DIR` / `BUCKYOS_PKG_SOURCE_DIR`
3. 探测 `/opt/buckyos/bin/<appid>` 或 `/opt/buckyos/bin/buckyos_<appid>`

启动层会：

- 确保 `agent_root` 顶层目录预创建（`ensure_agent_rootfs_layout`）
- 把 `agent_package_root` 的可写内容同步进 `agent_root`，保护本地修改（`sync_agent_rootfs_from_package`，
  记录 sha256 到 `.meta/rootfs_sync.json`）
- 把 `agent_did` / `display_name` 写回 `agent.toml`（`write_agent_did_to_toml`）

### 8.2 Agent 配置层（[agent_config.rs](../../src/frame/opendan/src/agent_config.rs)）

- **`AgentLayout`** 是目录指针单一来源，所有子路径从 `agent_root` join 派生
- **`AgentTomlFile`** 是 `<agent_root>/agent.toml` 反序列化结构：
  - `agent_did` / `display_name`
  - `default_ui_behavior` / `default_work_behavior`
  - `subscribe_events`
  - `cancel_reason`（`Observation::Cancelled` 文案兜底）
- **`AgentConfig::open(root)`** 容忍 agent.toml 缺失；`builtin_ui_default()` 兜底；
  `list_behavior_names()` 扫盘列出所有 behavior

### 8.3 Behavior 层（[behavior_cfg.rs](../../src/frame/opendan/src/behavior_cfg.rs)）

每个 behavior 是一份 TOML，落在 `<agent_root>/behaviors/<name>.toml`，详见
[NewOpenDANRuntime.md §3](../../notepads/NewOpenDANRuntime.md)。

`BehaviorCfg` 关键字段：

- `system_prompt_template`、`tool_whitelist`
- `mode = "agent" | "behavior"`（决定是否装 parser+renderer）
- `parser` / `renderer` / `parser_strict` / renderer 调参
- `output` / `max_rounds` / `max_consecutive_errors`
- `switch_mode = "normal" | "independent"`（`fork` 不通过 `next_behavior` 触发）
- `tool_plan = "<plan_name>"`（可选；缺省 = 全部可见）

### 8.4 运行时依赖层（[ai_runtime.rs](../../src/frame/opendan/src/ai_runtime.rs)）

`AgentRuntime` 是进程级单例，持有：

- `aicc: Arc<AiccClient>`（适配为 `LlmClient`）
- `worklog: Arc<WorklogService>`（SQLite 句柄）
- `msg_center: Option<Arc<MsgCenterClient>>`（边界客户端）
- `kevent_client: Option<Arc<KEventClient>>`（边界客户端）
- `task_mgr: Option<Arc<TaskManagerClient>>`（边界客户端）

三个 Option 为 None 时退化成"只接受 `submit_text` 注入"模式，CLI / 单测可不连 zone 服务。

---

## 9. Agent 加载逻辑

### 9.1 启动器到 opendan 进程

本地部署流程里，`node_daemon` 启动 agent 时会：

- 先解析 agent package 根目录（启动器/loader 视角）
- 取 app data 目录作为 `agent_root`
- 通过 `--appid` / `--agent-root` / `--agent-bin` / `--owner-id` 启动 `opendan`

容器化部署见 §10。`opendan` 本身只消费已经准备好的 `--agent-root`。

### 9.2 启动时的元数据加载

`opendan` 启动后先做 BuckyOS 身份注册：`init_buckyos_api_runtime` → `login` →
`set_buckyos_api_runtime`；然后拉取 `agents/<appid>/doc`（`load_agent_document`）取 `agent_did`，
回填到 `<agent_root>/agent.toml`。

> 当前实现里"agent spec / instance doc 快照缓存到本地"的 `agent.app.json` / `agent.json.doc` 已经
> 不再使用——只保留 `agent.toml`（本地配置）。如果未来需要落地上游 spec 快照，应放在 `.meta/` 下。

### 9.3 AgentRootFS 初始化

确定 `agent_root` 后，运行时会：

1. `ensure_agent_rootfs_layout` 预创建顶层目录（见 §2 目录树）
2. `sync_agent_rootfs_from_package`（若 `package_root` 可用）按文件粒度同步发布包到 root，记录
   sha256 到 `.meta/rootfs_sync.json`；**本地修改保护**：若本地文件 hash 与 manifest 中
   `installed_sha256` 不一致，认为是用户/Agent 改过的，跳过覆盖并 log warn
3. `write_agent_did_to_toml` 把 `agent_did` 持久化进 `agent.toml`
4. `AIAgent::open(root, runtime)` 加载 `AgentConfig` + 启动 dispatch loop

### 9.4 Session 与本地 workspace 绑定恢复

当 session 绑定到本地 workspace 时：

- session 侧：`AgentSession.meta.workspace_id` 持久化进 `sessions/<sid>/.meta/session.json`
- workspace 侧：`WorkspaceRecord.current_session` 更新为该 session_id（冲突检测 hint）

重启时 `restore_active_sessions`：

- 遍历 `sessions/` 子目录
- 对每个非 Ended session，通过 `AgentSessionBuild::existing_meta` 还原全套状态
  （`pending_inputs` / `event_subscriptions` / `workspace_id` / peer / `pending_task_calls` /
  `process_entry` / `process_stack` 等都一并恢复）
- 把订阅推回 `session_event_pump`

因此运行期路径解析只需要读取 session 当前 `meta`，**不需要通过目录结构反推**。

### 9.5 行为、提示词与 package overlay

> 当前实现按"启动时 sync"语义，不再做运行期 overlay 回退：
>
> - `role.md` / `self.md` / `behaviors/` 在启动时由 `sync_agent_rootfs_from_package` 从 package 拷到
>   `agent_root`，本地未改时随 package 升级覆盖，已改时保留
> - 运行期只读 `agent_root` 内对应文件，不再"先 root 后 package fallback"
>
> 设计目标：通过容器层 OverlayFS（§10）让 package 与 data 自然合并，**应用层只看一个合并视图**。
> Docker 化前的 sync-on-boot 是过渡方案。

### 9.6 默认数据库与路径

- worklog DB：`OPENDAN_WORKLOG_DB` 或 `/opt/buckyos/opendan/worklog.db`（默认）
- 归档 worklog DB：`<agent_root>/archive/worklog.db`

不再有"workshop todo DB"——todo 现在落在 worklog 同库或具体工具自管理的位置。

---

## 10. COW（容器内 OverlayFS）

```text
Agent RootFS view = OverlayFS(Package[RO], Data[RW])
```

每个 Agent 运行时看到一个统一的文件系统视图，由两层 overlay 合并而成：

- `Package`：不可变发布包，只读
- `Data`：Agent 实例的全部可写状态（即 §2 的 `agent_root`）

本节定义 **Docker-first COW** 方案：host 只负责启动容器和提供持久卷，OverlayFS 由容器内的 Linux
运行时负责。

### 10.1 为什么采用 Docker-first

host 大部分情况下是 Windows / macOS，直接在 host 上统一做 OverlayFS 生命周期管理跨平台成本高且
行为不稳定。因此边界划分：

- **host / `node_daemon`**：`docker run`、卷挂载、端口、环境变量、实例生命周期
- **container entrypoint**：把 `Package + Data` 挂成统一的 `agent_root`
- **`opendan`**：只消费已经准备好的 `--agent-root` 与 `--agent-bin`

### 10.2 为什么必须用 OS 层 OverlayFS

应用层 `open_file()` / `list_dir()` 模拟 overlay 有一个无法修补的问题：Bash、shell 脚本、第三方
工具直接走内核 syscall，绕过 API。结果就是 Agent 代码看到的文件系统和 Bash 看到的不一致。

OverlayFS 在内核层完成合并，`open()` / `readdir()` / `stat()` 全部自动生效，任何进程看到的视图
完全一致。这对 OpenDAN 大量依赖 bash/tooling 的执行模型是必要条件。

### 10.3 四路径模型

实现时必须区分 4 个路径，而不是把 merged root 和 upperdir 视为同一路径：

| 角色            | 容器内示例路径               | 说明                                                                 |
| --------------- | ---------------------------- | -------------------------------------------------------------------- |
| `package_root`  | `/opt/agent/package`         | lowerdir，只读镜像内容或只读挂载                                     |
| `data_upper`    | `/opt/agent/data`            | upperdir，全部可写状态落在这里                                       |
| `overlay_work`  | `/opt/agent/data/.overlay_work` | OverlayFS workdir，必须与 upperdir 同文件系统                       |
| `agent_root`    | `/opt/agent/rootfs`          | merged root，运行时统一视图，也是传给 `--agent-root` 的路径          |

关键约束：

- `data_upper` 不是 `agent_root`
- `agent_root` 是挂载点，不是持久数据目录
- `overlay_work` 不应暴露给 Agent 作为业务目录使用

### 10.4 目录语义（来源切分）

在 merged root 里，OpenDAN 仍然看到 §2 定义的标准布局，但来源分两类：

- **来自 `Package`（lower）**：`role.md` / `self.md` / `behaviors/` / 出厂 `skills/` 等"出厂模板"
- **来自 `Data`（upper）**：`sessions/` / `workspace/` / `tools/` / `tool_plans/` / `memory/` /
  `notepads/` / `archive/` 等"实例状态"

`role.md` / `self.md` / `behaviors/` 一旦修改则 copy-up 到 `Data`。

### 10.5 OverlayFS 行为速查

| 操作                       | 行为                                              |
| -------------------------- | ------------------------------------------------- |
| 读文件                     | 优先读 `Data`，没有则透到 `Package`               |
| 写文件（Package 中已有）   | 自动 copy-up 到 `Data` 后再写                     |
| 写新文件                   | 直接写入 `Data`                                   |
| 删除 Package 文件          | 在 `Data` 中生成 whiteout                         |
| 列目录                     | 内核自动合并两层视图                              |
| Package 升级               | 只对未被 copy-up / whiteout 的路径生效            |

### 10.6 与 `opendan` 启动协议的关系

`opendan` 启动参数：`--appid` / `--agent-root` / `--agent-bin` / `--owner-id`。

COW 方案不改写 `opendan` 启动接口，而由 container entrypoint 先完成 overlay 挂载、再以现有协议
启动 `opendan`：

- `--agent-root = merged root`
- `--agent-bin  = package_root`

`node_daemon` 未来需要从"直接起本地进程"切换为"启动容器并传同等实例参数"，但 `opendan` 本身不需要
理解 Docker 细节。

### 10.7 容器内 entrypoint 示例

```bash
#!/bin/sh
set -eu

PACKAGE_ROOT=/opt/agent/package
DATA_UPPER=/opt/agent/data
OVERLAY_WORK=/opt/agent/data/.overlay_work
AGENT_ROOT=/opt/agent/rootfs

mkdir -p "$DATA_UPPER" "$OVERLAY_WORK" "$AGENT_ROOT"

mount -t overlay overlay \
  -o lowerdir="$PACKAGE_ROOT",upperdir="$DATA_UPPER",workdir="$OVERLAY_WORK" \
  "$AGENT_ROOT"

exec /opt/agent/opendan \
  --appid "${OPENDAN_AGENT_ID}" \
  --agent-root "$AGENT_ROOT" \
  --agent-bin  "$PACKAGE_ROOT"
```

说明：

- 如果容器内不允许 `mount -t overlay`，可退化为 `fuse-overlayfs`
- 如果连 `fuse-overlayfs` 也不可用，entrypoint 应退化为把 `package_root` 递归播种到 `data_upper`，
  不保留时间戳（macOS / OrbStack 一类卷后端上 `utimens` 可能被拒绝）
- 这个递归播种行为与当前 `sync_agent_rootfs_from_package` 一致——把"启动时拷贝 + sha256 保护本地修改"
  当作 OverlayFS 不可用时的过渡实现

### 10.8 Host / node_daemon 的职责

- 准备只读 Package 来源
- 准备 Agent 实例级 Data volume（即宿主机视角的 AgentRootFS）
- 注入 `appid` / `owner_id` / `service_port` 等实例参数
- 管理容器启停、日志、健康检查

容器启动后，entrypoint 负责生成真正的 `agent_root`，再把该路径传给 `opendan`。

### 10.9 docker run 示意

```bash
docker run \
  --cap-add SYS_ADMIN \
  -e OPENDAN_AGENT_ID=jarvis \
  -e BUCKYOS_OWNER_USER_ID=alice \
  -e BUCKYOS_ROOT=/opt/buckyos \
  -v jarvis_data:/opt/agent/data \
  -v buckyos_tools:/opt/buckyos/tools \
  opendan/agent-runtime:latest
```

`--cap-add SYS_ADMIN` 是容器内直接使用内核 OverlayFS 的最小权限要求。若安全策略不允许，可评估：

- `fuse-overlayfs`
- 预先在镜像构建期生成基础层，运行期只做 volume 覆盖

但这两种都属于兼容实现，不是主设计。

### 10.10 Package 更新语义

**默认语义：下次容器重启生效**——更新 `Package` 后，新容器实例会看到最新 lowerdir；已 copy-up 到
`Data` 的文件继续优先使用实例自己的版本。不承诺"运行中立刻热切换到新 Package"。

如果未来需要在线热切换，应单独定义 remount / restart 策略，而不是把它作为当前基础合同的一部分。

---

## 11. 兼容性与边界

需要区分两个阶段：

- **启动阶段**：为了兼容历史部署数据，`opendan main` 仍会从多个 CLI/env/spec/doc 字段里解析
  `agent_root` 与 `agent_package_root`（见 §8.1）
- **运行阶段**：一旦实例已经启动，session / workspace / 路径解析必须遵守 §6 的确定性读取规则，
  不再做祖先扫描或多 key 猜测

因此：

- 本规范不保留运行期路径解析的向前兼容
- 历史字段兼容只允许保留在启动入口，不应扩散到 workshop / session / path resolver 的新实现中
- `agent_env_root` 与 `--agent-env` 视为 `agent_root` / `--agent-root` 的别名，新代码不要再产生

---

## 一句话总结

> AgentRootFS 是宿主机上的平台无关数据目录，定义了 §2 的标准布局；4 层 Bin 把"工具声明"（持久化在
> AgentRootFS）和"执行视图"（容器临时渲染）严格分离；COW 由容器内的 Linux OverlayFS 实现，host 只
> 负责编排容器；`opendan` 进程不理解 Docker，只消费一个准备好的 `--agent-root`。
