# My Network 产品升级 TODO：DID 网络的找人 / 加好友 / 找群公共 UI

> 创建日期：2026-10-17
> 版本：v0.3（2026-10-17 第二轮 review：后台公共能力放 msg-center，公共 UI 组件放 `src/frame/desktop/src/components/`，其余按建议执行）。v0.2 已定：D2 好友请求 = 向对方 Zone 投递特殊 MsgObject（§3.4）；D5 用 tags 分组，应用之间约定公共 tag（§3.5）。
> 状态：2026-10-17 已实施 M0–M6（提交 f5c609c88），实施记录见 §8；2026-10-18 在 `start.py --all` 新装的 DV 上验收通过（§8.5），验收中的修正未提交。
> 依据：[My Network PRD](../product/my_networks/My_Network_PRD.md)（2026-08）、[My Network 相关协议设计](<../doc/message_hub/My Network相关协议设计.md>)、[Contact Mgr](<../doc/message_hub/Contact Mgr.md>)、[Message Center](<../doc/message_hub/Message Center.md>) §2.2 / §4.5、[HomeStation 架构设计 v0.7](<../doc/homestation/BuckyOS HomeStation 架构设计.md>)、[HomeStation 协议与实现](<../doc/homestation/HomeStation 协议与实现.md>)、MessageHub [UI_DATAMODEL.md](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)，以及 2026-10-17 对三端代码和后端的核对。
> 第 2 节“现状”只以源码为准；PRD 和协议文档里写了但没实现的能力，都单独标明。

## 1. 目标与范围

My Network 自 8 月从 Users & Agents 拆出后基本没更新：添加联系人写死一个 “Nora”，Join Group 是 “coming soon”，Collections 是 mock。同期 MessageHub 和 HomeStation 各自长出了一部分关系能力（联系人多选、邀请链接入群、接受/屏蔽陌生人、关注某个 DID），但都只认“已经认识的人”。三个应用都没有在 DID 网络里找人、加好友、找群的入口。

本轮目标：

1. **公共关系能力放在 msg-center**（D1）：查找、关系状态、好友请求、分组 tag 都由 msg-center 的联系人模块提供。所有应用经 websdk / buckyos-api 的 msg-center 客户端调用，前端不另建逻辑层。
2. **好友请求**（D2）：“加好友”由 msg-center 代用户构造一个特殊 MsgObject，经现有投递路径 post 到对方 Zone。要做到跨 Zone 可用，需同时补齐原生投递的三个缺口（§3.4）。
3. **公共关系 UI**：找人、加好友、找群/入群、选人、关系操作。这些组件只是 msg-center 联系人能力的视图，放在 Desktop 公共组件目录 `src/frame/desktop/src/components/network/`，My Network、MessageHub、HomeStation 都直接调用。
4. **My Network 本体升级**：按 PRD §5–§11 接真实数据，覆盖 Contacts / Friends / Requests / Joined Groups / Collections，删除语义正确。Collections 就是 tag（D5）。
5. **接入两端**：MessageHub、HomeStation 换用公共 UI 并删掉重复实现；HomeStation 的关注写成公共 tag，在 My Network 里也能看到。

不做：

- 全网用户/群目录、全局索引（HomeStation 架构 L113 明确非目标；协议设计 §12.3 暂不做公开好友列表）。本轮“搜索”的含义见 §3.2。
- 手机号/邮箱发现 DID、Agent 生成 View Collection、共同好友推荐（PRD P3）。
- Self-hosted Group 的托管管理（属 Users & Agents / MessageHub GroupPanel）。
- 第三方 App 经 Sys Dlg 调用：M0 只预留参数 schema，实现见 M7。
- 旧数据兼容：My Network 现有 mock 类型、`relation` 字段直接替换；`Contact.groups` 按 D11 删除；`AccessGroupLevel::Friend` 按 D7 改名。

## 2. 现状（2026-10-17 核对）

### 2.1 My Network（`src/frame/desktop/src/app/my-network/`）

| 项 | 现状 |
| --- | --- |
| 添加联系人 | [MyNetworkAppPanel.tsx:74-94](../src/frame/desktop/src/app/my-network/MyNetworkAppPanel.tsx) `addContact` 在内存里插入写死的 `Nora / did:bns:nora`，不调后端 |
| Join Group | 同文件 :144 弹 “Join Group is coming soon.” |
| 数据 | [datamodel/api.ts](../src/frame/desktop/src/app/my-network/datamodel/api.ts) 只调 `contact.list_contacts`（limit 100）；`entityGroups` 恒为空；Collections 来自 users-agents mock seed，增删改只在内存 |
| 关系语义 | `toContactEntity` 把 `is_verified` 映射成 `relation: 'mutual'`。这与协议设计 §7.1 冲突：`AccessGroupLevel::Friend` 只是投递策略，`is_verified` 也不代表双向好友 |
| 删除 | 详情页 Remove 只改内存（[ContactDetailPage.tsx:25](../src/frame/desktop/src/app/users-agents/components/detail/ContactDetailPage.tsx) → `store.removeContact`） |
| 代码归属 | 联系人详情、Collection 列表/卡片、mock seed 仍在 `app/users-agents/` 下，My Network 跨目录引用；Users & Agents 侧栏已不再展示联系人 |
| 展示 | 头像/名称部分走了 presentation（`usableImageUrl`、`didDisplayName`），未用 `ObjectAvatar` / `useDidPresentation`（[desktop-object-presentation-todo.md](desktop-object-presentation-todo.md) L97 记为未迁） |
| 测试 | 只有 `tests/e2e/pages/users-agents.spec.ts:78` 一个打开面板的用例 |

### 2.2 MessageHub（`src/frame/desktop/src/app/messagehub/`）

| 能力 | 现状 |
| --- | --- |
| 找人 / 输入 DID | 没有。“新建会话”（`SessionDialogs.tsx:36-60` `CreateSessionForm`）只能从已有实体里 `<select>`；列表搜索 `EntityList.tsx:77-85` 只过滤本地 |
| 打开陌生 DID 的会话 | 不行。`MessageHubView.tsx:154` `resolvedInitialEntityId` 要求 `findEntity` 命中，launch 带陌生 DID 什么也打不开 |
| 加好友 | 只能对已进 REQUEST_BOX 的发送者“接受 / 屏蔽”：`ConversationView.tsx:121-127` 横幅、`EntityDetails.tsx:304-331` `AdmissionSection`。store `admission()` / `setAdmission()`（`api/store.ts:468-490`）调 `contact.update_contact{access_level:'friend'}` / `contact.block_contact`。这里的“接受”只是允许对方的消息进 INBOX，不是好友关系。mock 下 `admission()` 返回 null，界面上看不到 |
| 选人 | `GroupDialogs.tsx:12-49` `MemberPicker`，候选来自 `groupModel.ts:43-64` `groupMemberCandidates`（已有 person/agent 实体，排除 msgtunnel 和屏蔽）。用于建群、邀请、会话成员与 Guest |
| 入群 | `GroupDialogs.tsx:51-74` `JoinByLinkForm`，藏在建群对话框底部。`groupModel.ts:132-137` `parseInviteLink` 接受 `<group_did>?invite=<token>` 或裸 DID，再调 `group.request_join` |
| 找群 | 没有 |
| 特殊消息卡片 | `groupModel.ts:83-103` `parseGroupNotice` 按 intent `buckyos.group_invitation` 渲染邀请卡和按钮，好友请求卡可照做 |

