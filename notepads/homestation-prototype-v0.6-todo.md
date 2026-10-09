# HomeStation 原型按架构 v0.6 修改 TODO

> 创建日期：2026-10-09
> 状态：2026-10-09 已实施（第 4–11 节、第 12 节 UI_DATAMODEL 完成）；PRD 同步待用户确认改动清单（第 15 节）；第 13 节按临时做法实现，待用户拍板
> 依据：[HomeStation 架构设计 v0.6](<../doc/homestation/BuckyOS HomeStation 架构设计.md>)、[产品原型设计](../product/homestation/homestation.md)，以及 2026-10-08 对原型的 review。
> 本轮任务：只改 Mock 原型、UI DataModel、Playwright 用例和 PRD。不实现 HomeStation 后端，不接 CYFS dispatch、Message Center 或真实签名。执行前重新核对架构文档，它仍在演进。

## 1. 目标与范围

现在的原型是“一张卡片就是一个对象”的普通社交 App 模型：[types.ts](../src/frame/desktop/src/app/homestation/types.ts) 的 `FeedObject` 把作者签名的内容和本地私有状态混在一起，架构最核心的几样东西在 UI 上都没有位置。本轮让原型表达 v0.6 的语义，可演示、可评审：

- 不可变对象、入口状态（Head）和本地私有投影三层分开（§5.7、§17.4）。
- 评论区与多视图、转发和引用转发、版本与撤回、发表受众、来源与抓取者、资源就绪、本地标签与过滤。
- 发表流（§4.4）作为个人主页和访客视图的唯一数据来源。

保留：桌面中栏 + 右侧信息面板、移动端单栏 + 发布按钮的基本布局，四种阅读模式，三类详情页。

不做（见第 12 节）：后端、AI 派生内容、Topic Dashboard、商品交易页、Reader 插件、广告与消费证明、作者侧通知。

不做兼容：旧 `FeedObject`、`FeedInteractions`、`Source` 等类型直接替换，不保留旧字段。

## 2. 必读资料与修改入口

架构文档中与原型直接相关的章节：

| 章节 | 原型需要的内容 |
|---|---|
| §2.1 | 发表流、待读列表、关注补看、评论列表四者的边界 |
| §4.4 | 发表流：条目是签名 Head，展示读取与变化读取，按读者身份过滤 |
| §4.5 | 受众五档及受限内容规则，点赞默认公开 |
| §5.4–§5.6、E01–E17 | 入口与 Head、自包含／包裹、URL 只作 `link`／`source`、私人抓取不进发表流 |
| §7.6–§7.7 | 关注依据、好友默认关注、“不看”规则、来源状态 |
| §8.1、§8.5 | 推荐理由、有效标签与过滤规则 |
| §9.2–§9.7 | 游标、关注补看、阅读模式不改变查询、门户、媒体发布 |
| §12–§16 | 评论即对象、三种评论视图、可复算统计、编辑撤回语义 |
| §17.4–§17.5 | UI 能力清单与原型功能映射 |
| §20 | 验收场景，第 11 节的用例按编号对应 |

参照实现：

