# MessageHub UI 原型修改 TODO

- 日期：2026-09-06
- 状态：已完成 T0–T8（2026-09-06，mock 原型）；2026-09-07 完成第 3 节的真实后端集成主体，剩余项见第 3 节末尾。
- 交付与验收：[实现说明、检查结果与截图](IMPLEMENTATION.md)。
- 目标：在现有 MessageHub 原型中完成 Session 列表、创建、详情、归档 / 删除、状态展示及 Action Message 的交互闭环。
- 实施方式：先由可交互的 mock 数据层支撑全部流程；真实消息服务、平台 API、DDL 和跨 owner 后端授权按后续集成任务落地。
- 依据：[产品 PRD](../../product/message_hub/MessageHub_Web_UI_PRD.md)、[UI DataModel](../../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)、[Session State 与 Action Log](../../doc/message_hub/Session%20State%20and%20Action%20Log.md)。

## 1. 本次新增要求与解释优先级

本清单承接此前的实体 DID、owner 视角、tunnel 绑定、创建策略、状态与 Action Message 设计，新增以下明确要求：

1. 删除 Session 行右侧的选中竖条，显示距最后消息活动的时间，如 `2m`、`4h`；悬浮时显示删除按钮。
2. 点击删除按钮，弹出“归档 / 彻底删除 / 取消”的选择，不直接执行删除。
3. Session 整体状态、成员状态及对端在该 Session 内的状态变化不更新 last update，不改变排序；
   标题区域应即时反映状态，例如 typing 图标。
4. 新建 Session 从入口、提交到打开空会话、发送第一条消息的流程必须完整可用。
5. 增加独立的 Session 详情视图，保持与实体详情的入口和对象区分。

**新的时间口径优先于 UI_DATAMODEL.md 现有的 `lastActiveAt = SessionSummary.updated_at_ms` 写法。**
状态变化产生的 Action Log 虽然是持久消息，也不能间接更新时间、重排或解除归档。
后续 agent 完成原型时须同步修正相关数据模型说明，不能按旧等式重新引入状态驱动排序。
“状态会影响 title 展现”指标题旁的状态呈现随数据变化，不把 `typing…` 写入持久标题。

## 2. 已核对的当前实现与修改入口

以下路径相对于仓库根目录；这是任务编写时的源码现状，不代表这些目标已经实现。

| 当前入口 | 现状 | 主要任务 |
|---|---|---|
| `src/frame/desktop/src/app/messagehub/SessionSidebar.tsx` | 右侧选中竖条；新建按钮没有 onClick；没有时间 / 删除入口 | T2、T3、T4 |
| `src/frame/desktop/src/app/messagehub/MessageHubView.tsx` | Session 直接读取静态 mock；默认选数组首项；发送仅追加本地 reader；只有 showDetails 布尔值 | T0、T1、T3–T5、T7 |
| `src/frame/desktop/src/app/messagehub/ConversationView.tsx` | 标题整体和更多按钮都打开 EntityDetails；Session 数 > 1 才有侧栏入口 | T2、T4–T7 |
| `src/frame/desktop/src/app/messagehub/EntityDetails.tsx` | 只有实体详情，尚无 SessionDetails | T5 |
| `src/frame/desktop/src/app/messagehub/types.ts`、`mock/data.ts` | 短 ID、旧 Session 类型与静态列表；缺 owner、稳定绑定、能力和生命周期 | T0、T7 |
| `src/frame/desktop/src/app/messagehub/conversation/history/` | 有原始 reader、索引 / 窗口投影及通用 renderer；尚无 Action 识别与过滤 | T6 |
| `src/frame/desktop/src/app/messagehub/MessageHubRoute.tsx`、`MessageHubAppPanel.tsx` | 独立路由只消费 entityId；桌面 Panel 尚未消费启动参数 | T4、T7 |
| `src/frame/desktop/src/app/users-agents/components/detail/AgentDetailPage.tsx` | 已有 Agent 详情页，可承载“查看 Agent 会话”入口 | T7 |
| `src/frame/desktop/src/i18n/dictionaries.ts`、`desktop/windows/dialogs.tsx` | 已有文案与窗口内 dialog 基础设施 | T2–T7，复用现有能力 |
| `src/frame/desktop/tests/e2e/pages/messagehub.spec.ts` | 已覆盖 Composer 尺寸与桌面 / 移动端长历史滚动 | T8，扩展而非替换现有回归 |

