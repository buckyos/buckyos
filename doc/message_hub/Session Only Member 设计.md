# Session Only Member 设计

- 版本：草案，2026-10-01；同日修订：补充与 SSO 的关系（§1.4），身份改为“迁移就绪”（§8）
- 状态：设计结论，**尚未实现**；实施前需要再评审，未定事项见 §12。文中“现有实现”按仓库 `573f7924` 核对。
- 前置：**实施排在 BuckyOS SSO 设计梳理之后。** 本文的身份与凭据部分（§3、§5.5、§6.1）是在 SSO 尚未提供“应用身份域”时的方案；SSO 设计完成后，按 §1.4 对齐或替换。
- 上游：
  - [SSO.md](<../sdk/SSO.md>)：BuckyOS SSO 使用者指南
  - [Self-Host-Groupv2.md](<./Self-Host-Groupv2.md>)（下称 v2），尤其是 §2.3.2 Session Guest、§2.4 双方同意、§4.1 判定顺序、§6.5 Guest 的加入与退出
  - [Message Tunnel Minimal Spec.md](<./Message Tunnel Minimal Spec.md>)：shadow endpoint DID
  - [Contact Mgr.md](<./Contact Mgr.md>)
  - [扩展 Self-host Group：应用开发指南](<../sdk/扩展self-host-group.md>)（下称 SDK 指南）§3.4、§6.1
- 目标读者：msg-center 开发者、Message Hub 开发者、客服类群应用开发者。

## 1. 背景与结论

### 1.1 场景

典型场景是客服群：客服是群成员，客户没有 BuckyOS 身份，既没有自己的 Zone，也没有本 Zone 的账号，但客户在另一个系统里有身份，例如手机号或 CRM 客户号。管理员或业务系统发给客户一个链接，客户在浏览器里打开，就能进入自己的工单 Session。客户每次都以同一个身份出现，历史保持连续。

### 1.2 现有实现为什么做不到

- msg-center 的群操作者只来自本 Zone verify-hub 签发的 session token（`group_service.rs` 的 `group_actor`）。群 HTTP 入口也用同一个函数（`group_http.rs` 的 `principal`）。
- verify-hub 不给 `UserType::Guest` 签发 token，外部客户也没有可以登录的账号。
- 现有的邀请链接（`GroupInviteLink.token`）只是入群准入凭据，持有人仍要先登录；用它加入得到的是全群 `Member` 角色，而不是 Session Guest。
- Session Guest 必须有 DID：`invite_session_guest` 需要事先知道 DID，`submit_guest_request` 要求调用者已经通过认证。
- 外部身份现有的通道是 shadow DID 加 tunnel attestation，但 tunnel 实例只有 Telegram 一种，而且 shadow DID 访问群 HTTP 入口会被拒绝（`shadow-endpoint-local-only`）。
- Message Hub 的 `/messagehub` 路由要求登录，数据层读的是登录用户自己的邮箱投影。

### 1.3 结论

1. 外部身份按 Telegram 的方式换算成稳定的 shadow DID（§3）。
2. 需要时在 Contact Mgr 里为该 DID 建立 contact；换算时已经顺带完成（§3.3）。
3. 群侧把该 DID 作为 Session Guest 拉进 Session，由身份源为它签发会话链接（§4、§5）。网关不需要改，Message Hub 只开放一个访客路由（§7）。
4. msg-center 自己解析会话链接和访客凭据，以该 DID 作为操作者，在独立分支里按方法白名单处理（§6）。
5. 访客凭据不是 BuckyOS token，其它系统服务都不认它（§6.4）。
6. 访客页面不能假设当前用户有 BuckyOS 身份。在 BuckyOS 看来，访客是匿名的（§7.3）。
7. 访客身份从第一天起满足迁移和联合登录的就绪条件，迁移流程等产品提出时再做（§8）。
8. 这是“应用自有账号 + 桥接到系统身份”的一个实例。其中的通用部分应由系统 SSO 提供，本文的身份与凭据方案在 SSO 设计完成后对齐（§1.4）。

Session Only Member 不是新的群成员类型。在群授权模型里，它就是 v2 的 Session Guest：群规则、能力、Hook 和可见区间全部沿用。本文只补充两件事：访客的身份从哪里来，访客凭什么访问。

### 1.4 与 SSO 的关系

推广到其它应用，应用的身份体系有两条路：

