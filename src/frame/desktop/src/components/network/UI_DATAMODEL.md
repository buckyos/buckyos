# Network 公共组件 UI DataModel

- 文档版本：v0.2（2026-10-17，M6：后端定稿后的错误码、`msg.list_sessions` 消息请求、`group.list_by_member` 的 `member` / `invitations`、PeoplePicker `candidates` 与内嵌模式、mock 宿主桥）；v0.1（M0/M4）
- 模块位置：`src/frame/desktop/src/components/network/`（组织方式同 `components/ContentPreview.tsx` + `components/preview/`）
- 上游：[My Network 升级 TODO](../../../../../../notepads/my-network-upgrade-todo.md) §3.1–§3.6、[My Network PRD](../../../../../../product/my_networks/My_Network_PRD.md) §6.2–§6.4、§11.3
- 后端契约：websdk `src/msg_center_client.ts` 的 “Relationships (msg_contact.rs)” 一节；buckyos-api `msg_contact.rs`；`group.*` 见 msg-center `group_service.rs`
- 使用方：My Network（本轮），MessageHub、HomeStation（M6），第三方 App 经 `/sysdlg/*`（M7）
- My Network 独有的视图见 [`app/my-network/UI_DATAMODEL.md`](../../app/my-network/UI_DATAMODEL.md)

## 1. 分层

```text
msg-center (contact.* / group.* / msg.list_sessions / msg.list_session / kevent)
  └─ NetworkBackend           backend.ts：mock/backend.ts（isMockRuntime）或 realBackend.ts（websdk MsgCenterClient + kRPC）
      └─ networkCache          cache.ts：只缓存返回结果、共享在途请求、按变更事件重载；没有规则
          └─ hooks.ts          useNetworkResource / useLookup / useRequests / useOwnerDid
              └─ relation.ts   §3.3 显示规则、可用动作、tag 分类、错误码 → 文案、Requests 聚合（纯函数）
                  └─ 组件       FindPeople / PeoplePicker / AddFriendDialog / JoinGroupDialog / RelationBadge / RelationActions
                      └─ dialogs.tsx  useNetworkDialogs()：以 WindowDialog 弹出，结果以 Promise 返回
```

- 关系判断（能否删除、是否好友、能否入群、查找结果）都由 msg-center 返回；`relation.ts` 只决定“怎么显示”和“显示哪些按钮”。
- 名称与头像一律经 `presentation/`：`personPresentation.ts` 先用 msg-center 给的名称/头像（联系人备注名、lookup 的 profile），没有时用 `useDidPresentation` 读 DID 文档，最后用 DID 可读部分；渲染用 `ObjectAvatar` / `ObjectTitle`。
- 组件在任何 `WindowDialogProvider` 下工作：Desktop 窗口、移动端窗口 sheet，以及自己挂 provider 的独立路由（HomeStation 门户）。窄屏（≤767px）对话框用 `presentation: 'sheet'`。对话框正文组件（`AddFriendDialog` 等）也可以直接内嵌渲染，不经弹窗。

## 2. 展示模型

后端类型直接复用 websdk（`Contact`、`FriendRequest`、`LookupCandidate`、`LookupResult`、`Reachability` 等，见 `types.ts` 的 re-export），UI 只新增下列模型（`types.ts`）：

