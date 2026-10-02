# 扩展 Self-host Group：应用开发指南

本文面向希望在 BuckyOS 群聊上增加业务能力的应用开发者。实现核对日期为 **2026-10-01**，基于当前仓库 `573f7924`；“已实现”表示当前源码存在相应执行路径，不代表已经完成真实部署或跨 Zone 联调。

**现在可以开发配置生成器、HTTP Hook 和使用默认群服务的自定义应用；架构稿中的嵌入式应用侧面板、通用 Process Chain、自动跨 Zone 接入和群服务多副本仍需补齐。** 最容易落地的组合是 L1，或 L1 + L3 + L4A。

本文将 [应用扩展架构设计](<../message_hub/BuckyOS Self-host Group的应用扩展架构设计.md>) 的构想对应到现有代码。协议背景见 [Self-host Group v2](../message_hub/Self-Host-Groupv2.md)，完整 RPC 参数见 [群服务后台接口](../../src/frame/msg_center/doc/group-v2-backend.md)。架构稿中的长期兼容性属于设计目标；当前仓库仍处于 beta 2.2 开发阶段，不能据此假定 API、配置或 UI 源码已经有长期兼容承诺。

## 1. 选择扩展方式

| 方式 | 当前状态 | 开发者现在能做什么 | 尚缺的能力 |
| --- | --- | --- | --- |
| L1 配置型扩展 | 核心已实现，展示配置部分待实现 | 原子创建群、初始 Session 和邀请；配置成员策略、能力、Session 模板和消息规则；初始化程序随后退出 | `display` 的业务链接、文件、指定成员等通用渲染；完整配置编辑 UI |
| L2 嵌入式应用侧面板 | **待实现** | 可以先提供独立业务页面，为后续嵌入做准备 | 面板声明解析、HTML 加载、Group/Session 上下文传递、生命周期、来源校验和跨来源接受机制 |
| L3 Hook / Process Chain | HTTP Hook 已实现，通用处理链待实现 | 在 `join`、`post`、`read` 上调用应用 HTTP 服务，拒绝操作或按配置提供动态发言许可 | 通用 Process Chain 接入、其他 Hook 点、返回消息修改或持久化权限变更指令、默认 UI 的 Hook 信息展示 |
| L4A 自定义 UI | 服务调用与源码复用可做 | 自己实现业务界面，调用 `group.*`、`msg.*`；按需复制 Message Hub 组件和数据层 | 独立发布的稳定聊天组件 SDK；跨 Zone 应用身份的标准证明 |
| L4B 独立协议实现 | HTTP 协议基础已实现，完整互操作待验证 | 按 v2 协议实现服务或客户端，在显式路由与可验证凭据下对接 | 自动 endpoint 发现、自动建立 Joined Group 绑定、凭据续期，以及不同独立实现之间的联调 |

配置生成器、业务页面和 Hook 服务可以分别部署。只配置群规则的程序无需常驻；绑定 Hook 后，对应操作会依赖业务服务的可用性。

## 2. 接入前先分清身份、地址和状态

默认 Self-host Group Service 内置于 `msg-center`，Message Hub 是它的默认 UI。应用复用群服务时，无需另起一个群消息内核。

- **Group DID** 是群实体的地址，消息接收者是群。默认建群会在 host Zone 下生成二级 DID，也可以通过 `group_id` 指定合法子标签。自备一级 DID 必须预先发布授权当前 host 和调用者 controller 的群文档。
- **操作者**来自服务验证过的 token。不要传 `actor_did` 或 `host_owner`，服务会拒绝这两个参数。建群时，认证操作者成为群 Owner；应用不能仅靠请求参数替用户建群或冒充成员。
- **默认 Session** 的 RPC `session_id` 省略或为 `null`，地址是裸 Group DID。**具名 Session** 的 RPC `session_id` 是原始字符串，例如 `officers`；返回的 `session` / `state_ref.session_key` 是规范 MailboxAddress，例如 `did:web:guild.example/officers`。
- **Group Configuration** 是规则来源；GroupDoc 是可公开读取的摘要。修改 GroupDoc、本地联系人或 UI 状态不能改变群授权。
- **群的 Session** 是 host 权威管理的对象；`msg.*` 的个人 Session 是成员自己的消息投影。创建前者用 `group.create_session`，不要用 `msg.create_session` 代替。

