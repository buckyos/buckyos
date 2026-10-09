# BuckyOS HomeStation 协议与实现

> **版本**：v0.1
> **日期**：2026-10-09
> **文档定位**：《BuckyOS HomeStation 架构设计》v0.6 的首版实现说明，面向实现与联调团队。记录线格式、跨节点接口、Owner kRPC、持久数据、运行方式与验证方法，以及对架构 §21 待定项采取的实现选择。
> **代码**：`src/frame/homestation`（crate `homestation`）；Desktop 前端在 `src/frame/desktop/src/app/homestation`。
> **约定**：本文的“实现选择”都可以在宣布网络格式稳定前修改；凡与架构不一致之处以架构为准，并在 §9 列出。

## 1. 范围与落点

首版在一个服务进程内实现架构 §4.3 的全部逻辑模块，按文件划分：

| 模块（架构 §4.3） | 文件 | 说明 |
| --- | --- | --- |
| 对象与校验 | `protocol.rs`、`objects.rs`、`sign.rs` | Feed Object / Head / 关注声明 / 消费证明 / 共享评价；ObjId、JWT、入口命名空间校验；本地对象库与 chunk 存储 |
| 定位与密钥 | `directory.rs` | DID → Zone → HTTP 源站；签名者授权；单机配置与名字服务两种实现 |
| Publish Service | `publish.rs` | 签发对象与 Head（串行 seq）、发表流变化序列、读取授权、发布任务 |
| 发表流读取 | `stream.rs` | 展示读取、变化读取、条目 Head、对象 / chunk、作者评论视图、门户资料、收录索引 |
| Social Ingress | `ingress.rs` | CYFS dispatch 接收、准入、幂等；Push 与 Pull 共用的入库与 Head 合并 |
| Delivery | `delivery.rs` | 发件箱、dispatch 发送、结果分层、重试 |
| Pull | `pull.rs` | 关注对象的变化同步、按 ObjId 取对象与 chunk、远端入口解析、收录者冷启动 |
| Subscription / Source Manager | `sources.rs` | Follow / URL / 自然语言来源，关注声明，好友派生关注 |
| Spider Runtime | `spider.rs` | RSS / Atom 与网页链接卡片、快照 |
| Evaluation Service | `evaluation.rs` | 身份 / 内容按需评价、路径绑定、人工修正、变化通知、共享 |
| Selection / Reading List | `selection.rs`、`resources.rs` | 有效标签与过滤、不看规则、打分、资源准备、待读列表、关注补看 |
| Comment Sync / Index | `comments.rs`、`interact.rs` | 评论与特殊评论、多维护者视图、统计、讨论追踪 |
| Local Feedback / Receipt | `feedback.rs` | 临时行为事件、消费约定与证明 |
| Web UI 支撑 | `projection.rs`、`api.rs`、`http.rs` | UI 投影、Owner kRPC、HTTP 路由 |

运行形态（`main.rs`）：

- **BuckyOS 服务模式**：kernel service `homestation`，端口 4130；Owner kRPC 经网关通用路由 `/kapi/homestation`；协议路径 `/home/*` 由 `boot_gateway.yaml` 转发到本服务。
- **单机模式**：`homestation --data-dir <dir> --config <json>`，身份、密钥、对端与联系人都写在配置里（§7.2），用于开发与多节点测试。

一个 HomeStation 实例服务一个发表者身份：服务模式下是 Zone 的 owner（ZoneDocument.owner）。同 Zone 的其他用户只能以读者身份访问（§9）。

## 2. 对象与线格式

### 2.1 通用规则

- ObjId = `<obj_type>:<hex(sha256(JCS(claims)))>`，与 ndn-lib `build_named_object_by_json` 一致；对象内引用一律写这种 hex 形式，接收时也接受 base32 并归一。对象内不允许浮点数（JCS 失败时锁定版本的 ndn-lib 会静默哈希 `{}`，因此先行拒绝）。
- 签名对象以 JWT（EdDSA）承载，claims 就是对象本身，header `kid = <签名者 DID>#<key id>`。ObjId 只由 claims 计算，与是否签名无关。
- 签名者可以是 `publisher` 本人、其 DID Document `authentication` 列出的密钥，或 `owner == publisher` 的设备。服务模式用 OOD 设备私钥签名（`<device DID>#main_key`）；单机模式用配置中的密钥。
- 大小上限：对象 ≤ 64 KiB（dispatch 默认值）；内联正文（text + title + summary）≤ 8000 字符；部件 ≤ 9；标签 ≤ 16。