```ts
/** 一个人当前关系的事实，来自 Contact 或 LookupCandidate.relation。 */
interface RelationFacts {
  isContact: boolean
  friendState: 'none' | 'outgoing_pending' | 'incoming_pending' | 'friend'
  accessLevel: 'block' | 'stranger' | 'temporary' | 'allow'
  temporaryExpiresAtMs?: number   // 仅 temporary 且 grant 未过期
  tags: string[]
  pendingRequestId?: string       // Contact 记录不带，由 friendRequests 补（withPendingRequest）
  messageRequest?: boolean
}

/** 我所在的群：group.list_by_member + 自己的成员记录 + 发言权限。 */
interface GroupRef {
  did: string
  name?: string; description?: string; avatar?: string
  owner?: string; host?: string; joinPolicy?: string
  memberCount?: number            // list_members 可见时为 active 成员数
  role?: 'owner' | 'admin' | 'member'
  state: 'active' | 'invited' | 'pending_admin_approval'
  hostedHere: boolean             // list_by_member.items = true；joined（他 Zone 同步）= false
  canPost: boolean | null         // group.check_access(session.post)；null = 未能检查
  postBlockedReason?: string
}

/** `msg.list_sessions` 中 `request_count > 0` 的会话（`dm:<peer>`）。 */
interface RequestSession { sessionId; peerDid; peerName?; count; lastAtMs; intent?; preview? }
interface MessageRequest { senderDid; senderName?; count; lastAtMs; preview?; sessionId }
/** `group.list_by_member` 的 `invitations`（state `invited`）；`recordId` 是 `dm:<inviter>` 里的邀请通知（找到时）。 */
interface GroupInvitationItem { groupDid; groupName?; recordId?; inviteId; inviterDid; role; receivedAtMs?; expiresAtMs? }
interface JoinRequestItem { groupDid; groupName?; memberDid; entityKind? }

type RequestItem =
  | { type: 'friend_request'; key; request: FriendRequest }
  | { type: 'message_request'; key; request: MessageRequest }
  | { type: 'group_invitation'; key; invitation: GroupInvitationItem }
  | { type: 'join_request'; key; request: JoinRequestItem }
```

## 3. 关系显示规则（TODO §3.3）

`relationBadges(facts)` 的输出顺序即显示顺序；`RelationBadge compact` 在列表里只显示第一项和“我关注的”。

| 徽标 | 条件 | 说明 |
| --- | --- | --- |
| `blocked` 已屏蔽 | `accessLevel = block` | 覆盖其它一切，单独显示 |
| `friend` 好友 | `friendState = friend` | 唯一显示为双向的状态 |
| `outgoing_pending` 等待对方确认 | `friendState = outgoing_pending` | |
| `incoming_pending` 请求加你为好友 | `friendState = incoming_pending` | 同时出现在 Requests |
| `following` 我关注的 | tag `homestation.following` | 单向；可与 `friend` 并存 |
| `contact` 联系人 | 有记录且以上都不是 | |
| `stranger` 不在联系人中 | 无记录且以上都不是 | |
| `message_request` 消息请求 | `messageRequest` 且 `friendState = none` | 追加显示 |
| `temporary` 临时允许至 … | `accessLevel = temporary` 且 grant 未过期 | 追加显示，带到期时间 |

消息请求的判定（`messageRequests()`）：`msg.list_sessions` 中 `request_count > 0` 的私聊会话（`dm:<peer>`；`msg.list_box_by_time` 只列 session 为空的记录，看不到它们），对方没有待处理好友请求（任一方向），且不是好友、未被允许（allow 或有效 temporary）、未被屏蔽；最新一条是好友请求或群邀请通知且只有这一条请求的会话不算。

### 3.1 可用动作与确认强度（PRD §11.3，写死在 `RelationActions`）

`availableActions(facts, { collectionTag })`：

| 当前关系 | 动作（按显示顺序） |
| --- | --- |
| 在某个用户 tag 视图中 | 先加 `remove_from_collection` |
| 已屏蔽 | `unblock`，有记录时 `remove_contact` |
| 非好友 | `add_friend`（打开 AddFriendDialog；外部账号 `msgtunnel` 与群不给），`allow_messages`（未被允许时），`block`，有记录时 `remove_contact` |
| 收到请求 | `accept_request`，`allow_messages`，`block`，`remove_contact` |
| 已发请求 | `cancel_request`，`allow_messages`，`block`，`remove_contact` |
| 好友 | `unfriend`，`block`，`remove_contact` |

| 动作 | 确认强度 | 调用 |
| --- | --- | --- |
| 移出分组 `remove_from_collection` | light：一句话，焦点在确认按钮，说明“仍保留在联系人中” | `contact.update_tags{remove:[tag]}` |
| 删除联系人 `remove_contact` | explicit：列出后果，焦点在取消，危险按钮 | `contact.remove_contact`；系统关系被拒（`system-relation`）时显示原因 |
| 解除好友 `unfriend` | strong：列出后果 + 必须勾选“我明白…”才能确认；“告知对方”默认勾选 | `contact.remove_friend{notify}` |
| 屏蔽 `block` | explicit | `contact.block_contact` |
| 解除屏蔽 / 允许消息 / 撤回请求 / 接受 | 无确认（可逆） | `unblock_contact` / `update_contact{access_level:'allow'}` / `cancel_friend_request` / `respond_friend_request{accept}` |

