# ContactMgr 的 DID 自动构造问题 QA

本文记录 Message Tunnel、ContactMgr、MsgCenter、OpenDAN 在 DID 自动构造、确定性发送和 Contact 漂移上的设计结论，用作下一步代码修改参考。

## 1. 基本结论

### Q1: Message Object 里的 `to` 应该是什么？

A: `MsgObject.to` 必须是已经确定的 DID。

`MsgObject` 是 Named Object。它一旦构造完成，消息语义、对象 ID、签名、幂等和审计都应稳定。因此 `to` 不能承载“选择过程”，也不能包含还需要运行时解释的 route selector。

允许：

```text
to = did:bns:bob
to = did:msgtunnel:12345.user.tg-main
to = did:msgtunnel:-100123456.group.tg-main
```

不允许：

```text
to = did:bns:bob/tg
to = did:bns:bob?via=tg
to = did:bns:bob#telegram
```

这些写法表达的是“选择 Bob 的 Telegram binding”，不是最终 target DID。

### Q2: 一级 DID 和二级 DID 如何区分？

A: 一级 DID 是 BuckyOS canonical identity；二级 DID 是 Message Tunnel 暴露的平台 endpoint identity。

一级 DID 示例：

```text
did:bns:bob
did:web:alice.com
```

二级 DID 示例：

```text
did:msgtunnel:12345.user.tg-main
did:msgtunnel:-100123456.group.tg-main
did:msgtunnel:bob%40example.com.addr.email-main
```

二级 DID 的推荐结构：

```text
did:msgtunnel:<encoded_account_id>.<account_type>.<tunnel_id>
```

含义：

- `encoded_account_id`：平台账号、群、频道或地址的稳定 ID，需要做可逆编码，避免 `.`、`:`、`/` 等字符破坏 DID 解析。
- `account_type`：平台实体类型，例如 `user`、`group`、`channel`、`addr`。
- `tunnel_id`：具体 Message Tunnel 实现或实例的唯一 ID。每个 msg tunnel 实现都应有独立 tunnel id。

注意：上面的 `did:msgtunnel:*` 是目标设计格式，不是当前代码已经生成的 DID 格式。

当前实现里，`ContactMgr.resolve_did(platform, account_id, owner)` 会创建 `did:bns:mc-...` 形态的自动联系人 DID。例如 Telegram `platform = telegram`、`account_id = user:12345`、`owner = did:bns:alice` 时，大致会生成：

```text
did:bns:mc-alice-telegram-user-12345-<seq>
```

如果没有 owner scope，则 owner 部分来自系统 scope：

```text
did:bns:mc-system-telegram-user-12345-<seq>
```

二级 DID 也是合法 DID，但它的身份语义是“某个外部平台 endpoint”，不是 BuckyOS 的正式联系人身份。

### Q3: binding 是不是 DID 的别名？

A: 不应把 binding 定义成普通 alias。binding 更准确地说是一级 DID 到二级 DID 的可路由绑定。

例如：

```text
canonical contact DID: did:bns:bob
telegram endpoint DID: did:msgtunnel:12345.user.tg-main
binding: did:bns:bob --telegram--> did:msgtunnel:12345.user.tg-main
```

这里的 `telegram endpoint DID` 是目标模型里的 endpoint DID。当前实现里的自动 DID 更接近 shadow contact DID，它同时承担了“自动联系人”和“平台账号 endpoint”的角色。

这表示当用户想通过 Telegram 联系 Bob 时，可以先解析到 Bob 的 Telegram endpoint DID，然后再构造 `MsgObject`。

binding 可以有 alias/display_name/priority/last_active_at/default_session 等属性，但 binding 本身不是 `MsgObject.to` 中的目标表达式。

### Q4: `did:bns:bob/tg` 这种写法是否成立？

A: 可以作为 UI/API 层的便捷 target selector，但不应进入最终 `MsgObject.to`。

如果产品或 API 想支持：

```text
send to did:bns:bob/tg
```

它应该在 MessageBuilder 或 Send API 入口被解析为：

```text
resolve_target(did:bns:bob, selector=tg)
  -> did:msgtunnel:12345.user.tg-main

MsgObject.to = [did:msgtunnel:12345.user.tg-main]
```