| obj_type | kind | 用途 |
| --- | --- | --- |
| `cyfeed` | `post` / `comment` | Feed Object（架构 §5） |
| `cyfhead` | `feed_head` | 入口状态 Head（§16.5） |
| `cyfollow` | `follow` | 关注声明（E17） |
| `cyfproof` | `consumption_proof` | 消费证明（§11.3） |
| `cyfeval` | `evaluation` | 对外共享的评价（§6.5） |
| `cyfile` | — | 部件与包裹对象（ndn-lib FileObject，`mime`/`width`/`height`/`duration_ms` 平铺在 meta） |

### 2.2 Feed Object（`cyfeed`）

字段沿用架构 E01–E13：`kind`、`comment_type`（`text`/`like`/`dislike`/`bookmark`/`repost`/`quote`）、`publisher`、`iat`（Unix 秒）、`nonce`、`entry`、`content`（`type`、`text`、`title`、`summary`、`cover`、`media[{object, alt}]`）、`wraps`、`references[{relation, object_id}]`、`tags`、`source{kind, original_url, original_author, captured_at_ms}`、`link`、`publication_category`（`work`/`product`）、`base_on`。

接收方的结构校验（`validate_feed_object`，A48）：

- `post` 无 `comment_type`；`comment` 必须有。
- `wraps`、`base_on`、`cover`、`media[].object`、`references[].object_id` 必须是 ObjId，出现 `://` 即拒绝；`link` 与 `source.original_url` 必须是 http(s) URL。
- `text` 评论恰好一个 `comment_on` 且有正文；`like`/`dislike`/`bookmark` 恰好一个 `comment_on`、不包裹；`repost` 包裹目标；`quote` 包裹目标且有正文；`post` 自包含或包裹。
- 评论目标：`repost`/`quote` 为 `wraps`，其余为 `comment_on`。

### 2.3 Head（`cyfhead`）

```json
{ "kind": "feed_head", "publisher": "<DID>", "entry": "<入口>", "seq": 3, "state": "withdrawn", "updated_at_ms": 1791433800000 }
```

`active` 必须带 `current`，`withdrawn` 不能带。**实现选择**：比架构 E14 示例多一个 `publisher` 字段，用来直接验证“签名者被入口控制者授权”和“入口位于该发表者命名空间”，无需从 Zone 反查 DID。

### 2.4 入口与键

- 入口 URL：`cyfs://<zone>/home/<namespace>/@/<key>`，namespace 为 `feed`（正文、评论、引用转发）、`reactions`（点赞、点踩、公开收藏、普通转发）、`follows`（关注声明）。键字符集 `[A-Za-z0-9._-]{1,128}`。DID 形式的入口（E08）只接受名字系统解析出 `owner == publisher` 的内容 DID。
- 发表时的键：正文 `p-<随机>`、评论 `c-…`、引用转发 `q-…`、分享抓取 `clip-…`。
- **互动键摘要**（§21）：`<type>-` + `hex(sha256("homestation/reaction/v1\n<互动者 DID>\n<目标 ObjId>\n<type>"))` 的前 16 字节；关注键 `f-` + `hex(sha256("homestation/follow/v1\n<关注者>\n<被关注者>"))` 前 16 字节。
- 入口归属（§5.4 规则 2）：`zone_of(publisher) == 入口的 zone`；不满足时对象按终态对象处理，Head 直接拒绝（A46）。
- 版本归属（规则 3）：只有声明同一入口、发表者相同的对象才记为该入口的版本；Head 的 `current` 指向的对象到达后同样核对。

### 2.5 关注声明、消费证明、共享评价

```json
{ "kind": "follow", "publisher": "<Bob>", "iat": 1791421200, "entry": "cyfs://<Bob Zone>/home/follows/@/f-…", "target": { "publisher": "<Alice>", "stream": "cyfs://<Alice Zone>/home/feed" } }
{ "kind": "consumption_proof", "publisher": "<Bob>", "iat": …, "target": "<ObjId>", "action": "video_complete", "result": { … }, "receiver": "<DID>", "agreement": "agr-…", "nonce": "…" }
```

共享评价把置信度写成整数千分比 `confidence_permille`（对象内不允许浮点数）。

## 3. 跨节点协议

### 3.1 定位（§4.4 发现）

DID → Zone：服务模式先查 owner document 的默认 Zone，失败时用 `DID::to_host_name()`；单机模式查配置。Zone → HTTP 源站：`https://<zone>`，可在服务 settings 的 `peers` 或单机配置的 `directory.zones` 声明。发表流 `cyfs://<zone>/home/feed`，投递入口 `cyfs://<zone>/home/inbox`，`GET /home/profile` 也返回这两个地址。

