# MessageHub Agent Chat：可编辑回复与 task 状态展示 TODO

2026-10-05 体验修正：Agent 消息打开后的主视图改为此 turn 的 worklog 时间线，TaskMgr 作为定位和状态入口；消息元数据与 task tree 收到「消息详情」页签。见 §8，此决定替代此前“打开详情主要展示消息元数据和 task 状态”的体验。

日期：2026-10-04。状态：设计已评审，§0 的决定已冻结；P0-a、P0-b 与 P1（除动态组授权）已实施，本地测试通过，DV 环境的原生私聊验收通过（§7.5）；原生群聊与 Telegram 尚未验收。实施记录见 §7。

基线：当前工作区代码（HEAD `3b05fa61`，包含已有的未提交修改）。以下“现状”以代码为准；旧版 OpenDAN TODO 中的完成记录不能直接视为当前 Loader 已具备的能力。

## 0. 评审决定（2026-10-04）

| # | 决定 | 影响 |
|---|---|---|
| D1 | **Agent Session 收到 input 开始处理，总是开一个 TaskMgr task。** 一个 Turn 对应一个 root task，几秒就回完的快速 Turn 也建——TaskMgr 同时是审计中心。同一请求内的后续回复原理上是该 root 的 SubTask，但 runtime 无法识别续接关系时不建立父子关系 | §3.1、§3.3、§3.4 |
| D2 | **OpenDAN 接入 TaskMgr 是 P0，先于消息侧工作。** 现状是完全缺失。BuckyOS 鼓励为长期工作建 task，并建立正确的父子关系：work session 是 task，它开的 sub session 是其子 task；逻辑上 stop 父 task 即 stop 整棵子树。实际 stop 的主路径仍是 Session 控制协议，但 task tree 必须在逻辑上成立 | §3.3、§6 |
| D3 | **占位消息延迟触发。** task 在 Turn 打开时即创建（D1），但“收到，开始处理…”只在 Turn 超过阈值仍未结束或首次工具调用时才发；此前完成的 Turn 直接发普通回复 | §3.2 |
| D4 | **只有 MessageHub UI 理解 `agent_task`。** MsgObject 只多携带一个 task_id；msg-center、tunnel、group host 不解析、不校验它。UI 不认识或读不到 task 时不展示任务区，正文照常显示 | §3.1、§4 |
| D5 | 评审中提出的简化项全部采纳，见各节标注“（简化）”处 | §2、§4、§5 |
| D6 | **task 由 Agent 自建并 grant 给 Agent 的 owner。** 其他人（群成员、外部联系人）首版读不到，按 D4 不展示任务区；后续通过动态组授权给其他人 | §3.3、§4.2 |
| D7 | **Agent Turn 的 task 正常进入 Task Center**，不做默认过滤；Task Center 自带查询，且面向高级用户 | §6 |
| D8 | **work session 的 task 就是它首个 Turn 的 task**，不为 Session 另建一层；UI Session 这类长期 Session 不建 Session 级 task | §3.3 |

## 1. 评审结论

建议推进。用一条稳定的回复承载“已接受 → 正在工作 → 最终结果”，很适合 Agent Chat；task_id 让消息成为任务的观察入口，也避免中间进度不断刷屏。

三条原则：

1. **用户看到原消息被编辑，存储层仍追加不可变消息。** 复用 `relates_to = { rel: "edit", target: <原消息 ObjId> }`，不覆盖 Named Object，不要求最终回复与占位消息具有相同 ObjId。稳定的是原消息锚点和 task_id。
2. **正文与运行状态分开。** 正文起初为“收到，开始处理…”，终结时一次性替换；工作中的两行状态、子任务树与详情由 task 查询驱动，不为每次状态变化发送一条 edit。
3. **task 跟随 Turn，不绑定整个长期聊天 Session。** 每个 Turn 一个 task（D1）；UI Session 的历史回复不随下一轮工作变化。终结后的 task_id 不改变。

用户要求的完整体验：

- 原生 MessageHub：发送携带 task 引用的占位消息，默认展示两行状态，展开显示子任务（子 Session）工作状态，点击消息进入通用消息详情；最终在同一气泡显示完整回复，仍能查看该 task。
- 经 msg-tunnel 回复：发送占位前检查实际出站目标的编辑能力。支持才启用；不支持或能力未知时不发占位，结束后正常发送最终回复。
- 失败、取消是明确状态，不能让占位消息永久停留在“处理中”。Agent 需要用户回答时，问题就是这条气泡的最终正文（§3.4）。

## 2. 当前实现与缺口

