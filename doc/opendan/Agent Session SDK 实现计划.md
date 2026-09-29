# Agent Session SDK 实现计划

**项目：OpenDAN / libOpenDAN**
**版本：0.8（草案）｜日期：2026-09-29**
**需求基线：** [《Agent Session SDK 化核心需求》](<Agent Session SDK化核心需求.md>)（下文简称“需求”，条目编号 S-xx / A-xx 沿用）

**现状依据：**
- [Agent Session](<Agent Session.md>)、[Agent RootFS](<Agent RootFS.md>)、[LLM Context 设计](<LLM Context 设计.md>)、[xllm Rust SDK](../agent_tool/xllm_rust_sdk.md)、[kmsg](../arch/kmsg.md)、[NewOpenDANRuntime](../../notepads/NewOpenDANRuntime.md)
- [agent-did-object-lib 需求](../../notepads/Agent元能力/agent-did-object-lib%20需求.md)、[Agent DID-Object Protocol Spec](../../notepads/Agent元能力/Agent%20DID-Object%20Protocol%20Spec.md)、[Agent Memory v2](../../notepads/Agent元能力/Agent%20Memory%20v2.md)
- 当前代码（2026-09-29）：`src/frame/opendan`、`llm_context`、`agent_tool`、`kernel/buckyos-api`

> **v0.8 变更**
>
> 1. **Q12 已定**：DFS 在设计上基本是独占单写（只有一个 client 能拿到文件写锁），原子语义通常强于单机系统。因此：
>    - §9.1 的文件原语全部满足；
>    - 锁直接建立在“文件写锁”上：单节点用 flock，多节点用 DFS 独占写锁，两种环境下 fencing 都是严格的；
>    - 去掉 `lease_mode`、OpenDAN 锁服务和 service 模式的安全余量（§5）。
> 2. **Q14 / Q16 已定**：msg-center 的 inbox 正在改为以 `did + session_id` 标识，每个 UI session 有自己的 inbox，权限可以正确区分。
>    - UI session 直接消费自己的 inbox，OpenDAN 不再转投消息。
>    - session 的身份与权限由驱动它的进程的 appid 决定；回复消息、调用 LLM 都用这个身份（§4.5）。
> 3. 待确认问题清零（§15.1）。
>
> **v0.7 变更**
>
> 1. **llm_context 的构建与读取策略**：先读 `summary.json`，再从 `state.json` 记录的已提交末尾**反向读** `worklog.jsonl`，读到起点或预算用尽就停止。运行时从不全量扫描 worklog（§4.4）。
> 2. **Q8**：session 类型沿用 `work`，不改名为 task。
> 3. **Q12**：计划上线的 DFS 支持 ACL，由 BuckyOS 管理权限。
> 4. **Q13**：session 里的产物通常是一次性的，长期产物放 workspace，于是问题变成“Agent 内部 workspace 与外部 workspace”之分；**AgentSession 协议不承担 workspace 版本化的复杂度**。因此删除 v0.6 的交付方式与版本化策略（git_worktree / staged_copy / in_place），Agent State 的产物列表只负责登记与指向（§6.5）。
> 5. **Q14**：inbox 的读取权限由 BuckyOS 权限机制决定。
>    - OpenDAN 主进程有权读 Agent 的各类 inbox。
>    - 其它 runner 通常创建 work session。
>    - 如果权限允许某个 app 读取特定 inbox，这个 app 也可以创建并驱动对应的 UI session，也就是企业软件集成 Agent Chat Box（§4.5、§10.2）。
>
> 历史：v0.4 采用你整理的 session 目录，输入改走 kmsg；v0.5 明确 summary.json 只服务于压缩；v0.6 确定 `state.json` 为提交点、worklog 严格只追加。

---

## 0. 结论先行

本期把“以某个 Agent 身份推进一个 Session”的能力从 opendan 进程中剥离出来，做成独立的 **libOpenDAN**：

- **Agent Session**：自包含、与位置无关的目录协议。任何进程在 DFS 的任意位置按协议建出 session 目录，就能以某个 Agent 的身份工作。
- **Agent State**：Agent 跨 session 的状态，落在 AgentRoot 上，包括 session 登记表（session mgr）、感知、认知和产物列表。
- **Agent Runtime**：为 `exec_bash` 提供执行环境（native 或 tmux）。session 首次推进时绑定 runtime，之后不再改变。
- **Session Runner**：把以上三者与 LM Context 组合起来，把 session 推进到结束条件。

opendan 服务本期不动；它将来依赖 libOpenDAN，只保留托管职责（附录 A）。

**关键决策**

| # | 决策 | 主要对应 |
|---|---|---|
| D1 | 文件即真相。持久格式只用 JSON / JSONL / Markdown；SQLite 只做可删除、可重建的派生缓存 | S-03、跨语言 |
| D2 | **Session 目录**：<br>- 目录本身存放产物（通常一次性）；<br>- `.opendan_agent_session/` 存放状态；<br>- `.runtime/` 存放 runtime 配置。<br>只通过 `agent_did` 引用 Agent，位置不限 | 你的目录设计 |
| D3 | **状态共享走 DFS**（ACL 由 BuckyOS 管理），不为访问 session 开 RPC。Agent 默认能看到自己的全部 session，可以配置为只看状态摘要 | S-03、Q10、Q12 |
| D4 | **Session 登记表**是发现 Agent 全部 session 的唯一入口。驱动者每次提交后回报状态摘要（rev 单调），并追加感知 | S-14、Q9 |
| D5 | **Session 绑定驱动者身份**，同身份的不同进程可以接手。采用单写者分区 + lease（带 fencing token）；锁建立在文件写锁上：单节点用 flock，多节点用 DFS 独占写锁，fencing 都是严格的 | S-22、A-14、Q9、Q12 |
| D6 | **输入走 BuckyOS 基础设施**：<br>- 每个 session 一条 kmsg 队列，承载 msg / event / change / control / perception；<br>- UI 消息来自 msg-center inbox（以 `did + session_id` 标识，每个 UI session 一个），谁获准读这个 inbox，谁就能驱动对应的 UI session；<br>- kevent 负责唤醒。<br>“进入推理 = commit pop”落在各输入源的确认上 | S-15 ~ S-19、S-23、Q14 |
| D7 | **`state.json` 是提交点**（小文件，整体原子替换）。**`worklog.jsonl` 严格只追加**：每次 llm_context run 结束时批量写入该 run 的历史，随后由 state.json 提交确认 | S-23 |
| D8 | **下一次 llm_context 的构成**：<br>- 有未结束的 run，就从 `runs/` 直接恢复；<br>- 否则先读 `summary.json`（历史摘要 + 起点 + 机械压缩配置），再从末尾反向读 worklog，读到起点为止。<br>运行时从不全量扫描 worklog | S-04、Q5 |
| D9 | 首次推进时绑定 runtime（含 workdir），之后不变；绑定失败发生在任何推理之前，没有成本 | S-07、Q3 |
| D10 | **产物**：session 产物放在 session 目录，通常一次性；长期产物放 workspace（Agent 内部 / 外部）。workspace 的版本化与回滚**不进 Session 协议**；Agent State 的产物列表只登记和指向 | S-24 ~ S-26、Q13 |
| D11 | 崩溃恢复**复用 llm_context 的 checkpoint / resume**，最细到一次 do-action 之后 | S-04、A-03、Q5 |
| D12 | session 的身份与权限由**驱动它的进程的 appid** 决定：读 inbox、回复消息、调用 LLM 都用这个身份。外部 runner 即 App 身份，OpenDAN 托管时即 OpenDAN 的 appid | S-08、Q7、Q16 |
| D13 | libOpenDAN 提供 DID Object 宿主组件，面向 Agent 的 `read` / `xcall` 和外部访问（经 cyfs-gateway），**不是** runner 的数据通路 | Q6 |

---

## 1. 范围、已定决策与假设

### 1.1 你给出的思路（复述）

新建工程 libOpenDAN，原则上其中的代码都要能跨语言实现。它包括三部分：

1. **基于文件系统的 Agent State**（跨 session 状态）：session mgr、感知管理、认知管理、产物列表。
2. **Agent Session**：用协议级文档确定目录结构，实现创建和读取；本计划补上了推进与恢复。session 目录可以放在 AgentRoot 之外。
3. **Agent Runtime**：核心是为 `exec_bash` 提供环境，可以是 OS native，也可以是(容器里)的 tmux session。session 一旦确定了 runtime，就不再修改。

OpenDAN 的后续职责（本期不实施，见附录 A）：

- 基于 libOpenDAN，提供 Agent State 的 DID Object 访问服务和分布式锁（v0.8：锁改由 DFS 独占写锁承担，见 §5）。
- 创建默认 Agent Runtime（paios 容器内 Agent 的私有 infra）。
- 用协程托管默认 session：基于 msg-tunnel 的 UI session，以及定期运行的 self-improve。

### 1.2 本期范围

| 本期交付 | 本期不做 |
|---|---|
| `src/frame/libopendan` crate：<br>- Session 目录协议<br>- Agent State<br>- Runtime<br>- Runner<br>- 输入通道（kmsg / msg-center）<br>- 锁（基于文件写锁：flock / DFS）<br>- DID Object 宿主组件 | 修改 opendan（附录 A 只描述对接方式） |
| llm_context 的可选钩子（现有调用方的行为不变） | `agent_tool` CLI 改依赖（随 opendan 切换一起改） |
| 协议文档、fixtures；libopendan 的开发 CLI（cargo example） | msg-tunnel UI session 的发现与托管、self-improve 定时、agent.delegate 托管（属于 OpenDAN） |
| TS 版 State / Session 客户端（T1） | **workspace 的版本化、合并与回滚**（Agent 内部 / 外部 workspace，另行设计）；Memory Graph 内部算法；提示词 |

### 1.3 已定决策（2026-09-29 多轮反馈）

| 项 | 结论 |
|---|---|
| 范围 | 本期只做 libopendan，不动 opendan；opendan 之后依赖 libopendan |
| Session 位置与目录 | 可以放在 AgentRoot 之外；目录结构按你的整理（§4.1） |
| inbox / outbox / changes | 用 BuckyOS 基础设施（message center、kmsg 等），不用文件系统重新实现 |
| summary.json | 只为 session history 压缩准备：历史摘要 + 起点 + 机械压缩配置 |
| state.json / worklog | 用两个文件：state.json 是提交点；worklog 严格只追加，每次 run 结束时批量写入 |
| llm_context 构建 | 先读 summary.json，再从末尾反向读 worklog（§4.4） |
| Q1 self-improve 启动 | 属于启动条件检查，至少要求 Agent 空闲（附录 A.5） |
| Q2 本机直写 | 同意。数据一律直接读写文件（DFS）；只有多节点下的锁需要走服务 |
| Q3 绑定时机 | 首次推进时绑定；启动失败发生在首次推理之前，没有成本 |
| Q4 + Q13 产物 | session 产物通常一次性；长期产物放 workspace（Agent 内部 / 外部），其版本化不由 Session 协议承担。默认 paios runtime 内置 git，可供 Agent 内部 workspace 使用 |
| Q5 崩溃恢复 | 复用 llm_context 的崩溃恢复策略，最细到一次 do-action 之后 |
| Q6 访问入口 | 正式的 DID Object 协议访问由 cyfs-gateway 路由到 OpenDAN |
| Q7 LLM 身份 | 用 App 身份 |
| Q8 命名 | session 类型沿用 `work`（Work Session），保持惯性 |
| Q9 登记与推进者 | 需要记录 session 上次被哪个 runner 推进；推进身份不变，但两次推进可以发生在不同进程 |
| Q10 可见性 | Agent 原则上能看到自己的所有 session，也可以通过配置拒绝；状态共享通过 DFS 实现，App 不需要开 RPC |
| Q12 DFS | 计划上线的 DFS 支持 ACL（由 BuckyOS 管理权限）。它在设计上基本是独占单写：只有一个 client 能拿到文件写锁，原子语义通常强于单机系统。因此 §9.1 的原语全部满足，锁直接使用 DFS 写锁 |
| Q14 输入权限 | msg-center 的 inbox 改为以 `did + session_id` 标识（改造进行中），权限按 inbox 区分：<br>- OpenDAN 主进程可以读 Agent 的各类 inbox；<br>- 其它 runner 通常创建 work session；<br>- 获准读取某个 session inbox 的 app 可以创建并驱动该 UI session（企业软件集成 Agent Chat Box） |
| Q16 身份 | session 的身份与权限由驱动它的进程的 appid 决定；回复消息也以该身份发送 |

### 1.4 假设

- **A2**：需求引用的 `Agent Memroy 认知管理需求.md` 在仓库中不存在。感知 / 认知的内部 schema 以该文档为准，本计划只定义接口面。
- **A3**：新协议不读取旧格式；opendan 切换时清理旧数据。
- **A5**：代码从 opendan **复制后改造**到 libopendan，过渡期两份并存（§12）。
- **A7**：`runns` 按 `runs` 处理；统计文件写作 `stats.json`。如果 `static.json` 是有意的命名，定稿时改回即可。
- **需求术语映射**：需求里的 Task Session 即本文的 Work Session（`kind = work`）。

---

## 2. 总体架构

### 2.1 分层与部署形态

```text
 应用进程（任意语言；App 身份）                      OpenDAN（后续版本，附录 A；OpenDAN 的 appid）
 ┌───────────────────────────────────┐            ┌─────────────────────────────────────┐
 │ libOpenDAN                        │            │ libOpenDAN ＋ 协程托管                │
 │  SessionRunner ─ LM Context       │            │ DID Object 宿主组件 ◄─────────────────┼── cyfs-gateway
 │  Runtime（native | tmux）          │            │ 发现新的 UI session inbox            │
 └──┬─────────┬──────────┬───────────┘            └──────────────────┬──────────────────┘
    │ 文件     │ 文件      │ kmsg / msg-center / kevent                  │ 文件 / kmsg / msg-center
    ▼         ▼          ▼                                             ▼
 ┌──────────┐ ┌─────────────────────────────────────┐   ┌──────────────────────────────┐
 │ app data │ │ AgentRoot                            │   │ kmsg：每个 session 一条输入队列 │
 │ <sid>/   │ │ state/sessions/ state/perception/    │   │ msg-center：inbox(did+sid)     │
 │ 产物       │ │ state/artifacts/ memory/ notebook/   │   │ kevent：唤醒通知               │
 │ .opendan_│ │ sessions/<sid>/ workspace/ .locks/   │   └──────────────────────────────┘
 │ agent_   │ └─────────────────────────────────────┘
 │ session/ │   （以上目录都在 DFS 上，ACL 由 BuckyOS 管理；单节点部署时就是本地文件系统）
 └──────────┘
```

