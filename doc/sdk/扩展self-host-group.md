# 扩展 Self-host Group：应用开发指南

本文面向希望在 BuckyOS 群聊上增加业务能力的应用开发者。实现核对日期为 **2026-10-24**，基于当前工作区（[应用扩展实施 TODO](../../notepads/self-host-group-extension-todo.md) 阶段 A～D 的改动）；“已实现”表示当前源码存在相应执行路径，不代表已经完成真实部署或跨 Zone 联调。

**现在可以开发配置生成器、嵌入式应用侧面板、HTTP Hook、使用默认群服务的自定义应用，以及为外部客户代登录；通用 Process Chain、自动跨 Zone 接入和群服务多副本仍需补齐。** 可落地的组合是 L1 + L2 + L3（Message Hub 内增强）和 L1 + L4A + L3（自己的 UI 服务外部客户）。

本文将 [应用扩展架构设计](<../message_hub/BuckyOS Self-host Group的应用扩展架构设计.md>) 的构想对应到现有代码。协议背景见 [Self-host Group v2](../message_hub/Self-Host-Groupv2.md)，完整 RPC 参数见 [群服务后台接口](../../src/frame/msg_center/doc/group-v2-backend.md)。架构稿中的长期兼容性属于设计目标；当前仓库仍处于 beta 2.2 开发阶段，不能据此假定 API、配置或 UI 源码已经有长期兼容承诺。

## 1. 选择扩展方式

| 方式 | 当前状态 | 开发者现在能做什么 | 尚缺的能力 |
| --- | --- | --- | --- |
| L1 配置型扩展 | 已实现（含 `display`） | 原子创建群、初始 Session 和邀请；配置成员策略、能力、Session 模板、消息规则、面板声明和展示配置；初始化程序随后退出 | 完整配置编辑 UI |
| L2 嵌入式应用侧面板 | 已实现（§4） | 在 `extensions.panels` 声明 https 入口，Message Hub 在群 / Session 详情旁以 iframe 加载，访客页同样显示 | `postMessage` 通道；跨来源面板的用户接受 UX（v1 直接拒绝） |
| L3 Hook / Process Chain | HTTP Hook 已实现（HMAC 签名出站），通用处理链待实现 | 在 `join`、`post`、`read` 上调用应用 HTTP 服务，拒绝操作或按配置提供动态发言许可；Message Hub 展示绑定并按 `hook_dependent` 放开输入框 | 通用 Process Chain 接入、其他 Hook 点、返回消息修改或持久化权限变更指令 |
| L4A 自定义 UI | 已实现（§6，含外部客户代登录） | 自己实现业务界面，调用 `group.*`、`msg.*`；以 App 身份为外部客户签发一次性登录码；按需复制 Message Hub 组件和数据层 | 独立发布的稳定聊天组件 SDK；跨 Zone 应用身份的标准证明；External（DID 登录）进 App |
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
| `access` | 本地经过验证的应用身份白名单（值是 token 的 `appid`，系统界面统一为 `control-panel`，见 §6.2）；远端未验证客户端身份的处理方式 |
| `display` | 群级展示：`announcement`、`links[{title,url}]`（https）、`files[{title,obj_id}]`、`featured_members[did]`（须是活跃成员）；`SessionRules.display` 同构，Session 字段覆盖群字段；只展示、不授权 |
| `extensions.panels` | 面板声明 `[{id,title,entry_url,scope}]`，见 §4；每群最多 `limits.max_panels`（默认 8） |
| `rules.panels` | 引用已声明的面板 `id`，把 `scope` 未覆盖的面板额外挂到该 Session |
| `rules.hooks[].secret` | 由服务生成，只在写入响应的 `hook_secrets` 中出现一次，`get_config` 不回显；见 §5.3 |

Session 成员策略采用以下 JSON：`"inherit"`、`{"roles":["owner","admin"]}` 或 `"explicit"`。发言和回应规则采用 `"all_participants"`、`{"only":["owner","admin"]}` 或 `"nobody"`。历史范围是 `"all"` 或 `"from_join"`。