当前代码还没有这样的 `resolve_target` API；当前最近似的入口是 `resolve_did(platform, account_id, owner)`，它返回的是 `did:bns:mc-...` 自动联系人 DID。

也就是说：

```text
Selector belongs to message construction.
Resolved DID belongs to MsgObject.
```

### Q5: 如果直接发给一级 DID，是否也是确定发送？

A: `to = did:bns:bob` 是确定的 Message Object target。

一级 DID 本身可以收消息。理想情况下，一级 DID 可以通过自己的 Message Hub 接收消息，不需要 tunnel binding。

但如果当前系统需要通过传统平台投递到这个人，那么发送前有两种选择：

1. 调用方先通过 selector 解析出二级 DID，再构造消息。
2. selector有“last_active / default"这种逻辑名字，可以明确的绑定到另一个2级DID（或指向一级）


## 2. Target 解析与发送流程

### Q6: 确定性发送的推荐流程是什么？

A: 先解析 target，再构造 message。

```text
user intent:
  send to Bob via Telegram

construction-time resolution:
  ContactMgr.resolve_target(
    canonical_did = did:bns:bob,
    selector = telegram
  ) -> did:msgtunnel:12345.user.tg-main

message object:
  MsgObject.to = [did:msgtunnel:12345.user.tg-main]

post_send:
  MsgCenter creates owner outbox and tunnel delivery for that endpoint DID.
```

这样 `MsgObject` 不依赖发送时的 binding 状态。即使 Bob 后来改了默认 binding，旧消息仍然清楚地表达“当时发给了哪个 endpoint DID”。

### Q7: `SendContext.preferred_tunnel` 的定位是什么？

A: 按现在的设计，

它可以用于：

- 同一个 endpoint DID 有多个 tunnel instance 可投递时选择 tunnel。
- 同一个平台 endpoint 可经多个 bot/account 发送时指定偏好。
- 兼容当前实现中已有的 tunnel 选择能力。

它不应该用于：

- 把 `to = did:bns:bob` 隐式改成 Bob 的 Telegram endpoint。
- 在多个 binding 之间选择一个实际收件人。
- 让 `MsgObject.to` 的含义依赖发送时上下文。

如果调用方需要“Bob via Telegram”，应该先调用 target resolver 得到二级 DID。

### Q8: `SendContext.extra.route.chat_id` 这类字段如何看待？

A: 它是 route hint，不是身份 target。

对于 Telegram 群、topic、thread、bot account 等平台维度，`SendContext.extra` 可以临时携带平台投递需要的上下文。但这些字段不能改变 `MsgObject.to/from`。

后续需要把这类字段 schema 化，避免不同调用方和 tunnel 私下约定。

### Q9: `post_send` 是否应该接受空 `to`？

A: 不应该。空 `to` 表示没有确定 target，应直接拒绝。

当前已经明确：如果 `msg.to` 为空，`post_send` 不应创建 sender outbox，也不应返回看似成功的结果。

## 3. Tunnel 入站 DID 自动构造

### Q10: 外部平台账号进入系统时是否一定要生成 DID？

A: 进入 `MsgObject.from/to` 的身份必须是 DID。

外部平台的原始 ID 不应直接写进 `MsgObject.from/to`。如果入站消息需要表达 sender、group、channel 等身份，就应先解析或构造二级 DID。

平台原始字段可以保留在：

- `IngressContext`
- `RouteInfo`
- `MsgRecord.route`
- `MsgObject.meta`
- `MsgContent.machine`

但这些字段是平台上下文，不是 message identity。

### Q11: 自动 DID 应该基于哪些字段生成？

A: 自动 DID 至少需要能区分平台实体和 owner scope。

待定输入包括：

- platform，例如 `telegram`
- entity kind，例如 `user`、`group`、`channel`

核心待决策点：

- 同一个 Telegram user 通过不同 bot/tunnel 进入时，是同一个二级 DID，还是不同二级 DID？
- 自动 DID 是全局唯一


### Q12: 自动 DID 的字符串格式是否要成为稳定协议？

A: 需要分层。

二级 DID 本身会进入 `MsgObject`，因此它一旦出现在消息历史里，就不能随意改变语义。