| 层 | 组件 | 回答的问题 | 需求 §2.3 |
|---|---|---|---|
| L1 | AICC / 裸模型 | 一次模型输出 | 裸模型推理 |
| L2 | `llm_context`（TS 版为 xllm） | 一次上下文推理循环：工具、快照、resume | LM Context |
| L3 | **libOpenDAN** | 以某个 Agent 的身份，把一个 Session 推进到结束条件 | Agent Session |
| L4 | OpenDAN 服务（后续） | 让 Agent 常驻：收消息、定时自省、对外服务 | 长期 Agent 服务 |

依赖方向：`llm_context` ← `agent_tool` ← `libopendan` ← `opendan`（后续）。libopendan 通过 `buckyos-api` 使用 kmsg / msg-center / kevent / AICC（放在 `buckyos` feature 中），测试时可以换成内存实现。

### 2.2 协议对象之间的关系

- **Session 目录 ↔ Agent State**：创建时登记；驱动者每次提交后回报状态摘要、追加感知；读取认知线索；登记产物。这些都是对 DFS 共享文件的直接读写。
- **Session ↔ 输入源**：kmsg 输入队列（所有 session 都有）＋ 可选的 msg-center inbox（UI session 自己的 inbox，`did + session_id`）。只有持有 lease 的驱动者会消费输入。
- **Session ↔ workspace**：`session_config` 引用一个 workspace（Agent 内部或外部），`binding.json` 记录解析后的 workdir。workspace 自身的管理不属于 Session 协议。
- **Agent → 任意 session**：通过登记表找到位置，直接读取目录。默认允许；session 可以配置为只给状态摘要（`acl.agent_access`，由 DFS ACL 执行）。

### 2.3 访问方式

| 场景 | 数据 | 输入 | 锁 |
|---|---|---|---|
| 单节点（当前形态） | 直接读写本地文件 | kmsg / msg-center + kevent | 文件写锁 = flock |
| 多节点 | 直接读写 DFS 文件 | 同上 | 文件写锁 = DFS 独占写锁（只有一个 client 能拿到） |
| 应用驱动 work session（最常见） | 同上 | 应用自建的 kmsg 队列 | 同上 |
| 应用驱动 UI session（企业软件内嵌 Agent Chat Box） | 同上 | 获准读取的 session inbox（`did + session_id`）+ kmsg 队列（control 等） | 同上 |
| runtime 内的子进程 | Agent 状态只读 | 向本 session 的 kmsg 队列投递（如 perception） | 不持有 |

两种环境下锁的语义相同：lease 协议（§5）只依赖“文件写锁”这一个原语，fencing 都是严格的。

### 2.4 crate 结构

```text
src/frame/libopendan/
  Cargo.toml            # features: default=["local"]，另有 "buckyos"（kmsg / msg-center / kevent / AICC）、"host"
  src/
    lib.rs
    protocol/           # 所有磁盘 / 消息结构（serde + schemars 导出 JSON Schema，与协议文档同源）
    fsutil.rs           # atomic_replace / append_batch / truncate_to / reverse_lines / publish_noreplace / publish_dir / write_lock（flock / DFS 写锁）
    store/              # StateFs trait；LocalFs（本地路径或 DFS 挂载）
    lease.rs            # LeaseManager：FileLeaseManager（基于文件写锁：单节点 flock，多节点 DFS 写锁）
    session/            # SessionDir：config / state / worklog / summary.json / runs / stats / binding / lease
    channel/            # InputChannel：KmsgInput、MsgCenterInput、内存实现；OutboundSink：MsgCenterOutbound；Waker：KEvent
    state/              # AgentState：registry、perception、cognition（门面）、artifacts（产物列表）
    runtime/            # AgentRuntime trait、native、tmux、bin overlay、paths
    runner/             # drive、assembler、next_llm_context、tools、deps
    host/               # DID Object 宿主组件（host）
  examples/session.rs   # 开发 CLI：create / run / read / post / decide
  tests/
```

依赖都已在 workspace 中，**不新增第三方依赖**：

- 核心：`llm_context`、`agent_tool`、`fs2`、`uuid`、serde 系列。
- `buckyos` feature：`buckyos-api`（`MsgQueueClient`、`MsgCenterClient`、`KEventClient`、AICC）。
- `host` feature：`buckyos-http-server`、`name-lib`、`name-client`。

---

## 3. 设计原则（硬约束）

1. **协议先行**：目录布局、提交顺序、lease 语义、输入消息格式都是契约；Rust 是参考实现。
2. **位置无关**：`.opendan_agent_session/` 里不出现 AgentRoot 路径；与主机相关的信息只出现在 `binding.json` 和 `.runtime/`。
3. **共享靠文件，投递靠消息基础设施**：状态读写走 DFS；多方投递走 kmsg / msg-center；唤醒走 kevent。不在文件系统上重新实现队列，也不为 session 开 RPC。
4. **单写者分区**：`.opendan_agent_session/` 中的文件只由 session lease 的持有者写入。
5. **worklog 严格只追加、只反向读**：已提交部分从不改写，恢复时只截断未提交的尾部。运行时从末尾反向读到需要的位置为止；全量读取只用于审计 / 导出工具。
6. **先提交，后回报与通知**：回报失败的，下次 drive 时补发。
7. **身份不来自提示词**：runner 能做什么，只由驱动者身份、BuckyOS 权限（DFS ACL、inbox 读权限）和 lease 决定（S-08）。
8. **崩溃恢复不另起炉灶**：遵循 llm_context 的 checkpoint / resume 纪律。
9. **Session 协议不承担 workspace 的复杂度**：长期产物的版本、合并与回滚属于 workspace（Q13）。
10. **组合优于发明**：复用 `llm_context`、xllm 的 run 布局与 `.llm_context` 配置、`agent_tool`、kmsg / msg-center / kevent、buckyos-base 的 `DIDObjectServer`。
11. **本期不动 opendan**。

---

## 4. Agent Session 目录协议（libOpenDAN::session）

> L0 阶段整理为独立协议文档 `doc/opendan/protocol/Session Directory Protocol.md`。

### 4.1 目录结构

```text
<any_parent>/<sid>/                  # session 本身的目录：保存 session 里的产物（通常是一次性的）
  readme.md                          # 人读：标题 / 目标 / 起源（进入环境上下文）
  report.md                          # 人读：阶段 / 最终报告
  <产物…>                            # 一次性产物；需要长期保存的放到 workspace（§6.5）

  .opendan_agent_session/            # 保存 session 状态（只由 lease 持有者写）
    session_config.json              # 启动配置：runtime 配置、提示词配置（.llm_context 的超集）、订阅列表、输入通道、app 扩展项；
                                     #   大部分不可在运行中修改；单文件，每次整体原子替换
    state.json                       # 会话状态（提交点）：run_state / acceptance / 当前 run / 输入与订阅游标 / worklog 已提交边界；
                                     #   小文件，整体原子替换
    runs/<run_id>/                   # 恢复 llm context 用（沿用 xllm run 布局：run.json + snapshots/NNNN.json）；
                                     #   原则上只保留最后一个，llm context 结束即删除
    summary.json                     # 重建 llm context 的核心状态 ①：为 history 压缩准备（历史摘要 + 起点 + 机械压缩配置）
    worklog.jsonl                    # 重建 llm context 的核心状态 ②：严格只追加的工作日志（通常是最大的文件）；
                                     #   每次 llm_context run 结束时批量写入该 run 的历史
    stats.json                       # 统计信息（token、轮数、耗时、费用）
    lease.json                       # session 推进权 lease（检查-修改在文件写锁内完成：flock / DFS 独占写锁）
    binding.json                     # 首次推进时写入：runtime + workdir（只写一次）

  .runtime/                          # runtime 相关配置（与绑定的 runtime 相关，不要求可迁移）
    bin/                             # session 级工具（进入 PATH 的 Session Bin）
```

**位置无关规则**：

1. session 目录位于 DFS 上任意父目录下，目录名等于 `session_id`。
2. `.opendan_agent_session/` 中的路径都相对于 session 目录；只通过 `agent_did` 引用 Agent。
3. 与主机相关的信息只出现在 `binding.json` 与 `.runtime/`，它们随 runtime 绑定（D9）。
4. session 未被持有时可以整体移动；移动后由驱动者更新登记表的 location。
5. `session_id` 在一个 Agent 内全局唯一，登记时由 `publish_noreplace` 检测冲突。

旧文档要求“session 目录不放 `bin/`”，`.runtime/bin` 与这条约束不冲突。旧约束的出发点是 session 数据要能跨平台迁移；在新协议中，需要迁移的是 `.opendan_agent_session/` 与产物，`.runtime/` 明确随 runtime 绑定。runtime 自身的临时文件（例如 tmux 的 exec 脚本和输出日志）仍然放在 runtime 的实例卷里。

### 4.2 session_config.json（启动配置）

```jsonc
{
  "schema": "opendan.session_config/1",
  "config_rev": 1,                                     // 运行中允许修改的少数字段（动态订阅等）变更时 +1；整文件原子替换
  "session": {
    "session_id": "work-20260929T101500-3f9c2a",
    "agent_did": "did:bns:jarvis.alice",
    "kind": "ui | work | self_improve | self_check",   // Q8：沿用 work
    "class": "work",                                   // agent.toml [session.<class>]，决定 loop_mode / driver 配置
    "created_at_ms": 0,
    "created_by": { "principal": "did:app:app2", "via": "app | ui_session:<sid> | opendan | task_mgr:<task_id>" },
    "driver": { "principal": "did:app:app2" },         // 推进身份 = 驱动进程的 appid，创建后不变（Q9、Q16）；为 OpenDAN 时由 OpenDAN 托管
    "idempotency_key": "…",
    "route_key": "msgtunnel:telegram:acc:chat42",      // 仅 UI
    "origin": { "parent_session": "ui-…", "intent_ref": "…", "reason_messages": ["…"] },
    "objective": "…",
    "end_condition": { "type": "llm_declares_done | output_schema | max_rounds", "detail": {} },
    "input_policy": "any | supplement_only | none",
    "acl": { "owner": "did:…", "readers": ["did:…"], "agent_access": "full | status_only" },   // Q10：默认 full
    "task_binding": { "task_id": "…" }                // 来自 agent.delegate 时（沿用）
  },
  "prompt": {                                          // .llm_context 的超集：保存合并后的有效配置
    "provider": { … }, "model": { … }, "loop": "behavior | function_call",
    "groups": { … }, "sections": { … }, "tools": { … },   // 与 xllm .llm_context 字段一致（凭据只存 SecretRef）
    "behavior": "plan",                                // OpenDAN 扩展：behaviors/<name> 引用（BehaviorAssembler）
    "system_prompt": "…",                              // 应用自带的 system prompt（S-05；按 §8.1 的固定顺序合成）
    "context": ["…"],                                  // 应用提供的初始上下文材料
    "mechanical_compress": { "recent_full_steps": 2, "summary_chars": 280, "max_result_chars": 4096 }   // summary.json 的初始机械压缩配置
  },
  "runtime": { "requirement": { "runtime_id": null, "tools": ["node"] }, "tool_plan": "minimal_safe", "env": { … } },
  "workspace": null,                                   // 可选：长期产物所在的 workspace（§6.5）
                                                       //   { "kind": "agent", "id": "ws-…" } | { "kind": "external", "path": "/…/app2/snake" }
  "artifact_id": null,                                 // 可选：在产物列表中登记为哪个长期产物（§6.5）
  "subscriptions": [                                   // 订阅声明（已感知游标在 state.json）
    { "id": "s1", "mode": "semi", "source": { "type": "session", "ref": "work-…" }, "watch": ["run_state", "outcome", "acceptance", "one_line_status"] },
    { "id": "s2", "mode": "active", "source": { "type": "object_event", "object": "https://…/cam01", "event": "motion" } }
  ],
  "channels": {
    "inputs": [
      { "id": "q", "kind": "kmsg", "queue": "app2::alice::opendan.session.work-…", "subscriber": "opendan.session.work-…" },
      { "id": "inbox", "kind": "msg_center", "did": "did:bns:jarvis.alice", "session_id": "msgtunnel:telegram:acc:chat42" }   // 仅 UI session：自己的 inbox（did + session_id，Q14）
    ],
    "outbound": { "kind": "msg_center" },                                        // 仅 UI session：以驱动进程的 appid 身份回复（Q16）
    "wake_event": "/opendan/<agent_id>/session/<sid>/input"                      // kevent：投递后发布，用于低延迟唤醒
  },
  "extensions": { "<app_id>": { } }                    // app 扩展配置项，libopendan 原样保留
}
```

**session_id 规则**（不新增依赖：时间前缀 + uuid v4 截断）：

| kind | id |
|---|---|
| ui | `ui-<sanitize(route_key)[..40]>-<hash8>` |
| work | `work-<yyyymmddThhmmss>-<6hex>`；带 idempotency_key 时为 `work-<hash(creator, key)>`（取代现在含空格的 `"YYYY-MM-DD <title>"`） |
| self_improve | `si-<yyyymmddThhmmss>-<6hex>` |
| self_check | `self_check` |

### 4.3 state.json：会话状态（提交点）

```jsonc
{
  "schema": "opendan.session_state/1",
  "rev": 17,                                         // 每次提交 +1；回报与版本比对都用它
  "writer": { "runner_id": "rn-…", "principal": "did:app:app2", "host": "did:dev:…", "pid": 1234, "lease_token": 42 },
  "run_state": "created | ready | running | waiting | finished",
  "waiting_for": null,                               // { kind: input|tool|event|children, refs: [], deadline_ms }
  "outcome": null,                                   // succeeded | failed | stopped
  "acceptance": "n/a",                               // n/a | pending | accepted | discarded（仅 work）
  "result": null,                                    // finished 时：{ answer_ref, artifact_ref, discard_report }
  "round": 12, "current_behavior": "do", "process_entry": "plan", "process_stack": [],
  "pending_task_calls": [], "bootstrap_done": true,
  "topic": { "title": "…", "tags": ["snake", "ui"] },
  "live_run": "20260929-101500-3f9c2a",              // 未结束的 run；null = 没有进行中的 llm_context
  "worklog": { "committed_seq": 340, "committed_bytes": 1048576 },   // 已提交边界；其后的尾部视为未提交，恢复时截断
  "inputs": {                                        // 按输入源分别记录消费进度（§4.5）
    "q":     { "acked_index": 118, "consumed_above": [121] },
    "inbox": { "reading": ["rec_…"] },                                // msg-center：已消费、尚未标记 Readed 的记录
    "recent_keys": ["msg:…", "…"]
  },
  "subscription_cursors": { "s1": { "rev": 15, "run_state": "running" }, "s2": { "key": "cam01#motion", "version": "evt_…" } },
  "perception_seq": 88, "reported_rev": 17,          // 已追加的感知序号、已回报登记表的 rev（用于补发）
  "one_line_status": "…", "last_error": null, "updated_at_ms": 0
}
```