### 2.3 HomeStation（前端 `src/frame/desktop/src/app/homestation/`，后端 `src/frame/homestation/`）

| 能力 | 现状 |
| --- | --- |
| 关注 | `source/SourceManager.tsx:166-201` `AddSource` 有三种模式（Follow “A name or DID” / Paste URL / Describe），调用 `sources.resolve` → `sources.follow`。关注列表存在 HomeStation 自己的 `sources` 表，`basis` 为 `active` / `friend` |
| 找人 | `sources.resolve{kind:'follow'}`（`sources.rs:199-225`）只在本人 Message Center 联系人里按 DID 精确、名称子串匹配（≤4 条），否则 `DID::from_friendly_name`。URL 模式 `homestation_at` 把 `/homestation/<user>`、`/home/<user>/…` 解析成 DID |
| 好友 / 分组 | 不自建，读 Message Center：friend = `access_level == Friend`，blocked = `Block`，分组 = **`Contact.groups`**（`contacts.rs:38-72`，缓存 30s；`audience.rs`、`publish.rs:453`、`selection.rs:156`、`api.rs:357-363`、`stream.rs:219` 都读 groups）。好友默认关注由 `sync_friends` 每小时轮询（`lib.rs:218-231`），没有变更事件 |
| 写联系人 | HomeStation 只有 RBAC `obj://msg-center/owners/*,read`，不能写用户的联系人 |
| 受众 | `publish/AudiencePicker.tsx`：分组下拉的名称就是 group id；“指定 DID”是自由文本框（:63-73），没有选人 |
| 访客页关注 | `HomeStationPortalRoute.tsx:162-170` 只有同 Zone 登录者能关注；他 Zone 访客只见 “Sign in to follow” |
| 群 | 无。PRD 有“加入 Topic/Group/Channel”，[原型 TODO](homestation-prototype-v0.6-todo.md) L262 暂缓 |
| 展示 | 自有 `Avatar`（`ui/primitives.tsx`），未用公共 presentation |

### 2.4 msg-center 与相关后端

| 能力 | 现状 |
| --- | --- |
| 联系人 | ContactMgr（`src/frame/msg_center/src/contact_mgr.rs`），按 owner 分库，`contact_mgr_owner` 传本人 DID。RPC 有 `list_contacts`（keyword 对 did/name/note/groups/tags 子串匹配）、`get_contact`、`update_contact`（带 `access_level` 时不存在即创建）、`block_contact`、`import_contacts`、`merge_contacts`、`grant_temporary_access`、`check_access_permission`、`resolve_target`、`resolve_canonical_did` 等，方法名见 `buckyos-api/src/msg_center_client.rs:416-432`；websdk 封装在 `msg_center_client.ts:813-1010` |
| 联系人缺口 | 没有删除联系人、显式 unblock（只能 `update_contact` 改级别），也没有好友请求、接受、拒绝；Contact 没有“好友关系”字段，只有 `access_level` |
| 分组字段 | Contact 同时有 `groups` 和 `tags`。`upsert_zone_contacts`（`contact_mgr.rs:1279`）把 `zone_user` / `zone_agent` 同时写进两者；HomeStation 读 groups，My Network 两者合并显示 |
| 准入 | Block → DROP；Friend → INBOX；Temporary（有效期内）→ INBOX；Stranger 或无记录 → REQUEST_BOX（`contact_mgr.rs:493-659`）。REQUEST_BOX 只是陌生人消息箱，没有结构化的请求记录 |
| 同 Zone | msg-center 把 Zone 用户写进每个用户的联系人：`Shared`、`is_verified`、tag `zone_user`、Friend 级别；Agent 与 owner 互为好友（`main.rs:850` `sync_zone_agent_contacts`）。control-panel `user.list` 任何登录用户可调（`control_panel/src/user_mgr.rs:510`） |
| 写权限 | `owner_session.rs:552` `authorize_contact_scope`：User principal 只能写自己的库；服务 principal 只看 RBAC `owners/*` 的 write，粒度是整个库 |
| 特殊 MsgObject 先例 | 群邀请：`group_service.rs` `notify_group_member` 构造 `MsgObject{kind: Operation, machine.intent: "buckyos.group_invitation", data:{group_did, action, data}}`，`tx.object` 存对象；本地收件人按 `peek_access_permission` 写 INBOX / REQUEST_BOX / 丢弃，远端收件人写 `DeliveryRecord` 交给原生 executor |
| 跨 Zone 送达 | 发送：远端收件人要手工配 `cyfs_dispatch.outgoing[<did>]`，否则 `native-route-not-configured` 且不重试（`message_hub.rs:210-224`）。接收：`cyfs_dispatch.rs:215` `serve` 只接受手工配置的 `accepted_paths`，且必须带本 Zone 信任的 Bearer token（`verify_trusted_session_token`），陌生 Zone 进不来 |
| 签名 | 接收方已支持 JWT 签名的 MsgObject（`cyfs_dispatch.rs:450` `verify_signed_message`，用 name_client 解析 `from` 的密钥），但签名者必须是 `from` 本人的密钥，“授权给设备或 Agent 的密钥尚未支持”（Message Center.md §4.5）。msg-center 进程没有用户私钥 |
| 浏览器侧 DID 查询 | `presentation/store.ts` 先试 `buckyos.resolve_did()`（只在钱包环境可用），否则请求 `/1.0/identifiers/<did>`，由 system-config ZoneDidResolver 回答，**Zone 外的 DID 一律 503 NoOpinion**（`zone_did_resolver.rs:670-680`）。msg-center 已引入 name_client（`group_publication.rs:43`），可以在服务端解析。`contact.resolve_did` 是“平台账号 → `did:msgtunnel`”，不是 DID 文档查询 |
| 名称搜索 / 目录 | 任何地方都没有。websdk BNS client 只能精确查名，SN 只有用户名可用性检查 |
| 群 | `group.*` 只对本 Zone 托管的群有效。`group.get_doc` 公开；`group.request_join{group_did, invite?}` 按 `join_policy` 处理（`invite_only` → `invite-required`）。他 Zone 群须手工配 `cyfs_dispatch.joined_groups`（[messagehub-group-v2-todo.md](messagehub-group-v2-todo.md) §4.5） |
| 好友协议 | `bucky.social.friend.*` 只在协议设计文档里，代码中没有 |
| 可借鉴 | HomeStation 已实现 DID → Zone 查询（owner document 默认 Zone → `GET https://<zone>/home/?did=`，缓存 10 分钟）和 Zone 托管签名（OOD 设备密钥替 Zone 用户签名，验证方接受“设备所在 Zone = 发表者默认 Zone”，已知风险），见 HomeStation 协议与实现 L45、L100、L366 |

### 2.5 结论