按 T0 → T1 → T2–T5 → T6–T7 → T8 的顺序收敛。T7 的 owner / 权限数据基础在 T0 先建立，之后再接入口。

## T0. 可交互 mock 数据层与状态边界

- [x] 将静态 `mockSessions` 作为 seed，引入 MessageHub 内可读写的 mock store / provider。
  复用仓库现有 store 模式，不把创建、归档、删除、状态更新分散为组件里互不一致的数组副本。
- [x] 按 UI DataModel 补齐 Entity / Session 的 DID、`ownerDid`、稳定 binding、origin、创建策略与有效访问能力。
  更新相关 mock 引用和路由默认参数，父子实体均有独立 DID；不同 tunnel 的 Session 不因联系人聚合而合并。
- [x] store 提供创建、归档、恢复、删除本地会话、更新共享状态 / 自己的成员状态及读取详情等动作。
  这些是原型数据层方法，不冒充已经存在的新 KRPC；组件不直接调用尚未落地的后端 API。
- [x] 所有 Session 引用以 `(ownerDid, sessionId)` 为作用域；缓存、草稿、选择与 UI 偏好再按 viewer 隔离。
  当前选择由视图持有，废弃 Session 数据里的 `isActive`。
- [x] 区分普通消息活动时间、共享 / 成员状态的 updated_at、临时运行态与本地展示偏好，按 T1 更新。
  本地生命周期至少表达活动 / 已归档；彻底删除保留必要的 mock 删除标记，避免 seed 重新加载后复活。
- [x] 创建、归档、删除及持久字段修改在刷新后可恢复；typing / 临时写入风险确认不恢复。
  小型元数据可持久化为 mock 差量，长历史沿用现有 reader / IndexedDB 能力；不要把数千条 seed 反复复制到 localStorage。
- [x] mock 动作支持可控加载与失败，以及可控时钟 / 状态事件注入，供 T8 验收；调试控制不进入产品正常流程。

验收：同一操作的结果同时体现在 Session 列表、Conversation、详情与实体聚合中，刷新和切换身份不会串数据。

## T1. last update、相对时间与稳定排序

沿用 UI 字段 `lastActiveAt` 表示“最后有效消息活动时间”，不将它用作任意属性的最后修改时间。

| 事件 | 更新 lastActiveAt | 其它变化 |
|---|---|---|
| 新建空 Session | 初始化为创建时间 | 打开空历史；不能靠伪造消息初始化 |
| 新普通收发消息 / 有内容的任务结果 | 按消息活动时间推进，取 max 防止迟到消息使时间倒退 | 更新摘要与正常未读 |
| typing / active / processing / status_line / 对端状态 | 否 | 标题旁图标 / 状态说明更新，过期后移除 |
| 共享标题、成员昵称、个人显示标题更新 | 否 | 对应标题 / 名称即时更新 |
| 状态变化对应的 Action Message | 否 | 正常写入历史，按 T6 展示 / 隐藏 |
| 已读、投递进度、失败重试状态更新 | 否 | 未读 / 投递图标更新；同一条消息不重复算活动 |
| 归档 / 恢复、删除、切换显示过滤、纯时间刷新 | 否 | 生命周期 / 可见集合 / 相对时间文本更新 |

- [x] 提取统一的消息活动判定与排序逻辑，原型普通 chat / group_msg 和真实内容结果按上表推进。
  不能对所有 `MsgObject` 一律更新，也不能把整个 state.updated_at 或最后一条 Action Message 的时间拿来排序。
- [x] 活动列表置顶优先，其余按 lastActiveAt 降序；时间相同使用稳定 Session ID 次序。
  置顶是明确的用户排序操作，不属于状态自动重排。实体列表使用同口径聚合，typing 不使实体跳位。
- [x] 相对时间使用同一 lastActiveAt：`<1m` 显示 `now` / “刚刚”，其后取整为 `2m`、`4h`、`3d`。
  悬浮时间文本可查看本地完整日期时间；未知时间显示 `—`，未来时间差按 0 处理。