Group 消息保持 `from = 实际发送主体`、`to = [group_did]`、`kind = "group_msg"`。具名会话通过 `to_session` 指定原始 Session ID；默认会话省略该字段。`thread.topic` 不能代替路由字段。

应用需先完成自身的运行时初始化与登录，取得服务认可的身份，参见 [SSO 接入](SSO.md)。建群另外需要 Zone RBAC 的 `obj://msg-center/group` / `create` 权限；现有默认策略授予 `users` 和 `admin`。群内操作再由成员资格、角色能力、Session 参与关系及客户端限制判定。

以下 TypeScript 示例沿用 Message Hub 的 Web SDK 调用方式。前提是 `buckyos` 已完成当前应用的初始化和登录；示例中的域名、DID 和应用 ID 要替换为实际部署值。

```ts
import { buckyos } from 'buckyos'

const rpc = buckyos.getServiceRpcClient('msg-center')
const call = <T>(method: string, params: Record<string, unknown>) =>
  rpc.call<T, Record<string, unknown>>(method, params)
```

服务入口是 `/kapi/msg-center`。当前 Message Hub 的 `datamodel/sessionApi.ts` 已封装常用群操作，但其 `createGroup` 等 UI 包装只暴露部分参数；完整业务配置可以像本文一样直接调用 RPC。当前 Rust `MsgCenterClient` 也没有覆盖 v2 全部 `group.*` 方法的强类型包装，后端应用可使用现有 kRPC 客户端按同一方法名调用。

## 3. L1：用配置创建一个业务群

### 3.1 一次性创建群及初始 Session

下面创建一个公会群：默认 Session 用于普通聊天，公告 Session 只允许 Owner/Admin 发言，管理 Session 只对 Owner/Admin 开放。配置、初始 Session 和邀请在同一事务中创建；任何一步失败都会回滚。

```ts
type CreatedGroup = { group_did: string; revision: string }

const createRequest = {
  idempotency_key: crypto.randomUUID(),
  group_id: 'guild',
  configuration: {
    schema_version: 1,
    required_features: ['sessions'],
    profile: { name: '游戏公会', description: '公会日常交流与公告' },
    membership: {
      join_policy: 'request_and_approve',
      member_list_visibility: 'members',
    },
    default_session: {
      post: 'all_participants',
      history: 'from_join',
    },
    session_templates: {
      announcement: {
        membership: 'inherit',
        rules: { post: { only: ['owner', 'admin'] }, history: 'all' },
      },
      officers: {
        membership: { roles: ['owner', 'admin'] },
        rules: { history: 'from_join' },
      },
    },
    extensions: {
      'com.example.guild': { business_group_id: 'guild-001' },
    },
  },
  sessions: [
    {
      session_id: 'announcements',
      template: 'announcement',
      title: '公会公告',
      announcement: '请先阅读公会规则',
    },
    { session_id: 'officers', template: 'officers', title: '管理讨论' },
  ],
}

const created = await call<CreatedGroup>('group.create', createRequest)
const groupDid = created.group_did
```

`group.create` 实际返回 `{ group_did, revision }`；需要公开 Doc 或 Session 列表时再调用 `group.get_doc`、`group.list_sessions`。保存建群请求及其幂等键，网络超时时用原请求重试。相同主体和键返回原结果，改变参数后复用该键会得到 `idempotency-key-reused`。

不传 `group_id` 时服务自动生成标签。显式指定标签时，已有群或删除墓碑都不能复用。配置中省略的标准字段使用服务默认值；`revision` 由服务生成。上例的 `com.example.guild` 是应用自行约定的数据命名空间，默认群服务不会解释它。

**创建模板不会创建 Session。** 具名 Session 的规则来自所引用模板叠加 `rule_overrides`，未引用模板时用内置默认规则；它不会继承 `default_session`。因此，默认会话上的发言限制或 Hook 不会自动覆盖具名会话。