- **提交顺序**：同一次提交中的其它内容（worklog 追加、report.md、runs checkpoint）都先于 `state.json` 写入。`commit_state` 总是把 worklog 当前末尾记为 `committed_*`。读者看到新的 rev 时，它引用的内容一定已经存在（S-23）。
- **其它参与方**获取状态时，默认读登记表的 `status`（§6.2）。

完成、停止、放弃用三个正交维度表达：

```text
run_state:  created ─► ready ◄──► running ◄──► waiting
                          │          │             │
                          └──────────┴──── stop ───┴──► finished { outcome: succeeded | failed | stopped }

acceptance (work):  n/a ─(finished)─► pending ─► accepted
                                         └─────► discarded ◄── accepted（事后放弃）
```

| 操作 | 合法前置 | 效果 |
|---|---|---|
| `stop` | run_state ∈ {ready, running, waiting} | 在下一个安全点结束：finished + outcome=stopped（沿用 Graceful 中断） |
| `accept` | work ∧ finished ∧ acceptance=pending | acceptance=accepted；产物列表把本 session 的版本置为有效（§6.5） |
| `discard` | work ∧ acceptance ∈ {pending, accepted} | acceptance=discarded；产物版本失效；对 workspace 的变更交给 workspace 处理，结果写入 `result.discard_report`；追加 `task_discarded` 感知 |

- 进入 `finished` 后不能再回到 running；追问或继续改进一律新建 work session（S-12）。
- stop / accept / discard **都由驱动者执行**（Q9）。其它参与方向该 session 的 kmsg 队列投递 `control(decide|stop)` 即可（§4.5）。
- 与现有状态的映射：Idle → ready，Running → running，WaitingInput / WaitingTool → waiting，Ended → finished，Error → ready + `last_error`。

### 4.4 worklog.jsonl、summary.json、runs/ 与下一次 llm_context

**worklog.jsonl** 是 session 的**严格只追加历史**，由现有的 round_history、topic_log 和 worklog 事件合并而来。每行一条，`seq` 单调。它不写状态，也从不改写；唯一允许的修改，是恢复时截掉 `state.worklog.committed_bytes` 之后的**未提交尾部**。

写入时机：

- **每次 llm_context run 结束时，批量追加该 run 的全部历史**：round_started、user_message、step、action_result、outcome。这通常是 worklog 最大的一次写入，只做一次 fsync。
- run 之外的少量条目（decide、input_rejected、compaction）在发生时追加。
- run 进行中的历史只在 `runs/`。
- 每次追加之后都要提交一次 state.json 作为确认。

```jsonc
{"seq":312,"t":"round_started","run_id":"20260929-101500-3f9c2a","round":12,"inputs":[{"src":"q","index":121,"key":"msg:…"}],"changes":["s1@16"],"hook":"on_wakeup","at_ms":0}
{"seq":313,"t":"user_message","run_id":"…","round":12,"content":"…"}
{"seq":314,"t":"step","run_id":"…","round":12,"behavior":"do","assistant":"…","actions":[{"call_id":"c-12-1","tool":"exec_bash","args":{…},"effect":"unknown"}]}
{"seq":315,"t":"action_result","run_id":"…","round":12,"call_id":"c-12-1","status":"ok","result":"…"}
{"seq":316,"t":"outcome","run_id":"…","round":12,"kind":"done","next_behavior":null}
{"seq":317,"t":"compaction","summary_start_seq":301}                  // 生成了新的 summary.json（审计用）
{"seq":318,"t":"decide","decision":"accept","by":"did:user:alice","report":{…}}
```

**summary.json** 专门为 session history 压缩准备：用“历史摘要 + 起点 + 机械压缩配置”描述下一次 llm_context 如何由 worklog 构成。

```jsonc
{
  "schema": "opendan.session_summary/1",
  "history_summary": "…",               // 起点之前全部历史的摘要（LLM 生成）
  "start_seq": 301,                     // 起点：从这条 worklog 开始使用原始记录
  "start_offset": 912384,               // 起点在 worklog 中的字节偏移：反向读取读到这里即停止
  "mechanical": {                       // 机械压缩配置：起点之后的原始记录如何渲染
    "recent_full_steps": 2,             //   最近 N 步全量
    "summary_chars": 280,               //   更早步骤的 assistant 文本截断长度
    "max_result_chars": 4096,           //   单个动作结果上限
    "drop_kinds": ["decide", "compaction", "input_rejected"]   //   不进入上下文的条目类型
  },
  "made_at_seq": 318, "made_by": "context_limit | ratio | manual", "updated_at_ms": 0
}
```

首次压缩之前，这个文件可以不存在，等价于 `start_seq = start_offset = 0`、没有摘要、机械压缩配置取 `session_config.prompt.mechanical_compress`。