确认对话框根节点 `data-testid="network-confirm"` 带 `data-strength`，便于测试。

## 4. 查找（TODO §3.2）

- `useLookup(input, { kinds, limit })`：250ms 防抖调用 `contact.lookup`；15 秒内相同问题共享答案；任一关系变更后清空并重问（不闪 loading）。
- 结果按 `section` 分段：`resolved`（查找结果）→ `relation`（我的关系）→ `zone`（本 Zone）；宿主解析器（`FindPeopleResolver`）的结果单独成段，段标题由宿主给。
- 空输入返回本人联系人与本 Zone 用户，PeoplePicker 用它做默认列表。

### 4.1 五种状态

| 状态 | FindPeople / PeoplePicker | AddFriend / JoinGroup |
| --- | --- | --- |
| 加载 | `ListSkeleton`（`network-loading`） | 骨架 |
| 空 | “没有匹配的结果” / “你还没有联系人” | — |
| 错误 | 请求失败：错误文案 + 重试；`LookupResult.error`：按错误码的文案（`not-found` 为普通提示，其余为错误） | 同左 |
| 部分可达 | 结果中既有可达又有 `unreachable`：顶部提示，每项带可达性标签与原因（title） | 资料卡标注不可达，禁用“加好友”，提供“复制我的分享链接” |
| 成功 | 分段列表 | 资料卡 / 结果页 |

## 5. 错误码 → 文案

`networkErrorCode(error)` 从 RPC 错误消息里取第一个已知的 kebab-case 原因码；网络失败归为 `unavailable`；含 permission 归为 `forbidden`；其余显示 `network.error.generic`（带原因码）。文案键 `network.error.<code>`：

| 码 | 来源 |
| --- | --- |
| `invalid-input` / `not-found` / `unreachable` / `forbidden` | `LookupResult.error.code`（另有 `network.lookup.<code>` 专用文案）；`not-found` 也来自 respond/cancel、remove_friend/remove_contact |
| `peer-blocked` / `friend-request-cooldown` / `cannot-befriend-self` / `peer-not-befriendable` / `message-too-long` | `contact.send_friend_request`（已是好友不是错误：返回 `friend_state: 'friend'`、无 `request`） |
| `request-not-pending` / `request-expired` | `contact.respond_friend_request` / `contact.cancel_friend_request` |
| `system-relation` | `contact.remove_friend` / `contact.remove_contact` 拒绝系统建立的关系 |
| `tag-not-writable` / `invalid-tag` | `contact.update_tags` |
| `owner-not-hosted` / `contact_mgr_owner-required` | 好友 RPC / 所有 `contact.*` |
| `invite-required` / `join-not-allowed` / `invalid-invite-link` / `invite-expired` / `member-already-participating` / `capability-denied` / `owner-must-transfer-first` / `member-list-hidden` / `blocked` / `rate-limited` | `group.*` |
| `remote-group-unsupported` | UI 自身（`group.hosted_here = false` 时在请求前拦下） |
| `unavailable` | fetch / 网络失败 |

> 原因码在 RPC 错误消息文本里；`networkErrorCode` 按 `[a-z0-9_]+(-[a-z0-9_]+)+` 取第一个已知码。新增码只需在 `relation.ts` 的 `KNOWN_ERRORS` 与 `i18n/network.ts` 各加一项。

## 6. Requests 四类条目（TODO §3.6）