### 3.2 当前可配置范围

| 配置位置 | 当前生效内容 |
| --- | --- |
| `membership` | `join_policy`：`invite_only` / `request_and_approve` / `open`；成员列表可见性：`members` / `admins_only` / `public`；Guest 请求入口模板 |
| `roles` | `admin`、`member` 的能力集合。Owner 的固定能力不由配置撤销；默认 Admin 没有 `group.read_all`、`group.update_role` |
| `default_session` | 默认会话的发言、回应、历史、编辑/撤回窗口、回执、慢速模式和 Hook |
| `session_templates` | 具名会话模板，包含 `membership` 和 `rules`；实际会话通过模板与覆盖项生效 |
| `limits` | 成员、Session、Guest、消息大小和频率上限；只能在平台允许范围内收紧，不能任意提高 |
| `access` | 本地经过验证的应用身份白名单；远端未验证客户端身份的处理方式 |
| `display` / `extensions` / `rules.panels` | 有存储位置；任意业务展示及面板加载**待实现**，保存成功不代表默认 UI 会使用它们 |

Session 成员策略采用以下 JSON：`"inherit"`、`{"roles":["owner","admin"]}` 或 `"explicit"`。发言和回应规则采用 `"all_participants"`、`{"only":["owner","admin"]}` 或 `"nobody"`。历史范围是 `"all"` 或 `"from_join"`。

当前配置 `schema_version` 只接受 `1`。`required_features` 仅识别 `sessions`、`session_guests`、`message_relations`、`read_markers`、`hooks`、`allowed_clients`；未知必需能力会报 `unsupported-required-feature:<name>`。未知顶层字段以及 `extensions` 数据会保留，但不会自动成为运行逻辑。完整字段和默认值以 [group_types.rs](../../src/frame/msg_center/src/group_types.rs) 为准。

`group.get_config` 对有相应读取能力的群成员开放，业务配置中应只保存适合该读取范围的数据。凭据和业务数据库由应用服务管理。

### 3.3 更新配置和会话状态

配置更新使用 revision 比较和 JSON Merge Patch 语义：对象递归合并，数组整体替换，`null` 删除键。

```ts
const config = await call<{ revision: string }>('group.get_config', {
  group_did: groupDid,
})
const patchRequest = {
  group_did: groupDid,
  expected_revision: config.revision,
  idempotency_key: crypto.randomUUID(),
  patch: { profile: { description: '每周六组织活动' } },
}
await call<{ revision: string }>('group.apply_config', patchRequest)
```

发生 `revision-conflict:<current>` 时，重新读取配置，检查并发修改后生成新的请求和幂等键。网络结果未知时则保留旧请求重试。修改模板会影响已经引用它的 Session；删除仍被引用的模板会失败。替换 `roles.member` 或 Hook 数组时，要提交希望保留的完整集合。

Session 的标题、说明和公告使用共享状态接口。它有独立 revision，不使用配置 revision 或 Session 记录 revision。

```ts
const shared = await call<{ revision: string }>('group.get_shared_state', {
  group_did: groupDid,
  session_id: 'announcements',
})
await call('group.update_shared_state', {
  group_did: groupDid,
  session_id: 'announcements',
  expected_revision: shared.revision,
  idempotency_key: crypto.randomUUID(),
  set: { announcement: '本周活动时间：周六 20:00' },
  unset: [],
})
```

当前共享状态只允许 `title`、`description`、`announcement`；个人成员状态只允许本人修改 `nickname`。标题/昵称最多 64 字符，说明/公告最多 1024 字符。不支持通过这些接口写入积分、权限或任意 `extensions` 状态；这类业务状态放在应用数据库中。

### 3.4 邀请、审批和客服 Guest

邀请用 `group.invite_member(member_did, role?)`，返回 `invite_id` 与状态。成员接受时调用 `group.accept_invitation`，参数名是 `invitation_id`，必须使用本次邀请的 ID。