### 3.2 读者身份

| 方式 | 头 | 读者 |
| --- | --- | --- |
| 无 | — | 匿名，只得公开条目 |
| 读者证明 | `Authorization: DID <JWT>` | 其他 Zone 的用户 |
| 会话令牌 | `Authorization: Bearer <session token>` 或 `?access=<token>` | 同 Zone 用户；owner 本人得到 owner 视图 |

读者证明是自签 JWT：`{ "iss": "<读者 DID>", "aud": "cyfs://<被读 zone>/home", "iat", "exp" (≤ iat+600), "jti" }`，签名者须被 `iss` 授权。HomeStation 代表 owner Pull 时使用自己的签名者。**实现选择**：架构 §21 的“读者身份认证方式”在接入 CYFS 跨 Zone 身份机制前采用这一格式；证明在有效期内可以重放，只用于幂等读取。

### 3.3 HTTP 路径

| 方法与路径 | 说明 |
| --- | --- |
| `GET /home/feed?mode=display[&kind=&cursor=&limit=&objects=1]` | 展示读取：当前有效条目，按 `iat` 倒序；`kind` = `all`/`feed`/`posts`/`comments`/`reposts`/`reactions`/`follows`/`work`/`product`（非 owner 的 `all` 不含互动与关注） |
| `GET /home/feed?mode=changes&since=<cursor>[&limit=&objects=1]` | 变化读取：`since` 之后的 Head 变化（含受众变化，`kind=audience`）；游标早于压缩点时 `resync: true` |
| `GET /home/<feed\|reactions\|follows>/@/<key>` | 条目当前 Head（JWT）；不存在与无权读取同为 404 |
| `GET /home/objects/<ObjId>` | 对象（JWT 或 canonical JSON）；受限条目附 `hs-audience: restricted` |
| `GET /home/objects/<cyfile ObjId>/content` | 单 chunk 文件的内容，供媒体元素使用 |
| `GET /home/chunks/<ChunkId>` | chunk 数据，接收方按 ChunkId 校验 |
| `GET /home/comments?target=<ObjId>[&type=]` | 作者视图（目标是本发表者的版本）或收录者视图（收录模式） |
| `GET /home/profile` | 门户资料：名称、简介、发表流 / 投递入口、关注者数、作品精选（按读者过滤） |
| `GET /home/index/recent?since=&limit=` | 收录者最近收到的公开发表（冷启动） |
| `PUT /home/inbox` | CYFS dispatch 投递（§3.4） |
| `GET /home/inbox?dispatch-status=<ObjId>` | dispatch 状态查询 |

列表响应不签名，条目是签名的 Head。展示 / 变化读取的 `objects` 是“随附”：读者有权读的对象及其部件（ObjId → JWT/JSON），接收方逐个按 ObjId 校验（§5.5）。条目的 `restricted` 与 `tier`（`public`/`followers`/`friends`/`group`/`dids`）是不签名的附带信息。

对象读取授权（A32）：对象必须属于某个读者可见、且未撤回的条目（`object_grants`；FileObject 的授权延伸到其 content chunk，被包裹的 Feed Object 延伸到其部件与 Head）。Head 对象在条目撤回后仍可读，以保留撤回的发现能力。私人对象（私有收藏、私人抓取）只有 owner 可读。

### 3.4 投递入口（CYFS dispatch）

请求：`PUT /home/inbox`，`Host: <zone>`，`Content-Type: application/cyfs-named-object+jwt`，`cyfs-obj-id: <base32 ObjId>`，`cyfs-original-user: <发表者>`；受限条目另带 `hs-audience: <tier>`。每次一个对象：发表时依次投递对象和它的 Head，接收方对两者的到达顺序不作假设（A24）。

接收方顺序：目标归一化（必须是本 Zone 的 `/home/inbox`，否则 404 `no-handler`）→ 内容类型（415）→ 大小（413）→ `validate_cyfs_dispatch_body`（400 `invalid-object`）→ 只接受 JWT（400 `signature-required`）→ 验签（403 `invalid-signature`；签名者密钥暂不可得为 503 `signing-key-unavailable`、可重试）→ 幂等（已接收的 ObjId 直接返回 `accepted`）→ 准入 → 入库。

准入（§7.3）：

