# MessageHub Agent Chat：可编辑回复与 task 状态展示 TODO

日期：2026-10-04。状态：设计评审与待实施清单，本次只新增本文档。

基线：当前工作区代码（HEAD `fc2c36a0`，包含已有的未提交修改）。以下“现状”以代码为准；旧版 OpenDAN TODO 中的完成记录不能直接视为当前 Loader 已具备的能力。

## 1. 评审结论

建议推进。用一条稳定的回复承载“已接受 → 正在工作 → 最终结果”，很适合 Agent Chat；task_id 让消息成为任务的观察入口，也避免中间进度不断刷屏。

建议固定三条原则：

1. **用户看到原消息被编辑，存储层仍追加不可变消息。** 复用 `relates_to = { rel: "edit", target: <原消息 ObjId> }`，不覆盖 Named Object，不要求最终回复与占位消息具有相同 ObjId。稳定的是原消息锚点和 task_id。
2. **正文与运行状态分开。** 正文起初为“收到，开始处理…”，终结时一次性替换；工作中的两行状态、子 Session 树与详情由 task 查询驱动，不为每次状态变化发送一条 edit。
3. **task 对应一次逻辑请求，不能绑定整个长期聊天 Session。** UI Session 可处理很多个 Turn；其历史回复不能随着下一轮工作一起变化。终结后的 task_id 不改变，新的独立请求另建 task。

用户要求的完整体验：

- 原生 MessageHub：发送携带 task 引用的占位消息，默认展示两行状态，展开显示子 Session 工作状态，点击消息进入通用消息详情；最终在同一气泡显示完整回复，仍能查看该 task。
- 经 msg-tunnel 回复：发送占位前检查实际出站目标的编辑能力。支持才启用；不支持或能力未知时不发“收到，开始处理…”这条，结束后正常发送最终回复。
- 等待用户输入、失败、取消也是明确状态，不能让占位消息永久停留在“处理中”。等待输入不是完成，必须让用户看到需要回答的问题。

## 2. 当前实现与缺口