群侧与成员侧都同意才进入有效成员状态：Owner/Admin 的邀请在接受时仍满足审批能力则直接激活，普通成员的邀请需要管理员审批；主动申请受 `join_policy` 控制。当前已取消成员 proof，不应照旧版设计增加签名入群步骤。同 Zone 好友可能被自动接受；Agent 仅自动接受其 owner 的邀请，其他邀请由 owner 确认。不要假定邀请 RPC 返回后一定处于 `invited` 或 `active`，应读取返回的 `state`。

客服场景可配置 `membership.guest_entry` 引用一个 `allow_guests: true` 且成员策略为 `explicit` 或限定角色的模板，通过 `group.submit_guest_request(request_id)` 原子创建会话。已有具名 Session 可使用 `group.invite_session_guest` / `group.accept_session_invitation`。默认 Session、`inherit` 会话不能允许 Guest；Guest 只获得指定 Session 的参与权，不成为全群成员。

## 4. L2：嵌入式应用侧面板（待实现）

当前后端能保存 `extensions` 和 `SessionRules.panels: string[]`，但 Message Hub 没有按这些声明加载业务 HTML 的代码。`MessageHubAppPanel.tsx` 是桌面承载 Message Hub 本身的入口，不是群应用扩展面板。

v2 设计提议在 `extensions.panels` 中声明 `id`、`title`、`entry_url`、`scope`，由 Session 规则引用面板 ID。这仍是设计约定，没有完成执行和校验，不能把它当成可用的面板 SDK。

要让架构中的 L1 + L2 真正成立，还需要实现：

1. 解析群面板声明，在实体/会话侧栏加载页面，并处理加载失败、切换和卸载。
2. 明确 Group DID、原始 Session ID 和规范 Session 地址的上下文协议，以及初始化和会话切换通知。
3. 以 Group host 的可信来源为基准校验入口；跨来源默认阻止，并提供明确的用户接受、撤销及来源变更处理。
4. 定义面板如何以当前用户或授权应用访问服务，区分页面加载许可与消息权限。
5. 验证面板超时、服务错误和页面崩溃只影响面板，基础聊天仍可用。

同一 OOD/Zone 上部署的两个子域不自动满足浏览器同源条件，同源仍由协议、主机和端口决定。上述信任映射也需要在面板实现中明确。

现在可以把业务页面作为 L4A 的独立应用，直接调用群服务；不能通过配置 URL 获得自动嵌入或凭据传递。

## 5. L3：接入 HTTP Hook

### 5.1 已有 Hook 点与边界

Hook 是 `SessionRules.hooks` 中按顺序执行的 HTTP 绑定，不是任意群操作的插件。当前实现没有把通用 Process Chain 接入群服务。

| `point` | 执行位置 | 许可范围 |
| --- | --- | --- |
| `join` | 群成员激活前；Session Guest 接受邀请/请求时 | 只能在原生流程允许的基础上放行或拒绝；不会替代邀请和审批。群成员激活使用默认 Session 的 `join` Hook，Guest 使用目标 Session 的规则 |
| `post` | 消息写入前，包括关系消息；在关系校验、限流等检查后 | 可以拒绝；`may_grant: true` 时可对原生无发言许可者动态放行 |
| `read` | Session 列表、读取、同步、对象访问等对应读取路径 | 可以拒绝整个 Session 的读取；不能扩大原生历史范围，也不做逐条消息过滤 |

`post` 的动态许可只作用于本次消息，不写入角色或能力集合。它可覆盖原生发言/回应规则、发言能力或禁言造成的拒绝，但不能绕过有效参与关系、封禁、客户端限制、归档状态、消息关系合法性、@all 能力、消息大小及限流等前置检查。

### 5.2 配置绑定

下面在默认会话上增加业务入群检查。具名会话上的 Hook 要另外放到相应模板 `rules.hooks` 或 Session 的 `rule_overrides.hooks` 中。

```ts
const beforeHook = await call<{ revision: string }>('group.get_config', {
  group_did: groupDid,
})
await call('group.apply_config', {
  group_did: groupDid,
  expected_revision: beforeHook.revision,
  idempotency_key: crypto.randomUUID(),
  patch: {
    default_session: {
      hooks: [{
        point: 'join',
        service: 'https://guild.example/api/group-hook',
        may_grant: false,
        on_unavailable: 'deny',
        timeout_ms: 1500,
        result_ttl_ms: 0,
      }],
    },
  },
})
```