| 范围 | 已有能力（代码入口） | 本需求仍需补齐 |
|---|---|---|
| 消息编辑协议 | [msgobj.ts](../src/frame/desktop/src/app/messagehub/protocol/msgobj.ts) 有 `relates_to/edit`；[msg_center.rs](../src/frame/msg_center/src/msg_center.rs) 按 `gen_obj_id()` 保存对象，`post_send` 有幂等支持 | 将 Agent 占位与最终回复接到已有关系协议 |
| 编辑校验 | [group_service.rs](../src/frame/msg_center/src/group_service.rs) 的群消息校验检查原作者、Session、目标是否已撤回、编辑时间窗（默认 `None` 不限），且不允许编辑关系消息。`post_send` 对非群消息**没有任何关系校验** | 私聊 edit 的服务端校验降到 P1：前端折叠已校验作者一致，且发送侧校验防不了跨 Zone 对端，有效的是接收侧 |
| 气泡投影 | [relations.ts](../src/frame/desktop/src/app/messagehub/conversation/history/relations.ts) 隐藏 edit 记录、折叠到原消息；目前只保留编辑后的文本 | 替换完整 `MsgContent`（format、refs、machine 等）；[renderers.tsx](../src/frame/desktop/src/app/messagehub/conversation/history/renderers.tsx) 仍从原对象读取格式和附件。（简化）一个占位只有一个作者、一条最终 edit，P0 不做接收侧可靠次序或服务端有效投影 |
| UI 编辑与详情 | [ConversationView.tsx](../src/frame/desktop/src/app/messagehub/ConversationView.tsx) 的 edit 操作只向群聊开放；[MessageHubView.tsx](../src/frame/desktop/src/app/messagehub/MessageHubView.tsx) 的详情对象是 entity/session；[launch.ts](../src/frame/desktop/src/app/messagehub/launch.ts) 可定位 entity/session | 补普通消息详情和消息级定位，再挂 task 信息 |
| Agent 回复 | [runner/outbound.rs](../src/frame/lib_opendan/src/runner/outbound.rs) 在 Turn 关闭的提交中落盘 outbox，随后发送并重试；[opendan/ui.rs](../src/frame/opendan/src/ui.rs) 的 `MsgCenterSink` 调用 `post_send`；文案来自 Agent 包 `i18n [outbound]`，没有文案就不发 | 没有占位及其 task 绑定；Stopped Turn 当前不发任何消息，有占位时需补“已停止”收尾 |
| Session / Turn | [protocol/state.rs](../src/frame/lib_opendan/src/protocol/state.rs) 保存 open_turn、reply、outbox；Turn 进行中到达的输入并入当前 Turn（[runner/live.rs](../src/frame/lib_opendan/src/runner/live.rs) `commit_input_batch`）；Agent 向用户提问即 `TurnStatus::Completed`，回答开新 Turn | Turn 与 TaskMgr task 的持久关联；“提问后的回答 Turn 属于同一请求”目前无法识别，不建父子关系（§3.4） |
| TaskMgr 接入 | **OpenDAN / libopendan 目前没有任何 TaskMgr 客户端调用。** [bridge/task.rs](../src/frame/lib_opendan/src/bridge/task.rs) 只有“外部 task 状态 → AgentEvent”的映射；[xAgent.md](../doc/opendan/xAgent.md) 把 TaskMgr 接入列为后移项 | P0：Turn / Session ↔ task 的创建、状态上报、父子关系（§3.3） |
| 子 Session | [protocol/config.rs](../src/frame/lib_opendan/src/protocol/config.rs) 有 `origin.parent_session`、`origin.created_by_call`；[protocol/agent_state.rs](../src/frame/lib_opendan/src/protocol/agent_state.rs) 有 `one_line_status`、`report_brief`、`waiting_for`、rev；[state/krpc.rs](../src/frame/lib_opendan/src/state/krpc.rs) 提供 `sessions.children_of` | [runner/children.rs](../src/frame/lib_opendan/src/runner/children.rs) 的 `session:<sid>` 是 TaskResolver 的引用，不是 TaskMgr task_id；子 Session 需要成为调用方 Turn task 的子 task |
| TaskMgr 服务 | [task_mgr.rs](../src/kernel/buckyos-api/src/task_mgr.rs) 有稳定 task_id、parent/root、phase/outcome/wait_reason、message/progress、revision、树查询、持久事件、ACL grant、`CreateDelegatedTaskReq`；[Desktop task_mgr.ts](../src/frame/desktop/src/api/task_mgr.ts) 已有 Task Center 客户端与状态投影 | 新增 Agent Turn 的 task schema；MessageHub 按 task 加载与订阅，不能每个气泡重复拉整个任务列表 |
| Tunnel | [msg_tunnel.rs](../src/frame/msg_center/src/msg_tunnel.rs) 的 `DeliveryExecutor` 只声明 ingress/egress；[tg_tunnel.rs](../src/frame/msg_center/src/tg_tunnel.rs) **不识别 `relates_to`，edit 会被当成新消息发出**；其 status_line / turn_nonce 链路在 Loader 侧没有任何写入方，是死链路 | 能力查询完成前，tunnel 目标一律不发占位（P0 门禁）；P1 把通用 edit 落到平台编辑 API |

补充：`DeliveryRecord` 已持久化 `external_msg_id`，`delivery_id` 由 (msg_id, target, transport) 确定性生成。（简化）用原消息 ObjId 即可查到外部消息 ID，Telegram 不需要新建映射表。

