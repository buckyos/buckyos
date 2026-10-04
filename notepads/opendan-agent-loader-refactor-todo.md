# OpenDAN 重构需求：Agent Loader

- 状态：v0.7（2026-10-04）。M0–M3 已实施，通过库级、进程级验收（§10.3）和 DV 环境的真实 zone 验收（§10.6）。M5 切换还没做；M4 与 `agent.delegate` 按 §2.2 后做。
- 对象：`src/frame/opendan`（服务进程）与 `src/apps/jarvis_runtime`（Agent 包）。
- 依据：[xAgent.md](../doc/opendan/xAgent.md) v0.4、[Agent Session SDK 实现计划](<../doc/opendan/Agent Session SDK 实现计划.md>) 附录 A、`doc/opendan/protocol/`、[lib_opendan README](../src/frame/lib_opendan/README.md)。
- 读者：实施重构的开发者与 Code Agent。实施前先读 xAgent.md §0–§2、§4.6–§4.8、§7、§9.6、§11。

---

## 0. 结论先行

1. **OpenDAN 变成 Agent Loader**。Agent 内核（推理循环、Context 调度、Turn、输入总线、Runtime、Agent State 文件协议）已经在 `llm_context` / `agent_tool` / `libopendan` 里。OpenDAN 只负责：以某个 Agent 的 runtime app 身份启动，按配置把 Session 拉起来并托管，把 Agent State 作为服务提供出去。
2. **从零新写，不在旧代码上改**。旧 `opendan` 约 4 万行，其中 `agent.rs` + `agent_session.rs` 就有 1.3 万行，绝大部分职责已被下层取代。新代码只依赖 `libopendan` 的公开 API（`host`、`state`、`bridge`、`template`、`api`），不复制 Runner。
3. **功能三块，按顺序做**：
   - **A. Agent State 服务**：kRPC 服务端 + 只读为主的 WebUI。
   - **B. UI Session + Message Tunnel**：按配置发现 inbox、创建并托管 UI Session，接通 msg-center 入站与出站。
   - **C. self-check / self-improve**：按配置定时或按条件启动对应 Session（后做）。
4. **第一版目标是把架子跑起来**：能启动、能托管、能对话、能派出 work session、能在 WebUI 里看到状态、重启后不丢不重。不追求 Agent 能力。
5. **之后的能力迭代收束到 `src/apps/jarvis_runtime`**：memory、workspace、提示词、behavior、skill 的优化都通过 Agent 包里的配置与素材完成。判断标准是“改 Jarvis 的能力不需要改 Rust”；做不到的地方就是框架缺扩展点，补扩展点而不是往 Loader 里加特例（§5）。

---

## 1. 职责划分

| 职责 | 归属 | OpenDAN 做什么 |
|---|---|---|
| 推理循环、工具调用、快照、resume | `llm_context` + xllm | 不碰 |
| Session 目录协议、lease、Turn、Context 调度、输入路由、崩溃恢复 | `libopendan::runner` | 只调用 `drive` / `host::serve` 一类入口 |
| 工具执行环境（native / tmux / ssh） | `agent_tool::runtime` | 按配置选默认 runtime，交给 Session 工厂（xAgent §5.2.1） |
| Agent State 文件布局与锁 | `libopendan::state`（`FsAgentStateClient`） | 持有实例，注册为进程内实现，并对外提供 kRPC |
| behavior 冻结、Session 模板 | `libopendan::{state::behaviors, template}` | 不碰；素材来自 Agent 包 |
| **进程启动、身份、AgentRoot 同步** | **OpenDAN** | §3.1 |
| **托管：哪些 Session 该被推进、谁来推进、何时卸载** | **OpenDAN（Supervisor）** | §3.2 |
| **bridge：把系统事件翻译成 Session 输入** | **OpenDAN**（映射函数在 `libopendan::bridge`） | msg-center、kevent、timer、TaskMgr |
| **出站：把 Session 的回复发给人** | **OpenDAN** | 经 msg-center |
| **Agent State kRPC 服务与 WebUI** | **OpenDAN** | §4.1 |

Loader 里不应该出现：提示词文本、behavior 名字的特判、对 LLM 输出的解析、Session 状态的直接改写（只有驱动者经 `drive` 提交 state.json）。

---

## 2. 第一版的范围

### 2.1 做

- Loader 启动流程与 Supervisor（§3）。
- Agent State kRPC 服务端、`KrpcAgentStateClient`、WebUI（§4.1）。
- UI Session + msg-center 入站 / 出站（§4.2），包括 UI Session 派出的 work session 的托管。
- 可扩展框架的配置约定（§5），以及 Jarvis 包按新约定的**最小迁移**：一个 ui 入口 behavior、一组 work behavior，能对话、能做一个简单任务即可。
- 旧代码与旧数据的处置（§8）。

### 2.2 后做

- self-check / self-improve（§4.3），接口位置在 v1 留好。
- `agent.delegate`（TaskMgr Dispatch Runner）接入：改为创建 `driver = OpenDAN` 的 work session（附录 A.4）。放在 M3 之后（§9.1 第 5 项）。
- 群聊专有行为（@ 提及保护、群成员关系）、typing / status_line、graceful / discard interrupt 的顺序屏障。
- DID Object 宿主（`/opendan/agents/<aid>/…`）、ActionGuard / grant / 审批、多节点。

### 2.3 不做

本版是 breaking change，不向下兼容：旧数据、旧配置、旧消息格式都不保留读取路径。

- 不保留旧 Session 数据的兼容读取；切换时清理旧数据（`remove_open_dan_data.sh`）。
- 不保留旧 `agent.toml` 的 `[[channel]]` / `[dispatch]` / `[session.*.driver]` 结构与旧 behavior 字段名的兼容（libopendan 已不提供旧名兼容，xAgent §6.2）。
- 不做 Agent 能力优化（memory、hints 召回质量、提示词调优等）。

---

## 3. 总体结构

```text
 opendan 进程（身份 who = 该 Agent runtime app 的 app principal）
 ┌────────────────────────────────────────────────────────────────────┐
 │ Loader                                                             │
 │  ├─ 启动：登录 → Agent 定位 → AgentRoot 同步 → 读 agent.toml         │
 │  ├─ AgentStateClient（Fs 实现，注册为 InProcess）                    │
 │  ├─ HTTP（调度器分配的端口）                                         │
 │  │    ├─ kRPC：Agent State 服务                                     │
 │  │    └─ www ：WebUI 静态资源                                       │
 │  ├─ Supervisor：每个托管 Session 一个协程，循环 drive(Idle)          │
 │  └─ Loader 模块（按 agent.toml 启用）                                │
 │       ├─ ui        ：inbox 发现 → UI Session；msg-center 入站 / 出站 │
 │       ├─ self_check：timer → singleton Session                      │
 │       └─ self_improve：启动条件检查 → 一次性 Session                 │
 └────────────────────────────────────────────────────────────────────┘
        │ libopendan API            │ 文件 + flock           │ kmsg / kevent / msg-center
        ▼                           ▼                        ▼
   drive / serve / bridge      AgentRoot、Session 目录     BuckyOS 基础设施
```

### 3.1 启动

沿用实现计划附录 A.1 的顺序，要点：

1. `init_buckyos_api_runtime(appid, owner, AppService)` 并登录。
2. 定位 Agent：system-config `users/<owner>/agents/*` 中绑定到本 app instance 的 AgentDID 配置；没有就等待（沿用现有 `wait_for_bound_agent_spec` 的语义）。
3. AgentRoot：`$BUCKYOS_ROOT/data/home/<owner>/.local/share/<runtime_appid>/agents/<agent_id>`。从 Agent 包同步素材，**保留本地修改**（沿用 manifest 比对的做法）。
4. 读 `agent.toml`（§5.2）。配置错误在这里报出并退出，不带病启动。
5. 打开 `FsAgentStateClient`，`register_in_process`。之后进程内所有 `state::connect` 得到同一个实例。
6. 起 HTTP：kRPC 与 www。端口取 `OPENDAN_SERVICE_PORT`（§7 第 3 条）。
7. 起 Supervisor 与各 Loader 模块。
8. 收到退出信号：停止接收新工作，结束各 Session 协程并**等待它们退出**（释放 lease），不把 Session 标成 stopped（§7 第 9 条）。

