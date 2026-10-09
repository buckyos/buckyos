# HomeStation UI DataModel

- 文档版本：v0.2（2026-10-09，v0.1 按架构 v0.6 重构原型后提炼；v0.2 接入 homestation 服务：§10 映射、§11 devnet）
- 文档类型：UI DataModel 设计文档（WebUI Dev Loop 阶段三产物）
- 模块位置：`src/frame/desktop/src/app/homestation`
- 上游文档：
  - 架构：`doc/homestation/BuckyOS HomeStation 架构设计.md` v0.6（§4.4 发表流、§4.5 受众、§5 Feed Object、§8.5 标签过滤、§9 Feed UI、§12–§16 评论与 Head、§17.4 应用能力）
  - PRD：`product/homestation/homestation.md`
  - 任务：`notepads/homestation-prototype-v0.6-todo.md`
- 下游用途：`integrate-ui-datamodel-with-backend`

---

## 1. Overview

### 1.1 分层

| 层 | 文件 | 内容 | 边界 |
| --- | --- | --- | --- |
| 协议层 | `protocol/feed.ts` | `FeedObject`、`FeedHead`、`FollowDeclaration`、`FileObject` | 跨节点对象的镜像，字段取自架构 §5.8–§5.11 示例，**不是冻结线格式**；UI 不得往里加私有字段 |
| UI 投影 | `datamodel/types.ts` | `FeedItemView`、`EntryState`、`ReadingEntry`、`PersonalState`、`InteractionStats`、`CandidateEntry` 等 | 由不可变对象 + 本地私有状态组合而成，可随时重算 |
| 输入模型 | `datamodel/inputs.ts` | zod schema：发布、评论、受众、来源输入、过滤规则 | 表单的唯一校验来源，错误消息是 i18n key |
| Store 接口 | `store/types.ts`（`HomeStationStore`） | 异步 API + 同步 peek + 按领域通知 | 组件只依赖这个接口 |
| Mock store | `mock/store.ts`（seed 在 `mock/data.ts`，占位图在 `mock/media.ts`） | 异步 API（300–800ms 延迟）+ 同步 peek | 只在 Mock 运行时且未设开发覆盖时使用 |
| 服务 store | `api/store.ts`、`api/transport.ts` | homestation 服务 kRPC + 本地缓存 + `ui.versions` 轮询 | 映射见 §10 |
| 组件 | 其余 `*.tsx` | 视图 | 只经 store 读写；不直接改 seed |

架构 §5.7 的四层在 UI 中对应为：不可变 Feed Object → `FeedItemView.object`；签名与验证 → `FeedItemView.verification`；入口状态 → `EntryState`；私有应用投影 → `ReadingEntry` / `PersonalState` / `InteractionStats` / `CandidateEntry`。**`isLiked`、计数、推荐理由、本地标签都不在对象上。**

### 1.2 视图

| 视图 | 组件 | 数据来源 |
| --- | --- | --- |
| 待读 Feed（筛选、Topic、搜索、四种阅读模式） | `feed/FeedPage.tsx`、`FilterBar.tsx`、`ImmersiveMode.tsx` | `listReading` |
| 内容详情 + 评论区 | `detail/ItemDetail.tsx`、`detail/CommentSection.tsx` | `getWrappedBody`、`listComments`、`peekCard` |
| 我的发表（发表 / 投递状态、编辑、撤回、调整受众、Head 调试面板） | `publish/MyPublications.tsx` | `listPublished(owner, { reader: owner, kind })`、`listChanges` |
| 我的主页 + 以访客身份预览 | `PublicProfileView.tsx` | `getProfile`、`listPublished(owner, { reader })` |
| 访客门户 `/homestation/u/:did` | `HomeStationVisitorRoute.tsx` | 同上，`reader` 来自 `?reader=anonymous｜follower｜friend` |
| 关注补看 | `candidates/FollowedCandidates.tsx` | `listFollowedCandidates`、`openCandidate` |
| 收藏 / 稍后再看 | `saved/SavedList.tsx` | `listSaved` |
| 来源 | `source/SourceManager.tsx` | `listSources`、`resolveSourceInput`、`follow`、`unfollow`、`pauseSource` |
| 过滤与不看 | `prefs/FeedPreferences.tsx` | `setMuteRule`、`setFilterRule`、`setDefaultAudience` |
| 发布（移动端整页；桌面右栏快速发布） | `publish/PublishComposer.tsx` | `publish`、`retryPublish`、`uploadAttachment`、`fetchLinkPreview` |
| 桌面左栏 / 右栏、移动端“我”页 | `SidebarPanel.tsx`、`InfoPanel.tsx`、`MePage.tsx` | `peekSyncStatus`、`peekTopics`、`peekReadingSummary` |