## 3. 数据与生命周期约定

### 3.1 三个 ID 各司其职

| 标识 | 含义 | 生命周期 |
|---|---|---|
| 原始占位消息 ObjId | 时间线位置、引用和消息详情的稳定锚点 | 最终 edit 始终指向它，不指向上一次 edit |
| task_id | 此 Turn 的 TaskMgr task | 运行、等待、终结期间不变；重试投递不新建 task |
| session_id + turn | Agent 的执行位置 | 用于恢复；记录在 task 的 input / origin_ref 里，不进消息 |

MsgObject 只携带 task_id（D4）：

```json
{ "agent_task": { "task_id": "<TaskMgr task id>" } }
```

- Rust 的 `meta` 会平铺到消息顶层，因此不要再包一层 `meta`。
- 只有 MessageHub UI 解释这个字段。msg-center、tunnel、group host 当它是普通扩展字段透传，不解析、不校验。
- UI 只信**原锚点消息**上的 `agent_task`，忽略 edit 携带的，因此不需要服务端“禁止借 edit 更换 task”的校验。
- UI 不认识该字段、task 不存在或无权读取时，不展示任务区，正文照常显示。
- 不从正文解析 ID；task_id 可在详情中复制，普通气泡不展示内部编号。
- Agent DID、Session、Turn 等信息从 task 本身读取，不在消息里重复。

### 3.2 Turn 进入与结束

```text
输入批次提交、打开 Turn（has_msg）
→ 幂等创建该 Turn 的 task，持久化绑定（D1）
→ 推进工作，上报 task 状态
→ 延迟触发点（超过阈值仍未结束，或首次工具调用）：
  ├─ 目标可编辑：占位进入 outbox → post_send → 保存原消息 ObjId
  └─ 不可编辑、未知、或 Agent 包没有占位文案：不发占位
→ Turn 结束：
  ├─ 已发占位：排入最终 edit
  └─ 未发占位：排入普通最终消息（同样携带 agent_task）
→ 出站失败恢复 / 重试，直到终结内容可见或有明确投递失败状态
```

- **task 创建时点**：Turn 打开时（`commit_input_batch` 的 `opens_turn`），快速 Turn 也建。只为含消息的 Turn 发占位；由事件打开、之后才并入消息的 Turn 不补发占位。
- **占位延迟**（D3）：阈值暂定 3 秒，可配置。延迟期内结束的 Turn 直接发普通回复，不存在“排队”态——Turn 进行中到达的消息并入当前 Turn，不会产生新的占位。
- **开关**：占位文案放 Agent 包 `i18n [outbound]`（如 `accepted`、`stopped`）。沿用“没有文案就不发”的现有规则，未配置的 Agent 不启用占位。
- **接入位置**：libopendan 不直接依赖 TaskMgr。仿照 `OutboundSink` 增加宿主钩子（Turn 打开 / 状态变化 / Turn 关闭），由 OpenDAN 用 TaskMgr 客户端实现；xagent 等没有 TaskMgr 的宿主不提供钩子即不建 task。
- **outbox**：增加条目用途（占位、最终 edit、普通回复）；占位与最终回复用不同幂等键。同一项重试重用完整 MsgObject，包括 nonce 和 created_at_ms。
- **绑定保留期**：Turn ↔ task ↔ 占位 ObjId 的绑定独立于 outbox “保留最近 16 条结束项”的清理策略。
- **快速完成**：尚未发送的占位可在持久状态中取消，直接发最终回复；已发送则必须编辑。发送结果不确定时先按原幂等键恢复，防止多出一条占位。
- **task 终态与消息投递是两件事**：task 已成功但 edit 待重试时，显示“处理完成，回复同步中”。
- **收尾**：失败、预算耗尽、停止都要收尾已存在的占位（Stopped 现在不发消息，需新增“已停止”文案的 edit）；未发占位时保留现有回复路径。task 创建失败时不发占位，最终回复仍应送达。
- **无正文结束**：Turn 已结束但没有可发送正文时，用结束摘要收尾已有占位。
- **未读**：占位与 edit 各产生一条 INBOX 记录。最终结果的到达提示由 edit 记录天然提供；占位那条随 edit 一并标为已读，一个气泡只计一次未读。

### 3.3 task tree（D2，P0）

TaskMgr 里的树与 Agent 的执行结构一一对应：

```text
Turn task（UI Session 的一个 Turn，root）
├─ 子 Session task（该 Turn 里创建的 work / explorer session）
│   └─ 子 Session 再开的子 Session task …
└─ 长工具 / 后台任务（已有 task_id 的，挂到所属 Turn task 下）
```