### 3.2 Supervisor

- 启动时从登记表恢复：`driver = me` 且未结束的 Session，以及 `pending_decision` 的 Session，逐个 `ensure_task`。不依赖“被触碰才恢复”。
- 每个 Session 一个协程，循环 `drive(StopWhen::Idle)`；结果处理按 xAgent §9.6 的表（Busy 退避、阻塞类错误报告后退出该 Session、Idle 后检查待推进工作再决定等待或卸载）。
- 唤醒：队列通知 + 有界轮询。通知只是加速，轮询兜底。
- 子 Session：复用 `host::ChildDriver`，或把它的逻辑并进 Supervisor（同一个“按登记表找 `driver = me` 的未结束 Session”的循环）。实施时二选一，不要两套并存。
- 每个 Session 用 `HostDeps::runner_deps` 装配自己的 runtime 依赖，不共享 tmux 实例（xAgent §5.2.1 第 6 条）。
- Supervisor 对外暴露只读的托管状态（哪些 Session 在托管、上次 drive 结果、下一次唤醒原因），供 WebUI 展示。这份状态只在内存里，不是协议。

优先直接使用 `libopendan::host::{serve, run_session, ChildDriver}`。它们是为 xagent CLI 写的；Loader 需要而它们没有的能力（动态增删托管目标、托管状态查询、按 Session 的 idle unload）在 `libopendan::host` 里扩展，xagent 与 OpenDAN 共用，不在 OpenDAN 里另写一份循环。

---

## 4. 功能块

### 4.1 A：Agent State 服务与 WebUI

**kRPC 服务端**

- 门面与 `AgentStateClient` trait 一一对应：`sessions`、`activity`、`perception`、`cognition`、`artifacts`、`locks`、`behaviors`。真相仍是 AgentRoot 上的文件；服务是 `FsAgentStateClient` 外面的一层鉴权与转发，不引入第二份存储。
- 客户端 `KrpcAgentStateClient` 放在 `libopendan::state`，接到 `state::connect` 的 `Krpc` 分支（现在是转发桩）。`OPENDAN_AGENT_STATE_URL` 与 DID 文档 / zone 服务发现两种定位都要通。
- 验收沿用 xAgent E4：同一组 fixtures 跑在文件、进程内、真 kRPC 三种实现上，结果一致。
- **v1 的调用方范围**：看不到 AgentRoot 的非驱动者——容器或远端沙箱里的层 ② 工具（`agent-session recall / note / sessions / read-session / post / create-worksession`）、WebUI、其它应用。因此 v1 必须覆盖的是读操作与带署名的写（`register`、`post_input`、`notebook_append`、`artifacts.decide`）。带 lease 的驱动者写入（`report_state`、`perception.append`、`commit_cursor`）与 `locks` 依赖 flock，v1 的驱动者都在能看到 AgentRoot 的位置，v1 不上 kRPC（§9.1 第 6 项）。
- 鉴权：`who` 来自调用方的 BuckyOS 会话 token，由 verify-hub 校验；访问规则来自 Agent 包的配置（§4.4）。`acl.agent_access` 继续生效。
- **Session 目录读取**：Session 目录不经过 `AgentStateClient`，但 WebUI 和 `read-session` 需要读它。服务端增加只读的观测接口（state 摘要、worklog 尾部、report、run 列表），基于 `api::read_session`；不提供任何对 Session 目录的写接口。

**WebUI**

- 目的是观测与排障，v1 只读为主。页面：
  - Session 列表：按 class / run_state / 父子关系；活动视图（谁在跑、在碰什么）。
  - Session 详情：当前 Turn、`waiting_for`、`process_stack`、`pending_events`、最近 worklog、report、子 Session、runtime 绑定、冻结的 behavior 版本。
  - Agent State：感知积压与游标、认知（notebook / hints）、产物列表与 head、behavior 目录与 revision。
  - Loader：托管状态、各模块与 bridge 的状态、最近错误。
- v1 可选的操作只有三个，都走已有协议：`stop`、`decide accept|discard`、向有队列的 Session `post` 一条消息。其余一律只读。
- 数据全部来自 kRPC 服务，WebUI 不读文件。先定 UI DataModel 与 mock，再接真后端（与 desktop 里其它应用的做法一致）。
- 挂在 Agent app 已声明的 `www` 端点（`dapp_meta/app.json`）。源码在 `src/frame/opendan/web`。

### 4.2 B：UI Session + Message Tunnel

一条 UI Session 与 msg-center 的一个会话是**双向绑定**的：消息从哪个 inbox 进来，回复就沿同一个会话、同一个渠道出去。本节先列 msg-center 的现状（以 `src/frame/msg_center/` 与 `buckyos-api/src/msg_center_client.rs` 为准，2026-10-03 核对），再给出绑定、入站、出站三段流程。

#### 4.2.1 msg-center 现状

| 事实 | 说明 |
|---|---|
| inbox 是 `(owner DID, session_id)` | `MailboxAddress`，字符串形式 `<did>[/<percent 编码的 session>]`。箱子类型：`INBOX` / `SENT` / `GROUP_INBOX` / `REQUEST_BOX` |
| 记录的 session 由 msg-center 推导（`derive_session_id`） | 以发送方声明的 `msg.to_session` 为准，**不写就是默认会话**，由 msg-center 处理（私聊默认会话是 `dm:<对端 DID>`，群是群地址串 `<group_did>[/<sid>]`）。`thread.topic` 不是会话键。代码里还有按 `thread.correlation_id`、`meta.session_id` 推导的旧回退，新实现不依赖也不产生这些字段 |
| SENT 与 INBOX 用同一规则 | 私聊缺省时 SENT 取 `dm:<to>`、INBOX 取 `dm:<from>`，所以同一段私聊的收发两个方向落在同一个会话 |
| tunnel 入站 | `from` 是对端的本地影子 DID（`did:msgtunnel:…`，内含 tunnel 实例与平台账号）；`to_session = <platform>:<bot 账号>:<chat>`（`build_msg_tunnel_ui_session_id`）；私聊 `to = [owner]`，群聊 `to = [群的影子 DID]`、`kind = GroupMsg`。`record.ingress` 只是审计信息，不是路由输入 |
| 出站只有 `post_send(msg, idempotency_key)` | 选路只看 `msg.to`，且是确定的：影子 DID → 它内含的 tunnel；其它 DID → MessageHub 原生投递。**没有默认 tunnel，没有“最后活跃渠道”回退**，解析失败则整个 `post_send` 失败 |
| `post_send` 的结果 | 写一条 SENT 记录（表示“已离开我的发件箱”，不表示送达）和每个目标一条 `DeliveryRecord`。投递状态 `WAIT → SENDING → SENT`，失败退避重试，最多 5 次后 `DEAD` |
| DEAD 的兜底 | 消息 `meta.delivery_failure_notice` 带了文本时，msg-center 在 DEAD 后代发一条纯文本提示（带 `delivery_failure_fallback`，不会再触发兜底） |
| 幂等 | `post_send` 的幂等键按作者范围保存 30 天，有行数上限；重发同一个键返回首次结果 |
| 读状态 | `Unread → Reading → Read`；`get_next` 默认 `lock_on_take = true` 把记录置为 `Reading`，**没有租约**。允许 `Unread → Read` 与 `Reading → Unread` |
| 权限 | 带锁的 `get_next` 需要 inbox 的写权限，不带锁只要读权限；`post_send` 需要 `sent/<from>/<session>` 的写权限。Loader 怎么得到这些权限见 §4.4 |
| 事件 | 每个箱子一个事件：`/msg_center/<owner>/<box_kind>/<hex(mailbox)>/changed`，payload 带 `operation`、`record_id`、`session_id` |
| 易失状态 | typing / status_line 按 `(owner, session_id, key)` 存取（`update_owner_ui_session_state`），不走可靠投递 |