1. **DID 身份路线**：与系统同构，用户身份由自己的 Zone 或钱包背书，所有系统服务和其它 Zone 都认。门槛较高，开发者需要了解 DID 体系。
2. **应用自有账号 + 系统 SSO 整合**：应用相当于内置了一个自己的 verify-hub，用手机号、第三方登录等方式管理自己的用户；Zone 用户通过 SSO 登录后映射成应用账号。灵活度高，但身份只在应用内有效。

两者之间还有一档：由 Zone 的 verify-hub 接入手机号、Gmail 等外部账号，映射成本 Zone 的二级 DID（SSO.md 已列为规划中）。多个应用要服务同一批外部用户时，外部账号登录应在这一档做一次，而不是每个应用各做一遍，否则同一个人在不同应用里是互不相关的身份。

路线 2 的用户迟早要用到消息、群、分享这类系统能力。应用以自己的 App 身份代为调用时，系统分不清是谁：群里所有外部用户都成了同一个发送者，Hook、限流、封禁和审计都无法按人进行。所以需要一座桥，把应用账号映射成系统能识别的身份。**Session Only Member 就是“路线 2 + 桥”的一个实例**：身份源（例如 CRM）是路线 2 的应用，shadow DID 是映射出来的身份。

这座桥是通用需求，应当由**系统 SSO 作为基础设施提供**（下称“应用身份域”），而不是由各个系统服务分别发明访客凭据和影子身份。本文对 SSO 设计提出以下需求：

| # | SSO 需要提供 | 本文目前的临时方案 |
| --- | --- | --- |
| 1 | 身份域注册：应用声明身份域，只有该应用能在域内创建身份、代表域内身份 | 身份型 tunnel 实例（§3.2） |
| 2 | 命名规则：由“身份域 + 账号 ID”确定性生成 DID，不含隐私原文；命名空间挂在 Zone 本地，还是挂在可解析的 DID 之下 | `did:msgtunnel:<account_id>.user.<instance>`（§3.1） |
| 3 | 外部账号登录（手机号验证码、OAuth 等）与账号绑定，登录方式与身份分离 | 由身份源自行负责 |
| 4 | 域内身份的访问凭据：应用内嵌 IdP 签发（只有应用认），或 verify-hub 托管签发（系统服务也认） | msg-center 自行签发访客凭据（§5.5） |
| 5 | 代表调用：应用以 App token 声明“代表域内某个 DID”调用系统服务；权限不超过应用本身，并受资源方授权约束 | 身份源不能代访客操作（§3.2），访客自己持凭据访问 |
| 6 | 迁移与联合登录就绪（§8） | §8 的约束 |
| 7 | 生命周期：应用卸载后域内身份冻结、签发权回到 Zone owner | 未定义 |

路线 2 有一个固有代价：应用总能冒充自己的用户，系统只能把这种冒充限制在应用自己的身份域内。本文把身份源和访问通道分开（§5.1），因此比一般的路线 2 更严格；通用的路线 2 应用账号与界面合一，代表用户操作是天然的。

采用 verify-hub 托管签发时，会出现一种新的 token 主体类型（现有的是 User、Device、App、System、Agent）。msg-center 的邮箱授权对非 User 主体一律放行（§6.1）；task_manager、control-panel、sys_config 等服务同样按主体类型分支处理。引入新类型前，必须把这些分支逐一审计，改成默认拒绝。

SSO 设计完成后，本文 §3、§5.5、§6.1 按其结论对齐或替换；§4（加入 Session）、§6.2（方法白名单）、§7（Message Hub 访客入口）不受影响。

## 2. 术语

| 术语 | 含义 |
| --- | --- |
| Session Only Member（下称访客） | 没有 BuckyOS 身份，只以 Session Guest 身份参与某些 Session 的外部用户。身份来自外部系统，访问只依靠 host 签发的凭据 |
| 身份源 | 掌握访客外部身份并负责验证的系统，例如 CRM 或短信验证服务。它以 BuckyOS App 身份调用 msg-center |
| 身份型 tunnel 实例 | 在 msg-center 登记、只用来构造 shadow DID 的 tunnel 实例。它的 transport DID 是身份源，没有投递执行器 |
| 会话链接 | 身份源为某个访客签发的 URL，携带一次性的链接 token |
| 访客凭据 | 兑换会话链接后得到的访问凭据，只有 msg-center 认可 |

## 3. 身份

### 3.1 DID 构造

沿用 shadow endpoint DID：

```text
did:msgtunnel:<encoded_account_id>.<account_type>.<tunnel_instance_id>
```