- [x] 按分钟刷新相对时间文本；纯时钟 tick 不写 store、不改变 lastActiveAt、不重建消息历史。
- [x] 标题主文本继续遵守“个人显示覆盖 → 共享标题 → 派生标题”的优先级。
  标题区域按当前 Session 的 member DID 展示 typing / processing 等状态，区分自己与对端，不读取其它 Session 的状态。
- [x] 明确摘要与排序的独立性：Action Log 可按已有摘要契约出现，但不能通过摘要 timestamp 反推 lastActiveAt。

验收示例：A 最后活动为 2m、B 为 4h；B 开始 typing、修改标题并生成 Action Log 后，B 的标题区域更新，
时间仍显示 4h、顺序仍在 A 后面；B 收到一条新的普通消息后才正常前移。

## T2. Session 行布局与可访问操作

- [x] 删除现有行右侧绝对定位的选中竖条及阴影；用文字字重 / 轻背景和恰当的选择态语义保持当前项可辨认。
- [x] 行结构为“来源图标 + 标题 / 状态 + 未读 + 相对时间”；长标题截断，相对时间保留固定空间。
  同平台多 tunnel 要能通过账号 / 连接名称分辨，不只显示两个相同 Telegram 图标。
- [x] 鼠标悬浮行时，在时间旁预留的操作位显示删除按钮，退出后隐藏按钮；时间保持可见，避免标题左右跳动。
  键盘 focus-within 同样可见；触屏通过行操作菜单或等价可发现入口访问，不能依赖 hover。
- [x] 点击删除按钮只打开 T3 的处理对话框，不能先选中该 Session 或触发父行点击。
  当前整行是 button，改造时避免嵌套 button，确保 Tab / Enter / Space 操作正常。
- [x] 删除图标、来源和状态图标有可读标签；中英文文案走现有 i18n。

验收：桌面 / 移动端均能选择 Session、识别时间并打开处理对话框；右侧选中竖条完全消失。

## T3. 归档与彻底删除的完整流程

本 TODO 对原型采用如下默认语义，后端具体删除协议列入集成待办：

| 选择 | 效果 | 后续行为 |
|---|---|---|
| 取消 | 不修改数据与选择 | 关闭对话框，焦点回到触发处 |
| 归档 | 当前 owner 的 Session 移出活动列表，保留历史、状态与草稿 | 可在“已归档”入口查看与恢复；不自动标已读 |
| 彻底删除 | 删除当前 owner 的该会话记录、本地历史引用、草稿与个人配置 | 不提供恢复；不删除对端 / 其它 owner 的记录，也不删除实体或断开 tunnel |

- [x] 对话框标题 / 描述明确目标 Session，提供“归档”“彻底删除”“取消”三个清晰动作；
  删除为危险样式，不设为回车默认动作；实际执行前的文案明确“删除此视角中的会话及本地历史，无法恢复”。
- [x] 成功后统一更新列表、计数、详情和 reader 引用；失败保留原数据与选择，显示错误并可重试，重复点击不能重复执行。
- [x] 处理当前选中 Session 后，选择同实体下排序后的下一条活动 Session；没有则进入正常“尚无会话”状态。
  非当前 Session 被处理时不打断当前历史和草稿；最后一个 Session 被删除不删除实体、不自动造一个替代会话。
- [x] 增加轻量“已归档”入口和恢复动作，即使活动 Session 为 0 / 1 条也可到达。
  恢复沿用原时间与历史；新普通消息可使归档会话恢复活动，typing / 状态变更日志不能解除归档。
  归档本身不改未读状态，App badge 沿用 owner 的原有统计口径。
- [x] mock 删除结果刷新后仍有效；重复 seed / 已有历史重放不能恢复已删除内容。
  tunnel 仍然连接时，后续真实新消息可以重新形成可见会话，但不能恢复已删除历史；提供可验证的 mock 场景。
- [x] 本地会话管理权限独立于 Composer 只读：用户自己的 tunnel Session 可以有归档 / 删除权限，
  Agent 只读观察视角不开放这些写动作。能力不可用时在详情中说明原因。

注意：当前 `msg.update_record_state` 是单条记录状态修改，`DELETED` 也不等于会话级物理清除。
原型只操作 mock store；接后端前需确认 Session 级处理、共享对象引用与新消息重现规则，不能用一串逐条删除伪装成已完成的服务契约。