#### 4.2.2 绑定

**渠道不需要单独记。** 回复规则只有一条：`to_session` 原样取来信的 `msg.to_session`，来信没有就不填。 渠道已经编码在对端 DID 里（影子 DID 内含 tunnel 实例），会话已经编码在 `to_session` 里。“从哪来回哪去”就是：回复的 `to` 取来信的说话对象，`to_session` 取来信的会话。`ReplyRoute.tunnel` 只用于审计。

| 场景 | 来信 | Agent inbox 的 session | 回复 `to` | 回复 `to_session` | `kind` |
|---|---|---|---|---|---|
| MessageHub 私聊，命名会话 | `from = 用户`，`to_session = S` | `S` | 用户 | `S` | Chat |
| 私聊，默认会话 | `from = 对端`，无 `to_session` | `dm:<对端>` | 对端 | 不填 | Chat |
| tunnel 私聊 | `from = 影子 DID`，`to_session = tg:bot:chat` | `tg:bot:chat` | 影子 DID | `tg:bot:chat` | Chat |
| tunnel 群聊 | `to = [群影子 DID]`，`to_session = tg:bot:chat` | `<群影子 DID>/<编码的 sid>` | 群影子 DID | `tg:bot:chat` | GroupMsg |
| 自托管群，默认 / 命名会话 | `to = [群 DID]`，`to_session = 无 / S` | `<群 DID>` / `<群 DID>/<S>` | 群 DID | 无 / `S` | GroupMsg |

绑定关系落在三处，各管各的：

1. **inbox → Agent Session（Loader 维护）**。一个 inbox 同一时刻对应一个当前的 UI Session。登记表条目带 `route_key = MailboxAddress 字符串` 与箱子类型；“当前 Session”= 该 `route_key` 下最新的未结束 Session。不另建映射表，重启后从登记表重建（取代旧的内存表 `tunnel_to_ui_session`）。
2. **Agent Session → 回复坐标（创建时固定）**。一个 inbox 就是一段对话，所以 `to`、`to_session`、`kind` 在 Session 生命周期内不变。创建 UI Session 时把它们写进 session_config（不可变部分）。
3. **每条回复的 `reply_to`（随输入变化）**。来自 `state.reply`，即最后消费的那条消息的 ObjId。

出站时以第 2 项为准；`state.reply` 给出的 `to` / `to_session` 若与第 2 项不一致，说明消息被投进了错的 Session，记错误并不发送。

sid 由 `route_key` 派生（散列或规整，加一个代次），不直接用地址串：地址里有 `:`、`/`、`%`，而 sid 要进 kevent 路径和 tmux 名。

同一个人从 MessageHub 和 Telegram 来是两个 inbox（DID 与 `to_session` 都不同），对应两个 UI Session。跨渠道认出是同一个人是 ContactMgr 的事，Loader 不合并会话，也不把联系人合并结果改写进 `from` / `to`。

#### 4.2.3 入站流程

1. **发现**：按 `[[loader.ui]]` 规则对 Agent DID 分别 `list_mailboxes(INBOX)`（v1 不消费 `REQUEST_BOX`）。订阅 `/msg_center/<agent>/inbox/**` 加速，定时全量扫描兜底。`GROUP_INBOX` 是群主机一侧的权威箱，Agent 作为成员收到的群消息已投影到自己的 INBOX，不消费 `GROUP_INBOX`。
2. **归属**：`route_key` 在登记表里已有且 `driver != me` → 跳过（该 inbox 由获准的应用驱动）。没有当前 Session → 按规则的模板创建，`driver = me`，带输入队列，写入 §4.2.2 第 2 项的回复坐标。
3. **取记录**：`get_next(mailbox, INBOX, [Unread], lock_on_take = false, with_object = true)`。不加锁读，避开 `Reading` 无租约的问题；本 inbox 只有 bridge 一个消费者，重复读到同一条靠总线 key（消息 ObjId）去重。
4. **翻译**：`bridge::msg::route_msg_record`。普通消息 → `SessionMsg{msg, delivery}`，MsgObject 原样不改；已登记的斜杠命令 → `control` 或由 Loader 自己处理；回声、空消息、reaction、群邀请通知 → 丢弃。
5. **投递**：`sessions().post_input`。
6. **确认**：投递成功或决定丢弃后 `update_record_state(record_id, Read)`。`input_full` 不确认，该 inbox 停在这条记录上退避重试（背压）。投递成功但确认失败 → 下次重读、重投，被去重。
7. `ensure_task(sid)`。

#### 4.2.4 出站流程

要发的东西有几类，v1 只做前两类：

| 类别 | 来源 | v1 |
|---|---|---|
| Turn 的回复 | Turn 关闭时的最终回答；`WAIT_USER_MSG` 之前交付的回复 | 做 |
| Loader 自己的回复 | 应用级斜杠命令的应答、转换失败的兜底提示 | 做 |
| 推理过程中的中间消息 | 类似旧 `sendmsg` 的工具 | 后做 |
| work 子 Session 的报告直发用户 | 旧 `worksession report` | 后做；v1 经父 UI Session 转述 |
| Agent 主动发起（没有来信） | 需要显式目标的发送工具 | 后做 |
| typing / status_line | Supervisor 的托管状态 | 可选；不来自 LLM |

**提交与发送分两步，中间可恢复：**

1. **生成**：产生回复的那次提交（Turn 关闭，或等待前交付回复）里，由回复坐标 + `state.reply.reply_to` 生成信封（`bridge::msg::outbound_base`），经 `llm_context::ai_message_to_msg_object_with_base_validated_async` 填内容（文本、`<attachment>` 标签转成 `refs`；需要附件校验与 NamedStore 解析器）。**完整的 MsgObject 连同幂等键 `outbound_key(sid, turn, run_id, n)` 写进 state.json 的待发列表，与该次提交同时落盘。** `created_at_ms` 在这里定死，重发不重新生成，否则 ObjId 会变。
2. **发送**：驱动者提交之后调用 `OutboundSink::send(record)`。OpenDAN 的实现是 `post_send(msg, idempotency_key = key)`；xagent 的实现是打印或丢弃。
3. **标记**：`post_send` 返回 `ok` → 下一次提交把该条标为已发，记下 `msg_id` 与 `delivery_id`。返回被拒（如 `blocked_author`、选路失败）→ 标为失败并记原因，不重试。RPC 错误 → 保留待发，退避重试。
4. **恢复**：重启后 reconcile 发现待发列表非空 → 直接重发原 MsgObject。msg-center 按幂等键返回首次结果，不会出现第二条消息。

约束：

- 发送失败不让 Turn 失败，也不阻塞下一批输入；但同一 Session 的待发消息按序发送。
- 待发消息带 `meta.delivery_failure_notice`（文案来自 i18n），让 msg-center 在投递 DEAD 时代发兜底提示。
- 内容转换失败 → 发一条 i18n 的失败提示代替（带 `delivery_failure_fallback`），原回复留在 worklog。
- 空回复（无文本、无附件）不发。
- 群聊回复带 `reply_to`；群里 Agent 自己的消息会回流到 inbox，由入站第 4 步按回声丢弃。
- headless 的 work 子 Session 的 `state.reply` 是 `ParentSession`，`outbound_base` 返回 `None`，不产生出站消息。
- 投递最终结果（SENT / DEAD）v1 不回流给 Agent，只在 WebUI 的 Session 详情里显示（读 `DeliveryRecord`）。