`service` 必须是可从 msg-center 访问的 HTTP/HTTPS URL，不能带 URL 用户名/密码；调用不跟随重定向。`timeout_ms` 范围为 1–30000。`may_grant: true` 只允许用于 `post`。Hook 数组会整体替换，更新时合并需要保留的绑定。

### 5.3 HTTP 请求、认证和返回值

Group Service 发出 `POST`，JSON body 例如：

```json
{
  "operation": "join",
  "actor": "did:web:alice.example",
  "group_did": "did:web:guild.example",
  "session_id": null,
  "msg": null
}
```

具名会话的 `session_id` 为原始字符串。`post` 请求的 `msg` 是完整 MsgObject，包含消息内容；当前请求没有另外传入客户端 ID、配置 revision 或完整成员表。

请求的 `Authorization` 优先使用 msg-center 的 BuckyOS runtime 服务 token；无可用 runtime token 时使用服务 Settings 的 `cyfs_dispatch.group_hook_authorization`。两者都没有时不会发出 HTTP 请求，而是按不可用策略处理。这个显式配置是服务级凭据，不是每个绑定的独立凭据，也不会在正常 runtime token 存在时覆盖它。

Hook 服务需要验证调用的是获准的 Group Service，再使用 body 中的 `actor` 查询业务资格；它接收到的服务身份不是成员本人的登录凭据。来源身份验证可使用应用现有的 token 验证设施，不能仅凭 body 的 `actor` 放行。业务资格映射及数据库由应用自己提供。

正常允许或业务拒绝均返回成功 HTTP 状态和 JSON：

```json
{ "decision": "allow" }
```

```json
{ "decision": "deny" }
```

服务只解释 `decision`。错误 HTTP 状态、超时、非法 JSON、缺失或未知 decision 都视为不可用。**业务拒绝应返回成功状态加 `decision: "deny"`**；返回 HTTP 403 会进入不可用分支，在 `native_only` 策略下可能按原生权限继续。

当前返回协议不支持替换消息、指定新的成员角色或直接返回持久化权限修改。需要改变群状态时，应由有权限的应用主体另外调用管理 RPC。

### 5.4 故障、缓存和幂等

| 情况 | 当前结果 |
| --- | --- |
| `decision: "deny"` | 返回 `hook-denied`，任何不可用策略都不能覆盖明确拒绝 |
| 不可用且 `on_unavailable: "deny"` | 写操作返回 `hook-unavailable`、不提交，调用方可重试；读取路径拒绝或过滤对应 Session 内容 |
| 不可用且 `on_unavailable: "native_only"` | 回到原生许可；原生可发言者继续，依赖动态许可者失去本次许可 |
| `read` 允许且 `result_ttl_ms > 0` | 允许结果在进程内按群、Session、读者、客户端、配置/Session revision 及服务入口隔离缓存 |
| 同一个消息对象重复提交 | 已有成功或终局拒绝结果直接复用；不会再次调用 `post` Hook。Hook 不可用不保存为终局消息结果 |

`result_ttl_ms` 仅用于 `read` 的允许缓存；希望业务撤权立即作用于下一次读取时设为 `0`。读者已经获取的副本不会被回收。

`group.list_sessions` 和 `group.changes` 会把读取 Hook 拒绝或不可用的对应内容过滤掉，可能返回成功但列表不完整；直接读取该 Session 或对象则返回错误。客户端不能仅凭列表请求成功就认定所有读取依赖都健康。

多个 Hook 串行执行，明确拒绝立即终止。`native_only` 回退会重置本次累计许可为原生许可，后续绑定仍继续执行；不要把多绑定当作通用布尔表达式处理链。

只绑定 `join` 时，已有成员的消息操作不会执行这个 Hook。但当前同群写操作在数据库事务中串行，写路径上的 Hook 等待可能拖慢同群其他写操作。因此，应设置短超时，让 Hook 只做快速资格查询；不要把它用作长任务、通知或积分结算的提交回调。Hook 判定发生在消息提交前，不是业务数据库与群消息之间的分布式事务。