| 类型 | 来源 | 动作 |
| --- | --- | --- |
| 好友请求（收到） | `contact.list_friend_requests`（全部状态，最近 200 条；Requests 只取 `pending`），`direction = incoming` | 接受 / 忽略（`ignore`）/ 拒绝并告知（`reject`）/ 屏蔽（`ignore` 后 `block_contact`，explicit 确认） |
| 好友请求（发出） | 同上，`direction = outgoing`；`delivery.state` 显示为 等待送达 / 等待对方确认 / 未送达 | 撤回（`cancel_friend_request`） |
| 消息请求 | `msg.list_sessions{with_object:true, lifecycle:'active'}` 翻页（每页 100，最多 5 页）中 `request_count > 0` 的会话，按 §3 规则过滤 | 加好友（AddFriendDialog）/ 允许消息 / 在 MessageHub 中打开该会话 / 屏蔽 |
| 群邀请 | `group.list_by_member` 的 `invitations`（`member.state = invited`，含 `invitation_id`、`invited_by`、`expires_at_ms`），去掉已过期和已是成员的群 | 加入（`group.accept_invitation { group_did, invitation_id }`）/ 忽略：没有拒绝 RPC，在 `msg.list_session { session_id: dm:<inviter> }` 里找到邀请通知则归档它（之后列表据此隐藏），找不到时只在本页隐藏 |
| 待我审批的入群申请 | `group.list_by_member` 中我的 `member.role` 为 owner/admin 的群：`group.list_members` 中 `pending_admin_approval` | 批准 / 拒绝（`group.approve_member` / `reject_member`） |

`useRequests()` 返回 `{ status: 'loading' | 'ready' | 'error' | 'partial', items, errors, retry }`；部分列表失败时为 `partial`。`actionableCount(items)` 不计发出的请求，用于角标。

## 7. 对话框契约（zod，`schemas.ts`）

所有结果只含 DID 与展示文字，不含邀请 token、会话 token 或消息正文；关闭/取消统一返回 `{ action: 'cancelled' }`，Promise 不会因取消而 reject。

```ts
const pickedPersonSchema = z.object({
  did: didSchema, kind: lookupKindSchema, name: z.string().max(256),
  avatar: z.string().max(4096).optional(), zone: z.string().max(256).optional(),
  isContact: z.boolean(), friendState: friendStateSchema.optional(),
})

// FindPeople → /sysdlg/find_people（预留）
findPeopleInputSchema  = { input?, kinds?, limit = 30, title?, actions: [{ id, label }] ≤ 4 }
findPeopleResultSchema = { action: 'invoke', actionId, person } | { action: 'cancelled' }

// PeoplePicker → /sysdlg/people_picker
peoplePickerInputSchema = {
  title?, confirmLabel?, mode: 'single' | 'multi' = 'multi', limit? (1–500),
  filter: { kinds?, excludeBlocked = true, exclude: [{ did, reason }] = [], tag? },
  /** 只在这些人里选（如群的成员），搜索只在本地过滤。 */
  candidates?: [{ did, name?, kind? }],
  initialSelection: did[] = [], allowSaveContact = true,
}
peoplePickerResultSchema = { action: 'selected', people: PickedPerson[] } | { action: 'cancelled' }

// AddFriendDialog → /sysdlg/add_friend
addFriendInputSchema  = { input?, did?, message? (≤500), tags?: userTag[] (≤20) }
addFriendFormSchema   = { message: string ≤500, tags: userTag[] }       // 资料卡内的表单（react-hook-form）
addFriendResultSchema = { action: 'done', outcome: 'friends' | 'request_pending' | 'request_queued' | 'request_failed' | 'saved', person, tags }
                      | { action: 'cancelled' }

// JoinGroupDialog → /sysdlg/join_group
joinGroupInputSchema  = { input? }                                       // 群 DID / BNS 名 / 邀请链接
joinGroupFormSchema   = { input: string 1–2048 }
joinGroupResultSchema = { action: 'joined', groupDid, name, state: 'active' | 'pending_admin_approval' } | { action: 'cancelled' }
```

用户 tag 校验 `userTagSchema`：trim、1–64 字符、`contactTagKind(name) === 'user'`（不能是 `zone_user` / `zone_agent`，也不能形如 `<app>.<name>`）。