---

## 2. 协议层（`protocol/feed.ts`）

```ts
type ObjId = string; type Did = string; type EntryUrl = string
type CommentType = 'text' | 'like' | 'bookmark' | 'repost' | 'quote'
type ContentType = 'text' | 'image' | 'video' | 'audio' | 'article' | 'link' | 'product'

interface FeedObject {
  kind: 'post' | 'comment'
  comment_type?: CommentType
  publisher: Did
  iat: number                    // Unix 秒，只用于展示
  entry?: EntryUrl               // 可变入口；缺省 = 终态对象
  content?: { type: ContentType; text?; title?; summary?; cover?: ObjId; media?: { object: ObjId; alt? }[] }
  wraps?: ObjId                  // 包裹形态
  references?: { relation: 'comment_on'; object_id: ObjId }[]
  tags?: string[]                // 作者标签
  source?: { kind: 'web' | 'rss' | 'platform'; original_url; original_author?; captured_at_ms }
  link?: string                  // 跳转目标，不是内容引用
  publication_category?: 'work' | 'product'
  base_on?: ObjId
}
interface FeedHead { kind: 'feed_head'; entry: EntryUrl; seq: number; state: 'active' | 'withdrawn'; current?: ObjId; updated_at_ms: number }
interface FileObject { kind: 'file'; name: string; meta: { mime; size; width?; height?; duration_ms? } }
```

- 媒体只以 ObjId 引用；mime、尺寸、时长在 `FileObject.meta`。UI 通过 `media.ts` 把 ObjId 解析成地址：服务 store 下是同源 `/home/objects/<ObjId>/content?access=<session token>`，Mock 下是 `mock/media.ts` 生成的 SVG 占位图；图片详情把 `homestation-file` 源注册给 `ContentPreview`（服务 store 下取回该地址的 blob）。
- URL 只出现在 `link` 与 `source.original_url`。
- `commentTarget(object)`：转发／引用转发的目标是 `wraps`，其他评论是 `references` 中的 `comment_on`。
- 入口：正文与评论 `cyfs://<zone>/home/feed/@/<key>`，公开互动 `cyfs://<zone>/home/reactions/@/<互动键摘要>`，商品可用内容 DID（seed：`did:bns:echoes-of-the-void`）。

---

## 3. UI 投影（`datamodel/types.ts`）

### 3.1 卡片