- 身份源调用 `contact.resolve_did`，在 `profile_hint` 中带上 `account_type` 和 `tunnel_instance_id`，确定性地生成 DID：同一输入永远得到同一个 DID。
- `account_type` 取 `user`。
- `account_id` 使用身份源里**稳定且不含隐私信息**的主键，例如 CRM 客户号，**不要使用手机号原文**，原因如下：
  - DID 会写进 MsgObject 的 `from`，被 ObjectId 和签名覆盖，发出后不能修改；它还会出现在参与者列表、审计日志和远端成员 Zone 的副本里。用手机号原文，等于把它公开给所有参与者。
  - 手机号会被运营商回收。如果用号码当身份，新机主会继承上一位客户的 DID 和全部历史。
- 推荐身份源先为客户分配稳定的账号，手机号只作为绑定在账号上的凭据，这样客户换号时 DID 不变（§8.4）。
- 身份源只有手机号时，退而用 `HMAC(身份源密钥, E.164 号码)` 作为 `account_id`。这种情况下换号会产生新 DID，需要按 §8 迁移；号码回收也由身份源负责处理。
- 手机号等验证手段只放在 contact binding 的 `display_id` 里，并做脱敏，不进入 DID。

### 3.2 身份型 tunnel 实例

- `tunnel_instance_id` 是 DID 的一部分，必须**永久稳定、不可复用**，并按 Tunnel 规范以 `-tunnel` 结尾，例如 `crm-main-tunnel`。实例改名，等于所有访客都换了身份。
- msg-center 的 settings 增加身份型实例的声明，包含 `tunnel_instance_id`、`platform` 和 `transport_did`（即身份源的 DID）。重载 settings 时，与 Telegram 实例一样重建注册表；实例 ID 重复时启动失败。现在注册表只由 Telegram tunnel 在启动时登记（`main.rs` 中的 `register_tunnel`）。
- 身份型实例的 transport **不能代访客执行任何操作**：不接受它为该实例的 DID 提交的 attestation，也不允许它以访客 DID 发消息。与 Telegram 不同，这里访客本人在场；如果身份源能代访客发言或加入 Session，就等于能冒充客户。现有的 `group_message_actor` 和 `verify_tunnel_attestation` 要按实例类型区分。
- 身份型实例没有投递执行器。msg-center 在以下位置遇到这类 DID 时**不写 DeliveryRecord**，改由访客自己到 host 拉取：
  - 群消息写入后的投递循环（`group_service.rs` 中对 `msgtunnel` 成员生成 DeliveryRecord 的分支）；
  - `notify_group_member` 发出的个人通知，例如 Guest 邀请、移除通知。访客没有个人收件箱，这些通知不投递，对访客可见的状态变化通过 Session 变更流体现。
- 同一个外部账号经不同的身份型实例进入，会得到不同的 DID。这是 shadow DID 的既有语义。

### 3.3 Contact

- `contact.resolve_did` 在生成 DID 时已经建好 contact 和 binding，不需要单独一步。
- Contact 按 owner 分库。访客的 contact 建在群主或专门的客服 Agent 名下，用于与身份源对账、屏蔽和备注；各个客服自己的通讯录里不会有这条记录。
- 因此，**客服界面上的访客显示名不能依赖 Contact Mgr**，统一取自 Session 成员昵称（§4.3）。
- 判断“是不是同一个人”以及决定显示哪个身份时，统一经过规范身份解析（`resolve_canonical_did`），不直接比较原始 DID（§8 第 4 条）。

### 3.4 shadow DID 的边界

- shadow DID 只在本 Zone 内有意义。如果 Session 里有其它 Zone 的成员，他们看到的是一个无法解析的 DID，显示信息只能依赖 host 提供的参与者资料。这一点与 Telegram 访客相同。
- 访客 DID 没有密钥，无法签名。访客的消息由 host 背书，符合 v2 §1.5 “全听群主”的信任模型。
- 访客身份的签发链是：身份源（身份型实例的 transport）→ Zone owner。身份源负责日常签发；身份源停用或失联时，由 Zone owner 接管该实例的签发权，包括将来签发迁移声明（§8）。

## 4. 加入 Session

### 4.1 两种入口

| 入口 | 群侧动作 | 兑换时 |
| --- | --- | --- |
| 指定工单 | 具有 `session.invite_guest` 能力的成员调用 `group.invite_session_guest(member_did)`，记录状态为 `Invited` | 以访客身份执行 `accept_session_invitation`，状态变为 `Included` |
| 公开咨询 | 群配置开启 `membership.guest_entry` | 以访客身份执行 `submit_guest_request(request_id)`，host 按模板新建 Session。`request_id` 由链接记录确定，重复兑换得到同一个 Session |