但 DID 字符串的具体编码格式可以先作为 beta 内部协议，只要满足：

- 可稳定复现或可通过索引查回。
- 不暴露不必要的敏感平台字段。
- 可以区分 user/group/channel。
- 可以在 ContactMgr 中反查到 platform endpoint。

### Q13: 入站消息中的 `from`、`to`、`session`、`route` 如何分工？

A: 推荐分工如下：

```text
MsgObject.from:
  入站消息的发送者 DID。可以是一级 DID，也可以是二级 DID。

MsgObject.to:
  入站消息的接收方 DID。私聊可指向本地 agent/user；群聊可指向 group DID 或平台 group endpoint DID。

MsgObject.thread/session:
  消息流、thread、topic、conversation 维度。

IngressContext / RouteInfo:
  tunnel、platform、chat_id、message_id、account_id、投递方向等平台路由信息。
```

## 4. Session 与消息流

### Q14: session 是不是 Contact？

A: 不是。session 是消息流维度，Contact 是身份维度。

一个 Contact 可以有多个 session：

- 和 Bob 的普通私聊
- 和 Bob 的某个工作上下文
- Bob 所在的 Telegram group
- Bob 的 email thread

一个 session 也可能包含多个 Contact：

- group
- channel
- multi-party thread

因此 session 不应混进 DID 的身份语义。

### Q15: 一级 DID 和二级 DID 是否都可以有 session？

A: 可以。

例如：

```text
to = did:bns:bob
session = worksession:xxx
```

也可以：

```text
to = did:msgtunnel:-100123456.group.tg-main
session = telegram-topic:42
```

`to` 决定身份目标，`session` 决定消息流归类，`route` 决定投递路径。

### Q16: UI 上看到的是 Contact 还是 session？

A: 产品上通常看到的是 session，但 session 需要能关联回 Contact / group / endpoint。

因此 UI 查询不能只按单个 peer DID 精确匹配。Contact merge 后，UI 需要能基于 canonical DID + alias endpoint DIDs 聚合历史。

## 5. Contact 漂移与合并

### Q17: Shadow DID 合并到正式 Contact 后，旧消息是否要改写？

A: 不改写旧 `MsgObject`。

旧消息里的 `from/to` 应保持当时的确定 DID。Contact 合并后，应通过 ContactMgr 建立 canonical DID、二级 DID、历史 alias DID 之间的关系。

查询、展示、发送前解析可以使用这层关系，但不要破坏 Named Object 的不可变性。

### Q18: 合并后的 source DID 应该删除吗？

A: 不应删除。




## 6. Contact、Binding、Route 的边界

### Q21: Contact 表示什么？

A: Contact 表示 BuckyOS 视角下的联系人实体，可以是正式一级 DID，也可以是尚未确认的 shadow/contact projection。

Contact 不应等同于某个平台账号。平台账号应由二级 DID 和 binding 表达。

### Q22: Binding 表示什么？

A: Binding 表示一个 canonical Contact DID 与一个 endpoint DID 之间的可路由关系。

Binding 至少需要表达：

- canonical DID
- endpoint DID
- platform
- account id / address
- tunnel preference
- owner scope
- priority / last_active_at
- verified / inferred / user_confirmed 状态



### Q24: ContactMgr、MsgCenter、Tunnel 的职责如何划分？

A: 建议职责如下：

```text
ContactMgr:
  管理 canonical DID、endpoint DID、binding、alias、merge、owner scope。

MsgCenter:
  保存 MsgObject 和 MsgRecord，执行 dispatch/post_send，调用 ContactMgr 解析 delivery plan。

Message Tunnel:
  连接外部平台，把平台事件转换成 DID + route context；消费 TUNNEL_OUTBOX 并发送到平台。

OpenDAN / UI:
  根据用户意图先解析 target，再构造确定 MsgObject。
```

## 7. Owner Scope

### Q25: ContactMgr owner scope 为什么是全局问题？

A: 因为同一个平台账号在不同 owner scope 下可能对应不同联系人关系。根本上，每个人都有自己玩去独立的Contact Mgr

需要明确：

- owner scope 是当前 user
- OpenDAN agent 发消息时，使用 owner user scope
- 同一个 Telegram user 在两个 owner scope 共享二级 DID
- binding 的 verified/user_confirmed 状态不跨 scope 共享