```ts
interface CardView {
  item: FeedItemView
  reading?: ReadingEntry          // 仅所有者，且条目在待读列表中
  personal?: PersonalState        // 仅所有者
  stats?: InteractionStats        // 本地合并视图
  resources: ResourceState        // 'local' | 'reachable' | 'preparing' | 'unavailable'
  canRepost: boolean
  repostBlockedReason?: 'restricted' | 'private_capture' | 'withdrawn' | 'not_visible' | 'reaction'
  sharedAs?: ObjId                // 私人抓取已分享后对应的发表
}

interface FeedItemView {
  objId: ObjId
  object: FeedObject
  contentType: ContentType | 'comment' | 'reaction'
  publisher: IdentityView         // 签名者
  originalAuthor?: string         // source.original_author
  capturedFrom?: string           // source.original_url 的主机名
  isCapture: boolean
  isPrivateCapture: boolean       // 本地候选，无入口，只对所有者可见
  verification: 'verified' | 'unverified' | 'wrapper_only'
  entry?: EntryState
  audience: { spec: AudienceSpec; restricted: boolean }
  media: ResolvedMedia[]; cover?: ResolvedMedia; wrappedFile?: FileObject
  embedded?: { relation: 'repost' | 'quote' | 'comment_on' | 'wraps'; visibility: 'visible' | 'not_visible' | 'withdrawn' | 'missing'; item?: FeedItemView }
  category?: 'work' | 'product'
  createdAt: number               // iat * 1000
  isOwn: boolean
}

interface EntryState {
  entry: EntryUrl; seq: number
  state: 'active' | 'withdrawn' | 'conflict'
  currentObjId?: ObjId
  isLatest: boolean               // 当前看到的 objId 是否就是 Head 指向的版本
  version: number; versionCount: number
}
```

展示规则：

- 头部显示签名者与验证结果；私人抓取显示来源站点，副行“由你的 Spider 抓取 · 原作者 X”；他人分享的抓取副行“由 Bob 分享 · 抓取自 host · 原作者 X”。
- `embedded.visibility = 'not_visible'` 显示“原文不可见”，`withdrawn` 显示“原文已撤回”。
- `entry.isLatest = false` 显示“已更新 · 查看最新”，互动仍绑定在看到的版本上；`state = 'withdrawn'` 时 Feed、收藏中显示撤回占位。
- 受限条目（`audience.restricted`）显示受众标记；他人内容只显示档位（好友可见等），自己的内容显示完整受众。

### 3.2 待读条目、标签与过滤

```ts
interface ReadingEntry {
  objId: ObjId
  admittedAt: number
  reason: { code: 'followed' | 'friend' | 'topic' | 'subscription' | 'collector' | 'rule'; text: string; refs: { kind: 'source' | 'topic' | 'rule' | 'intent' | 'collector'; id: string; label: string }[] }
  effectiveTags: EffectiveTag[]
  resources: ResourceState
  filteredBy: string[]            // 命中的过滤规则 id；非空时默认隐藏
  topics: string[]                // 按有效标签映射的本地 Topic
}
interface EffectiveTag {
  tag: string; label: string
  source: 'author' | 'model' | 'user'
  status: 'declared' | 'inferred' | 'confirmed'
  scope: 'whole_content' | 'part'
  confidence?: number; basis?: string; classifierRevision?: string
}
interface FilterRule { id; enabled; conditions: ('ai_full' | 'ai_assisted' | 'low_quality')[]; acceptInferred: boolean; minConfidence: number; unknown: 'show' | 'hide' }
type MuteRule = { kind: 'person'; did; name } | { kind: 'group'; groupId; name }
```

- 有效标签 = 作者标签（`declared`）+ 模型推断（`inferred`）+ 用户修正（`remove` / `confirm` / `to_assisted`）。作者标签永远不被改写。
- 规则命中：规则内所有条件都成立。模型推断只有在 `acceptInferred` 且 `confidence ≥ minConfidence` 时成立；条目未分类（`classified = false`）且缺少该标签时按 `unknown` 处理。
- 推荐理由的展示文案由 UI 按 `code` + `refs` 做 i18n；`text` 是推荐服务的说明，在“为什么推荐”中原样展示。
- “不看”按直接发表者（含其评论和转发）过滤，同时作用于待读与关注补看；不改关注与好友关系。

### 3.3 个人状态与统计

```ts
interface InteractionFlag { on: boolean; visibility: 'public' | 'author_only' | 'private'; delivery: 'none' | 'delivering' | 'delivered' | 'partially_failed'; entry?: EntryUrl; seq?: number }
interface PersonalState { like: InteractionFlag; bookmark: InteractionFlag; repost: InteractionFlag; readLater: boolean; dislike: boolean }
interface InteractionStats {
  view: 'local' | 'author' | `collector:${string}`
  asOf: number; sync: 'synced' | 'syncing' | 'partial'
  textComments: number; likes: number; reposts: number; quotes: number
  claimed?: { likes?: number; source: string }   // 作者自报，只能标为“作者声明”
}
```