- 三端都不能“认识新的人或群”，原因有两层：UI 没有公共入口；后端在浏览器侧查不了 Zone 外 DID，也没有目录。
- 好友请求所需的部件 msg-center 基本都有：特殊 MsgObject（群邀请）、本地按准入分箱、远端 DeliveryRecord 与重试、JWT 签名校验。缺的是好友语义本身，以及跨 Zone 的自动路由、陌生人入站和代签名。
- 跨 Zone 三个缺口不补，好友请求只在本 Zone 内有效，而本 Zone 用户本来就是自动好友，所以它们必须和好友请求一起做（§3.4、M3）。补齐后普通的跨 Zone 私聊也随之可用。

## 3. 设计

### 3.1 分层（D1：后台公共能力放 msg-center，公共 UI 放 Desktop components）

| 层 | 位置 | 内容 |
| --- | --- | --- |
| 公共能力 | msg-center 联系人模块（`contact_mgr.rs` 与 `contact.*` RPC） | 查找（`contact.lookup`）、关系状态（`friend_state` + `access_level` + tags）、好友请求、tag 约定与写权限、联系人变更事件。找群复用现有 `group.*` |
| 客户端 | websdk `msg_center_client.ts`（Desktop、第三方 App）；buckyos-api `msg_center_client.rs`（HomeStation 等服务） | 只做类型与调用 |
| UI | `src/frame/desktop/src/components/network/`（组织方式同 `ContentPreview.tsx` + `preview/`） | FindPeople / PeoplePicker / AddFriendDialog / JoinGroupDialog / RelationBadge / RelationActions，以及对话框入参与返回值的 zod schema。只做展示与交互，规则（输入解析、关系判断、删除约束）都由 msg-center 返回；同目录放一个薄缓存，只缓存 msg-center 的返回结果并订阅联系人变更事件，不做规则判断 |
| 跨应用 | `app/my-network/launch.ts` 的 `openMyNetwork(launch)`（仿 `openUsersAgents`） | `{ view: 'requests' }`、`{ entity: did }`、`{ action: 'add-friend', input? }`、`{ action: 'join-group', input? }` |

- 宿主（MessageHub、HomeStation、My Network 本身）在同一个 Desktop bundle 里，直接 import 组件，用 `useWindowDialog` 在自己窗口内弹出，结果以 Promise 返回。HomeStation 门户等不在桌面窗口内的路由，组件也要能独立渲染。
- 应用特有的动作不进公共组件：FindPeople 结果卡的 `actions` 由宿主传入（MessageHub：发消息、拉进群；HomeStation：关注；My Network：加好友、加入 Collection）。
- 名称和头像一律走 `presentation/`（`ObjectAvatar`、`useDidPresentation`）。

### 3.2 “在 DID 网络里搜索”= `contact.lookup`

DID 网络没有全局目录，本轮也不建（D4）。`contact.lookup { input, contact_mgr_owner, kinds?, limit? }` 接受原始输入，由 msg-center 统一解析并合并结果，前端只做分段展示：

| 段 | 来源 | 匹配方式 |
| --- | --- | --- |
| 我的关系 | 本人联系人（含好友、屏蔽、各类 tag）、已加入的群（`group.list_by_member`） | 名称 / DID / 备注 / tag 子串 |
| 本 Zone | Zone 用户、本人的 Agent | 名称 / DID 子串 |
| 解析结果 | 输入是 DID、BNS 名、friendly name、群邀请链接时，服务端用 name_client 解析出唯一对象（OwnerDocument / DID 文档 / GroupDoc），取公开展示字段 | 精确 |
| 宿主扩展 | 宿主在 FindPeople 上注册的解析器，例如 HomeStation 主页 URL（`sources.resolve{kind:'url'}`） | 由宿主定义 |
| 网络目录（预留） | HomeStation 收录者、SN、公开群目录等 | 本轮不实现 |

- 每个候选返回：`did`、`kind`（user / agent / group / zone / msgtunnel / unknown）、展示字段、所属 Zone、可达性（本 Zone / 可自动路由 / 不可达及原因）、当前关系（§3.3），群候选附带邀请 token 与 `join_policy`。
- 失败原因区分 `invalid-input` / `not-found` / `unreachable` / `forbidden`，不能统一显示成“找不到”。
- 解析缓存与超时参照 HomeStation：正反结果缓存 10 分钟，网络失败不缓存。
- MessageHub 的 `parseInviteLink`、HomeStation 后端 `sources.resolve{kind:'follow'}` 的名称匹配都由它取代。

### 3.3 关系模型

Contact 上的三样东西各管一件事：

| 字段 | 含义 | 谁改 |
| --- | --- | --- |
| `friend_state`（新增） | `none` / `outgoing_pending` / `incoming_pending` / `friend`，即好友关系 | 好友请求流程（§3.4）；同 Zone 用户、Agent 与 owner 由 msg-center 同步直接置为 `friend` |
| `access_level`（已有） | 投递策略：对方消息进 INBOX、REQUEST_BOX 还是丢弃 | 成为好友时若不是 Block 则设为 `Allow`（现名 `Friend`，按 D7 改名）；解除好友后降为 Stranger；用户也可单独调整（例如允许某个陌生人直接发消息），不影响好友关系 |
| `tags`（已有） | 分组：系统 tag、应用 tag（如 HomeStation 的“我关注的人”）、用户 tag | 见 §3.5 |

UI 显示规则：

| UI 状态 | 来源 | 说明 |
| --- | --- | --- |
| 好友 | `friend_state = friend` | 唯一可以显示为双向的情况 |
| 等待对方确认 | `outgoing_pending` | |
| 请求加我 | `incoming_pending` | 进 Requests |
| 我关注的（单向） | tag `homestation.following` | 可与“好友”并存 |
| 联系人 | 有记录，以上都不是 | |
| 已屏蔽 | `access_level = Block` | 覆盖其他显示 |
| 临时允许 | `access_level = Temporary` 且 grant 有效 | 显示到期时间 |
| 消息请求 | 没有好友请求，但 REQUEST_BOX 里有其普通消息 | Requests 里单列，与好友请求分开 |

- 删除 My Network `ContactEntity.relation: 'one-way' | 'mutual'` 以及 `is_verified → mutual` 的映射。
- MessageHub 现在的“接受”只改 `access_level`，迁移后分成两个动作：“加为好友”（发好友请求）和“允许消息”（只改 `access_level`）。
- 群关系 `GroupRef`：`group.list_by_member` 的结果，加上本人状态（member / admin / owner / pending_admin_approval / invited）、是否由本 Zone 托管、能否在 MessageHub 发言。

### 3.4 好友请求（D2）

原则沿用协议设计 §1：不新增通道。“加好友”由 msg-center 代用户构造一个特殊 MsgObject，经现有投递路径 post 到对方 Zone；双方各自维护本地状态，不要求强一致（协议设计 §6.3）。实现照搬群邀请（§2.4“特殊 MsgObject 先例”）。

**消息**：命名改为与现有 intent 一致的风格，取代协议设计 §5 的 `bucky.social.friend.*`。

```text
MsgObject {
  kind: Operation,
  from: 本人 DID, to: [对方 DID],
  content: { content: 可读文本（附言或“请求加你为好友”）,
             machine: { intent: "buckyos.friend", data: { action, request_id, ... } } }
}
```

