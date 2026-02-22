# Agent Workshop 需求

## 1) Workshop 需求（Agent 私有工作区）

### 1.1 定义与目标

* **Workshop 是 Agent 私有工作区**：用于承载 Agent 自身的工具、技能、会话数据、以及其私有的本地工作空间集合。
* 目标：

  * 给 Agent 提供一个“可持续、可回收、可审计”的私有落盘空间；
  * 作为 **Workspace 管理与交付的控制面**：集中维护 workspace 元信息、索引、UI 数据源、以及写入审计（diff + 归因）。

### 1.2 范围与非目标

* **包含**

  * Workshop 目录结构与元信息管理
  * 创建/挂载/登记 local workspace 与 remote workspace 的管理能力（不负责 remote 的具体实现细节，remote 由 skill/connector 提供）
  * 文件写入的 diff 记录与任务归因（可观测性）
  * Git 协作桥接（至少对接基础 git 操作的“框架/入口”）
* **不包含（本需求明确排除）**

  * TODO 的创建、分解、状态流转、依赖关系、UI 交互与增量协议等（会由单独文档列出）

### 1.3 目录与数据结构要求

**P0（必须）**：提供稳定、可预测的目录布局（用于工具链与提示词模板引用），至少包含：

* `/<agent_root>/workshop/tools/`：工具定义与脚本
* `/<agent_root>/workshop/skills/`：技能规则/配置
* `/<agent_root>/workshop/sessions/`：会话运行态数据与摘要归档入口（具体 session 数据结构由 Session 模块定义）
* `/<agent_root>/workshop/workspaces/`：所有已登记 workspace 的管理目录（包含 local 与 remote）


**P0（必须）**：Workshop 元信息文件（建议 `workshop/index.json` ）记录：

* `agent_did`
* `workspace_id` 列表及其类型（local/remote）
* 每个 workspace 的 `owner`（agent-created / user-provided）、创建来源 session、创建时间
* 权限/策略引用（例如 policy profile id）
* 最近一次变更时间、最近一次同步时间（remote）
* 发生冲突/错误的标记与摘要（供 UI 与恢复使用）

### 1.4 生命周期管理

**P0（必须）**

* `create_workshop(agent_did)`：初始化目录与元信息
* `load_workshop(agent_did)`：加载元信息与索引
* `list_workspaces()`：列出 workshop 下登记的所有 workspace（含状态）
* `archive_workspace(workspace_id)`：归档 workspace（不一定删除物理数据；可改为只读、移出活跃索引）
* `cleanup()`（可被 self-improve 行为调用）：清理临时文件、过期缓存、失败的挂载点等（策略可配）

### 1.5 写入审计与可观测性（强约束）

文档要求“写文件必须记录 diff 与任务归因”，且所有重要动作要可追踪（TaskMgr/Worklog/Ledger）。

**P0（必须）**

* Workshop 提供统一写入入口（或写入拦截器），任何对 workspace 的写入都必须：

  1. 产生 **结构化 diff/patch**（或至少保存 before/after + 摘要）
  2. 关联归因：`agent_did / session_id / step_id / action_id(tool_call_id)`
  3. 写入 Worklog/Ledger（由 Ledger/Worklog 模块落库，但 Workshop 必须提供字段与调用点）
  4. 若写入过程较长/可取消，应挂到 TaskMgr，提供进度、取消、超时与日志（至少留下 Task 引用）

**P1（建议）**

* 支持“写入策略”：例如仅允许在特定目录写、限制单次 diff 大小、对二进制文件写入走特殊流程（产物指针而非内联 diff）。

### 1.6 Git 协作桥接（Workshop 侧能力）

文档明确 Workshop 需要“对接 Git：commit/PR/issue 模板、冲突记录与提示”。

**P0（必须）**

* 为 workspace 提供 git 能力入口（可通过内置 git tool 实现）：

  * 初始化 repo / 检测 repo 状态
  * 生成 commit（commit message 模板化，包含 session/step 归因）
  * 冲突检测与记录（至少把冲突摘要写入 worklog 并标红状态）

**P1（建议）**

* PR/Issue 模板化：提供模板生成器（不要求绑定某个平台 API，但应提供扩展点）
* 将冲突记录与修复建议写入可观测 UI 数据源

### 1.7 与其他模块的接口约束

**必须对齐**（来自文档系统边界与运行机制）：