- 点赞默认 `public`；目标受限时为 `author_only`（入口受众 = 原作者）。首次公开点赞前弹出“点赞是公开的”。
- 收藏默认 `private`，不产生入口；“公开收藏”时签发互动键入口的 Head，改回私有时签发撤回 Head，本地收藏保留。
- 稍后再看、点踩只在本地（点踩语义待确认，见 §8）。
- 统计按视图从互动记录派生：同一发表者对同一目标的点赞／转发／收藏至多计一次，撤回的不计。评论数默认只算正文评论。

### 3.4 评论

```ts
interface CommentView {
  objId: ObjId; item: FeedItemView; commentType: CommentType
  targetObjId: ObjId
  onOldVersion: boolean; targetVersion: number   // 针对同一入口的旧版本
  sourcePaths: { kind: 'author_list' | 'collector' | 'push' | 'participant'; label: string }[]
  listedByAuthor: boolean; listedByCollector: boolean
}
```

- `listComments(objId, { view, type })` 返回该入口内**当前版本及更早版本**的记录；旧版本的单独分组并标“针对旧版本”。
- 本地合并视图中 `listedByAuthor = false` 只能表述为“作者视图当前未列出”。
- 评论受限内容时，评论的受众固定为原作者：出现在“我的发表”（标“不出现在你的公开主页”），不出现在任何访客读取中。

### 3.5 发表流、发布任务与 Head

```ts
type AudienceSpec = { kind: 'public' } | { kind: 'followers' } | { kind: 'friends' } | { kind: 'group'; groupId } | { kind: 'dids'; dids: Did[] }
type ReaderIdentity = { kind: 'owner' } | { kind: 'anonymous' } | { kind: 'did'; did: Did }
interface PublishedEntryView { entry; objId?; head: EntryState; audience; task?: PublishTask; kind: 'post' | 'comment' | 'repost' | 'quote' | 'like' | 'bookmark'; publishedAt }
interface PublishTask { key: string; stage: 'uploading' | 'failed' | 'published'; entry?; objId?; delivery?: { state: 'delivering' | 'delivered' | 'partially_failed'; delivered; total }; error?; createdAt }
interface StreamChange { cursor: number; entry; seq; state; current?; kind: 'head' | 'audience'; at }
```

- 受众不写入对象，存在入口上；`listPublished` 按读者身份过滤：匿名只得公开；关注者得公开 + 关注者；好友另得好友与所在分组的条目；指定 DID 只给名单。所有者看到全部（含已撤回）。
- 访客读取不含已撤回条目，也不含点赞／公开收藏（“我的发表”里才有）。
- 每次点赞、取消、转发、取消、公开收藏、编辑、撤回都在对应入口签发 `seq + 1` 的 Head，并追加一条发表流级变化（`StreamChange.cursor`）。调整受众不产生 Head，只记一条 `kind: 'audience'` 的变化。“我的发表”的 Head 调试面板展示两者。
- 发布幂等：`publish(input, key)` 对同一 `key` 只会产生一条发表；失败（`stage: 'failed'`）不显示为成功，`retryPublish(key)` 复用同一 key。发布不会插入待读列表。

### 3.6 关注补看、来源

```ts
interface CandidateEntry { objId; selection: 'unscreened' | 'not_selected' | 'preparing'; sourcePaths: { transport: 'pull' | 'push'; label }[]; arrivedAt; firstAdmittedAt: number | null; openedAt: number | null; resources }
interface SourceView { id; name; kind: 'person' | 'rss' | 'website' | 'channel'; did?; url?; basis: ('active' | 'friend')[]; notify: 'acknowledged' | 'unsupported' | 'retrying' | 'pending'; lastSuccessAt?; lastError?: { at; reason }; paused; intentId?; credibility?; updateMode? }
interface SubscriptionIntent { id; text; sourceIds: string[]; status: 'collecting' | 'mapping' | 'paused'; updatedAt }
```