无论哪种入口，访客都只能进入满足以下条件的 Session：具名、`allow_guests = true`、成员策略不是 `inherit`。这与 v2 §2.3.2 一致。

### 4.2 双方同意

- 群一方：发出邀请（指定工单），或开启 `guest_entry`（公开咨询）。
- 访客一方：访客在页面上点击“进入会话”触发兑换，这个动作就是访客的同意。审计中记录链接 ID 和兑换时间。
- 不用 tunnel attestation 替访客声明同意。attestation 转述的是外部平台上发生的事件，而这里访客本人在场。

### 4.3 显示名

- 身份源签发会话链接时可以附带一个显示名，例如“张先生”。兑换时，系统把它写成访客在该 Session 的成员昵称。
- 兑换由访客本人完成，写昵称的操作者就是访客自己，因此不需要放宽 `group.update_member_state` “只能修改本人昵称”的规则。访客之后也可以自己修改昵称。

## 5. 会话链接与访客凭据

### 5.1 职责分工

| 角色 | 负责 | 不能做 |
| --- | --- | --- |
| 群（Owner、Admin 及有相应能力的成员） | 把 DID 拉进 Session，或开放 `guest_entry`；移除、封禁访客；撤销链接 | 为访客签发会话链接 |
| 身份源 | 换算 DID、验证外部身份、为 DID 签发会话链接、撤销链接与凭据 | 绕过群规则把访客拉进 Session；代访客发言或加入 |
| msg-center | 校验链接与凭据，以访客身份执行加入，按 Guest 权限判定 | — |

**会话链接只能由该 DID 所属身份型实例的 transport（即身份源）签发。** 否则任何有邀请能力的客服都能给自己签发某个客户的链接，然后以客户身份发言。群主仍然掌握最终控制权，可以移除、封禁访客或删除 Session，但冒用访客身份需要身份源配合。

### 5.2 链接形式

```text
https://<zone_host>/messagehub/guest#t=<link_token>
```

- token 放在 `#` 之后，不会进入服务器访问日志，也不会随 Referer 外泄。页面取出 token 后，立即用 `history.replaceState` 从地址栏中删除。
- URL 中不带 group DID 和 Session ID，由服务端根据 token 解析。这样链接本身不会泄露 Session 的存在（v2 §4.1）。
- 链接 token 是至少 128 位熵的随机串，服务端只保存其哈希。

### 5.3 链接记录

```rust
pub struct GuestLink {
    pub link_id: String,
    pub token_hash: [u8; 32],          // 只保存哈希
    pub member_did: DID,               // 访客的 shadow DID
    pub target: GuestLinkTarget,       // Session(session_id) | GuestEntry
    pub display_name: Option<String>,
    pub issued_by: DID,                // 身份源的 transport DID
    pub expires_at_ms: u64,
    pub redeemed: Option<Redemption>,  // 兑换一次后失效
    pub revoked: bool,
}

pub struct Redemption {
    pub at_ms: u64,
    pub nonce_hash: [u8; 32],          // 兑换请求的幂等 nonce（§5.4）
    pub credential_id: String,
}
```

- 链接记录随所属群的状态保存，与现有 `invite_links` 一样纳入群事务和备份。
- 签发：`group.create_guest_link { group_did, member_did, session_id | guest_entry, display_name?, ttl_ms }`。调用者必须是 `member_did` 所属身份型实例的 transport。选择指定工单时，要求该 DID 在目标 Session 中已有 `Invited` 或 `Included` 记录；选择公开咨询时，要求群已开启 `guest_entry`。
- 撤销：`group.revoke_guest_link { group_did, link_id }`。身份源可以撤销，群内具有 `session.invite_guest` 能力的成员也可以撤销。
- 链接默认**只能兑换一次**。访客换设备或丢失凭据时，由身份源重新验证后签发新链接（§5.6）。

### 5.4 预览与兑换

1. 访客打开链接后，页面用 token 调用预览接口，展示群的公开资料和目标 Session 的标题。展示内容与 v2 §6.5.1 邀请中包含的信息相同。页面上提供“进入会话”按钮。
2. 访客点击按钮，页面生成一个随机 nonce，连同 token 提交到兑换接口。
3. msg-center 在群事务中依次执行：
   1. 校验 token 哈希：未过期、未撤销、访客未被封禁；如果已兑换过，只有 nonce 与当时一致时才返回原结果。
   2. 以访客 DID 为操作者，执行 `accept_session_invitation`（已经是 `Included` 时跳过）或 `submit_guest_request`。照常执行 `join` Hook、Guest 数量上限和频率限制。
   3. 写入显示名（如果有）。
   4. 把链接标记为已兑换，写入审计记录。
   5. 生成访客凭据（§5.5）。