| 范围 | 已有能力（代码入口） | 本需求仍需补齐 |
|---|---|---|
| 消息编辑协议 | [msgobj.ts](../src/frame/desktop/src/app/messagehub/protocol/msgobj.ts) 有 `relates_to/edit`；[msg_center.rs](../src/frame/msg_center/src/msg_center.rs) 按 `gen_obj_id()` 保存对象，`post_send` 有幂等支持 | 将 Agent 占位与最终回复接到已有关系协议，固定跨版本的 task 绑定 |
| 编辑校验 | [group_service.rs](../src/frame/msg_center/src/group_service.rs) 的群消息校验检查原作者、Session、目标是否已撤回、编辑时间窗，且不允许编辑关系消息 | 普通私聊应有对应的服务端关系校验；长任务还需处理群编辑时间窗，不能假定原生通道永远可编辑 |
| 气泡投影 | [relations.ts](../src/frame/desktop/src/app/messagehub/conversation/history/relations.ts) 隐藏 edit 记录、折叠到原消息；目前只保留编辑后的文本，按 `created_at_ms` 选最新 | 替换完整 `MsgContent`（format、refs、machine 等），并补可靠排序与任务绑定检查；[renderers.tsx](../src/frame/desktop/src/app/messagehub/conversation/history/renderers.tsx) 仍从原对象读取格式和附件 |
| UI 编辑与详情 | [ConversationView.tsx](../src/frame/desktop/src/app/messagehub/ConversationView.tsx) 的 edit 操作只向群聊开放；[MessageHubView.tsx](../src/frame/desktop/src/app/messagehub/MessageHubView.tsx) 的详情对象是 entity/session；[launch.ts](../src/frame/desktop/src/app/messagehub/launch.ts) 可定位 entity/session | 补普通消息详情和消息级定位，再挂 task 信息；不能把现有 Session 详情当成已完成的消息详情 |
| Agent 回复 | [runner/outbound.rs](../src/frame/lib_opendan/src/runner/outbound.rs) 在 Turn 关闭的提交中落盘 outbox，随后发送并重试；[opendan/ui.rs](../src/frame/opendan/src/ui.rs) 的 `MsgCenterSink` 调用 `post_send` | 当前没有请求开始时发送占位及其 task 绑定；Stopped Turn 当前不发提示，启用占位后需补取消收尾 |
| Session / Turn | [opendan/ui.rs](../src/frame/opendan/src/ui.rs) 按 mailbox route_key 复用未结束 UI Session；[protocol/state.rs](../src/frame/lib_opendan/src/protocol/state.rs) 保存 open_turn、reply、outbox | 增加请求/Turn 与真实 TaskMgr task 的持久关联；当前路径没有“每个 Chat 请求自动创建 TaskMgr task” |
| 子 Session | [protocol/agent_state.rs](../src/frame/lib_opendan/src/protocol/agent_state.rs) 有 `origin.parent_session`、`one_line_status`、`report_brief`、`waiting_for`、rev；[state/krpc.rs](../src/frame/lib_opendan/src/state/krpc.rs) 提供 `sessions.children_of` | [runner/children.rs](../src/frame/lib_opendan/src/runner/children.rs) 的 `session:<sid>` 是 TaskResolver 的引用，不是 TaskMgr task_id；还没有本需求所需的请求级树映射 |
| TaskMgr | [task_mgr.rs](../src/kernel/buckyos-api/src/task_mgr.rs) 有稳定 task_id、parent/root、phase/outcome/wait_reason、message/progress、revision、树查询、持久事件、ACL/data_scope；[Desktop task_mgr.ts](../src/frame/desktop/src/api/task_mgr.ts) 已有 Task Center 客户端与状态投影 | 复用协议，新增 MessageHub 所需的按 task 加载与状态订阅，不能每个气泡重复拉整个任务列表 |
| Tunnel | [msg_tunnel.rs](../src/frame/msg_center/src/msg_tunnel.rs) 的 `DeliveryExecutor` 只声明 ingress/egress，没有通用 edit 能力查询；[tg_tunnel.rs](../src/frame/msg_center/src/tg_tunnel.rs) 已有 status_line、nonce、编辑状态消息为最终文本、失败转新消息的实现 | 抽出供 Agent 使用的目标能力；复用 Telegram 的执行动作，但把会话级内存状态升级为可恢复的消息/task 映射 |

补充：Telegram 当前仅在最终消息没有附件时把 `replace_message_id` 指向状态消息；其 `TgUiSessionTracker` 保存于内存。它证明已有可复用的实现，但并不等于已支持通用 `relates_to/edit`，也不具备本方案要求的跨重启请求绑定。当前 Loader 出站也没有接通这套 status_line/turn_nonce 链路。

## 3. 建议的数据与生命周期约定

### 3.1 三个 ID 各司其职

| 标识 | 含义 | 生命周期 |
|---|---|---|
| 原始占位消息 ObjId | 时间线位置、引用和消息详情的稳定锚点 | 最终 edit 始终指向它，不指向上一次 edit |
| task_id | 此请求的 TaskMgr 任务 | 接受、运行、等待、完成期间不变；重试投递不新建 task |
| session_id + turn / request key | Agent 的执行位置与请求归属 | 用于恢复和查询子 Session；不能代替 task_id |

建议复用 MsgObject 现有扩展字段承载结构化引用，暂定：

```json
{
  "agent_task": {
    "version": 1,
    "task_id": "<TaskMgr task id>",
    "agent_did": "<Agent DID>"
  }
}
```

这是待冻结的字段约定，不是现有接口。Rust 的 `meta` 会平铺到消息顶层，因此不要再包一层 `meta`；TS、schema、SDK 与协议文档应同步。不要从“收到…($taskid)”正文解析 ID，task_id 可在详情中复制，普通气泡不必展示内部编号。

占位和最终 edit 必须携带同一个绑定，服务端拒绝借 edit 更换 task/Agent。任务端通过已有 `origin_ref` 和约定的输入字段关联原始请求、Agent Session、Turn；具体字段在实施前冻结。`thread.reply_to` 等原有对话关系继续保留，不能拿来替代任务绑定。

### 3.2 请求进入与结束

建议流程：