- 子 Session 开始处理 input 时同样开 task，`parent_id` 是创建它的那个 Turn 的 task。父 task id 随 `origin`（与 `parent_session`、`created_by_call` 并列）传给子 Session，不靠事后按 Session 反查。
- 一个 Session 处理多个 Turn 时每个 Turn 各有 task。work session 的 task 就是它首个 Turn 的 task，不另建 Session 级 task（D8）；它后续的 Turn（例如父 Session 追加指令）各自建 task，挂在同一个父节点下。
- **ACL**（D6）：task 由 Agent 自建，创建时 grant 给 Agent 的 owner（读取与控制，覆盖整棵子树）。P0-a 第一步先确认 TaskMgr 默认 ACL 下 owner 实际能读到什么，再决定 grant 的具体范围。动态组授权后移。
- **状态写入方向单一**：Session runner 是唯一写入方，把 run_state、`one_line_status`、`waiting_for`、结果摘要映射为 task 的 phase、message、wait_reason、outcome。TaskMgr 不反向推导 Session 状态。
- **stop**：主路径仍是 Session 控制协议（`control(stop)` 及其向子 Session 的传播）。TaskMgr 对 root task 的 cancel 请求经 bridge 翻译为对应 Session 的 stop；task tree 保证“stop 父即 stop 子树”在逻辑上成立，子 task 的终态仍由各自 Session 上报。
- **展示**：MessageHub 的展开树直接用 `get_task_tree/get_subtasks`，不再需要 OpenDAN 另提供“请求范围的子 Session 视图”。Session 级的细节（worklog、工具时间线）从 task 的 origin_ref 跳到 OpenDAN WebUI，首版不在 MessageHub 内展示。
- 因 Turn 结束仍继续运行的后台子工作，其 task 保持在原 Turn task 下并标明状态。

### 3.4 同一请求的续接（D1）

原理上，Agent 在 Turn 中向用户提问、用户回答后继续处理，回答那个 Turn 的 task 应是原 root 的 SubTask。

但代码里提问就是 `TurnStatus::Completed`，与普通最终回复没有区别，runtime 无法机械判断“下一个 Turn 是续接还是新请求”。决定：**无法识别就不建立父子关系**。

- 每个 Turn 结束即其 task 终结；用户的回答打开新 Turn、新 root task、新回复气泡。
- 提问就是该气泡的最终正文，task 以成功终结；UI 不显示“等待用户”这种 task 状态。root task 的 `Waiting` 只用于等子任务或外部依赖。
- 不为此新增 Agent 侧信号，也不做猜测式关联。以后若出现可机械识别的续接（例如结构化的提问/应答关系），再用 `parent_id` 挂接，协议不需要改。

## 4. 展示与刷新

### 4.1 默认两行

- 第 1 行：任务阶段 + 当前活动，例如“正在处理 · 检查 MessageHub 消息投影”。
- 第 2 行：子任务统计、等待原因或最近更新时间，例如“2 个子任务运行中 · 更新于 3 秒前”。没有可计量的总量时不显示推测的百分比。
- `Running` 显示工作；`Waiting` 区分等子任务和外部依赖；`Paused` 显示暂停；`Terminal` 根据 outcome 显示成功、失败或取消。`pending_control` 可用于显示停止请求处理中，不能提前宣称已取消。
- 最终正文替换后保留折叠的任务摘要与详情入口。没有子任务、已成功终结的 task（普通快速回复）默认不显示任务区，只在详情里可见。
- 查询超时、任务不可访问、任务已清理时不展示任务区或显示具体的不可用状态，保留可读正文，不持续伪装“运行中”。

### 4.2 展开与消息详情

- 气泡展开：本 Turn 的 task tree、各节点状态、简短活动说明、等待原因；支持逐层或分页加载。
- 点击消息进入通用消息详情，也提供明确的“详情”操作和键盘入口；正文中的链接、附件、选择文本与展开按钮继续各自工作。
- 通用详情展示原消息、当前有效内容、编辑记录和投递状态；存在 `agent_task` 时追加实时任务区，展示摘要、task tree、可访问事件、结果和产物引用。
- 消息级深链用原消息锚点，补齐路由与 launch schema；未加载该条历史时能够定位，刷新页面、重新进入详情仍得到相同 task。
- 消息可读不代表任务可读。读取 task 沿用真实登录者在 TaskMgr 的权限：首版只有 Agent 的 owner 能读（D6），其他人读不到就不展示（D4）；观察模式不借 owner 身份读取。

### 4.3 实时性

- 原生气泡按可见 task 集合共享缓存和订阅；按 task_id/revision 去重。展开或进入详情后才取树和事件，折叠/离开时释放不需要的订阅。
- 复用 `/task_mgr/<id>` 与 `/task_mgr/tree/<root_id>` 事件路径，把事件作为“需要重读”的提示；保留有界轮询、断线重连补读与旧 revision 丢弃。
- 复用 msg-center `box_changed` 刷新占位及 edit 投影；TaskMgr 事件只刷新任务区。停止订阅前确认终态快照与最终消息投递各自已收敛。
- 列表摘要、搜索/引用预览读取有效消息内容；进度不产生新通知。

## 5. msg-tunnel 能力与降级