4. 返回访客 DID、群 DID、Session 地址和访客凭据。此后页面只使用访客凭据。

预览接口和兑换接口都不要求 BuckyOS token，因此必须单独限流（按来源 IP 和 token 前缀）。失败时统一返回 `not-found`，不区分 token 不存在、已过期还是已使用。nonce 用于处理“响应丢失后重试”：同一浏览器带同一个 nonce 重试，会得到同一个结果，不会被当成重复兑换。

### 5.5 访客凭据

- 凭据是不透明的随机串，服务端只保存其哈希。不使用 JWT：凭据只有 msg-center 一处校验，而且撤销必须立即生效。
- 凭据记录包含：凭据哈希、访客 DID、群 DID、作用范围、来源链接 ID、签发时间、过期时间、最后使用时间、撤销标记。
- **作用范围**限定为一个群内、该访客作为 Guest 参与的 Session。即使同一个 DID 也是其它群的 Guest，凭据也不能访问那个群。是否进一步收窄到单个 Session，见 §12。
- 有效期按使用滑动续期，但设有上限，到期后需要新的链接。默认时长见 §12。
- 以下任一情况发生后，访客的下一次请求即失败：访客被移出或封禁、目标 Session 被删除、身份源或群侧撤销、凭据过期。
- 凭据的载体待定（§12）：
  - **HttpOnly cookie**：脚本读不到，可以抵御 XSS 窃取。但 kRPC 的 `RPCContext` 不含 cookie，需要 msg-center 在 HTTP 层把 cookie 转成内部凭据。cookie 名不能用 `buckyos_session_token`，Path 限定到 msg-center 入口，并设置 `SameSite=Strict`。
  - **kRPC 的 `token` 字段**：实现最简单。凭据带一个可识别的前缀（例如 `mhg_`），msg-center 在入口按前缀分流。凭据保存在页面内存或 `sessionStorage` 中，需要配合严格的 CSP。

### 5.6 身份长期有效，链接短期有效

- DID 一旦生成就不再改变；链接和凭据都可以过期、撤销和重新签发。
- 访客丢失链接或更换设备时，身份源重新验证外部身份（例如通过短信验证码），再为**同一个 DID** 签发新链接，历史自然接上。
- 一个访客可以同时持有多个有效凭据（多设备）。身份源可以按 DID 撤销该访客的全部凭据。
- 如果链接被转发，持有链接的人一旦兑换，就会以访客身份出现。一次性兑换加上较短的有效期，把这个风险限制在首次兑换之前；已兑换的链接再次被打开时，页面提示联系客服重新获取。

## 6. msg-center 的认证与授权

### 6.1 独立的认证分支

在 RPC 入口（`main.rs` 的 `handle_rpc_call`，现在位于 `group.*` 分流之前）、群 HTTP 入口（`group_http::serve`）和对象访问入口（`object_access::serve`）识别访客凭据，生成独立的访客身份，**不进入现有的 `CallerIdentity`**：

- `owner_session.rs` 中的 `authorize_owner_read_identity` 等邮箱授权，对非 User 类型的调用者直接放行。如果访客被映射成某种非 User 类型的调用者，就能读取任意用户的邮箱。
- `caller_identity` 在请求没有 token 也没有来源 IP 时，返回“内部调用”。访客路径绝不能产生这个结果。
- 访客请求先经过方法白名单，不在白名单中的方法直接拒绝，不进入各个 handler。

### 6.2 方法白名单

| 方法 | 限制 |
| --- | --- |
| `group.get_doc` | 公开资料 |
| `group.list_sessions` | 只返回凭据范围内、访客是有效 Guest 的 Session（现有过滤逻辑已经如此） |
| `group.get_shared_state` | 范围内的 Session |
| `group.list_session_members` | 按 v2 §3.3 规定的 Guest 可见范围 |
| `group.list_messages`、`group.changes` | 范围内的 Session |
| `group.update_read_marker`、`group.get_read_markers` | 只限本人，按 Session 回执规则 |
| `group.get_member_state`、`group.update_member_state` | 只限本人昵称 |
| `group.check_access` | 范围内的 Session |
| `group.leave_session` | 访客退出 Session |
| `msg.get_message` | 只走群消息授权（`authorize_group_message`），不进入 `authorize_message` 的邮箱分支 |
| `msg.post_send` | 只接受 `kind = group_msg`、`to = [凭据范围内的 group_did]`、`from = 访客 DID` |
| 群对象读取（HTTP `objects/<obj_id>`） | 必须带 `context_path`，按现有规则检查消息可读，且消息确实引用了该对象 |