`OutboundSink` trait、state.json 的待发列表与 reconcile 重发都落在 libopendan（现在只有信封 helper，xAgent §11.3 列为后移项）。按“下层先行”的规则单列 TODO，先于 Loader 的 UI 模块实施。

#### 4.2.5 UI Session 派出 work session

- UI 的入口 behavior 用 `call_behavior` 做路由判断（原 `try_create_worksession`，xAgent §3.6），需要时用层 ② 工具 `agent-session create-worksession` 创建子 Session。
- 子 Session 由 Supervisor 托管，汇报按 `--report`（xAgent §4.14）；结果作为事件进入父 UI Session，由父生成回复，沿父的绑定发出。
- 这条链路是 v1 的主验收场景（§6 M3）。

#### 4.2.6 依赖的下层缺口

1. **出站**：`OutboundSink`、待发列表、reconcile 重发（§4.2.4），落在 libopendan。
2. **正式的 UI kind**：现在 ui 模板落在 work kind 上。
3. **提问是否关闭 Turn**（xAgent §11.5 第 1 条）直接影响“回复后继续等用户”的体验，需先定。

v1 不做会话轮换（同一对端切到新的 UI Session）。以后要做时，形式是同一 `route_key` 下结束旧 Session、创建下一代。

### 4.3 C：self-check / self-improve（后做）

- self-check：`TimerBridge` 向 singleton `self_check` Session 投 timer 事件。timer 间隔的配置约定未定（xAgent §11.5 第 6 条）。
- self-improve：ticker 做启动条件检查（空闲、没有其它整理在跑、感知水位或每日时点），满足则创建一次性 `self_improve` Session，幂等键为感知窗口摘要（附录 A.5）。不再用 `.meta/self_improve_scheduler.json`。
- v1 只要求：`[loader.self_check]` / `[loader.self_improve]` 配置段可解析，默认关闭，模块接口与 ui 模块一致。

### 4.4 权限：由 Agent 包的配置决定

Loader 自己不带任何写死的权限假设。它需要什么权限，取决于它加载的那个 Agent 包（agent-rootfs）里的声明，做法与现在的 app-loader 一致：

- **声明**：Agent 包声明这个 Agent 需要的权限（上限），与 AppDoc 的 `permissions` 同一套语义。位置是 `dapp_meta/app.json` 的 `permissions`（现在是空数组）：Agent 包本身就是一个 app。条目格式沿用 [permissions 配置指南](<./service_config_tips + permissions 实用配置指南.md>)，不新造一套；`agent.toml` 不放权限。
- **授予**：安装时用户批准的是声明的子集；调度器据此下发授权。Loader 运行时以 app principal 身份使用这些授权，不自行扩大。
- **用在哪**：
  - msg-center：读写 Agent DID 的 inbox、写 sent、写 Agent 名下的 UI 状态（§4.2）。**v1 先跳过**：现有 scope 里没有对应条目，不声明、不校验声明，直接按 msg-center 当前的行为使用（§9.1 第 16 项、§9.2）。
  - Agent State 服务：谁能读、谁能 `post_input` / `register` / `decide`，同样由包的配置给出默认规则，服务端按规则校验调用方的 `who`。
  - Session 的 runtime 与文件访问范围（如现有的 `filesystem_policy`）、是否允许出站联网等。
- **降级**：用户没有批准某项可选权限时，对应的 Loader 模块不启动并在 WebUI 里标明原因，其它模块照常运行。

权限判断一律按稳态 token 设计与测试（§7 第 4 条）。

---

## 5. 可扩展框架

### 5.1 原则

- **Loader 是通用的**：同一个二进制能加载任何符合约定的 Agent 包；Jarvis 只是其中一个包。
- **Agent 的个性全部在包里**：身份、behavior、模板、工具清单、skill、i18n、记忆与 workspace 的策略配置。
- **扩展点先于特例**：迭代中发现“必须改 Rust 才能调 Agent 行为”，先判断是不是缺扩展点；是就补扩展点并写进本节的表。

### 5.2 Agent 包里能改什么

| 想改的东西 | 改哪里（`src/apps/jarvis_runtime/agent/`） | 由谁读取 |
|---|---|---|
| 身份与人设 | `role.md`、`self.md` | `BehaviorCatalog::identity`，冻结进 Session |
| 某个阶段的提示词、模型、工具、预算、进入模式 | `behaviors/<name>.toml`、`*.inc` | `BehaviorCatalog`，冻结 |
| Session 形态（Turn 数、等不等用户、要不要队列、默认 behavior、深度上限） | `agent.toml [session.<class>]` | `SessionTemplate::load` |
| 启动哪些 UI 入口、哪类消息进哪个模板 | `agent.toml [[loader.ui]]` | OpenDAN |
| self-check / self-improve 开关与条件 | `agent.toml [loader.self_check]`、`[loader.self_improve]` | OpenDAN |
| 需要的权限（msg-center、文件范围、联网、Agent State 访问规则） | `dapp_meta/app.json` 的 `permissions`（§4.4） | OpenDAN / 调度器 |
| 默认 runtime（native / tmux）与环境 | `agent.toml [runtime]` → `prompt.llm_context.runtime` | Session 工厂 |
| 工具与 CLI | 包内 `bin/`、`bash_tools` 声明、skill 目录 | Runtime / bin overlay |
| 状态行、失败提示等文案 | `i18n/*.toml` | assembler |
| memory / hints / workspace 能力 | 包内的 CLI 工具 + behavior 提示词（§5.4） | Runtime（经 `exec`） |

### 5.3 `agent.toml` 草案

只示意 Loader 新增的段；字段在实施时定稿，定稿后反写到 `doc/opendan/`。

```toml
[loader]
agent_state_service = true
webui = true

[[loader.ui]]
on = "msg.chat"              # msg.chat | msg.group
session_class = "ui"
sid_strategy = "per_peer"    # per_peer | per_group
idle_unload_secs = 900

[[loader.ui]]
on = "msg.group"
session_class = "group"
sid_strategy = "per_group"

[loader.self_check]
enabled = false

[loader.self_improve]
enabled = false

[session.ui]                 # libopendan SessionTemplate 的覆盖项
default_behavior = "chat_route"
```

### 5.4 能力扩展走标准 CLI 工具

memory、workspace 以及以后的新能力，默认做成 **Agent 包里的 CLI 工具**（xAgent §5.5 的层 ②），不在 Loader 或 Runner 里加进程内 tool（层 ③）：

- 工具是包内 `bin/` 下的可执行文件，经 bin overlay 进入 Session 的 PATH，在 `bash_tools` 里声明 schema；LLM 看到的是普通 function / action，执行经 `exec` 进 Runtime。
- 需要读写 Agent State 的工具在子进程里 `AgentStateClient::connect`（本机文件或 kRPC），与 `agent-session recall / note` 的做法相同。这也是 Agent State 服务必须有 kRPC 形态的原因。
- 什么时候用、怎么用，由 behavior 提示词决定。
- 好处：改能力只改包；工具可以单独在终端里跑和测；xllm 能接手使用这些工具的 run；容器与远端 runtime 下同样可用。

只有 CLI 做不到的事才考虑别的扩展点，并先在本节登记理由。目前已知的一个候选是“每批输入前自动注入的材料”（如 hints 召回）：优先看能否由输入模板调用同一个 CLI 得到，而不是在 Loader 里加 Rust 钩子；v1 不做，现有 `recall_hints` 的行为不变。

### 5.5 Jarvis 包的最小迁移

现有 `agent.toml` 与 `behaviors/*.toml` 是旧结构。v1 只迁移跑通主链路所需的最小集，其余 behavior 先不动、不加载：