```text
收到用户输入 → 确定请求/Turn 与回复目标
→ 幂等创建或关联真实 TaskMgr task，持久化绑定
→ 查询目标编辑能力
  ├─ 可编辑：占位进入 outbox → post_send → 保存原消息 ObjId / delivery IDs
  └─ 不可编辑或未知：不排入占位
→ 推进工作，发布可展示状态
→ 等待用户时展示问题并保持同一 task
→ 终结：有已发送占位则排入最终 edit，否则排入普通最终消息
→ 出站失败恢复 / 重试，直到终结内容可见或有明确投递失败状态
```

- 把“已接受请求”作为发占位的时点，由 runtime 生成固定文案；不能等 LLM 自行回复“收到”才有反馈。排队阶段如实显示“已收到，等待处理”，真正开始后再显示“正在处理”。
- 按逻辑 Turn 绑定请求，并保存其输入消息集合。当前 Turn 可能包含多条输入；追问、补充、等待输入恢复如何归属应显式决定，不能靠“最近一条消息”猜测。新独立请求或显式重做使用新 task，可通过已有 `retry_of` 表达任务关系。
- 有上游 task 的执行可关联该 task，但先确认它对应本次用户请求，不能直接挂到跨多个请求的总任务。工具 task 和后台子工作也不能冒充用户请求的根 task。
- outbox 增加占位、最终 edit 等操作用途及必要绑定，复用现有落盘与重试机制；占位与最终回复用不同幂等键。同一项重试重用完整 MsgObject，包括 nonce 和 created_at_ms。
- 请求绑定的保留期应独立于 outbox 当前“保留最近 16 条结束项”的清理策略；不能因下一轮对话或清理丢失原消息锚点。
- 最终 edit 的业务前置条件是占位已被 msg-center 接受；同一外部目标的编辑执行还必须等原消息投递成功、external_msg_id 已知。`post_send` 成功不等于外部平台已收到。
- 快速完成时：尚未发送的占位可在持久状态中取消，直接发送最终回复；已经发送则必须编辑。发送结果不确定时先按原幂等键恢复，防止多出一条占位。
- TaskMgr 终态与最终消息投递是两件事。任务已成功但 edit 待重试时，显示“处理完成，回复同步中”；不能提前停止消息投递状态刷新。
- 失败、预算耗尽、停止/取消都应收尾已存在的占位；未启用占位时保留正常回复路径。task 创建或能力查询故障时不发无效的任务占位，最终回复仍应能送达。
- Turn 已结束但没有可发送正文时，用明确的结束摘要收尾已有占位；最终普通消息或兜底消息也保留已建立的 task 引用，不能因降级丢失关联。

### 3.3 子 Session 树

建议首版以真实请求 task 为入口，由 OpenDAN 提供该请求的只读执行视图，复用 Agent State 登记表和 `sessions.children_of`，不必先把每个 Session 镜像成 TaskMgr task。

- 保存请求与其子 Session 的关联，结合 `origin.created_by_call` 等执行关系追溯所属 Turn；只按长期 UI Session 的 `parent_session` 查询，会混入历史请求的子 Session。
- TaskMgr 管任务生命周期，Session state/registry 管执行状态；适配层明确由谁更新 task 的 phase、message、progress，禁止两边互相推导写回形成循环。
- 真实 TaskMgr 子任务用现有 `get_task_tree/get_subtasks` 读取；Session 子节点保留 session_id 类型。混合展示时区分两种节点，不能把 `session:<sid>` 传给 TaskMgr。
- 终结后保留此请求的树归属和结束摘要；详情不能改为展示同一 UI Session 后续 Turn 的状态。因请求结束仍继续运行的后台子工作要标明其关系。
- 首版状态来自 `one_line_status`、`report_brief`、等待原因与已有任务事件；更细的工具时间线依赖 worklog 完整度，不承诺当前已经记录了所有工具动作。

## 4. 展示与刷新

### 4.1 默认两行