| action | 方向 | data | 接收方处理 |
| --- | --- | --- | --- |
| `request` | A → B | `request_id`、`message?`、`profile_hint{name, avatar}`、`expires_at_ms` | 建请求记录，B 端 `incoming_pending`，同时写一条 REQUEST_BOX 记录让 MessageHub 可见。B 已屏蔽 A → 丢弃且不留记录。B 已把 A 当好友，或 B 也有发给 A 的待处理请求（交叉请求）→ 直接成为好友并回 `accept` |
| `accept` | B → A | `request_id` | A 有对应的 `outgoing_pending` → `friend`；没有（已取消或过期）→ 忽略 |
| `reject` | B → A | `request_id` | 默认不发（协议设计 §5.2：不要求回复），B 明确选“拒绝并告知”时才发 |
| `cancel` | A → B | `request_id` | B 的请求记录 → `cancelled` |
| `remove` | 任一方 | — | 尽力通知：对方 `friend` → `none`，保留联系人记录（协议设计 §13 Q4 的建议答案） |

`request_id` 是幂等键，重复投递和 Gateway 重放都按 `(peer, request_id)` 去重。

**RPC**：浏览器没有用户私钥，消息必须由服务端构造，所以走专用 RPC，不让前端拼 MsgObject 调 `msg.send`。

- `contact.send_friend_request { did, message?, contact_mgr_owner }` → `{ request_id, state, delivery }`
- `contact.respond_friend_request { request_id, decision: accept | reject | ignore }`
- `contact.cancel_friend_request { request_id }`
- `contact.remove_friend { did, notify? }`
- `contact.list_friend_requests { direction?, state?, limit, offset }`

**本地存储**（ContactMgr，按 owner 分库）：

- Contact 新增 `friend_state`（§3.3）。
- 新表 `friend_requests(owner_key, request_id, peer_did, direction, state, message, msg_id, created_at_ms, expires_at_ms, updated_at_ms)`，`state` 取值 pending / accepted / rejected / cancelled / expired / ignored。
- 默认有效期 7 天，由现有的定时清扫顺带处理过期。
- 被拒绝或忽略后，同一对方的再次请求设冷却时间。这是防骚扰的最低限度，其余按“先能力后安全”记为已知风险。

**接收入口**：本地 dispatch 和 `cyfs_dispatch::serve` → `dispatch_to_receiver` 两条路，在写入收件人邮箱前都要识别 intent `buckyos.friend`。

- A 发出请求后，B 发回的 `buckyos.friend` 消息必须越过 REQUEST_BOX 直接处理，否则 `accept` 会被当成陌生人消息。
- `accept` / `cancel` / `remove` 是控制消息，只更新状态并发联系人变更事件；是否在会话里留痕在 M0 定。
- MessageHub 按 intent 渲染好友请求卡（接受 / 忽略 / 屏蔽），做法同 `parseGroupNotice`。

**跨 Zone 的三个缺口**：放在 msg-center 原生投递层实现，不写成好友请求专用逻辑。

| 缺口 | 现状 | 建议方案 |
| --- | --- | --- |
| G1 路由 | 发送方要手工配 `cyfs_dispatch.outgoing[<did>]`；接收方要手工配 `accepted_paths` | 发送方：DID → name_client `resolve_owner_document` → 默认 Zone → 约定的接收路径（建议 `/<user_did>/inbox`，与群的 `/<group_did>/inbox` 对齐），upstream 为对方 Zone 网关；结果缓存，路由变化按现有 `native-route-changed` 处理。接收方：本 Zone 每个用户的约定路径自动接受。`outgoing` / `accepted_paths` 只保留为显式覆盖 |
| G2 入站认证 | `serve` 必须带本 Zone 信任的 Bearer token | 没有 Bearer 时，接受 JWT 签名的 MsgObject，以签名验出的 `from` 作为 principal，随后照常按 ContactMgr 准入（陌生人 → REQUEST_BOX，屏蔽 → DROP）。所有签名消息都放行（D9），跨 Zone 私聊一并可用；垃圾消息记为已知风险，先靠冷却与大小限制 |
| G3 签名 | 签名者必须是 `from` 本人的密钥；msg-center 没有用户私钥 | Zone 托管签名：发送方 Zone 的 OOD 设备密钥代用户签名；验证方接受“签名设备所在 Zone = `from` 的默认 Zone”，与 HomeStation 同一规则（已知风险：Zone 可以替用户签名）。用户自有密钥（`security/<user>/key`）以后再换（D10） |

对方 Zone 离线时沿用现有 Gateway 缓存与 DeliveryRecord 重试，UI 显示“等待送达”。

### 3.5 Tag 约定（D5）

- 分组一律用 `Contact.tags`，Collection 就是 tag。应用之间只要约定公共 tag 就能对上。例如 HomeStation 维护 `homestation.following`（显示为“我关注的人”），My Network 通过这个 tag 直接显示出这组人，即单向关系。
- `Contact.groups` 与 tags 现在是重复的（`zone_user` / `zone_agent` 两边都写），HomeStation 却只读 groups。合并到 tags 并删除 groups 字段（D11），HomeStation 读取处一起改。
- tag 分三类：

| 类别 | 例 | 谁写 | My Network 中的样子 |
| --- | --- | --- | --- |
| 系统 | `zone_user`、`zone_agent` | msg-center | 只读视图（“本 Zone 用户”“我的 Agent”） |
| 应用 | `homestation.following`（“我关注的人”） | 拥有该 tag 的应用 | 视图，成员只读，提供“在 HomeStation 中管理”（D12） |
| 用户 | “家人”“客户” | 用户 | Static Collection，可增删成员、改名、删除 |

- well-known tag 表同时放在 buckyos-api（Rust 常量）和 websdk（TS 常量）。每项写明 key、所属应用、用户能否编辑；显示名走 Desktop i18n。用户新建的 tag 不得与 well-known key 或保留前缀（`<app>.`）冲突。
- 写权限：新增 `contact.update_tags { did, add, remove }`。User principal 只能改自己库里的用户 tag；服务 principal（如 homestation）只能改自己命名空间的 tag，联系人不存在时以 stranger 创建。现在服务写联系人只看 RBAC 对整个库的 write，HomeStation 也只有 read，所以要改。
- 用户 tag 的显示名就是 tag 文本；Collection 的描述是否需要另存，M0 定。

**HomeStation 怎么用**：

- 关注成功（`basis` 含 `active`）时给对方加 `homestation.following`，取消关注时移除。因好友关系而产生的关注不打这个 tag，它们已经在 Friends 视图里。
- 好友默认关注改为读 `friend_state = friend`，不再读 `access_level == Friend`；并订阅联系人变更事件，取代每小时一次的 `sync_friends` 轮询。
- 受众“分组”和“不看”分组改读用户 tag，下拉显示 tag 名。

### 3.6 UI 流程

**加好友 / 加联系人**（PRD §6.2）

1. 用户输入 DID、名称或链接，或选择一条 `contact.lookup` 结果，系统显示资料卡：名称、头像、DID（可复制）、类型、所属 Zone、当前关系、可达性。
2. 用户选择“加好友”（`send_friend_request`，可填附言）或“仅保存为联系人”，可同时加用户 tag。
3. 结果如实显示：同 Zone 用户 → 已是好友；可达 → “等待对方确认”；对方 Zone 不可达 → 说明原因，并提供“复制我的分享链接”（协议设计 §13 Q7 的降级方案）。