- `agent.toml`：删除 `[[channel]]`、`[dispatch]`、`[session.*.driver]`；`loop_mode` 移到 behavior 的 `prompt.mode`；加 §5.3 的段。
- behavior：`prompt.on_init` → `prompt.system`；`on_wakeup` / `on_behavior_switch` → `on_input` / `on_context_switch`；每个非入口 behavior 声明进入模式（`switch_context` / `create_sub_context` / `fork`），没有缺省回退。
- 先迁移：`chat_route`（ui 入口）、路由用的子 context behavior、`plan` / `do`（work）。`groupchat_route`、`self_check`、`self_improve_*`、`improve_skills`、`try_create_new_skill` 随对应功能块迁移。

---

## 6. 里程碑与验收

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M0 准备 | 解除 `agent_tool_cli_dev` 对 `opendan` crate 的依赖；旧 crate 改名为 `opendan_legacy`（§9.1 第 4 项）；`agent.toml` 新结构解析；下层先行项各自立 TODO | workspace 能在不编译旧 opendan 的情况下构建 |
| M1 Loader 骨架 + Agent State 服务 | §3.1 启动、Supervisor、kRPC 服务端与客户端 | `./debug_jarvis.sh` 能起；用 xagent 创建的 `driver = OpenDAN` 的 work session 被托管推进到结束；`xagent --state-url …` 经 kRPC 能 `list` / `status`；E4 三种实现一致；kill 后重启，未结束的 Session 自动恢复 |
| M2 WebUI | §4.1 页面与三个操作 | mock 与真后端下页面一致；能看到 M1 的 Session 从创建到结束的状态变化 |
| M3 UI Session + Tunnel | §4.2 入站、出站、子 Session 托管；Jarvis 最小迁移 | 在 MessageHub 给 Jarvis 发消息得到回复；让它做一个需要 work session 的任务并收到结果；推理中、回复发送前后分别 kill，重启后不丢消息、不重复回复；队列满时不确认上游 |
| M4 self-check / self-improve | §4.3 | timer 触发 self_check；满足条件时恰好启动一个 self_improve |
| M5 切换 | 删除旧代码、清理旧数据、文档整理（§8） | `uv run src/check.py` 与 DV 用例通过；容器形态部署回归 |

每个里程碑记录实际运行的命令与结果，不照抄历史数字。基础验证命令（在 `src/` 下）：`cargo test -p libopendan -- --test-threads=1`、`cargo build -p libopendan --bin xagent`、`cargo build -p opendan`。

---

## 7. 之前踩过的坑

新实现要逐条对照；多数已经由下层的设计规避，Loader 不要再绕回去。

**托管与状态**

1. **全局串行 main_loop 与 awake queue**：一个 Session 卡住拖住全部。新模型是每 Session 一个协程，各持自己的 lease。
2. **重启后遗留的输入要等被触碰才恢复**：启动时必须主动从登记表恢复未结束的 Session。
3. **端口**：实际端口由调度器按 AppIndex 分配（Jarvis 是 10016，不是 AppDoc 声明的 4060），经 `OPENDAN_SERVICE_PORT` 注入；4060 只是原生调试形态的回退。上报给别的服务的 endpoint 目前只允许 loopback。
4. **稳态身份不是 zone-trusted**：AppService 启动时拿的是设备签发的 token，首个 keep-alive（约 5 秒）后换成 verify-hub 签发的正式 token。启动窗口里能过的权限，稳态会失败。所有权限判断按稳态的 app 身份设计，并在启动 30 秒后的路径上测。
5. **Session 的驱动者身份是 OpenDAN 的 appid，不是 Agent DID**：读 inbox、回复、调 LLM 都用它。
6. **多个写者改同一份 Session 状态**：旧 CLI 直接改写 `.meta/session.json`，与内存里的 meta 竞争。现在只有 lease 持有者经提交点写 state.json；子进程想改状态就投 control。Loader 也不例外。
7. **“假 fsync”与静默重置**：旧 journal 写入不落盘、损坏时当空文件启动。Loader 自己如果有持久状态（尽量不要有，托管状态都能从登记表重建），写入要真 fsync，损坏要报错停下。
8. **取消协程后不等待**：tokio 任务 `abort()` 后不 `await` JoinHandle，它持有的 lease 不释放，下一次 drive 得到 Busy。卸载与退出路径都要等。
9. **宿主的 SIGINT 不等于 stop Session**：常驻宿主退出只结束托管，Session 保留已提交状态；只有显式 stop 才结束 Session。
10. **drive 的 future 很大**：在测试线程或小栈协程里会栈溢出，需要 `Box::pin`。
11. **进程内 task 随宿主退出丢失**：重启后按 Unknown 回填，不重放。需要跨重启的长任务用可持久查询的外部 task。
12. **tmux runtime 的 id 必须等于 sid**：显式给了别的 id 会 BindFailed；不同 Session 不共用 tmux 目标。

**消息与事件**

13. **ack 的两个方向都错过**：没有真正落库成功的不能 ack；已经决定丢弃的必须 ack。
14. **msg-center 的 `Reading` 没有租约**：带锁 take 之后崩溃会一直卡住。入站桥用不加锁的读取，投递成功后直接置 `Read`（§4.2.3）。
15. **inbox 的 session 由发送方声明**：现在是 `msg.to_session`（早先是 `thread.topic`），不是可信的归属证明；Session 归属以登记表的 `route_key` 绑定为准，回复坐标在创建时固定并在出站时核对（§4.2.2；历史修复 `773e9f7f`、`f0c06c25`）。
15a. **出站没有默认渠道**：`post_send` 只按 `msg.to` 选路，没有“最后活跃渠道”回退。回复目标必须是来信的对端 DID（tunnel 来信就是影子 DID），写成联系人的规范 DID 会走错渠道或失败。
15b. **重发时重新生成消息**：`created_at_ms` 变了 ObjId 就变，幂等键之外又多一条消息。待发消息必须整条持久化后原样重发。
16. **事件只是加速**：kevent 跨节点投递未接线、可能丢。任何依赖事件的路径都要有“重读状态”的幂等处理和轮询兜底。不做全局通配订阅（如 `/task_mgr/**`）。
17. **kevent 事件名每段只允许字母、数字和 `_ - .`**：DID 不能直接进路径，用 `agent_id`。
18. **kmsg 的限制**：`create_queue` / `subscribe` 不幂等；`commit_ack` 可回退、无 fencing；没有权限校验与 retention；直接写 kmsg 绕过 64 条上限。所有投递走 `SessionRegistry::post_input`。
19. **系统事件的错误不要回复给用户**（历史修复 `703741ff`）：timer、task 事件触发的 Session 失败时，没有可回复的人。
20. **入站消息要保留结构化边界**（历史修复 `bf54fcdb`）：现在总线上直接放 MsgObject，bridge 不改写、不裁剪、不拼接。
21. **后台任务的回复语言**（历史修复 `b22be253`）：迟到的任务结果要沿用用户的语言；语言与时区来自 Session 绑定，不取执行机设置。

**架构**

22. **工具里嵌一个推理**（旧 `fork_and_run_agent_loop`，工具持有 `Weak<AIAgent>`）：违反串行约束、崩溃即丢。改为 `call_behavior` 触发的子 context。
23. **同一个 run 里换 system**（旧“普通切换”）：已废弃，没有回退。
24. **渲染带副作用**：旧 `prompt_env` 在渲染时推进游标并写盘。渲染必须是纯函数，游标随 receipt 提交。
25. **全局 SQLite worklog**：观测数据另存一份，容易与真相不一致。WebUI 直接读 worklog.jsonl 与登记表。
26. **`agent_tool_cli_dev` 反向依赖 `opendan`**：工具 CLI 不应该依赖服务 crate。先解掉这条依赖，新 opendan 才能自由重写。
27. **AgentRoot 同步覆盖本地修改**：Agent 自己改过的文件（skill、notebook 等）升级包时不能被覆盖。