| 对象 | 准入条件 | 披露 |
| --- | --- | --- |
| 关注声明 | `target.publisher` 是本 owner | 好友或已关注者 `preferred`，否则 `candidate` |
| 评论 / 互动 | 目标是本 owner 的版本，或是本节点追踪的讨论 | 同上 |
| 正文、转发 | 发送者是好友、已关注对象或配置的收录者 | 同上 |
| Head | 入口已知，或发送者满足上面任一关系，或是本 owner 的关注者 | 同上 |
| 消费证明 | `receiver` 是本 owner | `candidate` |
| 任何对象 | 发送者被拉黑：`rejected not-admitted`；非好友每小时超过 500 次：`rejected rate-limited` | — |

收录模式接受任何人的公开提交，并把公开 Feed Object 记入收录索引。

结果按 ndn-lib `CyfsDispatchResult` 编码：`accepted` 200、`rejected` 4xx/5xx 带 `reason` 与 `retryable`。**实现选择**（§21 披露字段编码）：准入披露写在状态体的附加字段 `admission`（`candidate`/`preferred`），只随 `accepted` 出现；`disclose_admission = false` 时省略。

发送方（§7.4、A73）：`accepted` 记为完成；`cached`、无响应（含 `cyfs-dispatch-error`）、可重试的 `rejected` 继续重试，退避 `5s × 2^n`、上限 1 小时、最多 12 次，之后标记失败，可由用户重试；不可重试的 `rejected` 停止。发布任务的投递进度只统计对象投递，不统计 Head。

Push 对象（§13.1）：

| 发表 | 接收方 |
| --- | --- |
| 正文 / 引用转发 | 受众内已知的人：公开与关注者受众为关注者 ∪ 好友，好友受众为好友，分组为组员，指定 DID 为名单；公开时另加收录者 |
| 评论 | 原作者；目标公开时另加已发现的讨论参与者、评论受众内已知的人、收录者 |
| 点赞 / 公开收藏 | 原作者；目标公开时另加收录者 |
| 普通转发 | 原作者、受众内已知的人、收录者 |
| 关注声明 | 被关注者 |
| 编辑 / 撤回 / 再次互动 | 该条目此前的全部接收方，加当前受众 |

### 3.5 接收方的 Head 规则（§16.6）

每个入口只保留已验证的最高 seq；更低 seq 无论何时到达都忽略（A23）；同 seq 不同内容记录 `conflict_obj_id`，冲突期间该条目不计入互动统计，UI 显示 `conflict`（A30）；签发更高 seq 即消解。Head 可以由任何持有者转交（A50）。对象内容可以淘汰，入口的 Head 保留在 `heads` 表。

## 4. Owner kRPC（`/kapi/homestation`）

请求 `{method, params, sys:[seq, token]}`，成功 `{result, sys:[seq]}`，失败 `{error: "<code>: <message>", sys:[seq]}`（HTTP 200）。可选字段省略而不是返回 `null`（关注补看条目的 `firstAdmittedAt`/`openedAt` 按类型保留 `null`）。会话令牌的 DID 等于 owner 才能调用全部方法；同 Zone 其他用户只能调用 `profile.get`、`published.list`、`item.get`，且以读者身份得到结果。