## T4. 新建 Session 从入口到第一条消息

- [x] 接通 SessionSidebar 的新建按钮；Conversation 标题操作区 / 零会话空态也提供同一入口。
  侧栏在 0 / 1 个 Session 时可以隐藏，但新建和已归档入口不能一起消失。
- [x] 入口按当前 owner 与目标实体的 sessionCreation 策略和有效能力判断：默认 Agent 允许，其它实体关闭；
  显式配置可覆盖。Agent 只读观察禁止创建，能力不足要有原因。
- [x] 从实体上下文进入时预填目标；全局入口只展示允许创建的实体，默认可选 Agent。
  简单表单包含目标实体、可选标题、必要时的连接选择；当前先创建 chat，不展示无法完成的 task / workspace 选项。
- [x] 标题 trim 后最多 64 字符，空标题按已有默认标题规则；相同标题允许共存，ID 必须稳定且唯一。
  用户给定的创建标题初始化共享状态，个人显示覆盖保持独立。Session 的 owner 与 binding 不能从标题猜测。
- [x] 多连接时显式选定连接；在 tunnel 内创建还需 supportsMultipleSessions 与 canCreateRemoteSession。
  原生 Agent 会话可直接创建；外部仅可读线程时，不能假装创建了远端线程。
- [x] 提交时显示进度并防重复，成功后登记空 Session、创建空 reader、更新实体计数、选中新会话并进入 Conversation。
  不复制其它 Session 的历史、草稿或 typing，不生成假的“第一条聊天消息”。
- [x] 可写空会话显示“开始对话”并可输入；只读空会话显示“暂无消息”和只读原因。
  第一条普通消息写入同一 Session ID，列表摘要 / lastActiveAt 随之更新；刷新、重新进入后仍可读取。
- [x] 取消不创建；失败保留表单值可重试；请求期间切换实体 / owner 或关闭入口后，迟到结果不能覆盖新上下文选择。

验收：默认 Agent 新建两次相同标题得到两个独立会话；Person 默认无法创建，策略显式允许且连接能力满足后可以完成流程。

## T5. 新增 Session 详情页

- [x] 新增模块内 `SessionDetails` 视图，复用现有详情面板 / 移动端详情页布局，不另建顶级管理 App。
- [x] 将“实体名 / 头像”与“Session 标题 / 详情”拆为独立入口；前者保留 EntityDetails，后者打开 SessionDetails。
  更多菜单明确“会话详情”，不能继续所有入口共用 onOpenDetails。
- [x] 详情状态使用实体 / Session 可区分的目标；切换会话后更新到新目标，处理掉目标后关闭或进入有效空态。
  返回 Conversation 时保留所选 Session、滚动位置与草稿；无 Session 时禁用会话详情入口。
- [x] 详情至少展示：会话标题、类型、对端实体、会话所属身份、来源 / 连接、读写模式及原因、创建时间、
  最后消息活动时间、活动 / 归档状态。技术标识放可展开的来源信息中，不要求普通用户输入 DID 或 Session ID。
- [x] 分开呈现共享状态与“我在此会话中的状态”：共享标题 / 说明、自己的会话昵称；
  按能力编辑并显示保存中 / 失败 / 成功，修改后更新相关展示、生成对应 mock Action Message，遵守 T1 不重排。
- [x] 个人显示标题、置顶 / 静音等本地偏好与共享编辑分开；个人标题修改不生成共享日志。
  其它成员状态可在可见范围内只读查看；不增加本轮无关的群角色 / 入群审批管理流程。
- [x] 详情提供 T3 的归档 / 删除 / 恢复入口；在 tunnel 会话提供按能力启用写入及恢复只读的入口。
  启用写入须确认“可能造成另一个软件中的会话历史记录错误或不一致”，确认不授予状态编辑权限。
- [x] EntityDetails 提供“允许手工创建到该实体的会话”简单配置，按 owner / entity 保存；
  默认值、显式覆盖与有效能力分开，不能通过此开关绕过平台限制。

验收：打开两种详情可明确分辨正在查看实体还是会话；共享标题、个人显示标题、自己的昵称三个编辑结果互不覆盖。

## T6. Session 状态呈现与 Action Message