**处理请求**（PRD §6.4）

Requests 视图有四类：好友请求（`list_friend_requests`）、消息请求（REQUEST_BOX 里的普通消息，按发送者聚合）、群邀请（`buckyos.group_invitation`）、待我审批的入群申请。好友请求的动作是接受 / 忽略 / 拒绝并告知 / 屏蔽；消息请求的动作是加为好友 / 允许消息 / 屏蔽。

**找群 / 入群**（PRD §6.3）

用户输入群 DID 或邀请链接，系统用 `group.get_doc` 显示预览：名称、头像、owner/host、成员规模（受 `member_list_visibility` 限制）、`join_policy`。之后调用 `group.request_join`，显示 active / pending_admin_approval / invite-required 三种结果。他 Zone 的群在一期如实提示“暂不支持加入其它 Zone 的群”（等 messagehub-group-v2-todo §4.5），不能显示成“申请已发送”。

**选人**（替代 MessageHub `MemberPicker` 与 HomeStation 的 DID 文本框）

PeoplePicker 默认列出我的联系人与本 Zone 用户，可按 tag 筛选；搜索时调 `contact.lookup`，可以当场把新 DID 保存为联系人后选中。它支持 `filter`（kind、排除屏蔽、排除已在群内并显示原因）、`limit`、单选/多选，返回选中的 DID 及展示信息。

**删除语义**（PRD §2.3、§11.3）

Remove from Collection（去掉用户 tag）/ Delete Contact / Unfriend（`remove_friend`）/ Block 的确认强度各不相同，写死在组件内，不由宿主决定。系统建立的关系（`zone_user`、Agent 与 owner）由 msg-center 拒绝删除并给出原因。

## 4. 决定（2026-10-17 已全部确定）

| # | 决定 |
| --- | --- |
| D1 | 后台公共能力放 msg-center 联系人模块，Desktop 不另建前端逻辑层；公共 UI 组件放 `src/frame/desktop/src/components/network/`（§3.1） |
| D2 | 加好友 = msg-center 向对方 Zone post 一个特殊 MsgObject，设计见 §3.4 |
| D3 | 查询 RPC `contact.lookup` 在 msg-center（随 D1） |
| D4 | 本轮不做网络目录，只在 FindPeople 留扩展点 |
| D5 | 用 tags 分组，应用之间约定公共 tag；HomeStation 的“我关注的人”在 My Network 中可见，表示单向关系（§3.5） |
| D6 | Joined Groups 包含本 Zone 托管的群，并标注“本 Zone 托管”；管理操作跳 MessageHub GroupPanel |
| D7 | `AccessGroupLevel::Friend` 改名为 `Allow`，随 `friend_state` 一起改，避免“Friend”同时表示投递策略和好友关系（协议设计 §13 Q6）；涉及 MessageHub、HomeStation `contacts.rs`、websdk 与文档，不做兼容 |
| D8 | My Network 二期只读展示 HomeStation 的被关注（Followers）；“我关注的”已由 D5 的 tag 解决 |
| D9 | 无 Bearer 的签名入站（G2）放行所有签名消息，统一进准入（陌生人 → REQUEST_BOX），跨 Zone 私聊一并可用；垃圾消息记为已知风险，先靠冷却和大小限制 |
| D10 | 跨 Zone 先用 Zone 托管签名（G3），与 HomeStation 一致；同时更新 Message Center.md §4.5 中“设备密钥尚未支持”的描述 |
| D11 | 删除 `Contact.groups`，统一为 tags；涉及 buckyos-api 类型、`ContactPatch`、`ContactQuery`、import、websdk、HomeStation、My Network |
| D12 | 应用 tag（如 `homestation.following`）在 My Network 中一期只读，提供“在 HomeStation 中管理”；HomeStation 的 `sources` 表仍是关注的真相源，tag 是它写出的投影。以后若要在 My Network 里直接编辑，HomeStation 须订阅 tag 变化做对账 |

## 5. 实施计划

M1–M3（后端）与 M4（前端组件，mock-first）可以并行；M5、M6 依赖 M4，真实接入依赖 M1–M3。

### M0：契约与文档（评审通过后再写代码）

- [x] D1–D12 全部确定（2026-10-17）。
- [x] 改写 [My Network 相关协议设计](<../doc/message_hub/My Network相关协议设计.md>) §5–§7：intent 命名、RPC、请求记录、`friend_state` 与 `access_level` 分离；§13 写入结论。
- [x] 更新 [Contact Mgr.md](<../doc/message_hub/Contact Mgr.md>)：`friend_state`、`friend_requests`、tag 约定与写权限、`lookup`、`remove_contact`、unblock。
- [x] 更新 [Message Center.md](<../doc/message_hub/Message Center.md>) §2.2 / §4.5：自动路由与约定接收路径（G1）、签名入站（G2）、Zone 托管签名（G3）。
- [x] 新建 `src/frame/desktop/src/components/network/UI_DATAMODEL.md`：§3.3 的显示规则、GroupRef、Requests 四类条目、错误码到文案的映射；My Network 独有的视图另写在 `app/my-network/` 下。
- [x] `app/my-network/launch.ts` 的 `openMyNetwork`，以及 `components/network/` 中四个对话框的 zod 入参与返回值。返回值不含 token 等敏感数据，写法参照 [BuckyOS Sys Dlg.md](<../product/BuckyOS Sys Dlg.md>)，为 M7 留口。
- [x] 更新 [My_Network_PRD.md](../product/my_networks/My_Network_PRD.md)：§4.3 / §6.2 / §6.4 写入好友请求与单向关注的语义；§4.5 写 Collection = tag；§15 写“后台公共能力在 msg-center，公共 UI 在 Desktop components，My Network 是关系管理入口”；§19 Q2 / Q3 写入结论。

### M1：msg-center 联系人能力

- [x] `contact.lookup`（§3.2）。
- [x] `contact.remove_contact { did }`：删除记录与别名，不删历史消息；系统建立的关系拒绝删除。
- [x] unblock：沿用 `update_contact{access_level}`，或新增 `contact.unblock_contact`，二选一。
- [x] Contact 新增 `friend_state`；`upsert_zone_contacts` 与 `sync_zone_agent_contacts` 把同 Zone 用户、Agent 与 owner 置为 `friend`。同时把 `AccessGroupLevel::Friend` 改名为 `Allow`（D7），同步 MessageHub、HomeStation `contacts.rs`、websdk 与文档。
- [x] tags：按 D11 合并 groups；well-known tag 表；`contact.update_tags` 与按命名空间的写权限（`owner_session.rs` `authorize_contact_scope`）；RBAC 规则同步到 `rbac_config.rs`。
- [x] 联系人变更按 owner 发 kevent（好友状态、access_level、tags 变化都发）。
- [x] 同步 websdk `msg_center_client.ts` 与 buckyos-api `msg_center_client.rs`。
- [x] 按 AGENTS.md 检查 cmd_skills（身份/RBAC 类变更看 `buckyos_admin`），写明受影响与否的依据。
- [x] 验证：`cargo test -p msg_center -- --test-threads=1`、`cargo test -p buckyos-api -- --test-threads=1`。