**默认 UI 的限制：** `group.check_access(action: "session.post")` 当前只检查原生发言许可，不执行 `post` Hook。仅靠 `may_grant` 获得发言权的人，可能仍被 Message Hub 禁用输入；反过来，通过预检查也可能在提交时被 Hook 拒绝。业务动态发言场景应由自定义 UI 正确处理实际提交结果；统一的动态许可预检查和 UI 呈现**待实现**。Message Hub 展示 Hook 绑定、内容可见性及依赖说明也**待实现**。

## 6. L4A：在自己的应用里使用群聊

### 6.1 最小调用流程

1. 以已认证用户身份调用 `group.list_by_member` 或 `group.get_doc`，定位群。
2. 调用 `group.list_sessions`，使用服务返回的可访问 Session，不从配置推测用户能看到哪些会话。
3. 在 host 上调用 `group.list_messages` 获取序号和 ObjectId，再以同一身份调用 `msg.get_message` 获取正文。对成员本地投影，可沿用 Message Hub 的 `msg.list_sessions` / `msg.list_session` 数据层。
4. 发消息调用 `msg.post_send`；读取后用 `group.update_read_marker(last_read_seq)` 推进已读序号。订阅本地 `box_changed` 或轮询刷新投影。
5. 如需同步变更，调用 `group.changes` 并保存返回的读者专属 opaque token；token 无效时按 Session 水位补同步。

这些管理 RPC 面向当前 msg-center 托管的群。Joined Group 的本地数据读取与跨 Zone 发送另见下一节，不能把所有本地 `group.*` 方法都当成远端代理。

例如向公告会话发送消息（调用者须具备相应许可）：

```ts
type PostSendResult = { ok: boolean; msg_id: string; reason?: string }

// senderDid 必须是当前认证主体的 DID，由用户资料/身份上下文取得。
async function postAnnouncement(senderDid: string, text: string) {
  const request = {
    idempotency_key: crypto.randomUUID(),
    msg: {
      from: senderDid,
      to: [groupDid],
      kind: 'group_msg',
      to_session: 'announcements',
      created_at_ms: Date.now(),
      nonce: crypto.getRandomValues(new Uint32Array(1))[0],
      content: { format: 'text/plain', content: text },
    },
  }
  const result = await call<PostSendResult>('msg.post_send', request)
  if (!result.ok) throw new Error(result.reason ?? 'message-rejected')
  return { request, result }
}
```

实际应用应在发送前保存 `request`，以便结果未知时复用同一幂等键和请求。`msg.post_send` 成功返回还可能带投递任务；发送到远端时不能将本地接收请求等同于 host 已接受，应跟踪投递状态。读取和显示顺序以 host 的 `seq` 为准，不能用 `created_at_ms` 做群历史游标。终局拒绝后重新发起业务操作应生成新请求；对同一 ObjectId 重试只会得到原结论。

群读者的消息 `to` 始终是群 DID；个人邮箱记录的 `owner` 才是读者。发件人只保存自己的 SENT 记录，后台不再向其 INBOX 重复投影同一条群消息。

### 6.2 限制只允许自己的应用访问

可通过配置设置本地应用身份白名单：

```json
{
  "access": {
    "allowed_clients": { "only": ["your-verified-appid"] },
    "allow_unverified_remote": false
  }
}
```

白名单值要与 token 验证后得到的 `appid` 完全匹配，不是任意页面名称或 URL 参数。操作仍受成员资格和群能力限制；网页隐藏 Message Hub 入口不能代替授权。写入白名单前，应确认当前调用应用及后续管理入口也在允许集合中。

现有群 HTTP 入口将调用者视为远端且没有已验证客户端 ID，因此 `only` 默认拒绝该路径。开启 `allow_unverified_remote` 会放过远端的**客户端身份限制**，仍检查用户和成员权限，但不再保证远端只能通过指定应用访问。跨 Zone 的客户端身份标准证明**待实现**。

### 6.3 复用 Message Hub 源码