| 分组 | 方法 |
| --- | --- |
| UI 状态 | `ui.versions`（各领域变化计数，只在数据真正变化时递增，UI 轮询后按需重新验证）、`ui.bootstrap`（含 `followers`、`following`、`friends`、`groups`、`identities`，供预览读者与“不看”选择）、`prefs.get` |
| 待读 | `reading.list {query, cursor, limit}`（含 `cards`）、`reading.summary {query}` |
| 关注补看 | `candidates.list {cursor, includeRead, limit}`、`candidates.open {objId}`、`candidates.admit {objId}` |
| 条目与资源 | `item.get {objId, reader?}`、`item.cards {objIds}`、`item.wrapped_body`、`item.retry_resources`、`item.fetch {objId, from}` |
| 评论 | `comments.list {objId, view: local\|author\|collector:<did>, type}`、`comments.set_listing {target, commentId, listed}`、`comments.sync` |
| 发表流 | `published.list {owner, reader, kind, cursor, limit}`、`published.changes {reader, after}`、`published.entries`（Head 调试）。owner 不是自己时远程读取对方发表流：读者为 owner 时带读者证明，否则匿名读取；以第三人的身份读取无法实现，按匿名返回并标 `readerApproximated: true`；非 owner 读者的卡片不含 `personal`/`reading` |
| 互动 | `interact.like / bookmark {public} / read_later / dislike / repost {objId, on}`、`interact.quote {objId, text, audience}`、`interact.comment {objId, text}` |
| 条目 | `entry.withdraw`、`entry.edit {entry, text}`、`entry.set_audience {entry, audience}`、`entry.retry_delivery` |
| 发布 | `publish.create {key, input}`、`publish.retry {key}`、`publish.share_capture {objId}`、`publish.task {key}`、`publish.link_preview {url}`；上传 `PUT /kapi/homestation/upload?name=&mime=[&width=&height=&duration_ms=]` |
| 收藏 | `saved.list {kind: bookmark\|read_later}` |
| 门户 | `profile.get {did?, reader?}`、`profile.set {name, bio}`、`profile.set_featured {order}` |
| 来源 | `sources.list`、`sources.resolve {kind: follow\|url\|natural, text}`、`sources.follow {resolution}`、`sources.unfollow {sourceId}`、`sources.pause {sourceId, paused}`、`sources.sync {sourceId?}` |
| 偏好 | `prefs.set_mute_rule {rule, on}`、`prefs.set_filter_rule {rule}`、`prefs.set_tag_override {objId, tag, override: remove\|confirm\|to_assisted\|null}`、`prefs.set_default_audience`、`prefs.set_topics`、`prefs.set_collectors`、`prefs.set_comments_open`、`prefs.mark_less_like` |
| 反馈与回执 | `feedback.record {objId, event, value?}`、`consumption.join {target, receiver, action, terms?, joined}`、`consumption.report {agreementId, result?}`、`consumption.list` |
| 评价服务（§6.4） | `eval.evaluate {request}`、`eval.refresh`、`eval.get {id}`、`eval.find {target, profileId?}`、`eval.set_tag_override {target, dimension, tag, action?, replacement?, scope?}`、`eval.overrides {target}`、`eval.changes {since}`、`eval.share {resultId}` |
| 运维 | `admin.run {task: delivery\|pull\|selection\|comments\|friends}`、`admin.compact_stream {through}` |

评价服务供其他应用与 Agent 调用：调用方的 app id 决定修正的作用域，Desktop/control-panel 与 homestation 为 `global`，其他应用为 `app:<id>`（A65）；`eval.set_tag_override` 可以显式要求 `scope: "global"`。

## 5. 业务规则的实现要点

### 5.1 受众（§4.5）

受众只存在于 `published.audience`。判定：匿名只看公开；关注者受众给已收到有效关注声明的人（好友也算，好友默认互相关注）；好友与分组按读取时的 Message Center 关系（缓存 30 秒）；指定 DID 按名单；owner 看全部。点赞默认公开；目标受限时，评论、点赞、公开收藏的受众固定为原作者，且不投递收录者；受限对象不能普通转发或引用转发（`repost_blocked:restricted`）。受众调整不签发 Head，只追加 `audience` 变化；扩大后新读者在变化读取里看到该条目，并收到一次 Push。

### 5.2 待读管线（§8）

1. **候选**：Push 自好友 / 已关注者、关注对象的 Pull、收录者、订阅抓取形成候选，同一 ObjId 一行、来源路径合并（A01）。好友或关注者的正文评论、普通转发、引用转发也进入候选；点赞类不进入。
2. **筛查**：规则档案评价 `topic`、`generation_method`、`quality`、`ad`；有效标签 = 作者标签 + 评价判断 + 用户修正；已撤回的丢弃；过滤规则命中的记为 `filtered`（保留推荐理由，查询时可显式查看）。
3. **打分**（可替换）：好友 3、关注 2.5、订阅 1.5、收录者 1；每个订阅 Topic 命中 +1.5；每小时 −0.05（最多 −2）；本地行为偏好 −2…+1。≥1 才入选，每轮最多 `admission_batch`（12）条。
4. **准备**：私人抓取的链接卡片先做快照；按 ObjId 从发表者、推送者或收录者获取被包裹对象、媒体与封面，chunk 按 ChunkId 校验后存入本地；单个文件超过预取预算（32 MiB）或内容是 chunk list 时记为 `reachable`。封面缺失不阻止入列。
5. **复查与入列**：带正文重新评价、复查过滤，通过后写入待读列表并记录首次入列时间。待读窗口默认 300 条，超出部分移出窗口，入列历史保留（A39）。

“不看”与过滤规则在读取时计算，改规则立即生效；修改过滤规则或标签修正后，被过滤的候选重新参与筛选（A56）。待读查询：`filter`（`all`/`following`/`images`/`videos`/`longform`/`news`）、`topicId`、`search`（在整个视图上搜索标题、正文、作者、原作者与被包裹内容）、`showFiltered`。