### M2：好友请求（本 Zone 先通）

- [x] §3.4 的五个 RPC 与 `friend_requests` 表；构造 MsgObject 时复用 `notify_group_member` 的模式（存对象 → 本地写邮箱 / 远端写 DeliveryRecord），可以抽出公共的“发送 Operation 消息”函数。
- [x] 本地 dispatch 与 `dispatch_to_receiver` 两条入口识别 `buckyos.friend`：建记录、交叉请求直接成为好友、屏蔽丢弃、已发请求方的回复越过 REQUEST_BOX。
- [x] 过期清扫、再次请求冷却、按 `(peer, request_id)` 幂等。
- [x] 测试：同一个 msg-center 里两个没有同 Zone 好友种子的 owner 走完请求 → 接受 / 忽略 / 取消 / 交叉请求 / 解除 / 屏蔽后请求；重放同一请求不产生重复记录。

### M3：跨 Zone 原生送达（G1–G3）

- [x] G1：发送方自动路由（DID → 默认 Zone → 约定路径），带缓存；接收方为本 Zone 用户自动接受约定路径；`outgoing` / `accepted_paths` 降为显式覆盖。
- [x] G2：无 Bearer 时接受签名的 MsgObject，所有签名消息都放行并进准入（D9）；加同一发送方的冷却。
- [x] G3：发送方用 Zone 托管签名；`verify_signed_message` / `authorizes_signer` 接受“签名设备所在 Zone = `from` 的默认 Zone”（D10）。
- [x] 测试：两个 msg-center 实例模拟两个 Zone，跑通好友请求往返与普通私聊（`test_contact_network.rs::friend_request_and_chat_cross_two_zones`）。DV 只有一个 Zone，真实跨 Zone 未验证。
- [x] 回写 Message Center.md §4.5 的实现边界。

### M4：公共 UI 组件（mock-first）

- [x] 在 `src/frame/desktop/src/components/network/` 实现 FindPeople、PeoplePicker、AddFriendDialog、JoinGroupDialog、RelationBadge、RelationActions 和薄缓存，数据走 websdk msg-center 客户端；mock 放同目录 `mock/`，按 `isMockRuntime()` 切换（同 `components/preview/provider.ts`），按 webui-prototype skill 模拟延迟与五种状态（加载 / 空 / 错误 / 部分可达 / 成功）。
- [x] 窄屏用 sheet；i18n（en + zh-CN），well-known tag 显示名进字典；浅色和深色都截图核对。
- [x] 删除语义与确认强度按 §3.6。
- [x] 名称和头像只走 `presentation/`。

### M5：My Network 本体升级

- [x] 导航按 PRD §5.3：首页概况（各类数量、最近新增、Requests 数）、Contacts、Friends、Requests、Joined Groups、Collections。Collections 分三段：用户 tag（可编辑）、应用 tag（如“我关注的人”）、系统 tag。
- [x] Add Friend、Join Group 两个一级入口改用 M4 组件，删掉写死的 Nora 和 coming soon 对话框。
- [x] Requests 视图的四类条目（§3.6）。
- [x] Joined Groups：数据来自 `group.list_by_member`；详情页显示群资料、我的状态，提供“在 MessageHub 打开”和“退出”。
- [x] 把联系人详情、Collection 组件和 mock seed 从 `app/users-agents/` 移到 `app/my-network/`，Users & Agents 不再持有联系人代码。
- [x] 详情页的“发消息”跳 MessageHub；对方有主页时，“查看主页”跳 HomeStation。
- [x] 新建 `tests/e2e/pages/my-network.spec.ts`（mock），覆盖：
  - 加好友：本 Zone / 他 Zone / 不可达；处理好友请求与消息请求；
  - 入群：成功 / 待审批 / 需邀请 / 他 Zone 群；
  - 用户 tag 的 Collection 增删；“我关注的人”可见且只读；
  - 四种删除语义。

### M6：接入 MessageHub 与 HomeStation，删除重复实现

MessageHub：

- [x] `MemberPicker` 换成 PeoplePicker（建群、邀请、会话成员与 Guest），`groupMemberCandidates` 的过滤规则并入 PeoplePicker 的 `filter`。
- [x] `JoinByLinkForm` + `parseInviteLink` 换成 JoinGroupDialog，入口从建群对话框底部提到“+”菜单。
- [x] “新建会话”可以搜索或输入新 DID（FindPeople）。同时修 `resolvedInitialEntityId`：launch 带陌生 DID 时要能打开会话（用草稿实体），否则从 My Network 点“发消息”跳不过来。
- [x] 准入横幅与 `AdmissionSection` 改为“加为好友 / 允许消息 / 屏蔽”，调 RelationActions；mock 下也要能演示。
- [x] 渲染 `buckyos.friend` 好友请求卡，做法同 `parseGroupNotice`。
- [x] 更新 MessageHub UI_DATAMODEL（§4.6 准入部分引用 `components/network/UI_DATAMODEL.md`）；回归 `messagehub*.spec.ts`，尤其是 group、admission、deeplink。

HomeStation：

- [x] 前端 SourceManager 的 Follow 模式改用 FindPeople，HomeStation 注入“关注”动作与 URL 解析器；Describe 模式仍归 HomeStation。后端 `sources.resolve{kind:'follow'}` 只接受 DID，名称匹配交给 `contact.lookup`。
- [x] 关注 / 取消关注时调 `contact.update_tags` 维护 `homestation.following`（§3.5），RBAC 给 homestation 开对应权限。
- [x] 好友默认关注改读 `friend_state`，订阅联系人变更事件，去掉每小时轮询。
- [x] `groups` → tags：`contacts.rs`、`audience.rs`、`publish.rs`、`selection.rs`、`stream.rs`、`api.rs`；受众与“不看”的分组下拉显示 tag 名。
- [x] AudiencePicker 的“指定 DID”换成 PeoplePicker；FeedPreferences 中“不看”的人改用 PeoplePicker。
- [x] 作者头像迁到 `presentation/`（desktop-object-presentation-todo 的剩余项）。
- [x] 他 Zone 访客页的 “Sign in to follow” 本轮不改。
- [x] 回归 `homestation.spec.ts` 与 `cargo test -p homestation`。

### M7：后续

- [ ] 他 Zone 的群：messagehub-group-v2-todo §4.5 的自动接入落地后，JoinGroupDialog 去掉“暂不支持”分支（可复用 M3 的路由与签名）。
- [ ] 网络目录（如果 D4 改变）：HomeStation 收录者检索、公开群目录。
- [ ] 第三方 App：提供 `/sysdlg/people_picker`、`/sysdlg/add_friend`、`/sysdlg/join_group`，经 `AppFrameHost` 的 shell capability 调用，写入 BuckyOS Sys Dlg.md。
- [ ] 用户自有签名密钥取代 Zone 托管签名。
- [ ] PRD P3：手机号/邮箱发现 DID、Agent 生成 View Collection、Followers。

## 6. 验收