- 第 1 行：任务阶段 + 当前活动，例如“正在处理 · 检查 MessageHub 消息投影”。
- 第 2 行：可用的子工作统计、等待原因或最近更新时间，例如“2 个子 Session 运行中 · 更新于 3 秒前”。没有可计量的总量时不显示推测的百分比。
- `Promised/Accepted` 显示排队/已接受；`Running` 显示工作；`Waiting` 区分等用户、等子工作和外部依赖；`Paused` 显示暂停；`Terminal` 根据 outcome 显示成功、失败或取消。`pending_control` 可用于显示停止请求处理中，不能提前宣称已取消。
- 最终正文替换后保留折叠的任务摘要与详情入口。查询超时、任务不可访问、任务已清理时显示具体的不可用状态，保留可读正文，不持续伪装“运行中”。

### 4.2 展开与消息详情

- 气泡展开：本请求的子 Session / 子任务树、各节点状态、简短活动说明、等待原因；支持逐层或分页加载。
- 点击消息进入通用消息详情，也提供明确的“详情”操作和键盘入口；正文中的链接、附件、选择文本与展开按钮继续各自工作。
- 通用详情展示原消息、当前有效内容、编辑记录和投递状态；存在 `agent_task` 时追加实时任务区，展示摘要、请求树、可访问事件、结果和产物引用。
- 消息级深链用原消息锚点，补齐路由与 launch schema；未加载该条历史时能够定位，刷新页面、重新进入详情仍得到相同 task。
- 消息可读不代表整个任务树可读。读取 task、Session 视图、事件与产物分别沿用真实登录者的权限；观察模式不借 owner 身份读取，群消息也不能默认公开 Agent 的全部 Session。必要时仅授予本请求的展示范围，权限撤销后及时收敛缓存。

### 4.3 实时性

- 原生气泡按可见 task 集合共享缓存和订阅；按 task_id/revision 去重。展开或进入详情后才取树和事件，折叠/离开时释放不需要的订阅。
- 复用 `/task_mgr/<id>` 与 `/task_mgr/tree/<root_id>` 事件路径，把事件作为“需要重读”的提示；保留有界轮询、断线重连补读与旧 revision 丢弃。Session 视图也需独立的刷新来源，不能假定其变化已有对应 TaskMgr 事件。
- 复用 msg-center `box_changed` 刷新占位及 edit 投影；TaskMgr 事件只刷新任务区。停止订阅前确认终态快照与最终消息投递各自已收敛。
- 列表摘要、搜索/引用预览、未读与通知策略应读取有效消息内容：进度不产生新通知，最终结果应有一次可感知的到达提示。占位已读不应吞掉用户离线期间到达的最终结果。

## 5. msg-tunnel 能力与降级

不要只增加一个静态 `supports_edit = true`。至少区分“目标支持编辑自己的消息”和“该条消息此刻可编辑”；还要表达文本/附件限制、编辑时间窗及最终执行错误。

- [ ] 由 msg-center 按确定的出站 route/target 提供能力查询；`MailService` / `OutboundSink` 暴露给请求接入路径。Agent 不从平台名称、入站 transport 或最近活跃聊天推断出站能力。
- [ ] 原生 MessageHub 复用同一能力语义，结合群规则/时间窗判断。外部能力未知按不支持处理；未知或不支持时跳过占位，照常发送最终结果。
- [ ] 同一请求多个目标分别决策与保存投递映射；一个目标不可编辑不影响另一个可编辑目标。群聊以实际群目标及 host 规则判断，不按各成员私聊能力猜测。
- [ ] Telegram 复用已有 edit/send 操作，新增按原消息 ObjId、delivery_id、transport、账号、chat 定位 external_msg_id 的持久映射；不能仅依靠 `TgUiSessionTracker` 的内存槽位或“该 Session 最近一次状态消息”。
- [ ] 将通用 edit 关系落到对应平台编辑 API；禁止把 edit 正文不加区分地当成新消息发出。停用或接入既有 status_line 自动生成逻辑，确保不会同时出现两套占位。
- [ ] 处理编辑时间窗过期、原消息被删除、格式或附件无法原地替换：原生消息展示完整最终内容；外部平台按能力更新占位为终态摘要，并以必要的独立消息交付完整附件/结果。确实无法编辑时发送一次普通最终回复作为兜底，记录与原锚点的关系。
- [ ] 暂时失败按原键重试，永久失败才进入持久化的兜底发送状态；处理“平台编辑成功但 ACK 丢失”，避免重试又发送一份最终答案。编辑完成后旧进度刷新不得覆盖最终内容。