P0 门禁：能力查询落地前，凡经 tunnel 出站的目标一律视为不可编辑，不发占位。

不要只增加一个静态 `supports_edit = true`。至少区分“目标支持编辑自己的消息”和“该条消息此刻可编辑”；还要表达文本/附件限制、编辑时间窗及最终执行错误。

- [x] 由 msg-center 按确定的出站 route/target 提供能力查询；`MailService` / `OutboundSink` 暴露给 Turn 接入路径。Agent 不从平台名称、入站 transport 或最近活跃聊天推断出站能力。
- [x] 原生 MessageHub 复用同一能力语义。（简化）群配置了 `edit_window_ms`（非 `None`）即视为不可编辑、跳过占位，不做服务端豁免。外部能力未知按不支持处理。
- [x] 同一 Turn 多个目标分别决策；一个目标不可编辑不影响另一个可编辑目标。
- [x] Telegram 把通用 edit 关系落到平台编辑 API：用原消息 ObjId 算出 delivery_id，读已持久化的 `external_msg_id`。禁止把 edit 正文当成新消息发出。
- [x] （简化）删除 Telegram 既有的 status_line / turn_nonce / `TgUiSessionTracker` 状态消息链路，不做接入，避免两套占位。
- [x] 处理编辑时间窗过期、原消息被删除、格式或附件无法原地替换：外部平台按能力更新占位为终态摘要，并以必要的独立消息交付完整附件/结果。确实无法编辑时发送一次普通最终回复作为兜底。
- [x] 暂时失败按原键重试，永久失败才进入持久化的兜底发送状态；处理“平台编辑成功但 ACK 丢失”，避免重试又发送一份最终答案。

不支持编辑的 tunnel 不需要支持 MessageHub 的两行状态、树或详情页；这是原生 UI 的增强能力。能力不足不应阻断最终答复。

## 6. 实施顺序

### P0-a：OpenDAN 接入 TaskMgr（D2，先做）

- [x] 确认 TaskMgr 默认 ACL 下 Agent 自建 task 对 owner 的可见性，定 grant 范围（D6）。
- [x] 定 Agent Turn 的 task schema（input 含 agent_did、session_id、turn、输入消息引用）与 phase / wait_reason / outcome 映射表；task 正常出现在 Task Center（D7），名称与摘要要让人在列表里看得懂。
- [x] libopendan 增加宿主钩子与 `origin` 里的父 task id；OpenDAN 用 TaskMgr 客户端实现创建（幂等键 = sid + turn）、状态上报、终结。
- [x] 子 Session 的 task 挂到创建它的 Turn task 下；已有 task_id 的长工具挂到所属 Turn task 下。
- [x] TaskMgr cancel → Session stop 的 bridge；重启恢复时按幂等键找回 task，不重复创建。
- [x] 验证：UI Session 一个 Turn 开两个子 Session，`get_task_tree` 得到正确的树与终态。

### P0-b：原生闭环

- [x] 冻结 `agent_task` 字段（§3.1），同步 TS 类型与协议文档。
- [x] libopendan / OpenDAN：延迟占位、outbox 条目用途、最终 edit、失败/停止收尾与恢复；tunnel 目标门禁。
- [x] MessageHub：edit 折叠替换完整 `MsgContent`；按 task 查询与默认两行状态；占位记录随 edit 标已读。
- [x] 跑通一条消息从占位到最终结果，刷新后 task_id 不变；快速回复不出现占位。

### P1：完整三层体验与通道适配

- [x] MessageHub 展开树、通用消息详情和消息级深链。
- [x] 事件加速、轮询兜底、终态缓存、列表摘要同步。
- [x] tunnel 能力查询与 Telegram 通用 edit；删除旧 status_line 链路。
- [x] 私聊 edit 的接收侧校验；需要时再做服务端有效投影。
- [ ] 动态组授权：让 owner 之外的人（群成员等）读到 task。

### 联动与验收

- [x] 协议改动同步 Rust API、Desktop 类型、实际使用的 websdk 客户端、Session 持久 schema，以及 `doc/message_hub/`、`doc/opendan/protocol/`、`doc/opendan/xAgent.md`、MessageHub `UI_DATAMODEL.md`。
- [x] 与 [lib-opendan-outbound-todo.md](./lib-opendan-outbound-todo.md)、[opendan-agent-loader-refactor-todo.md](./opendan-agent-loader-refactor-todo.md) 对齐接口与恢复语义。
- [x] Runtime 测试：快速完成（有 task、无占位）；阈值后发占位；Turn 中并入的输入；失败/预算耗尽/停止；子 Session task 的父子关系；task 创建、占位发送、响应丢失、task 终结、最终 edit 各阶段重启恢复；没有 TaskMgr 钩子的宿主行为不变。
- [x] 投影测试：完整 Markdown/附件替换、原消息与 edit 乱序/重放、原消息已撤回、非原作者的 edit 被忽略、edit 上的 `agent_task` 被忽略。
- [x] Tunnel 测试：门禁期间完全没有占位；P1 后覆盖支持/不支持/未知能力、编辑窗口过期、带附件、编辑成功但 ACK 丢失、兜底仅发一次、重启后不编辑错消息。
- [x] UI 测试：原气泡位置及锚点稳定；两行/展开树/详情三层贯通；断线恢复；权限不足与任务已清理时不展示任务区；观察模式；最终答案不会被旧状态覆盖。
- [ ] 真实 Zone 验收至少覆盖原生私聊、原生群聊和 Telegram。