- MessageHub 的分层：`protocol/`（协议镜像）、`datamodel/`（UI 投影）、`mock/store.ts`（统一 mock store）、[UI_DATAMODEL.md](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。
- 查看器：[ContentPreview.tsx](../src/frame/desktop/src/components/ContentPreview.tsx)。
- 流程规范：[webui-prototype skill](../harness/SKILLS/webui-prototype/SKILL.md)，包括 mock 延迟、五种状态、zod 输入 schema、i18n。

允许修改的入口：

| 范围 | 文件或目录 |
|---|---|
| 原型代码 | `src/frame/desktop/src/app/homestation/` |
| 路由（访客视图） | `src/frame/desktop/src/App.tsx` 中 `/homestation` 相关路由 |
| 文案 | 新建 `src/frame/desktop/src/i18n/homestation.ts`，在 `dictionaries.ts` 中合并（与 `messagehub.ts` 相同方式） |
| 回归 | 新建 `src/frame/desktop/tests/e2e/pages/homestation.spec.ts` |
| 模型文档 | 新建 `src/frame/desktop/src/app/homestation/UI_DATAMODEL.md` |
| 产品文档 | `product/homestation/homestation.md` |

不引入新依赖。

## 3. 已确定的设计：不要按旧假设做

| 主题 | v0.6 | 原型做法 |
|---|---|---|
| 对象与私有状态 | 签名对象不含 `isLiked`、计数、推荐理由、本地标签 | 拆成第 4 节的 protocol / datamodel 两层 |
| 媒体引用 | 只能是 ObjId（FileObject），mime、尺寸、时长在 FileObject meta | `FeedMedia.url` 改为 `object: ObjId`，UI 经 mock resolver 解析成地址 |
| URL | 只能是 `link`（跳转）或 `source.original_url`（出处） | 删除 `originalUrl` |
| 身份 | 签名者（publisher）、原作者、抓取者不同 | 删除 `author.sourceType`；外部内容同时显示抓取者与原作者 |
| 编辑与撤回 | 新版本是新 ObjId，入口 Head `seq` 递增；撤回是 withdrawn Head | 卡片显示“已更新”“已撤回”，互动绑定具体版本 |
| 计数 | 指定视图中有效记录的派生值，作者自报只算“声明” | 计数标明范围；评论数默认只算正文评论 |
| 受众 | 公开／关注者／好友／联系人分组／指定 DID，不写入对象 | 发表时选择、始终可见；受限内容不可转发；对受限内容的评论与点赞只给原作者 |
| 点赞 | 默认公开发表 | 首次点赞时告知“点赞是公开的” |
| 收藏 | 默认私有，可选公开 | 菜单提供“公开收藏” |
| 私人抓取 | 只是本地候选，不进发表流 | 外部条目提供“分享到我的主页”，分享后才进发表流 |
| 自己发表 | 发表不等于投递给自己；上传失败不能显示成功 | 不插进待读列表；进“我的发表”并显示发表与投递状态 |
| 阅读模式 | 只是展示方式，不丢弃分类、Topic 与搜索条件 | 沉浸式使用当前视图的查询 |
| 个人主页 | 就是发表流的展示读取，含评论与转发 | 主页从发表流读，不从待读列表按作者过滤 |

## 4. P0：数据层重构（其余各项都依赖它）

### 4.1 分层

```text
homestation/
  protocol/feed.ts     跨节点对象：FeedObject、FeedHead、FollowDeclaration（字段取自 §5.8–§5.11 示例，非冻结线格式）
  datamodel/*.ts       UI 投影，见 4.2
  mock/data.ts         seed，见 4.4
  mock/store.ts        统一 mock store，见 4.3
```

- [x] `protocol/feed.ts`：`kind`、`comment_type`、`publisher`、`iat`（秒）、`entry?`、`content`、`wraps?`、`references`、`tags`（作者标签）、`source?`、`link?`、`publication_category?`、`base_on?`；`FeedHead { entry, seq, state: 'active' | 'withdrawn', current? }`。
- [x] 删除旧类型：`FeedMedia.url`、`originalUrl`、`FeedInteractions`、`FeedAuthor.sourceType`、对象上的 `recommendReason`、`topics`、`sourceId`，以及未使用的 `HomeStationState`、`MobileBottomTab`。

### 4.2 UI 投影

- [x] `FeedItemView`：`objId`、对象本身、解析后的发表者／原作者／抓取者、`verification`（mock：`verified` / `unverified` / `wrapper_only`）。
- [x] `EntryState`：`seq`、`state: 'active' | 'withdrawn' | 'conflict'`、`currentObjId`，以及当前看到的是否最新版本。
- [x] `ReadingEntry`：`admittedAt`、`reason { code, text, refs }`、`effectiveTags`（每项带来源 author/model/user、状态 inferred/confirmed、作用范围）、`resources: 'local' | 'reachable' | 'preparing' | 'unavailable'`。
- [x] `PersonalState`：点赞、收藏、稍后再看、转发各自的 `public | author_only | private` 和投递状态。
- [x] `InteractionStats`：按 `view: 'local' | 'author' | collector:<id>` 分开，带 `asOf`、同步状态；作者自报的数字单列 `claimed`。
- [x] `CandidateEntry`（关注补看用）：`selection: 'unscreened' | 'not_selected' | 'preparing'`、来源路径、首次入列记录。
- [x] `AudienceSpec`、`PublishTask`（`uploading | published | delivering | partially_failed`，`delivered/total`）、`SourceView`（关注依据、通知状态、最近成功与失败原因、暂停）、`FilterRule`、`CommentView`（含来源路径、所属视图、是否针对旧版本）。

### 4.3 mock store

按 webui-prototype skill，异步并带 300–800ms 延迟。接口按职责命名（不冒充后端已有）：

- [x] 待读：`listReading(viewKey, cursor)`、`listFollowedCandidates(cursor)`、`openCandidate(objId)`（按需准备，可能失败）。
- [x] 发表流：`listPublished(ownerDid, { reader, mode: 'display' | 'changes', cursor })`，**按 reader 身份执行受众过滤**；访客视图和“我的发表”都用它。
- [x] 详情与评论：`getItem(objId)`、`getWrappedBody(objId)`、`listComments(objId, { view, type })`。
- [x] 互动：`setLike`、`setBookmark({ public })`、`setReadLater`、`repost`、`quote`、`withdraw(entry)`。每次状态变化在 store 内递增该入口的 Head `seq`，可在 UI 调试面板中查看。
- [x] 发表：`publish(input)`、`editPost(entry, input)`、`setAudience(entry, audience)`、`shareCapture(objId)`。支持失败注入（例如 `?scenario=publish-fail`），并用幂等键保证重试不重复。
- [x] 来源与偏好：`resolveSourceInput(kind, text)`、`follow`、`unfollow`、`pauseSource`、`setMuteRule(person | group)`、`setFilterRule`、`setTagOverride`。

### 4.4 seed 必须覆盖的情形

尽量沿用现有 mock 的内容与人物，补齐以下情形：

| 情形 | 架构依据 |
|---|---|
| 文字、多图、视频（包裹）、语音、长文（包裹 markdown 正文）、作品链接卡片、商品（DID 入口） | E01–E08 |
| 私人抓取的外部文章（只在待读，不在发表流）；另一条已被用户分享的抓取（E06） | §5.6 |
| 普通转发、引用转发、好友评论卡（带原文引用） | E09、E12、E13 |
| 已更新的入口（P1→P2，旧版本上有评论）、已撤回的入口 | E14、E15 |
| 好友可见的动态；针对它的评论（只给作者）；他人包裹了它的 ObjId | §4.5 |
| 资源分别处于本地／可达来源／准备中／不可用 | §8.3 |
| 本地推断“疑似 AI 生成”的条目，以及被过滤规则隐藏的条目 | E19、§8.5 |
| 来自关注但未入列的候选，三种状态各至少一条 | E18、§9.5 |
| 自己的发表：已发表、投递中 3/5、部分失败 | §9.7 |
| 作者自报 1.2K 赞但本地只验证到 37 | §15.5 |
| 好友派生的关注、主动关注、RSS（不支持关注通知）、自然语言订阅映射到多个来源 | §7.6–§7.7 |

## 5. P0：卡片与状态表达

- [x] 卡片头部显示签名者及该对象的验证结果。外部内容显示“由 Bob 分享 · 抓取自 example.org · 原作者 X”。修正 [FeedCard.tsx:56](../src/frame/desktop/src/app/homestation/FeedCard.tsx) 的 `SourceBadge`：现在只看 `sourceType === 'did'`，未验证的作者也挂着盾牌。
- [x] 嵌套卡：转发／引用转发显示“Bob 转发了”及内层原作者卡片；评论卡显示“Bob 评论了”及原文引用区。内层不可读时显示“原文不可见”，已撤回时显示“原文已撤回”。
- [x] 版本状态：“已更新”标记可跳到最新版本；撤回占位；评论标记“针对旧版本”。
- [x] 资源状态：未就绪的视频不显示播放按钮与时长，改为准备中／不可用及重试（A06）。
- [x] 标签：作者标签与“疑似 AI 生成（本地推断）”分开显示，点击可查看依据与纠正（A51、A53）。
- [x] 计数：标明范围（如“本地已验证”）；作者自报数字只能标为“作者声明”；评论数默认只算正文评论。
- [x] 受限条目显示受众标记（如“好友可见”）。
- [x] 推荐理由来自 `ReadingEntry`，点击后说明命中的关注、主题或规则（PRD P1 关键交互）。

## 6. P0：互动、评论与发布

### 6.1 互动

- [x] 点赞：默认公开，首次点赞时用对话框或提示告知；对受限对象点赞时提示“仅作者可见”。
- [x] 转发：改为“转发／引用转发”二选一并二次确认（PRD §10.2）。转发后出现在“我的发表”里；取消转发即撤回自己的转发入口。受限对象禁用转发，并说明原因。
- [x] 收藏默认私有，菜单提供“公开收藏”；收藏和稍后再看列表中，目标已撤回的条目显示状态（A29）。
- [x] 点踩：语义未定（第 13 节）。先按“仅本地反馈、不对外发表”实现，并在 UI 上注明。
- [x] 补全现在为空的 `…` 菜单（[FeedCard.tsx:232](../src/frame/desktop/src/app/homestation/FeedCard.tsx)）：不看 TA、不看 TA 所在分组、减少类似、为什么推荐、纠正标签、查看原文（`link`／`source`）、复制对象 ID。移动端也可以长按触发（PRD P1）。

### 6.2 评论区（与普通社交 App 最大的差异，优先做）

- [x] 详情页底部：评论列表与输入框。
- [x] 视图切换：本地合并／作者视图／收录者视图（mock 至少一个收录者，覆盖范围与作者视图不同）。
- [x] 类型筛选：正文、点赞、转发、引用转发。
- [x] 每条评论可展开来源路径（如“来自作者列表、收录者 X”），并标记“针对旧版本”。
- [x] 本地有但作者视图未列出的评论，只表述为“作者视图当前未列出”，不能写成“已被删除”（§16.3、A16）。
- [x] 发表的评论出现在“我的发表”里；被评论对象受限时，提示该评论只有作者可见。
- [x] 多级回复关系未冻结（§12.2），本轮不做。

### 6.3 发布

- [x] 发布面板始终显示当前受众，可在五档中选择，默认值取自 mock 设置（第 13 节）；用 zod 定义输入 schema。
- [x] 附加图片、视频、语音：mock 上传得到 FileObject ObjId 后才能发布；“从剪贴板/URL 导入”生成链接卡片。
- [x] 发布状态：上传中、已发表、投递 n/m、部分失败可重试。失败不显示为成功，重试不产生第二条（A41）。
- [x] 修改 [HomeStationView.tsx:167](../src/frame/desktop/src/app/homestation/HomeStationView.tsx) 的 `handlePublish`：不再插进待读列表，改为进入“我的发表”并给出提示。
- [x] 自己的发表可以编辑（产生新版本）、删除（撤回），界面说明已传播的副本无法收回。
- [x] 调整受众：扩大后新读者可见；缩小时说明不能收回已取得的副本，需要时应撤回（§4.5）。
- [x] 私人抓取条目提供“分享到我的主页”，分享后它才出现在发表流（A72）。
- [x] 桌面右侧面板的快速发布与发布面板共用同一输入模型和受众。

## 7. P1：来源、关注与过滤

- [x] SourceManager 的输入要真正生效（mock 解析）：
  - 按 DID 或名字关注：显示通知状态。
  - 粘贴 URL：显示解析出的来源、可信度、预计更新方式（PRD P8）。
  - 自然语言：显示 Agent 维护的来源映射（可能多个）及采集状态。
- [x] 来源行显示：关注依据（主动／好友派生）；通知状态（对方已知情／来源不支持关注通知／重试中）；最近成功时间与失败原因。“暂停采集”和“取消订阅”分开（A42、A45）。
- [x] 好友派生的关注不能单独取消，界面引导使用“不看 TA 的朋友圈”（§7.6）。
- [x] “不看某人／某分组”设置页：同时作用于主 Feed 和关注补看，不改变关注与好友关系（A35、A36）。
- [x] 关注补看：在“已关注”视图底部提供入口“另有 N 条来自关注、尚未进入 Feed”。列表显示 未筛选／未选中／准备中；打开时按需准备；读过的默认不再出现（A37、A38）。
- [x] 过滤规则：完全 AI 生成、AI 辅助、低质量可组合，并指定未知状态如何处理。显示“已隐藏 N 条”，可显式查看被过滤的条目（A52、A56）。
- [x] 右侧信息面板：常驻显示生效中的排除规则和隐藏数量。[InfoPanel.tsx:50](../src/frame/desktop/src/app/homestation/InfoPanel.tsx) 的 Trending Topics 标注为“你的 Feed · 近 7 天”（§7.7）。增加同步状态：候选数量、准备中数量、最近抓取时间。

## 8. P1：主页、访客与导航

- [x] 个人主页改读 `listPublished`，包含正文、评论（带原文引用）、转发。作品／商品按 `publication_category` 分栏；精选是用户维护的引用顺序。
- [x] 访客视图：新增 mock 路由（例如 `/homestation/u/:did`），用 `listPublished` 按读者身份过滤；不显示推荐理由和个人互动状态，提供关注按钮。
- [x] 主人可以“以访客身份预览”。开发模式下可以切换读者为 匿名／关注者／好友，用来演示受众过滤（A43、A69）。启用 `types.ts` 中已定义但未使用的 `ViewPerspective`。
- [x] 关注者数标明是“已收到的有效关注声明”。
- [x] 桌面端接上现在没有被引用的 [SidebarPanel.tsx](../src/frame/desktop/src/app/homestation/SidebarPanel.tsx)（PRD P3 左栏）：我的主页、我的发表、Topics、来源、收藏、稍后再看。现在桌面端到不了主页、来源管理和收藏。
- [x] 移动端保证新页面（我的发表、收藏、稍后再看、关注补看、过滤规则、来源）都有入口。是否改成 PRD 的底部五栏导航，见第 13 节。

## 9. P1：阅读模式与详情

- [x] 沉浸式模式使用当前视图的 filter、Topic 和搜索条件（§9.6、A44）。现在它直接取全部视频（[HomeStationView.tsx:216](../src/frame/desktop/src/app/homestation/HomeStationView.tsx) 与 238 行）。图片作为视觉卡片；长文先做简单分页，不做 AI 改写；补上空态。
- [x] 长文详情通过 `getWrappedBody(objId)` 异步加载被包裹的正文，有加载、准备中、不可用三种状态，不再读卡片里的 `body`。
- [x] 作品和商品的主操作打开 `link`（新标签页），不再落到图片详情；卡片上的外链按钮补上处理函数。
- [x] 图片、视频、文档查看优先复用 `ContentPreview.tsx`。
- [x] 搜索框标明“在当前视图中搜索”。
- [x] 每个视图（默认、各 filter、各 Topic）各自记住滚动位置，切换回来时恢复（§9.2、A07、A08）。

## 10. P2：杂项

- [x] 新建 `i18n/homestation.ts`，提供 en 与 zh-CN。现在字典里没有任何 `homestation.*` 词条，界面全靠英文 fallback；`formatTimeAgo` 中的 `'just now'` 等也要走 i18n。
- [x] `formatCount`、`formatDuration` 在多个文件里重复，统一放到一处。
- [x] 浅色和深色主题都截图核对；选中、按下状态从 `--cp-*` 按比例派生，不直接用 accent-soft 原色铺底。

## 11. 验证

在 `src/frame/desktop/` 下运行：

```bash
pnpm check
pnpm lint
pnpm build
pnpm test:e2e tests/e2e/pages/homestation.spec.ts
```

`homestation.spec.ts` 至少覆盖以下场景（括号内为架构 §20 编号）：

- [x] 视频资源未就绪时不显示可播放（A06）。
- [x] 收藏默认私有，访客视图中看不到（A11、A43）。
- [x] 旧版本上的评论标记“针对旧版本”；已撤回条目显示占位，收藏仍保留（A17、A29）。
- [x] 设置“不看某人”后，主 Feed 与关注补看都隐藏其条目，关注关系不变（A35）。
- [x] 关注补看中打开候选后，默认列表不再显示它（A38）。
- [x] 发布失败不显示成功，重试后只有一条（A41）。
- [x] 匿名、关注者、好友三种读者看到的发表流不同（A69）。
- [x] 好友可见的条目不能转发；被他人包裹时，受众以外的读者看到“原文不可见”（A70）。
- [x] 对受限条目的评论不出现在评论者的公开主页（A71）。
- [x] 私人抓取不出现在发表流，分享后出现（A72）。
- [x] 首次点赞出现“点赞是公开的”提示（A74）。
- [x] 显式查看被过滤条目；调整过滤规则后列表重算，滚动位置稳定（A56）。
- [x] 切换到沉浸式后仍保持当前 filter 与 Topic（A44）。
- [x] 移动端 375px 走通主流程，无水平滚动；全程无 console error。

## 12. 文档

- [ ] 原型收敛后按 webui-prototype skill 第三阶段写 `UI_DATAMODEL.md`：协议层、UI 投影、mock store 三层，以及五种状态、分页与游标、字段稳定性。KRPC 映射一节指向架构 §17.1 与 §17.4。
- [x] 同步 PRD `product/homestation/homestation.md`：
  - §5.1 “FeedObject 包含指向原始页面的 URL”改为 `link`／来源信息。
  - §14.3 “素材先作为多个 FeedObject 存在”改为素材是草稿状态的 FileObject，不是已发表对象（架构 §9.7）。
  - 补充：发表受众、标签评价与 AI 生成过滤、关注补看、版本与撤回、评论多视图、点赞公开、私人抓取不进主页。
  - 改 PRD 前先把改动清单给用户确认。

## 13. 需要先问用户的

以下是产品决定，执行者不要自行拍板；未确认前按括号内的临时做法实现，并在代码和 UI_DATAMODEL 中标注“待确认”：

- [ ] 正文、评论、转发的默认受众（临时：公开，设置中可改）。
- [ ] 点踩的语义（临时：仅本地反馈）。
- [ ] 移动端是否改为 PRD 的底部五栏导航（临时：保留现有结构，只补入口）。
- [ ] 公开门户最终由 Zone 根 `$` 上的 HomeStation 服务提供，原型阶段在 Desktop 内用路由模拟访客视图，是否可以。

## 14. 本轮不做（等架构或后续阶段）

| 项目 | 原因 |
|---|---|
| AI 派生内容（配音、长文智能分页、Agent 摘要） | 架构尚未定义派生内容层 |
| 作者侧互动通知、评论审核入口 | 架构尚未定义接收策略与通知面 |
| Agent 以自己的 DID 发表摘要 | 架构未写入 |
| 加入 Group／Channel／Topic | 架构仅在 review 中提出映射，未写入 |
| Topic Dashboard、商品交易页、Reader 插件、广告与消费证明 | PRD 第二、三阶段 |
| 真实后端、CYFS dispatch、签名验证 | 本轮只做 mock |

## 15. 执行记录（2026-10-09）

### 15.1 落点

| 层 | 文件 |
| --- | --- |
| 协议镜像 | `src/frame/desktop/src/app/homestation/protocol/feed.ts` |
| UI 投影 / 输入模型 / 格式化 | `datamodel/types.ts`、`datamodel/inputs.ts`、`datamodel/format.ts` |
| Mock | `mock/data.ts`（seed，覆盖第 4.4 节全部情形）、`mock/store.ts`（异步 API + 同步 peek + 领域通知）、`mock/media.ts`（ObjId → SVG 占位地址，注册 `homestation-file` 给 ContentPreview） |
| 卡片 | `card/FeedCard.tsx`、`card/parts.tsx`、`card/ActionBar.tsx`、`card/ItemMenu.tsx`、`card/ReasonDetail.tsx`、`card/labels.ts` |
| 页面 | `feed/FeedPage.tsx`、`detail/ItemDetail.tsx`、`detail/CommentSection.tsx`、`publish/*`、`candidates/FollowedCandidates.tsx`、`saved/SavedList.tsx`、`source/SourceManager.tsx`、`prefs/FeedPreferences.tsx`、`PublicProfileView.tsx`、`MePage.tsx`、`ImmersiveMode.tsx` |
| 外壳 | `HomeStationView.tsx`（桌面三栏 / 移动单栏）、`HomeStationVisitorRoute.tsx`（`/homestation/u/:did`，`App.tsx` 注册） |
| 文案 | `src/frame/desktop/src/i18n/homestation.ts`（532 个 key，en + zh-CN），`dictionaries.ts` 合并 |
| 回归 | `src/frame/desktop/tests/e2e/pages/homestation.spec.ts`（16 例） |
| 模型文档 | `src/frame/desktop/src/app/homestation/UI_DATAMODEL.md` |

### 15.2 验证（在 `src/frame/desktop/` 下）

- `pnpm check`：通过。
- `pnpm lint`：0 error；8 个 warning 都在 filebrowser / users-agents / e2e fixtures，与本任务无关。
- `pnpm build`：通过（chunk 体积告警为原有）。
- `npx playwright test tests/e2e/pages/homestation.spec.ts`：16 例全部通过，连续两次（约 15 秒）；全程无 console error。
- 另用脚本截图核对：桌面浅色 / 深色、zh-CN、375px 的“我”页 / 发布页 / 访客页、Desktop 窗口内打开 HomeStation 与对话框。

### 15.3 与任务描述的差异

- `listReading(viewKey, cursor)` 实现为 `listReading(query, cursor)`，视图键由 query 序列化得到；`listPublished` 的 `mode: 'changes'` 拆成独立的 `listChanges`（两种读取返回结构不同）。
- 视频、语音没有真实字节，详情页是模拟播放器；只有图片走 ContentPreview。
- 推荐理由在 UI 按 `code` + `refs` 本地化，store 的 `reason.text` 作为“推荐服务说明”原样展示。

### 15.4 待用户确认的 PRD 改动清单（未改 PRD）

1. §5.1 FeedObject：删去“指向原始页面的 URL”，改为“跳转链接（link）与来源信息（原始出处、原作者、抓取者）；媒体以对象 ID 引用；交互状态、推荐理由、本地标签不属于 FeedObject，是读者本地状态”。
2. §7.1 公开视角 / §15 P2：公开主页 = 发表流按访客身份的展示读取，包含正文、评论（带原文引用）和转发；“只展示公开内容”改为“只展示该访客受众允许的内容”；私人抓取、待读、收藏和推荐理由都不出现。
3. §8 新增“标签评价与 AI 生成过滤”：作者标签与本地推断分开展示；“疑似 AI 生成”标明本地推断并可查看依据、纠正；完全 AI 生成 / AI 辅助 / 低质量可组合过滤，指定未知状态处理；显示“已隐藏 N 条”并可显式查看。
4. §9 新增“关注补看”：关注提高采集确定性但不保证入列；“已关注”视图底部提供补看入口；条目显示未筛选 / 未选中 / 准备中；打开时按需准备，读过默认不再出现；“不看某人 / 分组”同时作用于主 Feed 和补看，不改关注与好友关系。来源行显示关注依据、通知状态、最近成功与失败原因；暂停采集与取消订阅分开；好友派生关注不能单独取消。
5. §10.1 可见性差异：点赞默认公开发表、首次点赞时告知；收藏默认私有、可选公开；稍后再看仅自己；点踩语义待定（暂为本地反馈）。
6. §10 新增“发表受众”：公开 / 关注者 / 好友 / 联系人分组 / 指定 DID，发布时始终可见；受限内容不能转发；对受限内容的评论与点赞只给原作者，不进入公开主页；缩小受众不能收回已取得的副本。
7. §10 新增“版本与撤回”：编辑产生新版本，旧版本上的评论与点赞不算作对新版本的认可；卡片显示“已更新”“已撤回”；撤回不能收回已传播副本。
8. §10.3 评论：补充本地合并 / 作者 / 收录者三种视图、类型筛选、来源路径，以及“作者视图当前未列出”不等于“已删除”。
9. §14.1 / P7 发布：发布面板显示受众与发布状态（上传中、投递 n/m、部分失败可重试），失败不显示成功、重试不重复；自己的发表进入“我的发表”，不插入自己的待读列表。
10. §14.3：“这些素材先作为多个 FeedObject 存在”改为“素材先作为草稿状态的文件对象保存，不是已发表对象，组织完成后才发表”。
11. §9.3 / §14.2：私人抓取只是本地候选，不进入主页；用户“分享到我的主页”后才以自己的入口发表。
12. §16.3 发布流程图：“进入自己主页与他人可见流”改为“写入自己的发表流（按受众），并投递给受众”。