**runs/** 采用 xllm 的 run 布局：`runs/<run_id>/run.json`（RunRecord）+ `snapshots/NNNN.json`（LLMContextSnapshot），只服务于**一次 llm_context 的执行期**。一个 run 就是一个 LLMContext 从 new / resume 到终态 outcome 的生命周期：

```text
开始：   建立 runs/<run_id>/（run.json 记录本轮输入 + s0 快照）→ 提交 state.json（live_run、已消费输入）→ 确认输入源
进行中： checkpoint 写入 runs/<run_id>/snapshots/（只需保留最新一份；最细到一次 do-action 之后）；worklog 不写
挂起：   PendingTool 等：保留 run，下一次输入直接 resume
结束：   ① 从最终快照生成该 run 的全部历史，一次性追加到 worklog（fsync）
         ② 提交 state.json（live_run = null，worklog.committed_* 前移）   ← 提交点
         ③ 删除 runs/<run_id>/
```

**下一次 llm_context 的构成与读取策略**：worklog 将来一定会很大，所以读取顺序固定为：先读 summary.json，再从 worklog 已提交的末尾**反向**读。

```python
def next_llm_context(s, lease, deps, env) -> LLMContext:
    live = resume_live_run(s, lease, deps, env)                  # ⓪ 有未结束的 run（state.live_run）：直接恢复（§8.6）
    if live: return live
    sm = s.summary_json() or Summary.initial(s.config)           # ① 先读 summary.json（小文件）：摘要 + 起点 + 机械压缩配置
    req = deps.assembler.build_request(s.config, env)            #    system：session_config.prompt（.llm_context 语义）+ 身份 / 约束段
    budget = history_budget(req, sm.history_summary)             #    上下文窗口减去 system 与摘要后的预算
    tail, reached_start = [], True
    for e in s.worklog.reverse_lines(end=s.state.worklog.committed_bytes,   # ② 从已提交末尾反向读（按块读取，由新到旧）
                                     stop=sm.start_offset):                 #    读到起点就停，从不扫描起点之前的内容
        if e.t in sm.mechanical.drop_kinds: continue
        r = render_mechanical(e, age=len(tail), cfg=sm.mechanical)          #    越新越完整，越旧截断越多
        if not budget.take(r): reached_start = False; break      #    预算先于起点耗尽：说明摘要已经过旧
        tail.append(r)
    if not reached_start:                                        # ③ 摘要与原始记录之间不能留空洞：先压缩，再构建
        sm = compact(s, lease, deps, cut_offset=offset_of(tail[-1]))        #   新起点 = 已保留条目中最旧的一条
    tail.reverse()                                               # ④ 恢复时间顺序
    return LLMContext.new(req.with_history(render(sm.history_summary, tail)), deps.llm_deps(s))
```

- **只反向读、按需停止**：正常情况下，读取量约等于起点之后的记录量，受上下文预算约束，与 worklog 总大小无关。`reverse_lines` 是各语言共用的原语：从指定字节位置向前按块读取，并按 `\n` 切行（§4.7）。
- **其它读取也用同一策略**：`read_session` 取最近历史、补发 round_digest、统计本 session 的副作用动作，都从末尾反向读。唯一的正向读取是压缩时读取 `[start_offset, cut_offset)` 这个有界区间；全量读取只留给审计 / 导出工具。
- **确定性**：相同的 summary.json + worklog 渲染出相同的结果。system + 摘要在下一次压缩之前保持不变，构成稳定前缀，有利于 KV cache。
- **压缩**只产出新的 summary.json（摘要前移、`start_seq` / `start_offset` 后移），不改写 worklog。触发时机沿用现有的两种：
  - run 结束后，按上下文占用比例触发（`maybe_compact`，§8.3）；
  - run 进行中遇到 `ContextLimitReached` 时，压缩后用 `ResumeFill::RewrittenHistory` 续跑，最多 3 轮。

```python
def compact(s, lease, deps, cut_offset) -> Summary:          # 只写 summary.json（外加一条审计条目）
    sm = s.summary_json() or Summary.initial(s.config)
    segment = s.worklog.read_range(sm.start_offset, cut_offset) # 唯一的正向读取：有界区间
    new = Summary(history_summary=summarize(sm.history_summary, segment),
                  start_seq=seq_at(cut_offset), start_offset=cut_offset,
                  mechanical=sm.mechanical, made_at_seq=s.state.worklog.committed_seq, made_by="ratio|context_limit")
    fenced(lease, lambda: atomic_replace(s.state_dir / "summary.json", new))
    s.worklog.append(lease, compaction_entry(new)); s.commit_state(lease)   # 审计条目 + 确认
    return new

def reconcile_runs(lease, s):                                 # drive 开头调用：让 worklog、runs/ 与 state.json 一致
    s.worklog.truncate_to(lease, s.state.worklog.committed_bytes)   # 截掉未提交尾部
    for run in s.runs.list():
        if run != s.state.live_run: s.runs.remove(lease, run)       # 开始阶段未提交的孤儿，或已提交但没来得及删除的 run
        elif s.runs.record(run).status.is_terminal():               # 已到终态，但结束流程未完成：重做（幂等）
            finish_run(lease, s, run, s.runs.latest_snapshot(run), …)   # §8.3
```

### 4.5 输入通道：kmsg + msg-center + kevent（不用文件系统）

session 的输入源在 `session_config.channels.inputs` 中声明，可以有多个：

| 输入源 | 适用 | 承载 | 确认（commit pop） |
|---|---|---|---|
| **kmsg 队列**（`KmsgInput`） | 所有 session；创建时建立 | msg / event / change / control / perception | `commit_ack(index)`（累积） |
| **msg-center inbox**（`MsgCenterInput`） | UI session：自己的 inbox，以 `did + session_id` 标识，驱动进程须**获准读取**（Q14） | 用户 / peer 消息 | `update_record_state(Readed)` |

kmsg 队列中的五类输入：

| type | 生产者 | 触发推理 | 说明 |
|---|---|---|---|
| `msg` | UI session 的 forward、应用、其它 session | 按 driver 的 pull 策略 | 消息 |
| `event` | 事件桥（active 订阅）、timer、task_mgr | 是（active） | 主动订阅的事件 |
| `change` | 事件桥（semi 订阅的外部对象事件） | **否**，只在观察边界注入 | 半订阅；同 key 在读取时合并，终态用独立 key |
| `control` | UI session、应用、OpenDAN | 否，drive 开头直接应用 | subscribe / unsubscribe / stop / decide |
| `perception` | runtime 子进程（如 `perceive` CLI） | 否 | 持有者在 round 提交时并入感知 |

**谁能驱动哪类 session**（Q14、Q16，由 BuckyOS 权限机制决定）：

- msg-center 的 inbox 以 **`did + session_id`** 标识（改造进行中）：每个 UI session 有自己的 inbox，权限按 inbox 区分。UI session 用 `MsgCenterInput` 直接消费自己的 inbox，不需要路由器转投。
- **OpenDAN 主进程**有权读取 Agent 的各类 inbox，负责发现新的 session inbox，并托管对应的 UI session（附录 A.3）。
- **其它 runner** 通常创建 work session，输入来自自建的 kmsg 队列。
- **获准读取某个 session inbox 的 app**，可以创建并驱动这个 UI session。企业软件就是这样在流程中集成 Agent Chat Box 的（§10.2）。
- **身份与权限**由驱动 session 的进程的 appid 决定（Q16）：读 inbox、回复消息、调用 LLM 都用这个身份。

```python
def post_input(cfg, inp: Input, who):                       # 投递到 kmsg 队列；任意有写权限的一方都可以调用
    if registry_status(cfg.sid).run_state == "finished" and not inp.allowed_after_finish():   # 尽力预检；以消费端为准
        raise SessionFinished
    q = cfg.channels.kmsg()
    kmsg.post_message(q.queue, Message(payload=to_json(inp.payload),
        headers={"type": inp.type, "key": inp.dedup_key, "from": who, "intent": inp.intent}))
    kevent.publish(cfg.channels.wake_event, {"sid": cfg.sid})   # 只通知“有变化”；丢了靠轮询兜底

def fetch_inputs(s) -> Inputs:                               # 只由持有者调用
    out = Inputs()
    for src in s.config.channels.inputs:
        for m in src.fetch(s.state.inputs[src.id]):          # kmsg：从服务端游标开始，不自动提交；msg-center：只读本 session 的 inbox（did + session_id），Unread + lock_on_take
            if not s.state.inputs.is_consumed(src.id, m) and m.key not in s.state.inputs.recent_keys:
                out.add(src.id, m)                           # 过滤“已消费但未确认”的消息与重复投递
    return out

def commit_pop(lease, s, consumed):                          # 进入推理 = commit pop：在 begin_round 中调用
    st = s.state.copy(); st.inputs.mark_consumed(consumed)   # 各输入源分别记录；recent_keys 有上限
    s.commit_state(lease, st)                                # ① 先提交 state.json（本地持久）
    for src in s.config.channels.inputs:                     # ② 再确认输入源
        src.confirm(st.inputs[src.id])                       #    kmsg：commit_ack 到“连续已消费”的最大 index；msg-center：标记 Readed
```

- **为什么先提交 state.json 再确认**：两步之间崩溃时，输入源会重投，重投的消息会被消费记录与 `recent_keys` 过滤掉；反过来先确认的话，崩溃会丢失输入。
- **选择性消费**：kmsg 的 ack 是累积的，只能 ack 到“连续已消费”的位置，跳过的消息会留在游标之后。永远不会处理的输入（例如 finished 之后的普通输入、不认识的类型），持有者要显式标记为已消费，并写一条带原因的 `input_rejected` worklog，避免游标被卡死。
- **kmsg 队列的所有权**：由创建者（通常是驱动者所属 App）创建，并开放 `other_app_can_write`，让 UI、OpenDAN 等其它参与方可以投递 control / msg。kmsg 目前还没有实现权限校验（kevent/kmsg 测试方案 D-07），见风险表。

### 4.6 订阅与变化（半订阅的落地）

- **本地状态对象**（其它 session、产物）：拉模式。在观察边界比较 Agent State 登记表中对方 `status` 的 rev 与 `state.subscription_cursors`，不依赖事件是否送达（S-16、A-09）。
- **外部对象事件**：以 `change` 类型进入 kmsg 队列，读取时按 key 合并。终态和用户放弃使用独立 key `…#terminal`，不会被进度覆盖（A-10）。消费进度落在 `state.inputs` 与 `state.subscription_cursors`（S-18）。因合并、过期或不相关而丢弃的变化，也要写入 worklog 并注明原因。
- 游标只存在于接收方 session 内：一个 UI session 已感知的变化，不会导致另一个 UI session 漏收。
- **active 与 semi 的区别**：active 源以 `event` 类型投递，会唤醒 session 并触发推理；semi 源以 `change` 类型投递，只在观察边界注入（S-15、A-07）。

### 4.7 文件原语（所有语言实现同一组语义）

```python
def atomic_replace(path, data: bytes):             # state.json / session_config / summary.json / stats / 登记条目：整文件替换
    tmp = f"{path}.tmp-{uuid()}"
    write_all(tmp, data); fsync(tmp)
    rename(tmp, path); fsync(parent_dir(path))

def append_batch(path, objs) -> (seq, bytes):       # worklog / 感知：一次写入多行完整 JSON（单写者），一次 fsync
    fd = open(path, O_APPEND | O_CREAT)
    write(fd, "".join(to_json(o) + "\n" for o in objs)); fsync(fd)
    return end_position(fd)

def truncate_to(path, committed_bytes):             # 恢复时截掉未提交尾部（worklog 唯一允许的修改）
    if size(path) > committed_bytes: truncate(path, committed_bytes); fsync(path)

def reverse_lines(path, end, stop, block=64 * 1024):   # 从 end 向前逐块读，由新到旧产出完整行，到 stop 为止
    pos, carry = end, b""
    while pos > stop:
        n = min(block, pos - stop); pos -= n
        chunk = read_at(path, pos, n) + carry
        lines = chunk.split(b"\n")
        carry = lines[0] if pos > stop else b""     # 块首的半行留给下一块拼接
        for line in reversed(lines[1:] if pos > stop else lines):
            if line: yield parse(line)

def publish_noreplace(path, data: bytes) -> bool:   # 只写一次的文件：binding.json、登记条目
    tmp = f"{parent_dir(path)}/.tmp-{uuid()}"
    write_all(tmp, data); fsync(tmp)
    ok = link_noreplace(tmp, path)                  # POSIX link()/renameat2(NOREPLACE)；Windows MoveFileEx 不带 REPLACE_EXISTING
    unlink(tmp); return ok

def publish_dir(tmp_dir, final_dir) -> bool:        # 目录原子发布（session 创建）
    return rename_noreplace(tmp_dir, final_dir)
```

### 4.8 创建、打开与读取

```python
def create_session(parent_dir, spec, agent: AgentStateHandle, who) -> SessionDir:
    sid = spec.session_id or derive_id(spec)
    spec.driver = spec.driver or who                             # 缺省：创建者就是驱动者
    if spec.kind == "ui":
        authorize_inbox_read(who, (spec.agent_did, spec.route_key))   # Q14：驱动进程的 appid 必须获准读取该 session inbox（did + session_id）
    queue = kmsg.create_queue(f"opendan.session.{sid}", appid=who.app, owner=who.owner,
                              config=QueueConfig(sync_write=True, other_app_can_write=True))   # 已存在则复用（幂等）
    kmsg.subscribe(queue, sub_id=f"opendan.session.{sid}", position=Earliest)
    tmp = parent_dir / f".tmp-{uuid()}"
    write(tmp / ".opendan_agent_session/session_config.json", config_from(spec, who, queue))
    end = write(tmp / ".opendan_agent_session/worklog.jsonl", [created_entry(spec, who)])
    write(tmp / ".opendan_agent_session/state.json", initial_state(committed=end))   # rev=1，run_state=created
    write(tmp / "readme.md", render_readme(spec))
    apply_dfs_acl(tmp, spec.acl)                                 # 按 agent_access 设置 ACL（Q10、Q12）
    if not publish_dir(tmp, parent_dir / sid):
        rm_rf(tmp)
        c = read_config(parent_dir / sid)
        if not (spec.idempotency_key and c.session.idempotency_key == spec.idempotency_key):
            raise SessionIdConflict(sid)                        # 同 key 重试：目录已存在，继续补登记
    sd = SessionDir.open(parent_dir / sid)
    agent.sessions.register(RegistryEntry.of(sd.config(), location=sd.path), who)   # 幂等
    if spec.origin.parent_session:                               # 由 UI 派生的 work session：父 session 自动半订阅它（S-17）
        agent.sessions.post_input(spec.origin.parent_session,
            Input.control("subscribe", mode="semi", source=session(sid)), who)
    return sd

def read_session(agent, sid, who, scope=("status",)) -> SessionView:
    e = agent.sessions.lookup(sid)                               # 登记表：Agent 总能看到全部 session 的 status
    view = SessionView(entry=e)
    if e.agent_access == "status_only" and who.is_agent_side(): return view   # 配置为只给状态摘要（Q10）
    sd = SessionDir.open(e.location)                             # 直接从 DFS 读；ACL 拒绝时返回 status 并注明原因
    view.state = sd.state()                                      # 先读提交点 state.json
    if "report" in scope: view.report = sd.report()
    if "worklog" in scope: view.worklog = sd.worklog().reverse_lines(end=view.state.worklog.committed_bytes).take(scope.n)   # 只反向读
    return view
```

---

## 5. Lease（分布式锁）协议

### 5.1 资源与位置

| 资源 | 文件 | 用途 |
|---|---|---|
| `session:<sid>` | `<sid>/.opendan_agent_session/lease.json` | 同一时刻只有一个进程推进；持有者身份必须等于 `session.driver` |
| `self_improve` | `<agent_root>/.locks/self_improve.lease` | 全局只有一个认知整理 |
| `artifact:<aid>` | `<agent_root>/.locks/artifact/<aid>.lease` | 串行化产物 head 的移动 |

lease 文件都放在 DFS 上（单节点时就是本地文件系统）。“检查-修改”靠文件写锁完成：单节点用 flock，多节点用 DFS 独占写锁。

```jsonc
{ "resource": "session:work-…", "token": 42,
  "holder": { "runner_id": "rn-…", "principal": "did:app:app2", "host": "did:dev:…", "pid": 1234, "runtime_id": "rt-…" },
  "granted_at_ms": 0, "expires_at_ms": 0, "released": false }
```

### 5.2 操作

```python
LEASE_TTL = 30s; RENEW_EVERY = TTL / 3

def acquire(res, holder, ttl=LEASE_TTL) -> Lease | Busy | NotDriver:
    if res.kind == "session" and holder.principal != config(res).session.driver.principal:
        return NotDriver                                # Q9：推进身份不变，进程可以换
    with write_lock(res.path):                          # 文件写锁：单节点 flock，多节点 DFS 独占写锁（只在“检查-修改”期间持有）
        cur = read_or_none(res.path)
        if cur and not cur.released and cur.expires_at_ms > now() and cur.holder.runner_id != holder.runner_id:
            return Busy(cur.holder, cur.expires_at_ms)
        lease = Lease(res, token=(cur.token if cur else 0) + 1, holder=holder,
                      granted_at_ms=now(), expires_at_ms=now() + ttl)
        atomic_replace(res.path, lease); return lease

def fenced(lease, write_fn):                            # 所有受 lease 保护的写都必须经过这里
    with write_lock(lease.path):
        cur = read(lease.path)
        if cur.token != lease.token or cur.released or cur.expires_at_ms <= now(): raise LeaseLost(cur)
        write_fn()                                      # 严格 fencing（单节点与多节点相同）
```

### 5.3 写入范围

| 权限来源 | 可写范围 |
|---|---|
| `session:<sid>` lease（驱动者身份） | `.opendan_agent_session/` 下全部文件；`.runtime/`；产物；session 绑定的 workspace（按 ACL）；Agent State 中**本 session 的**登记条目 status 与 `state/perception/<sid>.jsonl`；输入源确认（kmsg `commit_ack` / msg-center `Readed`） |
| 投递（无需 lease） | session 的 kmsg 队列 |
| `self_improve` | `state/perception/.cursor.json`、`memory/`、`attention_signals/`（`notebook/` 由自身锁串行） |
| `artifact:<aid>` | `state/artifacts/<aid>/artifact.json`（head） |

---

## 6. Agent State 协议（libOpenDAN::state）

### 6.1 布局

```text
<agent_root>/                               # 位于 DFS
  agent.toml role.md self.md users/ behaviors/ tool_plans/ tools/ skills/ i18n/   # 包层（OpenDAN rootfs_sync 维护；libopendan 只读）
  sessions/<sid>/                           # session 目录的默认位置（可选）
  state/
    sessions/<sid>.json                     # session 登记表（所有 session，无论目录在哪）
    perception/<sid>.jsonl                  # 感知
    perception/.cursor.json                 # 认知整理游标
    artifacts/<aid>/artifact.json           # 产物列表：长期产物与 head
    artifacts/<aid>/versions/<ver>.json     #   各 work session 的贡献（每个 session 一个文件）
  memory/  notebook/  attention_signals/    # 认知（沿用 agent_tool 实现与各自的内部锁）
  workspace/<wid>/                          # Agent 内部 workspace（沿用 .workspace.json；管理方式见 §6.5）
  .locks/self_improve.lease  .locks/artifact/<aid>.lease
```

### 6.2 Session 登记表（session mgr）

```jsonc
// state/sessions/<sid>.json
{
  "session_id": "work-…", "kind": "work", "class": "work",
  "created_by": { "principal": "did:app:app2", "via": "app" }, "idempotency_key": "…",
  "driver": { "principal": "did:app:app2" },
  "location": "/…/app2/agent_sessions/work-…",       // DFS 路径
  "input_queue": "app2::alice::opendan.session.work-…",   // 其它参与方据此投递（例如 UI 投递 decide）
  "agent_access": "full",
  "origin": { "parent_session": "ui-…" },
  "workspace": { "kind": "external", "path": "/…/app2/snake" }, "artifact_id": "snake-game",
  "status": { "rev": 17, "run_state": "finished", "outcome": "succeeded", "acceptance": "pending",   // rev = state.json 的 rev
              "one_line_status": "…", "report_brief": "≤500 字",
              "pending_decision": null,
              "last_runner": { "runner_id": "rn-…", "host": "did:dev:…", "pid": 1234, "at_ms": 0 },   // Q9
              "updated_at_ms": 0 }
}
```

```python
def register(entry, who):                                   # 创建者调用一次；幂等
    authorize(who, "session.create", entry)                 # DFS ACL
    if publish_noreplace(reg(entry.sid), entry): return entry
    cur = read(reg(entry.sid))
    if cur.idempotency_key == entry.idempotency_key and cur.created_by == entry.created_by: return cur
    raise SessionIdConflict(entry.sid)

def report_state(lease, sid, status):                       # 驱动者在每次提交后调用；本条目 status 的唯一写者
    fenced(lease, lambda: update_status_if_newer(reg(sid), status))   # rev ≤ 已有值时忽略
```

- `query(filter)` 是派生视图：扫描登记表并按 mtime 缓存。可以额外维护 `state/sessions/.index.sqlite`，但它必须能删除重建。两个典型用法：
  - S-14：`query(run_state != finished or acceptance = pending)`。
  - S-10：`query(artifact_id = X)`。
- `status` 只是缓存，真相是 session 自己的 `state.json`。回报失败的，下次 drive 开头补发（依据 `state.reported_rev`）。
- `pending_decision` 由驱动者在发现队列里有未处理的 decide 时写入，用于让 UI 显示“等待 app2 处理”。
- `verify` 巡检：location 已不存在的条目只标记为 `unreachable`，不删除。

### 6.3 感知管理

感知是**跨 session、只追加、低成本**的观察流：写入时不调用 LLM，也不同步触发整理（S-29）。

```jsonc
// state/perception/<sid>.jsonl（单写者 = 本 session 的驱动者，持 session lease；seq 严格单调）
{"seq":31,"at_ms":0,"session_id":"work-…","kind":"round_digest","round":12,
 "source":"session|self","tags":["snake","ui"],"objects":["artifact:snake-game"],"summary":"<one_line_status>","refs":{"worklog_seq":316}}
{"seq":32,"kind":"observation","source":"session","payload":{ /* Discover* 结构：event / object / relationship，含 evidence worklog refs */ }}
{"seq":33,"kind":"task_outcome","payload":{"outcome":"succeeded","acceptance":"pending","artifact_version":"…"}}
{"seq":34,"kind":"task_discarded","payload":{"session":"work-…","artifact_version":"…"}}   // S-27：保留来源
```

| 来源 | 时机 | 成本 |
|---|---|---|
| `round_digest` | runner 在每个 round 提交后自动写 | 零 LLM |
| `observation` | LLM 调用 `perceive` 工具，或子进程以 `perception` 类型投递到 kmsg 队列 | 一次工具调用 |
| `task_outcome` / `task_discarded` | finished / accept / discard 时自动写 | 零 LLM |

```python
def append_perceptions(lease, sid, records):                 # 驱动者调用
    last = tail_seq(perc(sid))                               # 反向读最后一行；重试幂等：跳过 seq ≤ last 的记录
    fenced(lease, lambda: append_batch(perc(sid), [r for r in records if r.seq > last]))

def backlog(cursor) -> Backlog:                              # 派生：只用 stat
    b = Backlog()
    for f in list(perception_root, "*.jsonl"):
        sid = stem(f); off = cursor.offsets.get(sid, 0)
        if size(f) > off and registry(sid).kind != "self_improve":   # 防自我回声
            b.add(sid, from_offset=off, to_offset=size(f))
    return b