AddFriend 的结果如实反映投递（TODO §3.6）：`friend_state = friend` → `friends`（本 Zone 显示“同一个 Zone，直接成为好友”）；`outgoing_pending` 且 `delivery.state` 为 `delivered`（或缺省）→ `request_pending`“等待对方确认”，`pending` → `request_queued`“等待送达”，`failed` → `request_failed` + 复制分享链接；lookup 已判定 `unreachable` 时不发送，显示原因与复制分享链接。“分享链接”目前复制本人 DID（`NetworkBackend.shareText()`）。

JoinGroup：`contact.lookup{kinds:['group']}` 解析输入（含邀请 token）→ 本 Zone 托管时 `group.get_doc` 补资料 → `group.request_join{group_did, invite?}` → `active` / `pending_admin_approval` / `invite-required`。`group.hosted_here = false` 时只显示“暂不支持加入其它 Zone 的群”，没有加入按钮。

## 8. 组件 API

```ts
// 打开对话框（任何 WindowDialogProvider 内）
const dialogs = useNetworkDialogs()
dialogs.findPeople(input?: FindPeopleInput): Promise<FindPeopleResult>
dialogs.pickPeople(input?: PeoplePickerInput): Promise<PeoplePickerResult>
dialogs.addFriend(input?: AddFriendInput): Promise<AddFriendResult>
dialogs.joinGroup(input?: JoinGroupInput): Promise<JoinGroupResult>
dialogs.confirm(options: ConfirmOptions): Promise<{ option: boolean } | null>

// 内嵌组件
<FindPeople initialInput? kinds? limit? actions?: FindPeopleAction[] resolvers?: FindPeopleResolver[]
            onOpen?(candidate) placeholder? autoFocus? showDefault? onInputChange? />
interface FindPeopleAction { id; label; icon?; primary?; appliesTo?(c): boolean; onSelect(c): void | Promise<void> }
interface FindPeopleResolver { id; label; matches(input): boolean; resolve(input): Promise<LookupCandidate[]> }
<PeoplePicker params={PeoplePickerParams} onDone={(r: PeoplePickerResult) => …} />
<PeoplePicker params embedded onChange={(people: PickedPerson[]) => …} />   // 表单内嵌（MessageHub 建群），无底栏
<AddFriendDialog params={AddFriendParams} onDone={(r: AddFriendResult) => …} />
<JoinGroupDialog params={JoinGroupParams} onDone={(r: JoinGroupResult) => …} />
<RelationBadge relation={RelationFacts} compact? />
<RelationActions person={{ did, name?, avatar?, kind? }} relation={RelationFacts} collectionTag? exclude?: RelationActionId[] onDone?(action) />
<TagEditor value onChange suggestions? disabled? label? />    // 只编辑用户 tag

// 数据
useNetworkResource('contacts' | 'friendRequests' | 'requestSessions' | 'groups' | 'invitations' | 'joinRequests')
  → { status: 'loading' | 'ready' | 'error', data?, error? }
useLookup(input, { kinds?, limit?, enabled?, debounceMs? }) → { status, result?, error?, input, retry }
useRequests() → { status, items: RequestItem[], errors, retry }
networkActions.*  // 每个关系动作，完成后重载受影响的列表
networkCache.refresh(key) / networkCache.lookup(input, opts) / networkCache.ownerDid()
```

## 9. 薄缓存

| 资源 | 读取 | 变更事件 → 重载 |
| --- | --- | --- |
| `contacts` | `contact.list_contacts{limit:1000}` | `contacts` |
| `friendRequests` | `contact.list_friend_requests{limit:200}`（全部状态，好友请求卡片要显示已处理的状态） | `requests` |
| `requestSessions` | `msg.list_sessions`（见 §6） | `mailbox` |
| `groups` | `group.list_by_member`：`items`（`member.role`）+ `invitations` 中 `pending_admin_approval`（显示为等待审批）+ `joined`；本 Zone 群另调 `check_access(session.post)` | `groups` |
| `invitations` | `group.list_by_member` 的 `invitations`（见 §6） | `mailbox`、`groups` |
| `joinRequests` | 我管理的群的 `list_members` | `groups` |