完成标准：每个含消息的 Turn 在 TaskMgr 有一个 task，子 Session 的 task 挂在正确的父节点下；正常可编辑路径从接受到终结只有一个主要回复气泡，并可由不变的 task_id 追溯本 Turn 的工作；快速回复和不支持编辑的目标只收到正常最终答复；重启、重试、多个 Turn 和编辑失败都不会造成串任务或遗失最终结果。

## 7. 实施记录（2026-10-04）

未提交 git。§2“现状”描述的是实施前的代码。

### 7.1 落点

| 范围 | 文件 | 内容 |
|---|---|---|
| task schema | `src/kernel/buckyos-api/src/task_mgr.rs` | 内置 `opendan.agent_turn/v1`（只允许 App 执行者）；task-manager 重启后种入 |
| Turn task 钩子 | `src/frame/lib_opendan/src/runner/turn_task.rs` | `TurnTaskSink`（open / update / close）、`ensure_turn_task`、`sync_turn_task`（去重上报）、`flush_turn_tasks`（终态重试） |
| 持久绑定 | `lib_opendan/src/protocol/{state,config}.rs`、`api.rs` | `state.turn_tasks`、`OutboxEntry.purpose`、`origin.parent_task`（建子 Session 时取父 Session 打开的 Turn 的 task） |
| 占位与最终 edit | `lib_opendan/src/runner/outbound.rs` | `OutboundSink::placeholder`、`maybe_placeholder`（阈值定时器 / 首次工具调用 / drive 入口）、`queue_reply` 的 edit 与撤销分支、`agent_task` 写入锚点消息 |
| OpenDAN | `src/frame/opendan/src/tasks.rs`、`ui.rs`、`loader.rs`、`config.rs` | `ZoneTaskService`、`TurnTasks`、`CancelBridge`；`MsgCenterSink::placeholder`（文案 + 能力查询）；`[loader] placeholder_delay_ms`；模块状态 `task_mgr` |
| Agent 包 | `src/apps/jarvis_runtime/agent/i18n/{en,zh}.toml` | `[outbound]` 的 `accepted` / `stopped` / `finished` |
| msg-center | `src/frame/msg_center/src/{msg_center,msg_tunnel,message_hub,tg_tunnel,main}.rs`、`buckyos-api/src/msg_center_client.rs` | `msg.get_edit_capability`（与 `post_send` 同一套路由规划）；tunnel 的 `EditCapability` / `EditFailure`；Telegram 通用 edit 与一次性兜底；删除 status_line / turn_nonce / `TgUiSessionTracker`；私聊 edit 接收侧校验 |
| MessageHub | `src/frame/desktop/src/app/messagehub/`（`protocol/msgobj.ts`、`conversation/history/{relations,renderers,locate}.ts(x)`、`conversation/tasks/`、`MessageDetails.tsx`、`launch.ts`、`api/store.ts`、`mock/`）、`src/api/task_mgr.ts` | 完整 `MsgContent` 折叠、两行任务区、共享 task 缓存与订阅、展开树、通用消息详情、`messageId` 深链、占位与 edit 一并标已读、mock 会话“Release Checklist” |
| 文档 | `doc/opendan/protocol/`（Session Directory / Input / README、schema、fixtures）、`doc/opendan/xAgent.md`、`doc/message_hub/`、`src/frame/opendan/README.md`、MessageHub `UI_DATAMODEL.md`、[lib-opendan-outbound-todo.md](./lib-opendan-outbound-todo.md) §7 | 与上面的改动同步 |

### 7.2 实施中定下的细节