```

- `perception/.cursor.json` 按 session 记录字节偏移，只由 `self_improve` lease 的持有者推进。它取代现有的 `already_improved` 字段。
- 提交后如果追加失败或进程崩溃，下次 drive 开头按 `state.perception_seq` 补发；缺失的 round_digest 从 worklog 末尾反向读来重建。

### 6.4 认知管理

认知由三部分组成：Memory Graph（`memory/`）、Notebook（`notebook/`）、整理中间态（`attention_signals/`）。实现沿用 `agent_tool::{agent_memory, agent_notebook, agent_attention_signal}`，libopendan 只包一层门面：

```rust
pub trait Cognition {
    /// 任何 session 都可调用，只读；返回 time + sentence + id 形式的线索（HintRecallEngine 移植）
    async fn recall_hints(&self, q: RecallQuery) -> Result<Vec<Hint>>;
    /// 普通 session 的显式声明（用户要求“记一下”）
    async fn notebook_append(&self, item: NotebookItem, who: &Principal) -> Result<()>;
    /// 仅限 self_improve lease 持有者：提交一批整理结果后推进感知游标（at-least-once 整理）
    async fn commit_consolidation(&self, lease: &Lease, batch: ConsolidationBatch, upto: PerceptionCursor) -> Result<()>;
}
```

- **跨语言边界是 CLI**（Memory v2 §0）：非 Rust 实现通过调用 `agent_tool agent-memory|agent-notebook …` 完成，不重写 Memory Graph。
- **S-30**：`<hints>`（过去形成的线索）与 `<changes>`（当前世界的变化）在 prompt 中分块，共享上下文预算。
- **S-27**：`task_discarded` 保留了来源链；self-improve 如何处理相关认知，**策略待定**。

### 6.5 产物列表（只登记与指向）

按 Q13，产物的复杂度不进入 Session 协议：

| 产物 | 位置 | 管理者 | Session 协议负责什么 |
|---|---|---|---|
| 一次性产物 | session 目录本身 | session | 随 session 保存；accept / discard 只改变其有效性标记 |
| 长期产物 | workspace | workspace | session_config 引用 workspace；binding.json 记录 workdir；产物列表登记“哪个 work session 对哪个 workspace 做了什么” |

workspace 分两类，它们的版本化、合并与回滚都**另行设计**：

- **Agent 内部 workspace**（`<agent_root>/workspace/<wid>`）：由 Agent 自己管理。默认 paios runtime 内置 git，可以用来做版本管理。
- **外部 workspace**（用户或应用指定的目录）：由其所有者管理，Agent 侧不承诺能回滚。

Agent State 的产物列表只负责**登记与指向**，服务于三件事：

- 意图分析定位“用户接受过的那个”（S-10）；
- 新 session 从有效版本继承（S-12）；
- 放弃后标记失效（S-14）。

```jsonc
// state/artifacts/<aid>/artifact.json（head 由 artifact lease 串行化）
{ "aid": "snake-game", "workspace": { "kind": "agent", "id": "ws-…" }, "head": "v-work-A" }
// state/artifacts/<aid>/versions/v-<sid>.json（每个 work session 一个文件，由其驱动者单写）
{ "ver": "v-work-B", "session": "work-B", "base": "v-work-A", "state": "produced | accepted | discarded",
  "outputs": ["report.md", "snake.js"],                          // session 目录中的一次性产物
  "workspace_ref": { "rev": "<git commit | 快照 id | null>" },   // 对 workspace 的变更引用，由 workspace 机制给出；外部 workspace 可以为空
  "side_effects": [{ "call_id": "c-9-1", "tool": "sendmsg", "note": "已发送的消息无法撤回" }] }   // 由反向读 worklog 得到
```

```python
def apply_decide(ls, sd, decision, agent):                 # 驱动者在 drive 开头应用 control(decide)（Q9）
    v = agent.artifacts.version(sd.config().artifact_id, f"v-{sd.id}") if sd.config().artifact_id else None
    report = None
    if decision == "accept":
        if v:
            with agent.leases.acquire(f"artifact:{v.aid}") as la:
                agent.artifacts.set_state(la, v, "accepted", head=True)
    else:
        if v: agent.artifacts.set_state(None, v, "discarded")      # 不动 head；如果 head 原本指向本版本，则回退到 base
        report = workspace_discard(sd.config().workspace, v)       # 交给 workspace：Agent 内部 workspace 可回滚；外部 workspace 通常 unsupported
        report.unsupported += v.side_effects if v else []          # S-26：外部副作用逐项列为不可撤销
        agent.perception.append(ls, sd.id, [task_discarded(v)])
    sd.worklog.append(ls, decide_entry(decision, report))          # 决策先进入历史
    sd.commit_state(ls, acceptance=decision_to_acceptance(decision), result=merge(result, discard_report=report))   # 提交点
```

**意图分析定位对象**（S-10、A-05）：用 `sessions.query` / `artifacts.query` 找出相关的长期产物，取它的 **head**（用户接受过的版本），而不是最后一个 finished 的 session；候选不止一个时先澄清。新 work session 引用同一个 workspace，base 取 head（S-12、A-06）。

### 6.6 AgentState 句柄（SDK 表面）

```rust
pub struct AgentStateHandle { /* agent_did + LocalFs(agent_root) + LeaseManager + who */ }
impl AgentStateHandle {
    /// agent_root 为本地或 DFS 路径；锁基于文件写锁（flock / DFS），不需要额外服务
    pub fn open(agent_root: PathBuf, who: Principal) -> Result<Self>;
    pub fn sessions(&self) -> &dyn SessionRegistry;    // register / report_state / lookup / query / post_input(sid, …)
    pub fn perception(&self) -> &dyn Perception;       // append / backlog / cursor
    pub fn cognition(&self) -> &dyn Cognition;
    pub fn artifacts(&self) -> &dyn Artifacts;         // head / versions / register_version / set_state / query
    pub fn leases(&self) -> &dyn LeaseManager;
}
```

---

## 7. Agent Runtime（libOpenDAN::runtime）

### 7.1 逻辑 runtime 与绑定

**Runtime** 是为 `exec_bash` 提供执行环境的**逻辑身份**：在哪台主机上执行、看到什么文件系统视图、PATH 上有哪些工具、用 native 还是 tmux 执行。它不是一个进程；tmux server 或容器重启后，仍是同一个 runtime。

```jsonc
// RuntimeDescriptor（由提供方声明）
{
  "runtime_id": "rt-jarvis-default", "kind": "tmux | native", "host": "did:dev:…", "provider": "opendan | app:<app_id>",
  "fs_view": { "agent_root": "/…/agents/jarvis", "workspace_root": "…/workspace", "extra": ["/…/app2"] },
  "path_layers": ["<sid>/.runtime/bin", "<agent_root>/tools", "<instance>/tools/bin", "/opt/buckyos/tools/store"],
  "capabilities": { "tools": { "git": "2.43", "node": "22" }, "network": true, "os": "linux" }
}
// .opendan_agent_session/binding.json（首次推进时写入，只写一次）
{ "runtime_id": "rt-jarvis-default", "kind": "tmux", "workdir": "/…/workspace/ws-…", "bound_at_ms": 0, "bound_by": "rn-…" }
```

```python
def bind_or_verify(sd, lease, rt: AgentRuntime) -> Binding:
    b = sd.binding_opt()
    if b is None:
        check_requirement(sd.config().runtime.requirement, rt.descriptor())   # 缺工具 / 缺目录：拒绝（S-07）
        workdir = resolve_workdir(sd.config().workspace, rt.descriptor()) or sd.dir   # 有 workspace 用 workspace，否则用 session 目录
        if not rt.can_access(workdir): raise BindError("runtime 看不到 workspace")    # 首次推理之前失败，没有成本（Q3）
        b = Binding(rt.descriptor().runtime_id, rt.kind, workdir)
        if not publish_noreplace(sd.state_dir / "binding.json", to_json(b)):
            b = sd.binding()                                                   # 并发绑定：以先到者为准
        rt.prepare_session_bin(sd)                                             # 渲染 .runtime/bin（tool_plan 墓碑 + Agent tools）
    if b.runtime_id != rt.descriptor().runtime_id:
        raise RuntimeMismatch(b.runtime_id)          # 换进程只能在同一个逻辑 runtime 上恢复（A-03）
    return b
```

驱动者身份（`session.driver`）与 runtime（`binding.json`）共同决定“谁、在哪里”推进这个 session，两者都不可变。绑定失败发生在任何推理之前：提交带 `last_error` 的 state.json 并回报，然后返回（Q3）。

### 7.2 接口与实现

```rust
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    fn descriptor(&self) -> &RuntimeDescriptor;
    /// 渲染 <sid>/.runtime/bin：Agent tools 同步 + tool_plan 墓碑（沿用 SessionBinRenderer 语义）
    async fn prepare_session_bin(&self, sd: &SessionDir) -> Result<()>;
    /// 为 session 准备执行视图：tmux session、cwd（= binding.workdir）、PATH（.runtime/bin 在最前）、env
    async fn open_session_env(&self, binding: &Binding, s: &SessionCtx) -> Result<SessionEnv>;
    async fn exec_bash(&self, env: &SessionEnv, req: BashRunRequest) -> Result<BashRunOutput, AgentToolError>;
    async fn interrupt(&self, env: &SessionEnv) -> Result<()>;
    async fn status(&self) -> RuntimeStatus;   // 给提示词引擎：活跃 tmux 会话、后台进程、磁盘、工具清单
    async fn close_session_env(&self, env: SessionEnv) -> Result<()>;
}
```

| 实现 | 来源 | 执行方式 | 典型用途 |
|---|---|---|---|
| `NativeRuntime` | 包装 `agent_tool::llm_bash::LocalProcessBashRunner` | 每条命令一个 `/bin/bash -c`，独立进程组 | 外部应用、测试 |
| `TmuxRuntime` | 从 `opendan::agent_bash::TmuxBashRunner` 移植 | 每个 session 一个 `od_<sid>` tmux session，可以 attach 审计 | 将来 OpenDAN 的默认 paios runtime（容器内） |

`prompt_env` 新增 `runtime.status` 变量，数据来自 `AgentRuntime::status()`。

### 7.3 exec_bash 的环境契约

| 变量 | 说明 |
|---|---|
| `OPENDAN_AGENT_DID` | Agent DID（新增） |
| `OPENDAN_AGENT_ROOT` | AgentRoot 的 DFS 路径（沿用） |
| `OPENDAN_SESSION_DIR` | session 目录（新增：session 可以在 AgentRoot 之外，不能再从 AgentRoot 推出） |
| `OPENDAN_SESSION_ID` / `OPENDAN_TRACE_ID` / `OPENDAN_RUNTIME_ID` | 标识 |
| `OPENDAN_INPUT_QUEUE` | 本 session 的 kmsg 队列 URN（新增）；子进程可向它投递 `perception` 等输入 |

- 子进程以驱动者身份运行，但**不持有 lease**，也不直接写 `.opendan_agent_session/`。
- 现有 `agent_tool` CLI 按 `OPENDAN_AGENT_ROOT` + `OPENDAN_SESSION_ID` 推导 session 目录，需要在 opendan 切换时改为支持 `OPENDAN_SESSION_DIR`（附录 A.7）。

---

## 8. Session Runner（libOpenDAN::runner）

### 8.1 扩展点

```rust
pub struct RunnerDeps {
    pub who: Principal,                              // 驱动者身份 = 驱动进程的 appid（Q7、Q16）：决定读 inbox、回复、调用 LLM 的身份与权限
    pub agent: AgentStateHandle,
    pub inputs: Arc<dyn InputChannelFactory>,        // 按 session_config.channels 建立 KmsgInput / MsgCenterInput
    pub waker: Arc<dyn Waker>,                       // kevent 订阅 wake_event + 轮询兜底
    pub runtime: Arc<dyn AgentRuntime>,
    pub llm: Arc<dyn llm_context::LlmClient>,        // 以 `who` 的身份调用 AICC（计费、审计归 App；Q7）
    pub tools: Arc<dyn ToolFactory>,
    pub assembler: Arc<dyn SessionAssembler>,        // 装配差异点（S-05、S-06）
    pub notifier: Arc<dyn Notifier>,                 // 状态变化通知（kevent）；默认 Noop
    pub outbound: Option<Arc<dyn OutboundSink>>,     // UI 回送：以驱动进程的 appid 身份发送（Q16）；libopendan 提供 MsgCenterOutbound
}

#[async_trait]
pub trait SessionAssembler: Send + Sync {
    /// 由 session_config.prompt（.llm_context 语义）+ 身份 / 约束段生成 request 的 system 段与策略（历史部分见 §4.4）
    async fn build_request(&self, cfg: &SessionConfig, env: &PromptEnv) -> Result<LLMContextRequest>;
    /// 每个 hook point：把本轮的 inputs / changes / hints 渲染成 user message；返回 None 表示本轮无需推理
    async fn render_turn(&self, s: &SessionView, hook: HookPoint, turn: &TurnMaterial) -> Result<Option<AiMessage>>;
}
```

- **默认实现 `BehaviorAssembler`**：`session_config.prompt.behavior` 指向 `behaviors/<name>`，再加上 `prompt_env` 与 driver hook point；从 opendan 移植。
- **应用装配**（S-05）按以下固定顺序合成，应用 prompt 不能替换前两段（S-08）：
  1. Agent 身份（role.md / self.md）
  2. Agent 不可覆盖的约束段
  3. 应用 system prompt（`prompt.system_prompt`）
  4. hints 段
  5. objective / end_condition
- **新鲜度**（S-20）：时间、时区等由 PromptEnv 在渲染时求值。

### 8.2 drive：推进一个 session

```python
async def drive(sd: SessionDir, deps: RunnerDeps, until: StopWhen) -> DriveResult:
    lease = await deps.agent.leases.acquire(sd.lease_resource(), deps.holder())   # 内含 driver 身份校验（Q9）
    if isinstance(lease, NotDriver): return DriveResult.not_driver()
    if isinstance(lease, Busy): return DriveResult.busy(lease.holder, lease.expires_at_ms)
    renewer = spawn(renew_loop(lease)); extra = []
    try:
        s = await sd.load()                              # session_config + state.json（都是小文件）
        reg = deps.agent.sessions.lookup(s.id)
        if reg is None or reg.location != sd.path: return DriveResult.unregistered()   # 未登记的 session 不推进
        reconcile_runs(lease, s)                         # 截掉 worklog 未提交尾部；清理孤儿 run；重做未完成的 run 结束（§4.4）
        await catch_up_reports(s, lease, deps.agent)     # 补发登记表回报 / 感知（§6.2、§6.3）
        s.channels = deps.inputs.open(s.config.channels) # KmsgInput + 可选的 MsgCenterInput（Q14）
        inputs = fetch_inputs(s)
        s.apply_controls(lease, inputs.take("control"))  # subscribe / unsubscribe / stop / decide（finished 之后仍处理 decide）
        if s.state.run_state == "finished":
            s.reject_leftovers(lease, inputs); return DriveResult.finished(s.state)
        try:
            binding = bind_or_verify(sd, lease, deps.runtime)                        # 首次推理之前（Q3）
        except (BindError, RuntimeMismatch) as e:
            s.commit_state(lease, last_error=e.to_json()); report(s, lease, deps)
            return DriveResult.bind_failed(e)
        env = await deps.runtime.open_session_env(binding, s.ctx())
        extra = await acquire_kind_leases(s, deps.agent) # self_improve：拿不到就以 stopped/busy 结束
        ctx = resume_live_run(s, lease, deps, env)       # §8.6：state.live_run 指向未结束的 llm_context 时直接恢复，否则为 None

        while not lease.lost:
            hook = s.next_hook_point()                   # on_init | on_wakeup | on_behavior_switch（driver 沿用）
            picked = inputs.select(s.driver.pull(hook))  # pull_msg / pull_event（沿用）；change 不在此列
            changes = await check_changes(s, inputs, deps.agent, budget=s.driver.change_budget)   # §8.4
            hints = await deps.agent.cognition().recall_hints(s.topic()) if s.driver.load_hints(hook) else []
            msg = await deps.assembler.render_turn(s.view(), hook, TurnMaterial(picked, changes, hints, await deps.runtime.status()))
            if msg is None and not s.needs_bootstrap():  # 不让 LLM 空转
                if until.idle: break
                await deps.waker.wait(s, until)          # 等 kevent 唤醒或轮询间隔到
                inputs = fetch_inputs(s); s.apply_controls(lease, inputs.take("control")); continue

            ctx = ctx or next_llm_context(s, lease, deps, env)   # 先读 summary.json，再反向读 worklog（§4.4）
            ctx.append_turn(msg)
            await begin_round(s, lease, ctx, picked, changes, msg)   # 建立或续用 run → 提交 state.json → 确认输入源
            outcome = await ctx.run()                            # llm_context；工具经 §8.5 的适配层执行
            nxt = await commit_round(s, lease, ctx, outcome, deps)
            ctx = None if nxt.run_ended else ctx
            if nxt.finished or until.satisfied(s): break
            if nxt.waiting and until.idle: break
            inputs = fetch_inputs(s)

        return DriveResult.from_state(s.state)
    except LeaseLost:
        return DriveResult.lost()
    finally:
        cancel(renewer); await release_all([lease] + extra)
