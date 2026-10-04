# MessageHub Agent Chat：可编辑回复与 task 状态展示 TODO

日期：2026-10-04。状态：设计已评审，§0 的决定已冻结，没有遗留的待确认项；尚未实施。

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

- [ ] 由 msg-center 按确定的出站 route/target 提供能力查询；`MailService` / `OutboundSink` 暴露给 Turn 接入路径。Agent 不从平台名称、入站 transport 或最近活跃聊天推断出站能力。
- [ ] 原生 MessageHub 复用同一能力语义。（简化）群配置了 `edit_window_ms`（非 `None`）即视为不可编辑、跳过占位，不做服务端豁免。外部能力未知按不支持处理。
- [ ] 同一 Turn 多个目标分别决策；一个目标不可编辑不影响另一个可编辑目标。
- [ ] Telegram 把通用 edit 关系落到平台编辑 API：用原消息 ObjId 算出 delivery_id，读已持久化的 `external_msg_id`。禁止把 edit 正文当成新消息发出。
- [ ] （简化）删除 Telegram 既有的 status_line / turn_nonce / `TgUiSessionTracker` 状态消息链路，不做接入，避免两套占位。
- [ ] 处理编辑时间窗过期、原消息被删除、格式或附件无法原地替换：外部平台按能力更新占位为终态摘要，并以必要的独立消息交付完整附件/结果。确实无法编辑时发送一次普通最终回复作为兜底。
- [ ] 暂时失败按原键重试，永久失败才进入持久化的兜底发送状态；处理“平台编辑成功但 ACK 丢失”，避免重试又发送一份最终答案。

不支持编辑的 tunnel 不需要支持 MessageHub 的两行状态、树或详情页；这是原生 UI 的增强能力。能力不足不应阻断最终答复。

## 6. 实施顺序

### P0-a：OpenDAN 接入 TaskMgr（D2，先做）

- [ ] 确认 TaskMgr 默认 ACL 下 Agent 自建 task 对 owner 的可见性，定 grant 范围（D6）。
- [ ] 定 Agent Turn 的 task schema（input 含 agent_did、session_id、turn、输入消息引用）与 phase / wait_reason / outcome 映射表；task 正常出现在 Task Center（D7），名称与摘要要让人在列表里看得懂。
- [ ] libopendan 增加宿主钩子与 `origin` 里的父 task id；OpenDAN 用 TaskMgr 客户端实现创建（幂等键 = sid + turn）、状态上报、终结。
- [ ] 子 Session 的 task 挂到创建它的 Turn task 下；已有 task_id 的长工具挂到所属 Turn task 下。
- [ ] TaskMgr cancel → Session stop 的 bridge；重启恢复时按幂等键找回 task，不重复创建。
- [ ] 验证：UI Session 一个 Turn 开两个子 Session，`get_task_tree` 得到正确的树与终态。

### P0-b：原生闭环

- [ ] 冻结 `agent_task` 字段（§3.1），同步 TS 类型与协议文档。
- [ ] libopendan / OpenDAN：延迟占位、outbox 条目用途、最终 edit、失败/停止收尾与恢复；tunnel 目标门禁。
- [ ] MessageHub：edit 折叠替换完整 `MsgContent`；按 task 查询与默认两行状态；占位记录随 edit 标已读。
- [ ] 跑通一条消息从占位到最终结果，刷新后 task_id 不变；快速回复不出现占位。

### P1：完整三层体验与通道适配

- [ ] MessageHub 展开树、通用消息详情和消息级深链。
- [ ] 事件加速、轮询兜底、终态缓存、列表摘要同步。
- [ ] tunnel 能力查询与 Telegram 通用 edit；删除旧 status_line 链路。
- [ ] 私聊 edit 的接收侧校验；需要时再做服务端有效投影。
- [ ] 动态组授权：让 owner 之外的人（群成员等）读到 task。

### 联动与验收

- [ ] 协议改动同步 Rust API、Desktop 类型、实际使用的 websdk 客户端、Session 持久 schema，以及 `doc/message_hub/`、`doc/opendan/protocol/`、`doc/opendan/xAgent.md`、MessageHub `UI_DATAMODEL.md`。
- [ ] 与 [lib-opendan-outbound-todo.md](./lib-opendan-outbound-todo.md)、[opendan-agent-loader-refactor-todo.md](./opendan-agent-loader-refactor-todo.md) 对齐接口与恢复语义。
- [ ] Runtime 测试：快速完成（有 task、无占位）；阈值后发占位；Turn 中并入的输入；失败/预算耗尽/停止；子 Session task 的父子关系；task 创建、占位发送、响应丢失、task 终结、最终 edit 各阶段重启恢复；没有 TaskMgr 钩子的宿主行为不变。
- [ ] 投影测试：完整 Markdown/附件替换、原消息与 edit 乱序/重放、原消息已撤回、非原作者的 edit 被忽略、edit 上的 `agent_task` 被忽略。
- [ ] Tunnel 测试：门禁期间完全没有占位；P1 后覆盖支持/不支持/未知能力、编辑窗口过期、带附件、编辑成功但 ACK 丢失、兜底仅发一次、重启后不编辑错消息。
- [ ] UI 测试：原气泡位置及锚点稳定；两行/展开树/详情三层贯通；断线恢复；权限不足与任务已清理时不展示任务区；观察模式；最终答案不会被旧状态覆盖。
- [ ] 真实 Zone 验收至少覆盖原生私聊、原生群聊和 Telegram。

完成标准：每个含消息的 Turn 在 TaskMgr 有一个 task，子 Session 的 task 挂在正确的父节点下；正常可编辑路径从接受到终结只有一个主要回复气泡，并可由不变的 task_id 追溯本 Turn 的工作；快速回复和不支持编辑的目标只收到正常最终答复；重启、重试、多个 Turn 和编辑失败都不会造成串任务或遗失最终结果。

本次验证范围：静态核对上述代码入口与文档相对链接；未修改实现，未运行构建、服务测试或真实 tunnel 验收。