其余方法一律拒绝，包括所有 `contact.*`、所有读写邮箱的 `msg.*`、所有群管理方法、`group.create`、`group.list_by_member` 和 `group.sync_joined`。

### 6.3 访客在群权限判定中的身份

- 访客凭据解析为 `GroupActor { did: 访客 DID, client: Some(<固定的访客客户端 ID>), remote: false }`，之后走 v2 §4.1 的全部判定：封禁、Guest 记录、Session 规则、频率限制和 Hook。
- `msg.post_send` 现有的“`from` 必须等于认证得到的操作者”这条规则自然成立。
- 群配置设置了 `access.allowed_clients.only` 时，必须显式列入访客客户端 ID，访客才能访问；未设置时默认允许。
- Hook 收到的 `actor` 是访客 DID，业务服务可以据此回查身份源。

### 6.4 与其它系统服务的隔离

- 访客凭据不是 verify-hub 签发的。其它服务用 `verify_trusted_session_token` 校验时一律失败，不需要额外配置。
- 风险只存在于 msg-center 内部，所以所有入口都必须先分流、再授权（§6.1）。

### 6.5 投递与更新

- 群消息写入后的投递循环和个人通知，都跳过身份型实例的 DID（§3.2）。
- kevent 需要 BuckyOS token，访客无法订阅。访客页面通过轮询 `group.changes` 获取更新。

## 7. 网关与 Message Hub

### 7.1 网关不需要改

- 网关把 zone 根域名映射到 control-panel service，`/kapi/msg-center` 也按 service 转发，这两条路径都不经过 `check_oauth`。`check_oauth` 只作用于应用子域名（见 `src/rootfs/etc/boot_gateway.yaml`）。
- 前端静态文件本来就是公开的（登录页也在同一个包里），登录拦截只在前端的 `main.tsx` 中进行。
- 因此，Message Hub 不需要在网关层变成 public app。需要放行的只有两处：前端的访客路由，以及 msg-center 对访客凭据的识别。

### 7.2 访客入口

- 新增一个独立路由，例如 `/messagehub/guest`，并加入 `publicRoutes.ts`。
- 访客入口只加载会话视图，不加载 `MessageHubView` 的实体列表、群管理、通讯录和桌面外壳，匿名访客不需要下载这些代码。
- 复用 `ConversationView`，数据层换成实现同一接口的 GuestStore。

### 7.3 当前假设 BuckyOS 身份的地方

| 位置 | 现在依赖 | 访客模式下的处理 |
| --- | --- | --- |
| `datamodel/sessionApi.ts` 的 `getAccountInfo`（3 处） | 从登录账号得到当前用户 DID | 抽象出可替换的身份来源；访客模式取兑换结果中的 DID |
| `api/store.ts` 的 `subscribeKEvent` | Zone token | 轮询 `group.changes` |
| 数据层的 `msg.list_sessions`、`list_session`、`get_record`、`update_record_state` 等 | 个人邮箱投影 | 直接读取 host 上的群存储：`group.list_sessions`、`group.list_messages`、`msg.get_message`、`group.update_read_marker`（SDK 指南 §6.1 的流程） |
| `contact.*` 调用 | 本人通讯录 | 名字和头像取自 `group.list_session_members` 和成员昵称 |
| 按原始 DID 判断同一个人、选择显示的身份 | 直接比较 DID | 统一经过规范身份解析（客服端与访客端都如此，§8） |
| `api/upload.ts`（经 control-panel 上传）、预览组件的对象解析 | 用户 token 和个人存储 | 读取走群对象 HTTP 路径；第一期不支持访客上传附件（§10） |
| `main.tsx` 的登录跳转 | 未登录时跳转到 `/login` | 访客路由免登录 |
| 个人会话操作（删除、归档、置顶等） | 个人邮箱投影 | 访客界面不提供 |

## 8. 迁移就绪与数据的长期可用性

### 8.1 原则

产品上说“不可迁移”，一般只是指今天的产品流程里不需要迁移。长生命周期的服务，总有一天产品会提出迁移。这和联合登录一样：任何产品今天都不能说“我不做联合登录”。

因此，本设计不提供“不可迁移”的身份。访客身份从第一天起就满足迁移和联合登录的就绪条件（§8.2），迁移流程本身等产品提出时再做（§8.3）。

迁移和联合登录是同一件事的两面，都依赖“身份不等于凭据”这一层间接：