## 8. 新的设计约束



1. `MsgObject` 一旦构造完成，`from/to` 必须是确定 DID。
2. selector/path/binding choice 只能发生在构造 `MsgObject` 之前。
3. `SendContext` 删除
4. 自动生成的平台身份 DID 是二级 DID，不等同于正式 Contact DID。
5. Contact merge 不应改写历史 `MsgObject`。
6. session 是消息流维度，不是身份维度。
7. RouteInfo 是投递记录，不是身份真相源。
8. owner scope 必须在 ContactMgr API 中显式且一致。

## 9. TODO: 代码修改任务清单

本节基于当前实现 review，供后续 Code Agent 执行 breaking change。目标是落实新的设计约束：`MsgObject.from/to` 只保存确定 DID，Message Tunnel 外部 endpoint 使用二级 DID，删除 `SendContext`，target/binding 选择必须发生在构造 `MsgObject` 之前。

### T1: 删除 `SendContext` 并收窄 `post_send` API

当前事实：

- `SendContext` 定义在 `src/kernel/buckyos-api/src/msg_center_client.rs`，字段包括 `context_id/contact_mgr_owner/preferred_tunnel/priority/extra`。
- `MsgCenter.post_send`、`MsgCenterHandler.handle_post_send`、RPC request/response 和多处调用方都依赖 `Option<SendContext>`。
- 当前 `post_send_internal` 会从 `send_ctx.contact_mgr_owner` 推导 ContactMgr owner，并把 `send_ctx.priority/extra/preferred_tunnel` 写入 delivery route。

修改目标：

- 删除 `SendContext` 类型，或至少从 `post_send` API 中移除。
- `post_send` 不再接受 target selection、binding selection、tunnel selection 或 route extra。
- 如确实需要保留发送队列控制，新增窄类型，例如：

```rust
pub struct PostSendOptions {
    pub priority: Option<i32>,
}
```

但第一版优先考虑 `post_send(msg, idempotency_key)`，避免重新引入隐式 route 语义。

需要修改：

- `src/kernel/buckyos-api/src/msg_center_client.rs`
- `src/frame/msg_center/src/msg_center.rs`
- `src/frame/msg_center/src/test_msg_center.rs`
- 所有 `.post_send(msg, Some(SendContext { ... }), ...)` 调用点。

验收标准：

- 仓库中不再存在业务代码构造 `SendContext`。
- `post_send` 不再因为 context 改变 `MsgObject.to` 的投递目标。
- `post_send` 对 `msg.to.is_empty()` 继续显式失败。

### T2: 实现 Message Tunnel 二级 DID 生成规则

当前事实：

- `ContactMgr.resolve_did(platform, account_id, owner)` 当前生成 `did:bns:mc-<owner>-<platform>-<account_id>-<seq>`。
- 该 DID 当前同时承担 shadow contact 和平台 endpoint 两种语义。
- Telegram tunnel 入站通过 `handle_resolve_did("telegram", account_id, profile_hint, owner)` 获得 sender/chat DID。

修改目标：

- 新增统一 helper 生成二级 DID：

```text
did:msgtunnel:<encoded_account_id>.<account_type>.<tunnel_id>
```

- `encoded_account_id` 必须可逆编码。
- `account_type` 使用稳定枚举：`user/group/channel/addr` 等。
- `tunnel_id` 来自 tunnel 配置或 tunnel 实现注册信息，必须稳定且不可复用。
- 当前 Telegram user/group/channel 入站应生成二级 DID，而不是 `did:bns:mc-*`。

需要修改：

- `src/frame/msg_center/src/contact_mgr.rs`
- `src/frame/msg_center/src/tg_tunnel.rs`
- `src/kernel/buckyos-api/src/msg_center_client.rs` 中 Contact/Binding 共享类型。

验收标准：

- Telegram user `account_id=user:12345`、`account_type=user`、`tunnel_id=tg-main` 能稳定生成类似 `did:msgtunnel:12345.user.tg-main` 的 DID。
- 群组、频道、邮箱地址等包含特殊字符的 account id 能正确编码和反解。
- 同一 `(account_id, account_type, tunnel_id)` 重复解析得到同一个 DID。