1. MessageHub、HomeStation 中不再有各自的选人、入群、加好友、DID 输入解析代码（grep `MemberPicker`、`parseInviteLink`、`didsPlaceholder` 无结果）；HomeStation 不再读 `Contact.groups`。
2. 在三个应用任意一处输入同一个 DID，看到同一张资料卡、同一个关系状态。一处修改关系后，另两处不刷新页面即更新；HomeStation 的好友关注在变更事件后即时生效。
3. 在 HomeStation 关注一个人，My Network 的“我关注的人”里立刻出现，显示为单向；取消关注后消失。
4. 好友请求：本 Zone 内两个非好友 owner，以及两个 msg-center 实例模拟的两个 Zone 之间，都能走完请求 → 接受，双方显示为好友；忽略、取消、交叉请求、解除、屏蔽后请求的行为符合 §3.4；重放不产生重复记录。
5. 不可达的 DID 有明确原因；UI 不把单向记录显示成好友，也不在投递未完成时声称请求已送达。
6. 本 Zone 托管的群：凭邀请链接直接加入；申请制显示待审批；`invite_only` 显示需要邀请。他 Zone 的群如实提示不支持。
7. 四种删除操作的语义和确认强度符合 PRD §11.3；系统建立的关系不能删除。
8. 自动化检查通过：
   - mock e2e：`my-network.spec`、`messagehub*.spec`、`homestation.spec`；
   - Desktop：`pnpm check`、`pnpm exec eslint src tests`；
   - Rust：`cargo test -p msg_center -- --test-threads=1`、`cargo test -p buckyos-api -- --test-threads=1`、`cargo test -p homestation`。
9. DV（test.buckyos.io，devtest / alice / bob 在同一个 Zone）：验证本 Zone 内的查找、tag 联动、Requests 处理、建群邀请与入群。DV 没有第二个 Zone，真实跨 Zone 部分标为未验证。

## 7. 参考入口

| 范围 | 入口 |
| --- | --- |
| 公共 UI | `src/frame/desktop/src/components/network/`（新建），组织方式参照 `components/ContentPreview.tsx` + `components/preview/` |
| My Network | `src/frame/desktop/src/app/my-network/`；复用中的 `app/users-agents/components/{detail,layout,cards}/`、`app/users-agents/mock/seed.ts` |
| MessageHub | `app/messagehub/GroupDialogs.tsx`、`groupModel.ts`、`SessionDialogs.tsx`、`EntityDetails.tsx`、`ConversationView.tsx`、`MessageHubView.tsx`、`api/store.ts`、`mock/store.ts`、`datamodel/sessionApi.ts`、`launch.ts` |
| HomeStation | 前端 `app/homestation/source/SourceManager.tsx`、`publish/AudiencePicker.tsx`、`prefs/FeedPreferences.tsx`、`HomeStationPortalRoute.tsx`；后端 `src/frame/homestation/src/{sources,contacts,directory,audience,publish,selection,stream,api}.rs` |
| msg-center | `src/frame/msg_center/src/contact_mgr.rs`（`upsert_zone_contacts`、准入）、`main.rs`（`sync_zone_agent_contacts`）、`group_service.rs`（`notify_group_member`）、`cyfs_dispatch.rs`（`serve`、`verify_signed_message`、`authorizes_signer`、`message_route`）、`message_hub.rs`、`owner_session.rs`（`authorize_contact_scope`）、`msg_center.rs`（`dispatch_to_receiver`、`is_local_recipient`） |
| 共享类型与客户端 | `src/kernel/buckyos-api/src/msg_center_client.rs`、`rbac_config.rs`；websdk `src/msg_center_client.ts` |
| 其他后端 | `src/frame/control_panel/src/user_mgr.rs`、`src/kernel/sys_config_service/src/zone_did_resolver.rs` |
| 公共展示与跨应用调用 | `src/frame/desktop/src/presentation/`；`app/users-agents/launch.ts`、`app/preview/launch.ts`、`app/web-frame/WebAppFramePanel.tsx` |
| 文档 | PRD、协议设计、Contact Mgr、Message Center、HomeStation 架构 v0.7 与协议与实现、MessageHub UI_DATAMODEL、[messagehub-group-v2-todo.md](messagehub-group-v2-todo.md)、[zone-did-resolver需求.md](zone-did-resolver需求.md) |

## 8. 实施记录（2026-10-17）

M0–M6 已实施，未提交 git。与 §3 设计不同或 §3 未定的地方以本节为准，契约细节见 [Contact Mgr.md](<../doc/message_hub/Contact Mgr.md>) §3.4–§3.9、[Message Center.md](<../doc/message_hub/Message Center.md>) §2.1.4 / §2.2 / §4.5、[My Network相关协议设计](<../doc/message_hub/My Network相关协议设计.md>) §5–§7、`components/network/UI_DATAMODEL.md`。

### 8.1 落点

| 范围 | 文件 |
| --- | --- |
| 共享类型 | buckyos-api 新模块 `msg_contact.rs`（intent、FriendState、FriendRequest*、lookup 类型、well-known tag 与 `contact_tag_kind`、CONTACTS kevent、9 个新方法经 `handle_contact_ext`）；`msg_center_client.rs`（`Allow`、删 `Contact.groups` / `ContactPatch.tags`、`ContactQuery.friend_state/tag`、schema v13 `friend_requests`）；`rbac_config.rs`（`system:homestation` 写 `obj://msg-center/contact_tags/homestation`） |
| msg-center | `contact_rpc.rs`（权限）、`contact_lookup.rs`、`friend_service.rs`（状态机）、`network_directory.rs`（DID → Zone / 资料，ZoneSigner）、`contact_mgr.rs`（迁移、tag、remove/unblock、变更事件）、`cyfs_dispatch.rs` / `message_hub.rs`（G1–G3）、`group_service.rs`（`list_by_member` 带 `member` 与 `invitations`）、`test_contact_network.rs` |
| HomeStation 后端 | `contacts.rs`（friend_state、用户 tag、`set_following`）、`sources.rs`（关注写 tag、`reconcile_following`、follow 只收 DID）、`lib.rs` / `main.rs`（订阅 `/msg_center/*/CONTACTS/**` 取代每小时轮询） |
| websdk | `src/msg_center_client.ts`（类型与 `lookup` / `updateContactTags` / 好友请求等方法）、`sdk_core.ts` 导出；SDK dist 已重建（只跑 SDK 构建，没碰 cli/dist） |
| Desktop 公共 UI | `src/components/network/`（FindPeople、PeoplePicker、AddFriendDialog、JoinGroupDialog、RelationBadge、RelationActions、TagEditor、薄缓存、zod 入参与返回值、mock） |
| My Network | `src/app/my-network/`（重写，联系人详情与 Collection 从 users-agents 迁来）、`launch.ts` |
| MessageHub / HomeStation 前端 | 选人、入群、新会话找人、准入横幅与关系区、好友请求卡、陌生 DID 深链；HomeStation 关注、受众与“不看”选人、作者头像走 presentation |
| 文档 | 协议设计、Contact Mgr、Message Center、Durable Data Schema、HomeStation 架构与协议实现、Self-Host-Groupv2、Message Tunnel Design、PRD |
| DV 脚本 | `test/test_msg_center/test_my_network.ts` |