```

`StopWhen` 的取值：

- `idle`：没有输入就退出。用于常驻托管（OpenDAN，或驱动 UI session 的应用）。
- `finished`：一直推进到 work session 结束。
- `max_rounds(n)`：最多推进 n 轮。

### 8.3 round 的提交顺序

```python
async def begin_round(s, lease, ctx, picked, changes, msg):
    n = s.state.round + 1
    s.runs.begin_round(lease, ctx.run_id, round=n, inputs=picked, changes=changes,   # run.json 记录本轮输入；新 run 时同时写 s0 快照
                       message=msg, snapshot=ctx.snapshot())                         #   （run 结束时这些都进入 worklog）
    s.state.update(round=n, run_state="running", live_run=ctx.run_id, subscription_cursors=advance(changes))
    commit_pop(lease, s, picked + changes.consumed_inputs)   # 提交 state.json → 确认输入源（§4.5）

async def commit_round(s, lease, ctx, outcome, deps) -> Next:
    nxt = classify(outcome, s)       # 沿用 handle_outcome：Done / WAIT_USER_MSG / next_behavior / END / PendingTool / Budget / Error
    s.runs.checkpoint(lease, ctx.run_id, ctx.snapshot(), status=nxt.run_status)   # 1. outcome 边界 checkpoint（run.json.status：终态或挂起）
    return finish_run(lease, s, ctx.run_id, ctx.snapshot(), outcome, nxt, deps)

def finish_run(lease, s, run_id, snapshot, outcome, nxt, deps) -> Next:   # reconcile_runs 重做时也调用它（幂等）
    if nxt.finished:
        v = register_outputs(lease, s, deps.agent)                       # 2. 产物登记（§6.5；幂等）
        fenced(lease, lambda: atomic_replace(s.dir / "report.md", render_report(outcome)))
        s.state.update(result=build_result(outcome, v), acceptance="pending")
    if nxt.run_ended:
        flush_run(lease, s, run_id, snapshot)                            # 3. 该 run 的全部历史一次性追加到 worklog（§4.4）
        s.state.update(live_run=None)
    s.commit_state(lease, **nxt.state_patch())                          # 4. 提交点：state.json 原子替换（rev+1，worklog.committed_* 前移）
    if nxt.run_ended: s.runs.remove(lease, run_id)                       # 5. 删除已结束的 run（失败无妨，下次 reconcile 清理）
    maybe_compact(s, lease, deps)                                        #    按上下文占用比例触发 compact（§4.4）
    s.stats.update(lease, outcome.usage)
    # ── 以下在提交之后执行；失败或崩溃时，下次 drive 开头补发 ──
    deps.agent.sessions.report_state(lease, s.id, s.status())            # 6. 登记表 status（rev 单调）
    deps.agent.perception.append(lease, s.id,                            # 7. 感知（seq 单调；包含队列里的 perception 输入）
        s.fold_perception_inputs(lease) + [round_digest(s, outcome)] + ([task_outcome(s, outcome)] if nxt.finished else []))
    deps.notifier.session_changed(s.id, s.state.rev)                     # 8. 通知
    if s.is_ui and outcome.has_text and deps.outbound: await deps.outbound.post(s, outcome.text)
    return nxt

def flush_run(lease, s, run_id, final_snapshot) -> WorklogEnd:
    entries = run_history_entries(run_id, s.runs.record(run_id), final_snapshot)   # round_started / user_message / step / action_result / outcome
    s.worklog.truncate_to(lease, s.state.worklog.committed_bytes)   # 截掉上次崩溃遗留的未提交尾部（如果有）
    return s.worklog.append_batch(lease, entries)                   # 一次写入 + fsync
```

- 读者只有看到新的 state.json rev 之后，才去读 report 或其它新内容（S-23）。
- **worklog 写入规则**：每次追加（run 结束时的批量写入、decide、compaction、input_rejected）之后，都要提交一次 state.json 作为确认；未确认的尾部在恢复时截掉。
- behavior 切换（normal / fork / independent）与 PendingTool 转 task_mgr，保留在 `classify` 之后。independent / fork 挂起的父 process 快照放在当前 run 的目录内。

### 8.4 观察边界的变化注入（S-16、A-08）

```python
async def check_changes(s, inputs, agent, budget) -> Changes:
    out = []
    for sub in s.config.subscriptions:
        if sub.source.type == "session":                               # 拉模式：比对登记表 status
            e = agent.sessions.lookup(sub.source.ref); cur = s.state.subscription_cursors.get(sub.id)
            if e and e.status.rev > cur.rev and any(e.status[f] != cur[f] for f in sub.watch):
                out.append(Change.session(sub, e.status, text=one_line(e.status)))   # “任务 X 已完成：<report_brief>”（S-17）
    out += coalesce_by_key(inputs.peek("change"))                      # 推模式：队列中的 change 输入，读取时合并
    return prioritize_and_trim(out, budget, keep_terminal=True)        # 终态优先；超出预算的写入 worklog dropped 并注明原因
```

注入点有两类：

- **每个 hook point**：wakeup、behavior switch，现有机制已经支持。
- **每个 do-action 之后**：
  - Behavior 模式用现有的 `StepResultHook::on_behavior_step_ob`。
  - Agent（传统）模式需要 waist 新增一个可选钩子（L3 waist 变更 ①）：

```rust
// llm_context 新增（可选；未设置时行为不变）：一轮工具结果都追加完、下一次推理之前调用
#[async_trait] pub trait ObservationHook: Send + Sync {
    async fn after_observations(&self, snap: &LLMContextSnapshot) -> Result<Vec<ContentBlock>, String>;
}
```

run 进行中到达的 change，由钩子在 do-action 之间注入；它们的消费标记与游标推进，在下一次提交 state.json 时统一落盘。

### 8.5 工具调用适配层（effect 层）

```python
class SessionToolManager(llm_context.ToolManager):          # 移植并改造 opendan 的 OpendanToolAdapter
    async def call_tool(self, call):
        tool = self.tools.get(call.name); eff = tool.effect()   # read_only | idempotent | side_effect | unknown
        if eff != "read_only":
            self.runs.set_inflight(self.lease, self.run_id, call, eff)    # 在途动作记录在当前 run 的 run.json 中
        try:
            obs = await tool.run(call, runtime=self.env)          # exec_bash → runtime.exec_bash（cwd = binding.workdir）
        except DispatchError:
            obs = Unresolved(call.id, effect_unknown=(eff != "read_only"))   # llm_context 的既有语义
        if eff != "read_only": self.runs.clear_inflight(self.lease, self.run_id)
        return obs                                                # 结果进入 llm_context 状态，随 checkpoint 落在 runs/；run 结束时写入 worklog
```

- session-aware 工具（`create_worksession`、`forward_msg`、`try_create_worksession`、`update_session_topic`、`read_session_history` …）改为调用 `AgentStateHandle` 与输入通道 API，不再持有 `Weak<AIAgent>`。
- 意图分析（S-09）继续使用 fork 原语；它是一个逻辑分支，不是持久化的 session。

### 8.6 崩溃恢复：复用 llm_context（Q5）

恢复遵循 [LLM Context 设计 §9](<LLM Context 设计.md>) 的纪律：

- checkpoint 写入 `runs/<run_id>/snapshots/`，run.json 沿用 xllm 的 RunRecord。
- `state.live_run` 指向的 run 就是未结束的 llm_context。
- 如果这个 run 已经到达终态（说明结束阶段中途崩溃），由 `reconcile_runs` 重做结束流程（§4.4）。

| checkpoint 时机 | 来源 | 覆盖 |
|---|---|---|
| run 启动前 s0 | L4（`begin_round`） | 首轮 |
| 每次推理前 | `TurnHook::before_inference` | Agent 模式：上一轮 do-action 之后 |
| **每个 step 的 do-action 之后** | **waist 变更 ③**：Behavior 模式的步边界 checkpoint（sediment 之后、包含该 step 的外层快照） | Behavior 模式：一次 do-action 之后 |
| outcome 边界 | L4（`commit_round`） | 挂起态 / 终态 |

```python
def resume_live_run(s, lease, deps, env) -> LLMContext | None:
    run = s.state.live_run                                     # reconcile_runs 之后，这里只可能是未到终态的 run
    if run is None: return None                                # 没有进行中的 llm_context：下一轮按 §4.4 构成
    cp, record = s.runs.latest(run)                            # 最新 checkpoint（最细到上一个 do-action 之后）
    fill = (ResumeFill.ToolResults(collect_task_results(s)) if cp.state.pending_tool_calls
            else ResumeFill.ResumeFromMidRun)                  # 挂起态 / 运行中：按 llm_context 纪律选择
    ctx = LLMContext.resume(cp, fill, deps.llm_deps(s, lease, env))
    if record.inflight:                                        # 崩溃时正在执行的 do-action：不自动重放
        s.pending_notes.append(render_unresolved(record.inflight))   # 以“结果未知”注入（Observation::Unresolved{effect_unknown} 语义）
        s.runs.note_unresolved(lease, run, record.inflight)    # 记在 run 内，run 结束时随历史写入 worklog
    return ctx
```

- UI session 与 work session 使用同一套逻辑。
- 按 llm_context 的纪律，最后一个 checkpoint 之后的 inference 可能被重跑；工具幂等属于 effect 层，本层通过在途标记和 idempotency key 处理。
- run 进行中的历史只在 `runs/`。run 目录损坏时，损失的是该 run 的执行现场和尚未写入 worklog 的历史；已提交的 worklog 与 state.json 不受影响。此时把 `live_run` 置空，下一轮按 §4.4 构成 llm_context。

---

## 9. 状态共享与对象协议

### 9.1 StateFs 与对存储的要求

```rust
#[async_trait]
pub trait StateFs: Send + Sync {        // LocalFs：本地路径或 DFS 挂载；如果 DFS 只提供 API 访问，另加一个实现即可
    async fn read(&self, path: &Path) -> Result<Option<Bytes>>;
    async fn read_at(&self, path: &Path, offset: u64, len: u64) -> Result<Bytes>;   // reverse_lines / read_range 依赖它
    async fn list(&self, path: &Path) -> Result<Vec<DirEntry>>;
    async fn stat(&self, path: &Path) -> Result<Option<Meta>>;
    async fn atomic_replace(&self, path: &Path, data: Bytes) -> Result<()>;
    async fn append_batch(&self, path: &Path, lines: Bytes) -> Result<u64>;
    async fn truncate_to(&self, path: &Path, len: u64) -> Result<()>;
    async fn publish_noreplace(&self, path: &Path, data: Bytes) -> Result<bool>;
    async fn publish_dir(&self, tmp: &Path, dst: &Path) -> Result<bool>;
}
```

| 存储原语 | 用途 | 单节点本地文件系统 | DFS（独占单写） |
|---|---|---|---|
| 原子替换（rename 覆盖） | state.json / session_config / summary.json / stats / 登记条目 | ✔ | ✔ |
| 不覆盖发布（link / rename-noreplace） | binding、登记、session 目录发布 | ✔ | ✔ |
| 单写者追加 + fsync、截断、按偏移随机读 | worklog（批量追加、反向读）、感知 | ✔ | ✔ |
| ACL | agent_access、App 隔离 | ✔（共享物理根 + ACL） | ✔（BuckyOS 管理权限） |
| 文件写锁 | lease 的“检查-修改”（§5） | flock | 独占写锁（只有一个 client 能拿到） |

DFS 的原子语义通常强于单机文件系统，所以协议按单机语义设计，放到 DFS 上同样成立（Q12）。

### 9.2 DID Object 宿主组件（`host` feature）

实现 buckyos-base 的 `DIDObjectServer`，由 `DIDObjectHttpServer` 暴露。它**面向 Agent 的 `read` / `xcall` 和外部访问**；runner 不通过它读写数据。

| 对象（相对 URL 根） | Trait | properties | actions | events |
|---|---|---|---|---|
| `/` Agent | — | `card`、`status` | — | — |
| `/sessions` | `index@1` | `index_schema` | `query` / `page` | `registered` / `changed{rev}` |
| `/sessions/{sid}` | `opendan-session@1` | `status`；`agent_access=full` 时另有 `state` / `report` / `worklog_tail`（反向读） | `post_input`（宿主代为投递到 kmsg，供无法访问 kmsg 的外部调用方使用） | `state_changed{rev}` |
| `/perception` | `opendan-perception@1` | `backlog` | `query` | `appended` |
| `/cognition` | `opendan-cognition@1` | — | `recall_hints` / `notebook_append` | `changed` |
| `/artifacts`、`/artifacts/{aid}` | `index@1` / `opendan-artifact@1` | `head` / `versions` | — | `changed` |

libopendan 定义 `PrincipalResolver` 与 `Authorizer` 两个 trait；OpenDAN 切换时接入 verify-hub 与 RBAC。

---

## 10. 外部 Runner 示例

### 10.1 应用驱动一个 work session（A-01，最常见的形态）

```ts
// TS 示意；Rust API 同名。app2 的 session 目录在自己的 data 目录，AgentRoot 在 DFS 上
const me = app2Identity                                                    // 驱动者身份 = App 身份（Q7、Q9）
const agent = AgentStateHandle.open(jarvisAgentRoot, me)
const sd = await SessionDir.create(`${app2DataDir}/agent_sessions`, {
  agentDid, kind: "work", driver: me,
  objective: "给贪吃蛇加穿墙模式", endCondition: { type: "llm_declares_done" },
  prompt: { systemPrompt: appPrompt, behavior: "do", provider: { type: "buckyos" } },   // .llm_context 的超集
  workspace: { kind: "external", path: `${app2DataDir}/snake` }, artifactId: "snake-game",   // 长期产物放在外部 workspace
  runtime: { requirement: { tools: ["node"] } }, acl: { agentAccess: "full" },
  extensions: { app2: { ticket: "T-42" } }, idempotencyKey: requestId,
}, agent)                                                                  // 同时创建 kmsg 输入队列并登记
const runner = SessionRunner.create(agent, {
  who: me, runtime: NativeRuntime.local({ runtimeId: `app2:${hostDid}` }),
  llm: AiccClient.as(me), inputs: InputChannels.buckyos(), waker: KEventWaker(), tools: defaultTools() })