| 源码入口 | 可参考或复制的能力 |
| --- | --- |
| [datamodel/sessionApi.ts](../../src/frame/desktop/src/app/messagehub/datamodel/sessionApi.ts) | kRPC 调用与常用 wire 类型 |
| [api/store.ts](../../src/frame/desktop/src/app/messagehub/api/store.ts) | 用户投影、群管理、发送、幂等和本地事件刷新 |
| [protocol/msgobj.ts](../../src/frame/desktop/src/app/messagehub/protocol/msgobj.ts) | MsgObject、Session ID、关系和提及类型 |
| [ConversationView.tsx](../../src/frame/desktop/src/app/messagehub/ConversationView.tsx) | 聊天视图，组合历史、输入及媒体能力 |
| [GroupPanel.tsx](../../src/frame/desktop/src/app/messagehub/GroupPanel.tsx) / [SessionDetails.tsx](../../src/frame/desktop/src/app/messagehub/SessionDetails.tsx) | 群成员管理、共享状态和 Session 参与者 UI |

这是源码级集成。组件依赖现有 store、登录身份上下文、i18n、样式、桌面对话框和对象访问层，并非单独拷贝一个 TSX 文件就能运行。可先复用服务调用与协议类型，再按应用需要迁入界面。不要把 `mock/` 中的演示数据和能力当成后台承诺；正式环境需要使用真实 API store。

## 7. 跨 Zone 与 L4B：当前对接范围

默认服务已实现以下群 HTTP 路径；具名 Session 的 `<sid>` 必须按 MailboxAddress 编码为单个路径段。优先使用服务返回的规范地址构造路径。

| 路径 | 方法与用途 |
| --- | --- |
| `/<group_did>/doc`、`/<group_did>/did.json` | GET：公开 Doc 摘要 |
| `/<group_did>/sessions` | GET：可访问会话 |
| `/<group_did>/inbox`、`/<group_did>/sessions/<sid>/inbox` | GET：消息索引；PUT：提交消息 |
| `/<group_did>/join`、`/<group_did>/sessions/<sid>/join` | PUT：申请/接受群邀请，或接受 Guest 邀请 |
| `/<group_did>/guest_requests` | PUT：按模板创建 Guest 请求会话 |
| `/<group_did>/changes` | GET：按读者过滤的变更流 |
| `/<group_did>/read_markers` | PUT：推进本人已读序号 |
| `/<group_did>/objects/<obj_id>` | GET：经权限检查的消息或附件对象 |

除公开 Doc 外，需要 **host 可验证**的 `Authorization: Bearer ...` 和与 token 主体一致的 `cyfs-original-user`。不能假定成员自己 Zone 签发的 token 自然就被所有 host 接受。当前不再要求 `cyfs-proofs`。

HTTP 发送必须提供合法 canonical MsgObject JSON 或 `application/cyfs-named-object+jwt` 及相应 `cyfs-obj-id`；JWT 必须通过验证，读取保留签名原文。成功有 accepted 状态及 `cyfs-session-seq`。附件访问还需 `context_path=<规范群 Session 地址>/<引用消息 ObjectId>`，服务会检查消息可读且实际引用目标对象。

### 7.1 显式 Joined Group 绑定

当前通过成员所在 Zone 的 msg-center Settings 登记路由和访问凭据，例如：

```json
{
  "cyfs_dispatch": {
    "target_zone": "member.example",
    "joined_groups": [{
      "owner_did": "did:web:alice.member.example",
      "group_did": "did:web:guild.host.example",
      "host": "host.example",
      "upstream": "https://host.example/",
      "authorization": "Bearer <host可验证的该成员凭据>"
    }]
  }
}
```

这里的 `owner_did` 是本地消息投影的所有者，不是远端群主。host 上必须已经有有效成员/Guest 关系；绑定本身不授予群访问权。配置按现有 Settings 流程写入并调用 `reload_settings` 刷新，不要向浏览器分发整份服务 Settings。