- [x] 原型能注入共享状态、成员昵称和带有效期的运行态；标题区域只展示当前会话相关的状态。
  typing 结束 / 过期应恢复正常标题呈现；它不进入持久标题、lastActiveAt 或 Action Log。
- [x] 实现共用类别判断：`kind === 'event' && content.machine?.intent === 'buckyos.action_log'`。
  读取结构前校验 schema_version；按 data.action 展示入群、主动退出、被移除、改标题 / 昵称等系统消息。
- [x] 增加 Action 专用展示分支，排在通用文本 / 图片渲染前；用系统事件行等轻量样式，保留完整消息数据。
  actor 与 subject 不混用，未知操作者 / 未知动作 / 不支持版本显示合理摘要，不能崩溃或执行载荷里的动作。
- [x] 增加“显示 Action Message”个人 UI 开关，默认显示；可按 viewer / owner / Session 记住选择。
  特殊展示与隐藏使用同一识别规则，不能按文案匹配，也不能把所有 event 一起隐藏。
- [x] 隐藏在 UI 可见投影层完成：原始 reader 和消息 ID / messageIndex 保留，重新计算可见 entries、
  时间分隔和 totalCount。不能靠 renderer 返回 null 隐藏，否则 fallback 仍可能重新显示。
- [x] 过滤本身不写后端、不改未读、归档、状态或 lastActiveAt；重新显示恢复历史。
  避免空白虚拟行 / 孤立日期，保留可见锚点；全被过滤时提示“当前消息已被过滤”，不是“尚无历史”。
- [x] 模拟状态修改成功后生成一条对应日志；失败、无变化、重复请求不重复生成。
  纯状态更新和它产生的日志都遵守 T1，不因 Action renderer / 过滤开关变化而重排 Session。

## T7. 承接之前的实体、tunnel 与 Agent 视角设计

- [x] mock 覆盖一个实体对应多个 tunnel、同一 tunnel 多 Session，以及父实体 / 独立子实体各自的会话。
  建立连接自动添加默认空 Session，重复发现幂等；只有联系人资料时允许零会话。
- [x] tunnel Session 默认只读；有能力时通过 T5 风险确认启用当前会话写入，并持续显示来源与模式。
  刷新 / 切换 owner 清除临时确认；连接失效或绑定未知时禁用发送，不自动换 tunnel。
- [x] Composer 输入、粘贴 / 拖拽附件、快捷键发送和失败重试共用能力判断；新建入口与状态编辑分别判断权限。
- [x] 默认入口始终是登录用户视角；从 Agent 主页增加“查看 Agent 的会话”入口，进入 Agent owner 的只读原型视角。
  与“用户和 Agent 对话”入口分开，路由和桌面 Panel 启动参数都传递明确 context。
- [x] Agent 观察显示清楚的 owner / 只读标识，允许查看 Session 和两种详情；
  禁止新建、发送、归档 / 删除、状态修改及修改 Agent 的已读 / 草稿 / 配置。
  允许观察者自己的纯 UI 展示过滤，不把偏好写进 Agent 状态。
- [x] 退出 / 切换视角后隔离选择、分页、reader、草稿、临时状态和迟到响应。
  Agent 未读不加入用户 badge；无查看权限显示拒绝态，不能回落到其它身份缓存。

## T8. 验收、回归与文档交付

后续 agent 应补充有意义的行为测试，优先覆盖时间 / 权限 / 生命周期边界，避免只断言组件内部字段。