- **联合登录**：多个凭据（手机号、第三方账号、以后的真实 DID）对应同一个身份。
- **迁移**：同一个身份改由另一个身份承接，例如新的 DID、新的 Zone、新的身份域。

迁移**不改写历史**。旧消息的 `from` 已被 ObjectId 和签名固定，永远是旧 DID。迁移在身份层进行：有资格的签发方发布一条“A 已由 B 接管”的声明，使用方解析 A 时得到 B。Contact Mgr 的 `merge_contacts` 和 `resolve_canonical_did` 已经有这种做法的雏形，但目前按 owner 分别存储、只能手动合并，还不是全系统都认可的机制。

### 8.2 第一天必须满足的约束

这些约束事后补救的代价很高，必须在实施时就落实：

| # | 约束 | 本文的落实 |
| --- | --- | --- |
| 1 | 账号主键是稳定、不含隐私信息的内部 ID；手机号、邮箱、外部账号都只是绑定的凭据，可以增加和更换 | `account_id` 用身份源的稳定主键，手机号只放在 binding 中（§3.1） |
| 2 | 数据里引用的是身份（DID），不是凭据 | 消息、成员记录、链接记录都引用访客 DID |
| 3 | 每个身份都能追溯到签发方及其上级，保证将来总有人有资格签发迁移声明 | 身份源 → Zone owner（§3.4） |
| 4 | 判断“是不是同一个人”以及显示身份时，统一经过规范身份解析，不直接比较原始 DID；今天这个解析可以原样返回输入 | 客服端和访客端都经过 `resolve_canonical_did`（§3.3、§7.3）。Message Hub 目前多处直接比较原始 DID，需要改造 |
| 5 | 保留身份来源和用户同意的审计记录，作为将来迁移的依据 | 签发、兑换的审计记录（§4.2、§9） |

授权不走规范身份解析：群成员资格等权限仍按原始 DID 判定，迁移后的权限由资源方决定（§8.3）。

### 8.3 等产品提出时再做

- 迁移声明的格式、签名（旧身份签发方的签名，以及新身份控制者表示同意的签名）和发布位置；
- 权限是否随身份迁移：身份层面（显示、消息归属）随声明自动转移；群成员资格等权限不自动转移，由资源方决定是否接受某个身份域的迁移；
- 迁移界面和批量迁移工具。

这些属于 SSO 的通用设施（§1.4 第 6 项），本文不单独设计。

### 8.4 典型场景

| 场景 | 处理方式 |
| --- | --- |
| 访客换手机号 | 身份源为同一个账号更换绑定，DID 不变。这属于联合登录，不需要迁移 |
| 手机号被回收、转给新用户 | 身份源停用旧绑定。新用户在身份源中是新账号，得到新 DID |
| 更换身份源（例如换了 CRM），或身份域改为 SSO 的应用身份域 | 用迁移声明把旧 DID 映射到新 DID |
| 访客有了真正的 BuckyOS DID | 访客用新 DID 签名表示同意，再发布迁移声明。新 DID 能否参与原 Session，由群决定，例如重新邀请并开放全部历史 |

### 8.5 数据的长期可用性

- **访客数据只存在于 host 上**：群成员被移出后，自己 Zone 里的副本仍然保留；访客没有 Zone，一旦被移出或 Session 被删除，就什么也看不到了。如果需要让访客留存记录，应在工单结束前提供导出，或发送一份副本（例如由身份源通过邮件发送）。
- **备份**：身份型实例的 settings（实例 ID 是身份的一部分）、链接记录（随群状态保存）和身份审计记录必须纳入 host 备份。访客凭据可以不备份：凭据丢失只影响登录，不影响身份，重新签发链接即可。

## 9. 安全要求

- 链接 token 和访客凭据只保存哈希，比较时使用常量时间算法。
- 预览和兑换接口要限流，失败时统一返回 `not-found`，并用 nonce 保证兑换的幂等性。
- 访客请求先分流并经过方法白名单，再进入授权判定，不进入 `CallerIdentity`。
- 会话链接只能由身份源签发；身份型实例的 transport 不能代访客发言，也不能代访客加入。
- **前置修复 `contact.resolve_did` 的授权**：现在调用它只需要带任意有效 token，`handle_resolve_did` 不再检查调用者，所以本 Zone 的任何主体都能向任意 owner 的通讯录写入 contact。修复后，身份型实例的 DID 只能由该实例的 transport 换算。
- 访客页面配置严格的 CSP；token 不放进 URL 的 query 参数。
- 访客被移出、封禁或凭据被撤销后，下一次请求立即失效。
- 审计记录覆盖签发、预览、兑换、撤销操作，以及凭据的最后使用时间。