### T3: 拆分 shadow contact DID 与 endpoint DID

当前事实：

- `Contact.did` 当前既可能是正式联系人 DID，也可能是 `AutoInferred` 的自动联系人 DID。
- `AccountBinding` 当前只有 `platform/account_id/display_id/tunnel_id/last_active_at/meta`，没有明确的 `endpoint_did` 字段。
- `binding_index` 当前按 `(platform, account_id)` 指向 contact DID。

修改目标：

- `Contact.did` 表示一级联系人或本地联系人投影，不再直接表示平台 endpoint。
- `AccountBinding` 增加 `endpoint_did: DID`。
- ContactMgr 增加 endpoint DID 索引：

```text
endpoint_did -> canonical/contact DID
(platform, account_id, account_type, tunnel_id) -> endpoint_did
```

- `resolve_did` 的语义要拆清楚：解析平台 endpoint 时返回二级 DID；查联系人时返回 canonical/contact DID。

建议 API：

```text
resolve_endpoint_did(platform, account_id, account_type, tunnel_id) -> endpoint_did
bind_endpoint(contact_did, endpoint_did, owner_scope)
resolve_contact_for_endpoint(endpoint_did, owner_scope) -> Option<contact_did>
```

验收标准：

- 一个正式 Contact 可以绑定多个二级 DID。
- 一个二级 DID 可以被反查到 endpoint metadata。
- ContactMgr 不再需要通过 `did:bns:mc-*` 伪造平台 endpoint。

### T4: 增加构造前 target resolver，替代 `preferred_tunnel`

当前事实：

- `MsgCenter.build_delivery_plan` 现在会对 `target_did` 调 `get_preferred_binding`，按 `last_active_at` 选择 binding。
- `SendContext.preferred_tunnel` 只覆盖 route 的 `tunnel_did`，不能确定 binding。
- 这会导致 `MsgObject` 构造后仍有 route selection 行为。

修改目标：

- target/binding 选择必须发生在构造 `MsgObject` 之前。
- 增加 resolver API：

```text
resolve_target(contact_did, selector, owner_scope) -> endpoint_did
```

- selector 可以是 tunnel id、platform、binding alias 或更严格的枚举；第一版建议优先支持 `tunnel_id`。
- selector 找不到唯一 binding 时必须失败，不做 fallback。

验收标准：

- “Bob via Telegram” 调用方先解析到 `did:msgtunnel:*`，再构造 `MsgObject.to`。
- `post_send` 不再使用 `get_preferred_binding` 为一级 DID 隐式选路。
- `get_preferred_binding` 如保留，只用于 UI 默认建议，不用于 `post_send` 的确定性投递。

### T5: 重写 `post_send` delivery plan 逻辑

当前事实：

- `build_delivery_plan(target_did, send_ctx, contact_mgr_owner)` 会读取 ContactMgr binding，填充 `RouteInfo.platform/account_id/address/tunnel_did`。
- 找不到 binding 时会 fallback 到 `did:bns:msg-center-default-tunnel`。
- `RouteInfo.target_did` 当前记录传入的 target DID，但 target 可能是一级 DID。

修改目标：

- `post_send` 只接受已经确定的 `MsgObject.to`。
- 如果 `to` 是 `did:msgtunnel:*`，从二级 DID 反解或查索引得到 `tunnel_id/account_id/account_type/platform`，生成 `TUNNEL_OUTBOX`。
- 如果 `to` 是一级 DID，优先走 BuckyOS Message Hub 直达逻辑；没有直达能力时应失败或 pending，不能静默 fallback 到任意 tunnel。
- 删除 default tunnel fallback，除非明确目标 DID 就是某个 default tunnel endpoint。

验收标准：

- `to=did:msgtunnel:*` 能生成确定的 tunnel delivery。
- `to=did:bns:bob` 不会按 `last_active_at` 隐式投递到某个传统平台 binding。
- 找不到 route 时返回明确错误或 failed delivery，不返回“看似成功但无确定目标”。

### T6: 调整 Telegram tunnel 入站与回发路径

当前事实：