- 关注补看 = 候选中 `firstAdmittedAt === null`、未撤回、可读、未被“不看”和过滤规则命中的条目；默认排除 `openedAt !== null`（读过的），可勾选“包括已读过的”。
- `openCandidate` 按需准备资源，可能失败（seed 中 Alice 的长文第一次失败）。
- 只有好友依据（`basis = ['friend']`）的来源不能单独取消订阅，界面引导改用“不看”。暂停采集与取消订阅是两个操作。

---

## 4. 输入模型（`datamodel/inputs.ts`）

| schema | 用途 | 约束 |
| --- | --- | --- |
| `audienceSchema` | 发布、引用转发、调整受众、默认受众 | 五档；`group` 需 `groupId`；`dids` 至少 1 个，格式 `did:<method>:…`，最多 20 |
| `publishInputSchema` | 发布面板、快速发布（同一模型） | 文字 ≤2000；附件 ≤9，必须全部 `uploaded` 且有 ObjId；视频、语音各至多 1；文字、附件、链接卡片至少一项 |
| `linkCardSchema` | 从 URL 导入链接卡片 | http(s) URL；标题 1–120；摘要 ≤280 |
| `commentInputSchema` | 评论、编辑正文 | 1–1000 |
| `sourceInputSchema` | 来源输入 | `follow` ≥2 字符；`url` 为 http(s)；`natural` ≥4 字符 |
| `filterRuleInputSchema` | 过滤规则编辑 | 至少 1 个条件；阈值 0.5–0.99；`unknown` 为 show／hide |

样本：

```ts
// 合法
{ text: 'Saturday trail plan', attachments: [], link: null, audience: { kind: 'group', groupId: 'group-hiking' } }
// 非法 → homestation.validation.empty
{ text: '', attachments: [], link: null, audience: { kind: 'public' } }
// 非法 → homestation.validation.uploadPending
{ text: '', attachments: [{ id: 'a1', kind: 'image', name: 'x.png', status: 'uploading' }], link: null, audience: { kind: 'public' } }
// 非法 → homestation.validation.didFormat
{ text: 'hi', attachments: [], link: null, audience: { kind: 'dids', dids: ['alice'] } }
```

默认值：受众取 `peekSettings().defaultAudience`（临时为公开，待确认）。

---

## 5. 状态模型

| 视图 | 加载 | 空 | 错误 | 进度 |
| --- | --- | --- | --- | --- |
| 待读 Feed | 骨架卡片 | 区分“列表为空 / 视图为空 / 搜索无结果” | 错误 + 重试 | 加载更多；隐藏数量提示 |
| 详情正文（长文） | “按对象 ID 获取正文…” | — | 不可用 + 重试 | 准备中 + 再检查 |
| 评论区 | 骨架 | “这个视图里还没有内容” | 错误 + 重试 | 收录者视图标“部分同步” |
| 我的发表 | 骨架 | 分类为空 | 错误 + 重试 | 上传中 / 投递 n/m / 部分失败 + 重试 |
| 关注补看 | 骨架 | “没有需要补看的内容” | 错误 + 重试 | 打开时“准备中…”，失败“资源暂时不可用 + 重试” |
| 主页 / 访客 | 骨架 | “这位读者在这里看不到内容” | 错误 + 重试 | — |
| 视频 / 语音资源 | — | — | 不可用 + 重试 | 准备中（不显示播放和时长） |

Mock 场景（URL `?scenario=`，可逗号组合）：`empty`（待读与候选为空）、`error`（每个列表接口第一次调用失败）、`publish-fail`（第一次发布失败）。

---

## 6. 分页、游标与查询