- **D6 的 ACL 结论**：TaskMgr 默认 preset 只认 `{user, app}` 完全相同的主体；另有一条“对 `obj://task/{owner}` 有 RBAC read 的控制面（Task Center）可读该用户全部 task”。在此之上，root task 创建时 grant 给 `User{owner}`：ReadMeta / ReadInput / ReadResult / Control，Subtree，Full。子 task 经父链继承。
- **task 状态映射**：Running + `message`（`running <tool>` / `working (<behavior>)`）与 `progress = {behavior, tool}`；`waiting_for` children → Waiting(ChildTask)、tool → Dependency、event → External、input → Other；Completed → `commit_result({summary})`；Failed / BudgetExhausted → `fail_task(turn_failed | budget_exhausted)`；Stopped → 确认已有的 cancel 请求，没有则自己请求再确认，终态 Canceled。
- **session_id / turn 在 task 的 input 里**：普通 `create_task` 不能写 `origin_ref`。
- **占位的 ObjId 在入队时就能算出**（MsgObject 内容定址），所以 Turn 结束时不必等占位发送结果；占位落定后按 msg-center 返回的 `msg_id` 修正 edit 的 target。
- **占位从未交给 sink 就结束的 Turn** 撤销占位、发普通回复；占位被拒时最终 edit 改回普通回复。
- **回复先于 stop 到达**：推理已经返回最终答复时，Turn 以 Completed 结束，随后 Session 才 stop；task 记为成功。
- **Telegram 兜底**是至少一次：平台已收下兜底消息、状态未落盘时进程退出，重试会再发一条（Telegram 没有幂等发送）。
- **“处理完成，回复同步中”是 UI 侧推断**：协议里占位与快速回复无法区分（D4 只带 task_id）。task 成功、没有折叠到 edit、消息早于 task 完成 2 秒以上、完成不到 10 分钟时显示。
- **长工具挂到 Turn task 下**：当前 Loader 的工具只产生进程内 task，没有产生 TaskMgr task 的工具，没有可挂接的对象；`parent_task` 的传递方式已就绪。

### 7.3 验证

- `cargo test -p libopendan -- --test-threads=1`：全部通过，新增 `tests/turn_task.rs` 10 例（快速 Turn、阈值占位、首次工具调用、不可编辑目标、占位被拒、停止收尾、task 服务不可用与终态重试、子 Session 父子关系、六个故障点的重启恢复）。
- `cargo test -p opendan -- --test-threads=1`：全部通过，`tests/loader.rs` 新增 3 例（占位 → edit 与快速回复与不可编辑、一个 Turn 开两个子 Session 的 task 树与终态、TaskMgr cancel → Session stop → Canceled 与占位收尾），用假的 msg-center 与假的 TaskMgr。
- `cargo test -p msg_center -- --test-threads=1`：124 通过。`cargo test -p task_manager builtin`：通过。`cargo check --workspace`：通过。
- Desktop（`src/frame/desktop`）：`tsc -b`、`eslint`（0 error）、`pnpm run build`、deno 数据模型测试 41 例、Playwright 66 例（mock 模式）。
- fixtures 与 JSON schema 已重新生成。

### 7.4 未完成与未验证

- 真实 Zone 验收只做了原生私聊（§7.5）。原生群聊没有做：Jarvis 包没有启用 `[[loader.ui]] on = "msg.group"` 规则。Telegram 没有做：需要真实 bot。
- 动态组授权（owner 之外的人读 task）。
- 未打开的会话，msg-center 的 `unread_count` 仍把占位与 edit 各算一条；UI 只在气泡可见时把两条一并标已读。
- websdk / desktop 的 TS 客户端没有加 `msg.get_edit_capability`（目前只有 OpenDAN 调用）；`src/rootfs/libexec/buckyos-tool/dist/msg_center_client.d.ts` 是旧的构建产物。
- Telegram：等待中的原消息可能让 edit 在 5 次投递后进入 DEAD；redact / reaction / thread 仍按普通消息发出。
- MessageHub：窄屏下深链只滚动到消息、不自动打开详情；独立路由点击打开详情时不把 `messageId` 写进 URL；真实后端定位消息最多回翻 50 页；观察模式没有专门的 UI 测试；详情里的产物引用按 `result.artifacts | outputs | files` 猜测（Turn task 的 result 目前只有 `summary`）。
- 续接关系（§3.4）按决定不做。

### 7.5 DV 环境验收（2026-10-04，原生私聊）

环境：本机 DV（`test.buckyos.io`），task-mgr 与 msg-center 数据已清空。`uv run buckyos-build.py` → `./build_aios --local-test`（`local/aios-test`）→ `uv run start.py`。脚本：`test/test_opendan/test_agent_chat_task.ts`（devtest 登录，`OPENDAN_URL` 指向调度器分配给 Jarvis 的端口，本次是 10032）；UI：`src/frame/desktop` 下 `MESSAGEHUB_REAL_E2E=1 MESSAGEHUB_REAL_ZONE_IP=127.0.0.1 AGENT_TASK_TREE_MSG=… pnpm exec playwright test --config=playwright.real.config.ts messagehub-agent-task`。