- Telegram tunnel 入站使用 `handle_resolve_did` 为 sender/chat 生成 DID。
- OpenDAN 回发时可能使用 `SendContext.preferred_tunnel` 和 `SendContext.extra.route.chat_id`。
- `tg_route_extra_for_session` / `with_tg_route_chat_id` 把 Telegram `chat_id` 塞进 `SendContext.extra.route`。

修改目标：

- Telegram 入站 sender/chat 直接解析为 `did:msgtunnel:*` 二级 DID。
- 入站 `MsgObject.from/to` 使用二级 DID 或正式一级 DID，但必须确定。
- Telegram chat_id/topic/thread 等会话信息放入 session/thread/RouteInfo，不再通过 `SendContext.extra` 影响 target。
- 回发旧 session 时，如果 session 保存了 endpoint DID，则直接构造 `to=endpoint_did`。

验收标准：

- OpenDAN 回复 Telegram 消息时不再构造 `SendContext`。
- Telegram 群、频道、私聊均能生成稳定二级 DID。
- 群 topic/thread 仍作为 session/thread 维度，不混入 DID。

### T7: 调整 OpenDAN、Control Panel 和 workflow 调用方

当前调用点：

- `src/frame/opendan/src/agent.rs`
- `src/frame/opendan/src/agent_session.rs`
- `src/frame/control_panel/src/message_hub.rs`
- `src/kernel/workflow/src/send_message_executor.rs`

修改目标：

- 所有发送调用方先拿到确定 DID，再构造 `MsgObject`。
- 删除 `SendContext` 构造。
- Control Panel 如果用户选择某个 contact + channel，应先调用 target resolver。
- OpenDAN session meta 中应保存 endpoint DID / canonical DID / session id 的清晰边界。

验收标准：

- `rg "SendContext \\{" src` 无业务调用点。
- `rg "preferred_tunnel" src` 不再出现在发送路径。
- OpenDAN 和 Control Panel 的发送路径不依赖 `extra.route.chat_id` 决定收件人。

### T8: Contact merge 增加 alias 和历史查询支持

当前事实：

- `merge_contacts_in_store` 会删除 source contact，并迁移 bindings、grant、groups、tags。
- 当前没有 alias/tombstone 结构。
- 历史 `MsgObject` 不会改写，旧 DID 需要可解释。

修改目标：

- merge 后保留 source DID 为 alias / merged source / tombstone。
- ContactMgr 提供：

```text
resolve_canonical_did(did, owner_scope) -> canonical_did
list_alias_dids(canonical_did, owner_scope) -> Vec<DID>
```

- UI 和 OpenDAN 历史查询使用 canonical DID + aliases 聚合。

验收标准：

- 合并 shadow contact 到正式 contact 后，旧消息仍能在正式 contact 的会话视图中出现。
- 旧 session 保存的 peer DID 可以 resolve 到当前可用 DID。
- source DID 不会因为 merge 后删除而失去解释能力。

### T9: 更新测试

必须覆盖：

- `post_send` 拒绝空 `to`。
- `post_send` 不接受或不使用 `SendContext`。
- `did:msgtunnel:*` 生成、解析、编码/解码。
- Telegram user/group/channel endpoint DID 稳定性。
- `to=did:msgtunnel:*` 生成确定 tunnel delivery。
- `to=did:bns:bob` 不隐式选择 traditional platform binding。
- selector 找不到唯一 binding 时失败。
- Contact merge 保留 alias，历史查询可聚合 old DID。

建议命令：

```bash
cd src
cargo test -p buckyos-api msg_center
cargo test -p msg_center
cargo test -p opendan
uv run buckyos-build.py --skip-web
```

### T10: 文档联动

需要同步更新：

- `doc/message_hub/Message Tunnel Design.md`
- `doc/message_hub/Contact Mgr.md`
- `doc/message_hub/Message Center.md`
- `doc/message_hub/Message Tunnel Minimal Spec.md`

重点保持一致：

- 二级 DID 规则：`did:msgtunnel:<encoded_account_id>.<account_type>.<tunnel_id>`。
- `MsgObject.from/to` 必须是确定 DID。
- `SendContext` 删除。
- selector 只属于 message construction 阶段。
- Contact merge 不改写历史 `MsgObject`，通过 alias/canonical 查询解决。