---

## 8. 旧代码、旧数据与文档

- 旧 `src/frame/opendan/src/` 的模块按实现计划 §12 的表逐个核对去向：已移植到 libopendan 的直接删除；托管职责（`main.rs` 启动与 rootfs 同步、`msg_center_pump.rs` 的路由部分、`command_dispatcher.rs`、`dispatch_adapter.rs` / `agent_task_executor.rs` / `task_event_pump.rs`、`contact.rs`）作为新实现的参考，不复制结构。
- `dispatch_adapter.rs` 一组（TaskMgr 2.0 Dispatch Runner，见 [完成记录](./opendan-taskmgr2-followup-todo.md)）是近期才做完并有测试的代码；`agent.delegate` 模块把这部分整体搬过来，只把“启动任务”的实现换成创建 work session。
- 旧数据：切换时执行 `remove_open_dan_data.sh`；不做迁移工具。
- `doc/opendan/` 里与新架构冲突的文档（`NewOpenDANRuntime.md`、`Agent Session.md`、`Agent配置改进.md`、`OpenDAN Agent配置开发指导.md`、`behavior 提示词编写逻辑.md`、`Agent Task Executor.md` 等）在 M5 统一处理：按新分层重写或删除，不留 legacy 段落。新增一份 OpenDAN（Loader）自身的设计文档与 Agent 包开发指南，取代本文。

---

## 9. 决定与待确认

### 9.1 已定（2026-10-03）

1. **本版是 breaking change，不向下兼容**（用户定）。
2. **权限由 Agent 包的配置决定**，参考 app-loader（用户定，§4.4）。
3. **`to_session` 不写就是默认会话，由 msg-center 处理**（用户定）。回复只原样带回来信的 `msg.to_session`；不处理旧提示字段。
4. **crate 处置**：旧目录改名 `src/frame/opendan_legacy`（包名同改，只为过渡期对照，不再维护）；新代码写在 `src/frame/opendan`，包名与二进制名不变；M5 删除 legacy。
5. **`agent.delegate`（TaskMgr Dispatch Runner）**：作为第四个 Loader 模块放在 M3 之后，复用现有 `dispatch_adapter`，把“启动任务”换成创建 work session；没有真实调用方之前可以不注册 runner。
6. **kRPC 范围**：v1 只服务非驱动者。带 lease 的写入与 `locks` 继续走文件 + flock；出现看不到 AgentRoot 的驱动者时再设计。
7. **WebUI**：源码放 `src/frame/opendan/web`，沿用 desktop 的前端栈与 UI DataModel 流程，由 opendan 的 www 端点提供。
8. **msg-center 接入**：bridge 转投到 Session 队列（xAgent.md 的做法），Session 不直接消费 inbox。
9. **会话轮换**：v1 不做。
10. **`REQUEST_BOX`**：v1 不消费。
11. **待发列表**：放 state.json，与产生回复的提交同一个提交点。

12. **权限声明放 `dapp_meta/app.json` 的 `permissions`**（用户定）。`src/apps/jarvis_runtime` 本身就是一个 app，Agent 包按 app 的方式声明权限。
13. **一个进程托管一个 Agent**（用户定）。kRPC 路径与 WebUI 不带 agent 维度。
14. **扩展优先用标准 CLI 工具，尽量不开进程内 tool**（用户定，§5.4）。memory / workspace 的最小扩展接口按这个原则定。
15. **xAgent §11.5 中影响 UI 的几条先按下面的默认选择走**。它们不属于核心框架，之后可以单独调整，不阻塞 Loader：
    - 提问是否关闭 Turn：保持现状，等待前已交付回复即完成该 Turn，用户的回答开启下一个 Turn。
    - 无队列 Session 的验收状态：以 artifact 为准；与 UI Session 无关（UI Session 都有队列）。
    - timer / kevent 配置约定：沿用 `extensions.opendan.timers.<name>.every_secs` 与“object 即 kevent id”。
    - ChildDriver 轮询成本：在 `libopendan::host` 里改成只在登记表 rev 或队列有变化时再驱动空闲 Session；托管大量空闲 UI Session 之前完成即可。

16. **msg-center 权限的 scope 表达先跳过**（用户定）。`permissions` 里现在没有对应 msg-center 新 inbox 模型的 scope，v1 不为它设计条目，`app.json` 里不声明 msg-center 权限；§4.4 的声明机制先用于已有 scope（文件范围、联网等）。等 msg-center 的权限模型定了再补。

### 9.2 实施时要核对

1. **msg-center 现在实际校验到什么程度**。代码里 `get_next` / `post_send` 都会走 `authorize_mailbox`（`owner_session.rs`：owner 读写身份，非 owner 走按会话的 `delegate`），不是完全不校验；但它在稳态 token 下对 Loader 的 app 身份是放行还是拒绝，没有实测。M3 接 msg-center 时先在真实 zone 里跑一次收发：能通就按现状用；被拒就在这里记下缺的授权，再决定是 msg-center 放宽还是补授权。

---

## 10. 实施记录（2026-10-03）

### 10.1 落点

| 里程碑 | 内容 | 落点 |
|---|---|---|
| M0 | 旧 crate 改名并移出 workspace | `src/frame/opendan_legacy`（`Cargo.toml` `exclude`，不再编译；M5 删除） |
| M0 | 解除 `agent_tool_cli_dev → opendan` 依赖 | `agent_tool_cli_dev`：删除读旧 Session 格式的 `read_session_history` / `commit_session_history_improved`；attention-signal 工具保留，参数 / 结果类型搬进本 crate；`BeginAttentionSignalExtraction` 不再读旧 `.meta/session.json`（owner 固定为 `system`，不再拒绝“另一个 self-improve Session”）。新增依赖 `schemars`（workspace 已有，`TypedTool` 需要） |
| 下层先行 | 出站、ui kind、Supervisor、kRPC Agent State | libopendan，见 [lib-opendan-outbound-todo.md](./lib-opendan-outbound-todo.md) |
| M1 | Loader 骨架 | `src/frame/opendan/src/{main,loader,config,rootfs}.rs`；托管用 `libopendan::host::Supervisor` |
| M1 | Agent State 服务 | `service.rs`（鉴权 + `libopendan::state::krpc::serve_call` + `session.read` + 三个操作 + `loader.status`），接口见 [opendan README](../src/frame/opendan/README.md) |
| M2 | WebUI | `src/frame/opendan/web`（见 §10.4） |
| M3 | UI Session + Tunnel | `ui.rs`：`UiModule`（inbox 发现、绑定、入站桥）、`MsgCenterSink`（出站）、`MailService`（msg-center 的最小接口，zone 实现 `ZoneMailService`） |
| M3 | Jarvis 最小迁移 | `src/apps/jarvis_runtime/agent/agent.toml`、`behaviors/{chat_route,task_route,plan,do}.toml`、`i18n/{en,zh}.toml [outbound]` |
| 脚本 | 启动参数 | `debug_jarvis.sh`、`rootfs/bin/service_debug.tsx` 去掉 `--worksession-test` 透传 |

### 10.2 与本文不同或本文没写的决定