* FileSystem/NameStore：所有路径/命名/权限落到系统文件服务的抽象上
* PolicyEngine：写入范围、是否允许 git/网络、子 Agent 的 fs_scope 等必须可被策略收敛
* TaskMgr：长任务与可取消任务必须入 TaskMgr
* Session Loop：session 可绑定 workspace，step 结束后会追加 worklog（Workshop 需提供可追加的落点/索引）

---

## 2) Local Workspace 需求（本地交付/开发空间）

### 2.1 定义与目标

* **local_workspace 是一种“本地目录型 workspace”**，通常位于 Agent 的 Workshop 内，由 Agent 私有使用；也允许用户先创建本地目录再交给 Agent 使用（此时语义上更偏“用户资产”）。
* 用途：代码/文档/数据在本地落盘，供 bash action、文件 diff 写入、测试等操作使用。

### 2.2 会话绑定约束（强约束）

* **Session 可以绑定 0 个 local_workspace 和 0..n 个（其他）workspace**。

  * 含义：local_workspace 更像“本次会话的主工作目录（cwd）”，而其他 workspace 可作为交付目标、镜像或参考输入。
* local_workspace 与 session 的绑定信息必须可恢复（重启后可继续执行 behavior step）。

### 2.3 并发与锁（必须实现）

文档明确要求：多个 session 可并行运行，但**同一个 local_workspace** 不能被多个 session 同时 RUNNING。

**P0（必须）**

* 为每个 local_workspace 提供互斥锁（建议可重入但绑定 session）：

  * `acquire(local_ws_id, session_id)`：成功则 session 可进入 RUNNING
  * `release(local_ws_id, session_id)`：step 或 session 结束/暂停时释放
* 调度器策略：

  * 如果 session READY 但拿不到锁：session 必须保持 WAIT/READY（实现可选）而不是进入 RUNNING
  * 锁必须在崩溃恢复场景下可回收（例如基于租约 TTL，或由 runtime 恢复流程清理）

**P1（建议）**

* 锁策略可升级为“工作项级别的顺序性约束”（文档提到可以等待另一个 session 的工作完成以避免非顺序修改）。但**工作项/待办完成判定属于 TODO 模块范围**，这里只要求留出接口点，不定义其语义。

### 2.4 权限与安全（Policy）

**P0（必须）**

* local_workspace 必须纳入 PolicyEngine 的 fs permissions：

  * root agent 的读写范围（默认只允许其 workshop/workspaces 下的目录）
  * subagent 的 fs_scope（通常更小，且必须白名单继承）
* 与 bash action 结合时：

  * bash 的 `cwd` 必须能指向 local_workspace（或其子目录）
  * bash 是否允许网络/可执行权限由 policy gate 控制（Workshop 只提供能力入口，不绕过 gate）

### 2.5 结构与操作接口（建议最小集合）

**P0（必须）**

* `create_local_workspace(name, template?, owner)`：创建目录并登记到 Workshop
* `bind_local_workspace(session_id, local_ws_id)`：绑定到 session（写入 session.state 的 workspace_info）
* `get_local_workspace_path(local_ws_id)`：返回本地路径（供 ActionExecutor / 工具使用）
* `snapshot_metadata(local_ws_id)`：记录基本统计与状态（大小、文件数、最近改动时间）

**P1（建议）**

* `reset/clean(local_ws_id, mode)`：清理构建产物、临时文件（可被 self-improve 调用）
* `export_artifacts(local_ws_id)`：将指定产物“提升”为可交付对象（例如发布到 remote workspace 或 Content Network；发布策略与权限仍走 Policy）

### 2.6 可观测性

**P0（必须）**

* local_workspace 的关键事件必须可追踪：

  * 创建/绑定/释放锁/归档
  * 文件写入 diff 的聚合索引（由 Workshop 统一写入审计实现）
  * 与 session/step 的归因链路可追溯（便于 UI 展示“这次改动是谁在第几步做的”）

---

## 3) Remote Workspace 需求（远程交付空间：Git/SMB/服务等）

### 3.1 定义与目标

* Workspace “不必由 runtime 管辖”，可以是 **远程 git repo、SMB 共享、甚至公网服务**，通过 skills 扩展接入。
* Remote workspace 的核心目标：

  * 作为“交付成果的最终落点”（对外协作、代码托管、共享目录、发布服务等）
  * 让 Agent 能在 Policy 护栏内执行：同步、提交、发布、回滚（能力分级）