- 列表统一用不透明游标（上一页最后一项的稳定 id，不是数组下标）。待读、候选每页 8 条，发表流每页 10 条。前端用 SWR infinite 缓存每个视图的页面。
- 待读查询 `ReadingQuery = { filter, topicId, search, showFiltered }`；视图键是它的序列化（`feed/useReadingList.ts`）。搜索在整个视图范围内执行（store 在全量上过滤），不是只搜已加载的页。
- 每个视图记住自己的滚动锚点（首个可见条目 id + 偏移，附 5 个后备 id）。切换视图、从详情返回、列表因规则变化重算时都按锚点恢复（`feed/useScrollMemory.ts`）。
- 沉浸式模式读取同一个视图键，因此保留筛选、Topic 与搜索。
- Store 写操作按领域（`reading`、`candidates`、`published`、`comments`、`saved`、`sources`、`prefs`、`profile`）通知，列表据此在原地重新验证；卡片通过 `peekCard` 同步读取，点赞等操作立即反映（投递状态随后从 `delivering` 变为 `delivered`）。

---

## 7. 字段稳定性

| 字段 | 稳定性 | 说明 |
| --- | --- | --- |
| `objId`、`FeedObject.*` | Volatile | 跟随架构 §21 的线格式决定；`kind`/`comment_type`/`entry`/`wraps`/`references` 的语义稳定 |
| `EntryState.seq/state/currentObjId` | Frozen（语义） | Head 单调序号与撤回语义是协议核心 |
| `AudienceSpec` | Extensible | 受众枚举最终形式待定（§21） |
| `ReadingEntry.reason`、`effectiveTags`、`filteredBy` | Extensible | 推荐与评价服务可新增 code／来源 |
| `PersonalState` | Frozen | 默认可见性（点赞公开、收藏私有）已拍板 |
| `InteractionStats` | Extensible | 视图键、同步状态可增加 |
| `PublishTask` | Extensible | 投递阶段可能细分 |
| `SourceView.updateMode`、`credibility` | Volatile | mock 文案，接后端时替换为结构化字段 |
| `CandidateEntry.selection` | Extensible | 可能增加淘汰、过滤原因 |

---

## 8. 待确认（TODO §13，代码中以“待确认”注释标出）

| 事项 | 临时做法 | 位置 |
| --- | --- | --- |
| 正文、评论、转发的默认受众 | 公开，可在“过滤与不看”页修改 | `mock/data.ts` settings、`prefs/FeedPreferences.tsx` |
| 点踩语义 | 仅本地反馈，不发表、不计数，UI 注明 | `mock/store.ts` `setDislike`、`card/ActionBar.tsx` |
| 移动端是否改为 PRD 底部五栏 | 保留顶栏 + 发布按钮，新页面入口集中在“我”页 | `HomeStationView.tsx` |
| 公开门户由 Zone 根 `$` 的 HomeStation 服务提供 | Desktop 内用 `/homestation/u/:did` 路由模拟 | `HomeStationVisitorRoute.tsx` |

---

## 9. Mock 数据契约（`mock/data.ts`）

| 情形 | seed |
| --- | --- |
| 文字 / 多图 / 视频（包裹）/ 语音 / 长文（包裹 markdown）/ 作品链接卡片 / 商品（DID 入口） | Alice 的 DID 模块动态、Sarah 的卡片设计三图、Bob 的教程视频、Bob 的徒步语音、Alice 的长文、Sarah 的作品集、David 的游戏商品 |
| 私人抓取（只在待读）／已分享的抓取 | TechCrunch GPT-5 快照等私人抓取；Bob 分享的 example.org 园艺指南；我分享的 Bloomberg 文章 |
| 普通转发、引用转发、好友评论卡 | Bob 转发 Foodie 的拉面；Sarah 引用转发 Alice 长文；Bob 评论 Alice 长文 |
| 已更新入口（v1→v2，旧版本有评论）、已撤回入口 | Alice 的 DID 模块动态；Bob 的金门大桥照片（我曾收藏） |
| 好友可见动态、对它的评论、他人包裹的受限 ObjId | Alice 的家庭烧烤（我的评论受众 = Alice）；Bob 引用了 David 的好友可见试玩贴（我看到“原文不可见”） |
| 资源四态 | Bob 教程 local、Alice 演示 reachable、David devlog preparing、David boss 战 unavailable |
| 疑似 AI 生成（本地推断）、被过滤 | Show HN（0.62，显示标签）；AI Daily Digest（0.86，被默认规则隐藏）；r/programming（AI 辅助 + 低质量，第二条规则默认关闭） |
| 关注未入列候选三态 | Alice 咖啡笔记（未筛选）、Sarah 配色（未选中）、David 配乐（准备中）、Alice 解析器长文（首次打开失败）、Sarah 字体（已读） |
| 我的发表 | 已发表 5/5、投递中 3/5、部分失败 4/6；关注者、好友、分组、指定 DID 受众各一条 |
| 作者自报 1.2K、本地验证 37 | David 的游戏商品 |
| 来源 | Alice（好友 + 主动）、Bob（仅好友派生）、Sarah（主动）、David（通知重试中）、RSS（不支持通知）、自然语言意图映射 3 个来源 |