1. **端口**：启动命令是 `opendan --app-id <id> --agent-bin <pkg> --service-port <n>`（node-daemon 与 service_debug 的实际用法）；环境变量 `BUCKYOS_SERVICE_PORT`（node-daemon）与 `OPENDAN_SERVICE_PORT`（service_debug）都认。
2. **不再把 agent DID 写回 `agent.toml`**。旧实现每次启动改写包文件，导致它永远是“本地已修改”，包升级到不了。Agent DID 只来自 AgentSpec。
3. **`--dev` 形态**：`opendan --dev --agent-root … --agent-did … --queue-dir …` 不需要 zone（文件队列、不校验 token、没有 msg-center），用于本机验收与 WebUI 开发。
4. **UI Session 的代次**：sid = `ui-H("ui", agent_did, route_key, 代次)`，代次 = 该 `route_key` 下已有的 Session 数。v1 不做轮换，但 `/stop` 会结束 UI Session，之后同一个 inbox 的下一条消息创建下一代（否则这个 inbox 再也没人接）。
5. **斜杠命令**：只登记了 `/stop`（→ `control(stop)`），可发令的人是 Agent 的 owner。没有应用级命令，所以“Loader 自己的回复”这一类出站目前没有内容可发。
6. **`[llm_context]`**：`agent.toml` 顶层的 `[llm_context]` 表是 Loader 创建的 Session 的基础 `.llm_context`（provider、model、tools、runtime），behavior 在它上面叠加。没有再单设 `[runtime]` → `llm_context.runtime` 的映射；`[runtime]` 只读 `language`（选 i18n 文案）。
7. **包自定义 class**：`[session.<class>] base = "ui"`。`[[loader.ui]]` 引用的 class 必须是 ui kind 且有队列，否则启动报错。
8. **Agent State 服务的访问规则**：v1 是固定规则——Agent 的 owner（经任何 app）与 zone root 可读可写，其余拒绝；`who` = `app:<token.appid>@<token.sub>`。“规则来自 Agent 包配置”没有实现：`permissions` 里没有对应的 scope（与 §9.1 第 16 项同一个原因）。
9. **权限降级**（§4.4 最后一条）没有实现：v1 没有可选权限的声明，也就没有“未批准则模块不启动”。模块不启动的原因目前只有：没有规则、没有 msg-center、该版本未实现。
10. **self-check / self-improve**：配置段可解析、默认关闭；打开时模块状态显示“not available in this version”，不启动。
11. **回复的失败提示**：Turn 失败（`failed` / `budget_exhausted`）且该 Turn 回答的是一条消息时，发 i18n `[outbound] turn_failed`；只由系统事件触发的 Turn 失败不发（§7 第 19 条）。文案缺失就不发。
12. **附件出站**：`compose_text` 没有接 NamedStore 的本地文件解析器（旧 `attachment_resolver.rs` / `attachment_policy.rs` 没有移植）。`<attachment obj_id=…>` 可用；`<attachment path=…>` 会转换失败，改发 `convert_failed` 提示。
13. **联系人 / 群名**：`MsgBridgeCtx.contact_name / conversation_name` 没有填（旧 `contact.rs` 没有移植）；说话人名字只来自记录的 `from_name`。
14. **投递最终结果**（SENT / DEAD）没有读 `DeliveryRecord`；WebUI 只显示 `outbox` 条目里 `post_send` 返回的 `msg_id` 与 `delivery_id`。

### 10.3 验收

记录的是实际运行的命令与结果（在 `src/` 下）。

| 项 | 命令 / 场景 | 结果 |
|---|---|---|
| M0 | `cargo check --workspace`（不含 `opendan_legacy`） | 通过 |
| 库 | `cargo test -p libopendan -p opendan -p agent_tool_cli_dev -- --test-threads=1` | 全部通过，0 失败：libopendan 单元 18 + 集成 139（另 1 例真实 kmsg 为 ignored，未跑）；opendan 单元 6 + `tests/loader.rs` 7；agent_tool_cli_dev 38 |
| M1 进程级 | `opendan --dev` + `xagent new … --no-run`（driver = OpenDAN）+ OpenAI 兼容 mock（每次推理 3 秒） | Session 被托管；`xagent status <sid> --state-url http://127.0.0.1:<port>/kapi/opendan` 返回 `running`；推理中 `kill -9` 后重启，未结束的 Session 自动恢复并 `finished / succeeded`；`xagent list --state-url …` 一致；SIGTERM 退出码 0 |
| M1 / E4 | `tests/loader.rs::agent_state_over_krpc_matches_the_files`（真 HTTP kRPC） | 文件实现与 kRPC 客户端对同一组读操作结果相同；带署名的写到达；错误种类保留；驱动者写入被拒 |
| M3 | `tests/loader.rs`（假 msg-center + 脚本化 LLM） | inbox 消息创建绑定的 UI Session 并沿原会话回复（私聊、群聊、回声丢弃、空消息确认不投递、`/stop` 后下一代）；回复已提交、msg-center 不可达时退出，重启后原样发送一次（同键、同 ObjId、不重新推理）；Session 队列满时上游记录保持未读，腾出后全部送达；子 work session 由 Supervisor 托管，结果经父 UI Session 转述；Jarvis 包能加载、冻结 `chat_route` + `task_route` 并回复 |

**当时没有验证的**（前两项已在 §10.6 的 DV 验收里补上）：

- 真实 zone：稳态 token（启动 30 秒后）下 msg-center 的 `get_next` / `update_record_state` / `post_send` 对 Loader 的 app 身份是否放行（§9.2）；kmsg / kevent 真服务；AICC 推理；tunnel 来信；容器形态。需要 `buckyos-build` + 安装后在 DV 环境跑。
- “让它做一个需要 work session 的任务”的真实链路：`call_behavior(task_route)` → `agent-session create-worksession` → `plan` / `do`。测试里子 Session 是直接用 `create_sub_session` 创建的；迁移后的四个 behavior 只验证了能解析、冻结、进 system，没有用真实模型跑过。
- “回复发送后 kill”：只覆盖了“已提交、未发送”这一个窗口；“已发送、未标记”靠 msg-center 的幂等键，假 msg-center 按键去重，真服务未测。

### 10.4 WebUI

`src/frame/opendan/web`：Vite + React + TypeScript + Tailwind（依赖是 desktop 的子集）。`OpenDanDataModel` 接口（`src/model/datamodel.ts`）有 mock 与 kRPC 两个实现，`?data=mock | krpc` 切换，`VITE_OPENDAN_PROXY=http://127.0.0.1:<port> pnpm dev` 代理到后端。四个页面：Sessions（列表 / 父子 / 活动视图）、Session 详情（Turn、等待、process_stack、pending_events、outbox、worklog、report、绑定、冻结的 behavior、托管状态，三个操作各有确认）、Agent State（感知、产物、behavior、身份、hints）、Loader（模块、托管、inbox 桥、错误）。构建：`bucky_project.yaml` 新增 `opendan_web` 模块，产物到 `rootfs/bin/opendan/web/`，由 opendan 在 `/` 提供。

验证（在 `src/frame/opendan/web`）：`pnpm build`、`pnpm lint` 通过；`pnpm exec playwright test`（mock）7 例通过；`OPENDAN_URL=… pnpm exec playwright test -c playwright.real.config.ts`（真实 `opendan --dev`）1 例通过，并在真实后端上对一个 ui Session 手工走过 Post 与 Stop。

三个操作只对有输入队列的 Session 显示（它们都是投到输入总线的记录）；无队列的 work session 的产物验收走 `artifacts.decide`，WebUI 里还没有这个入口。

zone 形态下的 token 路径已在 DV 环境验证（§10.6）：页面先 `GET /kapi/opendan` 取托管它的 app id，用它初始化 websdk，再经 SSO 刷新拿 token。

### 10.6 DV 环境验收（2026-10-04）

环境：本机 DV（`test.buckyos.io`，`/opt/buckyos`）。清空 `data` / `logs` / `storage` / `local` 与 `buckyos-instance-*` 卷后全新安装（`start.py --all` 的两步）。Jarvis 由 node-daemon 用 worker 镜像在容器里拉起，镜像里的 opendan 是发布版本，所以验收用的是在 `paios/aios:latest-amd64` 上覆盖 `bin/opendan/`（opendan、xagent、agent_tool、web）的本地镜像 `local/aios-opendan-dv`，经 `/opt/buckyos/etc/devenv.json` 的 `{"aios": "local/aios-opendan-dv"}` 指过去。