| 项 | 做法 | 结果 |
|---|---|---|
| 能力查询 | `msg.get_edit_capability`（用户 → Agent 的信封） | `editable = true` |
| 占位 → edit | “用 shell 执行 `sleep 6; date`” | 约 5 秒收到占位（带 `agent_task`），约 16 秒收到一条 edit；task Running（`working (chat_route)` → `running shell`）→ Terminal/Succeeded，result 有 `summary`；`turn_tasks.reported = true`，outbox 为 `placeholder` + `final_edit` |
| 快速回复 | “只回复两个字：你好” | 3.6 秒，一条普通消息带 `agent_task`，没有占位，task 成功 |
| task 树 | “请派一个 work session 完成…” | work session 的 task 是 UI Turn task 的子节点，各自成功；work session 的结果由父 Session 的新 Turn（新的 root task）转述 |
| owner 授权 | 事件里有 `AccessGranted`；用 devtest 的 control-panel token `request_control` | 放行。`list_task_access` 需要 Grant 权限，owner 读不到授权列表（未授予，按设计） |
| TaskMgr 取消 | 占位出现后 `request_control(Cancel, recursive)` | 约 3 秒后 Session stop，占位被 edit 为“已停止。”，task Terminal/Canceled |
| 重启恢复 | 占位发出后 `docker kill buckyos-app-jarvis` | 容器重建后同一个 task 继续，最终一条占位、一条 edit，outbox 两条都是 `attempts = 1` |
| MessageHub | 真实桌面页面登录后打开 Jarvis 会话 | 占位与 edit 是同一个气泡（带 edited）；任务区显示状态与“1 sub-task(s) finished”，展开有子任务节点；快速回复没有任务区；取消的气泡显示已取消；消息详情有原消息、当前内容、编辑记录、task 摘要与结果 |

验收中发现并修掉的问题：

1. **MessageHub 每个 task 开一条 kevent 流**，五个可见 task 加上消息流就占满浏览器对同一 host 的连接数，`get_task` 发不出去，任务区一直不出现。`createTaskWatchSource` 改为所有 task 共用一条流（订阅集合变化时 250 ms 去抖后重开），按事件路径分发。
2. **task 结束后仍显示最后一次活动**（“Done · running shell”）。OpenDAN 在终结前把 `message` 写成空（失败时写错误信息）。

验收中遇到、不属于本次改动的问题：

- kmsg 队列被清掉而 AgentRoot 里的 UI Session 还在时，inbox 桥对该 Session 投递一直报 `Queue not found`，这个收件箱就卡住了。本次删掉 Jarvis 的 AgentRoot 后恢复。Loader 对“登记的 Session 的输入队列不存在”没有自愈。
- `start.py` 重启后 cyfs-gateway 前几次启动报 3180 端口占用（旧实例尚未退出），约半分钟后自行恢复；这期间 app 容器因为连不上 verify-hub 反复重启。
- 重启恢复时，被打断的 `sleep 25; date` 按既有语义以“executor exited”作为工具结果返回，模型自己补跑了一次 `date`。

## 8. Turn worklog 主视图（2026-10-05）

- 打开 Agent 回复默认展示 `agent_task.task_id → task.input.agent_did / session_id / turn → session.worklog`。不会跟随同一 UI session 的后续 turn。消息仍只携带 task_id。
- 工具调用按 `(run_id, call_id)` 配对为 IN/OUT 时间线，旧在上、新在下；长内容可展开。向上翻页保持位置，底部实时补读并跟随，离开底部阅读时不被新日志拉走。
- 在创建调用中提供子 session 的点击入口，进入其首个 turn 的 worklog，可再打开 TaskMgr 详情与返回；关联依据为登记表 `origin.created_by_call`。结果顶层结构化 task_id 也提供任务入口，不解析普通文本。
- OpenDAN 新增只读 `session.worklog`：turn 过滤、字节游标、已提交边界、倒序分页/正序补读、明确 complete。字段契约见 `src/frame/opendan/README.md`。挂载 `/kapi/<完整 AgentId>`，网关路径匹配支持域名中的点，复用调度器已有 Agent service_info。没有新增依赖或持久协议版本。
- 代码入口：Desktop `conversation/worklog/{model,source,useWorklog,AgentWorklog}.ts(x)`；后端 `lib_opendan/src/session/worklog.rs`、`opendan/src/service.rs`；界面数据契约同步到 `UI_DATAMODEL.md`。
- 生效需要更新 Desktop、OpenDAN 与 boot_gateway 配置；本轮不改动正在运行的 Zone。

验证记录：

- OpenDAN `tests/loader.rs` 12 例通过，覆盖 `session.worklog` 的返回身份、分页、增量读取与 AgentId 别名路径；另补取消轮次 `turns_completed = 0` 但 `complete = true` 的断言。
- libopendan `tests/l1.rs` 20 例通过；分页测试覆盖跨 2100 条后续 turn 日志定位旧 turn、倒序分页顺序、2100 条增量补读、未提交尾部排除、非法游标与参数拒绝。
- Desktop 数据模型 13 例通过；MessageHub Agent Task 原有 6 例与新增 worklog 3 例 Playwright 通过。新增覆盖桌面/手机、默认页签、跨页、子任务进入/返回、旧 turn 刷新后不串轮、实时跟随/保留阅读位置、断线恢复、终态停止轮询。
- 网关 debug 回归 29 例通过，新增完整域名 AgentId 的同源 `/kapi` 路由。
- 本次前端文件 ESLint 通过；HEAD 加本次 Desktop 文件的隔离副本 `tsc -b` 与 Vite build 通过。原工作区另有正在进行的 AIWorkspace 修改，其类型错误/入口缺失使整树类型检查未通过；未修改这些文件。未做真实 Zone 联调或部署。