后台每 30 秒同步一次，也可由该成员调用 `group.sync_joined(group_did)`。它获取 Session、变更和必要正文，在本地事务中保存投影与水位；发送则通过对应成员绑定投递到远端 host。远端变化当前靠轮询，本地投影提交会产生 `box_changed` 事件。

### 7.2 实际部署与独立实现的责任

群路径需要 Gateway 转发到 msg-center；公开 GroupDoc 的 resolver 发布还依赖 msg-center 的既有服务身份有控制面 cache 写权限。创建群的数据库事务成功与公开 DID 发布成功不是同一步：已有发布待办和后台重试，但仍需检查 resolver 和 Gateway 的真实访问链路。

L4B 服务应实现 v2 §11.1 的协议语义，包括权限过滤、序号、幂等、变化流、对象获取、签名和回执。现有 Joined Group 同步器可作为对接客户端，但这不等于任意第三方服务已能自动出现在 Message Hub。独立实现不继承默认配置、Hook、存储和可靠性保障；跨实现兼容性需要实际联调。

**待实现或待联调：** 自动从 DID endpoint 建立连接、自动维护成员 Joined Group 绑定与凭据续期、跨 Zone 邀请自动接受、远端变更通知、客户端身份标准证明，以及真实 Gateway/resolver 和跨 Zone token 签发的完整闭环。

## 8. 数据责任与开发验证

默认实现是**单 host 权威服务**，群配置、成员、Session、消息元数据/正文、序号、状态和幂等结果持久化到平台 RDB。相关状态变更与消息投影使用事务，已有重启恢复测试。附件对象仍由 NamedStore/CYFS 保存，完整恢复需要覆盖相应对象存储。

同一个 Group 的主从复制、多副本切换以及面向开发者的完整群导入/导出和备份恢复工具均**待实现**。普通成员的历史投影不是完整群备份；平台 RDB 的持久化也不等同于自动多副本。应用的积分、账号映射、Hook 业务数据库和页面部署由应用自己备份。

开发时建议验证以下实际行为：

| 验证场景 | 需要确认的结果 |
| --- | --- |
| 配置生成器退出、msg-center 重启 | 群仍能查询、收发，配置和 Session 保持 |
| 建群或写操作超时重试 | 原请求和幂等键得到相同结果；参数变化不能复用同键 |
| Member、Admin、Guest 分别访问 | Session 和正文不越权；Admin 不自动拥有全群读取权，Guest 仅限自己的会话 |
| 修改模板/配置/共享状态 | 使用各自 revision；冲突重新读取，模板变更实际影响所引用会话 |
| Hook 正常拒绝、HTTP 403、超时、缺少服务凭据 | 区分业务拒绝与不可用，按 `deny` / `native_only` 策略执行 |
| 原生发言受限且 `may_grant: true` | 实际提交可动态放行；默认 Message Hub 的输入状态限制另行处理 |
| 客户端白名单与远端 HTTP | 使用真实 token 的 appid 判定；确认远端默认被拒，例外没有扩大成员权限 |
| 跨 Zone 建立绑定并收发 | host 验证用户凭据、路由正确、轮询投影和投递结果可追踪 |

现有回归入口包括 [test_group_service.rs](../../src/frame/msg_center/src/test_group_service.rs)（事务、权限、Guest、Hook、HTTP、重启等）和 [messagehub-group.test.ts](../../src/frame/desktop/tests/datamodel/messagehub-group.test.ts)（群通知、关系、提及和参与者展示等）。后台测试可在 `buckyos/src` 执行：

```bash
cargo test -p msg_center test_group_service -- --test-threads=1
```

测试代码的存在不代表目标部署已验证。真实跨 Zone、PostgreSQL、最大容量、多副本以及 L2 面板故障隔离需要各自在对应实现/环境中验证。

实现入口：[group_service.rs](../../src/frame/msg_center/src/group_service.rs)、[group_types.rs](../../src/frame/msg_center/src/group_types.rs)、[group_http.rs](../../src/frame/msg_center/src/group_http.rs)、[group_sync.rs](../../src/frame/msg_center/src/group_sync.rs)。持久化范围和性能边界见 [群 v2 存储格式](../../src/frame/msg_center/doc/group-v2-storage.md)。