| 场景 | 必须观察到的结果 |
|---|---|
| 两条 Session：2m / 4h，旧会话 typing → 改标题 → Action Log | 标题呈现变化，时间与顺序不变；新普通消息才推进 |
| 相同活动时间、迟到消息、分钟 tick、投递状态刷新 | 排序稳定，不倒退、不无故前移 |
| 悬浮 / 键盘 focus / 触屏操作 | 删除入口都可到达；没有右侧竖条，无嵌套按钮 / 误选中 |
| 取消、归档 → 已归档 → 恢复、彻底删除 | 语义正确，刷新后保留结果，错误可重试，最后一个 Session 不留下悬空详情 |
| 归档后的 typing / 日志 / 普通新消息 | 前两者仍归档，普通新消息恢复活动 |
| Agent 新建、Person 默认禁止、配置覆盖、平台不支持 | 入口与提交都遵守能力，0 / 1 Session 不影响入口可达性 |
| 新建成功 → 空历史 → 发首条 → 刷新 | 同一个 Session ID，独立历史 / 草稿，摘要与活动时间正确 |
| 共享标题、个人显示标题、自己的会话昵称编辑 | 正确同步 / 记录日志，互不覆盖；不允许直接修改别人或权限字段 |
| Action 显示 / 隐藏，混合普通 event、长历史及全被过滤 | 只过滤 Action，没有虚拟空行，恢复不丢消息、不修改状态 |
| 同一联系人下两个同平台 tunnel / 同名 topic | Session、状态和发送目标均不串线 |
| 用户 / Agent owner 同名 Session ID、权限拒绝、迟到响应 | 数据隔离，观察无写入，纯 UI 偏好不污染 Agent |
| `/messagehub` 独立页面与桌面内嵌窗口，1440px / 375px | 创建、处理对话框、两类详情和返回路径均可用 |

- [x] 扩展 `tests/e2e/pages/messagehub.spec.ts`；复用原有长历史滚动和 Composer 回归。
  涉及 Agent 主页入口时扩展 `users-agents.spec.ts` 的相关流程。
- [x] 排序 / 活动判定等纯数据逻辑可按现有 `tests/datamodel/*.test.ts` 的 Deno 测试方式验证，
  时间边界使用可控时钟，避免真实等待几分钟。
- [x] 在 `src/frame/desktop` 执行与本改动相关的检查：

```bash
pnpm run check
pnpm run build
pnpm exec playwright test tests/e2e/pages/messagehub.spec.ts --project=chromium
```

- [x] 若修改 Agent 主页，补跑对应 `users-agents.spec.ts`；对变更的 TS / TSX 文件运行 ESLint。
  Playwright 使用现有 config 的本地 dev server。不要把原型测试通过报告成真实 tunnel / 后端删除已经验证。
- [x] 保留桌面 / 移动端 Session 行、创建表单、归档 / 删除对话框、SessionDetails、Action 展示及 Agent 观察的截图证据。
- [x] 更新 `UI_DATAMODEL.md` 的时间口径、Session 生命周期 / 详情 / 过滤状态，更新
  `MessageHub_Current_UI_Model_Data.md` 为实际落地字段；后端依赖仍明确标为未接入。
- [x] 最终交付说明列出已完成 TODO、真实运行的检查、截图位置与未实现的后端能力，不以“按钮已出现”代替流程完成。

## 3. 后端集成边界（2026-09-07 集成，已执行）

2026-09-07 Review 时的四条边界及处理结果：

- `list_session_index` 只按 `MAX(updated_at_ms)` 聚合：已补 `order_by: activity`（`chat` / `group_msg` / `deliver` 记录的最大 `sort_key`）
  与同口径游标 `(last_activity_ms, session_id)`，服务端排序跨页一致；旧 `updated` 排序保留为默认值。
- 空会话登记、生命周期权威、Action Log 可靠发布、tunnel 能力与代理授权：登记与生命周期已落地（`owner_sessions`），
  跨 owner 读取授权已落地；共享 / 成员状态权威、Action Log 发布、tunnel 能力声明与代发授权仍未实现。
- 会话级归档 / 彻底删除：`msg.archive_session` / `msg.restore_session` / `msg.delete_session` 以 owner 范围的生命周期与删除水位实现，
  不使用 `RecipientState.DELETED`，不清理其它 owner 引用或共享对象。
- Agent 观察仍只读：服务端按 verify-hub 用户 token 校验 viewer → owner，写动作只允许 owner 自己。

真实接入完成项（前端 `api/` 真实 store + msg-center 后端，验证方式见 [IMPLEMENTATION.md](IMPLEMENTATION.md)）：

- [x] 修正实体归属：`api/projection.ts` 按登记 peer → `group:` tag / group_msg 目标 → `dm:` → 原始 msg.from / msg.to（相对 owner）归属，
  多目标歧义与无证据进入“未归类会话”容器；同一 topic 收发方向切换不改变实体。Deno `messagehub-projection.test.ts` 覆盖。