await runner.drive(sd, { until: "finished" })                               // acceptance = pending
// 用户在 UI 里说“接受”：UI session 向该 session 的 kmsg 队列投递 control(decide: accept)，并发布 kevent
// app2 的任一进程被唤醒后执行 drive：登记产物 head；workspace 本身由 app2 自己管理
await runner.drive(sd, { until: "idle" })
```

### 10.2 企业软件集成 Agent Chat Box：由应用驱动 UI session（Q14）

```ts
// 前提：BuckyOS 权限机制允许 erpApp 读取该 session 的 inbox（did + session_id，Q14）
const me = erpAppIdentity                                                  // 身份与权限由驱动进程的 appid 决定（Q16）
const agent = AgentStateHandle.open(jarvisAgentRoot, me)
const sd = await SessionDir.create(`${erpDataDir}/agent_sessions`, {
  agentDid, kind: "ui", driver: me, routeKey: `erp:approval:${flowId}`,     // routeKey 即 inbox 的 session_id
  prompt: { systemPrompt: approvalFlowPrompt, behavior: "chat_route" },
  inbox: true,                                                             // channels.inputs 加入 msg_center 源：{ did: agentDid, session_id: routeKey }
}, agent)
const runner = SessionRunner.create(agent, {
  who: me, runtime: NativeRuntime.local(...), llm: AiccClient.as(me),
  inputs: InputChannels.buckyos(), outbound: MsgCenterOutbound(), waker: KEventWaker(), tools: erpTools() })
for (;;) await runner.drive(sd, { until: "idle" })                         // 常驻：没有输入时由 kevent / 轮询唤醒
```

- 这个 UI session 与 OpenDAN 托管的 UI session 使用同一套协议，同样登记到 Agent State 的登记表。因此 Agent 能看到它的状态摘要，它产生的感知也进入 Agent 的记忆，跨入口保持连续（S-21、S-28）。
- 如果这个 UI session 派生出 work session，由谁驱动取决于该 work session 的 `driver`：可以是 erpApp 自己，也可以交给 OpenDAN 托管。

### 10.3 并存与恢复（A-02、A-03、A-14）

```text
app2 进程 P1                session 目录 / 登记表 / kmsg          app2 进程 P2（同一 App 身份）
 │ acquire session:T ─────► token=7
 │ fetch inputs ──────────► kmsg[119..121]
 │ begin_round r3 ────────► runs/<run>（本轮输入 + s0）→ state.json(live_run, consumed 121) → commit_ack(121)
 │ do-action c-3-1 ───────► runs/<run>/run.json: inflight=c-3-1
 ✗ crash
                           lease 过期
                                                             │ acquire session:T ──► token=8（driver 身份相同 ✓）
                                                             │ 已登记 ✓；reconcile：截断 worklog 未提交尾部
                                                             │ runtime_id 相同 ✓；resume runs/<run> 最新 checkpoint
                                                             │ inflight c-3-1 → 以“结果未知”注入，不自动重放
 │（P1 若复活）fenced(token=7) ──► LeaseLost，停止
 其它身份（例如 app3）尝试 acquire session:T ──► NotDriver
```

---

## 11. 跨语言策略

| 层级 | 内容 | Rust | TS（buckyos-websdk） | 其它语言 |
|---|---|---|---|---|
| T0 协议 | 目录布局、JSON Schema、提交顺序、lease、输入消息格式、状态对象 profile、fixtures | 权威实现，schema 由它导出 | 一致性测试 | 一致性测试 |
| T1 State / Session 客户端 | SessionDir（config / state / worklog 反向读 / summary.json）、登记表、感知、recall、产物列表；kmsg / msg-center 输入（websdk 已有 `msg_queue_client.ts`、`msg_center_client.ts`、`kevent_client.ts`）；FileLeaseManager（flock / DFS 写锁） | ✔ | ✔（L7） | 按需 |
| T2 Runner | drive、assembler、next_llm_context、runs；依赖同语言的 LM Context | ✔ | ✔（依赖 xllm TS 版；runs 布局与 xllm 一致） | 按需 |
| T3 Runtime | native / tmux | ✔ | 仅 native | — |
| T4 宿主组件 | DID Object 宿主 | 只有 Rust | — | — |

- **fixtures 驱动一致性**：`doc/opendan/protocol/fixtures/` 中放一组黄金目录，每种语言都要断言“解析结果 + 下一步动作”一致。覆盖以下场景：
  - 新建 work session；
  - 位于 AgentRoot 之外的 session；
  - 运行中崩溃（含 inflight）；
  - run 结束阶段中途崩溃（截断未提交尾部后重做）；
  - 已消费但未确认的重投；
  - 选择性消费；
  - finished 之后的投递与 decide；
  - 半订阅的 rev 比对；
  - lease 过期后同一身份接管；
  - 非驱动者被拒；
  - 带摘要与起点的大 worklog：反向读取在起点停止，读取量与总大小无关；
  - llm_context 的构成字节级稳定。
- **不跨语言复刻**：Memory Graph / Notebook / attention signal 统一调用 `agent_tool` CLI。

---

## 12. 从 opendan 移植（不修改 opendan）

原则是**复制后改造**，opendan 保持原样。在 libopendan 里修复的问题如果也影响 opendan，要在 PR 中注明。

| 来源（`src/frame/opendan/src/`） | libopendan 模块 | 改造要点 |
|---|---|---|
| `session_model.rs` | `protocol::{config, state, summary, input}` | `SessionMeta` 拆分：不可变部分进 session_config，可变部分进 state.json；`pending_inputs` 改为 kmsg / msg-center 输入 + `state.inputs` 游标；去掉 `already_improved` |
| `agent_session.rs` | `runner` | worker → `drive`；`run_one_round` → begin / commit_round；`flush_meta` / `enqueue_pending` / `persist_snapshot` → state.json 提交、输入源确认、runs；opendan 的消息压缩 → summary.json；`Weak<AIAgent>` → `AgentStateHandle`；`post_outbound_*` → `OutboundSink`；`mirror_status_to_task` 不移植 |
| `round_history.rs`、`session_topic.rs` | `session::worklog`；topic 归入 state.json 的 `topic` | round_logs + 每轮文件 → 单个 `worklog.jsonl`（run 结束时批量写入、反向读取）；条目类型沿用 |
| `hint_recall.rs` | `state::cognition::recall` | — |
| `ai_runtime.rs` | `runner::deps` | `SessionToolManager`（在途标记）；`SessionSnapshotHook` → `runs/` checkpoint；`AgentPolicy`；`AiccLlmClient` 按 `who` 的身份调用 |
| `behavior_cfg.rs`、`behavior_hooks.rs`、`hook_point.rs`、`prompt_env.rs`、`i18n.rs`、`llm_context_helper.rs` | `runner::assembler` | 成为 `BehaviorAssembler`；与 `session_config.prompt`（.llm_context 语义）对齐 |
| `agent_config.rs`（AgentLayout / session class / driver 配置） | `config` | `[[channel]]` / `[dispatch]` 不移植 |
| `agent_bash.rs`、`tool_plan.rs`、`paths.rs` | `runtime::{tmux, bin_overlay, paths}` | Session Exec Bin → `<sid>/.runtime/bin`；cwd 取 `binding.workdir` |
| `local_workspace.rs` | 只移植 workspace 的引用与解析 | workspace 版本化另行设计（Q13） |
| `buildin_tool.rs`、`worksession_tools.rs`、`attachment_*.rs`、`task_util.rs` | `runner::tools` | 改为调用 AgentState 与输入通道 API |
| `msg_center_pump.rs` 的记录翻译部分 | `channel::MsgCenterInput` / `MsgCenterOutbound` | 供获准的应用直接驱动 UI session（Q14） |
| `agent_tool::local_llm_context`（xllm） | 复用 `RunStore` / `RunRecord` 布局与 `.llm_context` 配置解析 | `runs/` 与 `session_config.prompt` 直接沿用 xllm 的定义 |
| 不移植：`agent.rs`、`main.rs`、`dispatch*.rs`、`command_dispatcher.rs`、pump 的路由部分、`contact.rs`、`agent_task_executor.rs`、`task_dispatch.rs`、`worklog.rs` 的存储部分 | — | 属于托管职责，见附录 A |

---

## 13. 分阶段实施（仅 libopendan）

每个阶段结束时，都必须满足 `cargo test -p libopendan -- --test-threads=1`、`cargo test -p llm_context`、`uv run buckyos-build.py` 全部通过。本期不改 opendan，Jarvis 的行为不受影响。

### L0 协议文档与 fixtures

- **交付**：`doc/opendan/protocol/` 下的 `Session Directory Protocol.md`、`Agent State Protocol.md`、`Lease Protocol.md`、`Session Input Protocol.md`、`State Objects Profiles.md`；首批 fixtures。
- **前置**：无（待确认问题已清零）。
- **验收**：协议评审通过；L1 由 schemars 导出的 schema 与文档逐字段一致。

### L1 骨架：文件原语、本地锁、Session 目录、kmsg 输入、登记表

- **交付**：
  - crate 骨架、`fsutil`（含 `reverse_lines` / `append_batch` / `truncate_to`）、`LocalFs`；
  - `FileLeaseManager`（含 driver 校验）；
  - `SessionDir`：config / state / worklog / summary.json / stats / binding；
  - `InputChannel`：kmsg 实现 + 内存实现，支持 `commit_pop` 与选择性消费；
  - AgentState registry：register / report_state / lookup / query / post_input。
- **验证**：
  - **多进程 lease**：token 单调；过期后可以接管；旧 token 的写入被拒绝；非驱动者返回 NotDriver；同身份的另一进程可以接手。
  - **输入**：在 state.json 提交与确认之间崩溃时，重投会被过滤；选择性消费不丢事件；finished 后的普通输入被拒绝并写入 worklog；kevent 丢失时由轮询兜底。
  - **worklog**：未提交尾部被截断；反向读在起点停止。基准：1 GB 的 worklog 上构建上下文时，读取量只与起点之后的记录有关。
  - **位置无关**：AgentRoot 与 session 目录分别放在两个 tempdir；移动 session 目录后更新 location。
  - **DV**：用真实 kmsg 执行 `uv run test/run.py -p <新 case>`。

### L2 Runtime

- **交付**：`AgentRuntime` trait、`NativeRuntime`、`TmuxRuntime`（移植）、`.runtime/bin` 渲染、`bind_or_verify`（含 workspace → workdir 解析）、§7.3 的 env。
- **验证**：exec_bash 在 native 与 tmux 下结果一致；runtime 不匹配、缺少工具、看不到 workspace 时都拒绝，并确认拒绝发生在任何推理之前。

### L3 Runner

- **交付**：
  - `drive`、begin / commit_round、`finish_run`、`flush_run`、`reconcile_runs`、`next_llm_context`（summary.json + 反向读 worklog）、`compact` / `maybe_compact`、`resume_live_run`、`SessionToolManager`、`check_changes`、`BehaviorAssembler`；
  - 从 `agent_session.rs` 移植 outcome、切换、压缩、fork、report 逻辑；
  - UI session 支持：`MsgCenterInput` / `MsgCenterOutbound`（`buckyos` feature；依赖 msg-center inbox 改为 did + session_id）；
  - 开发 CLI：`cargo run -p libopendan --example session -- create|run|read|post|decide`；
  - **waist 变更**（llm_context；都是可选项，现有行为不变；xllm TS 版要同步，并更新 `local_llm_context_protocol.md`）：
    - ① Agent 模式的 `ObservationHook`；
    - ③ Behavior 模式的步边界 checkpoint。
- **验证**：用 OpenAI 兼容的 mock LLM 覆盖：
  - A-01：work session 在独立进程中完成；
  - A-02：两个 runner 共享同一个 AgentRoot；
  - A-03：在 do-action 执行中 kill，同身份的另一进程从上一个 do-action 之后恢复，在途动作以“结果未知”注入；run 结束阶段崩溃后能正确重做，且不重复写入 worklog；
  - A-07 ~ A-10：半订阅不引发额外推理；执行中的变化在观察边界注入；重放不重复注入；终态不被淹没；
  - 预算先于起点耗尽时，先压缩再构建，摘要与原始记录之间没有空洞；
  - llm_context 构成的前缀字节级稳定；
  - DV：用 session inbox（did + session_id）验证应用驱动的 UI session（§10.2）；inbox 改造完成前，先用转投 kmsg 的方式验证。

### L4 感知与认知

- **交付**：感知的 append / backlog / cursor / 补发；`perceive` 工具与 `perception` 输入并入；`Cognition` 门面与 recall 注入；`self_improve` lease，以及“一个 self-improve session 内的 behavior 链”。本期由测试手动触发；定时与空闲检查属于 OpenDAN。
- **验证**：seq 单调且幂等；self-improve 自己产生的感知不计入水位（防自我回声）；整理失败时游标不推进；两个 self-improve 并发时只有一个运行。

### L5 产物列表与意图定位

- **交付**：
  - 产物列表：登记（`register_outputs`）、head、`apply_decide`、discard report（包括不可撤销的副作用）；
  - `workspace_discard` 只定义接口，由 workspace 机制实现（另行设计）；
  - 意图分析用的 `sessions.query` / `artifacts.query` 工具与定位规则。
- **验证**：
  - A-05：定位到 head；有歧义时先澄清。
  - A-06：新 session 引用同一个 workspace，并以 head 为 base。
  - A-12：decide(discard) 经 kmsg 送达驱动者，逐项报告。
  - 外部 workspace 的回滚报告为 unsupported，并列出变更与副作用。

### L6 多节点 DFS 与对象协议

- **交付**：DFS 上的 `write_lock`（DFS 独占写锁）；DID Object 宿主组件与 `PrincipalResolver` / `Authorizer`；测试用本地挂载。
- **验证**：
  - L1 / L3 的多进程与恢复用例在多节点 DFS 上重跑：不同节点上的进程争抢同一个 lease。
  - A-14：多个进程争抢同一个 session。
  - `agent_access=status_only` 时只能看到状态摘要。
  - 通过 DID Object 执行 `read` / `post_input`。

### L7 TS（buckyos-websdk）

- **交付**：先做 T1：复用 websdk 的 `msg_queue_client.ts` / `msg_center_client.ts` / `kevent_client.ts`；锁基于 flock / DFS 写锁。再做 T2，依赖 xllm TS 版的进度。
- **验证**：fixtures 一致性测试；Rust ↔ TS 互操作。

依赖关系：L0 → L1 → L2 → L3 → {L4, L5} → L6 → L7。之后是 OpenDAN 切换（附录 A）与 workspace 管理（另行设计），都不在本期。

---

## 14. 需求覆盖矩阵

“闭环依赖”一列打 ✔ 的项：libopendan 已提供所需原语，但端到端场景要等 OpenDAN 切换或 workspace 设计完成后才成立。

| 需求 | 落点 | 阶段 | 闭环依赖 |
|---|---|---|---|
| S-01 Task 充分准备 | session_config（prompt / objective / end_condition / workspace）；首轮装配 | L3 | |
| S-02 UI 持续反馈 | UI session 原语；应用或 OpenDAN 都可以驱动 | L3 | OpenDAN ✔ |
| S-03 执行位置与身份解耦 | 位置无关的 session 目录 + DFS 共享 + 登记表 | L1 | |
| S-04 创建 / 恢复 / 推进 | create / drive / runs resume + 反向读 worklog 构成上下文 + inflight | L1、L3 | |
| S-05 / S-06 应用装配、装配与循环分离 | session_config.prompt（.llm_context 超集）+ 固定合成顺序；`SessionAssembler` | L3 | |
| S-07 执行环境明确 | runtime.requirement + 首次推进绑定 | L2 | |
| S-08 权限不由提示词授予 | driver 身份 + BuckyOS 权限（DFS ACL / inbox 读权限）+ lease + 约束段不可覆盖 + App 身份调用 LLM | L1、L3、L6 | 接入 verify-hub |
| S-09 ~ S-11 意图分析、定位对象、一次任务一次工作 | fork 原语 + query 工具 + head 定位；finished 之后不 reopen | L3、L5 | UI 端 ✔ |
| S-12 产物继承 | 引用同一个 workspace + base = head | L5 | workspace ✔ |
| S-13 稳定执行 | input_policy、stop | L3 | |
| S-14 管理未完成工作 | registry.query + acceptance 维度 + pending_decision | L1、L5 | UI 端 ✔ |
| S-15 ~ S-19 订阅、去重、合并 | event / change 输入 + 登记表 rev 比对 + state.json 中的游标 + 终态独立 key + worklog 记录丢弃 | L1、L3 | 事件桥 ✔ |
| S-20 新鲜度 | PromptEnv 渲染时求值 | L3 | |
| S-21 多信道 | 登记表 + agent_access；应用驱动的 UI session 同样登记 | L1、L3 | OpenDAN ✔ |
| S-22 / S-23 一致性、提交与通知 | 单写者 + lease + fencing；state.json 为提交点，worklog 严格只追加；先提交 state 再确认输入源；幂等创建 | L1、L3、L6 | |
| S-24 ~ S-26 产物版本、回滚与范围 | 产物列表登记 + session 级 accept / discard + discard report（不可撤销的副作用逐项列出）；workspace 级版本与回滚另行设计 | L5 | workspace ✔ |
| S-27 任务回滚与 Memory 回滚 | `task_discarded` 感知保留来源；策略待定 | L4 | |
| S-28 ~ S-30 Memory 默认机制、异步整理、上下文入口 | 自动 round_digest + 游标 + hints / changes 分块 | L4 | 定时 ✔ |
| A-01 ~ A-03 | §10 | L3 | |
| A-14 | §5、§10.3 | L6 | |
| A-04、A-11 | UI 场景 | — | OpenDAN ✔ |
| A-05、A-06、A-12 | §6.5 | L5 | |
| A-13 | 并发修改冲突属于 workspace 的职责 | — | workspace ✔ |
| A-07 ~ A-10 | §4.6、§8.4 | L3 | |

---

## 15. 待决问题与风险

### 15.1 仍需确认

暂无待确认问题。以下事项另行设计，不阻塞本期：

- workspace 管理（Agent 内部 / 外部 workspace 的版本化、合并与回滚，Q13）；
- S-27 认知回滚策略；
- self-improve 空闲条件的细节（附录 A.5）。

外部依赖：msg-center inbox 改为以 `did + session_id` 标识（进行中），UI session 的 `MsgCenterInput` 依赖它。

### 15.2 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| 过渡期 opendan 与 libopendan 两份代码并存 | 修复不同步 | 移植 PR 中列出受影响的 opendan 位置；尽快进入 OpenDAN 切换 |
| `agent_session.rs` 体量大（8.5k 行），同时存储语义在变 | L3 周期长 | 先原样复制、编译通过，再逐类替换；把现有 56 个 session 单测一并移植作为回归集 |
| worklog 会变得很大 | 读取变慢、占用磁盘 | 只反向读、读到起点即停，读取量受上下文预算约束；全量读取只用于审计 / 导出；归档策略另议 |
| run 进行中的历史只在 `runs/` | run 目录损坏时丢失该 run 尚未写入 worklog 的历史 | checkpoint 与 run.json 都用原子替换写入；run 结束时先写 worklog 再删目录；reconcile 负责重做与清理 |
| run 结束时 worklog 批量写入量大 | 单次 IO 峰值 | 一次顺序追加 + 一次 fsync；机械压缩配置可以限制单个动作结果落盘的长度 |
| kmsg 的 ack 是累积的，永远不被消费的输入会卡住游标 | 输入窗口被堵死 | 持有者把不处理的输入显式标记为已消费并写入 worklog；`consumed_above` 设上限并告警 |
| kmsg 的 retention 与权限校验尚未实现（D-06、D-07） | 队列无限增长；越权投递 | 本期由驱动者在 ack 之后执行 `delete_message_before`；把补齐权限校验作为 L6 的前置条件 |
| msg-center inbox 改造（did + session_id）尚在进行 | UI session 的 `MsgCenterInput` 依赖它 | L3 先用“路由器转投 kmsg”验证 UI session；改造完成后切换为直接消费 |
| 多节点下 lease 过期由各节点自己的时钟判断 | 时钟偏差过大时可能提前接管 | zone 内节点做时钟同步，偏差远小于 TTL；如果 DFS 写锁自带会话失效机制，优先依赖它 |
| worklog 渲染必须是确定性的 | KV cache 失效、重建结果漂移 | 用 fixtures 验证字节级稳定；渲染器不读取时间、随机数等外部状态 |
| waist 变更（① ③）需要 TS xllm 同步 | 跨语言实现漂移 | 两个钩子都是可选项；L3 同步更新协议文档 |
| decide 必须由驱动者执行 | 驱动者离线时，决定会一直挂起 | 登记表的 `pending_decision` 让 UI 显示“等待 app2 处理”；该驱动者身份下的任一进程都可以处理 |

---

## 附录 A：OpenDAN 后续如何基于 libOpenDAN（本期不实施）

### A.1 启动

```python
async def opendan_main(args):
    rt = await init_buckyos_api_runtime(appid, owner, AppService); await rt.login()      # 沿用
    spec = await resolve_agent_spec(args.agent_did or bound_to(rt.app_instance_id))      # AgentDID 配置：system-config users/{owner}/agents/*
    root = agent_root(appid, owner, spec.agent_id)                                       # DFS 路径
    await sync_agent_rootfs_from_package(package_root(args), root)                       # agent pkg；保留本地修改（沿用）
    cfg = AgentConfig.load(root, overrides=spec)
    me = rt.app_principal()                                                              # 托管 session 的驱动者身份：OpenDAN 进程的 appid（Q16）
    agent = libopendan.AgentStateHandle.open(root, me)                                   # 锁基于文件写锁（flock / DFS）
    runtimes = RuntimeManager.from_config(cfg.runtimes)      # 默认：paios 容器内的 TmuxRuntime（Linux，内置 git）
    await runtimes.start_all()
    http = HttpServer(port=BUCKYOS_SERVICE_PORT or OPENDAN_SERVICE_PORT)                  # 与现有 Dispatch Runner 共用（4060）
    http.mount(f"/opendan/agents/{spec.agent_id}", libopendan.host.StateObjectServer(agent, auth=VerifyHubAuth(rt)))  # cyfs-gateway 路由（Q6）
    http.mount(RUNNER_KAPI_PATH, TaskRunnerHandler(...))     # agent.delegate（沿用）
    sup = Supervisor(cfg, agent, runtimes, who=me, notifier=OpendanNotifier(rt), outbound=libopendan.MsgCenterOutbound(rt))
    await gather(http.serve(), sup.run(), until=shutdown_signal())