| 项 | 做法 | 结果 |
|---|---|---|
| 启动 | node-daemon 拉起容器 | AgentRoot 从包同步（26 个文件），HTTP 在调度器分配的端口，UI 模块开始扫描 inbox |
| §9.2 msg-center 权限 | Loader 的 app 身份调 `get_next` / `update_record_state` / `post_send` | 都放行，按现状使用，不需要补授权。回复落在用户的 `REQUEST_BOX`（Jarvis 不是 devtest 的联系人时 msg-center 的既有行为） |
| 对话 | `test/test_opendan/test_agent_loader.ts`：devtest 登录 → `post_send` 给 Jarvis → 等自己时间线里的回复 | 真实模型（AICC `llm.chat`）6.6 秒回复；第二条消息进同一个 UI Session，能复述上一条 |
| 派出 work session | “请派一个 work session 完成…” | `chat_route` → `call_behavior(task_route)` → `agent-session create-worksession` → Supervisor 托管 work session → `plan` 执行并结束 → 结束事件进父 Session → 父转述给用户。先回“已派出”（约 11 秒），再回结果（约 20–30 秒）；连续 3 次都走完，产物存在 |
| 重启恢复 | 推理进行中 `docker kill buckyos-app-jarvis` | node-daemon 约 6 秒后重建容器，Supervisor 从登记表接管同一个 run，用户收到一条完整回复，`outbox` 该条 `attempts = 1` |
| `/stop` | 发 `/stop` | UI Session 以 `stopped` 结束、不回消息；下一条消息进新一代 Session（冻结到包里最新的 behavior） |
| kRPC 权限 | 不带 token / 带 devtest 的 control-panel token | 前者拒绝，后者放行 |
| WebUI | `playwright.zone.config.ts`：经网关 `https://jarvis.test.buckyos.io` → SSO 登录 → Sessions / Session 详情 / Loader | 通过，没有权限错误 |

验收中发现并修掉的问题：

1. **Jarvis 包**：`[llm_context]` 没给 `max_tokens`，Claude 路由直接拒绝请求；`chat_route` 没把 `call_behavior` 列进工具；提示词里残留旧格式的 `<<process_rules>>` 标记；`task_route` 会自己把活干掉而不创建 work session（提示词改为“调度员，只许运行 `agent-session create-worksession`”，`chat_route` 传给它的 `task` 加上“Route this request…”前缀）；转述前先用 `agent-session read-session <sid> --report` 读完整报告。
2. **libopendan**（记在 [lib-opendan-outbound-todo.md](lib-opendan-outbound-todo.md) §6）：`tool_whitelist` 生成的工具条目带 `kind`，被 xllm 配置解析拒绝，现在生成 `{name}`，并支持 `group:<内置工具组>`；Turn 失败时“这一轮是否在回答消息”按输入 id 前缀判断，永远为假，失败提示从未发出，现在 `open_turn.has_msg` 记录；`wait_user_msg = finish_failed` 的 Session（work）遇到不可重试错误或预算耗尽时停在 `ready`，父 Session 永远等不到，现在同时结束为 failed，`one_line_status` 带错误信息。
3. **agent_tool / Loader**：native runtime 与 lease 的宿主身份取 `/etc/hostname`，容器每次重建都会变，重启后所有 Session `bind_failed`。新增 `AGENT_TOOL_HOST_ID`，Loader 在 zone 内设为 `<device did>/<app instance id>`。
4. **WebUI**：websdk 校验 token 的 appid，页面写死的 `opendan` 与实际的 `jarvis.buckyos.bns.did` 不符，拿不到 token。新增 `GET /kapi/opendan` 返回 app id。

还没解决的：

1. **worker 镜像**：发布的 `paios/aios` 里还是旧 opendan。`build_aios` 已经包含 `xagent` 与 `web/`，本地可以用 `--local-test` 模拟发布后的效果（§10.7）；正式镜像要等发布流程跑一次。镜像入口用的是 `--appid`，新参数解析兼容。
2. **`llm.plan` 在 DV 的 AICC 里没有可用候选**（唯一候选是被禁用的 experimental 模型，`no_candidate_model`）。正常应该有（claude / gpt 的旗舰模型），是 AICC 的配置问题，单独定位；包里 `plan` / `do` 保持 `llm.plan` 不动。验收时在已部署的 AgentRoot 里把这两个文件改成了 `llm.chat`（本地修改会被包同步保留，定位清楚后删掉这两处本地修改即可）。
3. **behavior 循环里的原生工具调用不进 worklog**：`plan`（`mode = "behavior"`）在一个 Step 里直接调了 `shell`，worklog 只有最后的 `step` 与 `outcome`，命令只在 `runs/<id>/exec/` 与快照里。WebUI 因此看不到 work session 做了什么。属于 libopendan 的 flush，单独立项。
4. **work session 的工作目录是父 UI Session 的目录**（`--workspace inherit` 的缺省）。产物因此落在 UI Session 目录下。是否让 `task_route` 用 `--workspace new` 或指向 Agent 的 workspace，随 workspace 设计定。
5. `bind_failed` 的 Session 只在 `loader.status` 的托管状态里可见，用户侧没有任何提示。
6. `task_route` 是否创建 work session 仍然靠提示词约束（改后 3/3，改前 1/2）；要稳就得给它一个只能创建 work session 的工具，而不是 `shell`。
7. tunnel 来信、群聊（包里没启用 `msg.group`）、附件、“已发送未标记”窗口下真 msg-center 的幂等，没有测。

### 10.7 调试流程（2026-10-04）

1. **宿主机调试**：`src/debug_jarvis.sh` 改为新 Loader 的入口——debug 构建、包与 WebUI 读源码目录、停掉 app 容器并占住服务端口、`--trust-loopback` 让本机直接开 WebUI。`service_debug.tsx` 的签名改成 node-daemon 现在的格式（给 system-config 的是 `aud = system-config-bootstrap` 的 bootstrap assertion，原来的格式已被拒绝），新增 `--opendan-bin` 与 `--` 之后透传给 opendan 的参数。Loader 在服务端口被占时直接退出，避免与容器里的实例同时托管一个 Agent。
2. **本地测试镜像**：`build_aios --local-test`（本机架构、`local/aios-test`、复用 cargo 产物、不推送，并写 `$BUCKYOS_ROOT/etc/devenv.json`）。`build_aios` 同时开始把 WebUI（`frame/opendan/web` 的 `pnpm build`）打进镜像的 `bin/opendan/web/`，CI 的 `build-aios.yml` 加了安装 pnpm 的一步（CI 没有实跑过）。

用法在 [opendan README](../src/frame/opendan/README.md) 的“调试流程”。

### 10.5 剩下的事

1. §10.6 “还没解决的”各项。
2. M5：删除 `src/frame/opendan_legacy`；`doc/opendan/` 里与新架构冲突的文档重写或删除；新增 Loader 设计文档与 Agent 包开发指南取代本文。
3. M4：self-check / self-improve 模块；`self_improve_signals.toml` 仍引用已删除的 `read_session_history` / `commit_session_history_improved`，随 M4 改写。
4. `agent.delegate`：把 `opendan_legacy/src/dispatch_adapter.rs` 一组搬过来，启动任务换成创建 work session（M5 删除 legacy 之前做，或先把这组文件留下）。
5. §10.2 第 8、9、12、13、14 项；Agent State 服务的 zone 服务发现；`serve` / `run_session` 改用 `Supervisor`。
6. `groupchat_route`、`self_check`、`self_improve_*`、`try_create_new_skill` 等 behavior 仍是旧结构（目录 `list` 时跳过并告警）；`[[loader.ui]] on = "msg.group"` 规则在 Jarvis 包里没有启用。