## 10. 实施拆分

前置：完成 BuckyOS SSO 设计梳理，确定应用身份域、域内身份的凭据模型、命名空间和迁移声明的归属（§1.4）。下面的第一期按 SSO 的结论调整，其中第 1、2 项中的凭据部分、第 3 项可能由 SSO 的通用设施取代。

第一期：

1. 访客身份：身份型 tunnel 实例（或 SSO 的应用身份域）的声明与注册；群消息投递和个人通知跳过这类 DID；transport 不能代发言、代加入；修复 `resolve_did` 的授权。
2. 会话链接：`group.create_guest_link`、`group.revoke_guest_link`，以及预览接口、兑换接口和访客凭据表。
3. msg-center 访客分支：RPC 入口、群 HTTP 入口、对象访问入口的分流与方法白名单。
4. Message Hub：访客路由和 GuestStore；判断同一个人和显示身份改为经过规范身份解析（§8.2 第 4 条）。
5. 测试：在 `test_group_service.rs` 中覆盖以下场景：
   - 兑换及其幂等性；
   - 越权访问：其它群、邮箱、contact、群管理方法；
   - 移出、封禁、撤销后凭据失效；
   - 客服不能为客户签发链接；
   - `join` Hook。

第二期：访客上传附件（存入群存储并设置配额）、记录导出，以及没有身份源时的匿名访客（§12 第 5 项）。

## 11. 与现有文档的关系

- SSO：§1.4 的需求表和 §8 的迁移就绪约束，是对 SSO 设计的输入。SSO 设计完成后，本文按其结论修订。
- v2 §2.3.2、§6.5：访客是 Session Guest 的一种，规则不变。实现后在 v2 §6.5 中加一段说明，指向本文。
- Message Tunnel 文档：新增“身份型实例”这一类，它没有投递执行器，transport 不能代理访客。
- SDK 指南 §3.4：实现后补充访客链接的用法。

## 12. 待确认事项

标注“SSO”的事项由 SSO 设计决定，本文只给出在 SSO 结论出来之前的倾向。

| # | 问题 | 建议 |
| --- | --- | --- |
| 1 | 访客凭据的载体：HttpOnly cookie，还是 kRPC 的 `token` 字段（§5.5）。SSO | 随第 10 项决定。如果仍由 msg-center 签发，用 cookie，可抵御 XSS；需要 msg-center 在 HTTP 层支持 |
| 2 | 凭据的作用范围：单个 Session，还是该访客在本群的全部 Guest Session | 本群全部，访客可以查看自己的历史工单；对隔离要求更严的群可以按 Session 收窄 |
| 3 | 会话链接是否只能兑换一次 | 一次；多设备时由身份源重新签发 |
| 4 | 链接与凭据的默认有效期 | 链接短（例如 24 小时）；凭据按使用滑动续期，并设上限。具体数值待定 |
| 5 | 没有身份源时（管理员直接把链接发给匿名客户）怎么办 | 由 msg-center 内置一个匿名身份源，用随机 `account_id` 生成 DID。匿名访客也是一个账号，只是暂时没有绑定凭据；它同样要满足 §8.2，以后访客验证手机号时给这个账号追加绑定，DID 不变。绑定之前，身份的稳定性完全取决于凭据，丢失后无法重新签发 |
| 6 | 预览和兑换接口做成 RPC 方法还是 HTTP 路径 | 随第 1 项决定 |
| 7 | 公开咨询入口下，访客能否自己开新工单 | 允许，受 `max_open_per_guest` 限制 |
| 8 | 身份源的 transport DID 是否也必须是群成员 | 不需要。签发链接时只校验 transport 身份，以及目标 Session 中的 Guest 记录或 `guest_entry` 配置 |
| 9 | 访客上传附件 | 放到第二期 |
| 10 | 域内身份的访问凭据由谁签发：msg-center 自行签发（§5.5），还是由 verify-hub 托管签发。SSO | 倾向 verify-hub 托管：凭据体系只有一套，其它系统服务以后也能复用。前提是先完成 §1.4 所述的主体类型审计 |
| 11 | 访客 DID 的命名空间：沿用 `did:msgtunnel` 并把定义扩展为“由本 Zone 某个登记实例背书的外部账号”，还是采用 SSO 新定义的应用身份域 DID。SSO | 由于满足 §8.2 后总能迁移，这已不是阻塞项；但旧名字会永远留在已签名的历史和远端副本中，最好在首次实施前就定下来 |