当前配置 `schema_version` 只接受 `1`。`required_features` 仅识别 `sessions`、`session_guests`、`message_relations`、`read_markers`、`hooks`、`allowed_clients`、`panels`；未知必需能力会报 `unsupported-required-feature:<name>`。未知顶层字段以及 `extensions` 下除 `panels` 以外的数据会保留，但不会自动成为运行逻辑。完整字段和默认值以 [group_types.rs](../../src/frame/msg_center/src/group_types.rs) 为准。

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

## 4. L2：嵌入式应用侧面板

面板在 `extensions.panels` 中声明，Message Hub 在群详情 / Session 详情旁（手机为二级页）按 Tab 加载；访客页 `/messagehub/guest` 也显示访客所在 Session 的面板。

```ts
await call('group.apply_config', {
  group_did: groupDid,
  expected_revision: current.revision,
  idempotency_key: crypto.randomUUID(),
  patch: {
    extensions: {
      panels: [
        { id: 'tickets', title: '工单', entry_url: 'https://crm.guild.example/panel', scope: 'group' },
        { id: 'sla', title: 'SLA', entry_url: 'https://crm.guild.example/sla', scope: { sessions: ['vip'] } },
      ],
    },
    session_templates: { support: { rules: { panels: ['tickets'] } } },
  },
})
```

**声明规则（`apply_config` / `group.create` 校验，不符即拒绝 `invalid-panel-decl:*`）：**

- `id` 是 DNS 标签（小写字母、数字、`-`），群内唯一；`title` 1～64 字符。
- `entry_url` 必须是 https，不能带用户名密码或 fragment，**host 必须是群主 Zone 的 App 子域**：`<app>.<host_zone>` 或 `<app>-<host_zone>`，`host_zone` 取自群 Doc 的 `host`（D-2）。外部来源直接拒绝；v1 没有“用户接受外部来源”的 UX。
- `scope` 是 `"group"`（每个 Session 都显示，含默认 Session）或 `{ "sessions": [sid] }`（只在这些具名 Session 显示，Session 必须存在）。`SessionRules.panels` 可以额外把某个面板挂到一个 Session 上，引用不存在的 `id` 报 `unknown-panel:<id>`。
- 每群最多 `limits.max_panels`（默认 8）个声明；`extensions.panels` 在 patch 中整体替换。
- `required_features` 可声明 `panels`。

**宿主行为（Message Hub）：**

- `group.list_sessions` 的每个条目带 `panels: [{id,title,entry_url}]`，是该 Session 的生效面板（`scope` 覆盖 ∪ 规则引用）。`group.get_doc` 不含面板。
- 加载前 Message Hub 再核一次 `entry_url` host 与群 Doc `host`（第二道，自托管 Zone 的 host 用 `owner_canonical_zone` 解析，不走 `binded_zone_list`）；不符显示“来源不受信”，不加载。
- 以 `<iframe sandbox="allow-scripts allow-same-origin allow-forms" referrerpolicy="origin">` 加载 `entry_url?group_did=<群 DID>&session_id=<原始 sid 或空>&session=<规范 MailboxAddress>&locale=<zh-CN 等>`（追加到已有 query）。切换 Session 时换 `src`。**不传任何 token、viewer DID 或 cookie**（v2 §14#5）。
- 加载前先对 `entry_url` 做一次 `no-cors` 可达性探测（Chromium 对跨源 iframe 的网络错误页也会触发 `load`，所以不能只靠 `onload`）：探测失败、10 秒内未完成或 `onerror` 都显示“面板不可用”并提供重试。**只能识别网络层失败**：网关对不存在的 App 子域回空 403、对停掉的容器回 5xx，这些页面会原样显示在 iframe 里（跨源 iframe 无法读到状态码）；要让宿主判出 HTTP 层故障需要面板侧就绪握手（`postMessage`，不在本批）；面板区有独立 Error Boundary，面板崩溃不影响会话列表和消息视图。

