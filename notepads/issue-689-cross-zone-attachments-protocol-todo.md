# Issue #689：MessageHub 跨 Zone 附件 TODO

日期：2026-11-06 重写（America/Los_Angeles）。关联：[issue #689](https://github.com/buckyos/buckyos/issues/689)、[Bucket 设计 §4.6.5 与 §21 N18](../doc/arch/应用对象桶%20Bucket%20设计.md)、[跨 Zone 登录断言契约](../doc/arch/verify-hub/跨Zone登录断言契约.md)。

核对基线：`buckyos@3efd8ccf1`、`cyfs-ndn@c5dd866`（与 `src/Cargo.lock` 锁定的提交相同）。

本轮验证范围：阅读相关源码，跑了一个临时用例复现 §5.1（用完已删）。没有改服务代码，没有重跑 DV。下文标“DV”的结果来自 Bucket §21 N18 在 2026-11-05 的记录；标“静态”的只读过代码，没有运行。

旧版是 10-28／29 的全链路分析（`git show 780e102f0:notepads/issue-689-cross-zone-attachments-protocol-todo.md`）。它列出的缺口大多已被基础设施解决，所以删除了，旧章节的去向见 §3.2。§15 是 Agent 授权分析，被 [agent-appid TODO](agent-appid-authorization-implementation-todo.md) 引用，原样保留。

## 1. 结论：还要做，但范围小很多

旧 TODO 要回答的是：远端凭什么读、从哪里读、怎么完整取回。这三个问题现在都有答案，私聊的图片和文件已在真实三 Zone 环境跑通。**不需要再做通用认证层、专用传输协议、持久获取状态机和来源字段。**

剩下四块：

| 优先级 | 内容 | 为什么要做 |
| --- | --- | --- |
| P0 | 堵住本地重发越权（§5.1）和伪造大小绕过上限（§5.2） | 前者已复现：知道 ObjId 的人能读到本 Zone 别人的私有对象。这是 N1 没覆盖到的路径 |
| P1 | 私聊收尾（§6）：陌生人消息不拉取、未读收藏、UI 状态、未验证场景 | 基本流程通了，但拉取超过 20 秒时界面直接报错，要手动点重试；陌生人的消息一渲染就外连 |
| P2 | 群（§7） | 跨 Zone 的群附件现在全部不可读，#689 的现象在群里原样存在 |
| P3 | 目录、超过 64 MiB、预取（§8） | Desktop 能发目录，对方读到 404。大文件和预取按产品优先级决定 |

#689 的验收项对照：

| issue 验收项 | 现状 |
| --- | --- |
| 图片首次到达时自动获取并展示 | 私聊成立（DV，没经过 Desktop UI）；群不成立 |
| 多附件、重复读取、并发不错绑、不重复下载 | 绑定 key 是确定的，同一进程按 key 合并并发拉取。没有并发用例。同一对象在不同消息里会各下载一次，这是有意的（§4 第 1 条） |
| 临时故障恢复后可重试，不用重发 | 下一次读取重新拉取，失败后 30 秒内不再尝试。短时断网没有实测 |
| 未授权用户不能凭 ObjId 读取 | 读取路径成立（DV）；§5.1 是例外 |
| 明确文件、音视频、目录的覆盖范围 | 64 MiB 以内的文件成立，音视频走同一路径但没有单独验证；目录不支持；超过 64 MiB 返回 413 |

建议 P0、P1、P2 完成后再关 issue，P3 拆成独立 issue。

## 2. 现状：按入口核对

| 入口 | 现在的行为 | 依据 | 验证 |
| --- | --- | --- | --- |
| 私聊，FileObject，64 MiB 以内 | 首次 `describe` 或 `content` 触发拉取，读取最多等 20 秒，之后 503。拉取在后台继续，完成后绑定 | [remote_attachment.rs](../src/frame/msg_center/src/remote_attachment.rs)、[object_access.rs:316](../src/frame/msg_center/src/object_access.rs#L316) | 单测和三 Zone DV（40 项，含 NAT 中继、非收件人、跨 chunk）。Desktop UI、并发、短时断网、发送方离线未跑 |
| 私聊，超过 64 MiB | 413，不取内容。本 Zone 的读取路由有同样的上限，Desktop 也发不出；Agent／API 可以发 | [remote_attachment.rs:187](../src/frame/msg_center/src/remote_attachment.rs#L187)、[object_access.rs:388](../src/frame/msg_center/src/object_access.rs#L388)、[attachmentDraft.ts:3](../src/frame/desktop/src/app/messagehub/conversation/input/attachmentDraft.ts#L3) | DV |
| 私聊，目录 | 判为“非远端”，404，前端显示“no longer available”。Desktop 可以发目录 | [remote_attachment.rs:110](../src/frame/msg_center/src/remote_attachment.rs#L110)、[upload.ts:33](../src/frame/desktop/src/app/messagehub/api/upload.ts#L33) | 静态 |
| 群：host 本地作者，host 本地读者 | 绑定，可读 | [group_service.rs:3220](../src/frame/msg_center/src/group_service.rs#L3220) | 单测 |
| 群：joined 作者，host 上的读者 | `Arrival::Remote` 不绑定；群消息不拉取，404 | 同上；`a_remote_message_cannot_open_a_local_object` | 单测 |
| 群：joined 成员在自己 Zone 读（作者在哪都一样） | Desktop 只带 msg_id，消息权限通过；没有绑定，群消息不拉取，404。host 有内容也读不到 | [store.ts:586](../src/frame/desktop/src/app/messagehub/api/store.ts#L586)、[remote_attachment.rs:110](../src/frame/msg_center/src/remote_attachment.rs#L110) | 静态 |
| 群：作者在自己 Zone 读自己发的 | 按本地发件绑定，可读。这份绑定就是 §7 里 host 的上游来源 | [msg_center.rs:1999](../src/frame/msg_center/src/msg_center.rs#L1999) | 静态 |
| 群展示文件（`_display`） | 只认本 Zone 托管的群，joined 成员读不到 | [group_service.rs:3528](../src/frame/msg_center/src/group_service.rs#L3528) | 静态 |
| 转发没读过的远端附件 | 授权放行（见 §5.1），绑定时内容不全，发送失败 | [buckets.rs:429](../src/frame/msg_center/src/buckets.rs#L429) | 静态 |
| 收藏没读过的远端附件 | 没有绑定，直接失败，不触发拉取 | [buckets.rs:387](../src/frame/msg_center/src/buckets.rs#L387) | 静态 |
| Agent 读取 | 不带 context 时在邮箱里找引用，之后走同一路径。Agent 作为收件人时 requester 是 AgentDID，custody 按 AgentDocument 的 owner 判定，没有用例 | [buckets.rs:468](../src/frame/msg_center/src/buckets.rs#L468) | 静态 |
| Telegram 外发 | 直接读本地 named store，不触发拉取 | [tg_tunnel.rs:492](../src/frame/msg_center/src/tg_tunnel.rs#L492) | 静态 |
| 陌生人消息（REQUEST_BOX） | 和普通消息一样：卡片一渲染就 `describe`，就触发完整拉取。已定要改成不拉（§6） | [MediaAttachment.tsx:57](../src/frame/desktop/src/app/messagehub/conversation/media/MediaAttachment.tsx#L57) | 静态 |
| 失败状态 | 获取中、网络失败、源端拒绝、校验失败都是 503；Desktop 一律显示不可用加手动重试 | [remote_attachment.rs:150](../src/frame/msg_center/src/remote_attachment.rs#L150)、[source.ts:142](../src/frame/desktop/src/app/messagehub/conversation/media/source.ts#L142)、[MediaAttachment.tsx:79](../src/frame/desktop/src/app/messagehub/conversation/media/MediaAttachment.tsx#L79) | 静态 |

External 和 Session Only 访客向 host 发带附件的消息会被拒绝（[test_hybrid_sso.rs:529](../src/frame/msg_center/src/test_hybrid_sso.rs#L529)），这是既定限制，不在本 TODO 范围内。

## 3. 已经解决的部分

### 3.1 现在的协议

```text
收到远端消息：不绑定，无论内容是否已在本 Zone
读取：本 Zone 消息权限通过，发现没有绑定
  1. 本 Zone 设备以收件人 DID 签断言，换取源 Zone 的 External 会话（login_by_cross_zone_assertion）
  2. msg.attachment_ticket {msg_id, obj_id}
     源 Zone 检查：私聊、请求者在 msg.to 里、FileObject、绑定持有它
     返回 bkt_ 令牌：只读、单 key、1 小时、不限次
  3. GET https://<源 Zone>/ndn/<file>?hint={"token":...}
     FileObject、ChunkList、chunk 逐项校验写入本 Zone，不跳过本地已有的内容
  4. bucket.put(<session>/<msg_id>/<n>)，内容完整才成功，读取继续
状态只有消息和绑定；失败由下一次读取重做
```

### 3.2 旧问题的去向

其他文档还在引用旧章节号（Bucket §4.6.5、§4.6.6，`done/cross-zone-comm-todo.md`），按下表对应。

| 旧问题 | 现状 | 旧章节 |
| --- | --- | --- |
| 跨 Zone 请求者身份，session token 不能跨 Zone | `CrossZoneClient` 加 `login_by_cross_zone_assertion`：设备签断言，目标 Zone 验 custody 后签发自己的 External 会话。HTTPS 和 RTCP 同一规则 | §4、§5.1、§5.2 |
| 谁裁决读取 | 源 Zone 的 msg-center（`msg.attachment_ticket`）。私聊已实现，群见 §7 | §5.3、§6.1 第 4–6 步 |
| `bkt_` 能否做数据面凭据 | 能。`/ndn/` 的 `hint.token` 只接受只读、单 key、未绑定主体、不限次的令牌，且 key 当前绑定必须等于授权根，解绑即失效 | §5.4 |
| 消息与凭据分离 | 令牌只在拉取时申请，不进 `MsgObject` | §5.4 末段 |
| bucket GET 不是对象图导出，子请求丢凭据 | `/ndn/` 的 `resp=raw` 和 `path`；`CyfsNdnClient` 子请求带路径和令牌，不跟随重定向 | §7.1、§7.2 第 1 条 |
| 内容校验 | 逐项校验，本地副本不能代替拉取 | §7.3 |
| 来源定位 | 由 `msg.from` 经 custody 解析当前 Zone，消息不加字段，`uri_hint` 不参与 | §6.3 |
| 保留策略与获取时机 | 取回后进 `attachments`（AppData、Recursive pin），和本地发件相同；首次读取时获取 | §1、§8.3 |
| 持久状态、任务认领、恢复 | 不需要。拉取中没有 lease，GC 命中只表现为这次失败，下次读取重拉 | §8.1、§8.2 |
| 本地命中越权 | N1：远端消息不绑定。N1 漏掉的路径见 §5.1 | §9 |
| 已有的缺附件私聊消息 | 下一次读取自动取回，不用迁移 | §12 末节 |
| 群两段流程、撤回竞争 | 未实现，见 §7 | §6.2、§8.2 第 6 条、§10 |
| 目录递归 pull、Range、大文件 | `/ndn/` 已支持 Range；目录 pull 和大文件未实现，见 §8 | §7.2 第 2、3 条 |

### 3.3 明确不再做

除非出现具体需求：

- 持久获取状态表、任务 claim、后台 reconcile。
- 来源字段进 `MsgObject` 或 sidecar，多源回源，Zone 迁移后的来源证据。迁移后解析到新 Zone，新 Zone 没有绑定就是取不到。
- 附件取得回执。
- 让用户逐个登录源 Zone（External SSO）作为附件入口。收件人 Zone 代为登录已经够用。

## 4. 仍然成立的约束

1. 消息签名、ObjId、本地副本都不授权。绑定的依据只有两种：本人持有或可读的证明；字节确实从有权的来源取回并校验。本地副本不能代替拉取，所以同一对象在不同消息里各下载一次。
2. 业务裁决在 msg-center，数据面只判令牌。不开放通用 NDM 读权限，不公开附件桶，私聊和群附件不进 `public-cache`。
3. 令牌和 `hint` 不进 `MsgObject`、日志、错误详情、Task 数据。
4. 内容完整才绑定。
5. 消息接收不等附件。附件失败只重试获取，不重新投递。
6. 撤销只控制后续获取。已取回、收藏、导出的副本不承诺远程删除。
7. 新的跨 Zone 调用用 `CrossZoneClient`，不再造身份协议。

## 5. P0：越权和限额

### 5.1 本地重发绕过 N1（已复现）

远端消息不再绑定，但 `authorize_outgoing_attachments` 判断能否复用引用时，只看调用者的邮箱里有一条引用该对象的消息，不看那条消息是否真的持有它（[buckets.rs:429](../src/frame/msg_center/src/buckets.rs#L429)）。通过后走本地发件，用 msg-center 的服务身份绑定。

```text
1. 外部 DID S 给本 Zone 用户 R 发签名消息，引用本 Zone 的私有对象 X。不绑定，R 读到 404
2. R 发一条新消息，同样引用 X。post_send 接受，新消息的 key 绑定了 X
3. R 经新消息读 X，200，拿到原文
```

临时用例放在 `test_group_service.rs` 的 `attachment_buckets` 模块，用 `bound_center` 和 `verifier(false)`，输出：

```text
remote message bound: None
read via remote message: 404
resend accepted: Ok(true)
resend bound: Some("cyfile:75b0…")
read via resend: 200 "someone's private bytes"
```

前提：知道 X 的 ObjId，X 已完整存在本 Zone，R 有本 Zone 账号并能收到这条消息。R 可以是攻击者自己的账号配一个外部 DID；也可以是被诱导转发的普通用户，此时转发给 S，S 还能用 `msg.attachment_ticket` 把 X 取走（这一步是静态推断）。

验证的边界：

- 用例里的 nfs-server 没开鉴权（`Session::Open`）。真实 Zone 上由 `ServicePolicy::holds` 决定，`frame` 角色有 `obj://ndm/objects` read（[bucket/mod.rs:336](../src/frame/nfs_server/src/bucket/mod.rs#L336)、[rbac_config.rs:256](../src/kernel/buckyos-api/src/rbac_config.rs#L256)），静态核对会放行，没有在真实 Zone 跑。
- 群分支（[buckets.rs:434](../src/frame/msg_center/src/buckets.rs#L434)）同样只看消息是否引用，静态判断同样可达，没有运行。
- External 和 Limited 会话带附件的发送已被拒绝，不走这条路。

TODO：

- [ ] 复用引用的条件改成“来源消息的绑定持有该对象”，私聊和群两个分支都改。自己发过的消息、已取回的消息都满足，合法转发不受影响。
- [ ] 没有绑定且来源是远端私聊消息时，先 `ensure_remote_attachment`，得到 `Bound` 才算数，否则拒绝发送并返回可重试错误。这同时修好未读转发，转发到 Telegram 也因此先取得内容。
- [ ] `own_message_referencing` 的读取分支优先选已绑定的消息。现在只取最新一条引用，一条新到的未绑定消息会让已有附件的无 context 读取变成 503 或重新外连（静态）。
- [ ] 把上面的复现转成正式回归用例，加上合法转发对照和群分支，再补一条真实 Zone DV。
- [ ] 修完后更新 Bucket §4.6.3 的 Z7、§4.6.6 的 G2 和 §21 N1。它们现在把这个问题写成已修。

### 5.2 伪造 `size` 绕过 64 MiB 上限（静态）

`pull_attachment` 用 FileObject 里的 `size` 判断是否超限（[remote_attachment.rs:186](../src/frame/msg_center/src/remote_attachment.rs#L186)）。`size` 是发送方写的，`pull_chunk_list_by_object` 按 ChunkList 逐个下载，既不和 `size` 比较，也没有总量上限（[cyfs_ndn_client.rs:1941](../../cyfs-ndn/src/ndn-toolkit/src/cyfs_ndn_client.rs#L1941)）。发送方写一个小 `size`，content 指向很大的 ChunkList，接收方就会全部下载并绑定，读取时才因实际长度超限返回 413。

- [ ] 拉取前先取回 ChunkList，按 chunk 自带长度累加后判断上限；content 是单个 chunk 时同样比较。`size` 与实际长度不符按校验失败处理，不带长度的 chunk 拒绝。
- [ ] 上限作为 `CyfsNdnClient` 的 pull 参数传入，目录（§8）直接复用。
- [ ] 回归用例：伪造 `size`、不带长度的 chunk。

## 6. P1：私聊收尾

- [ ] 陌生人消息不拉（§9 第 1 项）：`ensure_remote_attachment` 外连前按读者对发送方的当前访问策略判定，复用 `decide_inbox_kind`（[msg_center.rs:1225](../src/frame/msg_center/src/msg_center.rs#L1225)），不允许投递就不外连。不能看记录在哪个 box：接受（好友或 `allow`）只改联系人的 `access_level`，消息记录不会搬出 REQUEST_BOX。检查放在 `ensure` 里，转发和收藏一起受约束；群消息不走这个判定。
- [ ] 陌生人消息的前端：显示“接受后可查看附件”，不算失败；接受后失效该会话的附件缓存并重新加载。
- [ ] `remote_attachment_dv.mjs` 没有建立联系人关系，静态看会被上面的规则挡住。先允许发送者，再加一条“陌生人不外连、允许后可读”的用例。
- [ ] `msg.collect_attachment` 没有绑定时先 `ensure_remote_attachment`，完整取回后再复制。
- [ ] 拆开 `RemoteAttachment::Unavailable`：获取中（503 + `Retry-After`）、暂时不可达（503）、等待接受（403）、源端拒绝或对象不存在（404）、校验失败（502）。票据错误按 `CrossZoneError::is_retryable` 分类。`Retry-After` 固定 5 秒，和失败后的 30 秒退避不一致，一并对齐。
- [ ] Desktop 读 `Retry-After` 自动重试，有次数上限；增加“正在从对方 Zone 获取”的状态。现在拉取超过 20 秒（经中继的几十 MiB 文件就可能超过）时，用户看到的是错误，后台其实还在下载。
- [ ] 目录在 P3 完成之前返回 415，不再是 404 加“no longer available”。
- [ ] 按 §9 第 5、6 项的决定实现非图片附件的拉取时机和屏蔽后的票据规则。
- [ ] 补验证：Desktop UI 全流程（收图、重试、换账号）、并发读取、短时断网恢复、发送方 Zone 离线、Agent 作为收件人和发送方、带 refs 的 edit。
- [ ] 同步文档：[Message Center](../doc/message_hub/Message%20Center.md) 附件一节（625–640 行）还写着“任一不满足返回 404”，没有远端拉取、503、413 和 `msg.attachment_ticket`；[UI_DATAMODEL](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md) 的附件状态；`buckyos_admin` 手册第 306 行的附件排查补上“远端附件未取回”。实施时检查 `aigc_image`、`aigc_video` 里涉及消息附件的部分。

## 7. P2：群

### 7.1 目标流程

host 是群附件的唯一裁决者，并保留一份副本。作者 Zone 不看成员表，成员 Zone 不找作者。两段都复用 §3.1 的做法：

```text
作者到 host：host 接受消息后后台拉取，不阻塞 accept；没拉到时由读取或票据请求再次触发
  host 以群 DID 登录作者 Zone，作者 Zone 的 verify-hub 按群 DID 文档判定 custody
  msg.attachment_ticket：调用者是 msg.to 里的群，绑定持有它
  host 拉取、校验、绑定进 attachments，按 AppData 保留

host 到成员：ZB 的读者没有绑定
  ZB 本地消息权限通过
  以读者 DID 登录 host，申请票据，带 context_path=<session>/<msg_id>
  host 用 authorize_group_attachment 裁决（成员、历史可见范围、read hooks、客户端策略）
  host 还没有内容时再次触发上一段，完成前返回可重试
  ZB 经 host 的 /ndn/ 拉取、校验、绑定
```

群 DID 就是这条消息的收件人，所以作者到 host 这一段的票据规则和私聊相同，都是“请求者在 `msg.to` 里”。

host 接受后就拉取，是因为按首次读取拉取时，如果作者在有人读之前离线，host 就取不到，保留副本也就失去了意义。

### 7.2 TODO

- [ ] `canonical_zone_from_doc` 增加群文档分支（[cross_zone_auth.rs:174](../src/kernel/buckyos-api/src/cross_zone_auth.rs#L174)）。现在群文档走到末尾的 `zone_did`，得到空，custody 拒绝。改成沿文档的 `owner` 走到它的 canonical Zone。群主可能是 Agent，而 Agent 分支要求 owner 文档带 `binded_zone_list` 或 `ns/owner`，不能直接共用那段检查。这是 `check_custody` 的共用逻辑，verify-hub、msg-center 消息验签和 HomeStation 都受影响，同步[跨 Zone 登录断言契约](../doc/arch/verify-hub/跨Zone登录断言契约.md) §3，并补正反例：host 的设备可以代表群，其他 Zone 的设备不可以。
- [ ] 守住“群主 Zone 就是 host”：`group.transfer_owner` 和 `group.accept_owner_transfer` 拒绝其他 Zone 的成员。现在只判成员身份（[group_service.rs:2003](../src/frame/msg_center/src/group_service.rs#L2003)）。Desktop 走不到跨 Zone 接受，因为 joined 成员的 Zone 上没有这个群，只有 `accept_invitation` 和 `get_doc` 会转给 host；但 External 会话直接调 host 的 kRPC，代码里没有拦（静态）。
- [ ] `ensure_remote_attachment` 放开群消息，并接受调用者身份。host 上 requester 是群 DID，用 `groups.load` 判断群是否托管在本 Zone，不依赖 `is_local_recipient`：`register_local_recipients` 只在建群时调用（[group_service.rs:920](../src/frame/msg_center/src/group_service.rs#L920)），重启后一级 DID 的群不算本地（静态）。成员 Zone 上 `msg.to` 是群，要用实际读者的 DID。
- [ ] 扩展 `msg.attachment_ticket`：调用者是 `msg.to` 里的群时走作者到 host；带 `context_path` 时走 host 到成员，用 `authorize_group_attachment` 裁决。令牌都只覆盖对应 key。joined 路由只知道 host 主机名，host 到成员这一段用 `CrossZoneClient::request_host`（[group_sync.rs:95](../src/frame/msg_center/src/group_sync.rs#L95) 已有同样的用法）。
- [ ] host 接受消息后后台拉取，复用 `spawn_fetch` 的合并逻辑。不阻塞 accept，失败不影响消息接受，崩溃后由下一次读取补上，不新建任务表。§5.2 的上限同样适用；群附件计入 msg-center 的桶配额，超配额按不可用处理。
- [ ] `_display`：`authorize_display_file` 支持 joined 成员在本 Zone 读，票据 key 是 `<group>/_display/<obj_id>`。展示文件移除后解绑，旧令牌随之失效，这一点 `/ndn/` 已保证。
- [ ] 撤回竞争：`pull_attachment` 绑定前确认消息还在，绑定后再查一次，已撤回就解绑。否则晚到的拉取会复活已撤回的附件。
- [ ] 退群、移出：host 不再发票据；本地已有副本的展示规则不变。
- [ ] 独立 ZA、G、ZB 三 Zone 的 DV，覆盖 §10 的群相关行。
- [ ] 同步文档：[Self-Host-Groupv2](../doc/message_hub/Self-Host-Groupv2.md) §5.7 的保留策略和 §6.2 的转让群主（写明跨 Zone 转让等于转服，本版本不支持）、[group-v2-backend](../src/frame/msg_center/doc/group-v2-backend.md)、Durable Data Schema 里“group's attachments stay unreadable”一句。

## 8. P3：目录、大文件、预取

目录：

- [ ] 票据和 `ensure` 放开 DirObject。`/ndn/` 已能沿目录子项走路径（[ndn_read.rs:238](../../cyfs-ndn/src/bucket_store/src/ndn_read.rs#L238)）。
- [ ] `CyfsNdnClient` 增加目录 pull，现在是 Unsupported（[cyfs_ndn_client.rs:1534](../../cyfs-ndn/src/ndn-toolkit/src/cyfs_ndn_client.rs#L1534)）：递归、深度／节点数／总字节上限、去重、子项名规范化。`bucket.cache_public` 也在等这个（Bucket §4.6.5）。

超过 64 MiB：

- [ ] 不给 msg-center 加流式。消息权限通过后给读者签本 Zone 的短期令牌，客户端走 `/ndn/`，Range、HEAD、`<video>` 都已支持。跨 Zone 拉取的上限和配额按 §9 第 4 项决定。

预取和预算：

- [ ] 按来源、按用户的下载预算，可信会话预取。等有实际需求再做。

## 9. 决定

已定（2026-11-06）：

| # | 问题 | 决定 | 影响 |
| --- | --- | --- | --- |
| 1 | 陌生人消息（REQUEST_BOX）是否自动拉取 | 不拉。发送方被接受（好友或 `allow`）后，按普通消息处理。否则卡片渲染就外连，陌生人能得知读取时间和对方 Zone 的出口地址，还占用磁盘 | §6 |
| 2 | 作者 Zone 怎么确认调用者是 G 的 host | 看群的 DID 文档：host 以群 DID 登录，custody 沿文档里的 `owner` 走到它的 canonical Zone，和 Agent 的规则同类。不比对 `JoinedGroupRoute`，作者退群后路由失效也不影响判定。前提是群主 Zone 就是 host：转让群主在运维上等于转服，本版本不支持。群管理员是 host 上的成员角色，可以在其他 Zone，不影响判定 | §7 |
| 3 | host 是否必须保留副本 | 保留：取回后按 AppData 保留。群要有自己的消息处理逻辑，host 必须持有内容。Self-Host §5.7 现在写“不是协议要求”，要同步改 | §7 |

待定：

| # | 问题 | 建议 | 影响 |
| --- | --- | --- | --- |
| 4 | 是否支持超过 64 MiB 的跨 Zone 附件 | 先不做，拆独立 issue | §8 |
| 5 | 非图片附件是否延后到点击再拉 | 建议延后。现在文件卡片一渲染就下载完整内容。卡片只需要 FileObject 描述，但描述和绑定要分开处理 | §6 |
| 6 | 对方被屏蔽后是否还发票据 | 建议不发。现在只看收件关系和绑定 | §6 |

## 10. 验收

| 类别 | 场景 | 判断标准 | 状态 |
| --- | --- | --- | --- |
| 私聊从零副本 | 图片、ChunkList 文件、多附件，经中继 | 原 ObjId 不变，逐字节一致，非收件人拿不到令牌 | DV 通过，未经过 Desktop UI |
| 本地重发越权 | §5.1 的私聊和群分支；合法转发对照 | 非法引用不产生绑定；合法转发照常 | **现在失败** |
| 伪造大小 | 小 `size` 加大 ChunkList；不带长度的 chunk | 不下载超限内容，不绑定 | 未测 |
| 并发 | 同一 key 多读者；同一对象在多条消息；describe 和 content 同时请求 | 只下载一次（同 key），各自鉴权 | 未测 |
| 恢复 | 短时断网、服务重启、发送方离线后恢复、拉取中 GC 命中 | 不用重发消息，下一次读取成功 | 未测 |
| 陌生人消息 | 陌生人发附件；接受后；屏蔽后 | 接受前发送方网关收不到任何请求；接受后可读；屏蔽后不外连 | 未实现 |
| 前端 | 获取中、等待接受、自动重试、源端拒绝、换账号 | 状态能看懂，不把获取中显示成不可用 | 未实现 |
| 群三 Zone | ZA 作者、G host、ZB 成员；非成员、被移出的成员、Guest 越 Session | 两段都授权，joined 不依赖本地恰好有内容，host 的裁决贯穿 | 未实现 |
| host 身份 | host 的设备代表群、其他 Zone 的设备冒充、群主是 Agent、其他 Zone 的管理员、向其他 Zone 的成员转让群主 | 只有真正的 host 能取到票据，跨 Zone 转让被拒绝 | 未实现 |
| host 保留 | 消息被接受后作者立刻离线，之后才有人读；host 重启 | 成员照常可读；未拉完的由下一次读取补上 | 未实现 |
| 撤回竞争 | 下载中、绑定前后撤回 | 不复活已撤回的附件，其他绑定和收藏不受影响 | 未实现 |
| 展示文件 | `_display` 读取、配置替换和移除 | 展示配置和 Session 权限都通过，旧令牌失效 | 未实现 |
| 关联操作 | 未读转发、收藏、Agent 收发、Telegram 外发 | 先合法取得再操作，不传旧令牌 | 未实现 |
| 目录 | 嵌套目录、同一子对象多处引用、缺子节点、超规模 | 图完整才绑定，路径规范化 | 未实现 |

验证入口：

```sh
# 在 buckyos/src
cargo test -p msg_center -- --test-threads=1
cargo test -p nfs_server -- --test-threads=1
# 在 cyfs-ndn/src
cargo test -p ndn-toolkit
cargo test -p bucket_store
# 在 buckyos/test/nfs_server_test，用 PVE 上的 ssotest 三 Zone，说明见脚本头部
BUCKYOS_TEST_DNS=<sn ip> BUCKYOS_TEST_DEVICE_DIR=<dir> node remote_attachment_dv.mjs
```

mock 和单 Zone 测试通过不能代替不同信任域的验收。

---

以下 §15 是 10-29 的原文，基线 `buckyos@688df1577`，本轮没有重新核对。它对 #689 的影响已经很小：源 Zone 按 External 会话的 DID 判收件关系，不依赖 Agent App 的 RBAC；本 Zone 的读取走 §2 的 Agent 读取路径。后续在 agent-appid TODO 跟踪。

## 15. 2026-10-29：Agent 作为特殊 appid 的授权模型与资源命名

### 15.1 用户确认的框架方向

Agent 的资源执行身份保持为特殊 App。Owner 是权限上限，调度器根据受 Owner 控制的 Agent 配置构造 App 侧 RBAC，最多放开到 Owner 当前拥有的权限。扩大的是 Agent App 的授权范围，不给 Owner 增加权限，也不把 Agent 提升为系统服务。

普通资源调用的目标判定为：

```text
允许 = Owner 对该资源和操作有权
    AND Agent App 对该资源和操作有权
    AND 该服务的资源关系／业务规则通过
```

AgentDID 仍可以标识消息作者、收件人、群成员和对象归属。它与执行授权主体的职责应明确：本地执行主体可以统一为 `(owner_user_id, app:<runtime_app_id>)`，业务 actor 可以是该 App 绑定的 AgentDID，绑定必须来自可信 AgentSpec，不能由请求者随意声明。不能为了统一 appid，把 Agent 发言改成 Owner 发言。

“最大等于 Owner”按当前正常会话权限理解；Owner 能通过另一次 sudo 获得的权限，不自动变成 Agent 常驻权限。只读 Agent 仍可写自身运行状态、Session、记忆、自己的 mailbox／产物等已明确允许的资源；这些例外应在配置和策略里可见，并保持 Owner 上限。

### 15.2 当前普通 RBAC 已具备核心交集

在 `688df1577`：

- `AgentSpec.permission_mode` 支持 `read_write`／`read_only`，默认读写。
- scheduler 的 `update_rbac` 为绑定 App 生成 `g, app:<appid>, agent_read_write` 或 `agent_read_only`。读写 App 还有 agent_runtime／agent 角色；只读 App 不进入普通 app／agent_runtime 组，并在生成策略尾部追加兜底 deny，前序规则保留自身运行必需的操作。
- 默认 `agent_read_write` 规则为 `obj://* all`；`agent_read_only` 为 `obj://*` 的 read／list／query／subscribe，另有敏感配置 deny 和自身产物写入等例外。当前模式粒度主要是读写／只读，尚不是任意资源的细粒度权限配置。
- `runtime::enforce_verified` 从已验 token 取 `sub` 和 target 的 `authorization_key`，把原 resource/action 传给 `rbac::enforce`。
- `buckyos-base` 的 `rbac::enforce` 先检查 appid，再检查 userid，合取放行；常规 Agent App session 的 sub 为 Owner、App target 绑定其 AppInstance。它没有在底层统一转换资源 URL。

因此，对已经使用这条链路的 `obj://...` 调用，读写 Agent 的 App 侧已经很宽。失败应先定位 Owner 侧、实际资源名、token 主体及服务规则，而不是继续增加一条相同的 app 通配 allow。

依据：[scheduler 动态策略](../src/kernel/scheduler/src/system_config_agent.rs)、[默认策略与模型](../src/kernel/buckyos-api/src/rbac_config.rs)、[runtime 鉴权](../src/kernel/buckyos-api/src/runtime.rs)、[底层双主体检查](../../buckyos-base/src/rbac/src/lib.rs)、[AgentSpec](../src/kernel/buckyos-api/src/app_schema.rs)。已有 `agent_permissions_preserve_owner_limits_and_read_only_runtime_state` 等测试表达了这一约束；本轮只阅读，未重新运行。

### 15.3 整个 Home 的授权确实存在，但走的是挂载链

创建 Agent 时，Control Panel 给构造 App 写入 `PermissionItem {scope_path: "user/home", actions: [read, write] 或 [read]}`。node-daemon 读取该授权，把 `$BUCKYOS_ROOT/data/home/<owner>` 整个映射到容器 `/opt/buckyos/data/home/<owner>`，读写模式分别为 rw／ro；自己的 AgentRoot 单独保持 rw，供 Session、记忆与技能运行。

这里的 `user/home` 是权限声明 scope，不是传给 Casbin 的资源 URI。容器内程序直接 open/read/write 该挂载，受 Docker mount／OS 权限约束，不经过 `runtime::enforce`。宿主调试或 host runtime 还需按各自 OS／executor 边界审视，不能把 Docker 挂载限制当成所有执行路径都已统一。

依据：[构造 App 权限](../src/frame/control_panel/src/agent_mgr.rs)、[权限 scope](../src/kernel/buckyos-api/src/permission.rs)、[挂载执行](../src/kernel/node_daemon/src/app_loader.rs)。关键位置分别为 `agent_mgr.rs:781`、`app_loader.rs:2638,2761`。

### 15.4 `obj://` 与实际传入值的核对结果

| 路径／接口 | 权限定义或定位方式 | 实际检查 | 结论 |
|---|---|---|---|
| system-config | 裸 key、`/config/...`、`obj://config/...` | 服务归一化为 `obj://config/<key>` 后鉴权 | 这里有显式转换，不能把客户端原参数直接等同 RBAC resource |
| bucket | `obj://bucket/<scope>/<appid>/<owner>/<bucket>/<key>` | `BucketId::resource(key)` 生成同样的 `obj://bucket/...` | 资源名主线一致，但授权还包含 grant／token／服务分支 |
| NDM | `/ndm/v1/...` HTTP 接口 | `obj://ndm/uploads`、`objects`、`retain`、`manage`，另校验 holding | HTTP URL 和 RBAC resource 不同，但当前常量与策略对应 |
| msg-center 附件 | `/kapi/msg-center/objects/<id>` + context | `obj://msg-center/objects`，再检查消息和精确附件绑定 | 全局 API read 不等于任意附件 read |
| NFS／NFSP 文件面 | `cyfs:///home/<user>/...` | 自己的 path ACL；App／Agent 映射为 Delegated，file plane 直接拒绝 | 即使 App 有 `obj://* all` 也不会放行此入口，需调整服务接入模型 |
| Agent 容器 Home | `user/home` scope + 挂载路径 | mount rw／ro 与 OS 文件权限 | 没有一条 `obj://home/...` 检查覆盖这些本地文件操作 |

当前还不能把问题概括为所有 `obj://` 都写错：bucket／NDM／msg-center 的上述资源名确实相互对应。更明确的偏差是**权限声明、资源 URI、文件 locator、挂载和服务内特殊分支没有统一的转换及执行契约**。

通用 Casbin matcher 不会自动把 `cyfs:///...`、`obj://dfs/...`、`user/home`、HTTP 路径视为同一资源，`obj://*` 也不匹配 `cyfs://...`。路径别名、URL 编码、DID／AppId 中的标点、尾斜杠都应在服务入口通过规范化和共享 builder 处理，再使用同一结果执行鉴权与资源访问。

文档也有滞后：[key url.md](../doc/key%20url.md) 的示例是 `cyfs:///users/{users}/*`，当前 NFSP 的 Home 是 `cyfs:///home/<user>/...`，且目前走自有 ACL；其中 `obj://dfs` 被定义为服务对象，不是文件路径。不能照抄该示例推导当前 Agent 的 Home 已通过 RBAC 放行。

### 15.5 仍需统一的 Agent 特殊分支

1. **NFSP 文件面**：`auth.rs::require_file_actor` 直接拒绝 App／Agent 的 Delegated actor，path ACL 对 Delegated 返回 None。它尚不能表达“Owner 可访问，Agent App 授权范围也覆盖，所以放行”。这与用户要求的统一 App 模型有实际偏差，不能靠 scheduler 再加一条 allow 修复。
2. **独立 Agent token**：系统仍支持 `principal_kind=Agent、sub=AgentDID`，部分服务把它映射回 Owner，部分沿用 AgentDID。若要保证所有独立资源访问均不超过 Owner，应在通用认证／授权适配后形成可信 `(Owner, AppId)`，避免各服务自行猜测主体；消息／群身份仍保留 AgentDID。
3. **bucket AgentPolicy**：App token 走 Owner＋App 的 RbacPolicy；Agent token 则有专门的 AgentPolicy，按 Owner 名下 user buckets 直接允许 list/read，并按 permission mode 允许 create/write，拒绝 delete/manage。它没有在该分支对每个资源调用同样的 Owner＋App RBAC，权限效果可能与 App session 不同。按统一方向需收敛这一差异，并覆盖 Limited／动态授权变化；不能仅声称“有 Owner 字段，所以已满足 Owner 上限”。
4. **AppInstance 粒度**：当前动态角色主要绑定 `app:<appid>`；只读集合也按 AppId 汇总。一个 AppId 被多个实例／AgentSpec 复用时，应确认共享授权语义。当前共享 runtime 采用最严格只读的保护逻辑，后续细粒度 scope 不能把一个 Owner 的配置意外推广到其他实例。

bucket 还有一层关键语义：`authorize_any` 依次尝试 Identity、grant、`bkt_` token，Identity 返回 false 只表示没有经该来源放行，不是强制 deny。Agent Caller 使用 Owner 作为 user，还会匹配授予 Owner 的 user grant；因此 read_only 的 Identity 拒绝仍可能被有效 write grant／token 放行。当前 Agent Caller 也没有设置 app，无法匹配 App grant 或带 bind_app_id 的 token。这些差异都应在统一 App 模型下消除。

如果“Owner／Agent 配置上限”是必须遵守的约束，应在所有授权来源之外执行共同 ceiling，再选择 RBAC／grant／token 作为具体资源的授权依据；不能把 ceiling 仅实现为可被后续 grant 替代的 Identity false。Owner 的有效资源权利本身可以来自合法 grant，故 Owner 上限也不应被误写为仅检查默认 Owner RBAC 角色。依据：[bucket 授权组合](../../cyfs-ndn/src/bucket_store/src/access.rs)。

依据：[NFS 认证与 file gate](../src/frame/nfs_server/src/auth.rs)、[NFS path ACL](../src/frame/nfs_server/src/acl.rs)、[bucket session／RbacPolicy／AgentPolicy](../src/frame/nfs_server/src/bucket/mod.rs)、[verify-hub 登录映射](../src/kernel/verify_hub/src/main.rs)。这些是静态代码结论，尚未在运行中比较两类 token 的效果。

### 15.6 对 #689 的落地含义

Agent 读取自己收到的附件，先在本 Zone 按可信 Owner＋App 授权；消息关系仍以实际 Agent 收件人／群成员身份检查。源 Zone 的授权也需要能验证这种身份与代理关系，不能直接使用接收 Zone 的 RBAC item 或转发本地 session token。

扩大 Agent appid 权限可以解决“本 Zone 不允许该 App 发起读取”的问题；它本身不会赋予源 Zone 的附件权限，也不补齐 §6–8 的来源获取、完整性和保留链路。独立 Agent 与普通 MessageHub 应调用同一附件 ensure／读取流程，使用不同的可信 actor 上下文。

### 15.7 按该方向新增 TODO

- [ ] 将 `(Owner, appid)` 作为 Agent 独立资源执行的统一授权输入，校验 AgentSpec／AppInstance 绑定与当前状态；AgentDID 保留为业务 actor／地址。
- [ ] 明确动态 App 权限是 Owner 权限的上限内授权，所有服务 adapter 都保留 Owner 侧检查；覆盖 Owner 降权、Limited、停用、Agent 配置收紧和旧 token。
- [ ] 明确只读 Agent 的自身运行写入例外，保持 Session／记忆／产物可运行；不以全局服务提权实现这些例外。
- [ ] 建立 PermissionItem scope → canonical resource／action → 执行边界的映射表；由 scheduler 和各服务使用共享类型／builder，不把 `user/home` 直接当资源 URI。
- [ ] 对齐 NFSP 的 App／Agent 入口：以 Owner 文件 ACL 决定资源上限，以 App RBAC／grant 决定授权范围；替换仅凭 principal kind 一律拒绝的路径，保留 path／子树检查。
- [ ] 收敛 bucket 的 App 与 Agent token 授权差异；对 grant／capability 路径同样明确 Owner 上限如何生效，不能默认 capability 旁路即可代表 Owner。
- [ ] 审计 service API 的实际 `resource/action` 与 policy：config、bucket、NDM、msg-center、TaskMgr、NFSP；日志区分 App 拒绝、Owner 拒绝、业务 ACL 拒绝、token 类型 gate。
- [ ] 对规范化后的资源做正反例验证：scheme、Home 名称、编码、DID 中的点、精确 key／前缀、跨 Owner 路径；scheduler 输出与服务输入应使用同一规范。
- [ ] 分别验收 API 访问、Docker Home 挂载、host runtime／executor；记录各自执行边界，权限配置收紧需同步撤销旧执行能力。
- [ ] 将可信 Owner／App／Agent actor 上下文接入跨 Zone 附件授权，验证 Owner 允许而 App 拒绝、App 允许而 Owner 拒绝、两侧允许但非消息收件人仍拒绝等场景。

本轮只更新分析和 TODO，未改变授权行为或执行测试。核对依据是当前源码及已有测试断言，不表示部署中的 RBAC 已按该版本刷新。

### 15.8 后续具体实施方案

已继续整理为 [Agent App 统一授权契约与实施 TODO](agent-appid-authorization-implementation-todo.md)，包含按侧求值、grant/token 与强制权限上限的组合、匿名 bearer 的保证边界、NFSP 文件资源 URI 提案、各模块实施顺序和回归矩阵。该方案仍是待实施文档，不表示已改变授权行为。
