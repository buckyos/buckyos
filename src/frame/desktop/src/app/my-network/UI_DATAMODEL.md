# My Network UI DataModel

- 文档版本：v0.2（2026-10-17，M6：消息请求按会话、等待审批的群、从请求打开 MessageHub 会话）；v0.1（M0/M5）
- 模块位置：`src/frame/desktop/src/app/my-network/`
- 上游：[My Network PRD](../../../../../../product/my_networks/My_Network_PRD.md) §5–§11、[升级 TODO](../../../../../../notepads/my-network-upgrade-todo.md) §3.5、§3.6、M5
- 关系数据、显示规则、对话框与错误文案都来自公共组件，见 [`components/network/UI_DATAMODEL.md`](../../components/network/UI_DATAMODEL.md)；本文只写 My Network 独有的视图。

## 1. 结构

```text
MyNetworkAppPanel            解析 launch，按 requestId 重建 Shell
└─ MyNetworkShell            导航 | 条目列表 | 详情（窄屏：nav → view → detail 栈，返回走 useMobileBackHandler）
   ├─ NavSidebar             首页 / 联系人 / 好友 / 请求 / 已加入的群；分组三段；加好友、加入群两个一级入口
   ├─ HomeOverview           数量、最近添加、待处理请求、分组
   ├─ CollectionList         联系人 / 好友 / 某个分组的人：搜索、筛选、排序、多选批量
   ├─ ContactDetailPage      某人的身份、关系与动作、分组、可达性、来源
   ├─ RequestsView           四类请求
   └─ JoinedGroupList / JoinedGroupDetail
```

旧的 `relation: 'one-way' | 'mutual'`、`is_verified → mutual` 映射、内存 store、写死的 Nora 与 “coming soon” 对话框都已删除；联系人详情、分组组件从 `app/users-agents/` 移到这里，联系人 mock 变成 `components/network/mock/data.ts`（MessageHub、HomeStation 将共用）。

## 2. 视图与启动参数

```ts
type MyNetworkView =
  | { kind: 'home' } | { kind: 'contacts' } | { kind: 'friends' }
  | { kind: 'requests' } | { kind: 'groups' }
  | { kind: 'collection'; tag: string }

// launch.ts，openMyNetwork(launch) 仿 openUsersAgents
const myNetworkLaunchSchema = z.union([
  z.object({ view: z.literal('requests') }),
  z.object({ entity: z.string().startsWith('did:') }),            // 打开某人（或某个已加入的群）的详情
  z.object({ action: z.literal('add-friend'), input: z.string().max(2048).optional() }),
  z.object({ action: z.literal('join-group'), input: z.string().max(2048).optional() }),
])
```

- `entity`：联系人视图 + 详情；不是联系人时详情页用 `contact.lookup` 显示资料与“加好友 / 允许消息 / 屏蔽”；是已加入的群时显示群详情。
- `action`：打开后弹出 AddFriendDialog / JoinGroupDialog（带 `input`）；加好友完成后打开此人详情，入群成功后打开群详情。

## 3. 分组 = tag（TODO §3.5、D5）

`collectionsOf(contacts)` 统计每个 tag 的人数，分三段；well-known 的应用 tag 与系统 tag 即使没人也列出：

```ts
interface CollectionSummary { tag: string; kind: 'user' | 'app' | 'system'; app?: string; count: number; editable: boolean }
interface CollectionSections { user: CollectionSummary[]; app: CollectionSummary[]; system: CollectionSummary[] }
```

| 段 | 例 | 显示名 | 可做的事 |
| --- | --- | --- | --- |
| 我的分组（用户 tag） | “Project Alpha Team” | tag 文本 | 添加成员（PeoplePicker，已在组内的显示原因）、移出（light 确认）、重命名（逐个成员改 tag）、删除分组（explicit 确认，人仍在联系人中）、批量加入/移出 |
| 来自应用（应用 tag） | `homestation.following` | “我关注的人” / People I follow（i18n） | 只读；“在 HomeStation 中管理”打开 HomeStation |
| 系统 | `zone_user`、`zone_agent` | “本 Zone 用户”“我的 Agent” | 只读 |

- 新建分组 = 输入名称（`userTagSchema` + 不与已有分组重名）→ PeoplePicker 选至少一人 → 给每人 `update_tags{add}`。tag 没有成员就不存在，所以不提供空分组；分组描述不另存（M0 决定）。
- 联系人详情的“分组”卡片用 `TagEditor` 直接增删用户 tag（删除走 light 确认），其余 tag 以只读标签列在“同时出现在”。

## 4. 列表（第二栏）

```ts
type PeopleFilter = 'all' | 'friends' | 'others' | 'blocked'
type PeopleSort = 'name' | 'recent'      // recent = created_at 倒序
```

- 视图成员：联系人 = 全部 `contacts`；好友 = `friend_state = friend`；分组 = 带该 tag 的联系人（`peopleOfView`）。
- 搜索在当前视图内过滤名称、DID、备注、tag、绑定账号（`filterPeople`）。
- 多选后的批量操作：加入分组（选已有分组或新建）；在用户分组视图里另有“移出分组”。
- 每行：头像、名称、备注 / 绑定账号 / DID、`RelationBadge compact`（已用 `withPendingRequest` 补上请求状态）。
- 状态：加载骨架；错误（`my-network-list-error`）带重试；空（按视图给不同文案）；无匹配。

## 5. 详情（第三栏）

| 区块 | 内容 |
| --- | --- |
| 头部 | 头像、名称、DID（可复制）、类型、Zone、`RelationBadge`、可达性、已验证；“发消息”（`openMessageHub(did)`，MessageHub 的 launch：`{ kind: 'messagehub', entityId, context }`）、“查看主页”（见 components 文档 §12.3） |
| 关系 | `RelationActions`（在用户分组视图中带 `collectionTag`） |
| 分组 | 用户 tag 可编辑；应用 / 系统 tag 只读 |
| 可达性 | DID、Zone、送达状态与原因（lookup）、绑定账号 |
| 来源与历史 | `source`、添加时间、更新时间 |
| 备注 | `note` |

## 6. 已加入的群

- 列表来自 `useNetworkResource('groups')`（`GroupRef[]`）：名称、角色、“本 Zone 托管”标签；申请后等待审批的群也列出并标“等待审批”（导航与首页的数量只算已加入的）。成员数在详情里取 `contact.lookup` 的 `group.member_count`。
- 详情：群资料（群主、托管方、成员数或“不可见”）、我的状态、能否发言；“在 MessageHub 中打开”；owner/admin 另有“在 MessageHub 中管理”（同样打开 MessageHub，由其 GroupPanel 管理）；“退出群”（explicit 确认；群主被拒时显示 `owner-must-transfer-first`）。
- 他 Zone 托管的群标注“其它 Zone 托管”，说明本 Zone 只做同步。

## 7. 首页

数量卡片（联系人、好友、已加入的群、待处理请求，点击跳转）；有待处理请求时的提示条；最近添加的 5 人；分组快捷入口（用户分组 + 应用分组）；加好友、加入群两个按钮。

## 8. 验证入口

- mock e2e：`tests/e2e/pages/my-network.spec.ts`（加好友本 Zone / 他 Zone / 等待送达 / 不可达，好友请求与消息请求，群邀请与入群审批，入群成功 / 待审批 / 需邀请 / 他 Zone，用户分组增删与新建，“我关注的人”只读，四种删除语义与系统关系拒绝，加载失败重试与空态，375px 移动端）。
- 场景参数：`?networkScenario=empty|error|slow`。