### 8.2 与 §3 不同或补充的决定

- 约定接收路径是 `/kapi/msg-center/<did>/inbox`（Zone 网关原样转发 `/kapi/<service>/*`，不用改网关），只服务 msg-center 已登记的收件人；`cyfs_dispatch.zone_origins` 可覆盖 Zone 源站。显式 `outgoing` 仍须带 Bearer。
- G2 的防骚扰是每发送方每分钟 30 条的进程内配额（429，在验签之前扣），不是冷却；冷却只用于好友请求（接收方拒绝/忽略后 24h 丢弃该方请求；发送方收到拒绝后 24h 内不能再发）。
- unblock 新增 `contact.unblock_contact`（屏蔽解除后好友回 Allow，否则 Stranger）。
- 控制消息（accept / reject / cancel / remove）双方都不留邮箱记录；request 发送方留 SENT，接收方按普通准入写记录。`msg.post_send` 拒绝 `buckyos.friend`，格式不对的这类入站消息直接丢弃。
- 删除联系人先结清关系：好友解除并通知、撤回自己的待处理请求、忽略对方的；屏蔽（`block_contact` 或 `update_contact{block}`）只结清待处理请求，不解除好友。
- Zone 同步：`UserContactSettings.tags` 只在首次成为 Zone 联系人时写入，之后只维护 `zone_user` / `zone_agent`；Agent 不再带 `agent` tag。旧数据加载时迁移（`friend` → `allow`、groups 合入 tags、Zone 关系置 friend）。
- HomeStation 启动、联系人事件后与 `admin.run friends` 都会 `reconcile_following`，把已有的主动关注投影成 tag。
- Collection 就是 tag 文本，不存描述；空 Collection 不存在（新建时至少选一人）。

### 8.3 验证

- `cargo test -p msg_center -- --test-threads=1`（157 例）、`cargo test -p buckyos-api -- --test-threads=1`、`cargo test -p homestation`（42 例）、`cargo test -p kevent --test usage_contract`、`cargo check --workspace --tests --examples` 全过；`cargo build --release -p msg_center -p homestation` 通过。
- websdk `npx jest tests/msg_center_client.test.ts`（44 例）。
- Desktop：`pnpm check`、`pnpm exec eslint src tests`（0 error，8 个既有 warning）、`deno test --config tests/datamodel/messagehub.deno.json --no-check --allow-read --allow-env tests/datamodel/`（137 例）、Playwright（新起 vite）my-network / messagehub* / homestation / users-agents / agent-setup 共 137 例通过。
- 全量 e2e 跑一次（新 vite 端口）：301 passed / 7 failed，失败都与本改动无关：SN Router 与 mobile App Service 返回键是既有基线；content-open ×2 与 systest 写死 4173 端口；ai-center 完整目录单跑也失败（他人近期改动）；app-service opendan 一例是 page.goto 超时，单跑通过。`pnpm build` 通过。
- DV 结果见 §8.5。

### 8.4 未验证与已知缺口

- 真实的两个 Zone 之间没有跑过：DV 上验证了接收方（经公网网关的无 Bearer 签名入站、Zone 托管签名的校验）与发送方的路由解析失败路径，发送方签名后真正投到另一个 Zone 的网关还没验证。
- 没有拒绝群邀请的 RPC（前端归档通知卡代替）；`list_by_member.items` 不带成员数；`msg.list_sessions` 不能按 `request_count` 过滤（前端最多翻 500 个会话）；lookup 不知道对方有没有 HomeStation 主页（“查看主页”可能 404）。
- `merge_contacts` 不迁移 `friend_requests`；屏蔽 / 冷却造成的丢弃对发送方表现为投递失败；按声称 `from` 扣配额，伪造者可耗掉别人的配额；HomeStation 自己写 tag 会触发一次无变更的重同步。
- 联系人导入（CSV）与合并在 My Network 中没有入口（旧的内存假实现已删）。

### 8.5 DV 验收（2026-10-18，test.buckyos.io，`start.py --all` 新装，用户 devtest / alice / bob）

部署：`start.py --all` 安装的是提交 f5c609c88 的构建；验收中修正后又替换了 `bin/msg-center/msg_center`（备份 `.bak-<时间>`）与 `bin/control-panel/web`（备份 `web.bak-<时间>`）。

- `test/test_msg_center/test_my_network.ts`（deno，接口层）通过：lookup、用户 tag 的写入与 `tag-not-writable`、他人库写入被拒、Zone 关系 `system-relation`、发往不可达 DID 的请求带原因等待重试后撤回删除、HomeStation 主动关注写出与撤下 `homestation.following`（服务 token + 新 RBAC 规则真实生效）。脚本默认 member 密码改为 dev 种子的 `buckyos2026`，DID 从 `user.list` 取。
- `tests/dv/my-network.spec.ts`（Playwright，真实登录 bob）两例通过：
  - 真实好友列表（不含本人）、同 Zone 用户“已是好友”、未知 DID 的错误提示、新建并删除用户 Collection（接口回读 tag）、被邀请的群自动加入、凭邀请链接加入本 Zone 群、Requests 无错误、“发消息”打开 MessageHub。
  - 他 Zone 好友请求：每次生成新的 `did:dev` 身份签名 `buckyos.friend` 请求，不带 Bearer 经公网网关 PUT 到 `/kapi/msg-center/<bob>/inbox` → 200 accepted，重放 200，未签名 401；bob 的 Requests 出现该请求，联系人为“请求加我”，MessageHub 好友请求卡点“接受”后接口回读 friend / allow。
- 手工核对：用 OOD 设备私钥（`did:web:ood1.test.buckyos.io#main_key`）代 bob 签名发给 alice → 200 accepted（真实名字服务把设备文档解析到 Zone），同一 kid 换随机私钥 → 403 `invalid-signature`；不存在的本 Zone 用户 → 404；不带 `cyfs-obj-id` 头的 JWT 会按普通 JSON 对象计算 id → 400（发送方 `cyfs_dispatch::send` 总会带该头）。
- 既有 DV 回归：`homestation.spec`、`object-presentation.spec` 全过；`local-user.spec` “admin creates a real local user…” 失败是旧断言（8 月写的 `/^user$/`，10-09 起徽标显示翻译后的 `User` / `Active`），与本次无关。

验收中发现并已修正（未提交）：

- lookup 对裸名字（如 `bob`）每次都等名字服务超时（Owner 与默认文档各 5 秒，失败不缓存，共 10 秒）。改为：本地已有同名联系人时不解析；Owner 查询超时不再追问、单次 3 秒；暂时无回答缓存 30 秒；按名称输入且本地有结果时不报解析错误。DV 实测从 10 秒降到毫秒级。
- name_client 拒绝解析 `did:dev` 等密钥型 DID，原先被当作暂时失败每 60 秒重试；现按不存在处理，投递直接 `native-route-unresolved` 不重试。生产中 OOD 设备 DID 是 `did:web:<ood>.<zone>`，Zone 托管签名的校验不受影响（已在 DV 上验证）。
- My Network 真实列表里出现了 owner 本人（owner 的库里有自己这条 Zone 用户联系人）；`realBackend.listContacts` 现在排除 owner。