关注补看：保留期（14 天）内、来自有效关注或好友、未撤回、未被不看与过滤规则命中、从未入列的候选；`openCandidate` 绕过评分按需准备并记录阅读时间，默认视图不再出现（A38）。

### 5.3 评价服务（§6）

- **目标**：`identity`（DID）；`content` 的 `object_id`、`object_path`（`cyfs://`/`https://` 的入口路径，先取得并验证 Head，固定当前版本）或 `root_object_id + inner_path`（JSON Pointer；成员是 ObjId 时固定为该对象，否则按“根 + 路径”评价，作用范围 `part`）。路径不存在、已撤回或解析失败返回 `state: unknown` 与 `failure`，不作负面判断（A62）。缺本地对象时向入口发表者取证。
- **档案**：`rules`（修订号 `rules-1`）与 `model`（AICC `helper.llm_chat`，修订号 `model-1:<逻辑模型>`，需在 settings 配置 `evaluation_model`）。规则档案只在有证据时下判断：作者声明为 `declared`，规则推断为 `inferred` 并带置信度与依据；没有证据的维度列入 `unknown_dimensions`。
- **复用**：同一目标键、档案与修订号、维度覆盖、未超过 `max_age` / `refresh_after` 时复用；`refresh` 生成新记录，并以 `supersedes` 链接旧记录。
- **修正**：`add`/`remove`/`replace`/`confirm`，作用域为 `global` 或 `app:<id>`；修正不改评价记录，自动重算不会覆盖修正（A53）。变化流 `eval.changes` 记录新结果与修正。
- 异步请求（`async: true`）先入任务队列，由后台执行，经 `eval.get` 查询。

### 5.4 评论与统计（§12–§15）

- 每条评论的来源（push / pull / 作者列表 / 收录者 / fetch）都记录在 `comment_sources`。评论到达作者节点时，若作者开放评论就自动加入作者列表；作者可用 `comments.set_listing` 移除。
- 视图：`local`（全部已验证记录）、`author`（作者节点：作者列表；其他节点：来自作者列表的记录）、`collector:<did>`。`comments.list` 返回当前版本及更早版本的记录，旧版本标 `onOldVersion`。
- 统计只针对精确版本：正文评论与引用转发按逻辑发表（入口）计数，点赞 / 普通转发 / 公开收藏按互动者去重，撤回的不计，冲突的计入 `contested`；作者视图返回的数字只作为 `claimed` 展示（A22）。
- 追踪：参与过的讨论（评论、互动、引用）定时 Pull 作者视图与收录者视图，保存视图快照；只有两次**完整**快照之间消失的记录才标 `authorRemoval: observed`（A16）。

### 5.5 来源与关注（§7.6、§7.7）

- Follow：按 DID 或联系人名称解析，读取对方 `/home/profile` 补充名称；建立来源后发表关注声明并投递，变化读取同步对方发表流（首次用展示读取回填）。
- 好友：每小时与 Message Center 同步一次，好友派生 `friend` 依据并发表关注声明；好友解除后移除该依据，没有其他依据时撤回关注声明并删除来源（A36）。只有好友依据的来源不能单独取消关注，返回 `friend_basis`，界面引导改用“不看”。
- URL：先探测是否是 HomeStation（`/home/profile` 给出 DID），再识别 RSS/Atom 或网页声明的 feed，否则作为网站链接卡片来源；这些来源的通知状态为 `unsupported`（A45）。
- 自然语言：保存意图，生成订阅的 Topic（关键词：拉丁词与中文双字片段），并映射到联系人与收录者频道；意图与映射持久保存（A42）。

### 5.6 Spider（§5.6）

RSS/Atom 条目与网页形成**私人抓取**：owner 名义、无入口、仅本地可见的链接卡片，每个 URL 只抓一次（网站来源在卡片变化时更新）。入选时抓取页面快照（`text/html` FileObject）并以包裹快照的新本地对象替换原卡片。分享时以 owner 自己的入口发表包裹快照的对象，`source` 保留出处，接收方显示 `wrapper_only`。抓取不携带也不保存 Cookie 等会话凭据。

## 6. 持久数据

SQLite（WAL），文件 `<data dir>/homestation.db`，`meta.schema_version = 1`；单机模式的 chunk 存在 `<data dir>/chunks/`，服务模式使用 Zone 命名对象存储（NDM）。处于开发期，格式变化直接升版本、不做迁移。