`group.list_by_member` 在 1.5 秒内共享一次结果（三个资源都读它），入群、退群、审批后作废。事件来源（`realBackend.subscribe`）：kevent `/msg_center/<owner raw host>/CONTACTS/changed` → `contacts` + `requests`；`/REQUEST_BOX/**` → `mailbox`；`/INBOX/**` → `mailbox` + `groups`；浏览器 `online` → 全部。组件内的动作完成后立即按受影响资源重载，不等事件。只重载已经被读过的资源。

## 10. Mock

`mock/data.ts` 是一个完整的关系世界（owner `did:bns:alice`，Zone `alice.buckyos.io`），`mock/backend.ts` 实现同一个 `NetworkBackend`，读 240–500ms 延迟，写后 40ms 发变更事件。`?networkScenario=` 可组合：

| 场景 | 效果 |
| --- | --- |
| `empty` | 没有联系人、请求和群（本 Zone 用户仍可查找） |
| `error` | 每个列表第一次读取失败，重试成功 |
| `slow` | 延迟 ×4 |

| 输入 | 演示 |
| --- | --- |
| `bob` / `carol` | 本 Zone 用户，已是好友 |
| `dave` | 本 Zone 新用户，加好友后立即成为好友 |
| `did:bns:nora` | 他 Zone，可送达 → 等待对方确认 |
| `did:bns:oscar` | 他 Zone，Zone 离线 → 等待送达 |
| `did:bns:quinn` | 不可达（`zone-gateway-unreachable`） |
| `did:bns:nobody` / `%%%` / `did:bns:secret` | `not-found` / `invalid-input` / `forbidden` |
| `did:bns:book-club` / `design-guild` / `core-team` / `core-team?invite=core-2026` / `garden.eve` | 入群成功 / 待审批 / 需邀请 / 凭邀请加入 / 他 Zone 不支持 |
| Requests | Nina、Peggy 的好友请求；Victor（2 条）、Wendy 的消息请求；Hiking Club 邀请；Ruth 申请加入 Project Alpha；发出给 Ivan 的请求 |

### 10.1 mock 宿主桥（`mock/hosts.ts`）

真实 Zone 里所有应用读同一个 msg-center；mock 里各应用有自己的世界。`registerNetworkMockHost({ people, group, groups, requestJoin })` 让 MessageHub mock（人、Agent、自托管群，入群走 `requestGroupJoin`）与 HomeStation mock（作者）把各自的人和群并入同一个 network mock：宿主的人在首次读取时成为联系人（`friend` 标记的成为好友），宿主的群可以被查找、预览和加入，加入由宿主执行，所以 MessageHub 里随即出现该群。

## 11. 字段稳定性

| 字段 | 稳定性 | 说明 |
| --- | --- | --- |
| `LookupCandidate.did / kind / section / relation / reachability` | Frozen | websdk 契约 |
| `RelationFacts`、`relationBadges` 顺序 | Frozen | 三端显示一致的基础 |
| 对话框 input/result schema | Frozen | 未来 Sys Dlg 参数 |
| `GroupRef.canPost / memberCount` | Extensible | 取决于群的可见性设置 |
| `RequestSession`、`GroupInvitationItem` | Extensible | 来自 `msg.list_sessions` 与 `group.list_by_member`；邀请的 `recordId` 靠扫描 `dm:<inviter>` |
| 错误码集合 | Extensible | 随后端补充 |

## 12. 已知后端缺口（本轮前端的权宜做法）

1. 没有拒绝群邀请的 RPC：“忽略”归档 `dm:<inviter>` 里的邀请通知（每个邀请多一次 `msg.list_session` 调用来找通知），找不到通知时只在本页隐藏。
2. `group.list_by_member` 的 `items` 不带成员数：Joined Groups 详情用 `contact.lookup` 的 `group.member_count`（成员列表不可见时显示“不可见”）。
3. lookup 候选没有“是否有 HomeStation 主页”字段：“查看主页”对已知 Zone 的人给出 `https://<zone>/homestation/<DID 可读部分>`（本 Zone 用相对路径），可能 404。
4. `Contact` 不带待处理请求的 `request_id`，UI 用 `list_friend_requests` 按 `peer_did` 补。
5. 消息请求要翻 `msg.list_sessions`（最多 500 个会话）找 `request_count > 0`；没有按请求数筛选的参数。