### 3.2 类型与统一抽象（必须有扩展点）

**P0（必须）**

* Remote workspace 必须通过“统一抽象接口 + 可插拔实现（skills/connectors）”接入：

  * 不能把 remote workspace 只做成“git 专用”，否则违背文档中“workspace 可多形态”的定位
* 统一接口建议至少覆盖（按能力声明）：

  * `list(path)`
  * `read(path)`
  * `write(path, patch_or_blob)`（是否允许写由 capability + policy 决定）
  * `sync(direction)`：pull/push 或 fetch/publish
  * `status()`：连接状态、最近同步点、冲突/错误信息

**P0（必须）**：每个 remote workspace 需要 capability 声明，例如：

* `READ_ONLY`
* `READ_WRITE`
* `VERSIONED`（如 git）
* `PUBLISHABLE`（如公网发布）
* `EXECUTABLE`（一般应为 false，远程通常不执行）

### 3.3 认证与密钥管理（安全红线）

**P0（必须）**

* Remote workspace 的认证信息不得明文落盘在 workspace 目录：

  * 使用系统 Secret/Key 引用（例如 `credential_ref`），由 PolicyEngine 控制访问
* 网络访问必须受 policy gate 控制（允许/禁止、白名单域名、预算/频率限制）。
* 所有远程写入/发布行为必须进入 Ledger/Worklog，可审计且可追溯到 session/step。

### 3.4 Git 型 Remote Workspace（建议作为 P0 支持的第一个实现）

文档对 Git 桥接有明确期待（commit/PR/冲突）。

**P0（必须）**

* 支持将 remote git repo 作为 remote workspace：

  * clone 到本地缓存目录（缓存目录属于 Workshop 管控范围）
  * pull/fetch、push
  * 将本地改动（通常来自 local_workspace）同步到 remote（可采用“本地工作区 -> git 缓存 -> push”的两段式）
  * 冲突检测：产生冲突摘要、记录到 worklog，并把 remote workspace 置为 `CONFLICT` 状态

**P1（建议）**

* PR/Issue：提供平台适配扩展点（GitHub/GitLab 等），但默认实现可只做到“生成模板文件 + 提示用户/上层工具处理”。

### 3.5 SMB/共享盘/其他服务型 Remote Workspace（作为 P1/P2 扩展）

**P1（建议）**

* 支持 SMB/共享目录类 remote workspace（典型：企业内网共享交付）

  * mount/umount 的生命周期管理
  * 断线重连、只读降级
* 支持“服务型 workspace”（HTTP API、对象存储、发布平台）

  * 以 connector/skill 方式实现
  * 由 capability 决定是否可写、是否可发布

### 3.6 一致性、同步与状态机

**P0（必须）**

* remote workspace 至少应有状态：

  * `UNBOUND`（未登记）
  * `READY`（可用）
  * `SYNCING`
  * `ERROR`（含错误码与可读摘要）
  * `CONFLICT`（如 git 冲突）
* `sync()` 必须可观测：

  * 短同步可直接返回结果
  * 长同步必须挂 TaskMgr（可取消/超时/日志）

### 3.7 与 Session/Workshop 的关系

**P0（必须）**

* session 允许绑定 0..n 个 workspace（其中可包含 remote workspace）。
* Workshop 必须记录：

  * remote workspace 的 connector 类型、remote 标识（repo url / share path / service endpoint）
  * 本地缓存位置（如有）
  * 最近同步点与状态摘要
* remote workspace 不强制参与 local_workspace 的互斥锁；但若 remote 的本地缓存目录会被多个 session 写入，必须复用同样的锁机制（锁粒度可配置为 “workspace-level lock group”）。

---

###  “验收口径”（简版）

* **Workshop**：能创建/加载；有固定目录结构；能登记 workspace；任何写文件有 diff + session/step 归因 + 可审计；可挂 TaskMgr；有 git 桥接入口与冲突记录。
* **Local Workspace**：能创建/绑定到 session；同一 local_ws 不允许多个 session 同时 RUNNING（锁可恢复）；bash 的 cwd 可指向；权限受 policy 控制；改动可追溯。
* **Remote Workspace**：统一抽象接口 + connector 扩展；支持至少 git remote（clone/pull/push/冲突）；认证走 secret 引用；网络与写入受 policy；sync 可观测可取消。