```

### A.2 Supervisor：协程托管，无 awake queue

```python
class Supervisor:
    async def run(self):
        for e in agent.sessions.query(driver=me, run_state_not="finished") + agent.sessions.query(driver=me, pending_decision=True):
            self.ensure_task(e.sid)                           # 修复现状：重启后遗留的输入要等被触碰才恢复
        kevent.subscribe(f"/opendan/{agent_id}/session/*/input", on_event=lambda ev: self.ensure_task(ev.sid))   # 唤醒
        await gather(self.ui_session_discovery(), self.event_bridge(), self.self_improve_ticker(),
                     self.self_check_timer(), self.hosted_work_watcher(), self.poll_fallback())

    def ensure_task(self, sid):                               # 每个 session 一个协程；全部并行，没有全局队列
        if sid in self.tasks and not self.tasks[sid].done(): self.wake[sid].notify(); return
        self.wake[sid] = Notify(); self.tasks[sid] = spawn(self.session_loop(sid))

    async def session_loop(self, sid):
        sd = agent.sessions.open(sid); deps = self.deps_for(sd)
        while not self.shutdown:
            r = await libopendan.drive(sd, deps, until=StopWhen.idle)
            if r.not_driver or r.bind_failed or r.unregistered: return
            if r.finished and not sd.has_pending_input(): return
            if r.busy or r.lost:
                await wait_any(self.wake[sid].wait(), sleep_until(r.expires_at_ms)); continue
            fired = await wait_any(self.wake[sid].wait(), timeout=self.idle_unload(sid))   # UI 15 分钟，其它 3 分钟（沿用）
            if not fired and not sd.has_pending_input(): return      # 卸载协程；下次 ensure_task 再起
```

### A.3 基于 msg-tunnel 的 UI session（发现与托管）

```python
async def ui_session_discovery(self):                         # OpenDAN 有权读取 Agent 的各类 inbox（Q14）
    async for ib in msg_center.list_pending_inboxes(did=agent_did):  # 显式枚举有未读消息的 (did, session_id) inbox（不用裸 DID 的 get_next 隐式扫描）；kevent 加速 + 1s sweep
        if registry_has(ib.session_id) and registry_driver(ib.session_id) != me:
            continue                                          # 该 inbox 已由获准的应用驱动（§10.2），OpenDAN 不接管
        cls = cfg.dispatch.route("msg.group" if ib.is_group else "msg.chat")
        sd = ensure_ui_session(root / "sessions", ib.session_id, cls, driver=me,      # session_id 由 msg-tunnel 给出（platform:account:key）
                               inbox=(agent_did, ib.session_id), peer=ib.peer, tunnel=ib.tunnel)   # 幂等创建 + 登记
        self.ensure_task(sd.id)                               # session 协程用 MsgCenterInput 直接消费自己的 inbox，由 commit_pop 标记 Readed
```

MsgCenter 已按现有 session_id 划分 `$did/session_id` inbox，各 UI session 可直接用 `MsgCenterInput`。当 OpenDAN 需要重新决定会话归属时，由总路由器枚举待消费 inbox 再转投；不能用裸 DID 隐式扫描全部会话。

- 常规路径不再逐条转投到 kmsg。“投递成功后才 ack”的不变量由 §4.5 的 `commit_pop`（先提交 state.json 再标记 Readed）保证。
- 命令消息（原 `command_dispatcher`）改在 OpenDAN 注入的 session 输入预处理中执行，不进入推理，处理后同样标记为已消费。

### A.4 决策与托管 work session

- **UI 替用户做决定**：向目标 work session 的 kmsg 队列投递 `control(decide)`。驱动者是 OpenDAN 时，由 supervisor 被唤醒后执行；驱动者是应用时，由应用执行，此时登记表的 `pending_decision` 让 UI 显示等待状态。
- **应用委托托管**：应用创建 `driver = OpenDAN 的 appid` 的 work session，由 OpenDAN 用默认 runtime 执行。
- **agent.delegate**：改为 `create_session(kind=work, driver=opendan, task_binding)`；状态镜像通过 Notifier 完成。

### A.5 定期 self-improve：启动条件检查（Q1）

```python
async def self_improve_ticker(self):
    every(60s):
        c = cfg.session_class("self_improve")
        if c.enabled and self_improve_start_conditions(c):
            b = agent.perception.backlog(agent.perception.cursor())
            sd = SessionDir.create(root / "sessions", SessionSpec(kind="self_improve", driver=me,
                      objective="consolidate perceptions", prompt={"behavior": c.default_behavior},
                      extensions={"opendan": {"perception_window": b.window()}},
                      idempotency_key=f"si:{b.window_digest()}"), agent, me)
            self.ensure_task(sd.id)

def self_improve_start_conditions(c) -> bool:              # 启动条件检查，条件可配置
    return (agent_idle(c.idle_for)                         # ① 至少要空闲：没有 run_state=running 的其它 session，且 idle_for（默认 10 分钟）内没有新输入
            and not agent.leases.is_held("self_improve")   # ② 没有其它整理在运行（任意进程）
            and watermark_reached(c))                      # ③ 感知水位 ≥ threshold，或到达每日时点（沿用 20 / 03:00）
```

### A.6 其它保留职责

- **self-check**：timer 事件以 `event` 类型投递到 singleton `self_check` session。
- **事件桥**：按订阅把外部对象事件投递为 `event`（active）或 `change`（semi）。
- **worklog SQLite**（观测用）：可以逐步由各 session 的 `worklog.jsonl` 取代。

### A.7 切换时的改动清单

- **删除**：
  - `agent.rs` 中的 `sessions` / `session_locks` / `tunnel_to_ui_session` / `ensure_session_inner`；
  - 串行 `main_loop`、`restore_session_routes`；
  - `.meta/self_improve_scheduler.json`；
  - 所有已移植到 libopendan 的模块副本。
- **`agent_tool_cli_dev`**：改为依赖 libopendan，支持 `OPENDAN_SESSION_DIR` / `OPENDAN_INPUT_QUEUE`；同时修复两个现有问题：
  - `get_session` 读取的是旧路径；
  - CLI 直接改写 `.meta/session.json`，与内存中的 meta 竞争。
- **`agent-did-object-lib::AgentRuntimeAdapter`**（目前是桩）：实现 `agent://self/...` → `{base}/opendan/agents/{aid}/...`。
- **清理旧数据**：执行 `remove_open_dan_data.sh`。
- **DV 验证**：`uv run src/check.py`、`./debug_jarvis.sh`，再用新增的 DV case（`uv run test/run.py -p <case>`）覆盖 A-04、A-11。