| 表 | 内容 | 分类 |
| --- | --- | --- |
| `objects`、`feed_index` | 已获取的命名对象（含签名形式、签名者、是否仅本地）及 Feed Object 索引 | 持久；内容可按策略淘汰 |
| `heads`、`entry_versions` | 每个入口的最高 seq Head、冲突、受限标记与档位；已知版本 | 持久，不随内容淘汰（§16.6） |
| `published`、`head_history`、`stream_changes`、`object_grants`、`publish_tasks` | 本人的条目、受众、Head 历史、变化序列、读取授权、发布任务 | 持久 |
| `outbox`、`inbox_receipts` | 投递任务与结果；已接收 ObjId 与准入（幂等） | 持久 |
| `sources`、`intents`、`followers` | 来源与采集进度、订阅意图、收到的关注 | 持久 |
| `candidates`、`admission_history`、`reading`、`view_cursors` | 候选、首次入列与补看阅读、待读列表、视图游标 | 应用状态；候选按保留期清理 |
| `personal`、`capture_shares` | 点赞 / 转发 / 收藏 / 稍后再看 / 点踩及其入口、抓取分享关系 | 持久 |
| `comment_sources`、`author_list`、`tracked`、`participants`、`view_snapshots` | 评论来源、作者列表、追踪的讨论、参与者、视图快照 | 持久 |
| `eval_records`、`eval_tasks`、`tag_overrides`、`eval_changes` | 评价记录、任务、修正、变化流 | 持久 |
| `behavior_events` | 临时行为事件 | 可丢弃，默认 7 天 |
| `agreements`、`consumption_proofs`、`collector_index` | 消费约定与已发证明、收录索引 | 持久 |
| `settings` | 用户设置（默认受众、不看与过滤规则、Topic、收录者、门户资料等） | 持久 |

## 7. 运行

### 7.1 服务模式

- 构建与安装：`bucky_project.yaml` 模块 `homestation` → `bin/homestation/`；`rootfs/bin/homestation/kernel_pkg.toml`；scheduler `add_homestation()` 写入 `services/homestation/spec|settings`。
- 网关：`boot_gateway.yaml` 把 `/home/*` 转发到 `homestation`，`/kapi/homestation` 走通用路由；鉴权都在服务内完成。
- RBAC：`g, system:homestation, frame`，另加 `p, system:homestation, obj://msg-center/owners/*,read,allow`，用于读取 owner 的 Message Center 联系人。
- 设置 `services/homestation/settings`：

```json
{ "collector": false, "disclose_admission": true, "peers": { "<zone>": "<https origin>" }, "evaluation_model": null, "spider": true, "reading_window": 300 }
```

- 系统依赖：verify-hub 会话令牌、system-config（用户 DID、Zone 文档）、Message Center 联系人、NDM 命名对象存储、名字服务（跨 Zone 的 DID 与密钥）、AICC（仅模型评价）。

### 7.2 单机模式与开发网络

```bash
cd src
cargo run -p homestation -- --keygen                # 生成密钥：private_key_pem / public_key_x / did_dev
cargo run -p homestation -- --data-dir /tmp/hs-a --config hs-a.json
cargo run -p homestation --example devnet -- --port 4131 --data-dir /tmp/hs-devnet --fresh
```

单机配置字段：`owner`、`owner_name`、`zone`、`private_key_pem` 或 `private_key_file`、`kid`、`listen`、`collector`、`spider`、`workers`、`disclose_admission`、`intervals_s{pull,delivery,select,comments}`、`reading_window`、`tokens{<token>: {principal, did, app_id}}`、`directory{identities{<did>: {zone, public_key_x, devices, owner}}, zones{<zone>: <origin>}}`、`contacts[{did, name, friend, blocked, groups}]`、`settings`（初始用户设置）。

`devnet` 在一个进程里启动 `me`（Desktop 的 owner，令牌 `tok-me`）、alice、bob、sarah、收录者 index 和一个 RSS 夹具站点，预置好友、关注、各类发表、评论、点赞、引用转发与订阅，节点之间只通过协议路径通信。Desktop 用 `HS_BACKEND=http://127.0.0.1:4131` 接入（见前端 README / UI_DATAMODEL）。

## 8. 验证

```bash
cd src
cargo test -p homestation                    # 单元 + 多节点场景
cargo test -p buckyos-api homestation_reads  # RBAC
cargo test -p scheduler                      # 服务注册
```

真实 Zone（DV，见 `test/test_homestation/test_homestation_dv.ts` 与 Desktop `tests/dv/homestation.spec.ts`）：

```bash
cd test/test_homestation && deno run --config ../deno.json --allow-net --allow-env --unsafely-ignore-certificate-errors test_homestation_dv.ts
cd src/frame/desktop && BUCKYOS_UI_DV_BASE_URL=https://test.buckyos.io npx playwright test --config=playwright.dv.config.ts tests/dv/homestation.spec.ts
```