读者身份：关注者预览 = Sarah Kim（`did:bns:sarah`），好友预览 = Bob Zhang（`did:bns:bob`，属于“Hiking buddies”分组）。

---

## 10. KRPC 映射说明

Store 的选择（`store/createStore.ts`）：设置了开发覆盖（见 §11）或不在 Mock 运行时（`isMockRuntime()` 为 false）时用服务 store，否则用 Mock store（`?scenario=` 只对 Mock 生效）。服务 store 经 `buckyos.getServiceRpcClient('homestation')` 走 `/kapi/homestation`，上传与媒体用 SDK 的 session token；开发覆盖下直接发 kRPC（`{method, params, sys: [seq, token]}`）。

`HomeStationStoreProvider`（`store/HomeStationStoreProvider.tsx`）在视图挂载时调用 `connect()`：服务 store 先取 `ui.bootstrap`（加载中/失败时显示加载或重试），之后每 3 秒取 `ui.versions`（页面隐藏时暂停，重新可见时立即取一次），变化的领域触发 `onDomains` 重新验证（`evaluation` 视为 `reading`），并刷新 bootstrap（reading、candidates、sources、prefs）、待读摘要、Head 调试数据和已挂载的所有者卡片（published、comments、saved）。写操作完成后同样按领域通知。