**面板如何访问服务：** 面板页面运行在 App 子域，用自己 App 的会话（host-only cookie）直接调用 `/kapi/msg-center`——网关按路径转发服务，不看 host，msg-center 看到的是 `principal_kind=user`、`appid=<该 App>`。因此 L2 面板与 L4A 自定义 UI 共用同一身份路径，面板不需要新的凭据；`group_did` / `session_id` 只是定位上下文，权限仍按群与 Session ACL 判定。iframe 内无该 App 会话时网关 `check_oauth` 会 302 到登录页（桌面免密 `prepare_app_launch` 只作用于顶层导航），面板应在页内检测未登录并给出“在新窗口打开”的入口。iframe 内 host-only cookie 的送达取决于浏览器第三方 cookie 策略：同站（`*.<zone>`）在主流浏览器可用，跨站部署不保证——这两点属于真实浏览器验证项，尚未在真实 Zone 复核。

mock 模式下 `public/messagehub/mock-panel.html` 是一个本地样例面板，Playwright 用例覆盖显示、切换、失败态与来源拒绝；最小样例应用见 [group_ext_sample](../../test/test_msg_center/group_ext_sample/)。

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
// → { revision, hook_secrets: [{ scope: 'default_session', index: 0, secret: '<base64url 32 字节>' }] }
```

`service` 必须是可从 msg-center 访问的 HTTP/HTTPS URL，不能带 URL 用户名/密码；调用不跟随重定向。`timeout_ms` 范围为 1–30000。`may_grant: true` 只允许用于 `post`。Hook 数组会整体替换，更新时合并需要保留的绑定。

**secret。** 每个绑定有一个 `secret`：写入时不带 `secret` 的绑定由服务生成 32 字节随机值，并只在这次写入的响应 `hook_secrets` 里出现一次（`scope` 为 `default_session`、`template:<id>` 或 `session:<sid>`，后者来自 `create_session` / `update_session` 的 `rule_overrides.hooks`）。`get_config`、`list_sessions`、变更事件都不回显 secret；重新提交不带 `secret` 的 Hook 数组会生成新值。轮换用 `group.rotate_hook_secret {session_id?, index}`：默认 Session 或模板 Hook 需要 `group.update_config`，Session 覆盖项的 Hook 需要该 Session 的 `session.manage`；返回 `{revision, secret}`。

### 5.3 HTTP 请求、认证和返回值

Group Service 发出 `POST`，`content-type: application/json`，body 例如：

```json
{
  "operation": "join",
  "actor": "did:web:alice.example",
  "group_did": "did:web:guild.example",
  "group_host": "did:web:guild-host.example",
  "session_id": null,
  "msg": null
}
```

具名会话的 `session_id` 为原始字符串。`group_host` 是群所在 Zone 的 DID，供同一个 Hook 服务区分来源 Zone。`post` 请求的 `msg` 是完整 MsgObject，包含消息内容；当前请求没有另外传入客户端 ID、配置 revision 或完整成员表。

**认证改为按绑定签名（D-3），请求不再携带 msg-center 的服务 token。** 两个头：

- `X-BuckyOS-Hook-Timestamp`：Unix 秒；
- `X-BuckyOS-Hook-Signature`：小写 hex 的 `HMAC-SHA256(secret, "<timestamp>." + body原始字节)`。

Hook 服务用该绑定的 `secret` 验签，并拒绝与本地时钟偏差超过 5 分钟的时间戳；不依赖 BuckyOS runtime，也没有任何 BuckyOS 服务身份可以从请求里拿到。验签失败应返回 401（msg-center 按“不可用”处理，`deny` 策略下操作被拒），不能仅凭 body 的 `actor` 放行。Deno 样例：

```ts
const encoder = new TextEncoder()
const key = await crypto.subtle.importKey('raw', encoder.encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['verify'])