多节点测试（`tests/`）在同一进程内启动多个真实 HomeStation，各自有密钥、数据库、chunk 目录与 HTTP 监听，只通过协议路径交互：

| 文件 | 场景 |
| --- | --- |
| `network.rs` | 按读者身份的展示读取与读者证明（A66、A69、A32）；投递准入、披露、幂等、Push/Pull 合并（A01、A04、A05）；版本、撤回、旧 Head 重放、先 Head 后对象、同 seq 冲突（A17、A23、A24、A26、A30）；入口命名空间与内容规则（A46–A48）；他人门户的匿名预览、空闲时变化计数不动（A43） |
| `interactions.rs` | 评论与点赞到达作者与收录者、多视图合并与去重、作者移除可观测、取消点赞（A12–A16、A21–A23、A27）；受限内容的转发与评论（A70、A71）；私有 / 公开收藏、普通转发与引用转发（A11、A28、A40、A49、A74） |
| `reading.rs` | 媒体按 ObjId 准备与校验、发布幂等、AI 标签过滤与修正、搜索、不看（A02、A06、A35、A41、A44、A51–A53、A56）；关注补看（A37–A39）；收录者冷启动（A18）；变化读取与压缩重同步（A67、A68） |
| `services.rs` | 评价服务的身份 / 文件 / 路径 / InnerPath 目标、复用与刷新、异步任务、共享（A57–A62）；修正作用域（A64、A65）；消费证明（A19、A20）；权限边界（A25、A43） |
| `sources.rs` | 离线接收方与重试（A13、A33）；Spider、私人抓取、快照与分享（A03、A45、A72）；好友派生关注（A34、A36）；自然语言意图（A42） |

## 9. 与架构的差异、已知限制与待定

| 项 | 现状 |
| --- | --- |
| Head 字段 | 增加 `publisher`（§2.3） |
| 读者身份 | 自签读者证明（§3.2），待对齐 CYFS 跨 Zone 身份机制 |
| 每 Zone 一个发表者 | 服务模式只为 Zone owner 发表；同 Zone 的其他用户与 Agent（DID 解析到本 Zone 或其子域）没有自己的发表流和投递入口：不为他们派生关注、不 Pull、不 Push（已排队的投递以 `local-principal` 拒绝结束），他们以会话令牌按受众读取 owner 的发表流。多发表者需要按 owner 分库并约定 `/home` 下的用户路径 |
| 设备签名的跨 Zone 验证 | 依赖对方解析发表者 / 设备 DID Document（`authentication` 或设备 `owner`）；未在真实多 Zone 环境验证。did:dev 设备 DID 无可解析的 owner 关系 |
| 网关缓存 | 未配置 NamedInboxCacheServer，`cached` 只会来自对端网关 |
| 大文件 | 只预取与校验单 chunk 文件；chunk list 内容记为 `reachable`，不预取、不校验 |
| 媒体令牌 | 浏览器媒体元素用 `?access=<session token>` 取内容（已知风险） |
| 点踩 | 仅本地反馈，不发表、不计数（TODO §13 待确认） |
| 默认受众 | 公开，可在偏好设置修改（TODO §13 待确认） |
| 门户 | Desktop 路由 `/homestation/u/:did`；`/home/profile` 与展示读取已可由 Zone 根直接访问，访客页面由 Zone 根提供尚待确认 |
| 多级回复、按版本撤回、越过包裹层评论 | 未实现（§21 待定） |
| 模型评价 | 已接 AICC，但没有自动化测试；默认关闭 |
| 来源映射 | 自然语言意图由规则映射到联系人、收录者与 Topic；还没有 Agent 持续维护映射 |
| 收录者 | 只提供评论视图与最近公开发表；没有检索接口与运营策略 |
| 内联上限、受众枚举、游标与分页编码、压缩策略 | 按本文的实现选择，宣布稳定前可调整 |
| DV | 2026-10-09 在 test.buckyos.io 验证通过：服务模式启动与登录、网关 `/kapi/homestation` 与 `/home/*` 转发、verify-hub 会话令牌与用户 DID、Message Center 联系人（好友派生关注）、NDM 上传与内容读取、发表 / 编辑 / 点赞 / 评论 / 撤回、评价服务、Desktop 页面加载与发布。未验证：跨 Zone（对方验证设备签名、向其他 Zone 投递与 Pull），DV 环境里没有可达的第二个 Zone |