| Store | kRPC / HTTP | 说明 |
| --- | --- | --- |
| `connect`；`peekIdentity`、`peekSettings`、`peekGroups`、`peekMuteRules`、`peekFilterRules`、`peekTopics`、`peekSyncStatus`、`peekHiddenSummary`、`peekCollector`、`peekPreviewReaders`、`peekMuteCandidates`、`owner` | `ui.bootstrap` | `peekCollector` 取 `settings.collectors[0]`，评论区收录者视图键为 `collector:<DID>`；`peekPreviewReaders` 的“关注者”取 `followers` 中第一个不是好友的人（没有则取第一个），“好友”取 `friends[0]`，没有时隐藏该预览项；`peekMuteCandidates` = `friends` + `following` + `identities` 中的个人 |
| 变化跟踪 | `ui.versions` | 每 3 秒 |
| `peekReadingSummary` | `reading.summary {query}` | 按查询缓存，变化后重取 |
| `peekEntries`（Head 调试） | `published.entries` | |
| `peekCard`、`watchCard` | `item.cards {objIds, reader}` | 缓存主要来自列表响应的 `cards`；未命中时按读者补取 |
| `listReading` | `reading.list {query, cursor, limit: 8}` | |
| `listFollowedCandidates` | `candidates.list {cursor, includeRead, limit: 8}` | |
| `openCandidate` | `candidates.open {objId}` | |
| `listPublished` | `published.list {owner, reader, kind?, cursor, limit: 10}` | 不传 `kind` 即主页的 feed 种类；`owner` 不是自己时由服务读取对方 HomeStation，读者为他人时只能按匿名近似，返回 `readerApproximated: true`，访客视图显示说明 |
| `listChanges` | `published.changes {reader, after}` | |
| `getItem` | `item.get {objId, reader}` | |
| `getWrappedBody` | `item.wrapped_body {objId}` | |
| `retryResources` | `item.retry_resources {objId}` | |
| `listComments` | `comments.list {objId, view, type}` | |
| `listSaved` | `saved.list {kind}` + `item.cards` | |
| `getProfile` | `profile.get {did, reader}` + `item.cards`（精选） | |
| `setLike`、`setBookmark`、`setReadLater`、`setDislike`、`repost` | `interact.like {objId, on}`、`interact.bookmark {objId, on, public}`、`interact.read_later`、`interact.dislike`、`interact.repost` | 返回 `PersonalState`，直接写回卡片缓存 |
| `quote`、`comment` | `interact.quote {objId, text, audience}`、`interact.comment {objId, text}` | |
| `withdraw`、`editPost`、`setAudience`、`retryDelivery` | `entry.withdraw {entry}`、`entry.edit {entry, text}`、`entry.set_audience {entry, audience}`、`entry.retry_delivery {entry}` | |
| `publish`、`retryPublish`、`shareCapture` | `publish.create {key, input}`、`publish.retry {key}`、`publish.share_capture {objId}` | |
| `uploadAttachment(file, kind)` | `PUT /kapi/homestation/upload?name=&mime=&width=&height=&duration_ms=`（`Authorization: Bearer`） | 图片尺寸、音视频时长在浏览器读出 |
| `fetchLinkPreview` | `publish.link_preview {url}` | |
| `resolveSourceInput`、`follow`、`unfollow`、`pauseSource`、`listSources` | `sources.resolve {kind, text}`、`sources.follow {resolution}`、`sources.unfollow {sourceId}`、`sources.pause {sourceId, paused}`、`sources.list` | |
| `setMuteRule`、`setFilterRule`、`setTagOverride`、`markLessLike`、`setDefaultAudience` | `prefs.set_mute_rule {rule, on}`、`prefs.set_filter_rule {rule}`、`prefs.set_tag_override {objId, tag, override}`、`prefs.mark_less_like {objId}`、`prefs.set_default_audience {audience}` | 设置先在本地生效，写完后重取 bootstrap |
| `setFeatured` | `profile.set_featured {order}` | |
| 媒体地址 | `GET /home/objects/<ObjId>/content?access=<token>` | 令牌出现在 URL 中，已知风险 |

UI 尚未使用的服务方法：`candidates.admit`、`item.fetch`、`comments.set_listing`、`comments.sync`、`publish.task`、`profile.set`、`sources.sync`、`prefs.set_topics`、`prefs.set_collectors`、`prefs.set_comments_open`、`feedback.record`、`consumption.*`、`eval.*`、`admin.*`。

## 11. 连接 devnet 开发

后端示例 `devnet` 启动一组互相连通的 HomeStation（`me` 是 Desktop 的所有者，令牌 `tok-me`；alice、bob、sarah、收录者 index 依次占用后续端口，再后一个端口是 RSS 夹具站），并写入种子数据。

```bash
# buckyos/src
cargo run -p homestation --example devnet -- --port 4131 --data-dir /tmp/homestation-devnet --fresh
# buckyos/src/frame/desktop
HS_BACKEND=http://127.0.0.1:4131 pnpm run dev
```

打开 `/homestation?hsDevToken=tok-me`（令牌存入 `localStorage['homestation.dev']`，也可直接写 `{"token":"tok-me","baseUrl":"/kapi/homestation"}`；删除该键回到 Mock）。访客门户：`/homestation/u/did:test:alice?reader=anonymous`。`HS_BACKEND` 让 Vite 把 `/kapi/homestation` 与 `/home/*` 转发到 `me`。

真实后端 e2e：`pnpm exec playwright test --config=playwright.homestation.config.ts`（自行启动 devnet 于 4231–4236 和 Vite；`HS_DEVNET_BIN` 可指定已构建的 devnet）。