- [x] 保留 owner / record_id / msg_id / direction / box_kind / sort_key / recipient_state / 完整 delivery（`ui_record`）；
  展示副本不回写；无消息对象渲染“消息内容不可用”占位；owner 加载失败保留错误态与重试。
- [x] 分离本地已读与回执：可见入站 UNREAD 记录调用 `msg.update_record_state(READ)` 后才减少本地未读并重读摘要；
  UI 不写 `msg.set_read_state`；观察 Agent 不触发任何写入（服务端亦拒绝）。
- [x] 保留 PostSendResult：`ok:false` 展示服务端 reason，提交结果未知时保留原对象与同一幂等键重试并对账，
  乐观项在结果后移除；`partial_failed` / 失败按目标展示 state / attempts / error / duplicate_risk，不整条重发。
- [x] 会话归档、恢复、删除水位与并发边界：后端 `owner_sessions` 表（schema v9）+ 三个 RPC；归档保留阅读状态与活动时间，
  新普通消息自动解除归档而事件 / 状态更新不会；删除水位以前的记录（含重放）对该 owner 不可见，其它 owner 不受影响。Rust 测试覆盖。
- [x] REQUEST_BOX 入口、来源与准入动作：`SessionSummary.request_count`（水位后完整计数）、实体列表“请求”过滤、会话横幅与逐条“请求”标记，
  准入动作复用 `contact.update_contact(access_level=friend)` / `contact.block_contact`，只反馈本次权限变更。
- [x] native 路由与群 action 权限：群会话按 `group.check_access(group.post_message)` 决定 Composer；native 路由由 `post_send` 校验并展示拒绝原因；
  读取、发送、生命周期、字段编辑分别判断，共享 / 成员字段编辑在真实模式始终关闭并说明原因。
- [x] 对象附件上传 / 下载 / 预览：上传经 NDM TUS + `put_object` 发布 FileObject；下载经新增 `GET /kapi/msg-center/objects/{obj_id}[/content]`
  （需会话 token）按 `obj_id` 访问，覆盖 `cyfs://`、无 uri_hint、非图片文件与单附件失败局部展示。
- [x] API reader：`msg-center:{viewer}:{owner}:{session}` 隔离、最新页 + 向上分页、按 record_id upsert / remove、
  轮询 + kevent 信号触发尾部对账；不同 owner 同名 session 的缓存、偏好、游标与迟到响应隔离（epoch）。
- [x] 运行态来源与到期：typing 30s、status_line 10min 内有效，成员固定为 owner（生产者为 owner 的 tunnel / Agent）；
  真实 `event + buckyos.action_log` 消息沿用原型 renderer 与过滤，前端不再本地伪造日志。
- [x] 按 UI_DATAMODEL §9.4 执行真实 RPC / 数据夹具验证：`test/test_msg_center/test_messagehub_sessions.ts`（Deno，真实 zone）、
  `tests/e2e/real/messagehub.real.spec.ts`（Playwright，真实 zone，1440 / 375）；逐行状态见 §9.4。

仍未实现的后端契约（不影响上述 UI 功能，UI 中以只读 / 原因说明呈现）：

- [ ] 批量已读边界、回执持久化与私聊回执契约（当前 receipts 仅内存，UI 逐条 `update_record_state`）。
- [ ] 接受联系人后 REQUEST_BOX 旧记录的迁移 / 处理状态契约；当前只反馈权限变更，请求记录保留在请求箱。
- [ ] 共享标题 / 说明、成员昵称的权威状态（revision、幂等 patch）与 GroupEvent → Action Log 持久发布。
- [ ] tunnel 能力声明（多会话 / 远端创建 / 出站能力）与 Agent 代发授权；tunnel 会话仍按默认只读 + 风险确认写入。
- [ ] 已加载尾页之外的旧记录投递 / 删除 / 重新归类变更游标；当前只在重新打开会话或刷新时对账。
- [ ] `ui_session` owner 范围批量读取；当前按会话按需读取，实体列表置顶只覆盖已加载偏好。

本轮实现范围限于 MessageHub 前端真实 store、msg-center 的会话登记 / 生命周期 / 授权 / 对象访问、i18n 和相关测试 / 说明；
不重做 EntityList 布局、长历史引擎、Agent Runtime 或消息域持久协议。