async function verifyHook(req: Request): Promise<{ ok: boolean; body: string }> {
  const body = await req.text()
  const ts = req.headers.get('x-buckyos-hook-timestamp') ?? ''
  const sig = req.headers.get('x-buckyos-hook-signature') ?? ''
  if (!/^\d+$/.test(ts) || Math.abs(Date.now() / 1000 - Number(ts)) > 300) return { ok: false, body }
  const bytes = Uint8Array.from(sig.match(/../g)?.map((h) => parseInt(h, 16)) ?? [])
  const ok = sig.length === 64 &&
    (await crypto.subtle.verify('HMAC', key, bytes, encoder.encode(`${ts}.${body}`)))
  return { ok, body }
}
```

参考实现见 [buckyos-api `group_hook.rs`](../../src/kernel/buckyos-api/src/group_hook.rs)（`verify_group_hook_signature`）与样例 [group_ext_sample/hook_server.ts](../../test/test_msg_center/group_ext_sample/hook_server.ts)。业务资格映射及数据库由应用自己提供。

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

**默认 UI：** `group.check_access(action: "session.post")` 对配置了 `may_grant` 的 `post` Hook 的 Session 返回 `allowed: "hook_dependent"`（其余仍是 `true` / `false`），Message Hub 据此不禁用输入框，提交被 `hook-denied` 拒绝时在气泡上提示；预检查不执行 `post` Hook，提交结果以服务为准。群信息页展示“本群绑定了 N 个业务 Hook（join / post / read）”与各 Hook 的 service host，`post` Hook 另加“业务服务可读取消息内容”说明。

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

白名单值要与 token 验证后得到的 `appid` 完全匹配，不是任意页面名称或 URL 参数。**系统界面（Message Hub、Desktop、AI Workspace 等根域页面）都在 `System{control-panel}` 会话下，`appid` 统一是 `control-panel`**（D-4）：`only: ["your-app"]` 会把根域 Message Hub 拒之门外，`only: ["your-app", "control-panel"]` 则同时放行全部系统界面——v1 无法单独表达“只允许 Message Hub”，这是已知限制。操作仍受成员资格和群能力限制；网页隐藏 Message Hub 入口不能代替授权。写入白名单前，应确认当前调用应用及后续管理入口也在允许集合中。

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

### 6.4 让外部客户进来：两条路径

外部客户（没有本 Zone 普通账号的人）有两条进入业务群 Session 的路径，都以 **Limited 账号** 为主体（混合 SSO 契约 §2、§5）：

1. **Message Hub 访客页**（无自定义 UI 时的默认入口）：管理员在 CP 以 `user.exclusive_invite.create` 为客户建号并签发专属邀请链接，客户打开 `/sso/invite/<id>#s=<secret>` 后进入 `/messagehub/guest?group=&session=`，只看到自己所在的 Session，也能看到该 Session 的面板与 `display`。或者开放群的 `membership.guest_entry.auth_methods`，由客户自助注册 / 密码 / DID 登录后提交咨询。
2. **App 代登录 + 自定义 UI**（EXT-09，D-5 / D-7）：App 自己验证客户（手机号、CRM 账号……），用 **App 的服务身份**向 CP 领取一次性登录码，客户在 **App 子域**页面兑换成指向该 App 实例的受限会话，再以本人 Limited 身份调 `group.*`。流程：

```ts
// App 后端（Deno / Node），client 是持 App 服务 token 的 control-panel kRPC 客户端
import { issueAppLoginCode } from 'buckyos'
const issued = await issueAppLoginCode(client, {
  external_account_id: 'crm:42',            // 只存摘要
  show_name: 'Alice',                        // 新建账号的显示名
  resource: { kind: 'group_session', group_did: groupDid, session_id: 'ticket-42' },
  return_path: '/tickets/42',                // App 子域上的路径
})
// issued = { user_id: 'crm-<16hex>', code: '<invite_id>.<secret>', expires_at, new_account, invite }
// 把 code 放进给客户的链接 fragment：https://crm.<zone>/login#code=<code>

// App 页面（浏览器）
import { completeAppAccessTokenLogin } from 'buckyos'
await completeAppAccessTokenLogin({ code, nonce: sessionStorage.nonce ??= crypto.randomUUID(), return_path: '/tickets/42' })
// ready → 跳到 callback_url（App 子域 /sso_callback），cookie 落在 App 子域，再回到 return_path
```

规则：