不支持编辑的 tunnel 不需要支持 MessageHub 的两行状态、树或详情页；这是原生 UI 的增强能力。能力不足不应阻断最终答复。

## 6. 实施顺序

### P0：冻结协议与补齐原生闭环

- [ ] 固定结构化 task 引用、请求/Turn 归属、任务 schema 和生命周期映射；先验证“普通 Chat 请求能够创建/关联真实 task”，再开发依赖它的 UI。
- [ ] 在 msg-center 补全普通私聊 edit 目标与作者校验，并对齐群编辑规则；验证同一会话/目标、原锚点、绑定不可变和被撤回消息处理。
- [ ] 复用现有关系投影实现完整内容替换；采用接收侧可靠次序或明确 revision 规则，避免依赖发送者时钟决定最终内容。服务端有效投影或关系查询需保证原消息与 edit 跨页、乱序到达、重载时仍能收敛。
- [ ] 在 libopendan / OpenDAN 接入请求 task、占位 outbox、最终 edit、失败/取消收尾与恢复；不把平台 API 放进 libopendan。
- [ ] MessageHub 接入按 task 查询与默认两行状态，跑通一条消息从占位到最终结果，刷新后 task_id 不变。

### P1：完整三层体验与通道适配

- [ ] OpenDAN 提供请求范围的子 Session 视图；MessageHub 实现展开树、通用消息详情和消息级深链。
- [ ] 加入事件加速、轮询兜底、权限裁剪、终态缓存、最终回复通知与列表摘要同步。
- [ ] 实现能力查询及 Telegram 持久编辑映射；验证不支持编辑时完全没有占位消息。

### 联动与验收

- [ ] 协议改动同步 Rust API、Desktop 类型、实际使用的 websdk 客户端、Session 持久 schema，以及 `doc/message_hub/`、`doc/opendan/protocol/`、MessageHub `UI_DATAMODEL.md`。跨仓库共享 MsgObject 类型如需改动，再同步其权威定义，优先复用现有扩展能力。
- [ ] 与 [lib-opendan-outbound-todo.md](./lib-opendan-outbound-todo.md)、[opendan-agent-loader-refactor-todo.md](./opendan-agent-loader-refactor-todo.md) 对齐接口与恢复语义；细粒度工具时间线依赖前者记录的 worklog 后续项。
- [ ] 后端/投影测试：越权 edit、跨会话目标、改变 task_id、完整 Markdown/附件替换、原消息与 edit 跨页/乱序/重放、原消息已撤回。
- [ ] Runtime 测试：快速完成；合并输入与后续独立 Turn；等待用户后继续；失败/预算耗尽/取消；子 Session 属于正确请求；占位发送、响应丢失、task 终结、最终 edit 各阶段重启恢复。
- [ ] Tunnel 测试：支持/不支持/未知能力；同会话多个请求交错；占位投递晚于 task 完成；编辑窗口过期；带附件；外部编辑成功但 ACK 丢失；兜底仅发一次；重启后不编辑错消息。
- [ ] UI 测试：原气泡位置及锚点稳定；task_id 不变；两行/展开树/详情三层贯通；断线恢复；权限不足与任务已清理；观察模式；键盘及移动端；最终答案不会被旧状态覆盖。
- [ ] 真实 Zone 验收至少覆盖原生私聊、原生群聊和 Telegram；UI mock 或协议测试通过不能替代外部平台编辑回执的验收。

完成标准：正常可编辑路径从接受到终结只有一个主要回复气泡，并可由不变的 task_id 追溯本请求的工作；不支持编辑的目标只收到正常最终答复；遇到平台编辑限制时按第 5 节交付结果；重启、重试、多个 Turn 和编辑失败都不会造成串任务或遗失最终结果。

本次验证范围：静态核对上述代码入口、文档相对链接及 diff 格式；未修改实现，未运行构建、服务测试或真实 tunnel 验收。