- **开关**：App 实例的 owner 在 CP 用 `apps.app_login.set {app_instance_id, enabled}` 打开代登录；默认关。关闭时该 App 签入的全部会话立即不能 refresh（`source_revoked` / `app_login_disabled`）。
- **签发** `user.app_login.issue`：调用者必须是 `principal_kind=app` 的服务会话，且 target 是自己的实例；按 `(app_instance_id, sha256(external_account_id))` 查绑定表 `services/control_panel/app_identities/<instance>/<digest>`，无则创建 Limited 账号 `<app>-<16hex>`（不入系统组、`password_state=unset`）；显式 `user_id` 必须等于已绑定账号，不能指向任意账号。code 有效 10 分钟、单次消费；每实例每分钟最多 60 次。`resource` 为 Session 时 CP 用 App 的服务会话调 `group.invite_session_guest`——msg-center 让 App 的服务会话以其 owner 的 DID 行事、`appid` 为客户端，因此 App owner 须在该群有 `session.invite_guest`，`allowed_clients` 也要放行该 App。
- **兑换** `auth.app_login.exchange {code, nonce, return_path?}`（公开、按来源 IP 限流）：CP 代调 verify-hub `login_by_app_access_token`，签发 `auth_method=app_access_token`、`restricted=true`、`scopes=[msg_group, account_self]`、target=该 App 实例的 token pair；同 nonce 重试返回同一结果，换 nonce 拒绝 `nonce_mismatch`；之后 code 记录标为 `consumed`。回包是 `{status:"ready", callback_url, user_id}` 或 `{status:"denied", denial}`。
- **msg-center 侧**无新分支：该账号以 Limited 进 `group.*`，只读写被邀请的 Session；新账号没有个人投递记录。
- **不能做的**：App service token 本身不能换成客户会话；External（DID 登录）仍不能 delegate 进 App；Message Hub 不提供自己的兑换中心。

验收矩阵见 TODO §7；样例应用见 [group_ext_sample](../../test/test_msg_center/group_ext_sample/)。

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
| Hook 正常拒绝、HTTP 403、超时、验签失败 | 区分业务拒绝与不可用，按 `deny` / `native_only` 策略执行；出站请求不含 msg-center 服务 token |
| 原生发言受限且 `may_grant: true` | `check_access` 返回 `hook_dependent`，输入框不禁用；实际提交以服务为准 |
| 面板服务停掉 | 面板显示不可用，消息视图与发送正常；访客页同样 |
| 外部来源面板被写进配置 | `apply_config` 拒绝；手工改库后 Message Hub 第二道拒绝加载 |
| 客户端白名单与远端 HTTP | 使用真实 token 的 appid 判定（系统界面为 `control-panel`）；确认远端默认被拒，例外没有扩大成员权限 |
| 外部客户经 App 代登录 | 只能进签发 code 的 App 实例；换设备重新领取 code 映射同一账号；App 关闭代登录后 refresh 被拒 |
| 跨 Zone 建立绑定并收发 | host 验证用户凭据、路由正确、轮询投影和投递结果可追踪 |

现有回归入口包括 [test_group_service.rs](../../src/frame/msg_center/src/test_group_service.rs)（事务、权限、Guest、Hook 签名与 secret、面板声明、`display`、HTTP、重启等）、[messagehub-group.test.ts](../../src/frame/desktop/tests/datamodel/messagehub-group.test.ts)（群通知、关系、提及和参与者展示等）、verify-hub / control-panel 的 App 代登录单测，以及 Message Hub 的 Playwright 面板用例。后台测试可在 `buckyos/src` 执行：

```bash
cargo test -p msg_center test_group_service -- --test-threads=1
cargo test -p verify_hub app_access_token -- --test-threads=1
cargo test -p control_panel app_login:: -- --test-threads=1
```

真实 Zone 的端到端脚本见 [test/test_msg_center/group_ext_sample](../../test/test_msg_center/group_ext_sample/)。

测试代码的存在不代表目标部署已验证。真实跨 Zone、PostgreSQL、最大容量、多副本以及 L2 面板故障隔离需要各自在对应实现/环境中验证。

实现入口：[group_service.rs](../../src/frame/msg_center/src/group_service.rs)、[group_types.rs](../../src/frame/msg_center/src/group_types.rs)、[group_http.rs](../../src/frame/msg_center/src/group_http.rs)、[group_sync.rs](../../src/frame/msg_center/src/group_sync.rs)。持久化范围和性能边界见 [群 v2 存储格式](../../src/frame/msg_center/doc/group-v2-storage.md)。
